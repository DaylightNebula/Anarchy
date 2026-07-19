use std::sync::OnceLock;

use chrono::Utc;
use derive_more::{Deref, DerefMut};
use mutual::{CowData, Ref, SharedList, SharedMap};

use crate::{AsAny, EventImpl, EventContainer, EventID, EventInstanceID, EventMeta, resources::{Resource, ResourceID, ResourceMeta}};


/// Per-reader state tracking, for each `EventID`, the lowest `EventInstanceID` that
/// reader has not yet consumed. Systems typically own a `'static` instance of this so
/// repeated reads only return events broadcast since the last read.
pub type EventSystemMinIDTracker = OnceLock<SharedMap<EventID, CowData<EventInstanceID>>>;


/// The `World` resource that stores every broadcast event, keyed by `EventID`. Events
/// older than one second are dropped the next time their type is broadcast to again.
#[derive(Deref, DerefMut, Default)]
pub struct EventTracker(SharedMap<EventID, SharedList<EventContainer>>);

impl EventTracker {
    const ID: ResourceID = 594023;

    /// Creates an iterator over all events currently queued in this `EventTracker` that
    /// match the given `Event` as generic `E`.
    pub fn pull_events_raw<'a, E>(
        &'a self
    ) -> impl Iterator<Item = Ref<EventContainer>> 
        where E: EventImpl + EventMeta + 'static
    {
        let event_list = self
            .compute_if_absent(E::id(), || SharedList::new());

        event_list.iter()
    }

    /// Creates an iterator using this event tracker and a `EventSystemMinIDTracker` to
    /// immutable references to all matching events.
    pub fn pull_events_ref<'a, E>(
        &'a self, 
        min_id_tracker: &'static EventSystemMinIDTracker, 
    ) -> impl Iterator<Item = Ref<E>>
        where E: EventImpl + EventMeta + 'static
    {
        let min_return_id_cow = min_id_tracker.get_or_init(|| SharedMap::new())
            .compute_if_absent(E::id(), || CowData::new(0));
        let min_return_id = *min_return_id_cow.get_ref();
        
        let event_list = self
            .compute_if_absent(E::id(), || SharedList::new());

        event_list
            .iter()
            .filter_map(move |container| {
                if container.id >= min_return_id {
                    let next_min = container.id + 1;
                    if next_min > *min_return_id_cow.get_ref() { min_return_id_cow.set(next_min); }
                    Some(Ref::new(container, |node| node.downcast_ref::<Ref<EventContainer>>().unwrap().event.as_any().downcast_ref::<E>().unwrap()))
                } else { None }
            })
    }

    // /// Creates an iterator using this event tracker and a `EventSystemMinIDTracker` to
    // /// immutable references to all matching events.
    // pub fn pull_events_mut<'a, E>(
    //     &'a self, 
    //     min_id_tracker: &'static mut EventSystemMinIDTracker, 
    // ) -> impl Iterator<Item = &'a mut E>
    //     where E: Event + EventMeta + 'static
    // {
    //     let min_return_id_cow = min_id_tracker.get_or_init(|| SharedMap::new())
    //         .compute_if_absent(E::id(), || CowData::new(0));
    //     let min_return_id = *min_return_id_cow.get_ref();
        
    //     let event_list = self
    //         .compute_if_absent(E::id(), || SharedList::new());

    //     event_list
    //         .iter_mut()
    //         .filter_map(move |container| {
    //             if container.id >= min_return_id {
    //                 let next_min = container.id + 1;
    //                 if next_min > *min_return_id_cow.get_ref() { min_return_id_cow.set(next_min); }
    //                 let event = container.event.as_any_mut().downcast_mut::<E>().unwrap();
    //                 Some(event)
    //             } else { None }
    //         })
    // }

    /// Broadcast the given event.  These events will be returned by the `pull_events_ref` and `pull_events_mut`.
    pub fn broadcast_event<E>(
        &self,
        event: E
    ) where E: EventImpl + EventMeta + 'static {
        let boxed: Box<dyn EventImpl> = Box::new(event);
        let instance_id = E::next_instance_id();

        // get event list
        let event_list = self
            .compute_if_absent(E::id(), || SharedList::new());

        // remove all events that are too old
        let now = Utc::now();
        event_list.remove_all(|container| {
            now.signed_duration_since(container.created_at).as_seconds_f32() > 1.0
        });

        // create and save container
        let container = EventContainer { id: instance_id, event: boxed, created_at: now };
        event_list.push(container);
    }
}

impl AsAny for EventTracker {
    fn as_any(&self) -> &dyn std::any::Any { self }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}

impl ResourceMeta for EventTracker {
    fn id() -> ResourceID { Self::ID }
    fn name() -> &'static str { "EventTracker" }
}

impl Resource for EventTracker {
    fn get_id(&self) -> ResourceID { Self::ID }
    fn get_name(&self) -> &'static str { "EventTracker" }
}