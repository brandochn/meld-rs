#![cfg(feature = "gui")]
//! Main application window with complete header bar, notebook with close-button tabs,
//! view toolbar, filter buttons, and spinner.
//!
//! Matches the exact layout of `appwindow.ui` (284 lines) and dispatches
//! `DiffRequest`s from the NewDiffTab into the appropriate comparison type.

use gio::prelude::*;
use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use crate::config::recent::RecentType;
use crate::config::settings::{MeldSettings, PaneOrder};
use crate::diff::dirdiff::DirDiff;
use crate::diff::filediff::FileDiff;
use crate::ui::new_diff_tab::DiffType;
use crate::ui::tab_manager::TabLabel;
use crate::ui::vc_view::VcView;
use crate::vc::{ConflictKind, Vc, VcFileStatus};

/// The main application window.
pub struct MeldWindow {
    window: gtk::ApplicationWindow,
    notebook: gtk::Notebook,
    pages: Rc<RefCell<Vec<Box<dyn MeldPage>>>>,
    /// Action group behind the gear menu's `view.*` entries.
    view_group: gio::SimpleActionGroup,
    view_toolbar: gtk::Box,
    _spinner: gtk::Spinner,
    prev_change_btn: gtk::Button,
    next_change_btn: gtk::Button,
    vc_filter_btn: gtk::MenuButton,
    folder_filter_btn: gtk::MenuButton,
    text_filter_btn: gtk::MenuButton,
    prev_conflict_btn: gtk::Button,
    next_conflict_btn: gtk::Button,
    /// Persisted user settings (includes vc_left_is_local, vc_merge_file_order).
    settings: Rc<MeldSettings>,
}

/// Common interface for all tab content types.
pub trait MeldPage {
    fn widget(&self) -> &gtk::Widget;
    fn close(&self) -> gtk::ResponseType;
    fn on_container_switch_in(&self) {}
    fn on_container_switch_out(&self) {}
    fn label(&self) -> String;
    fn show_filters(&self) -> (bool, bool, bool) {
        (false, false, false)
    }
    fn show_conflict_nav(&self) -> bool {
        false
    }
    /// Navigate to the next diff chunk.
    fn go_next_diff(&self) {}
    /// Navigate to the previous diff chunk.
    fn go_prev_diff(&self) {}
    /// Navigate to the next merge conflict.
    fn go_next_conflict(&self) {}
    /// Navigate to the previous merge conflict.
    fn go_prev_conflict(&self) {}
    /// Called by the window to inject a callback for creating diffs.
    fn set_diff_created_callback(&self, _cb: DiffCreatedCallback) {}
    /// Re-apply settings after preferences dialog is closed.
    fn apply_settings(&self, _settings: &MeldSettings) {}

    // ── Gear-menu action handlers ──────────────────────────────────

    /// Save the currently focused pane to disk.
    fn action_save(&self) {}
    /// Save the focused pane with a new filename ("Save As…").
    fn action_save_as(&self) {}
    /// Save every modified pane.
    fn action_save_all(&self) {}
    /// Revert all panes to the last saved (or original) state.
    fn action_revert(&self) {}
    /// Open the focused file in the system's external editor.
    fn action_open_external(&self) {}
    /// Refresh / recompute the comparison from scratch.
    fn action_refresh(&self) {}
    /// Which `view.*` gear-menu actions this page actually handles.
    ///
    /// Actions not listed here are disabled while the page is active,
    /// mirroring the original Meld's per-document action registration.
    fn supported_view_actions(&self) -> &'static [&'static str] {
        &[]
    }
    /// Open the find bar.
    fn action_find(&self) {}
    /// Open the find-and-replace bar.
    fn action_find_replace(&self) {}
    /// Jump to the next search match.
    fn action_find_next(&self) {}
    /// Jump to the previous search match.
    fn action_find_previous(&self) {}
    /// Cancel the currently-running background task (diff, scan, etc.).
    fn action_stop(&self) {}
    /// Undo the most recent edit in the focused pane.
    fn action_undo(&self) {}
    /// Redo the most recently undone edit in the focused pane.
    fn action_redo(&self) {}
    /// Merge all non-conflicting changes from the right pane into the left.
    fn action_merge_all_left(&self) {}
    /// Merge all non-conflicting changes from the left pane into the right.
    fn action_merge_all_right(&self) {}
    /// Auto-merge all non-conflicting changes (3-way merge).
    fn action_merge_all(&self) {}
    /// Open the "Format as Patch" dialog.
    fn action_format_as_patch(&self) {}
    /// Swap the left and right panes.
    fn action_swap_panes(&self) {}
    /// Toggle the overview (chunk) map visibility.
    fn toggle_overview_map(&self) {}
    /// Toggle synchronized scrolling lock.
    fn toggle_lock_scrolling(&self) {}
    /// Toggle the version control console visibility.
    fn toggle_vc_console(&self) {}
}

/// Payload sent from `NewDiffTab` when the user requests a comparison.
pub struct DiffRequest {
    pub diff_type: DiffType,
    pub paths: Vec<Option<PathBuf>>,
}

/// Callback invoked when the user clicks Compare or Blank in the NewDiffTab.
pub type DiffCreatedCallback = Box<dyn Fn(DiffRequest)>;

/// Keyboard accelerators, mirroring `meld/meld/accelerators.py`.
///
/// Only actions that meld-rs actually registers are listed here. Accelerator
/// strings use `<Primary>` (Cmd on macOS, Ctrl elsewhere) and `<Control>`
/// exactly as the original Meld does.
const ACCELERATORS: &[(&str, &[&str])] = &[
    // Application actions
    ("app.quit", &["<Primary>Q"]),
    ("app.help", &["F1"]),
    ("app.preferences", &["<Primary>comma"]),
    // Window actions
    ("win.new-tab", &["<Primary>N"]),
    ("win.close", &["<Primary>W"]),
    ("win.fullscreen", &["F11"]),
    ("win.stop", &["Escape"]),
    // View actions
    ("view.save-as", &["<Primary><Shift>S"]),
    ("view.save-all", &["<Primary><Shift>L"]),
    ("view.open-external", &["<Primary><Shift>O"]),
    ("view.refresh", &["<Control>R", "F5"]),
    ("view.find", &["<Primary>F"]),
    ("view.find-replace", &["<Primary>H"]),
    ("view.undo", &["<Primary>Z"]),
    ("view.redo", &["<Primary><Shift>Z"]),
    ("view.show-overview-map", &["F9"]),
    ("view.swap-2-panes", &["<Alt>backslash"]),
];

/// Every `view.*` action registered in [`MeldWindow::setup_view_actions`].
/// Used to enable/disable the gear-menu entries for the active page.
const VIEW_ACTIONS: &[&str] = &[
    "save-as",
    "save-all",
    "revert",
    "open-external",
    "refresh",
    "find",
    "find-replace",
    "undo",
    "redo",
    "show-overview-map",
    "lock-scrolling",
    "swap-2-panes",
    "merge-all-left",
    "merge-all-right",
    "merge-all",
    "format-as-patch",
    "vc-console-visible",
];

/// Register all keyboard accelerators once at application startup.
pub(crate) fn register_accels(app: &gtk::Application) {
    for (action, accels) in ACCELERATORS {
        app.set_accels_for_action(action, accels);
    }
}

