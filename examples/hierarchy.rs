//! Parent and child links, and entity handles.
//!
//! Run with `cargo run --example hierarchy`.

use anarchy::*;
use mutual::RelaxedMutex;

#[derive(Debug, Component)]
struct Label(&'static str);

fn spawn(world: &World, id: EntityID, label: &'static str) {
    world.insert(id, Box::new([RelaxedMutex::new(Box::new(Label(label)) as DynComponent)]));
}

fn main() -> anyhow::Result<()> {
    let world = World::default();
    spawn(&world, 1, "player");
    spawn(&world, 2, "arm");
    spawn(&world, 3, "sword");
    spawn(&world, 4, "shield");

    // The hierarchy stores links by id. Linking never moves entities between tables.
    let hierarchy = world.hierarchy();
    hierarchy.set_parent(2, 1)?;  // 2 is a child of 1
    hierarchy.add_child(2, 3)?;   // 3 is a child of 2
    hierarchy.add_child(1, 4)?;   // 4 is a child of 1

    assert_eq!(hierarchy.parent(3), Some(2));
    assert_eq!(*hierarchy.children(1).unwrap(), [2, 4]);  // in the order they were added
    assert!(hierarchy.set_parent(1, 3).is_err());          // would make a cycle

    let ancestors: Vec<_> = hierarchy.ancestors(3).collect();
    println!("ancestors of 3, parent first: {ancestors:?}");
    let descendants: Vec<_> = hierarchy.descendants(1).collect();
    println!("descendants of 1, depth first: {descendants:?}");

    // `children` returns a guard that read-locks part of the hierarchy, so copy it out
    // before editing the hierarchy on the same thread.
    let children = hierarchy.children(1).unwrap().to_vec();
    for child in children {
        hierarchy.remove_parent(child);
    }
    hierarchy.set_parent(2, 1)?;
    hierarchy.set_parent(4, 1)?;

    // An `EntityHandle` pairs an id with its world. Making one is free.
    let player = world.handle(1);
    println!("{} has:", player.get::<&Label>()?.unwrap().0);
    for item in player.descendants() {
        let depth = item.ancestors().count();
        println!("{}- {}", "  ".repeat(depth), item.get::<&Label>()?.unwrap().0);
    }

    // Moving the sword to the shield moves it out of the arm.
    world.handle(3).set_parent(4)?;
    assert_eq!(world.handle(3).parent().map(|p| p.id()), Some(4));

    // `despawn` turns the entity's children into roots.
    world.handle(4).despawn();
    assert_eq!(world.hierarchy().parent(3), None);

    // `despawn_recursive` removes the whole subtree.
    let removed = player.despawn_recursive();
    println!("despawned {removed} entities with the player");
    assert!(world.entity(2).is_none());
    assert!(world.entity(3).is_some());

    Ok(())
}
