#![cfg(feature = "gui")]
//! Shared drag-and-drop helpers for file payloads.

use std::rc::Rc;

use gtk4 as gtk;
use gtk4::glib;

/// Application-level fallback used when a drop cannot be handled in place.
pub type FileDropFallback = Rc<dyn Fn(&[gio::File]) -> bool>;

/// Extract files from either a `GdkFileList` or a single `GFile` value.
pub fn files_from_drop_value(value: &glib::Value) -> Option<Vec<gio::File>> {
    if let Ok(list) = value.get::<gtk::gdk::FileList>() {
        let files = list.files();
        if !files.is_empty() {
            return Some(files);
        }
    }

    value.get::<gio::File>().ok().map(|file| vec![file])
}
