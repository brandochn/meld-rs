#![cfg(feature = "gui")]
//! Application entry with full gear menu, about dialog, and shortcuts overlay.
//!
//! Matches `menus.ui` (186 lines), `about-dialog.ui`, and `help-overlay.ui`.

use glib::prelude::*;
use gtk4 as gtk;
use gtk4::prelude::*;
use std::cell::Cell;
use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;

use crate::window::MeldWindow;

/// Compiled GResource providing the missing `language2.rng` RelaxNG schema.
#[cfg(gresource_available)]
const LANGUAGE_SCHEMA_GRESOURCE: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/meld-language-schema.gresource"));

pub const APP_ID: &str = "org.gnome.meld-rs";
pub const APP_NAME: &str = "Meld-rs";
pub const RESOURCE_BASE: &str = "/org/gnome/meld-rs";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonMode {
    Compare,
    AutoMerge,
}

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("Too many arguments (wanted 0–3, got {0})")]
    TooManyArgs(usize),
    #[error("Cannot auto-merge fewer than 3 files")]
    AutoMergeNeeds3Files,
    #[error("Cannot auto-merge directories")]
    AutoMergeNoDirectories,
    #[error("{0}")]
    ParseError(String),
}

#[derive(Debug, Default)]
pub struct CliOptions {
    pub paths: Vec<String>,
    pub labels: Vec<String>,
    pub new_tab: bool,
    pub auto_compare: bool,
    pub output: Option<String>,
    pub auto_merge: bool,
    pub diff_sets: Vec<Vec<String>>,
}

fn parse_args(args: &[String]) -> Result<CliOptions, CliError> {
    let mut opts = CliOptions::default();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-L" | "--label" => {
                i += 1;
                if i < args.len() {
                    opts.labels.push(args[i].clone());
                }
            }
            "-n" | "--newtab" => opts.new_tab = true,
            "-a" | "--auto-compare" => opts.auto_compare = true,
            "-o" | "--output" => {
                i += 1;
                opts.output = args.get(i).cloned();
            }
            "--auto-merge" => opts.auto_merge = true,
            "--diff" => {
                let mut diff_args = Vec::new();
                i += 1;
                while i < args.len() && !args[i].starts_with('-') {
                    diff_args.push(args[i].clone());
                    i += 1;
                }
                if diff_args.len() < 1 || diff_args.len() > 3 {
                    return Err(CliError::ParseError(
                        "wrong number of arguments supplied to --diff".into(),
                    ));
                }
                if !diff_args.is_empty() {
                    i -= 1;
                }
                opts.diff_sets.push(diff_args);
            }
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            "-v" | "--version" => {
                println!("meld-rs {VERSION}");
                std::process::exit(0);
            }
            "-u" | "--unified" => {}
            arg if !arg.starts_with('-') => opts.paths.push(arg.to_string()),
            other => return Err(CliError::ParseError(format!("unknown option: {other}"))),
        }
        i += 1;
    }
    if opts.paths.len() > 3 {
        return Err(CliError::TooManyArgs(opts.paths.len()));
    }
    if opts.auto_merge
        && opts
            .paths
            .iter()
            .chain(opts.diff_sets.iter().flatten())
            .count()
            < 3
    {
        return Err(CliError::AutoMergeNeeds3Files);
    }
    if opts.auto_merge
        && opts
            .paths
            .iter()
            .chain(opts.diff_sets.iter().flatten())
            .any(|p| Path::new(p).is_dir())
    {
        return Err(CliError::AutoMergeNoDirectories);
    }
    Ok(opts)
}

fn print_usage() {
    println!("Meld-rs — visual diff and merge tool (Rust rewrite)");
    println!();
    println!("Usage:");
    println!("  meld-rs                               Start with an empty window");
    println!("  meld-rs <file|folder>                 Start a version control comparison");
    println!("  meld-rs <file> <file> [<file>]        Start a 2- or 3-way file comparison");
    println!("  meld-rs <folder> <folder> [<folder>]  Start a 2- or 3-way folder comparison");
    println!();
    println!("Options:");
    println!("  -L, --label <label>    Set label to use instead of file name");
    println!("  -n, --newtab           Open a new tab in an already running instance");
    println!("  -a, --auto-compare     Automatically compare all differing files");
    println!("  -o, --output <file>    Set the target file for saving a merge result");
    println!("  --auto-merge           Automatically merge files");
    println!("  --diff <file>...       Create a diff tab for the supplied files or folders");
    println!("  -h, --help             Show this help message");
    println!("  -v, --version          Show version information");
}

