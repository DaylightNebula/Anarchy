use std::{sync::{Arc, atomic::{AtomicBool, AtomicPtr, Ordering}}, time::Duration};

use chrono::{DateTime, Utc};

use crate::{CowData, DeltaTime, FlexLocalId, Scheduler, TaskID, World, scheduler::{ScheduleID, schedule::Schedule}};

/// Represents an active task
pub struct Task {
    id: TaskID,
    task: Box<dyn FnMut(TaskID) -> () + Send + 'static>,
    tick_rate: u32,
    kill: Arc<AtomicBool>,
    last_run: CowData<DateTime<Utc>>
}

impl Task {
    /// Create a new task to run at the given target runtime.
    pub fn repeating<F>(task: F, tick_rate: u32) -> Self
        where 
            F: FnMut(TaskID) -> () + Send + 'static
    {
        // get thread ID and signal channels
        let thread_id = Scheduler::next_task_id();
        let kill = Arc::new(AtomicBool::new(false));

        Self {
            id: thread_id,
            task: Box::new(task),
            tick_rate,
            kill,
            last_run: CowData::new(Utc::now())
        }
    }

    /// Get the ID of this task
    pub fn id(&self) -> TaskID {
        self.id
    }

    /// Kill this task
    pub fn kill(&self) {
        self.kill.store(true, Ordering::Release);
    }

    /// Represents the tick rate of this task.  This is the number
    /// of times per second this task should run when able.  If this
    /// number is 0, the task should not be run multiple times.
    pub fn tick_rate(&self) -> u32 { self.tick_rate }

    /// When the `single-threaded-executors` feature is enabled, this
    /// function is used to execute this task if enough time has passed
    /// where the task should be executed.
    pub fn tick(&mut self, tick_start_time: DateTime<Utc>) {
        if self.kill.load(Ordering::Acquire) { return }

        let target_runtime = Duration::from_secs_f32(1.0 / self.tick_rate as f32);
        if tick_start_time.signed_duration_since(*self.last_run.get_ref()).to_std().unwrap_or(Duration::MAX) > target_runtime {
            (self.task)(self.id);
            self.last_run.set(tick_start_time);
        }
    }
}

/// An executor for to run the contained schedule.
pub struct ScheduleExecutor {
    id: ScheduleID,
    next_schedule: Arc<AtomicPtr<Schedule>>,
    next_lock: Arc<AtomicBool>,
    stop: Arc<AtomicBool>
}

impl ScheduleExecutor {
    /// Returns a reference to the ID of the contained schedule.
    pub fn id(&self) -> &ScheduleID { &self.id }

    /// Returns a mutable reference to the next schedule to be run.
    /// Useful for add functions to execute.
    pub(crate) fn next_schedule(&self) -> &mut Schedule { unsafe { &mut *self.next_schedule.load(Ordering::Acquire) } }

    /// Locks the next schedule tracker.
    pub(crate) fn lock_next_schedule(&self) { self.next_lock.store(true, Ordering::Release); }

    /// Unlocks the next schedule tracker.
    pub(crate) fn unlock_next_schedule(&self) { self.next_lock.store(false, Ordering::Release); }

    /// Creates a new executor with the given `ScheduleID` and `Schedule`.
    pub fn new(id: ScheduleID, schedule: Schedule) -> Self {
        let schedule = Box::leak(Box::new(schedule));
        Self {
            id,
            stop: Arc::new(AtomicBool::new(false)),
            next_schedule: Arc::new(AtomicPtr::new(schedule)),
            next_lock: Arc::new(AtomicBool::new(false))
        }
    }

    /// Shutsdown this executor.  This function returns immeidiately and does not wait for the executor to stop.
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Starts this executor, executing over the given `World`.
    pub fn start(&self, world: World) -> Vec<Task> {
        let schedule_id = *self.id();
        let tick_rate = self.id().tick_rate;
        let mut threads = Vec::with_capacity(schedule_id.max_threads as usize);

        // setup delta time globally or just for this schedule if one already exists
        if let Some(delta) = world.get_resource_mut::<DeltaTime>() {
            delta.set(FlexLocalId::Schedule(schedule_id), 0.0);
        } else {
            let delta = DeltaTime::default();
            delta.set(FlexLocalId::Schedule(schedule_id), 0.0);
            world.insert_resource(delta);
        }

        let current_schedule = Arc::new(AtomicPtr::new(std::ptr::null_mut::<Schedule>()));
        let next_schedule = self.next_schedule.clone();
        let mut last_time = Utc::now();

        threads.push(Task::repeating(move |_task_id| {
            // update current schedule
            let new_schedule = Box::leak(Box::new(Schedule::new_empty()));
            let schedule = unsafe { &mut *next_schedule.swap(new_schedule, Ordering::Release) };
            let old_schedule_ptr = current_schedule.swap(schedule, Ordering::Release);
            if !old_schedule_ptr.is_null() {
                let old_schedule = unsafe { &mut *old_schedule_ptr };
                let _old_schedule = unsafe { Box::from_raw(old_schedule) };
            }

            // execute startup tiles if needed, otherwise, execute and readd update tiles
            if schedule.has_next_startup() {
                while let Some(item) = schedule.next_startup() {
                    item.tile.execute(&world, schedule_id, item.first_run);
                }

                while let Some(item) = schedule.next_update() {
                    new_schedule.add_new(item.tile.clone());
                }
            } else {
                while let Some(item) = schedule.next_update() {
                    item.tile.execute(&world, schedule_id, item.first_run);
                    if !item.dont_save { new_schedule.post_run_add(item.tile.clone(), schedule.total_runtime); }
                }
            }
                    
            // find and update delta time
            let current_time = Utc::now();
            let delta_time = current_time.signed_duration_since(last_time).as_seconds_f32().max(0.0);
            last_time = current_time;
            world.get_resource_ref::<DeltaTime>()
                .expect("DeltaTime was lost!")
                .set(FlexLocalId::Schedule(schedule_id), delta_time);
        }, tick_rate));

        return threads;
    }
}

// Make sure schedule pointer is dropped
impl Drop for ScheduleExecutor {
    fn drop(&mut self) {
        let _schedule = unsafe { 
            Box::from_raw(self.next_schedule.load(Ordering::Acquire)) 
        };
    }
}
