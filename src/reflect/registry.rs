//! [`TypeRegistry`], the reflected types a world can build, cast and name at runtime.

use std::{any::TypeId, collections::HashMap};

use anyhow::Context;

use crate::*;

/// Casts between a component type and [`Reflect`], see [`TypeRegistration::component`].
#[derive(Debug, Clone, Copy)]
pub struct ReflectComponent {
    /// The component's id.
    pub id: ComponentID,
    /// The component as `dyn Reflect`, `None` if it is another type.
    pub as_reflect: fn(&dyn Component) -> Option<&dyn Reflect>,
    /// The component as `dyn Reflect`, `None` if it is another type.
    pub as_reflect_mut: fn(&mut dyn Component) -> Option<&mut dyn Reflect>,
    /// A reflected value of this type as a component, an error if it is another type.
    pub into_component: fn(Box<dyn Reflect>) -> anyhow::Result<DynComponent>
}

/// Casts between a resource type and [`Reflect`], see [`TypeRegistration::resource`].
#[derive(Debug, Clone, Copy)]
pub struct ReflectResource {
    /// The resource's id.
    pub id: ResourceID,
    /// The resource as `dyn Reflect`, `None` if it is another type.
    pub as_reflect: fn(&dyn Resource) -> Option<&dyn Reflect>,
    /// The resource as `dyn Reflect`, `None` if it is another type.
    pub as_reflect_mut: fn(&mut dyn Resource) -> Option<&mut dyn Reflect>,
    /// A reflected value of this type as a resource, an error if it is another type.
    pub into_resource: fn(Box<dyn Reflect>) -> anyhow::Result<DynResource>
}

/// A registered type: its info and the functions that build and cast it.
#[derive(Debug, Clone)]
pub struct TypeRegistration {
    /// The type's [`TypeId`].
    pub type_id: TypeId,
    /// Describes the type.
    pub info: &'static TypeInfo,
    /// Builds the type from a [`Value`], see [`FromValue::from_value`].
    pub from_value: fn(&Value) -> anyhow::Result<Box<dyn Reflect>>,
    /// The type's fallback, see [`FromValue::fallback`].
    pub fallback: fn() -> Option<Box<dyn Reflect>>,
    /// Set if the type is a component.
    pub component: Option<ReflectComponent>,
    /// Set if the type is a resource.
    pub resource: Option<ReflectResource>
}

impl ReflectComponent {
    /// The casts for the component `T`.
    pub fn of<T: ComponentMeta + Reflect + Typed + FromValue>() -> Self {
        Self {
            id: T::id(),
            as_reflect: |c| c.as_any().downcast_ref::<T>().map(|c| c as &dyn Reflect),
            as_reflect_mut: |c| c.as_any_mut().downcast_mut::<T>().map(|c| c as &mut dyn Reflect),
            into_component: |value| {
                let value = value.into_any().downcast::<T>().map_err(|_| anyhow::anyhow!("not a {}", T::type_info().short_name))?;
                Ok(value as DynComponent)
            }
        }
    }
}

impl ReflectResource {
    /// The casts for the resource `R`.
    pub fn of<R: ResourceMeta + Reflect + Typed + FromValue>() -> Self {
        Self {
            id: R::id(),
            as_reflect: |r| r.as_any().downcast_ref::<R>().map(|r| r as &dyn Reflect),
            as_reflect_mut: |r| r.as_any_mut().downcast_mut::<R>().map(|r| r as &mut dyn Reflect),
            into_resource: |value| {
                let value = value.into_any().downcast::<R>().map_err(|_| anyhow::anyhow!("not a {}", R::type_info().short_name))?;
                Ok(value as DynResource)
            }
        }
    }
}

