#[macro_use]
extern crate bitfield;
use thiserror::Error;

mod model;
pub mod row_data;
use row_data::RowData;
mod expander;

use std::sync::{Arc, Mutex};

use gtk::{
    prelude::*,
    ListView,
    Label,
    SignalListItemFactory,
    SingleSelection,
};
use expander::ExpanderWrapper;

mod capture;
use capture::{Capture, CaptureError};

mod decoder;
use decoder::Decoder;

mod tree_list_model;
use tree_list_model::TreeListModel;

mod id;
mod file_vec;
mod hybrid_index;
mod usb;

#[derive(Error, Debug)]
pub enum PacketryError {
    #[error(transparent)]
    CaptureError(#[from] CaptureError),
    #[error(transparent)]
    PcapError(#[from] pcap::Error),
}

fn run() -> Result<(), PacketryError> {
    let application = gtk::Application::new(
        Some("com.greatscottgadgets.packetry"),
        Default::default(),
    );

    let args: Vec<_> = std::env::args().collect();
    let mut pcap = pcap::Capture::from_file(&args[1])?;
    let mut cap = Capture::new()?;
    let mut decoder = Decoder::new(&mut cap)?;
    while let Ok(packet) = pcap.next() {
        decoder.handle_raw_packet(&packet)?;
    }
    cap.print_storage_summary();
    let capture = Arc::new(Mutex::new(cap));

    application.connect_activate(move |application| {
        let window = gtk::ApplicationWindow::builder()
            .default_width(320)
            .default_height(480)
            .application(application)
            .title("Packetry")
            .build();

        let tree_model = TreeListModel::new(capture.clone());
        let selection_model = SingleSelection::new(Some(&tree_model));
        let factory = SignalListItemFactory::new();

        factory.connect_setup(move |_, list_item| {
            let expander = ExpanderWrapper::new();
            list_item.set_child(Some(&expander));
        });

        let cap_arc = capture.clone();

        factory.connect_bind(move |_, list_item| {
            let row = list_item
                .item()
                .expect("The item has to exist.")
                .downcast::<RowData>()
                .expect("The item has to be RowData.");

            let container = list_item
                .child()
                .expect("The child has to exist");

            let text_label = container
                .last_child()
                .expect("The child has to exist")
                .downcast::<Label>()
                .expect("The child must be a Label.");

            let node = row.get_node();
            let mut cap = cap_arc.lock().unwrap();

            let summary = cap.get_summary(&node.borrow().item()).unwrap();
            text_label.set_text(&summary);

            let expander_wrapper = container
                .downcast::<ExpanderWrapper>()
                .expect("The child must be a ExpanderWrapper.");

            let connectors = cap.get_connectors(&node.borrow().item()).unwrap();
            expander_wrapper.set_connectors(Some(connectors));
            let expander = expander_wrapper.expander();
            expander.set_visible(true);
            expander.set_expanded(gtk::subclass::prelude::ObjectSubclassIsExt::imp(&tree_model).expanded(node.clone()));
            let model = tree_model.clone();
            let handler = expander.connect_expanded_notify(move |expander| {
                gtk::subclass::prelude::ObjectSubclassIsExt::imp(&model).set_expanded(node.clone(), expander.is_expanded());
            });
            expander_wrapper.set_handler(handler);
        });

        factory.connect_unbind(move |_, list_item| {
            let container = list_item
                .child()
                .expect("The child has to exist");

            let expander_wrapper = container
                .downcast::<ExpanderWrapper>()
                .expect("The child must be a ExpanderWrapper.");

            let expander = expander_wrapper.expander();
            match expander_wrapper.take_handler() {
                Some(handler) => expander.disconnect(handler),
                None => panic!("Handler was not set")
            };
        });

        let listview = ListView::new(Some(&selection_model), Some(&factory));

        let scrolled_window = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic) // Disable horizontal scrolling
            .min_content_height(480)
            .min_content_width(640)
            .build();

        scrolled_window.set_child(Some(&listview));
        window.set_child(Some(&scrolled_window));
        window.show();
    });
    application.run_with_args::<&str>(&[]);
    Ok(())
}

fn main() {
    match run() {
        Ok(()) => {},
        Err(e) => println!("Error: {:?}", e)
    }
}
