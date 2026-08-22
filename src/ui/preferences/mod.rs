#![cfg(feature = "gui")]
//! Full preferences dialog — an Adwaita replica of the original Meld's
//! `preferences.ui` / `preferences.py`.
//!
//! The dialog is an `Adw.PreferencesDialog` with four `Adw.PreferencesPage`s
//! (Editor, Folder Comparison, Version Control, Filters), each holding
//! `Adw.PreferencesGroup`s of rows, mirroring the original layout.
//!
//! Settings are applied in real time: every widget handler mutates the
//! settings, saves them, and fires the change callback — matching the
//! original Meld's live GSettings binding behaviour (no OK/Cancel).

mod column_list;
mod filter_list;

use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use sourceview5 as gsv;
use sourceview5::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use crate::config::settings::MeldSettings;

/// Callback invoked every time a setting is modified in the dialog.
/// The dialog saves to disk before calling this, so `apply_settings`
/// can simply read from the shared `RefCell` or reload from disk.
pub type SettingsChangedCallback = Box<dyn Fn()>;

/// Closures invoked when the dialog closes, to flush uncommitted edits.
type FlushSink = Rc<RefCell<Vec<Box<dyn Fn()>>>>;

/// Full preferences dialog with search and four Adwaita preferences pages.
/// Modeless — settings take effect immediately as the user changes them.
pub struct PreferencesDialog {
    dialog: adw::PreferencesDialog,
    parent: Option<gtk::Window>,
    settings: Rc<RefCell<MeldSettings>>,
}

impl PreferencesDialog {
    /// Create a new preferences dialog.
    ///
    /// `on_changed` is called after every setting modification so that
    /// the caller can apply the new values to open documents.
    /// `parent` is the window used to present the dialog.
    pub fn new(on_changed: SettingsChangedCallback, parent: Option<&gtk::Window>) -> Self {
        let settings = MeldSettings::load().unwrap_or_default();
        let settings_rc = Rc::new(RefCell::new(settings));
        let on_changed_rc = Rc::new(RefCell::new(Some(on_changed)));
        let ctx = Rc::new(SettingsCtx {
            settings: Rc::clone(&settings_rc),
            on_changed: on_changed_rc,
        });

        let dialog = adw::PreferencesDialog::new();
        dialog.set_title("Preferences");
        dialog.set_search_enabled(true);
        dialog.set_width_request(800);

        // Flush callbacks for widgets holding uncommitted text edits
        // (filter name/pattern entries commit on Enter or focus loss).
        let flushes: FlushSink = Rc::new(RefCell::new(Vec::new()));
        // Objects that must stay alive for as long as the dialog does
        // (e.g. ColumnList, whose row handlers hold weak references).
        let keep_alive: Rc<RefCell<Vec<Box<dyn std::any::Any>>>> =
            Rc::new(RefCell::new(Vec::new()));

        dialog.add(&build_editor_page(&ctx));
        dialog.add(&build_folder_page(&ctx, &keep_alive));
        dialog.add(&build_vc_page(&ctx));
        dialog.add(&build_filters_page(&ctx, &flushes));

        // Save on close (belt-and-suspenders — every change already saves).
        let s = Rc::clone(&settings_rc);
        dialog.connect_closed(move |_| {
            for flush in flushes.borrow().iter() {
                flush();
            }
            let _ = s.borrow().save();
            // Release the keep-alive objects now that the dialog is gone.
            keep_alive.borrow_mut().clear();
        });

        Self {
            dialog,
            parent: parent.cloned(),
            settings: settings_rc,
        }
    }

    /// Show the dialog.
    pub fn present(&self) {
        self.dialog.present(self.parent.as_ref());
    }

    /// Returns the shared settings being edited by this dialog.
    pub fn settings(&self) -> &Rc<RefCell<MeldSettings>> {
        &self.settings
    }
}

// ── Shared settings handle ─────────────────────────────────────────

