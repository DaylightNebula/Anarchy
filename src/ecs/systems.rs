use crate::{ScheduleID, World};

pub type BoxedSystem = Box<dyn System<(), ()>>;

pub trait System<I, O>: Send + Sync {
    fn name(&self) -> &str;

    fn execute<'a>(
        &self,
        schedule_id: ScheduleID,
        world: &'a World,
        inputs: &'a I
    ) -> O;
}
