#![cfg(feature = "gui")]
//! Full preferences dialog with tabs for General, Font, Filters, and Columns.
//!
//! Ported from the original `meld/preferences.py` (380 lines).

use gtk4 as gtk;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use crate::config::settings::{FilterEntry, MeldSettings};

/// Full preferences dialog with tabbed sections.
pub struct PreferencesDialog {
    dialog: gtk::Dialog,
    settings: Rc<RefCell<MeldSettings>>,
}

impl PreferencesDialog {
    /// Create a new preferences dialog loaded with current settings.
    pub fn new() -> Self {
        let settings = MeldSettings::load().unwrap_or_default();
        let settings_rc = Rc::new(RefCell::new(settings));

        let dialog = gtk::Dialog::new();
        dialog.set_title(Some("Preferences"));
        dialog.set_default_size(600, 480);
        dialog.add_button("Cancel", gtk::ResponseType::Cancel);
        dialog.add_button("OK", gtk::ResponseType::Ok);

        let content = dialog.content_area();
        let notebook = gtk::Notebook::new();
        notebook.set_scrollable(true);

        let general_page = build_general_page(&settings_rc);
        notebook.append_page(&general_page, Some(&gtk::Label::new(Some("General"))));

        let font_page = build_font_page(&settings_rc);
        notebook.append_page(&font_page, Some(&gtk::Label::new(Some("Font & Display"))));

        let filters_page = build_filters_page(&settings_rc);
        notebook.append_page(&filters_page, Some(&gtk::Label::new(Some("Filters"))));

        content.append(&notebook);

        let settings_save = Rc::clone(&settings_rc);
        dialog.connect_response(move |d, resp| {
            if resp == gtk::ResponseType::Ok {
                let _ = settings_save.borrow().save();
            }
            d.close();
        });

        Self {
            dialog,
            settings: settings_rc,
        }
    }

    /// Show the dialog.
    pub fn present(&self) {
        self.dialog.present();
    }

    /// Expose the underlying dialog for connecting external response handlers.
    pub fn dialog(&self) -> &gtk::Dialog {
        &self.dialog
    }
}

