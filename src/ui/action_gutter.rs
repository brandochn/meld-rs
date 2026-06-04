//! Action gutter widget — buttons for moving changes between diff panes.
//!
//! Ported from Meld's `meld/actiongutter.py`.  Renders per-chunk action
//! buttons (replace / delete / copy) between two text views and dispatches
//! chunk operations on click.
//!
//! Key behaviour matching Meld:
//!   * Replace mode (default) — copy chunk from source → target
//!   * Delete mode (Shift)    — remove chunk from source
//!   * Insert mode (Ctrl)     — copy chunk to target (popup for direction)

use gtk4 as gtk;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use crate::diff::engine::{Chunk, DiffOp};

/// Which direction the gutter "points" (left-to-right or right-to-left).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GutterDirection {
    LeftToRight,
    RightToLeft,
}

/// The kind of chunk operation to perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GutterAction {
    Replace,
    Delete,
    CopyUp,
    CopyDown,
}

/// Callback signature: `fn(chunk_index: usize, action: GutterAction)`.
pub type ActionCallback = Box<dyn Fn(usize, GutterAction) + 'static>;

/// Action mode selected by keyboard modifiers (matching Meld's ActionMode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionMode {
    Replace,
    Delete,
    Insert,
}

/// A single action-gutter strip that sits between two text views.
pub struct ActionGutter {
    drawing_area: gtk::DrawingArea,
    source_view: gtk::TextView,
    target_view: gtk::TextView,
    direction: GutterDirection,
    chunks: Rc<RefCell<Vec<Chunk>>>,
    /// Currently hovered button index.
    hovered_chunk: Rc<RefCell<Option<usize>>>,
    /// Currently pressed button index.
    pressed_chunk: Rc<RefCell<Option<usize>>>,
    /// Registered action callback.
    action_callback: Rc<RefCell<Option<ActionCallback>>>,
    /// Button hit-test rectangles: (x1, y1, x2, y2, chunk_index).
    buttons: Rc<RefCell<Vec<(f64, f64, f64, f64, usize)>>>,
    /// Pending copy-up/-down popover (if any).
    copy_popover: RefCell<Option<gtk::Popover>>,
}

impl ActionGutter {
    /// Create a new action gutter between `source` and `target`.
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
        let da_weak = drawing_area.downgrade();
        if let Some(vadj) = source_view.vadjustment() {
            vadj.connect_value_changed(move |_| {
                if let Some(d) = da_weak.upgrade() {
                    d.queue_draw();
                }
            });
        }

        // ── Hover tracking (motion controller) ───────────────────
        let hover_rc = Rc::clone(&hovered);
        let buttons_mc = Rc::clone(&buttons);
        let dma_motion = drawing_area.clone();

        let mc = gtk::EventControllerMotion::new();
        let hover_inner = Rc::clone(&hover_rc);
        let buttons_inner = Rc::clone(&buttons_mc);
        mc.connect_motion(move |_, x, y| {
            let btns = buttons_inner.borrow();
            let mut new_hover: Option<usize> = None;
            for (x1, y1, x2, y2, idx) in btns.iter() {
                if x >= *x1 && x <= *x2 && y >= *y1 && y <= *y2 {
                    new_hover = Some(*idx);
                    break;
                }
            }
            if *hover_inner.borrow() != new_hover {
                hover_inner.replace(new_hover);
                dma_motion.queue_draw();
            }
        });
        let dma_leave = drawing_area.clone();
        let hover_leave = Rc::clone(&hover_rc);
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
        let hover_click = Rc::clone(&hovered);
        let pressed_click = Rc::clone(&pressed);
        let dma_click = drawing_area.clone();
        let popover_click = Rc::new(RefCell::new(None::<gtk::Popover>));
        let action_cb_click = Rc::new(RefCell::new(None::<ActionCallback>));

        let hover_press = Rc::clone(&hover_click);
        let pressed_press = Rc::clone(&pressed_click);
        gesture.connect_pressed(move |_gesture, _n, _x, _y| {
            pressed_press.replace(*hover_press.borrow());
        });