impl TypeRegistration {
    /// A registration for a plain reflected type, neither a component nor a resource.
    pub fn of<T: Reflect + Typed + FromValue>() -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            info: T::type_info(),
            from_value: |value| Ok(Box::new(T::from_value(value)?)),
            fallback: || T::fallback().map(|value| Box::new(value) as Box<dyn Reflect>),
            component: None,
            resource: None
        }
    }

    /// A registration for a reflected component.
    pub fn of_component<T: ComponentMeta + Reflect + Typed + FromValue>() -> Self {
        Self { component: Some(ReflectComponent::of::<T>()), ..Self::of::<T>() }
    }

    /// A registration for a reflected resource.
    pub fn of_resource<R: ResourceMeta + Reflect + Typed + FromValue>() -> Self {
        Self { resource: Some(ReflectResource::of::<R>()), ..Self::of::<R>() }
    }

    /// The type's name without module paths.
    pub fn short_name(&self) -> &str { &self.info.short_name }

    /// Builds the type from `value`, see [`FromValue::from_value`].
    ///
    /// # Errors
    ///
    /// Errors if `value` doesn't fit the type.
    pub fn build(&self, value: &Value) -> anyhow::Result<Box<dyn Reflect>> {
        (self.from_value)(value).with_context(|| format!("building {}", self.info.short_name))
    }
}

/// A registration submitted by `#[derive(Reflect)]` for a non generic type, collected
/// with `inventory` by [`TypeRegistry::new`].
#[doc(hidden)]
pub struct AutoRegistration(pub fn() -> TypeRegistration);

inventory::collect!(AutoRegistration);

/// Picks a type's [`ReflectComponent`] and [`ReflectResource`] at compile time, for
/// `#[derive(Reflect)]`, which can't tell whether its type is also a component or a
/// resource.  Calling `(&&RegistrationProbe::<T>::new()).reflect_component()` finds the
/// impl on `&RegistrationProbe<T>` when `T` is a component, and falls back to the one on
/// `RegistrationProbe<T>`, returning `None`, when it isn't.
#[doc(hidden)]
pub struct RegistrationProbe<T>(std::marker::PhantomData<T>);

impl <T> RegistrationProbe<T> {
    pub const fn new() -> Self { Self(std::marker::PhantomData) }
}

#[doc(hidden)]
pub trait ProbeComponent { fn reflect_component(&self) -> Option<ReflectComponent>; }
impl <T: ComponentMeta + Reflect + Typed + FromValue> ProbeComponent for &RegistrationProbe<T> {
    fn reflect_component(&self) -> Option<ReflectComponent> { Some(ReflectComponent::of::<T>()) }
}

#[doc(hidden)]
pub trait ProbeNotComponent { fn reflect_component(&self) -> Option<ReflectComponent> { None } }
impl <T> ProbeNotComponent for RegistrationProbe<T> {}

#[doc(hidden)]
pub trait ProbeResource { fn reflect_resource(&self) -> Option<ReflectResource>; }
impl <R: ResourceMeta + Reflect + Typed + FromValue> ProbeResource for &RegistrationProbe<R> {
    fn reflect_resource(&self) -> Option<ReflectResource> { Some(ReflectResource::of::<R>()) }
}

#[doc(hidden)]
pub trait ProbeNotResource { fn reflect_resource(&self) -> Option<ReflectResource> { None } }
impl <R> ProbeNotResource for RegistrationProbe<R> {}

/// The reflected types known at runtime, looked up by name, [`TypeId`], [`ComponentID`]
/// or [`ResourceID`].
///
/// [`TypeRegistry::new`] holds every non generic type deriving [`Reflect`] in the program,
/// collected with `inventory`, and finds out which are components and resources.  Generic
/// types must be registered once per concrete type, e.g. `register::<Pair<f32>>()`.
///
/// Every [`World`] creates one the first time it is needed, see [`World::type_registry`].
///
/// On `wasm32-unknown-unknown` the collected types are found by constructors, which the
/// wasm linker usually runs on its own.  If types are missing, call `__wasm_call_ctors`
/// once at startup, see the `inventory` docs.
#[derive(Debug, Resource)]
pub struct TypeRegistry {
    types: Vec<TypeRegistration>,
    by_type: HashMap<TypeId, usize>,
    /// Full names without whitespace.
    by_name: HashMap<String, usize>,
    /// Short names without whitespace, `None` where two types share one.
    by_short_name: HashMap<String, Option<usize>>,
    by_component: HashMap<ComponentID, usize>,
    by_resource: HashMap<ResourceID, usize>
}

