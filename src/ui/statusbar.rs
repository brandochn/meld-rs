#![cfg(feature = "gui")]
//! Status bar widget showing display options, encoding, source language and
//! the cursor position.
//!
//! Ported from the original `meld/ui/statusbar.py`.  The layout mirrors Meld:
//!
//! ```text
//!   [Display ▾]            <spacer>     UTF-8 | TypeScript | Ln 1, Col 1 | INS
//! ```
//!
//! The "Display" menu button exposes the common per-view toggles (wrap, line
//! numbers, highlight current line) wired directly to the [`gsv::View`].

use gtk4 as gtk;
use gtk4::prelude::*;
use sourceview5 as gsv;
use sourceview5::prelude::*;

use crate::ui::chunk_gutter::ChunkGutterRenderer;

/// A status bar showing display options, encoding, language and position for
/// a single source view.
pub struct StatusBar {
    container: gtk::Box,
    position_label: gtk::Label,
    encoding_label: gtk::Label,
    language_label: gtk::Label,
    overwrite_label: gtk::Label,
}

impl StatusBar {
    /// Create a new status bar bound to `view` and its custom line-number
    /// gutter `line_gutter` (both used by the Display menu).
    pub fn new(view: &gsv::View, line_gutter: &ChunkGutterRenderer) -> Self {
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        container.add_css_class("meld-status-bar");
        container.set_margin_start(6);
        container.set_margin_end(6);
        container.set_margin_top(2);
        container.set_margin_bottom(2);

        // ── Display menu (left) ──
        let display_button = gtk::MenuButton::new();
        display_button.set_label("Display");
        display_button.add_css_class("flat");
        display_button.set_focus_on_click(false);
        display_button.set_popover(Some(&build_display_popover(view, line_gutter)));

        // ── Spacer pushes the rest to the right, matching Meld ──
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);

        let encoding_label = gtk::Label::new(Some("UTF-8"));
        let language_label = gtk::Label::new(Some("Plain Text"));
        let position_label = gtk::Label::new(Some("Ln 1, Col 1"));
        let overwrite_label = gtk::Label::new(Some("INS"));
        overwrite_label.set_width_chars(3);

        container.append(&display_button);
        container.append(&spacer);
        container.append(&encoding_label);
        container.append(&sep());
        container.append(&language_label);
        container.append(&sep());
        container.append(&position_label);
        container.append(&sep());
        container.append(&overwrite_label);

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
            .set_text(&format!("Ln {}, Col {}", line, column));
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

/// A thin vertical separator between status-bar items.
fn sep() -> gtk::Separator {
    gtk::Separator::new(gtk::Orientation::Vertical)
}

/// Build the "Display" popover with view toggles wired to `view`.
fn build_display_popover(view: &gsv::View, line_gutter: &ChunkGutterRenderer) -> gtk::Popover {
    let popover = gtk::Popover::new();
    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 4);
    vbox.set_margin_start(8);
    vbox.set_margin_end(8);
    vbox.set_margin_top(8);
    vbox.set_margin_bottom(8);

    // Wrap lines
    let wrap = gtk::CheckButton::with_label("Wrap lines");
    wrap.set_active(view.wrap_mode() != gtk::WrapMode::None);
    {
        let view = view.clone();
        wrap.connect_toggled(move |b| {
            view.set_wrap_mode(if b.is_active() {
                gtk::WrapMode::Word
            } else {
                gtk::WrapMode::None
            });
        });
    }
    vbox.append(&wrap);

    // Show line numbers (toggles our custom chunk-aware gutter).
    let line_numbers = gtk::CheckButton::with_label("Show line numbers");
    line_numbers.set_active(line_gutter.is_visible());
    {
        let gutter = line_gutter.clone();
        line_numbers.connect_toggled(move |b| gutter.set_visible(b.is_active()));
    }
    vbox.append(&line_numbers);

    // Highlight current line
    let highlight = gtk::CheckButton::with_label("Highlight current line");
    highlight.set_active(view.is_highlight_current_line());
    {
        let view = view.clone();
        highlight.connect_toggled(move |b| view.set_highlight_current_line(b.is_active()));
    }
    vbox.append(&highlight);

    popover.set_child(Some(&vbox));
    popover
}
