//! The single component terms a query is made of.

use std::{alloc::Layout, any::TypeId};

use crate::*;

/// One component a query asks for, as [`QueryComponent::param`] describes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Param {
    /// The component's id.
    pub id: ComponentID,
    /// The Rust type the table's column must hold.
    pub type_id: TypeId,
    /// The type's layout.
    pub layout: Layout,
    /// True if the component is borrowed mutably.
    pub mutable: bool,
    /// True if a table without the component still matches.
    pub optional: bool
}

/// A single component a query can ask for: `&A`, `&mut A`, `Option<&A>` or
/// `Option<&mut A>`.
///
/// # Safety
///
/// [`param`](Self::param) must describe `A` exactly, and the other methods must only make
/// references to `A`, mutable only when `param().mutable` is.
pub unsafe trait QueryComponent {
    /// The guard [`Query`] and [`WorldImpl::get`] hand out.
    type Output;
    /// What [`ChunkedTable::for_each_chunk`] hands out for a run of rows.
    type Slice<'a>;
    /// What [`ChunkedTable::for_each`] and [`ChunkedTable::get`] hand out for one row.
    type Item<'a>;

    /// Describes the component this term asks for.
    fn param() -> Param;

    /// True if this term borrows its component mutably.
    fn req_mut() -> bool { Self::param().mutable }

    /// The id of the component this term reads.
    fn req_comp() -> ComponentID { Self::param().id }

    /// Wraps a borrowed value in this term's guard, `None` if the table lacks the component.
    ///
    /// # Safety
    ///
    /// The caller must hold `value`'s borrow flag with this term's access, which moves
    /// into the guard, and the value must be an `A`.
    unsafe fn output(value: Option<RawBorrow>) -> Self::Output;

    /// Views `len` values starting at `column`, `None` if the table lacks the component.
    ///
    /// # Safety
    ///
    /// `column` must point at `len` live `A`s the caller holds with this term's access.
    unsafe fn slice<'a>(column: Option<*mut u8>, len: usize) -> Self::Slice<'a>;

    /// Views the value at `column`, `None` if the table lacks the component.
    ///
    /// # Safety
    ///
    /// As for [`slice`](Self::slice) with a `len` of 1.
    unsafe fn item<'a>(column: Option<*mut u8>) -> Self::Item<'a>;
}

macro_rules! term {
    (
        $term:ty, $mutable:literal, $optional:literal, $output:ty, $slice:ty, $item:ty,
        |$o:ident| $make_output:expr, |$c:ident, $len:ident| $make_slice:expr, |$i:ident| $make_item:expr
    ) => {
        unsafe impl <A: ComponentMeta + Sync> QueryComponent for $term {
            type Output = $output;
            type Slice<'a> = $slice;
            type Item<'a> = $item;

            fn param() -> Param {
                Param { id: A::id(), type_id: TypeId::of::<A>(), layout: Layout::new::<A>(), mutable: $mutable, optional: $optional }
            }

            unsafe fn output($o: Option<RawBorrow>) -> Self::Output { unsafe { $make_output } }
            unsafe fn slice<'a>($c: Option<*mut u8>, $len: usize) -> Self::Slice<'a> { unsafe { $make_slice } }
            unsafe fn item<'a>($i: Option<*mut u8>) -> Self::Item<'a> { unsafe { $make_item } }
        }
    };
}

term!(&A, false, false, Ref<A>, &'a [A], &'a A,
    |o| Ref::new(o.unwrap()),
    |c, len| std::slice::from_raw_parts(c.unwrap() as *const A, len),
    |c| &*(c.unwrap() as *const A));
term!(&mut A, true, false, Mut<A>, &'a mut [A], &'a mut A,
    |o| Mut::new(o.unwrap()),
    |c, len| std::slice::from_raw_parts_mut(c.unwrap() as *mut A, len),
    |c| &mut *(c.unwrap() as *mut A));
term!(Option<&A>, false, true, Option<Ref<A>>, Option<&'a [A]>, Option<&'a A>,
    |o| o.map(|o| Ref::new(o)),
    |c, len| c.map(|c| std::slice::from_raw_parts(c as *const A, len)),
    |c| c.map(|c| &*(c as *const A)));
term!(Option<&mut A>, true, true, Option<Mut<A>>, Option<&'a mut [A]>, Option<&'a mut A>,
    |o| o.map(|o| Mut::new(o)),
    |c, len| c.map(|c| std::slice::from_raw_parts_mut(c as *mut A, len)),
    |c| c.map(|c| &mut *(c as *mut A)));
