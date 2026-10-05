use std::any::Any;

use ahash::{AHashMap, AHashSet};
use getset::{CopyGetters, Getters};

use crate::*;

pub type SystemKey = std::any::TypeId;
pub type SystemMeta = std::any::TypeId;
pub type ErasedSystem = Box<dyn System<(), ()>>;

/// Force a system to run at the beginning middle or end of
/// a system graphs execution.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum SystemPin {
    Start,
    #[default]
    Normal,
    End
}

#[derive(Default)]
pub struct SystemGraph {
    /// Every system in the graph wrapped in a node with extra metadata.
    nodes: AHashMap<SystemKey, SystemNode>,
    /// Everything system that is pinned and has no dependencies.
    pin_roots: AHashMap<SystemPin, AHashSet<SystemKey>>,
    /// Lookup table for all systems with certain metadata.
    metadata: AHashMap<SystemMeta, AHashSet<SystemKey>>,
    /// Every system whose instruction names a key, whether or not that key
    /// is in the graph yet.  Lets instructions resolve in any append order.
    references: AHashMap<SystemKey, AHashSet<SystemKey>>,
    /// Every system whose instruction names a meta, whether or not any
    /// system has that meta yet.
    meta_references: AHashMap<SystemMeta, AHashSet<SystemKey>>,
}

impl SystemGraph {
    /// Create a new `SystemGraph`, same as `default`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a node in the graph.
    pub fn node(&self, key: SystemKey) -> Option<&SystemNode> {
        self.nodes.get(&key)
    }

