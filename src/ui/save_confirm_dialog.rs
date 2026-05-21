#![cfg(feature = "gui")]
//! Save confirm dialog — overwrite confirmation for save-as operations.
//!
//! Mirrors the original save confirmation from `save-confirm-dialog.ui`.

use gtk4 as gtk;
use gtk4::prelude::*;
use std::path::Path;

/// A confirmation dialog shown when overwriting an existing file.
pub struct SaveConfirmDialog {
    dialog: gtk::MessageDialog,
}

impl SaveConfirmDialog {
    /// Creates a new save confirmation dialog.
    ///
    /// * `path` — the file path that would be overwritten.
    pub fn new(path: &Path) -> Self {
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());

        let dialog = gtk::MessageDialog::new(
            None::<&gtk::Window>,
            gtk::DialogFlags::MODAL,
            gtk::MessageType::Question,
            gtk::ButtonsType::YesNo,
            &format!("A file named \"{filename}\" already exists."),
        );
        dialog.set_title(Some("Confirm Save"));
        dialog.set_secondary_text(Some("Do you want to replace it?"));

        dialog.connect_response(|d, _| d.close());

        Self { dialog }
    }

    /// Show the dialog modally.
    pub fn present(&self) {
        self.dialog.present();
    }
}
