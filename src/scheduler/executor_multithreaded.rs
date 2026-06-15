use std::{collections::LinkedList, sync::{Arc, atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering}}, thread::JoinHandle, time::Duration};

use chrono::Utc;
use mutual::SharedData;

use crate::{*, scheduler::{ScheduleID, schedule::{Schedule, ScheduleIteratorItem}}};

/// Represents an active task
#[allow(dead_code)] // handle needs to be tracked, but technically isn't used
pub struct Task {
    id: u32,
    handle: JoinHandle<()>,
    tick_rate: u32,
    kill: Arc<AtomicBool>
}

impl Task {
    /// Create a new task to run at the given target runtime.
    pub fn repeating<F>(mut task: F, tick_rate: u32) -> Self
        where 
            F: FnMut(TaskID) -> () + Send + 'static
    {
        // get thread ID and signal channels
        let thread_id = Scheduler::next_task_id();
        let kill = Arc::new(AtomicBool::new(false));
        let kill2 = kill.clone();

        Self {
            id: thread_id,
            handle: std::thread::spawn(move || {
                loop {
                    let start = Utc::now();

                    // check if we should shutdown
                    if kill2.load(Ordering::Acquire) { break; }

                    // call repeating task
                    task(thread_id);

                    // get duration and tick length, then calculate how long we 
                    // should wait to preserve the given tick rate
                    let tick_length_nanos = Utc::now().signed_duration_since(start).to_std().map(|a| a.as_nanos() as u64).unwrap_or(0);
                    let wait_time = (1_000_000 / tick_rate as u64).checked_sub(tick_length_nanos);

                    // tick length was less than the tick length, wait for wait_length nanoseconds
                    if let Some(wait_time) = wait_time {
                        if wait_time > 0 {
                            std::thread::sleep(Duration::from_nanos(wait_time));
                        }
                    }
                }
            }),
            tick_rate,
            kill
        }
    }

    /// Get the ID of this task
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Kill this task
    pub fn kill(&self) {
        self.kill.store(true, Ordering::Release);
    }

    /// Represents the target runtime per tick in nano seconds.  Divide
    /// 1 billion by this to get the number of ticks per second this task
    /// should run at.  If this returns 0, this is not a repeating task.
    pub fn tick_rate(&self) -> u32 { self.tick_rate }
}

/// An executor for to run the contained schedule.
pub struct ScheduleExecutor<I: Copy + 'static, O: Clone + 'static> {
    id: ScheduleID,
    next_schedule: Arc<AtomicPtr<Schedule<I, O>>>,
    next_lock: Arc<AtomicBool>,
    stop: Arc<AtomicBool>
}

impl <I: Copy + Send + 'static, O: Clone + 'static> ScheduleExecutor<I, O> {
    /// Returns a reference to the ID of the contained schedule.
    pub fn id(&self) -> &ScheduleID { &self.id }

    /// Returns a mutable reference to the next schedule to be run.
    /// Useful for add functions to execute.
    pub(crate) fn next_schedule(&self) -> &mut Schedule<I, O> { unsafe { &mut *self.next_schedule.load(Ordering::Acquire) } }

    /// Locks the next schedule tracker.
    pub(crate) fn lock_next_schedule(&self) { self.next_lock.store(true, Ordering::Release); }

    /// Unlocks the next schedule tracker.
    pub(crate) fn unlock_next_schedule(&self) { self.next_lock.store(false, Ordering::Release); }

