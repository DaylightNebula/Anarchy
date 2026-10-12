//! [`Reflect`], [`Typed`] and [`FromValue`] for std types: numbers, `bool`, `char`,
//! `String`, `Option`, `Vec`, arrays, tuples and `Box`.

use std::any::{Any, type_name};

use anyhow::{Context, bail};

use crate::*;

/// Implements the any casts of [`Reflect`], which are the same for every type.
macro_rules! any_casts {
    () => {
        fn reflect_any(&self) -> &dyn Any { self }
        fn reflect_any_mut(&mut self) -> &mut dyn Any { self }
        fn into_any(self: Box<Self>) -> Box<dyn Any> { self }
    };
}

/// Implements [`Reflect`] and [`Typed`] for a leaf type with a [`FromValue`] impl.
macro_rules! impl_reflect_value {
    ($($ty:ty => $to_value:expr),* $(,)?) => {$(
        impl Typed for $ty {
            fn type_info() -> &'static TypeInfo {
                static INFO: TypeInfoCell = TypeInfoCell::new();
                INFO.get_or_init::<Self>(|| TypeInfo { type_name: type_name::<Self>(), short_name: short_type_name(type_name::<Self>()), docs: "", kind: TypeKind::Value })
            }
        }

        impl Reflect for $ty {
            fn reflect_info(&self) -> &'static TypeInfo { <Self as Typed>::type_info() }
            fn reflect_ref(&self) -> ReflectRef<'_> { ReflectRef::Value(self) }
            fn reflect_mut(&mut self) -> ReflectMut<'_> { ReflectMut::Value(self) }
            any_casts!();
            fn to_value(&self) -> Value { let f: fn(&Self) -> Value = $to_value; f(self) }
            fn apply(&mut self, value: &Value) -> anyhow::Result<()> { *self = Self::from_value(value)?; Ok(()) }
        }
    )*};
}

macro_rules! impl_int {
    ($($ty:ty),*) => {
        $(impl FromValue for $ty {
            fn from_value(value: &Value) -> anyhow::Result<Self> {
                match value {
                    Value::Int(int) => <$ty>::try_from(*int).with_context(|| format!("{int} is out of range for {}", type_name::<Self>())),
                    other => bail!("expected an integer, found {other}")
                }
            }
            fn fallback() -> Option<Self> { Some(0) }
        })*
        impl_reflect_value!($($ty => |v| Value::Int(*v as i128)),*);
    };
}
impl_int!(i8, i16, i32, i64, isize, u8, u16, i128, usize, u32);

// u64 is also `EntityID`, so it reads entity references by id
impl FromValue for u64 {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Int(int) => u64::try_from(*int).with_context(|| format!("{int} is out of range for u64")),
            Value::Entity(EntityRef::Id(id)) => Ok(*id),
            Value::Entity(EntityRef::Named(name)) => bail!("entity #{name} was not resolved to an id"),
            other => bail!("expected an integer, found {other}")
        }
    }
    fn fallback() -> Option<Self> { Some(0) }
}
impl_reflect_value!(u64 => |v| Value::Int(*v as i128));

macro_rules! impl_float {
    ($($ty:ty),*) => {
        $(impl FromValue for $ty {
            fn from_value(value: &Value) -> anyhow::Result<Self> {
                match value {
                    Value::Float(float) => Ok(*float as $ty),
                    Value::Int(int) => Ok(*int as $ty),
                    other => bail!("expected a number, found {other}")
                }
            }
            fn fallback() -> Option<Self> { Some(0.0) }
        })*
        impl_reflect_value!($($ty => |v| Value::Float(*v as f64)),*);
    };
}
impl_float!(f32, f64);

impl FromValue for bool {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value { Value::Bool(value) => Ok(*value), other => bail!("expected a bool, found {other}") }
    }
    fn fallback() -> Option<Self> { Some(false) }
}

impl FromValue for String {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value { Value::Str(value) => Ok(value.clone()), other => bail!("expected a string, found {other}") }
    }
    fn fallback() -> Option<Self> { Some(String::new()) }
}

impl FromValue for char {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        let mut chars = match value { Value::Str(value) => value.chars(), other => bail!("expected a char, found {other}") };
        match (chars.next(), chars.next()) {
            (Some(c), None) => Ok(c),
            _ => bail!("expected a single char, found {value}")
        }
    }
}

impl_reflect_value!(
    bool => |v| Value::Bool(*v),
    String => |v| Value::Str(v.clone()),
    char => |v| Value::Str(v.to_string()),
);

