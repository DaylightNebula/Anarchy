//! Scenes: trees of entities described as component patches, spawned into a [`World`].
//!
//! A [`Scene`] comes from the [`scene!`] macro, from text with [`Scene::parse`], or from
//! a world with [`Scene::from_world`].  Each component is a patch over the type's
//! defaults, so a scene only says what differs:
//!
//! ```text
//! @Gravity { y: -9.8 },           // a resource
//! #Player                         // an entity named Player
//! Transform { pos: { x: 1.0 } }   // only pos.x is set, the rest are defaults
//! Health(100)
//! Target(#Enemy)                  // a reference to another entity in the scene
//! [
//!     #Sword
//!     :Weapon                     // inherits the Weapon scene from the SceneLibrary
//!     Transform { pos: { x: 0.5 } },
//! ],
//! #Enemy
//! Health(50),
//! ```
//!
//! Text scenes, and the inherited scenes they name, are built through the world's
//! [`TypeRegistry`], so every component in them must be registered.

use std::{collections::HashMap, fmt::{self, Debug}, sync::Arc};

pub use anarchy_macros::scene;

use crate::*;

pub mod parse;
pub mod print;
pub mod save;
pub mod spawn;

pub use parse::*;
pub use save::*;
pub use spawn::*;

/// A tree of entities and a set of resources, described as patches.  Spawn it with
/// [`Scene::spawn`].  Clones are cheap, typed patches share their builders.
#[derive(Debug, Clone, Default)]
pub struct Scene {
    /// Resources inserted, or patched if the world already holds them, before the entities spawn.
    pub resources: Vec<ResourcePatch>,
    /// The root entities.
    pub roots: Vec<SceneEntity>
}

/// One entity in a [`Scene`].
#[derive(Debug, Clone, Default)]
pub struct SceneEntity {
    /// The entity's name from `#name`, which other entities refer to it by.  Spawning
    /// adds a [`Name`] component holding it.
    pub name: Option<String>,
    /// A scene in the [`SceneLibrary`] this entity starts from, from `:base`.  The base's
    /// single root gives its components first, and its children before this entity's own.
    pub inherits: Option<String>,
    /// The entity's components, applied in order.  Patches of the same type merge.
    pub components: Vec<ScenePatch>,
    /// The entity's children.
    pub children: Vec<SceneEntity>
}

/// A component in a [`SceneEntity`].
#[derive(Clone)]
pub enum ScenePatch {
    /// A patch built through the [`TypeRegistry`]: the first patch of a type builds it
    /// with [`FromValue::from_value`], later ones [`apply`](Reflect::apply) to it.
    Reflected {
        /// The registered type's name, see [`TypeRegistry::get_by_name`].
        type_name: String,
        /// The patch.
        value: Value
    },
    /// A component built by Rust code, from [`scene!`].  It replaces any earlier patch
    /// of the same type.
    Typed {
        /// The component's id.
        id: ComponentID,
        /// The component's type name.
        type_name: &'static str,
        /// Builds the component.
        make: Arc<dyn Fn() -> DynComponent + Send + Sync>
    }
}

impl ScenePatch {
    /// A patch built through the [`TypeRegistry`].
    pub fn reflected(type_name: impl Into<String>, value: Value) -> Self {
        Self::Reflected { type_name: type_name.into(), value }
    }

    /// A component built by `make` each time the scene spawns.
    pub fn typed<C: ComponentMeta>(make: impl Fn() -> C + Send + Sync + 'static) -> Self {
        Self::Typed { id: C::id(), type_name: std::any::type_name::<C>(), make: Arc::new(move || Box::new(make()) as DynComponent) }
    }
}

impl Debug for ScenePatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reflected { type_name, value } => write!(f, "{type_name} = {value}"),
            Self::Typed { type_name, .. } => write!(f, "{type_name} (typed)")
        }
    }
}

