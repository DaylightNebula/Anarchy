//! An experimental ECS built for as much concurrency as possible while systems
//! run through a world at once.  Data is only locked where a general purpose ECS
//! has to, per component and per resource, and queries can share their work
//! across threads.
//!
//! - A [`World`] holds entities, [`Resource`]s and the [`Hierarchy`] linking entities.
//! - Entities with the same set of [`Component`]s share a [`ChunkedTable`], which stores
//!   each component as a column of raw bytes.
//! - A [`Query`] walks every entity holding a set of components, while
//!   [`World::handle`] and [`WorldImpl::get`] reach one entity by id.
//! - Plain functions become [`System`]s, ordered by a [`SystemGraph`] and run by
//!   a [`SingleThreadedExecutor`] or [`MultiThreadedExecutor`].
//! - Systems talk to each other through [`Event`]s.
//! - [`Reflect`] reads, builds and patches values by field name, and a [`TypeRegistry`]
//!   lets editors work on a world's components by id and path.
//! - A [`Scene`] describes entities as component patches, written as text or with
//!   [`scene!`], and is spawned with [`Scene::spawn`].
//! - With the `socket` feature, a `WorldSocket` lets other programs, such as the
//!   `anarchy` command line tool, inspect and edit a running world.
//!
//! ```
//! use anarchy::*;
//! use mutual::RelaxedMutex;
//!
//! #[derive(Debug, Component)]
//! struct Position(f32);
//!
//! #[derive(Debug, Component)]
//! struct Velocity(f32);
//!
//! #[derive(Debug, Resource)]
//! struct Ticks(u32);
//!
//! fn movement(mut query: Query<(&'static mut Position, &'static Velocity)>) {
//!     while let Some((_entity, (mut position, velocity))) = query.next().unwrap() {
//!         position.0 += velocity.0;
//!     }
//! }
//!
//! fn tick(mut ticks: ResMut<Ticks>) {
//!     ticks.0 += 1;
//! }
//!
//! fn main() -> anyhow::Result<()> {
//!     let world = World::default();
//!     world.insert_resource(Ticks(0));
//!     for id in [1, 2] {
//!         world.insert(id, Box::new([
//!             RelaxedMutex::new(Box::new(Position(0.0)) as DynComponent),
//!             RelaxedMutex::new(Box::new(Velocity(2.0)) as DynComponent),
//!         ]));
//!     }
//!     world.handle(2).set_parent(1)?;
//!
//!     let mut graph = SystemGraph::new();
//!     graph.append_system(movement, SystemInstruction::default(), std::iter::empty())?;
//!     graph.append_system(tick, after(movement), std::iter::empty())?;
//!
//!     SingleThreadedExecutor::new().run(&world, &graph)?;
//!
//!     assert_eq!(world.resource::<Ticks>()?.0, 1);
//!     assert_eq!(world.get::<&Position>(2)?.unwrap().0, 2.0);
//!     assert_eq!(world.handle(2).parent().unwrap().id(), 1);
//!     Ok(())
//! }
//! ```

extern crate self as anarchy;

pub mod builder;
pub mod components;
pub mod events;
pub mod hierarchy;
pub mod queries;
pub mod reflect;
pub mod resources;
pub mod scenes;
pub mod schedules;
#[cfg(feature = "socket")]
pub mod socket;
pub mod tables;
pub mod worlds;

pub use builder::*;
pub use components::*;
pub use events::*;
pub use hierarchy::*;
pub use queries::*;
pub use reflect::*;
pub use resources::*;
pub use scenes::*;
pub use schedules::*;
#[cfg(feature = "socket")]
pub use socket::*;
pub use tables::*;
pub use worlds::*;
