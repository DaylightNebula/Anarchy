---
name: systems
description: Use when writing Rust code with the anarchy ECS crate that defines or runs logic - function systems, system parameters (Query, Res, ResMut, Event, Hierarchy, World), ordering systems in a SystemGraph with before/after/pins/metadata, running them with SingleThreadedExecutor or MultiThreadedExecutor, sending and reading events, or splitting work across threads with ParIter. For data setup see the anarchy world skill.
---

# anarchy: systems, scheduling and events

## Function systems

Any function whose parameters are all `SystemParam`s is a system. It can take up to 16
parameters and must return `()`.

| Parameter | Gives |
| --- | --- |
| `Query<(&'static A, &'static mut B)>` | every entity with those components. Borrowed terms need `'static` here |
| `Res<R>` / `ResMut<R>` | read or write access to resource `R` |
| `Event<E>` | send and read events of type `E` |
| `Hierarchy` | the world's parent and child links |
| `World` | the whole world (a cheap clone) |
| `()` | nothing |

```rust
use anarchy::*;

fn gravity(mut q: Query<(&'static mut Velocity, &'static Mass)>) {
    while let Some((_id, (mut vel, mass))) = q.next().unwrap() {
        vel.0 -= 9.8 * mass.0;
    }
}

fn score(mut score: ResMut<Score>, hits: Event<Hit>) {
    for _ in hits.read() { score.0 += 1; }
}
```

- **Failing extraction.** If a parameter can't be built (for example `Res<R>` when `R` is
  missing), the system doesn't run and the executor returns that error. Insert resources
  before running.
- **Deadlock:** `Res<R>` plus `ResMut<R>` (or two `ResMut<R>`) for the same `R` in one
  system deadlocks. So does a system that holds `ResMut<R>` and calls
  `world.resource::<R>()` through a `World` param.
- **A system's identity is its function type.** Appending the same function again replaces
  it. To run the same logic twice, wrap it in two functions.
- Systems can't return errors. Use `.unwrap()`/`.expect()` inside, or implement `System`
  by hand (below) to return `anyhow::Result`.

## Building the graph

```rust
let mut graph = SystemGraph::new();
graph.append_system(input,   pin(SystemPin::Start), std::iter::empty())?;
graph.append_system(gravity, after(input),          std::iter::empty())?;
graph.append_system(physics, and(after(gravity), before(render)), std::iter::empty())?;
graph.append_system(render,  pin(SystemPin::End),   std::iter::empty())?;
```

- `before(f)` and `after(f)` order this system against another one.
- `pin(SystemPin::Start | Normal | End)` sets the phase, and `Normal` is the default. The
  three pins run one after another and never overlap.
- `before_meta(tags)` and `after_meta(tags)` order this system against every system tagged
  with one of `tags`. Tag a system through the third argument of `append_system`. A tag is
  any `TypeId`:

  ```rust
  struct PhysicsSet;
  let physics_tag = std::any::TypeId::of::<PhysicsSet>();
  graph.append_system(gravity, SystemInstruction::default(), [physics_tag].into_iter())?;
  graph.append_system(render, after_meta([physics_tag]), std::iter::empty())?;
  ```

- `and(a, b)` combines two instructions. `SystemInstruction::default()` means no
  constraints.
- `append_system` returns `Err`, and leaves the system out, if the system would create a
  cycle or contradict a pin. Always use `?`.
- `graph.remove_system(TypeId)` and `graph.remove_by_meta(tag)` remove systems again.
- Systems without an ordering between them may run at the same time on the multi-threaded
  executor. Add `before`/`after` wherever order matters.

## Running

```rust
// One pass of every pin on this thread. Good for tests, tools and fixed-step loops.
let exec = SingleThreadedExecutor::new();
exec.run(&world, &graph)?;

// `threads` threads (the caller counts as one). Runs the graph again and again
// until `running` is false at the end of a pass.
use std::sync::atomic::{AtomicBool, Ordering};
let running = AtomicBool::new(true);
MultiThreadedExecutor::new().run(&world, &graph, 4, &running)?;
```

