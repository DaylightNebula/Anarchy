use std::{cell::RefCell, sync::{Arc, atomic::{AtomicUsize, Ordering}}};

use mutual::ArcSwapOption;

use crate::*;

/// Points at the next node, `None` marks the end of the list.
type Link = ArcSwapOption<Node>;

struct Node {
    #[allow(dead_code)] // not read yet, kept so entities can be looked up by id later
    id: EntityID,
    components: DynComponents,
    next: Link
}

impl Drop for Node {
    fn drop(&mut self) {
        // unlink the nodes behind this one iteratively, otherwise dropping a long chain overflows the stack;
        // stops at the first node something else still holds, as that owner will drop the rest
        let mut next = self.next.swap(None);
        while let Some(node) = next.and_then(Arc::into_inner) {
            next = node.next.swap(None);
        }
    }
}

pub struct SingleLinkedListTable {
    comp_ids: Box<[ComponentID]>,
    head: Arc<Link>,
    len: Arc<AtomicUsize>
}

impl SingleLinkedListTable {
    pub fn new(comp_ids: &[ComponentID]) -> Self {
        Self {
            comp_ids: Box::from(comp_ids),
            head: Arc::new(Link::empty()),
            len: Arc::new(AtomicUsize::new(0))
        }
    }

    pub fn len(&self) -> usize {
        self.len.load(Ordering::Acquire)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl TableImpl for SingleLinkedListTable {
    fn group<'a>(&'a self) -> ComponentIDGroup<'a> {
        &*self.comp_ids
    }

    fn cursor(&self) -> Cursor {
        Cursor::new(SingleLinkedListCursor {
            head: self.head.clone(),
            previous: RefCell::new(None),
            len: self.len.clone()
        })
    }
}


/// Sits after `previous`, or at the start of the list when `previous` is `None`.
pub struct SingleLinkedListCursor {
    head: Arc<Link>,
    previous: RefCell<Option<Arc<Node>>>,
    len: Arc<AtomicUsize>
}

impl SingleLinkedListCursor {
    /// Runs `f` with the link that points at the node after the cursor.
    fn with_link<R>(&self, f: impl FnOnce(&Link) -> R) -> R {
        match &*self.previous.borrow() {
            Some(previous) => f(&previous.next),
            None => f(&self.head)
        }
    }
}

impl CursorImpl for SingleLinkedListCursor {
    fn has_next(&self) -> bool {
        self.with_link(|link| link.load().is_some())
    }

    /// Returns the element after the cursor and steps over it.
    fn next(&self) -> Option<(EntityID, DynComponents)> {
        let node = self.with_link(|link| link.load_full())?;
        let entity_id = node.id;
        let components = node.components.clone();
        *self.previous.borrow_mut() = Some(node);
        Some((entity_id, components))
    }

    /// Unlinks the element after the cursor (the one `next` would return) and returns it.
    fn pop(&self) -> Option<(EntityID, DynComponents)> {
        // retry if another cursor changed this link between the load and the swap
        let node = self.with_link(|link| loop {
            let current = link.load();
            let node = current.as_ref()?;
            let previous = link.compare_and_swap(&current, node.next.load_full());
            if same_node(&previous, &current) { break Some(node.clone()); }
        })?;
        self.len.fetch_sub(1, Ordering::AcqRel);

        // move the components out unless another cursor is holding the node right now
        Some((
            node.id, 
            match Arc::try_unwrap(node) {
                Ok(mut node) => std::mem::take(&mut node.components),
                Err(node) => node.components.clone()
            }
        ))
    }

    /// Inserts before the cursor, so the new element is not returned by `next`.
    fn insert(&self, id: EntityID, components: DynComponents) {
        let node = Arc::new(Node { id, components, next: Link::empty() });

        // retry if another cursor changed this link between the load and the swap
        self.with_link(|link| loop {
            let current = link.load();
            node.next.store(current.clone());
            let previous = link.compare_and_swap(&current, Some(node.clone()));
            if same_node(&previous, &current) { break; }
        });
        self.len.fetch_add(1, Ordering::AcqRel);

        *self.previous.borrow_mut() = Some(node);
    }
}

fn same_node(a: &Option<Arc<Node>>, b: &Option<Arc<Node>>) -> bool {
    a.as_ref().map(Arc::as_ptr) == b.as_ref().map(Arc::as_ptr)
}


#[cfg(test)]
mod tests {
    use mutual::{AsAny, RelaxedMutex, SharedData};

    use super::*;

    #[derive(Debug)]
    struct Value(u32);

