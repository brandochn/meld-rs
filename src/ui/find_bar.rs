//! Find bar — in-file search widget.
//!
//! Provides a bottom bar with a search entry, next/previous navigation,
//! and match counting for searching within source view panes.

use glib::prelude::*;
use gtk4 as gtk;
use gtk4::prelude::*;

/// A search bar that searches within a `gtk::TextView`.
pub struct FindBar {
    container: gtk::Box,
    entry: gtk::SearchEntry,
    status_label: gtk::Label,
    text_view: gtk::TextView,
}

impl FindBar {
    /// Create a new find bar attached to the given text view.
    pub fn new(text_view: &gtk::TextView) -> Self {
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        container.add_css_class("toolbar");

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

        let text_view_weak = text_view.downgrade();
        let entry_clone = entry.clone();
        let status_clone = status_label.clone();
        next_btn.connect_clicked(move |_| {
            if let Some(tv) = text_view_weak.upgrade() {
                find_next(&tv, &entry_clone, &status_clone);
            }
        });

        let tv_prev = text_view.downgrade();
        let entry_prev = entry.clone();
        let status_prev = status_label.clone();
        prev_btn.connect_clicked(move |_| {
            if let Some(tv) = tv_prev.upgrade() {
                find_previous(&tv, &entry_prev, &status_prev);
            }
        });

        let tv_entry = text_view.downgrade();
        let status_entry = status_label.clone();
        entry.connect_search_changed(move |entry| {
            if let Some(tv) = tv_entry.upgrade() {
                highlight_all(&tv, entry, &status_entry);
            }
        });

        // Hide the bar on close
        let container_weak = container.downgrade();
        close_btn.connect_clicked(move |_| {
            if let Some(c) = container_weak.upgrade() {
                c.set_visible(false);
            }
        });

        Self {
            container,
            entry,
            status_label,
            text_view: text_view.clone(),
        }
    }

    /// Reference to the container widget.
    pub fn widget(&self) -> &gtk::Widget {
        self.container.upcast_ref()
    }

    /// Show the find bar and focus the search entry.
    pub fn show(&self) {
        self.container.set_visible(true);
        self.entry.grab_focus();
    }

    /// Hide the find bar.
    pub fn hide(&self) {
        self.container.set_visible(false);
    }

    /// Clear search highlights from the text view.
    pub fn clear_highlights(&self) {
        remove_search_highlights(&self.text_view);
    }
}

fn find_next(view: &gtk::TextView, entry: &gtk::SearchEntry, status: &gtk::Label) {
    let query = entry.text().to_string();
    if query.is_empty() {
        return;
    }

    let buffer = view.buffer();
    let text = buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .to_string();

    let cursor_pos = get_cursor_offset(&buffer);
    let search_from = if cursor_pos < text.len() {
        &text[cursor_pos..]
    } else {
        ""
    };

    if let Some(pos) = search_from.find(&query) {
        let offset = cursor_pos + pos;
        let iter = buffer.iter_at_offset(offset as i32);
        let mut end = iter.clone();
        end.forward_chars(query.len() as i32);
        buffer.select_range(&iter, &end);
        let mark = buffer.create_mark(Some("find-start"), &iter, false);
        view.scroll_to_mark(&mark, 0.0, false, 0.0, 0.0);
        buffer.delete_mark(&mark);
    } else {
        status.set_text("Not found");
    }
}

fn find_previous(view: &gtk::TextView, entry: &gtk::SearchEntry, status: &gtk::Label) {
    let query = entry.text().to_string();
    if query.is_empty() {
        return;
    }

    let buffer = view.buffer();
    let text = buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .to_string();

    let cursor_pos = get_cursor_offset(&buffer);
    let search_in = &text[..cursor_pos.min(text.len())];

    if let Some(pos) = search_in.rfind(&query) {
        let iter = buffer.iter_at_offset(pos as i32);
        let mut end = iter.clone();
        end.forward_chars(query.len() as i32);
        buffer.select_range(&iter, &end);
        let mark = buffer.create_mark(Some("find-start"), &iter, false);
        view.scroll_to_mark(&mark, 0.0, false, 0.0, 0.0);
        buffer.delete_mark(&mark);
    } else {
        status.set_text("Not found");
    }
}

fn highlight_all(view: &gtk::TextView, entry: &gtk::SearchEntry, status: &gtk::Label) {
    remove_search_highlights(view);

    let query = entry.text().to_string();
    if query.is_empty() {
        status.set_text("");
        return;
    }

    let buffer = view.buffer();
    let text = buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .to_string();

    let tag_table = buffer.tag_table();
    let highlight_tag = gtk::TextTag::builder()
        .name("search-highlight")
        .background("rgba(255,255,0,0.5)")
        .build();
    tag_table.add(&highlight_tag);

    let mut match_count = 0;
    for (idx, _) in text.match_indices(&query) {
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