    /// Every system of a pin that has no dependencies.
    pub fn roots(&self, pin: SystemPin) -> impl Iterator<Item = SystemKey> + '_ {
        self.pin_roots.get(&pin).into_iter().flatten().copied()
    }

    /// Add a system to the graph, replacing any system with the same key.
    /// Fails, leaving the system out of the graph, if its instruction
    /// contradicts a pin or creates a cycle.
    pub fn append_system<I, Marker>(
        &mut self,
        system: I,
        instruction: SystemInstruction,
        metadata: impl Iterator<Item = SystemMeta>
    ) -> anyhow::Result<()> where I: IntoSystem<(), (), Marker> + 'static, I::System: 'static {
        self.append_raw(
            system.type_id(),
            Box::new(system.into_system()),
            instruction, metadata
        )
    }

    /// Add a system to the graph, replacing any system with the same key.
    /// Fails, leaving the system out of the graph, if its instruction
    /// contradicts a pin or creates a cycle.
    pub fn append_raw(
        &mut self,
        key: SystemKey,
        system: ErasedSystem,
        instruction: SystemInstruction,
        metadata: impl Iterator<Item = SystemMeta>
    ) -> anyhow::Result<()> {
        self.remove_system(key);

        let node = SystemNode::from_raw(key, system, instruction, metadata);
        self.pin_roots.entry(node.pin).or_default().insert(key);
        for target in node.instruction.before().iter().chain(node.instruction.after()) {
            self.references.entry(*target).or_default().insert(key);
        }
        for meta in node.instruction.before_meta().iter().chain(node.instruction.after_meta()) {
            self.meta_references.entry(*meta).or_default().insert(key);
        }
        for meta in &node.metadata {
            self.metadata.entry(*meta).or_default().insert(key);
        }
        self.nodes.insert(key, node);

        if let Err(err) = self.resolve(key) {
            self.remove_system(key);
            return Err(err);
        }
        Ok(())
    }

    /// Remove a single system from a `SystemGraph`.
    pub fn remove_system(&mut self, key: SystemKey) {
        let Some(node) = self.nodes.remove(&key) else { return };

        if let Some(roots) = self.pin_roots.get_mut(&node.pin) {
            roots.remove(&key);
        }
        for meta in &node.metadata {
            if let Some(keys) = self.metadata.get_mut(meta) {
                keys.remove(&key);
                if keys.is_empty() { self.metadata.remove(meta); }
            }
        }
        // other systems' references to this key stay, so they resolve if it is appended again
        for target in node.instruction.before().iter().chain(node.instruction.after()) {
            if let Some(declarers) = self.references.get_mut(target) {
                declarers.remove(&key);
                if declarers.is_empty() { self.references.remove(target); }
            }
        }
        for meta in node.instruction.before_meta().iter().chain(node.instruction.after_meta()) {
            if let Some(declarers) = self.meta_references.get_mut(meta) {
                declarers.remove(&key);
                if declarers.is_empty() { self.meta_references.remove(meta); }
            }
        }

        for dependency in &node.dependencies {
            if let Some(dependency) = self.nodes.get_mut(dependency) {
                dependency.dependents.remove(&key);
            }
        }
        for dependent in &node.dependents {
            let Some(dependent) = self.nodes.get_mut(dependent) else { continue };
            dependent.dependencies.remove(&key);
            if dependent.dependencies.is_empty() {
                self.pin_roots.entry(dependent.pin).or_default().insert(dependent.key);
            }
        }
    }

    /// Remove all systems with some metadata from a `SystemGraph`.
    pub fn remove_by_meta(&mut self, meta: SystemMeta) {
        let Some(keys) = self.metadata.remove(&meta) else { return };
        keys.into_iter().for_each(|key| self.remove_system(key));
    }

    /// Link a newly added system to everything its instruction names, and to
    /// everything whose instruction names it.
    fn resolve(&mut self, key: SystemKey) -> anyhow::Result<()> {
        let node = &self.nodes[&key];
        let instruction = &node.instruction;
        let targets = instruction.before_meta().iter().chain(instruction.after_meta())
            .flat_map(|meta| self.metadata.get(meta).into_iter().flatten())
            .chain(instruction.before().iter().chain(instruction.after()))
            .copied().collect::<AHashSet<_>>();
        let declarers = node.metadata.iter()
            .flat_map(|meta| self.meta_references.get(meta).into_iter().flatten())
            .chain(self.references.get(&key).into_iter().flatten())
            .copied().collect::<AHashSet<_>>();

        for target in targets {
            self.resolve_edge(key, target)?;
        }
        for declarer in declarers {
            self.resolve_edge(declarer, key)?;
        }
        Ok(())
    }

    /// Link `declarer` to `target` as `declarer`'s instruction says, if both
    /// are in the graph.
    fn resolve_edge(&mut self, declarer: SystemKey, target: SystemKey) -> anyhow::Result<()> {
        if declarer == target { return Ok(()) }
        let (Some(node), Some(target_node)) = (self.nodes.get(&declarer), self.nodes.get(&target)) else { return Ok(()) };
        let has_meta = |metas: &Vec<SystemMeta>| metas.iter().any(|meta| target_node.metadata.contains(meta));
        let before = node.instruction.before().contains(&target) || has_meta(node.instruction.before_meta());
        let after = node.instruction.after().contains(&target) || has_meta(node.instruction.after_meta());

        if before { self.link(declarer, target)?; }
        if after { self.link(target, declarer)?; }
        Ok(())
    }

    /// Make `to` run after `from`.  Both must be in the graph.
    fn link(&mut self, from: SystemKey, to: SystemKey) -> anyhow::Result<()> {
        let (from_pin, to_pin) = (self.nodes[&from].pin, self.nodes[&to].pin);
        if from_pin > to_pin {
            anyhow::bail!("system {from:?} pinned to {from_pin:?} cannot run before system {to:?} pinned to {to_pin:?}");
        }
        // pins already run in order, so only systems sharing a pin are linked
        if from_pin < to_pin || self.nodes[&from].dependents.contains(&to) { return Ok(()) }
        if self.reaches(to, from) {
            anyhow::bail!("running system {to:?} after system {from:?} creates a cycle");
        }

        self.nodes.get_mut(&from).unwrap().dependents.insert(to);
        self.nodes.get_mut(&to).unwrap().dependencies.insert(from);
        if let Some(roots) = self.pin_roots.get_mut(&to_pin) {
            roots.remove(&to);
        }
        Ok(())
    }

    /// If `to` runs after `from`, directly or through other systems.
    fn reaches(&self, from: SystemKey, to: SystemKey) -> bool {
        let mut visited = AHashSet::new();
        let mut stack = vec![from];
        while let Some(key) = stack.pop() {
            if key == to { return true }
            if visited.insert(key) {
                stack.extend(self.nodes[&key].dependents.iter().copied());
            }
        }
        false
    }
}

#[derive(Getters, CopyGetters)]
pub struct SystemNode {
    #[getset(get_copy = "pub")]
    key: SystemKey,
    #[getset(get = "pub")]
    system: ErasedSystem,
    /// Systems in the graph that run after this one.
    #[getset(get = "pub")]
    dependents: AHashSet<SystemKey>,
    /// Systems in the graph that run before this one.
    #[getset(get = "pub")]
    dependencies: AHashSet<SystemKey>,
    #[getset(get = "pub")]
    instruction: SystemInstruction,
    /// Every meta this system was appended with.
    #[getset(get = "pub")]
    metadata: AHashSet<SystemMeta>,
    #[getset(get_copy = "pub")]
    pin: SystemPin
}