pub struct MeldApp {
    app: gtk::Application,
}

impl MeldApp {
    pub fn new() -> Self {
        let app = gtk::Application::new(Some(APP_ID), gio::ApplicationFlags::HANDLES_COMMAND_LINE);
        glib::set_application_name(APP_NAME);
        glib::set_prgname(Some(APP_ID));
        gtk::Window::set_default_icon_name(APP_ID);

        let app_weak = app.downgrade();
        app.connect_window_removed(move |app, _| {
            if app.windows().iter().count() == 0 {
                app.quit();
            }
        });

        Self { app }
    }

    pub fn run_with_args(&self, args: &[String]) -> ExitCode {
        let initialized = Rc::new(Cell::new(false));

        // HANDLES_COMMAND_LINE tells GLib not to process the command line
        // itself. The `command-line` signal is emitted with the raw args
        // and we handle all CLI parsing here.
        let init_cl = Rc::clone(&initialized);
        self.app.connect_command_line(move |app, cmd_line| {
            ensure_initialized(app, &init_cl);

            let cmd_args: Vec<String> = cmd_line
                .arguments()
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();

            log::info!("command-line args: {:?}", &cmd_args);

            let opts = match parse_args(&cmd_args) {
                Ok(o) => o,
                Err(e) => {
                    log::error!("CLI parse error: {e}");
                    crate::log_diag(&format!("Error parsing arguments: {e}"));
                    cmd_line.set_exit_status(2);
                    return glib::ExitCode::from(2);
                }
            };

            open_comparisons(app, &opts);
            glib::ExitCode::SUCCESS
        });

        // Activate is still emitted for D-Bus activation (desktop menu, etc.).
        let init_ac = Rc::clone(&initialized);
        self.app.connect_activate(move |app| {
            ensure_initialized(app, &init_ac);
            if app.windows().iter().count() == 0 {
                let window = MeldWindow::new(app);
                window.present();
            }
        });

        // Pass explicit args to GLib so the `command-line` signal receives
        // them reliably on all platforms (especially Windows GUI-subsystem).
        let string_args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let exit_code = self.app.run_with_args(&string_args);

        if exit_code == glib::ExitCode::SUCCESS {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        }
    }
}

fn setup_actions(app: &gtk::Application) {
    let app_weak = app.downgrade();
    let quit = gio::SimpleAction::new("quit", None);
    quit.connect_activate(move |_, _| {
        if let Some(a) = app_weak.upgrade() {
            a.quit();
        }
    });
    app.add_action(&quit);
    app.set_accels_for_action("app.quit", &["<Ctrl>Q"]);

    let about = gio::SimpleAction::new("about", None);
    about.connect_activate(|_, _| {
        show_about_dialog();
    });
    app.add_action(&about);

    let prefs = gio::SimpleAction::new("preferences", None);
    let app_w = app.downgrade();
    prefs.connect_activate(move |_, _| {
        // Preferences would be shown here
    });
    app.add_action(&prefs);
}

/// Run one-time application setup (actions, CSS, style schemes).
///
/// Guarded by a [`Cell<bool>`] so it is only executed once, whether the
/// app is launched via the `command-line` signal or D-Bus `activate`.
fn ensure_initialized(app: &gtk::Application, done: &Cell<bool>) {
    if done.get() {
        return;
    }
    setup_actions(app);
    setup_css();
    setup_style_schemes();
    setup_language_schema();
    done.set(true);
}

