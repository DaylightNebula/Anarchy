//! Spawning a [`Scene`] into a [`World`].

use std::collections::HashMap;

use anyhow::{Context, bail};
use mutual::RelaxedMutex;

use crate::*;

/// The entities a [`Scene`] spawned, see [`Scene::spawn`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SceneInstance {
    /// The ids of the scene's root entities, in order.
    pub roots: Vec<EntityID>,
    /// The ids of every entity spawned, each parent before its children.
    pub entities: Vec<EntityID>,
    /// Named entities by name.  Where names repeat, the first entity spawned keeps it.
    pub by_name: HashMap<String, EntityID>
}

impl SceneInstance {
    /// Removes every entity this instance spawned from `world`.
    pub fn despawn(&self, world: &World) {
        for id in &self.entities { world.remove(*id); }
    }
}

/// A [`SceneEntity`] with its inherited scenes merged in.
struct Expanded {
    name: Option<String>,
    components: Vec<ScenePatch>,
    children: Vec<Expanded>
}

impl Scene {
    /// Spawns this scene into `world`, calling `alloc_id` for each new entity's id.
    /// Resources are inserted or patched first, then every entity is built, and only
    /// once all have built are they inserted and linked to their parents, so a scene
    /// that fails to build leaves the world's entities untouched.
    ///
    /// Reflected patches are built through the world's [`TypeRegistry`], see
    /// [`World::type_registry`], and inherited scenes come from its [`SceneLibrary`].  Entity references like `#Enemy` resolve to the first entity
    /// of that name in this scene.
    ///
    /// # Errors
    ///
    /// Errors if a type isn't registered or its name is ambiguous, a patch doesn't fit its type, an entity
    /// reference or inherited scene is missing, scenes inherit in a cycle, or an entity
    /// ends up with no components (give it a `#name` or a component).
    pub fn spawn(&self, world: &World, mut alloc_id: impl FnMut() -> EntityID) -> anyhow::Result<SceneInstance> {
        let roots = self.roots.iter()
            .map(|root| expand(world, root, &mut vec![]))
            .collect::<anyhow::Result<Vec<_>>>()?;

        // pass one: ids and names, so references can point anywhere in the scene
        let mut instance = SceneInstance::default();
        let mut parents = vec![];
        for root in &roots {
            let id = allocate(root, None, &mut alloc_id, &mut instance, &mut parents);
            instance.roots.push(id);
        }

        let registry = world.type_registry();

        // pass two: build every entity before touching the world
        let mut built = vec![];
        let mut ids = instance.entities.iter();
        let mut stack = roots.iter().rev().collect::<Vec<_>>();
        while let Some(entity) = stack.pop() {
            let id = *ids.next().unwrap();
            let components = build_entity(entity, &instance.by_name, &registry)
                .with_context(|| format!("building entity {}", entity.name.as_deref().map(|n| format!("#{n}")).unwrap_or(id.to_string())))?;
            built.push((id, components));
            stack.extend(entity.children.iter().rev());
        }

        let resources = self.resources.iter()
            .map(|patch| prepare_resource(patch, &instance.by_name, &registry))
            .collect::<anyhow::Result<Vec<_>>>()?;
        drop(registry);
        for resource in resources { resource(world)?; }

        for (id, components) in built {
            world.insert(id, components.into_iter().map(RelaxedMutex::new).collect());
        }
        for (child, parent) in parents { world.hierarchy().set_parent(child, parent)?; }
        Ok(instance)
    }
}

/// Merges the scenes `entity` inherits into it.  `chain` holds the scenes being
/// expanded, to catch cycles.
fn expand(world: &World, entity: &SceneEntity, chain: &mut Vec<String>) -> anyhow::Result<Expanded> {
    let mut expanded = match &entity.inherits {
        Some(base) => {
            if chain.contains(base) { bail!("scenes inherit in a cycle: {} -> {base}", chain.join(" -> ")) }
            let scene = world.resource::<SceneLibrary>().ok()
                .and_then(|library| library.get(base).cloned())
                .with_context(|| format!("no scene `{base}` in the SceneLibrary"))?;
            let [root] = &scene.roots[..] else { bail!("inherited scene `{base}` must have exactly one root, it has {}", scene.roots.len()) };
            chain.push(base.clone());
            let expanded = expand(world, root, chain).with_context(|| format!("in inherited scene `{base}`"))?;
            chain.pop();
            expanded
        }
        None => Expanded { name: None, components: vec![], children: vec![] }
    };
    if entity.name.is_some() { expanded.name = entity.name.clone(); }
    expanded.components.extend(entity.components.iter().cloned());
    for child in &entity.children { expanded.children.push(expand(world, child, chain)?); }
    Ok(expanded)
}

