#![cfg(feature = "gui")]
//! Full preferences dialog with tabs for General, Font, Filters, and Columns.
//!
//! Ported from the original `meld/preferences.py` (380 lines).
//! Settings are applied in real-time via a callback, matching the original
//! Meld's live GSettings binding behaviour.

use gtk4 as gtk;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use crate::config::settings::{FilterEntry, MeldSettings};

/// Callback invoked every time a setting is modified in the dialog.
/// The dialog saves to disk before calling this, so `apply_settings`
/// can simply read from the shared `RefCell` or reload from disk.
pub type SettingsChangedCallback = Box<dyn Fn()>;

/// Full preferences dialog with tabbed sections.  Modeless — settings
/// take effect immediately as the user changes them (no OK/Cancel).
pub struct PreferencesDialog {
    dialog: gtk::Window,
    settings: Rc<RefCell<MeldSettings>>,
    on_changed: Rc<RefCell<Option<SettingsChangedCallback>>>,
}

impl PreferencesDialog {
    /// Create a new preferences dialog.
    ///
    /// `on_changed` is called after every setting modification so that
    /// the caller can apply the new values to open documents.
    /// `parent` is the transient parent window (centers the dialog).
    pub fn new(on_changed: SettingsChangedCallback, parent: Option<&gtk::Window>) -> Self {
        let settings = MeldSettings::load().unwrap_or_default();
        let settings_rc = Rc::new(RefCell::new(settings));
        let on_changed_rc = Rc::new(RefCell::new(Some(on_changed)));

        let dialog = gtk::Window::new();
        dialog.set_title(Some("Preferences"));
        dialog.set_default_size(600, 480);
        dialog.set_transient_for(parent);

        let main_vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let notebook = gtk::Notebook::new();
        notebook.set_scrollable(true);
        notebook.set_vexpand(true);

        let general_page = build_general_page(&settings_rc, &on_changed_rc);
        notebook.append_page(&general_page, Some(&gtk::Label::new(Some("General"))));

        let font_page = build_font_page(&settings_rc, &on_changed_rc);
        notebook.append_page(&font_page, Some(&gtk::Label::new(Some("Font & Display"))));

        let filters_page = build_filters_page(&settings_rc, &on_changed_rc);
        notebook.append_page(&filters_page, Some(&gtk::Label::new(Some("Filters"))));

        main_vbox.append(&notebook);

        // Close button at the bottom, matching the old OK/Cancel layout
        let btn_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        btn_box.set_halign(gtk::Align::End);
        btn_box.set_margin_top(6);
        btn_box.set_margin_bottom(6);
        btn_box.set_margin_start(12);
        btn_box.set_margin_end(12);
        let close_btn = gtk::Button::with_label("Close");
        close_btn.set_focus_on_click(false);
        btn_box.append(&close_btn);
        main_vbox.append(&btn_box);

        dialog.set_child(Some(&main_vbox));

        let w = dialog.clone();
        close_btn.connect_clicked(move |_| w.close());

        // Save on close (belt-and-suspenders — every change already saves).
        let s = Rc::clone(&settings_rc);
        dialog.connect_close_request(move |_| {
            let _ = s.borrow().save();
            glib::Propagation::Proceed
        });

        Self {
            dialog,
            settings: settings_rc,
            on_changed: on_changed_rc,
        }
    }

    /// Show the dialog.
    pub fn present(&self) {
        self.dialog.present();
    }

    /// Returns the shared settings being edited by this dialog.
    pub fn settings(&self) -> &Rc<RefCell<MeldSettings>> {
        &self.settings
    }
}

// ── Helper: persist + notify ──────────────────────────────────────

fn notify(
    settings: &Rc<RefCell<MeldSettings>>,
    on_changed: &Rc<RefCell<Option<SettingsChangedCallback>>>,
) {
    let _ = settings.borrow().save();
    if let Some(ref cb) = *on_changed.borrow() {
        cb();
    }
}