/// Shared handle for persisting + notifying on every setting change.
///
/// Mirrors the original Meld's live GSettings binding behaviour: each widget
/// handler mutates the settings, saves them, and fires the change callback.
struct SettingsCtx {
    settings: Rc<RefCell<MeldSettings>>,
    on_changed: Rc<RefCell<Option<SettingsChangedCallback>>>,
}

impl SettingsCtx {
    /// Apply `f` to the settings, then persist and notify.
    fn update(&self, f: impl FnOnce(&mut MeldSettings)) {
        {
            let mut s = self.settings.borrow_mut();
            f(&mut s);
        }
        self.notify();
    }

    fn notify(&self) {
        let _ = self.settings.borrow().save();
        if let Some(ref cb) = *self.on_changed.borrow() {
            cb();
        }
    }
}

// ── Widget helpers ─────────────────────────────────────────────────

/// Build an `Adw.SwitchRow` that invokes `on_toggle` when the user
/// flips the switch.
fn switch_row(
    title: &str,
    subtitle: Option<&str>,
    active: bool,
    on_toggle: impl Fn(bool) + 'static,
) -> adw::SwitchRow {
    let row = adw::SwitchRow::new();
    row.set_title(title);
    if let Some(subtitle) = subtitle {
        row.set_subtitle(subtitle);
    }
    row.set_active(active);
    row.connect_active_notify(move |row| on_toggle(row.is_active()));
    row
}

/// Build an `Adw.ComboRow` listing `options` as `(id, label)` pairs.
///
/// The model holds the human-readable labels; `on_select` receives the id
/// of the newly selected entry.
fn combo_row(
    title: &str,
    subtitle: Option<&str>,
    options: &[(&'static str, &'static str)],
    selected_id: &str,
    on_select: impl Fn(&str) + 'static,
) -> adw::ComboRow {
    let row = adw::ComboRow::new();
    row.set_title(title);
    if let Some(subtitle) = subtitle {
        row.set_subtitle(subtitle);
    }
    let labels: Vec<&str> = options.iter().map(|(_, label)| *label).collect();
    let model = gtk::StringList::new(&labels);
    row.set_model(Some(&model));
    let ids: Vec<&'static str> = options.iter().map(|(id, _)| *id).collect();
    let selected = options
        .iter()
        .position(|(id, _)| *id == selected_id)
        .unwrap_or(0) as u32;
    row.set_selected(selected);
    row.connect_selected_notify(move |row| {
        let index = row.selected() as usize;
        if let Some(id) = ids.get(index) {
            on_select(id);
        }
    });
    row
}

/// Build an `Adw.ExpanderRow` with an enable switch, keeping its expanded
/// state in sync with the switch (mirrors the original's bidirectional
/// `sync-create` binding) and invoking `on_toggle` on changes.
fn expander_row(
    title: &str,
    subtitle: Option<&str>,
    enabled: bool,
    on_toggle: impl Fn(bool) + 'static,
) -> adw::ExpanderRow {
    let row = adw::ExpanderRow::new();
    row.set_show_enable_switch(true);
    row.set_title(title);
    if let Some(subtitle) = subtitle {
        row.set_subtitle(subtitle);
    }
    row.set_enable_expansion(enabled);
    row.set_expanded(enabled);
    row.connect_enable_expansion_notify(move |row| {
        let enabled = row.enables_expansion();
        row.set_expanded(enabled);
        on_toggle(enabled);
    });
    row.connect_expanded_notify(|row| {
        row.set_enable_expansion(row.is_expanded());
    });
    row
}

// ── Page builders ──────────────────────────────────────────────────

