//! Components, and the ids that identify them and the entities holding them.

use std::{any::TypeId, fmt::Debug, hash::{Hash, Hasher}};

use mutual::RelaxedMutex;
use rustc_hash::FxHasher;
pub use anarchy_macros::{AsAny, Component};
pub use mutual::AsAny;

pub mod info;

pub use info::*;

/// Identifies an entity. Ids are chosen by the caller and are not checked for uniqueness.
pub type EntityID = u64;
/// Identifies a component type, see [`fast_hash_type_id`].
pub type ComponentID = u64;
/// A list of component ids. A table's group is sorted, which lets queries find
/// components in it by binary search.
pub type ComponentIDGroup = Box<[ComponentID]>;
/// A type erased component.
pub type DynComponent = Box<dyn Component>;
/// A list of boxed components, which [`WorldImpl::insert`] moves into a table and
/// [`WorldImpl::remove`] hands back.
pub type DynComponents = Box<[RelaxedMutex<DynComponent>]>;

/// Type level access to a component's id, implemented by `#[derive(Component)]`.
pub trait ComponentMeta: Component + 'static {
    /// The id of this component type.
    fn id() -> u64 { fast_hash_type_id(TypeId::of::<Self>()) }
}

/// Data attached to an entity.  Usually implemented with `#[derive(Component)]`,
/// which also implements [`ComponentMeta`] and [`AsAny`], and requires the type to be `Sync`.
pub trait Component: AsAny + Debug + Send + 'static {
    /// The id of this value's component type, the same as [`ComponentMeta::id`].
    fn get_id(&self) -> u64 { fast_hash_type_id(TypeId::of::<Self>()) }

    /// How this value is stored, which a world needs to move it into a table.  By default
    /// the info registered under [`get_id`](Self::get_id), `#[derive(Component)]` returns
    /// [`ComponentInfo::of`] instead.
    fn info(&self) -> Option<&'static ComponentInfo> { ComponentInfo::get(self.get_id()) }
}

/// Hashes a [`TypeId`] into a `u64` with FxHash, used as component and resource ids.
#[inline(always)]
pub fn fast_hash_type_id(type_id: TypeId) -> u64 {
    let mut hasher = FxHasher::default();
    type_id.hash(&mut hasher);
    hasher.finish()
}