fn setup_css() {
    let css = include_str!("../resources/css/meld.css");
    let provider = gtk::CssProvider::new();
    provider.load_from_data(css);
    if let Some(display) = gdk4::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// Install Meld's `meld-base` / `meld-dark` GtkSourceView style schemes.
///
/// The schemes are embedded in the binary and written to the user data
/// directory at startup, then that directory is added to the default
/// `StyleSchemeManager` search path so panes can select them by id.
fn setup_style_schemes() {
    let base = include_str!("../resources/styles/meld-base.style-scheme.xml");
    let dark = include_str!("../resources/styles/meld-dark.style-scheme.xml");

    let Some(dir) = dirs::data_dir().map(|d| d.join("meld-rs").join("styles")) else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let _ = std::fs::write(dir.join("meld-base.style-scheme.xml"), base);
    let _ = std::fs::write(dir.join("meld-dark.style-scheme.xml"), dark);

    let manager = sourceview5::StyleSchemeManager::default();
    manager.append_search_path(&dir.to_string_lossy());
    manager.force_rescan();
}

/// Ensure GtkSourceView can load its `.lang` syntax-highlighting definitions.
///
/// On correctly-packaged systems (Linux, most distributions) the GtkSourceView
/// library bundles a complete GResource that includes the RelaxNG schema
/// (`language2.rng`) alongside the language definition files.  On those
/// systems this function is a no-op.
///
/// On MSYS2/Windows the bundled GResource is sometimes missing `language2.rng`,
/// which causes all `.lang` loading to fail with "could not find the RelaxNG
/// schema file".  When we detect this situation we:
/// 1. Extract the `.lang` files from the DLL's globally-registered GResources.
/// 2. Write them — plus our embedded `language2.rng` — to the user cache dir.
/// 3. Add that directory to the `LanguageManager` search path so that
///    GtkSourceView loads language specs from the filesystem instead of
///    the incomplete GResource.
fn setup_language_schema() {
    // Quick check: if the schema is already available globally, the system
    // is correctly packaged and we don't need any workaround.  This is the
    // common case on Linux and properly-built Windows packages.
    if gio::resources_lookup_data(
        "/org/gnome/gtksourceview/language-specs/language2.rng",
        gio::ResourceLookupFlags::NONE,
    )
    .is_ok()
    {
        log::info!("GtkSourceView language2.rng schema found in GResources — no workaround needed");
        return;
    }

    log::info!(
        "GtkSourceView language2.rng schema missing from GResources; \
         extracting language specs to filesystem"
    );

    // ── Determine the cache directory ──────────────────────────────────
    let Some(base_dir) = dirs::cache_dir().map(|d| d.join("meld-rs").join("language-specs")) else {
        log::warn!("Cannot determine cache directory; syntax highlighting may not work");
        return;
    };

    // Only extract once per application version
    let marker = base_dir.join(".meld-extracted");
    if marker.exists() {
        add_language_search_path(&base_dir);
        return;
    }

    // Clean any previously-failed partial extraction
    let _ = std::fs::remove_dir_all(&base_dir);
    if std::fs::create_dir_all(&base_dir).is_err() {
        log::warn!(
            "Cannot create language-specs cache dir: {}",
            base_dir.display()
        );
        return;
    }

    // ── Write the missing RelaxNG schema ───────────────────────────────
    if !write_language2_rng(&base_dir) {
        log::warn!("Failed to provide language2.rng; syntax highlighting may not work");
        let _ = std::fs::remove_dir_all(&base_dir);
        return;
    }

    // ── Extract .lang files from the DLL's GResources ──────────────────
    let count = extract_lang_files_from_resources(&base_dir);
    if count == 0 {
        log::warn!("Could not extract any .lang files; syntax highlighting will not work");
        let _ = std::fs::remove_dir_all(&base_dir);
        return;
    }

    // ── Register the search path ───────────────────────────────────────
    add_language_search_path(&base_dir);

    // ── Mark extraction complete ───────────────────────────────────────
    let _ = std::fs::write(&marker, VERSION);
    log::info!(
        "Extracted {} GtkSourceView language spec(s) to {}",
        count,
        base_dir.display()
    );
}

/// Write the `language2.rng` RelaxNG schema to `base_dir`.
///
/// Prefers our embedded (compile-time) copy; falls back to searching the
/// filesystem if the embed is unavailable (e.g. `glib-compile-resources`
/// was not found at build time).
fn write_language2_rng(base_dir: &std::path::Path) -> bool {
    #[cfg(gresource_available)]
    {
        // Register our compiled GResource so the file is available via
        // the global resource lookup, then extract it.
        let bytes = glib::Bytes::from_static(LANGUAGE_SCHEMA_GRESOURCE);
        if let Ok(resource) = gio::Resource::from_data(&bytes) {
            gio::resources_register(&resource);
        }
        if let Ok(data) = gio::resources_lookup_data(
            "/org/gnome/gtksourceview/language-specs/language2.rng",
            gio::ResourceLookupFlags::NONE,
        ) {
            return std::fs::write(base_dir.join("language2.rng"), data).is_ok();
        }
    }

    // Fallback: search the filesystem
    search_and_copy_language2_rng(base_dir)
}

/// Search common locations for `language2.rng` and copy it to `base_dir`.
fn search_and_copy_language2_rng(base_dir: &std::path::Path) -> bool {
    // MSYS2 prefix (MINGW_PREFIX or UCRT64_PREFIX)
    for env_var in &["MINGW_PREFIX", "UCRT64_PREFIX"] {
        if let Ok(prefix) = std::env::var(env_var) {
            let src = std::path::PathBuf::from(&prefix)
                .join("share")
                .join("gtksourceview-5")
                .join("language-specs")
                .join("language2.rng");
            if src.exists() {
                return std::fs::copy(&src, base_dir.join("language2.rng")).is_ok();
            }
        }
    }

    // XDG_DATA_DIRS (standard on Linux)
    if let Ok(dirs) = std::env::var("XDG_DATA_DIRS") {
        for d in std::env::split_paths(&dirs) {
            let src = d
                .join("gtksourceview-5")
                .join("language-specs")
                .join("language2.rng");
            if src.exists() {
                return std::fs::copy(&src, base_dir.join("language2.rng")).is_ok();
            }
        }
    }

    false
}

/// Try to extract `.lang` files from globally registered GResources.
///
/// Returns the number of files successfully extracted.
fn extract_lang_files_from_resources(base_dir: &std::path::Path) -> usize {
    let path = "/org/gnome/gtksourceview/language-specs";
    let Ok(children) = gio::resources_enumerate_children(path, gio::ResourceLookupFlags::NONE)
    else {
        return 0;
    };

    let mut count = 0;
    for child in children {
        if !child.ends_with(".lang") {
            continue;
        }
        let resource_path = format!("{}/{}", path, child);
        if let Ok(data) = gio::resources_lookup_data(&resource_path, gio::ResourceLookupFlags::NONE)
        {
            if std::fs::write(base_dir.join(&*child), &data).is_ok() {
                count += 1;
            }
        }
    }
    count
}

/// Add `base_dir` to GtkSourceView's `LanguageManager` search path.
fn add_language_search_path(base_dir: &std::path::Path) {
    let lang_mgr = sourceview5::LanguageManager::default();
    let path_str = base_dir.to_string_lossy();

    // Avoid adding duplicates (search_path returns platform-specific paths,
    // so this check is best-effort).
    let existing: Vec<glib::GString> = lang_mgr.search_path();
    if !existing.iter().any(|p| p.as_str() == path_str.as_ref()) {
        lang_mgr.append_search_path(&path_str);
        log::info!("Added language search path: {}", path_str);
    }
}

fn show_about_dialog() {
    let dialog = gtk::AboutDialog::new();
    dialog.set_program_name(Some(APP_NAME));
    dialog.set_version(Some(VERSION));
    dialog.set_comments(Some(
        "Visual diff and merge tool — rewritten in Rust with gtk-rs\nby Hildebrando Chávez Núñez",
    ));
    dialog.set_license_type(gtk::License::Gpl20);
    dialog.set_website(Some("https://github.com/brandochn/meld-rs"));
    dialog.set_copyright(Some("Copyright © 2002-2009 Stephen Kennedy\nCopyright © 2009-2022 Kai Willadsen\nCopyright © 2024 meld-rs contributors\nCopyright © 2026 Hildebrando Chávez Núñez (Rust rewrite)"));
    dialog.set_authors(&[
        "Stephen Kennedy",
        "Kai Willadsen",
        "Vincent Legoll",
        "Hildebrando Chávez Núñez (Rust rewrite)",
        "meld-rs contributors",
    ]);
    dialog.set_artists(&["GNOME Project", "Josef Vybíral"]);
    dialog.present();
}

fn open_comparisons(app: &gtk::Application, opts: &CliOptions) {
    let mut comparisons: Vec<Vec<String>> = Vec::new();
    if !opts.paths.is_empty() {
        comparisons.push(opts.paths.clone());
    }
    comparisons.extend(opts.diff_sets.clone());

    if comparisons.is_empty() {
        let window = MeldWindow::new(app);
        window.append_new_comparison();
        window.present();
        return;
    }

    let window = MeldWindow::new(app);
    for (i, paths) in comparisons.iter().enumerate() {
        let gfiles: Vec<gio::File> = paths
            .iter()
            .filter(|p| p.as_str() != "@blank")
            .map(|p| gio::File::for_path(p))
            .collect();
        if gfiles.is_empty() {
            continue;
        }
        if opts.auto_merge || gfiles.len() == 3 {
            window.open_file_merge(&gfiles, opts.output.as_deref());
        } else if gfiles.len() == 1 {
            if let Some(path) = gfiles[0].path() {
                if path.is_dir() {
                    window.open_vc_view(&path.to_string_lossy().into_owned(), opts.auto_compare);
                } else {
                    window.open_paths(&gfiles, false, false, i == 0);
                }
            }
        } else {
            window.open_paths(&gfiles, opts.auto_compare, false, i == 0);
        }
    }
    window.present();
}