impl Default for TypeRegistry {
    fn default() -> Self { Self::new() }
}

impl TypeRegistry {
    /// A registry holding every non generic type that derives [`Reflect`].
    pub fn new() -> Self {
        let mut registry = Self::empty();
        for auto in inventory::iter::<AutoRegistration> { registry.add((auto.0)()); }
        registry
    }

    /// A registry with nothing registered, for full control over which names resolve.
    pub fn empty() -> Self {
        Self {
            types: vec![],
            by_type: HashMap::new(),
            by_name: HashMap::new(),
            by_short_name: HashMap::new(),
            by_component: HashMap::new(),
            by_resource: HashMap::new()
        }
    }

    /// Register a plain reflected type, such as one used in component fields.  Only
    /// needed to look it up by name.
    pub fn register<T: Reflect + Typed + FromValue>(&mut self) -> &mut Self {
        self.add(TypeRegistration::of::<T>())
    }

    /// Register a reflected component, so scenes and editors can build and edit it.
    pub fn register_component<T: ComponentMeta + Reflect + Typed + FromValue>(&mut self) -> &mut Self {
        self.add(TypeRegistration::of_component::<T>())
    }

    /// Register a reflected resource, so scenes and editors can build and edit it.
    pub fn register_resource<R: ResourceMeta + Reflect + Typed + FromValue>(&mut self) -> &mut Self {
        self.add(TypeRegistration::of_resource::<R>())
    }

    /// Add a registration, replacing any for the same type.
    pub fn add(&mut self, registration: TypeRegistration) -> &mut Self {
        if let Some(&index) = self.by_type.get(&registration.type_id) {
            self.types[index] = registration;
            self.index(index);
        } else {
            self.types.push(registration);
            self.index(self.types.len() - 1);
        }
        self
    }

    fn index(&mut self, index: usize) {
        let registration = &self.types[index];
        self.by_type.insert(registration.type_id, index);
        self.by_name.insert(normalize(registration.info.type_name), index);
        self.by_short_name.entry(normalize(&registration.info.short_name))
            .and_modify(|existing| if *existing != Some(index) { *existing = None })
            .or_insert(Some(index));
        if let Some(component) = &registration.component { self.by_component.insert(component.id, index); }
        if let Some(resource) = &registration.resource { self.by_resource.insert(resource.id, index); }
    }