// GtkFontButton (and its GtkFontChooser interface methods) is deprecated
// since GTK 4.10 in favour of GtkFontDialogButton, but the original Meld
// `preferences.ui` uses it and it remains fully functional.
#[allow(deprecated)]
fn build_editor_page(ctx: &Rc<SettingsCtx>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.set_title("Editor");
    page.set_icon_name(Some("text-editor-symbolic"));

    // ── Appearance ──
    let appearance = adw::PreferencesGroup::new();
    appearance.set_title("Appearance");

    let style_variant = ctx.settings.borrow().style_variant.clone();
    let c = Rc::clone(ctx);
    appearance.add(&combo_row(
        "Interface Style",
        None,
        &[
            ("default", "Follow System"),
            ("force-light", "Light"),
            ("force-dark", "Dark"),
        ],
        &style_variant,
        move |id| {
            // Apply the colour scheme first so the settings callback (which
            // re-applies the GtkSourceView style scheme to open documents)
            // observes the new appearance.
            crate::ui::style::apply_style_variant(id);
            c.update(|s| s.style_variant = id.to_string());
        },
    ));

    let custom_font = ctx.settings.borrow().custom_font.clone();
    let use_system_font = ctx.settings.borrow().use_system_font;
    let c = Rc::clone(ctx);
    let font_expander = expander_row(
        "Use Custom Font",
        Some("Choose a font different to the system default monospace font"),
        !use_system_font,
        move |enabled| c.update(|s| s.use_system_font = !enabled),
    );
    let font_row = adw::ActionRow::new();
    font_row.set_title("Font");
    font_row.set_subtitle("The font used within the editor");
    let font_button = gtk::FontButton::new();
    font_button.set_use_font(true);
    font_button.set_font(&custom_font);
    font_button.set_valign(gtk::Align::Center);
    font_row.add_suffix(&font_button);
    font_expander.add_row(&font_row);
    let c = Rc::clone(ctx);
    font_button.connect_font_notify(move |button| {
        if let Some(font) = button.font() {
            c.update(|s| s.custom_font = font.to_string());
        }
    });
    appearance.add(&font_expander);

    let highlight_syntax = ctx.settings.borrow().highlight_syntax;
    let style_scheme = ctx.settings.borrow().style_scheme.clone();
    let c = Rc::clone(ctx);
    let syntax_expander = expander_row(
        "Use Syntax Highlighting",
        None,
        highlight_syntax,
        move |enabled| c.update(|s| s.highlight_syntax = enabled),
    );
    let scheme_row = adw::ActionRow::new();
    scheme_row.set_title("Color Scheme");
    let chooser = gsv::StyleSchemeChooserButton::new();
    chooser.set_valign(gtk::Align::Center);
    chooser.add_css_class("flat");
    let manager = gsv::StyleSchemeManager::default();
    if let Some(scheme) = manager.scheme(&style_scheme) {
        chooser.set_style_scheme(&scheme);
    }
    scheme_row.add_suffix(&chooser);
    syntax_expander.add_row(&scheme_row);
    let c = Rc::clone(ctx);
    chooser.connect_style_scheme_notify(move |chooser| {
        c.update(|s| s.style_scheme = chooser.style_scheme().id().to_string());
    });
    appearance.add(&syntax_expander);

    page.add(&appearance);

    // ── Wrap Text ──
    let wrap_group = adw::PreferencesGroup::new();
    let wrap_mode = ctx.settings.borrow().wrap_mode.clone();
    let c = Rc::clone(ctx);
    wrap_group.add(&combo_row(
        "Wrap Text",
        Some("How text should be wrapped when wider than the pane"),
        &[
            ("none", "Never"),
            ("word", "At Spaces"),
            ("char", "Anywhere"),
        ],
        &wrap_mode,
        move |id| c.update(|s| s.wrap_mode = id.to_string()),
    ));
    page.add(&wrap_group);

    // ── Highlighting toggles ──
    let toggles = adw::PreferencesGroup::new();
    let highlight_current_line = ctx.settings.borrow().highlight_current_line;
    let c = Rc::clone(ctx);
    toggles.add(&switch_row(
        "Highlight Current Line",
        Some("Make the current line stand out with highlights"),
        highlight_current_line,
        move |on| c.update(|s| s.highlight_current_line = on),
    ));
    let show_line_numbers = ctx.settings.borrow().show_line_numbers;
    let c = Rc::clone(ctx);
    toggles.add(&switch_row(
        "Show Line Numbers",
        Some("Display line numbers next to each line of text"),
        show_line_numbers,
        move |on| c.update(|s| s.show_line_numbers = on),
    ));
    page.add(&toggles);

    // ── Show whitespace ──
    let whitespace_group = adw::PreferencesGroup::new();
    let enable_space_drawer = ctx.settings.borrow().enable_space_drawer;
    let c = Rc::clone(ctx);
    whitespace_group.add(&switch_row(
        "Show whitespace",
        None,
        enable_space_drawer,
        move |on| c.update(|s| s.enable_space_drawer = on),
    ));
    page.add(&whitespace_group);

    // ── Indentation ──
    let indentation = adw::PreferencesGroup::new();
    indentation.set_title("Indentation");
    let spaces = ctx.settings.borrow().insert_spaces_instead_of_tabs;
    let tab_id = if spaces { "spaces" } else { "tab" };
    let c = Rc::clone(ctx);
    indentation.add(&combo_row(
        "Tab character",
        Some("The character to be inserted for Tab"),
        &[("tab", "Tab"), ("spaces", "Spaces")],
        tab_id,
        move |id| c.update(|s| s.insert_spaces_instead_of_tabs = id == "spaces"),
    ));
    let indent_width = ctx.settings.borrow().indent_width;
    let c = Rc::clone(ctx);
    let indent_spin = adw::SpinRow::with_range(1.0, 8.0, 1.0);
    indent_spin.set_title("Indentation Size");
    indent_spin.set_subtitle("The number of characters to indent");
    indent_spin.set_value(indent_width as f64);
    indent_spin.connect_value_notify(move |spin| {
        c.update(|s| s.indent_width = spin.value() as i32);
    });
    indentation.add(&indent_spin);
    page.add(&indentation);

    // ── Code Overview ──
    let overview = adw::PreferencesGroup::new();
    overview.set_title("Code Overview");
    let show_overview_map = ctx.settings.borrow().show_overview_map;
    let c = Rc::clone(ctx);
    overview.add(&switch_row(
        "Show overview map",
        None,
        show_overview_map,
        move |on| c.update(|s| s.show_overview_map = on),
    ));
    page.add(&overview);

    // ── External Editor ──
    let external = adw::PreferencesGroup::new();
    external.set_title("External Editor");
    let use_system_editor = ctx.settings.borrow().use_system_editor;
    let editor_command = ctx.settings.borrow().custom_editor_command.clone();
    let c = Rc::clone(ctx);
    let editor_expander = expander_row(
        "Use custom text editor",
        Some("Choose an external text editor different to the system default"),
        !use_system_editor,
        move |enabled| c.update(|s| s.use_system_editor = !enabled),
    );
    let command_row = adw::EntryRow::new();
    command_row.set_title("Custom Editor Command");
    command_row.set_text(&editor_command);
    editor_expander.add_row(&command_row);
    let c = Rc::clone(ctx);
    command_row.connect_changed(move |entry| {
        c.update(|s| s.custom_editor_command = entry.text().to_string());
    });
    external.add(&editor_expander);
    page.add(&external);

    page
}

