//! A socket other programs use to inspect and edit a [`World`] while it runs, with the
//! `socket` feature.  The `anarchy` command line tool and editors talk to it.
//!
//! The protocol is one JSON [`Request`] per line, each answered by one JSON [`Response`]
//! line.  Types are named as in scenes (short names, path ends or full paths), and
//! values are [`Value`]s in their serde form, such as `{"Int":55}`.
//!
//! - [`World::handle_request`] answers a request, for any transport.
//! - [`WorldSocket`] serves a world over TCP or, on unix, a unix socket (native only).
//! - [`SocketClient`] connects to one (native only).
//!
//! Requests run on the socket's threads as they arrive, alongside any running systems,
//! with the same per component and per resource locking.

use anyhow::{Context, bail};
use mutual::SharedData;
use serde::{Deserialize, Serialize};

use crate::*;

#[cfg(not(target_arch = "wasm32"))]
pub mod client;
#[cfg(not(target_arch = "wasm32"))]
pub mod server;

#[cfg(not(target_arch = "wasm32"))]
pub use client::*;
#[cfg(not(target_arch = "wasm32"))]
pub use server::*;

/// A component on an entity, or a resource, named by its registered type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    /// The component `type_name` on `entity`.
    Component {
        /// The entity holding the component.
        entity: EntityID,
        /// The component's registered type name.
        type_name: String
    },
    /// The resource `type_name`.
    Resource {
        /// The resource's registered type name.
        type_name: String
    }
}

/// Something a client asks a world to do, answered with a [`Response`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Every entity, answered with [`Response::Entities`].
    Entities,
    /// One entity and its components' values, answered with [`Response::Entity`].
    Entity {
        /// The entity.
        entity: EntityID
    },
    /// Every resource and its value, answered with [`Response::Resources`].
    Resources,
    /// Every registered type, answered with [`Response::Types`].
    Types,
    /// A registered type's [`TypeInfo`], answered with [`Response::TypeInfo`].
    TypeInfo {
        /// The type's name.
        type_name: String
    },
    /// The value at `path` in `target`, answered with [`Response::Value`].
    Get {
        /// The component or resource.
        target: Target,
        /// The path, see [`parse_path`].  Empty for the whole value.
        #[serde(default)]
        path: String
    },
    /// Applies `value` at `path` in `target` and sends a [`ReflectEdit`] event.
    Set {
        /// The component or resource.
        target: Target,
        /// The path, see [`parse_path`].  Empty for the whole value.
        #[serde(default)]
        path: String,
        /// The value, a patch where it is a struct.
        value: Value
    },
    /// Builds `target` from `value` and adds it, replacing any of the same type.
    Insert {
        /// The component or resource.
        target: Target,
        /// The value, missing fields take their fallbacks.
        value: Value
    },
    /// Removes `target`.  Removing an entity's last component despawns it.
    Remove {
        /// The component or resource.
        target: Target
    },
    /// Spawns a text scene, see [`Scene::parse`], answered with [`Response::Spawned`].
    /// New entities get ids above the highest id in the world.
    Spawn {
        /// The scene text.
        scene: String
    },
    /// Removes an entity, and every entity below it if `recursive`.
    Despawn {
        /// The entity.
        entity: EntityID,
        /// Also removes its descendants, otherwise its children become roots.
        #[serde(default)]
        recursive: bool
    },
    /// Sets or, with `None`, removes an entity's parent.
    SetParent {
        /// The entity to move.
        child: EntityID,
        /// Its new parent.
        parent: Option<EntityID>
    },
    /// Saves entities as scene text, see [`Scene::from_world`], answered with
    /// [`Response::Scene`].
    Save {
        /// The entities to save with their descendants.  Empty saves every root entity.
        #[serde(default)]
        roots: Vec<EntityID>
    }
}

