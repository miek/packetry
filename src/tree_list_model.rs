use std::cell::{Cell, RefCell};
use std::collections::{LinkedList, BTreeMap};
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex};

use gtk::subclass::prelude::*;
use gtk::{gio, glib};

use crate::capture::{Capture, Item};

#[derive(Debug)]
pub struct TreeNode {
    item: Option<Item>,
    parent: Option<Weak<RefCell<TreeNode>>>,

    /// Index of this node below the parent Item.
    item_index: u32,

    /// Total count of visible nodes below this node, recursively.
    child_count: u32,

    /// List of expanded child nodes directly below this node.
    children: BTreeMap<u32, Rc<RefCell<TreeNode>>>,
}

impl TreeNode {
    pub fn item(&self) -> Item {
        self.item.clone().unwrap()
    }

    /// Position of this node in a list, relative to its parent node.
    pub fn relative_position(&self) -> u32 {
        if let Some(parent) = self.parent.as_ref() {
            if let Some(parent) = parent.upgrade() {
                // Sum up the `child_count`s of any expanded nodes before this one, and add to `item_index`.
                return parent.borrow().children.iter()
                    .take_while(|(&key, _)| key < self.item_index)
                    .map(|(_, node)| node.borrow().child_count)
                    .sum::<u32>() + self.item_index;
            }
        }
        0
    }
}

glib::wrapper! {
    pub struct TreeListModel(ObjectSubclass<imp::TreeListModel>) @implements gio::ListModel;
}

impl TreeListModel {
    pub fn new(capture: Arc<Mutex<Capture>>) -> Self {
        let mut model: TreeListModel =
            glib::Object::new(&[]).expect("Failed to create TreeListModel");
        {
            let mut cap = capture.lock().unwrap();
            model.set_root(TreeNode{
                item: None,
                parent: None,
                item_index: 0,
                child_count: u32::try_from(cap.item_count(&None).unwrap()).unwrap().into(),
                children: Default::default(),
            });
        }
        model.set_capture(capture);
        model
    }

    fn set_capture(&mut self, capture: Arc<Mutex<Capture>>) {
        self.imp().capture.replace(Some(capture));
    }

    fn set_root(&mut self, root: TreeNode) {
        self.imp().root.replace(Some(Rc::new(RefCell::new(root))));
    }
}

mod imp {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    use gtk::subclass::prelude::*;
    use gtk::{prelude::*, gio, glib};
    use thiserror::Error;

    use crate::capture::{Capture, CaptureError};
    use crate::row_data::RowData;

    use super::TreeNode;

    #[derive(Error, Debug)]
    pub enum ModelError {
        #[error(transparent)]
        CaptureError(#[from] CaptureError),
        #[error("Capture not set")]
        CaptureNotSet,
        #[error("Locking capture failed")]
        LockError,
    }

    #[derive(Default)]
    pub struct TreeListModel {
        pub(super) capture: RefCell<Option<Arc<Mutex<Capture>>>>,
        pub(super) root: RefCell<Option<Rc<RefCell<TreeNode>>>>,
    }

