#![cfg(feature = "gui")]
//! Action gutter — per-chunk action buttons rendered with Cairo.
//!
//! Mirrors the original `ActionGutter` from `meld/actiongutter.py`.
//! Draws directional action icons (push left/right, delete, copy up/down)
//! for each diff chunk, aligned with the source textview's scroll position.
//!
//! Clicking a chunk button triggers the appropriate chunk operation:
//!   - Replace (push to other pane)
//!   - Delete (remove from source pane)
//!   - Insert menu (copy up/down — shows a popover with options)

use cairo;
use gtk4 as gtk;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use crate::diff::engine::{Chunk, DiffOp};

// ─── Types ──────────────────────────────────────────────────────────

/// Direction for chunk operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GutterDirection {
    /// Actions point left-to-right (source is left pane).
    LeftToRight,
    /// Actions point right-to-left (source is right pane).
    RightToLeft,
}

/// The action the user selected on a chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GutterAction {
    /// Push the change to the target pane (replace).
    Replace,
    /// Delete the change from the source pane.
    Delete,
    /// Copy the change upward from source to target.
    CopyUp,
    /// Copy the change downward from source to target.
    CopyDown,
}

/// Callback type for chunk actions.
pub type ActionCallback = Box<dyn Fn(usize, GutterAction) + 'static>;

// ─── ActionGutter ───────────────────────────────────────────────────

/// A Cairo-drawn gutter that displays per-chunk action buttons.
///
/// Placed between two diff panes. The source view provides the text for the
/// chunk operations; the target view receives the changes.
pub struct ActionGutter {
    /// The Cairo drawing surface.
    drawing_area: gtk::DrawingArea,
    /// Source textview (where chunks originate).
    source_view: gtk::TextView,
    /// Target textview (where actions are directed).
    target_view: gtk::TextView,
    /// Direction for visual arrows.
    direction: GutterDirection,
    /// Current diff chunks.
    chunks: Rc<RefCell<Vec<Chunk>>>,
    /// Index of the chunk being hovered.
    hovered_chunk: Rc<RefCell<Option<usize>>>,
    /// Index of the chunk being pressed.
    pressed_chunk: Rc<RefCell<Option<usize>>>,
    /// Callback invoked when a chunk action is triggered.
    action_callback: Rc<RefCell<Option<ActionCallback>>>,
    /// Cached button hit areas for click detection: (x1, y1, x2, y2, chunk_idx)
    buttons: Rc<RefCell<Vec<(f64, f64, f64, f64, usize)>>>,
    /// Popover for copy up/down menu (shown for Insert mode chunks).
    copy_popover: RefCell<Option<gtk::Popover>>,
}

impl ActionGutter {
    /// Creates a new action gutter between two text views.
    ///
    /// * `source_view` — the textview whose scroll position is tracked and
    ///   from which chunks are pushed/deleted.
    /// * `target_view` — the opposite textview to which chunks are pushed.
    /// * `direction` — visual direction for the action arrows.
    pub fn new(
        source_view: gtk::TextView,
        target_view: gtk::TextView,
        direction: GutterDirection,
    ) -> Self {
        let drawing_area = gtk::DrawingArea::new();
        drawing_area.set_width_request(20);
        drawing_area.set_vexpand(true);
        drawing_area.set_can_focus(false);
        drawing_area.add_css_class("action-gutter");

        let chunks = Rc::new(RefCell::new(Vec::<Chunk>::new()));
        let hovered = Rc::new(RefCell::new(None::<usize>));
        let pressed = Rc::new(RefCell::new(None::<usize>));
        let buttons = Rc::new(RefCell::new(Vec::<(f64, f64, f64, f64, usize)>::new()));

        // ── Scroll sync ──────────────────────────────────────────
        let dma_weak = drawing_area.downgrade();
        if let Some(vadj) = source_view.vadjustment() {
            vadj.connect_value_changed(move |_| {
                if let Some(dma) = dma_weak.upgrade() {
                    dma.queue_draw();
                }
            });
        }

        // Redraw on size change
        let dma_weak2 = drawing_area.downgrade();
        drawing_area.connect_resize(move |_, _, _| {
            if let Some(dma) = dma_weak2.upgrade() {
                dma.queue_draw();
            }
        });

        // ── Motion controller (hover detection) ──────────────────
        let hover_rc = Rc::new(RefCell::new(None::<usize>));
        let buttons_rc = Rc::new(RefCell::new(Vec::new()));
        let dma_motion = drawing_area.clone();

        let mc = gtk::EventControllerMotion::new();
        let hover_mc = Rc::clone(&hover_rc);
        let buttons_mc = Rc::clone(&buttons_rc);
        let dma_mc = dma_motion.clone();
        let hover_leave = Rc::clone(&hover_rc);
        mc.connect_motion(move |_, x, y| {
            let btns = buttons_mc.borrow();
            let mut new_hover: Option<usize> = None;
            for (x1, y1, x2, y2, idx) in btns.iter() {
                if x >= *x1 && x <= *x2 && y >= *y1 && y <= *y2 {
                    new_hover = Some(*idx);
                    break;
                }
            }
            if *hover_mc.borrow() != new_hover {
                hover_mc.replace(new_hover);
                dma_mc.queue_draw();
            }
        });

        let dma_leave = drawing_area.clone();
        mc.connect_leave(move |_| {
            if hover_leave.borrow().is_some() {
                hover_leave.replace(None);
                dma_leave.queue_draw();
            }
        });

        drawing_area.add_controller(mc);

        // ── Click gesture ────────────────────────────────────────
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_PRIMARY);

