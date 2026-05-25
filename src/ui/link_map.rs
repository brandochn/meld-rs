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
///
/// Holds the single representative point for the token:
/// the horizontal character position and the vertical centre of its line.
#[derive(Debug, Clone, Copy)]
struct TokenLayout {
    /// Buffer-space X of the token's representative character (pixels from
    /// the left edge of the text area).
    x: f64,
    /// Buffer-space Y at the vertical centre of the token's line (pixels
    /// from the top of the buffer, before scroll compensation).
    center_y: f64,
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

        drawing_area.set_draw_func(move |da, cr, width, height| {
            // ── FORCED DEBUG: verify rendering pipeline is active ──
            eprintln!("LINKMAP DRAW w={} h={}", width, height);
            let da_widget: &gtk::Widget = da.upcast_ref();

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

            // Background
            cr.set_source_rgba(0.95, 0.95, 0.95, 0.6);
            cr.paint().ok();

            // Helper: translate a line number in a text view to a Y
            // position in LinkMap widget coordinates.  Mirrors Python
            // Meld's `view_offset_line()`.  Correctly subtracts the
            // scroll offset so that curves move with the text.
            let line_to_y = |view_opt: &Option<gsv::View>, line: usize| -> f64 {
                if let Some(view) = view_opt {
                    // Scroll offset: how many buffer pixels are above
                    // the visible viewport.
                    let scroll = view.vadjustment().map(|a| a.value()).unwrap_or(0.0);

                    let buffer_y = if line >= view.buffer().line_count() as usize {
                        // Past-the-end: use the bottom of the last line.
                        let last = (view.buffer().line_count() - 1).max(0);
                        if let Some(iter) = view.buffer().iter_at_line(last) {
                            let rect = view.iter_location(&iter);
                            rect.y() as f64 + rect.height() as f64
                        } else {
                            return (line as f64 / max_lines as f64) * h;
                        }
                    } else if let Some(iter) = view.buffer().iter_at_line(line as i32) {
                        let rect = view.iter_location(&iter);
                        rect.y() as f64
                    } else {
                        return (line as f64 / max_lines as f64) * h;
                    };

                    // Convert buffer Y to view-relative Y (account for scroll).
                    let visible_y = buffer_y - scroll;

                    // Translate from the view's widget coordinate system
                    // to the LinkMap's DrawingArea coordinate system.
                    let (_, widget_y) = view
                        .upcast_ref::<gtk::Widget>()
                        .translate_coordinates(da_widget, 0.0, visible_y)
                        .unwrap_or((0.0, visible_y));
                    return widget_y;
                }
                (line as f64 / max_lines as f64) * h
            };

            for chunk in chunks.iter() {
                if chunk.op == DiffOp::Equal {
                    continue;
                }

                // Get line positions in LinkMap widget coordinates via the
                // text views' buffer iterators and coordinate translation.
                // This mirrors Python Meld's `view_offset_line()`, properly
                // handling scroll offsets and widget layout.
                let f0 = line_to_y(&left_view_opt, chunk.start_a);
                let f1 = if chunk.end_a == chunk.start_a {
                    f0
                } else {
                    line_to_y(&left_view_opt, chunk.end_a.saturating_sub(1))
                        + (line_to_y(&left_view_opt, chunk.end_a) - f0).max(1.0)
                };

                let t0 = line_to_y(&right_view_opt, chunk.start_b);
                let t1 = if chunk.end_b == chunk.start_b {
                    t0
                } else {
                    line_to_y(&right_view_opt, chunk.end_b.saturating_sub(1))
                        + (line_to_y(&right_view_opt, chunk.end_b) - t0).max(1.0)
                };

                let (r, g, b) = match chunk.op {
                    DiffOp::Delete => (1.0, 0.3, 0.3),
                    DiffOp::Insert => (0.3, 1.0, 0.3),
                    DiffOp::Replace => (0.3, 0.3, 1.0),
                    DiffOp::Equal => continue,
                };

                let y0 = f0.clamp(0.0, h);
                let y1 = f1.clamp(0.0, h);
                let t0c = t0.clamp(0.0, h);
                let t1c = t1.clamp(0.0, h);

                let x_left = 0.0;
                let x_mid = w / 2.0;
                let x_right = w;

                // Draw filled bezier shape (matching original Meld's LinkMap)
                cr.set_source_rgba(r, g, b, 0.2);
                cr.move_to(x_left, y0);
                cr.curve_to(x_mid, y0, x_mid, t0c, x_right, t0c);
                cr.line_to(x_right, t1c);
                cr.curve_to(x_mid, t1c, x_mid, y1, x_left, y1);
                cr.close_path();
                cr.fill().ok();

                // Stroke the outline
                cr.set_source_rgba(r, g, b, 0.5);
                cr.set_line_width(1.0);
                cr.move_to(x_left, y0);
                cr.curve_to(x_mid, y0, x_mid, t0c, x_right, t0c);
                cr.stroke().ok();
                cr.move_to(x_left, y1);
                cr.curve_to(x_mid, y1, x_mid, t1c, x_right, t1c);
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

            // Draw token-level moved-identifier connectors.
            //
            // Full coordinate pipeline:
            //   1. translate_coordinates(lv/rv → da) gives the text view's
            //      top-left corner in DrawingArea space (left_origin / right_origin).
            //   2. iter_at_offset(center_offset) → iter_location(iter) gives the
            //      character rectangle in buffer coordinates (buffer space).
            //   3. Final position = origin + buffer_rect - scroll.
            //      This correctly handles both horizontal and vertical offsets,
            //      regardless of widget layout or heading heights.

            let token_count = token_entries.len();
            if token_count > 0 {
                eprintln!("LINKMAP token_entries count={}", token_count);
            }

            if let (Some(lv), Some(rv)) = (left_view_opt.as_ref(), right_view_opt.as_ref()) {
                let lv_widget: &gtk::Widget = lv.upcast_ref();
                let rv_widget: &gtk::Widget = rv.upcast_ref();

                // Translate each text view's y-origin into DrawingArea space.
                // Only the Y component matters: for a link-map strip the left
                // connector endpoint is always at x=0 (left edge of the strip)
                // and the right endpoint at x=w (right edge). Adding the text
                // view's buffer x to a large negative/positive origin_x would
                // place both endpoints far outside the strip.
                let (_, left_origin_y) = lv_widget
                    .translate_coordinates(da_widget, 0.0, 0.0)
                    .unwrap_or((0.0, 0.0));
                let (_, right_origin_y) = rv_widget
                    .translate_coordinates(da_widget, 0.0, 0.0)
                    .unwrap_or((0.0, 0.0));

                // Scroll offsets: how many buffer pixels are above the viewport.
                let scroll_left = lv.vadjustment().map(|a| a.value()).unwrap_or(0.0);
                let scroll_right = rv.vadjustment().map(|a| a.value()).unwrap_or(0.0);

                for (idx, tr) in token_entries.iter().enumerate() {
                    // ── STEP 1: Validate relation — both sides must be present ──
                    println!(
                        "RELATION [{}]: left_line={} → right_line={}",
                        idx, tr.left_line, tr.right_line
                    );

                    // ONE offset per side: the centre of the character span.
                    // Never iterate over the range; use this single point only.
                    let left_center = ((tr.left_offset_start + tr.left_offset_end) / 2) as i32;
                    let right_center = ((tr.right_offset_start + tr.right_offset_end) / 2) as i32;

                    // ONE iter → ONE rect → ONE layout per side.
                    let left_layout = compute_token_layout(lv, left_center);
                    let right_layout = compute_token_layout(rv, right_center);

                    // ── STEP 2: Resolve left endpoint ──
                    // lx is always the LEFT edge of the strip (x = 0).
                    // ly = origin_y_offset + buffer_center_y − scroll, clamped
                    //      to the visible height so the dot stays inside the widget.
                    let (lx, ly) = match left_layout {
                        Some(ref l) => {
                            let y = (left_origin_y + l.center_y - scroll_left).clamp(0.0, h);
                            println!("LEFT:  ({:.1}, {:.1})", 0.0_f64, y);
                            (0.0_f64, y)
                        }
                        None => {
                            let y = (tr.left_line as f64 / max_lines as f64) * h + 2.0;
                            println!("LEFT:  fallback (0.0, {:.1})", y);
                            (0.0_f64, y)
                        }
                    };

                    // ── STEP 3: Resolve right endpoint ──
                    // rx is always the RIGHT edge of the strip (x = w).
                    let (rx, ry) = match right_layout {
                        Some(ref l) => {
                            let y = (right_origin_y + l.center_y - scroll_right).clamp(0.0, h);
                            println!("RIGHT: ({:.1}, {:.1})", w, y);
                            (w, y)
                        }
                        None => {
                            let y = (tr.right_line as f64 / max_lines as f64) * h + 2.0;
                            println!("RIGHT: fallback ({:.1}, {:.1})", w, y);
                            (w, y)
                        }
                    };

                    // ── Debug dots at the real endpoints (both inside the strip) ──
                    cr.set_source_rgb(1.0, 0.0, 0.0);
                    cr.arc(lx + 4.0, ly, 4.0, 0.0, 2.0 * std::f64::consts::PI);
                    cr.fill().ok();
                    cr.set_source_rgb(0.0, 0.7, 0.0);
                    cr.arc(rx - 4.0, ry, 4.0, 0.0, 2.0 * std::f64::consts::PI);
                    cr.fill().ok();

                    // ── Exactly ONE connector per relation ──
                    cr.set_source_rgba(0.27, 0.53, 1.0, 0.9);
                    cr.set_line_width(2.0);
                    cr.move_to(lx, ly);
                    cr.line_to(rx, ry);
                    cr.stroke().ok();
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

        // Whenever either text view scrolls, the connector Y positions change.
        // Queue a redraw so the draw function re-reads vadjustment().value()
        // and paints the connector at the correct updated position.
        if let Some(adj) = left.vadjustment() {
            let da = self.drawing_area.clone();
            adj.connect_value_changed(move |_| {
                da.queue_draw();
            });
        }
        if let Some(adj) = right.vadjustment() {
            let da = self.drawing_area.clone();
            adj.connect_value_changed(move |_| {
                da.queue_draw();
            });
        }

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

/// Return the single representative pixel point for the character at
/// `center_offset` within `view`.
///
/// Pipeline (one offset → one iter → one rect → one point):
/// 1. `buffer.iter_at_offset(center_offset)` → `GtkTextIter`
/// 2. `text_view.iter_location(&iter)`        → `GdkRectangle` (buffer space)
/// 3. `x      = rect.x()`
///    `center_y = rect.y() + rect.height() / 2`
///
/// All coordinates are in **buffer space** (origin at the top of the
/// full buffer, independent of scroll position).  The caller must
/// subtract `vadjustment().value()` to obtain DrawingArea-relative Y.
///
/// Returns `None` when `center_offset` is past the end of the buffer
/// (e.g. the buffer was modified since the token relations were built).
fn compute_token_layout(view: &gsv::View, center_offset: i32) -> Option<TokenLayout> {
    use gtk4::prelude::TextViewExt;

    let buffer = view.buffer();

    // Step 1: single GtkTextIter at the centre of the token span.
    let iter = buffer.iter_at_offset(center_offset);
    if iter.is_end() && center_offset > 0 {
        return None;
    }

    // Step 2: pixel rectangle in buffer coordinates.
    let tv: &gtk::TextView = view.upcast_ref();
    let rect = tv.iter_location(&iter);

    // Step 3: derive the single representative point.
    Some(TokenLayout {
        x: rect.x() as f64,
        center_y: rect.y() as f64 + rect.height() as f64 / 2.0,
    })
}
