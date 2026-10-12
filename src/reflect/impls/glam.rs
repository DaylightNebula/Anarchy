//! [`Reflect`], [`Typed`] and [`FromValue`] for glam's vectors and quaternion, with the
//! `glam` feature.  They reflect as structs of their components, e.g. `{ x: 1.0, y: 2.0 }`.

use std::any::{Any, type_name};

use ::glam::{IVec2, IVec3, Quat, UVec2, UVec3, Vec2, Vec3, Vec4};

use crate::*;

/// Implements reflection for a glam type with public scalar fields.  Fields left out
/// when building one come from `$fallback`.
macro_rules! impl_glam {
    ($($ty:ty : $scalar:ty = $fallback:expr => { $($field:ident),+ }),* $(,)?) => {$(
        impl Typed for $ty {
            fn type_info() -> &'static TypeInfo {
                static INFO: TypeInfoCell = TypeInfoCell::new();
                INFO.get_or_init::<Self>(|| TypeInfo {
                    type_name: type_name::<Self>(),
                    short_name: short_type_name(type_name::<Self>()),
                    docs: "",
                    kind: TypeKind::Struct(VariantKind::Struct(vec![$(FieldInfo {
                        name: stringify!($field),
                        ty: <$scalar as Typed>::type_info,
                        docs: "",
                        hints: Vec::new()
                    }),+]))
                })
            }
        }

        impl Reflect for $ty {
            fn reflect_info(&self) -> &'static TypeInfo { <Self as Typed>::type_info() }
            fn reflect_ref(&self) -> ReflectRef<'_> { ReflectRef::Struct(self) }
            fn reflect_mut(&mut self) -> ReflectMut<'_> { ReflectMut::Struct(self) }
            fn reflect_any(&self) -> &dyn Any { self }
            fn reflect_any_mut(&mut self) -> &mut dyn Any { self }
            fn into_any(self: Box<Self>) -> Box<dyn Any> { self }
        }

        impl ReflectFields for $ty {
            fn field_count(&self) -> usize { [$(stringify!($field)),+].len() }
            fn field_at(&self, index: usize) -> Option<&dyn Reflect> {
                let name = *[$(stringify!($field)),+].get(index)?;
                $(if name == stringify!($field) { return Some(&self.$field) })+
                None
            }
            // one field at a time, since SIMD types reach their fields through `DerefMut`
            fn field_at_mut(&mut self, index: usize) -> Option<&mut dyn Reflect> {
                let name = *[$(stringify!($field)),+].get(index)?;
                $(if name == stringify!($field) { return Some(&mut self.$field) })+
                None
            }
        }

        impl FromValue for $ty {
            fn from_value(value: &Value) -> anyhow::Result<Self> {
                let names = [$(stringify!($field)),+];
                let fields = FieldValues::new(value, &names)?;
                let mut built: $ty = $fallback;
                for (index, name) in names.into_iter().enumerate() {
                    if let Some(scalar) = fields.take::<Option<$scalar>>(name, index)? {
                        *built.field_at_mut(index).unwrap().reflect_any_mut().downcast_mut::<$scalar>().unwrap() = scalar;
                    }
                }
                Ok(built)
            }
            fn fallback() -> Option<Self> { Some($fallback) }
        }
    )*};
}

impl_glam!(
    Vec2: f32 = Vec2::ZERO => { x, y },
    Vec3: f32 = Vec3::ZERO => { x, y, z },
    Vec4: f32 = Vec4::ZERO => { x, y, z, w },
    Quat: f32 = Quat::IDENTITY => { x, y, z, w },
    IVec2: i32 = IVec2::ZERO => { x, y },
    IVec3: i32 = IVec3::ZERO => { x, y, z },
    UVec2: u32 = UVec2::ZERO => { x, y },
    UVec3: u32 = UVec3::ZERO => { x, y, z },
);

#[cfg(test)]
mod tests {
    use ::glam::{Quat, Vec3};

    use crate::*;

    #[test]
    fn glam_round_trips_and_paths() {
        let mut v = Vec3::new(1.0, 2.0, 3.0);
        assert_eq!(v.to_value().to_string(), "{ x: 1.0, y: 2.0, z: 3.0 }");
        assert_eq!(Vec3::from_value(&v.to_value()).unwrap(), v);
        set_path(&mut v, "y", &Value::Float(5.0)).unwrap();
        assert_eq!(v, Vec3::new(1.0, 5.0, 3.0));
        // missing fields come from the type's fallback, identity for a quaternion
        assert_eq!(Quat::from_value(&Value::Unit).unwrap(), Quat::IDENTITY);
        assert_eq!(Quat::from_value(&Value::Struct(vec![("w".into(), Value::Float(0.5))])).unwrap(), Quat::from_xyzw(0.0, 0.0, 0.0, 0.5));
    }
}
