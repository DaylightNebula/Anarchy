//! An experimental ECS that lets systems run concurrently across a shared world.
//!
//! Locks are taken at the lowest level a general purpose ECS can manage, a single
//! component or resource, so systems only wait on each other when they touch the
//! same data. Long running work inside a system can also be split up and shared
//! with idle threads through the executor's work queue.
//!
//! # Overview
//!
//! - [`World`] holds every entity and resource. Entities are grouped into
//!   [`Table`]s by their exact set of components.
//! - [`Component`]s and [`Resource`]s are plain types, usually set up with
//!   `#[derive(Component)]` or `#[derive(Resource)]`.
//! - [`Query`] walks every entity holding a set of components and locks each
//!   component as it is handed out.
//! - Systems are functions whose arguments are [`SystemParam`]s. They are added
//!   to a [`SystemGraph`] with a [`SystemInstruction`] that orders them.
//! - [`SingleThreadedExecutor`] and [`MultiThreadedExecutor`] run the graph.
//!
//! # Example
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
//! fn tick(world: World) {
//!     world.resource_mut::<Ticks>().unwrap().0 += 1;
//! }
//!
//! # fn main() -> anyhow::Result<()> {
//! let world = World::default();
//! world.insert_resource(Ticks(0));
//! world.insert(0, Box::new([
//!     RelaxedMutex::new(Box::new(Position(0.0)) as DynComponent),
//!     RelaxedMutex::new(Box::new(Velocity(2.0)) as DynComponent),
//! ]));
//!
//! let mut graph = SystemGraph::new();
//! graph.append_system(movement, SystemInstruction::default(), std::iter::empty())?;
//! graph.append_system(tick, after(movement), std::iter::empty())?;
//!
//! SingleThreadedExecutor::new().run(&world, &graph)?;
//!
//! let mut positions = Query::<&Position>::new(&world);
//! assert_eq!(positions.next()?.unwrap().1.0, 2.0);
//! assert_eq!(world.resource::<Ticks>().unwrap().0, 1);
//! # Ok(())
//! # }
//! ```

extern crate self as anarchy;

pub mod components;
pub mod events;
pub mod queries;
pub mod resources;
pub mod schedules;
pub mod tables;
pub mod worlds;

pub use components::*;
pub use events::*;
pub use queries::*;
pub use resources::*;
pub use schedules::*;
pub use tables::*;
pub use worlds::*;
