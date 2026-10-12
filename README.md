# Anarchy V2

An experimental ECS crate that allows for the highest possible degree of concurrency
in an ECS world while systems run rampant through it. This is achieved by only locking data
at the lowest point needed for a general purpose ECS, which is generally at the component
or resource level. This also enables faster data processing through queries whose work can
be shared across threads.

> **Status:** experimental and under active development. The API will change.

## Contents

- [Features](#features)
- [Getting started](#getting-started)
- [Quick example](#quick-example)
- [Examples](#examples)
- [Guide](#guide)
  - [Worlds and entities](#worlds-and-entities)
  - [Queries](#queries)
  - [Resources](#resources)
  - [Hierarchy](#hierarchy)
  - [Systems and scheduling](#systems-and-scheduling)
  - [Running systems](#running-systems)
  - [Events](#events)
  - [Reflection](#reflection)
  - [Scenes](#scenes)
  - [Sockets and the CLI](#sockets-and-the-cli)
- [Concepts at a glance](#concepts-at-a-glance)
- [AI assistant skills](#ai-assistant-skills)
- [Development](#development)

## Features

**Storage**

- **Per-component locking.** Every component and resource sits behind its own lock, so
  systems only block each other when they touch the same data.
- **Lock-free tables.** Entities with the same set of components share a table, a lock-free
  linked list that many threads can walk, insert into and pop from at once.
- **Indexed queries.** The default `IndexedWorld` indexes tables by component, so a query
  only visits tables that hold its rarest required component.
- **Lookups by id.** Every entity is also indexed by id, so `World::get` and
  `World::remove` are a single hash lookup instead of a table walk.
- **Entity handles.** `World::handle` pairs an id with its world, to read components and
  walk or edit the hierarchy from one value.
- **Hierarchy.** Parent and child links between entities, with cycle checks, ancestor and
  descendant walks, and recursive despawn. Links live outside the tables, so parenting never
  moves an entity.

**Scheduling**

- **Function systems.** Plain functions become systems. Their arguments are `SystemParam`s
  such as `World`, `Query<...>`, `Res<R>`, `ResMut<R>`, `Event<E>` and `Hierarchy`.
- **Dependency graph.** Order systems with `before`, `after`, metadata tags and
  `Start`/`Normal`/`End` pins. The graph rejects cycles and contradictory pins.
- **Single and multi-threaded executors.** Run the same graph on the calling thread or on
  a pool of threads.
- **Shared work queues.** A system can split work into a `ParIter` and submit it, and idle
  executor threads help finish it.
- **Events.** `Event<E>` sends events that every system reads exactly once, as long as it
  runs within a second of the send.

**Reflection and scenes**

- **Reflection.** `#[derive(Reflect)]` lets code read, build and patch values by field name
  at runtime through an untyped `Value`, with type info, docs and editor hints for each field.
- **Editing API.** A `TypeRegistry` on the world lets an editor list entities, components
  and resources, get and set fields by path, and add or remove components, all by id.
- **Scenes.** A text format and a `scene!` macro, both in the style of Bevy's BSN, describe
  entity trees as patches over defaults, with named entity references, inheritance from a
  `SceneLibrary`, and saving a world back to text.
- **Sockets and a CLI.** With the `socket` feature, `WorldSocket` serves a running world
  over TCP or a unix socket, so the `anarchy` command line tool, editors and other
  programs can list, read, edit, spawn and save its entities and resources.

## Getting started

The crate is not on crates.io yet, so add it as a git dependency. Building `DynComponents`
by hand also needs `mutual`:

```toml
[dependencies]
anarchy = { git = "https://github.com/DaylightNebula/anarchy" }
mutual = "1.0.2"
```

It uses the 2024 edition and `let` chains, so it needs Rust 1.88 or newer.

## Quick example

```rust
use anarchy::*;
use mutual::RelaxedMutex;

#[derive(Debug, Component)]
struct Position(f32);

#[derive(Debug, Component)]
struct Velocity(f32);

#[derive(Debug, Resource)]
struct Ticks(u32);

fn movement(mut query: Query<(&'static mut Position, &'static Velocity)>) {
    while let Some((_entity, (mut position, velocity))) = query.next().unwrap() {
        position.0 += velocity.0;
    }
}

fn tick(mut ticks: ResMut<Ticks>) {
    ticks.0 += 1;
}

fn main() -> anyhow::Result<()> {
    let world = World::default();
    world.insert_resource(Ticks(0));
    world.insert(0, Box::new([
        RelaxedMutex::new(Box::new(Position(0.0)) as DynComponent),
        RelaxedMutex::new(Box::new(Velocity(2.0)) as DynComponent),
    ]));

    let mut graph = SystemGraph::new();
    graph.append_system(movement, SystemInstruction::default(), std::iter::empty())?;
    graph.append_system(tick, after(movement), std::iter::empty())?;

    SingleThreadedExecutor::new().run(&world, &graph)?;
    Ok(())
}
```

A similar example is a doctest in the crate docs (`src/lib.rs`), so it is checked by
`cargo test`.

## Examples

[`examples/`](examples) has a runnable program for each major feature. Run one with
`cargo run --example <name>`:

| Example | Shows |
| --- | --- |
| [`world`](examples/world.rs) | Entities, lookups by id, queries, adding and removing components, resources |
| [`hierarchy`](examples/hierarchy.rs) | Parent and child links, ancestor and descendant walks, entity handles, recursive despawn |
| [`systems`](examples/systems.rs) | Function systems, `before`/`after`/pins/metadata tags, custom `SystemParam`s, hand-written systems |
| [`events`](examples/events.rs) | Sending and reading events between systems and from outside them |
| [`multithreaded`](examples/multithreaded.rs) | `MultiThreadedExecutor`, stopping it from a system, sharing work with `ParIter` (native only) |
| [`reflection`](examples/reflection.rs) | `#[derive(Reflect)]`, values, paths, `TypeInfo`, the registry, editing a world by path, `ReflectEdit` |
| [`scenes`](examples/scenes.rs) | Text scenes, `scene!`, `SceneLibrary` inheritance, saving a world and loading it back |
| [`serde_editor`](examples/serde_editor.rs) | Type info, values and edits as JSON for an external editor (needs `--features serde`) |
| [`socket`](examples/socket.rs) | A running app served over a `WorldSocket`, to poke at with the `anarchy` CLI (needs `--features socket`, native only) |

## Guide

### Worlds and entities

`World` is a cheap, cloneable handle to a world's entities, resources and hierarchy.
`World::default()` stores entities in an `IndexedWorld`. `World::new(ListWorld::new())`
uses the simpler `ListWorld`, which scans every table on each query and insert.

Entity ids are `u64`s chosen by the caller and are not checked for uniqueness. An entity
needs at least one component:

```rust
world.insert(7, Box::new([
    RelaxedMutex::new(Box::new(Position(0.0)) as DynComponent),
]));
```

Every entity is indexed by id, so single-entity access never walks a table:

```rust
// Locked access to some of its components. `None` if there is no entity 7,
// an error if it lacks a requested component.
let (mut position, velocity) = world.get::<(&mut Position, &Velocity)>(7)?.unwrap();

// Its components and their sorted ids.
let entity: Option<Entity> = world.entity(7);

// Remove it, returning its components. Its children become roots.
let components: Option<DynComponents> = world.remove(7);
```

`World::handle(id)` gives an `EntityHandle`, an id paired with its world. Making one is
free and looks nothing up, so it is a convenient way to pass an entity around:

```rust
let player = world.handle(7);
println!("player at {}", player.get::<&Position>()?.unwrap().0);
for child in player.descendants() {
    if let Some(position) = child.get::<&Position>()? {
        println!("child {} at {}", child.id(), position.0);
    }
}
player.despawn_recursive(); // removes 7 and everything below it
```

### Queries

`Query<QG>` walks every entity holding the components in `QG`, which is `&A`, `&mut A`,
`Option<&A>` or `Option<&mut A>`, or a tuple of up to 8 of them. Each component is locked
as it is handed out and stays locked until its guard is dropped.

```rust
let mut query = Query::<(&Position, &mut Velocity)>::new(&world);
while let Some((entity, (position, mut velocity))) = query.next()? {
    velocity.0 = -position.0;
}
```

Inside a system, take the query as a parameter with `'static` references, as in the
[quick example](#quick-example).

`Query::new` asks the world for every listed component, `Option` ones included. For now
that means an optional component only resolves to `None` when the query is built with
`Query::from_iter` over tables that lack it.

### Resources

A resource is one value per type, stored on the world. Derive `Resource` on a
`Debug + Send` type, then:

```rust
world.insert_resource(Ticks(0));                 // insert or replace
world.init_resource_with(|| Ticks(0));           // insert only if missing
world.resource_mut::<Ticks>()?.0 += 1;           // blocks while any other guard is held
assert_eq!(world.resource::<Ticks>()?.0, 1);     // blocks while a mutable guard is held
assert!(world.has_resource::<Ticks>());
world.remove_resource::<Ticks>();
```

Systems take `Res<R>` to read and `ResMut<R>` to write. If the world holds no `R`, the
system returns an error without running. A system that takes both `Res<R>` and `ResMut<R>`
for the same `R` deadlocks.

### Hierarchy

Every world holds a `Hierarchy` of parent and child links, reached with
`World::hierarchy()`, an `EntityHandle`, or as a system parameter:

```rust
let hierarchy = world.hierarchy();
hierarchy.set_parent(2, 1)?;     // 2 is now a child of 1
hierarchy.add_child(1, 3)?;      // the same, with the arguments the other way round

assert_eq!(hierarchy.parent(2), Some(1));
assert_eq!(*hierarchy.children(1).unwrap(), [2, 3]);  // in the order they were added
assert!(hierarchy.set_parent(1, 2).is_err());          // would make a cycle

let ancestors: Vec<_> = hierarchy.ancestors(2).collect();    // parent first, up to the root
let descendants: Vec<_> = hierarchy.descendants(1).collect(); // depth first

hierarchy.remove_parent(2);      // 2 becomes a root
```

- Setting a new parent moves the child from its old one.
- `World::remove` unlinks the entity, and its children become roots.
  `EntityHandle::despawn_recursive` removes the whole subtree instead.
- Only ids are stored. The entities are not checked to exist.
- Reads never wait on each other. Edits are serialised, so the links can never form a
  cycle, but a read racing an edit may see it half applied.
- `children` returns a guard that keeps part of the hierarchy read-locked. Drop it before
  editing the hierarchy on the same thread.

### Systems and scheduling

Any function whose arguments are all `SystemParam`s is a system. It may take up to 16 of
them: `()`, `World`, `Query`, `Res`, `ResMut`, `Event` and `Hierarchy`. Implement
`SystemParam` to add your own. `SystemParam::extract` is given the running system's key and
can fail, in which case the system returns that error without running. For full control,
implement `System` directly, which also gives access to the `SharedExecutionState`.

Systems are added to a `SystemGraph` with a `SystemInstruction` that orders them:

```rust
let mut graph = SystemGraph::new();
graph.append_system(input, pin(SystemPin::Start), std::iter::empty())?;
graph.append_system(movement, after(input), std::iter::empty())?;
graph.append_system(tick, and(after(movement), before(render)), std::iter::empty())?;
graph.append_system(render, pin(SystemPin::End), std::iter::empty())?;
```

- `before(system)` and `after(system)` order against one system.
- The last argument tags a system with metadata, any `TypeId`. `before_meta` and
  `after_meta` order against every system with a tag.
- `pin` places a system in `Start`, `Normal` (the default) or `End`. Pins run one after
  another and never overlap.
- `and` combines instructions.
- `append_system` fails, leaving the system out, if its instruction makes a cycle or
  contradicts a pin. Appending the same function again replaces it.
- `remove_system` and `remove_by_meta` take systems back out.

### Running systems

`SingleThreadedExecutor` runs one pass of the graph on the calling thread:

```rust
SingleThreadedExecutor::new().run(&world, &graph)?;
```

`MultiThreadedExecutor` runs on several threads, and keeps running the graph until a flag
is cleared:

```rust
use std::sync::atomic::AtomicBool;

let running = AtomicBool::new(true);
// Runs Start, Normal and End pins in order on 4 threads, looping while `running` is true.
MultiThreadedExecutor::new().run(&world, &graph, 4, &running)?;
```

A system can share its work with idle executor threads by submitting a `WorkQueueEntry`,
such as `ParIter` (one item per call) or `SingleRun` (one function, once), to the
`SharedExecutionState`:

```rust
struct Heavy;

impl System<(), ()> for Heavy {
    fn run(&self, world: &World, exec_state: &SharedExecutionState) -> anyhow::Result<()> {
        exec_state.submit_work(ParIter::new(0..1000, |i| expensive(i)));
        // help out until the queue is empty
        while exec_state.exec_from_work_queue() {}
        Ok(())
    }
}
```

### Events

Systems talk to each other through the `Event<E>` system param. Any `Clone + Send` type
can be an event. `send` queues an event, and `read` returns every event the calling
system hasn't seen yet, oldest first. Each system reads each event **once**, as long as
it runs within one second (`EVENT_LIFETIME`) of the send. After that the event expires.

```rust
use anarchy::*;

#[derive(Clone, Debug)]
struct Damage { entity: EntityID, amount: u32 }

#[derive(Debug, Resource)]
struct DamageTaken(u32);

// Sends one event per run.
fn attack(damage: Event<Damage>) {
    damage.send(Damage { entity: 7, amount: 5 });
}

// Sees each Damage event once, no matter how many times it runs.
fn apply_damage(mut taken: ResMut<DamageTaken>, damage: Event<Damage>) {
    for hit in damage.read() {
        taken.0 += hit.amount;
    }
}

// A second reader keeps its own place in the queue, so it sees the same events.
fn log_damage(damage: Event<Damage>) {
    for hit in damage.read() {
        println!("entity {} took {} damage", hit.entity, hit.amount);
    }
}

fn main() -> anyhow::Result<()> {
    let world = World::default();
    world.insert_resource(DamageTaken(0));

    let mut graph = SystemGraph::new();
    graph.append_system(attack, SystemInstruction::default(), std::iter::empty())?;
    graph.append_system(apply_damage, after(attack), std::iter::empty())?;
    graph.append_system(log_damage, after(attack), std::iter::empty())?;

    let executor = SingleThreadedExecutor::new();
    executor.run(&world, &graph)?;
    executor.run(&world, &graph)?;

    // Two runs sent two events, and `apply_damage` counted each one once.
    assert_eq!(world.resource::<DamageTaken>().unwrap().0, 10);
    Ok(())
}
```

Code outside any system, such as setup in `main`, can send with `World::send_event`.
Systems that run later still see these events, as long as they run within the lifetime:

```rust
world.send_event(Damage { entity: 3, amount: 20 });
```

A few more rules:

- A system that both sends and reads an event type also receives its own events.
- A system that runs for the first time sees every event from the last second.
- Each `send` and `read` locks only that event type's queue, and only for that call, so
  systems using different event types never block each other.
- To change how long events last, insert the queue yourself before anything sends:

```rust
use std::time::Duration;

world.insert_resource(EventQueue::<Damage>::with_lifetime(Duration::from_millis(250)));
```

### Reflection

`#[derive(Reflect)]` makes a type's fields readable and writable at runtime without
knowing its type. It is opt-in and works alongside `#[derive(Component)]` and
`#[derive(Resource)]`:

```rust
use anarchy::*;
use mutual::RelaxedMutex;

/// Where an entity is.
#[derive(Debug, Default, Component, Reflect)]
struct Transform {
    /// Position in world units.
    pos: (f32, f32),
    #[reflect(hint(min = 0.0, max = 10.0))]
    scale: f32,
}

fn main() -> anyhow::Result<()> {
    let world = World::default();
    world.insert(1, Box::new([RelaxedMutex::new(Box::new(Transform::default()) as DynComponent)]));

    // Edit by component id and path, as an editor would, without naming the type
    world.set_component_path(1, Transform::id(), "pos.0", &Value::Float(2.0))?;
    assert_eq!(world.get_component_path(1, Transform::id(), "pos")?, parse_value("(2.0, 0.0)")?);

    // Typed values are patched the same way
    let mut transform = Transform::default();
    transform.apply(&parse_value("{ scale: 3.0 }")?)?;
    assert_eq!(transform.scale, 3.0);
    Ok(())
}
```

- **`Value`** is untyped data: numbers, strings, lists, tuples, structs, enum variants
  and entity references. `to_value` reads a value, `apply` patches one (fields left out
  keep their values) and `FromValue::from_value` builds one. Missing fields come from each
  field's `fallback` (zero, empty, `None`), or from `Default` with `#[reflect(default)]`.
- **`TypeInfo`** (`Typed::type_info()`) describes fields, variants, doc comments and
  `#[reflect(hint(..))]` hints, which is what an editor needs to draw an inspector.
- **Paths** like `"pos.0"`, `"items[2].name"` or `"shape.radius"` reach nested fields with
  `get_path`, `set_path`, `path_ref` and `path_mut`.
- **The `TypeRegistry`** is a world resource that maps names and component or resource ids
  to their type info and to functions that build and cast them. Every non-generic type
  deriving `Reflect` registers itself through [`inventory`](https://docs.rs/inventory), and
  the derive works out on its own whether the type is a component or resource. Generic
  types need `World::register_component::<Foo<f32>>()` (or `register_resource` and
  `register_type`) for each concrete type.
- **Names** in scenes and `insert_reflected_component` can be a type's short name, the end
  of its path (`ui::Button`) when short names clash, or its full path.
- **Editing a world:** `entity_ids`, `component_ids` and `resource_ids` list what is there;
  `reflect_component(_mut)` and `reflect_resource(_mut)` lend a value as `dyn Reflect`;
  `get_component_path`, `set_component_path` and the resource versions edit by path;
  `insert_component_dyn`, `insert_reflected_component` and `remove_component_dyn` change
  an entity's components while keeping its hierarchy links. Every `set_*_path` sends a
  `ReflectEdit` event with the old and new values, for undo and live refresh.
- Fields holding entity ids can be marked `#[reflect(entity)]`, so saved scenes refer to
  those entities by name.
- With the `serde` feature, `Value` and `ReflectEdit` serialize both ways and `TypeInfo`
  serializes, so an editor in another process or a browser can talk to the world.
- With the `glam` feature, glam's `Vec2`/`Vec3`/`Vec4`, `IVec2`/`IVec3`, `UVec2`/`UVec3` and
  `Quat` implement `Reflect` as structs of their components (`{ x: 1.0, y: 2.0 }`), so
  components holding them can derive `Reflect`.

### Scenes

A `Scene` is a tree of entities, each a list of component patches over the type's
defaults, much like Bevy's BSN. Write one as text and spawn it through the registry:

```rust
use anarchy::*;

#[derive(Debug, Clone, Default, PartialEq, Component, Reflect)]
struct Transform { x: f32, y: f32 }

#[derive(Debug, Clone, PartialEq, Component, Reflect)]
struct Health(u32);

#[derive(Debug, Clone, PartialEq, Component, Reflect)]
struct Target(#[reflect(entity)] EntityID);

#[derive(Debug, Clone, Default, PartialEq, Resource, Reflect)]
struct Gravity { y: f32 }

fn main() -> anyhow::Result<()> {
    let world = World::default();

    let scene = Scene::parse(r#"
        @Gravity { y: -9.8 },          // a resource
        #Player                        // an entity named Player
        Transform { y: 2.0 }           // x keeps its default
        Health(100)
        Target(#Enemy)                 // refers to the entity named Enemy
        [
            #Sword Transform { x: 0.5 },
        ],
        #Enemy
        Health(50),
    "#)?;

    let mut next_id = 0;
    let spawned = scene.spawn(&world, || { next_id += 1; next_id })?;
    let (player, enemy) = (spawned.by_name["Player"], spawned.by_name["Enemy"]);
    assert_eq!(*world.get::<&Transform>(player)?.unwrap(), Transform { x: 0.0, y: 2.0 });
    assert_eq!(world.get::<&Target>(player)?.unwrap().0, enemy);
    assert_eq!(world.get::<&Name>(player)?.unwrap().0, "Player");

    // Save the entities back to text, keeping only what differs from the defaults
    let saved = Scene::from_world(&world, &spawned.roots)?;
    assert!(saved.scene.to_text()?.contains("Transform { y: 2.0 }"));
    Ok(())
}
```

The `scene!` macro takes the same format as Rust and checks it at compile time. Its
components don't need to be registered or even reflected:

```rust
let scene = scene! {
    #Player
    Transform { y: 2.0 }       // Transform::default() with y set
    Health(100)                // any Type(args) expression
    [
        #Sword
        { Transform { x: 0.5, y: 0.0 } },   // any expression in braces
    ],
};
```

- **Patches merge.** Two patches of the same type on one entity apply in order. In text,
  `Transform { pos: Vec2 { x: 1.0 } }` sets only `pos.x`, and type names inside values
  are optional.
- **Names.** `#name` adds a `Name` component and lets other entities refer to this one as
  `#name`. `SceneInstance::by_name` maps names to the spawned ids.
- **Inheritance.** `:base` starts an entity from the single root of the scene `base` in
  the world's `SceneLibrary` resource, and its children come first.
- **Spawning is all or nothing for entities.** Every entity is built before any is
  inserted, so a bad patch or a missing reference leaves the world's entities unchanged.
- **Ids** come from the closure passed to `spawn`, since anarchy leaves ids to the caller.
- **On the web**, registration runs from wasm constructors, which the linker normally
  calls for you. If reflected types seem to be missing, call `__wasm_call_ctors` once at
  startup, as the [`inventory` docs](https://docs.rs/inventory) describe.
- **Saving.** `Scene::from_world` saves reflected components as differences from their
  defaults, and lists the others in `SavedScene::skipped`. `Scene::to_text` writes
  text that `Scene::parse` reads back. Scenes from `scene!` can't be written as text.

### Sockets and the CLI

With the `socket` feature, a `WorldSocket` lets other programs inspect and edit a world
while it runs. The `anarchy` command line tool uses it, and editors can too:

```toml
anarchy = { git = "https://github.com/DaylightNebula/anarchy", features = ["socket"] }
```

```rust,no_run
use anarchy::*;

fn main() -> anyhow::Result<()> {
    let world = World::default();
    // TCP, or "unix:/tmp/game.sock" for a unix socket. Stops listening when dropped.
    let socket = WorldSocket::bind(&world, DEFAULT_SOCKET_ADDRESS)?;
    println!("listening on {}", socket.address());
    // ... run the app's systems as usual
    Ok(())
}
```

Then, from another terminal (`cargo install --path . --features socket` installs the tool):

```sh
anarchy ls                                  # entities, names, parents and components
anarchy show 1                              # one entity's component values
anarchy set 1 Health current 10             # a component field, in the scene value format
anarchy set @Gravity y -1.5                 # @ for resources
anarchy insert 1 Shield { amount: 5 }       # add or replace a component
anarchy spawn level.scene                   # spawn a text scene
anarchy save > world.scene                  # save every root entity as a scene
anarchy -a unix:/tmp/game.sock              # an interactive shell; `help` lists commands
```

- **The protocol** is one JSON `Request` per line, each answered by one JSON `Response`
  line, so any language can speak it. Types are named as in scenes and values are
  `Value`s in their serde form. `SocketClient` is a ready-made Rust client.
- **Requests run as they arrive** on the socket's threads, alongside running systems,
  with the same per-component and per-resource locks. Edits by path send `ReflectEdit`
  events, so the app can react to them or keep an undo history.
- **Other transports:** `World::handle_request` answers a `Request` directly, and
  `socket::serve` answers them over any `Read + Write` stream. `handle_request` also
  builds for the web, while `WorldSocket` and `SocketClient` are native only.
- **Spawned entities** get ids above the highest id in the world.
- **Only reflected types** can be read, edited or inserted. Others are listed by their
  `Debug` name with no value.
- **There is no authentication.** Anyone who can reach the socket can change the world,
  so keep TCP sockets on localhost, or use a unix socket.

## Concepts at a glance

| Concept | Types | Notes |
| --- | --- | --- |
| World | `World`, `WorldImpl`, `IndexedWorld`, `ListWorld` | `World` is a cheap, cloneable handle. It holds resources and the hierarchy, and derefs to its entity storage. |
| Entities | `EntityID`, `Entity`, `EntityHandle`, `DynComponents` | Ids are chosen by the caller. `WorldImpl::entity`, `get` and `remove` look an entity up by id. |
| Components | `Component`, `ComponentMeta`, `#[derive(Component)]` | Ids are a hash of the type's `TypeId`. |
| Resources | `Resource`, `ResourceMeta`, `Res`, `ResMut`, `#[derive(Resource)]` | One value per type. Code outside systems uses `World::resource` and `World::resource_mut`. |
| Hierarchy | `Hierarchy`, `Children` | Parent and child links by id, also a system param. |
| Events | `Event`, `EventQueue`, `EVENT_LIFETIME` | Each system has its own read cursor and sees its own events too. `World::send_event` sends from outside systems. |
| Tables | `Table`, `TableImpl`, `Cursor`, `SingleLinkedListTable` | One table per exact set of components. |
| Queries | `Query`, `QueryGroup`, `QueryComponent` | Terms are `&A`, `&mut A`, `Option<&A>` and `Option<&mut A>`, or tuples of up to 8 of them. |
| Systems | `System`, `IntoSystem`, `SystemParam` | Functions taking up to 16 `SystemParam`s. |
| Scheduling | `SystemGraph`, `SystemInstruction`, `SystemPin` | Build instructions with `before`, `after`, `before_meta`, `after_meta` and `pin`, and combine them with `and`. |
| Execution | `SingleThreadedExecutor`, `MultiThreadedExecutor`, `SharedExecutionState` | Pins run one after another and never overlap. |
| Work queues | `WorkQueueEntry`, `ParIter`, `SingleRun` | Submit with `SharedExecutionState::submit_work`. |
| Reflection | `Reflect`, `Typed`, `FromValue`, `Value`, `TypeInfo`, `#[derive(Reflect)]` | Read, build and patch values by field name. Paths with `get_path` and `set_path`. |
| Registry | `TypeRegistry`, `TypeRegistration`, `Name` | A world resource, filled automatically with every non-generic `Reflect` type. |
| Editing | `World::set_component_path`, `insert_reflected_component`, `ReflectEdit` | Type erased access to a world's components and resources by id, for editors. |
| Scenes | `Scene`, `SceneEntity`, `ScenePatch`, `SceneLibrary`, `SceneInstance`, `scene!` | Text or macro, spawned with `Scene::spawn`, saved with `Scene::from_world`. |
| Sockets | `WorldSocket`, `SocketClient`, `Request`, `Response`, `Target` | `socket` feature. JSON lines over TCP or a unix socket, used by the `anarchy` CLI. |

## AI assistant skills

This repository is also a Claude Code plugin marketplace. Its `anarchy` plugin teaches
Claude how to use the crate correctly, including the locking rules that are easy to get
wrong. To install it for a project that depends on anarchy:

```sh
claude plugin marketplace add DaylightNebula/anarchy
claude plugin install anarchy@anarchy
```

The plugin has four skills:

| Skill | Covers |
| --- | --- |
| `anarchy:world` | Setup, components, resources, entities, queries, entity handles and the hierarchy |
| `anarchy:systems` | System params, the system graph, executors, events, work queues and custom params |
| `anarchy:scenes` | Reflection, the type registry, editing a world by path, and text and macro scenes |
| `anarchy:recipes` | Id allocation, spawn helpers, adding and removing components, transform propagation, setup functions, game loops and parallel work |

The skills are plain Markdown in [`plugins/anarchy/skills/`](plugins/anarchy/skills), so
other AI tools can read them too. Point the tool at those files, or copy them into its
rules or context directory. Contributors working on the crate itself get a separate skill
in `.claude/skills/anarchy`.

## Development

```sh
cargo test               # unit tests and doctests
cargo test --features serde   # also the serde support for reflection
cargo test --features glam    # also Reflect for glam types
cargo test --features socket  # also the socket and the CLI
cargo bench              # compares ListWorld and IndexedWorld query and insert speed
cargo doc --no-deps --open
cargo test --target wasm32-unknown-unknown   # tests/wasm.rs in Node (needs rustup's wasm target and wasm-bindgen-cli)
```

The derive macros live in the `macros/` crate (`anarchy_macros`). Use them through the
re-exports in `anarchy`, since the generated code refers to `::anarchy`.
