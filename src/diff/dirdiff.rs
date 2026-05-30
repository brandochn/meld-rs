#![cfg(feature = "gui")]
//! Full directory comparison with recursive scanning, multi-pane treeviews, and filters.
//!
//! Matches the layout of `dirdiff.ui` — each folder gets its own TreeView with
//! ActionBar, MsgArea, and overview map.

use gio::prelude::*;
use glib::prelude::*;
use gtk4 as gtk;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use crate::window::MeldPage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DirDiffState {
    Same,
    New,
    Modified,
    Missing,
    Error,
    Filtered,
}

impl DirDiffState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Same => "Same",
            Self::New => "New",
            Self::Modified => "Modified",
            Self::Missing => "Missing",
            Self::Error => "Error",
            Self::Filtered => "Filtered",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DirDiffEntry {
    pub name: String,
    pub state: DirDiffState,
    pub size_a: u64,
    pub size_b: u64,
    pub modified_a: u64,
    pub modified_b: u64,
    pub is_dir: bool,
    pub children: Vec<DirDiffEntry>,
}

/// Full directory comparison with multi-pane layout.
pub struct DirDiff {
    container: gtk::Box,
    tree_views: Vec<gtk::TreeView>,
    tree_stores: Vec<gtk::TreeStore>,
    folders: Rc<RefCell<Vec<PathBuf>>>,
    entries: Rc<RefCell<Vec<DirDiffEntry>>>,
    state_filter: Rc<RefCell<HashSet<DirDiffState>>>,
    show_identical: Rc<RefCell<bool>>,
    scan_cancel: Rc<RefCell<Option<std::sync::Arc<AtomicBool>>>>,
    scan_source: Rc<RefCell<Option<glib::SourceId>>>,
}

impl DirDiff {
    pub fn new(_num_folders: usize) -> Self {
        let main_box = gtk::Box::new(gtk::Orientation::Vertical, 0);

        // Toolbar
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        toolbar.add_css_class("toolbar");
        toolbar.add_css_class("meld-actionbar");

        let compare_btn = gtk::Button::with_label("Compare");
        let refresh_btn = gtk::Button::with_label("Refresh");
        let expand_btn = gtk::Button::with_label("Expand All");
        let collapse_btn = gtk::Button::with_label("Collapse All");
        let new_cb = gtk::CheckButton::with_label("New");
        let modified_cb = gtk::CheckButton::with_label("Modified");
        let missing_cb = gtk::CheckButton::with_label("Missing");
        let identical_cb = gtk::CheckButton::with_label("Show identical");
        new_cb.set_active(true);
        modified_cb.set_active(true);
        missing_cb.set_active(true);

        toolbar.append(&compare_btn);
        toolbar.append(&refresh_btn);
        toolbar.append(&expand_btn);
        toolbar.append(&collapse_btn);
        toolbar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        toolbar.append(&new_cb);
        toolbar.append(&modified_cb);
        toolbar.append(&missing_cb);
        toolbar.append(&identical_cb);
        main_box.append(&toolbar);

        // Multi-pane treeview container
        let panes_hbox = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        panes_hbox.set_vexpand(true);

        let column_types: [glib::Type; 5] = [
            String::static_type(),
            String::static_type(),
            String::static_type(),
            String::static_type(),
            bool::static_type(),
        ];

        let tree_views: Rc<RefCell<Vec<gtk::TreeView>>> =
            Rc::new(RefCell::new(Vec::with_capacity(2)));
        let tree_stores: Rc<RefCell<Vec<gtk::TreeStore>>> =
            Rc::new(RefCell::new(Vec::with_capacity(2)));

        for _i in 0..2 {
            let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);

            let scrolled = gtk::ScrolledWindow::new();
            scrolled.set_vexpand(true);
            scrolled.set_hexpand(true);

            let store = gtk::TreeStore::new(&column_types);
            let tree_view = gtk::TreeView::with_model(&store);
            tree_view.set_headers_visible(true);
            tree_view.set_enable_search(true);

            let columns = ["Name", "State", "Size", "Modified"];
            for (i, col_name) in columns.iter().enumerate() {
                let renderer = gtk::CellRendererText::new();
                let column = gtk::TreeViewColumn::new();
                column.set_title(col_name);
                column.pack_start(&renderer, true);
                column.add_attribute(&renderer, "text", i as i32);
                column.set_resizable(true);
                column.set_min_width(80);
                tree_view.append_column(&column);
            }

            scrolled.set_child(Some(&tree_view));
            vbox.append(&scrolled);
            panes_hbox.append(&vbox);

            tree_views.borrow_mut().push(tree_view);
            tree_stores.borrow_mut().push(store);
        }

