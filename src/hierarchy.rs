//! Parent and child links between entities.

use std::{ops::Deref, sync::{Arc, Mutex}};

use anyhow::bail;
use mutual::{DashMap, mapref::one::Ref};

use crate::*;

/// Parent and child links between entities, every [`World`] holds one, see
/// [`World::hierarchy`].  A shared handle, clones are cheap and share the same links.
///
/// Links are kept outside the tables, so parenting never moves an entity between
/// tables, and both directions are a single hash lookup.  Only ids are stored,
/// the entities themselves are not checked to exist.
///
/// Reads never wait on each other.  Edits are serialised so the links can never
/// form a cycle, but a read racing an edit may see it half applied, e.g. a child
/// whose parent is set before it appears in that parent's [`children`](Self::children).
#[derive(Clone, Default)]
pub struct Hierarchy(Arc<HierarchyInner>);

/// The data behind a [`Hierarchy`], shared by its clones.
#[derive(Default)]
struct HierarchyInner {
    /// Child to its parent.
    parent: DashMap<EntityID, EntityID>,
    /// Parent to its children, in the order they were added.  Never holds an empty list.
    children: DashMap<EntityID, Vec<EntityID>>,
    /// Held while editing so the cycle check and the updates to both maps can't interleave.
    write_lock: Mutex<()>
}

/// The children of an entity, keeps part of the hierarchy read locked until dropped,
/// so drop it before editing the hierarchy on the same thread.
pub struct Children<'a>(Ref<'a, EntityID, Vec<EntityID>>);

impl Deref for Children<'_> {
    type Target = [EntityID];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Hierarchy {
    /// Create an empty hierarchy.
    pub fn new() -> Self {
        Self::default()
    }

    /// The parent of `id`, or `None` if it has none.
    pub fn parent(&self, id: EntityID) -> Option<EntityID> {
        self.0.parent.get(&id).map(|parent| *parent)
    }

    /// The children of `id` in the order they were added, or `None` if it has none.
    pub fn children(&self, id: EntityID) -> Option<Children<'_>> {
        self.0.children.get(&id).map(Children)
    }

    /// Walks up from the parent of `id` to the root of its tree.
    pub fn ancestors(&self, id: EntityID) -> impl Iterator<Item = EntityID> + '_ {
        let mut current = id;
        std::iter::from_fn(move || {
            current = self.parent(current)?;
            Some(current)
        })
    }

    /// Walks every entity below `id` depth first, each parent before its children.
    /// Holds no locks between steps.
    pub fn descendants(&self, id: EntityID) -> impl Iterator<Item = EntityID> + '_ {
        let mut stack = vec![];
        self.push_children(id, &mut stack);
        std::iter::from_fn(move || {
            let next = stack.pop()?;
            self.push_children(next, &mut stack);
            Some(next)
        })
    }

    /// Pushes the children of `id` so they pop in order.
    fn push_children(&self, id: EntityID, stack: &mut Vec<EntityID>) {
        if let Some(children) = self.children(id) {
            stack.extend(children.iter().rev());
        }
    }

    /// Make `parent` the parent of `child`, moving it from any previous parent.
    ///
    /// # Errors
    ///
    /// Errors if `child` is `parent` or one of its ancestors, as that would make a cycle.
    pub fn set_parent(&self, child: EntityID, parent: EntityID) -> anyhow::Result<()> {
        let _guard = self.0.write_lock.lock().unwrap();
        if child == parent { bail!("entity {child} can not be its own parent") }
        if self.ancestors(parent).any(|ancestor| ancestor == child) {
            bail!("entity {child} is an ancestor of {parent}, so can not be its child")
        }
        if self.parent(child) == Some(parent) { return Ok(()) }

        self.detach(child);
        self.0.children.entry(parent).or_default().push(child);
        self.0.parent.insert(child, parent);
        Ok(())
    }

    /// Make `parent` the parent of `child`, the same as [`set_parent`](Self::set_parent)
    /// with the arguments the other way round.
    ///
    /// # Errors
    ///
    /// Errors if `child` is `parent` or one of its ancestors, as that would make a cycle.
    pub fn add_child(&self, parent: EntityID, child: EntityID) -> anyhow::Result<()> {
        self.set_parent(child, parent)
    }

    /// Unlink `child` from its parent, returns the old parent if it had one.
    pub fn remove_parent(&self, child: EntityID) -> Option<EntityID> {
        let _guard = self.0.write_lock.lock().unwrap();
        self.detach(child)
    }

    /// Unlink `id` from its parent and children, returns the children it had,
    /// which are now roots.  Called by [`World::remove`](WorldImpl::remove).
    pub fn remove(&self, id: EntityID) -> Vec<EntityID> {
        // most entities have no links, so skip the lock for them
        if !self.0.parent.contains_key(&id) && !self.0.children.contains_key(&id) {
            return vec![];
        }

        let _guard = self.0.write_lock.lock().unwrap();
        self.detach(id);
        let Some((_, children)) = self.0.children.remove(&id) else { return vec![] };
        for child in children.iter() {
            self.0.parent.remove(child);
        }
        children
    }

    /// Unlinks `child` from its parent, the write lock must be held.
    fn detach(&self, child: EntityID) -> Option<EntityID> {
        let (_, parent) = self.0.parent.remove(&child)?;
        if let Some(mut siblings) = self.0.children.get_mut(&parent) {
            siblings.retain(|sibling| *sibling != child);
        }
        // the guard above must be dropped first, as both lock the same shard
        self.0.children.remove_if(&parent, |_, siblings| siblings.is_empty());
        Some(parent)
    }
}

