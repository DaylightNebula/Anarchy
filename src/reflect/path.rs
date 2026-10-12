//! Paths to nested fields of a [`Reflect`] value, such as `"transform.pos.x"`, `"0"` or `"items[2].name"`.

use anyhow::{Context, bail};

use crate::*;

/// One step of a path: a field by name, a field by position, or a list item.
#[derive(Debug, Clone, PartialEq)]
pub enum PathSegment<'a> {
    /// A named field, or a field of a tuple by position.
    Field(&'a str),
    /// A list item.
    Index(usize)
}

/// Splits `path` into segments.  Fields are separated by `.`, list items are `[i]`, and
/// an empty path is the value itself.
///
/// # Errors
///
/// Errors on an empty field name or a malformed index.
pub fn parse_path(path: &str) -> anyhow::Result<Vec<PathSegment<'_>>> {
    let mut segments = vec![];
    let mut rest = path;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('[') {
            let end = after.find(']').with_context(|| format!("unclosed `[` in path `{path}`"))?;
            let index = after[..end].trim().parse().with_context(|| format!("bad index `{}` in path `{path}`", &after[..end]))?;
            segments.push(PathSegment::Index(index));
            rest = &after[end + 1..];
        } else {
            let field = rest.strip_prefix('.').unwrap_or(rest);
            if field.len() == rest.len() && !segments.is_empty() { bail!("expected `.` or `[` in path `{path}`") }
            let end = field.find(['.', '[']).unwrap_or(field.len());
            if end == 0 { bail!("empty field name in path `{path}`") }
            segments.push(PathSegment::Field(&field[..end]));
            rest = &field[end..];
        }
    }
    Ok(segments)
}

/// The value at `path` inside `root`, see [`parse_path`].
///
/// # Errors
///
/// Errors if the path is malformed or doesn't exist in `root`.
pub fn path_ref<'r>(root: &'r dyn Reflect, path: &str) -> anyhow::Result<&'r dyn Reflect> {
    let mut current = root;
    for segment in parse_path(path)? {
        let info = current.reflect_info();
        current = match (current.reflect_ref(), &segment) {
            (ReflectRef::Struct(fields), PathSegment::Field(name)) => struct_field_index(info, name).and_then(|i| fields.field_at(i)),
            (ReflectRef::Enum(e), PathSegment::Field(name)) => variant_field_index(info, e.variant_index(), name).and_then(|i| e.field_at(i)),
            (ReflectRef::List(list), PathSegment::Index(i)) => list.get(*i),
            _ => None
        }.with_context(|| missing(info, &segment, path))?;
    }
    Ok(current)
}

/// The value at `path` inside `root`, see [`parse_path`].
///
/// # Errors
///
/// Errors if the path is malformed or doesn't exist in `root`.
pub fn path_mut<'r>(root: &'r mut dyn Reflect, path: &str) -> anyhow::Result<&'r mut dyn Reflect> {
    let mut current = root;
    for segment in parse_path(path)? {
        let info = current.reflect_info();
        current = match (current.reflect_mut(), &segment) {
            (ReflectMut::Struct(fields), PathSegment::Field(name)) => struct_field_index(info, name).and_then(|i| fields.field_at_mut(i)),
            (ReflectMut::Enum(e), PathSegment::Field(name)) => variant_field_index(info, e.variant_index(), name).and_then(|i| e.field_at_mut(i)),
            (ReflectMut::List(list), PathSegment::Index(i)) => list.get_mut(*i),
            _ => None
        }.with_context(|| missing(info, &segment, path))?;
    }
    Ok(current)
}

/// The value at `path` inside `root` as a [`Value`].
///
/// # Errors
///
/// Errors if the path is malformed or doesn't exist in `root`.
pub fn get_path(root: &dyn Reflect, path: &str) -> anyhow::Result<Value> {
    Ok(path_ref(root, path)?.to_value())
}

/// Applies `value` to the value at `path` inside `root`, see [`Reflect::apply`].
///
/// # Errors
///
/// Errors if the path is malformed, doesn't exist in `root`, or `value` doesn't fit.
pub fn set_path(root: &mut dyn Reflect, path: &str, value: &Value) -> anyhow::Result<()> {
    path_mut(root, path)?.apply(value).with_context(|| format!("setting `{path}`"))
}

fn struct_field_index(info: &TypeInfo, name: &str) -> Option<usize> {
    match &info.kind { TypeKind::Struct(kind) => kind.field_index(name), _ => None }
}

fn variant_field_index(info: &TypeInfo, variant: usize, name: &str) -> Option<usize> {
    match &info.kind { TypeKind::Enum(variants) => variants.get(variant)?.kind.field_index(name), _ => None }
}

fn missing(info: &TypeInfo, segment: &PathSegment, path: &str) -> String {
    match segment {
        PathSegment::Field(name) => format!("{} has no field `{name}` (in path `{path}`)", info.short_name),
        PathSegment::Index(i) => format!("{} has no item {i} (in path `{path}`)", info.short_name)
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[derive(Debug, PartialEq, Reflect)]
    struct Inner { x: f32, tags: Vec<String> }

    #[derive(Debug, PartialEq, Reflect)]
    struct Outer { inner: Inner, pair: (u8, u8), shape: Shape }

    #[derive(Debug, PartialEq, Reflect)]
    enum Shape { Circle { radius: f32 }, Square(f32) }

    fn outer() -> Outer {
        Outer { inner: Inner { x: 1.0, tags: vec!["a".into(), "b".into()] }, pair: (2, 3), shape: Shape::Circle { radius: 4.0 } }
    }

    #[test]
    fn parses_paths() {
        assert_eq!(parse_path("a.b[2].0").unwrap(), [PathSegment::Field("a"), PathSegment::Field("b"), PathSegment::Index(2), PathSegment::Field("0")]);
        assert_eq!(parse_path(".a").unwrap(), [PathSegment::Field("a")]);
        assert_eq!(parse_path("").unwrap(), []);
        assert!(parse_path("a..b").is_err());
        assert!(parse_path("a[x]").is_err());
        assert!(parse_path("a[1]b").is_err());
    }

    #[test]
    fn gets_nested_values() {
        let outer = outer();
        assert_eq!(get_path(&outer, "inner.x").unwrap(), Value::Float(1.0));
        assert_eq!(get_path(&outer, "inner.tags[1]").unwrap(), Value::Str("b".into()));
        assert_eq!(get_path(&outer, "pair.1").unwrap(), Value::Int(3));
        assert_eq!(get_path(&outer, "shape.radius").unwrap(), Value::Float(4.0));
        assert!(get_path(&outer, "inner.y").is_err());
        assert!(get_path(&outer, "inner.tags[5]").is_err());
    }

    #[test]
    fn sets_nested_values() {
        let mut outer = outer();
        set_path(&mut outer, "inner.x", &Value::Float(5.0)).unwrap();
        set_path(&mut outer, "inner.tags[0]", &Value::Str("z".into())).unwrap();
        set_path(&mut outer, "pair.0", &Value::Int(9)).unwrap();
        set_path(&mut outer, "shape", &Value::Enum { variant: "Square".into(), value: Box::new(Value::Tuple(vec![Value::Float(2.0)])) }).unwrap();
        set_path(&mut outer, "shape.0", &Value::Float(3.0)).unwrap();
        assert_eq!(outer, Outer { inner: Inner { x: 5.0, tags: vec!["z".into(), "b".into()] }, pair: (9, 3), shape: Shape::Square(3.0) });
        assert!(set_path(&mut outer, "pair.0", &Value::Int(300)).is_err());
    }
}
