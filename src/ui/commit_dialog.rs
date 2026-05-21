#![cfg(feature = "gui")]
//! Commit dialog — version control commit message editor.
//! Matches `commit-dialog.ui`.

use gtk4 as gtk;
use gtk4::prelude::*;
use sourceview5 as gsv;
use sourceview5::prelude::ViewExt;

pub struct CommitDialog {
    dialog: gtk::Dialog,
}

impl CommitDialog {
    pub fn new(files: &[String], margin: u32) -> Self {
        let dialog = gtk::Dialog::new();
        dialog.set_title(Some("Commit"));
        dialog.set_modal(true);
        dialog.set_default_size(450, 500);
        dialog.add_button("_Cancel", gtk::ResponseType::Cancel);
        dialog.add_button("Co_mmit", gtk::ResponseType::Ok);

        let content = dialog.content_area();
        content.set_spacing(18);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);

        let files_label = gtk::Label::new(Some("Commit Files"));
        files_label.set_xalign(0.0);
        files_label.set_markup("<b>Commit Files</b>");
        content.append(&files_label);

        let files_scrolled = gtk::ScrolledWindow::new();
        files_scrolled.set_min_content_height(100);
        let files_text = files.join("\n");
        let fl = gtk::Label::new(Some(&files_text));
        fl.set_xalign(0.0);
        fl.set_yalign(0.0);
        fl.set_wrap(true);
        fl.set_selectable(true);
        files_scrolled.set_child(Some(&fl));
        content.append(&files_scrolled);

        let log_label = gtk::Label::new(Some("Log Message"));
        log_label.set_xalign(0.0);
        log_label.set_markup("<b>Log Message</b>");
        content.append(&log_label);

        let log_scrolled = gtk::ScrolledWindow::new();
        log_scrolled.set_min_content_height(150);
        log_scrolled.set_vexpand(true);

        let buffer = gsv::Buffer::new(None::<&gtk::TextTagTable>);
        let commit_view = gsv::View::with_buffer(&buffer);
        commit_view.set_wrap_mode(gtk::WrapMode::Word);
        commit_view.set_monospace(true);
        commit_view.set_show_line_numbers(false);
        commit_view.set_right_margin_position(margin);
        commit_view.set_show_right_margin(true);

        log_scrolled.set_child(Some(&commit_view));
        content.append(&log_scrolled);

        dialog.connect_response(|d, _| d.close());

        Self { dialog }
    }

    pub fn present(&self) {
        self.dialog.present();
    }
}