/// Info for a list of `T`, with a fixed `len` for arrays.
fn list_info<L: 'static, T: Typed>(cell: &'static TypeInfoCell, len: Option<usize>) -> &'static TypeInfo {
    cell.get_or_init::<L>(|| TypeInfo {
        type_name: type_name::<L>(),
        short_name: short_type_name(type_name::<L>()),
        docs: "",
        kind: TypeKind::List { item: T::type_info, len }
    })
}

/// Builds every item of a [`Value::List`].
fn items_from_value<T: FromValue>(value: &Value) -> anyhow::Result<Vec<T>> {
    match value {
        Value::List(items) => items.iter().enumerate()
            .map(|(i, item)| T::from_value(item).with_context(|| format!("in item {i}")))
            .collect(),
        other => bail!("expected a list, found {other}")
    }
}

impl <T: Reflect + Typed + FromValue> Typed for Vec<T> {
    fn type_info() -> &'static TypeInfo {
        static INFO: TypeInfoCell = TypeInfoCell::new();
        list_info::<Self, T>(&INFO, None)
    }
}

impl <T: Reflect + Typed + FromValue> Reflect for Vec<T> {
    fn reflect_info(&self) -> &'static TypeInfo { <Self as Typed>::type_info() }
    fn reflect_ref(&self) -> ReflectRef<'_> { ReflectRef::List(self) }
    fn reflect_mut(&mut self) -> ReflectMut<'_> { ReflectMut::List(self) }
    any_casts!();
    /// Replaces every item, lists are not patched item by item.
    fn apply(&mut self, value: &Value) -> anyhow::Result<()> { *self = Self::from_value(value)?; Ok(()) }
}

impl <T: Reflect + Typed + FromValue> ReflectList for Vec<T> {
    fn len(&self) -> usize { Vec::len(self) }
    fn get(&self, index: usize) -> Option<&dyn Reflect> { <[T]>::get(self, index).map(|item| item as &dyn Reflect) }
    fn get_mut(&mut self, index: usize) -> Option<&mut dyn Reflect> { <[T]>::get_mut(self, index).map(|item| item as &mut dyn Reflect) }
    fn push_value(&mut self, value: &Value) -> anyhow::Result<()> { self.push(T::from_value(value)?); Ok(()) }
    fn remove_at(&mut self, index: usize) -> anyhow::Result<()> {
        if index >= Vec::len(self) { bail!("index {index} is out of range for a list of {}", Vec::len(self)) }
        self.remove(index);
        Ok(())
    }
}

impl <T: FromValue> FromValue for Vec<T> {
    fn from_value(value: &Value) -> anyhow::Result<Self> { items_from_value(value) }
    fn fallback() -> Option<Self> { Some(Vec::new()) }
}

impl <T: Reflect + Typed + FromValue, const N: usize> Typed for [T; N] {
    fn type_info() -> &'static TypeInfo {
        static INFO: TypeInfoCell = TypeInfoCell::new();
        list_info::<Self, T>(&INFO, Some(N))
    }
}

impl <T: Reflect + Typed + FromValue, const N: usize> Reflect for [T; N] {
    fn reflect_info(&self) -> &'static TypeInfo { <Self as Typed>::type_info() }
    fn reflect_ref(&self) -> ReflectRef<'_> { ReflectRef::List(self) }
    fn reflect_mut(&mut self) -> ReflectMut<'_> { ReflectMut::List(self) }
    any_casts!();
    /// Applies item by item, so a shorter list only changes the first items.
    fn apply(&mut self, value: &Value) -> anyhow::Result<()> {
        let Value::List(items) = value else { bail!("expected a list, found {value}") };
        if items.len() > N { bail!("expected at most {N} items, found {}", items.len()) }
        items.iter().zip(self.iter_mut()).enumerate()
            .try_for_each(|(i, (value, item))| item.apply(value).with_context(|| format!("in item {i}")))
    }
}

impl <T: Reflect + Typed + FromValue, const N: usize> ReflectList for [T; N] {
    fn len(&self) -> usize { N }
    fn get(&self, index: usize) -> Option<&dyn Reflect> { <[T]>::get(self, index).map(|item| item as &dyn Reflect) }
    fn get_mut(&mut self, index: usize) -> Option<&mut dyn Reflect> { <[T]>::get_mut(self, index).map(|item| item as &mut dyn Reflect) }
    fn push_value(&mut self, _value: &Value) -> anyhow::Result<()> { bail!("can't push to an array of fixed length {N}") }
    fn remove_at(&mut self, _index: usize) -> anyhow::Result<()> { bail!("can't remove from an array of fixed length {N}") }
}

