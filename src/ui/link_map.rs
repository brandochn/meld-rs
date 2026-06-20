#![cfg(feature = "gui")]
//! Link map widget drawing bezier curves between corresponding diff lines.
//!
//! Ported from the original `meld/linkmap.py`. Renders visual connectors
//! between matching/changed line regions in two side-by-side diff panes.

use gtk4 as gtk;
use gtk4::prelude::*;
use sourceview5 as gsv;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::diff::engine::{Chunk, DiffOp};
use crate::ui::style;

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
    /// Index of the currently-focused chunk for highlight overlay.
    current_chunk: Rc<Cell<Option<usize>>>,
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
        let left_view: Rc<RefCell<Option<gsv::View>>> = Rc::new(RefCell::new(None));
        let right_view: Rc<RefCell<Option<gsv::View>>> = Rc::new(RefCell::new(None));
        let hover_zones = Rc::new(RefCell::new(Vec::<HoverZone>::new()));
        let active_hover = Rc::new(RefCell::new(HoverInfo::None));
        let hover_callback: Rc<RefCell<Option<Box<dyn Fn(&HoverInfo)>>>> =
            Rc::new(RefCell::new(None));
        let current_chunk = Rc::new(Cell::new(None::<usize>));

        let draw_chunks = Rc::clone(&chunks_rc);
        let draw_total_a = Rc::clone(&total_a);
        let draw_total_b = Rc::clone(&total_b);
        let draw_left_view = Rc::clone(&left_view);
        let draw_right_view = Rc::clone(&right_view);
        let draw_hover_zones = Rc::clone(&hover_zones);
        let draw_current = Rc::clone(&current_chunk);

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
            //
            // `get_y_for_line_num` is GtkTextView's `get_line_yrange`, which
            // returns the top of the *full line allocation* (it includes the
            // `pixels-above-lines` spacing). That is exactly where the panes
            // paint their `paragraph-background` chunk bands, so the connector
            // lines up with them. Using `iter_location` instead returns the
            // glyph top — a couple of pixels lower — which is what left the
            // connectors 1–2px off and broke the continuity at the boundary.
            let view_offset_line =
                |view_opt: &Option<gsv::View>, pix: f64, off: f64, line: usize| -> Option<f64> {
                    let view = view_opt.as_ref()?;
                    let buf = view.buffer();
                    // Round to the pixel grid so the cairo connector snaps to
                    // the same device row as GtkTextView's pixel-snapped
                    // paragraph background (scroll/offset can be fractional,
                    // which otherwise leaves an anti-aliased 1px mismatch).
                    if line >= buf.line_count() as usize {
                        let last = (buf.line_count() - 1).max(0);
                        let i = buf.iter_at_line(last)?;
                        let (y, hgt) = view.line_yrange(&i);
                        return Some(((y + hgt) as f64 - pix + off).round());
                    }
                    let iter = buf.iter_at_line(line as i32)?;
                    let (y, _) = view.line_yrange(&iter);
                    Some((y as f64 - pix + off).round())
                };

            // Bezier control points (x_steps = [-0.5, w/2, w + 0.5])
            let xl = -0.5;
            let xm = w / 2.0;
            let xr = w + 0.5;

            let mut zones: Vec<HoverZone> = Vec::new();

            // ---- Chunk bezier curves (matching Meld's do_draw exactly) ----
            for (i, chunk) in chunks.iter().enumerate() {
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
                    f1_raw
                };

                let t0 = view_offset_line(&right_view_opt, pix_right, off_right, chunk.start_b)
                    .unwrap_or((chunk.start_b as f64 / max_lines as f64) * h);
                let t1_raw = view_offset_line(&right_view_opt, pix_right, off_right, chunk.end_b)
                    .unwrap_or((chunk.end_b as f64 / max_lines as f64) * h);
                let t1 = if chunk.end_b == chunk.start_b {
                    t0
                } else {
                    t1_raw
                };

                let y0 = f0.clamp(0.0, h);
                let y1 = f1.clamp(0.0, h);
                let t0c = t0.clamp(0.0, h);
                let t1c = t1.clamp(0.0, h);

                // Meld style scheme colours (see `crate::ui::style`):
                //   insert/delete fill=#d0ffa3 stroke=#a5ff4c
                //   replace       fill=#bdddff stroke=#65b2ff
                // Filled regions use the "background" (fill) colour; the
                // stroked outline uses the "line-background" (line) colour.
                let (r, g, b) = match style::fill_color(chunk.op) {
                    Some(c) => c,
                    None => continue,
                };
                let (sr, sg, sb) = match style::line_color(chunk.op) {
                    Some(c) => c,
                    None => continue,
                };

                // Filled region — opaque `fill_colors`, matching Meld so the
                // connector reads as solid colour continuous with the panes
                // (not a washed-out translucent shape).  The fill edges sit on
                // the exact line tops (`y0/t0/t1/y1`, no half-pixel offset) so
                // they meet the action-gutter fill and the panes' paragraph
                // backgrounds with no 1px step.
                cr.set_source_rgb(r, g, b);
                cr.move_to(xl, y0);
                cr.curve_to(xm, y0, xm, t0c, xr, t0c);
                cr.line_to(xr, t1c);
                cr.curve_to(xm, t1c, xm, y1, xl, y1);
                cr.close_path();
                cr.fill().ok();

                // Stroked outline (Meld's line_colors), opaque.
                cr.set_source_rgb(sr, sg, sb);
                cr.set_line_width(1.0);
                cr.move_to(xl, y0 - 0.5);
                cr.curve_to(xm, y0 - 0.5, xm, t0c - 0.5, xr, t0c - 0.5);
                cr.stroke().ok();
                cr.move_to(xl, y1 - 0.5);
                cr.curve_to(xm, y1 - 0.5, xm, t1c - 0.5, xr, t1c - 0.5);
                cr.stroke().ok();

                // Current chunk highlight overlay (mirrors Meld's current-chunk-highlight)
                if draw_current.get() == Some(i) {
                    cr.set_source_rgba(1.0, 0.8, 0.0, 0.25);
                    cr.move_to(xl, y0);
                    cr.curve_to(xm, y0, xm, t0c, xr, t0c);
                    cr.line_to(xr, t1c);
                    cr.curve_to(xm, t1c, xm, y1, xl, y1);
                    cr.close_path();
                    cr.fill().ok();
                }

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
            let changed = {
                let current = active_hover_leave.borrow();
                match &*current {
                    HoverInfo::None => false,
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
            current_chunk,
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

    /// Set the currently focused chunk index for visual highlight.
    pub fn set_current_chunk(&self, idx: Option<usize>) {
        self.current_chunk.set(idx);
        self.drawing_area.queue_draw();
    }
}
