#![cfg(feature = "gui")]
//! Link map widget drawing bezier curves between corresponding diff lines.
//!
//! Ported from the original `meld/linkmap.py`. Renders visual connectors
//! between matching/changed line regions in two side-by-side diff panes.
//!
//! Features:
//!   - Filled bezier curves for Equal chunks (matching Meld's continuous
//!     visual linking)
//!   - Stroked bezier curves for Delete/Insert/Replace chunks
//!   - Dotted amber connectors for cross-line similarity matches
//!   - Viewport culling (only draws curves for the visible region)

use gtk4 as gtk;
use gtk4::prelude::*;
use sourceview5 as gsv;
use std::cell::RefCell;
use std::rc::Rc;

use crate::diff::engine::{Chunk, DiffOp};
use crate::diff::movement::MoveMap;
use crate::diff::similarity::SimilarityMap;

pub struct LinkMap {
    drawing_area: gtk::DrawingArea,
    chunks: Rc<RefCell<Vec<Chunk>>>,
    total_lines_a: Rc<RefCell<usize>>,
    total_lines_b: Rc<RefCell<usize>>,
    similarity: Rc<RefCell<Vec<SimilarityLink>>>,
    moves: Rc<RefCell<Vec<MoveLink>>>,
    token_relations: Rc<RefCell<Vec<TokenRelation>>>,
    /// Optional references to left/right text views for viewport culling.
    left_view: Rc<RefCell<Option<gsv::View>>>,
    right_view: Rc<RefCell<Option<gsv::View>>>,
}

#[derive(Debug, Clone)]
struct SimilarityLink {
    left_line: usize,
    right_line: usize,
    score: f64,
}

#[derive(Debug, Clone)]
struct MoveLink {
    left_start: usize,
    left_end: usize,
    right_start: usize,
    right_end: usize,
    score: f64,
}

/// A token-level relation linking a moved identifier's position on the
/// left pane to its position on the right pane.
#[derive(Debug, Clone)]
pub struct TokenRelation {
    pub left_line: usize,
    pub left_col_start: usize,
    pub left_col_end: usize,
    pub right_line: usize,
    pub right_col_start: usize,
    pub right_col_end: usize,
    /// Character offset of the token start in the left buffer.
    pub left_offset_start: usize,
    /// Character offset of the token end in the left buffer.
    pub left_offset_end: usize,
    /// Character offset of the token start in the right buffer.
    pub right_offset_start: usize,
    /// Character offset of the token end in the right buffer.
    pub right_offset_end: usize,
}

/// Pixel layout for a single token, computed from the text view.
#[derive(Debug, Clone, Copy)]
struct TokenLayout {
    /// Buffer-space Y (pixels from top of buffer).
    buffer_y: f64,
    /// Line height in pixels.
    height: f64,
}

