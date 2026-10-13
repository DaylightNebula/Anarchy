//! Guards over components in a [`ChunkedTable`], typed ([`Ref`], [`Mut`]) and type
//! erased ([`ComponentRef`], [`ComponentMut`]).  Each holds its row's borrow flag until
//! dropped, and keeps the table alive.

use std::{fmt::{self, Debug, Formatter}, marker::PhantomData, ops::{Deref, DerefMut}, ptr::NonNull, sync::{Arc, atomic::AtomicU32}};

use crate::{tables::chunked::borrow, *};

/// A value in a table whose borrow flag the holder has taken, handed to
/// [`QueryComponent::output`] to wrap in a guard.
pub struct RawBorrow {
    pub(crate) info: &'static ComponentInfo,
    pub(crate) ptr: NonNull<u8>,
    pub(crate) flag: NonNull<AtomicU32>,
    pub(crate) table: Arc<ChunkedTable>
}

/// Shared access to a component, released when dropped.
pub struct Ref<T> {
    value: NonNull<T>,
    flag: NonNull<AtomicU32>,
    _table: Arc<ChunkedTable>,
    _owns: PhantomData<T>
}

/// Mutable access to a component, released when dropped.
pub struct Mut<T> {
    value: NonNull<T>,
    flag: NonNull<AtomicU32>,
    _table: Arc<ChunkedTable>,
    _owns: PhantomData<T>
}

// safety: guards only hand out `&T` (and `&mut T` for `Mut`), and their flags are atomics
unsafe impl <T: Sync> Send for Ref<T> {}
unsafe impl <T: Sync> Sync for Ref<T> {}
unsafe impl <T: Send> Send for Mut<T> {}
unsafe impl <T: Sync> Sync for Mut<T> {}

impl <T> Ref<T> {
    /// Wraps a value borrowed shared.
    ///
    /// # Safety
    ///
    /// The caller must hold a shared borrow of `raw`'s flag, which moves into the guard,
    /// and the value must be a `T`.
    pub(crate) unsafe fn new(raw: RawBorrow) -> Self {
        Self { value: raw.ptr.cast(), flag: raw.flag, _table: raw.table, _owns: PhantomData }
    }
}

impl <T> Mut<T> {
    /// Wraps a value borrowed mutably.
    ///
    /// # Safety
    ///
    /// As for [`Ref::new`], with a mutable borrow.
    pub(crate) unsafe fn new(raw: RawBorrow) -> Self {
        Self { value: raw.ptr.cast(), flag: raw.flag, _table: raw.table, _owns: PhantomData }
    }
}

impl <T> Deref for Ref<T> {
    type Target = T;
    fn deref(&self) -> &T { unsafe { self.value.as_ref() } }
}

impl <T> Deref for Mut<T> {
    type Target = T;
    fn deref(&self) -> &T { unsafe { self.value.as_ref() } }
}

impl <T> DerefMut for Mut<T> {
    fn deref_mut(&mut self) -> &mut T { unsafe { self.value.as_mut() } }
}

impl <T> Drop for Ref<T> {
    fn drop(&mut self) { borrow::unlock_read(unsafe { self.flag.as_ref() }, 1) }
}

impl <T> Drop for Mut<T> {
    fn drop(&mut self) { borrow::unlock_write(unsafe { self.flag.as_ref() }) }
}

impl <T: Debug> Debug for Ref<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result { T::fmt(self, f) }
}

impl <T: Debug> Debug for Mut<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result { T::fmt(self, f) }
}

/// Shared access to a component whose type isn't known at compile time, see
/// [`Entity::component`].  Released when dropped.
pub struct ComponentRef(RawBorrow);

/// Mutable access to a component whose type isn't known at compile time, see
/// [`Entity::component_mut`].  Released when dropped.
pub struct ComponentMut(RawBorrow);

// safety: as for `Ref` and `Mut`, the registry only holds types that are `Send + Sync`
unsafe impl Send for ComponentRef {}
unsafe impl Sync for ComponentRef {}
unsafe impl Send for ComponentMut {}
unsafe impl Sync for ComponentMut {}

impl ComponentRef {
    /// Wraps a value borrowed shared, see [`Ref::new`].
    pub(crate) unsafe fn new(raw: RawBorrow) -> Self { Self(raw) }

    /// Describes the component.
    pub fn info(&self) -> &'static ComponentInfo { self.0.info }

    /// A pointer to the value, valid while this guard is.
    pub fn ptr(&self) -> *const u8 { self.0.ptr.as_ptr() }

    /// The value as a `dyn Component`, `None` for types that are only bytes.
    pub fn as_dyn(&self) -> Option<&dyn Component> {
        self.0.info.as_dyn.map(|as_dyn| unsafe { &*as_dyn(self.0.ptr.as_ptr()) })
    }

    /// The value as a `T`, or `None` if it is a different type.
    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        self.0.info.is::<T>().then(|| unsafe { self.0.ptr.cast::<T>().as_ref() })
    }
}

impl ComponentMut {
    /// Wraps a value borrowed mutably, see [`Mut::new`].
    pub(crate) unsafe fn new(raw: RawBorrow) -> Self { Self(raw) }

    /// Describes the component.
    pub fn info(&self) -> &'static ComponentInfo { self.0.info }

    /// A pointer to the value, valid while this guard is.
    pub fn ptr(&mut self) -> *mut u8 { self.0.ptr.as_ptr() }

    /// The value as a `dyn Component`, `None` for types that are only bytes.
    pub fn as_dyn(&self) -> Option<&dyn Component> {
        self.0.info.as_dyn.map(|as_dyn| unsafe { &*as_dyn(self.0.ptr.as_ptr()) })
    }

    /// The value as a mutable `dyn Component`, `None` for types that are only bytes.
    pub fn as_dyn_mut(&mut self) -> Option<&mut dyn Component> {
        self.0.info.as_dyn.map(|as_dyn| unsafe { &mut *as_dyn(self.0.ptr.as_ptr()) })
    }

    /// The value as a `T`, or `None` if it is a different type.
    pub fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.0.info.is::<T>().then(|| unsafe { self.0.ptr.cast::<T>().as_mut() })
    }
}

impl Drop for ComponentRef {
    fn drop(&mut self) { borrow::unlock_read(unsafe { self.0.flag.as_ref() }, 1) }
}

impl Drop for ComponentMut {
    fn drop(&mut self) { borrow::unlock_write(unsafe { self.0.flag.as_ref() }) }
}

impl Debug for ComponentRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result { unsafe { self.0.info.fmt_value(self.0.ptr.as_ptr(), f) } }
}

impl Debug for ComponentMut {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result { unsafe { self.0.info.fmt_value(self.0.ptr.as_ptr(), f) } }
}
