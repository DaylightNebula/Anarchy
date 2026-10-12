//! [`Value`], untyped data read from and written to [`Reflect`](crate::Reflect) values.

use std::fmt::{self, Display, Write};

use crate::EntityID;

/// An untyped tree of data.  Scenes, patches and editor edits are all values, given
/// a type by the [`Reflect`](crate::Reflect) value they are applied to.
///
/// Displays in the scene format, e.g. `{ x: 1.0, tags: ["a"], target: #Enemy }`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Value {
    /// No data, also an empty patch that changes nothing.
    Unit,
    /// A boolean.
    Bool(bool),
    /// Any integer, wide enough for every `i64` and `u64`.
    #[cfg_attr(feature = "serde", serde(with = "int_serde"))]
    Int(i128),
    /// A float.
    Float(f64),
    /// A string.
    Str(String),
    /// A list of items.
    List(Vec<Value>),
    /// Fields by position, for tuples and tuple structs.
    Tuple(Vec<Value>),
    /// Fields by name.  As a patch, the fields left out are unchanged.
    Struct(Vec<(String, Value)>),
    /// An enum variant and its fields, which are [`Value::Unit`], [`Value::Tuple`] or
    /// [`Value::Struct`].  Also a struct value annotated with its type name, such as
    /// `Vec3 { x: 1.0 }` in a scene, which structs accept as the fields alone.
    Enum {
        /// The variant, or the annotated type's name.
        variant: String,
        /// The variant's fields.
        value: Box<Value>
    },
    /// A reference to an entity, resolved to an id when a scene is spawned.
    Entity(EntityRef)
}

/// An entity referenced from a [`Value`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum EntityRef {
    /// An entity id.
    Id(EntityID),
    /// The entity named with `#name` in the same scene.
    Named(String)
}

impl Value {
    /// The fields of a type annotated value, `Vec3 { x: 1.0 }` gives `{ x: 1.0 }`.
    /// Other values are returned as they are.
    pub fn unannotated(&self) -> &Value {
        match self {
            Value::Enum { value, .. } if matches!(**value, Value::Unit | Value::Tuple(_) | Value::Struct(_)) => value,
            other => other
        }
    }

    /// The field `name` of a [`Value::Struct`], the last one if it is given twice.
    pub fn field(&self, name: &str) -> Option<&Value> {
        match self.unannotated() {
            Value::Struct(fields) => fields.iter().rev().find(|(key, _)| key == name).map(|(_, value)| value),
            _ => None
        }
    }

    /// Merges `patch` into this value, as if both were applied in turn.  Struct fields
    /// merge recursively, anything else in `patch` replaces this value.
    pub fn merge(&mut self, patch: &Value) {
        match (&mut *self, patch) {
            (_, Value::Unit) => {}
            (Value::Struct(fields), Value::Struct(patch_fields)) => for (name, patch) in patch_fields {
                match fields.iter_mut().find(|(key, _)| key == name) {
                    Some((_, value)) => value.merge(patch),
                    None => fields.push((name.clone(), patch.clone()))
                }
            },
            (Value::Unit, patch) => *self = patch.clone(),
            (Value::Enum { variant, value }, Value::Enum { variant: patch_variant, value: patch_value }) if variant == patch_variant => value.merge(patch_value),
            (_, patch) => *self = patch.clone()
        }
    }

    /// The parts of this value that differ from `base`, as a patch that turns `base`
    /// into this value.  Struct fields are compared one by one, anything else is kept
    /// whole if it differs.  Returns [`Value::Unit`] if they are equal.
    pub fn diff(&self, base: &Value) -> Value {
        if self == base { return Value::Unit }
        match (self, base) {
            (Value::Struct(fields), Value::Struct(_)) => Value::Struct(fields.iter()
                .filter_map(|(name, value)| match base.field(name) {
                    Some(base) if base == value => None,
                    Some(base) => Some((name.clone(), value.diff(base))),
                    None => Some((name.clone(), value.clone()))
                })
                .collect()),
            (Value::Enum { variant, value }, Value::Enum { variant: base_variant, value: base_value }) if variant == base_variant
                => Value::Enum { variant: variant.clone(), value: Box::new(value.diff(base_value)) },
            _ => self.clone()
        }
    }
}

macro_rules! from_int {
    ($($ty:ty),*) => { $(impl From<$ty> for Value { fn from(value: $ty) -> Self { Value::Int(value as i128) } })* };
}
from_int!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

impl From<f32> for Value { fn from(value: f32) -> Self { Value::Float(value as f64) } }
impl From<f64> for Value { fn from(value: f64) -> Self { Value::Float(value) } }
impl From<bool> for Value { fn from(value: bool) -> Self { Value::Bool(value) } }
impl From<&str> for Value { fn from(value: &str) -> Self { Value::Str(value.into()) } }
impl From<String> for Value { fn from(value: String) -> Self { Value::Str(value) } }

impl Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Unit => f.write_str("()"),
            Value::Bool(value) => write!(f, "{value}"),
            Value::Int(value) => write!(f, "{value}"),
            // Debug always keeps a `.0` or exponent, so it reads back as a float
            Value::Float(value) => write!(f, "{value:?}"),
            Value::Str(value) => write_str_literal(f, value),
            Value::List(items) => write_list(f, "[", items.iter(), "]"),
            Value::Tuple(items) => write_list(f, "(", items.iter(), ")"),
            Value::Struct(fields) => write_fields(f, fields),
            Value::Enum { variant, value } => {
                f.write_str(variant)?;
                match &**value {
                    Value::Unit => Ok(()),
                    Value::Struct(fields) => { f.write_char(' ')?; write_fields(f, fields) }
                    Value::Tuple(items) => write_list(f, "(", items.iter(), ")"),
                    other => write!(f, "({other})")
                }
            }
            Value::Entity(EntityRef::Id(id)) => write!(f, "#{id}"),
            Value::Entity(EntityRef::Named(name)) => { f.write_char('#')?; write_name(f, name) }
        }
    }
}

