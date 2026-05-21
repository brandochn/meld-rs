#![cfg(feature = "gui")]
//! Push dialog — simple confirmation dialog for VC push operations.
//!
//! Mirrors the original `PushDialog` from `push-dialog.ui` and `meld/ui/vcdialogs.py`.

use gtk4 as gtk;
use gtk4::prelude::*;

/// A simple confirmation dialog for pushing version control changes.
pub struct PushDialog {
    dialog: gtk::MessageDialog,
}

impl PushDialog {
    /// Creates a new push confirmation dialog.
    ///
    /// * `remote` — name of the remote to push to.
    pub fn new(remote: &str) -> Self {
        let dialog = gtk::MessageDialog::new(
            None::<&gtk::Window>,
            gtk::DialogFlags::MODAL,
            gtk::MessageType::Question,
            gtk::ButtonsType::OkCancel,
            &format!("Push changes to {remote}?"),
        );
        dialog.set_title(Some("Push"));

        dialog.connect_response(|d, _| d.close());

        Self { dialog }
    }

    /// Show the dialog modally.
    pub fn present(&self) {
        self.dialog.present();
    }
}
