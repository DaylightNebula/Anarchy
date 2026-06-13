use std::sync::atomic::AtomicU32;

use derive_more::{Deref, DerefMut};

use crate::{AsAny, MutCastGuard, RefCastGuard, ScheduleID, SystemExtractor, World};

pub type ResourceID = u32;
pub static NEXT_RESOURCE_ID: AtomicU32 = AtomicU32::new(0);

/// The standard metadata of a resource so we can get data without creating an instance
/// without validating dyn safety.
pub trait ResourceMeta {
    fn id() -> ResourceID;
}

/// The trait that must be implemented by all resources in use by a `World`.
pub trait Resource: AsAny + Send + Sync {
    fn get_id(&self) -> ResourceID;
}

impl AsAny for Box<dyn Resource> {
    fn as_any(&self) -> &dyn std::any::Any {
        (**self).as_any()
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        (**self).as_any_mut()
    }
}

/// Extractor to extract a immutable reference to a `Resource` from a `World` via a system.
#[derive(Deref, DerefMut)]
pub struct Res<R: Resource + ResourceMeta + 'static>(RefCastGuard<Box<dyn Resource + 'static>, R>);
impl <'a, I, R> SystemExtractor<'a, I> for Res<R> 
    where R: Resource + ResourceMeta + 'static
{
    fn extract(
        _id: ScheduleID, 
        world: &'a World, 
        _inputs: Option<I>
    ) -> (Self, Option<I>) {
        (Self(
            world
                .get_resource_ref::<R>()
                .expect("Failed to get resource")
        ), _inputs)
    }
}

/// Extractor to extract a mutable reference to a `Resource` from a `World` via a system.
#[derive(Deref, DerefMut)]
pub struct ResMut<R: Resource + ResourceMeta + 'static>(MutCastGuard<Box<dyn Resource + 'static>, R>);
impl <'a, I, R> SystemExtractor<'a, I> for ResMut<R> 
    where R: Resource + ResourceMeta + 'static
{
    fn extract(
        _id: ScheduleID, 
        world: &'a World, 
        _inputs: Option<I>
    ) -> (Self, Option<I>) {
        (
            Self(
                world
                    .get_resource_mut::<R>()
                    .expect("Failed to get resource")
            ), 
            _inputs
        )
    }
}