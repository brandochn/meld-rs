#![cfg(feature = "gui")]
//! Diff overview map (chunk map).
//!
//! A narrow strip rendered to the right of the diff panes that gives a
//! miniature overview of the whole file: every changed chunk is drawn as a
//! coloured block scaled to its position in the document, and the currently
//! visible viewport is shown as a translucent handle.  Clicking or dragging
//! on the map scrolls the associated text view.
//!
//! Mirrors the original Meld `meld/chunkmap.py`.

use gtk4 as gtk;
use gtk4::prelude::*;
use sourceview5 as gsv;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::diff::engine::{Chunk, DiffOp};
use crate::ui::style;

/// Horizontal padding (in px) around the coloured blocks, matching Meld's
/// `overdraw_padding`.
const PADDING: f64 = 2.0;

/// A drawing area that renders a miniature overview of the diff chunks for a
/// single pane and lets the user scroll the pane by clicking on it.
pub struct DiffMap {
    drawing_area: gtk::DrawingArea,
}

impl DiffMap {
    /// Create an overview map for `view`, displaying the chunks that belong to
    /// pane `pane` (0 = left/A, 1 = right/B).  `chunks` is the shared chunk
    /// list owned by the `FileDiff`; the map re-reads it on every redraw so it
    /// always reflects the latest diff.
    pub fn new(view: &gsv::View, pane: usize, chunks: Rc<RefCell<Vec<Chunk>>>) -> Self {
        let drawing_area = gtk::DrawingArea::new();
        drawing_area.set_width_request(16);
        drawing_area.set_vexpand(true);
        drawing_area.add_css_class("chunkmap");

        // ── Draw ──────────────────────────────────────────────────
        let draw_chunks = Rc::clone(&chunks);
        let draw_view = view.clone();
        drawing_area.set_draw_func(move |_, cr, width, height| {
            let w = width as f64;
            let h = height as f64;
            if w < 2.0 || h < 2.0 {
                return;
            }

            let buf = draw_view.buffer();
            let total = (buf.line_count().max(1)) as f64;

            let x0 = PADDING + 0.5;
            let x1 = (w - 2.0 * x0).max(1.0);

            cr.set_line_width(1.0);

            // ── Coloured chunk blocks ─────────────────────────────
            for chunk in draw_chunks.borrow().iter() {
                if chunk.op == DiffOp::Equal {
                    continue;
                }
                let (cs, ce) = if pane == 0 {
                    (chunk.start_a, chunk.end_a)
                } else {
                    (chunk.start_b, chunk.end_b)
                };

                let fill = match style::fill_color(chunk.op) {
                    Some(c) => c,
                    None => continue,
                };
                let line = style::line_color(chunk.op).unwrap_or(fill);

                let mut y0 = (cs as f64 / total) * h;
                // A zero-span chunk (an insertion/deletion that has no lines
                // on this side) still gets a thin visible marker.
                let mut y1 = if ce > cs {
                    (ce as f64 / total) * h
                } else {
                    y0 + 2.0
                };
                y0 = y0.round() + 0.5;
                y1 = (y1.round() - 0.5).max(y0 + 1.0);

                cr.rectangle(x0, y0, x1, y1 - y0);
                cr.set_source_rgb(fill.0, fill.1, fill.2);
                cr.fill_preserve().ok();
                cr.set_source_rgb(line.0, line.1, line.2);
                cr.stroke().ok();
            }

            // ── Scroll-position handle ────────────────────────────
            if let Some(adj) = draw_view.vadjustment() {
                let upper = adj.upper();
                let page = adj.page_size();
                if upper > 0.0 {
                    let hy0 = (adj.value() / upper) * h;
                    let hy1 = ((adj.value() + page) / upper) * h;
                    let hh = (hy1 - hy0).max(1.0);
                    cr.rectangle(x0 - 0.5, hy0 + 0.5, x1 + 1.0, hh - 1.0);
                    cr.set_source_rgba(0.0, 0.0, 0.0, 0.2);
                    cr.fill_preserve().ok();
                    cr.set_source_rgba(0.0, 0.0, 0.0, 0.4);
                    cr.stroke().ok();
                }
            }
        });

        // ── Redraw on scroll ──────────────────────────────────────
        if let Some(adj) = view.vadjustment() {
            let da = drawing_area.clone();
            adj.connect_value_changed(move |_| da.queue_draw());
            let da = drawing_area.clone();
            adj.connect_changed(move |_| da.queue_draw());
        }

        // ── Click / drag to scroll ────────────────────────────────
        let pressed = Rc::new(Cell::new(false));

        let scroll_to = {
            let view = view.clone();
            move |y: f64, height: f64| {
                if let Some(adj) = view.vadjustment() {
                    let upper = adj.upper();
                    let page = adj.page_size();
                    if upper <= 0.0 || height <= 0.0 {
                        return;
                    }
                    let frac = (y / height).clamp(0.0, 1.0);
                    let target = (frac * upper - page / 2.0).clamp(0.0, (upper - page).max(0.0));
                    adj.set_value(target);
                }
            }
        };

        let click = gtk::GestureClick::new();
        {
            let pressed = Rc::clone(&pressed);
            let scroll_to = scroll_to.clone();
            let da = drawing_area.clone();
            click.connect_pressed(move |_, _, _, y| {
                pressed.set(true);
                scroll_to(y, da.height() as f64);
            });
        }
        {
            let pressed = Rc::clone(&pressed);
            click.connect_released(move |_, _, _, _| pressed.set(false));
        }
        drawing_area.add_controller(click);

        let motion = gtk::EventControllerMotion::new();
        {
            let pressed = Rc::clone(&pressed);
            let scroll_to = scroll_to.clone();
            let da = drawing_area.clone();
            motion.connect_motion(move |_, _, y| {
                if pressed.get() {
                    scroll_to(y, da.height() as f64);
                }
            });
        }
        drawing_area.add_controller(motion);

        Self { drawing_area }
    }

    /// The underlying widget.
    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.drawing_area
    }

    /// Queue a redraw (call after the chunk list changes).
    pub fn queue_draw(&self) {
        self.drawing_area.queue_draw();
    }
}
