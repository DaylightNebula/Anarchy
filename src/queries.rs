//! Typed queries over the entities in a world.

use crate::*;

pub mod components;
pub mod groups;
pub mod iter;

pub use components::*;
pub use groups::*;
pub use iter::*;

/// Walks every entity holding the components in `QG`, handing out locked
/// access to them.
///
/// `QG` is a [`QueryComponent`] (`&A`, `&mut A`, `Option<&A>` or `Option<&mut A>`)
/// or a tuple of up to 8 of them.  Each component is locked as it is handed out
/// and stays locked until its guard is dropped.
///
/// [`Query::new`] asks the world for every listed component, `Option` ones
/// included, so an optional component only resolves to `None` when the query is
/// built with [`Query::from_iter`] over tables that lack it.
pub struct Query<QG: QueryGroup> {
    /// Iterator of cursors over tables
    raw_cursors: Box<dyn Iterator<Item = Cursor>>,
    /// The current table's cursor, with where the queried components sit in that table.
    cursor: Option<(Cursor, QG::Indices)>
}

impl <QG: QueryGroup> Query<QG> {
    /// Query every table in `world` that holds all of `QG`'s components.
    pub fn new<W: WorldImpl>(world: &W) -> Self {
        Self::from_iter(world.raw_query(QG::req_comps()))
    }

    /// Query the tables walked by each cursor in `iter`.  Tables missing a
    /// required component make [`next`](Self::next) return an error.
    pub fn from_iter(iter: Box<dyn Iterator<Item = Cursor>>) -> Self {
        Self { raw_cursors: iter, cursor: None }
    }

    /// Returns the next entity and its components, or `None` once every table is done.
    ///
    /// # Errors
    ///
    /// Errors if the entity is missing a required component.
    pub fn next(&mut self) -> anyhow::Result<Option<(EntityID, QG::Output)>> {
        loop {
            if let Some((cursor, indices)) = &self.cursor
                && let Some((entity_id, comps)) = cursor.next() {
                return Ok(Some((entity_id, QG::from_comps(&comps, indices)?)));
            }

            // current table is done (or empty), move on to the next one and resolve its layout,
            // if there are no more tables we are done
            let Some(cursor) = self.raw_cursors.next() else { return Ok(None) };
            let indices = QG::resolve(cursor.group());
            self.cursor = Some((cursor, indices));
        }
    }

    /// Create an iterator over this query.  
    /// This will eat the `anyhow::Result`s from `next`.
    pub fn iter(self) -> QueryIter<QG> {
        QueryIter(self)
    }
}

