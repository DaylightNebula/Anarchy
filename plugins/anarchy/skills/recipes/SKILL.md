---
name: recipes
description: Ready-made patterns for apps built on the anarchy ECS crate - allocating unique entity ids, spawn helpers, adding or removing a component on an existing entity, propagating transforms down the hierarchy, grouping systems into reusable setup functions, game loops with the multi-threaded executor, stopping the loop from a system, and parallel per-entity work. Use when structuring an application or game around anarchy.
---

# anarchy: recipes

Each recipe uses `use anarchy::*;`, `use mutual::RelaxedMutex;` and `anyhow::Result`. The
world and systems skills cover the basic API these recipes build on.

## Unique entity ids

anarchy never picks ids for you. Keep a counter resource:

```rust
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default, Resource)]
struct NextId(AtomicU64);

fn next_id(world: &World) -> EntityID {
    world.init_resource_with(NextId::default);
    world.resource::<NextId>().unwrap().0.fetch_add(1, Ordering::Relaxed)
}
```

`resource` (not `resource_mut`) is enough here, because the atomic does the mutation. That
also means systems on many threads can allocate ids at once without blocking each other.

## Spawn helper

```rust
fn spawn(world: &World, components: Vec<DynComponent>) -> EntityID {
    let id = next_id(world);
    world.insert(id, components.into_iter().map(RelaxedMutex::new).collect());
    id
}

let player = spawn(&world, vec![Box::new(Position { x: 0.0, y: 0.0 }), Box::new(Health(100))]);
let sword = spawn(&world, vec![Box::new(Position { x: 0.0, y: 0.0 })]);
world.handle(sword).set_parent(player)?;
```

## Add or remove a component on an existing entity

An entity's component set is fixed when it is inserted. To change it, take the entity out
and insert it again with the same id. The removed `DynComponents` can be reused directly,
so the values aren't copied. Removal drops the entity's hierarchy links, so save and
restore them:

```rust
use mutual::SharedData; // for lock_ref

/// Rebuilds `id` with `edit` applied to its component list, keeping its hierarchy links.
fn rebuild(
    world: &World,
    id: EntityID,
    edit: impl FnOnce(&mut Vec<RelaxedMutex<DynComponent>>),
) -> anyhow::Result<()> {
    let hierarchy = world.hierarchy();
    let parent = hierarchy.parent(id);
    let children = hierarchy.children(id).map(|c| c.to_vec()).unwrap_or_default();

    let mut components = world.remove(id)
        .ok_or_else(|| anyhow::anyhow!("no entity {id}"))?
        .into_vec();
    edit(&mut components);
    world.insert(id, components.into_boxed_slice());

    if let Some(parent) = parent { hierarchy.set_parent(id, parent)?; }
    for child in children { hierarchy.set_parent(child, id)?; }
    Ok(())
}

fn add_component(world: &World, id: EntityID, component: DynComponent) -> anyhow::Result<()> {
    rebuild(world, id, |c| c.push(RelaxedMutex::new(component)))
}

fn remove_component<C: ComponentMeta>(world: &World, id: EntityID) -> anyhow::Result<()> {
    rebuild(world, id, |c| c.retain(|comp| comp.lock_ref().get_id() != C::id()))
}
```

- Every change moves the entity to another table, so do it outside hot loops.
- Removing the last component leaves an empty entity, which `insert` doesn't allow. Check
  for this first, and despawn instead.
- Don't run this while a guard on one of the entity's components is held on the same
  thread.

## Transform propagation down the hierarchy

Store local values on every entity, and compute global values from the roots down:

```rust
#[derive(Debug, Component)] struct Local(f32);
#[derive(Debug, Component)] struct Global(f32);

fn propagate(world: World, hierarchy: Hierarchy, mut roots: Query<(&'static Local, &'static mut Global)>) {
    while let Some((id, (local, mut global))) = roots.next().unwrap() {
        if hierarchy.parent(id).is_some() { continue } // only start at roots
        global.0 = local.0;
        let mut stack = vec![(id, global.0)];
        drop(global); // release the root before walking its subtree
        while let Some((parent, parent_global)) = stack.pop() {
            let Some(children) = hierarchy.children(parent).map(|c| c.to_vec()) else { continue };
            for child in children {
                if let Ok(Some((local, mut global))) = world.get::<(&Local, &mut Global)>(child) {
                    global.0 = parent_global + local.0;
                    stack.push((child, global.0));
                }
            }
        }
    }
}
```

