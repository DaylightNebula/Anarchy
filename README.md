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
- **Events.** `Event<E>` sends events that every system reads exactly once, as long as it
  runs within a second of the send.
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

The same example is a doctest in the crate docs, so it is checked by `cargo test`.

To run the graph on several threads, and keep running it until a flag is cleared:

```rust
use std::sync::atomic::AtomicBool;

let running = AtomicBool::new(true);
// Runs Start, Normal and End pins in order on 4 threads, looping while `running` is true.
MultiThreadedExecutor::new().run(&world, &graph, 4, &running)?;
```

## Events

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

## Concepts

| Concept | Types | Notes |
| --- | --- | --- |
| World | `World`, `WorldImpl`, `IndexedWorld`, `ListWorld` | `World` is a cheap, cloneable handle. It holds resources and derefs to its entity storage. |
| Components | `Component`, `ComponentMeta`, `#[derive(Component)]` | Ids are a hash of the type's `TypeId`. Entity ids are chosen by the caller. |
| Resources | `Resource`, `ResourceMeta`, `Res`, `ResMut`, `#[derive(Resource)]` | One value per type. Systems read them with `Res<R>` and write them with `ResMut<R>`. Code outside systems uses `World::resource` and `World::resource_mut`. |
| Events | `Event`, `EventQueue`, `EVENT_LIFETIME` | `Event<E>::send` and `Event<E>::read`. Each system has its own read cursor and sees its own events too. `World::send_event` sends from outside systems. |
| Tables | `Table`, `TableImpl`, `Cursor`, `SingleLinkedListTable` | One table per exact set of components. |
| Queries | `Query`, `QueryGroup`, `QueryComponent` | Terms are `&A`, `&mut A`, `Option<&A>` and `Option<&mut A>`, or tuples of up to 8 of them. |
| Systems | `System`, `IntoSystem`, `SystemParam` | Functions taking up to 16 `SystemParam`s: `()`, `World`, `Query`, `Event`, `Res` and `ResMut`. `SystemParam::extract` is given the running system's key and can fail (for example, `Res<R>` when the world holds no `R`), in which case the system returns that error without running. |
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
