//! Groups of components a query yields together.

use crate::*;

/// The most components a [`QueryGroup`] or [`Bundle`] may list.
pub const MAX_PARAMS: usize = 8;

/// The components a query yields for each entity, a [`QueryComponent`] or a tuple of up
/// to [`MAX_PARAMS`] of them.
///
/// # Safety
///
/// `LEN` and [`param`](Self::param) must match the terms, and the other methods must pass
/// slot `i` to term `i`.
pub unsafe trait QueryGroup {
    /// The guards handed out for each entity, in the same shape as the group.
    type Output;
    /// The slices handed out for a run of rows, in the same shape as the group.
    type Slices<'a>;
    /// The references handed out for one row, in the same shape as the group.
    type Items<'a>;
    /// How many terms the group has.
    const LEN: usize;

    /// Describes term `i`.
    fn param(i: usize) -> Param;

    /// True if any component in the group is borrowed mutably.
    fn req_mut() -> bool { (0..Self::LEN).any(|i| Self::param(i).mutable) }

    /// The id of every component in the group, in the group's order.
    fn req_comps() -> ComponentIDGroup { (0..Self::LEN).map(|i| Self::param(i).id).collect() }

    /// The ids of the components a table must have to match, those of every term that
    /// isn't an `Option`.  All of them if every term is one.
    fn required() -> ComponentIDGroup {
        let required = (0..Self::LEN).map(Self::param).filter(|param| !param.optional).map(|param| param.id).collect::<ComponentIDGroup>();
        if required.is_empty() { Self::req_comps() } else { required }
    }

    /// Wraps each term's borrowed value in its guard.
    ///
    /// # Safety
    ///
    /// Each slot must satisfy [`QueryComponent::output`] for its term.
    unsafe fn output(values: &mut [Option<RawBorrow>; MAX_PARAMS]) -> Self::Output;

    /// Views `len` rows of each term's column.
    ///
    /// # Safety
    ///
    /// Each column must satisfy [`QueryComponent::slice`] for its term.
    unsafe fn slices<'a>(columns: &[Option<*mut u8>; MAX_PARAMS], len: usize) -> Self::Slices<'a>;

    /// Views one row of each term's column.
    ///
    /// # Safety
    ///
    /// Each column must satisfy [`QueryComponent::item`] for its term.
    unsafe fn items<'a>(columns: &[Option<*mut u8>; MAX_PARAMS]) -> Self::Items<'a>;
}

unsafe impl <A: QueryComponent> QueryGroup for A {
    type Output = A::Output;
    type Slices<'a> = A::Slice<'a>;
    type Items<'a> = A::Item<'a>;
    const LEN: usize = 1;

    fn param(_: usize) -> Param { A::param() }
    unsafe fn output(values: &mut [Option<RawBorrow>; MAX_PARAMS]) -> Self::Output { unsafe { A::output(values[0].take()) } }
    unsafe fn slices<'a>(columns: &[Option<*mut u8>; MAX_PARAMS], len: usize) -> Self::Slices<'a> { unsafe { A::slice(columns[0], len) } }
    unsafe fn items<'a>(columns: &[Option<*mut u8>; MAX_PARAMS]) -> Self::Items<'a> { unsafe { A::item(columns[0]) } }
}

macro_rules! tuple_group {
    (@count) => { 0 };
    (@count $head:ident $($tail:ident)*) => { 1 + tuple_group!(@count $($tail)*) };

    ($($name:ident $idx:tt),+) => {
        unsafe impl <$($name: QueryComponent),+> QueryGroup for ($($name,)+) {
            type Output = ($($name::Output,)+);
            type Slices<'a> = ($($name::Slice<'a>,)+);
            type Items<'a> = ($($name::Item<'a>,)+);
            const LEN: usize = tuple_group!(@count $($name)+);

            fn param(i: usize) -> Param { [$($name::param()),+][i] }
            unsafe fn output(values: &mut [Option<RawBorrow>; MAX_PARAMS]) -> Self::Output {
                unsafe { ($($name::output(values[$idx].take()),)+) }
            }
            unsafe fn slices<'a>(columns: &[Option<*mut u8>; MAX_PARAMS], len: usize) -> Self::Slices<'a> {
                unsafe { ($($name::slice(columns[$idx], len),)+) }
            }
            unsafe fn items<'a>(columns: &[Option<*mut u8>; MAX_PARAMS]) -> Self::Items<'a> {
                unsafe { ($($name::item(columns[$idx]),)+) }
            }
        }
    };
}

tuple_group!(A 0);
tuple_group!(A 0, B 1);
tuple_group!(A 0, B 1, C 2);
tuple_group!(A 0, B 1, C 2, D 3);
tuple_group!(A 0, B 1, C 2, D 3, E 4);
tuple_group!(A 0, B 1, C 2, D 3, E 4, F 5);
tuple_group!(A 0, B 1, C 2, D 3, E 4, F 5, G 6);
tuple_group!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7);


