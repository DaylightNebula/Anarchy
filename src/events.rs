//! Events, values one system sends that every system may read once.
//!
//! An event of type `E` lives in an [`EventQueue<E>`] resource on the world.
//! Each system that reads `E` has its own cursor in that queue, so it sees
//! every event once, including ones it sent itself.  Events expire
//! [`EVENT_LIFETIME`] after they were sent, so a system has to run within
//! that window to see them.

use std::{any::Any, collections::VecDeque, fmt::Debug, marker::PhantomData, time::{Duration, Instant}};

use ahash::AHashMap;
use mutual::AsAny;

use crate::{Resource, ResourceMeta, SharedExecutionState, SystemKey, SystemParam, World};

/// How long an event can be read after it was sent.
pub const EVENT_LIFETIME: Duration = Duration::from_secs(1);

/// Every live event of type `E` and how far each system has read through
/// them.  Kept as a resource, created the first time `E` is sent or read.
pub struct EventQueue<E> {
    /// Live events, oldest first, as `(sequence, sent at, event)`.
    events: VecDeque<(u64, Instant, E)>,
    /// The sequence number the next event gets.
    next_seq: u64,
    /// The first sequence number each system has not read yet.  A system
    /// missing from the map has read nothing.
    cursors: AHashMap<SystemKey, u64>,
    /// How long events stay readable.
    lifetime: Duration
}

impl <E: Clone + Send + 'static> EventQueue<E> {
    /// Create an empty queue whose events stay readable for `lifetime`.
    pub fn with_lifetime(lifetime: Duration) -> Self {
        Self { events: VecDeque::new(), next_seq: 0, cursors: AHashMap::new(), lifetime }
    }

    /// Queue an event for every system to read.
    pub fn send(&mut self, event: E) {
        let now = Instant::now();
        self.prune(now);
        self.events.push_back((self.next_seq, now, event));
        self.next_seq += 1;
    }

    /// Clone out every live event `system` has not read yet, oldest first,
    /// and mark them read for it.
    pub fn read(&mut self, system: SystemKey) -> Vec<E> {
        self.prune(Instant::now());
        let cursor = self.cursors.insert(system, self.next_seq).unwrap_or(0);
        let start = self.events.partition_point(|(seq, _, _)| *seq < cursor);
        self.events.range(start..).map(|(_, _, event)| event.clone()).collect()
    }

    /// The number of live events, read or not.
    pub fn len(&self) -> usize { self.events.len() }

    /// Returns true if there are no live events.
    pub fn is_empty(&self) -> bool { self.events.is_empty() }

    /// Drop every event older than the lifetime.
    fn prune(&mut self, now: Instant) {
        while let Some((_, sent, _)) = self.events.front()
            && now.duration_since(*sent) > self.lifetime
        {
            self.events.pop_front();
        }
    }
}

impl <E: Clone + Send + 'static> Default for EventQueue<E> {
    fn default() -> Self { Self::with_lifetime(EVENT_LIFETIME) }
}

impl <E> Debug for EventQueue<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventQueue")
            .field("event", &std::any::type_name::<E>())
            .field("live", &self.events.len())
            .field("next_seq", &self.next_seq)
            .field("readers", &self.cursors.len())
            .finish()
    }
}

// implemented by hand rather than derived, so `E` only needs `Clone + Send`
impl <E: Clone + Send + 'static> AsAny for EventQueue<E> {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}
impl <E: Clone + Send + 'static> Resource for EventQueue<E> {}
impl <E: Clone + Send + 'static> ResourceMeta for EventQueue<E> {}

/// A system parameter to send and read events of type `E`.
///
/// Each call locks the event queue only for its own length, so systems using
/// `Event<E>` may run at the same time.
pub struct Event<E> {
    world: World,
    system: SystemKey,
    marker: PhantomData<fn() -> E>
}

impl <E: Clone + Send + 'static> Event<E> {
    /// Send an event that every system may read once within [`EVENT_LIFETIME`].
    pub fn send(&self, event: E) {
        self.queue().send(event);
    }

    /// Every event this system has not read yet that is still live, oldest
    /// first.  Each event is only returned once per system.
    pub fn read(&self) -> std::vec::IntoIter<E> {
        self.queue().read(self.system).into_iter()
    }

    fn queue(&self) -> mutual::MutCastGuard<Box<dyn Resource>, EventQueue<E>> {
        // the queue is created in `extract` and never removed by this module
        self.world.resource_mut::<EventQueue<E>>()
            .expect("event queue was removed from the world")
    }
}

