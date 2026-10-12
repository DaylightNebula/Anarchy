//! Talking to a world from an editor in another process, with the `serde` feature.
//! Type info and edits go out as JSON, and values come back in as JSON.
//!
//! Run with `cargo run --example serde_editor --features serde`.

use anarchy::*;
use mutual::RelaxedMutex;

/// A unit's health.
#[derive(Debug, Component, Reflect)]
struct Health {
    #[reflect(hint(min = 0, max = 100))]
    current: u32,
    max: u32,
}

/// Forwards every edit to the "editor", here stdout.
fn send_edits(edits: Event<ReflectEdit>) {
    for edit in edits.read() {
        println!("-> edit: {}", serde_json::to_string(&edit).unwrap());
    }
}

fn main() -> anyhow::Result<()> {
    let world = World::default();
    world.insert(1, Box::new([
        RelaxedMutex::new(Box::new(Health { current: 80, max: 100 }) as DynComponent),
    ]));

    // The editor gets the type info to draw an inspector...
    println!("-> type: {}", serde_json::to_string(Health::type_info())?);
    // ...and the current value.
    let value = world.reflect_component(1, Health::id(), |c| c.to_value())?;
    println!("-> value: {}", serde_json::to_string(&value)?);

    // It sends back a new value for one field.
    let incoming = r#"{"Int":55}"#;
    println!("<- set current: {incoming}");
    let new: Value = serde_json::from_str(incoming)?;
    world.set_component_path(1, Health::id(), "current", &new)?;
    assert_eq!(world.get::<&Health>(1)?.unwrap().current, 55);

    let mut graph = SystemGraph::new();
    graph.append_system(send_edits, SystemInstruction::default(), std::iter::empty())?;
    SingleThreadedExecutor::new().run(&world, &graph)?;
    Ok(())
}