/// A resource in a [`Scene`].
#[derive(Clone)]
pub enum ResourcePatch {
    /// A patch built through the [`TypeRegistry`].  If the world already holds the
    /// resource the patch is applied to it, otherwise it is built and inserted.
    Reflected {
        /// The registered type's name, see [`TypeRegistry::get_by_name`].
        type_name: String,
        /// The patch.
        value: Value
    },
    /// A resource built by Rust code, from [`scene!`].  It replaces the world's resource.
    Typed {
        /// The resource's type name.
        type_name: &'static str,
        /// Builds the resource and inserts it.
        insert: Arc<dyn Fn(&World) + Send + Sync>
    }
}

impl ResourcePatch {
    /// A patch built through the [`TypeRegistry`].
    pub fn reflected(type_name: impl Into<String>, value: Value) -> Self {
        Self::Reflected { type_name: type_name.into(), value }
    }

    /// A resource built by `make` each time the scene spawns.
    pub fn typed<R: Resource>(make: impl Fn() -> R + Send + Sync + 'static) -> Self {
        Self::Typed { type_name: std::any::type_name::<R>(), insert: Arc::new(move |world| world.insert_resource(make())) }
    }
}

impl Debug for ResourcePatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reflected { type_name, value } => write!(f, "{type_name} = {value}"),
            Self::Typed { type_name, .. } => write!(f, "{type_name} (typed)")
        }
    }
}

/// Scenes that entities can inherit with `:name`, kept as a world resource.
#[derive(Debug, Default, Resource)]
pub struct SceneLibrary(HashMap<String, Scene>);

impl SceneLibrary {
    /// An empty library.
    pub fn new() -> Self { Self::default() }

    /// Add `scene` as `name`, replacing any scene with that name.
    pub fn insert(&mut self, name: impl Into<String>, scene: Scene) -> &mut Self {
        self.0.insert(name.into(), scene);
        self
    }

    /// The scene called `name`.
    pub fn get(&self, name: &str) -> Option<&Scene> { self.0.get(name) }

    /// Remove the scene called `name`, returning it.
    pub fn remove(&mut self, name: &str) -> Option<Scene> { self.0.remove(name) }

    /// Every scene and its name.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Scene)> {
        self.0.iter().map(|(name, scene)| (name.as_str(), scene))
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[derive(Debug, Clone, Default, PartialEq, Reflect)]
    struct Vec2 { x: f32, y: f32 }

    #[derive(Debug, Clone, Default, PartialEq, Component, Reflect)]
    struct Transform { pos: Vec2, scale: f32 }

    #[derive(Debug, Clone, PartialEq, Component, Reflect)]
    struct Health(u32);

    #[derive(Debug, Clone, Default, PartialEq, Component, Reflect)]
    struct Marker;

