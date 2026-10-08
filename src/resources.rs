//! Resources, single values stored on a [`World`](crate::World) instead of on an entity.

use std::{any::TypeId, fmt::Debug};

use mutual::AsAny;
pub use anarchy_macros::Resource;

use crate::{SystemMeta, fast_hash_type_id};

/// Identifies a resource type, see [`fast_hash_type_id`].
pub type ResourceID = u64;
/// A type erased resource.
pub type DynResource = Box<dyn Resource>;

/// Type level access to a resource's ids, implemented by `#[derive(Resource)]`.
pub trait ResourceMeta: Resource + 'static {
    /// The id of this resource type.
    fn id() -> ResourceID { fast_hash_type_id(TypeId::of::<Self>()) }
    /// This resource type as a [`SystemMeta`], so systems can be tagged with it.
    fn system_meta() -> SystemMeta { TypeId::of::<Self>() }
}

/// A single value of a type stored on a world.  Usually implemented with
/// `#[derive(Resource)]`, which also implements [`ResourceMeta`] and [`AsAny`].
pub trait Resource: AsAny + Debug + Send + 'static {
    /// The id of this value's resource type, the same as [`ResourceMeta::id`].
    fn get_id(&self) -> ResourceID { fast_hash_type_id(TypeId::of::<Self>()) }
    /// The same as [`ResourceMeta::system_meta`].
    fn get_system_meta(&self) -> SystemMeta { TypeId::of::<Self>() }
}