        let hover_release = Rc::clone(&hover_click);
        let pressed_release = Rc::clone(&pressed_click);
        let chunks_release = Rc::clone(&chunks_click);
        let action_release = Rc::clone(&action_cb_click);
        gesture.connect_released(move |_gesture, _n, _x, _y| {
            let pressed = *pressed_release.borrow();
            let hover = *hover_release.borrow();

            if let (Some(p), Some(h)) = (pressed, hover) {
                if p == h {
                    let action = {
                        let chunks = chunks_release.borrow();
                        if p < chunks.len() {
                            Some(classify_action(&chunks[p]))
                        } else {
                            None
                        }
                    };
                    if let (Some(action), Some(cb)) =
                        (action, action_release.borrow().as_ref())
                    {
                        cb(p, action);
                    }
                }
            }
            pressed_release.replace(None);
        });
        drawing_area.add_controller(gesture);

        // ── Draw function ────────────────────────────────────────
        let draw_source = source_view.clone();
        let draw_dir = direction;
        let draw_chunks = Rc::clone(&chunks);
        let draw_hover = Rc::clone(&hovered);
        let draw_pressed = Rc::clone(&pressed);
        let draw_buttons = Rc::clone(&buttons);

        drawing_area.set_draw_func(move |da, cr, width, height| {
            let chunks = draw_chunks.borrow();
            let hovered = *draw_hover.borrow();
            let pressed = *draw_pressed.borrow();
            let w = width as f64;

            let da_w: &gtk::Widget = da.upcast_ref();
            let src_w: &gtk::Widget = draw_source.upcast_ref();

            // Widget offset + scroll offset (matching Meld's coordinate system)
            let (_, src_off) = src_w
                .translate_coordinates(da_w, 0.0, 0.0)
                .unwrap_or((0.0, 0.0));
            let scroll = draw_source.vadjustment().map(|a| a.value()).unwrap_or(0.0);

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

            // Background is handled by CSS (.action-gutter { background: transparent })
            // so paragraph backgrounds from the text view can visually bridge
            // across the gutter to the link-map bezier curves.

            let mut new_buttons = Vec::new();

            for (i, chunk) in chunks.iter().enumerate() {
                if chunk.op == DiffOp::Equal {
                    continue;
                }

                if draw_dir == GutterDirection::RightToLeft && chunk.op == DiffOp::Delete {
                    continue;
                }

                let (chunk_start, chunk_end) = if draw_dir == GutterDirection::LeftToRight {
                    (chunk.start_a, chunk.end_a.max(chunk.start_a + 1))
                } else {
                    (chunk.start_b, chunk.end_b.max(chunk.start_b + 1))
                };

                let y_start = line_to_y(chunk_start);
                let y_end = line_to_y(chunk_end);
                let rect_y = y_start;
                let rect_h = (y_end - y_start).max(4.0);

                let is_hovered = hovered == Some(i);
                let is_pressed = pressed == Some(i);

                draw_chunk_action(
                    cr, chunk, w, rect_y, rect_h, draw_dir, is_hovered, is_pressed,
                );

                // Hit-test rectangle
                new_buttons.push((1.0, rect_y, w - 1.0, rect_y + rect_h, i));
            }

            *draw_buttons.borrow_mut() = new_buttons;
        });

