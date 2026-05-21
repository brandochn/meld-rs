#![cfg(feature = "gui")]
//! Link map widget drawing bezier curves between corresponding diff lines.
//!
//! Ported from the original `meld/linkmap.py`. Renders visual connectors
//! between matching/changed line regions in two side-by-side diff panes.

use gtk4 as gtk;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use crate::diff::engine::{Chunk, DiffOp};

/// A drawing area that renders bezier curve connectors between matching
/// and differing lines in two diff panes.
pub struct LinkMap {
    drawing_area: gtk::DrawingArea,
    chunks: Rc<RefCell<Vec<Chunk>>>,
    total_lines_a: Rc<RefCell<usize>>,
    total_lines_b: Rc<RefCell<usize>>,
}

impl LinkMap {
    /// Create a new link map with the given chunks and line counts.
    pub fn new(chunks: &[Chunk], total_lines_a: usize, total_lines_b: usize) -> Self {
        let drawing_area = gtk::DrawingArea::new();
        drawing_area.set_content_width(40);
        drawing_area.set_vexpand(true);

        let chunks_rc = Rc::new(RefCell::new(chunks.to_vec()));
        let total_a = Rc::new(RefCell::new(total_lines_a.max(1)));
        let total_b = Rc::new(RefCell::new(total_lines_b.max(1)));

        let draw_chunks = Rc::clone(&chunks_rc);
        let draw_total_a = Rc::clone(&total_a);
        let draw_total_b = Rc::clone(&total_b);

        drawing_area.set_draw_func(move |_, cr, width, height| {
            let chunks = draw_chunks.borrow();
            let total_a = *draw_total_a.borrow();
            let total_b = *draw_total_b.borrow();
            let max_lines = total_a.max(total_b).max(1);

            let w = width as f64;
            let h = height as f64;

            // Background
            cr.set_source_rgba(0.95, 0.95, 0.95, 0.6);
            cr.paint().ok();

            for chunk in chunks.iter() {
                // Map chunk positions to pixel coordinates
                let y_from_start = (chunk.start_a as f64 / max_lines as f64) * h;
                let y_from_end = (chunk.end_a as f64 / max_lines as f64) * h;
                let y_to_start = (chunk.start_b as f64 / max_lines as f64) * h;
                let y_to_end = (chunk.end_b as f64 / max_lines as f64) * h;

                let (r, g, b) = match chunk.op {
                    DiffOp::Equal => (0.5, 0.5, 0.5),
                    DiffOp::Delete => (1.0, 0.3, 0.3),
                    DiffOp::Insert => (0.3, 1.0, 0.3),
                    DiffOp::Replace => (0.3, 0.3, 1.0),
                };

                cr.set_source_rgba(r, g, b, 0.5);
                cr.set_line_width(1.0);

                // Draw bezier connector from start to start
                if chunk.start_a < total_a && chunk.start_b < total_b {
                    cr.move_to(0.0, y_from_start);
                    let cp_y = (y_from_start + y_to_start) / 2.0;
                    cr.curve_to(w * 0.4, cp_y - 5.0, w * 0.6, cp_y + 5.0, w, y_to_start);
                    cr.stroke().ok();
                }

                // Draw bezier connector from end to end
                if chunk.end_a > chunk.start_a || chunk.end_b > chunk.start_b {
                    cr.move_to(0.0, y_from_end);
                    let cp_y = (y_from_end + y_to_end) / 2.0;
                    cr.curve_to(w * 0.4, cp_y - 5.0, w * 0.6, cp_y + 5.0, w, y_to_end);
                    cr.stroke().ok();

                    // For equal chunks, fill the region between start and end
                    if chunk.op == DiffOp::Equal && y_from_end > y_from_start + 1.0 {
                        cr.set_source_rgba(r, g, b, 0.15);
                        cr.move_to(0.0, y_from_start);
                        cr.curve_to(
                            w * 0.4,
                            (y_from_start + y_to_start) / 2.0,
                            w * 0.6,
                            (y_from_end + y_to_end) / 2.0,
                            w,
                            y_to_end,
                        );
                        cr.line_to(w, y_to_end);
                        // Draw back the other way
                        cr.curve_to(
                            w * 0.6,
                            (y_from_end + y_to_end) / 2.0 + 5.0,
                            w * 0.4,
                            (y_from_start + y_to_start) / 2.0 - 5.0,
                            0.0,
                            y_from_start,
                        );
                        cr.close_path();
                        cr.fill().ok();
                    }
                }
            }
        });

        Self {
            drawing_area,
            chunks: chunks_rc,
            total_lines_a: total_a,
            total_lines_b: total_b,
        }
    }

    /// Underlying widget.
    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.drawing_area
    }

    /// Update the chunks and line counts.
    pub fn update_chunks(&self, chunks: &[Chunk]) {
        self.chunks.replace(chunks.to_vec());
        self.drawing_area.queue_draw();
    }

    /// Update the line counts.
    pub fn update_line_counts(&self, total_a: usize, total_b: usize) {
        *self.total_lines_a.borrow_mut() = total_a.max(1);
        *self.total_lines_b.borrow_mut() = total_b.max(1);
        self.drawing_area.queue_draw();
    }
}
