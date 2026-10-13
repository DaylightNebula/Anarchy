//! [`ChunkedTable`], the table worlds store entities in: each component is a column of
//! raw bytes, split into fixed-capacity chunks.

use std::{alloc::Layout, any::Any, fmt::{self, Debug, Formatter}, ops::Range, ptr::NonNull, sync::{Arc, OnceLock, atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering}}};

use anyhow::{Context, bail};

use crate::{worlds::IdMap, *};

pub mod access;
pub(crate) mod borrow;

pub use access::*;

/// The rows in each chunk unless [`ChunkedTable::with_chunk_rows`] says otherwise.
pub const DEFAULT_CHUNK_ROWS: usize = 256;

/// Chunk `c` lives in segment `log2(c + 1)`, and segment `s` holds `2^s` chunks, so the
/// segments never move and still cover more chunks than can ever be allocated.
const SEGMENTS: usize = 48;

/// Set in [`ChunkedTable::gate`] while the table compacts.
const COMPACTING: usize = 1 << (usize::BITS - 1);

/// Storage for every entity with one exact set of components, each component kept as a
/// column of raw bytes in fixed-capacity chunks instead of a box per value.  Any number
/// of threads may use a table at once.
///
/// - **No allocation per entity.**  Rows are reserved in the last chunk with an atomic
///   counter, and chunk memory never moves, so guards stay valid while other threads insert.
/// - **A borrow flag per row per column.**  Each component of each entity is borrowed on
///   its own, as with anarchy's other locks, so systems only wait for each other when
///   they touch the same component.  Flags within a row are always taken in column order.
/// - **No downcasts.**  A query checks once that its types match the columns, then casts
///   pointers, and [`for_each_chunk`](Self::for_each_chunk) hands out plain slices.
/// - **Tombstones and compaction.**  Removing an entity leaves a hole that queries skip.
///   Once holes outnumber live rows the table tries to [compact](Self::try_compact),
///   which only happens while nothing is borrowed from it.
///
/// Entities are also indexed by id.  Like the rest of anarchy, ids are not checked for
/// uniqueness: the index keeps the last entity inserted with an id.
pub struct ChunkedTable {
    group: Arc<ComponentIDGroup>,
    /// One per column, in `group` order.
    infos: Box<[&'static ComponentInfo]>,
    chunk_rows: usize,
    segments: Box<[OnceLock<Box<[OnceLock<Chunk>]>>]>,
    /// Rows handed out to inserts, live or not.  Row `r` is row `r % chunk_rows` of chunk `r / chunk_rows`.
    reserved: AtomicUsize,
    /// Rows inserted and not removed.
    live: AtomicUsize,
    /// Every entity's row, see [`ChunkedTable`].
    index: IdMap<usize>,
    /// How many callers are using rows by position, or [`COMPACTING`] while rows move.
    /// Compaction only starts when nobody is inside, and never waits, so entering only
    /// ever waits for a compaction already running.
    gate: AtomicUsize
}

/// `rows` rows of every column.
pub(crate) struct Chunk {
    entities: NonNull<EntityID>,
    columns: Box<[Column]>,
    /// A bit per row, set while the row holds a live entity.
    alive: Box<[AtomicU64]>,
    /// Rows below this are written and visible to readers.  Inserts publish their rows in
    /// order, so readers never see a half-written row, and inserts only ever write rows at
    /// or above it, so they never touch memory a reader holds.
    published: AtomicUsize,
    rows: usize
}

/// One column's rows in a chunk.
struct Column {
    info: &'static ComponentInfo,
    data: NonNull<u8>,
    /// A borrow flag per row, see [`borrow`].
    flags: Box<[AtomicU32]>
}

// safety: the registry only holds types that are safe to send and share, values are only
// reached through their borrow flags, and entity ids are written once before being published
unsafe impl Send for Chunk {}
unsafe impl Sync for Chunk {}

impl Chunk {
    fn new(infos: &[&'static ComponentInfo], rows: usize) -> Self {
        Self {
            entities: allocate(Layout::array::<EntityID>(rows).unwrap()).cast(),
            columns: infos.iter().map(|&info| Column {
                info,
                data: allocate(column_layout(info, rows)),
                flags: (0..rows).map(|_| AtomicU32::new(0)).collect()
            }).collect(),
            alive: (0..rows / 64).map(|_| AtomicU64::new(0)).collect(),
            published: AtomicUsize::new(0),
            rows
        }
    }

    fn is_alive(&self, row: usize) -> bool {
        self.alive[row / 64].load(Ordering::Relaxed) & (1 << (row % 64)) != 0
    }

    fn set_alive(&self, row: usize, alive: bool) {
        let bit = 1 << (row % 64);
        if alive { self.alive[row / 64].fetch_or(bit, Ordering::Relaxed); } else { self.alive[row / 64].fetch_and(!bit, Ordering::Relaxed); }
    }

    fn entity(&self, row: usize) -> EntityID {
        unsafe { *self.entities.as_ptr().add(row) }
    }

    fn slot(&self, column: usize, row: usize) -> *mut u8 {
        let column = &self.columns[column];
        unsafe { column.data.as_ptr().add(row * column.info.layout.size()) }
    }

    fn flag(&self, column: usize, row: usize) -> &AtomicU32 {
        &self.columns[column].flags[row]
    }

    /// Takes every column's flag of `row`, in column order.
    fn lock_all(&self, row: usize, mutable: bool) {
        for column in 0..self.columns.len() {
            let flag = self.flag(column, row);
            if mutable { borrow::write(flag) } else { borrow::read(flag) }
        }
    }

    fn unlock_all(&self, row: usize, mutable: bool) {
        for column in 0..self.columns.len() {
            let flag = self.flag(column, row);
            if mutable { borrow::unlock_write(flag) } else { borrow::unlock_read(flag, 1) }
        }
    }
}

impl Drop for Chunk {
    fn drop(&mut self) {
        let len = *self.published.get_mut();
        for column in self.columns.iter() {
            if let Some(drop) = column.info.drop {
                for row in (0..len).filter(|&row| self.is_alive(row)) {
                    unsafe { drop(column.data.as_ptr().add(row * column.info.layout.size())) }
                }
            }
            free(column.data, column_layout(column.info, self.rows));
        }
        free(self.entities.cast(), Layout::array::<EntityID>(self.rows).unwrap());
    }
}

/// One flag a [`Plan`] takes on each row.
#[derive(Clone, Copy, Default)]
struct Lock {
    column: usize,
    mutable: bool,
    /// How many terms share a shared borrow of this column, each guard releases one.
    shares: u32
}

/// Where a [`QueryGroup`]'s terms sit in one table, and the flags it takes, in column order.
pub(crate) struct Plan {
    /// The column of each term, `None` for optional terms the table lacks.
    columns: [Option<usize>; MAX_PARAMS],
    locks: [Lock; MAX_PARAMS],
    lock_count: usize
}

impl Plan {
    fn locks(&self) -> &[Lock] {
        &self.locks[..self.lock_count]
    }

    fn lock(&self, chunk: &Chunk, row: usize) {
        for lock in self.locks() {
            let flag = chunk.flag(lock.column, row);
            if lock.mutable { borrow::write(flag) } else { borrow::read(flag); share(flag, lock.shares) }
        }
    }