        Self {
            drawing_area,
            source_view,
            target_view,
            direction,
            chunks,
            hovered_chunk: hovered,
            pressed_chunk: pressed,
            action_callback: action_cb_click,
            buttons,
            copy_popover: RefCell::new(None),
        }
    }

    /// The underlying GTK widget.
    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.drawing_area
    }

    /// Update displayed chunks (called after a diff recompute).
    pub fn set_chunks(&self, chunks: &[Chunk]) {
        self.chunks.replace(chunks.to_vec());
        self.drawing_area.queue_draw();
    }

    /// Register a callback for chunk actions.
    pub fn connect_action<F: Fn(usize, GutterAction) + 'static>(&self, callback: F) {
        self.action_callback.replace(Some(Box::new(callback)));
    }

    pub fn source_view(&self) -> &gtk::TextView {
        &self.source_view
    }

    pub fn target_view(&self) -> &gtk::TextView {
        &self.target_view
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────

/// Classify the appropriate action for a chunk (matching Meld's
/// `_classify_change_actions` simplified for Replace-only mode).
fn classify_action(_chunk: &Chunk) -> GutterAction {
    GutterAction::Replace
}

// ─── Drawing ──────────────────────────────────────────────────────────

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

    // Chunk background (matching Meld's fill_colors)
    let (r, g, b) = match chunk.op {
        DiffOp::Delete | DiffOp::Insert => (0.816, 1.0, 0.639),
        DiffOp::Replace => (0.741, 0.867, 1.0),
        DiffOp::Equal => return,
    };

    // Fill the chunk region (matching link_map bezier fill alpha).
    // Insert chunks are filled only on the side that has content
    // (RightToLeft gutter uses start_b..end_b which has span).
    cr.set_source_rgba(r, g, b, 0.35);
    cr.rectangle(-0.5, y + 0.5, width + 1.0, h);
    cr.fill().ok();

    // Border (Meld's line_colors)
    let (lr, lg, lb) = match chunk.op {
        DiffOp::Delete | DiffOp::Insert => (0.647, 1.0, 0.298),
        DiffOp::Replace => (0.396, 0.698, 1.0),
        DiffOp::Equal => return,
    };
    cr.set_source_rgba(lr, lg, lb, 0.5);
    cr.set_line_width(1.0);
    cr.rectangle(-0.5, y + 0.5, width + 1.0, h);
    cr.stroke().ok();

    // LeftToRight Insert has no content on the source side —
    // skip action button, matching Meld's _classify_change_actions → None.
    if chunk.op == DiffOp::Insert && direction == GutterDirection::LeftToRight {
        return;
    }

    // Pressed highlight
    if pressed {
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.25);
        cr.rectangle(0.0, y, width, h);
        cr.fill().ok();
    }

    // Button background
    let button_x = 1.0;
    let button_width = width - 2.0;
    let alpha = if pressed {
        0.12
    } else if hovered {
        0.06
    } else {
        0.0
    };
    if alpha > 0.0 {
        cr.set_source_rgba(0.0, 0.0, 0.0, alpha);
        cr.rectangle(button_x, y + 1.0, button_width, h - 2.0);
        cr.fill().ok();
    }

    // Button border on hover/press
    if hovered || pressed {
        cr.set_source_rgba(0.0, 0.0, 0.0, if pressed { 0.25 } else { 0.12 });
        cr.set_line_width(1.0);
        cr.rectangle(button_x + 0.5, y + 1.5, button_width - 1.0, h - 3.0);
        cr.stroke().ok();
    }

    // Action icon (centered)
    let margin = 2.0;
    let cx = width / 2.0;
    let cy = y + h / 2.0;
    let half = (h.min(width) / 2.0 - margin - 2.0).max(3.0);
    let action = classify_action(chunk);

    match action {
        GutterAction::Replace => draw_replace_icon(cr, cx, cy, half, direction),
        GutterAction::Delete => draw_delete_icon(cr, width, y, h),
        _ => {}
    }
}

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

fn draw_delete_icon(cr: &cairo::Context, width: f64, y: f64, h: f64) {
    cr.set_source_rgba(0.7, 0.15, 0.15, 0.85);
    cr.set_line_width(1.8);
    cr.set_line_cap(cairo::LineCap::Round);
    let pad = 4.0;
    cr.move_to(pad, y + pad);
    cr.line_to(width - pad, y + h - pad);
    cr.stroke().ok();
    cr.move_to(width - pad, y + pad);
    cr.line_to(pad, y + h - pad);
    cr.stroke().ok();
}
