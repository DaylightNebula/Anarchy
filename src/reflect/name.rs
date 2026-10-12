//! [`Name`], a component naming an entity.

use crate::*;

/// A human readable name for an entity.  Scenes add it to entities named with `#name`,
/// and editors can show it in place of the id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Component, Reflect)]
pub struct Name(pub String);

impl Name {
    /// A name from anything that converts into a `String`.
    pub fn new(name: impl Into<String>) -> Self { Self(name.into()) }
}
