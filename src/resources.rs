use std::{any::TypeId, fmt::Debug};

use mutual::AsAny;
pub use anarchy_macros::Resource;

use crate::{SystemMeta, fast_hash_type_id};

pub type ResourceID = u64;
pub type DynResource = Box<dyn Resource>;

pub trait ResourceMeta: Resource + 'static {
    fn id() -> ResourceID { fast_hash_type_id(TypeId::of::<Self>()) }
    fn system_meta() -> SystemMeta { TypeId::of::<Self>() }
}

pub trait Resource: AsAny + Debug + Send + 'static {
    fn get_id(&self) -> ResourceID { fast_hash_type_id(TypeId::of::<Self>()) }
    fn get_system_meta(&self) -> SystemMeta { TypeId::of::<Self>() }
}
