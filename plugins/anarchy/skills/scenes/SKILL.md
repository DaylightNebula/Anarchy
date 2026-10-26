---
name: scenes
description: Use when writing Rust code with the anarchy ECS crate that needs runtime reflection or scenes - deriving Reflect, Value and paths, the TypeRegistry, editing a world's components and resources by id and path (for editors and inspectors), ReflectEdit events, the text scene format, the scene! macro, SceneLibrary inheritance, spawning scenes, saving a world back to a scene, and the socket feature (WorldSocket, SocketClient, the anarchy CLI) for editing a running app from another process.
---

# anarchy: reflection, editing and scenes

Reflection is opt-in. A type that derives `Reflect` can be read, built and patched by field
name at runtime, and a `TypeRegistry` on the world lets code that doesn't know the type
(an editor, a scene file) work with it. Non-generic `Reflect` types register themselves,
so there is no registration step. If this skill and the compiler disagree, trust the
compiler.

## Reflecting a type

```rust
use anarchy::*;

/// Where an entity is.                       // doc comments are kept in TypeInfo
#[derive(Debug, Default, Component, Reflect)]
struct Transform {
    pos: Vec2,
    #[reflect(hint(min = 0.0, max = 10.0))]   // editor hints, any `Value::from`-able literal
    scale: f32,
    #[reflect(skip)]                          // hidden, rebuilt with Default
    cache: Vec<f32>,
}

#[derive(Debug, Default, Reflect)]            // plain data types need no Component
struct Vec2 { x: f32, y: f32 }

#[derive(Debug, Component, Reflect)]
struct Target(#[reflect(entity)] EntityID);  // ids saved as #name references in scenes

#[derive(Debug, Component, Reflect)]
#[reflect(default)]                           // missing fields come from Default
struct Speed { value: f32, max: f32 }
impl Default for Speed { fn default() -> Self { Self { value: 1.0, max: 10.0 } } }
```

- The derive implements `Reflect`, `Typed` and `FromValue`. It works on structs (named,
  tuple, unit) and enums. Field types must be reflected too: numbers, `bool`, `char`,
  `String`, `Option`, `Vec`, arrays, tuples up to 12, `Box` and other derived types are.
  `HashMap` and other std types are not.
- Generic types get `T: Reflect + Typed + FromValue` bounds and are not registered
  automatically. Register each concrete type: `world.register_component::<Foo<f32>>()`.
- Without `#[reflect(default)]`, a missing field uses its type's `FromValue::fallback`:
  zero, `false`, empty string or list, `None`, or a struct of fallbacks. A field type
  with no fallback (`char`) must then be given.

## Values, patches and paths

`Value` is untyped data. Its `Display` form is the scene syntax, and `parse_value` reads it:

```rust
let mut t = Transform::default();
t.apply(&parse_value("{ pos: { x: 1.0 }, scale: 2.0 }")?)?; // patch: pos.y is unchanged
let value = t.to_value();                                   // read everything
let copy = Transform::from_value(&value)?;                  // build a new one

set_path(&mut t, "pos.y", &Value::Float(3.0))?;
assert_eq!(get_path(&t, "pos")?.to_string(), "{ x: 1.0, y: 3.0 }");
```

- Paths: `field`, `a.b`, `0` (tuple field), `items[2]`, `shape.radius` (field of the current
  enum variant).
- `apply` patches structs field by field. A different enum variant, a `Vec` or an
  `Option` going from `None` to `Some` is replaced whole. Arrays apply item by item.
- Values: `Int(i128)`, `Float`, `Bool`, `Str`, `List`, `Tuple`, `Struct`, `Enum { variant,
  value }` and `Entity(Id | Named)`. Floats accept integers. Integers are range checked.
  `Option` fields accept a bare value as `Some`.
- `Typed::type_info()` (or `reflect_info()` on a `dyn Reflect`) gives the `TypeInfo`:
  `short_name`, `docs` and `kind` (`Struct(VariantKind)`, `Enum(variants)`, `List`, `Value`),
  where each `FieldInfo` has `name`, `ty()`, `docs` and `hints`.
- `ReflectRef`/`ReflectMut` (`Struct`, `Enum`, `List`, `Value`) walk a value generically.
  `ReflectList` has `push_value` and `remove_at` for editors.

## The registry and editing a world

```rust
let world = World::default();                // its TypeRegistry is created on first use

for id in world.entity_ids().collect::<Vec<_>>() {          // collect before editing
    for component in world.component_ids(id).unwrap_or_default() {
        let value = world.reflect_component(id, component, |c| c.to_value());
        // Err for components that don't derive Reflect: show them as opaque
    }
}
world.set_component_path(id, Transform::id(), "pos.x", &Value::Float(5.0))?;
world.set_resource_path(Gravity::id(), "y", &Value::Float(-1.0))?;
world.insert_reflected_component(id, "Speed", &parse_value("{ max: 3.0 }")?)?;
world.remove_component_dyn(id, Speed::id())?;
```

- `TypeRegistry::new()` collects every non-generic type deriving `Reflect` with
  `inventory`, and detects whether each is a component and/or resource. `World` creates
  one on first use (`world.type_registry()`). To control what is in it, insert your own
  first, e.g. `world.insert_resource(TypeRegistry::empty())` plus `register_*` calls.
- Names: the short name, or the end of the path when short names clash (`ui::Button`),
  or the full path; whitespace ignored. `resolve(name)` errors with the clashing candidates.
  `name_of` gives the shortest name that finds a type.
