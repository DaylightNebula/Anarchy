//! Writing scenes as text, see [`Scene::to_text`].

use std::fmt::Write;

use anyhow::bail;

use crate::*;
use crate::reflect::value::write_name;

impl Scene {
    /// Writes this scene as text that [`Scene::parse`] reads back.
    ///
    /// # Errors
    ///
    /// Errors if the scene holds typed patches from [`scene!`], which have no text form.
    pub fn to_text(&self) -> anyhow::Result<String> {
        let mut out = String::new();
        for resource in &self.resources {
            let ResourcePatch::Reflected { type_name, value } = resource else { bail!("typed resource patches can't be written as text") };
            out.push('@');
            write_patch(&mut out, type_name, value);
            out.push_str(",\n");
        }
        for (i, root) in self.roots.iter().enumerate() {
            if i > 0 || !self.resources.is_empty() { out.push('\n') }
            write_entity(&mut out, root, 0)?;
            out.push_str(",\n");
        }
        Ok(out)
    }
}

fn write_entity(out: &mut String, entity: &SceneEntity, depth: usize) -> anyhow::Result<()> {
    let indent = "    ".repeat(depth);
    let mut lines = vec![];
    let mut head = String::new();
    if let Some(name) = &entity.name {
        head.push('#');
        write_name(&mut head, name).unwrap();
    }
    if let Some(base) = &entity.inherits {
        if !head.is_empty() { head.push(' ') }
        head.push(':');
        write_name(&mut head, base).unwrap();
    }
    if !head.is_empty() { lines.push(head) }
    for patch in &entity.components {
        let ScenePatch::Reflected { type_name, value } = patch else { bail!("typed component patches can't be written as text") };
        let mut line = String::new();
        write_patch(&mut line, type_name, value);
        lines.push(line);
    }
    for (i, line) in lines.iter().enumerate() {
        if i > 0 { out.push('\n') }
        out.push_str(&indent);
        out.push_str(line);
    }
    if !entity.children.is_empty() {
        if !lines.is_empty() { out.push('\n') }
        out.push_str(&indent);
        out.push_str("[\n");
        for child in &entity.children {
            write_entity(out, child, depth + 1)?;
            out.push_str(",\n");
        }
        out.push_str(&indent);
        out.push(']');
    }
    Ok(())
}

/// Writes `type_name` and its patch, e.g. `Health(100)`, `Pos { x: 1.0 }` or `Shape = Circle(1.0)`.
fn write_patch(out: &mut String, type_name: &str, value: &Value) {
    out.push_str(type_name);
    match value {
        Value::Unit => {}
        Value::Struct(_) => write!(out, " {value}").unwrap(),
        Value::Tuple(_) => write!(out, "{value}").unwrap(),
        other => write!(out, " = {other}").unwrap()
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    const TEXT: &str = r#"
        // resources come first
        @Gravity { y: -9.8 },
        #Player :Base
        Pos { x: 1.0, y: -2.0 }
        Health(100)
        Shape = Circle { radius: 0.5 }
        Tags = ["a", "b \"c\""]
        Target(#Enemy)
        [
            #"Big Sword"
            Pos { x: 0.5 },
            Marker /* no fields */,
        ],
        #Enemy
        Health(50)
        Link(#7)
        Generic<f32> { value: 1e3 },
    "#;

    #[test]
    fn parses_scenes() {
        let scene = Scene::parse(TEXT).unwrap();
        assert_eq!(scene.resources.len(), 1);
        assert_eq!(scene.roots.len(), 2);
        let player = &scene.roots[0];
        assert_eq!(player.name.as_deref(), Some("Player"));
        assert_eq!(player.inherits.as_deref(), Some("Base"));
        assert_eq!(player.components.len(), 5);
        assert_eq!(player.children.len(), 2);
        assert_eq!(player.children[0].name.as_deref(), Some("Big Sword"));
        let ScenePatch::Reflected { type_name, value } = &player.components[2] else { panic!() };
        assert_eq!(type_name, "Shape");
        assert_eq!(*value, Value::Enum { variant: "Circle".into(), value: Box::new(Value::Struct(vec![("radius".into(), Value::Float(0.5))])) });
        let ScenePatch::Reflected { value, .. } = &player.components[4] else { panic!() };
        assert_eq!(*value, Value::Tuple(vec![Value::Entity(EntityRef::Named("Enemy".into()))]));
        let ScenePatch::Reflected { type_name, value } = &scene.roots[1].components[2] else { panic!() };
        assert_eq!(type_name, "Generic<f32>");
        assert_eq!(value.field("value"), Some(&Value::Float(1000.0)));
    }

    #[test]
    fn prints_what_it_parses() {
        let scene = Scene::parse(TEXT).unwrap();
        let text = scene.to_text().unwrap();
        let again = Scene::parse(&text).unwrap();
        assert_eq!(again.to_text().unwrap(), text);
        assert_eq!(format!("{:?}", again.roots), format!("{:?}", scene.roots));
        assert!(text.starts_with("@Gravity { y: -9.8 },\n\n#Player :Base\nPos { x: 1.0, y: -2.0 }\nHealth(100)\n"), "{text}");
        assert!(text.contains("[\n    #\"Big Sword\"\n    Pos { x: 0.5 },\n    Marker,\n]"), "{text}");
    }

    #[test]
    fn errors_have_positions() {
        let error = Scene::parse("#A Pos { x: }").unwrap_err();
        assert!(format!("{error:#}").contains("line 1, column 13"), "{error:#}");
        let error = Scene::parse("#A\nPos\n#B").unwrap_err();
        assert!(format!("{error:#}").contains("line 3"), "{error:#}");
        assert!(Scene::parse("#A [ #B").is_err());
        assert!(Scene::parse(",").is_err());
        assert!(Scene::parse("").unwrap().roots.is_empty());
    }

    #[test]
    fn parses_values() {
        assert_eq!(parse_value("-12").unwrap(), Value::Int(-12));
        assert_eq!(parse_value("1_000.5").unwrap(), Value::Float(1000.5));
        assert_eq!(parse_value("-inf").unwrap(), Value::Float(f64::NEG_INFINITY));
        assert_eq!(parse_value("()").unwrap(), Value::Unit);
        assert_eq!(parse_value("(1,)").unwrap(), Value::Tuple(vec![Value::Int(1)]));
        assert_eq!(parse_value("None").unwrap(), Value::Enum { variant: "None".into(), value: Box::new(Value::Unit) });
        assert_eq!(parse_value("a::B::C(1)").unwrap(), Value::Enum { variant: "C".into(), value: Box::new(Value::Tuple(vec![Value::Int(1)])) });
        assert_eq!(parse_value(r#"{ 0: "\n", "two words": #3 }"#).unwrap(), Value::Struct(vec![
            ("0".into(), Value::Str("\n".into())),
            ("two words".into(), Value::Entity(EntityRef::Id(3)))
        ]));
        assert!(parse_value("1 2").is_err());
        let value = parse_value(r#"{ a: [1.5, true, "x", #E, None, V { b: (1, 2) }] }"#).unwrap();
        assert_eq!(parse_value(&value.to_string()).unwrap(), value);
    }
}
