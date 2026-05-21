#![cfg(feature = "gui")]
//! New diff tab — start screen with File/Folder/VC toggle buttons.
//!
//! Uses `gtk::FileDialog` (GTK 4.10+) for native file/folder selection.
//! An embedded `GtkEntry` displays the selected path and acts as a
//! fallback for manual typing when schemas are unavailable.

use gio::prelude::*;
use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use crate::window::{DiffRequest, MeldPage};

/// Type of comparison selected by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiffType {
    #[default]
    Unselected,
    File,
    Folder,
    VersionControl,
}

impl DiffType {
    /// Returns `true` if blank comparison is supported for this type.
    pub fn supports_blank(self) -> bool {
        matches!(self, Self::File | Self::Folder)
    }
}

pub struct NewDiffTab {
    container: gtk::Box,
    button_type_file: gtk::ToggleButton,
    button_type_dir: gtk::ToggleButton,
    button_type_vc: gtk::ToggleButton,
    choosers_notebook: gtk::Notebook,
    button_compare: gtk::Button,
    button_new_blank: gtk::Button,
    diff_type: Rc<Cell<DiffType>>,
    /// Guard against re-entrant toggle signals.
    toggling: Rc<Cell<bool>>,
    /// Path entry fields (3 for file, 3 for folder, 1 for VC).
    file_entries: [gtk::Entry; 3],
    dir_entries: [gtk::Entry; 3],
    vc_entry: gtk::Entry,
    /// Callback fired on Compare / Blank click.
    on_diff_created: Rc<RefCell<Option<crate::window::DiffCreatedCallback>>>,
}

impl NewDiffTab {
    pub fn new() -> Self {
        let alignment = gtk::Box::new(gtk::Orientation::Vertical, 0);
        alignment.set_valign(gtk::Align::Start);
        alignment.set_margin_top(40);

        let root_box = gtk::Box::new(gtk::Orientation::Vertical, 18);
        root_box.set_halign(gtk::Align::Center);
        root_box.set_margin_start(24);
        root_box.set_margin_end(24);
        root_box.set_margin_top(12);
        root_box.set_margin_bottom(12);
        root_box.set_width_request(620);

        // ── Title ──
        let title_label = gtk::Label::new(Some("New comparison"));
        title_label.set_xalign(0.0);
        title_label.add_css_class("new-diff-title");
        root_box.append(&title_label);

        let middle_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
        middle_box.set_vexpand(true);

        // ── Toggle buttons ──
        let button_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        button_row.set_homogeneous(true);
        let button_file = build_type_toggle("document-new-symbolic", "File");
        let button_dir = build_type_toggle("folder-new-symbolic", "Folder");
        let button_vc = build_type_toggle("appointment-new-symbolic", "Version control");
        button_row.append(&button_file);
        button_row.append(&button_dir);
        button_row.append(&button_vc);
        middle_box.append(&button_row);

        // ── Choosers notebook ──
        let choosers_notebook = gtk::Notebook::new();
        choosers_notebook.set_show_tabs(false);
        choosers_notebook.set_show_border(false);

        // Page 0: placeholder
        let pp = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        pp.set_homogeneous(true);
        for _ in 0..3 {
            pp.append(&gtk::Label::new(None));
        }
        choosers_notebook.append_page(&pp, Some(&gtk::Label::new(None)));

        // Page 1: File entries (3 rows)
        let (file_grid, file_entries) = build_entry_grid(
            "Select First File",
            "Select Second File",
            "Select Third File",
            false,
        );
        choosers_notebook.append_page(&file_grid, Some(&gtk::Label::new(None)));

        // Page 2: Folder entries (3 rows)
        let (dir_grid, dir_entries) = build_entry_grid(
            "Select First Folder",
            "Select Second Folder",
            "Select Third Folder",
            true,
        );
        choosers_notebook.append_page(&dir_grid, Some(&gtk::Label::new(None)));

        // Page 3: VC entry (1 row)
        let vc_entry = gtk::Entry::new();
        vc_entry.set_placeholder_text(Some("Select a Version-Controlled Folder"));
        vc_entry.set_hexpand(true);
        let vc_row = build_chooser_row_from_entry(&vc_entry, true);
        let vc_grid = gtk::Grid::new();
        vc_grid.set_row_spacing(6);
        vc_grid.set_column_spacing(12);
        vc_grid.set_column_homogeneous(true);
        vc_grid.attach(&vc_row, 0, 0, 1, 1);
        choosers_notebook.append_page(&vc_grid, Some(&gtk::Label::new(None)));

        middle_box.append(&choosers_notebook);
        root_box.append(&middle_box);

        // ── Action buttons ──
        let button_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        button_box.set_halign(gtk::Align::End);
        button_box.set_margin_top(6);
        let button_blank = gtk::Button::with_label("_Blank comparison");
        button_blank.set_use_underline(true);
        button_blank.set_sensitive(false);
        button_box.append(&button_blank);
        let button_compare = gtk::Button::with_label("C_ompare");
        button_compare.set_use_underline(true);
        button_compare.add_css_class("suggested-action");
        button_compare.set_sensitive(false);
        button_box.append(&button_compare);
        root_box.append(&button_box);
        alignment.append(&root_box);

        let toggling = Rc::new(Cell::new(false));
        let diff_type = Rc::new(Cell::new(DiffType::Unselected));
        let on_diff_created: Rc<RefCell<Option<crate::window::DiffCreatedCallback>>> =
            Rc::new(RefCell::new(None));

        let this = Self {
            container: alignment,
            button_type_file: button_file,
            button_type_dir: button_dir,
            button_type_vc: button_vc,
            choosers_notebook,
            button_compare,
            button_new_blank: button_blank,
            diff_type,
            toggling,
            on_diff_created,
            file_entries: [
                file_entries[0].clone(),
                file_entries[1].clone(),
                file_entries[2].clone(),
            ],
            dir_entries: [
                dir_entries[0].clone(),
                dir_entries[1].clone(),
                dir_entries[2].clone(),
            ],
            vc_entry,
        };

        Self::connect_toggle_signals(&this);
        Self::connect_action_buttons(&this);
        this.choosers_notebook.set_current_page(Some(0));
        this
    }

