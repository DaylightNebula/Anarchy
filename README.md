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
- [Guide](#guide)
  - [Worlds and entities](#worlds-and-entities)
  - [Queries](#queries)
  - [Resources](#resources)
  - [Hierarchy](#hierarchy)
  - [Systems and scheduling](#systems-and-scheduling)
  - [Running systems](#running-systems)
  - [Events](#events)
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

## AI assistant skills

This repository is also a Claude Code plugin marketplace. Its `anarchy` plugin teaches
Claude how to use the crate correctly, including the locking rules that are easy to get
wrong. To install it for a project that depends on anarchy:

```sh
claude plugin marketplace add DaylightNebula/anarchy
claude plugin install anarchy@anarchy
```

The plugin has three skills:

| Skill | Covers |
| --- | --- |
| `anarchy:world` | Setup, components, resources, entities, queries, entity handles and the hierarchy |
| `anarchy:systems` | System params, the system graph, executors, events, work queues and custom params |
| `anarchy:recipes` | Id allocation, spawn helpers, adding and removing components, transform propagation, setup functions, game loops and parallel work |

The skills are plain Markdown in [`plugins/anarchy/skills/`](plugins/anarchy/skills), so
other AI tools can read them too. Point the tool at those files, or copy them into its
rules or context directory. Contributors working on the crate itself get a separate skill
in `.claude/skills/anarchy`.

## Development

```sh
cargo test               # unit tests and doctests
cargo bench              # compares ListWorld and IndexedWorld query and insert speed
cargo doc --no-deps --open
```

The derive macros live in the `macros/` crate (`anarchy_macros`). Use them through the
re-exports in `anarchy`, since the generated code refers to `::anarchy`.
