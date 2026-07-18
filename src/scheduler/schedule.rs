use std::sync::{Arc, atomic::{AtomicU64, Ordering}};

use anarchy_macros::error;
use chrono::Utc;
use mutual::{CowData, Ref, SharedData, SharedList};

use crate::{self as anarchy,  System, World, scheduler::ScheduleID};


/// An ordered collection of `ScheduleTile`s to be run by a `ScheduleExecutor`.  `startup`
/// tiles run once, before anything else; `new` tiles are freshly added tiles that should
/// run once immediately then move to `update`; `update` tiles run every tick, ordered by
/// priority (highest first).
#[derive(Default)]
pub struct Schedule<I: 'static, O: 'static> {
    pub startup: SharedList<ScheduleTile<I, O>>,
    pub new: SharedList<ScheduleTile<I, O>>,
    pub update: SharedList<ScheduleTile<I, O>>,
    pub total_runtime: CowData<u64>
}

/// A single tile pulled off a `Schedule` for execution, along with how it should be
/// handled once it has run: `dont_save` marks a tile that should not be carried over
/// into the next schedule (used for startup tiles, which only ever run once), and
/// `first_run` marks whether this is the tile's first execution.
pub struct ScheduleIteratorItem<I: 'static, O: 'static> {
    pub tile: Ref<ScheduleTile<I, O>>,
    pub dont_save: bool,
    pub first_run: bool
}

impl <I: 'static, O: 'static> Schedule<I, O> {
    /// Creates a new, empty `Schedule`.
    pub fn new_empty() -> Self {
        Self {
            startup: SharedList::new(), 
            new: SharedList::new(),
            update: SharedList::new_ordered(|a, b| b.priority.cmp(&a.priority)),
            total_runtime: CowData::new(0)
        }
    }

    /// Drains every tile out of `other` and into this schedule, summing runtimes.
    pub fn merge(&self, other: Self) {
        self.startup.extend(other.startup.drain().map(|a| a.clone()));
        self.new.extend(other.new.drain().map(|a| a.clone()));
        self.update.extend(other.update.drain().map(|a| a.clone()));
        *self.total_runtime.lock_mut() += *other.total_runtime.lock_ref();
    }

    /// Creates a `Schedule` from a set of startup tiles and a set of tiles to run on
    /// every subsequent tick.
    pub fn from_iter(
        startup_systems: impl Iterator<Item = ScheduleTile<I, O>>,
        normal_systems: impl Iterator<Item = ScheduleTile<I, O>>
    ) -> Self {
        let startup = SharedList::new();
        startup.extend(startup_systems);
        let normal = SharedList::new();
        normal.extend(normal_systems);

        let mut total_runtime = 0;
        for sys in startup.iter() {
            total_runtime += sys.last_runtime.load(Ordering::SeqCst);
        }
        for sys in normal.iter() {
            total_runtime += sys.last_runtime.load(Ordering::SeqCst);
        }

        Self {
            startup, new: normal,
            update: SharedList::new_ordered(|a, b| b.priority.cmp(&a.priority)),
            total_runtime: CowData::new(total_runtime)
        }
    }

    /// Adds a tile to run once at startup.
    pub fn add_startup(&self, tile: ScheduleTile<I, O>) {
        *self.total_runtime.lock_mut() += tile.last_runtime.load(Ordering::SeqCst);
        self.startup.push(tile);
    }

    /// Adds a newly-created tile, to run once immediately then move to `update`.
    pub fn add_new(&self, tile: ScheduleTile<I, O>) {
        *self.total_runtime.lock_mut() += tile.last_runtime.load(Ordering::SeqCst);
        self.new.push(tile);
    }

    /// Re-adds a tile to `update` after it has already run at least once this tick.
    pub fn post_run_add(&self, tile: ScheduleTile<I, O>, _last_total_runtime: u64) {
        *self.total_runtime.lock_mut() += tile.last_runtime.load(Ordering::SeqCst);
        self.update.push(tile);
    }

    /// Returns true if there is a `new` or `update` tile left to run this tick.
    pub fn has_next_update(&self) -> bool {
        self.new.len() > 0 || self.update.len() > 0
    }

