// criterion doesn't build for wasm, so there the bench is empty and has no `main`
#![cfg_attr(target_arch = "wasm32", no_main)]
#![cfg(not(target_arch = "wasm32"))]

use std::hint::black_box;

use anarchy::*;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use mutual::RelaxedMutex;

macro_rules! components {
    ($($name:ident),*) => {$(
        #[derive(Debug, Component)]
        #[component(no_serialize)]
        struct $name;
    )*};
}

components!(C0, C1, C2, C3, C4, C5, C6, C7, C8, C9, C10, C11, C12, C13, C14, C15, Rare, Missing);

/// Constructors for the 16 common components, indexed by bit.
const COMMON: [fn() -> DynComponent; 16] = [
    || Box::new(C0), || Box::new(C1), || Box::new(C2), || Box::new(C3),
    || Box::new(C4), || Box::new(C5), || Box::new(C6), || Box::new(C7),
    || Box::new(C8), || Box::new(C9), || Box::new(C10), || Box::new(C11),
    || Box::new(C12), || Box::new(C13), || Box::new(C14), || Box::new(C15)
];

const ENTITIES_PER_TABLE: u64 = 4;

/// `tables` distinct groups built from the common components, plus one table that also holds `Rare`.
fn populate(world: &impl WorldImpl, tables: u32) {
    let mut id = 0;
    let mut spawn = |mask: u32, rare: bool| for _ in 0..ENTITIES_PER_TABLE {
        let mut comps = (0..16)
            .filter(|bit| mask & (1 << bit) != 0)
            .map(|bit| COMMON[bit]())
            .collect::<Vec<_>>();
        if rare { comps.push(Box::new(Rare)); }
        world.insert(id, comps.into_iter().map(RelaxedMutex::new).collect());
        id += 1;
    };

    // spread the masks over the whole 16 bit space with a multiplicative step so groups are
    // varied but distinct (odd multiplier => bijection mod 2^16), skipping the empty group
    for i in 1..=tables {
        let mask = (i.wrapping_mul(40503)) & 0xFFFF;
        spawn(if mask == 0 { 1 } else { mask }, false);
        // in the middle, so ListWorld (which pushes new tables to the head) gets neither its best nor worst case
        if i == tables / 2 { spawn(0b111, true); }
    }
}

fn count(world: &impl WorldImpl, req: &[ComponentID]) -> usize {
    world.raw_query(Box::from(req)).count()
}

fn bench_query(c: &mut Criterion) {
    let queries: [(&str, Vec<ComponentID>); 4] = [
        ("common", vec![C0::id()]),
        ("rare", vec![Rare::id()]),
        ("three", vec![C0::id(), C1::id(), C2::id()]),
        ("missing", vec![C0::id(), Missing::id()])
    ];

    for tables in [16, 256, 1024] {
        let list = ListWorld::new();
        let indexed = IndexedWorld::new();
        populate(&list, tables);
        populate(&indexed, tables);

        for (name, req) in &queries {
            assert_eq!(count(&list, req), count(&indexed, req), "{name} mismatch");
            let mut group = c.benchmark_group(format!("query/{name}"));
            group.bench_with_input(BenchmarkId::new("list", tables), req, |b, req| b.iter(|| count(&list, black_box(req))));
            group.bench_with_input(BenchmarkId::new("indexed", tables), req, |b, req| b.iter(|| count(&indexed, black_box(req))));
            group.finish();
        }
    }
}

fn bench_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert_existing");
    for tables in [16, 256, 1024] {
        let list = ListWorld::new();
        let indexed = IndexedWorld::new();
        populate(&list, tables);
        populate(&indexed, tables);

        // C0..C2 + Rare always exists, halfway down ListWorld's table list
        let entity = || [Box::new(C0) as DynComponent, Box::new(C1), Box::new(C2), Box::new(Rare)]
            .into_iter().map(RelaxedMutex::new).collect::<DynComponents>();
        group.bench_function(BenchmarkId::new("list", tables), |b| b.iter(|| list.insert(0, entity())));
        group.bench_function(BenchmarkId::new("indexed", tables), |b| b.iter(|| indexed.insert(0, entity())));
    }
    group.finish();
}

#[derive(Debug, Component)]
#[component(no_serialize)]
struct Local(u64);

#[derive(Debug, Component)]
#[component(no_serialize)]
struct Global(u64);

/// Links ids `0..` into a tree `depth` levels below root 0, each parent with `fan_out` children.  Returns the entity count.
fn link_tree(hierarchy: &Hierarchy, depth: u32, fan_out: u64) -> u64 {
    let (mut level, mut next) = (vec![0], 1);
    for _ in 0..depth {
        let mut below = vec![];
        for &parent in &level {
            for _ in 0..fan_out {
                hierarchy.set_parent(next, parent).unwrap();
                below.push(next);
                next += 1;
            }
        }
        level = below;
    }
    next
}

/// [`link_tree`] with every entity also spawned holding `Local` and `Global`.
fn spawn_tree(world: &World, depth: u32, fan_out: u64) -> u64 {
    let entities = link_tree(world.hierarchy(), depth, fan_out);
    for id in 0..entities {
        world.insert(id, [Box::new(Local(1)) as DynComponent, Box::new(Global(0))]
            .into_iter().map(RelaxedMutex::new).collect());
    }
    entities
}

