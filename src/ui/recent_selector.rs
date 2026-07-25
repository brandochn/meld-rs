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
    list_box: gtk::ListBox,
    _search_entry: gtk::SearchEntry,
    on_open: Rc<RefCell<Option<Box<dyn Fn(Vec<String>)>>>>,
}

impl RecentSelector {
    pub fn new<F: Fn(Vec<String>) + 'static>(on_open: F) -> Self {
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

        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_hexpand(true);

        let list_box = gtk::ListBox::new();
        list_box.set_selection_mode(gtk::SelectionMode::Single);
        scrolled.set_child(Some(&list_box));
        grid.attach(&scrolled, 0, 1, 1, 1);

        let open_button = gtk::Button::with_label("_Open");
        open_button.set_use_underline(true);
        open_button.set_receives_default(true);
        grid.attach(&open_button, 0, 2, 1, 1);

        let cb_holder: Rc<RefCell<Option<Box<dyn Fn(Vec<String>)>>>> = Rc::new(RefCell::new(None));
        let on_open_clone = Rc::clone(&cb_holder);

        let lb = list_box.clone();
        open_button.connect_clicked(move |_| {
            if let Some(row) = lb.selected_row() {
                if let Some(ref cb) = *on_open_clone.borrow() {
                    if let Some(label) = row.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
                        let text = label.text().to_string();
                        let paths: Vec<String> = text.split(" ↔ ").map(|s| s.to_string()).collect();
                        cb(paths);
                    }
                }
            }
        });

        let mut selector = Self {
            container: grid,
            list_box,
            _search_entry: search_entry,
            on_open: Rc::clone(&cb_holder),
        };

        *cb_holder.borrow_mut() = Some(Box::new(on_open));
        selector.reload();

        selector
    }

    pub fn widget(&self) -> &gtk::Widget {
        self.container.upcast_ref()
    }

    /// Reload the list from the recent comparisons JSON file.
    pub fn reload(&self) {
        // Clear existing rows
        while let Some(row) = self.list_box.first_child() {
            self.list_box.remove(&row);
        }

        // Load from disk
        match crate::config::recent::RecentManager::load() {
            Ok(recent_mgr) => {
                let count = recent_mgr.entries.len();
                log::info!("RecentSelector: loaded {count} entries from disk");
                for entry in recent_mgr.entries() {
                    let row = gtk::Label::new(Some(&entry.paths.join(" ↔ ")));
                    row.set_xalign(0.0);
                    row.set_margin_start(6);
                    row.set_margin_end(6);
                    row.set_margin_top(4);
                    row.set_margin_bottom(4);
                    self.list_box.append(&row);
                }
            }
            Err(e) => {
                log::warn!("RecentSelector: failed to load: {e}");
            }
        }
    }
}