        let chunks_click = Rc::clone(&chunks);
        let hover_click = Rc::clone(&hover_rc);
        let pressed_click = Rc::new(RefCell::new(None::<usize>));
        let action_cb_click = Rc::new(RefCell::new(None::<ActionCallback>));
        let direction_click = direction;
        let dma_click = drawing_area.clone();
        let popover_click = Rc::new(RefCell::new(None::<gtk::Popover>));

        // Clone before moving into closures
        let hover_cp = Rc::clone(&hover_click);
        let pressed_cp = Rc::clone(&pressed_click);
        gesture.connect_pressed(move |_gesture, _n_press, _x, _y| {
            let hover = *hover_cp.borrow();
            if let Some(idx) = hover {
                pressed_cp.replace(Some(idx));
            }
        });

        let hover_cr = Rc::clone(&hover_click);
        let pressed_cr = Rc::clone(&pressed_click);
        let chunks_cr = Rc::clone(&chunks_click);
        let action_cr = Rc::clone(&action_cb_click);
        gesture.connect_released(move |_gesture, _n_press, _x, _y| {
            let pressed = *pressed_cr.borrow();
            let hover = *hover_cr.borrow();

            if let (Some(pressed_idx), Some(hover_idx)) = (pressed, hover) {
                if pressed_idx == hover_idx {
                    // Trigger the action
                    let chunks = chunks_cr.borrow();
                    if pressed_idx < chunks.len() {
                        let _chunk = &chunks[pressed_idx];
                        let action = classify_action(_chunk, direction_click);

                        if let Some(cb) = action_cr.borrow().as_ref() {
                            cb(pressed_idx, action);
                        }
                    }
                }
            }
            pressed_cr.replace(None);
        });

        drawing_area.add_controller(gesture);

        // ── Draw function ────────────────────────────────────────
        let draw_source = source_view.clone();
        let draw_dir = direction;
        let draw_chunks = Rc::clone(&chunks);
        let draw_hover = Rc::clone(&hover_rc);
        let draw_pressed = Rc::clone(&pressed_click);
        let draw_buttons = Rc::clone(&buttons_rc);