/// A world's answer to a [`Request`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Response {
    /// The request succeeded and has nothing to return.
    Done,
    /// The request failed.
    Error(String),
    /// A value, from [`Request::Get`].
    Value(Value),
    /// From [`Request::Entities`], sorted by id.
    Entities(Vec<EntitySummary>),
    /// From [`Request::Entity`].
    Entity(EntityDetails),
    /// From [`Request::Resources`], sorted by name.
    Resources(Vec<Reflected>),
    /// From [`Request::Types`], sorted by name.
    Types(Vec<TypeSummary>),
    /// A [`TypeInfo`] as JSON, from [`Request::TypeInfo`].
    TypeInfo(serde_json::Value),
    /// The ids of the root entities spawned, from [`Request::Spawn`].
    Spawned(Vec<EntityID>),
    /// Scene text, from [`Request::Save`].
    Scene(String)
}

/// An entity in [`Response::Entities`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntitySummary {
    /// The entity's id.
    pub id: EntityID,
    /// Its [`Name`], if it has one.
    pub name: Option<String>,
    /// Its parent.
    pub parent: Option<EntityID>,
    /// Its components' type names, see [`Reflected::name`].
    pub components: Vec<String>
}

/// An entity in [`Response::Entity`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityDetails {
    /// The entity's id.
    pub id: EntityID,
    /// Its [`Name`], if it has one.
    pub name: Option<String>,
    /// Its parent.
    pub parent: Option<EntityID>,
    /// Its children.
    pub children: Vec<EntityID>,
    /// Its components, sorted by name.
    pub components: Vec<Reflected>
}

/// A component or resource and its value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reflected {
    /// The shortest name its type is found by, see [`TypeRegistry::name_of`], or its
    /// `Debug` form when it isn't registered.
    pub name: String,
    /// Its component or resource id.
    pub id: u64,
    /// Its value, `None` when its type isn't registered.
    pub value: Option<Value>
}

/// A registered type in [`Response::Types`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeSummary {
    /// The shortest name it is found by, see [`TypeRegistry::name_of`].
    pub name: String,
    /// Its full name.
    pub type_name: String,
    /// True if it is a component.
    pub component: bool,
    /// True if it is a resource.
    pub resource: bool
}

impl World {
    /// Answers `request`, see [`Request`].  Failures become [`Response::Error`].
    pub fn handle_request(&self, request: Request) -> Response {
        self.try_handle_request(request).unwrap_or_else(|e| Response::Error(format!("{e:#}")))
    }

