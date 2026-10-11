//! Groups of components a query yields together.

use crate::*;

/// The components a [`Query`] yields for each entity, a [`QueryComponent`] or
/// a tuple of up to 8 of them.
pub trait QueryGroup {
    /// The guards handed out for each entity, in the same shape as the group.
    type Output;
    /// Where each component sits in a table, resolved once per table.
    type Indices;

    /// True if any component in the group is borrowed mutably.
    fn req_mut() -> bool;
    /// The id of every component in the group, in the group's order.
    fn req_comps() -> ComponentIDGroup;
    /// Finds where each component sits in `group`, which must be sorted.
    /// Components not in `group` resolve to `None`.
    fn resolve(group: &ComponentIDGroup) -> Self::Indices;
    /// Extracts the group from an entity's components, using indices from
    /// [`resolve`](Self::resolve) for the entity's table.
    ///
    /// # Errors
    ///
    /// Errors if a required component is missing, or the indices were
    /// resolved for a different table.
    fn from_comps(comps: &DynComponents, indices: &Self::Indices) -> anyhow::Result<Self::Output>;
}

#[inline(always)]
fn index_of(group: &ComponentIDGroup, id: ComponentID) -> Option<usize> {
    group.binary_search(&id).ok()
}

impl <A: QueryComponent> QueryGroup for A {
    type Output = A::Output;
    type Indices = Option<usize>;

    fn req_mut() -> bool { A::req_mut() }
    fn req_comps() -> ComponentIDGroup { Box::new([A::req_comp()]) }
    fn resolve(group: &ComponentIDGroup) -> Self::Indices { index_of(group, A::req_comp()) }
    fn from_comps(comps: &DynComponents, indices: &Self::Indices) -> anyhow::Result<Self::Output> {
        A::extract(indices.and_then(|i| comps.get(i)))
    }
}

macro_rules! tuple_group {
    (@count) => { 0 };
    (@count $head:ident $($tail:ident)*) => { 1 + tuple_group!(@count $($tail)*) };

    ($($name:ident $idx:tt),+) => {
        impl <$($name: QueryComponent),+> QueryGroup for ($($name,)+) {
            type Output = ($($name::Output,)+);
            type Indices = [Option<usize>; tuple_group!(@count $($name)+)];

            fn req_mut() -> bool { false $(|| $name::req_mut())+ }
            fn req_comps() -> ComponentIDGroup { Box::new([$($name::req_comp()),+]) }
            fn resolve(group: &ComponentIDGroup) -> Self::Indices { [$(index_of(group, $name::req_comp())),+] }
            fn from_comps(comps: &DynComponents, indices: &Self::Indices) -> anyhow::Result<Self::Output> {
                Ok(($($name::extract(indices[$idx].and_then(|i| comps.get(i)))?,)+))
            }
        }
    };
}

tuple_group!(A 0, B 1);
tuple_group!(A 0, B 1, C 2);
tuple_group!(A 0, B 1, C 2, D 3);
tuple_group!(A 0, B 1, C 2, D 3, E 4);
tuple_group!(A 0, B 1, C 2, D 3, E 4, F 5);
tuple_group!(A 0, B 1, C 2, D 3, E 4, F 5, G 6);
tuple_group!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7);


#[cfg(test)]
mod tests {
    use mutual::SharedData;

    use super::*;
    use crate::worlds::test_support::*;

    /// The sorted layout of a table holding `ids`.
    fn group(ids: &[ComponentID]) -> ComponentIDGroup {
        sorted(ids.to_vec()).into_boxed_slice()
    }

    /// Components stored the way a table stores them, sorted by id.
    fn stored(components: Vec<DynComponent>) -> (ComponentIDGroup, DynComponents) {
        let mut components = comps(components);
        components.sort_by_key(|c| c.lock_ref().get_id());
        let ids = components.iter().map(|c| c.lock_ref().get_id()).collect();
        (ids, components)
    }

    /// Resolves `QG` against the components' own layout and extracts it.
    fn extract<QG: QueryGroup>(components: Vec<DynComponent>) -> anyhow::Result<QG::Output> {
        let (ids, components) = stored(components);
        QG::from_comps(&components, &QG::resolve(&ids))
    }

    /// Drains a query, panicking on any error.
    fn collect<QG: QueryGroup, T>(mut query: Query<QG>, map: impl Fn(QG::Output) -> T) -> Vec<(EntityID, T)> {
        let mut out = vec![];
        while let Some((id, output)) = query.next().unwrap() {
            out.push((id, map(output)));
        }
        out.sort_by_key(|(id, _)| *id);
        out
    }

    // ---- requirements ----