    /// Creates a new executor with the given `ScheduleID` and `Schedule`.
    pub fn new(id: ScheduleID, schedule: Schedule<I, O>) -> Self {
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
    pub fn start(&self, world: World, inputs: I) -> Vec<Task> {
        let id = *self.id();
        let tick_rate = self.id().tick_rate;
        let target_runtime = 1_000_000_000 / tick_rate as u128;
        let mut threads = Vec::with_capacity(id.max_threads as usize);
        let current_schedule = Arc::new(AtomicPtr::new(std::ptr::null_mut::<Schedule<I, O>>()));
        let start_cycle = Arc::new(AtomicBool::new(false));
        let complete_threads = Arc::new(AtomicU32::new(0));

        let tiles = Arc::new(vec![RelaxedMutex::new(LinkedList::new()); id.max_threads as usize].into_boxed_slice());

        // setup delta time globally or just for this schedule if one already exists
        if let Some(delta) = world.get_resource_mut::<DeltaTime>() {
            delta.set(FlexLocalId::Schedule(id), 0.0);
        } else {
            let delta = DeltaTime::default();
            delta.set(FlexLocalId::Schedule(id), 0.0);
            world.insert_resource(delta);
        }

        for thread_id in 0 .. id.max_threads {
            let handle = create_thread(
                inputs, id, thread_id,
                world.clone(), start_cycle.clone(),
                self.stop.clone(), 
                self.next_lock.clone(),
                complete_threads.clone(), tiles.clone(),
                current_schedule.clone(), self.next_schedule.clone(),
                id.max_threads as u32, 
                target_runtime, 
                // we want the last thread to be master to the master thread lifecycle handling starts last
                thread_id == id.max_threads - 1
            );

            threads.push(Task { id: u32::MAX, handle, tick_rate, kill: self.stop.clone() });
        }

        return threads;
    }
}

// Make sure schedule pointer is dropped
impl <I: Copy + 'static, O: Clone + 'static> Drop for ScheduleExecutor<I, O> {
    fn drop(&mut self) {
        let _schedule = unsafe { 
            Box::from_raw(self.next_schedule.load(Ordering::Acquire)) 
        };
    }
}

/// Creates a executor thread.
fn create_thread<I: Copy + Send + 'static, O: Clone + 'static>(
    inputs: I,
    schedule_id: ScheduleID,
    thread_id: u32,
    world: World,
    cycle_starter: Arc<AtomicBool>,
    stop_signal: Arc<AtomicBool>,
    next_schedule_lock: Arc<AtomicBool>,
    complete_threads_counter: Arc<AtomicU32>,
    tiles: Arc<Box<[RelaxedMutex<LinkedList<ScheduleIteratorItem<I, O>>>]>>,
    current_schedule: Arc<AtomicPtr<Schedule<I, O>>>,
    next_schedule: Arc<AtomicPtr<Schedule<I, O>>>,
    max_threads: u32,
    target_runtime: u128,
    is_master: bool
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name(format!("{} ThreadID: {}", schedule_id.id, thread_id))
        .spawn(move || {
            let mut start_cycle_next = !cycle_starter.load(Ordering::Acquire);

            loop {
                let start = Utc::now();

                let (schedule, new_schedule) = if is_master {
                    // spin-lock if schedule is locked
                    loop {
                        if !next_schedule_lock.load(Ordering::Acquire) { break }
                        std::thread::yield_now();
                    }

                    // update current schedule
                    let new_schedule = Box::leak(Box::new(Schedule::new_empty()));
                    let schedule = unsafe { &mut *next_schedule.swap(new_schedule, Ordering::Release) };
                    let old_schedule_ptr = current_schedule.swap(schedule, Ordering::Release);
                    if !old_schedule_ptr.is_null() {
                        let old_schedule = unsafe { &mut *old_schedule_ptr };
                        let _old_schedule = unsafe { Box::from_raw(old_schedule) };
                    }

                    // process schedule
                    let mut idx = 0;
                    if schedule.has_next_startup() {
                        while let Some(tile) = schedule.next_startup() {
                            tiles[(idx % max_threads) as usize].lock_mut().push_back(tile);
                            idx += 1;
                        }

                        while let Some(item) = schedule.next_update() {
                            new_schedule.add_new(item.tile.clone());
                        }
                    } else {
                        while let Some(tile) = schedule.next_update() {
                            tiles[(idx % max_threads) as usize].lock_mut().push_back(tile);
                            idx += 1;
                        }
                    }

                    // swap start cycle to trigger next tick
                    cycle_starter.store(!cycle_starter.load(Ordering::Acquire), Ordering::Release);
                
                    (schedule, new_schedule)
                } else {
                    // spin-lock until ct_local is less than max threads and start is called
                    loop {
                        if cycle_starter.load(Ordering::Acquire) == start_cycle_next { break }
                        std::thread::yield_now();
                        std::thread::sleep(Duration::from_micros(50));
                    }
                    start_cycle_next = !start_cycle_next;

                    let schedule = unsafe { &mut *current_schedule.load(Ordering::Acquire) };
                    let new_schedule = unsafe { &mut *next_schedule.load(Ordering::Acquire) };
                    (schedule, new_schedule)
                };

                // execute schedules until complete
                {
                    let list = &tiles[thread_id as usize];
                    loop {
                        let Some(item) = list.lock_mut().pop_front() else { break };
                        item.tile.execute(&world, &inputs, schedule_id, item.first_run);
                        if !item.dont_save { new_schedule.post_run_add(item.tile.clone(), schedule.total_runtime); }
                    }
                }

                // mark thread complete
                complete_threads_counter.fetch_add(1, Ordering::AcqRel);

                if stop_signal.load(Ordering::Acquire) { break }

                if is_master {
                    // wait until all threads complete
                    loop {
                        if complete_threads_counter.load(Ordering::Acquire) >= max_threads { break }
                        std::thread::yield_now();
                    }
                    complete_threads_counter.store(0, Ordering::Release);

                    let runtime = Utc::now().signed_duration_since(start).to_std().map(|a| a.as_nanos()).unwrap_or(0);

                    let total_runtime = if target_runtime > runtime {
                        let sleep_nanos = target_runtime - runtime;
                        std::thread::sleep(Duration::from_nanos(sleep_nanos as u64));
                        target_runtime
                    } else { runtime };
                    
                    let deltatime = total_runtime as f32 / 1_000_000_000.0;
                    world.get_resource_ref::<DeltaTime>()
                        .expect("DeltaTime was lost!")
                        .set(FlexLocalId::Schedule(schedule_id), deltatime);
                }
            }
        })
        .unwrap()
}
