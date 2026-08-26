//! Find bar — in-file search and replace widget.
//!
//! Provides a bottom bar with a search entry, next/previous navigation,
//! match counting, and (in replace mode) a replace entry with Replace and
//! Replace All actions. Search is backed by `GtkSource.SearchContext` /
//! `GtkSource.SearchSettings`, matching the original Meld `FindBar`.
//!
//! The bar is shared across all text views in a comparison: when shown
//! it attaches to the currently focused pane, and highlights are
//! cleared from the previous pane automatically.

use glib::prelude::*;
use gtk4 as gtk;
use gtk4::prelude::*;
use sourceview5 as gsv;
use sourceview5::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A search-and-replace bar that can be dynamically attached to any `gsv::View`.
pub struct FindBar {
    container: gtk::Box,
    entry: gtk::SearchEntry,
    replace_entry: gtk::Entry,
    replace_button: gtk::Button,
    replace_all_button: gtk::Button,
    status_label: gtk::Label,
    search_settings: gsv::SearchSettings,
    /// The currently associated source view, if any.
    text_view: Rc<RefCell<Option<gsv::View>>>,
    /// Search context for the currently associated buffer, if any.
    search_context: Rc<RefCell<Option<gsv::SearchContext>>>,
    /// Whether replace controls are visible.
    replace_mode: Rc<Cell<bool>>,
}

impl FindBar {
    /// Create a new find bar without a text view. Call [`start_find`] or
    /// [`start_find_replace`] to associate it with a view and show it.
    pub fn new() -> Self {
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        container.add_css_class("toolbar");
        container.set_visible(false);

        let entry = gtk::SearchEntry::new();
        entry.set_placeholder_text(Some("Find..."));
        entry.set_width_chars(24);
        container.append(&entry);

        let case_sensitive = gtk::CheckButton::with_label("Aa");
        case_sensitive.set_tooltip_text(Some("Match case"));
        container.append(&case_sensitive);

        let whole_word = gtk::CheckButton::with_label("W");
        whole_word.set_tooltip_text(Some("Whole words"));
        container.append(&whole_word);

        let regex = gtk::CheckButton::with_label(".*");
        regex.set_tooltip_text(Some("Regular expression"));
        container.append(&regex);

        let prev_btn = gtk::Button::from_icon_name("go-up-symbolic");
        container.append(&prev_btn);

        let next_btn = gtk::Button::from_icon_name("go-down-symbolic");
        container.append(&next_btn);

        let replace_entry = gtk::Entry::new();
        replace_entry.set_placeholder_text(Some("Replace with..."));
        replace_entry.set_width_chars(16);
        replace_entry.set_visible(false);
        container.append(&replace_entry);

        let replace_button = gtk::Button::with_label("Replace");
        replace_button.set_visible(false);
        container.append(&replace_button);

        let replace_all_button = gtk::Button::with_label("Replace All");
        replace_all_button.set_visible(false);
        container.append(&replace_all_button);

        let status_label = gtk::Label::new(None);
        container.append(&status_label);

        let close_btn = gtk::Button::from_icon_name("window-close-symbolic");
        container.append(&close_btn);

        // GtkSource search settings shared by every context we create.
        let search_settings = gsv::SearchSettings::new();
        search_settings.set_wrap_around(true);

        let text_view: Rc<RefCell<Option<gsv::View>>> = Rc::new(RefCell::new(None));
        let search_context: Rc<RefCell<Option<gsv::SearchContext>>> = Rc::new(RefCell::new(None));
        let replace_mode: Rc<Cell<bool>> = Rc::new(Cell::new(false));

        // Bind the option toggles to the search settings.
        let ss_case = search_settings.clone();
        case_sensitive.connect_toggled(move |b| ss_case.set_case_sensitive(b.is_active()));
        let ss_word = search_settings.clone();
        whole_word.connect_toggled(move |b| ss_word.set_at_word_boundaries(b.is_active()));
        let ss_regex = search_settings.clone();
        regex.connect_toggled(move |b| ss_regex.set_regex_enabled(b.is_active()));

        // Search text changed — update settings and re-run the search.
        let tv_search = Rc::clone(&text_view);
        let ctx_search = Rc::clone(&search_context);
        let status_search = status_label.clone();
        let ss_search = search_settings.clone();
        entry.connect_search_changed(move |entry| {
            let text = entry.text().to_string();
            ss_search.set_search_text(Some(&text));
            find_text(&tv_search, &ctx_search, &status_search, false, false);
        });

        // Next button.
        let tv_next = Rc::clone(&text_view);
        let ctx_next = Rc::clone(&search_context);
        let status_next = status_label.clone();
        next_btn.connect_clicked(move |_| {
            find_text(&tv_next, &ctx_next, &status_next, false, true);
        });

        // Previous button.
        let tv_prev = Rc::clone(&text_view);
        let ctx_prev = Rc::clone(&search_context);
        let status_prev = status_label.clone();
        prev_btn.connect_clicked(move |_| {
            find_text(&tv_prev, &ctx_prev, &status_prev, true, false);
        });

        // Replace button — replace the current match, then advance.
        let tv_replace = Rc::clone(&text_view);
        let ctx_replace = Rc::clone(&search_context);
        let status_replace = status_label.clone();
        let entry_replace = replace_entry.clone();
        replace_button.connect_clicked(move |_| {
            replace_current(
                &tv_replace,
                &ctx_replace,
                &entry_replace.text(),
                &status_replace,
            );
        });

        // Replace All button.
        let ctx_replace_all = Rc::clone(&search_context);
        let status_replace_all = status_label.clone();
        let entry_replace_all = replace_entry.clone();
        replace_all_button.connect_clicked(move |_| {
            let count = replace_all(&ctx_replace_all, &entry_replace_all.text());
            status_replace_all.set_text(&format!("{} replaced", count));
        });

        // Close button — hide the bar and clear the search context.
        let container_weak = container.downgrade();
        let ctx_close = Rc::clone(&search_context);
        let tv_close = Rc::clone(&text_view);
        close_btn.connect_clicked(move |_| {
            if let Some(c) = container_weak.upgrade() {
                c.set_visible(false);
            }
            *ctx_close.borrow_mut() = None;
            *tv_close.borrow_mut() = None;
        });

        Self {
            container,
            entry,
            replace_entry,
            replace_button,
            replace_all_button,
            status_label,
            search_settings,
            text_view,
            search_context,
            replace_mode,
        }
    }

