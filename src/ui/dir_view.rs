//! Directory comparison view widget.
//!
//! Renders the results of a [`DirDiff`](crate::diff::dirdiff::DirDiff) scan
//! using a `GtkTreeView`.

use gtk4 as gtk;
use gtk4::prelude::*;

/// A tree view widget specialised for displaying directory comparison results.
pub struct DirView {
    tree_view: gtk::TreeView,
    tree_store: gtk::TreeStore,
}

impl DirView {
    /// Create a new empty directory comparison view.
    pub fn new() -> Self {
        let column_types: [glib::Type; 4] = [
            String::static_type(),
            String::static_type(),
            String::static_type(),
            String::static_type(),
        ];
        let tree_store = gtk::TreeStore::new(&column_types);
        let tree_view = gtk::TreeView::with_model(&tree_store);
        tree_view.set_headers_visible(true);
        tree_view.set_enable_search(true);

        let columns = ["Name", "State", "Size", "Modified"];
        for (i, col_name) in columns.iter().enumerate() {
            let renderer = gtk::CellRendererText::new();
            let column = gtk::TreeViewColumn::new();
            column.set_title(col_name);
            column.pack_start(&renderer, true);
            column.add_attribute(&renderer, "text", i as i32);
            tree_view.append_column(&column);
        }

        Self {
            tree_view,
            tree_store,
        }
    }

    /// Underlying `GtkTreeView` widget.
    pub fn widget(&self) -> &gtk::TreeView {
        &self.tree_view
    }

    /// Underlying `GtkTreeStore` model.
    pub fn store(&self) -> &gtk::TreeStore {
        &self.tree_store
    }

    /// Clear all rows and add new entries.
    pub fn populate(&self, rows: &[(String, String, String, String)]) {
        self.tree_store.clear();
        for (name, state, size, modified) in rows {
            let iter = self.tree_store.append(None);
            self.tree_store.set_value(&iter, 0, &name.to_value());
            self.tree_store.set_value(&iter, 1, &state.to_value());
            self.tree_store.set_value(&iter, 2, &size.to_value());
            self.tree_store.set_value(&iter, 3, &modified.to_value());
        }
    }
}

impl Default for DirView {
    fn default() -> Self {
        Self::new()
    }
}
