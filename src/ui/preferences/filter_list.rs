#![cfg(feature = "gui")]
//! Editable filter list — GTK4 replica of the original Meld's `FilterList`
//! widget (`filter-list.ui` + `preferences.py`).
//!
//! Rows are plain widgets (active toggle, editable name, editable pattern
//! with a validity icon) in a `GtkListBox`, plus a toolbar with Add,
//! Remove, Move Up and Move Down buttons — mirroring the original
//! behaviour.  The original used a `GtkTreeView`; a `GtkListBox` is used
//! here because the gtk4-rs 0.10 TreeView bindings provide no way to read
//! cell values back from the model.

use gtk4 as gtk;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use crate::config::settings::FilterEntry;

/// The toolbar buttons whose sensitivity depends on the selection.
struct ToolbarButtons {
    remove: gtk::Button,
    move_up: gtk::Button,
    move_down: gtk::Button,
}

/// One editable filter row.
struct RowData {
    check: gtk::CheckButton,
    name: gtk::Entry,
    pattern: gtk::Entry,
    icon: gtk::Image,
    widget: gtk::Box,
}

/// Closures invoked when the dialog closes, to flush uncommitted edits.
type FlushSink = Rc<RefCell<Vec<Box<dyn Fn()>>>>;

/// Callback receiving the updated filter entries.
type FilterCallback = Rc<RefCell<Box<dyn Fn(Vec<FilterEntry>)>>>;

/// Editable list of filter entries with add/remove/reorder operations.
pub struct FilterList {
    container: gtk::Box,
}