    /// Takes every flag without waiting, or none of them.
    fn try_lock(&self, chunk: &Chunk, row: usize) -> bool {
        for (taken, lock) in self.locks().iter().enumerate() {
            let flag = chunk.flag(lock.column, row);
            let ok = if lock.mutable { borrow::try_write(flag) } else { borrow::try_read(flag) };
            if !ok {
                for lock in &self.locks()[..taken] { unlock(chunk.flag(lock.column, row), lock.mutable, 1) }
                return false
            }
            if !lock.mutable { share(flag, lock.shares) }
        }
        true
    }

    fn unlock(&self, chunk: &Chunk, row: usize) {
        for lock in self.locks() { unlock(chunk.flag(lock.column, row), lock.mutable, lock.shares) }
    }

    /// Each term's value at `row` of `chunk`.
    fn pointers(&self, chunk: &Chunk, row: usize) -> [Option<*mut u8>; MAX_PARAMS] {
        self.columns.map(|column| column.map(|column| chunk.slot(column, row)))
    }
}

/// Takes the extra shared borrows of a column read by more than one term.
fn share(flag: &AtomicU32, shares: u32) {
    if shares > 1 { borrow::read_again(flag, shares - 1) }
}

fn unlock(flag: &AtomicU32, mutable: bool, shares: u32) {
    if mutable { borrow::unlock_write(flag) } else { borrow::unlock_read(flag, shares) }
}

/// Releases rows a closure is running over, even if it panics.
struct Held<'a> {
    chunk: &'a Chunk,
    rows: Range<usize>,
    locks: HeldLocks<'a>
}

enum HeldLocks<'a> {
    Plan(&'a Plan),
    All(bool)
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        for row in self.rows.clone() {
            match self.locks {
                HeldLocks::Plan(plan) => plan.unlock(self.chunk, row),
                HeldLocks::All(mutable) => self.chunk.unlock_all(row, mutable)
            }
        }
    }
}

/// Inside a table's gate, see [`ChunkedTable::gate`].
struct Inside<'a>(&'a ChunkedTable);

impl Drop for Inside<'_> {
    fn drop(&mut self) { self.0.exit() }
}

/// A component in a [`ChunkedTable`] whose type isn't known at compile time, see
/// [`ChunkedTable::get_raw`].
#[derive(Clone, Copy)]
pub struct RawComponent<'a> {
    /// Describes the component.
    pub info: &'static ComponentInfo,
    ptr: *const u8,
    _borrow: std::marker::PhantomData<&'a ()>
}

impl <'a> RawComponent<'a> {
    /// A pointer to the value, valid for as long as this is.
    pub fn ptr(&self) -> *const u8 {
        self.ptr
    }

    /// The value as a `T`, or `None` if it is a different type.
    pub fn downcast_ref<T: 'static>(&self) -> Option<&'a T> {
        self.info.is::<T>().then(|| unsafe { &*(self.ptr as *const T) })
    }
}

impl Debug for RawComponent<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        unsafe { self.info.fmt_value(self.ptr, f) }
    }
}

/// A component boxed values are swapped for when they're moved out of a [`DynComponents`].
#[derive(Debug)]
struct Moved;

impl AsAny for Moved {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}

impl Component for Moved {}

/// Moves each boxed component out of `components`, leaving a placeholder behind.
pub(crate) fn unbox(components: DynComponents) -> Vec<DynComponent> {
    use mutual::SharedData;
    components.iter().map(|cell| std::mem::replace(&mut *cell.lock_mut(), Box::new(Moved))).collect()
}

impl ChunkedTable {
    /// Create an empty table for the components in the bundle `B`.
    ///
    /// # Panics
    ///
    /// Panics if `B` lists a component twice.
    pub fn new<B: Bundle>() -> Self {
        Self::from_infos((0..B::LEN).map(B::info)).unwrap()
    }

    /// Create an empty table for components described by registered infos, in any order.
    ///
    /// # Errors
    ///
    /// Errors if an info isn't the one registered for its id (see
    /// [`ComponentInfo::register`]), or a component is listed twice.
    pub fn from_infos(infos: impl IntoIterator<Item = &'static ComponentInfo>) -> anyhow::Result<Self> {
        let mut infos = infos.into_iter().collect::<Vec<_>>();
        for info in &infos {
            if !ComponentInfo::get(info.id).is_some_and(|registered| std::ptr::eq(registered, *info)) {
                bail!("{} (id {}) is not a registered component info", info.name, info.id)
            }
        }
        infos.sort_by_key(|info| info.id);
        if let Some(pair) = infos.windows(2).find(|pair| pair[0].id == pair[1].id) {
            bail!("{} is listed twice", pair[0].name)
        }

        Ok(Self {
            group: Arc::new(infos.iter().map(|info| info.id).collect()),
            infos: infos.into_boxed_slice(),
            chunk_rows: DEFAULT_CHUNK_ROWS,
            segments: (0..SEGMENTS).map(|_| OnceLock::new()).collect(),
            reserved: AtomicUsize::new(0),
            live: AtomicUsize::new(0),
            index: IdMap::default(),
            gate: AtomicUsize::new(0)
        })
    }

    /// Use chunks of `rows` rows instead of [`DEFAULT_CHUNK_ROWS`].
    ///
    /// # Panics
    ///
    /// Panics if `rows` isn't a positive multiple of 64, or the table isn't empty.
    pub fn with_chunk_rows(mut self, rows: usize) -> Self {
        assert!(rows > 0 && rows % 64 == 0, "chunk rows must be a positive multiple of 64");
        assert_eq!(*self.reserved.get_mut(), 0, "chunk rows can only change on an empty table");
        self.chunk_rows = rows;
        self
    }

    /// The sorted component ids every entity in this table has.
    pub fn group(&self) -> &ComponentIDGroup {
        &self.group
    }

    /// The same as [`group`](Self::group), shared.
    pub fn shared_group(&self) -> Arc<ComponentIDGroup> {
        self.group.clone()
    }

    /// The info of each column, in [`group`](Self::group) order.
    pub fn infos(&self) -> &[&'static ComponentInfo] {
        &self.infos
    }

    /// The number of live entities in the table.
    pub fn len(&self) -> usize {
        self.live.load(Ordering::Acquire)
    }

    /// True if the table holds no live entities.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The rows in each chunk.
    pub fn chunk_rows(&self) -> usize {
        self.chunk_rows
    }

    /// The number of chunks rows have been reserved in, see [`for_each_chunk_range`](Self::for_each_chunk_range).
    pub fn chunk_count(&self) -> usize {
        self.reserved.load(Ordering::Acquire).div_ceil(self.chunk_rows)
    }

    /// True if the index holds an entity with `id`.
    pub fn contains(&self, id: EntityID) -> bool {
        self.index.contains_key(&id)
    }

    /// The id of every live entity, in table order.
    pub fn ids(&self) -> Vec<EntityID> {
        let _inside = self.enter();
        let mut ids = Vec::with_capacity(self.len());
        for chunk in (0..self.chunk_count()).filter_map(|chunk| self.chunk(chunk)) {
            let len = chunk.published.load(Ordering::Acquire);
            ids.extend((0..len).filter(|&row| chunk.is_alive(row)).map(|row| chunk.entity(row)));
        }
        ids
    }

    /// The column holding the component `id`.
    fn column(&self, id: ComponentID) -> Option<usize> {
        self.group.binary_search(&id).ok()
    }

    /// The chunk `chunk`, if it has been created.
    fn chunk(&self, chunk: usize) -> Option<&Chunk> {
        let (segment, offset) = segment_of(chunk);
        self.segments.get(segment)?.get()?.get(offset)?.get()
    }