    fn try_handle_request(&self, request: Request) -> anyhow::Result<Response> {
        Ok(match request {
            Request::Entities => {
                let mut ids = self.entity_ids().collect::<Vec<_>>();
                ids.sort();
                let summaries = ids.into_iter().filter_map(|id| {
                    let group = self.component_ids(id)?;
                    let components = group.iter().map(|c| self.component_name(id, *c)).collect();
                    Some(EntitySummary { id, name: self.entity_name(id), parent: self.hierarchy().parent(id), components })
                });
                Response::Entities(summaries.collect())
            },
            Request::Entity { entity } => {
                let group = self.component_ids(entity).with_context(|| format!("no entity {entity}"))?;
                let mut components = group.iter().map(|&id| Reflected {
                    name: self.component_name(entity, id),
                    id,
                    value: self.reflect_component(entity, id, |c| c.to_value()).ok()
                }).collect::<Vec<_>>();
                components.sort_by(|a, b| a.name.cmp(&b.name));
                let children = self.hierarchy().children(entity).map(|c| c.to_vec()).unwrap_or_default();
                Response::Entity(EntityDetails { id: entity, name: self.entity_name(entity), parent: self.hierarchy().parent(entity), children, components })
            },
            Request::Resources => {
                let mut resources = self.resource_ids().into_iter().map(|id| {
                    let name = {
                        let registry = self.type_registry();
                        registry.get_resource(id).map(|r| registry.name_of(r).to_string())
                    };
                    let value = name.is_some().then(|| self.reflect_resource(id, |r| r.to_value()).ok()).flatten();
                    // the registry guard is dropped above, so the debug form can lock any resource, even the registry
                    let name = name.unwrap_or_else(|| self.resource_dyn(id).map(|r| debug_name(&format!("{:?}", &**r))).unwrap_or_default());
                    Reflected { name, id, value }
                }).collect::<Vec<_>>();
                resources.sort_by(|a, b| a.name.cmp(&b.name));
                Response::Resources(resources)
            },
            Request::Types => {
                let registry = self.type_registry();
                let mut types = registry.iter().map(|r| TypeSummary {
                    name: registry.name_of(r).to_string(),
                    type_name: r.info.type_name.to_string(),
                    component: r.component.is_some(),
                    resource: r.resource.is_some()
                }).collect::<Vec<_>>();
                types.sort_by(|a, b| a.name.cmp(&b.name));
                Response::Types(types)
            },
            Request::TypeInfo { type_name } => {
                let info = self.type_registry().resolve(&type_name)?.info;
                Response::TypeInfo(serde_json::to_value(info)?)
            },
            Request::Get { target, path } => Response::Value(match target {
                Target::Component { entity, type_name } => self.get_component_path(entity, self.component_id(&type_name)?, &path)?,
                Target::Resource { type_name } => self.get_resource_path(self.resource_id(&type_name)?, &path)?
            }),
            Request::Set { target, path, value } => {
                match target {
                    Target::Component { entity, type_name } => self.set_component_path(entity, self.component_id(&type_name)?, &path, &value)?,
                    Target::Resource { type_name } => self.set_resource_path(self.resource_id(&type_name)?, &path, &value)?
                }
                Response::Done
            },
            Request::Insert { target, value } => {
                match target {
                    Target::Component { entity, type_name } => self.insert_reflected_component(entity, &type_name, &value)?,
                    Target::Resource { type_name } => self.insert_reflected_resource(&type_name, &value)?
                }
                Response::Done
            },
            Request::Remove { target } => {
                let removed = match target {
                    Target::Component { entity, type_name } => self.remove_component_dyn(entity, self.component_id(&type_name)?)?,
                    Target::Resource { type_name } => self.remove_resource_dyn(self.resource_id(&type_name)?)
                };
                if !removed { bail!("nothing to remove") }
                Response::Done
            },
            Request::Spawn { scene } => {
                let scene = Scene::parse(&scene)?;
                let mut next = self.entity_ids().max().map_or(0, |id| id + 1);
                let instance = scene.spawn(self, || { next += 1; next - 1 })?;
                Response::Spawned(instance.roots)
            },
            Request::Despawn { entity, recursive } => {
                let handle = self.handle(entity);
                if handle.entity().is_none() { bail!("no entity {entity}") }
                if recursive { handle.despawn_recursive(); } else { handle.despawn(); }
                Response::Done
            },
            Request::SetParent { child, parent } => {
                if self.entity(child).is_none() { bail!("no entity {child}") }
                match parent {
                    Some(parent) => {
                        if self.entity(parent).is_none() { bail!("no entity {parent}") }
                        self.hierarchy().set_parent(child, parent)?;
                    },
                    None => { self.hierarchy().remove_parent(child); }
                }
                Response::Done
            },
            Request::Save { roots } => {
                let roots = if roots.is_empty() {
                    let mut roots = self.entity_ids().filter(|id| self.hierarchy().parent(*id).is_none()).collect::<Vec<_>>();
                    roots.sort();
                    roots
                } else { roots };
                Response::Scene(Scene::from_world(self, &roots)?.scene.to_text()?)
            }
        })
    }

    /// The id of the registered component `type_name`.
    fn component_id(&self, type_name: &str) -> anyhow::Result<ComponentID> {
        let registry = self.type_registry();
        let registration = registry.resolve(type_name)?;
        Ok(registration.component.with_context(|| format!("`{type_name}` is not a component"))?.id)
    }

    /// The id of the registered resource `type_name`.
    fn resource_id(&self, type_name: &str) -> anyhow::Result<ResourceID> {
        let registry = self.type_registry();
        let registration = registry.resolve(type_name)?;
        Ok(registration.resource.with_context(|| format!("`{type_name}` is not a resource"))?.id)
    }