// ── Page builders ──────────────────────────────────────────────────

fn build_general_page(
    settings: &Rc<RefCell<MeldSettings>>,
    on_changed: &Rc<RefCell<Option<SettingsChangedCallback>>>,
) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    page.set_margin_top(12);
    page.set_margin_start(16);
    page.set_margin_end(16);

    // Dark theme toggle
    let dark_row = labeled_switch("Prefer dark theme", settings.borrow().prefer_dark_theme);
    let s = Rc::clone(settings);
    let oc = Rc::clone(on_changed);
    dark_row.1.connect_state_set(move |_, state| {
        s.borrow_mut().prefer_dark_theme = state;
        notify(&s, &oc);
        glib::Propagation::Proceed
    });
    page.append(&dark_row.0);

    // Show line numbers
    let ln_row = labeled_switch("Show line numbers", settings.borrow().show_line_numbers);
    let s = Rc::clone(settings);
    let oc = Rc::clone(on_changed);
    ln_row.1.connect_state_set(move |_, state| {
        s.borrow_mut().show_line_numbers = state;
        notify(&s, &oc);
        glib::Propagation::Proceed
    });
    page.append(&ln_row.0);

    // Highlight syntax
    let hs_row = labeled_switch("Highlight syntax", settings.borrow().highlight_syntax);
    let s = Rc::clone(settings);
    let oc = Rc::clone(on_changed);
    hs_row.1.connect_state_set(move |_, state| {
        s.borrow_mut().highlight_syntax = state;
        notify(&s, &oc);
        glib::Propagation::Proceed
    });
    page.append(&hs_row.0);

    // Highlight current line
    let hcl_row = labeled_switch(
        "Highlight current line",
        settings.borrow().highlight_current_line,
    );
    let s = Rc::clone(settings);
    let oc = Rc::clone(on_changed);
    hcl_row.1.connect_state_set(move |_, state| {
        s.borrow_mut().highlight_current_line = state;
        notify(&s, &oc);
        glib::Propagation::Proceed
    });
    page.append(&hcl_row.0);

    // Show whitespace
    let ws_row = labeled_switch("Show whitespace", settings.borrow().enable_space_drawer);
    let s = Rc::clone(settings);
    let oc = Rc::clone(on_changed);
    ws_row.1.connect_state_set(move |_, state| {
        s.borrow_mut().enable_space_drawer = state;
        notify(&s, &oc);
        gtk::glib::Propagation::Stop
    });
    page.append(&ws_row.0);

    // Wrap lines
    let wl_row = labeled_switch(
        "Wrap lines",
        !settings.borrow().wrap_mode.is_empty() && settings.borrow().wrap_mode != "none",
    );
    let s = Rc::clone(settings);
    let oc = Rc::clone(on_changed);
    wl_row.1.connect_state_set(move |_, state| {
        s.borrow_mut().wrap_mode = if state { "word".into() } else { "none".into() };
        notify(&s, &oc);
        gtk::glib::Propagation::Stop
    });
    page.append(&wl_row.0);

    // Indent width
    let tw_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let tw_label = gtk::Label::new(Some("Indent width:"));
    tw_label.set_halign(gtk::Align::Start);
    tw_label.set_hexpand(true);
    tw_row.append(&tw_label);
    let tw_spin = gtk::SpinButton::with_range(1.0, 16.0, 1.0);
    tw_spin.set_value(settings.borrow().indent_width as f64);
    tw_row.append(&tw_spin);
    page.append(&tw_row);
    let s = Rc::clone(settings);
    let oc = Rc::clone(on_changed);
    tw_spin.connect_value_changed(move |spin| {
        s.borrow_mut().indent_width = spin.value() as i32;
        notify(&s, &oc);
    });

    // Diff section
    let diff_section = gtk::Label::new(Some("Diff Visualization"));
    diff_section.set_halign(gtk::Align::Start);
    diff_section.set_xalign(0.0);
    diff_section.set_margin_top(8);
    diff_section.add_css_class("heading");
    page.append(&diff_section);

    // Inline diff mode
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
    let s = Rc::clone(settings);
    let oc = Rc::clone(on_changed);
    id_dropdown.connect_selected_notify(move |dd| {
        let mode = match dd.selected() {
            1 => "characters",
            2 => "tokens",
            _ => "none",
        };
        s.borrow_mut().inline_diff_mode = mode.to_string();
        notify(&s, &oc);
    });

    // Ignore blank lines
    let bl_row = labeled_switch(
        "Ignore blank lines in diffs",
        settings.borrow().ignore_blank_lines,
    );
    let s = Rc::clone(settings);
    let oc = Rc::clone(on_changed);
    bl_row.1.connect_state_set(move |_, state| {
        s.borrow_mut().ignore_blank_lines = state;
        notify(&s, &oc);
        glib::Propagation::Proceed
    });
    page.append(&bl_row.0);

    page
}