#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::worlds::test_support::*;

    /// A world holding one entity, `1`, with `components`.
    fn world_of(components: Vec<DynComponent>) -> IndexedWorld {
        let world = IndexedWorld::new();
        world.insert(1, comps(components));
        world
    }

    /// Gets `QG` from the one entity holding `components`.
    fn extract<QG: QueryGroup>(components: Vec<DynComponent>) -> anyhow::Result<QG::Output> {
        world_of(components).get::<QG>(1).map(Option::unwrap)
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
    fn required_skips_optional_terms() {
        assert_eq!(*<(&B, Option<&D>, &mut A)>::required(), [B::id(), A::id()]);
        // a query of only optional terms still needs one of them
        assert_eq!(*<(Option<&A>, Option<&B>)>::required(), [A::id(), B::id()]);
    }

    #[test]
    fn req_mut_if_any_component_is_mut() {
        assert!(!<&A as QueryGroup>::req_mut());
        assert!(<&mut A as QueryGroup>::req_mut());
        assert!(!<(&A, &B, Option<&C>)>::req_mut());
        assert!(<(&A, &mut B)>::req_mut());
        assert!(<(&A, &B, Option<&mut C>)>::req_mut());
    }

    // ---- extraction ----

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
    fn duplicate_shared_borrows_release_separately() {
        let world = world_of(vec![Box::new(A(1))]);
        let (first, second) = world.get::<(&A, &A)>(1).unwrap().unwrap();
        drop(first);
        drop(second);
        // both borrows were released, so it can be borrowed mutably again
        world.get::<&mut A>(1).unwrap().unwrap().0 = 2;
        assert_eq!(world.get::<&A>(1).unwrap().unwrap().0, 2);
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
    fn borrowing_mutably_twice_errors() {
        assert!(extract::<(&mut A, &A)>(vec![Box::new(A(1))]).is_err());
        assert!(extract::<(&mut A, Option<&mut A>)>(vec![Box::new(A(1))]).is_err());
    }

    #[test]
    fn mut_writes_through_to_storage() {
        let world = world_of(vec![Box::new(A(1)), Box::new(B(2))]);
        {
            let (mut a, b) = world.get::<(&mut A, &B)>(1).unwrap().unwrap();
            a.0 += b.0;
        }
        assert_eq!(world.get::<&A>(1).unwrap().unwrap().0, 3);
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
    fn query_skips_empty_and_removed() {
        let world = IndexedWorld::new();
        let empty = world.table_for(vec![ComponentInfo::of::<A>(), ComponentInfo::of::<B>(), ComponentInfo::of::<C>()]).unwrap();
        for i in 0..5 { world.insert(i, comps(vec![Box::new(A(i as u32)), Box::new(B(0))])); }
        world.remove(1);
        world.remove(3);

        let tables: Vec<Arc<ChunkedTable>> = vec![empty.clone(), world.raw_query(ids(&[A::id(), B::id()])).find(|t| t.group().len() == 2).unwrap(), empty];
        let found = collect(Query::<(&A, &B)>::from_iter(Box::new(tables.into_iter())), |(a, _)| a.0);
        assert_eq!(found, [(0, 0), (2, 2), (4, 4)]);
    }

    #[test]
    fn query_on_empty_world_is_done() {
        let world = IndexedWorld::new();
        let mut query = Query::<(&A, &B)>::new(&world);
        assert!(query.next().unwrap().is_none());
        assert!(query.next().unwrap().is_none());
    }

    #[test]
    fn query_optional_resolves_to_none_without_it() {
        let world = IndexedWorld::new();
        world.insert(1, comps(vec![Box::new(A(1))]));
        world.insert(2, comps(vec![Box::new(A(2)), Box::new(B(20))]));
        let found = collect(Query::<(&A, Option<&B>)>::new(&world), |(a, b)| (a.0, b.map(|b| b.0)));
        assert_eq!(found, [(1, (1, None)), (2, (2, Some(20)))]);
    }

    #[test]
    fn guards_from_a_query_outlive_it() {
        let world = world_of(vec![Box::new(A(1))]);
        let mut query = Query::<&mut A>::new(&world);
        let (_, mut a) = query.next().unwrap().unwrap();
        drop(query);
        a.0 = 5;
        drop(a);
        assert_eq!(world.get::<&A>(1).unwrap().unwrap().0, 5);
    }

    #[test]
    fn holding_one_entity_while_getting_another() {
        // the hierarchy pattern: a parent's guard is held while children in the same table are read and written
        let world = IndexedWorld::new();
        for i in 0..10 { world.insert(i, comps(vec![Box::new(A(i as u32))])); }
        let mut query = Query::<&mut A>::new(&world);
        while let Some((id, mut parent)) = query.next().unwrap() {
            let child = (id + 1) % 10;
            let child = world.get::<&A>(child).unwrap().unwrap();
            parent.0 += child.0 * 0;
        }
    }
}