    /// The chunk `chunk`, creating it if needed.
    fn chunk_or_create(&self, chunk: usize) -> &Chunk {
        let (segment, offset) = segment_of(chunk);
        self.segments[segment]
            .get_or_init(|| (0..1usize << segment).map(|_| OnceLock::new()).collect())[offset]
            .get_or_init(|| Chunk::new(&self.infos, self.chunk_rows))
    }

    /// Enters the gate, waiting for a compaction already running.
    pub(crate) fn enter_raw(&self) {
        let mut spins = 0u32;
        loop {
            let users = self.gate.load(Ordering::Relaxed);
            if users & COMPACTING == 0 && self.gate.compare_exchange_weak(users, users + 1, Ordering::Acquire, Ordering::Relaxed).is_ok() { return }
            spins += 1;
            if spins < 64 { std::hint::spin_loop() } else { std::thread::yield_now() }
        }
    }

    /// Leaves the gate entered with [`enter_raw`](Self::enter_raw).
    pub(crate) fn exit(&self) {
        self.gate.fetch_sub(1, Ordering::Release);
    }

    fn enter(&self) -> Inside<'_> {
        self.enter_raw();
        Inside(self)
    }

    // ---- inserting ----

    /// Inserts an entity with the components in `bundle`.
    ///
    /// # Errors
    ///
    /// Errors if `bundle` isn't exactly this table's components.
    pub fn insert<B: Bundle>(&self, id: EntityID, bundle: B) -> anyhow::Result<()> {
        // find each component's column, every column must be written exactly once
        let mut columns = [0; MAX_PARAMS];
        let mut written = 0u16;
        if B::LEN != self.group.len() { bail!("the bundle has {} components, the table {}", B::LEN, self.group.len()) }
        for (i, column) in columns.iter_mut().enumerate().take(B::LEN) {
            let info = B::info(i);
            *column = self.column(info.id).with_context(|| format!("the table has no {}", info.name))?;
            if written & (1 << *column) != 0 { bail!("the bundle lists {} twice", info.name) }
            written |= 1 << *column;
        }

        self.insert_with(id, |chunk, row| {
            let mut dst = [std::ptr::null_mut(); MAX_PARAMS];
            for i in 0..B::LEN { dst[i] = chunk.slot(columns[i], row); }
            unsafe { bundle.write(&dst) }
        });
        Ok(())
    }

    /// Inserts an entity by moving each boxed component into its column, in any order.
    ///
    /// # Errors
    ///
    /// Errors if `components` aren't exactly this table's components, or one isn't the
    /// Rust type its component's info describes.
    pub fn insert_dyn(&self, id: EntityID, mut components: Vec<DynComponent>) -> anyhow::Result<()> {
        components.sort_by_key(|component| component.get_id());
        if components.len() != self.infos.len() { bail!("expected {} components, got {}", self.infos.len(), components.len()) }
        for (component, info) in components.iter().zip(self.infos.iter()) {
            if component.get_id() != info.id { bail!("the table has no component {:?}", component) }
            if Some(<dyn Any>::type_id(component.as_any())) != info.type_id { bail!("{component:?} isn't a {}", info.name) }
        }

        self.insert_with(id, |chunk, row| for (column, component) in components.into_iter().enumerate() {
            let layout = self.infos[column].layout;
            let raw = Box::into_raw(component) as *mut u8;
            // the value moves into the column, so only the box's memory is freed
            unsafe { std::ptr::copy_nonoverlapping(raw, chunk.slot(column, row), layout.size()) }
            free(NonNull::new(raw).unwrap(), layout);
        });
        Ok(())
    }

    /// Inserts an entity by copying each column's value from `components`, in
    /// [`group`](Self::group) order.  The table takes ownership of the values, so the
    /// caller must not drop or use them afterwards.
    ///
    /// # Errors
    ///
    /// Errors if there isn't one pointer per column.
    ///
    /// # Safety
    ///
    /// Each pointer must point at a live value of its column's component.
    pub unsafe fn insert_raw(&self, id: EntityID, components: &[*const u8]) -> anyhow::Result<()> {
        if components.len() != self.infos.len() { bail!("expected {} components, got {}", self.infos.len(), components.len()) }
        self.insert_with(id, |chunk, row| for (column, &src) in components.iter().enumerate() {
            unsafe { std::ptr::copy_nonoverlapping(src, chunk.slot(column, row), self.infos[column].layout.size()) }
        });
        Ok(())
    }

    /// Reserves a row, lets `write` fill every column of it, then publishes and indexes it.
    fn insert_with(&self, id: EntityID, write: impl FnOnce(&Chunk, usize)) {
        let _inside = self.enter();
        let reserved = self.reserved.fetch_add(1, Ordering::AcqRel);
        let (chunk_index, row) = (reserved / self.chunk_rows, reserved % self.chunk_rows);
        let chunk = self.chunk_or_create(chunk_index);

        // the row is above `published`, so nothing else reads or writes it yet
        unsafe { chunk.entities.as_ptr().add(row).write(id) }
        write(chunk, row);
        chunk.set_alive(row, true);

        // rows are published in order, so wait for inserts that reserved earlier rows here
        let mut spins = 0u32;
        while chunk.published.load(Ordering::Acquire) != row {
            spins += 1;
            if spins < 64 { std::hint::spin_loop() } else { std::thread::yield_now() }
        }
        chunk.published.store(row + 1, Ordering::Release);
        self.live.fetch_add(1, Ordering::AcqRel);
        self.index.insert(id, reserved);
    }

    // ---- removing ----

    /// Removes the entity with `id` and drops its components, returns false if there is none.
    /// Waits for every guard on its components, so don't call it while holding one.
    pub fn remove(&self, id: EntityID) -> bool {
        self.remove_with(id, |chunk, row| for (column, data) in chunk.columns.iter().enumerate() {
            if let Some(drop) = data.info.drop { unsafe { drop(chunk.slot(column, row)) } }
        }).is_some()
    }

    /// Removes the entity with `id` and moves its components into boxes, see
    /// [`remove`](Self::remove).  Components whose types are only bytes are dropped instead.
    pub fn take(&self, id: EntityID) -> Option<Vec<DynComponent>> {
        self.remove_with(id, |chunk, row| chunk.columns.iter().enumerate().filter_map(|(column, data)| {
            let slot = chunk.slot(column, row);
            match data.info.boxed {
                Some(boxed) => Some(unsafe { boxed(slot) }),
                None => { if let Some(drop) = data.info.drop { unsafe { drop(slot) } } None }
            }
        }).collect())
    }

    /// Unindexes `id`, waits for its row, and moves its values out with `f`.
    fn remove_with<R>(&self, id: EntityID, f: impl FnOnce(&Chunk, usize) -> R) -> Option<R> {
        let removed = {
            let _inside = self.enter();
            let (_, reserved) = self.index.remove(&id)?;
            let chunk = self.chunk(reserved / self.chunk_rows).expect("indexed rows are in created chunks");
            let row = reserved % self.chunk_rows;
            chunk.lock_all(row, true);
            let held = Held { chunk, rows: row..row + 1, locks: HeldLocks::All(true) };
            if !chunk.is_alive(row) { return None }
            chunk.set_alive(row, false);
            let removed = f(chunk, row);
            drop(held);
            self.live.fetch_sub(1, Ordering::AcqRel);
            removed
        };

        let (reserved, live) = (self.reserved.load(Ordering::Acquire), self.len());
        let holes = reserved - live.min(reserved);
        if holes > self.chunk_rows && holes > live { self.try_compact(); }
        Some(removed)
    }

    // ---- compacting ----

    /// Moves live rows into the holes removes left, keeping their order.  Only runs if
    /// nothing is borrowed from the table and no query is walking it, and never waits:
    /// returns false if it couldn't run.  Chunks left empty are kept for later inserts.
    pub fn try_compact(&self) -> bool {
        if self.gate.compare_exchange(0, COMPACTING, Ordering::Acquire, Ordering::Relaxed).is_err() { return false }

        // nobody is inside, so every reserved row is published; take every row's flags so
        // no guard can still be looking at one
        let chunks = (0..self.chunk_count()).filter_map(|chunk| self.chunk(chunk)).collect::<Vec<_>>();
        let published = chunks.iter().map(|chunk| chunk.published.load(Ordering::Acquire)).collect::<Vec<_>>();
        let flags = || chunks.iter().zip(&published).flat_map(|(chunk, &len)| chunk.columns.iter().flat_map(move |column| &column.flags[..len]));
        let taken = flags().take_while(|flag| borrow::try_write(flag)).count();
        if taken != flags().count() {
            flags().take(taken).for_each(borrow::unlock_write);
            self.gate.store(0, Ordering::Release);
            return false
        }

        let rows = self.chunk_rows;
        let reserved = self.reserved.load(Ordering::Acquire);
        let mut write = 0;
        for read in 0..reserved {
            let source = chunks[read / rows];
            let from = read % rows;
            if !source.is_alive(from) { continue }
            if read != write {
                let (target, to) = (chunks[write / rows], write % rows);
                let id = source.entity(from);
                unsafe {
                    target.entities.as_ptr().add(to).write(id);
                    for (column, data) in source.columns.iter().enumerate() {
                        std::ptr::copy_nonoverlapping(source.slot(column, from), target.slot(column, to), data.info.layout.size());
                    }
                }
                if let Some(mut indexed) = self.index.get_mut(&id) && *indexed == read { *indexed = write; }
                source.set_alive(from, false);
                target.set_alive(to, true);
            }
            write += 1;
        }

        // every row below `write` is now live and every row above it a hole
        for (index, chunk) in chunks.iter().enumerate() {
            chunk.published.store(write.saturating_sub(index * rows).min(rows), Ordering::Release);
        }
        self.reserved.store(write, Ordering::Release);
        flags().for_each(borrow::unlock_write);
        self.gate.store(0, Ordering::Release);
        true
    }

    /// [`try_compact`](Self::try_compact), then frees the chunks left empty.
    pub fn compact(&mut self) {
        assert!(self.try_compact(), "nothing can be borrowed from a table held mutably");
        for segment in self.segments.iter_mut().filter_map(OnceLock::get_mut) {
            for chunk in segment.iter_mut() {
                if chunk.get_mut().is_some_and(|chunk| *chunk.published.get_mut() == 0) { chunk.take(); }
            }
        }
    }

    // ---- queries ----

    /// Resolves `Q` against this table's columns.
    pub(crate) fn plan<Q: QueryGroup>(&self) -> anyhow::Result<Plan> {
        assert!(Q::LEN <= MAX_PARAMS);
        let mut plan = Plan { columns: [None; MAX_PARAMS], locks: [Lock::default(); MAX_PARAMS], lock_count: 0 };
        for i in 0..Q::LEN {
            let param = Q::param(i);
            let Some(column) = self.column(param.id) else {
                if param.optional { continue }
                bail!("the table has no component {}", param.id)
            };
            let info = self.infos[column];
            if info.type_id != Some(param.type_id) { bail!("the table's {} isn't the type the query asks for", info.name) }
            plan.columns[i] = Some(column);
            plan.locks[plan.lock_count] = Lock { column, mutable: param.mutable, shares: 1 };
            plan.lock_count += 1;
        }

        // lock in column order so rows can't deadlock each other, sharing repeated reads
        let locks = &mut plan.locks[..plan.lock_count];
        locks.sort_unstable_by_key(|lock| lock.column);
        let mut kept = 0;
        for i in 0..locks.len() {
            if kept > 0 && locks[kept - 1].column == locks[i].column {
                if locks[i].mutable || locks[kept - 1].mutable { bail!("{} is borrowed mutably more than once", self.infos[locks[i].column].name) }
                locks[kept - 1].shares += 1;
                continue
            }
            locks[kept] = locks[i];
            kept += 1;
        }
        plan.lock_count = kept;
        Ok(plan)
    }

    /// Steps `next` to the next live row from it, locked for `plan`.  The caller must be
    /// inside the gate.
    pub(crate) fn next_locked(&self, plan: &Plan, next: &mut usize) -> Option<(EntityID, &Chunk, usize)> {
        let reserved = self.reserved.load(Ordering::Acquire);
        while *next < reserved {
            let (chunk_index, row) = (*next / self.chunk_rows, *next % self.chunk_rows);
            let Some(chunk) = self.chunk(chunk_index).filter(|chunk| row < chunk.published.load(Ordering::Acquire)) else {
                *next = (chunk_index + 1) * self.chunk_rows;
                continue
            };
            *next += 1;
            if !chunk.is_alive(row) { continue }
            plan.lock(chunk, row);
            // removed while we waited for it
            if !chunk.is_alive(row) { plan.unlock(chunk, row); continue }
            return Some((chunk.entity(row), chunk, row))
        }
        None
    }

    /// The guards for `plan`'s terms on a row the caller has locked, which move into them.
    pub(crate) fn output<Q: QueryGroup>(self: &Arc<Self>, plan: &Plan, chunk: &Chunk, row: usize) -> Q::Output {
        let mut values = plan.columns.map(|column| column.map(|column| RawBorrow {
            info: self.infos[column],
            ptr: NonNull::new(chunk.slot(column, row)).unwrap(),
            flag: NonNull::from(chunk.flag(column, row)),
            table: self.clone()
        }));
        unsafe { Q::output(&mut values) }
    }

    /// Calls `f` with every live entity and its components in `Q`, one row at a time.
    ///
    /// # Errors
    ///
    /// Errors if the table lacks a required component, a component is a different type,
    /// or `Q` borrows a component mutably more than once.
    pub fn for_each<Q: QueryGroup>(&self, mut f: impl FnMut(EntityID, Q::Items<'_>)) -> anyhow::Result<()> {
        let plan = self.plan::<Q>()?;
        let _inside = self.enter();
        let mut next = 0;
        while let Some((id, chunk, row)) = self.next_locked(&plan, &mut next) {
            let _held = Held { chunk, rows: row..row + 1, locks: HeldLocks::Plan(&plan) };
            f(id, unsafe { Q::items(&plan.pointers(chunk, row)) });
        }
        Ok(())
    }

    /// Calls `f` with every run of consecutive live entities, and a slice of each of their
    /// components in `Q`.
    ///
    /// A run ends early at a row something else has borrowed, then waits for that row
    /// before carrying on, so a walk never holds rows while it waits.
    ///
    /// # Errors
    ///
    /// As for [`for_each`](Self::for_each).
    pub fn for_each_chunk<Q: QueryGroup>(&self, f: impl FnMut(&[EntityID], Q::Slices<'_>)) -> anyhow::Result<()> {
        self.for_each_chunk_range::<Q>(0..self.chunk_count(), f)
    }

    /// [`for_each_chunk`](Self::for_each_chunk) over only the chunks in `chunks`, so threads can
    /// split a table between them by [`chunk_count`](Self::chunk_count).
    ///
    /// # Errors
    ///
    /// As for [`for_each`](Self::for_each).
    pub fn for_each_chunk_range<Q: QueryGroup>(&self, chunks: Range<usize>, mut f: impl FnMut(&[EntityID], Q::Slices<'_>)) -> anyhow::Result<()> {
        let plan = self.plan::<Q>()?;
        let _inside = self.enter();
        for chunk in chunks.filter_map(|chunk| self.chunk(chunk)) {
            let len = chunk.published.load(Ordering::Acquire);
            let mut row = 0;
            while row < len {
                if !chunk.is_alive(row) { row += 1; continue }
                // wait for the first row of a run while holding nothing, then only take
                // rows that are free
                plan.lock(chunk, row);
                if !chunk.is_alive(row) { plan.unlock(chunk, row); row += 1; continue }
                let start = row;
                row += 1;
                while row < len && chunk.is_alive(row) && plan.try_lock(chunk, row) {
                    if !chunk.is_alive(row) { plan.unlock(chunk, row); break }
                    row += 1;
                }

                let _held = Held { chunk, rows: start..row, locks: HeldLocks::Plan(&plan) };
                let entities = unsafe { std::slice::from_raw_parts(chunk.entities.as_ptr().add(start), row - start) };
                f(entities, unsafe { Q::slices(&plan.pointers(chunk, start), row - start) });
            }
        }
        Ok(())
    }

    /// Finds the row of `id` and locks it with `lock`, leaving it locked.  The caller must
    /// be inside the gate.
    fn locked_row(&self, id: EntityID, lock: impl Fn(&Chunk, usize), unlock: impl Fn(&Chunk, usize)) -> Option<(&Chunk, usize)> {
        loop {
            let reserved = *self.index.get(&id)?;
            let chunk = self.chunk(reserved / self.chunk_rows)?;
            let row = reserved % self.chunk_rows;
            lock(chunk, row);
            if chunk.is_alive(row) { return Some((chunk, row)) }
            unlock(chunk, row);
            // removed while we waited, look again in case it was inserted again meanwhile
            if self.index.get(&id).is_none_or(|now| *now == reserved) { return None }
        }
    }

    /// Calls `f` with the components in `Q` of the entity `id`, returns `None` if there is
    /// no such entity.
    ///
    /// # Errors
    ///
    /// As for [`for_each`](Self::for_each).
    pub fn get<Q: QueryGroup, R>(&self, id: EntityID, f: impl FnOnce(Q::Items<'_>) -> R) -> anyhow::Result<Option<R>> {
        let plan = self.plan::<Q>()?;
        let _inside = self.enter();
        let Some((chunk, row)) = self.locked_row(id, |chunk, row| plan.lock(chunk, row), |chunk, row| plan.unlock(chunk, row)) else { return Ok(None) };
        let _held = Held { chunk, rows: row..row + 1, locks: HeldLocks::Plan(&plan) };
        Ok(Some(f(unsafe { Q::items(&plan.pointers(chunk, row)) })))
    }

    /// Guards over the components in `Q` of the entity `id`, or `None` if there is no
    /// such entity.  Blocks while other guards hold them mutably.
    ///
    /// # Errors
    ///
    /// As for [`for_each`](Self::for_each).
    pub fn fetch<Q: QueryGroup>(self: &Arc<Self>, id: EntityID) -> anyhow::Result<Option<Q::Output>> {
        let plan = self.plan::<Q>()?;
        let _inside = self.enter();
        let Some((chunk, row)) = self.locked_row(id, |chunk, row| plan.lock(chunk, row), |chunk, row| plan.unlock(chunk, row)) else { return Ok(None) };
        Ok(Some(self.output::<Q>(&plan, chunk, row)))
    }

    /// Calls `f` with every component of the entity `id` as [`RawComponent`]s in
    /// [`group`](Self::group) order, borrowed shared, returns `None` if there is no such entity.
    pub fn get_raw<R>(&self, id: EntityID, f: impl FnOnce(&[RawComponent<'_>]) -> R) -> Option<R> {
        let _inside = self.enter();
        let (chunk, row) = self.locked_row(id, |chunk, row| chunk.lock_all(row, false), |chunk, row| chunk.unlock_all(row, false))?;
        let _held = Held { chunk, rows: row..row + 1, locks: HeldLocks::All(false) };
        let components = chunk.columns.iter().enumerate()
            .map(|(column, data)| RawComponent { info: data.info, ptr: chunk.slot(column, row), _borrow: std::marker::PhantomData })
            .collect::<Vec<_>>();
        Some(f(&components))
    }

    /// The component `component` of the entity `id` borrowed shared, or `None` if there is
    /// no such entity or component.  Blocks while a mutable guard holds it.
    pub fn component(self: &Arc<Self>, id: EntityID, component: ComponentID) -> Option<ComponentRef> {
        let column = self.column(component)?;
        let raw = self.borrow_one(id, column, false)?;
        Some(unsafe { ComponentRef::new(raw) })
    }

    /// The component `component` of the entity `id` borrowed mutably, or `None` if there is
    /// no such entity or component.  Blocks while any other guard holds it.
    pub fn component_mut(self: &Arc<Self>, id: EntityID, component: ComponentID) -> Option<ComponentMut> {
        let column = self.column(component)?;
        let raw = self.borrow_one(id, column, true)?;
        Some(unsafe { ComponentMut::new(raw) })
    }

    fn borrow_one(self: &Arc<Self>, id: EntityID, column: usize, mutable: bool) -> Option<RawBorrow> {
        let _inside = self.enter();
        let (chunk, row) = self.locked_row(
            id,
            |chunk, row| if mutable { borrow::write(chunk.flag(column, row)) } else { borrow::read(chunk.flag(column, row)) },
            |chunk, row| unlock(chunk.flag(column, row), mutable, 1)
        )?;
        Some(RawBorrow {
            info: self.infos[column],
            ptr: NonNull::new(chunk.slot(column, row)).unwrap(),
            flag: NonNull::from(chunk.flag(column, row)),
            table: self.clone()
        })
    }
}

/// Walks a [`ChunkedTable`]'s live rows by position, see [`ChunkedTable::walk`].
///
/// While a walker lives the table can't compact, so rows don't move: a position it handed
/// out keeps naming the same entity until that entity is removed, and
/// [`read`](Self::read) and [`write`](Self::write) can reach it without looking up its id.
pub struct RowWalker {
    table: Arc<ChunkedTable>,
    /// The next position to look at.
    next: usize,
    /// Rows reserved when the walk began, so entities inserted since aren't visited.
    end: usize
}

impl ChunkedTable {
    /// Walk the table's live rows by position, holding no borrows between rows.  Entities
    /// inserted after this is called aren't visited.  Keeps the table from compacting until
    /// the walker is dropped, so don't keep one longer than needed.
    pub fn walk(self: &Arc<Self>) -> RowWalker {
        self.enter_raw();
        RowWalker { table: self.clone(), next: 0, end: self.reserved.load(Ordering::Acquire) }
    }

    /// The column holding the component `id`, in [`group`](Self::group) order.
    pub fn column_of(&self, id: ComponentID) -> Option<usize> {
        self.column(id)
    }
}

impl RowWalker {
    /// The table being walked.
    pub fn table(&self) -> &Arc<ChunkedTable> {
        &self.table
    }

    /// The next live row's entity and position.  Nothing is borrowed, so the row may be
    /// removed before it's read, which [`read`](Self::read) and [`write`](Self::write) report.
    pub fn next_row(&mut self) -> Option<(EntityID, usize)> {
        let rows = self.table.chunk_rows;
        while self.next < self.end {
            let (chunk, row) = (self.next / rows, self.next % rows);
            let Some(chunk) = self.table.chunk(chunk).filter(|chunk| row < chunk.published.load(Ordering::Acquire)) else {
                self.next = (self.next / rows + 1) * rows;
                continue
            };
            let position = self.next;
            self.next += 1;
            if chunk.is_alive(row) { return Some((chunk.entity(row), position)) }
        }
        None
    }

    /// Calls `f` with the component in `column` of the row at `position`, borrowed shared.
    /// `None` if the row's entity has been removed, or the column's type is only bytes.
    /// Blocks while a mutable guard holds it.
    pub fn read<R>(&self, position: usize, column: usize, f: impl FnOnce(&dyn Component) -> R) -> Option<R> {
        self.borrow(position, column, false, |value| f(unsafe { &*value }))
    }

    /// Calls `f` with the component in `column` of the row at `position`, borrowed mutably.
    /// `None` as for [`read`](Self::read).  Blocks while any other guard holds it.
    pub fn write<R>(&self, position: usize, column: usize, f: impl FnOnce(&mut dyn Component) -> R) -> Option<R> {
        self.borrow(position, column, true, |value| f(unsafe { &mut *value }))
    }

    fn borrow<R>(&self, position: usize, column: usize, mutable: bool, f: impl FnOnce(*mut dyn Component) -> R) -> Option<R> {
        /// Releases the flag, even if `f` panics.
        struct Release<'a>(&'a AtomicU32, bool);
        impl Drop for Release<'_> {
            fn drop(&mut self) { unlock(self.0, self.1, 1) }
        }

        let table = &self.table;
        if position >= self.end { return None }
        let as_dyn = table.infos.get(column)?.as_dyn?;
        let chunk = table.chunk(position / table.chunk_rows)?;
        let row = position % table.chunk_rows;
        let flag = chunk.flag(column, row);
        if mutable { borrow::write(flag) } else { borrow::read(flag) }
        let _release = Release(flag, mutable);
        // holes aren't reused while the table can't compact, so a live row is still ours
        if !chunk.is_alive(row) { return None }
        Some(f(unsafe { as_dyn(chunk.slot(column, row)) }))
    }
}

impl Drop for RowWalker {
    fn drop(&mut self) {
        self.table.exit();
    }
}

impl Debug for RowWalker {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("RowWalker").field("table", &self.table).field("next", &self.next).field("end", &self.end).finish()
    }
}

impl Drop for ChunkedTable {
    fn drop(&mut self) {
        debug_assert_eq!(*self.gate.get_mut(), 0);
    }
}

impl Debug for ChunkedTable {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChunkedTable")
            .field("components", &self.infos.iter().map(|info| info.name).collect::<Vec<_>>())
            .field("len", &self.len())
            .field("chunks", &self.chunk_count())
            .finish()
    }
}

/// The segment chunk `chunk` lives in, and its offset there.
fn segment_of(chunk: usize) -> (usize, usize) {
    let segment = (chunk + 1).ilog2() as usize;
    (segment, chunk + 1 - (1 << segment))
}

/// The layout of `rows` values of `info`.
fn column_layout(info: &ComponentInfo, rows: usize) -> Layout {
    Layout::from_size_align(info.layout.size() * rows, info.layout.align()).expect("chunk too large")
}

/// Allocates `layout`, or a dangling pointer for zero sized layouts.
fn allocate(layout: Layout) -> NonNull<u8> {
    if layout.size() == 0 { return NonNull::new(std::ptr::without_provenance_mut(layout.align())).unwrap() }
    NonNull::new(unsafe { std::alloc::alloc(layout) }).unwrap_or_else(|| std::alloc::handle_alloc_error(layout))
}

/// Frees memory from [`allocate`], or a box's memory once its value has moved out.
fn free(ptr: NonNull<u8>, layout: Layout) {
    if layout.size() != 0 { unsafe { std::alloc::dealloc(ptr.as_ptr(), layout) } }
}


#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::AtomicUsize};

    use super::*;
    use crate::worlds::test_support::{A, B, C, D};

    /// Counts how many times it has been dropped.
    #[derive(Debug, Component)]
    struct Tracked(Arc<AtomicUsize>);

    impl Drop for Tracked {
        fn drop(&mut self) { self.0.fetch_add(1, Ordering::Relaxed); }
    }

    #[derive(Debug, Component)]
    struct Marker;

    /// A table of `A(i)`, `B(i * 10)` for each id `0..count`, in small chunks.
    fn table_of(count: u32) -> ChunkedTable {
        let table = ChunkedTable::new::<(A, B)>().with_chunk_rows(64);
        for i in 0..count { table.insert(i as EntityID, (A(i), B(i * 10))).unwrap(); }
        table
    }

    fn ids(table: &ChunkedTable) -> Vec<EntityID> {
        let mut ids = vec![];
        table.for_each::<&A>(|id, _| ids.push(id)).unwrap();
        ids
    }

    // ---- insert and read ----

    #[test]
    fn insert_and_get() {
        let table = table_of(200);
        assert_eq!(table.len(), 200);
        assert_eq!(table.chunk_count(), 4);
        assert_eq!(table.group().to_vec(), crate::worlds::test_support::sorted(vec![A::id(), B::id()]));
        assert_eq!(table.get::<(&B, &A), _>(150, |(b, a)| (a.0, b.0)).unwrap(), Some((150, 1500)));
        assert_eq!(table.get::<&A, _>(200, |a| a.0).unwrap(), None);
        assert!(table.contains(3) && !table.contains(300));
    }

    #[test]
    fn bundle_order_does_not_matter() {
        let table = ChunkedTable::new::<(A, B, C)>();
        table.insert(1, (C(3), A(1), B(2))).unwrap();
        table.insert(2, (A(4), B(5), C(6))).unwrap();
        assert_eq!(table.get::<(&A, &B, &C), _>(1, |(a, b, c)| (a.0, b.0, c.0)).unwrap(), Some((1, 2, 3)));
        assert_eq!(table.get::<(&C, &A), _>(2, |(c, a)| (a.0, c.0)).unwrap(), Some((4, 6)));
    }

    #[test]
    fn wrong_bundles_are_rejected() {
        let table = ChunkedTable::new::<(A, B)>();
        assert!(table.insert(1, (A(1),)).is_err());
        assert!(table.insert(1, (A(1), C(1))).is_err());
        assert!(table.insert(1, (A(1), A(2))).is_err());
        assert!(table.insert(1, (A(1), B(1), C(1))).is_err());
        assert!(table.is_empty());
        assert!(ChunkedTable::from_infos([ComponentInfo::of::<A>(), ComponentInfo::of::<A>()]).is_err());
        // entities without components still get a table, though no query finds them
        let empty = ChunkedTable::from_infos([]).unwrap();
        unsafe { empty.insert_raw(1, &[]).unwrap() };
        assert!(empty.contains(1) && empty.take(1).unwrap().is_empty());

        // infos must be the registered ones
        let forged: &'static ComponentInfo = Box::leak(Box::new(*ComponentInfo::of::<A>()));
        assert!(ChunkedTable::from_infos([forged]).is_err());
    }

    #[test]
    fn raw_insert_and_access() {
        let table = ChunkedTable::from_infos([ComponentInfo::of::<B>(), ComponentInfo::of::<A>()]).unwrap();
        let (a, b) = (A(7), B(8));
        let pointers = table.infos().iter()
            .map(|info| if info.id == A::id() { &a as *const A as *const u8 } else { &b as *const B as *const u8 })
            .collect::<Vec<_>>();
        unsafe { table.insert_raw(1, &pointers).unwrap() };
        assert!(unsafe { table.insert_raw(2, &pointers[..1]) }.is_err());

        let printed = table.get_raw(1, |components| {
            assert_eq!(components.iter().find_map(|c| c.downcast_ref::<A>()).unwrap().0, 7);
            assert!(components.iter().all(|c| c.downcast_ref::<C>().is_none()));
            components.iter().map(|c| format!("{c:?}")).collect::<Vec<_>>()
        }).unwrap();
        let mut expected = vec!["A(7)".to_string(), "B(8)".to_string()];
        expected.sort_by_key(|name| if name.starts_with('A') { A::id() } else { B::id() });
        assert_eq!(printed, expected);
        assert!(table.get_raw(2, |_| ()).is_none());
    }

    #[test]
    fn zero_sized_components() {
        let table = ChunkedTable::new::<(Marker, A)>().with_chunk_rows(64);
        for i in 0..100 { table.insert(i, (Marker, A(i as u32))).unwrap(); }
        let mut count = 0;
        table.for_each_chunk::<(&Marker, &A)>(|_, (markers, a)| { assert_eq!(markers.len(), a.len()); count += a.len(); }).unwrap();
        assert_eq!(count, 100);
    }

    // ---- queries ----

    #[test]
    fn for_each_writes_through() {
        let table = table_of(130);
        table.for_each::<(&mut A, &B)>(|_, (a, b)| a.0 += b.0).unwrap();
        let mut sum = 0;
        table.for_each::<&A>(|id, a| { assert_eq!(a.0, id as u32 * 11); sum += 1 }).unwrap();
        assert_eq!(sum, 130);
    }

    #[test]
    fn for_each_chunk_hands_out_runs() {
        let table = table_of(150);
        let mut runs = vec![];
        table.for_each_chunk::<(&mut A, &B)>(|entities, (a, b)| {
            assert_eq!((entities.len(), a.len()), (b.len(), b.len()));
            for (a, b) in a.iter_mut().zip(b) { a.0 = b.0 + 1; }
            runs.push((entities[0], entities.len()));
        }).unwrap();
        assert_eq!(runs, [(0, 64), (64, 64), (128, 22)]);
        assert_eq!(table.get::<&A, _>(149, |a| a.0).unwrap(), Some(1491));

        // the range form only visits the chunks it's given
        let mut seen = 0;
        table.for_each_chunk_range::<&A>(1..2, |entities, _| seen += entities.len()).unwrap();
        assert_eq!(seen, 64);
    }

    #[test]
    fn optional_terms() {
        let table = table_of(3);
        let mut found = vec![];
        table.for_each::<(&A, Option<&C>, Option<&mut B>)>(|_, (a, c, b)| found.push((a.0, c.is_none(), b.map(|b| b.0)))).unwrap();
        assert_eq!(found, [(0, true, Some(0)), (1, true, Some(10)), (2, true, Some(20))]);
        table.for_each_chunk::<Option<&D>>(|_, d| assert!(d.is_none())).unwrap();
    }

    #[test]
    fn bad_queries_error() {
        let table = table_of(3);
        assert!(table.for_each::<(&A, &C)>(|_, _| {}).is_err());
        assert!(table.for_each::<(&mut A, &A)>(|_, _| {}).is_err());
        assert!(table.for_each::<(&mut A, Option<&mut A>)>(|_, _| {}).is_err());
        assert!(table.get::<(&A, &mut A), _>(1, |_| ()).is_err());
        // a repeated shared borrow is fine
        assert_eq!(table.get::<(&A, &A), _>(2, |(a, b)| a.0 + b.0).unwrap(), Some(4));
    }

    // ---- remove and compact ----

    #[test]
    fn removed_rows_are_skipped() {
        let table = table_of(200);
        for id in (0..200).filter(|id| id % 3 == 0 || (70..140).contains(id)) { assert!(table.remove(id)); }
        assert!(!table.remove(0));
        assert!(!table.remove(999));

        let expected = (0..200).filter(|id| id % 3 != 0 && !(70..140).contains(id)).collect::<Vec<_>>();
        assert_eq!(ids(&table), expected);
        assert_eq!(table.len(), expected.len());
        assert_eq!(table.get::<&A, _>(3, |a| a.0).unwrap(), None);

        // runs never cover a hole
        table.for_each_chunk::<(&A,)>(|entities, (a,)| for (id, a) in entities.iter().zip(a) {
            assert!(expected.contains(id));
            assert_eq!(a.0 as EntityID, *id);
        }).unwrap();
    }

    #[test]
    fn compact_fills_holes_and_frees_chunks() {
        let mut table = table_of(300);
        for id in (0..300).filter(|id| id % 2 == 0 || *id >= 200) { table.remove(id); }
        table.compact();
        assert_eq!(table.len(), 100);
        assert_eq!(table.chunk_count(), 2);
        assert_eq!(ids(&table), (0..200).filter(|id| id % 2 == 1).collect::<Vec<_>>());
        for id in (1..200).step_by(2) {
            assert_eq!(table.get::<(&A, &B), _>(id, |(a, b)| (a.0, b.0)).unwrap(), Some((id as u32, id as u32 * 10)));
        }

        // inserts carry on after the compacted rows
        table.insert(1000, (A(1000), B(0))).unwrap();
        assert_eq!(*ids(&table).last().unwrap(), 1000);
        assert_eq!(table.get::<&A, _>(1000, |a| a.0).unwrap(), Some(1000));

        // compacting everything away leaves an empty, usable table
        for id in ids(&table) { table.remove(id); }
        table.compact();
        assert_eq!((table.len(), table.chunk_count()), (0, 0));
        table.insert(5, (A(5), B(5))).unwrap();
        assert_eq!(ids(&table), [5]);
    }

    #[test]
    fn removes_compact_once_holes_outnumber_rows() {
        let table = table_of(1000);
        for id in 0..900 { table.remove(id); }
        // compacted along the way, so the remaining rows fit in the first chunks
        assert!(table.chunk_count() <= 3, "{} chunks", table.chunk_count());
        assert_eq!(ids(&table), (900..1000).collect::<Vec<_>>());
        assert_eq!(table.get::<&B, _>(950, |b| b.0).unwrap(), Some(9500));
    }

    #[test]
    fn borrowed_tables_dont_compact() {
        let table = Arc::new(table_of(200));
        for id in 0..150 { table.remove(id); }
        let guard = table.fetch::<&A>(199).unwrap().unwrap();
        assert!(!table.try_compact());
        drop(guard);
        assert!(table.try_compact());
        assert_eq!(table.chunk_count(), 1);
        assert_eq!(table.fetch::<&A>(199).unwrap().unwrap().0, 199);
    }

    #[test]
    fn take_moves_components_out() {
        let table = table_of(3);
        let taken = table.take(1).unwrap();
        assert_eq!(taken.len(), 2);
        assert_eq!(taken.iter().find_map(|c| c.as_any().downcast_ref::<B>()).unwrap().0, 10);
        assert!(table.take(1).is_none());
        assert_eq!(ids(&table), [0, 2]);
    }

    #[test]
    fn drops_every_value_once() {
        let drops = Arc::new(AtomicUsize::new(0));
        let mut table = ChunkedTable::new::<(Tracked, A)>().with_chunk_rows(64);
        for i in 0..150 { table.insert(i, (Tracked(drops.clone()), A(0))).unwrap(); }
        for i in 0..50 { table.remove(i); }
        assert_eq!(drops.load(Ordering::Relaxed), 50);
        table.compact();
        assert_eq!(drops.load(Ordering::Relaxed), 50);
        drop(table);
        assert_eq!(drops.load(Ordering::Relaxed), 150);
        assert_eq!(Arc::strong_count(&drops), 1);
    }

    #[test]
    fn duplicate_ids_index_the_last() {
        let table = ChunkedTable::new::<(A,)>();
        table.insert(1, (A(1),)).unwrap();
        table.insert(1, (A(2),)).unwrap();
        assert_eq!(ids(&table), [1, 1]);
        assert_eq!(table.get::<&A, _>(1, |a| a.0).unwrap(), Some(2));
        assert!(table.remove(1));
        assert_eq!(ids(&table), [1]);
        assert!(!table.remove(1));
    }

    // ---- concurrency ----

    #[test]
    fn concurrent_inserts_lose_nothing() {
        const THREADS: u32 = 8;
        const PER_THREAD: u32 = 5_000;
        let table = ChunkedTable::new::<(A, B)>().with_chunk_rows(64);
        std::thread::scope(|s| for t in 0..THREADS {
            let table = &table;
            s.spawn(move || for i in 0..PER_THREAD {
                let id = t * PER_THREAD + i;
                table.insert(id as EntityID, (A(id), B(id))).unwrap();
            });
        });

        let mut found = ids(&table);
        found.sort();
        assert_eq!(found, (0..(THREADS * PER_THREAD) as EntityID).collect::<Vec<_>>());
        assert_eq!(table.len(), (THREADS * PER_THREAD) as usize);
        table.for_each::<(&A, &B)>(|id, (a, b)| assert_eq!((a.0 as EntityID, b.0 as EntityID), (id, id))).unwrap();
    }

    #[test]
    fn readers_writers_inserts_and_removes_at_once() {
        const ROUNDS: u32 = 200;
        let table = ChunkedTable::new::<(A, B)>().with_chunk_rows(64);
        for i in 0..1000 { table.insert(i, (A(0), B(1))).unwrap(); }

        std::thread::scope(|s| {
            let table = &table;
            // writers add B to A, so every A stays a multiple of B's 1
            for _ in 0..2 {
                s.spawn(move || for _ in 0..ROUNDS { table.for_each_chunk::<(&mut A, &B)>(|_, (a, b)| for (a, b) in a.iter_mut().zip(b) { a.0 += b.0 }).unwrap() });
            }
            // readers check values are never torn or dropped
            for _ in 0..2 {
                s.spawn(move || for _ in 0..ROUNDS { table.for_each::<(&A, &B)>(|_, (a, b)| { assert_eq!(b.0, 1); assert!(a.0 <= ROUNDS * 2); }).unwrap() });
            }
            s.spawn(move || for i in 1000..3000 { table.insert(i, (A(0), B(1))).unwrap() });
            s.spawn(move || for i in (0..1000).step_by(2) { assert!(table.remove(i)); });
            s.spawn(move || for i in 0..1000 { table.get::<&mut A, _>(i, |a| a.0 += 0).unwrap(); });
        });

        assert_eq!(table.len(), 2500);
        let mut found = ids(&table);
        found.sort();
        assert_eq!(found, (1..1000).step_by(2).chain(1000..3000).collect::<Vec<_>>());
    }

    #[test]
    fn guards_removes_and_compaction_at_once() {
        const ROUNDS: u64 = 3_000;
        let table = Arc::new(ChunkedTable::new::<(A, B)>().with_chunk_rows(64));
        std::thread::scope(|s| {
            // each churner owns its own ids, inserting and removing them so compaction keeps moving rows
            for t in 0..3u64 {
                let table = &table;
                s.spawn(move || for i in 0..ROUNDS {
                    let id = t * 1_000_000 + i;
                    table.insert(id, (A(id as u32), B(7))).unwrap();
                    if i >= 10 { assert!(table.remove(id - 10)); }
                });
            }
            // readers hold guards on whatever they find, and check each still belongs to its entity
            for _ in 0..2 {
                let table = &table;
                s.spawn(move || for i in 0..ROUNDS {
                    let id = (i % 3) * 1_000_000 + i;
                    if let Some((a, mut b)) = table.fetch::<(&A, &mut B)>(id).unwrap() {
                        assert_eq!((a.0 as u64, b.0), (id, 7));
                        b.0 = 7;
                    }
                    table.try_compact();
                });
            }
        });
        assert_eq!(table.len(), 30);
        let mut found = table.ids();
        found.sort();
        let expected = (0..3u64).flat_map(|t| (ROUNDS - 10..ROUNDS).map(move |i| t * 1_000_000 + i)).collect::<Vec<_>>();
        assert_eq!(found, expected);
    }

    #[test]
    fn segments_cover_every_chunk() {
        assert_eq!(segment_of(0), (0, 0));
        assert_eq!(segment_of(1), (1, 0));
        assert_eq!(segment_of(2), (1, 1));
        assert_eq!(segment_of(3), (2, 0));
        assert_eq!(segment_of(6), (2, 3));
        assert_eq!(segment_of(7), (3, 0));
    }

    // ---- walking by position ----

    #[test]
    fn walkers_read_and_write_by_position() {
        let table = Arc::new(table_of(100));
        let (a, b) = (table.column_of(A::id()).unwrap(), table.column_of(B::id()).unwrap());
        let mut walker = table.walk();
        let mut rows = vec![];
        while let Some((id, position)) = walker.next_row() {
            assert_eq!(walker.read(position, a, |value| value.as_any().downcast_ref::<A>().unwrap().0), Some(id as u32));
            rows.push((id, position));
        }
        assert_eq!(rows.len(), 100);

        // positions still name their entities, and removed ones read as gone
        table.remove(7);
        for &(id, position) in &rows {
            let written = walker.write(position, b, |value| value.as_any_mut().downcast_mut::<B>().unwrap().0 += 1);
            assert_eq!(written.is_some(), id != 7);
        }
        drop(walker);
        assert_eq!(table.get::<&B, _>(8, |b| b.0).unwrap(), Some(81));
    }

    #[test]
    fn walkers_skip_later_inserts_and_block_compaction() {
        let table = Arc::new(table_of(10));
        let mut walker = table.walk();
        table.insert(100, (A(100), B(1000))).unwrap();
        for id in 0..10 { table.remove(id); }
        assert!(!table.try_compact(), "a walker keeps rows in place");
        assert!(walker.next_row().is_none(), "removed rows are skipped and the new one is past the end");
        drop(walker);
        assert!(table.try_compact());
        assert_eq!(ids(&table), [100]);
    }
}
