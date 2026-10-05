use std::marker::PhantomData;

use crate::{SharedExecutionState, SystemParam, World};

/// Something that may be run as a system inside a large system network.
/// Takes a `World`, and an `ExecutionState`.
pub trait System<In, Out> {
    fn run(&self, world: &World, exec_state: &SharedExecutionState) -> anyhow::Result<()>;
}

/// A system that may be constructed from a function (via the `IntoSystem` trait).
/// `Marker` records the function's `SystemParam`s, which keeps the
/// `IntoSystem` impls for each function arity from overlapping.
pub struct FunctionSystem<F, Marker> {
    func: F,
    marker: PhantomData<fn() -> Marker>
}

/// Common constructor trait for turning a anything into a `System`.
/// `Marker` lets many different implementors resolve to the same `In` and
/// `Out` types, so they can be stored together (see `ErasedSystem`).
pub trait IntoSystem<In, Out, Marker> {
    type System: System<In, Out>;
    fn into_system(self) -> Self::System;
}

/// Macro to generate all needed `IntoSystem` and `System` traits to turn
/// functions with any ammount of `SystemParam`s into a `FunctionSystem`.
/// Every function system takes no input and produces no output, so all
/// resolve to `System<(), ()>`.
macro_rules! function_system {
    ($($name:ident),*) => {
        impl<Function, $($name),*> System<(), ()> for FunctionSystem<Function, fn($($name),*)>
        where Function: Fn($($name),*), $($name: SystemParam),* {
            #[allow(non_snake_case, unused_variables)]
            fn run(&self, world: &World, exec_state: &SharedExecutionState) -> anyhow::Result<()> {
                $(let $name = $name::extract(world, exec_state);)*
                (self.func)($($name),*);
                Ok(())
            }
        }

        impl<Function, $($name),*> IntoSystem<(), (), fn($($name),*)> for Function
        where Function: Fn($($name),*), $($name: SystemParam),* {
            type System = FunctionSystem<Function, fn($($name),*)>;
            fn into_system(self) -> Self::System {
                FunctionSystem {
                    func: self,
                    marker: PhantomData,
                }
            }
        }
    };
}

function_system!();
function_system!(A);
function_system!(A, B);
function_system!(A, B, C);
function_system!(A, B, C, D);
function_system!(A, B, C, D, E);
function_system!(A, B, C, D, E, F);
function_system!(A, B, C, D, E, F, G);
function_system!(A, B, C, D, E, F, G, H);
function_system!(A, B, C, D, E, F, G, H, I);
function_system!(A, B, C, D, E, F, G, H, I, J);
function_system!(A, B, C, D, E, F, G, H, I, J, K);
function_system!(A, B, C, D, E, F, G, H, I, J, K, L);
function_system!(A, B, C, D, E, F, G, H, I, J, K, L, M);
function_system!(A, B, C, D, E, F, G, H, I, J, K, L, M, N);
function_system!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O);
function_system!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P);

mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[test]
    fn empty_system() {
        fn test(_ok: ()) {}

        let exec_state = SharedExecutionState::default();
        let system = test.into_system();
        system.run(&World::default(), &exec_state).unwrap();
    }

    #[test]
    fn multi_param_system() {
        fn two(_a: (), _b: World) {}
        fn sixteen(
            _a: (), _b: (), _c: (), _d: (), _e: (), _f: (), _g: (), _h: (),
            _i: (), _j: (), _k: (), _l: (), _m: (), _n: (), _o: (), _p: World,
        ) {}

        let exec_state = SharedExecutionState::default();
        two.into_system().run(&World::default(), &exec_state).unwrap();
        sixteen.into_system().run(&World::default(), &exec_state).unwrap();
    }

    #[test]
    fn no_param_system() {
        fn none() {}

        let exec_state = SharedExecutionState::default();
        none.into_system().run(&World::default(), &exec_state).unwrap();
    }

    #[test]
    fn mixed_arity_systems_share_a_type() {
        fn one(_a: ()) {}
        fn two(_a: (), _b: World) {}

        let exec_state = SharedExecutionState::default();
        let systems: Vec<Box<dyn System<(), ()>>> = vec![
            Box::new(one.into_system()),
            Box::new(two.into_system()),
        ];
        for system in systems {
            system.run(&World::default(), &exec_state).unwrap();
        }
    }
}