fn build_folder_page(
    ctx: &Rc<SettingsCtx>,
    keep_alive: &Rc<RefCell<Vec<Box<dyn std::any::Any>>>>,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.set_title("Folder Comparison");
    page.set_icon_name(Some("folder-symbolic"));

    // ── Comparison Options ──
    let options = adw::PreferencesGroup::new();
    options.set_title("Comparison Options");
    let shallow = ctx.settings.borrow().folder_shallow_comparison;
    let c = Rc::clone(ctx);
    let shallow_expander = expander_row(
        "Compare files based only on size and timestamp",
        None,
        shallow,
        move |enabled| c.update(|s| s.folder_shallow_comparison = enabled),
    );
    let resolution = ctx.settings.borrow().folder_time_resolution;
    let resolution_id = match resolution {
        1 => "one_ns",
        100 => "onehundred_ns",
        1_000_000_000 => "one_s",
        2_000_000_000 => "two_s",
        -1 => "ignore",
        _ => "onehundred_ns",
    };
    let c = Rc::clone(ctx);
    let resolution_row = combo_row(
        "Timestamp resolution",
        None,
        &[
            ("one_ns", "1ns (ext4)"),
            ("onehundred_ns", "100ns (NTFS)"),
            ("one_s", "1s (ext2/ext3)"),
            ("two_s", "2s (VFAT)"),
            ("ignore", "Ignore timestamp"),
        ],
        resolution_id,
        move |id| {
            let value = match id {
                "one_ns" => 1,
                "onehundred_ns" => 100,
                "one_s" => 1_000_000_000,
                "two_s" => 2_000_000_000,
                _ => -1,
            };
            c.update(|s| s.folder_time_resolution = value);
        },
    );
    shallow_expander.add_row(&resolution_row);
    options.add(&shallow_expander);
    page.add(&options);

    // ── Ignore symbolic links ──
    let symlinks_group = adw::PreferencesGroup::new();
    let ignore_symlinks = ctx.settings.borrow().folder_ignore_symlinks;
    let c = Rc::clone(ctx);
    symlinks_group.add(&switch_row(
        "Ignore symbolic links",
        None,
        ignore_symlinks,
        move |on| c.update(|s| s.folder_ignore_symlinks = on),
    ));
    page.add(&symlinks_group);

    // ── Apply text filters during folder comparisons ──
    let filter_group = adw::PreferencesGroup::new();
    let filter_text = ctx.settings.borrow().folder_filter_text;
    let c = Rc::clone(ctx);
    let filter_switch = switch_row(
        "Apply text filters during folder comparisons",
        Some("Enabling text filters will make comparing large files much slower"),
        filter_text,
        move |on| c.update(|s| s.folder_filter_text = on),
    );
    // Disabled while shallow comparison is active (mirrors the original's
    // inverted sensitivity binding).
    filter_switch.set_sensitive(!shallow);
    let filter_switch_weak = filter_switch.downgrade();
    shallow_expander.connect_enable_expansion_notify(move |row| {
        let enabled = row.enables_expansion();
        if let Some(switch) = filter_switch_weak.upgrade() {
            switch.set_sensitive(!enabled);
        }
    });
    filter_group.add(&filter_switch);
    page.add(&filter_group);

    // ── Visible Columns ──
    let columns_group = adw::PreferencesGroup::new();
    columns_group.set_title("Visible Columns");
    let columns = ctx.settings.borrow().folder_columns.clone();
    let c = Rc::clone(ctx);
    let column_list = column_list::ColumnList::new(&columns, move |columns| {
        c.update(|s| s.folder_columns = columns);
    });
    columns_group.add(&column_list.widget());
    // Keep the ColumnList alive for as long as the dialog (its row
    // handlers hold weak references to its bookkeeping state).
    keep_alive.borrow_mut().push(Box::new(column_list));
    page.add(&columns_group);

    page
}