    // ── Toggle signals ──────────────────────────────────────────

    fn connect_toggle_signals(this: &Self) {
        let btns: Vec<gtk::ToggleButton> = vec![
            this.button_type_file.clone(),
            this.button_type_dir.clone(),
            this.button_type_vc.clone(),
        ];
        let nb = this.choosers_notebook.clone();
        let bl = this.button_new_blank.clone();
        let cp = this.button_compare.clone();
        let st = Rc::clone(&this.diff_type);
        let guard = Rc::clone(&this.toggling);

        for (idx, btn) in [
            &this.button_type_file,
            &this.button_type_dir,
            &this.button_type_vc,
        ]
        .iter()
        .enumerate()
        {
            let btns2 = btns.clone();
            let nb2 = nb.clone();
            let bl2 = bl.clone();
            let cp2 = cp.clone();
            let st2 = Rc::clone(&st);
            let guard2 = Rc::clone(&guard);
            btn.connect_toggled(move |b| {
                if guard2.get() {
                    return;
                }
                guard2.set(true);

                if b.is_active() {
                    for (j, other) in btns2.iter().enumerate() {
                        if j != idx {
                            other.set_active(false);
                        }
                    }
                    nb2.set_current_page(Some(idx as u32 + 1));
                    let dt = match idx {
                        0 => DiffType::File,
                        1 => DiffType::Folder,
                        _ => DiffType::VersionControl,
                    };
                    bl2.set_sensitive(dt.supports_blank());
                    cp2.set_sensitive(true);
                    st2.set(dt);
                } else if btns2.iter().all(|b| !b.is_active()) {
                    b.set_active(true);
                }

                guard2.set(false);
            });
        }
    }

    // ── Compare / Blank ─────────────────────────────────────────

    fn connect_action_buttons(this: &Self) {
        let dt = Rc::clone(&this.diff_type);
        let on_cb = this.on_diff_created.clone();
        let fe = this.file_entries.clone();
        let de = this.dir_entries.clone();
        let ve = this.vc_entry.clone();

        // Compare
        this.button_compare.connect_clicked(move |_| {
            let typ = dt.get();
            let paths: Vec<Option<PathBuf>> = match typ {
                DiffType::File => fe.iter().map(|e| entry_path(e)).collect(),
                DiffType::Folder => de.iter().map(|e| entry_path(e)).collect(),
                DiffType::VersionControl => {
                    let p = entry_path(&ve);
                    if p.is_none() {
                        vec![]
                    } else {
                        vec![p]
                    }
                }
                DiffType::Unselected => return,
            };
            if let Some(ref cb) = *on_cb.borrow() {
                cb(DiffRequest {
                    diff_type: typ,
                    paths,
                });
            }
        });

        // Blank comparison
        let dt2 = Rc::clone(&this.diff_type);
        let on_cb2 = this.on_diff_created.clone();
        this.button_new_blank.connect_clicked(move |_| {
            let typ = dt2.get();
            let blank_paths: Vec<Option<PathBuf>> = match typ {
                DiffType::File | DiffType::Folder => vec![None, None, None],
                _ => return,
            };
            if let Some(ref cb) = *on_cb2.borrow() {
                cb(DiffRequest {
                    diff_type: typ,
                    paths: blank_paths,
                });
            }
        });
    }
}