impl LinkMap {
    pub fn new(chunks: &[Chunk], total_lines_a: usize, total_lines_b: usize) -> Self {
        let drawing_area = gtk::DrawingArea::new();
        drawing_area.set_content_width(40);
        drawing_area.set_content_height(100);
        drawing_area.set_hexpand(true);
        drawing_area.set_vexpand(true);

        let chunks_rc = Rc::new(RefCell::new(chunks.to_vec()));
        let total_a = Rc::new(RefCell::new(total_lines_a.max(1)));
        let total_b = Rc::new(RefCell::new(total_lines_b.max(1)));
        let similarity = Rc::new(RefCell::new(Vec::new()));
        let moves = Rc::new(RefCell::new(Vec::new()));
        let token_relations = Rc::new(RefCell::new(Vec::new()));
        let left_view: Rc<RefCell<Option<gsv::View>>> = Rc::new(RefCell::new(None));
        let right_view: Rc<RefCell<Option<gsv::View>>> = Rc::new(RefCell::new(None));

        let draw_chunks = Rc::clone(&chunks_rc);
        let draw_total_a = Rc::clone(&total_a);
        let draw_total_b = Rc::clone(&total_b);
        let draw_similarity = Rc::clone(&similarity);
        let draw_moves = Rc::clone(&moves);
        let draw_tokens = Rc::clone(&token_relations);
        let draw_left_view = Rc::clone(&left_view);
        let draw_right_view = Rc::clone(&right_view);

        drawing_area.set_draw_func(move |_da, cr, width, height| {
            // ── FORCED DEBUG: verify rendering pipeline is active ──
            eprintln!("LINKMAP DRAW w={} h={}", width, height);

            let w = width as f64;
            let h = height as f64;

            let chunks = draw_chunks.borrow();
            let total_a = *draw_total_a.borrow();
            let total_b = *draw_total_b.borrow();
            let max_lines = total_a.max(total_b).max(1);
            let sim_entries: std::cell::Ref<'_, Vec<SimilarityLink>> = draw_similarity.borrow();
            let move_entries: std::cell::Ref<'_, Vec<MoveLink>> = draw_moves.borrow();
            let token_entries: std::cell::Ref<'_, Vec<TokenRelation>> = draw_tokens.borrow();
            let left_view_opt = draw_left_view.borrow();
            let right_view_opt = draw_right_view.borrow();

            // Determine visible line ranges for viewport culling
            let (vis_start, vis_end) =
                if let (Some(lv), Some(rv)) = (left_view_opt.as_ref(), right_view_opt.as_ref()) {
                    let l_rect = lv.visible_rect();
                    let r_rect = rv.visible_rect();
                    let l_buf = lv.buffer();
                    let r_buf = rv.buffer();
                    let l_total = l_buf.line_count().max(1) as f64;
                    let r_total = r_buf.line_count().max(1) as f64;
                    let l_rect_h = l_rect.height() as f64;
                    let l_rect_y = l_rect.y() as f64;
                    let r_rect_h = r_rect.height() as f64;
                    let r_rect_y = r_rect.y() as f64;
                    let l_start = (l_rect_y / l_rect_h.max(1.0) * l_total) as usize;
                    let l_end = ((l_rect_y + l_rect_h) / l_rect_h.max(1.0) * l_total) as usize;
                    let r_start = (r_rect_y / r_rect_h.max(1.0) * r_total) as usize;
                    let r_end = ((r_rect_y + r_rect_h) / r_rect_h.max(1.0) * r_total) as usize;
                    let start = l_start.min(r_start);
                    let end = l_end.max(r_end);
                    (start.saturating_sub(5), end + 5)
                } else {
                    (0, max_lines)
                };

            // Background
            cr.set_source_rgba(0.95, 0.95, 0.95, 0.6);
            cr.paint().ok();

            // ── FORCED DEBUG: verify rendering pipeline is active ──
            // Drawn AFTER background so it's visible on top.
            cr.set_source_rgb(1.0, 0.0, 0.0);
            cr.set_line_width(3.0);
            cr.rectangle(2.0, 2.0, w - 4.0, h - 4.0);
            cr.stroke().ok();

            for chunk in chunks.iter() {
                // Viewport culling: skip chunks outside the visible range
                if chunk.end_a < vis_start && chunk.end_b < vis_start {
                    continue;
                }
                if chunk.start_a > vis_end && chunk.start_b > vis_end {
                    continue;
                }

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

                let has_from = chunk.end_a > chunk.start_a;
                let has_to = chunk.end_b > chunk.start_b;

                if !has_from && !has_to {
                    continue;
                }

                // Draw filled bezier shape (matching original Meld's LinkMap)
                let y0 = y_from_start.max(0.0);
                let y1 = y_from_end.min(h);
                let t0 = y_to_start.max(0.0);
                let t1 = y_to_end.min(h);

                let x_left = 0.0;
                let x_mid = w / 2.0;
                let x_right = w;

                // Fill the connected region
                cr.set_source_rgba(r, g, b, 0.2);
                cr.move_to(x_left, y0);
                cr.curve_to(x_mid, y0, x_mid, t0, x_right, t0);
                cr.line_to(x_right, t1);
                cr.curve_to(x_mid, t1, x_mid, y1, x_left, y1);
                cr.close_path();
                cr.fill().ok();

                // Stroke the outline
                cr.set_source_rgba(r, g, b, 0.5);
                cr.set_line_width(1.0);
                cr.move_to(x_left, y0);
                cr.curve_to(x_mid, y0, x_mid, t0, x_right, t0);
                cr.stroke().ok();
                cr.move_to(x_left, y1);
                cr.curve_to(x_mid, y1, x_mid, t1, x_right, t1);
                cr.stroke().ok();
            }

            // Draw cross-line similarity connectors
            cr.set_source_rgba(1.0, 0.75, 0.2, 0.55);
            cr.set_line_width(1.5);

            for sim in sim_entries.iter() {
                let y_left = (sim.left_line as f64 / max_lines as f64) * h + 2.0;
                let y_right = (sim.right_line as f64 / max_lines as f64) * h + 2.0;

                cr.set_dash(&[3.0, 4.0], 0.0);

                cr.move_to(0.0, y_left);
                let cp_y = (y_left + y_right) / 2.0;
                cr.curve_to(w * 0.25, cp_y - 3.0, w * 0.75, cp_y + 3.0, w, y_right);
                cr.stroke().ok();

                cr.set_dash(&[], 0.0);
            }

            // Draw movement connectors (thicker amber curves with dash)
            cr.set_source_rgba(1.0, 0.55, 0.1, 0.65);
            cr.set_line_width(2.0);

            for mv in move_entries.iter() {
                let y_left_start = (mv.left_start as f64 / max_lines as f64) * h + 2.0;
                let y_left_end = (mv.left_end as f64 / max_lines as f64) * h;
                let y_right_start = (mv.right_start as f64 / max_lines as f64) * h + 2.0;
                let y_right_end = (mv.right_end as f64 / max_lines as f64) * h;

                cr.set_dash(&[6.0, 3.0], 0.0);

                // Top connector
                cr.move_to(0.0, y_left_start);
                let cp_y_top = (y_left_start + y_right_start) / 2.0;
                cr.curve_to(
                    w * 0.3,
                    cp_y_top - 4.0,
                    w * 0.7,
                    cp_y_top + 4.0,
                    w,
                    y_right_start,
                );
                cr.stroke().ok();

                // Bottom connector
                cr.move_to(0.0, y_left_end);
                let cp_y_bot = (y_left_end + y_right_end) / 2.0;
                cr.curve_to(
                    w * 0.3,
                    cp_y_bot - 4.0,
                    w * 0.7,
                    cp_y_bot + 4.0,
                    w,
                    y_right_end,
                );
                cr.stroke().ok();

                cr.set_dash(&[], 0.0);
            }

            // ── FORCED TEST: fixed-position line to prove drawing works ──
            cr.set_source_rgb(0.0, 1.0, 0.0); // green
            cr.set_line_width(4.0);
            if w >= 40.0 && h >= 40.0 {
                cr.move_to(5.0, 5.0);
                cr.line_to(w - 5.0, h - 5.0);
                cr.stroke().ok();
                cr.move_to(w - 5.0, 5.0);
                cr.line_to(5.0, h - 5.0);
                cr.stroke().ok();
            }

            // Draw token-level moved-identifier connectors (thin blue curves)
            // using real pixel coordinates from GtkTextView APIs.
            //
            // Coordinate pipeline (no translate_coordinates, no
            // buffer_to_window_coords — only buffer-space coords +
            // scroll compensation):
            //   1. buffer.iter_at_offset(start_offset)  → GtkTextIter
            //   2. text_view.iter_location(&iter)        → GdkRectangle (buffer space)
            //   3. visible_y = rect.y() - vadjustment.value()
            //   4. draw curve at visible_y in DrawingArea
            cr.set_source_rgba(0.27, 0.53, 1.0, 0.7);
            cr.set_line_width(1.5);

            let token_count = token_entries.len();
            if token_count > 0 {
                eprintln!("LINKMAP token_entries count={}", token_count);
            }

            if let (Some(lv), Some(rv)) = (left_view_opt.as_ref(), right_view_opt.as_ref()) {
                // Get scroll offsets from both text views.
                // `vadjustment().value()` is the pixel offset from the top
                // of the buffer that is currently at the top of the visible
                // viewport.  Subtracting it from buffer_y gives the Y
                // position relative to the visible area — which matches
                // the DrawingArea coordinate space because both widgets
                // share the same vertical extent in the layout.
                let scroll_left = lv.vadjustment().map(|a| a.value()).unwrap_or(0.0);
                let scroll_right = rv.vadjustment().map(|a| a.value()).unwrap_or(0.0);

                // Track all Y values to detect same-Y bug
                let mut all_y_left: Vec<f64> = Vec::with_capacity(token_count);
                let mut all_y_right: Vec<f64> = Vec::with_capacity(token_count);

                for (idx, tr) in token_entries.iter().enumerate() {
                    // ── Build left-side token layout ──
                    let left_layout = compute_token_layout(lv, tr.left_offset_start as i32);

                    // ── Build right-side token layout ──
                    let right_layout = compute_token_layout(rv, tr.right_offset_start as i32);

                    // ── Compute visible Y: buffer_y - scroll ──
                    let (left_h, yl) = match left_layout {
                        Some(ref l) => {
                            let vy = l.buffer_y - scroll_left;
                            eprintln!(
                                "TOKEN [{}] LEFT  offset={} buf_y={:.0} scroll={:.0} -> y={:.0}",
                                idx, tr.left_offset_start, l.buffer_y, scroll_left, vy,
                            );
                            (l.height, vy)
                        }
                        None => {
                            eprintln!(
                                "TOKEN [{}] LEFT  offset={} -> LAYOUT FAILED (past end)",
                                idx, tr.left_offset_start,
                            );
                            (18.0, (tr.left_line as f64 / max_lines as f64) * h + 2.0)
                        }
                    };

                    let (right_h, yr) = match right_layout {
                        Some(ref l) => {
                            let vy = l.buffer_y - scroll_right;
                            eprintln!(
                                "TOKEN [{}] RIGHT offset={} buf_y={:.0} scroll={:.0} -> y={:.0}",
                                idx, tr.right_offset_start, l.buffer_y, scroll_right, vy,
                            );
                            (l.height, vy)
                        }
                        None => {
                            eprintln!(
                                "TOKEN [{}] RIGHT offset={} -> LAYOUT FAILED (past end)",
                                idx, tr.right_offset_start,
                            );
                            (18.0, (tr.right_line as f64 / max_lines as f64) * h + 2.0)
                        }
                    };

                    // Clamp to DrawingArea bounds
                    let yl_c = (yl + left_h / 2.0).clamp(0.0, h);
                    let yr_c = (yr + right_h / 2.0).clamp(0.0, h);

                    all_y_left.push(yl_c);
                    all_y_right.push(yr_c);

                    // ── FORCE DEBUG: red/green circles at token positions ──
                    cr.set_source_rgb(1.0, 0.0, 0.0);
                    cr.arc(5.0, yl_c, 4.0, 0.0, 2.0 * std::f64::consts::PI);
                    cr.fill().ok();
                    cr.set_source_rgb(0.0, 0.7, 0.0);
                    cr.arc(w - 5.0, yr_c, 4.0, 0.0, 2.0 * std::f64::consts::PI);
                    cr.fill().ok();

                    // ── Draw the connector curve ──
                    cr.set_source_rgba(0.27, 0.53, 1.0, 0.9);
                    cr.set_line_width(2.0);
                    cr.move_to(0.0, yl_c);
                    let cp_y = (yl_c + yr_c) / 2.0;
                    cr.curve_to(w * 0.3, cp_y - 2.0, w * 0.7, cp_y + 2.0, w, yr_c);
                    cr.stroke().ok();
                }

                // ── Detect same-Y bug: all left or all right Y equal ──
                if token_count > 1 {
                    let all_same_left = all_y_left.windows(2).all(|w| (w[0] - w[1]).abs() < 0.5);
                    let all_same_right = all_y_right.windows(2).all(|w| (w[0] - w[1]).abs() < 0.5);
                    if all_same_left {
                        eprintln!(
                            "LINKMAP BUG: all {} left Y values equal ({:.0}) — \
                             offsets or scroll may be wrong",
                            token_count,
                            all_y_left.first().copied().unwrap_or(0.0),
                        );
                    }
                    if all_same_right {
                        eprintln!(
                            "LINKMAP BUG: all {} right Y values equal ({:.0}) — \
                             offsets or scroll may be wrong",
                            token_count,
                            all_y_right.first().copied().unwrap_or(0.0),
                        );
                    }
                }
            } else {
                eprintln!("LINKMAP views not associated — skipping token connectors");
            }
        });

        Self {
            drawing_area,
            chunks: chunks_rc,
            total_lines_a: total_a,
            total_lines_b: total_b,
            similarity,
            moves,
            token_relations,
            left_view,
            right_view,
        }
    }

