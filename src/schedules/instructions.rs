use std::any::Any;

use getset::{CopyGetters, Getters};

use crate::{IntoSystem, SystemKey, SystemPin};

#[derive(Getters, CopyGetters, Default, Debug, Clone, Hash)]
pub struct SystemInstruction {
    #[getset(get = "pub")]
    before: Vec<SystemKey>,
    #[getset(get = "pub")]
    after: Vec<SystemKey>,
    #[getset(get_copy = "pub")]
    pin: Option<SystemPin>
}

pub fn before<I, Marker>(system: I) -> SystemInstruction
    where I: IntoSystem<(), (), Marker> + 'static
{
    let mut instruction = SystemInstruction::default();
    instruction.before.push(system.type_id());
    return instruction;
}

pub fn after<I, Marker>(system: I) -> SystemInstruction
    where I: IntoSystem<(), (), Marker> + 'static
{
    let mut instruction = SystemInstruction::default();
    instruction.after.push(system.type_id());
    return instruction;
}

pub fn pin(pin: SystemPin) -> SystemInstruction {
    let mut instruction = SystemInstruction::default();
    instruction.pin = Some(pin);
    return instruction;
}

pub fn and(mut a: SystemInstruction, b: SystemInstruction) -> SystemInstruction {
    a.before.extend(b.before);
    a.after.extend(b.after);
    a.pin = a.pin.or(b.pin);
    return a;
}
