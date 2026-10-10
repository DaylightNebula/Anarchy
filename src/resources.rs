use std::{any::TypeId, fmt::Debug, ops::{Deref, DerefMut}};

use mutual::AsAny;
pub use anarchy_macros::Resource;

use crate::{SharedExecutionState, SystemKey, SystemMeta, SystemParam, World, fast_hash_type_id};

/// Identifies a resource type, see [`fast_hash_type_id`].
pub type ResourceID = u64;
/// A type erased resource.
pub type DynResource = Box<dyn Resource>;

/// Type level access to a resource's ids, implemented by `#[derive(Resource)]`.
pub trait ResourceMeta: AsAny + Resource + 'static {
    /// The id of this resource type.
    fn id() -> ResourceID { fast_hash_type_id(TypeId::of::<Self>()) }
    /// This resource type as a [`SystemMeta`], so systems can be tagged with it.
    fn system_meta() -> SystemMeta { TypeId::of::<Self>() }
}

/// A single value of a type stored on a world.  Usually implemented with
/// `#[derive(Resource)]`, which also implements [`ResourceMeta`] and [`AsAny`].
pub trait Resource: AsAny + Debug + Send + 'static {
    /// The id of this value's resource type, the same as [`ResourceMeta::id`].
    fn get_id(&self) -> ResourceID { fast_hash_type_id(TypeId::of::<Self>()) }
    /// The same as [`ResourceMeta::system_meta`].
    fn get_system_meta(&self) -> SystemMeta { TypeId::of::<Self>() }
}

// Impl AsAny for Boxed Resource for casting
impl AsAny for Box<dyn Resource> {
    fn as_any(&self) -> &dyn std::any::Any { (**self).as_any() }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { (**self).as_any_mut() }
}

/// `SystemParam` to access a `Resource` immutably in a system.
///
/// Extracting fails, so the system returns an error without running, if the world
/// doesn't hold an `R`. Blocks while a [`ResMut<R>`] is held.
pub struct Res<R: Resource + ResourceMeta + 'static> { guard: mutual::RefCastGuard<Box<dyn Resource>, R> }
impl <R: Resource + ResourceMeta + 'static> SystemParam for Res<R> {
    fn extract(world: &World, _exec_state: &SharedExecutionState, _system: SystemKey) -> anyhow::Result<Self> {
        Ok(Self { guard: world.resource()? })
    }
}

impl <R: Resource + ResourceMeta + 'static> Deref for Res<R> {
    type Target = R;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}


/// `SystemParam` to access a `Resource` mutably in a system.
///
/// Extracting fails, so the system returns an error without running, if the world
/// doesn't hold an `R`. Blocks while any other guard to the resource is held, so a
/// system taking both `Res<R>` and `ResMut<R>` (or two `ResMut<R>`) deadlocks.
pub struct ResMut<R: Resource + ResourceMeta + 'static> { guard: mutual::MutCastGuard<Box<dyn Resource>, R> }
impl <R: Resource + ResourceMeta + 'static> SystemParam for ResMut<R> {
    fn extract(world: &World, _exec_state: &SharedExecutionState, _system: SystemKey) -> anyhow::Result<Self> {
        Ok(Self { guard: world.resource_mut()? })
    }
}

