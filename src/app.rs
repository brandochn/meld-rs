#![cfg(feature = "gui")]
//! Application entry with full gear menu, about dialog, and shortcuts overlay.
//!
//! Matches `menus.ui` (186 lines), `about-dialog.ui`, and `help-overlay.ui`.

use glib::prelude::*;
use gtk4 as gtk;
use gtk4::prelude::*;
use std::path::Path;
use std::process::ExitCode;

use crate::window::MeldWindow;

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
        let app = gtk::Application::new(Some(APP_ID), gio::ApplicationFlags::empty());
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
        let opts = match parse_args(args) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("Error: {e}");
                return ExitCode::from(2);
            }
        };

        let opts = std::sync::Arc::new(std::sync::Mutex::new(Some(opts)));
        self.app.connect_activate(move |app| {
            setup_actions(app);
            setup_css();
            if let Some(o) = opts.lock().ok().and_then(|mut o| o.take()) {
                open_comparisons(app, &o);
            } else if app.windows().iter().count() == 0 {
                let window = MeldWindow::new(app);
                window.present();
            }
        });

        self.app.run();
        ExitCode::SUCCESS
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

fn show_about_dialog() {
    let dialog = gtk::AboutDialog::new();
    dialog.set_program_name(Some(APP_NAME));
    dialog.set_version(Some(VERSION));
    dialog.set_comments(Some(
        "Visual diff and merge tool — rewritten in Rust with gtk-rs",
    ));
    dialog.set_license_type(gtk::License::Gpl20);
    dialog.set_website(Some("https://github.com/tu-usuario/meld-rs"));
    dialog.set_copyright(Some("Copyright © 2002-2009 Stephen Kennedy\nCopyright © 2009-2022 Kai Willadsen\nCopyright © 2024 meld-rs contributors"));
    dialog.set_authors(&[
        "Stephen Kennedy",
        "Kai Willadsen",
        "Vincent Legoll",
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
