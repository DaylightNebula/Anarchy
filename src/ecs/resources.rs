use std::sync::atomic::AtomicU32;

use derive_more::{Deref, DerefMut};

use crate::{AsAny, MutCastGuard, RefCastGuard, ScheduleID, SystemExtractor, World};

/// The unique ID of a resource type, one `World` may only hold a single instance per ID.
pub type ResourceID = u32;
/// Global counter used to hand out the next unused `ResourceID`.
pub static NEXT_RESOURCE_ID: AtomicU32 = AtomicU32::new(0);

/// The standard metadata of a resource so we can get data without creating an instance
/// without validating dyn safety.
pub trait ResourceMeta {
    /// Returns the ID assigned to this resource type, allocated once from
    /// `NEXT_RESOURCE_ID` and cached (see the `Resource` derive macro).
    fn id() -> ResourceID;
}

/// The trait that must be implemented by all resources in use by a `World`.
pub trait Resource: AsAny + Send + Sync {
    /// Returns the ID of this resource's type.
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
        _inputs: Option<&'a I>
    ) -> (Self, Option<&'a I>) {
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
        _inputs: Option<&'a I>
    ) -> (Self, Option<&'a I>) {
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