The query visits children too, which is why it skips any entity with a parent. Copying
`children` out with `.to_vec()` means no hierarchy lock is held while components are
locked.

## Group systems into setup functions

Give each feature one function that inserts its resources and appends its systems, tagged
so that other features can order against the whole group:

```rust
pub struct PhysicsSet;

pub fn physics_setup(world: &World, graph: &mut SystemGraph) -> anyhow::Result<()> {
    let tag = std::any::TypeId::of::<PhysicsSet>();
    world.init_resource_with(|| Gravity(9.8));
    graph.append_system(gravity, SystemInstruction::default(), [tag].into_iter())?;
    graph.append_system(integrate, after(gravity), [tag].into_iter())?;
    Ok(())
}

pub fn render_setup(world: &World, graph: &mut SystemGraph) -> anyhow::Result<()> {
    let _ = world;
    let physics = std::any::TypeId::of::<PhysicsSet>();
    graph.append_system(render, after_meta([physics]), std::iter::empty())?;
    Ok(())
}

let world = World::default();
let mut graph = SystemGraph::new();
physics_setup(&world, &mut graph)?;
render_setup(&world, &mut graph)?;
```

## Game loop

A fixed-step loop on one thread:

```rust
let exec = SingleThreadedExecutor::new();
let step = std::time::Duration::from_millis(16);
loop {
    let start = std::time::Instant::now();
    exec.run(&world, &graph)?;
    if world.resource::<Quit>()?.0 { break }
    std::thread::sleep(step.saturating_sub(start.elapsed()));
}
```

On many threads, `MultiThreadedExecutor::run` loops by itself until its flag is cleared.
To stop it from inside a system, keep the flag in a `static`:

```rust
use std::sync::atomic::{AtomicBool, Ordering};

static RUNNING: AtomicBool = AtomicBool::new(true);

fn quit_on_escape(input: Res<Input>) {
    if input.escape { RUNNING.store(false, Ordering::Release); }
}

let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
MultiThreadedExecutor::new().run(&world, &graph, threads, &RUNNING)?;
```

To stop it from another thread, run it in `std::thread::scope` and clear a local
`AtomicBool` from the other thread. The multi-threaded loop doesn't sleep between passes,
so add a frame-limiting system in `SystemPin::End` if you need a fixed rate.

## Parallel work inside one system

Collect the ids first, then let the executor's idle threads share the per-entity work:

```rust
struct HeavyAi;

impl System<(), ()> for HeavyAi {
    fn run(&self, world: &World, exec: &SharedExecutionState) -> anyhow::Result<()> {
        let mut ids = vec![];
        let mut q = Query::<&Brain>::new(world);
        while let Some((id, _)) = q.next()? { ids.push(id); }

        let world = world.clone();
        exec.submit_work(ParIter::new(ids.into_iter(), move |id| {
            if let Ok(Some(mut brain)) = world.get::<&mut Brain>(id) { brain.think(); }
        }));
        while exec.exec_from_work_queue() {}
        Ok(())
    }
}
```

This only pays off when the per-entity work is expensive. For cheap updates, a plain query
loop is faster.

## Request and response with events

```rust
#[derive(Clone, Debug)] struct SpawnRequest { x: f32 }
#[derive(Clone, Debug)] struct Spawned(EntityID);

fn spawner(world: World, requests: Event<SpawnRequest>, spawned: Event<Spawned>) {
    for req in requests.read() {
        let id = spawn(&world, vec![Box::new(Position { x: req.x, y: 0.0 })]);
        spawned.send(Spawned(id));
    }
}
```

Order consumers of `Spawned` with `after(spawner)` so they see the new entities in the same
pass.