/// A single entry in the shortcuts overlay: either an application action
/// (whose accelerator is looked up from [`ACCELERATORS`]) or a raw GTK
/// accelerator string.
enum ShortcutSource {
    Action(&'static str),
    Accel(&'static str),
}

/// Return the space-joined accelerator string registered for `action`, if any.
fn accelerator_for(action: &str) -> Option<String> {
    ACCELERATORS
        .iter()
        .find(|(name, _)| *name == action)
        .map(|(_, accels)| accels.join(" "))
}

/// Build and present the "Keyboard Shortcuts" overlay, mirroring the original
/// Meld `help-overlay.ui`.
fn show_help_overlay(parent: Option<&gtk::Window>) {
    let overlay = gtk::ShortcutsWindow::builder().build();
    overlay.set_modal(true);
    if let Some(win) = parent {
        overlay.set_transient_for(Some(win));
    }

    let section = gtk::ShortcutsSection::builder().build();
    section.set_section_name(Some("main"));
    section.set_visible(true);

    let groups: &[(&str, &[(ShortcutSource, &str)])] = &[
        (
            "Windows and Tabs",
            &[
                (ShortcutSource::Action("win.new-tab"), "New comparison"),
                (ShortcutSource::Action("win.close"), "Close a comparison"),
                (ShortcutSource::Action("app.quit"), "Quit Meld"),
            ],
        ),
        (
            "General",
            &[
                (ShortcutSource::Action("app.help"), "Show help"),
                (
                    ShortcutSource::Action("app.preferences"),
                    "Show preferences",
                ),
                (
                    ShortcutSource::Accel("<Primary>question"),
                    "Keyboard shortcuts",
                ),
                (ShortcutSource::Action("win.fullscreen"), "Fullscreen"),
            ],
        ),
        (
            "Common Actions",
            &[
                (
                    ShortcutSource::Action("win.stop"),
                    "Stop the current action",
                ),
                (ShortcutSource::Action("view.refresh"), "Refresh comparison"),
                (
                    ShortcutSource::Action("view.open-external"),
                    "Open externally",
                ),
            ],
        ),
        (
            "Panes",
            &[(
                ShortcutSource::Action("view.swap-2-panes"),
                "Swap left and right panes",
            )],
        ),
        (
            "Changes",
            &[
                (ShortcutSource::Accel("<Alt>Up"), "Go to previous change"),
                (ShortcutSource::Accel("<Alt>Down"), "Go to next change"),
            ],
        ),
        (
            "Tabs",
            &[
                (
                    ShortcutSource::Accel("<Primary><Alt>Page_Up"),
                    "Go to previous tab",
                ),
                (
                    ShortcutSource::Accel("<Primary><Alt>Page_Down"),
                    "Go to next tab",
                ),
                (ShortcutSource::Accel("<Alt>0...9"), "Switch to tab"),
                (
                    ShortcutSource::Accel("<Shift><Primary><Alt>Page_Up"),
                    "Move tab left",
                ),
                (
                    ShortcutSource::Accel("<Shift><Primary><Alt>Page_Down"),
                    "Move tab right",
                ),
            ],
        ),
        (
            "Editing",
            &[
                (ShortcutSource::Action("view.undo"), "Undo"),
                (ShortcutSource::Action("view.redo"), "Redo"),
                (ShortcutSource::Accel("<Primary>X"), "Cut"),
                (ShortcutSource::Accel("<Primary>C"), "Copy"),
                (ShortcutSource::Accel("<Primary>V"), "Paste"),
                (ShortcutSource::Action("view.find"), "Find"),
                (ShortcutSource::Accel("<Primary>G"), "Find Next"),
                (ShortcutSource::Accel("<Primary><Shift>G"), "Find Previous"),
                (ShortcutSource::Action("view.find-replace"), "Replace"),
            ],
        ),
        (
            "File comparison",
            &[
                (
                    ShortcutSource::Action("view.save-as"),
                    "Save current file to new path",
                ),
                (
                    ShortcutSource::Action("view.save-all"),
                    "Save all files in comparison",
                ),
                (
                    ShortcutSource::Action("view.show-overview-map"),
                    "Show overview map",
                ),
            ],
        ),
    ];

    for (group_title, shortcuts) in groups {
        let group = gtk::ShortcutsGroup::builder().build();
        group.set_title(Some(group_title));
        group.set_visible(true);
        for (source, title) in *shortcuts {
            let shortcut = gtk::ShortcutsShortcut::builder().build();
            shortcut.set_title(Some(title));
            // GTK only auto-resolves `action-name` when the shortcuts window
            // is associated via the private `gtk_shortcuts_window_set_window`,
            // which gtk-rs does not expose. Set the accelerator explicitly.
            match source {
                ShortcutSource::Action(action) => {
                    shortcut.set_action_name(Some(action));
                    if let Some(accel) = accelerator_for(action) {
                        shortcut.set_accelerator(Some(&accel));
                    }
                }
                ShortcutSource::Accel(accel) => {
                    shortcut.set_accelerator(Some(accel));
                }
            }
            shortcut.set_visible(true);
            group.add_shortcut(&shortcut);
        }
        section.add_group(&group);
    }

    overlay.add_section(&section);
    overlay.present();
}

impl MeldWindow {
    pub fn new(app: &gtk::Application) -> Self {
        let window = gtk::ApplicationWindow::new(app);
        window.set_title(Some("Meld-rs"));
        window.set_default_size(1280, 720);

        let settings = Rc::new(MeldSettings::load().unwrap_or_default());

        let header = gtk::HeaderBar::new();
        header.set_show_title_buttons(true);
        window.set_titlebar(Some(&header));

        let grp_left = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        grp_left.add_css_class("linked");

        let new_btn = gtk::Button::from_icon_name("tab-new-symbolic");
        new_btn.set_tooltip_text(Some("Start a new comparison"));
        new_btn.set_focus_on_click(false);
        grp_left.append(&new_btn);

        let recent_btn = gtk::MenuButton::new();
        recent_btn.set_icon_name("document-open-recent-symbolic");
        recent_btn.set_tooltip_text(Some("Open a recent comparison"));
        recent_btn.set_focus_on_click(false);
        grp_left.append(&recent_btn);
        header.pack_start(&grp_left);

        let grp_changes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        grp_changes.add_css_class("linked");
        let prev_change_btn = gtk::Button::from_icon_name("go-up-symbolic");
        prev_change_btn.set_tooltip_text(Some("Go to the previous change"));
        prev_change_btn.set_focus_on_click(false);
        prev_change_btn.add_css_class("image-button");
        grp_changes.append(&prev_change_btn);
        let next_change_btn = gtk::Button::from_icon_name("go-down-symbolic");
        next_change_btn.set_tooltip_text(Some("Go to the next change"));
        next_change_btn.set_focus_on_click(false);
        next_change_btn.add_css_class("image-button");
        grp_changes.append(&next_change_btn);
        header.pack_start(&grp_changes);

        let grp_conflicts = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        grp_conflicts.add_css_class("linked");
        let prev_conflict_btn = gtk::Button::from_icon_name("go-top-symbolic");
        prev_conflict_btn.set_tooltip_text(Some("Go to the previous conflict"));
        prev_conflict_btn.set_focus_on_click(false);
        prev_conflict_btn.add_css_class("image-button");
        prev_conflict_btn.set_visible(false);
        grp_conflicts.append(&prev_conflict_btn);
        let next_conflict_btn = gtk::Button::from_icon_name("go-bottom-symbolic");
        next_conflict_btn.set_tooltip_text(Some("Go to the next conflict"));
        next_conflict_btn.set_focus_on_click(false);
        next_conflict_btn.add_css_class("image-button");
        next_conflict_btn.set_visible(false);
        grp_conflicts.append(&next_conflict_btn);
        header.pack_start(&grp_conflicts);

        let view_toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        header.pack_start(&view_toolbar);

        let gear_btn = gtk::MenuButton::new();
        gear_btn.set_icon_name("open-menu-symbolic");
        gear_btn.set_tooltip_text(Some("Menu"));
        gear_btn.set_focus_on_click(false);
        header.pack_end(&gear_btn);

        let vc_filter_btn = gtk::MenuButton::new();
        vc_filter_btn.set_label("Version Filters");
        vc_filter_btn.set_visible(false);
        header.pack_end(&vc_filter_btn);

        let folder_filter_btn = gtk::MenuButton::new();
        folder_filter_btn.set_label("File Filters");
        folder_filter_btn.set_visible(false);
        header.pack_end(&folder_filter_btn);

        let text_filter_btn = gtk::MenuButton::new();
        text_filter_btn.set_label("Text Filters");
        text_filter_btn.set_visible(false);
        header.pack_end(&text_filter_btn);

        let spinner = gtk::Spinner::new();
        spinner.set_visible(false);
        header.pack_end(&spinner);

        let notebook = gtk::Notebook::new();
        notebook.set_scrollable(true);
        notebook.set_show_tabs(true);
        notebook.set_tab_pos(gtk::PositionType::Top);

        let main_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        main_box.append(&notebook);
        window.set_child(Some(&main_box));

        let pages: Rc<RefCell<Vec<Box<dyn MeldPage>>>> = Rc::new(RefCell::new(Vec::new()));
        let menu = build_gear_menu();
        gear_btn.set_menu_model(Some(&menu));
        let view_group = gio::SimpleActionGroup::new();

        let w = Self {
            window,
            notebook,
            pages,
            view_group,
            view_toolbar,
            _spinner: spinner,
            prev_change_btn: prev_change_btn.clone(),
            next_change_btn: next_change_btn.clone(),
            vc_filter_btn,
            folder_filter_btn,
            text_filter_btn,
            prev_conflict_btn,
            next_conflict_btn,
            settings,
        };

        w.setup_signals(&new_btn, &recent_btn);
        w.setup_accels();
        w.setup_drag_drop();
        w.setup_preferences_action();
        w.setup_theme_watcher();
        w.setup_gear_actions();
        w.setup_text_filter_popover();
        w
    }

    /// Show the window.
    pub fn present(&self) {
        self.window.present();
    }

    /// Append a "New Comparison" tab.
    pub fn append_new_comparison(&self) {
        let tab = crate::ui::new_diff_tab();
        let label = create_closeable_tab_label("New comparison", &self.notebook, &self.pages);
        wire_new_diff_tab_standalone(
            &tab,
            &self.notebook,
            &self.pages,
            &self.settings,
            &self.window,
        );
        self.notebook.append_page(tab.widget(), Some(&label.widget));
        self.pages.borrow_mut().push(Box::new(tab));
    }

    /// Open one or more file paths for comparison.
    pub fn open_paths(
        &self,
        gfiles: &[gio::File],
        auto_compare: bool,
        _auto_merge: bool,
        setup_cb: bool,
    ) {
        if setup_cb {
            open_comparison_in_notebook(
                &self.notebook,
                &self.pages,
                gfiles,
                auto_compare,
                _auto_merge,
                &self.settings,
                &self.window,
            );
        } else {
            let num_panes = gfiles.len().max(2);
            let filediff = FileDiff::new(num_panes);
            filediff.set_font(self.settings.use_system_font, &self.settings.custom_font);
            filediff.set_ignore_blanks(self.settings.ignore_blank_lines);
            filediff.set_show_connectors(self.settings.show_connectors);
            filediff.set_inline_diff_mode(&self.settings.inline_diff_mode);
            filediff.apply_settings(&self.settings);
            filediff.connect_gutter_key_modes(&self.window);
            filediff.set_files(gfiles);
            let label = create_closeable_tab_label("File Comparison", &self.notebook, &self.pages);
            self.notebook
                .append_page(filediff.widget(), Some(&label.widget));
            self.pages.borrow_mut().push(Box::new(filediff));
        }
    }

    pub fn open_file_merge(&self, gfiles: &[gio::File], output: Option<&str>) {
        let num_panes = 3usize;
        let filediff = FileDiff::new(num_panes);
        filediff.set_font(self.settings.use_system_font, &self.settings.custom_font);
        filediff.set_ignore_blanks(self.settings.ignore_blank_lines);
        filediff.set_show_connectors(self.settings.show_connectors);
        filediff.set_inline_diff_mode(&self.settings.inline_diff_mode);
        filediff.apply_settings(&self.settings);
        filediff.connect_gutter_key_modes(&self.window);
        filediff.set_files(gfiles);
        if let Some(out) = output {
            filediff.set_merge_output_file(out);
        }
        let label = create_closeable_tab_label("File Merge", &self.notebook, &self.pages);
        self.notebook
            .append_page(filediff.widget(), Some(&label.widget));
        self.pages.borrow_mut().push(Box::new(filediff));
    }

    pub fn open_vc_view(&self, location: &str, auto_compare: bool) {
        let vc = VcView::new();
        vc.set_location(location);
        let label = create_closeable_tab_label("Version Control", &self.notebook, &self.pages);
        self.notebook.append_page(vc.widget(), Some(&label.widget));
        self.pages.borrow_mut().push(Box::new(vc));
        let _ = auto_compare;
    }

    pub fn set_labels(&self, _labels: &[String]) {
        // Per-page labels are set in FileDiff::set_labels.
    }

    pub fn has_pages(&self) -> bool {
        !self.pages.borrow().is_empty()
    }

    fn open_single_path(&self, path: &std::path::Path, auto_compare: bool, setup_cb: bool) {
        if path.is_dir() {
            self.open_vc_view(&path.to_string_lossy().into_owned(), auto_compare);
        } else {
            self.open_paths(&[gio::File::for_path(path)], auto_compare, false, setup_cb);
        }
    }

    fn create_and_append_new_diff_tab(&self) {
        let tab = crate::ui::new_diff_tab();
        let label = create_closeable_tab_label("New comparison", &self.notebook, &self.pages);
        self.notebook.append_page(tab.widget(), Some(&label.widget));
        self.pages.borrow_mut().push(Box::new(tab));
    }

    fn wire_new_diff_tab(
        &self,
        tab: &dyn MeldPage,
        notebook: &gtk::Notebook,
        pages: &Rc<RefCell<Vec<Box<dyn MeldPage>>>>,
    ) {
        let nb = notebook.clone();
        let p = Rc::clone(pages);
        let s = Rc::new(self.settings.clone());
        let w = self.window.clone();
        tab.set_diff_created_callback(Box::new(move |req: DiffRequest| {
            let auto_compare = false;
            let auto_merge = false;
            handle_diff_request(&nb, &p, &req, auto_compare, auto_merge, &s, &w);

            let p_clone = Rc::clone(&p);
            let nb_clone = nb.clone();
            glib::idle_add_local(move || {
                let to_remove: Vec<usize> = {
                    let mut pages = p_clone.borrow_mut();
                    let mut indices = Vec::new();
                    for (i, page) in pages.iter().enumerate() {
                        if page.label() == "New comparison" {
                            indices.push(i);
                        }
                    }
                    for &idx in indices.iter().rev() {
                        pages.remove(idx);
                    }
                    indices
                };
                for idx in to_remove.iter().rev() {
                    nb_clone.remove_page(Some(*idx as u32));
                }
                glib::ControlFlow::Break
            });
        }));
    }

    fn setup_signals(&self, new_btn: &gtk::Button, recent_btn: &gtk::MenuButton) {
        let nb = self.notebook.clone();
        let pages = self.pages.clone();
        let settings = Rc::clone(&self.settings);
        let w = self.window.clone();
        new_btn.connect_clicked(move |_| {
            let tab = crate::ui::new_diff_tab();
            let label = create_closeable_tab_label("New comparison", &nb, &pages);
            wire_new_diff_tab_standalone(&tab, &nb, &pages, &settings, &w);
            nb.append_page(tab.widget(), Some(&label.widget));
            pages.borrow_mut().push(Box::new(tab));
        });

        let popover = gtk::Popover::new();
        let nb_sel = self.notebook.clone();
        let p_sel = Rc::clone(&self.pages);
        let s_sel = Rc::clone(&self.settings);
        let w_sel = self.window.clone();
        popover.connect_show(move |pop| {
            let selector = crate::ui::recent_selector::RecentSelector::new({
                let nb = nb_sel.clone();
                let p = Rc::clone(&p_sel);
                let s = Rc::clone(&s_sel);
                let w = w_sel.clone();
                move |paths| {
                    let gfiles: Vec<gio::File> =
                        paths.iter().map(|p| gio::File::for_path(p)).collect();
                    open_comparison_in_notebook(&nb, &p, &gfiles, false, false, &s, &w);
                }
            });
            pop.set_child(Some(selector.widget()));
        });
        recent_btn.set_popover(Some(&popover));

        let pages_nav = self.pages.clone();
        let nb_nav = self.notebook.clone();
        self.prev_change_btn.connect_clicked(move |_| {
            let pages = pages_nav.borrow();
            if let Some(idx) = nb_nav.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.go_prev_diff();
                }
            }
        });

        let pages_nav2 = self.pages.clone();
        let nb_nav2 = self.notebook.clone();
        self.next_change_btn.connect_clicked(move |_| {
            let pages = pages_nav2.borrow();
            if let Some(idx) = nb_nav2.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.go_next_diff();
                }
            }
        });

        let pages_cnf = self.pages.clone();
        let nb_cnf = self.notebook.clone();
        self.prev_conflict_btn.connect_clicked(move |_| {
            let pages = pages_cnf.borrow();
            if let Some(idx) = nb_cnf.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.go_prev_conflict();
                }
            }
        });

        let pages_cnf2 = self.pages.clone();
        let nb_cnf2 = self.notebook.clone();
        self.next_conflict_btn.connect_clicked(move |_| {
            let pages = pages_cnf2.borrow();
            if let Some(idx) = nb_cnf2.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.go_next_conflict();
                }
            }
        });

        let vcf = self.vc_filter_btn.clone();
        let ff = self.folder_filter_btn.clone();
        let tf = self.text_filter_btn.clone();
        let pc = self.prev_conflict_btn.clone();
        let nc = self.next_conflict_btn.clone();
        let view_tb = self.view_toolbar.clone();
        let pages_switch = self.pages.clone();
        let view_group_sw = self.view_group.clone();

        self.notebook.connect_switch_page(move |_, _, idx| {
            while let Some(child) = view_tb.first_child() {
                view_tb.remove(&child);
            }
            let pages = pages_switch.borrow();
            if let Some(page) = pages.get(idx as usize) {
                let (vc, folder, text) = page.show_filters();
                vcf.set_visible(vc);
                ff.set_visible(folder);
                tf.set_visible(text);
                let show_conf = page.show_conflict_nav();
                pc.set_visible(show_conf);
                nc.set_visible(show_conf);
                page.on_container_switch_in();
                // Enable only the `view.*` actions the active page handles,
                // mirroring the original Meld's per-document action groups.
                let supported = page.supported_view_actions();
                for name in VIEW_ACTIONS {
                    if let Some(action) = view_group_sw.lookup_action(name) {
                        if let Some(action) = action.downcast_ref::<gio::SimpleAction>() {
                            action.set_enabled(supported.contains(name));
                        }
                    }
                }
            }
        });
    }

    fn setup_accels(&self) {
        let nb = self.notebook.clone();
        let pages = self.pages.clone();
        let settings = Rc::clone(&self.settings);
        let w = self.window.clone();
        let new_action = gio::SimpleAction::new("new-tab", None);
        new_action.connect_activate(move |_, _| {
            let tab = crate::ui::new_diff_tab();
            let label = create_closeable_tab_label("New comparison", &nb, &pages);
            wire_new_diff_tab_standalone(&tab, &nb, &pages, &settings, &w);
            nb.append_page(tab.widget(), Some(&label.widget));
            pages.borrow_mut().push(Box::new(tab));
        });
        self.window.add_action(&new_action);

        let nb2 = self.notebook.clone();
        let p2 = self.pages.clone();
        let close_action = gio::SimpleAction::new("close", None);
        close_action.connect_activate(move |_, _| {
            if let Some(idx) = nb2.current_page() {
                let resp = p2
                    .borrow()
                    .get(idx as usize)
                    .map(|pg| pg.close())
                    .unwrap_or(gtk::ResponseType::Cancel);
                if resp == gtk::ResponseType::Ok {
                    p2.borrow_mut().remove(idx as usize);
                    nb2.remove_page(Some(idx));
                }
            }
        });
        self.window.add_action(&close_action);
    }

    fn setup_drag_drop(&self) {
        let drop_target = gtk::DropTarget::new(gio::File::static_type(), gdk4::DragAction::COPY);
        drop_target.set_actions(gdk4::DragAction::COPY);

        let nb = self.notebook.clone();
        let p = self.pages.clone();
        let settings = Rc::clone(&self.settings);
        let w = self.window.clone();
        drop_target.connect_drop(move |_, value, _x, _y| {
            if let Ok(gfile) = value.get::<gio::File>() {
                open_comparison_in_notebook(&nb, &p, &[gfile], false, false, &settings, &w);
                true
            } else {
                false
            }
        });
        self.window.add_controller(drop_target);
    }

    fn setup_preferences_action(&self) {
        let pages = self.pages.clone();
        let window = self.window.clone();

        let prefs_action = gio::SimpleAction::new("preferences", None);
        let pages_cb = pages.clone();
        let w = window.clone();
        prefs_action.connect_activate(move |_, _| {
            let p = pages_cb.clone();
            let parent = Some(w.upcast_ref::<gtk::Window>());
            let dialog = crate::ui::preferences::PreferencesDialog::new(
                Box::new(move || {
                    if let Ok(reloaded) = MeldSettings::load() {
                        for page in p.borrow().iter() {
                            page.apply_settings(&reloaded);
                        }
                    }
                }),
                parent,
            );
            dialog.present();
        });
        self.window.add_action(&prefs_action);
    }

    /// Re-apply settings to open pages when the system appearance changes,
    /// mirroring the original Meld's `on_style_manager_setting_notify`.
    ///
    /// Uses weak references so the handler does not keep closed windows
    /// (and their documents) alive.
    fn setup_theme_watcher(&self) {
        let pages_weak = Rc::downgrade(&self.pages);
        adw::StyleManager::default().connect_dark_notify(move |_| {
            let Some(pages) = pages_weak.upgrade() else {
                return;
            };
            if let Ok(reloaded) = MeldSettings::load() {
                for page in pages.borrow().iter() {
                    page.apply_settings(&reloaded);
                }
            }
        });
    }

    /// Register all actions referenced by the gear menu so they are enabled.
    ///
    /// Actions with a `win.` prefix are added to the window; those with a
    /// `view.` prefix dispatch to the current notebook page where applicable.
    fn setup_gear_actions(&self) {
        self.setup_win_actions();
        // The gear menu references `view.*` actions, so they must live in a
        // group registered with that prefix (GtkWindow's own action map is
        // addressed as `win.*`).
        self.window
            .insert_action_group("view", Some(&self.view_group));
        self.setup_view_actions();
    }

    /// Register window-level actions (`win.*` prefix).
    fn setup_win_actions(&self) {
        // Fullscreen toggle
        let w = self.window.clone();
        let fullscreen_action = gio::SimpleAction::new("fullscreen", None);
        fullscreen_action.connect_activate(move |_, _| {
            if w.is_fullscreen() {
                w.unfullscreen();
            } else {
                w.fullscreen();
            }
        });
        self.window.add_action(&fullscreen_action);

        // Stop: dispatch to the current page's action_stop(), matching
        // the original Meld pattern of delegating to current_doc().
        let pages_stop = self.pages.clone();
        let nb_stop = self.notebook.clone();
        let stop_action = gio::SimpleAction::new("stop", None);
        stop_action.connect_activate(move |_, _| {
            let pages = pages_stop.borrow();
            if let Some(idx) = nb_stop.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.action_stop();
                }
            }
        });
        self.window.add_action(&stop_action);

        // Keyboard shortcuts overlay
        let overlay_parent = self.window.clone();
        let help_overlay_action = gio::SimpleAction::new("show-help-overlay", None);
        help_overlay_action.connect_activate(move |_, _| {
            show_help_overlay(Some(overlay_parent.upcast_ref::<gtk::Window>()));
        });
        self.window.add_action(&help_overlay_action);
    }

    /// Register view-level actions (`view.*` prefix) that dispatch to the
    /// currently active notebook page via [`MeldPage`] trait methods.

    fn setup_view_actions(&self) {
        let pages = self.pages.clone();
        let nb = self.notebook.clone();

        // Each view.* action is registered explicitly below.
        // Pattern: clone Rc pointers, create action, connect_activate
        // dispatches to the current page's trait method.

        let p = pages.clone();
        let n = nb.clone();
        let action = gio::SimpleAction::new("save-as", None);
        action.connect_activate(move |_, _| {
            let pages = p.borrow();
            if let Some(idx) = n.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.action_save_as();
                }
            }
        });
        self.view_group.add_action(&action);

        let p = pages.clone();
        let n = nb.clone();
        let action = gio::SimpleAction::new("save-all", None);
        action.connect_activate(move |_, _| {
            let pages = p.borrow();
            if let Some(idx) = n.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.action_save_all();
                }
            }
        });
        self.view_group.add_action(&action);

        let p = pages.clone();
        let n = nb.clone();
        let action = gio::SimpleAction::new("revert", None);
        action.connect_activate(move |_, _| {
            let pages = p.borrow();
            if let Some(idx) = n.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.action_revert();
                }
            }
        });
        self.view_group.add_action(&action);

        let p = pages.clone();
        let n = nb.clone();
        let action = gio::SimpleAction::new("open-external", None);
        action.connect_activate(move |_, _| {
            let pages = p.borrow();
            if let Some(idx) = n.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.action_open_external();
                }
            }
        });
        self.view_group.add_action(&action);

        let p = pages.clone();
        let n = nb.clone();
        let action = gio::SimpleAction::new("refresh", None);
        action.set_enabled(false);
        action.connect_activate(move |_, _| {
            let pages = p.borrow();
            if let Some(idx) = n.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.action_refresh();
                }
            }
        });
        self.view_group.add_action(&action);

        let p = pages.clone();
        let n = nb.clone();
        let action = gio::SimpleAction::new("find", None);
        action.connect_activate(move |_, _| {
            let pages = p.borrow();
            if let Some(idx) = n.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.action_find();
                }
            }
        });
        self.view_group.add_action(&action);

        let p = pages.clone();
        let n = nb.clone();
        let action = gio::SimpleAction::new("find-replace", None);
        action.connect_activate(move |_, _| {
            let pages = p.borrow();
            if let Some(idx) = n.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.action_find_replace();
                }
            }
        });
        self.view_group.add_action(&action);

        let p = pages.clone();
        let n = nb.clone();
        let action = gio::SimpleAction::new("undo", None);
        action.connect_activate(move |_, _| {
            let pages = p.borrow();
            if let Some(idx) = n.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.action_undo();
                }
            }
        });
        self.view_group.add_action(&action);

        let p = pages.clone();
        let n = nb.clone();
        let action = gio::SimpleAction::new("redo", None);
        action.connect_activate(move |_, _| {
            let pages = p.borrow();
            if let Some(idx) = n.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.action_redo();
                }
            }
        });
        self.view_group.add_action(&action);

        let p = pages.clone();
        let n = nb.clone();
        let action = gio::SimpleAction::new("show-overview-map", None);
        action.connect_activate(move |_, _| {
            let pages = p.borrow();
            if let Some(idx) = n.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.toggle_overview_map();
                }
            }
        });
        self.view_group.add_action(&action);

        let p = pages.clone();
        let n = nb.clone();
        let action = gio::SimpleAction::new("lock-scrolling", None);
        action.connect_activate(move |_, _| {
            let pages = p.borrow();
            if let Some(idx) = n.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.toggle_lock_scrolling();
                }
            }
        });
        self.view_group.add_action(&action);

        let p = pages.clone();
        let n = nb.clone();
        let action = gio::SimpleAction::new("swap-2-panes", None);
        action.connect_activate(move |_, _| {
            let pages = p.borrow();
            if let Some(idx) = n.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.action_swap_panes();
                }
            }
        });
        self.view_group.add_action(&action);

        let p = pages.clone();
        let n = nb.clone();
        let action = gio::SimpleAction::new("merge-all-left", None);
        action.connect_activate(move |_, _| {
            let pages = p.borrow();
            if let Some(idx) = n.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.action_merge_all_left();
                }
            }
        });
        self.view_group.add_action(&action);

        let p = pages.clone();
        let n = nb.clone();
        let action = gio::SimpleAction::new("merge-all-right", None);
        action.connect_activate(move |_, _| {
            let pages = p.borrow();
            if let Some(idx) = n.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.action_merge_all_right();
                }
            }
        });
        self.view_group.add_action(&action);

        let p = pages.clone();
        let n = nb.clone();
        let action = gio::SimpleAction::new("merge-all", None);
        action.connect_activate(move |_, _| {
            let pages = p.borrow();
            if let Some(idx) = n.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.action_merge_all();
                }
            }
        });
        self.view_group.add_action(&action);

        let p = pages.clone();
        let n = nb.clone();
        let action = gio::SimpleAction::new("format-as-patch", None);
        action.connect_activate(move |_, _| {
            let pages = p.borrow();
            if let Some(idx) = n.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.action_format_as_patch();
                }
            }
        });
        self.view_group.add_action(&action);

        // vc-console-visible: dispatch to the current page's VcView.
        let p = pages.clone();
        let n = nb.clone();
        let action = gio::SimpleAction::new("vc-console-visible", None);
        action.connect_activate(move |_, _| {
            let pages = p.borrow();
            if let Some(idx) = n.current_page() {
                if let Some(page) = pages.get(idx as usize) {
                    page.toggle_vc_console();
                }
            }
        });
        self.view_group.add_action(&action);
    }

    /// Set up a popover for the text-filter dropdown button with checkboxes
    /// for every text filter defined in the current settings.
    fn setup_text_filter_popover(&self) {
        let settings = Rc::clone(&self.settings);
        let btn = self.text_filter_btn.clone();

        let popover = gtk::Popover::new();
        popover.connect_show(move |pop| {
            // Clear any previous content so the popover is rebuilt fresh
            // every time it opens, reflecting the latest settings on disk.
            pop.set_child(gtk::Widget::NONE);

            let vbox = gtk::Box::new(gtk::Orientation::Vertical, 4);
            vbox.set_margin_top(8);
            vbox.set_margin_bottom(8);
            vbox.set_margin_start(12);
            vbox.set_margin_end(12);

            let header = gtk::Label::new(Some("Text Filters"));
            header.add_css_class("heading");
            header.set_halign(gtk::Align::Start);
            vbox.append(&header);

            let filters = settings.text_filters.clone();

            for (i, filter) in filters.iter().enumerate() {
                let cb = gtk::CheckButton::with_label(&filter.name);
                cb.set_active(filter.enabled);
                cb.connect_toggled(move |check| {
                    // Persist the toggle by reloading settings from disk,
                    // updating the relevant filter, and saving back.
                    if let Ok(mut updated) = MeldSettings::load() {
                        if let Some(f) = updated.text_filters.get_mut(i) {
                            f.enabled = check.is_active();
                        }
                        if let Err(e) = updated.save() {
                            log::error!("Failed to save text filter setting: {}", e);
                        }
                    }
                });
                vbox.append(&cb);
            }

            pop.set_child(Some(&vbox));
        });

        btn.set_popover(Some(&popover));
    }
}

