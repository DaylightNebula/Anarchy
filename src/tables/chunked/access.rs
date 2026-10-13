//! [`Bundle`], the typed components inserted into a [`ChunkedTable`] together.

use crate::*;

/// Components inserted into a [`ChunkedTable`] together: a tuple of up to [`MAX_PARAMS`]
/// components, which must be exactly the table's components in any order.
///
/// # Safety
///
/// `LEN` and `info` must match the tuple, and `write` must move element `i` to `dst[i]`.
pub unsafe trait Bundle: Sized {
    /// How many components the bundle holds.
    const LEN: usize;

    /// The info of component `i`.
    fn info(i: usize) -> &'static ComponentInfo;

    /// Moves component `i` to `dst[i]` for every component.
    ///
    /// # Safety
    ///
    /// Each `dst[i]` must be valid for writes of component `i` and not overlap the others.
    unsafe fn write(self, dst: &[*mut u8; MAX_PARAMS]);
}

macro_rules! bundles {
    (@count) => { 0 };
    (@count $head:ident $($tail:ident)*) => { 1 + bundles!(@count $($tail)*) };

    ($($name:ident $idx:tt),+) => {
        unsafe impl <$($name: ComponentMeta + Sync),+> Bundle for ($($name,)+) {
            const LEN: usize = bundles!(@count $($name)+);

            fn info(i: usize) -> &'static ComponentInfo { [$(ComponentInfo::of::<$name>),+][i]() }
            unsafe fn write(self, dst: &[*mut u8; MAX_PARAMS]) {
                unsafe { $((dst[$idx] as *mut $name).write(self.$idx);)+ }
            }
        }
    };
}

bundles!(A 0);
bundles!(A 0, B 1);
bundles!(A 0, B 1, C 2);
bundles!(A 0, B 1, C 2, D 3);
bundles!(A 0, B 1, C 2, D 3, E 4);
bundles!(A 0, B 1, C 2, D 3, E 4, F 5);
bundles!(A 0, B 1, C 2, D 3, E 4, F 5, G 6);
bundles!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7);