    impl AsAny for Value {
        fn as_any(&self) -> &dyn std::any::Any { self }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }

    impl Component for Value {}

    fn components(value: u32) -> DynComponents {
        Box::new([RelaxedMutex::new(Box::new(Value(value)) as DynComponent)])
    }

    fn value(components: DynComponents) -> u32 {
        components[0].lock_ref().as_any().downcast_ref::<Value>().unwrap().0
    }

    fn values(table: &SingleLinkedListTable) -> Vec<u32> {
        let cursor = table.cursor();
        std::iter::from_fn(|| cursor.next()).map(|c| value(c.1)).collect()
    }

    fn table_of(values: &[u32]) -> SingleLinkedListTable {
        let table = SingleLinkedListTable::new(&[0]);
        let cursor = table.cursor();
        for &v in values { cursor.insert(v as EntityID, components(v)); }
        table
    }

    #[test]
    fn empty() {
        let table = table_of(&[]);
        let cursor = table.cursor();
        assert!(table.is_empty());
        assert!(!cursor.has_next());
        assert!(cursor.next().is_none());
        assert!(cursor.pop().is_none());
    }

    #[test]
    fn insert_keeps_order_and_skips_inserted() {
        let table = table_of(&[1, 2, 3]);
        assert_eq!(table.len(), 3);
        assert_eq!(values(&table), [1, 2, 3]);

        // inserting mid-iteration lands before the cursor, so `next` carries on from where it was
        let cursor = table.cursor();
        assert_eq!(cursor.next().map(|c| value(c.1)), Some(1));
        cursor.insert(10, components(10));
        assert_eq!(cursor.next().map(|c| value(c.1)), Some(2));
        assert_eq!(values(&table), [1, 10, 2, 3]);
        assert_eq!(table.len(), 4);
    }

    #[test]
    fn pop_front_middle_end() {
        let table = table_of(&[1, 2, 3, 4]);
        let cursor = table.cursor();
        assert_eq!(cursor.pop().map(|c| value(c.1)), Some(1));
        assert_eq!(cursor.next().map(|c| value(c.1)), Some(2));
        assert_eq!(cursor.pop().map(|c| value(c.1)), Some(3));
        assert_eq!(cursor.pop().map(|c| value(c.1)), Some(4));
        assert!(!cursor.has_next());
        assert!(cursor.pop().is_none());
        assert_eq!(values(&table), [2]);
        assert_eq!(table.len(), 1);

        assert_eq!(table.cursor().pop().map(|c| value(c.1)), Some(2));
        assert!(table.is_empty());
        assert!(!table.cursor().has_next());
    }

    #[test]
    fn pop_while_another_cursor_holds_node() {
        let table = table_of(&[1, 2]);
        let reader = table.cursor();
        assert_eq!(reader.next().map(|c| value(c.1)), Some(1));

        // `reader` still holds node 1, so pop has to clone its components out
        assert_eq!(table.cursor().pop().map(|c| value(c.1)), Some(1));
        assert_eq!(values(&table), [2]);
    }

    #[test]
    fn drop_long_list() {
        let table = SingleLinkedListTable::new(&[0]);
        let cursor = table.cursor();
        for i in 0..1_000_000 { cursor.insert(i, components(0)); }
        drop(cursor);
        drop(table);
    }

    #[test]
    fn concurrent_insert_and_pop() {
        const THREADS: usize = 8;
        const PER_THREAD: usize = 10_000;

        let table = SingleLinkedListTable::new(&[0]);
        let new_cursor = || SingleLinkedListCursor {
            head: table.head.clone(),
            previous: RefCell::new(None),
            len: table.len.clone()
        };

        std::thread::scope(|s| for _ in 0..THREADS {
            let cursor = new_cursor();
            s.spawn(move || for i in 0..PER_THREAD { cursor.insert(i as EntityID, components(1)); });
        });
        assert_eq!(values(&table).len(), THREADS * PER_THREAD);
        assert_eq!(table.len(), THREADS * PER_THREAD);

        // every thread pops from the head, so each node must be popped exactly once
        let popped = AtomicUsize::new(0);
        std::thread::scope(|s| for _ in 0..THREADS {
            let cursor = new_cursor();
            let popped = &popped;
            s.spawn(move || while cursor.pop().is_some() { popped.fetch_add(1, Ordering::Relaxed); });
        });
        assert_eq!(popped.into_inner(), THREADS * PER_THREAD);
        assert!(table.is_empty());
        assert!(values(&table).is_empty());
    }
}