fn open_comparison_in_notebook(
    notebook: &gtk::Notebook,
    pages: &Rc<RefCell<Vec<Box<dyn MeldPage>>>>,
    gfiles: &[gio::File],
    auto_compare: bool,
    _auto_merge: bool,
    settings: &MeldSettings,
    window: &gtk::ApplicationWindow,
) {
    if is_directory_comparison(gfiles) {
        let num_panes = gfiles.len().max(2);
        let dirdiff = DirDiff::new(num_panes);
        dirdiff.set_folders(gfiles);
        dirdiff.set_locations();
        if auto_compare {
            dirdiff.auto_compare();
        }
        let label = create_closeable_tab_label("Directory Comparison", notebook, pages);
        notebook.append_page(dirdiff.widget(), Some(&label.widget));
        pages.borrow_mut().push(Box::new(dirdiff));

        // Save to recent history
        let paths: Vec<String> = gfiles
            .iter()
            .filter_map(|f| f.path().map(|p| p.to_string_lossy().into_owned()))
            .collect();
        save_recent_comparison(RecentType::Folder, &paths);
    } else {
        // For file comparisons, always use at least 2 panes.
        // A single file opens alongside a blank pane for editing.
        let num_panes = gfiles.len().max(2);
        let filediff = FileDiff::new(num_panes);
        filediff.set_font(settings.use_system_font, &settings.custom_font);
        filediff.set_ignore_blanks(settings.ignore_blank_lines);
        filediff.set_show_connectors(settings.show_connectors);
        filediff.set_inline_diff_mode(&settings.inline_diff_mode);
        filediff.apply_settings(settings);
        filediff.connect_gutter_key_modes(window);
        filediff.set_files(gfiles);
        let label = create_closeable_tab_label("File Comparison", notebook, pages);
        notebook.append_page(filediff.widget(), Some(&label.widget));
        pages.borrow_mut().push(Box::new(filediff));

        // Save to recent history
        let paths: Vec<String> = gfiles
            .iter()
            .filter_map(|f| f.path().map(|p| p.to_string_lossy().into_owned()))
            .collect();
        save_recent_comparison(RecentType::File, &paths);
    }
}

