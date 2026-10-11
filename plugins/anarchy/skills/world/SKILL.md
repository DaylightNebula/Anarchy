---
name: world
description: Use when writing Rust code that depends on the anarchy ECS crate and touches its data side - adding the dependency, defining components and resources, creating a World, inserting, looking up or removing entities, running queries, using EntityHandle, or parenting entities with the Hierarchy. For systems, scheduling, executors and events see the anarchy systems skill; for larger patterns see the anarchy recipes skill.
---

# anarchy: worlds, entities and data

anarchy is an experimental, highly concurrent Rust ECS. Every component and every resource
has its own lock, so systems only block each other when they touch the same value. The API
is still changing. If this skill and the compiler disagree, trust the compiler and read
the crate docs (`cargo doc -p anarchy --open`).

## Setup

```toml
[dependencies]
anarchy = { git = "https://github.com/DaylightNebula/anarchy" }
mutual = "1.0.2"   # for RelaxedMutex, needed to build entities
anyhow = "1"       # every fallible anarchy call returns anyhow::Result
```

You need Rust 1.88 or newer (edition 2024). Import everything with `use anarchy::*;`.

## Components and resources

```rust
use anarchy::*;

#[derive(Debug, Component)]
struct Position { x: f32, y: f32 }

#[derive(Debug, Resource)]
struct Score(u32);
```

- Both derives require the type to be `Debug + Send + 'static`.
- A component's id is a hash of its `TypeId`, available as `Position::id()`.
- Always use the derives through `anarchy`, never by depending on `anarchy_macros`
  directly, because the generated code refers to `::anarchy`.

## The world

```rust
let world = World::default();               // IndexedWorld storage, the right default
let world = World::new(ListWorld::new());   // simpler storage, slower with many tables
```

`World` is an `Arc` handle. Cloning it is cheap, and clones share the same data, so pass
clones to threads freely. It holds entities, resources and the hierarchy.

## Entities

Entity ids are `EntityID` (`u64`). **You choose them**, and anarchy never checks that they
are unique, so keep your own counter (see the recipes skill). Each component is boxed as a
`DynComponent` and wrapped in its own `RelaxedMutex`. An entity needs at least one
component.

```rust
use mutual::RelaxedMutex;

world.insert(1, Box::new([
    RelaxedMutex::new(Box::new(Position { x: 0.0, y: 0.0 }) as DynComponent),
    RelaxedMutex::new(Box::new(Velocity(1.0)) as DynComponent),
]));

// From a Vec<DynComponent>:
let components: Vec<DynComponent> = vec![Box::new(Position { x: 1.0, y: 2.0 })];
world.insert(2, components.into_iter().map(RelaxedMutex::new).collect());
```

Look up one entity by id. Each of these is a single hash lookup:

```rust
// Ok(None) if there is no entity 1. Err if it lacks a requested component.
if let Some((mut pos, vel)) = world.get::<(&mut Position, &Velocity)>(1)? {
    pos.x += vel.0;
}

let entity: Option<Entity> = world.entity(1);         // .group (sorted ids) and .components
let removed: Option<DynComponents> = world.remove(1); // also unlinks it from the hierarchy
```

You can't add or remove one component on an existing entity. Instead, `remove` the entity
and `insert` it again with the new set (see the recipes skill).

## Queries

`Query<QG>` visits every entity that has the components in `QG`. A term is `&A`, `&mut A`,
`Option<&A>` or `Option<&mut A>`, and `QG` is one term or a tuple of up to 8.

```rust
let mut query = Query::<(&mut Position, &Velocity)>::new(&world);
while let Some((id, (mut pos, vel))) = query.next()? {
    pos.x += vel.0;
}
```

- `next()` returns `anyhow::Result<Option<(EntityID, Output)>>`. It is not an `Iterator`,
  so use `while let`.
- Each component is locked as it is handed out and unlocked when its guard drops. Don't
  collect guards into a `Vec` if another thread may need the same components.
- Inside a system parameter list, write `Query<(&'static mut A, &'static B)>`.
- **Limitation:** `Query::new` requires every listed component, `Option` ones included, so
  `Option<&A>` never yields `None` from `Query::new`. To handle "has A or not", run two
  queries, or use `world.get::<Option<&A>>(id)` on known ids.

## Resources

One value per type, stored on the world:

```rust
world.insert_resource(Score(0));          // insert or replace
world.init_resource_with(|| Score(0));    // insert only if missing, safe to race
world.resource_mut::<Score>()?.0 += 10;   // Err if missing. Blocks while any other guard is held
let score = world.resource::<Score>()?.0; // blocks while a mutable guard is held
world.has_resource::<Score>();
world.remove_resource::<Score>();
```

In systems, take `Res<Score>` or `ResMut<Score>` instead (see the systems skill).

## Entity handles

`world.handle(id)` returns an `EntityHandle`, an id paired with its world. It is `Copy`,
creating one costs nothing, and nothing is looked up until you call a method:

```rust
let player = world.handle(1);
player.get::<&Position>()?;                 // same as world.get
player.parent();                            // Option<EntityHandle>
player.children();                          // Option<Children> guard, see below
player.ancestors(); player.descendants();   // iterators of EntityHandle
player.set_parent(0)?; player.add_child(5)?; player.remove_parent();
player.despawn();                           // remove, children become roots
player.despawn_recursive();                 // remove it and its whole subtree
let id: EntityID = player.into();
```

## Hierarchy

Every world has one `Hierarchy` of parent and child links by id. Get it with
`world.hierarchy()`, through a handle, or as a system parameter.

```rust
let h = world.hierarchy();
h.set_parent(child, parent)?;   // moves child from any old parent. Err if it would make a cycle
h.add_child(parent, child)?;    // the same thing, with the arguments the other way round
h.parent(child);                // Option<EntityID>
h.children(parent);             // Option<Children>, derefs to &[EntityID] in insertion order
h.ancestors(id);                // parent first, up to the root
h.descendants(id);              // depth first, each parent before its children
h.remove_parent(child);
```

- Links don't move entities between tables, and anarchy doesn't check that the entities
  exist.
- **`Children` is a read-lock guard.** Copy it out (`.to_vec()`) or drop it before calling
  `set_parent`, `remove_parent`, `remove` or `despawn` on the same thread, or the call
  deadlocks.
- `world.remove(id)` turns the removed entity's children into roots, and
  `despawn_recursive` removes the whole subtree.

## Rules that prevent deadlocks and surprises

1. Locks are not reentrant. Never hold a guard (component, `resource_mut`, `Children`)
   while locking the same thing again on that thread.
2. Keep guards short-lived. Wrap them in a block or `drop` them before calling into other
   code.
3. Ids are yours to keep unique. If you reuse an id, the by-id lookup sees only the last
   insert, but queries still see every copy.
4. A missing resource is an `Err`, not a panic. Propagate it with `?`.
