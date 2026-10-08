# Anarchy V2

An experimental ECS crate that allows for the highest possible degree of concurrency
in an ECS world while systems run rampant through it. This is achieved by only locking data
at the lowest point needed for a general purpose ECS, which is generally at the component
or resource level. This also enables faster data processing through queries whose work can
be shared across threads.

> **Status:** experimental and under active development. The API will change.

## Features

- **Per-component locking.** Every component and resource sits behind its own lock, so
  systems only block each other when they touch the same data.
- **Lock-free tables.** Entities with the same set of components share a table, a lock-free
  linked list that many threads can walk, insert into and pop from at once.
- **Indexed queries.** The default `IndexedWorld` indexes tables by component, so a query
  only visits tables that hold its rarest required component.
- **Function systems.** Plain functions become systems. Their arguments are `SystemParam`s
  such as `World` or `Query<...>`.
- **Dependency graph.** Order systems with `before`, `after`, metadata tags and
  `Start`/`Normal`/`End` pins. The graph rejects cycles and contradictory pins.
- **Shared work queues.** A system can split work into a `ParIter` and submit it, and idle
  executor threads help finish it.

## Getting started

The crate is not on crates.io yet, so add it as a git dependency. Building `DynComponents`
by hand also needs `mutual`:

```toml
[dependencies]
anarchy = { git = "https://github.com/DaylightNebula/anarchy", branch = "v2" }
mutual = "1.0.2"
```

It uses the 2024 edition and `let` chains, so it needs Rust 1.88 or newer.

## Example

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

fn tick(world: World) {
    world.resource_mut::<Ticks>().unwrap().0 += 1;
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

The same example is a doctest in the crate docs, so it is checked by `cargo test`.

To run the graph on several threads, and keep running it until a flag is cleared:

```rust
use std::sync::atomic::AtomicBool;

let running = AtomicBool::new(true);
// Runs Start, Normal and End pins in order on 4 threads, looping while `running` is true.
MultiThreadedExecutor::new().run(&world, &graph, 4, &running)?;
```

## Concepts

| Concept | Types | Notes |
| --- | --- | --- |
| World | `World`, `WorldImpl`, `IndexedWorld`, `ListWorld` | `World` is a cheap, cloneable handle. It holds resources and derefs to its entity storage. |
| Components | `Component`, `ComponentMeta`, `#[derive(Component)]` | Ids are a hash of the type's `TypeId`. Entity ids are chosen by the caller. |
| Resources | `Resource`, `ResourceMeta`, `#[derive(Resource)]` | One value per type, accessed with `World::resource` and `World::resource_mut`. |
| Tables | `Table`, `TableImpl`, `Cursor`, `SingleLinkedListTable` | One table per exact set of components. |
| Queries | `Query`, `QueryGroup`, `QueryComponent` | Terms are `&A`, `&mut A`, `Option<&A>` and `Option<&mut A>`, or tuples of up to 8 of them. |
| Systems | `System`, `IntoSystem`, `SystemParam` | Functions taking up to 16 `SystemParam`s. |
| Scheduling | `SystemGraph`, `SystemInstruction`, `SystemPin` | Build instructions with `before`, `after`, `before_meta`, `after_meta` and `pin`, and combine them with `and`. |
| Execution | `SingleThreadedExecutor`, `MultiThreadedExecutor`, `SharedExecutionState` | Pins run one after another and never overlap. |
| Work queues | `WorkQueueEntry`, `ParIter`, `SingleRun` | Submit with `SharedExecutionState::submit_work`. |

`Query::new` asks the world for every listed component, `Option` ones included. For now
that means an optional component only resolves to `None` when the query is built with
`Query::from_iter` over tables that lack it.

## Development

```sh
cargo test               # unit tests and doctests
cargo bench              # compares ListWorld and IndexedWorld query and insert speed
cargo doc --no-deps --open
```

The derive macros live in the `macros/` crate (`anarchy_macros`). Use them through the
re-exports in `anarchy`, since the generated code refers to `::anarchy`.
