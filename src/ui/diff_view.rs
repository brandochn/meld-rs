//! Side-by-side diff view widget.
//!
//! Displays two (or three) source views with synchronised scrolling,
//! line numbers, and diff highlighting.

use gtk4 as gtk;
use gtk4::prelude::*;
use sourceview5 as gsv;
use sourceview5::prelude::*;

/// A diff view panel that wraps a `sourceview5::View`.
pub struct DiffView {
    view: gsv::View,
    buffer: gsv::Buffer,
}

impl DiffView {
    /// Create a new diff view panel.
    pub fn new() -> Self {
        let buffer = gsv::Buffer::new(None::<&gtk::TextTagTable>);
        buffer.set_highlight_syntax(true);

        let view = gsv::View::with_buffer(&buffer);
        view.set_show_line_numbers(true);
        view.set_monospace(true);
        view.set_wrap_mode(gtk::WrapMode::None);
        view.set_hexpand(true);
        view.set_vexpand(true);

        Self { view, buffer }
    }

    /// Underlying `gsv::View` widget.
    pub fn view(&self) -> &gsv::View {
        &self.view
    }

    /// Underlying `gsv::Buffer`.
    pub fn buffer(&self) -> &gsv::Buffer {
        &self.buffer
    }

    /// Set the text content of this panel.
    pub fn set_text(&self, text: &str) {
        self.buffer.set_text(text);
    }

    /// Returns the text content split into lines.
    pub fn text_lines(&self) -> Vec<String> {
        let start = self.buffer.start_iter();
        let end = self.buffer.end_iter();
        self.buffer
            .text(&start, &end, true)
            .to_string()
            .lines()
            .map(|l| l.to_owned())
            .collect()
    }
}

impl Default for DiffView {
    fn default() -> Self {
        Self::new()
    }
}