impl SystemNode {
    pub fn from_into_system<I, Marker>(
        system: I,
        instruction: SystemInstruction,
        metadata: impl Iterator<Item = SystemMeta>
    ) -> Self where I: IntoSystem<(), (), Marker> + 'static, I::System: 'static {
        Self::from_raw(system.type_id(), Box::new(system.into_system()), instruction, metadata)
    }

    pub fn from_raw(
        key: SystemKey,
        system: ErasedSystem,
        instruction: SystemInstruction,
        metadata: impl Iterator<Item = SystemMeta>
    ) -> Self {
        Self {
            key, system,
            dependents: AHashSet::new(),
            dependencies: AHashSet::new(),
            pin: instruction.pin().unwrap_or_default(),
            metadata: metadata.collect(),
            instruction
        }
    }

    pub fn run(
        &self,
        world: &World,
        exec_state: &SharedExecutionState
    ) -> anyhow::Result<()> {
        self.system().run(world, exec_state)
    }
}


#[cfg(test)]
mod tests {
    use std::any::Any;

    use ahash::AHashSet;

    use crate::*;

    pub struct TestMetaA;
    pub struct TestMetaB;

    fn key<I: Any>(system: I) -> SystemKey { system.type_id() }

    fn roots(graph: &SystemGraph, pin: SystemPin) -> AHashSet<SystemKey> {
        graph.roots(pin).collect()
    }

    fn set<const N: usize>(keys: [SystemKey; N]) -> AHashSet<SystemKey> {
        keys.into_iter().collect()
    }

    fn add<I, Marker>(graph: &mut SystemGraph, system: I, instruction: SystemInstruction) -> anyhow::Result<()>
        where I: IntoSystem<(), (), Marker> + 'static, I::System: 'static
    {
        graph.append_system(system, instruction, std::iter::empty())
    }

    #[test]
    fn test_single_add() {
        fn test_system(_: ()) {}
        fn test_system_two(_: (), _: World) {}

        let mut graph = SystemGraph::default();
        let meta = vec![TestMetaA.type_id()];
        graph.append_system(test_system, pin(SystemPin::Start), meta.clone().into_iter()).unwrap();
        graph.append_system(test_system_two, pin(SystemPin::Normal), meta.into_iter()).unwrap();

        assert_eq!(roots(&graph, SystemPin::Start), set([key(test_system)]));
        assert_eq!(roots(&graph, SystemPin::Normal), set([key(test_system_two)]));
    }

    #[test]
    fn after_makes_dependent() {
        fn a() {}
        fn b() {}

        let mut graph = SystemGraph::new();
        add(&mut graph, a, SystemInstruction::default()).unwrap();
        add(&mut graph, b, after(a)).unwrap();

        assert_eq!(roots(&graph, SystemPin::Normal), set([key(a)]));
        assert_eq!(*graph.node(key(a)).unwrap().dependents(), set([key(b)]));
        assert_eq!(*graph.node(key(b)).unwrap().dependencies(), set([key(a)]));
    }

    #[test]
    fn before_takes_root_from_existing_system() {
        fn a() {}
        fn b() {}

        let mut graph = SystemGraph::new();
        add(&mut graph, b, SystemInstruction::default()).unwrap();
        add(&mut graph, a, before(b)).unwrap();

        assert_eq!(roots(&graph, SystemPin::Normal), set([key(a)]));
        assert_eq!(*graph.node(key(a)).unwrap().dependents(), set([key(b)]));
    }

    #[test]
    fn instructions_resolve_when_target_is_added_later() {
        fn a() {}
        fn b() {}
        fn c() {}

        let mut graph = SystemGraph::new();
        add(&mut graph, b, and(after(a), before(c))).unwrap();
        assert_eq!(roots(&graph, SystemPin::Normal), set([key(b)]));

        add(&mut graph, c, SystemInstruction::default()).unwrap();
        add(&mut graph, a, SystemInstruction::default()).unwrap();
        assert_eq!(roots(&graph, SystemPin::Normal), set([key(a)]));
        assert_eq!(*graph.node(key(a)).unwrap().dependents(), set([key(b)]));
        assert_eq!(*graph.node(key(b)).unwrap().dependents(), set([key(c)]));
    }