    #[test]
    fn req_comps_follow_tuple_order() {
        assert_eq!(*<&A>::req_comps(), [A::id()]);
        assert_eq!(*<(&C, &A)>::req_comps(), [C::id(), A::id()]);
        assert_eq!(*<(&B, Option<&D>, &mut A)>::req_comps(), [B::id(), D::id(), A::id()]);
    }

    #[test]
    fn req_mut_if_any_component_is_mut() {
        assert!(!<&A as QueryGroup>::req_mut());
        assert!(<&mut A as QueryGroup>::req_mut());
        assert!(!<(&A, &B, Option<&C>)>::req_mut());
        assert!(<(&A, &mut B)>::req_mut());
        assert!(<(&A, &B, Option<&mut C>)>::req_mut());
    }

    // ---- resolve ----

    #[test]
    fn resolve_finds_positions_in_sorted_group() {
        let layout = group(&[A::id(), B::id(), C::id(), D::id()]);
        let pos = |id: ComponentID| layout.iter().position(|i| *i == id);

        assert_eq!(<&B>::resolve(&layout), pos(B::id()));
        assert_eq!(<(&D, &A)>::resolve(&layout), [pos(D::id()), pos(A::id())]);
        assert_eq!(
            <(&C, &mut B, Option<&D>, &A)>::resolve(&layout),
            [pos(C::id()), pos(B::id()), pos(D::id()), pos(A::id())]
        );
    }

    #[test]
    fn resolve_missing_component_is_none() {
        let layout = group(&[A::id(), C::id()]);
        assert_eq!(<&B>::resolve(&layout), None);
        assert_eq!(<(&A, Option<&B>, &C)>::resolve(&layout)[1], None);
        assert!(<(&A, Option<&B>, &C)>::resolve(&layout)[0].is_some());
        assert!(<(&A, Option<&B>, &C)>::resolve(&layout)[2].is_some());
    }

    // ---- from_comps ----

    #[test]
    fn extracts_single_component() {
        let a = extract::<&A>(vec![Box::new(B(2)), Box::new(A(1))]).unwrap();
        assert_eq!(a.0, 1);
    }

    #[test]
    fn extracts_tuple_regardless_of_storage_order() {
        let (c, a, b) = extract::<(&C, &A, &B)>(vec![
            Box::new(B(2)), Box::new(D(4)), Box::new(A(1)), Box::new(C(3))
        ]).unwrap();
        assert_eq!((a.0, b.0, c.0), (1, 2, 3));
    }

    #[test]
    fn extracts_every_tuple_size() {
        let all = || -> Vec<DynComponent> { vec![Box::new(A(1)), Box::new(B(2)), Box::new(C(3)), Box::new(D(4))] };
        let (a, b) = extract::<(&A, &B)>(all()).unwrap();
        assert_eq!((a.0, b.0), (1, 2));
        let (a, b, c) = extract::<(&A, &B, &C)>(all()).unwrap();
        assert_eq!((a.0, b.0, c.0), (1, 2, 3));
        let (a, b, c, d) = extract::<(&A, &B, &C, &D)>(all()).unwrap();
        assert_eq!((a.0, b.0, c.0, d.0), (1, 2, 3, 4));
        // a duplicate shared borrow resolves to the same component
        let (a, b, c, d, a2) = extract::<(&A, &B, &C, &D, &A)>(all()).unwrap();
        assert_eq!((a.0, b.0, c.0, d.0, a2.0), (1, 2, 3, 4, 1));
    }

    #[test]
    fn optional_components() {
        let (a, b) = extract::<(&A, Option<&B>)>(vec![Box::new(A(1))]).unwrap();
        assert_eq!((a.0, b.map(|b| b.0)), (1, None));

        let (a, b) = extract::<(&A, Option<&B>)>(vec![Box::new(A(1)), Box::new(B(2))]).unwrap();
        assert_eq!((a.0, b.map(|b| b.0)), (1, Some(2)));

        let b = extract::<Option<&mut B>>(vec![Box::new(A(1))]).unwrap();
        assert!(b.is_none());
    }

    #[test]
    fn missing_required_component_errors() {
        assert!(extract::<&B>(vec![Box::new(A(1))]).is_err());
        assert!(extract::<(&A, &B)>(vec![Box::new(A(1))]).is_err());
        assert!(extract::<(&A, &mut B)>(vec![Box::new(A(1)), Box::new(C(3))]).is_err());
    }

    #[test]
    fn indices_for_a_different_layout_error() {
        // indices resolved for one table must not silently read another table's components
        let indices = <&B>::resolve(&group(&[A::id(), B::id()]));
        let (_, components) = stored(vec![Box::new(A(1)), Box::new(C(3))]);
        assert!(<&B>::from_comps(&components, &indices).is_err());

        // and an index past the end is treated as missing
        let indices = <&D>::resolve(&group(&[A::id(), B::id(), C::id(), D::id()]));
        let (_, components) = stored(vec![Box::new(A(1))]);
        assert!(<&D>::from_comps(&components, &indices).is_err());
    }

