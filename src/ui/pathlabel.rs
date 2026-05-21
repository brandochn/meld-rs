#![cfg(feature = "gui")]
//! Path label widget with smart path shortening.
//!
//! Ported from the original `meld/ui/pathlabel.py`.

use gtk4 as gtk;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

/// A label that displays a file path, shortened intelligently (home-relative, parent-relative).
pub struct PathLabel {
    container: gtk::Box,
    label: gtk::Label,
    full_path: Rc<RefCell<String>>,
    home_dir: Rc<RefCell<Option<String>>>,
}

impl PathLabel {
    /// Create a new path label.
    pub fn new() -> Self {
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 4);

        let icon = gtk::Image::from_icon_name("folder-symbolic");
        container.append(&icon);

        let label = gtk::Label::new(None);
        label.set_ellipsize(pango::EllipsizeMode::Start);
        label.set_xalign(0.0);
        label.set_hexpand(true);
        container.append(&label);

        let home = dirs::home_dir().map(|p| p.to_string_lossy().into_owned());

        Self {
            container,
            label,
            full_path: Rc::new(RefCell::new(String::new())),
            home_dir: Rc::new(RefCell::new(home)),
        }
    }

    /// Underlying widget.
    pub fn widget(&self) -> &gtk::Widget {
        self.container.upcast_ref()
    }

    /// Set the displayed path.
    pub fn set_path(&self, path: &str) {
        self.full_path.replace(path.to_owned());
        let shortened = self.shorten(path);
        self.label.set_text(&shortened);
        self.label.set_tooltip_text(Some(path));
    }

    /// Get the full (unshortened) path.
    pub fn full_path(&self) -> String {
        self.full_path.borrow().clone()
    }

    fn shorten(&self, path: &str) -> String {
        // Home-relative
        if let Some(ref home) = *self.home_dir.borrow() {
            if let Ok(relative) = Path::new(path).strip_prefix(home) {
                return format!("~/{}", relative.to_string_lossy());
            }
        }

        // Just return as-is if no home dir
        path.to_owned()
    }
}

impl Default for PathLabel {
    fn default() -> Self {
        Self::new()
    }
}
