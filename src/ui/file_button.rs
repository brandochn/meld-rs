#![cfg(feature = "gui")]
//! File chooser button — GtkButton that opens native file dialog.
//! GTK4 removed `FileChooserButton`; uses `FileDialog` pattern instead.

use gtk4 as gtk;
use gtk4::prelude::*;
use std::cell::Cell;

pub struct MeldFileButton {
    button: gtk::Button,
    pane: Cell<usize>,
    select_folder: bool,
    dialog_title: String,
}

impl MeldFileButton {
    pub fn new(pane: usize, select_folder: bool, title: &str) -> Self {
        let button = gtk::Button::new();
        let icon_name = if select_folder {
            "folder-open-symbolic"
        } else {
            "document-open-symbolic"
        };
        button.set_tooltip_text(Some(if select_folder {
            "Select folder"
        } else {
            "Open file"
        }));
        button.set_has_frame(false);
        let icon = gtk::Image::from_icon_name(icon_name);
        button.set_child(Some(&icon));
        button.set_can_focus(false);

        let this = Self {
            button,
            pane: Cell::new(pane),
            select_folder,
            dialog_title: title.to_owned(),
        };
        let sf = this.select_folder;
        let t = this.dialog_title.clone();
        let btn_weak = this.button.downgrade();
        this.button.connect_clicked(move |b| {
            let action = if sf {
                gtk::FileChooserAction::SelectFolder
            } else {
                gtk::FileChooserAction::Open
            };
            let parent = b.root().and_then(|r| r.downcast::<gtk::Window>().ok());
            let dialog = gtk::FileChooserNative::new(
                Some(&t),
                parent.as_ref(),
                action,
                Some("_Select"),
                Some("_Cancel"),
            );
            dialog.set_select_multiple(false);
            dialog.connect_response(|d, _| {
                let _ = d.file();
            });
            dialog.show();
        });
        this
    }
    pub fn widget(&self) -> &gtk::Widget {
        self.button.upcast_ref()
    }
    pub fn pane(&self) -> usize {
        self.pane.get()
    }
}