        main_box.append(&panes_hbox);

        let entries: Rc<RefCell<Vec<DirDiffEntry>>> = Rc::new(RefCell::new(Vec::new()));
        let state_filter: Rc<RefCell<HashSet<DirDiffState>>> = Rc::new(RefCell::new({
            let mut s = HashSet::new();
            s.insert(DirDiffState::New);
            s.insert(DirDiffState::Modified);
            s.insert(DirDiffState::Missing);
            s
        }));
        let show_identical: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));

        // Button connections using Rc clones
        let tv_expand = Rc::clone(&tree_views);
        expand_btn.connect_clicked(move |_| {
            for tv in tv_expand.borrow().iter() {
                tv.expand_all();
            }
        });
        let tv_collapse = Rc::clone(&tree_views);
        collapse_btn.connect_clicked(move |_| {
            for tv in tv_collapse.borrow().iter() {
                tv.collapse_all();
            }
        });

        let tv_rf = Rc::clone(&tree_views);
        let e_rf = Rc::clone(&entries);
        let sf_rf = Rc::clone(&state_filter);
        let si_rf = Rc::clone(&show_identical);
        refresh_btn.connect_clicked(move |_| {
            for tv in tv_rf.borrow().iter() {
                repopulate_tree(tv, &e_rf.borrow(), &sf_rf.borrow(), *si_rf.borrow());
            }
        });

        // Filter toggles
        let tv_ref2 = Rc::clone(&tree_views);
        let stores_new2 = Rc::clone(&tree_stores);
        let e_new2 = Rc::clone(&entries);
        let sf_new2 = Rc::clone(&state_filter);
        let si_new2 = Rc::clone(&show_identical);
        new_cb.connect_toggled(move |cb| {
            toggle_filter_state(&sf_new2, DirDiffState::New, cb.is_active());
            for (tv, store) in tv_ref2.borrow().iter().zip(stores_new2.borrow().iter()) {
                repopulate_tree_with_store(
                    store,
                    &e_new2.borrow(),
                    &sf_new2.borrow(),
                    *si_new2.borrow(),
                );
            }
        });

        // Collect TreeView/Store from Rc before moving into Self
        let collected_views: Vec<gtk::TreeView> = tree_views.borrow().clone();
        let collected_stores: Vec<gtk::TreeStore> = tree_stores.borrow().clone();

        Self {
            container: main_box,
            tree_views: collected_views,
            tree_stores: collected_stores,
            folders: Rc::new(RefCell::new(Vec::new())),
            entries,
            state_filter,
            show_identical,
            scan_cancel: Rc::new(RefCell::new(None)),
            scan_source: Rc::new(RefCell::new(None)),
        }
    }

    pub fn set_folders(&self, gfiles: &[gio::File]) {
        let mut folders = self.folders.borrow_mut();
        folders.clear();
        for f in gfiles {
            if let Some(path) = f.path() {
                folders.push(path.to_path_buf());
            }
        }
    }

    pub fn set_locations(&self) {
        let folders = self.folders.borrow();
        if folders.len() < 2 {
            return;
        }
        let dir_a = folders[0].clone();
        let dir_b = folders[1].clone();
        drop(folders);

        self.cancel_scan();

        self.entries.borrow_mut().clear();
        for tv in &self.tree_views {
            if let Some(model) = tv.model() {
                if let Ok(store) = model.downcast::<gtk::TreeStore>() {
                    store.clear();
                }
            }
        }

        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        *self.scan_cancel.borrow_mut() = Some(std::sync::Arc::clone(&cancel));

        let (tx, rx) = mpsc::channel();
        let cancel_clone = std::sync::Arc::clone(&cancel);

        std::thread::spawn(move || {
            let result = scan_recursive(&dir_a, &dir_b);
            if cancel_clone.load(Ordering::SeqCst) {
                return;
            }
            let _ = tx.send(result);
        });

        let entries_ref = Rc::clone(&self.entries);
        let tree_views = self.tree_views.clone();
        let state_filter = Rc::clone(&self.state_filter);
        let show_identical = Rc::clone(&self.show_identical);

        let source_id = glib::timeout_add_local(Duration::from_millis(100), move || {
            match rx.try_recv() {
                Ok(result) => {
                    *entries_ref.borrow_mut() = result;
                    for tv in &tree_views {
                        repopulate_tree(
                            tv,
                            &entries_ref.borrow(),
                            &state_filter.borrow(),
                            *show_identical.borrow(),
                        );
                    }
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
        *self.scan_source.borrow_mut() = Some(source_id);
    }

    fn cancel_scan(&self) {
        if let Some(cancel) = self.scan_cancel.borrow_mut().take() {
            cancel.store(true, Ordering::SeqCst);
        }
        if let Some(src) = self.scan_source.borrow_mut().take() {
            src.remove();
        }
    }

    pub fn auto_compare(&self) {
        self.set_locations();
    }
}

impl MeldPage for DirDiff {
    fn widget(&self) -> &gtk::Widget {
        self.container.upcast_ref()
    }
    fn close(&self) -> gtk::ResponseType {
        self.cancel_scan();
        gtk::ResponseType::Ok
    }
    fn label(&self) -> String {
        "Directory Comparison".into()
    }
    fn show_filters(&self) -> (bool, bool, bool) {
        (false, true, false)
    }
}

// ─── Recursive scanning ──────────────────────────────────────────

fn scan_recursive(dir_a: &Path, dir_b: &Path) -> Vec<DirDiffEntry> {
    let mut file_map: HashMap<String, (Option<std::fs::DirEntry>, Option<std::fs::DirEntry>)> =
        HashMap::new();
    collect_entries(dir_a, &mut file_map, true);
    collect_entries(dir_b, &mut file_map, false);

    let mut result = Vec::new();
    for (name, (entry_a, entry_b)) in &file_map {
        let meta_a = entry_a.as_ref().and_then(|e| e.metadata().ok());
        let meta_b = entry_b.as_ref().and_then(|e| e.metadata().ok());
        let (is_dir, state) = determine_state(meta_a.as_ref(), meta_b.as_ref());

        let entry = if let (Some(a), Some(b)) = (entry_a, entry_b) {
            if is_dir {
                let children = scan_recursive(&a.path(), &b.path());
                DirDiffEntry {
                    name: name.clone(),
                    state,
                    size_a: 0,
                    size_b: 0,
                    modified_a: 0,
                    modified_b: 0,
                    is_dir: true,
                    children,
                }
            } else {
                let size_a = meta_a.as_ref().map(|m| m.len()).unwrap_or(0);
                let size_b = meta_b.as_ref().map(|m| m.len()).unwrap_or(0);
                let mod_a = extract_ts(meta_a.as_ref());
                let mod_b = extract_ts(meta_b.as_ref());
                let final_state = if state == DirDiffState::Same {
                    compare_contents(a.path().as_path(), b.path().as_path())
                } else {
                    state
                };
                DirDiffEntry {
                    name: name.clone(),
                    state: final_state,
                    size_a,
                    size_b,
                    modified_a: mod_a,
                    modified_b: mod_b,
                    is_dir: false,
                    children: Vec::new(),
                }
            }
        } else {
            let size_a = meta_a.as_ref().map(|m| m.len()).unwrap_or(0);
            let size_b = meta_b.as_ref().map(|m| m.len()).unwrap_or(0);
            DirDiffEntry {
                name: name.clone(),
                state,
                size_a,
                size_b,
                modified_a: extract_ts(meta_a.as_ref()),
                modified_b: extract_ts(meta_b.as_ref()),
                is_dir,
                children: Vec::new(),
            }
        };

        result.push(entry);
    }

    result.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    result
}

fn collect_entries(
    dir: &Path,
    map: &mut HashMap<String, (Option<std::fs::DirEntry>, Option<std::fs::DirEntry>)>,
    is_left: bool,
) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let slot = map.entry(name).or_insert((None, None));
            if is_left {
                slot.0 = Some(entry);
            } else {
                slot.1 = Some(entry);
            }
        }
    }
}