fn is_directory_comparison(gfiles: &[gio::File]) -> bool {
    gfiles.iter().any(|f| {
        f.query_file_type(gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)
            == gio::FileType::Directory
    })
}

/// Open a VC file comparison respecting `vc_left_is_local` and
/// `vc_merge_file_order` settings.
///
/// For conflicted files (3-way merge) uses `resolve_merge_order()`.
/// For normal files (2-way diff) uses `resolve_two_pane_order()`.
fn open_vc_file_comparison(
    notebook: &gtk::Notebook,
    pages: &Rc<RefCell<Vec<Box<dyn MeldPage>>>>,
    settings: &MeldSettings,
    repo_root: &str,
    relative_path: &str,
    status: VcFileStatus,
    window: &gtk::ApplicationWindow,
) {
    // Get the VCS backend for this repository
    let vc = match crate::vc::get_vc(repo_root) {
        Ok(v) => v,
        Err(e) => {
            log::error!("Failed to get VC backend for {}: {}", repo_root, e);
            return;
        }
    };

    let working_path = std::path::Path::new(repo_root).join(relative_path);

    if status == VcFileStatus::Conflicted {
        let local_content = vc.get_conflict_path(relative_path, repo_root, ConflictKind::Local);
        let base_content = vc.get_conflict_path(relative_path, repo_root, ConflictKind::Base);
        let remote_content = vc.get_conflict_path(relative_path, repo_root, ConflictKind::Remote);

        // Fall back gracefully if conflict paths aren't available
        let (local_content, base_content, remote_content) =
            match (local_content, base_content, remote_content) {
                (Ok(l), Ok(b), Ok(r)) => (l, b, r),
                _ => {
                    log::warn!(
                        "Could not resolve conflict paths for {}; \
                         falling back to plain file diff",
                        relative_path
                    );
                    // Fall through to 2-way comparison below
                    let files: Vec<gio::File> = vec![gio::File::for_path(&working_path)];
                    let filediff = FileDiff::new(2);
                    filediff.set_font(settings.use_system_font, &settings.custom_font);
                    filediff.set_ignore_blanks(settings.ignore_blank_lines);
                    filediff.set_show_connectors(settings.show_connectors);
                    filediff.set_inline_diff_mode(&settings.inline_diff_mode);
                    filediff.apply_settings(settings);
                    filediff.connect_gutter_key_modes(window);
                    filediff.set_files(&files);
                    filediff.set_labels(&[format!("{} — local", relative_path), String::new()]);
                    let label = create_closeable_tab_label(
                        &format!("{} (working, repository)", relative_path),
                        notebook,
                        pages,
                    );
                    notebook.append_page(filediff.widget(), Some(&label.widget));
                    pages.borrow_mut().push(Box::new(filediff));
                    return;
                }
            };

        // Write VCS content to temp files so FileDiff can read them.
        // Preserve the original file extension so that GtkSourceView's
        // language manager can detect the programming language.
        let ext = std::path::Path::new(relative_path)
            .extension()
            .and_then(|e| e.to_str());
        let tmp_local = write_temp_file(&local_content, "meld-local-", ext);
        let tmp_base = write_temp_file(&base_content, "meld-base-", ext);
        let tmp_remote = write_temp_file(&remote_content, "meld-remote-", ext);

        let (files, labels, tab_label) = match settings.resolve_merge_order() {
            PaneOrder::LocalMergeRemote => (
                vec![
                    gio::File::for_path(&tmp_local),
                    gio::File::for_path(&tmp_base),
                    gio::File::for_path(&tmp_remote),
                ],
                vec![
                    format!("{} — local", relative_path),
                    String::new(),
                    format!("{} — remote", relative_path),
                ],
                format!("{} (local, merge, remote)", relative_path),
            ),
            PaneOrder::RemoteMergeLocal => (
                vec![
                    gio::File::for_path(&tmp_remote),
                    gio::File::for_path(&tmp_base),
                    gio::File::for_path(&tmp_local),
                ],
                vec![
                    format!("{} — remote", relative_path),
                    String::new(),
                    format!("{} — local", relative_path),
                ],
                format!("{} (remote, merge, local)", relative_path),
            ),
            // `resolve_merge_order()` only returns 3-pane variants;
            // 2-pane variants are unreachable here.
            _ => unreachable!("resolve_merge_order returned a 2-pane order in 3-pane context"),
        };

        let filediff = FileDiff::new(3);
        filediff.set_font(settings.use_system_font, &settings.custom_font);
        filediff.set_ignore_blanks(settings.ignore_blank_lines);
        filediff.set_show_connectors(settings.show_connectors);
        filediff.set_inline_diff_mode(&settings.inline_diff_mode);
        filediff.apply_settings(settings);
        filediff.connect_gutter_key_modes(window);
        filediff.set_files(&files);
        filediff.set_labels(&labels);
        filediff.set_merge_output_file(&working_path.to_string_lossy().into_owned());
        let lbl = create_closeable_tab_label(&tab_label, notebook, pages);
        notebook.append_page(filediff.widget(), Some(&lbl.widget));
        pages.borrow_mut().push(Box::new(filediff));
    } else {
        let repo_content = match vc.get_repo_file(relative_path, repo_root) {
            Ok(c) => c,
            Err(e) => {
                log::error!("Failed to get repo file for {}: {}", relative_path, e);
                return;
            }
        };

        let ext = std::path::Path::new(relative_path)
            .extension()
            .and_then(|e| e.to_str());
        let tmp_repo = write_temp_file(&repo_content, "meld-repo-", ext);
        let repo_label = format!("{} — repository", relative_path);

        let (files, labels, tab_label) = match settings.resolve_two_pane_order() {
            PaneOrder::LocalRemote => (
                vec![
                    gio::File::for_path(&working_path),
                    gio::File::for_path(&tmp_repo),
                ],
                vec![String::new(), repo_label],
                format!("{} (working, repository)", relative_path),
            ),
            PaneOrder::RemoteLocal => (
                vec![
                    gio::File::for_path(&tmp_repo),
                    gio::File::for_path(&working_path),
                ],
                vec![repo_label, String::new()],
                format!("{} (repository, working)", relative_path),
            ),
            // 3-pane orders don't apply to 2-way, default to RemoteLocal
            _ => (
                vec![
                    gio::File::for_path(&tmp_repo),
                    gio::File::for_path(&working_path),
                ],
                vec![repo_label, String::new()],
                format!("{} (repository, working)", relative_path),
            ),
        };

        let filediff = FileDiff::new(2);
        filediff.set_font(settings.use_system_font, &settings.custom_font);
        filediff.set_ignore_blanks(settings.ignore_blank_lines);
        filediff.set_show_connectors(settings.show_connectors);
        filediff.set_inline_diff_mode(&settings.inline_diff_mode);
        filediff.apply_settings(settings);
        filediff.connect_gutter_key_modes(window);
        filediff.set_files(&files);
        filediff.set_labels(&labels);
        let lbl = create_closeable_tab_label(&tab_label, notebook, pages);
        notebook.append_page(filediff.widget(), Some(&lbl.widget));
        pages.borrow_mut().push(Box::new(filediff));
    }
}