fn build_vc_page(ctx: &Rc<SettingsCtx>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.set_title("Version Control");
    page.set_icon_name(Some("git-symbolic"));

    // ── Version Comparisons ──
    let comparisons = adw::PreferencesGroup::new();
    comparisons.set_title("Version Comparisons");
    let left_is_local = ctx.settings.borrow().vc_left_is_local;
    let order_id = if left_is_local { "llrr" } else { "lrrl" };
    let c = Rc::clone(ctx);
    comparisons.add(&combo_row(
        "Order when Comparing File Revisions",
        None,
        &[
            ("lrrl", "Left is remote, right is local"),
            ("llrr", "Left is local, right is remote"),
        ],
        order_id,
        move |id| c.update(|s| s.vc_left_is_local = id == "llrr"),
    ));
    let merge_order = ctx.settings.borrow().vc_merge_file_order.clone();
    let c = Rc::clone(ctx);
    comparisons.add(&combo_row(
        "Order when Merging Files",
        None,
        &[
            ("remote-merge-local", "Remote, merge, local"),
            ("local-merge-remote", "Local, merge, remote"),
        ],
        &merge_order,
        move |id| c.update(|s| s.vc_merge_file_order = id.to_string()),
    ));
    page.add(&comparisons);

    // ── Commit Message ──
    let commit = adw::PreferencesGroup::new();
    commit.set_title("Commit Message");
    let show_margin = ctx.settings.borrow().vc_show_commit_margin;
    let c = Rc::clone(ctx);
    let margin_switch = switch_row("Show right margin", None, show_margin, move |on| {
        c.update(|s| s.vc_show_commit_margin = on);
    });
    let commit_margin = ctx.settings.borrow().vc_commit_margin;
    let c = Rc::clone(ctx);
    let margin_spin = adw::SpinRow::with_range(70.0, 120.0, 1.0);
    margin_spin.set_title("Margin position");
    margin_spin.set_value(commit_margin as f64);
    margin_spin.set_sensitive(show_margin);
    margin_spin.connect_value_notify(move |spin| {
        c.update(|s| s.vc_commit_margin = spin.value() as i32);
    });
    let break_lines = ctx.settings.borrow().vc_break_commit_message;
    let c = Rc::clone(ctx);
    let break_switch = switch_row(
        "Automatically break lines at right margin on commit",
        None,
        break_lines,
        move |on| c.update(|s| s.vc_break_commit_message = on),
    );
    break_switch.set_sensitive(show_margin);
    // Sensitivity wiring (mirrors the original's value-before-sensitivity
    // GSettings bindings).
    let margin_spin_weak = margin_spin.downgrade();
    let break_switch_weak = break_switch.downgrade();
    margin_switch.connect_active_notify(move |row| {
        let active = row.is_active();
        if let Some(spin) = margin_spin_weak.upgrade() {
            spin.set_sensitive(active);
        }
        if let Some(switch) = break_switch_weak.upgrade() {
            switch.set_sensitive(active);
        }
    });
    commit.add(&margin_switch);
    commit.add(&margin_spin);
    commit.add(&break_switch);
    page.add(&commit);

    page
}

