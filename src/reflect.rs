//! Runtime reflection: look inside values by field name, build them from data and
//! edit them through `&mut dyn Reflect`, without knowing their types at compile time.
//!
//! - [`Reflect`] is implemented by `#[derive(Reflect)]` and for common std types.
//! - [`Value`] is an untyped tree of data, read with [`Reflect::to_value`] and written
//!   back with [`Reflect::apply`] (a patch, only the fields given change) or
//!   [`FromValue::from_value`].
//! - [`TypeInfo`] describes a type's fields, variants and docs, see [`Typed`].
//! - [`get_path`] and [`set_path`] reach nested fields with paths like `"pos.x"` or `"items[2]"`.
//! - A [`TypeRegistry`] maps names and component ids to the functions that build and
//!   cast reflected types, and [`World`](crate::World) uses it for type erased editing.

use std::{any::{Any, TypeId}, collections::HashMap, sync::{OnceLock, RwLock}};

use anyhow::{Context, bail};

use crate::EntityID;

pub use anarchy_macros::Reflect;

pub mod impls;
pub mod name;
pub mod path;
pub mod registry;
pub mod value;
pub mod world;

pub use name::*;
pub use path::*;
pub use registry::*;
pub use value::*;
pub use world::*;

/// A value whose fields can be read, written and described at runtime.  Usually
/// implemented with `#[derive(Reflect)]`, which also implements [`Typed`] and [`FromValue`].
///
/// [`to_value`](Self::to_value) and [`apply`](Self::apply) have default implementations
/// for structs and enums, built on [`reflect_ref`](Self::reflect_ref).  Leaf types,
/// those returning [`ReflectRef::Value`], and lists must implement them.
pub trait Reflect: Send + 'static {
    /// Describes this value's type.
    fn reflect_info(&self) -> &'static TypeInfo;
    /// This value's fields, variant or items.
    fn reflect_ref(&self) -> ReflectRef<'_>;
    /// Mutable access to this value's fields, variant or items.
    fn reflect_mut(&mut self) -> ReflectMut<'_>;
    /// This value as `dyn Any`, for downcasting.
    fn reflect_any(&self) -> &dyn Any;
    /// This value as `dyn Any`, for downcasting.
    fn reflect_any_mut(&mut self) -> &mut dyn Any;
    /// This boxed value as `Box<dyn Any>`, for downcasting.
    fn into_any(self: Box<Self>) -> Box<dyn Any>;

    /// This value as an untyped [`Value`].
    fn to_value(&self) -> Value {
        match self.reflect_ref() {
            ReflectRef::Struct(fields) => fields_to_value(fields, struct_fields(self.reflect_info())),
            ReflectRef::Enum(e) => {
                let variant = &variants(self.reflect_info())[e.variant_index()];
                Value::Enum { variant: variant.name.into(), value: Box::new(fields_to_value(e, Some(&variant.kind))) }
            }
            ReflectRef::List(list) => Value::List((0..list.len()).filter_map(|i| list.get(i)).map(|item| item.to_value()).collect()),
            ReflectRef::Value(_) => Value::Unit
        }
    }

    /// Patch this value with `value`.  Struct fields that `value` leaves out keep their
    /// current values, while a different enum variant replaces the whole value.
    ///
    /// # Errors
    ///
    /// Errors if `value` doesn't fit this type, e.g. an unknown field or a string for a
    /// number.  Fields applied before the error keep their new values.
    fn apply(&mut self, value: &Value) -> anyhow::Result<()> {
        let info = self.reflect_info();
        match self.reflect_mut() {
            ReflectMut::Struct(fields) => apply_fields(fields, struct_fields(info), value),
            ReflectMut::Enum(e) => {
                let Value::Enum { variant, value } = value else { bail!("expected an enum variant of {}, found {value}", info.short_name) };
                let current = &variants(info)[e.variant_index()];
                if current.name == variant { apply_fields(e, Some(&current.kind), value) } else { e.set_variant(variant, value) }
            }
            ReflectMut::List(_) | ReflectMut::Value(_) => bail!("{} must implement Reflect::apply", info.type_name)
        }
    }
}

/// Static access to a type's [`TypeInfo`].
pub trait Typed {
    /// Describes this type.
    fn type_info() -> &'static TypeInfo;
}

