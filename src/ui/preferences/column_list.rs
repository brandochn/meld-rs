#![cfg(feature = "gui")]
//! Visible-columns list — GTK4 replica of the original Meld's `ColumnList`
//! (`column-list.ui` + `columnlist.py`).
//!
//! Each available folder column is an `Adw.SwitchRow` in a boxed
//! `GtkListBox`.  Rows can be reordered by dragging their handle or via
//! the per-row menu (Move Up / Move Down), and the resulting
//! `(name, visible)` pairs are persisted in display order.

use gdk4 as gdk;
use gio::prelude::*;
use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

/// (id, label) for each available folder column, in default display order.
const AVAILABLE_COLUMNS: &[(&str, &str)] = &[
    ("size", "Size"),
    ("modification time", "Modification time"),
    ("iso-time", "Modification time (ISO)"),
    ("permissions", "Permissions"),
];

type ActionsMap = Rc<RefCell<HashMap<String, (gio::SimpleAction, gio::SimpleAction)>>>;
type WeakActions = Weak<RefCell<HashMap<String, (gio::SimpleAction, gio::SimpleAction)>>>;
type ColumnsCallback = Rc<RefCell<Box<dyn Fn(Vec<(String, bool)>)>>>;
/// (id, row) in display order — the source of truth for ordering, kept in
/// sync with the `GtkListBox` by every mutation.
type Rows = Rc<RefCell<Vec<(String, adw::SwitchRow)>>>;
type WeakRows = Weak<RefCell<Vec<(String, adw::SwitchRow)>>>;

/// Reorderable list of folder columns with visibility switches.
pub struct ColumnList {
    list: gtk::ListBox,
    /// Keeps the rows bookkeeping alive for the row handlers' weak refs.
    /// The dialog stores this struct in its `keep_alive` sink.
    #[allow(dead_code)]
    rows: Rows,
}

impl ColumnList {
    /// Create a new column list.
    ///
    /// `saved` holds the stored `(name, visible)` pairs; columns are shown
    /// in the saved order, with any missing available columns appended.
    /// `on_changed` receives the updated pairs after every change.
    pub fn new(
        saved: &[(String, bool)],
        on_changed: impl Fn(Vec<(String, bool)>) + 'static,
    ) -> Self {
        let on_changed = Rc::new(RefCell::new(
            Box::new(on_changed) as Box<dyn Fn(Vec<(String, bool)>)>
        ));
        let actions: ActionsMap = Rc::new(RefCell::new(HashMap::new()));
        let rows: Rows = Rc::new(RefCell::new(Vec::new()));

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("boxed-list");

        // Order the available columns by their saved position; unknown or
        // missing columns fall to the end (mirrors the original ColumnList).
        let order: HashMap<&str, usize> = saved
            .iter()
            .enumerate()
            .map(|(i, (name, _))| (name.as_str(), i))
            .collect();
        let mut ordered: Vec<(&str, &str, bool)> = AVAILABLE_COLUMNS
            .iter()
            .map(|(id, label)| {
                let active = saved
                    .iter()
                    .find(|(name, _)| name == id)
                    .map(|(_, active)| *active)
                    .unwrap_or(false);
                (*id, *label, active)
            })
            .collect();
        ordered.sort_by_key(|(id, _, _)| order.get(id).copied().unwrap_or(usize::MAX));

        for (id, label, active) in ordered {
            let row = create_row(id, label, active, &list, &on_changed, &actions, &rows);
            list.append(&row);
            rows.borrow_mut().push((id.to_string(), row));
        }
        update_action_states(&rows, &actions);

        Self { list, rows }
    }

    /// The top-level widget of this column list.
    pub fn widget(&self) -> gtk::ListBox {
        self.list.clone()
    }
}

