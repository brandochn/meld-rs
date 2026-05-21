//! Diff map sidebar.
//!
//! Provides a miniature overview of the diff results, showing coloured
//! blocks for inserted, deleted, and modified regions.

use gtk4 as gtk;
use gtk4::prelude::*;

use crate::diff::engine::DiffOp;

/// A drawing area that renders a miniature overview of diff chunks.
pub struct DiffMap {
    drawing_area: gtk::DrawingArea,
}

impl DiffMap {
    /// Creates a new diff map widget with the given diff chunks.
    pub fn new(chunks: &[crate::diff::engine::Chunk], total_lines: usize) -> Self {
        let drawing_area = gtk::DrawingArea::new();
        drawing_area.set_content_width(20);
        drawing_area.set_content_height(400);
        drawing_area.set_vexpand(true);

        let chunks_owned: Vec<DiffOp> = chunks.iter().map(|c| c.op).collect();
        let total = total_lines.max(1);

        drawing_area.set_draw_func(move |_, cr, width, height| {
            let w = width as f64;
            let h = height as f64;
            let line_h = h / total as f64;

            for (i, op) in chunks_owned.iter().enumerate() {
                let y = i as f64 * line_h;
                let colour = match op {
                    DiffOp::Equal => (0.5, 0.5, 0.5, 0.6),   // grey
                    DiffOp::Delete => (1.0, 0.3, 0.3, 0.8),  // red
                    DiffOp::Insert => (0.3, 1.0, 0.3, 0.8),  // green
                    DiffOp::Replace => (0.3, 0.3, 1.0, 0.8), // blue
                };

                cr.set_source_rgba(colour.0, colour.1, colour.2, colour.3);
                cr.rectangle(0.0, y, w, line_h.max(2.0));
                cr.fill().ok();
            }
        });

        Self { drawing_area }
    }

    /// The underlying `gtk::Widget`.
    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.drawing_area
    }
}