/// Write `content` to a temporary file with the given prefix.
///
/// If `extension` is provided, it is appended to the temp file name so
/// that GtkSourceView's language manager can detect the programming
/// language from the file extension.
///
/// Returns the path to the temporary file.
fn write_temp_file(content: &str, prefix: &str, extension: Option<&str>) -> std::path::PathBuf {
    let mut tmp = std::env::temp_dir();
    let mut name = prefix.to_owned();
    name.push_str(&uuid::Uuid::new_v4().to_string());
    if let Some(ext) = extension {
        if !ext.is_empty() {
            name.push('.');
            name.push_str(ext);
        }
    }
    tmp.push(&name);
    if let Err(e) = std::fs::write(&tmp, content) {
        log::error!("Failed to write temp file {}: {}", tmp.display(), e);
    }
    // Make read-only to match original Meld behaviour
    if let Ok(meta) = std::fs::metadata(&tmp) {
        let mut perms = meta.permissions();
        perms.set_readonly(true);
        if let Err(e) = std::fs::set_permissions(&tmp, perms) {
            log::warn!("Failed to set read-only on {}: {}", tmp.display(), e);
        }
    }
    tmp
}

/// Central handler for all diff requests (from NewDiffTab, CLI, drag-drop).
/// Respects the `diff_type` chosen by the user rather than inferring from paths.
fn handle_diff_request(
    notebook: &gtk::Notebook,
    pages: &Rc<RefCell<Vec<Box<dyn MeldPage>>>>,
    req: &DiffRequest,
    auto_compare: bool,
    auto_merge: bool,
    settings: &MeldSettings,
    window: &gtk::ApplicationWindow,
) {
    // Collect valid file paths, filtering out None entries (blank slots)
    let gfiles: Vec<gio::File> = req
        .paths
        .iter()
        .filter_map(|opt| opt.as_ref().map(|pb| gio::File::for_path(pb)))
        .collect();

    match req.diff_type {
        DiffType::File => {
            // File comparison: always create FileDiff, regardless of whether
            // paths happen to point to directories.
            let num_panes = if gfiles.is_empty() {
                2
            } else {
                gfiles.len().max(2)
            };
            let fd = FileDiff::new(num_panes);
            fd.set_font(settings.use_system_font, &settings.custom_font);
            fd.set_ignore_blanks(settings.ignore_blank_lines);
            fd.set_show_connectors(settings.show_connectors);
            fd.set_inline_diff_mode(&settings.inline_diff_mode);
            fd.apply_settings(settings);
            fd.connect_gutter_key_modes(window);
            if !gfiles.is_empty() {
                fd.set_files(&gfiles);
            }
            let label = if gfiles.len() <= 1 {
                "File Comparison".to_string()
            } else {
                let names: Vec<String> = gfiles
                    .iter()
                    .filter_map(|f| {
                        f.path()
                            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                    })
                    .collect();
                if names.len() >= 2 {
                    format!("{} vs {}", names[0], names[1])
                } else {
                    "File Comparison".to_string()
                }
            };
            let lbl = create_closeable_tab_label(&label, notebook, pages);
            notebook.append_page(fd.widget(), Some(&lbl.widget));
            pages.borrow_mut().push(Box::new(fd));
            let paths: Vec<String> = gfiles
                .iter()
                .filter_map(|f| f.path().map(|p| p.to_string_lossy().into_owned()))
                .collect();
            save_recent_comparison(RecentType::File, &paths);
        }
        DiffType::Folder => {
            // Folder comparison: always create DirDiff, regardless of actual
            // file types.
            let num_panes = if gfiles.is_empty() {
                2
            } else {
                gfiles.len().max(2)
            };
            let dd = DirDiff::new(num_panes);
            if !gfiles.is_empty() {
                dd.set_folders(&gfiles);
                dd.set_locations();
                if auto_compare {
                    dd.auto_compare();
                }
            }
            let label = if gfiles.len() <= 1 {
                "Directory Comparison".to_string()
            } else {
                let names: Vec<String> = gfiles
                    .iter()
                    .filter_map(|f| {
                        f.path()
                            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                    })
                    .collect();
                if names.len() >= 2 {
                    format!("{} vs {}", names[0], names[1])
                } else {
                    "Directory Comparison".to_string()
                }
            };
            let lbl = create_closeable_tab_label(&label, notebook, pages);
            notebook.append_page(dd.widget(), Some(&lbl.widget));
            pages.borrow_mut().push(Box::new(dd));
            let paths: Vec<String> = gfiles
                .iter()
                .filter_map(|f| f.path().map(|p| p.to_string_lossy().into_owned()))
                .collect();
            save_recent_comparison(RecentType::Folder, &paths);
        }
        DiffType::VersionControl => {
            // Version control view: opens the first path as a VC location.
            if let Some(path) = req.paths.first().and_then(|o| o.as_ref()) {
                let vc = VcView::new();
                vc.set_location(&path.to_string_lossy().to_string());
                let lbl = create_closeable_tab_label("Version Control", notebook, pages);
                notebook.append_page(vc.widget(), Some(&lbl.widget));
                pages.borrow_mut().push(Box::new(vc));
                save_recent_comparison(
                    RecentType::VersionControl,
                    &[path.to_string_lossy().to_string()],
                );
            }
        }
        DiffType::Unselected => {
            // User didn't select a comparison type — do nothing.
        }
    }
}

