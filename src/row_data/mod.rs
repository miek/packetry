//! Our GObject subclass for carrying a name and count for the ListBox model
//!
//! Both name and count are stored in a RefCell to allow for interior mutability
//! and are exposed via normal GObject properties. This allows us to use property
//! bindings below to bind the values with what widgets display in the UI

mod imp;

use std::cell::RefCell;
use std::rc::Rc;
use gtk::glib;
use gtk::subclass::prelude::*;
use crate::capture;
use crate::tree_list_model::TreeNode;

// Public part of the RowData type. This behaves like a normal gtk-rs-style GObject
// binding
glib::wrapper! {
    pub struct RowData(ObjectSubclass<imp::RowData>);
}
glib::wrapper! {
    pub struct DeviceRowData(ObjectSubclass<imp::DeviceRowData>);
}

impl RowData {
    pub fn new(node: Rc<RefCell<TreeNode>>) -> RowData
    {
        let mut row: RowData =
            glib::Object::new(&[]).expect("Failed to create row data");
        row.set_node(node);
        row
    }

    fn set_node(&mut self, node: Rc<RefCell<TreeNode>>) {
        self.imp().node.replace(Some(node));
    }

    pub fn get_node(&self) -> Rc<RefCell<TreeNode>> {
        self.imp().node.borrow().as_ref().unwrap().clone()
    }
}

impl DeviceRowData {
    pub fn new(item: Option<capture::DeviceItem>, summary: String) -> DeviceRowData {
        let mut row: DeviceRowData =
            glib::Object::new(&[]).expect("Failed to create row data");
        row.set_item(item);
        row.set_summary(summary);
        row
    }

    fn set_item(&mut self, item: Option<capture::DeviceItem>) {
        self.imp().item.replace(item);
    }

    fn set_summary(&mut self, summary: String) {
        self.imp().summary.replace(summary);
    }
}

pub trait GenericRowData<Item> {
    const CONNECTORS: bool;
    fn get_item(&self) -> Option<Item>;
    fn child_count(&self, capture: &mut capture::Capture)
        -> Result<u64, capture::CaptureError>;
    fn get_summary(&self) -> String;
    fn get_connectors(&self) -> Option<String>;
}

impl GenericRowData<capture::DeviceItem> for DeviceRowData {
    const CONNECTORS: bool = false;

    fn get_item(&self) -> Option<capture::DeviceItem> {
        self.imp().item.borrow().clone()
    }

    fn child_count(&self, capture: &mut capture::Capture)
        -> Result<u64, capture::CaptureError>
    {
        capture.device_item_count(&self.imp().item.borrow())
    }

    fn get_summary(&self) -> String {
        self.imp().summary.borrow().clone()
    }

    fn get_connectors(&self) -> Option<String> {
        None
    }
}