fn build_filters_page(ctx: &Rc<SettingsCtx>, flushes: &FlushSink) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.set_title("Filters");
    page.set_icon_name(Some("view-list-symbolic"));

    // ── Filename filters ──
    let file_group = adw::PreferencesGroup::new();
    file_group.set_title("Filename filters");
    file_group.set_description(Some(
        "When performing directory comparisons, you may filter out files and \
         directories by name. Each pattern is a list of shell style wildcards \
         separated by spaces.",
    ));
    let filename_filters = ctx.settings.borrow().filename_filters.clone();
    let c = Rc::clone(ctx);
    let file_filters = filter_list::FilterList::new(
        &filename_filters,
        true,
        move |entries| {
            c.update(|s| s.filename_filters = entries);
        },
        Some(flushes),
    );
    file_filters.widget().set_vexpand(true);
    file_group.add(&file_filters.widget());
    page.add(&file_group);

    // ── Change trimming ──
    let trimming = adw::PreferencesGroup::new();
    trimming.set_title("Change trimming");
    let ignore_blank_lines = ctx.settings.borrow().ignore_blank_lines;
    let c = Rc::clone(ctx);
    trimming.add(&switch_row(
        "Trim blank line differences from the start and end of changes",
        Some(
            "Removing blank lines can simplify comparisons, and can combine \
             with text filters to fully remove changes.",
        ),
        ignore_blank_lines,
        move |on| c.update(|s| s.ignore_blank_lines = on),
    ));
    page.add(&trimming);

    // ── Text filters ──
    let text_group = adw::PreferencesGroup::new();
    text_group.set_title("Text filters");
    text_group.set_description(Some(
        "When performing file comparisons, you may ignore certain types of \
         changes. Each pattern here is a regular expression which replaces \
         matching text with the empty string before comparison is performed. \
         If the expression contains groups, only the groups are replaced. \
         See the user manual for more details.",
    ));
    let text_filters = ctx.settings.borrow().text_filters.clone();
    let c = Rc::clone(ctx);
    let text_filters_widget = filter_list::FilterList::new(
        &text_filters,
        false,
        move |entries| {
            c.update(|s| s.text_filters = entries);
        },
        Some(flushes),
    );
    text_filters_widget.widget().set_vexpand(true);
    text_group.add(&text_filters_widget.widget());
    page.add(&text_group);

    page
}
