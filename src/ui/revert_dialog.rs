#![cfg(feature = "gui")]
//! Revert dialog — confirmation dialog for VC revert operations.
//!
//! Mirrors the original revert dialog from `revert-dialog.ui`.

use gtk4 as gtk;
use gtk4::prelude::*;

use crate::tr;

/// A confirmation dialog for reverting version-controlled files.
pub struct RevertDialog {
    dialog: gtk::MessageDialog,
}

impl RevertDialog {
    /// Creates a new revert confirmation dialog.
    ///
    /// * `files` — list of file paths to revert.
    pub fn new(files: &[String]) -> Self {
        let file_list = files.join("\n");
        let message = if files.len() == 1 {
            tr!("Revert the following file?\n\n{files}").replace("{files}", &file_list)
        } else {
            tr!("Revert the following files?\n\n{files}").replace("{files}", &file_list)
        };

        let dialog = gtk::MessageDialog::new(
            None::<&gtk::Window>,
            gtk::DialogFlags::MODAL,
            gtk::MessageType::Warning,
            gtk::ButtonsType::OkCancel,
            &message,
        );
        dialog.set_title(Some(&tr!("Revert")));
        dialog.set_secondary_text(Some(&tr!("This action will discard all local changes.")));

        dialog.connect_response(|d, _| d.close());

        Self { dialog }
    }

    /// Show the dialog modally.
    pub fn present(&self) {
        self.dialog.present();
    }
}
