//! Entry point for the meld-rs application.
//!
//! On start, locates the GSettings schema directory (needed by GTK4 file
//! dialogs on all platforms) via `XDG_DATA_DIRS`, then initialises GTK4.

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
    //    Detected via MINGW_PREFIX env var (set by scripts/run.ps1)
    //    or by walking PATH to find the GTK4 DLL location.
    let system_share = std::env::var("MINGW_PREFIX")
        .ok()
        .map(|p| PathBuf::from(&p).join("share"))
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
    if !dirs.is_empty() {
        unsafe {
            std::env::set_var(
                "XDG_DATA_DIRS",
                std::env::join_paths(&dirs).expect("valid Unicode paths"),
            );
        }
        log::info!("XDG_DATA_DIRS configured with {} entries", dirs.len());
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

    #[cfg(feature = "gui")]
    {
        #[cfg(target_os = "windows")]
        unsafe {
            // Disable client-side decorations on Windows for a native look
            std::env::set_var("GTK_CSD", "0");
        }

        init_data_dir();
        gtk4::init().expect("Failed to initialize GTK4");

        let args: Vec<String> = std::env::args().collect();
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
