use std::{collections::LinkedList, sync::{Arc, Mutex}};

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
    static ref GLOBAL_THREAD_POOL: threadpool::ThreadPool = threadpool::ThreadPool::new(
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
    );
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static RT: tokio::runtime::Runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    static LOCAL: tokio::task::LocalSet = tokio::task::LocalSet::new();
}

/// Drives a future to completion on this thread's tokio runtime + `LocalSet`, so it
/// may use tokio IO/timers and `spawn_local` without needing to be `Send`.
#[cfg(not(target_arch = "wasm32"))]
fn block_on_local<F: Future>(future: F) -> F::Output {
    RT.with(|rt| LOCAL.with(|local| local.block_on(rt, future)))
}

/// The unique ID of a repeating task registered via `Scheduler::repeating_task`.
pub type TaskID = u32;

lazy_static! {
    static ref REPEATING_TASKS: SharedMap<TaskID, RelaxedMutex<Task>> = SharedMap::new();
    static ref NEXT_ID: Arc<Mutex<TaskID>> = Arc::new(Mutex::new(0));
    static ref SCHEDULES: SharedMap<ScheduleID, RelaxedMutex<ScheduleExecutor<(), ()>>> = SharedMap::new();
    static ref SCHEDULE_TASKS: SharedMap<ScheduleID, RelaxedMutex<Vec<Task>>> = SharedMap::new();
}

/// Identifies a schedule: its name, how often it should tick (in ticks per second), and
/// how many threads its executor may use to run systems in parallel.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct ScheduleID { pub id: &'static str, pub tick_rate: u32, pub max_threads: u32 }

/// Entry point for registering schedules and repeating tasks to be run against a `World`.
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
    /// Ticks every schedule's tasks and every repeating task once. Only intended to be
    /// driven by the caller's own loop (e.g. a game loop) since single-threaded
    /// executors have no background thread of their own.
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

    /// Spawns the given future to run to completion in the background, off the calling
    /// thread, on the wasm32 target's local task queue.
    #[cfg(target_arch = "wasm32")]
    pub fn run_async<F>(future: F)
        where F: Future<Output = ()> + 'static
    {
        use wasm_bindgen_futures::spawn_local;
        spawn_local(future);
    }

    /// Spawns the given future to run to completion in the background, off the calling
    /// thread, on the global native thread pool.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn run_async<F>(future: F)
        where F: Future<Output = ()> + Send + 'static
    {
        GLOBAL_THREAD_POOL.execute(|| {
            block_on_local(future);
        });
    }

    /// Spawns the given future to run to completion in the background, off the calling
    /// thread, on the global native thread pool.  This is an alternative from run_async
    /// to allow the future to not implement Send.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn run_async_local<M, F>(make_future: M)
    where
        M: FnOnce() -> F + Send + 'static,
        F: Future<Output = ()> + 'static, // no Send needed
    {
        GLOBAL_THREAD_POOL.execute(|| {
            block_on_local(make_future());
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
    fn name() -> &'static str { "DeltaTime" }
}
impl Resource for DeltaTime {
    fn get_id(&self) -> crate::resources::ResourceID { Self::id() }
    fn get_name(&self) -> &'static str { "DeltaTime" }
}

/// Execute a schedule synchronously now.  The next schedule
/// will be returned.  This next schedule is meant to be run
/// during the next tick.  The tick is update interval determined
/// by the user of this function.
pub fn execute_schedule_sync<I, O>(
    prev_render_schedule: &Schedule<I, O>, 
    next_render_schedule: &Schedule<I, O>,
    schedule_id: ScheduleID,
    world: &World,
    inputs: &I
) {
    // setup tiles list
    let mut tiles = LinkedList::new();
    if prev_render_schedule.has_next_startup() {
        while let Some(tile) = prev_render_schedule.next_startup() {
            tiles.push_back(tile);
        }

        while let Some(item) = prev_render_schedule.next_update() {
            next_render_schedule.post_run_add(item.tile.clone(), 0);
        }
    } else {
        while let Some(tile) = prev_render_schedule.next_update() {
            tiles.push_back(tile);
        }
    }

    // execute previous tiles
    tiles.into_iter().for_each(|tile| {
        tile.tile.execute(world, inputs, schedule_id, tile.first_run);
        if !tile.dont_save { next_render_schedule.post_run_add(tile.tile.clone(), *prev_render_schedule.total_runtime.get_ref()); }
    });
}