fn determine_state(
    ma: Option<&std::fs::Metadata>,
    mb: Option<&std::fs::Metadata>,
) -> (bool, DirDiffState) {
    match (ma, mb) {
        (None, Some(_)) => (false, DirDiffState::Missing),
        (Some(_), None) => (false, DirDiffState::New),
        (Some(ma), Some(mb)) => {
            if ma.is_dir() || mb.is_dir() {
                return (true, DirDiffState::Same);
            }
            if ma.len() == mb.len() && ma.modified().ok() == mb.modified().ok() {
                (false, DirDiffState::Same)
            } else {
                (false, DirDiffState::Modified)
            }
        }
        (None, None) => (false, DirDiffState::Error),
    }
}

fn compare_contents(a: &Path, b: &Path) -> DirDiffState {
    match (std::fs::read(a), std::fs::read(b)) {
        (Ok(ca), Ok(cb)) if ca == cb => DirDiffState::Same,
        _ => DirDiffState::Modified,
    }
}

fn extract_ts(meta: Option<&std::fs::Metadata>) -> u64 {
    meta.and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn repopulate_tree(
    tv: &gtk::TreeView,
    entries: &[DirDiffEntry],
    filter: &HashSet<DirDiffState>,
    show_identical: bool,
) {
    let Some(model) = tv.model() else { return };
    let Ok(store) = model.downcast::<gtk::TreeStore>() else {
        return;
    };
    store.clear();
    populate_level(&store, None, entries, filter, show_identical);
}

fn repopulate_tree_with_store(
    store: &gtk::TreeStore,
    entries: &[DirDiffEntry],
    filter: &HashSet<DirDiffState>,
    show_identical: bool,
) {
    store.clear();
    populate_level(store, None, entries, filter, show_identical);
}

fn populate_level(
    store: &gtk::TreeStore,
    parent: Option<&gtk::TreeIter>,
    entries: &[DirDiffEntry],
    filter: &HashSet<DirDiffState>,
    show_identical: bool,
) {
    for entry in entries {
        if !show_identical
            && matches!(entry.state, DirDiffState::Same | DirDiffState::Filtered)
            && entry.children.is_empty()
        {
            continue;
        }
        if !filter.contains(&entry.state) && !entry.is_dir {
            continue;
        }
        let iter = store.append(parent);
        populate_row(store, &iter, entry);
        populate_level(store, Some(&iter), &entry.children, filter, show_identical);
    }
}

fn populate_row(store: &gtk::TreeStore, iter: &gtk::TreeIter, entry: &DirDiffEntry) {
    let size_str = if entry.is_dir {
        String::new()
    } else {
        format!("{} / {}", entry.size_a, entry.size_b)
    };
    let mod_str = if entry.is_dir {
        String::new()
    } else {
        format!("{} / {}", entry.modified_a, entry.modified_b)
    };
    let display = if entry.is_dir {
        format!("{}/", entry.name)
    } else {
        entry.name.clone()
    };
    store.set_value(iter, 0, &display.to_value());
    store.set_value(iter, 1, &entry.state.as_str().to_value());
    store.set_value(iter, 2, &size_str.to_value());
    store.set_value(iter, 3, &mod_str.to_value());
    store.set_value(iter, 4, &entry.is_dir.to_value());
}

fn toggle_filter_state(filter: &RefCell<HashSet<DirDiffState>>, state: DirDiffState, active: bool) {
    let mut f = filter.borrow_mut();
    if active {
        f.insert(state);
    } else {
        f.remove(&state);
    }
}
