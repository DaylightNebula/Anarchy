//! Running a graph on many threads, and sharing work inside a system with idle threads.
//! Threads aren't available on the web, so this example is native only.
//!
//! Run with `cargo run --example multithreaded`.

#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports))]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use anarchy::*;
use mutual::RelaxedMutex;

#[derive(Debug, Component)]
struct Brain { thoughts: u64 }

#[derive(Debug, Component)]
struct Position(f32);

#[derive(Debug, Resource)]
struct Frames(u32);

/// Cleared by `count_frames` to stop the executor's loop.
static RUNNING: AtomicBool = AtomicBool::new(true);

// `movement` and `count_frames` have no ordering between them, so they may run at once.
fn movement(mut query: Query<&'static mut Position>) {
    while let Some((_id, mut position)) = query.next().unwrap() {
        position.0 += 1.0;
    }
}

fn count_frames(mut frames: ResMut<Frames>) {
    frames.0 += 1;
    if frames.0 == 10 { RUNNING.store(false, Ordering::Release); }
}

/// Splits expensive per-entity work into a `ParIter`, which idle executor threads help with.
struct Think;

impl System<(), ()> for Think {
    fn run(&self, world: &World, exec: &SharedExecutionState) -> anyhow::Result<()> {
        // Collect the ids first, so no query guards are held while the work runs.
        let mut ids = vec![];
        let mut query = Query::<&Brain>::new(world);
        while let Some((id, _)) = query.next()? { ids.push(id); }

        let total = ids.len();
        let done = std::sync::Arc::new(AtomicUsize::new(0));
        let (world, counter) = (world.clone(), done.clone());
        exec.submit_work(ParIter::new(ids.into_iter(), move |id| {
            if let Ok(Some(mut brain)) = world.get::<&mut Brain>(id) {
                brain.thoughts += (0..10_000u64).sum::<u64>() % 7;
            }
            counter.fetch_add(1, Ordering::AcqRel);
        }));

        // Help until nothing is left to claim, then wait for the items other threads took.
        while exec.exec_from_work_queue() {}
        while done.load(Ordering::Acquire) < total { std::hint::spin_loop(); }
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> anyhow::Result<()> {
    let world = World::default();
    world.insert_resource(Frames(0));
    for id in 0..1000 {
        world.insert(id, Box::new([
            RelaxedMutex::new(Box::new(Brain { thoughts: 0 }) as DynComponent),
            RelaxedMutex::new(Box::new(Position(0.0)) as DynComponent),
        ]));
    }

    let mut graph = SystemGraph::new();
    graph.append_system(movement, SystemInstruction::default(), std::iter::empty())?;
    graph.append_raw(std::any::TypeId::of::<Think>(), Box::new(Think),
                     SystemInstruction::default(), std::iter::empty())?;
    graph.append_system(count_frames, pin(SystemPin::End), std::iter::empty())?;

    // Loops over the graph on `threads` threads until `RUNNING` is false after a pass.
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    MultiThreadedExecutor::new().run(&world, &graph, threads, &RUNNING)?;

    let frames = world.resource::<Frames>()?.0;
    let mut thoughts = 0;
    let mut query = Query::<&Brain>::new(&world);
    while let Some((_, brain)) = query.next()? { thoughts += brain.thoughts; }
    println!("ran {frames} frames on {threads} threads, {thoughts} thoughts in total");
    assert_eq!(world.get::<&Position>(0)?.unwrap().0, frames as f32);
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