impl SystemParam for Hierarchy {
    fn extract(world: &World, _exec_state: &SharedExecutionState, _system: SystemKey) -> anyhow::Result<Self> {
        Ok(world.hierarchy().clone())
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::worlds::test_support::*;

    fn children(hierarchy: &Hierarchy, id: EntityID) -> Vec<EntityID> {
        hierarchy.children(id).map(|c| c.to_vec()).unwrap_or_default()
    }

    /// Builds 1 -> [2, 3], 2 -> [4, 5], 3 -> [6].
    fn tree() -> Hierarchy {
        let hierarchy = Hierarchy::new();
        for (child, parent) in [(2, 1), (3, 1), (4, 2), (5, 2), (6, 3)] {
            hierarchy.set_parent(child, parent).unwrap();
        }
        hierarchy
    }

    #[test]
    fn empty() {
        let hierarchy = Hierarchy::new();
        assert_eq!(hierarchy.parent(1), None);
        assert!(hierarchy.children(1).is_none());
        assert_eq!(hierarchy.ancestors(1).count(), 0);
        assert_eq!(hierarchy.descendants(1).count(), 0);
        assert_eq!(hierarchy.remove_parent(1), None);
        assert!(hierarchy.remove(1).is_empty());
    }

    #[test]
    fn parent_and_children() {
        let hierarchy = tree();
        assert_eq!(hierarchy.parent(1), None);
        assert_eq!(hierarchy.parent(2), Some(1));
        assert_eq!(hierarchy.parent(6), Some(3));
        assert_eq!(children(&hierarchy, 1), [2, 3]);
        assert_eq!(children(&hierarchy, 2), [4, 5]);
        assert!(hierarchy.children(4).is_none());
    }

    #[test]
    fn set_same_parent_twice_does_not_duplicate() {
        let hierarchy = tree();
        hierarchy.set_parent(3, 1).unwrap();
        assert_eq!(children(&hierarchy, 1), [2, 3]);
    }

    #[test]
    fn reparent_moves_child() {
        let hierarchy = tree();
        hierarchy.set_parent(5, 3).unwrap();
        assert_eq!(hierarchy.parent(5), Some(3));
        assert_eq!(children(&hierarchy, 2), [4]);
        assert_eq!(children(&hierarchy, 3), [6, 5]);

        // moving the last child out drops the parent's list
        hierarchy.set_parent(4, 1).unwrap();
        assert!(hierarchy.children(2).is_none());
        assert_eq!(children(&hierarchy, 1), [2, 3, 4]);
    }

    #[test]
    fn cycles_are_rejected() {
        let hierarchy = tree();
        assert!(hierarchy.set_parent(1, 1).is_err());
        assert!(hierarchy.set_parent(1, 2).is_err());
        assert!(hierarchy.set_parent(1, 6).is_err());
        assert!(hierarchy.set_parent(2, 4).is_err());
        // nothing changed
        assert_eq!(hierarchy.parent(1), None);
        assert_eq!(hierarchy.parent(2), Some(1));
        assert_eq!(children(&hierarchy, 4), [] as [EntityID; 0]);
        // moving into a different branch is fine
        assert!(hierarchy.set_parent(2, 6).is_ok());
    }

    #[test]
    fn ancestors_walk_to_root() {
        let hierarchy = tree();
        assert_eq!(hierarchy.ancestors(5).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(hierarchy.ancestors(6).collect::<Vec<_>>(), [3, 1]);
        assert_eq!(hierarchy.ancestors(1).count(), 0);
    }

    #[test]
    fn descendants_are_depth_first() {
        let hierarchy = tree();
        assert_eq!(hierarchy.descendants(1).collect::<Vec<_>>(), [2, 4, 5, 3, 6]);
        assert_eq!(hierarchy.descendants(2).collect::<Vec<_>>(), [4, 5]);
        assert_eq!(hierarchy.descendants(6).count(), 0);
    }

    #[test]
    fn remove_parent_detaches() {
        let hierarchy = tree();
        assert_eq!(hierarchy.remove_parent(2), Some(1));
        assert_eq!(hierarchy.parent(2), None);
        assert_eq!(children(&hierarchy, 1), [3]);
        // its own subtree is untouched
        assert_eq!(children(&hierarchy, 2), [4, 5]);
        assert_eq!(hierarchy.remove_parent(2), None);
    }

    #[test]
    fn remove_orphans_children() {
        let hierarchy = tree();
        assert_eq!(hierarchy.remove(2), [4, 5]);
        assert_eq!(hierarchy.parent(4), None);
        assert_eq!(hierarchy.parent(5), None);
        assert!(hierarchy.children(2).is_none());
        assert_eq!(children(&hierarchy, 1), [3]);
        assert!(hierarchy.remove(2).is_empty());
    }

    #[test]
    fn clones_share_links() {
        let hierarchy = Hierarchy::new();
        hierarchy.clone().set_parent(2, 1).unwrap();
        assert_eq!(hierarchy.parent(2), Some(1));
    }

    #[test]
    fn world_remove_clears_links() {
        let world = World::default();
        for id in 1..=3 { world.insert(id, comps(vec![Box::new(A(0))])); }
        world.hierarchy().set_parent(2, 1).unwrap();
        world.hierarchy().set_parent(3, 2).unwrap();

        world.remove(2).unwrap();
        assert!(world.hierarchy().children(1).is_none());
        assert_eq!(world.hierarchy().parent(3), None);
    }

    #[test]
    fn concurrent_edits_stay_consistent() {
        const THREADS: u64 = 8;
        const ENTITIES: u64 = 64;

        let hierarchy = Hierarchy::new();
        std::thread::scope(|s| for t in 0..THREADS {
            let hierarchy = &hierarchy;
            s.spawn(move || {
                // a small LCG so each thread makes its own repeatable edits
                let mut seed = t * 7919 + 1;
                let mut rand = move || { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); seed >> 33 };
                for _ in 0..2_000 {
                    let child = rand() % ENTITIES;
                    match rand() % 8 {
                        0 => { hierarchy.remove_parent(child); }
                        1 => { hierarchy.remove(child); }
                        // cycles are expected to be rejected
                        _ => { let _ = hierarchy.set_parent(child, rand() % ENTITIES); }
                    }
                }
            });
        });

        for id in 0..ENTITIES {
            if let Some(parent) = hierarchy.parent(id) {
                assert_eq!(children(&hierarchy, parent).iter().filter(|c| **c == id).count(), 1, "{id} missing from {parent}");
            }
            for child in children(&hierarchy, id) {
                assert_eq!(hierarchy.parent(child), Some(id), "{child} listed under {id}");
            }
            assert!(hierarchy.ancestors(id).take(ENTITIES as usize + 1).count() <= ENTITIES as usize, "cycle through {id}");
        }
    }

    #[test]
    fn propagates_down_a_tree() {
        // A is a local offset, B the resolved global offset, summed down the tree
        let world = World::default();
        for id in 1..=6 { world.insert(id, comps(vec![Box::new(A(id as u32)), Box::new(B(0))])); }
        for (child, parent) in [(2, 1), (3, 1), (4, 2), (5, 2), (6, 3)] {
            world.hierarchy().set_parent(child, parent).unwrap();
        }

        let hierarchy = world.hierarchy();
        let mut roots = Query::<(&A, &mut B)>::new(&world);
        while let Some((id, (local, mut global))) = roots.next().unwrap() {
            if hierarchy.parent(id).is_some() { continue }
            global.0 = local.0;
            let mut stack = vec![(id, global.0)];
            while let Some((parent, parent_global)) = stack.pop() {
                let Some(children) = hierarchy.children(parent) else { continue };
                for &child in children.iter() {
                    let (local, mut global) = world.get::<(&A, &mut B)>(child).unwrap().unwrap();
                    global.0 = parent_global + local.0;
                    stack.push((child, global.0));
                }
            }
        }

        let globals = (1..=6).map(|id| world.get::<&B>(id).unwrap().unwrap().0).collect::<Vec<_>>();
        assert_eq!(globals, [1, 3, 4, 7, 8, 10]);
    }
}