    /// The registration for `T`.
    pub fn get<T: 'static>(&self) -> Option<&TypeRegistration> {
        self.get_by_type_id(TypeId::of::<T>())
    }

    /// The registration for the type `id`.
    pub fn get_by_type_id(&self, id: TypeId) -> Option<&TypeRegistration> {
        self.by_type.get(&id).map(|&i| &self.types[i])
    }

    /// The registration for a type by its full name, or by its short name or the end of
    /// its path (`ui::Button` for `my_game::ui::Button`) if no other registered type
    /// matches it.  Whitespace is ignored.
    pub fn get_by_name(&self, name: &str) -> Option<&TypeRegistration> {
        let name = normalize(name);
        if let Some(&i) = self.by_name.get(&name) { return Some(&self.types[i]) }
        if let Some(found) = self.by_short_name.get(&name) { return found.map(|i| &self.types[i]) }
        // a path suffix: scan, it only happens for names that aren't short or full
        let suffix = format!("::{name}");
        let mut matches = self.types.iter().filter(|r| normalize(r.info.type_name).ends_with(&suffix));
        let found = matches.next()?;
        matches.next().is_none().then_some(found)
    }

    /// The registration for `name`, as [`get_by_name`](Self::get_by_name) finds it.
    ///
    /// # Errors
    ///
    /// Errors if no registered type has that name, or if several match, naming them.
    pub fn resolve(&self, name: &str) -> anyhow::Result<&TypeRegistration> {
        if let Some(found) = self.get_by_name(name) { return Ok(found) }
        let normalized = normalize(name);
        let suffix = format!("::{normalized}");
        let candidates = self.types.iter()
            .map(|r| r.info.type_name)
            .filter(|full| { let full = normalize(full); full == normalized || full.ends_with(&suffix) })
            .collect::<Vec<_>>();
        match candidates[..] {
            [] => anyhow::bail!("no registered type `{name}`"),
            _ => anyhow::bail!("`{name}` is ambiguous, write more of its path: {}", candidates.join(", "))
        }
    }

    /// The registration for the component `id`.
    pub fn get_component(&self, id: ComponentID) -> Option<&TypeRegistration> {
        self.by_component.get(&id).map(|&i| &self.types[i])
    }

    /// The registration for the resource `id`.
    pub fn get_resource(&self, id: ResourceID) -> Option<&TypeRegistration> {
        self.by_resource.get(&id).map(|&i| &self.types[i])
    }

    /// The shortest name that finds `registration` with [`get_by_name`](Self::get_by_name):
    /// its short name if no other type shares it, otherwise the shortest unique end of
    /// its path, otherwise its full name.
    pub fn name_of<'r>(&self, registration: &'r TypeRegistration) -> &'r str {
        let full = registration.info.type_name;
        let finds = |name: &str| self.get_by_name(name).is_some_and(|found| found.type_id == registration.type_id);
        if finds(&registration.info.short_name) { return &registration.info.short_name }
        path_suffixes(full).find(|suffix| finds(suffix)).unwrap_or(full)
    }

    /// Every registration, in the order they were first added.  The order of the types
    /// collected by [`new`](Self::new) is unspecified.
    pub fn iter(&self) -> impl Iterator<Item = &TypeRegistration> {
        self.types.iter()
    }
}

/// The ends of `type_name`'s path, shortest first: `a::b::C<x::Y>` gives `C<x::Y>`,
/// `b::C<x::Y>` and `a::b::C<x::Y>`.  Only `::` outside generic arguments splits it.
fn path_suffixes(type_name: &str) -> impl Iterator<Item = &str> {
    let mut depth = 0;
    let mut starts = vec![0];
    let bytes = type_name.as_bytes();
    for i in 0..bytes.len() {
        match bytes[i] {
            b'<' | b'(' | b'[' => depth += 1,
            b'>' | b')' | b']' => depth -= 1,
            b':' if depth == 0 && bytes.get(i + 1) == Some(&b':') => starts.push(i + 2),
            _ => {}
        }
    }
    starts.into_iter().rev().map(move |start| &type_name[start..])
}

fn normalize(name: &str) -> String {
    name.chars().filter(|c| !c.is_whitespace()).collect()
}

#[cfg(test)]
mod tests {
    use super::path_suffixes;
    use crate::*;

    // the test binary holds several types called Health, so short names clash here

    #[derive(Debug, PartialEq, Component, Reflect)]
    struct Health(u32);

    #[derive(Debug, PartialEq, Resource, Reflect)]
    struct Gravity(f32);

    #[derive(Debug, PartialEq, Component, Reflect)]
    struct Both;
    impl Resource for Both {}
    impl ResourceMeta for Both {}

    #[derive(Debug, PartialEq, Reflect)]
    struct Plain;

    #[derive(Debug, PartialEq, Reflect)]
    struct Pair<T>(T, T);

    mod other {
        use crate::*;