/// Standalone version of `wire_new_diff_tab` for use in closures
/// where `self` is not available.
fn wire_new_diff_tab_standalone(
    tab: &dyn MeldPage,
    notebook: &gtk::Notebook,
    pages: &Rc<RefCell<Vec<Box<dyn MeldPage>>>>,
    settings: &MeldSettings,
    window: &gtk::ApplicationWindow,
) {
    let nb = notebook.clone();
    let p = Rc::clone(pages);
    let s = Rc::new(settings.clone());
    let w = window.clone();
    tab.set_diff_created_callback(Box::new(move |req: DiffRequest| {
        let auto_compare = false;
        let auto_merge = false;
        handle_diff_request(&nb, &p, &req, auto_compare, auto_merge, &s, &w);

        // Remove NewDiffTab on the next idle cycle
        let p_clone = Rc::clone(&p);
        let nb_clone = nb.clone();
        glib::idle_add_local(move || {
            let to_remove: Vec<usize> = {
                let mut pages = p_clone.borrow_mut();
                let mut indices = Vec::new();
                for (i, page) in pages.iter().enumerate() {
                    if page.label() == "New comparison" {
                        indices.push(i);
                    }
                }
                for &idx in indices.iter().rev() {
                    pages.remove(idx);
                }
                indices
            };
            for idx in to_remove.iter().rev() {
                nb_clone.remove_page(Some(*idx as u32));
            }
            glib::ControlFlow::Break
        });
    }));
}

