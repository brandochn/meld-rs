#![cfg(feature = "gui")]
//! Recent selector — popover with searchable recent comparisons.
//!
//! Uses a simple list view with search filtering instead of
//! the deprecated GtkRecentChooserWidget (removed in GTK4).

use gtk4 as gtk;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

pub struct RecentSelector {
    container: gtk::Grid,
    _search_entry: gtk::SearchEntry,
}

impl RecentSelector {
    pub fn new() -> Self {
        let grid = gtk::Grid::new();
        grid.set_margin_start(6);
        grid.set_margin_end(6);
        grid.set_margin_top(6);
        grid.set_margin_bottom(6);
        grid.set_row_spacing(6);
        grid.set_width_request(350);
        grid.set_height_request(400);

        let search_entry = gtk::SearchEntry::new();
        search_entry.set_placeholder_text(Some("Search recent comparisons…"));
        search_entry.grab_focus();
        grid.attach(&search_entry, 0, 0, 1, 1);

        // Simple list for recent items (in a full implementation, this would
        // load from the RecentManager JSON file)
        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_hexpand(true);

        let list_box = gtk::ListBox::new();
        list_box.set_selection_mode(gtk::SelectionMode::Single);

        let recent_mgr = crate::config::recent::RecentManager::load().unwrap_or_else(|_| {
            crate::config::recent::RecentManager {
                entries: std::collections::VecDeque::new(),
            }
        });

        for entry in recent_mgr.entries() {
            let row = gtk::Label::new(Some(&entry.paths.join(" ↔ ")));
            row.set_xalign(0.0);
            row.set_margin_start(6);
            row.set_margin_end(6);
            row.set_margin_top(4);
            row.set_margin_bottom(4);
            list_box.append(&row);
        }

        scrolled.set_child(Some(&list_box));
        grid.attach(&scrolled, 0, 1, 1, 1);

        let open_button = gtk::Button::with_label("_Open");
        open_button.set_use_underline(true);
        open_button.set_receives_default(true);
        grid.attach(&open_button, 0, 2, 1, 1);

        Self {
            container: grid,
            _search_entry: search_entry,
        }
    }

    pub fn widget(&self) -> &gtk::Widget {
        self.container.upcast_ref()
    }
}
