#![cfg(feature = "gui")]
//! Status bar widget showing cursor position, encoding, and source language.
//!
//! Ported from the original `meld/ui/statusbar.py`.

use gtk4 as gtk;
use gtk4::prelude::*;

/// A status bar showing line:column, encoding, and language for a source view.
pub struct StatusBar {
    container: gtk::Box,
    position_label: gtk::Label,
    encoding_label: gtk::Label,
    language_label: gtk::Label,
    overwrite_label: gtk::Label,
}

impl StatusBar {
    /// Create a new status bar.
    pub fn new() -> Self {
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        container.add_css_class("statusbar");
        container.set_margin_start(6);
        container.set_margin_end(6);
        container.set_margin_top(2);
        container.set_margin_bottom(2);

        let position_label = gtk::Label::new(Some("Line 1, Col 1"));
        position_label.set_halign(gtk::Align::Start);

        let overwrite_label = gtk::Label::new(Some("INS"));
        overwrite_label.set_width_chars(3);

        let encoding_label = gtk::Label::new(Some("UTF-8"));
        encoding_label.set_halign(gtk::Align::End);
        encoding_label.set_hexpand(true);

        let language_label = gtk::Label::new(Some("Plain Text"));
        language_label.set_halign(gtk::Align::End);

        container.append(&position_label);
        container.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        container.append(&overwrite_label);
        container.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        container.append(&encoding_label);
        container.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        container.append(&language_label);

        Self {
            container,
            position_label,
            encoding_label,
            language_label,
            overwrite_label,
        }
    }

    /// Underlying widget.
    pub fn widget(&self) -> &gtk::Widget {
        self.container.upcast_ref()
    }

    /// Update the cursor position display.
    pub fn set_position(&self, line: u32, column: u32) {
        self.position_label
            .set_text(&format!("Line {}, Col {}", line, column));
    }

    /// Update the encoding display.
    pub fn set_encoding(&self, encoding: &str) {
        self.encoding_label.set_text(encoding);
    }

    /// Update the language display.
    pub fn set_language(&self, language: &str) {
        self.language_label.set_text(language);
    }

    /// Update the overwrite/insert mode indicator.
    pub fn set_overwrite(&self, overwrite: bool) {
        self.overwrite_label
            .set_text(if overwrite { "OVR" } else { "INS" });
    }
}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}