    /// Pops the next tile to run this tick, preferring newly-added tiles over the
    /// priority-ordered `update` list.
    pub fn next_update(&self) -> Option<ScheduleIteratorItem<I, O>> {
        if let Some(group) = self.new.pop() {
            return Some(ScheduleIteratorItem { tile: group, dont_save: false, first_run: true })
        } else if let Some(group) = self.update.pop() {
            return Some(ScheduleIteratorItem { tile: group, dont_save: false, first_run: false })
        }

        None
    }

    /// Returns true if there is a startup tile left to run.
    pub fn has_next_startup(&self) -> bool {
        self.startup.len() > 0
    }

    /// Pops the next startup tile to run.
    pub fn next_startup(&self) -> Option<ScheduleIteratorItem<I, O>> {
        if let Some(startup) = self.startup.pop() { 
            Some(ScheduleIteratorItem { tile: startup, dont_save: true, first_run: true })
        } else { None }
    }
}

/// A group of systems that run together, in order, as a single scheduled unit, sharing
/// one priority and one set of runtime statistics (last, min, max, and a running average
/// runtime in microseconds).
pub struct ScheduleTile<I, O> {
    functions: Arc<Box<[Box<dyn System<I, Result<O, Box<dyn std::error::Error>>>>]>>,
    priority: i32,
    last_runtime: Arc<AtomicU64>,
    min_runtime: Arc<AtomicU64>,
    max_runtime: Arc<AtomicU64>,
    est_immeidate_average_runtime: Arc<AtomicU64>
}

impl <I, O> Clone for ScheduleTile<I, O> {
    fn clone(&self) -> Self {
        Self {
            functions: self.functions.clone(),
            priority: self.priority,
            last_runtime: self.last_runtime.clone(),
            min_runtime: self.min_runtime.clone(),
            max_runtime: self.max_runtime.clone(),
            est_immeidate_average_runtime: self.est_immeidate_average_runtime.clone()
        }
    }
}

impl <I, O> ScheduleTile<I, O> {
    /// Creates a new tile from an ordered set of systems, taking its priority from the
    /// first system in the set.
    pub fn new(functions: Vec<Box<dyn System<I, Result<O, Box<dyn std::error::Error>>>>>) -> Self {
        Self {
            priority: functions.first().map(|a| a.priority()).unwrap_or(0),
            functions: Arc::new(functions.into_boxed_slice()),
            last_runtime: Arc::new(AtomicU64::new(0)),
            min_runtime: Arc::new(AtomicU64::new(0)),
            max_runtime: Arc::new(AtomicU64::new(0)),
            est_immeidate_average_runtime: Arc::new(AtomicU64::new(0))
        }
    }

    /// Runs every system in this tile, in order, against `world`, then updates this
    /// tile's runtime statistics. A system returning an error is logged and skipped;
    /// it does not stop the rest of the tile from running.
    pub fn execute(&self, world: &World, inputs: &I, schedule_id: ScheduleID, is_first_run: bool) {
        // run all functions in order and track the total runtime
        let start = Utc::now();
        for func in self.functions.iter() {
            let result = func.execute(schedule_id, world, &inputs);
            if result.is_err() {
                error!("System {} error: {:?}", func.name(), result.err().unwrap());
            }
        }
        let runtime = Utc::now().signed_duration_since(start).to_std().map(|a| a.as_micros() as u64).unwrap_or(0);

        // update estimate average runtime
        if is_first_run {
            self.est_immeidate_average_runtime.store(runtime, Ordering::SeqCst);
        } else {
            let avg_runtime = self.est_immeidate_average_runtime.load(Ordering::SeqCst);

            if avg_runtime > runtime {
                let diff = (avg_runtime - runtime) / 2;
                self.est_immeidate_average_runtime.fetch_sub(diff, Ordering::SeqCst);
            } else {
                let diff = (runtime - avg_runtime) / 2;
                self.est_immeidate_average_runtime.fetch_add(diff, Ordering::SeqCst);
            }
        }

        // update other runtime trackers
        self.last_runtime.store(runtime, Ordering::SeqCst);
        if self.min_runtime.load(Ordering::SeqCst) > runtime { self.min_runtime.store(runtime, Ordering::SeqCst); }
        if self.max_runtime.load(Ordering::SeqCst) < runtime { self.max_runtime.store(runtime, Ordering::SeqCst); }
    }
}