    /// Associate this LinkMap with the two text views it sits between.
    ///
    /// Must be called after construction so the LinkMap can query visible
    /// line ranges for viewport culling. Mirrors the original Meld's
    /// `LinkMap.associate()`.
    pub fn associate(&self, left: &gsv::View, right: &gsv::View) {
        self.left_view.replace(Some(left.clone()));
        self.right_view.replace(Some(right.clone()));
        self.drawing_area.queue_draw();
    }

    /// Underlying widget.
    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.drawing_area
    }

    /// Update the chunks.
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

    /// Update the cross-line similarity data.
    pub fn update_similarity(&self, map: &SimilarityMap) {
        let entries: Vec<SimilarityLink> = map
            .matches
            .iter()
            .map(|e| SimilarityLink {
                left_line: e.left_line,
                right_line: e.right_line,
                score: e.score,
            })
            .collect();
        self.similarity.replace(entries);
        self.drawing_area.queue_draw();
    }

    /// Update the detected movement data for drawing amber connectors.
    pub fn update_moves(&self, map: &MoveMap) {
        let entries: Vec<MoveLink> = map
            .moves
            .iter()
            .map(|e| MoveLink {
                left_start: e.left_start,
                left_end: e.left_end,
                right_start: e.right_start,
                right_end: e.right_end,
                score: e.score,
            })
            .collect();
        self.moves.replace(entries);
        self.drawing_area.queue_draw();
    }

    /// Update token-level moved-identifier relations for blue connectors.
    pub fn update_token_relations(&self, relations: &[TokenRelation]) {
        self.token_relations.replace(relations.to_vec());
        self.drawing_area.queue_draw();
    }
}

