//! [`ComponentInfo`], which describes a component type by its memory layout, and the
//! registry that maps component ids to them.

use std::{alloc::Layout, any::TypeId, collections::hash_map::Entry, fmt::{self, Debug, Formatter}, sync::{LazyLock, RwLock}};

use anyhow::bail;
use rustc_hash::FxHashMap;

use crate::*;

/// Describes a component type by its layout instead of its Rust type, so a [`ChunkedTable`]
/// can keep any component as raw bytes, and code that doesn't know the type at compile time
/// can still drop, print and box it.
///
/// Native types get theirs from [`ComponentInfo::of`], or [`ComponentInfo::of_as`] for a
/// Rust type stored under another id.  Types only known at runtime, such as plain bytes a
/// wasm module defines, are added with [`ComponentInfo::register`].  Tables only accept
/// infos from the registry, so every id has exactly one description.
#[derive(Debug, Clone, Copy)]
pub struct ComponentInfo {
    /// The component's id, [`ComponentMeta::id`] for native types.
    pub id: ComponentID,
    /// The type's name, for debugging and error messages.
    pub name: &'static str,
    /// The size and alignment of one value.
    pub layout: Layout,
    /// The Rust type of the values, `None` for types that are only bytes.
    pub type_id: Option<TypeId>,
    /// Drops a value in place.  `None` for types without drop glue, such as plain data
    /// and wasm types.
    pub drop: Option<unsafe fn(*mut u8)>,
    /// Formats the value at the pointer.
    pub debug: unsafe fn(&ComponentInfo, *const u8, &mut Formatter) -> fmt::Result,
    /// Views the value at the pointer as a `dyn Component`, `None` for types that are only bytes.
    pub as_dyn: Option<unsafe fn(*mut u8) -> *mut dyn Component>,
    /// Moves the value at the pointer into a box, `None` for types that are only bytes.
    pub boxed: Option<unsafe fn(*mut u8) -> DynComponent>
}

/// Every registered component, leaked so storage can keep `&'static` references to them.
static REGISTRY: LazyLock<RwLock<FxHashMap<ComponentID, &'static ComponentInfo>>> = LazyLock::new(Default::default);

impl ComponentInfo {
    /// The info for `T`, registering it on first use.
    ///
    /// # Panics
    ///
    /// Panics if another type was registered under `T`'s id.
    pub fn of<T: ComponentMeta + Sync>() -> &'static ComponentInfo {
        Self::of_as::<T>(T::id(), std::any::type_name::<T>()).unwrap_or_else(|error| panic!("{error}"))
    }

    /// The info for the Rust type `T` stored under `id` and `name`, registering it on first
    /// use.  For types whose values carry their own id, such as one Rust type holding the
    /// bytes of many wasm types, where each value's [`Component::get_id`] must be `id`.
    ///
    /// # Errors
    ///
    /// Errors if a different type, name or layout is registered under `id`.
    pub fn of_as<T: Component + Sync>(id: ComponentID, name: &'static str) -> anyhow::Result<&'static ComponentInfo> {
        if let Some(info) = Self::get(id) && info.type_id == Some(TypeId::of::<T>()) && info.name == name {
            return Ok(info);
        }
        let info = ComponentInfo {
            id,
            name,
            layout: Layout::new::<T>(),
            type_id: Some(TypeId::of::<T>()),
            drop: std::mem::needs_drop::<T>().then_some(drop_value::<T> as unsafe fn(*mut u8)),
            debug: debug_value::<T>,
            as_dyn: Some(as_dyn::<T>),
            boxed: Some(boxed::<T>)
        };
        // safety: the info is built from `T` itself
        unsafe { Self::register(info) }
    }

    /// The registered info for the component `id`, if there is one.
    pub fn get(id: ComponentID) -> Option<&'static ComponentInfo> {
        REGISTRY.read().unwrap().get(&id).copied()
    }

    /// Registers `info` under its id and returns the registered copy.  Registering the
    /// same name, layout and type again returns the existing info.
    ///
    /// # Errors
    ///
    /// Errors if a component with a different name, layout or type already has this id.
    ///
    /// # Safety
    ///
    /// `info` must describe the values stored under its id: `layout` must be their exact
    /// size and alignment, `type_id` their Rust type if they have one, the hooks must be
    /// sound to call on any such value, and the values must be safe to send and share
    /// between threads.
    pub unsafe fn register(info: ComponentInfo) -> anyhow::Result<&'static ComponentInfo> {
        let mut registry = REGISTRY.write().unwrap();
        match registry.entry(info.id) {
            Entry::Occupied(existing) => {
                let existing = *existing.get();
                if existing.name != info.name || existing.layout != info.layout || existing.type_id != info.type_id {
                    bail!(
                        "component id {} is registered as {} ({:?}), not {} ({:?})",
                        info.id, existing.name, existing.layout, info.name, info.layout
                    );
                }
                Ok(existing)
            },
            Entry::Vacant(entry) => Ok(*entry.insert(Box::leak(Box::new(info))))
        }
    }

    /// True if this info's values are `T`s.
    pub fn is<T: 'static>(&self) -> bool {
        self.type_id == Some(TypeId::of::<T>())
    }

    /// Formats the value at `ptr` with this info's `debug` hook.
    ///
    /// # Safety
    ///
    /// `ptr` must point at a live value of this component.
    pub unsafe fn fmt_value(&self, ptr: *const u8, f: &mut Formatter) -> fmt::Result {
        unsafe { (self.debug)(self, ptr, f) }
    }
}

