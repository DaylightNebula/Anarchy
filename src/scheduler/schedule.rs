use std::sync::{Arc, atomic::{AtomicU64, Ordering}};

use anarchy_macros::error;
use chrono::Utc;
use mutual::{SharedList, Ref};

use crate::{self as anarchy,  System, World, scheduler::ScheduleID};


#[derive(Default)]
pub struct Schedule<I: 'static, O: 'static> {
    pub startup: SharedList<ScheduleTile<I, O>>,
    pub new: SharedList<ScheduleTile<I, O>>,
    pub high_inter: SharedList<ScheduleTile<I, O>>,
    pub high_drag: SharedList<ScheduleTile<I, O>>,
    pub low_inter: SharedList<ScheduleTile<I, O>>,
    pub low_drag: SharedList<ScheduleTile<I, O>>,
    pub total_runtime: u64
}

pub struct ScheduleIteratorItem<I: 'static, O: 'static> { 
    pub tile: Ref<ScheduleTile<I, O>>, 
    pub dont_save: bool, 
    pub first_run: bool 
}

impl <I: 'static, O: 'static> Schedule<I, O> {
    pub fn new_empty() -> Self {
        Self {
            startup: SharedList::new(), 
            new: SharedList::new(),
            high_inter: SharedList::new(),
            high_drag: SharedList::new(),
            low_inter: SharedList::new(),
            low_drag: SharedList::new(),
            total_runtime: 0
        }
    }

    pub fn merge(&mut self, other: Self) {
        self.startup.extend(other.startup.drain().map(|a| a.clone()));
        self.new.extend(other.new.drain().map(|a| a.clone()));
        self.high_inter.extend(other.high_inter.drain().map(|a| a.clone()));
        self.high_drag.extend(other.high_drag.drain().map(|a| a.clone()));
        self.low_inter.extend(other.low_inter.drain().map(|a| a.clone()));
        self.low_drag.extend(other.low_drag.drain().map(|a| a.clone()));
        self.total_runtime += other.total_runtime;
    }

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
            high_inter: SharedList::new(),
            high_drag: SharedList::new(),
            low_inter: SharedList::new(),
            low_drag: SharedList::new(),
            total_runtime
        }
    }

    pub fn add_startup(&mut self, tile: ScheduleTile<I, O>) {
        self.total_runtime += tile.last_runtime.load(Ordering::SeqCst);
        self.startup.push(tile);
    }

    pub fn add_new(&mut self, tile: ScheduleTile<I, O>) {
        self.total_runtime += tile.last_runtime.load(Ordering::SeqCst);
        self.new.push(tile);
    }

    pub fn post_run_add(&mut self, tile: ScheduleTile<I, O>, last_total_runtime: u64) {
        self.total_runtime += tile.last_runtime.load(Ordering::SeqCst);

        let is_high = (tile.last_runtime.load(Ordering::SeqCst) as f64 / last_total_runtime as f64) > 0.02;
        let is_inter = (tile.est_immeidate_average_runtime.load(Ordering::SeqCst) as f64 / tile.max_runtime.load(Ordering::SeqCst) as f64) < 0.65;

        if is_high {
            if is_inter { self.high_inter.push(tile); }
            else { self.high_drag.push(tile); }
        } else {
            if is_inter { self.low_inter.push(tile); }
            else { self.low_drag.push(tile); }
        }
    }

    pub fn has_next_update(&self) -> bool {
        self.new.len() > 0 || self.high_inter.len() > 0 || self.high_drag.len() > 0 || self.low_inter.len() > 0 || self.low_drag.len() > 0
    }

    pub fn next_update(&self) -> Option<ScheduleIteratorItem<I, O>> {
        if let Some(group) = self.new.pop() {
            return Some(ScheduleIteratorItem { tile: group, dont_save: false, first_run: true })
        } else if let Some(group) = self.high_inter.pop() {
            return Some(ScheduleIteratorItem { tile: group, dont_save: false, first_run: false })
        } else if let Some(group) = self.high_drag.pop() {
            return Some(ScheduleIteratorItem { tile: group, dont_save: false, first_run: false })
        } else if let Some(group) = self.low_inter.pop() {
            return Some(ScheduleIteratorItem { tile: group, dont_save: false, first_run: false })
        } else if let Some(group) = self.low_drag.pop() {
            return Some(ScheduleIteratorItem { tile: group, dont_save: false, first_run: false })
        }

        None
    }

    pub fn has_next_startup(&self) -> bool {
        self.startup.len() > 0
    }

    pub fn next_startup(&self) -> Option<ScheduleIteratorItem<I, O>> {
        if let Some(startup) = self.startup.pop() { 
            Some(ScheduleIteratorItem { tile: startup, dont_save: true, first_run: true })
        } else { None }
    }
}

pub struct ScheduleTile<I, O> {
    functions: Arc<Box<[Box<dyn System<I, Result<O, Box<dyn std::error::Error>>>>]>>,
    last_runtime: Arc<AtomicU64>,
    min_runtime: Arc<AtomicU64>,
    max_runtime: Arc<AtomicU64>,
    est_immeidate_average_runtime: Arc<AtomicU64>
}

impl <I, O> Clone for ScheduleTile<I, O> {
    fn clone(&self) -> Self {
        Self {
            functions: self.functions.clone(),
            last_runtime: self.last_runtime.clone(),
            min_runtime: self.min_runtime.clone(),
            max_runtime: self.max_runtime.clone(),
            est_immeidate_average_runtime: self.est_immeidate_average_runtime.clone()
        }
    }
}

impl <I, O> ScheduleTile<I, O> {
    pub fn new(functions: Vec<Box<dyn System<I, Result<O, Box<dyn std::error::Error>>>>>) -> Self {
        Self {
            functions: Arc::new(functions.into_boxed_slice()),
            last_runtime: Arc::new(AtomicU64::new(0)),
            min_runtime: Arc::new(AtomicU64::new(0)),
            max_runtime: Arc::new(AtomicU64::new(0)),
            est_immeidate_average_runtime: Arc::new(AtomicU64::new(0))
        }
    }

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