    impl TreeListModel {
        pub fn expanded(&self, node: Rc<RefCell<TreeNode>>) -> bool {
            let pos = node.borrow().item_index;
            let expanded = node.borrow().parent.as_ref().unwrap().upgrade().unwrap().borrow().children.get(&pos).is_some();
            expanded
        }
        pub fn set_expanded(&self, node: Rc<RefCell<TreeNode>>, expanded: bool) {
            let pos = node.borrow().item_index;
            let current = self.expanded(node.clone());
            if current != expanded {
                if expanded {
                    let count = node.borrow().child_count;
                    let mut position = node.borrow().relative_position();

                    // Add this node to the parent's list of expanded child nodes.
                    // TODO: split up this chain to be easier to follow & error handle
                    node.borrow().parent.as_ref().unwrap().upgrade().unwrap().borrow_mut().children.insert(pos, node.clone());

                    // Traverse back up the tree, increasing `child_count` to include the expanded entries.
                    let mut current_node = node;
                    loop {
                        if let Some(parent) = current_node.clone().borrow().parent.as_ref() {
                            if let Some(parent) = parent.upgrade() {
                                parent.borrow_mut().child_count += count;
                                current_node = parent;
                                position += current_node.borrow().relative_position() + 1;
                            } else {
                                break;
                            }
                        } else {
                            // `parent` is `None` so we should be at the root node and can stop here.
                            break;
                        }
                    }

                    self.instance().items_changed(position, 0, count);
                } else {
                    let count = node.borrow().child_count;
                    let mut position = node.borrow().relative_position();

                    // Remove this node from the parent's list of expanded child nodes.
                    // TODO: split up this chain to be easier to follow & error handle
                    node.borrow().parent.as_ref().unwrap().upgrade().unwrap().borrow_mut().children.remove(&pos);

                    // Traverse back up the tree, decreasing `child_count` to remove the collapsed entries.
                    let mut current_node = node;
                    loop {
                        if let Some(parent) = current_node.clone().borrow().parent.as_ref() {
                            if let Some(parent) = parent.upgrade() {
                                parent.borrow_mut().child_count -= count;
                                current_node = parent;
                                position += current_node.borrow().relative_position() + 1;
                            } else {
                                break;
                            }
                        } else {
                            // `parent` is `None` so we should be at the root node and can stop here.
                            break;
                        }
                    }

                    self.instance().items_changed(position, count, 0);
                }
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TreeListModel {
        const NAME: &'static str = "TreeListModel";
        type Type = super::TreeListModel;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for TreeListModel {}

    impl ListModelImpl for TreeListModel {
        fn item_type(&self, _list_model: &Self::Type) -> glib::Type {
            RowData::static_type()
        }

        fn n_items(&self, _list_model: &Self::Type) -> u32 {
            self.root.borrow().as_ref().unwrap().borrow().child_count
        }

        fn item(&self, _list_model: &Self::Type, position: u32)
            -> Option<glib::Object>
        {
            let opt = self.capture.borrow();
            let mut cap = match opt.as_ref() {
                Some(mutex) => match mutex.lock() {
                    Ok(guard) => guard,
                    Err(_) => return None
                },
                None => return None
            };

            let mut parent = self.root.borrow().as_ref().unwrap().clone();
            // First check that the position is valid (must be within the root node's `child_count`).
            if position < parent.borrow().child_count {
                let mut relative_position = position;
                'outer: loop {
                    for (_, node_rc) in parent.clone().borrow().children.iter() {
                        let node = node_rc.borrow();
                        // If the position is before this node, break out of the loop to look it up.
                        if relative_position < node.item_index {
                            break;
                        // If the position matches this node, return it.
                        } else if relative_position == node.item_index {
                            return Some(RowData::new(node_rc.clone()).upcast::<glib::Object>());
                        // If the position is within this node's children, traverse down the tree and repeat.
                        } else if relative_position <= node.item_index + node.child_count {
                            parent = node_rc.clone();
                            relative_position -= node.item_index + 1;
                            continue 'outer;
                        // Otherwise, if the position is after this node,
                        // adjust the relative position for the node's children above.
                        } else {
                            relative_position -= node.child_count;
                        }
                    }

                    // If we've broken out to this point, the node must be directly below `parent` - look it up.
                    let item = cap.get_item(&parent.borrow().item, relative_position as u64).ok()?;
                    let node = TreeNode {
                        item: Some(item),
                        parent: Some(Rc::downgrade(&parent)),
                        item_index: relative_position,
                        child_count: u32::try_from(cap.child_count(&item).unwrap()).unwrap().into(),
                        children: Default::default(),
                    };
                    let rowdata = RowData::new(Rc::new(RefCell::new(node)));

                    return Some(rowdata.upcast::<glib::Object>());
                }
            }
            None
        }
    }
}