fn build_font_page(
    settings: &Rc<RefCell<MeldSettings>>,
    on_changed: &Rc<RefCell<Option<SettingsChangedCallback>>>,
) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    page.set_margin_top(12);
    page.set_margin_start(16);
    page.set_margin_end(16);

    // Use system font
    let sf_row = labeled_switch(
        "Use system monospace font",
        settings.borrow().use_system_font,
    );
    let s = Rc::clone(settings);
    let oc = Rc::clone(on_changed);
    sf_row.1.connect_state_set(move |_, state| {
        s.borrow_mut().use_system_font = state;
        notify(&s, &oc);
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
    font_entry.set_placeholder_text(Some("monospace 12"));
    font_entry.set_hexpand(true);
    font_row.append(&font_entry);
    page.append(&font_row);

    let s = Rc::clone(settings);
    let oc = Rc::clone(on_changed);
    font_entry.connect_changed(move |entry| {
        s.borrow_mut().custom_font = entry.text().to_string();
        notify(&s, &oc);
    });

    page
}

fn build_filters_page(
    settings: &Rc<RefCell<MeldSettings>>,
    on_changed: &Rc<RefCell<Option<SettingsChangedCallback>>>,
) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    page.set_margin_top(12);
    page.set_margin_start(16);
    page.set_margin_end(16);

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

    let s = Rc::clone(settings);
    let oc = Rc::clone(on_changed);
    wire_filter_list(&tf_list, s, oc, true);

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

    let s = Rc::clone(settings);
    let oc = Rc::clone(on_changed);
    wire_filter_list(&ff_list, s, oc, false);

    page
}

// ── Filter list helpers ───────────────────────────────────────────

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

fn wire_filter_list(
    list: &gtk::ListBox,
    settings: Rc<RefCell<MeldSettings>>,
    on_changed: Rc<RefCell<Option<SettingsChangedCallback>>>,
    is_text: bool,
) {
    let mut i = 0;
    while let Some(child) = list.row_at_index(i) {
        if let Some(row) = child.child().and_downcast::<gtk::Box>() {
            if let Some(check) = row.first_child().and_downcast::<gtk::CheckButton>() {
                let s = Rc::clone(&settings);
                let oc = Rc::clone(&on_changed);
                let idx = i;
                // Clone `settings` before moving into the closure so the
                // outer loop can reuse it for the next row.
                let settings2 = Rc::clone(&settings);
                check.connect_toggled(move |cb| {
                    {
                        let mut s = s.borrow_mut();
                        let filters: &mut Vec<FilterEntry> = if is_text {
                            &mut s.text_filters
                        } else {
                            &mut s.filename_filters
                        };
                        if let Some(entry) = filters.get_mut(idx as usize) {
                            entry.enabled = cb.is_active();
                        }
                    }
                    notify(&settings2, &oc);
                });
            }
        }
        i += 1;
    }
}

// ── Widget helpers ─────────────────────────────────────────────────

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