    #[test]
    fn systems_in_different_pins_are_not_linked() {
        fn a() {}
        fn b() {}

        let mut graph = SystemGraph::new();
        add(&mut graph, a, pin(SystemPin::Start)).unwrap();
        add(&mut graph, b, after(a)).unwrap();

        assert_eq!(roots(&graph, SystemPin::Start), set([key(a)]));
        assert_eq!(roots(&graph, SystemPin::Normal), set([key(b)]));
        assert!(graph.node(key(a)).unwrap().dependents().is_empty());
    }

    #[test]
    fn instruction_against_pin_order_fails() {
        fn a() {}
        fn b() {}

        let mut graph = SystemGraph::new();
        add(&mut graph, a, pin(SystemPin::End)).unwrap();
        assert!(add(&mut graph, b, and(pin(SystemPin::Start), after(a))).is_err());

        assert!(graph.node(key(b)).is_none());
        assert!(roots(&graph, SystemPin::Start).is_empty());
    }

    #[test]
    fn cycle_fails_and_rolls_back() {
        fn a() {}
        fn b() {}
        fn c() {}

        let mut graph = SystemGraph::new();
        add(&mut graph, a, before(b)).unwrap();
        add(&mut graph, b, before(c)).unwrap();
        assert!(add(&mut graph, c, before(a)).is_err());

        assert!(graph.node(key(c)).is_none());
        assert!(graph.node(key(b)).unwrap().dependents().is_empty());
        assert_eq!(roots(&graph, SystemPin::Normal), set([key(a)]));

        // a cycle completed by a late arrival also fails
        fn d() {}
        fn e() {}
        add(&mut graph, d, after(e)).unwrap();
        assert!(add(&mut graph, e, after(d)).is_err());
        assert!(graph.node(key(e)).is_none());
        assert_eq!(roots(&graph, SystemPin::Normal), set([key(a), key(d)]));
    }

    #[test]
    fn removing_dependency_makes_dependents_roots() {
        fn a() {}
        fn b() {}
        fn c() {}

        let mut graph = SystemGraph::new();
        add(&mut graph, a, SystemInstruction::default()).unwrap();
        add(&mut graph, b, after(a)).unwrap();
        add(&mut graph, c, after(a)).unwrap();

        graph.remove_system(key(a));
        assert!(graph.node(key(a)).is_none());
        assert_eq!(roots(&graph, SystemPin::Normal), set([key(b), key(c)]));
        assert!(graph.node(key(b)).unwrap().dependencies().is_empty());

        // re-adding resolves the instructions that named it
        add(&mut graph, a, SystemInstruction::default()).unwrap();
        assert_eq!(roots(&graph, SystemPin::Normal), set([key(a)]));
        assert_eq!(*graph.node(key(a)).unwrap().dependents(), set([key(b), key(c)]));
    }

    #[test]
    fn removing_dependent_forgets_its_instruction() {
        fn a() {}
        fn b() {}

        let mut graph = SystemGraph::new();
        add(&mut graph, a, before(b)).unwrap();
        add(&mut graph, b, SystemInstruction::default()).unwrap();
        graph.remove_system(key(a));

        assert_eq!(roots(&graph, SystemPin::Normal), set([key(b)]));
        add(&mut graph, a, SystemInstruction::default()).unwrap();
        assert_eq!(roots(&graph, SystemPin::Normal), set([key(a), key(b)]));
    }

    #[test]
    fn appending_again_replaces_instruction() {
        fn a() {}
        fn b() {}

        let mut graph = SystemGraph::new();
        add(&mut graph, a, SystemInstruction::default()).unwrap();
        add(&mut graph, b, after(a)).unwrap();
        add(&mut graph, b, pin(SystemPin::End)).unwrap();

        assert!(graph.node(key(a)).unwrap().dependents().is_empty());
        assert_eq!(roots(&graph, SystemPin::Normal), set([key(a)]));
        assert_eq!(roots(&graph, SystemPin::End), set([key(b)]));
    }

    #[test]
    fn remove_by_meta_removes_only_tagged_systems() {
        fn a() {}
        fn b() {}
        fn c() {}

        let mut graph = SystemGraph::new();
        let (meta_a, meta_b) = (TestMetaA.type_id(), TestMetaB.type_id());
        graph.append_system(a, SystemInstruction::default(), [meta_a].into_iter()).unwrap();
        graph.append_system(b, after(a), [meta_a, meta_b].into_iter()).unwrap();
        graph.append_system(c, after(b), [meta_b].into_iter()).unwrap();

        graph.remove_by_meta(meta_a);
        assert!(graph.node(key(a)).is_none());
        assert!(graph.node(key(b)).is_none());
        assert_eq!(roots(&graph, SystemPin::Normal), set([key(c)]));

        graph.remove_by_meta(meta_b);
        assert!(graph.node(key(c)).is_none());
    }