impl <E: Clone + Send + 'static> SystemParam for Event<E> {
    fn extract(world: &World, _exec_state: &SharedExecutionState, system: SystemKey) -> anyhow::Result<Self> {
        world.init_resource_with(EventQueue::<E>::default);
        Ok(Self { world: world.clone(), system, marker: PhantomData })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, atomic::AtomicBool};

    use crate::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Ping(u32);

    #[test]
    fn reader_sees_event_once() {
        static READ: Mutex<Vec<Vec<Ping>>> = Mutex::new(Vec::new());
        static SENT: AtomicBool = AtomicBool::new(false);
        fn writer(events: Event<Ping>) {
            // only send on the first pass
            if !SENT.swap(true, std::sync::atomic::Ordering::AcqRel) { events.send(Ping(1)); }
        }
        fn reader(events: Event<Ping>) { READ.lock().unwrap().push(events.read().collect()); }

        let mut graph = SystemGraph::new();
        graph.append_system(writer, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(reader, after(writer), std::iter::empty()).unwrap();

        let world = World::default();
        let executor = SingleThreadedExecutor::new();
        executor.run(&world, &graph).unwrap();
        executor.run(&world, &graph).unwrap();
        assert_eq!(*READ.lock().unwrap(), vec![vec![Ping(1)], vec![]]);
    }

    #[test]
    fn every_reader_gets_its_own_copy() {
        static A: Mutex<Vec<Ping>> = Mutex::new(Vec::new());
        static B: Mutex<Vec<Ping>> = Mutex::new(Vec::new());
        fn read_a(events: Event<Ping>) { A.lock().unwrap().extend(events.read()); }
        fn read_b(events: Event<Ping>) { B.lock().unwrap().extend(events.read()); }

        let mut graph = SystemGraph::new();
        graph.append_system(read_a, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(read_b, SystemInstruction::default(), std::iter::empty()).unwrap();

        let world = World::default();
        world.send_event(Ping(1));
        world.send_event(Ping(2));
        let executor = SingleThreadedExecutor::new();
        executor.run(&world, &graph).unwrap();
        executor.run(&world, &graph).unwrap();
        assert_eq!(*A.lock().unwrap(), vec![Ping(1), Ping(2)]);
        assert_eq!(*B.lock().unwrap(), vec![Ping(1), Ping(2)]);
    }

    #[test]
    fn late_reader_sees_recent_events() {
        static READ: Mutex<Vec<Ping>> = Mutex::new(Vec::new());
        fn writer(events: Event<Ping>) { events.send(Ping(7)); }
        fn reader(events: Event<Ping>) { READ.lock().unwrap().extend(events.read()); }

        let world = World::default();
        let mut first = SystemGraph::new();
        first.append_system(writer, SystemInstruction::default(), std::iter::empty()).unwrap();
        SingleThreadedExecutor::new().run(&world, &first).unwrap();

        // the reader did not exist when the event was sent
        let mut second = SystemGraph::new();
        second.append_system(reader, SystemInstruction::default(), std::iter::empty()).unwrap();
        SingleThreadedExecutor::new().run(&world, &second).unwrap();
        assert_eq!(*READ.lock().unwrap(), vec![Ping(7)]);
    }

    #[test]
    fn events_expire() {
        static READ: Mutex<Vec<Ping>> = Mutex::new(Vec::new());
        fn reader(events: Event<Ping>) { READ.lock().unwrap().extend(events.read()); }

        let world = World::default();
        world.insert_resource(EventQueue::<Ping>::with_lifetime(std::time::Duration::from_millis(50)));
        world.send_event(Ping(1));
        std::thread::sleep(std::time::Duration::from_millis(60));
        world.send_event(Ping(2));

        let mut graph = SystemGraph::new();
        graph.append_system(reader, SystemInstruction::default(), std::iter::empty()).unwrap();
        SingleThreadedExecutor::new().run(&world, &graph).unwrap();
        assert_eq!(*READ.lock().unwrap(), vec![Ping(2)]);
        assert_eq!(world.resource::<EventQueue<Ping>>().unwrap().len(), 1);
    }

    #[test]
    fn system_reads_its_own_events() {
        static READ: Mutex<Vec<Ping>> = Mutex::new(Vec::new());
        fn echo(events: Event<Ping>) {
            events.send(Ping(3));
            READ.lock().unwrap().extend(events.read());
        }

        let mut graph = SystemGraph::new();
        graph.append_system(echo, SystemInstruction::default(), std::iter::empty()).unwrap();
        SingleThreadedExecutor::new().run(&World::default(), &graph).unwrap();
        assert_eq!(*READ.lock().unwrap(), vec![Ping(3)]);
    }

    #[test]
    fn concurrent_senders_lose_nothing() {
        static READ: Mutex<Vec<Ping>> = Mutex::new(Vec::new());
        macro_rules! senders {
            ($($name:ident = $value:expr),*) => {
                $(fn $name(events: Event<Ping>) { for _ in 0..100 { events.send(Ping($value)); } })*
            };
        }
        senders!(s0 = 0, s1 = 1, s2 = 2, s3 = 3, s4 = 4, s5 = 5, s6 = 6, s7 = 7);
        fn reader(events: Event<Ping>) { READ.lock().unwrap().extend(events.read()); }

        let mut graph = SystemGraph::new();
        graph.append_system(s0, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(s1, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(s2, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(s3, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(s4, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(s5, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(s6, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(s7, SystemInstruction::default(), std::iter::empty()).unwrap();
        graph.append_system(reader, pin(SystemPin::End), std::iter::empty()).unwrap();

        MultiThreadedExecutor::new().run(&World::default(), &graph, 4, &AtomicBool::new(false)).unwrap();
        let read = READ.lock().unwrap();
        assert_eq!(read.len(), 800);
        for value in 0..8 {
            assert_eq!(read.iter().filter(|ping| ping.0 == value).count(), 100);
        }
    }
}
