#![cfg(feature = "gui")]
//! Message area widget using a simple label with CSS styling.
//! GTK4 `InfoBar` API changed; this provides a simplified version.

use gtk4 as gtk;
use gtk4::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgType {
    Info,
    Warning,
    Error,
}

pub struct MsgArea {
    container: gtk::Box,
    label: gtk::Label,
}

impl MsgArea {
    pub fn new() -> Self {
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        container.set_visible(false);
        container.set_hexpand(true);
        container.set_margin_start(6);
        container.set_margin_end(6);
        container.set_margin_top(2);
        container.set_margin_bottom(2);
        container.set_css_classes(&["toolbar", "meld-msgarea"]);

        let label = gtk::Label::new(None);
        label.set_wrap(true);
        label.set_xalign(0.0);
        label.set_hexpand(true);
        container.append(&label);

        Self { container, label }
    }

    pub fn widget(&self) -> &gtk::Widget {
        self.container.upcast_ref()
    }

    pub fn show_info(&self, msg: &str) {
        self.show_msg(msg);
    }
    pub fn show_warning(&self, msg: &str) {
        self.show_msg(msg);
    }
    pub fn show_error(&self, msg: &str) {
        self.show_msg(msg);
    }
    pub fn hide(&self) {
        self.container.set_visible(false);
    }

    fn show_msg(&self, msg: &str) {
        self.label.set_text(msg);
        self.container.set_visible(true);
    }
}

impl Default for MsgArea {
    fn default() -> Self {
        Self::new()
    }
}
