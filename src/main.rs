//! Entry point for the meld-rs application.
//!
//! On start, locates the GSettings schema directory (needed by GTK4 file
//! dialogs on all platforms) via `XDG_DATA_DIRS`, then initialises GTK4.

// Hide the console window on Windows in release builds.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use std::path::PathBuf;
use std::process::ExitCode;

/// Populate `XDG_DATA_DIRS` so GLib can find GSettings schemas at runtime.
///
/// Two schema sources are needed:
/// 1. `target/share/` — our application schema (org.gnome.meld-rs)
/// 2. `$MINGW_PREFIX/share` — GTK4 system schemas (org.gtk.gtk4.Settings.FileChooser)
///
/// Both are prepended to any existing `XDG_DATA_DIRS` value so system
/// directories are not lost.
fn init_data_dir() {
    let mut dirs: Vec<PathBuf> = std::env::var("XDG_DATA_DIRS")
        .ok()
        .map(|s| std::env::split_paths(&s).collect())
        .unwrap_or_default();

    // 1. target/share/ — our application schema (cargo build/run)
    if let Some(our_share) = std::env::current_exe()
        .ok()
        .as_ref()
        .and_then(|e| e.parent())
        .map(|p| p.join("share"))
    {
        if our_share.exists() && !dirs.iter().any(|d| d == &our_share) {
            dirs.insert(0, our_share);
        }
    }

    // 2. MSYS2 MINGW64/share — GTK4 system schemas (FileChooser, etc.)
    //    Detected via MINGW_PREFIX env var (set by scripts/run.ps1 or
    //    the git wrapper).  If the path does not exist on this platform
    //    (e.g. Unix-style /mingw64 on Windows), fall back to scanning
    //    PATH for the GTK4 DLL.
    let system_share = std::env::var("MINGW_PREFIX")
        .ok()
        .map(|p| PathBuf::from(&p).join("share"))
        .filter(|p| p.exists())
        .or_else(|| {
            // Fallback: find libgtk-4-1.dll in PATH and use its share/
            std::env::var("PATH").ok().and_then(|path| {
                std::env::split_paths(&path)
                    .find(|d| d.join("libgtk-4-1.dll").exists())
                    .and_then(|d| d.parent().map(|p| p.join("share")))
            })
        });
    if let Some(share) = system_share {
        if share.exists() && !dirs.iter().any(|d| d == &share) {
            dirs.insert(0, share);
        }
    }

    // SAFETY: Setting XDG_DATA_DIRS is required for GLib to locate
    // GSettings schemas (both ours and GTK4's built-in schemas).
    // The paths come from the executable location and the MSYS2
    // environment variable, both of which are well-formed.
    //
    // Clear any existing value first — MSYS2 may have set it to Unix
    // paths that don't exist on Windows, confusing GTK4.
    if !dirs.is_empty() {
        unsafe {
            std::env::remove_var("XDG_DATA_DIRS");
            match std::env::join_paths(&dirs) {
                Ok(joined) => {
                    std::env::set_var("XDG_DATA_DIRS", joined);
                    log::info!("XDG_DATA_DIRS configured with {} entries", dirs.len());
                }
                Err(e) => {
                    log::warn!("Failed to join XDG_DATA_DIRS paths: {e}");
                }
            }
        }
    } else {
        log::warn!(
            "No GSettings schema directories found. \
             File choosers may not work. \
             Run: glib-compile-schemas target/share/glib-2.0/schemas"
        );
    }
}

fn main() -> ExitCode {
    env_logger::init();

    // Set a panic hook that writes to our persistent log file so that
    // crashes are diagnosable even on Windows GUI-subsystem builds.
    let default_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        meld_rs::log_diag(&format!("FATAL PANIC: {info}"));
        if let Some(loc) = info.location() {
            meld_rs::log_diag(&format!("  at {}:{}", loc.file(), loc.line()));
        }
        default_panic_hook(info);
    }));

    #[cfg(feature = "gui")]
    {
        #[cfg(target_os = "windows")]
        unsafe {
            // Force native Win32 backend
            std::env::set_var("GDK_BACKEND", "win32");
            std::env::set_var("GTK_CSD", "0");
            std::env::remove_var("WAYLAND_DISPLAY");
            std::env::remove_var("DISPLAY");
        }

        init_data_dir();

        if let Err(e) = gtk4::init() {
            let msg = format!(
                "Failed to initialize GTK4: {e}\n\
                 Make sure GTK4 runtime libraries are installed and on PATH."
            );
            meld_rs::log_diag(&msg);
            return ExitCode::from(1);
        }

        let args: Vec<String> = std::env::args().collect();
        log::info!("meld-rs starting with {} args: {:?}", args.len(), &args);

        let app = meld_rs::app::MeldApp::new();
        app.run_with_args(&args)
    }

    #[cfg(not(feature = "gui"))]
    {
        eprintln!("Error: This binary requires the 'gui' feature to be enabled.");
        eprintln!("Build with: cargo build --features gui");
        ExitCode::from(1)
    }
}
