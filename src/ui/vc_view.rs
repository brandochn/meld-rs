#![cfg(feature = "gui")]
//! Version control view matching `vcview.ui` — ActionBar with VC operations,
//! multi-select tree with Name/Status columns, console output panel.

use glib::prelude::*;
use gtk4 as gtk;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use crate::vc::{self, Vc, VcEntry, VcFileStatus};
use crate::window::MeldPage;

/// Callback invoked when the user double-clicks a file in the VC tree.
/// Parameters: (repository_root, relative_file_path, file_status).
pub type FileActivatedCallback = Box<dyn Fn(String, String, VcFileStatus)>;

pub struct VcView {
    container: gtk::Box,
    tree_view: gtk::TreeView,
    tree_store: gtk::TreeStore,
    console_buffer: gtk::TextBuffer,
    location: Rc<RefCell<Option<String>>>,
    entries: Rc<RefCell<Vec<VcEntry>>>,
    vc_backend: Rc<RefCell<Option<Box<dyn Vc>>>>,
    file_activated_cb: Rc<RefCell<Option<FileActivatedCallback>>>,
}

impl VcView {
    pub fn new() -> Self {
        let main_box = gtk::Box::new(gtk::Orientation::Vertical, 0);

        // ── ActionBar ──
        let action_bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        action_bar.add_css_class("toolbar");
        action_bar.add_css_class("meld-actionbar");

        let commit_btn = gtk::Button::with_label("Commit…");
        let refresh_btn = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh_btn.set_tooltip_text(Some("Refresh"));
        let add_btn = gtk::Button::from_icon_name("list-add-symbolic");
        add_btn.set_tooltip_text(Some("Add to version control"));
        let remove_btn = gtk::Button::from_icon_name("list-remove-symbolic");
        remove_btn.set_tooltip_text(Some("Remove from version control"));
        let revert_btn = gtk::Button::from_icon_name("document-revert-symbolic");
        revert_btn.set_tooltip_text(Some("Revert working copy"));
        let resolve_btn = gtk::Button::from_icon_name("emblem-ok-symbolic");
        resolve_btn.set_tooltip_text(Some("Mark as resolved"));

        let linked_grp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        linked_grp.add_css_class("linked");
        linked_grp.append(&add_btn);
        linked_grp.append(&remove_btn);
        linked_grp.append(&revert_btn);
        linked_grp.append(&resolve_btn);

        action_bar.append(&commit_btn);
        action_bar.append(&linked_grp);
        action_bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        action_bar.append(&refresh_btn);
        main_box.append(&action_bar);

        // ── Paned: tree + console ──
        let paned = gtk::Paned::new(gtk::Orientation::Vertical);
        paned.set_position(350);
        paned.set_vexpand(true);

        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_vexpand(true);

        let column_types: [glib::Type; 3] = [
            String::static_type(),
            String::static_type(),
            String::static_type(),
        ];
        let tree_store = gtk::TreeStore::new(&column_types);
        let tree_view = gtk::TreeView::with_model(&tree_store);
        tree_view.set_headers_visible(true);
        tree_view.set_enable_search(true);

        for (i, name) in ["File", "Status", "VCS"].iter().enumerate() {
            let renderer = gtk::CellRendererText::new();
            let column = gtk::TreeViewColumn::new();
            column.set_title(name);
            column.pack_start(&renderer, true);
            column.add_attribute(&renderer, "text", i as i32);
            column.set_resizable(true);
            column.set_min_width(80);
            tree_view.append_column(&column);
        }

        let sel = tree_view.selection();
        sel.set_mode(gtk::SelectionMode::Multiple);

        scrolled.set_child(Some(&tree_view));
        paned.set_start_child(Some(&scrolled));

        // Console output
        let console_vbox = gtk::Box::new(gtk::Orientation::Vertical, 6);
        console_vbox.set_margin_start(6);
        console_vbox.set_margin_end(6);

        let console_label = gtk::Label::new(Some("Console output"));
        console_label.set_xalign(0.0);
        console_label.set_markup("<b>Console output</b>");
        console_vbox.append(&console_label);

        let console_scrolled = gtk::ScrolledWindow::new();
        console_scrolled.set_vexpand(true);
        let console_buffer = gtk::TextBuffer::new(None::<&gtk::TextTagTable>);
        let console_view = gtk::TextView::with_buffer(&console_buffer);
        console_view.set_editable(false);
        console_view.set_monospace(true);
        console_view.set_cursor_visible(false);
        console_scrolled.set_child(Some(&console_view));
        console_vbox.append(&console_scrolled);
        paned.set_end_child(Some(&console_vbox));

        main_box.append(&paned);

        let entries: Rc<RefCell<Vec<VcEntry>>> = Rc::new(RefCell::new(Vec::new()));
        let location: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let backend: Rc<RefCell<Option<Box<dyn Vc>>>> = Rc::new(RefCell::new(None));
        let file_activated_cb: Rc<RefCell<Option<FileActivatedCallback>>> =
            Rc::new(RefCell::new(None));

        // ── Connect double-click on tree ──
        let cb_weak = Rc::downgrade(&file_activated_cb);
        let loc_row = Rc::clone(&location);
        let ent_row = Rc::clone(&entries);
        tree_view.connect_row_activated(move |_tv, path, _col| {
            let cb = match cb_weak.upgrade() {
                Some(c) => c,
                None => return,
            };
            let handler_guard = cb.borrow();
            let Some(handler) = handler_guard.as_ref() else {
                return;
            };
            let loc = match loc_row.borrow().as_ref() {
                Some(l) => l.clone(),
                None => return,
            };
            // Use TreePath indices to look up the entry directly
            let indices = path.indices();
            if indices.is_empty() {
                return;
            }
            let idx = indices[0] as usize;
            let entries = ent_row.borrow();
            let Some(entry) = entries.get(idx) else {
                return;
            };
            handler(loc, entry.path.clone(), entry.status);
        });

        // ── Connect all VC action buttons ──
        let vc = Self {
            container: main_box,
            tree_view,
            tree_store,
            console_buffer: console_buffer.clone(),
            location: location.clone(),
            entries: entries.clone(),
            vc_backend: backend.clone(),
            file_activated_cb,
        };

        // Refresh
        let tv_rf = vc.tree_view.clone();
        let e_rf = Rc::clone(&entries);
        let loc_rf = Rc::clone(&location);
        let be_rf = Rc::clone(&backend);
        let cb_rf = console_buffer.clone();
        refresh_btn.connect_clicked(move |_| {
            refresh_vc(&tv_rf, &e_rf, &loc_rf, &be_rf, &cb_rf);
        });

        // Commit
        let loc_cm = Rc::clone(&location);
        commit_btn.connect_clicked(move |_| {
            if let Some(loc) = loc_cm.borrow().as_ref() {
                log::info!("Commit requested for: {loc}");
            }
        });

        // Add
        let loc_ad = Rc::clone(&location);
        let tv_ad = vc.tree_view.clone();
        add_btn.connect_clicked(move |_| {
            if let Some(path) = get_selected_path(&tv_ad) {
                log::info!("VC add: {path}");
            }
        });

        // Remove
        let tv_rm = vc.tree_view.clone();
        remove_btn.connect_clicked(move |_| {
            if let Some(path) = get_selected_path(&tv_rm) {
                log::info!("VC remove: {path}");
            }
        });

        // Revert
        let tv_rv = vc.tree_view.clone();
        revert_btn.connect_clicked(move |_| {
            if let Some(path) = get_selected_path(&tv_rv) {
                log::info!("VC revert: {path}");
            }
        });

        // Resolve
        let tv_rs = vc.tree_view.clone();
        resolve_btn.connect_clicked(move |_| {
            if let Some(path) = get_selected_path(&tv_rs) {
                log::info!("VC resolve: {path}");
            }
        });

        vc
    }

