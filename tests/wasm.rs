//! Tests that run inside WebAssembly, in Node, through `wasm-bindgen-test`. The unit tests
//! are plain `#[test]`s, which the wasm runner skips, and many use threads, which wasm
//! doesn't have. These cover the single-threaded API instead, including the parts that
//! touch the clock (events) and so could panic on the web even though they compile.
//!
//! Run with `cargo test --target wasm32-unknown-unknown` (see `.cargo/config.toml`).
#![cfg(target_arch = "wasm32")]

use anarchy::*;
use mutual::RelaxedMutex;
use wasm_bindgen_test::wasm_bindgen_test;

#[derive(Debug, Component)]
struct Position(f32);

#[derive(Debug, Component)]
struct Velocity(f32);

#[derive(Debug, Default, Resource)]
struct Log(Vec<String>);

#[derive(Clone, Debug)]
struct Ping(u32);

fn insert(world: &World, id: EntityID, components: Vec<DynComponent>) {
    world.insert(id, components.into_iter().map(RelaxedMutex::new).collect());
}

fn run_once(world: &World, graph: &SystemGraph) {
    SingleThreadedExecutor::new().run(world, graph).unwrap();
}

#[wasm_bindgen_test]
fn entities_insert_get_and_remove() {
    let world = World::default();
    insert(&world, 1, vec![Box::new(Position(1.0)), Box::new(Velocity(2.0))]);

    let (position, velocity) = world.get::<(&Position, &Velocity)>(1).unwrap().unwrap();
    assert_eq!((position.0, velocity.0), (1.0, 2.0));
    drop((position, velocity));

    assert!(world.remove(1).is_some());
    assert!(world.get::<&Position>(1).unwrap().is_none());
}

#[wasm_bindgen_test]
fn queries_visit_matching_entities() {
    let world = World::default();
    insert(&world, 1, vec![Box::new(Position(0.0)), Box::new(Velocity(1.0))]);
    insert(&world, 2, vec![Box::new(Position(0.0)), Box::new(Velocity(2.0))]);
    insert(&world, 3, vec![Box::new(Position(0.0))]);

    let mut query = Query::<(&mut Position, &Velocity)>::new(&world);
    let mut moved = 0;
    while let Some((_, (mut position, velocity))) = query.next().unwrap() {
        position.0 += velocity.0;
        moved += 1;
    }
    assert_eq!(moved, 2);
    assert_eq!(world.get::<&Position>(2).unwrap().unwrap().0, 2.0);
    assert_eq!(world.get::<&Position>(3).unwrap().unwrap().0, 0.0);
}

#[wasm_bindgen_test]
fn resources_insert_and_mutate() {
    let world = World::default();
    world.insert_resource(Log::default());
    world.resource_mut::<Log>().unwrap().0.push("hi".into());
    assert_eq!(world.resource::<Log>().unwrap().0, vec!["hi"]);
    assert!(world.remove_resource::<Log>());
    assert!(world.resource::<Log>().is_err());
}

/// Events timestamp themselves when sent, which panicked on wasm while they used
/// `std::time::Instant`.
#[wasm_bindgen_test]
fn events_are_read_once_per_system() {
    fn read(pings: Event<Ping>, mut log: ResMut<Log>) {
        for ping in pings.read() { log.0.push(format!("ping {}", ping.0)); }
    }

    let world = World::default();
    world.insert_resource(Log::default());
    world.send_event(Ping(1));
    let mut graph = SystemGraph::new();
    graph.append_system(read, SystemInstruction::default(), std::iter::empty()).unwrap();

    run_once(&world, &graph);
    world.send_event(Ping(2));
    run_once(&world, &graph);

    assert_eq!(world.resource::<Log>().unwrap().0, vec!["ping 1", "ping 2"]);
}

#[wasm_bindgen_test]
fn graph_runs_pins_and_ordering() {
    fn start(mut log: ResMut<Log>) { log.0.push("start".into()); }
    fn first(mut log: ResMut<Log>) { log.0.push("first".into()); }
    fn second(mut log: ResMut<Log>) { log.0.push("second".into()); }
    fn end(mut log: ResMut<Log>) { log.0.push("end".into()); }

    let world = World::default();
    world.insert_resource(Log::default());
    let mut graph = SystemGraph::new();
    graph.append_system(end, pin(SystemPin::End), std::iter::empty()).unwrap();
    graph.append_system(second, after(first), std::iter::empty()).unwrap();
    graph.append_system(first, SystemInstruction::default(), std::iter::empty()).unwrap();
    graph.append_system(start, pin(SystemPin::Start), std::iter::empty()).unwrap();
    run_once(&world, &graph);

    assert_eq!(world.resource::<Log>().unwrap().0, vec!["start", "first", "second", "end"]);
}

#[wasm_bindgen_test]
fn missing_resources_fail_the_run() {
    fn needs_log(_log: Res<Log>) {}

    let mut graph = SystemGraph::new();
    graph.append_system(needs_log, SystemInstruction::default(), std::iter::empty()).unwrap();
    assert!(SingleThreadedExecutor::new().run(&World::default(), &graph).is_err());
}

#[wasm_bindgen_test]
fn hierarchy_links_and_despawns() {
    let world = World::default();
    for id in 1..=3 { insert(&world, id, vec![Box::new(Position(0.0))]); }
    world.handle(2).set_parent(1).unwrap();
    world.handle(3).set_parent(2).unwrap();

    assert_eq!(world.hierarchy().descendants(1).collect::<Vec<_>>(), vec![2, 3]);
    assert!(world.handle(1).set_parent(3).is_err(), "cycles are rejected");
    assert_eq!(world.handle(1).despawn_recursive(), 3);
    assert!(world.entity(3).is_none());
}
