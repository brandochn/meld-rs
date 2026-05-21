#![cfg(feature = "gui")]
//! Patch dialog for generating unified diff patches.
//!
//! Ported from the original `meld/patchdialog.py`.

use gtk4 as gtk;
use gtk4::prelude::*;

/// A dialog for generating and copying unified diff patches.
pub struct PatchDialog {
    dialog: gtk::Dialog,
    text_view: gtk::TextView,
}

impl PatchDialog {
    /// Create a new patch dialog with the given patch content.
    pub fn new(patch_content: &str) -> Self {
        let dialog = gtk::Dialog::new();
        dialog.set_title(Some("Format as Patch"));
        dialog.set_default_size(600, 400);
        dialog.add_button("Copy to Clipboard", gtk::ResponseType::Accept);
        dialog.add_button("Close", gtk::ResponseType::Close);

        let content = dialog.content_area();
        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_hexpand(true);

        let text_buffer = gtk::TextBuffer::new(None::<&gtk::TextTagTable>);
        text_buffer.set_text(patch_content);

        let text_view = gtk::TextView::with_buffer(&text_buffer);
        text_view.set_monospace(true);
        text_view.set_editable(false);
        text_view.set_wrap_mode(gtk::WrapMode::None);

        scrolled.set_child(Some(&text_view));
        content.append(&scrolled);

        let text_view_copy = text_view.clone();
        dialog.connect_response(move |d, resp| {
            if resp == gtk::ResponseType::Accept {
                copy_to_clipboard(&text_view_copy);
            }
            d.close();
        });

        Self { dialog, text_view }
    }
}

/// Generate a unified diff patch from two text buffers.
pub fn generate_patch(
    old_name: &str,
    new_name: &str,
    old_lines: &[String],
    new_lines: &[String],
) -> String {
    let mut patch = String::new();
    patch.push_str(&format!("--- {old_name}\n"));
    patch.push_str(&format!("+++ {new_name}\n"));

    use similar::TextDiff;
    let old_text = old_lines.join("\n");
    let new_text = new_lines.join("\n");

    let diff = TextDiff::from_lines(&old_text, &new_text);

    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            similar::ChangeTag::Equal => ' ',
            similar::ChangeTag::Delete => '-',
            similar::ChangeTag::Insert => '+',
        };

        for line in change.value().lines() {
            patch.push_str(&format!("{sign}{line}\n"));
        }
    }

    patch
}

fn copy_to_clipboard(text_view: &gtk::TextView) {
    let buffer = text_view.buffer();
    let start = buffer.start_iter();
    let end = buffer.end_iter();

    let text = buffer.text(&start, &end, true);
    if let Some(display) = gdk4::Display::default() {
        let clipboard = display.clipboard();
        clipboard.set_text(&text.to_string());
    }
}
