use std::marker::PhantomData;

use crate::World;

pub type ScheduleFn = fn(world: &World, exec_state: &ExecutionState) -> anyhow::Result<()>;

pub struct ExecutionState;

pub struct ScheduleGraph {
    pub root_nodes: Vec<ScheduleFn>
}

pub trait System<In, Out> {
    fn run(&self, world: &World, exec_state: &ExecutionState) -> anyhow::Result<()>;
}

pub struct EmptySystem<F> where F: Fn() { func: F }

impl<F> System<(), ()> for EmptySystem<F> where F: Fn() {
    fn run(&self, _world: &World, _exec_state: &ExecutionState) -> anyhow::Result<()> {
        (self.func)();
        Ok(())
    }
}

pub struct FunctionSystem<F, In, Out> where F: IntoSystem<In, Out> {
    func: F,
    marker: PhantomData<fn(In) -> Out>
}

impl<F, In, Out> System<In, Out> for FunctionSystem<F, In, Out> where F: Fn(In) -> Out {
    fn run(&self, world: &World, exec_state: &ExecutionState) -> anyhow::Result<()> {
        // (self.func)(world, exec_state)
        Ok(())
    }
}

pub trait IntoSystem<In, Out> {
    type System: System<In, Out>;
    fn into_system(self) -> Self::System;
}

// impl <F> IntoSystem<(), ()> for F where F: Fn() {
//     type System = EmptySystem<F>;
//     fn into_system(self) -> Self::System {
//         EmptySystem {
//             func: self
//         }
//     }
// }

impl <F, In, Out> IntoSystem<In, Out> for F where F: Fn(In) -> Out {
    type System = FunctionSystem<F, In, Out>;
    fn into_system(self) -> Self::System {
        FunctionSystem {
            func: self,
            marker: PhantomData,
        }
    }
}

mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[test]
    fn empty_system() {
        fn test(_ok: ()) {}

        let system = test.into_system();
        system.run(&World::default(), &ExecutionState).unwrap();
    }
}
