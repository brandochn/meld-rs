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

/// Information about which connector the pointer is hovering over.
#[derive(Debug, Clone)]
pub enum HoverInfo {
    None,
    Chunk {
        start_a: usize,
        end_a: usize,
        start_b: usize,
        end_b: usize,
        op: DiffOp,
    },
    Token {
        left_line: usize,
        right_line: usize,
    },
    Similarity {
        left_line: usize,
        right_line: usize,
    },
    Move {
        left_start: usize,
        left_end: usize,
        right_start: usize,
        right_end: usize,
    },
}

/// A vertical hit zone recorded during drawing for hover detection.
#[derive(Debug, Clone)]
struct HoverZone {
    y_min: f64,
    y_max: f64,
    info: HoverInfo,
}

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
    /// Hit zones recorded during the last draw for hover detection.
    hover_zones: Rc<RefCell<Vec<HoverZone>>>,
    /// Currently active hover info.
    active_hover: Rc<RefCell<HoverInfo>>,
    /// External callback for hover changes.
    hover_callback: Rc<RefCell<Option<Box<dyn Fn(&HoverInfo)>>>>,
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
        // Match Meld original default width (50 px).  The layout in
        // filediff.rs will override this to 90 px via set_width_request
        // so the bezier curves span the full separator area.
        drawing_area.set_width_request(50);
        drawing_area.set_content_height(100);
        drawing_area.set_hexpand(false);
        drawing_area.set_vexpand(true);
        drawing_area.set_css_classes(&["link-map"]);

        let chunks_rc = Rc::new(RefCell::new(chunks.to_vec()));
        let total_a = Rc::new(RefCell::new(total_lines_a.max(1)));
        let total_b = Rc::new(RefCell::new(total_lines_b.max(1)));
        let similarity = Rc::new(RefCell::new(Vec::<SimilarityLink>::new()));
        let moves = Rc::new(RefCell::new(Vec::<MoveLink>::new()));
        let token_relations = Rc::new(RefCell::new(Vec::<TokenRelation>::new()));
        let left_view: Rc<RefCell<Option<gsv::View>>> = Rc::new(RefCell::new(None));
        let right_view: Rc<RefCell<Option<gsv::View>>> = Rc::new(RefCell::new(None));
        let hover_zones = Rc::new(RefCell::new(Vec::<HoverZone>::new()));
        let active_hover = Rc::new(RefCell::new(HoverInfo::None));
        let hover_callback: Rc<RefCell<Option<Box<dyn Fn(&HoverInfo)>>>> =
            Rc::new(RefCell::new(None));

        let draw_chunks = Rc::clone(&chunks_rc);
        let draw_total_a = Rc::clone(&total_a);
        let draw_total_b = Rc::clone(&total_b);
        let draw_left_view = Rc::clone(&left_view);
        let draw_right_view = Rc::clone(&right_view);
        let draw_similarity = Rc::clone(&similarity);
        let draw_moves = Rc::clone(&moves);
        let draw_token_relations = Rc::clone(&token_relations);
        let draw_hover_zones = Rc::clone(&hover_zones);

        drawing_area.set_draw_func(move |da, cr, width, height| {
            let da_widget: &gtk::Widget = da.upcast_ref();

            let w = width as f64;
            let h = height as f64;

            if width < 2 || height < 2 {
                return;
            }

            let chunks = draw_chunks.borrow();
            let total_a = *draw_total_a.borrow();
            let total_b = *draw_total_b.borrow();
            let max_lines = total_a.max(total_b).max(1);
            let left_view_opt = draw_left_view.borrow();
            let right_view_opt = draw_right_view.borrow();

            // Background handled by CSS class "link-map"
            // (matching Meld original: background-color: @theme_bg_color)

            // ---- Match Python Meld's do_draw exactly ----
            //
            // Compute per-view offsets once so every line lookup is a
            // simple arithmetic operation, not a full widget-coordinate
            // translation.
            //
            //   pix_start  = vadjustment.value()   = scroll offset
            //   y_offset   = translate_coordinates = widget Y in LinkMap
            //   linkmap_y  = buffer_y - pix_start + y_offset + 1  (Meld adds +1)
            let (pix_left, off_left, pix_right, off_right) =
                if let (Some(lv), Some(rv)) = (left_view_opt.as_ref(), right_view_opt.as_ref()) {
                    let lw: &gtk::Widget = lv.upcast_ref();
                    let rw: &gtk::Widget = rv.upcast_ref();
                    let (_, lo) = lw
                        .translate_coordinates(da_widget, 0.0, 0.0)
                        .unwrap_or((0.0, 0.0));
                    let (_, ro) = rw
                        .translate_coordinates(da_widget, 0.0, 0.0)
                        .unwrap_or((0.0, 0.0));
                    let lp = lv.vadjustment().map(|a| a.value()).unwrap_or(0.0);
                    let rp = rv.vadjustment().map(|a| a.value()).unwrap_or(0.0);
                    (lp, lo, rp, ro)
                } else {
                    (0.0, 0.0, 0.0, 0.0)
                };

            // Python Meld:
            //   def view_offset_line(view_idx, line_num):
            //       line_start =
            //           self.views[view_idx].get_y_for_line_num(line_num)
            //       return line_start - pix_start[view_idx] +
            //              y_offset[view_idx]
            let view_offset_line =
                |view_opt: &Option<gsv::View>, pix: f64, off: f64, line: usize| -> Option<f64> {
                    let view = view_opt.as_ref()?;
                    let buf = view.buffer();
                    let iter = if line >= buf.line_count() as usize {
                        let last = (buf.line_count() - 1).max(0);
                        let i = buf.iter_at_line(last)?;
                        let rect = view.iter_location(&i);
                        // Meld adds +1 to y_offset, so: buffer_y - pix + off + 1
                        return Some(rect.y() as f64 + rect.height() as f64 - pix + off + 1.0);
                    } else {
                        buf.iter_at_line(line as i32)?
                    };
                    let rect = view.iter_location(&iter);
                Some(rect.y() as f64 - pix + off + 1.0)
            };

            // Bezier control points (x_steps = [-0.5, w/2, w + 0.5])
            let xl = -0.5;
            let xm = w / 2.0;
            let xr = w + 0.5;

            let mut zones: Vec<HoverZone> = Vec::new();

            // ---- Chunk bezier curves (matching Meld's do_draw exactly) ----
            for chunk in chunks.iter() {
                if chunk.op == DiffOp::Equal {
                    continue;
                }
                // f0, f1 = view_offset_line(0, ...) for start_a, end_a
                let f0 = view_offset_line(&left_view_opt, pix_left, off_left, chunk.start_a)
                    .unwrap_or((chunk.start_a as f64 / max_lines as f64) * h);
                let f1_raw = view_offset_line(&left_view_opt, pix_left, off_left, chunk.end_a)
                    .unwrap_or((chunk.end_a as f64 / max_lines as f64) * h);
                let f1 = if chunk.end_a == chunk.start_a {
                    f0
                } else {
                    f1_raw - 1.0
                };

                let t0 = view_offset_line(&right_view_opt, pix_right, off_right, chunk.start_b)
                    .unwrap_or((chunk.start_b as f64 / max_lines as f64) * h);
                let t1_raw = view_offset_line(&right_view_opt, pix_right, off_right, chunk.end_b)
                    .unwrap_or((chunk.end_b as f64 / max_lines as f64) * h);
                let t1 = if chunk.end_b == chunk.start_b {
                    t0
                } else {
                    t1_raw - 1.0
                };

                let y0 = f0.clamp(0.0, h);
                let y1 = f1.clamp(0.0, h);
                let t0c = t0.clamp(0.0, h);
                let t1c = t1.clamp(0.0, h);

                let (r, g, b) = match chunk.op {
                    // Meld style scheme colors (matching get_common_theme):
                    //   insert fill=#d0ffa3 stroke=#a5ff4c
                    //   delete fill=#d0ffa3 stroke=#a5ff4c  (same as insert)
                    //   replace fill=#bdddff stroke=#65b2ff
                    // For filled regions we use the "background" color.
                    DiffOp::Delete | DiffOp::Insert => (0.816, 1.0, 0.639),
                    DiffOp::Replace => (0.741, 0.867, 1.0),
                    DiffOp::Equal => continue,
                };

                // Stroke uses "line-background" color (slightly different shade)
                let (sr, sg, sb) = match chunk.op {
                    DiffOp::Delete | DiffOp::Insert => (0.647, 1.0, 0.298),
                    DiffOp::Replace => (0.396, 0.698, 1.0),
                    _ => continue,
                };

                // Filled region (Meld's fill_colors)
                cr.set_source_rgba(r, g, b, 0.35);
                cr.move_to(xl, y0 - 0.5);
                cr.curve_to(xm, y0 - 0.5, xm, t0c - 0.5, xr, t0c - 0.5);
                cr.line_to(xr, t1c - 0.5);
                cr.curve_to(xm, t1c - 0.5, xm, y1 - 0.5, xl, y1 - 0.5);
                cr.close_path();
                cr.fill().ok();

                // Stroked outline (Meld's line_colors)
                cr.set_source_rgba(sr, sg, sb, 0.55);
                cr.set_line_width(1.0);
                cr.move_to(xl, y0 - 0.5);
                cr.curve_to(xm, y0 - 0.5, xm, t0c - 0.5, xr, t0c - 0.5);
                cr.stroke().ok();
                cr.move_to(xl, y1 - 0.5);
                cr.curve_to(xm, y1 - 0.5, xm, t1c - 0.5, xr, t1c - 0.5);
                cr.stroke().ok();

                zones.push(HoverZone {
                    y_min: y0.min(t0c),
                    y_max: y1.max(t1c),
                    info: HoverInfo::Chunk {
                        start_a: chunk.start_a,
                        end_a: chunk.end_a,
                        start_b: chunk.start_b,
                        end_b: chunk.end_b,
                        op: chunk.op,
                    },
                });
            }

            // ── Token-level moved-identifier connectors ────────────
            // Blue dashed bezier curves linking moved identifiers
            // (e.g. EnvironmentContext split from a combined import).
            {
                let token_rels = draw_token_relations.borrow();
                if !token_rels.is_empty() {
                    cr.set_source_rgba(0.541, 0.761, 1.0, 0.55);
                    cr.set_line_width(1.2);
                    cr.set_dash(&[4.0, 3.0], 0.0);

                    for rel in token_rels.iter() {
                        let left_y = if let (Some(lv), Some(_rv)) =
                            (left_view_opt.as_ref(), right_view_opt.as_ref())
                        {
                            let center = ((rel.left_offset_start + rel.left_offset_end) / 2)
                                as i32;
                            compute_token_layout(lv, center)
                                .map(|tl| tl.center_y - pix_left + off_left + 1.0)
                                .unwrap_or_else(|| {
                                    view_offset_line(
                                        &left_view_opt,
                                        pix_left,
                                        off_left,
                                        rel.left_line,
                                    )
                                    .unwrap_or(
                                        (rel.left_line as f64 / max_lines as f64) * h,
                                    )
                                })
                        } else {
                            (rel.left_line as f64 / max_lines as f64) * h
                        };

                        let right_y =
                            if let (Some(_lv), Some(rv)) =
                                (left_view_opt.as_ref(), right_view_opt.as_ref())
                            {
                                let center =
                                    ((rel.right_offset_start + rel.right_offset_end) / 2)
                                        as i32;
                                compute_token_layout(rv, center)
                                    .map(|tl| tl.center_y - pix_right + off_right + 1.0)
                                    .unwrap_or_else(|| {
                                        view_offset_line(
                                            &right_view_opt,
                                            pix_right,
                                            off_right,
                                            rel.right_line,
                                        )
                                        .unwrap_or(
                                            (rel.right_line as f64 / max_lines as f64) * h,
                                        )
                                    })
                            } else {
                                (rel.right_line as f64 / max_lines as f64) * h
                            };

                        let ly = left_y.clamp(-10.0, h + 10.0);
                        let ry = right_y.clamp(-10.0, h + 10.0);

                        if (ly < -5.0 && ry < -5.0)
                            || (ly > h + 5.0 && ry > h + 5.0)
                        {
                            continue;
                        }

                        cr.move_to(xl, ly);
                        cr.curve_to(xm, ly, xm, ry, xr, ry);
                        cr.stroke().ok();

                        zones.push(HoverZone {
                            y_min: ly.min(ry),
                            y_max: ly.max(ry),
                            info: HoverInfo::Token {
                                left_line: rel.left_line,
                                right_line: rel.right_line,
                            },
                        });
                    }

                    cr.set_dash(&[], 0.0);
                }
            }

            // ── Similarity connectors (amber dotted) ──────────────
            // Dotted bezier curves for cross-line similarity matches
            // (lines that aren't aligned but share significant tokens).
            {
                let sims = draw_similarity.borrow();
                if !sims.is_empty() {
                    for entry in sims.iter() {
                        let score = entry.score.clamp(0.0, 1.0);
                        let alpha = 0.15 + score * 0.35;
                        cr.set_source_rgba(1.0, 0.75, 0.3, alpha);
                        cr.set_line_width(1.0);
                        cr.set_dash(&[2.0, 4.0], 0.0);

                        let left_y = view_offset_line(
                            &left_view_opt, pix_left, off_left, entry.left_line,
                        )
                        .unwrap_or(
                            (entry.left_line as f64 / max_lines as f64) * h,
                        );
                        let right_y = view_offset_line(
                            &right_view_opt,
                            pix_right,
                            off_right,
                            entry.right_line,
                        )
                        .unwrap_or(
                            (entry.right_line as f64 / max_lines as f64) * h,
                        );

                        let ly = left_y.clamp(-10.0, h + 10.0);
                        let ry = right_y.clamp(-10.0, h + 10.0);

                        if (ly < -5.0 && ry < -5.0)
                            || (ly > h + 5.0 && ry > h + 5.0)
                        {
                            continue;
                        }

                        cr.move_to(xl, ly);
                        cr.curve_to(xm, ly, xm, ry, xr, ry);
                        cr.stroke().ok();

                        zones.push(HoverZone {
                            y_min: ly.min(ry),
                            y_max: ly.max(ry),
                            info: HoverInfo::Similarity {
                                left_line: entry.left_line,
                                right_line: entry.right_line,
                            },
                        });
                    }
                    cr.set_dash(&[], 0.0);
                }
            }

            // ── Move connectors (dashed amber regions) ────────────
            // Bezier outlines for detected code-block movements.
            {
                let moves = draw_moves.borrow();
                if !moves.is_empty() {
                    cr.set_source_rgba(1.0, 0.8, 0.3, 0.5);
                    cr.set_line_width(1.2);
                    cr.set_dash(&[6.0, 4.0], 0.0);

                    for entry in moves.iter() {
                        let f0 = view_offset_line(
                            &left_view_opt, pix_left, off_left, entry.left_start,
                        )
                        .unwrap_or(
                            (entry.left_start as f64 / max_lines as f64) * h,
                        );
                        let f1_raw = view_offset_line(
                            &left_view_opt, pix_left, off_left, entry.left_end,
                        )
                        .unwrap_or(
                            (entry.left_end as f64 / max_lines as f64) * h,
                        );
                        let f1 = if entry.left_end == entry.left_start {
                            f0
                        } else {
                            f1_raw - 1.0
                        };

                        let t0 = view_offset_line(
                            &right_view_opt,
                            pix_right,
                            off_right,
                            entry.right_start,
                        )
                        .unwrap_or(
                            (entry.right_start as f64 / max_lines as f64) * h,
                        );
                        let t1_raw = view_offset_line(
                            &right_view_opt,
                            pix_right,
                            off_right,
                            entry.right_end,
                        )
                        .unwrap_or(
                            (entry.right_end as f64 / max_lines as f64) * h,
                        );
                        let t1 = if entry.right_end == entry.right_start {
                            t0
                        } else {
                            t1_raw - 1.0
                        };

                        let y0 = f0.clamp(0.0, h);
                        let y1 = f1.clamp(0.0, h);
                        let t0c = t0.clamp(0.0, h);
                        let t1c = t1.clamp(0.0, h);

                        if (t0c < 0.0 && t1c < 0.0 && y0 < 0.0 && y1 < 0.0)
                            || (t0c > h && t1c > h && y0 > h && y1 > h)
                        {
                            continue;
                        }

                        cr.move_to(xl, y0 - 0.5);
                        cr.curve_to(xm, y0 - 0.5, xm, t0c - 0.5, xr, t0c - 0.5);
                        cr.line_to(xr, t1c - 0.5);
                        cr.curve_to(xm, t1c - 0.5, xm, y1 - 0.5, xl, y1 - 0.5);
                        cr.close_path();
                        cr.stroke().ok();

                        zones.push(HoverZone {
                            y_min: y0.min(t0c),
                            y_max: y1.max(t1c),
                            info: HoverInfo::Move {
                                left_start: entry.left_start,
                                left_end: entry.left_end,
                                right_start: entry.right_start,
                                right_end: entry.right_end,
                            },
                        });
                    }
                    cr.set_dash(&[], 0.0);
                }
            }

            *draw_hover_zones.borrow_mut() = zones;
        });

        // ── Motion controller for hover detection ──
        let mc = gtk::EventControllerMotion::new();
        let hover_zones_mc = Rc::clone(&hover_zones);
        let active_hover_mc = Rc::clone(&active_hover);
        let hover_callback_mc = Rc::clone(&hover_callback);
        let da_mc = drawing_area.clone();
        mc.connect_motion(move |_, _x, y| {
            let zones = hover_zones_mc.borrow();
            let mut found = HoverInfo::None;
            for zone in zones.iter() {
                if y >= zone.y_min - 4.0 && y <= zone.y_max + 4.0 {
                    found = zone.info.clone();
                    break;
                }
            }
            let changed = {
                let current = active_hover_mc.borrow();
                match (&*current, &found) {
                    (HoverInfo::None, HoverInfo::None) => false,
                    _ => true,
                }
            };
            if changed {
                *active_hover_mc.borrow_mut() = found;
                let cb = hover_callback_mc.borrow();
                if let Some(ref cb) = *cb {
                    cb(&active_hover_mc.borrow());
                }
            }
            let _ = &da_mc;
        });
        let active_hover_leave = Rc::clone(&active_hover);
        let hover_callback_leave = Rc::clone(&hover_callback);
        mc.connect_leave(move |_| {
            let found = HoverInfo::None;
            let changed = {
                let current = active_hover_leave.borrow();
                match (&*current, &found) {
                    (HoverInfo::None, HoverInfo::None) => false,
                    _ => true,
                }
            };
            if changed {
                *active_hover_leave.borrow_mut() = HoverInfo::None;
                let cb = hover_callback_leave.borrow();
                if let Some(ref cb) = *cb {
                    cb(&active_hover_leave.borrow());
                }
            }
        });
        drawing_area.add_controller(mc);

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
            hover_zones,
            active_hover,
            hover_callback,
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

    /// Register a callback for hover events over link-map connectors.
    pub fn connect_hover<F: Fn(&HoverInfo) + 'static>(&self, callback: F) {
        self.hover_callback.replace(Some(Box::new(callback)));
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

    /// Clear similarity, movement, and token-relation overlay data.
    pub fn clear_overlays(&self) {
        self.similarity.replace(Vec::new());
        self.moves.replace(Vec::new());
        self.token_relations.replace(Vec::new());
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
