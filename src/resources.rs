use std::{any::TypeId, fmt::Debug};

use mutual::AsAny;

use crate::{SystemMeta, fast_hash_type_id};

pub trait ResourceMeta: Resource + 'static {
    fn id() -> u64 { fast_hash_type_id(TypeId::of::<Self>()) }
    fn system_meta() -> SystemMeta { TypeId::of::<Self>() }
}

pub trait Resource: AsAny + Debug + Send + 'static {
    fn get_id(&self) -> u64 { fast_hash_type_id(TypeId::of::<Self>()) }
    fn get_system_meta(&self) -> SystemMeta { TypeId::of::<Self>() }
}