impl FilterList {
    /// Create a new filter list.
    ///
    /// `is_shell` selects shell-glob validation (filename filters) versus
    /// regex validation (text filters).  `on_changed` receives the updated
    /// entries in display order after every change.
    ///
    /// `flush_sink` (optional) receives a closure that commits any pending
    /// edits; the preferences dialog invokes it when closing so that text
    /// typed without pressing Enter is not lost.
    pub fn new(
        entries: &[FilterEntry],
        is_shell: bool,
        on_changed: impl Fn(Vec<FilterEntry>) + 'static,
        flush_sink: Option<&FlushSink>,
    ) -> Self {
        let on_changed = Rc::new(RefCell::new(
            Box::new(on_changed) as Box<dyn Fn(Vec<FilterEntry>)>
        ));
        let rows: Rc<RefCell<Vec<RowData>>> = Rc::new(RefCell::new(Vec::new()));

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.add_css_class("rich-list");

        for entry in entries {
            let row = build_row(entry, is_shell, &rows, &on_changed);
            list.append(&row.widget);
            rows.borrow_mut().push(row);
        }

        // Toolbar with Add / Remove / Move Up / Move Down, matching the
        // original `filter-list.ui` layout.
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        toolbar.add_css_class("toolbar");
        toolbar.set_valign(gtk::Align::Center);
        toolbar.set_margin_top(6);

        let add = gtk::Button::with_label("_Add");
        add.set_use_underline(true);
        add.set_icon_name("list-add-symbolic");
        add.set_tooltip_text(Some("Add new filter"));
        let remove = gtk::Button::with_label("_Remove");
        remove.set_use_underline(true);
        remove.set_icon_name("list-remove-symbolic");
        remove.set_tooltip_text(Some("Remove selected filter"));
        let separator = gtk::Separator::new(gtk::Orientation::Vertical);
        let move_up = gtk::Button::with_label("Move _Up");
        move_up.set_use_underline(true);
        move_up.set_icon_name("go-up-symbolic");
        move_up.set_tooltip_text(Some("Move item up"));
        let move_down = gtk::Button::with_label("Move _Down");
        move_down.set_use_underline(true);
        move_down.set_icon_name("go-down-symbolic");
        move_down.set_tooltip_text(Some("Move item down"));
        toolbar.append(&add);
        toolbar.append(&remove);
        toolbar.append(&separator);
        toolbar.append(&move_up);
        toolbar.append(&move_down);
        let buttons = Rc::new(ToolbarButtons {
            remove,
            move_up,
            move_down,
        });

        // Add: appends the default entry (mirrors `EditableListWidget.add_entry`).
        {
            let list = list.clone();
            let rows = Rc::clone(&rows);
            let on_changed = Rc::clone(&on_changed);
            let buttons = Rc::clone(&buttons);
            add.connect_clicked(move |_| {
                let entry = FilterEntry {
                    name: "label".into(),
                    enabled: false,
                    pattern: "pattern".into(),
                    is_shell,
                };
                let row = build_row(&entry, is_shell, &rows, &on_changed);
                list.append(&row.widget);
                rows.borrow_mut().push(row);
                emit(&rows, is_shell, &on_changed);
                update_sensitivity(&list, &rows, &buttons);
            });
        }

        // Remove / Move Up / Move Down operate on the selected row.
        {
            let list = list.clone();
            let rows = Rc::clone(&rows);
            let on_changed = Rc::clone(&on_changed);
            let buttons = Rc::clone(&buttons);
            let remove = buttons.remove.clone();
            remove.connect_clicked(move |_| {
                let Some(index) = selected_index(&list) else {
                    return;
                };
                let data = rows.borrow_mut().remove(index);
                list.remove(&data.widget);
                emit(&rows, is_shell, &on_changed);
                update_sensitivity(&list, &rows, &buttons);
            });
        }
        {
            let list = list.clone();
            let rows = Rc::clone(&rows);
            let on_changed = Rc::clone(&on_changed);
            let buttons = Rc::clone(&buttons);
            let move_up = buttons.move_up.clone();
            move_up.connect_clicked(move |_| {
                let Some(index) = selected_index(&list) else {
                    return;
                };
                if index == 0 {
                    return;
                }
                {
                    let mut rows = rows.borrow_mut();
                    let data = rows.remove(index);
                    list.remove(&data.widget);
                    list.insert(&data.widget, index as i32 - 1);
                    rows.insert(index - 1, data);
                }
                emit(&rows, is_shell, &on_changed);
                update_sensitivity(&list, &rows, &buttons);
            });
        }
        {
            let list = list.clone();
            let rows = Rc::clone(&rows);
            let on_changed = Rc::clone(&on_changed);
            let buttons = Rc::clone(&buttons);
            let move_down = buttons.move_down.clone();
            move_down.connect_clicked(move |_| {
                let Some(index) = selected_index(&list) else {
                    return;
                };
                if index + 1 >= rows.borrow().len() {
                    return;
                }
                {
                    let mut rows = rows.borrow_mut();
                    let data = rows.remove(index);
                    list.remove(&data.widget);
                    list.insert(&data.widget, index as i32 + 1);
                    rows.insert(index + 1, data);
                }
                emit(&rows, is_shell, &on_changed);
                update_sensitivity(&list, &rows, &buttons);
            });
        }

        // Button sensitivity mirrors `EditableListWidget.setup_sensitivity_handling`.
        {
            let list2 = Rc::new(list.clone());
            let rows = Rc::clone(&rows);
            let buttons = Rc::clone(&buttons);
            let list_cb = Rc::clone(&list2);
            list2.connect_row_selected(move |_, _| {
                update_sensitivity(&list_cb, &rows, &buttons);
            });
        }
        {
            let list2 = Rc::new(list.clone());
            let rows = Rc::clone(&rows);
            let buttons = Rc::clone(&buttons);
            let list_cb = Rc::clone(&list2);
            list2.connect_row_activated(move |_, _| {
                update_sensitivity(&list_cb, &rows, &buttons);
            });
        }
        update_sensitivity(&list, &rows, &buttons);

        // Register a flush callback so uncommitted text edits are saved
        // when the dialog closes.
        if let Some(flushes) = flush_sink {
            let rows = Rc::clone(&rows);
            let on_changed = Rc::clone(&on_changed);
            flushes.borrow_mut().push(Box::new(move || {
                emit(&rows, is_shell, &on_changed);
            }));
        }

        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);

        // Column headers matching the original three-column layout.
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.set_margin_start(4);
        header.set_margin_end(4);
        header.set_margin_top(2);
        header.set_margin_bottom(2);
        let active_label = gtk::Label::new(Some("Active"));
        active_label.set_xalign(0.0);
        active_label.set_width_request(48);
        let name_label = gtk::Label::new(Some("Name"));
        name_label.set_xalign(0.0);
        name_label.set_hexpand(true);
        let pattern_label = gtk::Label::new(Some("Pattern"));
        pattern_label.set_xalign(0.0);
        pattern_label.set_hexpand(true);
        header.append(&active_label);
        header.append(&name_label);
        header.append(&pattern_label);
        container.append(&header);

        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_min_content_height(120);
        scrolled.set_vexpand(true);
        scrolled.set_focusable(true);
        scrolled.set_child(Some(&list));
        container.append(&scrolled);
        container.append(&toolbar);