    /// Reference to the container widget.
    pub fn widget(&self) -> &gtk::Widget {
        self.container.upcast_ref()
    }

    /// Attach to `view`, creating a search context for its buffer.
    fn set_view(&self, view: &gsv::View) {
        let buffer = match view.buffer().downcast::<gsv::Buffer>() {
            Ok(buffer) => buffer,
            Err(_) => return,
        };
        let context = gsv::SearchContext::new(&buffer, Some(&self.search_settings));
        context.set_highlight(true);

        let status = self.status_label.clone();
        context.connect_occurrences_count_notify(move |ctx| {
            let count = ctx.occurrences_count();
            let text = if count >= 0 {
                format!("{} matches", count)
            } else {
                String::new()
            };
            status.set_text(&text);
        });

        *self.search_context.borrow_mut() = Some(context);
        *self.text_view.borrow_mut() = Some(view.clone());
    }

    fn apply_replace_mode(&self) {
        let mode = self.replace_mode.get();
        self.replace_entry.set_visible(mode);
        self.replace_button.set_visible(mode);
        self.replace_all_button.set_visible(mode);
    }

    /// Show the find bar for `view` in find-only mode.
    pub fn start_find(&self, view: &gsv::View) {
        self.replace_mode.set(false);
        self.set_view(view);
        self.apply_replace_mode();
        self.container.set_visible(true);
        self.entry.grab_focus();
    }

