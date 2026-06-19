#![cfg(feature = "gui")]
//! Line-number gutter renderer that paints the chunk background behind the
//! numbers, mirroring Meld's `GutterRendererChunkLines`.
//!
//! Replaces GtkSourceView's built-in line-number gutter so the chunk fill
//! colour can be drawn *opaquely behind* each line number (something the
//! built-in renderer doesn't allow), matching the paragraph background of the
//! text area exactly.

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
            let width = digits * digit_w + 2 * Self::XPAD;
            (width, width, -1, -1)
        }
    }

    impl GutterRendererImpl for ChunkGutterRenderer {
        fn snapshot_line(&self, snapshot: &gtk::Snapshot, lines: &gsv::GutterLines, line: u32) {
            let widget = self.obj();
            let (y, height) = lines.line_yrange(line, gsv::GutterRendererAlignmentMode::Cell);
            let width = widget.width();

            // Opaque chunk background behind the number.
            if let Some(rgba) = self.bg_for_line(line as usize) {
                snapshot.append_color(
                    &rgba,
                    &graphene::Rect::new(0.0, y as f32, width as f32, height as f32),
                );
            }

            // Right-aligned line number, vertically centred in the cell.
            let num = (line + 1).to_string();
            let layout = widget.create_pango_layout(Some(&num));
            let (lw, lh) = layout.pixel_size();
            let tx = (width - lw - Self::XPAD) as f32;
            let ty = y as f32 + (height - lh) as f32 / 2.0;
            snapshot.save();
            snapshot.translate(&graphene::Point::new(tx, ty));
            snapshot.append_layout(&layout, &widget.color());
            snapshot.restore();
        }
    }

    impl ChunkGutterRenderer {
        pub(super) const XPAD: i32 = 4;

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
