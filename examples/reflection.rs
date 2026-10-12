//! Reflection: reading, building and patching values by field name, type info for
//! inspectors, and editing a world by component id and path, as an editor would.
//!
//! Run with `cargo run --example reflection`.

use anarchy::*;
use mutual::RelaxedMutex;

/// Where an entity is.
#[derive(Debug, Default, Component, Reflect)]
struct Transform {
    /// Position in world units.
    pos: Vec2,
    /// Size multiplier, shown as a slider.
    #[reflect(hint(min = 0.0, max = 10.0))]
    scale: f32,
    #[reflect(skip)]
    cache: Vec<f32>,
}

/// Plain data types derive `Reflect` without being components.
#[derive(Debug, Default, Clone, PartialEq, Reflect)]
struct Vec2 { x: f32, y: f32 }

#[derive(Debug, PartialEq, Component, Reflect)]
enum Shape {
    Point,
    Circle { radius: f32 },
    Rect(f32, f32),
}

/// Missing fields come from `Default` instead of each field's fallback.
#[derive(Debug, Component, Reflect)]
#[reflect(default)]
struct Speed { value: f32, max: f32 }

impl Default for Speed {
    fn default() -> Self { Self { value: 1.0, max: 10.0 } }
}

#[derive(Debug, Default, Resource, Reflect)]
struct Gravity { y: f32 }

fn typed_values() -> anyhow::Result<()> {
    // `Value` is untyped data. Its text form is the scene syntax.
    let mut transform = Transform::default();
    transform.apply(&parse_value("{ pos: { x: 1.0 }, scale: 2.0 }")?)?; // pos.y is unchanged
    println!("patched: {}", transform.to_value());

    // Paths reach nested fields.
    set_path(&mut transform, "pos.y", &Value::Float(3.0))?;
    assert_eq!(get_path(&transform, "pos")?.to_string(), "{ x: 1.0, y: 3.0 }");
    transform.cache.push(1.0); // skipped fields are invisible to reflection

    // Build new values. Fields left out use their fallback, or `Default` with `#[reflect(default)]`.
    let copy = Transform::from_value(&transform.to_value())?;
    assert_eq!(copy.pos, Vec2 { x: 1.0, y: 3.0 });
    assert!(copy.cache.is_empty());
    let speed = Speed::from_value(&parse_value("{ value: 4.0 }")?)?;
    assert_eq!((speed.value, speed.max), (4.0, 10.0));

    // Enums are patched and read by variant.
    let mut shape = Shape::from_value(&parse_value("Circle { radius: 1.0 }")?)?;
    set_path(&mut shape, "radius", &Value::Float(2.5))?;
    assert_eq!(shape, Shape::Circle { radius: 2.5 });
    shape.apply(&parse_value("Rect(1.0, 2.0)")?)?; // a new variant replaces the old one
    assert_eq!(shape, Shape::Rect(1.0, 2.0));
    let _ = Shape::Point;

    // `TypeInfo` holds what an inspector needs: fields, types, docs and hints.
    let info = Transform::type_info();
    println!("{}: {}", info.short_name, info.docs);
    if let TypeKind::Struct(fields) = &info.kind {
        for field in fields.fields() {
            let hint = field.hint("max").map(|max| format!(" (max {max})")).unwrap_or_default();
            println!("  {}: {}{hint}  /// {}", field.name, (field.ty)().short_name, field.docs);
        }
    }
    Ok(())
}

/// Logs every edit made by path, as an editor would for undo and refresh.
fn log_edits(edits: Event<ReflectEdit>) {
    for edit in edits.read() {
        println!("edit {:?} {}: {} -> {}", edit.target, edit.path, edit.old, edit.new);
    }
}

fn editing_a_world() -> anyhow::Result<()> {
    let world = World::default();
    world.insert_resource(Gravity { y: -9.8 });
    world.insert(1, Box::new([
        RelaxedMutex::new(Box::new(Transform::default()) as DynComponent),
        RelaxedMutex::new(Box::new(Shape::Point) as DynComponent),
    ]));

    // Every non-generic `Reflect` type is in the world's registry, found by name or id.
    {
        let registry = world.type_registry();
        let registration = registry.resolve("Transform")?;
        println!("resolved {}", registry.name_of(registration));
    } // drop the registry guard before editing

    // List what is there, without knowing the types.
    for id in world.entity_ids().collect::<Vec<_>>() {
        for component in world.component_ids(id).unwrap_or_default() {
            let value = world.reflect_component(id, component, |c| {
                format!("{} {}", c.reflect_info().short_name, c.to_value())
            })?;
            println!("entity {id}: {value}");
        }
    }

    // Edit by id and path. Each set sends a `ReflectEdit` event.
    world.set_component_path(1, Transform::id(), "pos.x", &Value::Float(5.0))?;
    world.set_component_path(1, Transform::id(), "scale", &Value::Float(2.0))?;
    world.set_resource_path(Gravity::id(), "y", &Value::Float(-1.6))?;
    assert_eq!(world.get_component_path(1, Transform::id(), "pos")?, parse_value("{ x: 5.0, y: 0.0 }")?);
    assert_eq!(world.get_resource_path(Gravity::id(), "y")?, Value::Float(-1.6f32 as f64));

    // Add a component by type name and value, then remove it by id.
    world.insert_reflected_component(1, "Speed", &parse_value("{ max: 3.0 }")?)?;
    assert_eq!(world.get::<&Speed>(1)?.unwrap().max, 3.0);
    world.remove_component_dyn(1, Speed::id())?;

    // Mutable access as `dyn Reflect`.
    world.reflect_component_mut(1, Shape::id(), |shape| shape.apply(&parse_value("Circle { radius: 1.0 }")?))??;
    assert_eq!(*world.get::<&Shape>(1)?.unwrap(), Shape::Circle { radius: 1.0 });

    let mut graph = SystemGraph::new();
    graph.append_system(log_edits, SystemInstruction::default(), std::iter::empty())?;
    SingleThreadedExecutor::new().run(&world, &graph)?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    typed_values()?;
    editing_a_world()
}
