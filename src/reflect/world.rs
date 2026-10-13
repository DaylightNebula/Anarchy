//! Type erased reflection of a [`World`]'s components and resources, through its
//! [`TypeRegistry`].  This is the surface an editor works through.

use anyhow::{Context, bail};
use mutual::{RefCastGuard, RelaxedMutex, SharedData};

use crate::*;

/// What a [`ReflectEdit`] changed.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ReflectTarget {
    /// A component on an entity.
    Component(EntityID, ComponentID),
    /// A resource.
    Resource(ResourceID)
}

/// Sent as an [`Event`] by [`World::set_component_path`] and [`World::set_resource_path`],
/// so editors can refresh and keep an undo history.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ReflectEdit {
    /// What was edited.
    pub target: ReflectTarget,
    /// The path that was set, see [`parse_path`].
    pub path: String,
    /// The value at `path` before the edit.
    pub old: Value,
    /// The value at `path` after the edit.
    pub new: Value
}

impl World {
    /// This world's [`TypeRegistry`], created with [`TypeRegistry::new`] the first time
    /// it is needed.  Insert one yourself beforehand to choose what it holds.
    ///
    /// The guard blocks registering types, so drop it before calling `register_*`.
    pub fn type_registry(&self) -> RefCastGuard<DynResource, TypeRegistry> {
        self.init_resource_with(TypeRegistry::new);
        self.resource::<TypeRegistry>().unwrap()
    }

    /// Register a plain reflected type in this world's [`TypeRegistry`].  Only generic
    /// types need this, see [`TypeRegistry::new`].
    pub fn register_type<T: Reflect + Typed + FromValue>(&self) {
        self.init_resource_with(TypeRegistry::new);
        self.resource_mut::<TypeRegistry>().unwrap().register::<T>();
    }

    /// Register a reflected component in this world's [`TypeRegistry`].  Only generic
    /// types need this, see [`TypeRegistry::new`].
    pub fn register_component<T: ComponentMeta + Reflect + Typed + FromValue>(&self) {
        self.init_resource_with(TypeRegistry::new);
        self.resource_mut::<TypeRegistry>().unwrap().register_component::<T>();
    }

    /// Register a reflected resource in this world's [`TypeRegistry`].  Only generic
    /// types need this, see [`TypeRegistry::new`].
    pub fn register_resource<R: ResourceMeta + Reflect + Typed + FromValue>(&self) {
        self.init_resource_with(TypeRegistry::new);
        self.resource_mut::<TypeRegistry>().unwrap().register_resource::<R>();
    }

    /// The registration of the component `id`, copied out so the registry isn't held.
    fn component_registration(&self, id: ComponentID) -> anyhow::Result<(ReflectComponent, &'static TypeInfo)> {
        let registry = self.type_registry();
        let registration = registry.get_component(id).with_context(|| format!("component {id} is not registered for reflection"))?;
        Ok((registration.component.unwrap(), registration.info))
    }

    /// The registration of the resource `id`, copied out so the registry isn't held.
    fn resource_registration(&self, id: ResourceID) -> anyhow::Result<ReflectResource> {
        let registry = self.type_registry();
        let registration = registry.get_resource(id).with_context(|| format!("resource {id} is not registered for reflection"))?;
        Ok(registration.resource.unwrap())
    }

    /// The entity `entity`, which must have the component `id`.
    fn entity_with(&self, entity: EntityID, id: ComponentID) -> anyhow::Result<Entity> {
        let found = self.entity(entity).with_context(|| format!("no entity {entity}"))?;
        if found.group.binary_search(&id).is_err() { bail!("entity {entity} has no component {id}") }
        Ok(found)
    }

    /// The ids of the components on `entity`, sorted, or `None` if there is no such entity.
    pub fn component_ids(&self, entity: EntityID) -> Option<Vec<ComponentID>> {
        self.entity(entity).map(|entity| entity.group.to_vec())
    }

    /// Calls `f` with the component `id` on `entity` as `dyn Reflect`.  Only that
    /// component is locked, and the registry is not held while `f` runs.
    ///
    /// # Errors
    ///
    /// Errors if there is no such entity or component, or the component isn't registered.
    pub fn reflect_component<T>(&self, entity: EntityID, id: ComponentID, f: impl FnOnce(&dyn Reflect) -> T) -> anyhow::Result<T> {
        let (registration, _) = self.component_registration(id)?;
        let guard = self.entity_with(entity, id)?.component(id).with_context(|| format!("no entity {entity}"))?;
        let reflect = guard.as_dyn().and_then(registration.as_reflect).context("component has an unexpected type")?;
        Ok(f(reflect))
    }

