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

use gdk4 as gdk;
use gtk4 as gtk;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::diff::engine::{Chunk, DiffOp};
use crate::ui::style;

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
    /// Current action mode (replace/delete/insert), toggled by keyboard modifiers.
    action_mode: Rc<Cell<ActionMode>>,
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
        let action_mode = Rc::new(Cell::new(ActionMode::Replace));

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
        let action_cb_click = Rc::new(RefCell::new(None::<ActionCallback>));
        let copy_popover: Rc<RefCell<Option<gtk::Popover>>> = Rc::new(RefCell::new(None));
        // Chunk index for the insert-mode popover (not cleared on release)
        let popover_chunk: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));

        // Build the insert-mode popover (Copy Up / Copy Down)
        {
            let popover = gtk::Popover::new();
            let vbox = gtk::Box::new(gtk::Orientation::Vertical, 2);

            let up_btn = gtk::Button::with_label("Copy Up");
            let down_btn = gtk::Button::with_label("Copy Down");

            let cb_pop = Rc::clone(&action_cb_click);
            let chunks_pop = Rc::clone(&chunks);
            let pc_pop = Rc::clone(&popover_chunk);
            let pop_clone = popover.clone();
            up_btn.connect_clicked(move |_| {
                let chunks = chunks_pop.borrow();
                if let Some(cb) = cb_pop.borrow().as_ref() {
                    if let Some(idx) = pc_pop.get() {
                        if idx < chunks.len() {
                            cb(idx, GutterAction::CopyUp);
                        }
                    }
                }
                pop_clone.popdown();
            });

            let cb_pop2 = Rc::clone(&action_cb_click);
            let chunks_pop2 = Rc::clone(&chunks);
            let pc_pop2 = Rc::clone(&popover_chunk);
            let pop_clone2 = popover.clone();
            down_btn.connect_clicked(move |_| {
                let chunks = chunks_pop2.borrow();
                if let Some(cb) = cb_pop2.borrow().as_ref() {
                    if let Some(idx) = pc_pop2.get() {
                        if idx < chunks.len() {
                            cb(idx, GutterAction::CopyDown);
                        }
                    }
                }
                pop_clone2.popdown();
            });

            vbox.append(&up_btn);
            vbox.append(&down_btn);
            popover.set_child(Some(&vbox));
            popover.set_parent(&drawing_area);
            *copy_popover.borrow_mut() = Some(popover);
        }

        let hover_press = Rc::clone(&hover_click);
        let pressed_press = Rc::clone(&pressed_click);
        gesture.connect_pressed(move |_gesture, _n, _x, _y| {
            pressed_press.replace(*hover_press.borrow());
        });

        let hover_release = Rc::clone(&hover_click);
        let pressed_release = Rc::clone(&pressed_click);
        let chunks_release = Rc::clone(&chunks_click);
        let action_release = Rc::clone(&action_cb_click);
        let mode_release = Rc::clone(&action_mode);
        let popover_release = Rc::clone(&copy_popover);
        gesture.connect_released(move |_gesture, _n, x, y| {
            let pressed = *pressed_release.borrow();
            let hover = *hover_release.borrow();

            if let (Some(p), Some(h)) = (pressed, hover) {
                if p == h {
                    let action = {
                        let chunks = chunks_release.borrow();
                        if p < chunks.len() {
                            Some(classify_action(&chunks[p], mode_release.get()))
                        } else {
                            None
                        }
                    };
                    match action {
                        // Insert mode: show popover instead of immediate action
                        Some(GutterAction::CopyUp | GutterAction::CopyDown) => {
                            popover_chunk.set(Some(p));
                            if let Some(pop) = popover_release.borrow().as_ref() {
                                let rect = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
                                pop.set_pointing_to(Some(&rect));
                                pop.popup();
                            }
                        }
                        Some(action) => {
                            if let Some(cb) = action_release.borrow().as_ref() {
                                cb(p, action);
                            }
                        }
                        None => {}
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
        let draw_mode = Rc::clone(&action_mode);

        drawing_area.set_draw_func(move |da, cr, width, height| {
            let chunks = draw_chunks.borrow();
            let hovered = *draw_hover.borrow();
            let pressed = *draw_pressed.borrow();
            let w = width as f64;
            let mode = draw_mode.get();

            let da_w: &gtk::Widget = da.upcast_ref();
            let src_w: &gtk::Widget = draw_source.upcast_ref();

            // Widget offset + scroll offset (matching Meld's coordinate system)
            let (_, src_off) = src_w
                .translate_coordinates(da_w, 0.0, 0.0)
                .unwrap_or((0.0, 0.0));
            let scroll = draw_source.vadjustment().map(|a| a.value()).unwrap_or(0.0);

            let line_to_y = |line: usize| -> f64 {
                let buf = draw_source.buffer();
                if line > buf.line_count() as usize {
                    return 0.0;
                }
                let is_end = line == buf.line_count() as usize;
                // Use the full line y-range (line top, including
                // `pixels-above-lines`) so the gutter fill aligns with the
                // panes' paragraph-background chunk bands. `iter_location`
                // returns the glyph top instead — a couple of pixels lower —
                // which left the fill misaligned with the connectors/panes.
                // Round to the pixel grid so the fill snaps to the same device
                // row as the panes' pixel-snapped paragraph background.
                if is_end {
                    let iter = buf.end_iter();
                    let (y, h) = draw_source.line_yrange(&iter);
                    return ((y + h) as f64 - scroll + src_off).round();
                }
                let Some(iter) = buf.iter_at_line(line as i32) else {
                    return 0.0;
                };
                let (y, _) = draw_source.line_yrange(&iter);
                (y as f64 - scroll + src_off).round()
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

                // Real chunk range in the *source* pane.  A zero-span range
                // (an insertion on the side without content) is drawn as a
                // line only, matching Meld's action gutter.
                let (cs, ce) = if draw_dir == GutterDirection::LeftToRight {
                    (chunk.start_a, chunk.end_a)
                } else {
                    (chunk.start_b, chunk.end_b)
                };
                let content = ce > cs;

                let y_start = line_to_y(cs);
                let y_end = if content { line_to_y(ce) } else { y_start };
                let rect_y = y_start;
                let rect_h = if content {
                    (y_end - y_start).max(4.0)
                } else {
                    0.0
                };
                // Hit area is always tall enough to click comfortably.
                let hit_h = (y_end - y_start).max(4.0);

                let is_hovered = hovered == Some(i);
                let is_pressed = pressed == Some(i);

                draw_chunk_action(
                    cr, chunk, w, rect_y, rect_h, content, draw_dir, is_hovered, is_pressed, mode,
                );

                // Hit-test rectangle
                new_buttons.push((1.0, rect_y, w - 1.0, rect_y + hit_h, i));
            }

            *draw_buttons.borrow_mut() = new_buttons;
        });

        Self {
            drawing_area,
            source_view,
            target_view,
            direction,
            chunks,
            action_mode,
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

    /// Expose the action-mode cell so external key controllers can update it.
    pub fn action_mode_cell(&self) -> Rc<Cell<ActionMode>> {
        Rc::clone(&self.action_mode)
    }

    pub fn source_view(&self) -> &gtk::TextView {
        &self.source_view
    }

    pub fn target_view(&self) -> &gtk::TextView {
        &self.target_view
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────

/// Classify the appropriate action for a chunk based on the current mode.
fn classify_action(chunk: &Chunk, mode: ActionMode) -> GutterAction {
    match mode {
        ActionMode::Delete => GutterAction::Delete,
        ActionMode::Insert => {
            // Insert mode shows copy-up/copy-down popup
            GutterAction::CopyDown
        }
        ActionMode::Replace => GutterAction::Replace,
    }
}

// ─── Drawing ──────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_chunk_action(
    cr: &cairo::Context,
    chunk: &Chunk,
    width: f64,
    y: f64,
    line_h: f64,
    content: bool,
    direction: GutterDirection,
    hovered: bool,
    pressed: bool,
    mode: ActionMode,
) {
    let (lr, lg, lb) = match style::line_color(chunk.op) {
        Some(c) => c,
        None => return,
    };
    let h = line_h.max(4.0);
    cr.set_line_width(1.0);

    if content {
        // Opaque fill + outline (Meld's fill_colors / line_colors), so the
        // gutter reads as solid colour continuous with the panes.  The fill
        // sits on the exact line tops (`y`, no half-pixel offset) so it lines
        // up with the link-map connector and the panes' paragraph backgrounds
        // with no 1px step at the boundary.
        if let Some((r, g, b)) = style::fill_color(chunk.op) {
            cr.set_source_rgb(r, g, b);
            cr.rectangle(-0.5, y, width + 1.0, h);
            cr.fill().ok();
        }
        cr.set_source_rgb(lr, lg, lb);
        cr.rectangle(-0.5, y - 0.5, width + 1.0, h);
        cr.stroke().ok();
    } else {
        // Zero-span (insertion on the side without content): a single line
        // at the insertion point, matching Meld.
        cr.set_source_rgb(lr, lg, lb);
        cr.move_to(-0.5, y + 0.5);
        cr.line_to(width + 1.0, y + 0.5);
        cr.stroke().ok();
    }

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

    // Action icon (centered).  Drawn subtly — faint when idle and brighter
    // on hover/press — mirroring Meld's flat image-button gutter icons.
    let cx = width / 2.0;
    let cy = y + h / 2.0;
    let half = 5.5;
    let icon_alpha = if pressed {
        0.95
    } else if hovered {
        0.85
    } else {
        0.55
    };
    let action = classify_action(chunk, mode);

    match action {
        GutterAction::Replace => draw_arrow_icon(cr, cx, cy, half, direction, icon_alpha),
        GutterAction::Delete => draw_delete_icon(cr, cx, cy, half, icon_alpha),
        GutterAction::CopyUp | GutterAction::CopyDown => {
            draw_insert_icon(cr, cx, cy, half, icon_alpha)
        }
    }
}

/// A slim directional arrow (replace / apply) pointing in the copy direction,
/// stroked rather than filled so it reads as a light Meld-style action icon.
fn draw_arrow_icon(
    cr: &cairo::Context,
    cx: f64,
    cy: f64,
    half: f64,
    direction: GutterDirection,
    alpha: f64,
) {
    cr.set_source_rgba(0.2, 0.2, 0.2, alpha);
    cr.set_line_width(1.5);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    let dir = match direction {
        GutterDirection::LeftToRight => 1.0,
        GutterDirection::RightToLeft => -1.0,
    };
    let tip = cx + dir * half;
    let tail = cx - dir * half;
    // Shaft
    cr.move_to(tail, cy);
    cr.line_to(tip, cy);
    cr.stroke().ok();
    // Arrowhead
    cr.move_to(tip - dir * half * 0.7, cy - half * 0.7);
    cr.line_to(tip, cy);
    cr.line_to(tip - dir * half * 0.7, cy + half * 0.7);
    cr.stroke().ok();
}

fn draw_delete_icon(cr: &cairo::Context, cx: f64, cy: f64, half: f64, alpha: f64) {
    cr.set_source_rgba(0.55, 0.15, 0.15, alpha);
    cr.set_line_width(1.5);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.move_to(cx - half, cy - half);
    cr.line_to(cx + half, cy + half);
    cr.stroke().ok();
    cr.move_to(cx + half, cy - half);
    cr.line_to(cx - half, cy + half);
    cr.stroke().ok();
}

fn draw_insert_icon(cr: &cairo::Context, cx: f64, cy: f64, half: f64, alpha: f64) {
    cr.set_source_rgba(0.15, 0.45, 0.15, alpha);
    cr.set_line_width(1.5);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.move_to(cx - half, cy);
    cr.line_to(cx + half, cy);
    cr.stroke().ok();
    cr.move_to(cx, cy - half);
    cr.line_to(cx, cy + half);
    cr.stroke().ok();
}