    /// The registered name of the component `id`, or the `Debug` form of the one on `entity`.
    fn component_name(&self, entity: EntityID, id: ComponentID) -> String {
        {
            let registry = self.type_registry();
            if let Some(registration) = registry.get_component(id) { return registry.name_of(registration).to_string() }
        }
        let Some(found) = self.entity(entity) else { return String::new() };
        let Ok(index) = found.group.binary_search(&id) else { return String::new() };
        debug_name(&format!("{:?}", &**found.components[index].lock_ref()))
    }

    /// The [`Name`] of `entity`.
    fn entity_name(&self, entity: EntityID) -> Option<String> {
        self.get::<&Name>(entity).ok().flatten().map(|name| name.0.clone())
    }
}

/// The type name at the start of a `Debug` form, `Foo { x: 1 }` gives `Foo`.
fn debug_name(debug: &str) -> String {
    debug.split([' ', '(', '{']).next().unwrap_or(debug).to_string()
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[derive(Debug, Clone, PartialEq, Component, Reflect)]
    struct Hp(u32);

    #[derive(Debug, Clone, Default, PartialEq, Component, Reflect)]
    struct Spot { x: f32, y: f32 }

    #[derive(Debug, Component)]
    struct Opaque;

    #[derive(Debug, Clone, Default, PartialEq, Resource, Reflect)]
    struct Wind { speed: f32 }

    fn world() -> World {
        let world = World::default();
        // only these types, so short names don't clash with other test modules
        world.insert_resource(TypeRegistry::empty());
        world.register_component::<Hp>();
        world.register_component::<Spot>();
        world.register_component::<Name>();
        world.register_resource::<Wind>();
        world.insert_resource(Wind { speed: 2.0 });
        Scene::parse("#Root Hp(5) [ #Leaf Spot { x: 1.0 } ],").unwrap().spawn(&world, { let mut next = 0; move || { next += 1; next } }).unwrap();
        world
    }

    fn component(entity: EntityID, type_name: &str) -> Target {
        Target::Component { entity, type_name: type_name.into() }
    }

    #[test]
    fn lists_entities_and_resources() {
        let world = world();
        world.insert_component_dyn(1, Box::new(Opaque)).unwrap();
        let Response::Entities(entities) = world.handle_request(Request::Entities) else { panic!() };
        assert_eq!(entities.iter().map(|e| (e.id, e.name.as_deref(), e.parent)).collect::<Vec<_>>(), [(1, Some("Root"), None), (2, Some("Leaf"), Some(1))]);
        assert!(entities[0].components.contains(&"Opaque".to_string()));

        let Response::Entity(root) = world.handle_request(Request::Entity { entity: 1 }) else { panic!() };
        assert_eq!(root.children, [2]);
        let hp = root.components.iter().find(|c| c.name == "Hp").unwrap();
        assert_eq!(hp.value, Some(Value::Tuple(vec![Value::Int(5)])));
        assert_eq!(root.components.iter().find(|c| c.name == "Opaque").unwrap().value, None);

        let Response::Resources(resources) = world.handle_request(Request::Resources) else { panic!() };
        let wind = resources.iter().find(|r| r.name == "Wind").unwrap();
        assert_eq!(wind.value, Some(Value::Struct(vec![("speed".into(), Value::Float(2.0))])));
        assert!(resources.iter().any(|r| r.name == "TypeRegistry" && r.value.is_none()));

        let Response::Types(types) = world.handle_request(Request::Types) else { panic!() };
        assert!(types.iter().any(|t| t.name == "Wind" && t.resource && !t.component));
        assert!(matches!(world.handle_request(Request::TypeInfo { type_name: "Spot".into() }), Response::TypeInfo(_)));
    }

    #[test]
    fn edits_values() {
        let world = world();
        let set = Request::Set { target: component(2, "Spot"), path: "y".into(), value: Value::Float(3.0) };
        assert_eq!(world.handle_request(set), Response::Done);
        assert_eq!(world.handle_request(Request::Get { target: component(2, "Spot"), path: "".into() }), Response::Value(parse_value("{ x: 1.0, y: 3.0 }").unwrap()));

        let wind = Target::Resource { type_name: "Wind".into() };
        world.handle_request(Request::Set { target: wind.clone(), path: "speed".into(), value: Value::Int(4) });
        assert_eq!(world.resource::<Wind>().unwrap().speed, 4.0);

        assert_eq!(world.handle_request(Request::Insert { target: component(2, "Hp"), value: Value::Tuple(vec![Value::Int(9)]) }), Response::Done);
        assert_eq!(*world.get::<&Hp>(2).unwrap().unwrap(), Hp(9));
        assert_eq!(world.handle_request(Request::Remove { target: component(2, "Hp") }), Response::Done);
        assert!(matches!(world.handle_request(Request::Remove { target: component(2, "Hp") }), Response::Error(_)));
        assert_eq!(world.handle_request(Request::Remove { target: wind }), Response::Done);
        assert!(!world.has_resource::<Wind>());

        assert!(matches!(world.handle_request(Request::Get { target: component(9, "Spot"), path: "".into() }), Response::Error(_)));
        assert!(matches!(world.handle_request(Request::Get { target: component(1, "Wind"), path: "".into() }), Response::Error(e) if e.contains("not a component")));
    }

    #[test]
    fn spawns_saves_and_despawns() {
        let world = world();
        let Response::Spawned(roots) = world.handle_request(Request::Spawn { scene: "Hp(1), Hp(2),".into() }) else { panic!() };
        assert_eq!(roots, [3, 4]);

        assert_eq!(world.handle_request(Request::SetParent { child: 4, parent: Some(3) }), Response::Done);
        assert_eq!(world.hierarchy().parent(4), Some(3));
        assert_eq!(world.handle_request(Request::SetParent { child: 4, parent: None }), Response::Done);
        assert_eq!(world.hierarchy().parent(4), None);

        let Response::Scene(text) = world.handle_request(Request::Save { roots: vec![1] }) else { panic!() };
        assert!(text.contains("#Root") && text.contains("#Leaf") && !text.contains("Hp(1)"));
        let Response::Scene(text) = world.handle_request(Request::Save { roots: vec![] }) else { panic!() };
        assert!(text.contains("#Root") && text.contains("Hp(2)"));

        assert_eq!(world.handle_request(Request::Despawn { entity: 1, recursive: true }), Response::Done);
        assert!(world.entity(1).is_none() && world.entity(2).is_none());
        assert!(matches!(world.handle_request(Request::Despawn { entity: 1, recursive: false }), Response::Error(_)));
    }

    #[test]
    fn requests_round_trip_as_json() {
        let request: Request = serde_json::from_str(r#"{"op":"set","target":{"kind":"component","entity":1,"type_name":"Hp"},"value":{"Tuple":[{"Int":3}]}}"#).unwrap();
        assert_eq!(request, Request::Set { target: component(1, "Hp"), path: "".into(), value: Value::Tuple(vec![Value::Int(3)]) });
        let response = Response::Value(Value::Int(3));
        assert_eq!(serde_json::from_str::<Response>(&serde_json::to_string(&response).unwrap()).unwrap(), response);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn talk(address: &str) {
        let world = world();
        let socket = WorldSocket::bind(&world, address).unwrap();
        let mut client = SocketClient::connect(&socket.address().to_string()).unwrap();
        client.request(Request::Set { target: component(1, "Hp"), path: "0".into(), value: Value::Int(7) }).unwrap();
        assert_eq!(*world.get::<&Hp>(1).unwrap().unwrap(), Hp(7));
        assert!(client.request(Request::Entity { entity: 99 }).is_err());
        // the connection survives errors
        assert!(matches!(client.request(Request::Entities).unwrap(), Response::Entities(e) if e.len() == 2));
        drop(socket);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn serves_over_tcp() { talk("127.0.0.1:0") }

    #[cfg(unix)]
    #[test]
    fn serves_over_unix_sockets() {
        let path = std::env::temp_dir().join(format!("anarchy-test-{}.sock", std::process::id()));
        talk(&format!("unix:{}", path.display()));
        assert!(!path.exists());
    }
}