    /// Calls `f` with the component `id` on `entity` as `&mut dyn Reflect`, see
    /// [`reflect_component`](Self::reflect_component).
    ///
    /// # Errors
    ///
    /// Errors if there is no such entity or component, or the component isn't registered.
    pub fn reflect_component_mut<T>(&self, entity: EntityID, id: ComponentID, f: impl FnOnce(&mut dyn Reflect) -> T) -> anyhow::Result<T> {
        let (registration, _) = self.component_registration(id)?;
        let mut guard = self.entity_with(entity, id)?.component_mut(id).with_context(|| format!("no entity {entity}"))?;
        let reflect = guard.as_dyn_mut().and_then(registration.as_reflect_mut).context("component has an unexpected type")?;
        Ok(f(reflect))
    }

    /// Calls `f` with the resource `id` as `dyn Reflect`.
    ///
    /// # Errors
    ///
    /// Errors if there is no such resource or it isn't registered.
    pub fn reflect_resource<T>(&self, id: ResourceID, f: impl FnOnce(&dyn Reflect) -> T) -> anyhow::Result<T> {
        let registration = self.resource_registration(id)?;
        let guard = self.resource_dyn(id).with_context(|| format!("no resource {id}"))?;
        let reflect = (registration.as_reflect)(&**guard).context("resource has an unexpected type")?;
        Ok(f(reflect))
    }

    /// Calls `f` with the resource `id` as `&mut dyn Reflect`.
    ///
    /// # Errors
    ///
    /// Errors if there is no such resource or it isn't registered.
    pub fn reflect_resource_mut<T>(&self, id: ResourceID, f: impl FnOnce(&mut dyn Reflect) -> T) -> anyhow::Result<T> {
        let registration = self.resource_registration(id)?;
        let mut guard = self.resource_dyn_mut(id).with_context(|| format!("no resource {id}"))?;
        let reflect = (registration.as_reflect_mut)(&mut **guard).context("resource has an unexpected type")?;
        Ok(f(reflect))
    }

    /// The value at `path` in the component `id` on `entity`, see [`get_path`].
    ///
    /// # Errors
    ///
    /// Errors as [`reflect_component`](Self::reflect_component) does, or if the path doesn't exist.
    pub fn get_component_path(&self, entity: EntityID, id: ComponentID, path: &str) -> anyhow::Result<Value> {
        self.reflect_component(entity, id, |reflect| get_path(reflect, path))?
    }

    /// Applies `value` at `path` in the component `id` on `entity`, see [`set_path`],
    /// and sends a [`ReflectEdit`] event.
    ///
    /// # Errors
    ///
    /// Errors as [`reflect_component`](Self::reflect_component) does, or if the path
    /// doesn't exist or `value` doesn't fit.
    pub fn set_component_path(&self, entity: EntityID, id: ComponentID, path: &str, value: &Value) -> anyhow::Result<()> {
        let (old, new) = self.reflect_component_mut(entity, id, |reflect| edit(reflect, path, value))??;
        self.send_event(ReflectEdit { target: ReflectTarget::Component(entity, id), path: path.into(), old, new });
        Ok(())
    }

    /// The value at `path` in the resource `id`, see [`get_path`].
    ///
    /// # Errors
    ///
    /// Errors as [`reflect_resource`](Self::reflect_resource) does, or if the path doesn't exist.
    pub fn get_resource_path(&self, id: ResourceID, path: &str) -> anyhow::Result<Value> {
        self.reflect_resource(id, |reflect| get_path(reflect, path))?
    }

    /// Applies `value` at `path` in the resource `id`, see [`set_path`], and sends a
    /// [`ReflectEdit`] event.
    ///
    /// # Errors
    ///
    /// Errors as [`reflect_resource`](Self::reflect_resource) does, or if the path
    /// doesn't exist or `value` doesn't fit.
    pub fn set_resource_path(&self, id: ResourceID, path: &str, value: &Value) -> anyhow::Result<()> {
        let (old, new) = self.reflect_resource_mut(id, |reflect| edit(reflect, path, value))??;
        self.send_event(ReflectEdit { target: ReflectTarget::Resource(id), path: path.into(), old, new });
        Ok(())
    }

    /// Adds `component` to `entity`, replacing any component of the same type.  The
    /// entity is removed and inserted again into the table for its new set of
    /// components, keeping its hierarchy links.
    ///
    /// Don't call this while holding a guard on one of the entity's components.
    ///
    /// # Errors
    ///
    /// Errors if there is no such entity.
    pub fn insert_component_dyn(&self, entity: EntityID, component: DynComponent) -> anyhow::Result<()> {
        let id = component.get_id();
        self.rebuild(entity, |components| {
            components.retain(|c| c.lock_ref().get_id() != id);
            components.push(RelaxedMutex::new(component));
        })
    }