// ─── Helper: compute real pixel position of a buffer token ──────────

/// Return the buffer-space Y and line height for the character at
/// `start_offset`.
///
/// Uses `iter_at_offset` → `GtkTextIter`, then `iter_location` →
/// `GdkRectangle` in **buffer coordinates** (origin at top of buffer,
/// not affected by window position or scroll).
///
/// The caller must compensate for the current scroll offset by
/// subtracting `vadjustment().value()` before drawing.
///
/// Returns `None` when the offset is past the end of the buffer (e.g.
/// the buffer was modified since token relations were built).
fn compute_token_layout(view: &gsv::View, start_offset: i32) -> Option<TokenLayout> {
    use gtk4::prelude::TextViewExt;

    let buffer = view.buffer();

    // Step 1: get a GtkTextIter at the character offset.
    // When the offset is past-end, gtk returns the end iter.
    let start_iter = buffer.iter_at_offset(start_offset);
    if start_iter.is_end() && start_offset > 0 {
        return None;
    }

    // Step 2: get the pixel rectangle in buffer coordinates.
    // rect.y() is the pixel distance from the top of the buffer.
    let tv: &gtk::TextView = view.upcast_ref();
    let rect = tv.iter_location(&start_iter);

    Some(TokenLayout {
        buffer_y: rect.y() as f64,
        height: rect.height() as f64,
    })
}
