// use std::{cell::RefCell, rc::{Rc, Weak}};

// use double_linked_list::double_linked_list::Node;

// use crate::*;

// pub struct DoublyLinkedListTable {
//     comp_ids: Box<[ComponentID]>,
//     table: double_linked_list::double_linked_list::DoubleLinkedList<DynComponents>
// }

// impl Table for DoublyLinkedListTable {
//     fn group<'a>(&'a self) -> ComponentIDGroup<'a> {
//         &*self.comp_ids
//     }

//     fn cursor(&self) -> Cursor {
//         Cursor::new(DoublyLinkedListCursor {
//             inner: RefCell::new(self.table.get_cursor())
//         })
//     }
// }


// pub struct DoublyLinkedListCursor {
//     inner: RefCell<double_linked_list::double_linked_list::Cursor<DynComponents>>
// }

// impl CursorImpl for DoublyLinkedListCursor {
//     fn has_next(&self) -> bool {
//         self.inner.borrow().after.as_ref()
//             .and_then(Weak::upgrade)
//             .is_some_and(|node| !node.borrow().is_root)
//     }

//     /// Returns the element after the cursor and steps over it.
//     fn next(&self) -> Option<(EntityID, DynComponents)> {
//         let mut cursor = self.inner.borrow_mut();
//         let after = cursor.after.as_ref()?.upgrade()?;
//         let (value, next) = {
//             let node = after.borrow();
//             if node.is_root { return None; }
//             (node.value.clone(), node.next.clone())
//         };

//         cursor.before = Some(Rc::downgrade(&after));
//         cursor.after = next.as_ref().map(Rc::downgrade);
//         value
//     }

//     /// Unlinks the element after the cursor (the one `next` would return) and returns it.
//     fn pop(&self) -> Option<(EntityID, DynComponents)> {
//         let mut cursor = self.inner.borrow_mut();
//         let after = cursor.after.as_ref()?.upgrade()?;
//         if after.borrow().is_root { return None; }

//         let (value, next, previous) = {
//             let mut node = after.borrow_mut();
//             (node.value.take(), node.next.take(), node.previous.take().and_then(|p| p.upgrade()))
//         };

//         // borrows are kept separate as `previous` and `next` may be the same node (the root)
//         if let Some(previous) = &previous {
//             previous.borrow_mut().next = next.clone();
//         }
//         if let Some(next) = &next {
//             next.borrow_mut().previous = previous.as_ref().map(Rc::downgrade);
//         }

//         cursor.before = previous.as_ref().map(Rc::downgrade);
//         cursor.after = next.as_ref().map(Rc::downgrade);
//         value
//     }

//     /// Inserts before the cursor, so the new element is not returned by `next`.
//     /// Hands the components back if the cursor is not attached to the list.
//     fn insert(&self, _id: EntityID, components: DynComponents) -> Option<DynComponents> {
//         let mut cursor = self.inner.borrow_mut();
//         let Some(before) = cursor.before.as_ref().and_then(Weak::upgrade) else {
//             return Some(components);
//         };

//         let next = before.borrow().next.clone();
//         let node = Rc::new(RefCell::new(Node {
//             value: Some(components),
//             next: next.clone(),
//             previous: Some(Rc::downgrade(&before)),
//             is_root: false,
//         }));

//         before.borrow_mut().next = Some(node.clone());
//         if let Some(next) = &next {
//             next.borrow_mut().previous = Some(Rc::downgrade(&node));
//         }

//         cursor.before = Some(Rc::downgrade(&node));
//         None
//     }
// }