/// Create a tab label whose close button actually closes the tab.
///
/// The close button triggers [`MeldPage::close`] and, if the page agrees
/// (returns [`gtk::ResponseType::Ok`]), removes the tab from the notebook.
fn create_closeable_tab_label(
    text: &str,
    notebook: &gtk::Notebook,
    pages: &Rc<RefCell<Vec<Box<dyn MeldPage>>>>,
) -> TabLabel {
    let label = TabLabel::new(text);
    let nb = notebook.clone();
    let p = Rc::clone(pages);
    let label_widget = label.widget.clone();

    label.connect_close(move || {
        // Find the page by matching our tab label widget against
        // each page's tab label.  `page_num()` looks up by child
        // widget, not by tab widget, so we iterate instead.
        let n = nb.n_pages() as i32;
        for i in 0..n {
            if let Some(child) = nb.nth_page(Some(i as u32)) {
                if let Some(tab) = nb.tab_label(&child) {
                    if tab == label_widget {
                        let resp = p
                            .borrow()
                            .get(i as usize)
                            .map(|pg| pg.close())
                            .unwrap_or(gtk::ResponseType::Cancel);
                        if resp == gtk::ResponseType::Ok {
                            p.borrow_mut().remove(i as usize);
                            nb.remove_page(Some(i as u32));
                        }
                        break;
                    }
                }
            }
        }
    });

    label
}

