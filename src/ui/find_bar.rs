//! Find bar — in-file search widget.
//!
//! Provides a bottom bar with a search entry, next/previous navigation,
//! and match counting for searching within source view panes.
//!
//! The bar is shared across all text views in a comparison: when shown
//! it attaches to the currently focused pane, and highlights are
//! cleared from the previous pane automatically.

use glib::prelude::*;
use gtk4 as gtk;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// A search bar that can be dynamically attached to any `gtk::TextView`.
pub struct FindBar {
    container: gtk::Box,
    entry: gtk::SearchEntry,
    status_label: gtk::Label,
    /// The currently associated text view, if any.
    text_view: Rc<RefCell<Option<gtk::TextView>>>,
}

impl FindBar {
    /// Create a new find bar without a text view. Call [`start_find`]
    /// to associate it with a text view and show it.
    pub fn new() -> Self {
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        container.add_css_class("toolbar");
        container.set_visible(false);

        let entry = gtk::SearchEntry::new();
        entry.set_placeholder_text(Some("Find..."));
        entry.set_width_chars(30);
        container.append(&entry);

        let prev_btn = gtk::Button::from_icon_name("go-up-symbolic");
        container.append(&prev_btn);

        let next_btn = gtk::Button::from_icon_name("go-down-symbolic");
        container.append(&next_btn);

        let status_label = gtk::Label::new(None);
        container.append(&status_label);

        let close_btn = gtk::Button::from_icon_name("window-close-symbolic");
        container.append(&close_btn);

        let text_view: Rc<RefCell<Option<gtk::TextView>>> = Rc::new(RefCell::new(None));

        // Next button
        let tv_next = Rc::clone(&text_view);
        let entry_next = entry.clone();
        let status_next = status_label.clone();
        next_btn.connect_clicked(move |_| {
            if let Some(ref tv) = *tv_next.borrow() {
                find_next(tv, &entry_next, &status_next);
            }
        });

        // Previous button
        let tv_prev = Rc::clone(&text_view);
        let entry_prev = entry.clone();
        let status_prev = status_label.clone();
        prev_btn.connect_clicked(move |_| {
            if let Some(ref tv) = *tv_prev.borrow() {
                find_previous(tv, &entry_prev, &status_prev);
            }
        });

        // Search entry changed — highlight all matches
        let tv_search = Rc::clone(&text_view);
        let status_search = status_label.clone();
        entry.connect_search_changed(move |entry| {
            if let Some(ref tv) = *tv_search.borrow() {
                highlight_all(tv, entry, &status_search);
            }
        });

        // Close button — hide the bar and clear highlights
        let tv_close = Rc::clone(&text_view);
        let container_weak = container.downgrade();
        close_btn.connect_clicked(move |_| {
            if let Some(c) = container_weak.upgrade() {
                c.set_visible(false);
            }
            if let Some(ref tv) = *tv_close.borrow() {
                remove_search_highlights(tv);
            }
        });

        Self {
            container,
            entry,
            status_label,
            text_view,
        }
    }

    /// Reference to the container widget.
    pub fn widget(&self) -> &gtk::Widget {
        self.container.upcast_ref()
    }

    /// Associate this find bar with a text view, clear any previous
    /// highlights, and show the bar with the search entry focused.
    pub fn start_find(&self, text_view: &gtk::TextView) {
        // Clear highlights from the previously attached text view
        if let Some(ref old_tv) = *self.text_view.borrow() {
            remove_search_highlights(old_tv);
        }
        *self.text_view.borrow_mut() = Some(text_view.clone());
        self.entry.set_text("");
        self.container.set_visible(true);
        self.entry.grab_focus();
    }

    /// Hide the find bar and clear search highlights.
    pub fn hide(&self) {
        self.container.set_visible(false);
        if let Some(ref tv) = *self.text_view.borrow() {
            remove_search_highlights(tv);
        }
    }

    /// Whether the find bar is currently visible.
    pub fn is_visible(&self) -> bool {
        self.container.is_visible()
    }
}

