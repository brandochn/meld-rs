//! Meld-rs — Visual diff and merge tool rewritten in Rust with gtk-rs.
//!
//! This library provides the core application logic for the meld-rs diff and merge
//! tool, including file comparison, directory comparison, version control integration,
//! and 3-way merge support.
//!
//! When compiled with the `gui` feature (default), the GTK4 UI is available.

/// Write a diagnostic message to stderr and to a persistent log file.
///
/// On Windows GUI-subsystem builds stderr is disconnected, so the log
/// file is the only way to see crash/error info.  Tries the executable's
/// directory first (convenient for development), falling back to the
/// system temp directory (works for production installs into protected
/// paths like `Program Files`).
pub fn log_diag(msg: &str) {
    eprintln!("{msg}");

    // Try the exe directory first, fall back to temp.
    let log_path = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join("meld-rs.log")))
        .unwrap_or_else(|| std::env::temp_dir().join("meld-rs").join("meld-rs.log"));

    // Also ensure the temp fallback is available when the exe-dir path
    // exists but isn't writable.
    let fallback = std::env::temp_dir().join("meld-rs").join("meld-rs.log");
    if let Some(parent) = fallback.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    for path in &[&log_path, &fallback] {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            use std::io::Write;
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let _ = writeln!(f, "[{timestamp}] {msg}");
            let _ = f.flush();
            break; // Success — don't write duplicates.
        }
    }
}

pub mod config;
pub mod diff;
pub mod utils;
pub mod vc;

#[cfg(feature = "gui")]
pub mod undo;

#[cfg(feature = "gui")]
pub mod app;

#[cfg(feature = "gui")]
pub mod window;

#[cfg(feature = "gui")]
pub mod ui;
