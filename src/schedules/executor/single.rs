//! [`SingleThreadedExecutor`], which runs a graph on the calling thread.

use crate::{SharedExecutionState, SystemGraph, SystemPin, World};

/// Runs every system of each pin in order on the calling thread.
#[derive(Default)]
pub struct SingleThreadedExecutor {
    state: SharedExecutionState
}

impl SingleThreadedExecutor {
    /// Create an executor with its own execution state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Run every system in the graph once, pin by pin.  Returns the first
    /// error a system produces, skipping every system not yet run.
    pub fn run(&self, world: &World, graph: &SystemGraph) -> anyhow::Result<()> {
        self.state.prepare(graph);
        for pin in SystemPin::ALL {
            self.state.submit_systems(graph.roots(pin));
            while !self.state.is_complete() {
                if let Err(err) = self.state.exec_single(world, graph) {
                    self.state.clear();
                    return Err(err);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use crate::*;

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

        SingleThreadedExecutor::new().run(&World::default(), &graph).unwrap();
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

        SingleThreadedExecutor::new().run(&World::default(), &graph).unwrap();
        assert_eq!(*ORDER.lock().unwrap(), vec![0, 1, 2]);
    }

    #[test]
    fn diamond_runs_join_once_after_both() {
        static ORDER: Mutex<Vec<char>> = Mutex::new(Vec::new());
        fn a() { ORDER.lock().unwrap().push('a'); }
        fn b() { ORDER.lock().unwrap().push('b'); }
        fn c() { ORDER.lock().unwrap().push('c'); }
        fn d() { ORDER.lock().unwrap().push('d'); }

        let mut graph = SystemGraph::new();
        graph.append_system(a, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(b, after(a), std::iter::empty()).unwrap();
        graph.append_system(c, after(a), std::iter::empty()).unwrap();
        graph.append_system(d, and(after(b), after(c)), std::iter::empty()).unwrap();

        let executor = SingleThreadedExecutor::new();
        executor.run(&World::default(), &graph).unwrap();
        executor.run(&World::default(), &graph).unwrap();

        let order = ORDER.lock().unwrap();
        assert_eq!(order.len(), 8);
        for pass in order.chunks(4) {
            assert_eq!(pass[0], 'a');
            assert_eq!(pass[3], 'd');
        }
    }

    #[test]
    fn error_is_returned() {
        struct Failing;
        impl System<(), ()> for Failing {
            fn run(&self, _: &World, _: &SharedExecutionState) -> anyhow::Result<()> {
                anyhow::bail!("failed")
            }
        }

        let mut graph = SystemGraph::new();
        graph.append_raw(std::any::TypeId::of::<Failing>(), Box::new(Failing), SystemInstruction::default(), std::iter::empty()).unwrap();

        let executor = SingleThreadedExecutor::new();
        assert!(executor.run(&World::default(), &graph).is_err());
        assert!(executor.state.is_complete());
    }
}
