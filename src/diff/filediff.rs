#![cfg(feature = "gui")]
//! Full file comparison view matching the original Meld `filediff.py`.
//!
//! Layout (left-to-right):
//!   [pane0] [action_gutter0] [link_map0] [action_gutter1] [pane1]
//!     optionally [action_gutter2] [link_map1] [action_gutter3] [pane2]
//!
//! Each pane contains an ActionBar (save button + file label), MsgArea,
//! ScrolledWindow with GtkSourceView, and StatusBar.
//!
//! Action gutters sit between panes and display per-chunk action buttons
//! (push/replace, delete, copy-up, copy-down).

use gio::prelude::*;
use gtk4 as gtk;
use gtk4::prelude::*;
use pango;
use sourceview5 as gsv;
use sourceview5::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::diff::engine::{
    merge_adjacent_replace_chunks, Chunk, DiffOp, Differ, InlineChange, InlineDiffer, LineCache,
};
use crate::diff::inline_cache::InlineDiffCache;
use crate::diff::movement::MoveMap;
use crate::diff::similarity::{SimilarityEntry, SimilarityMap};
use crate::ui::action_gutter::{ActionGutter, GutterAction, GutterDirection};
use crate::ui::link_map::LinkMap;
use crate::ui::msgarea::MsgArea;
use crate::ui::statusbar::StatusBar;
use crate::window::MeldPage;

// ─── FileDiff ───────────────────────────────────────────────────────

/// The main file-comparison view supporting 2 or 3 panes.
pub struct FileDiff {
    /// Top-level vertical container.
    container: gtk::Box,
    /// Per-pane data.
    panes: Vec<PaneData>,
    /// Total number of text panes (2 or 3).
    num_panes: usize,
    /// Cached diff chunks shared across gutters and highlights.
    chunks: Rc<RefCell<Vec<Chunk>>>,
    /// Optional merge output path.
    merge_output: Rc<RefCell<Option<String>>>,
    /// Per-pane display labels.
    labels: Rc<RefCell<Vec<String>>>,
    /// Action gutters (one per adjacent pane pair, two per pair for
    /// bidirectional actions).
    gutters: Vec<Rc<ActionGutter>>,
    /// Link maps (one per adjacent pane pair).
    link_maps: Vec<Rc<LinkMap>>,
    /// Shared message area at the top.
    shared_msgarea: Rc<MsgArea>,
    /// Guard against recomputing diffs during programmatic buffer changes.
    loading: Rc<Cell<bool>>,
    /// Currently selected chunk index for gutter operations, if any.
    current_chunk_idx: Rc<Cell<Option<usize>>>,
    /// Tracks the currently focused pane index for action targeting.
    focused_pane: Rc<Cell<usize>>,
    /// O(1) line-to-chunk mapping for fast navigation (mirrors Meld's line cache).
    line_cache: Rc<RefCell<LineCache>>,
    /// LRU cache for inline (character-level) diff results to avoid recomputation.
    inline_cache: Rc<InlineDiffCache>,
    /// Cross-line similarity matches for non-aligned changes.
    similarity_map: Rc<RefCell<SimilarityMap>>,
    /// Detected code movements (relocated blocks).
    move_map: Rc<RefCell<MoveMap>>,
    /// Token-level relations for moved identifiers (visual connectors).
    token_relations: Rc<RefCell<Vec<crate::ui::link_map::TokenRelation>>>,
}

/// Per-pane data bundles the widgets that make up one column.
struct PaneData {
    scrolled: gtk::ScrolledWindow,
    view: gsv::View,
    buffer: gsv::Buffer,
    msgarea: Rc<MsgArea>,
    statusbar: Rc<StatusBar>,
    save_button: gtk::Button,
    file_label: gtk::Label,
}

impl FileDiff {
    // ── Constructor ──────────────────────────────────────────────

    /// Create a new `FileDiff` with the given number of text panes
    /// (typically 2 for file diff, 3 for merge).
    pub fn new(num_panes: usize) -> Self {
        assert!(
            num_panes >= 2 && num_panes <= 3,
            "FileDiff requires 2 or 3 panes"
        );

        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.add_css_class("meld-notebook-child");

        // ── Shared message area ──
        let shared_msgarea = Rc::new(MsgArea::new());
        container.append(shared_msgarea.widget());

        // ── Main horizontal grid ──
        let grid = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        grid.set_vexpand(true);
        grid.set_hexpand(true);

        let mut panes: Vec<PaneData> = Vec::with_capacity(num_panes);
        let mut labels: Vec<String> = Vec::with_capacity(num_panes);

        // ── Build each pane column ──
        for i in 0..num_panes {
            let pane = Self::build_pane_column(i, num_panes);
            labels.push(format!("File {}", i + 1));
            panes.push(pane);
        }

        // ── Build action gutters and link maps ──
        let mut gutters: Vec<Rc<ActionGutter>> = Vec::new();
        let mut link_maps: Vec<Rc<LinkMap>> = Vec::new();

        // Between pane 0 and pane 1
        if num_panes >= 2 {
            // Gutter: push from left (0→1)
            let ag0 = Rc::new(ActionGutter::new(
                panes[0].view.clone().upcast::<gtk::TextView>(),
                panes[1].view.clone().upcast::<gtk::TextView>(),
                GutterDirection::LeftToRight,
            ));
            gutters.push(Rc::clone(&ag0));

            // Link map between 0 and 1
            let lm0 = Rc::new(LinkMap::new(
                &[],
                panes[0].buffer.line_count().max(0) as usize,
                panes[1].buffer.line_count().max(0) as usize,
            ));
            lm0.associate(&panes[0].view, &panes[1].view);
            link_maps.push(Rc::clone(&lm0));

            // Gutter: push from right (1→0)
            let ag1 = Rc::new(ActionGutter::new(
                panes[1].view.clone().upcast::<gtk::TextView>(),
                panes[0].view.clone().upcast::<gtk::TextView>(),
                GutterDirection::RightToLeft,
            ));
            gutters.push(Rc::clone(&ag1));
        }

        // Between pane 1 and pane 2 (3-way merge)
        if num_panes >= 3 {
            let ag2 = Rc::new(ActionGutter::new(
                panes[1].view.clone().upcast::<gtk::TextView>(),
                panes[2].view.clone().upcast::<gtk::TextView>(),
                GutterDirection::LeftToRight,
            ));
            gutters.push(Rc::clone(&ag2));

            let lm1 = Rc::new(LinkMap::new(
                &[],
                panes[1].buffer.line_count().max(0) as usize,
                panes[2].buffer.line_count().max(0) as usize,
            ));
            lm1.associate(&panes[1].view, &panes[2].view);
            link_maps.push(Rc::clone(&lm1));

            let ag3 = Rc::new(ActionGutter::new(
                panes[2].view.clone().upcast::<gtk::TextView>(),
                panes[1].view.clone().upcast::<gtk::TextView>(),
                GutterDirection::RightToLeft,
            ));
            gutters.push(Rc::clone(&ag3));
        }

        // ── Assemble the horizontal layout ──
        // Layout: [pane0_vbox] [gutter] [linkmap] [gutter] [pane1_vbox] [gutter] [linkmap] [gutter] [pane2_vbox]
        // We use a GtkBox for each pane and insert gutters/linkmaps between.

        let pane_widgets: Vec<gtk::Widget> = panes
            .iter()
            .map(|p| {
                let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);

                // Action bar
                let action_bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
                action_bar.add_css_class("toolbar");
                action_bar.add_css_class("meld-actionbar");
                action_bar.append(&p.save_button);
                action_bar.append(&p.file_label);
                vbox.append(&action_bar);

                // MsgArea
                vbox.append(p.msgarea.widget());

                // Scrolled window
                vbox.append(&p.scrolled);

                // Status bar
                vbox.append(p.statusbar.widget());

                vbox.upcast::<gtk::Widget>()
            })
            .collect();

        // Build the horizontal assembly
        grid.append(&pane_widgets[0]);

        if num_panes >= 2 {
            // Gutter 0 (0→1)
            grid.append(gutters[0].widget());
            // Link map 0
            grid.append(link_maps[0].widget());
            // Gutter 1 (1→0)
            grid.append(gutters[1].widget());
            // Pane 1
            grid.append(&pane_widgets[1]);
        }

        if num_panes >= 3 {
            // Gutter 2 (1→2)
            grid.append(gutters[2].widget());
            // Link map 1
            grid.append(link_maps[1].widget());
            // Gutter 3 (2→1)
            grid.append(gutters[3].widget());
            // Pane 2
            grid.append(&pane_widgets[2]);
        }

        container.append(&grid);

        let loading = Rc::new(Cell::new(false));
        let current_chunk_idx = Rc::new(Cell::new(None));
        let focused_pane = Rc::new(Cell::new(0usize));
        let line_cache = Rc::new(RefCell::new(LineCache::new(&[], 1)));
        let inline_cache = Rc::new(InlineDiffCache::new());
        let similarity_map = Rc::new(RefCell::new(SimilarityMap::default()));
        let move_map = Rc::new(RefCell::new(MoveMap::default()));
        let token_relations = Rc::new(RefCell::new(Vec::new()));

        let fd = Self {
            container,
            panes,
            num_panes,
            chunks: Rc::new(RefCell::new(Vec::new())),
            merge_output: Rc::new(RefCell::new(None)),
            labels: Rc::new(RefCell::new(labels)),
            gutters,
            link_maps,
            shared_msgarea,
            loading: Rc::clone(&loading),
            current_chunk_idx,
            focused_pane,
            line_cache,
            inline_cache,
            similarity_map,
            move_map,
            token_relations,
        };

