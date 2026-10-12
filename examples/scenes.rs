//! Scenes: the text format, the `scene!` macro, inheritance from a `SceneLibrary`, and
//! saving a world back to text.
//!
//! Run with `cargo run --example scenes`.

use anarchy::*;

#[derive(Debug, Clone, Default, PartialEq, Component, Reflect)]
struct Transform { x: f32, y: f32 }

#[derive(Debug, Clone, PartialEq, Component, Reflect)]
struct Health(u32);

/// `#[reflect(entity)]` fields refer to other entities by name in scenes.
#[derive(Debug, Clone, PartialEq, Component, Reflect)]
struct Target(#[reflect(entity)] EntityID);

#[derive(Debug, Clone, Default, PartialEq, Component, Reflect)]
enum Team { #[default] Neutral, Red, Blue }

#[derive(Debug, Clone, Default, PartialEq, Resource, Reflect)]
struct Gravity { y: f32 }

/// Not reflected, so it is only usable from `scene!` and isn't saved.
#[derive(Debug, Default, Component)]
struct Marker;

fn main() -> anyhow::Result<()> {
    let world = World::default();

    // Ids come from a closure, since anarchy leaves them to the caller.
    let mut next_id = 0;
    let mut alloc = move || { next_id += 1; next_id };

    // A library of scenes that others can inherit from with `:name`.
    let mut library = SceneLibrary::new();
    library.insert("Soldier", Scene::parse("Health(100) Team = Red [ #Helmet Transform { y: 1.5 } ]")?);
    world.insert_resource(library);

    let scene = Scene::parse(r#"
        @Gravity { y: -9.8 },          // a resource
        #Player                        // an entity named Player
        Transform { y: 2.0 }           // x keeps its default
        Transform { x: 1.0 }           // patches of one type merge in order
        Health(100)
        Target(#Enemy)                 // refers to the entity named Enemy
        [
            #Sword Transform { x: 0.5 },
        ],
        #Enemy :Soldier                // starts from the Soldier scene, children included
        Health(50),                    // then overrides its health
    "#)?;

    let spawned = scene.spawn(&world, &mut alloc)?;
    let (player, enemy) = (spawned.by_name["Player"], spawned.by_name["Enemy"]);
    println!("spawned {} entities: {:?}", spawned.entities.len(), spawned.by_name);

    assert_eq!(*world.get::<&Transform>(player)?.unwrap(), Transform { x: 1.0, y: 2.0 });
    assert_eq!(world.get::<&Target>(player)?.unwrap().0, enemy);
    assert_eq!(world.get::<&Name>(player)?.unwrap().0, "Player");
    assert_eq!(*world.get::<&Team>(enemy)?.unwrap(), Team::Red);
    assert_eq!(world.get::<&Health>(enemy)?.unwrap().0, 50);
    assert_eq!(world.resource::<Gravity>()?.y, -9.8);
    assert_eq!(world.handle(enemy).descendants().count(), 1); // the inherited helmet

    // Errors point at the line and column, and nothing is spawned.
    let error = Scene::parse("#Broken Transform { x: }").unwrap_err();
    println!("parse error: {error}");

    // The `scene!` macro is checked at compile time and takes any Rust expressions.
    // Its components don't need to be reflected.
    let height = 3.0;
    let macro_scene = scene! {
        #Tower
        Transform { y: height }       // Default, then only these fields set
        Health(500)                   // any Type(args) expression
        Marker
        [
            { Transform { x: 0.0, y: height * 2.0 } },  // any expression in braces
        ],
    };
    let tower = macro_scene.spawn(&world, &mut alloc)?;
    println!("tower is entity {}", tower.by_name["Tower"]);

    // Save entities back to text, keeping only what differs from the defaults.
    // `Marker` isn't reflected, so it is listed as skipped.
    let roots: Vec<_> = spawned.roots.iter().chain(&tower.roots).copied().collect();
    let saved = Scene::from_world(&world, &roots)?;
    let text = saved.scene.to_text()?;
    println!("saved scene:\n{text}");
    println!("skipped {} unreflected components", saved.skipped.len());

    // The text loads back into an equal set of entities.
    spawned.despawn(&world);
    tower.despawn(&world);
    let reloaded = Scene::parse(&text)?.spawn(&world, &mut alloc)?;
    assert_eq!(reloaded.entities.len(), spawned.entities.len() + tower.entities.len());
    Ok(())
}
