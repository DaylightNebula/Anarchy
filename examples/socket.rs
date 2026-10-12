//! A running app that other programs edit through a `WorldSocket`, with the `socket`
//! feature.  Start it, then poke at it from another terminal with the CLI:
//!
//! ```sh
//! cargo run --example socket --features socket
//! cargo run --features socket --bin anarchy -- ls
//! cargo run --features socket --bin anarchy -- set 1 Health current 10
//! cargo run --features socket --bin anarchy -- set @Gravity y -1.5
//! cargo run --features socket --bin anarchy        # an interactive shell
//! ```

#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports))]

use std::time::Duration;

use anarchy::*;

#[derive(Debug, Clone, Default, PartialEq, Component, Reflect)]
struct Height(f32);

/// A unit's health, regenerating over time.
#[derive(Debug, Clone, PartialEq, Component, Reflect)]
struct Health {
    #[reflect(hint(min = 0, max = 100))]
    current: u32,
    max: u32
}

#[derive(Debug, Clone, Default, PartialEq, Resource, Reflect)]
struct Gravity { y: f32 }

fn fall(mut query: Query<&'static mut Height>, gravity: Res<Gravity>) {
    while let Some((_, mut height)) = query.next().unwrap() {
        height.0 = (height.0 + gravity.y * 0.1).max(0.0);
    }
}

fn regenerate(mut query: Query<&'static mut Health>) {
    while let Some((_, mut health)) = query.next().unwrap() {
        health.current = (health.current + 1).min(health.max);
    }
}

/// Prints every edit made through the socket.
fn log_edits(edits: Event<ReflectEdit>) {
    for edit in edits.read() {
        println!("edit {:?} `{}`: {} -> {}", edit.target, edit.path, edit.old, edit.new);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> anyhow::Result<()> {
    let world = World::default();
    Scene::parse(r#"
        @Gravity { y: -9.8 },
        #Player
        Height(100.0)
        Health { current: 50, max: 100 }
        [ #Sword Height(101.0) ],
    "#)?.spawn(&world, { let mut next = 0; move || { next += 1; next } })?;

    let address = std::env::var("ANARCHY_SOCKET").unwrap_or_else(|_| DEFAULT_SOCKET_ADDRESS.into());
    let socket = WorldSocket::bind(&world, &address)?;
    println!("listening on {}", socket.address());

    let mut graph = SystemGraph::new();
    graph.append_system(fall, SystemInstruction::default(), std::iter::empty())?;
    graph.append_system(regenerate, SystemInstruction::default(), std::iter::empty())?;
    graph.append_system(log_edits, SystemInstruction::default(), std::iter::empty())?;
    let executor = SingleThreadedExecutor::new();
    loop {
        executor.run(&world, &graph)?;
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Wasm has no OS sockets.
#[cfg(target_arch = "wasm32")]
fn main() {}
