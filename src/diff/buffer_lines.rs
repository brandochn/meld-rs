#![cfg(feature = "gui")]
//! Lazy-cached line access to a GtkSourceBuffer.
//!
//! Mirrors the original Meld's `BufferLines` / `MeldBuffer` which provides
//! indexed line access with automatic cache invalidation on text edits.
//!
//! Lines are fetched on demand and cached. When the buffer is modified,
//! only the affected line range is invalidated — not the entire cache.

use gtk4::prelude::*;
use sourceview5 as gsv;
use std::cell::RefCell;
use std::rc::Rc;

/// Provides lazy, cached access to individual lines in a [`gsv::Buffer`].
///
/// The cache is automatically invalidated when text is inserted or deleted
/// within the buffer, but only for the affected line range.
pub struct BufferLines {
    buffer: gsv::Buffer,
    /// Cached line texts: `None` = not yet fetched, `Some(text)` = cached.
    cache: RefCell<Vec<Option<String>>>,
    /// Signal handler IDs for automatic cache invalidation.
    insert_handler: RefCell<Option<glib::SignalHandlerId>>,
    delete_handler: RefCell<Option<glib::SignalHandlerId>>,
    /// Optional text filter applied to each line before caching.
    filter: RefCell<Option<Box<dyn Fn(&str) -> String>>>,
}

impl BufferLines {
    /// Create a new `BufferLines` wrapping the given buffer.
    ///
    /// Automatically connects to the buffer's `insert-text` and `delete-range`
    /// signals to invalidate affected cache entries on edits.
    pub fn new(buffer: &gsv::Buffer) -> Rc<Self> {
        let cache = RefCell::new(vec![None; (buffer.line_count().max(1) + 1) as usize]);

        let this = Rc::new(Self {
            buffer: buffer.clone(),
            cache,
            insert_handler: RefCell::new(None),
            delete_handler: RefCell::new(None),
            filter: RefCell::new(None),
        });

        // Connect invalidation signals
        Self::connect_signals(Rc::clone(&this));

        this
    }

    /// Set an optional text filter applied to each line.
    pub fn set_filter<F: Fn(&str) -> String + 'static>(&self, filter: F) {
        self.filter.replace(Some(Box::new(filter)));
        self.invalidate_all();
    }

    /// Get the text of a specific line (0-indexed).
    /// Returns `None` if the index is out of bounds.
    pub fn line(&self, index: usize) -> Option<String> {
        let mut cache = self.cache.borrow_mut();

        // Ensure cache has enough capacity
        let line_count = self.buffer.line_count().max(0) as usize;
        if cache.len() <= line_count {
            cache.resize(line_count + 1, None);
        }

        if index >= line_count {
            return None;
        }

        if let Some(Some(ref text)) = cache.get(index) {
            return Some(text.clone());
        }

        // Fetch from buffer
        let start = self.buffer.iter_at_line_offset(index as i32, 0);
        let end = crate::diff::filediff::iter_at_line_or_end(
            &self.buffer, (index + 1) as i32,
        );

        if let Some(s) = start {
            let raw = self.buffer.text(&s, &end, true).to_string();
            let text = if let Some(ref filter) = *self.filter.borrow() {
                filter(&raw)
            } else {
                raw
            };

            if index < cache.len() {
                cache[index] = Some(text.clone());
            }
            Some(text)
        } else {
            None
        }
    }

    /// Get all lines as a `Vec<String>`. Uses cache where available.
    pub fn all_lines(&self) -> Vec<String> {
        let count = self.buffer.line_count().max(0) as usize;
        let mut result = Vec::with_capacity(count);
        for i in 0..count {
            if let Some(line) = self.line(i) {
                result.push(line);
            }
        }
        result
    }

    /// Get a slice of lines `[start..end)`. Uses cache where available.
    pub fn lines_range(&self, start: usize, end: usize) -> Vec<String> {
        let count = self.buffer.line_count().max(0) as usize;
        let end = end.min(count);
        let mut result = Vec::with_capacity(end.saturating_sub(start));
        for i in start..end {
            if let Some(line) = self.line(i) {
                result.push(line);
            }
        }
        result
    }

    /// Invalidate cached lines in the given range `[start..end)`.
    pub fn invalidate(&self, start: usize, end: usize) {
        let mut cache = self.cache.borrow_mut();
        let end = end.min(cache.len());
        let start = start.min(end);
        for entry in &mut cache[start..end] {
            *entry = None;
        }
    }

    /// Invalidate all cached lines.
    pub fn invalidate_all(&self) {
        let mut cache = self.cache.borrow_mut();
        for entry in cache.iter_mut() {
            *entry = None;
        }
    }

    /// Return the total number of lines in the buffer.
    pub fn len(&self) -> usize {
        self.buffer.line_count().max(0) as usize
    }

    /// Returns `true` if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    // ── Signal wiring ──────────────────────────────────────────

    fn connect_signals(this: Rc<Self>) {
        let this_insert = Rc::clone(&this);
        let this_delete = Rc::clone(&this);

        // On text insertion: invalidate from the insertion line onward
        let insert_id = this.buffer.connect_insert_text(move |_, pos, _text| {
            let line = pos.line().max(0) as usize;
            // Invalidate from this line to the end
            let total = this_insert.buffer.line_count().max(0) as usize;
            this_insert.invalidate(line, total + 1);
        });
        *this.insert_handler.borrow_mut() = Some(insert_id);

        // On text deletion: invalidate the affected range
        let delete_id = this.buffer.connect_delete_range(move |_, start, end| {
            let start_line = start.line().max(0) as usize;
            let end_line = end.line().max(0) as usize;
            let total = this_delete.buffer.line_count().max(0) as usize;
            this_delete.invalidate(start_line, total + 1);
        });
        *this.delete_handler.borrow_mut() = Some(delete_id);
    }
}

impl Drop for BufferLines {
    fn drop(&mut self) {
        if let Some(id) = self.insert_handler.borrow_mut().take() {
            self.buffer.disconnect(id);
        }
        if let Some(id) = self.delete_handler.borrow_mut().take() {
            self.buffer.disconnect(id);
        }
    }
}

#[cfg(test)]
mod tests {
    // BufferLines tests require a GTK context; skipped in unit tests.
    // Integration tests should exercise this via the actual GTK application.
}