fn write_list<'a>(f: &mut fmt::Formatter<'_>, open: &str, items: impl Iterator<Item = &'a Value>, close: &str) -> fmt::Result {
    f.write_str(open)?;
    for (i, item) in items.enumerate() {
        if i > 0 { f.write_str(", ")? }
        write!(f, "{item}")?;
    }
    f.write_str(close)
}

fn write_fields(f: &mut fmt::Formatter<'_>, fields: &[(String, Value)]) -> fmt::Result {
    if fields.is_empty() { return f.write_str("{}") }
    f.write_str("{ ")?;
    for (i, (name, value)) in fields.iter().enumerate() {
        if i > 0 { f.write_str(", ")? }
        write_name(f, name)?;
        write!(f, ": {value}")?;
    }
    f.write_str(" }")
}

/// Returns true if `name` can be written without quotes, as an identifier or a number.
pub(crate) fn is_plain_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_digit() => name.chars().all(|c| c.is_ascii_digit()),
        Some(c) if c.is_alphabetic() || c == '_' => chars.all(|c| c.is_alphanumeric() || c == '_'),
        _ => false
    }
}

/// Writes `name` bare if it is plain, otherwise as a string literal.
pub(crate) fn write_name(f: &mut impl Write, name: &str) -> fmt::Result {
    if is_plain_name(name) { f.write_str(name) } else { write_str_literal(f, name) }
}

pub(crate) fn write_str_literal(f: &mut impl Write, value: &str) -> fmt::Result {
    f.write_char('"')?;
    for c in value.chars() {
        match c {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\r' => f.write_str("\\r")?,
            '\t' => f.write_str("\\t")?,
            c => f.write_char(c)?
        }
    }
    f.write_char('"')
}

/// [`Value::Int`] as an `i64` or `u64`, since serde can't buffer `i128`, which
/// internally tagged enums holding a [`Value`] need.
#[cfg(feature = "serde")]
mod int_serde {
    use serde::{Deserialize, Deserializer, Serializer, de::Error, ser};

    pub fn serialize<S: Serializer>(value: &i128, serializer: S) -> Result<S::Ok, S::Error> {
        if let Ok(value) = i64::try_from(*value) { return serializer.serialize_i64(value) }
        if let Ok(value) = u64::try_from(*value) { return serializer.serialize_u64(value) }
        Err(ser::Error::custom("integer out of range for i64 and u64"))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i128, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Int { Signed(i64), Unsigned(u64) }
        match Int::deserialize(deserializer).map_err(|_| D::Error::custom("expected an integer"))? {
            Int::Signed(value) => Ok(value as i128),
            Int::Unsigned(value) => Ok(value as i128)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(fields: &[(&str, Value)]) -> Value {
        Value::Struct(fields.iter().map(|(name, value)| (name.to_string(), value.clone())).collect())
    }

    #[test]
    fn merge_combines_struct_fields() {
        let mut value = s(&[("a", Value::Int(1)), ("b", s(&[("x", Value::Int(2))]))]);
        value.merge(&s(&[("b", s(&[("y", Value::Int(3))])), ("c", Value::Int(4))]));
        assert_eq!(value, s(&[("a", Value::Int(1)), ("b", s(&[("x", Value::Int(2)), ("y", Value::Int(3))])), ("c", Value::Int(4))]));
    }

    #[test]
    fn diff_keeps_changed_fields() {
        let base = s(&[("a", Value::Int(1)), ("b", s(&[("x", Value::Int(2)), ("y", Value::Int(3))]))]);
        let value = s(&[("a", Value::Int(1)), ("b", s(&[("x", Value::Int(2)), ("y", Value::Int(9))]))]);
        assert_eq!(value.diff(&base), s(&[("b", s(&[("y", Value::Int(9))]))]));
        assert_eq!(base.diff(&base), Value::Unit);
    }

    #[test]
    fn displays_in_scene_syntax() {
        let value = s(&[
            ("pos", Value::Tuple(vec![Value::Float(1.0), Value::Int(-2)])),
            ("name", Value::Str("a \"b\"".into())),
            ("shape", Value::Enum { variant: "Circle".into(), value: Box::new(s(&[("r", Value::Float(0.5))])) }),
            ("target", Value::Entity(EntityRef::Named("Big One".into()))),
            ("tags", Value::List(vec![Value::Bool(true)])),
            ("empty", s(&[]))
        ]);
        assert_eq!(value.to_string(), r#"{ pos: (1.0, -2), name: "a \"b\"", shape: Circle { r: 0.5 }, target: #"Big One", tags: [true], empty: {} }"#);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let value = s(&[
            ("big", Value::Int(u64::MAX as i128)),
            ("shape", Value::Enum { variant: "Circle".into(), value: Box::new(Value::Tuple(vec![Value::Float(0.5)])) }),
            ("target", Value::Entity(EntityRef::Named("Enemy".into())))
        ]);
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&json).unwrap(), value);
    }
}
