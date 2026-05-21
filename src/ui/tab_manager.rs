#![cfg(feature = "gui")]
//! Tab label with close button matching `notebook-label.ui`.
//!
//! Provides an `EventBox` with centered label and close button,
//! plus middle-click-to-close support.

use glib::prelude::*;
use gtk4 as gtk;
use gtk4::prelude::*;

/// A custom tab label with ellipsized text and a close button.
pub struct TabLabel {
    pub widget: gtk::Box,
    label: gtk::Label,
    close_button: gtk::Button,
}

impl TabLabel {
    /// Create a new tab label.
    pub fn new(text: &str) -> Self {
        let event_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);

        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 4);

        let label = gtk::Label::new(Some(text));
        label.set_ellipsize(pango::EllipsizeMode::Middle);
        label.set_width_request(150);
        label.set_single_line_mode(true);
        label.set_halign(gtk::Align::Center);
        label.set_hexpand(true);
        hbox.append(&label);

        // Close button (small, flat)
        let close_icon = gtk::Image::from_icon_name("window-close-symbolic");
        let close_button = gtk::Button::new();
        close_button.set_child(Some(&close_icon));
        close_button.set_has_frame(false);
        close_button.set_focus_on_click(false);
        close_button.set_tooltip_text(Some("Close Tab"));
        close_button.add_css_class("flat");
        close_button.add_css_class("small-button");
        hbox.append(&close_button);

        event_box.append(&hbox);

        Self {
            widget: event_box,
            label,
            close_button,
        }
    }

    /// Update the label text.
    pub fn set_text(&self, text: &str) {
        self.label.set_text(text);
    }

    /// Connect to the close button's clicked signal.
    pub fn connect_close<F: Fn() + 'static>(&self, callback: F) {
        self.close_button.connect_clicked(move |_| callback());
    }
}