impl <T: FromValue, const N: usize> FromValue for [T; N] {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        let items = items_from_value::<T>(value)?;
        let len = items.len();
        items.try_into().map_err(|_| anyhow::anyhow!("expected {N} items, found {len}"))
    }
    fn fallback() -> Option<Self> {
        let items = (0..N).map(|_| T::fallback()).collect::<Option<Vec<T>>>()?;
        items.try_into().ok()
    }
}

impl <T: Reflect + Typed + FromValue> Typed for Option<T> {
    fn type_info() -> &'static TypeInfo {
        static INFO: TypeInfoCell = TypeInfoCell::new();
        INFO.get_or_init::<Self>(|| TypeInfo {
            type_name: type_name::<Self>(),
            short_name: short_type_name(type_name::<Self>()),
            docs: "",
            kind: TypeKind::Enum(vec![
                VariantInfo { name: "None", docs: "", kind: VariantKind::Unit },
                VariantInfo { name: "Some", docs: "", kind: VariantKind::Tuple(vec![FieldInfo { name: "0", ty: T::type_info, docs: "", hints: vec![] }]) }
            ])
        })
    }
}

impl <T: Reflect + Typed + FromValue> Reflect for Option<T> {
    fn reflect_info(&self) -> &'static TypeInfo { <Self as Typed>::type_info() }
    fn reflect_ref(&self) -> ReflectRef<'_> { ReflectRef::Enum(self) }
    fn reflect_mut(&mut self) -> ReflectMut<'_> { ReflectMut::Enum(self) }
    any_casts!();
    /// Accepts `None`, `Some(value)`, or a bare value which is patched into the current
    /// `Some` or becomes a new one.
    fn apply(&mut self, value: &Value) -> anyhow::Result<()> {
        match (&mut *self, value) {
            (_, Value::Enum { variant, .. }) if variant == "None" => *self = None,
            (Some(inner), Value::Enum { variant, value }) if variant == "Some" => inner.apply(single_field(value)?)?,
            (Some(inner), value) => inner.apply(value)?,
            (None, value) => *self = Self::from_value(value)?
        }
        Ok(())
    }
}

impl <T: Reflect + Typed + FromValue> ReflectFields for Option<T> {
    fn field_count(&self) -> usize { self.is_some() as usize }
    fn field_at(&self, index: usize) -> Option<&dyn Reflect> { self.as_ref().filter(|_| index == 0).map(|inner| inner as &dyn Reflect) }
    fn field_at_mut(&mut self, index: usize) -> Option<&mut dyn Reflect> { self.as_mut().filter(|_| index == 0).map(|inner| inner as &mut dyn Reflect) }
}

impl <T: Reflect + Typed + FromValue> ReflectEnum for Option<T> {
    fn variant_index(&self) -> usize { self.is_some() as usize }
    fn set_variant(&mut self, variant: &str, value: &Value) -> anyhow::Result<()> {
        *self = Self::from_value(&Value::Enum { variant: variant.into(), value: Box::new(value.clone()) })?;
        Ok(())
    }
}

impl <T: FromValue> FromValue for Option<T> {
    /// Accepts `None`, `Some(value)` or a bare value, which becomes `Some`.
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Enum { variant, .. } if variant == "None" => Ok(None),
            Value::Enum { variant, value } if variant == "Some" => match &**value {
                Value::Unit => T::fallback().context("missing value for Some").map(Some),
                value => T::from_value(single_field(value)?).map(Some)
            },
            value => T::from_value(value).map(Some)
        }
    }
    fn fallback() -> Option<Self> { Some(None) }
}

/// The only field of a one field tuple variant.
fn single_field(value: &Value) -> anyhow::Result<&Value> {
    match value {
        Value::Tuple(items) if items.len() == 1 => Ok(&items[0]),
        Value::Struct(fields) if fields.len() == 1 && fields[0].0 == "0" => Ok(&fields[0].1),
        other => bail!("expected one field, found {other}")
    }
}

impl <T: Reflect + Typed + FromValue> Typed for Box<T> {
    fn type_info() -> &'static TypeInfo { T::type_info() }
}

impl <T: Reflect + Typed + FromValue> Reflect for Box<T> {
    fn reflect_info(&self) -> &'static TypeInfo { (**self).reflect_info() }
    fn reflect_ref(&self) -> ReflectRef<'_> { (**self).reflect_ref() }
    fn reflect_mut(&mut self) -> ReflectMut<'_> { (**self).reflect_mut() }
    fn reflect_any(&self) -> &dyn Any { (**self).reflect_any() }
    fn reflect_any_mut(&mut self) -> &mut dyn Any { (**self).reflect_any_mut() }
    fn into_any(self: Box<Self>) -> Box<dyn Any> { (*self).into_any() }
    fn to_value(&self) -> Value { (**self).to_value() }
    fn apply(&mut self, value: &Value) -> anyhow::Result<()> { (**self).apply(value) }
}

