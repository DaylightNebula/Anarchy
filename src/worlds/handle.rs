//! [`EntityHandle`], a borrowed view of one entity in a world.

use std::fmt::Debug;

use crate::*;

/// An entity id paired with the [`World`] it lives in, see [`World::handle`].
/// Just an id and a reference, so making one is free and nothing is looked up
/// until a method is called.  The entity is not checked to exist.
#[derive(Clone, Copy)]
pub struct EntityHandle<'w> {
    id: EntityID,
    world: &'w World
}

impl <'w> EntityHandle<'w> {
    /// A handle to `id` in `world`.
    pub fn new(id: EntityID, world: &'w World) -> Self {
        Self { id, world }
    }

    /// The id of this entity.
    pub fn id(&self) -> EntityID {
        self.id
    }

    /// The world this entity lives in.
    pub fn world(&self) -> &'w World {
        self.world
    }

    /// This entity's components, or `None` if it isn't in the world, see [`WorldImpl::entity`].
    pub fn entity(&self) -> Option<Entity> {
        self.world.entity(self.id)
    }

    /// Locked access to the components in `QG`, or `None` if the entity isn't in the world,
    /// see [`WorldImpl::get`].
    ///
    /// # Errors
    ///
    /// Errors if the entity is missing a required component.
    pub fn get<QG: QueryGroup>(&self) -> anyhow::Result<Option<QG::Output>> {
        self.world.get::<QG>(self.id)
    }

    /// This entity's parent, or `None` if it has none.
    pub fn parent(&self) -> Option<EntityHandle<'w>> {
        self.world.hierarchy().parent(self.id).map(|id| self.world.handle(id))
    }

    /// This entity's children, or `None` if it has none, see [`Hierarchy::children`].
    pub fn children(&self) -> Option<Children<'w>> {
        self.world.hierarchy().children(self.id)
    }

    /// Walks up from this entity's parent to the root of its tree.
    pub fn ancestors(&self) -> impl Iterator<Item = EntityHandle<'w>> + 'w {
        let world = self.world;
        world.hierarchy().ancestors(self.id).map(move |id| world.handle(id))
    }

    /// Walks every entity below this one depth first, each parent before its children.
    pub fn descendants(&self) -> impl Iterator<Item = EntityHandle<'w>> + 'w {
        let world = self.world;
        world.hierarchy().descendants(self.id).map(move |id| world.handle(id))
    }

    /// Make `parent` this entity's parent, see [`Hierarchy::set_parent`].
    ///
    /// # Errors
    ///
    /// Errors if this entity is `parent` or one of its ancestors.
    pub fn set_parent(&self, parent: EntityID) -> anyhow::Result<()> {
        self.world.hierarchy().set_parent(self.id, parent)
    }

    /// Make this entity the parent of `child`, see [`Hierarchy::add_child`].
    ///
    /// # Errors
    ///
    /// Errors if `child` is this entity or one of its ancestors.
    pub fn add_child(&self, child: EntityID) -> anyhow::Result<()> {
        self.world.hierarchy().add_child(self.id, child)
    }

    /// Unlink this entity from its parent, returns the old parent if it had one.
    pub fn remove_parent(&self) -> Option<EntityHandle<'w>> {
        self.world.hierarchy().remove_parent(self.id).map(|id| self.world.handle(id))
    }

    /// Remove this entity from the world, leaving its children as roots, see [`World::remove`](WorldImpl::remove).
    pub fn despawn(self) -> Option<DynComponents> {
        self.world.remove(self.id)
    }

    /// Remove this entity and every entity below it from the world, returns how many were removed.
    pub fn despawn_recursive(self) -> usize {
        let below = self.descendants().collect::<Vec<_>>();
        below.into_iter().chain(std::iter::once(self))
            .filter_map(|entity| entity.despawn())
            .count()
    }
}

impl From<EntityHandle<'_>> for EntityID {
    fn from(handle: EntityHandle<'_>) -> Self {
        handle.id
    }
}

impl Debug for EntityHandle<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("EntityHandle").field(&self.id).finish()
    }
}


