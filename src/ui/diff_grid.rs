//! Diff grid — manages side-by-side layout of diff panes.
//!
//! Composes multiple [`DiffView`] panels into a horizontal layout
//! with synchronised scrolling.

use gtk4 as gtk;
use gtk4::prelude::*;

use crate::ui::diff_view::DiffView;

/// A grid containing 2 or 3 [`DiffView`] panels side-by-side.
pub struct DiffGrid {
    container: gtk::Box,
    panes: Vec<DiffView>,
}

impl DiffGrid {
    /// Create a diff grid with the given number of panes.
    pub fn new(num_panes: usize) -> Self {
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let mut panes = Vec::with_capacity(num_panes);

        for i in 0..num_panes {
            let scrolled = gtk::ScrolledWindow::new();
            let diff_view = DiffView::new();

            scrolled.set_child(Some(diff_view.view()));
            container.append(&scrolled);

            // Last pane (merge result) is editable
            if i == num_panes - 1 {
                diff_view.view().set_editable(true);
            }

            panes.push(diff_view);
        }

        // Synchronise scrolling between first two panes
        if panes.len() >= 2 {
            sync_scroll_between(
                panes[0].view().upcast_ref::<gtk::TextView>(),
                panes[1].view().upcast_ref::<gtk::TextView>(),
            );
        }

        Self { container, panes }
    }

    /// Reference to the container widget.
    pub fn widget(&self) -> &gtk::Widget {
        self.container.upcast_ref()
    }

    /// Mutable reference to the container widget.
    pub fn widget_mut(&mut self) -> &mut gtk::Box {
        &mut self.container
    }

    /// Get a reference to a specific pane.
    pub fn pane(&self, index: usize) -> Option<&DiffView> {
        self.panes.get(index)
    }

    /// Set the content of a specific pane.
    pub fn set_pane_text(&self, index: usize, text: &str) {
        if let Some(view) = self.panes.get(index) {
            view.set_text(text);
        }
    }
}

/// Synchronise the vertical adjustment of two `gsv::View` widgets.
fn sync_scroll_between(left: &gtk::TextView, right: &gtk::TextView) {
    let adj_left = match left.vadjustment() {
        Some(a) => a,
        None => return,
    };
    let adj_right = match right.vadjustment() {
        Some(a) => a,
        None => return,
    };

    let adj_left_weak = adj_left.downgrade();
    adj_right.connect_value_changed(move |adj| {
        if let Some(target) = adj_left_weak.upgrade() {
            target.set_value(adj.value());
        }
    });

    let adj_right_weak = adj_right.downgrade();
    adj_left.connect_value_changed(move |adj| {
        if let Some(target) = adj_right_weak.upgrade() {
            target.set_value(adj.value());
        }
    });
}