/// Gives `entity` and its descendants ids, depth first, recording names and parents.
fn allocate(
    entity: &Expanded,
    parent: Option<EntityID>,
    alloc_id: &mut impl FnMut() -> EntityID,
    instance: &mut SceneInstance,
    parents: &mut Vec<(EntityID, EntityID)>
) -> EntityID {
    let id = alloc_id();
    instance.entities.push(id);
    if let Some(name) = &entity.name { instance.by_name.entry(name.clone()).or_insert(id); }
    if let Some(parent) = parent { parents.push((id, parent)); }
    for child in &entity.children { allocate(child, Some(id), alloc_id, instance, parents); }
    id
}

/// Builds the components of `entity`, merging patches of the same type in order.
fn build_entity(
    entity: &Expanded,
    names: &HashMap<String, EntityID>,
    registry: &TypeRegistry
) -> anyhow::Result<Vec<DynComponent>> {
    let mut components: Vec<DynComponent> = vec![];
    for patch in &entity.components {
        match patch {
            ScenePatch::Typed { id, make, .. } => {
                let component = make();
                match components.iter_mut().find(|c| c.get_id() == *id) {
                    Some(existing) => *existing = component,
                    None => components.push(component)
                }
            }
            ScenePatch::Reflected { type_name, value } => {
                let registration = registry.resolve(type_name)?;
                let reflect = registration.component.with_context(|| format!("`{type_name}` is not a component"))?;
                let value = resolve_entities(value, names)?;
                match components.iter_mut().find(|c| c.get_id() == reflect.id) {
                    Some(existing) => (reflect.as_reflect_mut)(&mut **existing).context("component has an unexpected type")?
                        .apply(&value).with_context(|| format!("applying {type_name}"))?,
                    None => components.push((reflect.into_component)(registration.build(&value)?)?)
                }
            }
        }
    }
    if let Some(name) = &entity.name && !components.iter().any(|c| c.get_id() == Name::id()) {
        components.push(Box::new(Name::new(name)));
    }
    if components.is_empty() { bail!("the entity has no components, give it a #name or a component") }
    Ok(components)
}

/// Checks a resource patch and returns what inserts or patches it.
fn prepare_resource(
    patch: &ResourcePatch,
    names: &HashMap<String, EntityID>,
    registry: &TypeRegistry
) -> anyhow::Result<Box<dyn FnOnce(&World) -> anyhow::Result<()>>> {
    match patch {
        ResourcePatch::Typed { insert, .. } => {
            let insert = insert.clone();
            Ok(Box::new(move |world| { insert(world); Ok(()) }))
        }
        ResourcePatch::Reflected { type_name, value } => {
            let registration = registry.resolve(type_name)?;
            let reflect = registration.resource.with_context(|| format!("`{type_name}` is not a resource"))?;
            let value = resolve_entities(value, names)?;
            let from_value = registration.from_value;
            let type_name = type_name.clone();
            Ok(Box::new(move |world| {
                if let Some(mut existing) = world.resource_dyn_mut(reflect.id) {
                    return (reflect.as_reflect_mut)(&mut **existing).context("resource has an unexpected type")?
                        .apply(&value).with_context(|| format!("applying {type_name}"));
                }
                let built = from_value(&value).with_context(|| format!("building {type_name}"))?;
                world.insert_resource_dyn((reflect.into_resource)(built)?);
                Ok(())
            }))
        }
    }
}

/// `value` with every [`EntityRef::Named`] replaced by the id of that entity.
fn resolve_entities(value: &Value, names: &HashMap<String, EntityID>) -> anyhow::Result<Value> {
    Ok(match value {
        Value::Entity(EntityRef::Named(name)) => Value::Entity(EntityRef::Id(*names.get(name).with_context(|| format!("no entity #{name} in the scene"))?)),
        Value::List(items) => Value::List(items.iter().map(|item| resolve_entities(item, names)).collect::<anyhow::Result<_>>()?),
        Value::Tuple(items) => Value::Tuple(items.iter().map(|item| resolve_entities(item, names)).collect::<anyhow::Result<_>>()?),
        Value::Struct(fields) => Value::Struct(fields.iter()
            .map(|(name, value)| Ok((name.clone(), resolve_entities(value, names)?)))
            .collect::<anyhow::Result<_>>()?),
        Value::Enum { variant, value } => Value::Enum { variant: variant.clone(), value: Box::new(resolve_entities(value, names)?) },
        other => other.clone()
    })
}
