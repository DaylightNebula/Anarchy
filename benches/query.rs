use std::hint::black_box;

use anarchy::*;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use mutual::RelaxedMutex;

macro_rules! components {
    ($($name:ident),*) => {$(
        #[derive(Debug, Component)]
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

criterion_group!(benches, bench_query, bench_insert);
criterion_main!(benches);
