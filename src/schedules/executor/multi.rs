use std::sync::{Barrier, Mutex, atomic::{AtomicBool, Ordering}};

use crate::{SharedExecutionState, SystemGraph, SystemPin, World};

/// Runs each pin across a fixed set of threads, one pin at a time, looping
/// from the start while a running flag stays true.
#[derive(Default)]
pub struct MultiThreadedExecutor {
    state: SharedExecutionState
}

impl MultiThreadedExecutor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run the graph on `threads` threads (the calling thread included),
    /// which are spawned once and joined when this returns.  Each loop runs
    /// every pin in order, no systems from different pins run at the same
    /// time, and another loop only starts if `running` is true after the end
    /// pin.  If a system fails, the current loop finishes and the first error
    /// is returned.
    pub fn run(
        &self,
        world: &World,
        graph: &SystemGraph,
        threads: usize,
        running: &AtomicBool
    ) -> anyhow::Result<()> {
        let threads = threads.max(1);
        let barrier = Barrier::new(threads);
        let stop = AtomicBool::new(false);
        let error = Mutex::new(None);
        self.state.prepare(graph);

        let worker = |leader: bool| loop {
            for pin in SystemPin::ALL {
                if leader { self.state.submit_systems(graph.roots(pin)); }
                // roots must be queued before anyone checks for completion
                barrier.wait();
                while !self.state.is_complete() {
                    match self.state.exec_single(world, graph) {
                        Ok(true) => {}
                        // another thread is mid system and may push dependents
                        Ok(false) => std::hint::spin_loop(),
                        Err(err) => {
                            error.lock().unwrap().get_or_insert(err);
                            stop.store(true, Ordering::Release);
                        }
                    }
                }
                // no thread starts the next pin until this one is done everywhere
                barrier.wait();
            }

            if leader && !running.load(Ordering::Acquire) {
                stop.store(true, Ordering::Release);
            }
            // everyone sees the leaders decision
            barrier.wait();
            if stop.load(Ordering::Acquire) { break; }
        };

        std::thread::scope(|scope| {
            for _ in 1..threads {
                scope.spawn(|| worker(false));
            }
            worker(true);
        });

        match error.into_inner().unwrap() {
            Some(err) => Err(err),
            None => Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering}};

    use crate::*;

    fn run_once(graph: &SystemGraph, threads: usize) -> anyhow::Result<()> {
        MultiThreadedExecutor::new().run(&World::default(), graph, threads, &AtomicBool::new(false))
    }

    #[test]
    fn pins_run_in_order() {
        static ORDER: Mutex<Vec<SystemPin>> = Mutex::new(Vec::new());
        fn start() { ORDER.lock().unwrap().push(SystemPin::Start); }
        fn normal() { ORDER.lock().unwrap().push(SystemPin::Normal); }
        fn end() { ORDER.lock().unwrap().push(SystemPin::End); }

        let mut graph = SystemGraph::new();
        graph.append_system(end, pin(SystemPin::End), std::iter::empty()).unwrap();
        graph.append_system(normal, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(start, pin(SystemPin::Start), std::iter::empty()).unwrap();

        run_once(&graph, 4).unwrap();
        assert_eq!(*ORDER.lock().unwrap(), vec![SystemPin::Start, SystemPin::Normal, SystemPin::End]);
    }

    #[test]
    fn chain_runs_in_order() {
        static ORDER: Mutex<Vec<u32>> = Mutex::new(Vec::new());
        fn a() { ORDER.lock().unwrap().push(0); }
        fn b() { ORDER.lock().unwrap().push(1); }
        fn c() { ORDER.lock().unwrap().push(2); }

        let mut graph = SystemGraph::new();
        graph.append_system(c, after(b), std::iter::empty()).unwrap();
        graph.append_system(b, after(a), std::iter::empty()).unwrap();
        graph.append_system(a, SystemInstruction::default(), std::iter::empty()).unwrap();

        run_once(&graph, 4).unwrap();
        assert_eq!(*ORDER.lock().unwrap(), vec![0, 1, 2]);
    }

    #[test]
    fn pins_do_not_overlap() {
        static FINISHED: AtomicUsize = AtomicUsize::new(0);
        static END_SAW: AtomicUsize = AtomicUsize::new(0);
        fn slow() {
            std::thread::sleep(std::time::Duration::from_millis(10));
            FINISHED.fetch_add(1, Ordering::AcqRel);
        }
        fn n0() { slow() } fn n1() { slow() } fn n2() { slow() } fn n3() { slow() }
        fn n4() { slow() } fn n5() { slow() } fn n6() { slow() } fn n7() { slow() }
        fn end() { END_SAW.store(FINISHED.load(Ordering::Acquire), Ordering::Release); }

        let mut graph = SystemGraph::new();
        graph.append_system(n0, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(n1, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(n2, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(n3, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(n4, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(n5, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(n6, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(n7, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(end, pin(SystemPin::End), std::iter::empty()).unwrap();

        run_once(&graph, 4).unwrap();
        assert_eq!(END_SAW.load(Ordering::Acquire), 8);
    }

    #[test]
    fn loops_while_running() {
        static COUNT: AtomicU32 = AtomicU32::new(0);
        static RUNNING: AtomicBool = AtomicBool::new(true);
        fn count() { COUNT.fetch_add(1, Ordering::AcqRel); }
        fn end() {
            if COUNT.load(Ordering::Acquire) >= 3 { RUNNING.store(false, Ordering::Release); }
        }

        let mut graph = SystemGraph::new();
        graph.append_system(count, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(end, pin(SystemPin::End), std::iter::empty()).unwrap();

        MultiThreadedExecutor::new().run(&World::default(), &graph, 4, &RUNNING).unwrap();
        assert_eq!(COUNT.load(Ordering::Acquire), 3);
    }

    #[test]
    fn diamond_runs_join_once_after_both() {
        static ORDER: Mutex<Vec<char>> = Mutex::new(Vec::new());
        static PASSES: AtomicU32 = AtomicU32::new(0);
        static RUNNING: AtomicBool = AtomicBool::new(true);
        fn a() { ORDER.lock().unwrap().push('a'); }
        fn b() { ORDER.lock().unwrap().push('b'); }
        fn c() { ORDER.lock().unwrap().push('c'); }
        fn d() { ORDER.lock().unwrap().push('d'); }
        fn end() {
            if PASSES.fetch_add(1, Ordering::AcqRel) + 1 >= 3 { RUNNING.store(false, Ordering::Release); }
        }

        let mut graph = SystemGraph::new();
        graph.append_system(a, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(b, after(a), std::iter::empty()).unwrap();
        graph.append_system(c, after(a), std::iter::empty()).unwrap();
        graph.append_system(d, and(after(b), after(c)), std::iter::empty()).unwrap();
        graph.append_system(end, pin(SystemPin::End), std::iter::empty()).unwrap();

        MultiThreadedExecutor::new().run(&World::default(), &graph, 4, &RUNNING).unwrap();

        let order = ORDER.lock().unwrap();
        assert_eq!(order.len(), 12);
        for pass in order.chunks(4) {
            assert_eq!(pass[0], 'a');
            assert_eq!(pass[3], 'd');
        }
    }

    #[test]
    fn threads_assist_work_queue() {
        static TOTAL: AtomicU32 = AtomicU32::new(0);
        struct Par;
        impl System<(), ()> for Par {
            fn run(&self, _: &World, exec_state: &SharedExecutionState) -> anyhow::Result<()> {
                let counter = Arc::new(AtomicU32::new(0));
                let counter2 = Arc::clone(&counter);
                exec_state.submit_work(ParIter::new(0..100, move |_| {
                    std::thread::sleep(std::time::Duration::from_micros(100));
                    counter2.fetch_add(1, Ordering::AcqRel);
                }));
                while exec_state.exec_from_work_queue() {}
                // other threads may still be finishing items they claimed
                while counter.load(Ordering::Acquire) < 100 { std::hint::spin_loop(); }
                TOTAL.store(counter.load(Ordering::Acquire), Ordering::Release);
                Ok(())
            }
        }

        let mut graph = SystemGraph::new();
        graph.append_raw(std::any::TypeId::of::<Par>(), Box::new(Par), SystemInstruction::default(), std::iter::empty()).unwrap();

        run_once(&graph, 4).unwrap();
        assert_eq!(TOTAL.load(Ordering::Acquire), 100);
    }

    #[test]
    fn error_stops_loop() {
        struct Failing;
        impl System<(), ()> for Failing {
            fn run(&self, _: &World, _: &SharedExecutionState) -> anyhow::Result<()> {
                anyhow::bail!("failed")
            }
        }

        let mut graph = SystemGraph::new();
        graph.append_raw(std::any::TypeId::of::<Failing>(), Box::new(Failing), SystemInstruction::default(), std::iter::empty()).unwrap();

        let running = AtomicBool::new(true);
        assert!(MultiThreadedExecutor::new().run(&World::default(), &graph, 4, &running).is_err());
    }
}
