//! 3-way merge view widget.
//!
//! Displays base, local, and remote file contents side-by-side
//! with a fourth editable pane for the merged result.

use gtk4 as gtk;
use gtk4::prelude::*;
use sourceview5 as gsv;
use sourceview5::prelude::*;

/// A 4-pane merge view: base, local, remote, and merged result.
pub struct MergeView {
    container: gtk::Box,
    panes: Vec<gsv::View>,
    buffers: Vec<gsv::Buffer>,
}

impl MergeView {
    /// Create a new merge view with three source panes and one result pane.
    pub fn new() -> Self {
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let mut panes = Vec::with_capacity(4);
        let mut buffers = Vec::with_capacity(4);

        let labels = ["Base", "Local", "Remote", "Merged"];

        for (i, label) in labels.iter().enumerate() {
            let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);

            let header_label = gtk::Label::new(Some(label));
            header_label.add_css_class("title-4");
            vbox.append(&header_label);

            let scrolled = gtk::ScrolledWindow::new();
            scrolled.set_vexpand(true);
            scrolled.set_hexpand(true);

            let buffer = gsv::Buffer::new(None::<&gtk::TextTagTable>);
            buffer.set_highlight_syntax(true);

            let view = gsv::View::with_buffer(&buffer);
            view.set_show_line_numbers(true);
            view.set_monospace(true);
            // Only the merged (last) pane is editable
            view.set_editable(i == 3);
            view.set_wrap_mode(gtk::WrapMode::None);

            scrolled.set_child(Some(&view));
            vbox.append(&scrolled);
            container.append(&vbox);

            panes.push(view);
            buffers.push(buffer);
        }

        Self {
            container,
            panes,
            buffers,
        }
    }

    /// Reference to the container widget.
    pub fn widget(&self) -> &gtk::Widget {
        self.container.upcast_ref()
    }

    /// Set the content for a single pane.
    pub fn set_pane_text(&self, index: usize, text: &str) {
        if let Some(buffer) = self.buffers.get(index) {
            buffer.set_text(text);
        }
    }

    /// Get the merged result text.
    pub fn merged_text(&self) -> Vec<String> {
        if let Some(buffer) = self.buffers.get(3) {
            let start = buffer.start_iter();
            let end = buffer.end_iter();
            buffer
                .text(&start, &end, true)
                .to_string()
                .lines()
                .map(|l| l.to_owned())
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Accept a change from a specific side into the merged result.
    ///
    /// `source_pane` should be 0 (base), 1 (local), or 2 (remote).
    pub fn accept_change(&self, source_pane: usize, start_line: usize, end_line: usize) {
        if source_pane >= 3 || source_pane >= self.buffers.len() {
            return;
        }
        if let (Some(src_buf), Some(dst_buf)) = (self.buffers.get(source_pane), self.buffers.get(3))
        {
            let src_text = src_buf
                .text(&src_buf.start_iter(), &src_buf.end_iter(), true)
                .to_string();
            let lines: Vec<&str> = src_text.lines().collect();
            let slice: Vec<&str> = lines
                .iter()
                .skip(start_line)
                .take(end_line.saturating_sub(start_line))
                .copied()
                .collect();
            dst_buf.set_text(&slice.join("\n"));
        }
    }
}