#[cfg(test)]
mod tests {
    use crate::*;
    use crate::worlds::test_support::*;

    /// Entities 1..=6 holding `A(id)`, linked 1 -> [2, 3], 2 -> [4, 5], 3 -> [6].
    fn world() -> World {
        let world = World::default();
        for id in 1..=6 { world.insert(id, comps(vec![Box::new(A(id as u32))])); }
        for (child, parent) in [(2, 1), (3, 1), (4, 2), (5, 2), (6, 3)] {
            world.hierarchy().set_parent(child, parent).unwrap();
        }
        world
    }

    fn ids<'w>(handles: impl Iterator<Item = EntityHandle<'w>>) -> Vec<EntityID> {
        handles.map(|handle| handle.id()).collect()
    }

    #[test]
    fn reads_components() {
        let world = world();
        let handle = world.handle(4);
        assert_eq!(handle.id(), 4);
        assert_eq!(handle.get::<&A>().unwrap().unwrap().0, 4);
        handle.get::<&mut A>().unwrap().unwrap().0 = 40;
        assert_eq!(world.get::<&A>(4).unwrap().unwrap().0, 40);
        assert_eq!(handle.entity().unwrap().group.to_vec(), [A::id()]);
        assert!(handle.get::<&B>().is_err());
    }

    #[test]
    fn missing_entity() {
        let world = world();
        let handle = world.handle(99);
        assert!(handle.entity().is_none());
        assert!(handle.get::<&A>().unwrap().is_none());
        assert!(handle.parent().is_none());
        assert!(handle.children().is_none());
        assert!(handle.despawn().is_none());
    }

    #[test]
    fn walks_hierarchy() {
        let world = world();
        assert_eq!(world.handle(5).parent().unwrap().id(), 2);
        assert!(world.handle(1).parent().is_none());
        assert_eq!(*world.handle(2).children().unwrap(), [4, 5]);
        assert_eq!(ids(world.handle(5).ancestors()), [2, 1]);
        assert_eq!(ids(world.handle(1).descendants()), [2, 4, 5, 3, 6]);

        // handles from a walk read their own components
        let values = world.handle(2).descendants()
            .map(|child| child.get::<&A>().unwrap().unwrap().0)
            .collect::<Vec<_>>();
        assert_eq!(values, [4, 5]);
    }

    #[test]
    fn edits_hierarchy() {
        let world = world();
        world.handle(6).add_child(4).unwrap();
        assert_eq!(world.hierarchy().parent(4), Some(6));

        world.handle(5).set_parent(world.handle(3).id()).unwrap();
        assert_eq!(*world.handle(3).children().unwrap(), [6, 5]);
        assert!(world.handle(2).children().is_none());

        assert!(world.handle(1).set_parent(4).is_err());
        assert!(world.handle(1).add_child(1).is_err());

        assert_eq!(world.handle(5).remove_parent().unwrap().id(), 3);
        assert!(world.handle(5).remove_parent().is_none());
    }

    #[test]
    fn despawn_orphans_children() {
        let world = world();
        assert_eq!(world.handle(2).despawn().map(|c| value_of::<A>(&c, |a| a.0)), Some(2));
        assert!(world.entity(2).is_none());
        assert!(world.handle(4).parent().is_none());
        assert_eq!(*world.handle(1).children().unwrap(), [3]);
    }

    #[test]
    fn despawn_recursive_removes_subtree() {
        let world = world();
        assert_eq!(world.handle(2).despawn_recursive(), 3);
        for id in [2, 4, 5] { assert!(world.entity(id).is_none(), "{id} still spawned"); }
        assert_eq!(entities(&world, &[A::id()]), [1, 3, 6]);
        assert_eq!(ids(world.handle(1).descendants()), [3, 6]);
    }

    #[test]
    fn converts_to_id() {
        let world = world();
        let id: EntityID = world.handle(3).into();
        assert_eq!(id, 3);
        assert_eq!(format!("{:?}", world.handle(3)), "EntityHandle(3)");
    }
}
