use std::marker::PhantomData;

use crate::{SystemParam, World};

/// The execution state of a system.  Used for SystemParams to read
/// some common state like schedule and system IDs.
pub struct ExecutionState;

/// Something that may be run as a system inside a large system network.
/// Takes a `World`, and an `ExecutionState`.
pub trait System<In, Out> {
    fn run(&self, world: &World, exec_state: &ExecutionState) -> anyhow::Result<()>;
}

/// A system that may be constructed from a function (via the `IntoSystem` trait).
pub struct FunctionSystem<F, In, Out> where F: IntoSystem<In, Out> {
    func: F,
    marker: PhantomData<fn(In) -> Out>
}

impl<Function, A, Out> System<A, Out> for FunctionSystem<Function, A, Out> where Function: Fn(A) -> Out, A: SystemParam {
    fn run(&self, world: &World, exec_state: &ExecutionState) -> anyhow::Result<()> {
        let a = A::extract(world, exec_state);
        (self.func)(a);
        Ok(())
    }
}

/// Common constructor trait for turning a function into `FunctionSystem`.
pub trait IntoSystem<In, Out> {
    type System: System<In, Out>;
    fn into_system(self) -> Self::System;
}

impl <F, A, Out> IntoSystem<A, Out> for F where F: Fn(A) -> Out, A: SystemParam {
    type System = FunctionSystem<F, A, Out>;
    fn into_system(self) -> Self::System {
        FunctionSystem {
            func: self,
            marker: PhantomData,
        }
    }
}

/// Macro to generate all needed `IntoSystem` and `System` traits to turn
/// functions with any ammount of `SystemParam`s into a `FunctionSystem`.
macro_rules! function_system {
    ($($name:ident),+) => {
        impl<Function, $($name,)+ Out> System<($($name,)+), Out> for FunctionSystem<Function, ($($name,)+), Out>
        where Function: Fn($($name),+) -> Out, $($name: SystemParam),+ {
            #[allow(non_snake_case)]
            fn run(&self, world: &World, exec_state: &ExecutionState) -> anyhow::Result<()> {
                $(let $name = $name::extract(world, exec_state);)+
                (self.func)($($name),+);
                Ok(())
            }
        }

        impl<F, $($name,)+ Out> IntoSystem<($($name,)+), Out> for F
        where F: Fn($($name),+) -> Out, $($name: SystemParam),+ {
            type System = FunctionSystem<F, ($($name,)+), Out>;
            fn into_system(self) -> Self::System {
                FunctionSystem {
                    func: self,
                    marker: PhantomData,
                }
            }
        }
    };
}

function_system!(A, B);
function_system!(A, B, C);
function_system!(A, B, C, D);
function_system!(A, B, C, D, E);
function_system!(A, B, C, D, E, F0);
function_system!(A, B, C, D, E, F0, G);
function_system!(A, B, C, D, E, F0, G, H);
function_system!(A, B, C, D, E, F0, G, H, I);
function_system!(A, B, C, D, E, F0, G, H, I, J);
function_system!(A, B, C, D, E, F0, G, H, I, J, K);
function_system!(A, B, C, D, E, F0, G, H, I, J, K, L);
function_system!(A, B, C, D, E, F0, G, H, I, J, K, L, M);
function_system!(A, B, C, D, E, F0, G, H, I, J, K, L, M, N);
function_system!(A, B, C, D, E, F0, G, H, I, J, K, L, M, N, O);
function_system!(A, B, C, D, E, F0, G, H, I, J, K, L, M, N, O, P);

mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[test]
    fn empty_system() {
        fn test(_ok: ()) {}

        let system = test.into_system();
        system.run(&World::default(), &ExecutionState).unwrap();
    }

    #[test]
    fn multi_param_system() {
        fn two(_a: (), _b: World) {}
        fn sixteen(
            _a: (), _b: (), _c: (), _d: (), _e: (), _f: (), _g: (), _h: (),
            _i: (), _j: (), _k: (), _l: (), _m: (), _n: (), _o: (), _p: World,
        ) {}

        two.into_system().run(&World::default(), &ExecutionState).unwrap();
        sixteen.into_system().run(&World::default(), &ExecutionState).unwrap();
    }
}