        drawing_area.set_draw_func(move |da, cr, width, height| {
            let chunks = draw_chunks.borrow();
            let hovered = *draw_hover.borrow();
            let pressed = *draw_pressed.borrow();
            let w = width as f64;
            let h = height as f64;

            let da_w: &gtk::Widget = da.upcast_ref();
            let src_w: &gtk::Widget = draw_source.upcast_ref();

            // Widget Y offset: source view origin in gutter DrawingArea coords
            let (_, src_off) = src_w
                .translate_coordinates(da_w, 0.0, 0.0)
                .unwrap_or((0.0, 0.0));

            // Scroll offset
            let scroll = draw_source.vadjustment().map(|a| a.value()).unwrap_or(0.0);

            // Helper: buffer line → Y in gutter DrawingArea coords
            let line_to_y = |line: usize| -> f64 {
                if line >= draw_source.buffer().line_count() as usize {
                    return 0.0;
                }
                if let Some(iter) = draw_source.buffer().iter_at_line(line as i32) {
                    let rect = draw_source.iter_location(&iter);
                    return rect.y() as f64 - scroll + src_off;
                }
                0.0
            };

            // Background fill
            cr.set_source_rgba(0.96, 0.96, 0.96, 0.9);
            cr.paint().ok();

            let mut new_buttons = Vec::new();

            for (i, chunk) in chunks.iter().enumerate() {
                let chunk_start = if draw_dir == GutterDirection::LeftToRight {
                    chunk.start_a
                } else {
                    chunk.start_b
                };
                let chunk_end = if draw_dir == GutterDirection::LeftToRight {
                    chunk.end_a.max(chunk.start_a + 1)
                } else {
                    chunk.end_b.max(chunk.start_b + 1)
                };

                let y_start = line_to_y(chunk_start);
                let y_end = line_to_y(chunk_end);
                let rect_y = y_start;
                let rect_h = (y_end - y_start).max(4.0);

                let is_hovered = hovered == Some(i);
                let is_pressed = pressed == Some(i);

                // Determine action type for this chunk
                let action = classify_action(chunk, draw_dir);
                if action == GutterAction::Replace && chunk.op == DiffOp::Equal {
                    continue;
                }

                draw_chunk_action(
                    cr, chunk, w, rect_y, rect_h, draw_dir, is_hovered, is_pressed,
                );

                // Track button area for click detection
                let btn_x = 1.0;
                let btn_w = w - 2.0;
                let btn_y = rect_y;
                let btn_h = rect_h;
                new_buttons.push((btn_x, btn_y, btn_x + btn_w, btn_y + btn_h, i));
            }

            *draw_buttons.borrow_mut() = new_buttons;
        });

        Self {
            drawing_area,
            source_view,
            target_view,
            direction,
            chunks,
            hovered_chunk: hover_rc,
            pressed_chunk: pressed_click,
            action_callback: action_cb_click,
            buttons: buttons_rc,
            copy_popover: RefCell::new(None),
        }
    }

    /// The underlying `gtk::DrawingArea` widget.
    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.drawing_area
    }

    /// Update the chunks to display.
    pub fn set_chunks(&self, chunks: &[Chunk]) {
        self.chunks.replace(chunks.to_vec());
        self.drawing_area.queue_draw();
    }

    /// Set the callback invoked when a chunk action is triggered.
    /// The callback receives `(chunk_index, action)`.
    pub fn connect_action<F: Fn(usize, GutterAction) + 'static>(&self, callback: F) {
        self.action_callback.replace(Some(Box::new(callback)));
    }

    /// Get source view (for external use).
    pub fn source_view(&self) -> &gtk::TextView {
        &self.source_view
    }

    /// Get target view (for external use).
    pub fn target_view(&self) -> &gtk::TextView {
        &self.target_view
    }
}

// ─── Action classification ──────────────────────────────────────────

/// Determine which action should be available for a given chunk and direction.
///
/// Mirrors Python Meld's `_classify_change_actions()`:
///   - Replace chunks get the push/replace arrow (►)
///   - Delete chunks get the delete button (×) — removes extra lines from source
///   - Insert chunks get the push arrow (►) — pushes inserted lines back to source
fn classify_action(chunk: &Chunk, _direction: GutterDirection) -> GutterAction {
    match chunk.op {
        DiffOp::Equal => GutterAction::Replace,
        DiffOp::Delete => GutterAction::Delete,
        DiffOp::Insert => GutterAction::Replace,
        DiffOp::Replace => GutterAction::Replace,
    }
}

// ─── Drawing ────────────────────────────────────────────────────────

