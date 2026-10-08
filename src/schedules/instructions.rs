//! Instructions that order systems in a [`SystemGraph`](crate::SystemGraph).

use std::any::Any;

use getset::{CopyGetters, Getters};

use crate::{IntoSystem, SystemKey, SystemMeta, SystemPin};

/// Where a system runs relative to others.  Build one with [`before`], [`after`],
/// [`before_meta`], [`after_meta`] or [`pin`], and combine them with [`and`].
/// The default instruction places no constraints.
#[derive(Getters, CopyGetters, Default, Debug, Clone, Hash)]
pub struct SystemInstruction {
    /// Run before each of these systems.
    #[getset(get = "pub")]
    before: Vec<SystemKey>,
    /// Run after each of these systems.
    #[getset(get = "pub")]
    after: Vec<SystemKey>,
    /// Run before every system with any of these metas.
    #[getset(get = "pub")]
    before_meta: Vec<SystemMeta>,
    /// Run after every system with any of these metas.
    #[getset(get = "pub")]
    after_meta: Vec<SystemMeta>,
    /// The pin to run in, `None` for [`SystemPin::Normal`].
    #[getset(get_copy = "pub")]
    pin: Option<SystemPin>
}

/// Run before `system`.
pub fn before<I, Marker>(system: I) -> SystemInstruction
    where I: IntoSystem<(), (), Marker> + 'static
{
    let mut instruction = SystemInstruction::default();
    instruction.before.push(system.type_id());
    return instruction;
}

/// Run after `system`.
pub fn after<I, Marker>(system: I) -> SystemInstruction
    where I: IntoSystem<(), (), Marker> + 'static
{
    let mut instruction = SystemInstruction::default();
    instruction.after.push(system.type_id());
    return instruction;
}

/// Run before every system tagged with any of `metas`.
pub fn before_meta(metas: impl IntoIterator<Item = SystemMeta>) -> SystemInstruction {
    let mut instruction = SystemInstruction::default();
    instruction.before_meta.extend(metas);
    return instruction;
}

/// Run after every system tagged with any of `metas`.
pub fn after_meta(metas: impl IntoIterator<Item = SystemMeta>) -> SystemInstruction {
    let mut instruction = SystemInstruction::default();
    instruction.after_meta.extend(metas);
    return instruction;
}

/// Run in `pin`.
pub fn pin(pin: SystemPin) -> SystemInstruction {
    let mut instruction = SystemInstruction::default();
    instruction.pin = Some(pin);
    return instruction;
}

/// Combine two instructions.  If both set a pin, `a`'s is kept.
pub fn and(mut a: SystemInstruction, b: SystemInstruction) -> SystemInstruction {
    a.before.extend(b.before);
    a.after.extend(b.after);
    a.before_meta.extend(b.before_meta);
    a.after_meta.extend(b.after_meta);
    a.pin = a.pin.or(b.pin);
    return a;
}