- Reuse one executor for many runs. Event read cursors are kept per system, not per
  executor, so creating a new executor doesn't replay events.
- The multi-threaded `run` blocks until it stops. To stop it, clear the flag from a system
  or another thread (see the recipes skill). If a system fails, the current pass finishes
  and `run` returns the first error.

## Events

```rust
#[derive(Clone, Debug)]
struct Hit { target: EntityID, damage: u32 }

fn attack(hits: Event<Hit>) { hits.send(Hit { target: 7, damage: 5 }); }
fn apply(hits: Event<Hit>)  { for hit in hits.read() { /* ... */ } }

world.send_event(Hit { target: 3, damage: 1 }); // from outside any system
```

- Any `Clone + Send + 'static` type can be an event. No derive is needed.
- Every system that reads `E` has its own cursor, so every reader sees every event exactly
  once, oldest first. A system also receives events it sent itself.
- Events expire after `EVENT_LIFETIME` (1 second). A reader that runs less often than that
  misses events. To change the lifetime, insert the queue before anything sends:
  `world.insert_resource(EventQueue::<Hit>::with_lifetime(Duration::from_secs(5)))`.
- Order the readers `after` the senders if they need this pass's events. Otherwise they
  see them on the next pass.

## Hand-written systems and shared work

Implement `System<(), ()>` to return errors, or to reach the `SharedExecutionState` and
hand work to idle threads:

```rust
struct Physics;

impl System<(), ()> for Physics {
    fn run(&self, world: &World, exec: &SharedExecutionState) -> anyhow::Result<()> {
        let world = world.clone();
        let ids: Vec<EntityID> = (0..1000).collect();
        exec.submit_work(ParIter::new(ids.into_iter(), move |id| {
            if let Ok(Some(mut pos)) = world.get::<&mut Position>(id) { pos.x += 1.0; }
        }));
        while exec.exec_from_work_queue() {} // help until no work is left to claim
        Ok(())
    }
}

graph.append_raw(std::any::TypeId::of::<Physics>(), Box::new(Physics),
                 SystemInstruction::default(), std::iter::empty())?;
```

- `ParIter::new(iter, f)` calls `f` on each item, with any thread claiming the next item.
  `SingleRun::new(f)` runs `f` once.
- When `exec_from_work_queue` returns `false`, there's nothing left to claim, but other
  threads may still be finishing items. If you need every item done before returning,
  count them with an `AtomicUsize` and wait for the count.
- `ParIter`'s closure must be `Send + Sync`, so capture a `World` clone, not a `Query`.
- `before(f)` / `after(f)` only take functions. To order against a hand-written system,
  tag it with metadata through `append_raw`'s last argument and use
  `before_meta`/`after_meta`.

## Custom system parameters

```rust
struct Time(f32);

impl SystemParam for Time {
    fn extract(world: &World, _exec: &SharedExecutionState, _system: SystemKey) -> anyhow::Result<Self> {
        Ok(Time(world.resource::<DeltaTime>()?.0))
    }
}
```

`extract` runs every time the system runs. Returning `Err` skips the system and surfaces
the error from the executor. `system` is the running system's key, which is useful for
per-system state, as `Event` uses it.

## Testing systems

Build a world and a graph, run one pass with `SingleThreadedExecutor`, then assert on the
world:

```rust
#[test]
fn gravity_pulls_down() -> anyhow::Result<()> {
    let world = World::default();
    world.insert(1, Box::new([
        mutual::RelaxedMutex::new(Box::new(Velocity(0.0)) as DynComponent),
        mutual::RelaxedMutex::new(Box::new(Mass(1.0)) as DynComponent),
    ]));
    let mut graph = SystemGraph::new();
    graph.append_system(gravity, SystemInstruction::default(), std::iter::empty())?;
    SingleThreadedExecutor::new().run(&world, &graph)?;
    assert!(world.get::<&Velocity>(1)?.unwrap().0 < 0.0);
    Ok(())
}
```
