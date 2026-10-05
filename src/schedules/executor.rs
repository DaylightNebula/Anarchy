use std::{collections::LinkedList, sync::Arc};

use crossbeam_queue::SegQueue;
use mutual::{RelaxedMutex, SharedData};

use crate::{SystemGraph, SystemKey, WorkQueueEntry, World};

#[derive(Default, Clone)]
pub struct SharedExecutionState {
    system_queue: Arc<SegQueue<SystemKey>>,
    work_queue: RelaxedMutex<LinkedList<Arc<dyn WorkQueueEntry>>>
}

impl SharedExecutionState {

    /// Submit an iterator of systems to be executed.
    pub fn submit_systems<I>(&self, iter: I)
        where I: Iterator<Item = SystemKey>
    {
        iter.for_each(|key| self.system_queue.push(key));
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
    /// This may be run by any number of threads simulatenously.
    pub fn exec_single(&self, world: &World, graph: &SystemGraph) -> anyhow::Result<bool> {
        // empty work queue
        while self.exec_from_work_queue() {}

        // pick one system to run
        let Some(key) = self.system_queue.pop() else { return Ok(false) };

        // find a system to run
        let Some(node) = graph.node(key) else { return Ok(false) };
        let result = node.run(world, self);

        // add dependent systems back to running state
        let dependents = node.dependents();
        dependents.iter().for_each(|key| self.system_queue.push(*key));

        return result.map(|_| true);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::{AtomicBool, AtomicU32, Ordering}};

    use derive_more::{Deref, DerefMut};
use mutual::{AsAny, RelaxedMutex};

    use crate::{Component, ComponentMeta, ParIter, Query, SharedExecutionState, SystemGraph, SystemInstruction, SystemPin, WorkQueueEntry, World};

    #[derive(Deref, DerefMut, Debug)]
    struct AtomicFlag(AtomicBool);
    impl AsAny for AtomicFlag {
        fn as_any(&self) -> &dyn std::any::Any { self }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }
    impl ComponentMeta for AtomicFlag {}
    impl Component for AtomicFlag {}

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
}