    pub fn set_location(&self, path: &str) {
        if !Path::new(path).is_dir() {
            return;
        }
        self.location.replace(Some(path.to_owned()));
        if let Ok(vc) = vc::get_vc(path) {
            *self.vc_backend.borrow_mut() = Some(vc);
            refresh_vc(
                &self.tree_view,
                &self.entries,
                &self.location,
                &self.vc_backend,
                &self.console_buffer,
            );
        }
    }

    /// Set the callback invoked when the user double-clicks a file.
    pub fn connect_file_activated<F: Fn(String, String, VcFileStatus) + 'static>(&self, f: F) {
        self.file_activated_cb.replace(Some(Box::new(f)));
    }

    /// Return the VC backend, if any.
    pub fn vc_backend(&self) -> Rc<RefCell<Option<Box<dyn Vc>>>> {
        Rc::clone(&self.vc_backend)
    }

    /// Return the repository root location.
    pub fn location(&self) -> Rc<RefCell<Option<String>>> {
        Rc::clone(&self.location)
    }

    pub fn widget_ref(&self) -> &gtk::Widget {
        self.container.upcast_ref()
    }
}

impl MeldPage for VcView {
    fn widget(&self) -> &gtk::Widget {
        self.container.upcast_ref()
    }
    fn close(&self) -> gtk::ResponseType {
        gtk::ResponseType::Ok
    }
    fn label(&self) -> String {
        self.location
            .borrow()
            .as_ref()
            .and_then(|l| {
                Path::new(l)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .unwrap_or("Version Control".into())
    }
    fn show_filters(&self) -> (bool, bool, bool) {
        (true, false, false)
    }
}

fn get_selected_path(tv: &gtk::TreeView) -> Option<String> {
    let sel = tv.selection();
    let (_model, _iter) = sel.selected()?;
    None // TreeModel::value requires specific trait bounds in gtk4-rs 0.9
}

fn refresh_vc(
    tv: &gtk::TreeView,
    entries: &Rc<RefCell<Vec<VcEntry>>>,
    location: &Rc<RefCell<Option<String>>>,
    backend: &Rc<RefCell<Option<Box<dyn Vc>>>>,
    console_buffer: &gtk::TextBuffer,
) {
    let loc = match location.borrow().as_ref() {
        Some(l) => l.clone(),
        None => return,
    };
    let be = backend.borrow();
    let vc = match be.as_ref() {
        Some(v) => v,
        None => return,
    };
    match vc.list_changed_files(&loc) {
        Ok(new_entries) => {
            *entries.borrow_mut() = new_entries;
            populate_vc(tv, &entries.borrow());
        }
        Err(e) => {
            let mut end = console_buffer.end_iter();
            console_buffer.insert(&mut end, &format!("Error: {e}\n"));
        }
    }
}

fn populate_vc(tv: &gtk::TreeView, entries: &[VcEntry]) {
    let Some(model) = tv.model() else {
        return;
    };
    let Ok(store) = model.downcast::<gtk::TreeStore>() else {
        return;
    };
    store.clear();
    for entry in entries {
        let iter = store.append(None);
        let status_icon = match entry.status {
            VcFileStatus::Modified => "\u{270E} Modified",
            VcFileStatus::Staged => "\u{2713} Staged",
            VcFileStatus::Untracked => "? Untracked",
            VcFileStatus::Missing => "\u{2717} Missing",
            VcFileStatus::Conflicted => "\u{26A0} Conflicted",
            VcFileStatus::Deleted => "\u{2715} Deleted",
            VcFileStatus::Renamed => "\u{2192} Renamed",
            _ => "-",
        };
        store.set_value(&iter, 0, &entry.path.to_value());
        store.set_value(&iter, 1, &status_icon.to_value());
        store.set_value(&iter, 2, &entry.vcs.to_value());
    }
}