- Lookups: `get::<T>()`, `get_by_name`, `resolve`, `get_component(id)`, `get_resource(id)`, `iter()`.
- The registry is a world resource, so drop any `world.type_registry()` guard before
  calling `register_*` on the same thread, or it deadlocks.
- On `wasm32-unknown-unknown`, registration runs from wasm constructors, normally called by
  the linker. If types are missing, call `__wasm_call_ctors` once at startup (see the
  `inventory` docs).
- `reflect_component(_mut)` locks only that one component, and doesn't hold the registry
  while your closure runs. Don't lock the same component inside the closure.
- `set_component_path` and `set_resource_path` send a `ReflectEdit { target, path, old, new }`
  event. Read it with `Event<ReflectEdit>` in a system for undo history or refresh.
- `entity_ids()` read locks parts of the by-id index while iterating, so collect it before
  inserting or removing entities.
- With the crate's `serde` feature, `Value`, `EntityRef`, `ReflectEdit` and `ReflectTarget`
  serialize and deserialize, and `TypeInfo` serializes, for an editor in another process.

## Text scenes

```rust
let scene = Scene::parse(r#"
    @Gravity { y: -9.8 },             // resource: patched if present, else built
    #Player                           // name: adds Name("Player"), referable as #Player
    Transform { pos: Vec2 { x: 1.0 } } // type names inside values are optional
    Transform { scale: 2.0 }          // same type again: merged in order
    Target(#Enemy)                    // entity reference, resolved at spawn
    Shape = Circle { radius: 0.5 }    // `Type = value` for enum components
    [                                 // children, comma separated
        #Hat :Hat,                    // :name inherits a SceneLibrary scene
    ],
    #Enemy Speed,                     // a bare type is its fallback / Default
"#)?;

let spawned = scene.spawn(&world, next_entity_id)?; // or any FnMut() -> EntityID
let player = spawned.by_name["Player"];
```

- Entities are separated by commas, components by whitespace. `//` and `/* */` comments.
- Every type in a text scene must derive `Reflect` (generic ones also registered by hand).
  Parse errors give a line and column.
- Spawn builds every entity before inserting any, so a failure leaves entities untouched.
  An entity needs at least one component or a `#name`.
- `#name` references resolve to the first entity with that name in the scene.
- Inheritance: `world.insert_resource(SceneLibrary::new())` then
  `library.insert("Hat", scene)`. The inherited scene must have exactly one root. Its
  components apply first and its children come before the entity's own.
- `SceneInstance { roots, entities, by_name }`. `despawn(&world)` removes everything it spawned.

## The scene! macro

```rust
let scene = scene! {
    @Gravity { y: -9.8 },
    #Player
    Transform { pos: Vec2 { x: 1.0 }, scale: 2.0 } // Default, then only these fields set
    Health(100)                                     // any Type(args) expression
    { Shape::Circle(0.5) }                          // any expression, in braces
    [ #Hat :Hat Marker ],
};
```

- Types are checked by the compiler and don't need `Reflect`. `Type { .. }`
  and bare `Type` need `Default`.
- Each patch is a `move` closure run on every spawn. Clone captured values inside it.
- `#Enemy` references inside values only work in text scenes.
- Macro scenes can't be written as text (`to_text` errors).

## Saving

```rust
let saved = Scene::from_world(&world, &spawned.roots)?;  // roots and all descendants
std::fs::write("level.scene", saved.scene.to_text()?)?;
for (entity, component) in &saved.skipped { /* not reflected, not saved */ }
```

- Each component is saved as the difference from its fallback, so files stay short.
- A unique `Name` becomes `#name`. `#[reflect(entity)]` fields pointing at saved
  entities become `#name` references, and unnamed targets get `#e<id>`. So do entities
  with nothing to save.

## Sockets and the CLI

With the `socket` feature (native only), a running app can be inspected and edited from
outside, by the `anarchy` CLI or an editor:

```rust
let socket = WorldSocket::bind(&world, DEFAULT_SOCKET_ADDRESS)?; // or "unix:/tmp/app.sock"
// keep `socket` alive while the app runs, dropping it stops listening

let mut client = SocketClient::connect(DEFAULT_SOCKET_ADDRESS)?;   // from another program
let target = Target::Component { entity: 1, type_name: "Health".into() };
client.request(Request::Set { target, path: "current".into(), value: Value::Int(55) })?;
let Response::Entities(entities) = client.request(Request::Entities)? else { unreachable!() };
```

```sh
anarchy ls | show 1 | res | types | info Health
anarchy get 1 Health current        anarchy set @Gravity y -1.5
anarchy insert 1 Health { current: 5, max: 10 }   anarchy remove 1 Health
anarchy spawn level.scene | despawn 1 -r | parent 2 1 | save > world.scene
anarchy -a unix:/tmp/app.sock       # no command: an interactive shell
```

- The protocol is one JSON `Request` per line, answered by one JSON `Response` line.
  Types are named as in scenes; values are `Value`s in serde form (`{"Int":55}`).
- `World::handle_request` answers a request directly, `socket::serve` over any stream.
- Requests run on the socket's threads alongside systems, under the same locks, and
  `Set` sends `ReflectEdit` events. Only reflected types can be read or edited.
- Spawned entities get ids above the highest id in the world.
- No authentication: keep TCP on localhost, or use a unix socket.