    #[test]
    fn mut_writes_through_to_storage() {
        let (ids, components) = stored(vec![Box::new(A(1)), Box::new(B(2))]);
        {
            let (mut a, b) = <(&mut A, &B)>::from_comps(&components, &<(&mut A, &B)>::resolve(&ids)).unwrap();
            a.0 += b.0;
        }
        let a = <&A>::from_comps(&components, &<&A>::resolve(&ids)).unwrap();
        assert_eq!(a.0, 3);
    }

    // ---- through Query ----

    #[test]
    fn query_resolves_each_tables_layout() {
        // every table has a different layout, so A and C sit at different positions in each
        let world = IndexedWorld::new();
        world.insert(1, comps(vec![Box::new(A(1)), Box::new(C(10))]));
        world.insert(2, comps(vec![Box::new(A(2)), Box::new(B(0)), Box::new(C(20))]));
        world.insert(3, comps(vec![Box::new(D(0)), Box::new(C(30)), Box::new(A(3))]));
        world.insert(4, comps(vec![Box::new(A(4)), Box::new(B(0)), Box::new(C(40)), Box::new(D(0))]));
        world.insert(5, comps(vec![Box::new(A(5)), Box::new(B(0))]));
        world.insert(6, comps(vec![Box::new(A(6)), Box::new(C(60))]));

        let found = collect(Query::<(&C, &A)>::new(&world), |(c, a)| (a.0, c.0));
        assert_eq!(found, [(1, (1, 10)), (2, (2, 20)), (3, (3, 30)), (4, (4, 40)), (6, (6, 60))]);
    }

    #[test]
    fn query_works_on_every_world() {
        fn check(world: &impl WorldImpl) {
            for i in 1..=20u32 {
                world.insert(i as EntityID, comps(by_mask(i % 16)));
            }
            let found = collect(Query::<(&B, &D)>::new(world), |(b, d)| (b.0, d.0));
            let expected = (1..=20u32)
                .filter(|i| i % 16 & 0b1010 == 0b1010)
                .map(|i| (i as EntityID, (i % 16, i % 16)))
                .collect::<Vec<_>>();
            assert_eq!(found, expected);
        }
        check(&IndexedWorld::new());
        check(&ListWorld::new());
    }

    #[test]
    fn query_mutations_are_visible_to_later_queries() {
        let world = IndexedWorld::new();
        for i in 0..10 {
            world.insert(i, comps(vec![Box::new(A(i as u32)), Box::new(B(1))]));
            world.insert(i + 10, comps(vec![Box::new(A(i as u32)), Box::new(B(1)), Box::new(C(0))]));
        }

        let mut query = Query::<(&mut A, &B)>::new(&world);
        while let Some((_, (mut a, b))) = query.next().unwrap() {
            a.0 += b.0 * 100;
        }

        let found = collect(Query::<&A>::new(&world), |a| a.0);
        let expected = (0..20).map(|i| (i, i as u32 % 10 + 100)).collect::<Vec<_>>();
        assert_eq!(found, expected);
    }

    #[test]
    fn query_skips_empty_tables() {
        let layout = group(&[A::id(), B::id()]);
        let empty = Table::default(&layout);
        let full = Table::default(&layout);
        full.insert(1, stored(vec![Box::new(A(1)), Box::new(B(2))]).1);
        let emptied = Table::default(&layout);
        emptied.insert(2, stored(vec![Box::new(A(0)), Box::new(B(0))]).1);
        emptied.cursor().pop();

        let cursors = vec![empty.cursor(), emptied.cursor(), full.cursor(), empty.cursor()];
        let found = collect(Query::<(&A, &B)>::from_iter(Box::new(cursors.into_iter())), |(a, b)| (a.0, b.0));
        assert_eq!(found, [(1, (1, 2))]);
    }

    #[test]
    fn query_on_empty_world_is_done() {
        let world = IndexedWorld::new();
        let mut query = Query::<(&A, &B)>::new(&world);
        assert!(query.next().unwrap().is_none());
        assert!(query.next().unwrap().is_none());
    }

    #[test]
    fn query_with_optional_in_a_table_without_it() {
        // the world is asked for every listed component, so drive the cursor directly
        // to check an optional component missing from the table resolves to None
        let table = Table::default(&group(&[A::id()]));
        table.insert(1, comps(vec![Box::new(A(1))]));
        let query = Query::<(&A, Option<&B>)>::from_iter(Box::new(std::iter::once(table.cursor())));
        let found = collect(query, |(a, b)| (a.0, b.map(|b| b.0)));
        assert_eq!(found, [(1, (1, None))]);
    }
}