    #[test]
    fn meta_instructions_link_every_tagged_system() {
        fn a() {}
        fn b() {}
        fn first() {}
        fn last() {}

        let mut graph = SystemGraph::new();
        let meta = TestMetaA.type_id();
        graph.append_system(a, SystemInstruction::default(), [meta].into_iter()).unwrap();
        add(&mut graph, first, before_meta(vec![meta])).unwrap();
        add(&mut graph, last, after_meta(vec![meta])).unwrap();
        // tagged after the instructions were given
        graph.append_system(b, SystemInstruction::default(), [meta].into_iter()).unwrap();

        assert_eq!(roots(&graph, SystemPin::Normal), set([key(first)]));
        assert_eq!(*graph.node(key(first)).unwrap().dependents(), set([key(a), key(b)]));
        assert_eq!(*graph.node(key(last)).unwrap().dependencies(), set([key(a), key(b)]));
    }

    #[test]
    fn meta_instructions_accept_many_metas() {
        fn a() {}
        fn b() {}
        fn c() {}
        fn last() {}

        let mut graph = SystemGraph::new();
        let (meta_a, meta_b) = (TestMetaA.type_id(), TestMetaB.type_id());
        graph.append_system(a, SystemInstruction::default(), [meta_a].into_iter()).unwrap();
        graph.append_system(b, SystemInstruction::default(), [meta_b].into_iter()).unwrap();
        add(&mut graph, c, SystemInstruction::default()).unwrap();
        add(&mut graph, last, after_meta(vec![meta_a, meta_b])).unwrap();

        assert_eq!(*graph.node(key(last)).unwrap().dependencies(), set([key(a), key(b)]));
        assert_eq!(roots(&graph, SystemPin::Normal), set([key(a), key(b), key(c)]));
    }

    #[test]
    fn meta_instruction_skips_own_meta() {
        fn a() {}
        fn b() {}

        let mut graph = SystemGraph::new();
        let meta = TestMetaA.type_id();
        graph.append_system(a, SystemInstruction::default(), [meta].into_iter()).unwrap();
        graph.append_system(b, after_meta(vec![meta]), [meta].into_iter()).unwrap();

        assert_eq!(*graph.node(key(b)).unwrap().dependencies(), set([key(a)]));
    }

    #[test]
    fn meta_instruction_cycle_fails() {
        fn a() {}
        fn b() {}

        let mut graph = SystemGraph::new();
        let meta = TestMetaA.type_id();
        graph.append_system(a, before_meta(vec![meta]), [meta].into_iter()).unwrap();
        assert!(graph.append_system(b, before_meta(vec![meta]), [meta].into_iter()).is_err());
        assert!(graph.node(key(b)).is_none());
        assert!(graph.node(key(a)).unwrap().dependents().is_empty());
    }

    #[test]
    fn removing_meta_instruction_unlinks() {
        fn a() {}
        fn last() {}

        let mut graph = SystemGraph::new();
        let meta = TestMetaA.type_id();
        add(&mut graph, last, after_meta(vec![meta])).unwrap();
        graph.append_system(a, SystemInstruction::default(), [meta].into_iter()).unwrap();
        graph.remove_system(key(last));

        assert!(graph.node(key(a)).unwrap().dependents().is_empty());
        graph.remove_system(key(a));
        graph.append_system(a, SystemInstruction::default(), [meta].into_iter()).unwrap();
        assert_eq!(roots(&graph, SystemPin::Normal), set([key(a)]));
    }

    #[test]
    fn node_stores_its_metadata() {
        fn a() {}
        fn b() {}

        let mut graph = SystemGraph::new();
        let (meta_a, meta_b) = (TestMetaA.type_id(), TestMetaB.type_id());
        graph.append_system(a, SystemInstruction::default(), [meta_a, meta_b].into_iter()).unwrap();
        graph.append_system(b, SystemInstruction::default(), [meta_b].into_iter()).unwrap();
        assert_eq!(*graph.node(key(a)).unwrap().metadata(), set([meta_a, meta_b]));

        // removing by one meta leaves the system out of its other metas too
        graph.remove_by_meta(meta_a);
        graph.remove_by_meta(meta_b);
        assert!(graph.node(key(a)).is_none());
        assert!(graph.node(key(b)).is_none());
    }
}