    /// Removes the component `id` from `entity`, returning true if it had one.  Removing
    /// the last component despawns the entity, since entities need at least one.
    ///
    /// Don't call this while holding a guard on one of the entity's components.
    ///
    /// # Errors
    ///
    /// Errors if there is no such entity.
    pub fn remove_component_dyn(&self, entity: EntityID, id: ComponentID) -> anyhow::Result<bool> {
        let group = self.component_ids(entity).with_context(|| format!("no entity {entity}"))?;
        if !group.contains(&id) { return Ok(false) }
        if group.len() == 1 {
            self.remove(entity);
        } else {
            self.rebuild(entity, |components| components.retain(|c| c.lock_ref().get_id() != id))?;
        }
        Ok(true)
    }

    /// Builds the registered component `type_name` from `value` and adds it to `entity`,
    /// see [`insert_component_dyn`](Self::insert_component_dyn).
    ///
    /// # Errors
    ///
    /// Errors if the type isn't a registered component, `value` doesn't fit it, or there
    /// is no such entity.
    pub fn insert_reflected_component(&self, entity: EntityID, type_name: &str, value: &Value) -> anyhow::Result<()> {
        let component = self.build_component(type_name, value)?;
        self.insert_component_dyn(entity, component)
    }

    /// Builds the registered resource `type_name` from `value` and inserts it, replacing
    /// any resource of the same type.
    ///
    /// # Errors
    ///
    /// Errors if the type isn't a registered resource or `value` doesn't fit it.
    pub fn insert_reflected_resource(&self, type_name: &str, value: &Value) -> anyhow::Result<()> {
        let (registration, built) = {
            let registry = self.type_registry();
            let registration = registry.resolve(type_name)?;
            let resource = registration.resource.with_context(|| format!("`{type_name}` is not a resource"))?;
            (resource, registration.build(value)?)
        };
        self.insert_resource_dyn((registration.into_resource)(built)?);
        Ok(())
    }

    /// Builds the registered component `type_name` from `value`.
    pub(crate) fn build_component(&self, type_name: &str, value: &Value) -> anyhow::Result<DynComponent> {
        let registry = self.type_registry();
        let registration = registry.resolve(type_name)?;
        let component = registration.component.with_context(|| format!("`{type_name}` is not a component"))?;
        (component.into_component)(registration.build(value)?)
    }

    /// Removes `entity`, edits its components and inserts it again, keeping its hierarchy links.
    fn rebuild(&self, entity: EntityID, edit: impl FnOnce(&mut Vec<RelaxedMutex<DynComponent>>)) -> anyhow::Result<()> {
        let hierarchy = self.hierarchy();
        let parent = hierarchy.parent(entity);
        let children = hierarchy.children(entity).map(|c| c.to_vec()).unwrap_or_default();

        let mut components = self.remove(entity).with_context(|| format!("no entity {entity}"))?.into_vec();
        edit(&mut components);
        if components.is_empty() { bail!("entity {entity} would have no components") }
        self.insert(entity, components.into_boxed_slice());

        if let Some(parent) = parent { hierarchy.set_parent(entity, parent)?; }
        for child in children { hierarchy.set_parent(child, entity)?; }
        Ok(())
    }
}

/// Sets `value` at `path`, returning the old and new values there.
fn edit(reflect: &mut dyn Reflect, path: &str, value: &Value) -> anyhow::Result<(Value, Value)> {
    let target = path_mut(reflect, path)?;
    let old = target.to_value();
    target.apply(value).with_context(|| format!("setting `{path}`"))?;
    Ok((old, target.to_value()))
}

#[cfg(test)]
mod tests {
    use mutual::RelaxedMutex;

    use crate::*;

    #[derive(Debug, PartialEq, Component, Reflect)]
    struct Pos { x: f32, y: f32 }

    #[derive(Debug, PartialEq, Component, Reflect)]
    struct Health(u32);

    #[derive(Debug, Component)]
    struct Opaque;

    #[derive(Debug, PartialEq, Resource, Reflect)]
    struct Gravity { y: f32 }

    fn world() -> World {
        let world = World::default();
        // only these types, since other test modules also have types called Health and Gravity
        world.insert_resource(TypeRegistry::empty());
        world.register_component::<Pos>();
        world.register_component::<Health>();
        world.register_resource::<Gravity>();
        world.insert(1, Box::new([
            RelaxedMutex::new(Box::new(Pos { x: 1.0, y: 2.0 }) as DynComponent),
            RelaxedMutex::new(Box::new(Opaque) as DynComponent)
        ]));
        world.insert(2, Box::new([RelaxedMutex::new(Box::new(Health(3)) as DynComponent)]));
        world.insert_resource(Gravity { y: -9.8 });
        world
    }