/// Draw a single chunk action button.
///
/// Mimics the original Meld ActionGutter visual style:
///   - Colored background block for the chunk region
///   - Flat image-button style for the clickable action area
///   - Directional icon (arrow, ×, or copy indicator)
fn draw_chunk_action(
    cr: &cairo::Context,
    chunk: &Chunk,
    width: f64,
    y: f64,
    line_h: f64,
    direction: GutterDirection,
    hovered: bool,
    pressed: bool,
) {
    let h = line_h.max(4.0);
    let margin = 2.0;

    // ── Chunk background color (full gutter width) ──
    let (r, g, b, _a) = match chunk.op {
        DiffOp::Delete => (0.9, 0.3, 0.3, 0.25),
        DiffOp::Insert => (0.3, 0.8, 0.3, 0.25),
        DiffOp::Replace => (0.3, 0.45, 0.95, 0.30),
        DiffOp::Equal => (0.7, 0.7, 0.7, 0.10),
    };

    // Fill the chunk region with the diff colour
    cr.set_source_rgba(r, g, b, 0.18);
    cr.rectangle(-0.5, y + 0.5, width + 1.0, h);
    cr.fill().ok();

    // Border line at chunk edges
    cr.set_source_rgba(r, g, b, 0.5);
    cr.set_line_width(1.0);
    cr.rectangle(-0.5, y + 0.5, width + 1.0, h);
    cr.stroke().ok();

    // ── Current chunk highlight ──
    if pressed {
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.25);
        cr.rectangle(0.0, y, width, h);
        cr.fill().ok();
    }

    // ── Button rendering (flat image-button style) ──
    let button_x = 1.0;
    let button_width = width - 2.0;

    // Button background
    cr.set_source_rgba(
        0.0,
        0.0,
        0.0,
        if pressed {
            0.12
        } else if hovered {
            0.06
        } else {
            0.0
        },
    );
    cr.rectangle(button_x, y + 1.0, button_width, h - 2.0);
    cr.fill().ok();

    // Button border on hover
    if hovered || pressed {
        cr.set_source_rgba(0.0, 0.0, 0.0, if pressed { 0.25 } else { 0.12 });
        cr.set_line_width(1.0);
        cr.rectangle(button_x + 0.5, y + 1.5, button_width - 1.0, h - 3.0);
        cr.stroke().ok();
    }

    // ── Action icon ──
    let cx = width / 2.0;
    let cy = y + h / 2.0;
    let half = (h.min(width) / 2.0 - margin - 2.0).max(3.0);
    let action = classify_action(chunk, direction);

    match action {
        GutterAction::Replace => {
            draw_replace_icon(cr, cx, cy, half, direction);
        }
        GutterAction::Delete => {
            draw_delete_icon(cr, cx, cy, half, width, y, h, margin);
        }
        GutterAction::CopyUp => {
            draw_copy_up_icon(cr, cx, cy, half);
        }
        GutterAction::CopyDown => {
            draw_copy_down_icon(cr, cx, cy, half);
        }
    }
}

/// Draw the replace/push arrow icon (pointing in the action direction).
fn draw_replace_icon(cr: &cairo::Context, cx: f64, cy: f64, half: f64, direction: GutterDirection) {
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.7);
    cr.set_line_width(1.8);
    cr.set_line_join(cairo::LineJoin::Round);

    match direction {
        GutterDirection::LeftToRight => {
            // Right-pointing arrow ►
            cr.move_to(cx - half * 0.7, cy - half);
            cr.line_to(cx + half * 0.8, cy);
            cr.line_to(cx - half * 0.7, cy + half);
        }
        GutterDirection::RightToLeft => {
            // Left-pointing arrow ◄
            cr.move_to(cx + half * 0.7, cy - half);
            cr.line_to(cx - half * 0.8, cy);
            cr.line_to(cx + half * 0.7, cy + half);
        }
    }
    cr.close_path();
    cr.fill().ok();
}

/// Draw the delete × icon.
fn draw_delete_icon(
    cr: &cairo::Context,
    _cx: f64,
    _cy: f64,
    _half: f64,
    width: f64,
    y: f64,
    h: f64,
    margin: f64,
) {
    cr.set_source_rgba(0.7, 0.15, 0.15, 0.85);
    cr.set_line_width(1.8);
    cr.set_line_cap(cairo::LineCap::Round);
    let pad = margin + 2.0;
    cr.move_to(pad + 2.0, y + pad + 2.0);
    cr.line_to(width - pad - 2.0, y + h - pad - 2.0);
    cr.stroke().ok();
    cr.move_to(width - pad - 2.0, y + pad + 2.0);
    cr.line_to(pad + 2.0, y + h - pad - 2.0);
    cr.stroke().ok();
}

/// Draw the copy-up ▲ icon.
fn draw_copy_up_icon(cr: &cairo::Context, cx: f64, cy: f64, half: f64) {
    cr.set_source_rgba(0.0, 0.45, 0.0, 0.85);
    cr.set_line_width(1.0);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.move_to(cx, cy - half);
    cr.line_to(cx - half, cy + half * 0.5);
    cr.line_to(cx + half, cy + half * 0.5);
    cr.close_path();
    cr.fill().ok();
}

/// Draw the copy-down ▼ icon.
fn draw_copy_down_icon(cr: &cairo::Context, cx: f64, cy: f64, half: f64) {
    cr.set_source_rgba(0.0, 0.45, 0.0, 0.85);
    cr.set_line_width(1.0);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.move_to(cx, cy + half);
    cr.line_to(cx - half, cy - half * 0.5);
    cr.line_to(cx + half, cy - half * 0.5);
    cr.close_path();
    cr.fill().ok();
}
