//! Worlds, entities, components, queries and resources.
//!
//! Run with `cargo run --example world`.

use anarchy::*;
use mutual::RelaxedMutex;

#[derive(Debug, Component)]
struct Position { x: f32, y: f32 }

#[derive(Debug, Component)]
struct Velocity { x: f32, y: f32 }

#[derive(Debug, Component)]
struct Poisoned(u32);

#[derive(Debug, Resource)]
struct Score(u32);

fn main() -> anyhow::Result<()> {
    // `World::default()` stores entities in an `IndexedWorld`. `World::new(ListWorld::new())`
    // is the simpler alternative, which scans every table.
    let world = World::default();

    // Ids are chosen by the caller, and every component sits behind its own lock.
    world.insert(1, Box::new([
        RelaxedMutex::new(Box::new(Position { x: 0.0, y: 0.0 }) as DynComponent),
        RelaxedMutex::new(Box::new(Velocity { x: 1.0, y: 0.5 }) as DynComponent),
    ]));
    world.insert(2, Box::new([
        RelaxedMutex::new(Box::new(Position { x: 10.0, y: 10.0 }) as DynComponent),
        RelaxedMutex::new(Box::new(Velocity { x: -1.0, y: 0.0 }) as DynComponent),
    ]));

    // Building an entity from a `Vec<DynComponent>`. This one has no velocity.
    let components: Vec<DynComponent> = vec![Box::new(Position { x: 5.0, y: 5.0 })];
    world.insert(3, components.into_iter().map(RelaxedMutex::new).collect());

    // A query visits every entity holding all of its components, so entity 3 is skipped.
    let mut query = Query::<(&mut Position, &Velocity)>::new(&world);
    while let Some((id, (mut position, velocity))) = query.next()? {
        position.x += velocity.x;
        position.y += velocity.y;
        println!("moved {id} to ({}, {})", position.x, position.y);
    }

    // Single entity access by id is a hash lookup, not a table walk.
    // `Ok(None)` means no such entity, `Err` means it lacks a requested component.
    let position = world.get::<&Position>(1)?.unwrap();
    println!("entity 1 is at ({}, {})", position.x, position.y);
    drop(position);
    assert!(world.get::<&Velocity>(3).is_err());
    assert!(world.get::<&Position>(99)?.is_none());

    // Adding or removing a component moves the entity to another table.
    world.insert_component_dyn(3, Box::new(Poisoned(3)))?;
    println!("entity 3 is poisoned for {} turns", world.get::<&Poisoned>(3)?.unwrap().0);
    assert!(world.remove_component_dyn(3, Poisoned::id())?);

    // Removing an entity hands back its components.
    let removed = world.remove(2).unwrap();
    println!("removed entity 2 with {} components", removed.len());

    // Resources are one value per type, stored on the world.
    world.insert_resource(Score(0));
    world.init_resource_with(|| Score(100)); // does nothing, a Score already exists
    world.resource_mut::<Score>()?.0 += 10;
    println!("score: {}", world.resource::<Score>()?.0);
    assert!(world.has_resource::<Score>());
    world.remove_resource::<Score>();
    assert!(world.resource::<Score>().is_err());

    Ok(())
}
