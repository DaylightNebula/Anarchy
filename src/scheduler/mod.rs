use std::{sync::{Arc, Mutex}};

use derive_more::Deref;
use lazy_static::lazy_static;
use mutual::SharedData;

use crate::{AsAny, FlexLocal, RelaxedMutex, Resource, ResourceMeta, SharedMap, World};

pub mod schedule;
pub use schedule::*;

#[cfg(any(
    all(not(target_arch = "wasm32"), not(feature = "single-threaded-executors")),
    feature = "multi-threaded-executors"
))]
pub mod executor_multithreaded;
#[cfg(any(
    all(not(target_arch = "wasm32"), not(feature = "single-threaded-executors")),
    feature = "multi-threaded-executors"
))]
pub use executor_multithreaded::*;

#[cfg(all(
    any(target_arch = "wasm32", feature = "single-threaded-executors"),
    not(feature = "multi-threaded-executors")
))]
pub mod executor_singlethreaded;
#[cfg(all(
    any(target_arch = "wasm32", feature = "single-threaded-executors"),
    not(feature = "multi-threaded-executors")
))]
pub use executor_singlethreaded::*;

#[cfg(not(target_arch = "wasm32"))]
lazy_static! {
    static ref GLOBAL_THREAD_POOL: threadpool::ThreadPool = threadpool::ThreadPool::new(4);
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static RT: tokio::runtime::Runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    static LOCAL: tokio::task::LocalSet = tokio::task::LocalSet::new();
}

pub type TaskID = u32;

lazy_static! {
    static ref REPEATING_TASKS: SharedMap<TaskID, RelaxedMutex<Task>> = SharedMap::new();
    static ref NEXT_ID: Arc<Mutex<TaskID>> = Arc::new(Mutex::new(0));
    static ref SCHEDULES: SharedMap<ScheduleID, RelaxedMutex<ScheduleExecutor<(), ()>>> = SharedMap::new();
    static ref SCHEDULE_TASKS: SharedMap<ScheduleID, RelaxedMutex<Vec<Task>>> = SharedMap::new();
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct ScheduleID { pub id: &'static str, pub tick_rate: u32, pub max_threads: u32 }

pub struct Scheduler;

impl Scheduler {
    /// Gets the next unused task ID.
    pub fn next_task_id() -> TaskID {
        let mut id = NEXT_ID.lock().unwrap();
        *id += 1;
        *id - 1
    }

    /// Schedule a `Schedule` instance to execute
    pub fn schedule(id: ScheduleID, schedule: Schedule<(), ()>, world: World) {
        if let Some(existing) = SCHEDULES.get(&id) {
            let executor = existing.lock_mut();
            executor.lock_next_schedule();
            executor.next_schedule().merge(schedule);
            executor.unlock_next_schedule();
        } else {
            let executor = ScheduleExecutor::new(id.clone(), schedule);
            let threads = executor.start(world, ());
            SCHEDULES.insert(id.clone(), RelaxedMutex::new(executor));
            SCHEDULE_TASKS.insert(id, RelaxedMutex::new(threads));
        }
    }

    /// Creates a new repeating task that will be attempt to run the given task at the given rate.
    /// Before each function call, the thread will check if it should shutdown, which it will do if asked.
    /// Then after the task is called, the thread will calculate the time needed to wait to maintain the given
    /// tick rate (ticks per second), then wait for that long.  If the task took longer than what the tick rate
    /// says each length should take, no wait is preformed and the next loop begins.
    /// The task will be called with there own task ID so they can identify themselves, as well as the same
    /// task ID will be returned from this function.
    pub fn repeating_task<F>(
        task: F,
        tick_rate: u64
    ) -> TaskID 
        where F: FnMut(TaskID) -> () + Send + Sync + 'static
    {
        let task = Task::repeating(task, tick_rate as u32);
        let task_id = task.id();

        // save child thread
        REPEATING_TASKS
            .insert(task_id, RelaxedMutex::new(task));

        return task_id;
    }

    /// Cancel a repeating task of the given task ID.  Returns true if the task was found
    /// and the shutdown signal was sent successfully.
    pub fn cancel_repeating_task(id: &TaskID) -> bool {
        REPEATING_TASKS
            .remove(id)
            .map(|task| {
                task.lock_ref().kill();
                true
            }).unwrap_or(false)
    }

    /// Only available with the `single-threaded-executors` feature, allows all schedules
    /// and tasks to be executed in a single threaded form.
    #[cfg(all(
        any(target_arch = "wasm32", feature = "single-threaded-executors"),
        not(feature = "multi-threaded-executors")
    ))]
    pub fn tick_tasks() {
        use chrono::Utc;

        let tick_start_time = Utc::now();

        SCHEDULE_TASKS.iter().for_each(|entry| {
            entry.2.lock_mut().iter_mut().for_each(|task| {
                task.tick(tick_start_time);
            });
        });

        REPEATING_TASKS.iter().for_each(|entry| {
            entry.2.lock_mut().tick(tick_start_time);
        });
    }

    #[cfg(target_arch = "wasm32")]
    pub fn run_async<F>(future: F) 
        where F: Future<Output = ()> + Send + 'static
    {
        use wasm_bindgen_futures::spawn_local;
        spawn_local(future);
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn run_async<F>(future: F) 
        where F: Future<Output = ()> + Send + 'static
    {
        GLOBAL_THREAD_POOL.execute(|| {
            pollster::block_on(future);
        });
    }
}

/// Tracks the delta time of each schedule with an active executor.
#[derive(Deref, Default)]
pub struct DeltaTime(FlexLocal<f32>);
impl AsAny for DeltaTime {
    fn as_any(&self) -> &dyn std::any::Any { self }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}
impl ResourceMeta for DeltaTime {
    fn id() -> crate::resources::ResourceID { 462345 }
}
impl Resource for DeltaTime {
    fn get_id(&self) -> crate::resources::ResourceID { Self::id() }
}
