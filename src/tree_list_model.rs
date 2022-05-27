use std::sync::{Arc, Mutex};

use gtk::subclass::prelude::*;
use gtk::{gio, glib};

use crate::capture::{Capture, Item};

#[derive(Copy, Clone)]
pub struct TreeNode {
    pub item: Item,
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
            model.set_item_count(cap.item_count(&None).unwrap());
        }
        model.set_capture(capture);
        model
    }

    fn set_item_count(&mut self, count: u64) {
        self.imp().item_count.set(count.try_into().unwrap());
    }

    fn set_capture(&mut self, capture: Arc<Mutex<Capture>>) {
        self.imp().capture.replace(Some(capture));
    }
}

mod imp {
    use std::cell::{Cell, RefCell};
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
        pub(super) item_count: Cell<u32>,
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
            self.item_count.get()
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
            let item = cap.get_item(&None, position as u64).ok()?;
            let node = TreeNode {
                item
            };
            Some(RowData::new(node).upcast::<glib::Object>())
        }
    }
}