impl <QG: QueryGroup> SystemParam for Query<QG> {
    fn extract(world: &World, _exec_state: &SharedExecutionState, _system: SystemKey) -> anyhow::Result<Self> {
        Ok(Self::new(world))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worlds::test_support::*;

    /// A tree of worlds, each holding one `A` entity tagged with its own value:
    ///
    /// ```text
    /// root (0)
    /// ├── left (1)
    /// │   ├── left_left (11)
    /// │   │   └── left_left_left (111)
    /// │   └── left_right (12)
    /// └── right (2)
    ///     └── right_left (21)
    /// ```
    struct Tree {
        root: World,
        left: World,
        left_left: World,
        left_left_left: World,
        left_right: World,
        right: World,
        right_left: World
    }

    impl Tree {
        fn new() -> Self {
            let tree = Self {
                root: World::default(),
                left: World::default(),
                left_left: World::default(),
                left_left_left: World::default(),
                left_right: World::default(),
                right: World::default(),
                right_left: World::default()
            };
            adopt(&tree.root, &tree.left);
            adopt(&tree.root, &tree.right);
            adopt(&tree.left, &tree.left_left);
            adopt(&tree.left, &tree.left_right);
            adopt(&tree.left_left, &tree.left_left_left);
            adopt(&tree.right, &tree.right_left);

            for (world, tag) in tree.all() {
                world.insert(100, comps(vec![Box::new(A(tag))]));
            }
            tree
        }

        fn all(&self) -> [(&World, u32); 7] {
            [
                (&self.root, 0),
                (&self.left, 1),
                (&self.left_left, 11),
                (&self.left_left_left, 111),
                (&self.left_right, 12),
                (&self.right, 2),
                (&self.right_left, 21)
            ]
        }
    }

    /// Nest `child` in `parent` as an entity of its own.
    fn adopt(parent: &World, child: &World) {
        parent.insert(1, comps(vec![Box::new(child.clone())]));
    }

    /// The tag of every `A` a query on `world` reaches, sorted.
    fn tags(world: &World) -> Vec<u32> {
        let mut tags = Query::<&A>::new(world).iter().map(|(_, a)| a.0).collect::<Vec<_>>();
        tags.sort();
        tags
    }

    #[test]
    fn query_cascades_into_every_descendant() {
        let tree = Tree::new();
        assert_eq!(tags(&tree.root), [0, 1, 2, 11, 12, 21, 111]);
    }

    #[test]
    fn query_on_a_child_skips_parents_and_siblings() {
        let tree = Tree::new();
        // left reaches its own subtree, never root above it or right beside it
        assert_eq!(tags(&tree.left), [1, 11, 12, 111]);
        assert_eq!(tags(&tree.right), [2, 21]);
    }

    #[test]
    fn query_on_a_grandchild_skips_its_uncles_and_cousins() {
        let tree = Tree::new();
        // left_left must not see its sibling left_right, its parent left, root,
        // or the right branch and its children
        assert_eq!(tags(&tree.left_left), [11, 111]);
        assert_eq!(tags(&tree.left_right), [12]);
        assert_eq!(tags(&tree.right_left), [21]);
    }

    #[test]
    fn query_on_a_leaf_sees_only_itself() {
        let tree = Tree::new();
        assert_eq!(tags(&tree.left_left_left), [111]);
    }

    #[test]
    fn query_filters_components_in_nested_worlds() {
        let tree = Tree::new();
        tree.left_left.insert(200, comps(vec![Box::new(A(1100)), Box::new(B(0))]));
        tree.right.insert(200, comps(vec![Box::new(A(200)), Box::new(B(0))]));
        tree.root.insert(200, comps(vec![Box::new(A(0)), Box::new(B(0))]));

        let with_b = |world: &World| {
            let mut tags = Query::<(&A, &B)>::new(world).iter().map(|(_, (a, _))| a.0).collect::<Vec<_>>();
            tags.sort();
            tags
        };
        assert_eq!(with_b(&tree.root), [0, 200, 1100]);
        assert_eq!(with_b(&tree.left), [1100]);
        assert_eq!(with_b(&tree.right), [200]);
        assert_eq!(with_b(&tree.left_right), [] as [u32; 0]);
    }

    #[test]
    fn query_mutations_on_a_child_stay_in_its_subtree() {
        let tree = Tree::new();
        let mut query = Query::<&mut A>::new(&tree.left);
        while let Some((_, mut a)) = query.next().unwrap() {
            a.0 += 1000;
        }

        // left and its descendants changed, root and the right branch did not
        assert_eq!(tags(&tree.root), [0, 2, 21, 1001, 1011, 1012, 1111]);
        assert_eq!(tags(&tree.right), [2, 21]);
    }

    #[test]
    fn query_sees_worlds_nested_after_the_tree_is_built() {
        let tree = Tree::new();
        let late = World::default();
        late.insert(100, comps(vec![Box::new(A(121))]));
        adopt(&tree.left_right, &late);

        assert_eq!(tags(&tree.left_right), [12, 121]);
        assert_eq!(tags(&tree.left), [1, 11, 12, 111, 121]);
        // still nowhere near its new parent's sibling
        assert_eq!(tags(&tree.left_left), [11, 111]);
        assert_eq!(tags(&tree.right), [2, 21]);
    }
}