impl <T: FromValue> FromValue for Box<T> {
    fn from_value(value: &Value) -> anyhow::Result<Self> { T::from_value(value).map(Box::new) }
    fn fallback() -> Option<Self> { T::fallback().map(Box::new) }
}

macro_rules! impl_tuple {
    ($($name:ident $index:tt),+) => {
        impl <$($name: Reflect + Typed + FromValue),+> Typed for ($($name,)+) {
            fn type_info() -> &'static TypeInfo {
                static INFO: TypeInfoCell = TypeInfoCell::new();
                INFO.get_or_init::<Self>(|| TypeInfo {
                    type_name: type_name::<Self>(),
                    short_name: short_type_name(type_name::<Self>()),
                    docs: "",
                    kind: TypeKind::Struct(VariantKind::Tuple(vec![
                        $(FieldInfo { name: stringify!($index), ty: $name::type_info, docs: "", hints: vec![] }),+
                    ]))
                })
            }
        }

        impl <$($name: Reflect + Typed + FromValue),+> Reflect for ($($name,)+) {
            fn reflect_info(&self) -> &'static TypeInfo { <Self as Typed>::type_info() }
            fn reflect_ref(&self) -> ReflectRef<'_> { ReflectRef::Struct(self) }
            fn reflect_mut(&mut self) -> ReflectMut<'_> { ReflectMut::Struct(self) }
            any_casts!();
        }

        impl <$($name: Reflect + Typed + FromValue),+> ReflectFields for ($($name,)+) {
            fn field_count(&self) -> usize { [$($index),+].len() }
            fn field_at(&self, index: usize) -> Option<&dyn Reflect> {
                match index { $($index => Some(&self.$index),)+ _ => None }
            }
            fn field_at_mut(&mut self, index: usize) -> Option<&mut dyn Reflect> {
                match index { $($index => Some(&mut self.$index),)+ _ => None }
            }
        }

        impl <$($name: FromValue),+> FromValue for ($($name,)+) {
            fn from_value(value: &Value) -> anyhow::Result<Self> {
                let names = [$(stringify!($index)),+];
                let fields = FieldValues::new(value, &names)?;
                Ok(($(fields.take::<$name>(stringify!($index), $index)?,)+))
            }
            fn fallback() -> Option<Self> { Some(($($name::fallback()?,)+)) }
        }
    };
}

impl_tuple!(A 0);
impl_tuple!(A 0, B 1);
impl_tuple!(A 0, B 1, C 2);
impl_tuple!(A 0, B 1, C 2, D 3);
impl_tuple!(A 0, B 1, C 2, D 3, E 4);
impl_tuple!(A 0, B 1, C 2, D 3, E 4, F 5);
impl_tuple!(A 0, B 1, C 2, D 3, E 4, F 5, G 6);
impl_tuple!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7);
impl_tuple!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8);
impl_tuple!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9);
impl_tuple!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10);
impl_tuple!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10, L 11);

#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn numbers_convert_in_range() {
        assert_eq!(f32::from_value(&Value::Int(2)).unwrap(), 2.0);
        assert!(i32::from_value(&Value::Float(2.0)).is_err());
        assert!(i8::from_value(&Value::Int(-129)).is_err());
        assert_eq!(u64::from_value(&Value::Entity(EntityRef::Id(4))).unwrap(), 4);
        assert_eq!(u64::MAX.to_value(), Value::Int(u64::MAX as i128));
    }

    #[test]
    fn lists_push_and_remove() {
        let mut list = vec![1u8, 2];
        let ReflectMut::List(items) = list.reflect_mut() else { panic!("not a list") };
        items.push_value(&Value::Int(3)).unwrap();
        items.remove_at(0).unwrap();
        assert!(items.remove_at(5).is_err());
        assert_eq!(list, [2, 3]);

        let mut array = [1u8, 2];
        array.apply(&Value::List(vec![Value::Int(5)])).unwrap();
        assert_eq!(array, [5, 2]);
        assert!(<[u8; 2]>::from_value(&Value::List(vec![Value::Int(5)])).is_err());
    }

    #[test]
    fn tuples_are_positional() {
        let mut tuple = (1u8, String::from("a"));
        assert_eq!(tuple.to_value(), Value::Tuple(vec![Value::Int(1), Value::Str("a".into())]));
        tuple.apply(&Value::Struct(vec![("1".into(), Value::Str("b".into()))])).unwrap();
        assert_eq!(tuple, (1, "b".into()));
    }
}

#[cfg(feature = "glam")]
mod glam;