/// Builds a value from an untyped [`Value`].
pub trait FromValue: Sized {
    /// Build a value from `value`.  Struct fields that `value` leaves out are filled from
    /// their [`fallback`](Self::fallback), or from `Default` on types deriving with
    /// `#[reflect(default)]`.
    ///
    /// # Errors
    ///
    /// Errors if `value` doesn't fit this type, or leaves out a field without a fallback.
    fn from_value(value: &Value) -> anyhow::Result<Self>;

    /// The value used for a missing field, `None` if the field must be given.  Numbers
    /// fall back to zero, strings and lists to empty and `Option` to `None`.
    fn fallback() -> Option<Self> { None }
}

/// Read access to the inside of a [`Reflect`] value.
pub enum ReflectRef<'a> {
    /// A struct, tuple struct, unit struct or tuple.
    Struct(&'a dyn ReflectFields),
    /// An enum, its fields are those of the current variant.
    Enum(&'a dyn ReflectEnum),
    /// A list, such as a `Vec` or an array.
    List(&'a dyn ReflectList),
    /// A leaf with no reflected parts, such as a number or a string.
    Value(&'a dyn Reflect)
}

/// Write access to the inside of a [`Reflect`] value.
pub enum ReflectMut<'a> {
    /// A struct, tuple struct, unit struct or tuple.
    Struct(&'a mut dyn ReflectFields),
    /// An enum, its fields are those of the current variant.
    Enum(&'a mut dyn ReflectEnum),
    /// A list, such as a `Vec` or an array.
    List(&'a mut dyn ReflectList),
    /// A leaf with no reflected parts, such as a number or a string.
    Value(&'a mut dyn Reflect)
}

/// Fields by index, in declaration order.  Their names are in the type's [`TypeInfo`].
pub trait ReflectFields {
    /// The number of reflected fields.
    fn field_count(&self) -> usize;
    /// The field at `index`, or `None` if out of range.
    fn field_at(&self, index: usize) -> Option<&dyn Reflect>;
    /// The field at `index`, or `None` if out of range.
    fn field_at_mut(&mut self, index: usize) -> Option<&mut dyn Reflect>;
}

/// An enum, whose [`ReflectFields`] are those of its current variant.
pub trait ReflectEnum: ReflectFields {
    /// The index of the current variant in [`TypeKind::Enum`].
    fn variant_index(&self) -> usize;
    /// Replace this value with `variant`, built from `value` as by [`FromValue::from_value`].
    ///
    /// # Errors
    ///
    /// Errors if there is no such variant or `value` doesn't fit it.
    fn set_variant(&mut self, variant: &str, value: &Value) -> anyhow::Result<()>;
}

/// A list of reflected items.
pub trait ReflectList {
    /// The number of items.
    fn len(&self) -> usize;
    /// Returns true if there are no items.
    fn is_empty(&self) -> bool { self.len() == 0 }
    /// The item at `index`, or `None` if out of range.
    fn get(&self, index: usize) -> Option<&dyn Reflect>;
    /// The item at `index`, or `None` if out of range.
    fn get_mut(&mut self, index: usize) -> Option<&mut dyn Reflect>;
    /// Append an item built from `value`.
    ///
    /// # Errors
    ///
    /// Errors if `value` doesn't fit the item type, or the list has a fixed length.
    fn push_value(&mut self, value: &Value) -> anyhow::Result<()>;
    /// Remove the item at `index`.
    ///
    /// # Errors
    ///
    /// Errors if `index` is out of range, or the list has a fixed length.
    fn remove_at(&mut self, index: usize) -> anyhow::Result<()>;
}

/// Describes a reflected type, see [`Typed::type_info`].
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TypeInfo {
    /// The full name, from [`std::any::type_name`].
    pub type_name: &'static str,
    /// The name without module paths, e.g. `Vec<f32>` for `alloc::vec::Vec<f32>`.
    pub short_name: String,
    /// The type's doc comment.
    pub docs: &'static str,
    /// The type's shape.
    pub kind: TypeKind
}

/// The shape of a reflected type.
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum TypeKind {
    /// A leaf, such as a number or a string.
    Value,
    /// A struct, tuple struct, unit struct or tuple, with its reflected fields.
    Struct(VariantKind),
    /// An enum with its variants.
    Enum(Vec<VariantInfo>),
    /// A list of items, with a fixed `len` for arrays.
    List {
        /// The item type.
        #[cfg_attr(feature = "serde", serde(serialize_with = "serialize_type_name"))]
        item: fn() -> &'static TypeInfo,
        /// The length of a fixed size list.
        len: Option<usize>
    }
}

/// The fields of a struct or enum variant.
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum VariantKind {
    /// No fields.
    Unit,
    /// Fields by position, named `"0"`, `"1"` and so on.
    Tuple(Vec<FieldInfo>),
    /// Fields by name.
    Struct(Vec<FieldInfo>)
}

impl VariantKind {
    /// The fields, empty for [`VariantKind::Unit`].
    pub fn fields(&self) -> &[FieldInfo] {
        match self { Self::Unit => &[], Self::Tuple(fields) | Self::Struct(fields) => fields }
    }

    /// The index of the field `name`, which may be a position like `"0"`.
    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.fields().iter().position(|field| field.name == name)
            .or_else(|| name.parse().ok().filter(|i| *i < self.fields().len()))
    }
}

/// Describes an enum variant.
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct VariantInfo {
    /// The variant's name.
    pub name: &'static str,
    /// The variant's doc comment.
    pub docs: &'static str,
    /// The variant's fields.
    pub kind: VariantKind
}

/// Describes a field.
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FieldInfo {
    /// The field's name, or its position for tuple fields.
    pub name: &'static str,
    /// The field's type.
    #[cfg_attr(feature = "serde", serde(serialize_with = "serialize_type_name"))]
    pub ty: fn() -> &'static TypeInfo,
    /// The field's doc comment.
    pub docs: &'static str,
    /// Editor hints from `#[reflect(hint(key = value))]`, e.g. a range for a slider.
    pub hints: Vec<(&'static str, Value)>
}

impl FieldInfo {
    /// The value of the hint `key`, if the field has it.
    pub fn hint(&self, key: &str) -> Option<&Value> {
        self.hints.iter().find(|(k, _)| *k == key).map(|(_, value)| value)
    }

    /// Returns true if the field holds entity ids, marked with `#[reflect(entity)]`, which
    /// is the hint `entity = true`.  Its integers are read as [`EntityRef`]s, so saved
    /// scenes refer to entities by name.
    pub fn is_entity(&self) -> bool {
        self.hint("entity") == Some(&Value::Bool(true))
    }
}

/// Serializes a type by its full name.
#[cfg(feature = "serde")]
fn serialize_type_name<S: serde::Serializer>(ty: &fn() -> &'static TypeInfo, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(ty().type_name)
}

/// Lazily built [`TypeInfo`]s keyed by type, so a `static` in a generic function can
/// hold one per instantiation.  Used by `#[derive(Reflect)]`.
#[doc(hidden)]
pub struct TypeInfoCell(OnceLock<RwLock<HashMap<TypeId, &'static TypeInfo>>>);

impl TypeInfoCell {
    /// An empty cell.
    pub const fn new() -> Self { Self(OnceLock::new()) }

    /// The info for `T`, built by `f` on first use and leaked so it lives forever.
    pub fn get_or_init<T: 'static>(&self, f: impl FnOnce() -> TypeInfo) -> &'static TypeInfo {
        let map = self.0.get_or_init(Default::default);
        if let Some(info) = map.read().unwrap().get(&TypeId::of::<T>()) { return info }
        // build without the lock held, since `f` may need other types' info
        let info = f();
        *map.write().unwrap().entry(TypeId::of::<T>()).or_insert_with(|| Box::leak(Box::new(info)))
    }
}

/// `type_name` with module paths removed, keeping generic arguments.
pub fn short_type_name(type_name: &str) -> String {
    let mut short = String::with_capacity(type_name.len());
    let mut segment = String::new();
    for c in type_name.chars() {
        match c {
            ':' => segment.clear(),
            '<' | '>' | ',' | ' ' | '(' | ')' | '[' | ']' | ';' | '&' => {
                short.push_str(&segment);
                segment.clear();
                short.push(c);
            }
            _ => segment.push(c)
        }
    }
    short + &segment
}

/// Turns every integer in `value` into an [`EntityRef::Id`], looking inside lists, tuples
/// and enum variants such as `Some`.
fn ids_to_entities(value: Value) -> Value {
    match value {
        Value::Int(id) => match EntityID::try_from(id) { Ok(id) => Value::Entity(EntityRef::Id(id)), Err(_) => Value::Int(id) },
        Value::List(items) => Value::List(items.into_iter().map(ids_to_entities).collect()),
        Value::Tuple(items) => Value::Tuple(items.into_iter().map(ids_to_entities).collect()),
        Value::Enum { variant, value } => Value::Enum { variant, value: Box::new(ids_to_entities(*value)) },
        other => other
    }
}

/// The fields of a struct type, `None` for other kinds.
fn struct_fields(info: &TypeInfo) -> Option<&VariantKind> {
    match &info.kind { TypeKind::Struct(kind) => Some(kind), _ => None }
}

/// The variants of an enum type, empty for other kinds.
fn variants(info: &TypeInfo) -> &[VariantInfo] {
    match &info.kind { TypeKind::Enum(variants) => variants, _ => &[] }
}

/// `fields` as a [`Value::Struct`], [`Value::Tuple`] or [`Value::Unit`] depending on `kind`.
/// Fields marked with [`FieldInfo::is_entity`] give entity references instead of integers.
fn fields_to_value(fields: &(impl ReflectFields + ?Sized), kind: Option<&VariantKind>) -> Value {
    let infos = kind.map(VariantKind::fields).unwrap_or_default();
    let values = (0..fields.field_count()).filter_map(|i| fields.field_at(i).map(|field| (i, field))).map(|(i, field)| {
        let value = field.to_value();
        if infos.get(i).is_some_and(FieldInfo::is_entity) { ids_to_entities(value) } else { value }
    });
    match kind {
        Some(VariantKind::Struct(infos)) => Value::Struct(infos.iter().map(|info| info.name.to_string()).zip(values).collect()),
        Some(VariantKind::Tuple(_)) => Value::Tuple(values.collect()),
        _ => Value::Unit
    }
}

/// Applies a struct, tuple or unit `value` to `fields`, field by field.
fn apply_fields(fields: &mut (impl ReflectFields + ?Sized), kind: Option<&VariantKind>, value: &Value) -> anyhow::Result<()> {
    let kind = kind.context("not a struct")?;
    let mut apply = |name: &str, value: &Value| -> anyhow::Result<()> {
        let field = kind.field_index(name).and_then(|i| fields.field_at_mut(i)).with_context(|| format!("no field `{name}`"))?;
        field.apply(value).with_context(|| format!("in field `{name}`"))
    };
    match value.unannotated() {
        Value::Unit => Ok(()),
        Value::Struct(values) => values.iter().try_for_each(|(name, value)| apply(name, value)),
        Value::Tuple(values) => values.iter().enumerate().try_for_each(|(i, value)| apply(&i.to_string(), value)),
        other => bail!("expected fields, found {other}")
    }
}

/// The fields of a [`Value`] being built into a struct or enum variant, see [`FromValue`].
/// Used by `#[derive(Reflect)]`.
#[doc(hidden)]
pub struct FieldValues<'a>(FieldSource<'a>);

enum FieldSource<'a> { None, Named(&'a [(String, Value)]), Positional(&'a [Value]) }

impl <'a> FieldValues<'a> {
    /// The fields in `value`, which must name only fields in `known`.
    pub fn new(value: &'a Value, known: &[&str]) -> anyhow::Result<Self> {
        let source = match value.unannotated() {
            Value::Unit => FieldSource::None,
            Value::Struct(values) => {
                if let Some((name, _)) = values.iter().find(|(name, _)| !known.contains(&name.as_str()) && name.parse().ok().is_none_or(|i: usize| i >= known.len())) {
                    bail!("no field `{name}`")
                }
                FieldSource::Named(values)
            }
            Value::Tuple(values) if values.len() <= known.len() => FieldSource::Positional(values),
            Value::Tuple(values) => bail!("expected at most {} fields, found {}", known.len(), values.len()),
            other => bail!("expected fields, found {other}")
        };
        Ok(Self(source))
    }

    /// Build the field `name` at `index`, from its fallback if it is missing.
    pub fn take<T: FromValue>(&self, name: &str, index: usize) -> anyhow::Result<T> {
        let value = match self.0 {
            FieldSource::None => None,
            FieldSource::Named(values) => values.iter().rev()
                .find(|(key, _)| key == name || key.parse() == Ok(index))
                .map(|(_, value)| value),
            FieldSource::Positional(values) => values.get(index)
        };
        match value {
            Some(value) => T::from_value(value).with_context(|| format!("in field `{name}`")),
            None => T::fallback().with_context(|| format!("missing field `{name}`"))
        }
    }
}

/// Re-exports for code generated by `#[derive(Reflect)]` and `scene!`.
#[doc(hidden)]
pub mod __private {
    pub use anyhow;
    pub use inventory;

    pub use crate::reflect::registry::{AutoRegistration, ProbeComponent, ProbeNotComponent, ProbeNotResource, ProbeResource, RegistrationProbe};
}

#[cfg(test)]
mod tests {
    use crate::*;

    /// A point.
    #[derive(Debug, Clone, PartialEq, Reflect)]
    struct Point {
        /// Across.
        x: f32,
        #[reflect(hint(min = 0, max = 10))]
        y: f32
    }

    #[derive(Debug, Clone, PartialEq, Reflect)]
    struct Pair(u32, String);

    #[derive(Debug, Clone, PartialEq, Reflect)]
    struct Marker;

    #[derive(Debug, Clone, PartialEq, Reflect)]
    #[reflect(default)]
    struct Defaults { a: u32, b: u32 }

    impl Default for Defaults {
        fn default() -> Self { Self { a: 1, b: 2 } }
    }

    #[derive(Debug, Clone, PartialEq, Reflect)]
    struct Skips { kept: u32, #[reflect(skip)] skipped: u32 }

    #[derive(Debug, Clone, PartialEq, Reflect)]
    enum Shape {
        Empty,
        Circle(f32),
        Rect { w: f32, h: f32 }
    }

    #[derive(Debug, Clone, PartialEq, Reflect)]
    struct Wrapper<T> { inner: T, items: Vec<T> }

    #[derive(Debug, Clone, PartialEq, Reflect)]
    struct Links { #[reflect(entity)] target: EntityID, #[reflect(entity)] others: Vec<EntityID>, count: u64 }

    #[derive(Debug, Clone, PartialEq, Reflect)]
    struct Nested { point: Point, shape: Shape, maybe: Option<Pair>, list: Vec<Point>, array: [u8; 2], tuple: (u8, bool) }

    fn nested() -> Nested {
        Nested {
            point: Point { x: 1.0, y: 2.0 },
            shape: Shape::Rect { w: 3.0, h: 4.0 },
            maybe: Some(Pair(5, "five".into())),
            list: vec![Point { x: 6.0, y: 7.0 }],
            array: [8, 9],
            tuple: (10, true)
        }
    }

    #[test]
    fn to_value_and_back() {
        let value = nested().to_value();
        assert_eq!(Nested::from_value(&value).unwrap(), nested());
        assert_eq!(Point { x: 1.0, y: 2.0 }.to_value(), Value::Struct(vec![("x".into(), Value::Float(1.0)), ("y".into(), Value::Float(2.0))]));
        assert_eq!(Pair(1, "a".into()).to_value(), Value::Tuple(vec![Value::Int(1), Value::Str("a".into())]));
        assert_eq!(Marker.to_value(), Value::Unit);
        assert_eq!(Shape::Circle(1.0).to_value(), Value::Enum { variant: "Circle".into(), value: Box::new(Value::Tuple(vec![Value::Float(1.0)])) });
    }

    #[test]
    fn apply_patches_only_given_fields() {
        let mut point = Point { x: 1.0, y: 2.0 };
        point.apply(&Value::Struct(vec![("y".into(), Value::Int(5))])).unwrap();
        assert_eq!(point, Point { x: 1.0, y: 5.0 });

        let mut pair = Pair(1, "a".into());
        pair.apply(&Value::Tuple(vec![Value::Int(2)])).unwrap();
        assert_eq!(pair, Pair(2, "a".into()));
    }

    #[test]
    fn apply_rejects_bad_values() {
        let mut point = Point { x: 1.0, y: 2.0 };
        assert!(point.apply(&Value::Struct(vec![("z".into(), Value::Int(5))])).is_err());
        assert!(point.apply(&Value::Struct(vec![("x".into(), Value::Str("no".into()))])).is_err());
        assert!(u8::from_value(&Value::Int(256)).is_err());
    }

    #[test]
    fn missing_fields_fall_back() {
        assert_eq!(Point::from_value(&Value::Unit).unwrap(), Point { x: 0.0, y: 0.0 });
        assert_eq!(Defaults::from_value(&Value::Struct(vec![("b".into(), Value::Int(5))])).unwrap(), Defaults { a: 1, b: 5 });
        assert_eq!(Defaults::fallback(), Some(Defaults::default()));
        assert_eq!(Skips::from_value(&Value::Struct(vec![("kept".into(), Value::Int(3))])).unwrap(), Skips { kept: 3, skipped: 0 });
        assert!(Skips::from_value(&Value::Struct(vec![("skipped".into(), Value::Int(3))])).is_err());
    }

    #[test]
    fn enums_switch_variants() {
        let mut shape = Shape::Circle(1.0);
        shape.apply(&Value::Enum { variant: "Circle".into(), value: Box::new(Value::Tuple(vec![Value::Float(2.0)])) }).unwrap();
        assert_eq!(shape, Shape::Circle(2.0));
        shape.apply(&Value::Enum { variant: "Rect".into(), value: Box::new(Value::Struct(vec![("w".into(), Value::Float(3.0))])) }).unwrap();
        assert_eq!(shape, Shape::Rect { w: 3.0, h: 0.0 });
        shape.apply(&Value::Enum { variant: "Empty".into(), value: Box::new(Value::Unit) }).unwrap();
        assert_eq!(shape, Shape::Empty);
        assert!(shape.apply(&Value::Enum { variant: "Nope".into(), value: Box::new(Value::Unit) }).is_err());
        assert_eq!(Shape::fallback(), Some(Shape::Empty));
    }

    #[test]
    fn options_accept_bare_values() {
        assert_eq!(Option::<u32>::from_value(&Value::Int(3)).unwrap(), Some(3));
        assert_eq!(Option::<u32>::from_value(&Value::Enum { variant: "None".into(), value: Box::new(Value::Unit) }).unwrap(), None);
        assert_eq!(Option::<u32>::fallback(), Some(None));
    }

    #[test]
    fn generics_get_their_own_info() {
        assert_eq!(Wrapper::<u8>::type_info().short_name, "Wrapper<u8>");
        assert_eq!(Wrapper::<f32>::type_info().short_name, "Wrapper<f32>");
        let wrapper = Wrapper { inner: 1u8, items: vec![2, 3] };
        assert_eq!(Wrapper::<u8>::from_value(&wrapper.to_value()).unwrap(), wrapper);
    }

    #[test]
    fn type_info_describes_fields() {
        let info = Point::type_info();
        assert_eq!(info.short_name, "Point");
        assert_eq!(info.docs, "A point.");
        let TypeKind::Struct(VariantKind::Struct(fields)) = &info.kind else { panic!("not a struct") };
        assert_eq!(fields[0].name, "x");
        assert_eq!(fields[0].docs, "Across.");
        assert_eq!((fields[0].ty)().type_name, "f32");
        assert_eq!(fields[1].hints, vec![("min", Value::Int(0)), ("max", Value::Int(10))]);
        let TypeKind::Enum(variants) = &Shape::type_info().kind else { panic!("not an enum") };
        assert_eq!(variants.iter().map(|v| v.name).collect::<Vec<_>>(), ["Empty", "Circle", "Rect"]);
    }

    #[test]
    fn entity_fields_give_entity_refs() {
        let links = Links { target: 1, others: vec![2], count: 3 };
        let value = links.to_value();
        assert_eq!(value, Value::Struct(vec![
            ("target".into(), Value::Entity(EntityRef::Id(1))),
            ("others".into(), Value::List(vec![Value::Entity(EntityRef::Id(2))])),
            ("count".into(), Value::Int(3))
        ]));
        assert_eq!(Links::from_value(&value).unwrap(), links);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn type_info_serializes() {
        let json = serde_json::to_value(Point::type_info()).unwrap();
        assert_eq!(json["short_name"], "Point");
        assert_eq!(json["kind"]["Struct"]["Struct"][0]["ty"], "f32");
        assert_eq!(json["kind"]["Struct"]["Struct"][1]["hints"][0][0], "min");
    }

    #[test]
    fn short_names_strip_paths() {
        assert_eq!(short_type_name("alloc::vec::Vec<core::option::Option<my::Foo>>"), "Vec<Option<Foo>>");
        assert_eq!(short_type_name("(u8, a::B)"), "(u8, B)");
        assert_eq!(short_type_name("[a::B; 3]"), "[B; 3]");
    }
}