/// A `debug` hook for types with no Rust `Debug`, prints the value's bytes.
///
/// # Safety
///
/// `ptr` must point at `info.layout.size()` initialized bytes.
pub unsafe fn debug_bytes(info: &ComponentInfo, ptr: *const u8, f: &mut Formatter) -> fmt::Result {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, info.layout.size()) };
    write!(f, "{} {bytes:02x?}", info.name)
}

unsafe fn drop_value<T>(ptr: *mut u8) {
    unsafe { std::ptr::drop_in_place(ptr as *mut T) }
}

unsafe fn debug_value<T: Debug>(_: &ComponentInfo, ptr: *const u8, f: &mut Formatter) -> fmt::Result {
    unsafe { Debug::fmt(&*(ptr as *const T), f) }
}

unsafe fn as_dyn<T: Component>(ptr: *mut u8) -> *mut dyn Component {
    ptr as *mut T as *mut dyn Component
}

unsafe fn boxed<T: Component>(ptr: *mut u8) -> DynComponent {
    Box::new(unsafe { (ptr as *mut T).read() })
}


#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Component)]
    struct Plain(#[allow(dead_code)] u32, #[allow(dead_code)] u8);

    #[derive(Debug, Component)]
    struct Owning(#[allow(dead_code)] Vec<u32>);

    struct Display<'a>(&'a ComponentInfo, *const u8);

    impl Debug for Display<'_> {
        fn fmt(&self, f: &mut Formatter) -> fmt::Result { unsafe { self.0.fmt_value(self.1, f) } }
    }

    #[test]
    fn describes_the_type() {
        let info = ComponentInfo::of::<Plain>();
        assert_eq!(info.id, Plain::id());
        assert_eq!(info.layout, Layout::new::<Plain>());
        assert!(info.drop.is_none());
        assert!(ComponentInfo::of::<Owning>().drop.is_some());
        assert!(info.is::<Plain>() && !info.is::<Owning>());
        assert!(ComponentInfo::of_as::<Plain>(Plain::id(), "Renamed").is_err());

        let value = Plain(7, 1);
        assert_eq!(format!("{:?}", Display(info, &value as *const Plain as *const u8)), "Plain(7, 1)");
    }

    #[test]
    fn registers_once() {
        let first = ComponentInfo::of::<Plain>();
        assert!(std::ptr::eq(first, ComponentInfo::of::<Plain>()));
        assert!(std::ptr::eq(first, ComponentInfo::get(Plain::id()).unwrap()));
    }

    #[test]
    fn runtime_types() {
        let info = ComponentInfo {
            id: 0xC0FFEE,
            name: "guest::Pair",
            layout: Layout::new::<[u16; 2]>(),
            type_id: None,
            drop: None,
            debug: debug_bytes,
            as_dyn: None,
            boxed: None
        };
        let registered = unsafe { ComponentInfo::register(info) }.unwrap();
        assert!(std::ptr::eq(registered, unsafe { ComponentInfo::register(info) }.unwrap()));

        let other = ComponentInfo { layout: Layout::new::<u64>(), ..info };
        assert!(unsafe { ComponentInfo::register(other) }.is_err());
        assert!(unsafe { ComponentInfo::register(ComponentInfo { name: "guest::Other", ..info }) }.is_err());

        let value = [1u16, 0x0203];
        assert_eq!(format!("{:?}", Display(registered, value.as_ptr() as *const u8)), "guest::Pair [01, 00, 03, 02]");
    }
}
