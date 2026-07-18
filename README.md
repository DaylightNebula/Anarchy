# Anarchy

**Status: work in progress.** APIs are unstable and may change or break without notice.
There are no published releases, few unit tests, and some pieces (e.g.
`BSTWorldDatabase`) are unfinished experiments. This crate is primarily developed and
exercised inside a game engine being built in the background; it's published so that
work can be seen and reused, not because it is production-ready.

Anarchy is an experimental ECS (Entity Component System) crate aiming for the highest
possible degree of concurrency in an ECS `World`. Rather than locking whole tables or
the world while a system runs, it locks data at the lowest point that's actually
needed — generally the individual component or resource — so unrelated systems can run
against the same world at the same time.

## Adding the dependency

Anarchy's derive macros (`Component`, `Resource`, `Event`) generate code that refers to
the `mutual` crate directly, so it needs to be a dependency alongside `anarchy` itself:

```toml
[dependencies]
anarchy = { git = "https://github.com/DaylightNebula/anarchy" }
mutual = "1.0.2"
```

## Core concepts

- **`World`** — holds all entities/components (via a `LinearDatabase`) plus a map of
  singleton **resources**.
- **Components** — plain structs deriving `Component`, attached to entities.
- **Resources** — plain structs deriving `Resource`, singletons shared across systems.
- **Events** — plain structs deriving `Event`, broadcast and read by systems, expiring
  automatically about a second after being sent.
- **Systems** — plain functions annotated with `#[system]`, whose arguments describe
  what they need from the `World` (a `Query`, a `Res`/`ResMut`, an `Event`, ...).
- **Schedules** — an ordered, priority-sorted set of systems, run repeatedly by a
  `Scheduler` at a configured tick rate, with as many threads as you allow.

## Quick start: components, queries, and a system

```rust
use anarchy::*;
use anarchy::macros::{Component, system};

#[derive(Component)]
struct Position { x: f32, y: f32 }

#[derive(Component)]
struct Velocity { x: f32, y: f32 }

#[system]
fn move_system(query: Query<(&mut Position, &Velocity)>) {
    for (mut pos, vel) in query.as_iter() {
        pos.x += vel.x;
        pos.y += vel.y;
    }
}

fn main() {
    let world = World::new();

    world.insert(
        EntityBuilder::default()
            .add(Position { x: 0.0, y: 0.0 })
            .add(Velocity { x: 1.0, y: 0.5 })
            .build()
    );

    // one tile containing one system, run every tick
    let schedule = Schedule::from_iter(
        std::iter::empty(),
        std::iter::once(ScheduleTile::new(vec![Box::new(move_system)]))
    );

    // starts the schedule's own background thread(s) at 60 ticks/second
    Scheduler::schedule(
        ScheduleID { id: "main", tick_rate: 60, max_threads: 1 },
        schedule,
        world
    );

    std::thread::sleep(std::time::Duration::from_secs(1));
}
```

A query's generic argument is a single `&A` / `&mut A` / `Option<&A>` / `Option<&mut A>`,
or a tuple of up to 20 of them, one per component you want from each matching entity.

## Resources

Resources are singletons attached to the `World`, read with `Res<T>` and read/written
with `ResMut<T>`:

```rust
use anarchy::*;
use anarchy::macros::{Component, Resource, system};

#[derive(Resource, Default)]
struct FrameCount(u32);

#[derive(Component)]
struct Velocity { x: f32, y: f32 }

#[system]
fn count_frames(mut frames: ResMut<FrameCount>) {
    frames.0 += 1;
}

fn main() {
    let world = World::new();
    world.insert_resource(FrameCount::default());
    // ... schedule `count_frames` as shown above
}
```

## Events

Events are broadcast with `Event::write` and read with `Event::read`. Each reader needs
its own `'static` `EventSystemMinIDTracker` so repeated reads only return events it
hasn't seen yet:

```rust
use std::sync::OnceLock;
use anarchy::*;
use anarchy::macros::{Event, system};

#[derive(Event, Debug)]
struct Collided(EntityID, EntityID);

static COLLIDED_TRACKER: EventSystemMinIDTracker = OnceLock::new();

#[system]
fn log_collisions(events: Event<Collided>) {
    for collision in events.read(&COLLIDED_TRACKER) {
        println!("collided: {:?}", *collision);
    }
}

#[system]
fn detect_collisions(events: Event<Collided>) {
    events.write(Collided(0, 1));
}
```

## Executors

By default (native targets, without the `single-threaded-executors` feature), a
scheduled `Schedule` runs on its own pool of OS threads, one tick at a time, sized by
`ScheduleID::max_threads`. On `wasm32`, or with the `single-threaded-executors` feature
enabled, schedules instead queue their work and are advanced by calling
`Scheduler::tick_tasks()` yourself (e.g. once per frame).

## Features

- `single-threaded-executors` — run all schedules/tasks cooperatively via
  `Scheduler::tick_tasks()` instead of spawning OS threads.
- `multi-threaded-executors` — force the multithreaded executors even on `wasm32`.
- `log-trace` — raise the crate's internal `log!`/`trace!` logging up to `TRACE` level.

## Testing

This crate has some unit tests per module, but is mostly exercised indirectly through
the game engine it was built for, rather than through a broad standalone test suite or
examples directory — expect gaps.