/// Sums `Local` down the tree into `Global`, starting from every root.
fn propagate(world: &World) {
    let hierarchy = world.hierarchy();
    let mut roots = Query::<(&Local, &mut Global)>::new(world);
    while let Some((id, (local, mut global))) = roots.next().unwrap() {
        if hierarchy.parent(id).is_some() { continue }
        global.0 = local.0;
        let mut stack = vec![(id, global.0)];
        while let Some((parent, parent_global)) = stack.pop() {
            let Some(children) = hierarchy.children(parent) else { continue };
            for &child in children.iter() {
                let (local, mut global) = world.get::<(&Local, &mut Global)>(child).unwrap().unwrap();
                global.0 = parent_global + local.0;
                stack.push((child, global.0));
            }
        }
    }
}

fn bench_hierarchy(c: &mut Criterion) {
    let mut group = c.benchmark_group("hierarchy/propagate");
    for (depth, fan_out) in [(3, 6), (5, 6)] {
        let world = World::default();
        let entities = spawn_tree(&world, depth, fan_out);
        propagate(&world);
        assert_eq!(world.get::<&Global>(entities - 1).unwrap().unwrap().0, depth as u64 + 1);
        group.bench_function(BenchmarkId::from_parameter(entities), |b| b.iter(|| propagate(&world)));
    }
    group.finish();
}

/// Sums the children of `parent`, so the read can't be optimised away.
fn sum_children(hierarchy: &Hierarchy, parent: EntityID) -> u64 {
    hierarchy.children(parent).map_or(0, |children| children.iter().sum())
}

fn bench_children(c: &mut Criterion) {
    const PARENTS: u64 = 64;
    const FAN_OUT: u64 = 6;
    const READERS: usize = 4;
    // moved back and forth between parents 0 and 1 by the writer
    const MOVING: EntityID = u64::MAX;

    let hierarchy = Hierarchy::new();
    for parent in 0..PARENTS {
        for i in 0..FAN_OUT { hierarchy.set_parent(PARENTS + parent * FAN_OUT + i, parent).unwrap(); }
    }
    hierarchy.set_parent(MOVING, 0).unwrap();

    c.bench_function("hierarchy/children/read", |b| b.iter(|| sum_children(&hierarchy, black_box(5))));

    // time for every reader to do `iters` reads while one writer keeps editing
    c.bench_function("hierarchy/children/contended", |b| b.iter_custom(|iters| {
        let stop = std::sync::atomic::AtomicBool::new(false);
        let barrier = std::sync::Barrier::new(READERS + 1);
        std::thread::scope(|s| {
            let (hierarchy, stop) = (&hierarchy, &stop);
            s.spawn(move || {
                let mut parent = 0;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    parent = 1 - parent;
                    hierarchy.set_parent(MOVING, parent).unwrap();
                }
            });
            let readers = (0..READERS).map(|_| {
                let barrier = &barrier;
                s.spawn(move || {
                    barrier.wait();
                    for i in 0..iters { black_box(sum_children(hierarchy, i % PARENTS)); }
                })
            }).collect::<Vec<_>>();
            barrier.wait();
            let start = std::time::Instant::now();
            for reader in readers { reader.join().unwrap(); }
            let elapsed = start.elapsed();
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            elapsed
        })
    }));

    c.bench_function("hierarchy/build/9331", |b| b.iter(|| link_tree(&Hierarchy::new(), 5, 6)));
}

/// [`propagate`] written with [`EntityHandle`]s.
fn propagate_handles(world: &World) {
    let mut roots = Query::<(&Local, &mut Global)>::new(world);
    while let Some((id, (local, mut global))) = roots.next().unwrap() {
        let root = world.handle(id);
        if root.parent().is_some() { continue }
        global.0 = local.0;
        let mut stack = vec![(root, global.0)];
        while let Some((parent, parent_global)) = stack.pop() {
            let Some(children) = parent.children() else { continue };
            for child in children.iter().map(|&id| world.handle(id)) {
                let (local, mut global) = child.get::<(&Local, &mut Global)>().unwrap().unwrap();
                global.0 = parent_global + local.0;
                stack.push((child, global.0));
            }
        }
    }
}

fn bench_handle(c: &mut Criterion) {
    let world = World::default();
    let entities = spawn_tree(&world, 5, 6);
    let leaf = entities - 1;

    let mut group = c.benchmark_group("handle/get");
    group.bench_function("world", |b| b.iter(|| world.get::<&Local>(black_box(leaf)).unwrap().unwrap().0));
    group.bench_function("handle", |b| b.iter(|| world.handle(black_box(leaf)).get::<&Local>().unwrap().unwrap().0));
    group.finish();

    let mut group = c.benchmark_group("handle/descendants");
    group.bench_function("hierarchy", |b| b.iter(|| world.hierarchy().descendants(black_box(0)).count()));
    group.bench_function("handle", |b| b.iter(|| world.handle(black_box(0)).descendants().count()));
    group.finish();

    let mut group = c.benchmark_group("handle/propagate");
    group.bench_function("ids", |b| b.iter(|| propagate(&world)));
    group.bench_function("handles", |b| b.iter(|| propagate_handles(&world)));
    group.finish();
}

/// Walks a [`Query`] over every entity, reading one component and writing another.
fn bench_query_iter(c: &mut Criterion) {
    let world = World::default();
    let entities = spawn_tree(&world, 5, 6);
    c.bench_function(&format!("query/iterate/{entities}"), |b| b.iter(|| {
        let mut query = Query::<(&Local, &mut Global)>::new(&world);
        while let Some((id, (local, mut global))) = query.next().unwrap() {
            global.0 = local.0 + id;
        }
    }));
}

criterion_group!(benches, bench_query, bench_insert, bench_hierarchy, bench_children, bench_handle, bench_query_iter);
criterion_main!(benches);
