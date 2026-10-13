//! Saving entities from a [`World`] as a [`Scene`], see [`Scene::from_world`].

use std::collections::{HashMap, HashSet};

use anyhow::Context;

use crate::*;

/// A scene saved from a world, see [`Scene::from_world`].
#[derive(Debug, Clone, Default)]
pub struct SavedScene {
    /// The saved entities.
    pub scene: Scene,
    /// Components left out because their types aren't registered, by entity.
    pub skipped: Vec<(EntityID, ComponentID)>
}

impl Scene {
    /// Saves `roots` and all their descendants from `world` as a scene.  Each
    /// registered component is saved as a patch holding only what differs from its
    /// type's [`fallback`](FromValue::fallback), so spawning the scene rebuilds the same
    /// values.  Components that aren't registered are left out and listed in
    /// [`SavedScene::skipped`].
    ///
    /// A [`Name`] component becomes the entity's `#name` when no other saved entity has
    /// the same name.  References to saved entities from `#[reflect(entity)]` fields are
    /// written by name, and entities referenced without a usable name are named
    /// `e<id>`, as are entities with nothing to save.  References to entities outside
    /// the saved set keep their ids.
    ///
    /// # Errors
    ///
    /// Errors if a root doesn't exist.
    pub fn from_world(world: &World, roots: &[EntityID]) -> anyhow::Result<SavedScene> {
        let registry = world.type_registry();
        let hierarchy = world.hierarchy();

        // every saved entity, each parent before its children
        let mut order = vec![];
        for &root in roots {
            world.entity(root).with_context(|| format!("no entity {root}"))?;
            order.push(root);
            order.extend(hierarchy.descendants(root).filter(|id| world.entity(*id).is_some()));
        }

        let mut saved = SavedScene::default();
        let mut entities = HashMap::new();
        let mut names = HashMap::new();
        for &id in &order {
            let entity = world.entity(id).unwrap();
            let mut scene_entity = SceneEntity::default();
            for component_id in entity.group.iter() {
                let Some(registration) = registry.get_component(*component_id) else {
                    saved.skipped.push((id, *component_id));
                    continue;
                };
                let reflect = registration.component.unwrap();
                let Some(guard) = entity.component(*component_id) else { continue };
                let Some(component) = guard.as_dyn().and_then(reflect.as_reflect) else { continue };
                let value = component.to_value();
                let name = component.reflect_any().downcast_ref::<Name>().map(|name| name.0.clone());
                drop(guard);
                let value = match (registration.fallback)() {
                    Some(base) => value.diff(&base.to_value()),
                    None => value
                };
                let patch = ScenePatch::Reflected { type_name: registry.name_of(registration).to_string(), value };
                // held back until we know whether the name can be the entity's #name
                match name {
                    Some(name) => { names.insert(id, (name, patch)); }
                    None => scene_entity.components.push(patch)
                }
            }
            entities.insert(id, scene_entity);
        }

        // names that are unique among the saved entities become #names, others stay components
        let mut seen = HashSet::new();
        let repeated = names.values().filter(|(name, _)| !seen.insert(name.as_str())).map(|(name, _)| name.clone()).collect::<HashSet<_>>();
        let mut scene_names = HashMap::new();
        for (id, (name, patch)) in names {
            let entity = entities.get_mut(&id).unwrap();
            if name.is_empty() || repeated.contains(&name) {
                entity.components.push(patch);
            } else {
                entity.name = Some(name.clone());
                scene_names.insert(id, name);
            }
        }

        // name the saved entities other entities refer to
        let saved_ids = order.iter().copied().collect::<HashSet<_>>();
        let mut referenced = HashSet::new();
        for entity in entities.values() {
            for patch in &entity.components {
                if let ScenePatch::Reflected { value, .. } = patch { collect_refs(value, &saved_ids, &mut referenced); }
            }
        }
        // entities with nothing saved are named too, so they still spawn and keep the tree
        let unnamed_empty = order.iter().filter(|id| {
            let entity = &entities[*id];
            entity.name.is_none() && entity.components.is_empty()
        });
        let mut taken = scene_names.values().cloned().collect::<HashSet<_>>();
        for id in referenced.iter().chain(unnamed_empty).collect::<Vec<_>>() {
            if scene_names.contains_key(id) { continue }
            let mut name = format!("e{id}");
            while taken.contains(&name) { name.push('_') }
            taken.insert(name.clone());
            entities.get_mut(id).unwrap().name = Some(name.clone());
            scene_names.insert(*id, name);
        }
        for entity in entities.values_mut() {
            for patch in &mut entity.components {
                if let ScenePatch::Reflected { value, .. } = patch { name_refs(value, &scene_names); }
            }
        }

        // build the tree, children in hierarchy order
        fn take(id: EntityID, entities: &mut HashMap<EntityID, SceneEntity>, hierarchy: &Hierarchy) -> SceneEntity {
            let mut entity = entities.remove(&id).unwrap_or_default();
            let children = hierarchy.children(id).map(|children| children.to_vec()).unwrap_or_default();
            for child in children {
                if entities.contains_key(&child) { entity.children.push(take(child, entities, hierarchy)); }
            }
            entity
        }
        for &root in roots {
            if entities.contains_key(&root) { saved.scene.roots.push(take(root, &mut entities, hierarchy)); }
        }
        Ok(saved)
    }
}

/// Adds the saved entities `value` refers to by id to `found`.
fn collect_refs(value: &Value, saved: &HashSet<EntityID>, found: &mut HashSet<EntityID>) {
    match value {
        Value::Entity(EntityRef::Id(id)) if saved.contains(id) => { found.insert(*id); }
        Value::List(items) | Value::Tuple(items) => items.iter().for_each(|item| collect_refs(item, saved, found)),
        Value::Struct(fields) => fields.iter().for_each(|(_, value)| collect_refs(value, saved, found)),
        Value::Enum { value, .. } => collect_refs(value, saved, found),
        _ => {}
    }
}

/// Replaces references by id with references by name where `names` has one.
fn name_refs(value: &mut Value, names: &HashMap<EntityID, String>) {
    match value {
        Value::Entity(EntityRef::Id(id)) => if let Some(name) = names.get(id) { *value = Value::Entity(EntityRef::Named(name.clone())) },
        Value::List(items) | Value::Tuple(items) => items.iter_mut().for_each(|item| name_refs(item, names)),
        Value::Struct(fields) => fields.iter_mut().for_each(|(_, value)| name_refs(value, names)),
        Value::Enum { value, .. } => name_refs(value, names),
        _ => {}
    }
}