impl MeldPage for NewDiffTab {
    fn widget(&self) -> &gtk::Widget {
        self.container.upcast_ref()
    }
    fn close(&self) -> gtk::ResponseType {
        gtk::ResponseType::Ok
    }
    fn label(&self) -> String {
        "New comparison".into()
    }

    fn set_diff_created_callback(&self, cb: crate::window::DiffCreatedCallback) {
        self.on_diff_created.replace(Some(cb));
    }
}

// ── Widget builders ───────────────────────────────────────────────

/// Build a type-toggle button where child widgets do NOT intercept clicks.
fn build_type_toggle(icon_name: &str, label_text: &str) -> gtk::ToggleButton {
    let btn = gtk::ToggleButton::new();
    btn.add_css_class("new-diff-button");
    btn.set_can_focus(true);

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 6);
    vbox.set_halign(gtk::Align::Center);
    vbox.set_valign(gtk::Align::Center);
    vbox.set_sensitive(false);
    vbox.set_can_target(false);

    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_pixel_size(48);
    icon.set_halign(gtk::Align::Center);
    icon.set_sensitive(false);
    icon.set_can_target(false);
    vbox.append(&icon);

    let label = gtk::Label::new(Some(label_text));
    label.set_halign(gtk::Align::Center);
    label.set_sensitive(false);
    label.set_can_target(false);
    vbox.append(&label);

    btn.set_child(Some(&vbox));
    btn
}

/// Build a grid with 3 chooser rows (entry + browse button).
fn build_entry_grid(
    p0: &str,
    p1: &str,
    p2: &str,
    select_folder: bool,
) -> (gtk::Grid, [gtk::Entry; 3]) {
    let grid = gtk::Grid::new();
    grid.set_row_spacing(6);
    grid.set_column_spacing(12);
    grid.set_column_homogeneous(true);

    let e0 = gtk::Entry::new();
    e0.set_placeholder_text(Some(p0));
    e0.set_hexpand(true);
    let row0 = build_chooser_row_from_entry(&e0, select_folder);
    grid.attach(&row0, 0, 0, 1, 1);

    let e1 = gtk::Entry::new();
    e1.set_placeholder_text(Some(p1));
    e1.set_hexpand(true);
    let row1 = build_chooser_row_from_entry(&e1, select_folder);
    grid.attach(&row1, 1, 0, 1, 1);

    let e2 = gtk::Entry::new();
    e2.set_placeholder_text(Some(p2));
    e2.set_hexpand(true);
    e2.set_sensitive(false);
    let row2 = build_chooser_row_from_entry(&e2, select_folder);
    grid.attach(&row2, 2, 0, 1, 1);

    (grid, [e0, e1, e2])
}

/// Build a chooser row: [Entry (path)] [Browse button].
///
/// The Browse button opens a native `gtk::FileDialog` (GTK 4.10+).
/// The selected path is written into the entry field.
/// If schemas are unavailable at runtime, the entry still works for
/// manual path typing.
fn build_chooser_row_from_entry(entry: &gtk::Entry, select_folder: bool) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    row.append(entry);

    let browse_btn = gtk::Button::from_icon_name("folder-open-symbolic");
    browse_btn.set_tooltip_text(Some(if select_folder {
        "Select folder"
    } else {
        "Select file"
    }));

    let entry_weak = entry.downgrade();
    let sf = select_folder;

    browse_btn.connect_clicked(move |btn| {
        let dialog = gtk::FileDialog::builder().modal(true).build();
        dialog.set_title(if sf { "Select Folder" } else { "Select File" });

        let entry_w = entry_weak.clone();
        let window = btn.root().and_then(|r| r.downcast::<gtk::Window>().ok());

        glib::spawn_future_local(async move {
            let result = if sf {
                dialog.select_folder_future(window.as_ref()).await
            } else {
                dialog.open_future(window.as_ref()).await
            };

            match result {
                Ok(file) => {
                    if let Some(path) = file.path() {
                        if let Some(e) = entry_w.upgrade() {
                            e.set_text(&path.to_string_lossy().to_string());
                        }
                    }
                }
                Err(err) => {
                    log::warn!("FileDialog error: {err}");
                }
            }
        });
    });

    row.append(&browse_btn);
    row
}

/// Extract an optional path from an entry widget.
fn entry_path(entry: &gtk::Entry) -> Option<PathBuf> {
    let text = entry.text().to_string();
    if text.is_empty() {
        None
    } else {
        Some(PathBuf::from(&text))
    }
}