fn find_next(view: &gtk::TextView, entry: &gtk::SearchEntry, status: &gtk::Label) {
    let query = entry.text().to_string();
    if query.is_empty() {
        return;
    }
    let query_lower = query.to_ascii_lowercase();

    let buffer = view.buffer();
    let text = buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .to_string();
    // to_ascii_lowercase() preserves byte length (only A-Z → a-z),
    // so offsets found in text_lower are valid for the original text.
    let text_lower = text.to_ascii_lowercase();

    let cursor_pos = get_cursor_offset(&buffer);

    // Search from cursor position; wrap around to the beginning
    // if no match is found past the cursor.
    let start = cursor_pos.min(text_lower.len());
    let (found_offset, wrapped) = if let Some(pos) = text_lower[start..].find(&query_lower) {
        (start + pos, false)
    } else if let Some(pos) = text_lower[..start].find(&query_lower) {
        (pos, true)
    } else {
        status.set_text("Not found");
        return;
    };

    let iter = buffer.iter_at_offset(found_offset as i32);
    let mut end = iter.clone();
    end.forward_chars(query.len() as i32);
    // Put the cursor at the *end* of the match so the next
    // find-next call advances past this occurrence.
    buffer.select_range(&end, &iter);
    let mark = buffer.create_mark(Some("find-start"), &iter, false);
    view.scroll_to_mark(&mark, 0.0, false, 0.0, 0.0);
    buffer.delete_mark(&mark);
    if wrapped {
        status.set_text("Wrapped to top");
    }
}

fn find_previous(view: &gtk::TextView, entry: &gtk::SearchEntry, status: &gtk::Label) {
    let query = entry.text().to_string();
    if query.is_empty() {
        return;
    }
    let query_lower = query.to_ascii_lowercase();

    let buffer = view.buffer();
    let text = buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .to_string();
    // to_ascii_lowercase() preserves byte length (only A-Z → a-z),
    // so offsets found in text_lower are valid for the original text.
    let text_lower = text.to_ascii_lowercase();

    let cursor_pos = get_cursor_offset(&buffer);

    // Search backwards from the cursor; wrap around to the end
    // if no match is found before the cursor.
    let limit = cursor_pos.min(text_lower.len());
    let (found_offset, wrapped) = if let Some(pos) = text_lower[..limit].rfind(&query_lower) {
        (pos, false)
    } else if let Some(pos) = text_lower.rfind(&query_lower) {
        (pos, true)
    } else {
        status.set_text("Not found");
        return;
    };

    let iter = buffer.iter_at_offset(found_offset as i32);
    let mut end = iter.clone();
    end.forward_chars(query.len() as i32);
    // Put the cursor at the *start* of the match so the next
    // find-previous call looks before this occurrence.
    buffer.select_range(&iter, &end);
    let mark = buffer.create_mark(Some("find-start"), &iter, false);
    view.scroll_to_mark(&mark, 0.0, false, 0.0, 0.0);
    buffer.delete_mark(&mark);
    if wrapped {
        status.set_text("Wrapped to bottom");
    }
}

fn highlight_all(view: &gtk::TextView, entry: &gtk::SearchEntry, status: &gtk::Label) {
    remove_search_highlights(view);

    let query = entry.text().to_string();
    if query.is_empty() {
        status.set_text("");
        return;
    }
    let query_lower = query.to_ascii_lowercase();

    let buffer = view.buffer();
    let text = buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .to_string();
    // to_ascii_lowercase() preserves byte length (only A-Z → a-z),
    // so offsets found in text_lower are valid for the original text.
    let text_lower = text.to_ascii_lowercase();

    let tag_table = buffer.tag_table();
    let highlight_tag = gtk::TextTag::builder()
        .name("search-highlight")
        .background("rgba(255,255,0,0.5)")
        .build();
    tag_table.add(&highlight_tag);

    let mut match_count = 0;
    for (idx, _) in text_lower.match_indices(&query_lower) {
        match_count += 1;
        let start = buffer.iter_at_offset(idx as i32);
        let mut end = buffer.iter_at_offset(idx as i32);
        end.forward_chars(query.len() as i32);
        buffer.apply_tag(&highlight_tag, &start, &end);
    }

    status.set_text(&format!("{} matches", match_count));
}

fn get_cursor_offset(buffer: &gtk::TextBuffer) -> usize {
    buffer.cursor_position() as usize
}

fn remove_search_highlights(view: &gtk::TextView) {
    let buffer = view.buffer();
    let tag_table = buffer.tag_table();
    if let Some(tag) = tag_table.lookup("search-highlight") {
        let start = buffer.start_iter();
        let end = buffer.end_iter();
        buffer.remove_tag(&tag, &start, &end);
    }
}