        #[derive(Debug, PartialEq, Component, Reflect)]
        pub struct Health(pub u32);
    }

    #[test]
    fn collects_derived_types() {
        let registry = TypeRegistry::new();
        let health = registry.get::<Health>().unwrap();
        assert_eq!(health.component.unwrap().id, Health::id());
        assert!(health.resource.is_none());
        let gravity = registry.get::<Gravity>().unwrap();
        assert_eq!(gravity.resource.unwrap().id, Gravity::id());
        assert!(gravity.component.is_none());
        let both = registry.get::<Both>().unwrap();
        assert!(both.component.is_some() && both.resource.is_some());
        let plain = registry.get::<Plain>().unwrap();
        assert!(plain.component.is_none() && plain.resource.is_none());
        assert!(registry.get::<Name>().unwrap().component.is_some());
        assert!(registry.get::<Pair<f32>>().is_none(), "generic types aren't collected");
        assert_eq!(registry.get_component(Health::id()).unwrap().short_name(), "Health");
    }

    #[test]
    fn looks_up_by_name() {
        let mut registry = TypeRegistry::empty();
        registry.register_component::<Health>().register_resource::<Gravity>().register::<Pair<f32>>();

        assert_eq!(registry.get_by_name("Health").unwrap().short_name(), "Health");
        assert_eq!(registry.get_by_name(std::any::type_name::<Health>()).unwrap().short_name(), "Health");
        assert_eq!(registry.get_by_name("Pair< f32 >").unwrap().short_name(), "Pair<f32>");
        assert_eq!(registry.get_resource(Gravity::id()).unwrap().short_name(), "Gravity");
        assert!(registry.get_by_name("Missing").is_none());
        assert!(registry.resolve("Missing").unwrap_err().to_string().contains("no registered type"));
    }

    #[test]
    fn clashing_names_need_more_path() {
        let registry = TypeRegistry::new();
        assert!(registry.get_by_name("Health").is_none());
        assert!(registry.resolve("Health").unwrap_err().to_string().contains("ambiguous"));
        assert_eq!(registry.get_by_name("other::Health").unwrap().type_id, std::any::TypeId::of::<other::Health>());
        assert_eq!(registry.name_of(registry.get::<other::Health>().unwrap()), "other::Health");
        assert_eq!(registry.name_of(registry.get::<Health>().unwrap()), "registry::tests::Health");
        assert_eq!(registry.name_of(registry.get::<Plain>().unwrap()), "Plain");
    }

    #[test]
    fn registering_twice_replaces() {
        let mut registry = TypeRegistry::empty();
        registry.register_component::<Health>().register_component::<Health>();
        assert_eq!(registry.iter().count(), 1);
    }

    #[test]
    fn splits_paths_outside_generics() {
        assert_eq!(path_suffixes("a::b::C<x::Y>").collect::<Vec<_>>(), ["C<x::Y>", "b::C<x::Y>", "a::b::C<x::Y>"]);
        assert_eq!(path_suffixes("u8").collect::<Vec<_>>(), ["u8"]);
    }

    #[test]
    fn builds_and_casts_components() {
        let registry = TypeRegistry::new();
        let registration = registry.get::<Health>().unwrap();
        let component = registration.component.unwrap();
        let built = registration.build(&Value::Tuple(vec![Value::Int(5)])).unwrap();
        let mut component_value = (component.into_component)(built).unwrap();
        assert_eq!(component_value.get_id(), Health::id());
        assert_eq!((component.as_reflect)(&*component_value).unwrap().to_value(), Value::Tuple(vec![Value::Int(5)]));
        (component.as_reflect_mut)(&mut *component_value).unwrap().apply(&Value::Tuple(vec![Value::Int(6)])).unwrap();
        assert_eq!(component_value.as_any().downcast_ref::<Health>(), Some(&Health(6)));
        assert!((component.into_component)(Box::new(1u8)).is_err());
    }
}