        Self { container }
    }

    /// Whether `pattern` is a valid shell glob (or regex, when `is_shell`
    /// is false), mirroring the original `FilterEntry.check_filter`.
    fn is_valid(pattern: &str, is_shell: bool) -> bool {
        if is_shell {
            // An empty pattern would match everything, so it is invalid.
            pattern.split_whitespace().next().is_some()
        } else {
            regex::Regex::new(pattern).is_ok()
        }
    }

    /// The top-level widget of this filter list.
    pub fn widget(&self) -> gtk::Box {
        self.container.clone()
    }
}

/// Build one editable row (without appending it anywhere).
fn build_row(
    entry: &FilterEntry,
    is_shell: bool,
    rows: &Rc<RefCell<Vec<RowData>>>,
    on_changed: &FilterCallback,
) -> RowData {
    // Invalid patterns are loaded inactive (mirrors the original's
    // `FilterEntry.new_from_gsetting`).
    let valid = FilterList::is_valid(&entry.pattern, is_shell);

    let widget = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    widget.set_margin_top(4);
    widget.set_margin_bottom(4);
    widget.set_margin_start(4);
    widget.set_margin_end(4);

    let check = gtk::CheckButton::new();
    check.set_active(entry.enabled && valid);
    check.set_sensitive(valid);
    widget.append(&check);

    let name = gtk::Entry::new();
    name.set_text(&entry.name);
    name.set_hexpand(true);
    widget.append(&name);

    let icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
    icon.set_visible(!valid);
    icon.set_tooltip_text(Some("Invalid filter pattern"));
    widget.append(&icon);

    let pattern = gtk::Entry::new();
    pattern.set_text(&entry.pattern);
    pattern.set_hexpand(true);
    widget.append(&pattern);

    // The toggle commits immediately; name and pattern edits commit on
    // Enter or focus loss, mirroring the original's cell "edited" semantics.
    {
        let rows = Rc::clone(rows);
        let on_changed = Rc::clone(on_changed);
        check.connect_toggled(move |_| emit(&rows, is_shell, &on_changed));
    }
    for entry_widget in [&name, &pattern] {
        let rows2 = Rc::clone(rows);
        let oc = Rc::clone(on_changed);
        let is_shell2 = is_shell;
        entry_widget.connect_has_focus_notify(move |entry| {
            if !entry.has_focus() {
                emit(&rows2, is_shell2, &oc);
            }
        });
        let rows2 = Rc::clone(rows);
        let oc = Rc::clone(on_changed);
        let is_shell2 = is_shell;
        entry_widget.connect_activate(move |_| emit(&rows2, is_shell2, &oc));
    }

    RowData {
        check,
        name,
        pattern,
        icon,
        widget,
    }
}

/// Index of the currently selected row, if any.
fn selected_index(list: &gtk::ListBox) -> Option<usize> {
    let selected = list.selected_row()?;
    let mut i = 0;
    while let Some(row) = list.row_at_index(i as i32) {
        if row.as_ptr() == selected.as_ptr() {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Refresh validity indicators and invoke the callback with the current
/// entries (in display order).
fn emit(rows: &Rc<RefCell<Vec<RowData>>>, is_shell: bool, on_changed: &FilterCallback) {
    for data in rows.borrow().iter() {
        let valid = FilterList::is_valid(&data.pattern.text(), is_shell);
        data.icon.set_visible(!valid);
        data.check.set_sensitive(valid);
    }
    let entries: Vec<FilterEntry> = rows
        .borrow()
        .iter()
        .map(|data| FilterEntry {
            name: data.name.text().to_string(),
            enabled: data.check.is_active(),
            pattern: data.pattern.text().to_string(),
            is_shell,
        })
        .collect();
    let cb = on_changed.borrow();
    cb.as_ref()(entries);
}

/// Update the toolbar button sensitivity from the current selection.
fn update_sensitivity(
    list: &gtk::ListBox,
    rows: &Rc<RefCell<Vec<RowData>>>,
    buttons: &Rc<ToolbarButtons>,
) {
    let count = rows.borrow().len();
    let index = selected_index(list);
    buttons.remove.set_sensitive(index.is_some());
    buttons.move_up.set_sensitive(index.is_some_and(|i| i > 0));
    buttons
        .move_down
        .set_sensitive(index.is_some_and(|i| i + 1 < count));
}