/// Build one switch row with drag handle and per-row move menu.
fn create_row(
    id: &str,
    label: &str,
    active: bool,
    list: &gtk::ListBox,
    on_changed: &ColumnsCallback,
    actions: &ActionsMap,
    rows: &Rows,
) -> adw::SwitchRow {
    let row = adw::SwitchRow::new();
    row.set_title(label);
    row.set_active(active);

    // Drag handle for reordering rows.
    let handle = gtk::Image::from_icon_name("list-drag-handle-symbolic");
    row.add_prefix(&handle);

    // Per-row menu actions (Move Up / Move Down).
    let group = gio::SimpleActionGroup::new();
    let move_up = gio::SimpleAction::new("move-up", None);
    let move_down = gio::SimpleAction::new("move-down", None);
    group.add_action(&move_up);
    group.add_action(&move_down);
    actions
        .borrow_mut()
        .insert(id.to_string(), (move_up.clone(), move_down.clone()));

    let menu_button = gtk::MenuButton::new();
    menu_button.set_icon_name("view-more-symbolic");
    menu_button.add_css_class("flat");
    menu_button.set_valign(gtk::Align::Center);
    menu_button.set_tooltip_text(Some("Column options"));
    menu_button.insert_action_group("row", Some(&group));
    let menu = gio::Menu::new();
    menu.append(Some("Move Up"), Some("row.move-up"));
    menu.append(Some("Move Down"), Some("row.move-down"));
    menu_button.set_menu_model(Some(&menu));
    row.add_suffix(&menu_button);

    // Persist whenever the visibility switch flips.
    {
        let rows_weak = Rc::downgrade(rows);
        let oc = Rc::clone(on_changed);
        row.connect_active_notify(move |_| {
            emit_columns(&rows_weak, &oc);
        });
    }

    // Drag source on the handle.
    {
        let row_weak = row.downgrade();
        let source = gtk::DragSource::new();
        source.set_actions(gdk::DragAction::MOVE);
        let row_weak2 = row_weak.clone();
        source.connect_prepare(move |_source, _x, _y| {
            row_weak2
                .upgrade()
                .map(|row| gdk::ContentProvider::for_value(&row.to_value()))
        });
        source.connect_drag_begin(move |source, _drag| {
            if let Some(row) = row_weak.upgrade() {
                let paintable = gtk::WidgetPaintable::new(Some(&row));
                source.set_icon(Some(&paintable), 0, 0);
            }
        });
        handle.add_controller(source);
    }

    // Drop target on the row itself.
    {
        let list_weak = list.downgrade();
        let row_weak = row.downgrade();
        let oc = Rc::clone(on_changed);
        let actions_weak = Rc::downgrade(actions);
        let rows_weak = Rc::downgrade(rows);
        let drop_target = gtk::DropTarget::new(gtk::Widget::static_type(), gdk::DragAction::MOVE);
        drop_target.connect_drop(move |_target, value, _x, _y| {
            let (Some(list), Some(target_row)) = (list_weak.upgrade(), row_weak.upgrade()) else {
                return false;
            };
            let Ok(source) = value.get::<gtk::Widget>() else {
                return false;
            };
            let Some(source_row) = source.downcast_ref::<adw::SwitchRow>().cloned() else {
                return false;
            };
            move_row(
                &list,
                &source_row,
                target_row.index(),
                &oc,
                &actions_weak,
                &rows_weak,
            );
            true
        });
        row.add_controller(drop_target);
    }

    // Menu actions move this row.
    {
        let list_weak = list.downgrade();
        let row_weak = row.downgrade();
        let oc = Rc::clone(on_changed);
        let actions_weak = Rc::downgrade(actions);
        let rows_weak = Rc::downgrade(rows);
        move_up.connect_activate(move |_, _| {
            let (Some(list), Some(row)) = (list_weak.upgrade(), row_weak.upgrade()) else {
                return;
            };
            let index = row_position(&rows_weak, &row);
            move_row(&list, &row, index - 1, &oc, &actions_weak, &rows_weak);
        });
    }
    {
        let list_weak = list.downgrade();
        let row_weak = row.downgrade();
        let oc = Rc::clone(on_changed);
        let actions_weak = Rc::downgrade(actions);
        let rows_weak = Rc::downgrade(rows);
        move_down.connect_activate(move |_, _| {
            let (Some(list), Some(row)) = (list_weak.upgrade(), row_weak.upgrade()) else {
                return;
            };
            let index = row_position(&rows_weak, &row);
            move_row(&list, &row, index + 1, &oc, &actions_weak, &rows_weak);
        });
    }

    row
}

/// Move `source` to `target_index` (clamped) in both the bookkeeping
/// vector and the `GtkListBox`, then persist and refresh action states.
fn move_row(
    list: &gtk::ListBox,
    source: &adw::SwitchRow,
    target_index: i32,
    on_changed: &ColumnsCallback,
    actions: &WeakActions,
    rows: &WeakRows,
) {
    let Some(rows) = rows.upgrade() else {
        return;
    };
    let count = rows.borrow().len() as i32;
    if count == 0 {
        return;
    }
    let target = target_index.clamp(0, count - 1);
    let current = rows
        .borrow()
        .iter()
        .position(|(_, row)| row.as_ptr() == source.as_ptr());
    let Some(current) = current else {
        return;
    };
    if current as i32 == target {
        return;
    }
    {
        let mut rows = rows.borrow_mut();
        let (id, row) = rows.remove(current);
        rows.insert(target as usize, (id, row));
    }
    list.remove(source);
    list.insert(source, target);
    if let Some(actions) = actions.upgrade() {
        update_action_states(&rows, &actions);
    }
    emit_columns(&Rc::downgrade(&rows), on_changed);
}

/// Position of `target` in the rows vector, or -1.
fn row_position(rows: &WeakRows, target: &adw::SwitchRow) -> i32 {
    let Some(rows) = rows.upgrade() else {
        return -1;
    };
    let rows = rows.borrow();
    match rows
        .iter()
        .position(|(_, row)| row.as_ptr() == target.as_ptr())
    {
        Some(i) => i as i32,
        None => -1,
    }
}

/// Enable the per-row Move Up / Move Down actions based on row position.
fn update_action_states(rows: &Rows, actions: &ActionsMap) {
    let actions = actions.borrow();
    let count = rows.borrow().len();
    for (index, (id, _)) in rows.borrow().iter().enumerate() {
        if let Some((move_up, move_down)) = actions.get(id) {
            move_up.set_enabled(index > 0);
            move_down.set_enabled(index + 1 < count);
        }
    }
}

/// Collect `(name, visible)` pairs in display order and invoke the callback.
fn emit_columns(rows: &WeakRows, on_changed: &ColumnsCallback) {
    let Some(rows) = rows.upgrade() else {
        return;
    };
    let columns: Vec<(String, bool)> = rows
        .borrow()
        .iter()
        .map(|(id, row)| (id.clone(), row.is_active()))
        .collect();
    let cb = on_changed.borrow();
    cb.as_ref()(columns);
}
