//! Running a [`SystemGraph`] against a world.

use std::{collections::LinkedList, sync::{Arc, atomic::{AtomicUsize, Ordering}}};

use ahash::AHashMap;
use crossbeam_queue::SegQueue;
use mutual::{RelaxedMutex, SharedData};

use crate::{SystemGraph, SystemKey, WorkQueueEntry, World};

pub mod single;
pub mod multi;

pub use single::*;
pub use multi::*;

/// The queues and counters shared by every thread running a [`SystemGraph`].
/// Clones share the same state.
///
/// Systems are handed it too, so they can [`submit_work`](Self::submit_work)
/// that other threads help run.
#[derive(Default, Clone)]
pub struct SharedExecutionState {
    system_queue: Arc<SegQueue<SystemKey>>,
    work_queue: RelaxedMutex<LinkedList<Arc<dyn WorkQueueEntry>>>,
    /// Systems submitted or pushed as dependents that have not finished running.
    in_flight: Arc<AtomicUsize>,
    /// Dependencies of each system that have not finished this pass.  The
    /// last dependency to finish queues the system and resets its counter.
    pending: RelaxedMutex<AHashMap<SystemKey, AtomicUsize>>
}

impl SharedExecutionState {

    /// Submit an iterator of systems to be executed.
    pub fn submit_systems<I>(&self, iter: I)
        where I: Iterator<Item = SystemKey>
    {
        iter.for_each(|key| {
            self.in_flight.fetch_add(1, Ordering::Release);
            self.system_queue.push(key);
        });
    }

    /// Rebuild the dependency counters from a graph.  Must be called before
    /// running a graph and again whenever the graph changes, but not while
    /// any thread is running `exec_single`.
    pub fn prepare(&self, graph: &SystemGraph) {
        *self.pending.lock_mut() = graph.nodes()
            .map(|node| (node.key(), AtomicUsize::new(node.dependencies().len())))
            .collect();
    }

    /// Returns true once every submitted system, and every dependent they
    /// queued, has finished running.
    pub fn is_complete(&self) -> bool {
        self.in_flight.load(Ordering::Acquire) == 0
    }

    /// Drop every queued system and reset the in flight counter.  The
    /// dependency counters may be partway through a pass, call `prepare`
    /// before running again.
    #[allow(dead_code)]
    pub(crate) fn clear(&self) {
        while self.system_queue.pop().is_some() {}
        self.in_flight.store(0, Ordering::Release);
    }

    /// Submits a work entry that other threads sharing this executor state
    /// may assist in running.
    pub fn submit_work<E: WorkQueueEntry + 'static>(&self, entry: E) {
        if !entry.is_finished() {
            self.work_queue.lock_mut().push_back(Arc::new(entry));
        }
    }

    /// Execute one entry from the work queue.  This will go through the work queue,
    /// prune all finished work entries, then run the first entry it finds.  Nothing
    /// will be run if it cannot find a work entry to run.
    ///
    /// Returns true if some work was executed.
    pub fn exec_from_work_queue(&self) -> bool {
        // lock and loop through queue, removing all finished tasks, then return
        // either the first element with work, then execute.
        let work = {
            let mut queue = self.work_queue.lock_mut();
            loop {
                let Some(entry) = queue.front().cloned()
                    else { break None };
                if entry.is_finished() {
                    queue.pop_front();
                    continue;
                }
                break Some(entry);
            }
        };

        let Some(work) = work else { return false; };
        work.run();
        return true;
    }

    /// Run a single execution of a shared execution state.  This will execute
    /// everything from the work queue then one item from the system queue.
    /// This may be run by any number of threads simultaneously.
    ///
    /// A dependent is queued once its last dependency finishes, which needs
    /// the counters from `prepare`.
    pub fn exec_single(&self, world: &World, graph: &SystemGraph) -> anyhow::Result<bool> {
        // empty work queue
        while self.exec_from_work_queue() {}

        // pick one system to run
        let Some(key) = self.system_queue.pop() else { return Ok(false) };

        // find a system to run
        let Some(node) = graph.node(key) else {
            self.in_flight.fetch_sub(1, Ordering::AcqRel);
            return Ok(false)
        };
        let result = node.run(world, self);

        // queue each dependent this system was the last dependency of,
        // counting them before removing this system so the counter only
        // hits 0 once nothing is left to run
        {
            let pending = self.pending.lock_ref();
            for dependent in node.dependents() {
                let ready = match pending.get(dependent) {
                    Some(count) => count.fetch_sub(1, Ordering::AcqRel) == 1,
                    None => true
                };
                if !ready { continue }
                if let (Some(count), Some(dependent_node)) = (pending.get(dependent), graph.node(*dependent)) {
                    count.store(dependent_node.dependencies().len(), Ordering::Release);
                }
                self.in_flight.fetch_add(1, Ordering::Release);
                self.system_queue.push(*dependent);
            }
        }
        self.in_flight.fetch_sub(1, Ordering::AcqRel);

        return result.map(|_| true);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::{AtomicBool, AtomicU32, Ordering}};

    use derive_more::{Deref, DerefMut};