/// Save a comparison to the recent history.
fn save_recent_comparison(comparison_type: RecentType, paths: &[String]) {
    if paths.is_empty() {
        return;
    }
    if let Ok(mut mgr) = crate::config::recent::RecentManager::load() {
        mgr.add(crate::config::recent::RecentEntry {
            comparison_type,
            paths: paths.to_vec(),
            label: None,
        });
        if let Err(e) = mgr.save() {
            log::warn!("Failed to save recent comparisons: {e}");
        } else {
            log::info!("Saved recent comparison: {paths:?}");
        }
    } else {
        log::warn!("Failed to load recent manager");
    }
}

/// Build the complete gear menu matching `menus.ui` from the original Meld.
fn build_gear_menu() -> gio::Menu {
    let menu = gio::Menu::new();

    // File section
    let file_section = gio::Menu::new();
    file_section.append(Some("Save As..."), Some("view.save-as"));
    file_section.append(Some("Save A_ll"), Some("view.save-all"));
    file_section.append(Some("Revert Files..."), Some("view.revert"));
    file_section.append(Some("_Open Externally"), Some("view.open-external"));
    menu.append_section(None, &file_section);

    // Refresh section
    let refresh_section = gio::Menu::new();
    refresh_section.append(Some("Refresh Comparison"), Some("view.refresh"));
    menu.append_section(None, &refresh_section);

    // Find section
    let find_section = gio::Menu::new();
    find_section.append(Some("_Find..."), Some("view.find"));
    find_section.append(Some("_Replace..."), Some("view.find-replace"));
    menu.append_section(None, &find_section);

    // View submenu with sections matching the original menus.ui
    let view_sub = gio::Menu::new();
    let view_section = gio::Menu::new();
    view_section.append(Some("Fullscreen"), Some("win.fullscreen"));
    view_section.append(Some("Overview Map"), Some("view.show-overview-map"));
    view_section.append(
        Some("Version Control Console"),
        Some("view.vc-console-visible"),
    );
    view_section.append(Some("Lock Scrolling"), Some("view.lock-scrolling"));
    view_sub.append_section(None, &view_section);
    let swap_section = gio::Menu::new();
    swap_section.append(Some("Swap Left and Right Panes"), Some("view.swap-2-panes"));
    view_sub.append_section(None, &swap_section);
    menu.append_submenu(Some("_View"), &view_sub);

    // Comparison submenu with sections matching the original menus.ui
    let cmp_sub = gio::Menu::new();
    cmp_sub.append(Some("_Stop"), Some("win.stop"));
    let merge_section = gio::Menu::new();
    merge_section.append(Some("Merge All from _Left"), Some("view.merge-all-left"));
    merge_section.append(Some("Merge All from _Right"), Some("view.merge-all-right"));
    merge_section.append(Some("Merge _All"), Some("view.merge-all"));
    cmp_sub.append_section(None, &merge_section);
    let tool_section = gio::Menu::new();
    tool_section.append(Some("Format as _Patch..."), Some("view.format-as-patch"));
    cmp_sub.append_section(None, &tool_section);
    menu.append_submenu(Some("_Comparison"), &cmp_sub);

    // Application section
    let app_section = gio::Menu::new();
    app_section.append(Some("_Preferences"), Some("win.preferences"));
    app_section.append(Some("Keyboard Shortcuts"), Some("win.show-help-overlay"));
    app_section.append(Some("_Help"), Some("app.help"));
    app_section.append(Some("_About Meld"), Some("app.about"));
    menu.append_section(None, &app_section);

    menu
}