        // Wire up everything
        fd.sync_scroll();
        fd.connect_save_buttons();
        fd.connect_buffer_signals(loading);
        fd.connect_gutter_signals();
        fd.connect_focus_tracking();
        fd.connect_cursor_tracking();
        fd.connect_hover_sync();
        fd.compute_diff();

        fd
    }

    // ── Pane column builder ──────────────────────────────────────

    fn build_pane_column(index: usize, _num_panes: usize) -> PaneData {
        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_hexpand(true);

        let buffer = gsv::Buffer::new(None::<&gtk::TextTagTable>);

        // Apply the default style scheme so that syntax highlighting and
        // theme-aware colours work.  Python Meld uses "classic" as its
        // base, falling back to the system scheme on unavailability.
        let manager = gsv::StyleSchemeManager::new();
        let scheme = manager
            .scheme("classic")
            .or_else(|| manager.scheme("Adwaita"))
            .or_else(|| manager.scheme("Adwaita-dark"));
        if let Some(ref s) = scheme {
            buffer.set_style_scheme(Some(s));
        }

        let view = gsv::View::with_buffer(&buffer);
        view.set_monospace(true);
        view.set_show_line_numbers(true);
        view.set_editable(true);
        view.set_wrap_mode(gtk::WrapMode::None);
        view.set_vexpand(true);
        view.set_hexpand(true);
        view.set_pixels_below_lines(2);
        view.set_pixels_above_lines(2);

        // Set font size via CSS — matching original Meld's 12px default
        view.add_css_class("diff-view-12px");

        scrolled.set_child(Some(&view));

        let msgarea = Rc::new(MsgArea::new());
        let statusbar = Rc::new(StatusBar::new());

        let save_btn = gtk::Button::from_icon_name("document-save-symbolic");
        save_btn.set_tooltip_text(Some(&format!("Save file in pane {}", index + 1)));
        save_btn.set_focus_on_click(false);

        let file_label = gtk::Label::new(Some(&format!("File {}", index + 1)));
        file_label.set_ellipsize(pango::EllipsizeMode::Middle);
        file_label.set_halign(gtk::Align::Center);
        file_label.set_hexpand(true);

        PaneData {
            scrolled,
            view,
            buffer,
            msgarea,
            statusbar,
            save_button: save_btn,
            file_label,
        }
    }

    // ── Public API ───────────────────────────────────────────────

    /// Load files from disk into the panes.
    pub fn set_files(&self, gfiles: &[gio::File]) {
        self.loading.set(true);
        for (i, gfile) in gfiles.iter().enumerate().take(self.num_panes) {
            if let Some(path) = gfile.path() {
                let path_str = path.to_string_lossy().into_owned();
                self.load_file(i, &path_str);
                if let Some(name) = path.file_name() {
                    let name_str = name.to_string_lossy().into_owned();
                    self.labels.borrow_mut()[i] = name_str.clone();
                    self.panes[i].file_label.set_text(&name_str);
                }
            }
        }
        self.loading.set(false);
        self.compute_diff();
    }

    /// Set the output file path for merge operations.
    pub fn set_merge_output_file(&self, path: &str) {
        self.merge_output.replace(Some(path.to_owned()));
    }

    /// Set display labels for each pane.
    pub fn set_labels(&self, labels: &[String]) {
        let mut lbls = self.labels.borrow_mut();
        for (i, label) in labels.iter().enumerate() {
            if i < lbls.len() {
                lbls[i] = label.clone();
                self.panes[i].file_label.set_text(label);
            }
        }
    }

    /// Apply the configured font to all panes.
    ///
    /// When `use_system_font` is true the monospace font is read from the
    /// system theme.  Otherwise the `custom_font` string (e.g. "Consolas 11")
    /// is parsed and applied via `override_font`.
    pub fn set_font(&self, use_system: bool, custom: &str) {
        let desc = if use_system {
            None
        } else if !custom.is_empty() {
            Some(pango::FontDescription::from_string(custom))
        } else {
            None
        };
        if let Some(ref d) = desc {
            // GTK4 removed `override_font`; apply via CSS provider instead.
            let provider = gtk::CssProvider::new();
            let font_css = format!("textview {{ font: {}; }}", d.to_string());
            provider.load_from_data(&font_css);
            for pane in &self.panes {
                pane.view
                    .style_context()
                    .add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
            }
        }
    }

    /// Enable or disable blank-line ignoring during diff computation.
    pub fn set_ignore_blanks(&self, ignore: bool) {
        // Store the flag for use by compute_diff (and buffer-change handler)
        // This is a minimal implementation; a full solution would pass the
        // flag into the Differ engine itself.
        if ignore {
            log::info!("ignore_blank_lines enabled (not yet wired to engine)");
        }
    }

    /// (Re)compute the diff between panes 0 and 1 and update
    /// highlights, gutters, and link maps.
    pub fn compute_diff(&self) {
        if self.num_panes < 2 {
            return;
        }

        let text_a = buffer_text_lines(&self.panes[0].buffer);
        let text_b = buffer_text_lines(&self.panes[1].buffer);

        log::debug!("compute_diff: text_a={:?}, text_b={:?}", text_a, text_b);

        let is_empty = text_a.is_empty() && text_b.is_empty();

        // Compute line counts before text is moved into Differ
        let max_lines = text_a.len().max(text_b.len());

        let differ = Differ::new(text_a.clone(), text_b.clone());
        let result = differ.compare();
        let merged = merge_adjacent_replace_chunks(&result.chunks);

        // ── Cross-line similarity matching ──────────────────────────
        // Build the set of line indices that are already matched by the
        // line-level diff (Equal chunks). These are NOT candidates for
        // cross-line similarity matching.
        let mut matched_left = std::collections::HashSet::new();
        let mut matched_right = std::collections::HashSet::new();
        for chunk in &merged {
            if chunk.op == DiffOp::Equal {
                for i in chunk.start_a..chunk.end_a {
                    matched_left.insert(i);
                }
                for i in chunk.start_b..chunk.end_b {
                    matched_right.insert(i);
                }
            }
        }

        // Detect semantically similar lines at different positions
        // (e.g., same function call with extra parameters).
        let similarity = SimilarityMap::build(
            &text_a,
            &text_b,
            &matched_left,
            &matched_right,
            0.25, // Threshold: allow partial matches (e.g., extra params)
            50,   // Search window: ±50 lines around expected position
        );

        // Detect moved code blocks (e.g., reordered imports).
        let movement = MoveMap::build(
            &text_a,
            &text_b,
            &matched_left,
            &matched_right,
            0.6, // Higher threshold for movement: blocks should be nearly identical
            2,   // Minimum 2 consecutive unmatched lines to form a block
        );

        // Apply tags to each pane
        self.apply_diff_tags(0, &merged);
        self.apply_diff_tags(1, &merged);

        // Apply inline diff tags from cross-line similarity matches
        self.apply_similarity_inline_tags(&similarity);

        // Update gutters
        for gutter in &self.gutters {
            gutter.set_chunks(&merged);
        }

        // Build token-level moved-identifier relations for visual connectors.
        // Must happen BEFORE the link-map update loop so every link map
        // receives the freshly computed relations, not stale data from the
        // previous diff run.
        *self.token_relations.borrow_mut() = build_token_relations(&text_a, &text_b);

        // Update link maps with chunks, similarity, movement, and token relations
        for lm in &self.link_maps {
            lm.update_chunks(&merged);
            lm.update_similarity(&similarity);
            lm.update_moves(&movement);
            lm.update_token_relations(&self.token_relations.borrow());
        }

        // Store for later use (e.g., tooltips, hover sync)
        *self.similarity_map.borrow_mut() = similarity;
        *self.move_map.borrow_mut() = movement;

        // Rebuild line cache for O(1) navigation
        *self.line_cache.borrow_mut() = LineCache::new(&merged, max_lines);

        *self.chunks.borrow_mut() = merged;

        // Show appropriate message
        if is_empty {
            self.shared_msgarea.show_info("Enter text to compare files");
        } else if self.chunks.borrow().iter().all(|c| c.op == DiffOp::Equal) {
            self.shared_msgarea.show_info("Files are identical");
        } else {
            self.shared_msgarea.hide();
        }
    }

    /// Push the chunk at the given index from source to target pane.
    /// `push_left` determines direction: true = leftward, false = rightward.
    pub fn push_chunk(&self, chunk_idx: usize, push_left: bool) {
        let chunks = self.chunks.borrow();
        if chunk_idx >= chunks.len() {
            return;
        }
        let chunk = chunks[chunk_idx].clone();
        drop(chunks);

        let (src, dst) = if push_left { (1, 0) } else { (0, 1) };

        if matches!(chunk.op, DiffOp::Delete | DiffOp::Replace) {
            self.replace_chunk(src, dst, &chunk);
        } else if chunk.op == DiffOp::Insert && push_left {
            // "Push left" an insert = delete from right pane
            self.delete_chunk(1, &chunk);
        }
    }

    /// Merge all non-conflicting changes from one side.
    /// `push_left`: true = merge all from right to left.
    pub fn merge_all_non_conflicting(&self, push_left: bool) {
        let chunks = self.chunks.borrow().clone();
        let (src, dst) = if push_left { (1, 0) } else { (0, 1) };

        for (i, chunk) in chunks.iter().enumerate() {
            match chunk.op {
                DiffOp::Replace | DiffOp::Delete => {
                    self.replace_chunk(src, dst, chunk);
                }
                DiffOp::Insert if push_left => {
                    self.delete_chunk(1, chunk);
                }
                _ => {}
            }
        }
    }

    /// Delete the chunk at the given index from the specified pane.
    pub fn delete_chunk(&self, pane: usize, chunk: &Chunk) {
        if pane >= self.num_panes {
            return;
        }

        self.loading.set(true);
        let buffer = &self.panes[pane].buffer;

        buffer.begin_user_action();

        let start_iter = buffer.iter_at_line_offset(chunk.start_a.max(0) as i32, 0);
        let end_iter = if chunk.end_a > chunk.start_a {
            buffer.iter_at_line_offset(chunk.end_a as i32, 0)
        } else {
            // Zero-width chunk: delete at position
            buffer.iter_at_line_offset(chunk.start_a as i32, 0)
        };

        if let (Some(start), Some(end)) = (start_iter, end_iter) {
            if start.offset() < end.offset() {
                buffer.delete(&mut start.clone(), &mut end.clone());
            }
        }

        buffer.end_user_action();
        self.loading.set(false);
    }

    /// Replace the target pane's chunk content with the source pane's content.
    pub fn replace_chunk(&self, src: usize, dst: usize, chunk: &Chunk) {
        if src >= self.num_panes || dst >= self.num_panes {
            return;
        }

        self.loading.set(true);

        let src_buffer = &self.panes[src].buffer;
        let dst_buffer = &self.panes[dst].buffer;

        // Get source text
        let src_start = src_buffer.iter_at_line_offset(chunk.start_a as i32, 0);
        let src_end = src_buffer.iter_at_line_offset(chunk.end_a as i32, 0);

        let src_text = if let (Some(s), Some(e)) = (src_start, src_end) {
            if s.offset() < e.offset() {
                src_buffer.text(&s, &e, true).to_string()
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // Replace in destination
        dst_buffer.begin_user_action();

        let dst_start = dst_buffer.iter_at_line_offset(chunk.start_b as i32, 0);
        let dst_end = dst_buffer.iter_at_line_offset(chunk.end_b as i32, 0);

        if let (Some(ds), Some(de)) = (dst_start, dst_end) {
            if ds.offset() < de.offset() {
                dst_buffer.delete(&mut ds.clone(), &mut de.clone());
            }
            // Insert at correct position
            let insert_pos = dst_buffer.iter_at_line_offset(chunk.start_b as i32, 0);
            if let Some(pos) = insert_pos {
                dst_buffer.insert(&mut pos.clone(), &src_text);
            }
        }

        dst_buffer.end_user_action();
        self.loading.set(false);
    }

    /// Copy chunk content from source pane to destination pane.
    /// `copy_up`: if true, copy above the destination chunk; if false, copy below.
    pub fn copy_chunk(&self, src: usize, dst: usize, chunk: &Chunk, copy_up: bool) {
        if src >= self.num_panes || dst >= self.num_panes {
            return;
        }

        self.loading.set(true);

        let src_buffer = &self.panes[src].buffer;
        let dst_buffer = &self.panes[dst].buffer;

        // Get source text
        let src_start = src_buffer.iter_at_line_offset(chunk.start_a as i32, 0);
        let src_end = src_buffer.iter_at_line_offset(chunk.end_a as i32, 0);

        let mut src_text = if let (Some(s), Some(e)) = (src_start, src_end) {
            if s.offset() < e.offset() {
                src_buffer.text(&s, &e, true).to_string()
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        dst_buffer.begin_user_action();

        if copy_up {
            // Insert before the destination chunk
            if chunk.end_a >= src_buffer.line_count().max(0) as usize
                && chunk.start_b < dst_buffer.line_count().max(0) as usize
            {
                src_text.push('\n');
            }
            let insert_pos = dst_buffer.iter_at_line_offset(chunk.start_b as i32, 0);
            if let Some(mut pos) = insert_pos {
                dst_buffer.insert(&mut pos, &src_text);
            }
        } else {
            // Insert after the destination chunk
            let insert_pos = dst_buffer.iter_at_line_offset(chunk.end_b as i32, 0);
            if let Some(mut pos) = insert_pos {
                dst_buffer.insert(&mut pos, &src_text);
            }
        }

        dst_buffer.end_user_action();
        self.loading.set(false);
    }

    /// Navigate to the next or previous diff chunk (direction: +1 or -1).
    pub fn go_to_diff(&self, direction: i32) {
        let chunks = self.chunks.borrow();
        if chunks.is_empty() {
            return;
        }

        // Find the current chunk based on cursor position in focused pane
        let fp = self.focused_pane.get().min(self.num_panes - 1);
        let buffer = &self.panes[fp].buffer;
        let cursor_line = {
            let cursor = buffer.cursor_position();
            let iter = buffer.iter_at_offset(cursor as i32);
            iter.line().max(0) as usize
        };

        // Use O(1) line cache lookup instead of linear scan
        let line_cache = self.line_cache.borrow();
        let current_idx = if let Some(ci) = line_cache.locate_chunk(cursor_line) {
            ci as i32
        } else {
            // Fallback: find nearest chunk via linear scan
            let mut idx = 0i32;
            for (i, chunk) in chunks.iter().enumerate() {
                let line = if fp == 0 {
                    chunk.start_a
                } else {
                    chunk.start_b
                };
                if (line as i32) <= cursor_line as i32 {
                    idx = i as i32;
                } else {
                    break;
                }
            }
            idx
        };
        drop(line_cache);

        let new_idx = if direction > 0 {
            // Next non-equal chunk
            let mut idx = current_idx + 1;
            while (idx as usize) < chunks.len() {
                if chunks[idx as usize].op != DiffOp::Equal {
                    break;
                }
                idx += 1;
            }
            if (idx as usize) >= chunks.len() {
                // Wrap around
                idx = 0;
                while (idx as usize) < chunks.len() {
                    if chunks[idx as usize].op != DiffOp::Equal {
                        break;
                    }
                    idx += 1;
                }
            }
            idx.min(chunks.len() as i32 - 1)
        } else {
            // Previous non-equal chunk
            let mut idx = current_idx - 1;
            while idx >= 0 {
                if chunks[idx as usize].op != DiffOp::Equal {
                    break;
                }
                idx -= 1;
            }
            if idx < 0 {
                // Wrap around
                idx = chunks.len() as i32 - 1;
                while idx >= 0 {
                    if chunks[idx as usize].op != DiffOp::Equal {
                        break;
                    }
                    idx -= 1;
                }
            }
            idx.max(0)
        };

        if (new_idx as usize) < chunks.len() {
            let chunk = &chunks[new_idx as usize];
            let target_line = if fp == 0 {
                chunk.start_a
            } else {
                chunk.start_b
            };
            self.scroll_to_line(fp, target_line);
            self.current_chunk_idx.set(Some(new_idx as usize));
        }
    }

    /// Navigate to next/previous conflict (for merge views).
    pub fn go_to_conflict(&self, _direction: i32) {
        if self.num_panes < 3 {
            return;
        }
        // For 3-way merge, conflicts are marked as "replace" chunks
        // where both sides have changed.
        // This would require conflict detection from the 3-way merge engine.
        // For now, delegate to go_to_diff.
        self.go_to_diff(_direction);
    }

    /// Scroll the given pane to the specified line.
    fn scroll_to_line(&self, pane: usize, line: usize) {
        if pane >= self.num_panes {
            return;
        }
        let buffer = &self.panes[pane].buffer;
        if let Some(iter) = buffer.iter_at_line_offset(line as i32, 0) {
            buffer.place_cursor(&iter);
            let mark = buffer.create_mark(Some("scroll_target"), &iter, true);
            self.panes[pane]
                .view
                .scroll_to_mark(&mark, 0.2, true, 0.0, 0.5);
        }
    }

    // ── Private helpers ───────────────────────────────────────────

    fn load_file(&self, pane_idx: usize, path: &str) {
        if pane_idx >= self.panes.len() {
            return;
        }
        match std::fs::read_to_string(path) {
            Ok(content) => self.panes[pane_idx].buffer.set_text(&content),
            Err(e) => {
                self.panes[pane_idx]
                    .msgarea
                    .show_error(&format!("Error loading file: {e}"));
            }
        }
    }

    fn apply_diff_tags(&self, pane: usize, chunks: &[Chunk]) {
        if pane >= self.panes.len() {
            return;
        }
        let buffer = &self.panes[pane].buffer;
        let tag_table = buffer.tag_table();

        // Clear existing diff tags
        clear_diff_tags_single(buffer, &tag_table);

        // Ensure tags exist
        ensure_diff_tags(&tag_table);

        // Get the other pane's buffer for inline diff and import checks
        let other_pane = if pane == 0 { 1 } else { 0 };
        let other_buffer = if other_pane < self.panes.len() {
            Some(&self.panes[other_pane].buffer)
        } else {
            None
        };
        let other_import_sets = other_buffer
            .map(|buf| build_import_sets(buf))
            .unwrap_or_default();

        // Apply tags for this pane
        for chunk in chunks {
            let (start, end, tag_name) = match (&chunk.op, pane) {
                (DiffOp::Delete, 0) => (chunk.start_a, chunk.end_a, "diff-delete"),
                (DiffOp::Insert, 1) => (chunk.start_b, chunk.end_b, "diff-insert"),
                (DiffOp::Replace, 0) => (chunk.start_a, chunk.end_a, "diff-replace"),
                (DiffOp::Replace, 1) => (chunk.start_b, chunk.end_b, "diff-replace"),
                _ => continue,
            };

            log::debug!(
                "apply_tags: pane={}, op={:?}, tag={}, start={}, end={}",
                pane,
                chunk.op,
                tag_name,
                start,
                end
            );

            if start < end {
                // Apply line-level tag per-line.  For Replace chunks on import
                // lines we apply BOTH the line-level background (blue for
                // Replace) AND per-token inline tags (red/green for individual
                // identifiers) — matching Meld's original behaviour.  The
                // inline tags are applied after the loop by apply_inline_diff()
                // and their per-character backgrounds take visual precedence
                // over the line-level background for the changed identifiers.
                for line_num in start..end {
                    let ls = buffer.iter_at_line_offset(line_num as i32, 0);
                    let le = buffer.iter_at_line_offset((line_num + 1) as i32, 0);
                    if let (Some(ls), Some(le)) = (ls, le) {
                        if let Some(tag) = tag_table.lookup(tag_name) {
                            buffer.apply_tag(&tag, &ls, &le);
                        }
                    }
                }

                // For Replace chunks, also apply inline diff
                if chunk.op == DiffOp::Replace {
                    if let Some(other_buf) = other_buffer {
                        apply_inline_diff(
                            buffer,
                            other_buf,
                            &tag_table,
                            &chunk,
                            pane,
                            &self.inline_cache,
                        );
                    }
                }
            }
        }
    }

    /// Apply inline diff tags for cross-line similarity matches.
    ///
    /// When [`SimilarityMap`] detects semantically similar lines at different
    /// positions (e.g., the same function call with extra parameters), this
    /// method applies token-level inline diff tags to both sides. This lets
    /// users see *what changed* within matched lines even when they are not
    /// aligned by the line-level diff.
    fn apply_similarity_inline_tags(&self, similarity: &SimilarityMap) {
        if similarity.matches.is_empty() {
            return;
        }

        for entry in &similarity.matches {
            // Apply inline diff on the left (A) pane
            self.apply_single_similarity_inline(0, entry);
            // Apply inline diff on the right (B) pane
            self.apply_single_similarity_inline(1, entry);
        }
    }

    /// Apply the inline diff from a single similarity entry to one pane.
    fn apply_single_similarity_inline(&self, pane: usize, entry: &SimilarityEntry) {
        if pane >= self.panes.len() {
            return;
        }

        let buffer = &self.panes[pane].buffer;
        let tag_table = buffer.tag_table();

        // Get the line number in this pane
        let base_line = if pane == 0 {
            entry.left_line
        } else {
            entry.right_line
        };

        let base_iter = match buffer.iter_at_line_offset(base_line as i32, 0) {
            Some(iter) => iter,
            None => return,
        };

        // Use precomputed inline diff from the similarity entry
        let changes = &entry.inline_diff;
        if changes.is_empty() {
            return;
        }

        // Ensure inline tags exist
        ensure_diff_tags(&tag_table);

        // Apply differentiated inline tags at the correct character offsets.
        // Filter by pane: Delete offsets refer to left line, Insert to right.
        for change in changes.iter() {
            let apply = match (pane, change.op) {
                (0, DiffOp::Delete | DiffOp::Replace) => true,
                (0, DiffOp::Insert) => false,
                (1, DiffOp::Insert | DiffOp::Replace) => true,
                (1, DiffOp::Delete) => false,
                _ => false,
            };
            if !apply {
                continue;
            }
            let tag_name = match change.op {
                DiffOp::Delete => "diff-inline-delete",
                DiffOp::Insert => "diff-inline-insert",
                DiffOp::Replace => "diff-inline-replace",
                DiffOp::Equal => continue,
            };

            if let Some(tag) = tag_table.lookup(tag_name) {
                let start_offset = base_iter.offset() as usize + change.start;
                let end_offset = base_iter.offset() as usize + change.end;
                let mut s = buffer.iter_at_offset(start_offset as i32);
                let mut e = buffer.iter_at_offset(end_offset as i32);
                // Adjust iterators to valid cursor positions so that
                // combining characters (Unicode diacritics) are not split
                // by the tag boundary.
                if !s.is_cursor_position() {
                    s.backward_cursor_position();
                }
                if !e.is_cursor_position() {
                    e.forward_cursor_position();
                }
                if s.offset() < e.offset() {
                    buffer.apply_tag(&tag, &s, &e);
                }
            }
        }
    }

    fn map_scroll_proportionally(
        master_view: &gsv::View,
        slave_view: &gsv::View,
        chunks: &[Chunk],
        master_value: f64,
        master_max: f64,
        from_pane: usize,
        to_pane: usize,
    ) -> f64 {
        if master_max <= 0.0 {
            return 0.0;
        }

        let master_line_count = (master_view.buffer().line_count().max(1) - 1) as f64;
        let slave_line_count = (slave_view.buffer().line_count().max(1) - 1) as f64;
        let master_scroll_frac = (master_value / master_max.max(1.0)).clamp(0.0, 1.0);
        let master_line = master_scroll_frac * master_line_count;
        let mut slave_line = master_scroll_frac * slave_line_count;

        for chunk in chunks {
            let m_start = if from_pane == 0 {
                chunk.start_a as f64
            } else {
                chunk.start_b as f64
            };
            let m_end = if from_pane == 0 {
                chunk.end_a as f64
            } else {
                chunk.end_b as f64
            };
            let m_range = m_end - m_start;
            if m_range <= 0.0 {
                continue;
            }

            if master_line >= m_start && master_line < m_end {
                let chunk_frac = (master_line - m_start) / m_range;

                let s_start = if to_pane == 0 {
                    chunk.start_a as f64
                } else {
                    chunk.start_b as f64
                };
                let s_end = if to_pane == 0 {
                    chunk.end_a as f64
                } else {
                    chunk.end_b as f64
                };
                let s_range = s_end - s_start;

                if s_range > 0.0 {
                    slave_line = s_start + chunk_frac * s_range;
                }
                break;
            }
        }
        let slave_scroll_max = slave_view
            .vadjustment()
            .map(|a| a.upper() - a.page_size())
            .unwrap_or(1.0)
            .max(1.0);
        let slave_scroll_frac = (slave_line / slave_line_count.max(1.0)).clamp(0.0, 1.0);
        slave_scroll_frac * slave_scroll_max
    }

    fn sync_scroll(&self) {
        if self.panes.len() < 2 {
            return;
        }

        let scroll_lock: Rc<Cell<bool>> = Rc::new(Cell::new(false));

        // ── Pane 0 ↔ Pane 1 bidirectional sync with proportional chunk mapping ──
        let adj0 = self.panes[0].view.vadjustment();
        let adj1 = self.panes[1].view.vadjustment();

        if let (Some(a0), Some(a1)) = (adj0, adj1) {
            let view0 = self.panes[0].view.clone();
            let view1 = self.panes[1].view.clone();
            let chunks = Rc::clone(&self.chunks);
            let lock = Rc::clone(&scroll_lock);

            // Pane 0 scroll → sync Pane 1 (master=0, slave=1)
            let a1_weak = a1.downgrade();
            let lock_0_to_1 = Rc::clone(&lock);
            let chunks_01 = Rc::clone(&chunks);
            let view0_01 = view0.clone();
            let view1_01 = view1.clone();
            a0.connect_value_changed(move |master_adj| {
                if lock_0_to_1.get() {
                    return;
                }
                if let Some(slave_adj) = a1_weak.upgrade() {
                    let new_val = FileDiff::map_scroll_proportionally(
                        &view0_01,
                        &view1_01,
                        &chunks_01.borrow(),
                        master_adj.value(),
                        master_adj.upper() - master_adj.page_size(),
                        0,
                        1,
                    );
                    if (slave_adj.value() - new_val).abs() > 0.5 {
                        lock_0_to_1.set(true);
                        slave_adj.set_value(new_val);
                        lock_0_to_1.set(false);
                    }
                }
            });

            // Pane 1 scroll → sync Pane 0 (master=1, slave=0)
            let a0_weak = a0.downgrade();
            let lock_1_to_0 = Rc::clone(&lock);
            let chunks_10 = Rc::clone(&chunks);
            let view0_10 = view0.clone();
            let view1_10 = view1.clone();
            a1.connect_value_changed(move |master_adj| {
                if lock_1_to_0.get() {
                    return;
                }
                if let Some(slave_adj) = a0_weak.upgrade() {
                    let new_val = FileDiff::map_scroll_proportionally(
                        &view1_10,
                        &view0_10,
                        &chunks_10.borrow(),
                        master_adj.value(),
                        master_adj.upper() - master_adj.page_size(),
                        1,
                        0,
                    );
                    if (slave_adj.value() - new_val).abs() > 0.5 {
                        lock_1_to_0.set(true);
                        slave_adj.set_value(new_val);
                        lock_1_to_0.set(false);
                    }
                }
            });
        }

        // ── Pane 1 ↔ Pane 2 bidirectional sync (3-pane mode) ──
        if self.panes.len() >= 3 {
            let adj1b = self.panes[1].view.vadjustment();
            let adj2 = self.panes[2].view.vadjustment();
            if let (Some(a1b), Some(a2)) = (adj1b, adj2) {
                let a1b_weak = a1b.downgrade();
                let lock3 = Rc::clone(&scroll_lock);
                a2.connect_value_changed(move |adj| {
                    if lock3.get() {
                        return;
                    }
                    if let Some(target) = a1b_weak.upgrade() {
                        let v = adj.value();
                        if (target.value() - v).abs() > 0.5 {
                            lock3.set(true);
                            target.set_value(v);
                            lock3.set(false);
                        }
                    }
                });
                let a2_weak = a2.downgrade();
                let lock4 = Rc::clone(&scroll_lock);
                a1b.connect_value_changed(move |adj| {
                    if lock4.get() {
                        return;
                    }
                    if let Some(target) = a2_weak.upgrade() {
                        let v = adj.value();
                        if (target.value() - v).abs() > 0.5 {
                            lock4.set(true);
                            target.set_value(v);
                            lock4.set(false);
                        }
                    }
                });
            }
        }
    }

    fn connect_save_buttons(&self) {
        for pane in &self.panes {
            let buffer = pane.buffer.clone();
            let msgarea = Rc::clone(&pane.msgarea);
            pane.save_button.connect_clicked(move |_| {
                let text = buffer_text_lines(&buffer).join("\n");
                log::info!("Save requested ({} bytes)", text.len());
                msgarea.show_info("Save functionality: use Ctrl+S or menu");
            });
        }
    }

    fn connect_buffer_signals(&self, loading: Rc<Cell<bool>>) {
        let chunks = Rc::clone(&self.chunks);
        let gutters = self.gutters.clone();
        let link_maps = self.link_maps.clone();
        let shared_msgarea = Rc::clone(&self.shared_msgarea);
        let inline_cache = Rc::clone(&self.inline_cache);
        let similarity_map = Rc::clone(&self.similarity_map);
        let move_map = Rc::clone(&self.move_map);

        let buffers: Vec<gsv::Buffer> = self.panes.iter().map(|p| p.buffer.clone()).collect();
        let tag_tables: Vec<gtk::TextTagTable> =
            self.panes.iter().map(|p| p.buffer.tag_table()).collect();

        for (pi, pane) in self.panes.iter().enumerate() {
            let buffers = buffers.clone();
            let tag_tables = tag_tables.clone();
            let chunks = Rc::clone(&chunks);
            let gutters = gutters.clone();
            let link_maps = link_maps.clone();
            let loading = Rc::clone(&loading);
            let shared_msgarea = Rc::clone(&shared_msgarea);
            let inline_cache = Rc::clone(&inline_cache);
            let similarity_map = Rc::clone(&similarity_map);
            let move_map = Rc::clone(&move_map);

            pane.buffer.connect_changed(move |_| {
                if loading.get() || buffers.len() < 2 {
                    return;
                }

                let text_a = buffer_text_lines(&buffers[0]);
                let text_b = buffer_text_lines(&buffers[1]);

                log::debug!("changed: text_a={:?}, text_b={:?}", text_a, text_b);

                let is_empty = text_a.is_empty() && text_b.is_empty();
                let differ = Differ::new(text_a.clone(), text_b.clone());
                let result = differ.compare();
                let merged = merge_adjacent_replace_chunks(&result.chunks);

                // Clear and re-apply tags
                for (bi, buf) in buffers.iter().enumerate() {
                    clear_diff_tags_single(buf, &tag_tables[bi]);
                    ensure_diff_tags(&tag_tables[bi]);
                }

                apply_diff_tags_to_buffer(
                    &buffers[0],
                    &tag_tables[0],
                    0,
                    &merged,
                    Some(&buffers[1]),
                    &inline_cache,
                );
                apply_diff_tags_to_buffer(
                    &buffers[1],
                    &tag_tables[1],
                    1,
                    &merged,
                    Some(&buffers[0]),
                    &inline_cache,
                );

                for gutter in &gutters {
                    gutter.set_chunks(&merged);
                }
                for lm in &link_maps {
                    lm.update_chunks(&merged);
                    lm.update_similarity(&similarity_map.borrow());
                    lm.update_moves(&move_map.borrow());
                }

                // Build cross-line similarity map for non-aligned changes
                {
                    let mut matched_left = std::collections::HashSet::new();
                    let mut matched_right = std::collections::HashSet::new();
                    for chunk in &merged {
                        if chunk.op != DiffOp::Delete {
                            for l in chunk.start_a..chunk.end_a {
                                matched_left.insert(l);
                            }
                        }
                        if chunk.op != DiffOp::Insert {
                            for l in chunk.start_b..chunk.end_b {
                                matched_right.insert(l);
                            }
                        }
                    }
                    let sim = SimilarityMap::build(
                        &text_a,
                        &text_b,
                        &matched_left,
                        &matched_right,
                        0.6,
                        50,
                    );
                    *similarity_map.borrow_mut() = sim;

                    let mov =
                        MoveMap::build(&text_a, &text_b, &matched_left, &matched_right, 0.8, 1);
                    *move_map.borrow_mut() = mov;
                }

                if is_empty {
                    shared_msgarea.show_info("Enter text to compare files");
                } else if merged.iter().all(|c| c.op == DiffOp::Equal) {
                    shared_msgarea.show_info("Files are identical");
                } else {
                    shared_msgarea.hide();
                }

                *chunks.borrow_mut() = merged;
            });
        }
    }

    /// Wire up action gutter signals to the actual chunk operations.
    fn connect_gutter_signals(&self) {
        // Collect buffer pairs and chunk data for each gutter
        for (gi, gutter) in self.gutters.iter().enumerate() {
            let chunks = Rc::clone(&self.chunks);
            let loading = Rc::clone(&self.loading);

            // Determine source/target pane indices for this gutter.
            // Gutter layout: [0, 1] for pair (0,1), [2, 3] for pair (1,2)
            let pair_idx = gi / 2;
            let is_right_to_left = gi % 2 == 1;

            let (src_pane_idx, dst_pane_idx) = if is_right_to_left {
                (pair_idx + 1, pair_idx) // e.g., gutter 1: 1→0
            } else {
                (pair_idx, pair_idx + 1) // e.g., gutter 0: 0→1
            };

            // Clone the buffers for use in the closure
            let src_buffer = self.panes[src_pane_idx].buffer.clone();
            let dst_buffer = self.panes[dst_pane_idx].buffer.clone();

            gutter.connect_action(move |chunk_idx, action| {
                let chunks = chunks.borrow();
                if chunk_idx >= chunks.len() {
                    return;
                }
                let chunk = chunks[chunk_idx].clone();
                drop(chunks);

                // Perform the chunk operation directly on the buffers
                match action {
                    GutterAction::Replace => {
                        // Push from source to target
                        execute_replace(&src_buffer, &dst_buffer, &chunk);
                    }
                    GutterAction::Delete => {
                        // Delete from source
                        execute_delete(&src_buffer, &chunk);
                    }
                    GutterAction::CopyUp => {
                        execute_copy(&src_buffer, &dst_buffer, &chunk, true);
                    }
                    GutterAction::CopyDown => {
                        execute_copy(&src_buffer, &dst_buffer, &chunk, false);
                    }
                }

                log::info!(
                    "Gutter action: idx={}, action={:?}, src={}, dst={}",
                    chunk_idx,
                    action,
                    src_pane_idx,
                    dst_pane_idx
                );
            });
        }
    }

    /// Track which pane has focus for action targeting.
    fn connect_focus_tracking(&self) {
        let focused = Rc::clone(&self.focused_pane);
        for (pi, pane) in self.panes.iter().enumerate() {
            let fp = Rc::clone(&focused);
            pane.view.connect_has_focus_notify(move |view| {
                if view.has_focus() {
                    fp.set(pi);
                }
            });
        }
    }

    /// Hover sync: when the cursor hovers over a line in one pane, briefly
    /// highlight the corresponding line in the other pane.
    ///
    /// Uses EventControllerMotion for cross-pane cursor tracking.
    fn connect_hover_sync(&self) {
        if self.panes.len() < 2 {
            return;
        }

        let chunks = Rc::clone(&self.chunks);
        let buffers: Vec<gsv::Buffer> = self.panes.iter().map(|p| p.buffer.clone()).collect();

        for (pi, pane) in self.panes.iter().enumerate() {
            let other_pi = if pi == 0 { 1 } else { 0 };
            if other_pi >= self.panes.len() {
                continue;
            }

            let chunks = Rc::clone(&chunks);
            let other_buffer = buffers[other_pi].clone();
            let this_view_for_motion = pane.view.clone();
            let this_view = pane.view.clone();

            let motion = gtk::EventControllerMotion::new();
            motion.connect_motion(move |_controller, _x, y| {
                let truncated = y as i32;
                if truncated < 0 {
                    return;
                }

                // Approximate line from y coordinate using simple proportion
                let buffer = this_view_for_motion.buffer();
                let line_count = buffer.line_count().max(1) as f64;
                let view_height = this_view_for_motion.height() as f64;
                let line = if view_height > 0.0 {
                    (y / view_height * line_count) as usize
                } else {
                    0
                };

                let chunks = chunks.borrow();
                for chunk in chunks.iter() {
                    let pane_line = if pi == 0 {
                        chunk.start_a
                    } else {
                        chunk.start_b
                    };
                    let pane_end = if pi == 0 { chunk.end_a } else { chunk.end_b };

                    if line >= pane_line && line < pane_end && chunk.op != DiffOp::Equal {
                        let other_line = if pi == 0 {
                            chunk.start_b
                        } else {
                            chunk.start_a
                        };
                        let other_end = if pi == 0 { chunk.end_b } else { chunk.end_a };

                        if other_line < other_end {
                            let tag_name = "meld-hover-sync";
                            let tag_table = other_buffer.tag_table();
                            if tag_table.lookup(tag_name).is_none() {
                                let tag = gsv::Tag::new(Some(tag_name));
                                tag.set_background(Some("rgba(255,255,0,0.3)"));
                                tag.set_draw_spaces(true);
                                tag_table.add(&tag);
                            }
                            if let (Some(s), Some(e)) = (
                                other_buffer.iter_at_line_offset(other_line as i32, 0),
                                other_buffer.iter_at_line_offset(other_end as i32, 0),
                            ) {
                                if let Some(tag) = tag_table.lookup(tag_name) {
                                    let os = other_buffer.start_iter();
                                    let oe = other_buffer.end_iter();
                                    other_buffer.remove_tag(&tag, &os, &oe);
                                    other_buffer.apply_tag(&tag, &s, &e);
                                }
                            }
                        }
                        break;
                    }
                }
            });
            this_view.add_controller(motion);
        }
    }

    /// Update status bar with cursor position.
    fn connect_cursor_tracking(&self) {
        for pane in &self.panes {
            let buffer = pane.buffer.clone();
            let statusbar = Rc::clone(&pane.statusbar);
            buffer.connect_cursor_position_notify(move |buf| {
                let pos = buf.cursor_position() as u32;
                let iter = buf.iter_at_offset(pos as i32);
                let line = iter.line().max(0) as u32 + 1;
                let line_offset = iter.line_offset().max(0) as u32 + 1;
                statusbar.set_position(line, line_offset);
            });
        }
    }
}

// ─── MeldPage impl ──────────────────────────────────────────────────

impl MeldPage for FileDiff {
    fn widget(&self) -> &gtk::Widget {
        self.container.upcast_ref()
    }

    fn close(&self) -> gtk::ResponseType {
        if let Some(out) = self.merge_output.borrow().as_ref() {
            if self.num_panes >= 3 {
                let text = buffer_text_lines(&self.panes[self.num_panes - 1].buffer).join("\n");
                let _ = std::fs::write(out, &text);
            }
        }
        gtk::ResponseType::Ok
    }

    fn label(&self) -> String {
        self.labels.borrow().join(" vs ")
    }

    fn show_filters(&self) -> (bool, bool, bool) {
        (false, false, true)
    }

    fn show_conflict_nav(&self) -> bool {
        self.num_panes >= 3
    }

    fn go_next_diff(&self) {
        self.go_to_diff(1);
    }

    fn go_prev_diff(&self) {
        self.go_to_diff(-1);
    }

    fn go_next_conflict(&self) {
        self.go_to_conflict(1);
    }

    fn go_prev_conflict(&self) {
        self.go_to_conflict(-1);
    }
}

impl Drop for FileDiff {
    fn drop(&mut self) {
        if let Some(out) = self.merge_output.borrow().as_ref() {
            if self.num_panes >= 3 {
                let text = buffer_text_lines(&self.panes[self.num_panes - 1].buffer).join("\n");
                let _ = std::fs::write(out, &text);
            }
        }
    }
}

// ─── Tag helpers ────────────────────────────────────────────────────

/// Extract text from a GtkSourceBuffer as a Vec of line strings.
pub fn buffer_text_lines(buffer: &gsv::Buffer) -> Vec<String> {
    let start = buffer.start_iter();
    let end = buffer.end_iter();
    let text = buffer.text(&start, &end, true);
    text.as_str().lines().map(|l| l.to_owned()).collect()
}

fn diff_tag_names() -> Vec<&'static str> {
    vec![
        "diff-delete",
        "diff-insert",
        "diff-replace",
        "diff-inline",
        "diff-inline-delete",
        "diff-inline-insert",
        "diff-inline-replace",
    ]
}

fn clear_diff_tags_single(buffer: &gsv::Buffer, tag_table: &gtk::TextTagTable) {
    for name in diff_tag_names() {
        if let Some(tag) = tag_table.lookup(name) {
            let s = buffer.start_iter();
            let e = buffer.end_iter();
            buffer.remove_tag(&tag, &s, &e);
        }
    }
}

fn ensure_diff_tags(tag_table: &gtk::TextTagTable) {
    // Match the original Meld base style scheme colors exactly.
    // Insert: green background (#d0ffa3) with dark green text (#008800)
    // Replace: blue background (#bdddff) with dark blue text (#0044dd)
    // Delete: white/gray background (#cccccc) with dark red text (#880000)
    //
    // Use paragraph_background for edge-to-edge line highlighting (like
    // line-background in GtkSourceView style schemes).
    ensure_tag_full(tag_table, "diff-insert", "#d0ffa3", "#008800", "#a5ff4c");
    ensure_tag_full(tag_table, "diff-replace", "#bdddff", "#0044dd", "#65b2ff");
    ensure_tag_full(tag_table, "diff-delete", "#cccccc", "#880000", "#cccccc");
    // Inline differences within a line — single intense blue for BOTH panes,
    // matching the original Meld "meld:inline" style.
    // Use GtkSource.Tag (not plain GtkTextTag) with draw_spaces = true so
    // that whitespace changes are visible, matching Python Meld behaviour.
    if tag_table.lookup("diff-inline").is_none() {
        let tag = gsv::Tag::new(Some("diff-inline"));
        tag.set_background(Some("#8ac2ff"));
        tag.set_foreground(Some("#000000"));
        tag.set_draw_spaces(true);
        tag_table.add(&tag);
    }
    // Differentiated inline tags for delete/insert/replace at token level.
    // These use GtkSource.Tag with draw_spaces=true for whitespace visibility.
    // Colours are more intense than the line-level backgrounds so that
    // individual token changes stand out clearly.
    if tag_table.lookup("diff-inline-delete").is_none() {
        let tag = gsv::Tag::new(Some("diff-inline-delete"));
        tag.set_background(Some("#ff6666"));
        tag.set_foreground(Some("#880000"));
        tag.set_draw_spaces(true);
        tag_table.add(&tag);
    }
    if tag_table.lookup("diff-inline-insert").is_none() {
        let tag = gsv::Tag::new(Some("diff-inline-insert"));
        tag.set_background(Some("#66ff66"));
        tag.set_foreground(Some("#008800"));
        tag.set_draw_spaces(true);
        tag_table.add(&tag);
    }
    if tag_table.lookup("diff-inline-replace").is_none() {
        let tag = gsv::Tag::new(Some("diff-inline-replace"));
        tag.set_background(Some("#4488ff"));
        tag.set_foreground(Some("#000044"));
        tag.set_draw_spaces(true);
        tag_table.add(&tag);
    }
}

fn apply_diff_tags_to_buffer(
    buffer: &gsv::Buffer,
    tag_table: &gtk::TextTagTable,
    pane: usize,
    chunks: &[Chunk],
    other_buffer: Option<&gsv::Buffer>,
    inline_cache: &InlineDiffCache,
) {
    for chunk in chunks {
        let (start, end, tag_name) = match (&chunk.op, pane) {
            (DiffOp::Delete, 0) => (chunk.start_a, chunk.end_a, "diff-delete"),
            (DiffOp::Insert, 1) => (chunk.start_b, chunk.end_b, "diff-insert"),
            (DiffOp::Replace, 0) => (chunk.start_a, chunk.end_a, "diff-replace"),
            (DiffOp::Replace, 1) => (chunk.start_b, chunk.end_b, "diff-replace"),
            _ => continue,
        };
        if start < end {
            if let (Some(s), Some(e)) = (
                buffer.iter_at_line_offset(start as i32, 0),
                buffer.iter_at_line_offset(end as i32, 0),
            ) {
                if let Some(tag) = tag_table.lookup(tag_name) {
                    buffer.apply_tag(&tag, &s, &e);
                }
            }
        }

        // For Replace chunks, apply inline (word-level) diff
        if chunk.op == DiffOp::Replace {
            if let Some(other_buf) = other_buffer {
                apply_inline_diff(buffer, other_buf, tag_table, chunk, pane, inline_cache);
            }
        }
    }
}

/// Apply word-level (inline) diff highlighting within a Replace chunk.
///
/// For each line in the Replace chunk, computes character-level diff between
/// the corresponding lines in both buffers and applies differentiated inline
/// tags to highlight the specific characters that changed:
///   - `diff-inline-delete`: characters removed from the left side
///   - `diff-inline-insert`: characters added to the right side
///   - `diff-inline-replace`: contiguous delete+insert merged as a single change

fn build_import_sets(
    buffer: &gsv::Buffer,
) -> std::collections::HashMap<String, std::collections::HashSet<String>> {
    use crate::diff::engine::InlineDiffer;
    let mut map = std::collections::HashMap::new();
    let line_count = buffer.line_count().max(0) as usize;
    for i in 0..line_count {
        let start = buffer.iter_at_line_offset(i as i32, 0);
        let end = buffer.iter_at_line_offset((i + 1) as i32, 0);
        if let (Some(s), Some(e)) = (start, end) {
            let line = buffer.text(&s, &e, true).to_string();
            if let Some((module, ids)) = InlineDiffer::parse_import_line(&line) {
                let entry = map
                    .entry(module)
                    .or_insert_with(std::collections::HashSet::new);
                for (id, _) in ids {
                    entry.insert(id);
                }
            }
        }
    }
    map
}

fn build_token_relations(
    left: &[String],
    right: &[String],
) -> Vec<crate::ui::link_map::TokenRelation> {
    use crate::diff::engine::InlineDiffer;
    use crate::ui::link_map::TokenRelation;
    use std::collections::HashMap;

    let left_lines = build_import_line_sets_from_text(left);
    let right_lines = build_import_line_sets_from_text(right);

    let mut relations = Vec::new();

    // Precompute the character offset of the start of each line.
    let left_line_offsets = compute_line_char_offsets(left);
    let right_line_offsets = compute_line_char_offsets(right);

    // ── Build a 1-to-1 lookup map for right-side tokens ──
    // Maps identifier name → (line, col_start, col_end).
    // Each right-side token can be matched at most once.
    let mut right_token_map: HashMap<String, (usize, usize, usize)> = HashMap::new();
    for (r_line, r_ids) in &right_lines {
        for id in r_ids {
            let (cs, ce) = find_identifier_col(right, *r_line, id);
            right_token_map.insert(id.clone(), (*r_line, cs, ce));
        }
    }

    // Convert byte positions to character positions.
    let byte_to_char_col =
        |line: &str, byte_pos: usize| -> usize { line[..byte_pos.min(line.len())].chars().count() };

    // ── Match each left token to exactly ONE right token ──
    for (l_line, l_ids) in &left_lines {
        for id in l_ids {
            // Only match if the identifier exists on the right AND
            // hasn't already been consumed by a previous match.
            let Some(&(r_line, r_cs, r_ce)) = right_token_map.get(id) else {
                continue;
            };

            // Check whether this identifier *moved* (changed grouping
            // between left and right), mirroring the original Meld logic.
            let is_alone_left = l_ids.len() == 1;
            let right_ids_for_line = right_lines.get(&r_line);
            let is_alone_right = right_ids_for_line.map(|s| s.len() == 1).unwrap_or(true);
            let moved = (!is_alone_left && is_alone_right) || (is_alone_left && !is_alone_right);

            if !moved {
                continue;
            }

            // Consume the right token so it cannot be matched again.
            right_token_map.remove(id);

            // Get column positions for the left token.
            let (l_cs, l_ce) = find_identifier_col(left, *l_line, id);

            // Compute buffer character offsets.
            let left_line_base = left_line_offsets.get(*l_line).copied().unwrap_or(0);
            let right_line_base = right_line_offsets.get(r_line).copied().unwrap_or(0);
            let l_start = left_line_base + byte_to_char_col(&left[*l_line], l_cs);
            let l_end = left_line_base + byte_to_char_col(&left[*l_line], l_ce);
            let r_start = right_line_base + byte_to_char_col(&right[r_line], r_cs);
            let r_end = right_line_base + byte_to_char_col(&right[r_line], r_ce);

            relations.push(TokenRelation {
                left_line: *l_line,
                left_col_start: l_cs,
                left_col_end: l_ce,
                right_line: r_line,
                right_col_start: r_cs,
                right_col_end: r_ce,
                left_offset_start: l_start,
                left_offset_end: l_end,
                right_offset_start: r_start,
                right_offset_end: r_end,
            });
        }
    }

    let left_total: usize = left_lines.values().map(|s| s.len()).sum();
    let right_total: usize = right_lines.values().map(|s| s.len()).sum();
    eprintln!(
        "build_token_relations: {} relations ({} left ids, {} right ids)",
        relations.len(),
        left_total,
        right_total,
    );

    relations
}

/// Compute the cumulative character offset at the start of each line.
///
/// GtkTextBuffer uses character (not byte) offsets. For each line we count
/// `line.chars().count()` characters plus 1 for the terminating newline.
fn compute_line_char_offsets(lines: &[String]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(lines.len());
    let mut running = 0usize;
    for line in lines {
        offsets.push(running);
        running += line.chars().count() + 1; // +1 for newline
    }
    offsets
}

fn find_identifier_col(lines: &[String], line_num: usize, id: &str) -> (usize, usize) {
    use crate::diff::engine::InlineDiffer;
    let line = &lines[line_num];
    if let Some((_module, ids)) = InlineDiffer::parse_import_line(line) {
        for (parsed_id, (start, end)) in &ids {
            if parsed_id == id {
                return (*start, *end);
            }
        }
    }
    (10, 20)
}

#[allow(dead_code)]
fn build_import_sets_from_text(
    lines: &[String],
) -> std::collections::HashMap<String, std::collections::HashSet<String>> {
    use crate::diff::engine::InlineDiffer;
    let mut map = std::collections::HashMap::new();
    for line in lines {
        if let Some((module, ids)) = InlineDiffer::parse_import_line(line) {
            let entry = map
                .entry(module)
                .or_insert_with(std::collections::HashSet::new);
            for (id, _) in ids {
                entry.insert(id);
            }
        }
    }
    map
}

fn build_import_line_sets_from_text(
    lines: &[String],
) -> std::collections::HashMap<usize, std::collections::HashSet<String>> {
    use crate::diff::engine::InlineDiffer;
    let mut map = std::collections::HashMap::new();
    for (i, line) in lines.iter().enumerate() {
        if let Some((_module, ids)) = InlineDiffer::parse_import_line(line) {
            let set: std::collections::HashSet<String> =
                ids.into_iter().map(|(id, _)| id).collect();
            map.insert(i, set);
        }
    }
    map
}

fn build_import_line_sets(
    buffer: &gsv::Buffer,
) -> std::collections::HashMap<usize, std::collections::HashSet<String>> {
    use crate::diff::engine::InlineDiffer;
    let mut map = std::collections::HashMap::new();
    let line_count = buffer.line_count().max(0) as usize;
    for i in 0..line_count {
        let s = buffer.iter_at_line_offset(i as i32, 0);
        let e = buffer.iter_at_line_offset((i + 1) as i32, 0);
        if let (Some(si), Some(ei)) = (s, e) {
            let line = buffer.text(&si, &ei, true).to_string();
            if let Some((_module, ids)) = InlineDiffer::parse_import_line(&line) {
                let set: std::collections::HashSet<String> =
                    ids.into_iter().map(|(id, _)| id).collect();
                map.insert(i, set);
            }
        }
    }
    map
}

fn compute_import_changes(
    line_this: &str,
    other_aggr_sets: &std::collections::HashMap<String, std::collections::HashSet<String>>,
    missing_op: DiffOp,
    other_line_sets: &std::collections::HashMap<usize, std::collections::HashSet<String>>,
) -> Vec<InlineChange> {
    use crate::diff::engine::{InlineChange, InlineDiffer};
    let (module, ids_this) = match InlineDiffer::parse_import_line(line_this) {
        Some(p) => p,
        None => return Vec::new(),
    };
    let empty_set = std::collections::HashSet::new();
    let other_ids = other_aggr_sets.get(&module).unwrap_or(&empty_set);
    let mut changes = Vec::new();
    for (id, (start, end)) in &ids_this {
        if !other_ids.contains(id) {
            changes.push(InlineChange {
                start: *start,
                end: *end,
                op: missing_op,
            });
        } else {
            let is_alone_here = ids_this.len() == 1;
            let alone_there = other_line_sets
                .iter()
                .any(|(_, l)| l.len() == 1 && l.contains(id));
            let merged_here = !is_alone_here && alone_there;
            let extracted_there = is_alone_here
                && other_line_sets
                    .iter()
                    .any(|(_, l)| l.len() > 1 && l.contains(id));
            if merged_here || extracted_there {
                changes.push(InlineChange {
                    start: *start,
                    end: *end,
                    op: DiffOp::Replace,
                });
            }
        }
    }
    changes
}

fn is_all_unchanged_import_line(
    buffer: &gsv::Buffer,
    start: &gtk::TextIter,
    end: &gtk::TextIter,
    other_sets: &std::collections::HashMap<String, std::collections::HashSet<String>>,
) -> bool {
    use crate::diff::engine::InlineDiffer;
    let line_text = buffer.text(start, end, true).to_string();
    let (module, ids) = match InlineDiffer::parse_import_line(&line_text) {
        Some(p) => p,
        None => return false,
    };
    let empty_set = std::collections::HashSet::new();
    let other_ids = other_sets.get(&module).unwrap_or(&empty_set);
    ids.iter().all(|(id, _)| other_ids.contains(id))
}

fn apply_inline_diff(
    buffer: &gsv::Buffer,
    other_buffer: &gsv::Buffer,
    tag_table: &gtk::TextTagTable,
    chunk: &Chunk,
    pane: usize,
    cache: &InlineDiffCache,
) {
    let (start_a, end_a, start_b, end_b) = (chunk.start_a, chunk.end_a, chunk.start_b, chunk.end_b);

    // Build aggregated import-identifier sets from ALL lines of the OTHER
    // buffer, grouped by module string.  This enables correct set-based
    // comparison even when an import is split across multiple lines on
    // one side.
    let other_import_sets = build_import_sets(other_buffer);
    let other_line_sets = build_import_line_sets(other_buffer);

    // Process each line pair within the chunk
    let line_count = (end_a - start_a).min(end_b - start_b);
    for offset in 0..line_count {
        let line_a_num = start_a + offset;
        let line_b_num = start_b + offset;

        // Get text of line from buffer A (this pane's buffer or other buffer)
        let (text_a, text_b) = if pane == 0 {
            // Pane 0: this buffer is A, other is B
            let a_start = buffer.iter_at_line_offset(line_a_num as i32, 0);
            let a_end = buffer.iter_at_line_offset((line_a_num + 1) as i32, 0);
            let b_start = other_buffer.iter_at_line_offset(line_b_num as i32, 0);
            let b_end = other_buffer.iter_at_line_offset((line_b_num + 1) as i32, 0);
            if let (Some(sa), Some(ea), Some(sb), Some(eb)) = (a_start, a_end, b_start, b_end) {
                (
                    buffer.text(&sa, &ea, true).to_string(),
                    other_buffer.text(&sb, &eb, true).to_string(),
                )
            } else {
                continue;
            }
        } else {
            // Pane 1: this buffer is B, other is A
            let a_start = other_buffer.iter_at_line_offset(line_a_num as i32, 0);
            let a_end = other_buffer.iter_at_line_offset((line_a_num + 1) as i32, 0);
            let b_start = buffer.iter_at_line_offset(line_b_num as i32, 0);
            let b_end = buffer.iter_at_line_offset((line_b_num + 1) as i32, 0);
            if let (Some(sa), Some(ea), Some(sb), Some(eb)) = (a_start, a_end, b_start, b_end) {
                (
                    other_buffer.text(&sa, &ea, true).to_string(),
                    buffer.text(&sb, &eb, true).to_string(),
                )
            } else {
                continue;
            }
        };

        if text_a.is_empty() || text_b.is_empty() {
            continue;
        }
        if text_a == text_b {
            continue;
        }

        // Try import-aware set comparison first.  When both lines are
        // imports from the same module, compare against the AGGREGATED
        // identifier set from the OTHER buffer (across all its lines)
        // rather than just the paired line.  This correctly classifies
        // identifiers that were split across multiple import lines.
        let this_line = if pane == 0 { &text_a } else { &text_b };
        let missing_op = if pane == 0 {
            DiffOp::Delete
        } else {
            DiffOp::Insert
        };
        let mut inline_changes =
            compute_import_changes(this_line, &other_import_sets, missing_op, &other_line_sets);
        if inline_changes.is_empty() {
            let is_import = InlineDiffer::parse_import_line(this_line).is_some();
            if !is_import {
                inline_changes = (*cache.compare_line_tokens(&text_a, &text_b)).clone();
            }
        }
        if inline_changes.is_empty() {
            continue;
        }

        // Determine base iterator for this pane's line
        let base_line = if pane == 0 { line_a_num } else { line_b_num };
        let base_iter = match buffer.iter_at_line_offset(base_line as i32, 0) {
            Some(iter) => iter,
            None => continue,
        };

        // Apply differentiated inline tags at the correct character offsets.
        // NOTE: token-level diffs mix Delete (left offsets) and Insert
        // (right offsets) changes in the same vector.  We must only apply
        // changes whose offsets refer to THIS pane's line.
        for change in inline_changes.iter() {
            let apply = match (pane, change.op) {
                (0, DiffOp::Delete | DiffOp::Replace) => true,
                (0, DiffOp::Insert) => false,
                (1, DiffOp::Insert | DiffOp::Replace) => true,
                (1, DiffOp::Delete) => false,
                _ => false,
            };
            if !apply {
                continue;
            }
            let tag_name = match change.op {
                DiffOp::Delete => "diff-inline-delete",
                DiffOp::Insert => "diff-inline-insert",
                DiffOp::Replace => "diff-inline-replace",
                DiffOp::Equal => continue,
            };

            if let Some(tag) = tag_table.lookup(tag_name) {
                let start_offset = base_iter.offset() as usize + change.start;
                let end_offset = base_iter.offset() as usize + change.end;
                let mut s = buffer.iter_at_offset(start_offset as i32);
                let mut e = buffer.iter_at_offset(end_offset as i32);
                // Adjust iterators to valid cursor positions so that
                // combining characters (Unicode diacritics) are not split
                // by the tag boundary.  Mirrors Python Meld.
                if !s.is_cursor_position() {
                    s.backward_cursor_position();
                }
                if !e.is_cursor_position() {
                    e.forward_cursor_position();
                }
                if s.offset() < e.offset() {
                    buffer.apply_tag(&tag, &s, &e);
                }
            }
        }

        // Extra lines on the side with more lines: compare against
        // aggregated opposite-side sets.
        let this_start = if pane == 0 { start_a } else { start_b };
        let this_end = if pane == 0 { end_a } else { end_b };
        let other_start = if pane == 0 { start_b } else { start_a };
        let other_end = if pane == 0 { end_b } else { end_a };
        let pair_count = (this_end - this_start).min(other_end - other_start);
        for extra in pair_count..(this_end - this_start) {
            let this_line_num = this_start + extra;
            let s_iter = buffer.iter_at_line_offset(this_line_num as i32, 0);
            let e_iter = buffer.iter_at_line_offset((this_line_num + 1) as i32, 0);
            if let (Some(si), Some(ei)) = (s_iter, e_iter) {
                let line_text = buffer.text(&si, &ei, true).to_string();
                let mut changes = compute_import_changes(
                    &line_text,
                    &other_import_sets,
                    missing_op,
                    &other_line_sets,
                );
                if changes.is_empty() {
                    continue;
                }
                for change in changes.iter() {
                    let tag_name = match change.op {
                        DiffOp::Delete => "diff-inline-delete",
                        DiffOp::Insert => "diff-inline-insert",
                        DiffOp::Replace => "diff-inline-replace",
                        DiffOp::Equal => continue,
                    };
                    if let Some(tag) = tag_table.lookup(tag_name) {
                        let so = si.offset() as usize + change.start;
                        let eo = si.offset() as usize + change.end;
                        let mut s = buffer.iter_at_offset(so as i32);
                        let mut e = buffer.iter_at_offset(eo as i32);
                        if !s.is_cursor_position() {
                            s.backward_cursor_position();
                        }
                        if !e.is_cursor_position() {
                            e.forward_cursor_position();
                        }
                        if s.offset() < e.offset() {
                            buffer.apply_tag(&tag, &s, &e);
                        }
                    }
                }
            }
        }
    }
}

fn ensure_tag_full(
    tag_table: &gtk::TextTagTable,
    name: &str,
    bg: &str,
    fg: &str,
    paragraph_bg: &str,
) {
    if tag_table.lookup(name).is_none() {
        let tag = gtk::TextTag::builder()
            .name(name)
            .background(bg)
            .foreground(fg)
            .paragraph_background(paragraph_bg)
            .build();
        tag_table.add(&tag);
    }
}

fn ensure_tag(tag_table: &gtk::TextTagTable, name: &str, bg: &str, fg: &str) {
    if tag_table.lookup(name).is_none() {
        let tag = gtk::TextTag::builder()
            .name(name)
            .background(bg)
            .foreground(fg)
            .build();
        tag_table.add(&tag);
    }
}

// ─── Chunk operation helpers (for use by gutter callbacks) ────────

/// Duration of the fading highlight animation for chunk actions (microseconds).
const FADE_DURATION_US: u32 = 500_000; // 500ms

/// Apply a temporary highlight to a range in the buffer, then remove it after
/// a delay. Mirrors the original Meld's `add_fading_highlight`.
fn add_fading_highlight(buffer: &gsv::Buffer, start: &gtk::TextIter, end: &gtk::TextIter) {
    let tag_table = buffer.tag_table();

    // Ensure the animation tag exists
    if tag_table.lookup("meld-fading-highlight").is_none() {
        let tag = gtk::TextTag::builder()
            .name("meld-fading-highlight")
            .background("#ffff00")
            .paragraph_background("rgba(255,255,0,0.3)")
            .build();
        tag_table.add(&tag);
    }

    if let Some(tag) = tag_table.lookup("meld-fading-highlight") {
        buffer.apply_tag(&tag, start, end);

        // Remove the highlight after the fade duration
        let buffer_clone = buffer.clone();
        let start_offset = start.offset();
        let end_offset = end.offset();
        glib::timeout_add_local(
            std::time::Duration::from_micros(FADE_DURATION_US as u64),
            move || {
                let s = buffer_clone.iter_at_offset(start_offset);
                let e = buffer_clone.iter_at_offset(end_offset);
                if s.offset() < e.offset() {
                    if let Some(t) = buffer_clone.tag_table().lookup("meld-fading-highlight") {
                        buffer_clone.remove_tag(&t, &s, &e);
                    }
                }
                glib::ControlFlow::Break
            },
        );
    }
}

/// Execute a replace operation: copy text from src to dst at the chunk position.
fn execute_replace(src_buffer: &gsv::Buffer, dst_buffer: &gsv::Buffer, chunk: &Chunk) {
    // Get source text
    let src_start = src_buffer.iter_at_line_offset(chunk.start_a as i32, 0);
    let src_end = src_buffer.iter_at_line_offset(chunk.end_a as i32, 0);

    let src_text = if let (Some(s), Some(e)) = (src_start, src_end) {
        if s.offset() < e.offset() {
            src_buffer.text(&s, &e, true).to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    // Replace in destination
    dst_buffer.begin_user_action();

    let dst_start = dst_buffer.iter_at_line_offset(chunk.start_b as i32, 0);
    let dst_end = dst_buffer.iter_at_line_offset(chunk.end_b as i32, 0);

    if let (Some(ds), Some(de)) = (dst_start, dst_end) {
        if ds.offset() < de.offset() {
            dst_buffer.delete(&mut ds.clone(), &mut de.clone());
        }
        let insert_pos = dst_buffer.iter_at_line_offset(chunk.start_b as i32, 0);
        if let Some(pos) = insert_pos {
            dst_buffer.insert(&mut pos.clone(), &src_text);
        }
    }

    dst_buffer.end_user_action();

    // Fading highlight at the inserted position
    if let Some(ins) = dst_buffer.iter_at_line_offset(chunk.start_b as i32, 0) {
        let line_count = src_text.lines().count().max(1);
        if let Some(end) = dst_buffer.iter_at_line_offset((chunk.start_b + line_count) as i32, 0) {
            add_fading_highlight(dst_buffer, &ins, &end);
        }
    }
}

/// Execute a delete operation: remove text from the source buffer.
fn execute_delete(buffer: &gsv::Buffer, chunk: &Chunk) {
    buffer.begin_user_action();

    let start_iter = buffer.iter_at_line_offset(chunk.start_a.max(0) as i32, 0);
    let end_iter = if chunk.end_a > chunk.start_a {
        buffer.iter_at_line_offset(chunk.end_a as i32, 0)
    } else {
        buffer.iter_at_line_offset(chunk.start_a as i32, 0)
    };

    if let (Some(start), Some(end)) = (start_iter, end_iter) {
        if start.offset() < end.offset() {
            buffer.delete(&mut start.clone(), &mut end.clone());
        }
    }

    buffer.end_user_action();
}

/// Execute a copy operation: copy text from src to dst (up or down).
fn execute_copy(src_buffer: &gsv::Buffer, dst_buffer: &gsv::Buffer, chunk: &Chunk, copy_up: bool) {
    let src_start = src_buffer.iter_at_line_offset(chunk.start_a as i32, 0);
    let src_end = src_buffer.iter_at_line_offset(chunk.end_a as i32, 0);

    let mut src_text = if let (Some(s), Some(e)) = (src_start, src_end) {
        if s.offset() < e.offset() {
            src_buffer.text(&s, &e, true).to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    dst_buffer.begin_user_action();

    if copy_up {
        if chunk.end_a >= src_buffer.line_count().max(0) as usize
            && chunk.start_b < dst_buffer.line_count().max(0) as usize
        {
            src_text.push('\n');
        }
        let insert_pos = dst_buffer.iter_at_line_offset(chunk.start_b as i32, 0);
        if let Some(mut pos) = insert_pos {
            dst_buffer.insert(&mut pos, &src_text);
        }
    } else {
        let insert_pos = dst_buffer.iter_at_line_offset(chunk.end_b as i32, 0);
        if let Some(mut pos) = insert_pos {
            dst_buffer.insert(&mut pos, &src_text);
        }
    }

    dst_buffer.end_user_action();
}
