#![cfg(feature = "gui")]
//! Line-number gutter renderer that paints the chunk background behind the
//! numbers, mirroring Meld's `GutterRendererChunkLines`.
//!
//! Replaces GtkSourceView's built-in line-number gutter so the chunk fill
//! colour can be drawn *opaquely behind* each line number (something the
//! built-in renderer doesn't allow), matching the paragraph background of the
//! text area exactly.
//!
//! ## Why the background is drawn in `snapshot` and not `snapshot_line`
//!
//! GtkSourceView pushes a per-line clip of `(0, y, width, cell_height)` around
//! every `snapshot_line` call. The strip of inter-line spacing between two
//! cells therefore lies outside *every* per-line clip and can never be painted
//! from `snapshot_line` — which left a 1px white seam between consecutive lines
//! of the same chunk. Instead we capture the visible `GutterLines` in `begin`
//! and paint the chunk backgrounds as continuous, gap-free bands from the full
//! widget `snapshot` (no per-line clip is active there), then chain up to the
//! parent so `snapshot_line` only draws the numbers on top.

use gdk4 as gdk;
use gtk4 as gtk;
use gtk4::graphene;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use sourceview5 as gsv;
use sourceview5::prelude::*;
use sourceview5::subclass::prelude::*;

use crate::diff::engine::{Chunk, DiffOp};
use crate::ui::style;

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub struct ChunkGutterRenderer {
        pub chunks: RefCell<Vec<Chunk>>,
        pub pane: Cell<usize>,
        /// Visible lines for the current snapshot cycle, captured in `begin`
        /// and cleared in `end`. Used to paint continuous chunk bands.
        pub lines: RefCell<Option<gsv::GutterLines>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ChunkGutterRenderer {
        const NAME: &'static str = "MeldChunkGutterRenderer";
        type Type = super::ChunkGutterRenderer;
        type ParentType = gsv::GutterRenderer;
    }

    impl ObjectImpl for ChunkGutterRenderer {}

    impl WidgetImpl for ChunkGutterRenderer {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            if orientation != gtk::Orientation::Horizontal {
                return (0, 0, -1, -1);
            }
            let widget = self.obj();
            let digit_w = widget.create_pango_layout(Some("0")).pixel_size().0.max(1);
            let line_count = widget.buffer().map(|b| b.line_count().max(1)).unwrap_or(1);
            let digits = line_count.to_string().len().max(2) as i32;
            let width = Self::LEFT_PAD + digits * digit_w + Self::RIGHT_PAD;
            (width, width, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            // Paint the chunk backgrounds first as gap-free bands, behind the
            // numbers. No per-line clip is active here (unlike in
            // `snapshot_line`), so a band covering several lines also covers the
            // inter-line spacing between them, leaving no white seam.
            self.snapshot_chunk_bands(snapshot);
            // Let the parent draw the line numbers via `snapshot_line`.
            self.parent_snapshot(snapshot);
        }
    }

    impl GutterRendererImpl for ChunkGutterRenderer {
        fn begin(&self, lines: &gsv::GutterLines) {
            self.lines.replace(Some(lines.clone()));
            self.parent_begin(lines);
        }

        fn end(&self) {
            self.lines.replace(None);
            self.parent_end();
        }

        fn snapshot_line(&self, snapshot: &gtk::Snapshot, lines: &gsv::GutterLines, line: u32) {
            let widget = self.obj();
            let (y, height) = lines.line_yrange(line, gsv::GutterRendererAlignmentMode::Cell);
            let width = widget.width();

            // Right-aligned line number, vertically centred in the cell. The
            // chunk background is painted separately in `snapshot` so it spans
            // the inter-line gaps continuously.
            let num = (line + 1).to_string();
            let layout = widget.create_pango_layout(Some(&num));
            let (lw, lh) = layout.pixel_size();
            let tx = (width - Self::RIGHT_PAD - lw) as f32;
            let ty = y as f32 + (height - lh) as f32 / 2.0;
            snapshot.save();
            snapshot.translate(&graphene::Point::new(tx, ty));
            snapshot.append_layout(&layout, &widget.color());
            snapshot.restore();
        }
    }

    impl ChunkGutterRenderer {
        /// Padding to the left of the digits (toward the link map).
        pub(super) const LEFT_PAD: i32 = 4;
        /// Padding to the right of the digits (toward the text). With the
        /// view's `left-margin` set to 0 this is the sole gap between the
        /// number and the first character; the chunk gutter paints it in the
        /// chunk colour so changed lines stay continuous into the text.
        pub(super) const RIGHT_PAD: i32 = 7;

        /// Paint the chunk backgrounds as continuous vertical bands. Runs of
        /// consecutive visible lines sharing the same chunk colour are merged
        /// into a single rectangle so the inter-line spacing inside a chunk is
        /// filled and no white seam appears between rows.
        fn snapshot_chunk_bands(&self, snapshot: &gtk::Snapshot) {
            let lines_ref = self.lines.borrow();
            let Some(lines) = lines_ref.as_ref() else {
                return;
            };
            let width = self.obj().width() as f32;
            if width <= 0.0 {
                return;
            }

            // Current open band: (colour, top, bottom).
            let mut band: Option<(gdk::RGBA, f32, f32)> = None;
            let flush = |band: &mut Option<(gdk::RGBA, f32, f32)>, snapshot: &gtk::Snapshot| {
                if let Some((color, top, bottom)) = band.take() {
                    snapshot.append_color(
                        &color,
                        &graphene::Rect::new(0.0, top, width, bottom - top),
                    );
                }
            };

            for line in lines.first()..=lines.last() {
                let (y, h) = lines.line_yrange(line, gsv::GutterRendererAlignmentMode::Cell);
                let (top, bottom) = (y as f32, (y + h) as f32);
                match self.bg_for_line(line as usize) {
                    Some(color) => {
                        // Same chunk colour as the open band: extend it down to
                        // this line's bottom, swallowing the inter-line gap.
                        if let Some(open) = band.as_mut() {
                            if open.0 == color {
                                open.2 = bottom;
                                continue;
                            }
                        }
                        // A coloured line that starts or breaks a band.
                        flush(&mut band, snapshot);
                        band = Some((color, top, bottom));
                    }
                    // An equal (uncoloured) line ends any open band.
                    None => flush(&mut band, snapshot),
                }
            }
            flush(&mut band, snapshot);
        }

        fn bg_for_line(&self, line: usize) -> Option<gdk::RGBA> {
            let pane = self.pane.get();
            for chunk in self.chunks.borrow().iter() {
                if chunk.op == DiffOp::Equal {
                    continue;
                }
                let (cs, ce) = if pane == 0 {
                    (chunk.start_a, chunk.end_a)
                } else {
                    (chunk.start_b, chunk.end_b)
                };
                if line >= cs && line < ce {
                    let (r, g, b) = style::fill_color(chunk.op)?;
                    return Some(gdk::RGBA::new(r as f32, g as f32, b as f32, 1.0));
                }
            }
            None
        }
    }
}

glib::wrapper! {
    pub struct ChunkGutterRenderer(ObjectSubclass<imp::ChunkGutterRenderer>)
        @extends gsv::GutterRenderer, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ChunkGutterRenderer {
    /// Create a renderer for `pane` (0 = left/A, 1 = right/B).
    pub fn new(pane: usize) -> Self {
        let obj: Self = glib::Object::new();
        obj.imp().pane.set(pane);
        obj.set_alignment_mode(gsv::GutterRendererAlignmentMode::Cell);
        obj.set_xalign(1.0);
        obj.set_yalign(0.5);
        obj
    }

    /// Update the chunk list and trigger a redraw.
    pub fn set_chunks(&self, chunks: &[Chunk]) {
        self.imp().chunks.replace(chunks.to_vec());
        self.queue_draw();
    }
}
