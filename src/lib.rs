use std::{any::TypeId, fmt::Debug, hash::{Hash, Hasher}};

use mutual::{AsAny, RelaxedMutex};
use rustc_hash::FxHasher;

pub mod queries;
pub mod tables;
pub mod worlds;

pub use queries::*;
pub use tables::*;
pub use worlds::*;

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
fn fast_hash_type_id(type_id: TypeId) -> u64 {
    let mut hasher = FxHasher::default();
    type_id.hash(&mut hasher);
    hasher.finish()
}