use mutual::RelaxedMutex;

    use crate::{Component, ParIter, Query, SharedExecutionState, SystemGraph, SystemInstruction, SystemPin, WorkQueueEntry, World};

    #[derive(Deref, DerefMut, Debug, Component)]
    struct AtomicFlag(AtomicBool);

    #[test]
    fn single_system() {
        let mut graph = SystemGraph::default();
        let world = World::default();
        let state = SharedExecutionState::default();

        fn test(world: World) {
            world.insert(0, Box::new([RelaxedMutex::new(Box::new(AtomicFlag(AtomicBool::new(false))))]));
        }

        graph.append_system(test, SystemInstruction::default(), std::iter::empty()).unwrap();
        state.submit_systems(graph.roots(SystemPin::Normal));
        state.exec_single(&world, &graph).unwrap();

        let mut query = Query::<&AtomicFlag>::new(&world);
        let mut vec = vec![];
        while let Some(entry) = query.next().unwrap() {
            vec.push(entry);
        }

        assert!(vec.len() == 1)
    }

    #[test]
    fn single_system_parallel() {
        let mut graph = SystemGraph::default();
        let world = World::default();
        let state = SharedExecutionState::default();

        fn test(_: ()) {
            let counter = Arc::new(AtomicU32::new(0));
            let counter2 = Arc::clone(&counter);
            ParIter::new((0..10).into_iter(), move |_| { counter2.fetch_add(1, Ordering::Release); })
                .complete();
            assert!(counter.load(Ordering::Acquire) == 10);
        }

        graph.append_system(test, SystemInstruction::default(), std::iter::empty()).unwrap();
        state.submit_systems(graph.roots(SystemPin::Normal));
        state.exec_single(&world, &graph).unwrap();
    }

    #[test]
    fn diamond_join_runs_once() {
        static RUNS: AtomicU32 = AtomicU32::new(0);
        fn a() {}
        fn b() {}
        fn c() {}
        fn d() { RUNS.fetch_add(1, Ordering::AcqRel); }

        let mut graph = SystemGraph::default();
        graph.append_system(a, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(b, crate::after(a), std::iter::empty()).unwrap();
        graph.append_system(c, crate::after(a), std::iter::empty()).unwrap();
        graph.append_system(d, crate::and(crate::after(b), crate::after(c)), std::iter::empty()).unwrap();

        let world = World::default();
        let state = SharedExecutionState::default();
        state.prepare(&graph);
        for pass in 1..=2 {
            state.submit_systems(graph.roots(SystemPin::Normal));
            let mut runs = 0;
            while !state.is_complete() {
                assert!(state.exec_single(&world, &graph).unwrap());
                runs += 1;
            }
            assert_eq!(runs, 4);
            assert_eq!(RUNS.load(Ordering::Acquire), pass);
        }
    }

    #[test]
    fn chain_runs_through_dependents() {
        fn a() {}
        fn b() {}
        fn c() {}

        let mut graph = SystemGraph::default();
        graph.append_system(a, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(b, crate::after(a), std::iter::empty()).unwrap();
        graph.append_system(c, crate::after(b), std::iter::empty()).unwrap();

        let world = World::default();
        let state = SharedExecutionState::default();
        state.prepare(&graph);
        state.submit_systems(graph.roots(SystemPin::Normal));
        let mut runs = 0;
        while !state.is_complete() {
            assert!(state.exec_single(&world, &graph).unwrap());
            runs += 1;
        }
        assert_eq!(runs, 3);
    }
}
