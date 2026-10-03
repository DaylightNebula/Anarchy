use crate::{ExecutionState, World};

/// A parameter that a system may use as a parameter to extract
/// something from the game world.
pub trait SystemParam {
    fn extract(world: &World, exec_state: &ExecutionState) -> Self;
}

impl SystemParam for () {
    fn extract(_world: &World, _exec_state: &ExecutionState) -> Self {
        ()
    }
}

impl SystemParam for World {
    fn extract(world: &World, _exec_state: &ExecutionState) -> Self {
        world.clone()
    }
}