    #[derive(Debug, Clone, PartialEq, Component, Reflect)]
    struct Target(#[reflect(entity)] EntityID);

    #[derive(Debug, Clone, PartialEq, Component, Reflect)]
    #[reflect(default)]
    struct Speed { value: f32, max: f32 }

    impl Default for Speed {
        fn default() -> Self { Self { value: 1.0, max: 10.0 } }
    }

    #[derive(Debug, Component)]
    struct Unregistered;

    #[derive(Debug, Clone, Default, PartialEq, Resource, Reflect)]
    struct Gravity { y: f32 }

    fn new_world() -> World {
        let world = World::default();
        // only these types, since other test modules also have types called Health and Gravity
        world.insert_resource(TypeRegistry::empty());
        world.register_component::<Name>();
        world.register_component::<Transform>();
        world.register_component::<Health>();
        world.register_component::<Marker>();
        world.register_component::<Target>();
        world.register_component::<Speed>();
        world.register_resource::<Gravity>();
        world
    }

    fn ids() -> impl FnMut() -> EntityID {
        let mut next = 100;
        move || { next += 1; next }
    }

    fn get<C: ComponentMeta + Clone>(world: &World, id: EntityID) -> C {
        world.get::<&C>(id).unwrap().unwrap().clone()
    }

    const TEXT: &str = r#"
        @Gravity { y: -9.8 },
        #Player
        Transform { pos: { y: 2.0 } }
        Transform { scale: 3.0 }
        Speed { max: 5.0 }
        Target(#Enemy)
        [
            #Sword Marker,
            Health(1),
        ],
        #Enemy
        Health(50),
    "#;

    #[test]
    fn spawns_text_scenes() {
        let world = new_world();
        let instance = Scene::parse(TEXT).unwrap().spawn(&world, ids()).unwrap();
        assert_eq!(instance.roots, [101, 104]);
        assert_eq!(instance.entities, [101, 102, 103, 104]);
        let (player, sword, enemy) = (instance.by_name["Player"], instance.by_name["Sword"], instance.by_name["Enemy"]);

        assert_eq!(get::<Transform>(&world, player), Transform { pos: Vec2 { x: 0.0, y: 2.0 }, scale: 3.0 });
        assert_eq!(get::<Speed>(&world, player), Speed { value: 1.0, max: 5.0 });
        assert_eq!(get::<Target>(&world, player), Target(enemy));
        assert_eq!(get::<Name>(&world, player), Name::new("Player"));
        assert_eq!(get::<Health>(&world, enemy), Health(50));
        assert!(world.get::<&Marker>(sword).unwrap().is_some());
        assert_eq!(world.hierarchy().children(player).unwrap().to_vec(), [sword, 103]);
        assert_eq!(*world.resource::<Gravity>().unwrap(), Gravity { y: -9.8 });

        // a resource that exists is patched rather than replaced
        Scene::parse("@Gravity").unwrap().spawn(&world, ids()).unwrap();
        assert_eq!(*world.resource::<Gravity>().unwrap(), Gravity { y: -9.8 });

        instance.despawn(&world);
        assert!(world.entity(player).is_none() && world.entity(103).is_none());
    }

    #[test]
    fn inherits_from_the_library() {
        let world = new_world();
        let mut library = SceneLibrary::new();
        library.insert("Unit", Scene::parse("Health(10) Transform { scale: 2.0 } [ #Hat Marker ]").unwrap());
        library.insert("Knight", Scene::parse(":Unit Health(20)").unwrap());
        world.insert_resource(library);

        let instance = Scene::parse("#K :Knight Transform { pos: { x: 1.0 } } [ #Sword Marker ]").unwrap().spawn(&world, ids()).unwrap();
        let knight = instance.by_name["K"];
        assert_eq!(get::<Health>(&world, knight), Health(20));
        assert_eq!(get::<Transform>(&world, knight), Transform { pos: Vec2 { x: 1.0, y: 0.0 }, scale: 2.0 });
        assert_eq!(world.hierarchy().children(knight).unwrap().to_vec(), [instance.by_name["Hat"], instance.by_name["Sword"]]);
    }

    #[test]
    fn failures_leave_entities_untouched() {
        let world = new_world();
        let mut library = SceneLibrary::new();
        library.insert("A", Scene::parse(":B Marker").unwrap());
        library.insert("B", Scene::parse(":A Marker").unwrap());
        library.insert("Two", Scene::parse("Marker, Marker").unwrap());
        world.insert_resource(library);

        for text in [
            "#A Marker, #B Unknown",
            "#A Target(#Nobody)",
            "#A Health(\"lots\")",
            "#A Gravity",
            ":A",
            ":Two",
            ":Missing",
            "Marker [ :Two ]",
            "[ Marker ]"
        ] {
            assert!(Scene::parse(text).unwrap().spawn(&world, ids()).is_err(), "{text}");
        }
        assert_eq!(world.entity_ids().count(), 0);
    }

    #[test]
    fn saves_what_differs() {
        let world = new_world();
        let instance = Scene::parse(TEXT).unwrap().spawn(&world, ids()).unwrap();
        world.insert(200, Box::new([mutual::RelaxedMutex::new(Box::new(Unregistered) as DynComponent)]));
        world.hierarchy().set_parent(200, instance.by_name["Enemy"]).unwrap();

        let saved = Scene::from_world(&world, &instance.roots).unwrap();
        assert_eq!(saved.skipped, [(200, Unregistered::id())]);
        let text = saved.scene.to_text().unwrap();
        for expected in ["#Player", "Transform { pos: { y: 2.0 }, scale: 3.0 }", "Speed { max: 5.0 }", "Target(#Enemy)", "#Sword\n    Marker", "Health(1)"] {
            assert!(text.contains(expected), "missing {expected:?} in\n{text}");
        }
        assert!(!text.contains("Name"), "{text}");
        assert!(text.contains("[\n    #e200,\n]"), "{text}");

        // spawning the saved scene rebuilds the same components
        let copy = new_world();
        let again = Scene::parse(&text).unwrap().spawn(&copy, ids()).unwrap();
        assert_eq!(get::<Transform>(&copy, again.by_name["Player"]), get::<Transform>(&world, instance.by_name["Player"]));
        assert_eq!(get::<Target>(&copy, again.by_name["Player"]), Target(again.by_name["Enemy"]));
        assert_eq!(Scene::from_world(&copy, &again.roots).unwrap().scene.to_text().unwrap(), text);
    }

    #[test]
    fn saving_names_referenced_entities() {
        let world = new_world();
        let instance = Scene::parse("Marker [ Target(#1) ], Health(1), Marker").unwrap().spawn(&world, ids()).unwrap();
        world.reflect_component_mut(102, Target::id(), |target| target.apply(&Value::Tuple(vec![Value::Int(103)]))).unwrap().unwrap();
        world.insert_component_dyn(104, Box::new(Name::new("Dup"))).unwrap();
        world.insert_component_dyn(101, Box::new(Name::new("Dup"))).unwrap();

        let text = Scene::from_world(&world, &instance.roots).unwrap().scene.to_text().unwrap();
        assert!(text.contains("Target(#e103)"), "{text}");
        assert!(text.contains("#e103\nHealth(1)"), "{text}");
        assert_eq!(text.matches("Name(\"Dup\")").count(), 2, "{text}");
    }

    #[test]
    fn macro_matches_text() {
        let world = new_world();
        world.insert_resource(Gravity { y: 0.0 });
        let scene = scene! {
            @Gravity { y: -9.8 },
            #Player
            Transform { pos: Vec2 { y: 2.0 } }
            Transform { pos: Vec2 { y: 2.0 }, scale: 3.0 }
            Speed { max: 5.0 }
            [
                #Sword Marker,
                { Health(1) },
            ],
            #Enemy
            Health(50),
        };
        let instance = scene.spawn(&world, ids()).unwrap();
        let text = new_world();
        let text_instance = Scene::parse(TEXT).unwrap().spawn(&text, ids()).unwrap();
        assert_eq!(instance.entities, text_instance.entities);
        for id in [101, 102, 103, 104] {
            assert_eq!(world.component_ids(id).unwrap().len() + (id == 101) as usize, text.component_ids(id).unwrap().len(), "entity {id}");
        }
        assert_eq!(get::<Transform>(&world, 101), get::<Transform>(&text, 101));
        assert_eq!(get::<Speed>(&world, 101), get::<Speed>(&text, 101));
        assert_eq!(get::<Health>(&world, 103), Health(1));
        assert_eq!(*world.resource::<Gravity>().unwrap(), Gravity { y: -9.8 });
        assert_eq!(world.hierarchy().children(101).unwrap().to_vec(), [102, 103]);
        assert!(scene.to_text().is_err());
    }
}
