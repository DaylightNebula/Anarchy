//! Components, and the ids that identify them and the entities holding them.

use std::{any::{TypeId, type_name}, fmt::Debug, hash::{Hash, Hasher}};

use mutual::RelaxedMutex;
use rustc_hash::FxHasher;
pub use anarchy_macros::{AsAny, Component};
pub use mutual::AsAny;

/// Identifies an entity. Ids are chosen by the caller and are not checked for uniqueness.
pub type EntityID = u64;
/// Identifies a component type, see [`fast_hash_type_id`].
pub type ComponentID = u64;
/// A list of component ids. A table's group is sorted, which lets queries find
/// components in it by binary search.
pub type ComponentIDGroup = Box<[ComponentID]>;
/// A type erased component.
pub type DynComponent = Box<dyn Component>;
/// An entity's components, each behind its own lock so they can be borrowed
/// independently. Clones share the same components.
pub type DynComponents = Box<[RelaxedMutex<DynComponent>]>;

/// Type level access to a component's id, implemented by `#[derive(Component)]`.
pub trait ComponentMeta: Component + 'static {
    /// The name of this component type.
    fn name() -> &'static str { type_name::<Self>() }

    /// The id of this component type.
    fn id() -> u64 { fast_hash_type_id(TypeId::of::<Self>()) }
}

/// Data attached to an entity.  Usually implemented with `#[derive(Component)]`,
/// which also implements [`ComponentMeta`] and [`AsAny`].
pub trait Component: AsAny + Debug + Send + 'static {
    /// The name of this value's component type, the same as [`ComponentMeta::name`].
    fn get_name(&self) -> &'static str { type_name::<Self>() }

    /// The id of this value's component type, the same as [`ComponentMeta::id`].
    fn get_id(&self) -> u64 { fast_hash_type_id(TypeId::of::<Self>()) }

    /// Serialize this component.
    fn serialize(&self) -> Option<(&'static str, &dyn erased_serde::Serialize)> { None }

    /// This component as a [`SerializedComponent`], which serializes tagged with its type
    /// so it can be deserialized again with [`deserialize_component`].
    fn as_serialized(&self) -> Option<&dyn SerializedComponent> { None }
}

/// A component that serializes tagged with its type, implemented by `#[derive(Component)]`
/// for non generic types without `#[component(no_serialize)]`. Each type is registered
/// with typetag under `"<crate>::<Type>"`.
#[typetag::serde]
pub trait SerializedComponent: Component {}

/// Deserializes a component written through [`Component::as_serialized`], picking its
/// type from the tag.
pub fn deserialize_component<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<DynComponent, D::Error> {
    let component: Box<dyn SerializedComponent> = serde::Deserialize::deserialize(deserializer)?;
    Ok(component)
}

/// Hashes a [`TypeId`] into a `u64` with FxHash, used as component and resource ids.
#[inline(always)]
pub fn fast_hash_type_id(type_id: TypeId) -> u64 {
    let mut hasher = FxHasher::default();
    type_id.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use anyhow::Context;

use super::*;

    #[derive(Debug, PartialEq, Component)]
    struct Pos { x: i32 }

    #[derive(Debug, PartialEq, Component)]
    #[serde(rename_all = "UPPERCASE")]
    enum Shape {
        Circle { radius: f32 },
        #[serde(rename = "box")]
        Square(f32),
    }

    #[derive(Debug, PartialEq, Component)]
    struct Wrapper<T: Debug + Send + 'static> {
        #[serde(skip)]
        skipped: u8,
        value: T,
    }

    #[derive(Debug, Component)]
    #[component(no_serialize)]
    struct Opaque;

    #[test]
    fn test_serder() -> anyhow::Result<()> {
        let precomp = Shape::Circle { radius: 32.0 };
        let serialized = serde_json::to_string(&precomp.as_serialized().context("Missing as serialized")?)?;

        let comp: DynComponent = deserialize_component(&mut serde_json::Deserializer::from_str(&serialized))?;
        let comp: &Shape = comp.as_any().downcast_ref().context("Failed to downcast")?;

        assert!(comp == &precomp);

        Ok(())
    }

    #[test]
    fn derive_serializes() {
        let (name, value) = Component::serialize(&Pos { x: 1 }).unwrap();
        assert_eq!(name, Pos::name());
        assert_eq!(serde_json::to_string(value).unwrap(), r#"{"x":1}"#);
    }

    #[test]
    fn derive_deserializes() {
        assert_eq!(serde_json::from_str::<Pos>(r#"{"x":1}"#).unwrap(), Pos { x: 1 });
    }

    #[test]
    fn serde_attributes_apply() {
        assert_eq!(serde_json::to_string(&Shape::Circle { radius: 1.0 }).unwrap(), r#"{"CIRCLE":{"radius":1.0}}"#);
        assert_eq!(serde_json::to_string(&Shape::Square(2.0)).unwrap(), r#"{"box":2.0}"#);
        assert_eq!(serde_json::from_str::<Shape>(r#"{"box":2.0}"#).unwrap(), Shape::Square(2.0));
    }

    #[test]
    fn generic_round_trip() {
        let wrapper = Wrapper { skipped: 7, value: String::from("hi") };
        let json = serde_json::to_string(&wrapper).unwrap();
        assert_eq!(json, r#"{"value":"hi"}"#);
        assert_eq!(serde_json::from_str::<Wrapper<String>>(&json).unwrap(), Wrapper { skipped: 0, value: String::from("hi") });
    }

    #[test]
    fn tagged_round_trip() {
        let json = serde_json::to_string(Pos { x: 1 }.as_serialized().unwrap()).unwrap();
        assert_eq!(json, r#"{"anarchy::Pos":{"x":1}}"#);

        let component = deserialize_component(&mut serde_json::Deserializer::from_str(&json)).unwrap();
        assert_eq!(component.get_id(), Pos::id());
        assert_eq!(component.as_any().downcast_ref::<Pos>(), Some(&Pos { x: 1 }));
    }

    #[test]
    fn generics_are_not_tagged() {
        assert!(Wrapper { skipped: 0, value: 1u8 }.as_serialized().is_none());
    }

    #[test]
    fn no_serialize_opts_out() {
        assert!(Opaque.serialize().is_none());
        assert!(Opaque.as_serialized().is_none());
    }
}
