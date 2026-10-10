//! Parameters systems take as arguments.

use crate::{SharedExecutionState, SystemKey, World};

/// A parameter that a system may use as a parameter to extract
/// something from the game world.
pub trait SystemParam {
    /// Build the parameter for one run of the system identified by `system`.
    fn extract(world: &World, exec_state: &SharedExecutionState, system: SystemKey) -> Self;
}

impl SystemParam for () {
    fn extract(_world: &World, _exec_state: &SharedExecutionState, _system: SystemKey) -> Self {
        ()
    }
}

impl SystemParam for World {
    fn extract(world: &World, _exec_state: &SharedExecutionState, _system: SystemKey) -> Self {
        world.clone()
    }
}