impl <R: Resource + ResourceMeta + 'static> Deref for ResMut<R> {
    type Target = R;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl <R: Resource + ResourceMeta + 'static> DerefMut for ResMut<R> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, atomic::{AtomicBool, AtomicU32, Ordering}};

    use crate::*;

    #[derive(Debug, PartialEq, Resource)]
    struct Counter(u32);

    #[derive(Debug, PartialEq, Resource)]
    struct Total(u32);

    #[test]
    fn res_reads_resource() {
        static READ: AtomicU32 = AtomicU32::new(0);
        fn read(counter: Res<Counter>) { READ.store(counter.0, Ordering::Release); }

        let world = World::default();
        world.insert_resource(Counter(3));
        read.into_system().run(&world, &SharedExecutionState::default()).unwrap();
        assert_eq!(READ.load(Ordering::Acquire), 3);
    }

    #[test]
    fn res_mut_writes_resource() {
        fn bump(mut counter: ResMut<Counter>) { counter.0 += 1; }

        let world = World::default();
        world.insert_resource(Counter(0));
        let exec_state = SharedExecutionState::default();
        let system = bump.into_system();
        system.run(&world, &exec_state).unwrap();
        system.run(&world, &exec_state).unwrap();
        assert_eq!(*world.resource::<Counter>().unwrap(), Counter(2));
    }

    #[test]
    fn reader_after_writer_sees_write() {
        static READ: AtomicU32 = AtomicU32::new(0);
        fn bump(mut counter: ResMut<Counter>) { counter.0 += 1; }
        fn read(counter: Res<Counter>) { READ.store(counter.0, Ordering::Release); }

        let mut graph = SystemGraph::new();
        graph.append_system(bump, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(read, after(bump), std::iter::empty()).unwrap();

        let world = World::default();
        world.insert_resource(Counter(10));
        SingleThreadedExecutor::new().run(&world, &graph).unwrap();
        assert_eq!(READ.load(Ordering::Acquire), 11);
    }

    #[test]
    fn missing_resource_is_an_error() {
        static RAN: AtomicBool = AtomicBool::new(false);
        fn read(_counter: Res<Counter>) { RAN.store(true, Ordering::Release); }
        fn bump(_counter: ResMut<Counter>) { RAN.store(true, Ordering::Release); }

        let world = World::default();
        let exec_state = SharedExecutionState::default();
        assert!(read.into_system().run(&world, &exec_state).is_err());
        assert!(bump.into_system().run(&world, &exec_state).is_err());
        assert!(!RAN.load(Ordering::Acquire));
    }

    #[test]
    fn removed_resource_is_an_error() {
        fn read(_counter: Res<Counter>) {}

        let world = World::default();
        world.insert_resource(Counter(1));
        let exec_state = SharedExecutionState::default();
        let system = read.into_system();
        assert!(system.run(&world, &exec_state).is_ok());
        world.remove_resource::<Counter>();
        assert!(system.run(&world, &exec_state).is_err());
    }

    #[test]
    fn mixed_params() {
        static READ: Mutex<Vec<u32>> = Mutex::new(Vec::new());
        fn add(counter: Res<Counter>, mut total: ResMut<Total>, events: Event<u32>, _world: World) {
            for amount in events.read() { total.0 += amount * counter.0; }
            READ.lock().unwrap().push(total.0);
        }

        let world = World::default();
        world.insert_resource(Counter(2));
        world.insert_resource(Total(0));
        world.send_event(3u32);
        world.send_event(4u32);
        add.into_system().run(&world, &SharedExecutionState::default()).unwrap();
        assert_eq!(*READ.lock().unwrap(), vec![14]);
        assert_eq!(*world.resource::<Total>().unwrap(), Total(14));
        assert_eq!(*world.resource::<Counter>().unwrap(), Counter(2));
    }

    #[test]
    fn concurrent_res_mut_loses_nothing() {
        macro_rules! bumpers {
            ($($name:ident),*) => {
                $(fn $name(mut counter: ResMut<Counter>) {
                    // read and write in separate steps so a missing lock would lose updates
                    for _ in 0..100 { let value = counter.0; counter.0 = value + 1; }
                })*
            };
        }
        bumpers!(b0, b1, b2, b3, b4, b5, b6, b7);

        let mut graph = SystemGraph::new();
        graph.append_system(b0, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(b1, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(b2, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(b3, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(b4, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(b5, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(b6, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(b7, SystemInstruction::default(), std::iter::empty()).unwrap();

        let world = World::default();
        world.insert_resource(Counter(0));
        MultiThreadedExecutor::new().run(&world, &graph, 4, &AtomicBool::new(false)).unwrap();
        assert_eq!(*world.resource::<Counter>().unwrap(), Counter(800));
    }
}
