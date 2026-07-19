use std::{marker::PhantomData, sync::atomic::AtomicU32};

use chrono::{DateTime, Utc};
use mutual::{AsAny, Ref, RefCastGuard};

use crate::{Resource, SystemExtractor};


pub mod tracker;

pub use tracker::*;


/// Global counter used to hand out the next unused `EventID`.
pub static NEXT_EVENT_ID: AtomicU32 = AtomicU32::new(0);

/// The unique ID of an event type, shared by all instances of that type.
pub type EventID = u32;
/// The unique ID of a single broadcast event instance, used to track which events a
/// reader has already consumed. These must never repeat.
pub type EventInstanceID = u32;

/// The standard `Event` trait, implemented by the `Event` derive macro.
pub trait EventImpl: AsAny {
    /// Returns the `EventID` of this event's type.
    fn get_id(&self) -> EventID;
}

/// The standard metadata of an `Event`.
/// This also provides future instance IDs, these IDs must *never* repeat.
pub trait EventMeta {
    /// Returns the `EventID` shared by every instance of this event type.
    fn id() -> EventID;
    /// Returns the next unused `EventInstanceID` for this event type.
    fn next_instance_id() -> EventInstanceID;
}

/// Simple container for an event paired with its ID and the `Instant` 
/// at which the event was fired at.
pub struct EventContainer {
    pub id: EventInstanceID,
    pub event: Box<dyn EventImpl>,
    pub created_at: DateTime<Utc>
}

/// The `SystemExtractor` to read events from an `EventTracker`
/// while minimizing boilerplate.
pub struct Event<E: EventImpl + EventMeta + 'static> {
    tracker: RefCastGuard<Box<dyn Resource + 'static>, EventTracker>,
    _phantom: PhantomData<E>
}

impl <'a, I, E> SystemExtractor<'a, I> for Event<E> 
    where E: EventImpl + EventMeta + 'static
{
    fn extract(
        _id: super::ScheduleID, 
        world: &'a super::World, 
        inputs: Option<&'a I>
    ) -> (Self, Option<&'a I>) where Self: Sized {
        (
            Self {
                tracker: world
                    .get_resource_ref::<EventTracker>()
                    .expect("Failed to get event tracker resource"),
                _phantom: PhantomData::default()
            }, 
            inputs
        )
    }
}

impl <E: EventImpl + EventMeta + 'static> Event<E> {
    /// Reads all events defined as `Event`s `E` generic.
    pub fn read(&self, min_id_tracker: &'static EventSystemMinIDTracker) -> 
        impl Iterator<Item = Ref<E>> 
    {
        return self.tracker.pull_events_ref(min_id_tracker);
    }

    /// Writes an event to the tracker of the `Event`s `E` generic.
    pub fn write(&self, event: E) {
        self.tracker.broadcast_event(event)
    }
}