    /// Show the find bar for `view` in find-and-replace mode.
    pub fn start_find_replace(&self, view: &gsv::View) {
        self.replace_mode.set(true);
        self.set_view(view);
        self.apply_replace_mode();
        self.container.set_visible(true);
        self.entry.grab_focus();
    }

    /// Jump to the next match in `view` without changing the bar's mode.
    pub fn start_find_next(&self, view: &gsv::View) {
        self.set_view(view);
        let status = self.status_label.clone();
        find_text(&self.text_view, &self.search_context, &status, false, true);
    }

    /// Jump to the previous match in `view` without changing the bar's mode.
    pub fn start_find_previous(&self, view: &gsv::View) {
        self.set_view(view);
        let status = self.status_label.clone();
        find_text(&self.text_view, &self.search_context, &status, true, false);
    }

    /// Hide the find bar and clear the search context.
    pub fn hide(&self) {
        self.container.set_visible(false);
        *self.search_context.borrow_mut() = None;
        *self.text_view.borrow_mut() = None;
    }

    /// Whether the find bar is currently visible.
    pub fn is_visible(&self) -> bool {
        self.container.is_visible()
    }
}

/// Run a search from the cursor and select the result.
///
/// `advance` mirrors Meld's `start_offset`: forward searches advance the
/// cursor by one character first so they don't re-match the current match.
fn find_text(
    text_view: &Rc<RefCell<Option<gsv::View>>>,
    search_context: &Rc<RefCell<Option<gsv::SearchContext>>>,
    status: &gtk::Label,
    backwards: bool,
    advance: bool,
) {
    let ctx = search_context.borrow();
    let Some(ctx) = ctx.as_ref() else {
        return;
    };
    let buffer = ctx.buffer();
    let mut insert = buffer.iter_at_mark(&buffer.get_insert());
    if !backwards && advance {
        insert.forward_char();
    }

    let result = if backwards {
        ctx.backward(&insert)
    } else {
        ctx.forward(&insert)
    };

    match result {
        Some((start, end, wrapped)) => {
            buffer.select_range(&start, &end);
            if let Some(view) = text_view.borrow().as_ref() {
                view.scroll_mark_onscreen(&buffer.get_insert());
            }
            status.set_text(if wrapped { "Wrapped" } else { "" });
        }
        None => {
            status.set_text("Not found");
        }
    }
}

/// Replace the currently selected match, if any.
fn replace_current(
    text_view: &Rc<RefCell<Option<gsv::View>>>,
    search_context: &Rc<RefCell<Option<gsv::SearchContext>>>,
    replacement: &str,
    status: &gtk::Label,
) {
    let ctx = search_context.borrow();
    let Some(ctx) = ctx.as_ref() else {
        return;
    };
    let buffer = ctx.buffer();

    let old_sel = buffer.selection_bounds();
    find_text(text_view, search_context, status, false, false);
    let new_sel = buffer.selection_bounds();

    let same_selection = match (&old_sel, &new_sel) {
        (Some((a0, a1)), Some((b0, b1))) => a0 == b0 && a1 == b1,
        _ => false,
    };

    if same_selection {
        if let Some((start, end)) = new_sel {
            let mut s = start;
            let mut e = end;
            if ctx.replace(&mut s, &mut e, replacement).is_err() {
                status.set_text("Replace failed");
                return;
            }
        }
        find_text(text_view, search_context, status, false, false);
    }
}

/// Replace every match and return the number of replacements made.
fn replace_all(search_context: &Rc<RefCell<Option<gsv::SearchContext>>>, replacement: &str) -> u32 {
    let ctx = search_context.borrow();
    let Some(ctx) = ctx.as_ref() else {
        return 0;
    };
    let buffer = ctx.buffer();
    let mut count = 0u32;
    let mut search_from = buffer.start_iter();

    loop {
        let Some((start, end, _)) = ctx.forward(&search_from) else {
            break;
        };
        let mut s = start;
        let mut e = end;
        if ctx.replace(&mut s, &mut e, replacement).is_err() {
            break;
        }
        count += 1;
        search_from = e;
        if !search_from.forward_char() {
            break;
        }
    }

    count
}