fn build_general_page(settings: &Rc<RefCell<MeldSettings>>) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    page.set_margin_top(12);
    page.set_margin_start(16);
    page.set_margin_end(16);

    // Dark theme toggle
    let dark_row = labeled_switch("Prefer dark theme", settings.borrow().prefer_dark_theme);
    let s_dark = Rc::clone(settings);
    dark_row.1.connect_state_set(move |_, state| {
        s_dark.borrow_mut().prefer_dark_theme = state;
        glib::Propagation::Proceed
    });
    page.append(&dark_row.0);

    // Show line numbers
    let ln_row = labeled_switch("Show line numbers", settings.borrow().show_line_numbers);
    let s_ln = Rc::clone(settings);
    ln_row.1.connect_state_set(move |_, state| {
        s_ln.borrow_mut().show_line_numbers = state;
        glib::Propagation::Proceed
    });
    page.append(&ln_row.0);

    // Highlight syntax
    let hs_row = labeled_switch("Highlight syntax", settings.borrow().highlight_syntax);
    let s_hs = Rc::clone(settings);
    hs_row.1.connect_state_set(move |_, state| {
        s_hs.borrow_mut().highlight_syntax = state;
        glib::Propagation::Proceed
    });
    page.append(&hs_row.0);

    // Ignore whitespace
    let ws_row = labeled_switch("Show whitespace", settings.borrow().enable_space_drawer);
    let s_ws = Rc::clone(settings);
    ws_row.1.connect_state_set(move |_, state| {
        s_ws.borrow_mut().enable_space_drawer = state;
        gtk::glib::Propagation::Stop
    });
    page.append(&ws_row.0);

    let wl_row = labeled_switch(
        "Wrap lines",
        !settings.borrow().wrap_mode.is_empty() && settings.borrow().wrap_mode != "none",
    );
    let s_wl = Rc::clone(settings);
    wl_row.1.connect_state_set(move |_, state| {
        s_wl.borrow_mut().wrap_mode = if state { "word".into() } else { "none".into() };
        gtk::glib::Propagation::Stop
    });
    page.append(&wl_row.0);

    let tw_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let tw_label = gtk::Label::new(Some("Indent width:"));
    tw_label.set_halign(gtk::Align::Start);
    tw_label.set_hexpand(true);
    tw_row.append(&tw_label);
    let tw_spin = gtk::SpinButton::with_range(1.0, 16.0, 1.0);
    tw_spin.set_value(settings.borrow().indent_width as f64);
    tw_row.append(&tw_spin);
    page.append(&tw_row);
    let s_tw = Rc::clone(settings);
    tw_spin.connect_value_changed(move |spin| {
        s_tw.borrow_mut().indent_width = spin.value() as i32;
    });

    // Font picker button
    let font_btn = gtk::Button::with_label("Choose Font...");
    font_btn.set_halign(gtk::Align::Start);
    page.append(&font_btn);

    // ── Diff section separator ──
    let diff_section = gtk::Label::new(Some("Diff Visualization"));
    diff_section.set_halign(gtk::Align::Start);
    diff_section.set_xalign(0.0);
    diff_section.set_margin_top(8);
    diff_section.add_css_class("heading");
    page.append(&diff_section);

    // Inline diff mode dropdown
    let id_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let id_label = gtk::Label::new(Some("Inline highlighting:"));
    id_label.set_halign(gtk::Align::Start);
    id_label.set_hexpand(true);
    id_row.append(&id_label);
    let modes = gtk::StringList::new(&["None", "Characters", "Tokens"]);
    let id_dropdown = gtk::DropDown::new(Some(modes), None::<&gtk::Expression>);
    let current_mode = settings.borrow().inline_diff_mode.clone();
    let selected = match current_mode.as_str() {
        "characters" => 1u32,
        "tokens" => 2u32,
        _ => 0u32,
    };
    id_dropdown.set_selected(selected);
    id_row.append(&id_dropdown);
    page.append(&id_row);
    let s_id = Rc::clone(settings);
    id_dropdown.connect_selected_notify(move |dd| {
        let mode = match dd.selected() {
            1 => "characters",
            2 => "tokens",
            _ => "none",
        };
        s_id.borrow_mut().inline_diff_mode = mode.to_string();
    });

    // Ignore blank lines
    let bl_row = labeled_switch(
        "Ignore blank lines in diffs",
        settings.borrow().ignore_blank_lines,
    );
    let s_bl = Rc::clone(settings);
    bl_row.1.connect_state_set(move |_, state| {
        s_bl.borrow_mut().ignore_blank_lines = state;
        glib::Propagation::Proceed
    });
    page.append(&bl_row.0);

    page
}

fn build_font_page(settings: &Rc<RefCell<MeldSettings>>) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    page.set_margin_top(12);
    page.set_margin_start(16);
    page.set_margin_end(16);

    // Use system font
    let sf_row = labeled_switch(
        "Use system monospace font",
        settings.borrow().use_system_font,
    );
    let s_sf = Rc::clone(settings);
    sf_row.1.connect_state_set(move |_, state| {
        s_sf.borrow_mut().use_system_font = state;
        glib::Propagation::Proceed
    });
    page.append(&sf_row.0);

    // Custom font entry
    let font_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let font_label = gtk::Label::new(Some("Custom font:"));
    font_label.set_halign(gtk::Align::Start);
    font_row.append(&font_label);

    let font_entry = gtk::Entry::new();
    font_entry.set_text(&settings.borrow().custom_font);
    font_entry.set_placeholder_text(Some("monospace 11"));
    font_entry.set_hexpand(true);
    font_row.append(&font_entry);
    page.append(&font_row);

    let s_font = Rc::clone(settings);
    font_entry.connect_changed(move |entry| {
        s_font.borrow_mut().custom_font = entry.text().to_string();
    });

    // Font picker button (future enhancement)
    let font_btn = gtk::Button::with_label("Choose Font...");
    font_btn.set_halign(gtk::Align::Start);
    page.append(&font_btn);

    page
}

