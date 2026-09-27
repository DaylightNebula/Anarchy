use std::fmt::Debug;

use mutual::{AsAny, RelaxedMutex};

pub mod tables;

pub use tables::*;

pub type EntityID = u64;
pub type ComponentID = u64;
pub type ComponentIDGroup<'a> = &'a [ComponentID];
pub type DynComponent = Box<dyn Component>;
pub type DynComponents = Box<[RelaxedMutex<DynComponent>]>;

pub trait Component: AsAny + Debug + Send {}
