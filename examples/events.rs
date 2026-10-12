//! Sending and reading events between systems.
//!
//! Run with `cargo run --example events`.

use std::time::Duration;

use anarchy::*;

/// Any `Clone + Send` type can be an event.
#[derive(Clone, Debug)]
struct Damage { entity: EntityID, amount: u32 }

#[derive(Debug, Resource)]
struct DamageTaken(u32);

// Sends one event per run.
fn attack(damage: Event<Damage>) {
    damage.send(Damage { entity: 7, amount: 5 });
}

// Sees each event once, no matter how many times it runs.
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

    // Events last `EVENT_LIFETIME` (1s) by default. To change that, insert the queue
    // before anything sends.
    world.insert_resource(EventQueue::<Damage>::with_lifetime(Duration::from_secs(5)));

    let mut graph = SystemGraph::new();
    graph.append_system(attack, SystemInstruction::default(), std::iter::empty())?;
    // Ordered after the sender, so readers see this pass's events in this pass.
    graph.append_system(apply_damage, after(attack), std::iter::empty())?;
    graph.append_system(log_damage, after(attack), std::iter::empty())?;

    // Code outside any system sends through the world.
    world.send_event(Damage { entity: 3, amount: 20 });

    let executor = SingleThreadedExecutor::new();
    executor.run(&world, &graph)?;
    executor.run(&world, &graph)?;

    // 20 from main plus 5 from each of the two runs, each counted once.
    let taken = world.resource::<DamageTaken>()?.0;
    println!("total damage taken: {taken}");
    assert_eq!(taken, 30);
    Ok(())
}