    #[test]
    fn lists_entities_and_resources() {
        let world = world();
        let mut ids = world.entity_ids().collect::<Vec<_>>();
        ids.sort();
        assert_eq!(ids, [1, 2]);
        let mut expected = vec![Pos::id(), Opaque::id()];
        expected.sort();
        assert_eq!(world.component_ids(1).unwrap(), expected);
        assert!(world.resource_ids().contains(&Gravity::id()));
        assert!(world.resource_ids().contains(&TypeRegistry::id()));
    }

    #[test]
    fn derived_types_need_no_registering() {
        let world = World::default();
        world.insert(1, Box::new([RelaxedMutex::new(Box::new(Pos { x: 1.0, y: 2.0 }) as DynComponent)]));
        assert_eq!(world.get_component_path(1, Pos::id(), "y").unwrap(), Value::Float(2.0));
        world.insert_reflected_component(1, "world::tests::Health", &Value::Tuple(vec![Value::Int(4)])).unwrap();
        assert_eq!(*world.get::<&Health>(1).unwrap().unwrap(), Health(4));
        assert!(world.insert_reflected_component(1, "Health", &Value::Unit).unwrap_err().to_string().contains("ambiguous"));
    }

    #[test]
    fn reads_and_writes_paths() {
        let world = world();
        assert_eq!(world.get_component_path(1, Pos::id(), "y").unwrap(), Value::Float(2.0));
        world.set_component_path(1, Pos::id(), "x", &Value::Float(5.0)).unwrap();
        assert_eq!(*world.get::<&Pos>(1).unwrap().unwrap(), Pos { x: 5.0, y: 2.0 });

        world.set_resource_path(Gravity::id(), "y", &Value::Int(-1)).unwrap();
        assert_eq!(world.get_resource_path(Gravity::id(), "").unwrap(), Value::Struct(vec![("y".into(), Value::Float(-1.0))]));

        let edits = world.resource_mut::<EventQueue<ReflectEdit>>().unwrap().read(std::any::TypeId::of::<()>());
        assert_eq!(edits, [
            ReflectEdit { target: ReflectTarget::Component(1, Pos::id()), path: "x".into(), old: Value::Float(1.0), new: Value::Float(5.0) },
            ReflectEdit { target: ReflectTarget::Resource(Gravity::id()), path: "y".into(), old: Value::Float(-9.8f32 as f64), new: Value::Float(-1.0) }
        ]);
    }

    #[test]
    fn unregistered_and_missing_are_errors() {
        let world = world();
        assert!(world.get_component_path(1, Opaque::id(), "").is_err());
        assert!(world.get_component_path(1, Health::id(), "").is_err());
        assert!(world.get_component_path(3, Pos::id(), "").is_err());
        assert!(world.set_component_path(1, Pos::id(), "z", &Value::Float(1.0)).is_err());
        assert!(World::default().get_component_path(1, Pos::id(), "").is_err());
    }

    #[test]
    fn adds_and_removes_components() {
        let world = world();
        world.hierarchy().set_parent(1, 2).unwrap();

        world.insert_reflected_component(1, "Health", &Value::Tuple(vec![Value::Int(7)])).unwrap();
        assert_eq!(*world.get::<&Health>(1).unwrap().unwrap(), Health(7));
        world.insert_reflected_component(1, "Health", &Value::Tuple(vec![Value::Int(8)])).unwrap();
        assert_eq!(world.component_ids(1).unwrap().len(), 3);
        assert_eq!(world.hierarchy().parent(1), Some(2));

        assert!(world.remove_component_dyn(1, Health::id()).unwrap());
        assert!(!world.remove_component_dyn(1, Health::id()).unwrap());
        assert_eq!(world.component_ids(1).unwrap().len(), 2);
        assert_eq!(world.hierarchy().children(2).unwrap().to_vec(), [1]);

        assert!(world.remove_component_dyn(2, Health::id()).unwrap());
        assert!(world.entity(2).is_none());
        assert!(world.insert_reflected_component(1, "Gravity", &Value::Unit).is_err());
    }

    #[test]
    fn inserts_reflected_resources() {
        let world = world();
        world.insert_reflected_resource("Gravity", &Value::Struct(vec![("y".into(), Value::Float(3.0))])).unwrap();
        assert_eq!(*world.resource::<Gravity>().unwrap(), Gravity { y: 3.0 });
        assert!(world.insert_reflected_resource("Pos", &Value::Unit).is_err());
    }
}
