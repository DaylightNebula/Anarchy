use std::{any::TypeId, fmt::Debug, hash::{Hash, Hasher}};

use mutual::RelaxedMutex;
use rustc_hash::FxHasher;
pub use anarchy_macros::{AsAny, Component};
pub use mutual::AsAny;

pub type EntityID = u64;
pub type ComponentID = u64;
pub type ComponentIDGroup = Box<[ComponentID]>;
pub type DynComponent = Box<dyn Component>;
pub type DynComponents = Box<[RelaxedMutex<DynComponent>]>;

pub trait ComponentMeta: Component + 'static {
    fn id() -> u64 { fast_hash_type_id(TypeId::of::<Self>()) }
}

pub trait Component: AsAny + Debug + Send + 'static {
    fn get_id(&self) -> u64 { fast_hash_type_id(TypeId::of::<Self>()) }
}

#[inline(always)]
pub fn fast_hash_type_id(type_id: TypeId) -> u64 {
    let mut hasher = FxHasher::default();
    type_id.hash(&mut hasher);
    hasher.finish()
}