fn build_filters_page(settings: &Rc<RefCell<MeldSettings>>) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    page.set_margin_top(12);
    page.set_margin_start(16);
    page.set_margin_end(16);

    // Text filters section
    let tf_label = gtk::Label::new(Some("Text Filters (regex patterns to ignore in diffs):"));
    tf_label.set_halign(gtk::Align::Start);
    tf_label.set_xalign(0.0);
    page.append(&tf_label);

    let tf_scrolled = gtk::ScrolledWindow::new();
    tf_scrolled.set_min_content_height(160);
    tf_scrolled.set_vexpand(true);
    let tf_list = build_filter_list(&settings.borrow().text_filters);
    tf_scrolled.set_child(Some(&tf_list));
    page.append(&tf_scrolled);

    let s_tf = Rc::clone(settings);
    wire_filter_list(&tf_list, s_tf, true);

    // Filename filters section
    let ff_label = gtk::Label::new(Some("Filename Filters (shell glob patterns):"));
    ff_label.set_halign(gtk::Align::Start);
    ff_label.set_xalign(0.0);
    ff_label.set_margin_top(8);
    page.append(&ff_label);

    let ff_scrolled = gtk::ScrolledWindow::new();
    ff_scrolled.set_min_content_height(140);
    ff_scrolled.set_vexpand(true);
    let ff_list = build_filter_list(&settings.borrow().filename_filters);
    ff_scrolled.set_child(Some(&ff_list));
    page.append(&ff_scrolled);

    let s_ff = Rc::clone(settings);
    wire_filter_list(&ff_list, s_ff, false);

    page
}

// ── Filter list helpers ───────────────────────────────────────────

/// Build a vertical list of filter rows, each with a checkbox, name, and pattern.
fn build_filter_list(entries: &[FilterEntry]) -> gtk::ListBox {
    let list = gtk::ListBox::new();
    list.add_css_class("rich-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    for entry in entries {
        let row = build_filter_row(entry);
        list.append(&row);
    }
    list
}

/// Build a single filter row: [✓] Name — Pattern
fn build_filter_row(entry: &FilterEntry) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_margin_top(4);
    row.set_margin_bottom(4);

    let check = gtk::CheckButton::new();
    check.set_active(entry.enabled);
    row.append(&check);

    let label = gtk::Label::new(Some(&format!("{} \u{2014} {}", entry.name, entry.pattern)));
    label.set_halign(gtk::Align::Start);
    label.set_xalign(0.0);
    label.set_ellipsize(pango::EllipsizeMode::End);
    row.append(&label);

    row
}

/// Wire up the filter list so that toggling checkboxes updates the backing
/// `Vec<FilterEntry>` in settings. Pass `is_text` = true for text_filters.
fn wire_filter_list(list: &gtk::ListBox, settings: Rc<RefCell<MeldSettings>>, is_text: bool) {
    let mut i = 0;
    while let Some(child) = list.row_at_index(i) {
        if let Some(row) = child.child().and_downcast::<gtk::Box>() {
            if let Some(check) = row.first_child().and_downcast::<gtk::CheckButton>() {
                let s = Rc::clone(&settings);
                let l = list.clone();
                let idx = i;
                check.connect_toggled(move |cb| {
                    let mut s = s.borrow_mut();
                    let filters: &mut Vec<FilterEntry> = if is_text {
                        &mut s.text_filters
                    } else {
                        &mut s.filename_filters
                    };
                    if let Some(entry) = filters.get_mut(idx as usize) {
                        entry.enabled = cb.is_active();
                    }
                    // Suppress unused
                    let _ = &l;
                });
            }
        }
        i += 1;
    }
}

/// Helper: creates a labeled switch row.
fn labeled_switch(label: &str, active: bool) -> (gtk::Box, gtk::Switch) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let lbl = gtk::Label::new(Some(label));
    lbl.set_halign(gtk::Align::Start);
    lbl.set_hexpand(true);
    row.append(&lbl);

    let switch = gtk::Switch::new();
    switch.set_active(active);
    switch.set_valign(gtk::Align::Center);
    row.append(&switch);

    (row, switch)
}
