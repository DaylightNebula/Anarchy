//! Function systems, ordering them in a `SystemGraph`, and running the graph.
//!
//! Run with `cargo run --example systems`.

use std::any::TypeId;

use anarchy::*;
use mutual::RelaxedMutex;

#[derive(Debug, Component)]
struct Position(f32);

#[derive(Debug, Component)]
struct Velocity(f32);

#[derive(Debug, Resource)]
struct Gravity(f32);

#[derive(Debug, Resource)]
struct Frame(u32);

/// Tags every physics system, so other systems can order against the whole group.
struct PhysicsSet;

fn begin_frame(mut frame: ResMut<Frame>) {
    frame.0 += 1;
    println!("frame {}", frame.0);
}

fn gravity(gravity: Res<Gravity>, mut query: Query<&'static mut Velocity>) {
    while let Some((_id, mut velocity)) = query.next().unwrap() {
        velocity.0 -= gravity.0;
    }
}

fn movement(mut query: Query<(&'static mut Position, &'static Velocity)>) {
    while let Some((_id, (mut position, velocity))) = query.next().unwrap() {
        position.0 += velocity.0;
    }
}

fn misplaced() {}

fn render(mut query: Query<&'static Position>) {
    while let Some((id, position)) = query.next().unwrap() {
        println!("  entity {id} at height {:.1}", position.0);
    }
}

/// A custom system parameter, built from the world each time a system runs.
struct FrameNumber(u32);

impl SystemParam for FrameNumber {
    fn extract(world: &World, _exec: &SharedExecutionState, _system: SystemKey) -> anyhow::Result<Self> {
        Ok(FrameNumber(world.resource::<Frame>()?.0))
    }
}

fn end_frame(frame: FrameNumber) {
    println!("  end of frame {}", frame.0);
}

/// A hand-written system, which can return errors.
struct CheckGround;

impl System<(), ()> for CheckGround {
    fn run(&self, world: &World, _exec: &SharedExecutionState) -> anyhow::Result<()> {
        let mut query = Query::<&Position>::new(world);
        while let Some((id, position)) = query.next()? {
            if position.0 < 0.0 { println!("  entity {id} is below the ground"); }
        }
        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    let world = World::default();
    world.insert_resource(Gravity(1.0));
    world.insert_resource(Frame(0));
    world.insert(1, Box::new([
        RelaxedMutex::new(Box::new(Position(2.0)) as DynComponent),
        RelaxedMutex::new(Box::new(Velocity(1.0)) as DynComponent),
    ]));

    let physics = TypeId::of::<PhysicsSet>();
    let mut graph = SystemGraph::new();

    // Pins run one after another: every Start system, then Normal, then End.
    graph.append_system(begin_frame, pin(SystemPin::Start), std::iter::empty())?;
    graph.append_system(end_frame, pin(SystemPin::End), std::iter::empty())?;

    // `before` and `after` order against one system, `and` combines instructions,
    // and the last argument tags a system with metadata.
    graph.append_system(gravity, SystemInstruction::default(), [physics].into_iter())?;
    graph.append_system(movement, after(gravity), [physics].into_iter())?;

    // `after_meta` orders against every system with a tag.
    graph.append_system(render, after_meta([physics]), std::iter::empty())?;

    // Hand-written systems go in with `append_raw` under a key of your choosing.
    graph.append_raw(TypeId::of::<CheckGround>(), Box::new(CheckGround),
                     and(after_meta([physics]), before(render)), std::iter::empty())?;

    // Instructions that would make a cycle are rejected, and the system is left out.
    assert!(graph.append_system(misplaced, and(after(movement), before(gravity)), std::iter::empty()).is_err());

    // One pass of the graph on this thread, per call.
    let executor = SingleThreadedExecutor::new();
    for _ in 0..4 {
        executor.run(&world, &graph)?;
    }

    // A system whose parameters can't be extracted doesn't run, and the executor errors.
    world.remove_resource::<Gravity>();
    let error = executor.run(&world, &graph).unwrap_err();
    println!("without Gravity: {error}");

    // Systems can be taken back out, by key or by tag.
    graph.remove_by_meta(physics);
    world.insert_resource(Gravity(1.0));
    executor.run(&world, &graph)?;

    Ok(())
}
