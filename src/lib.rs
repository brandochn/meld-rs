//! Meld-rs — Visual diff and merge tool rewritten in Rust with gtk-rs.
//!
//! This library provides the core application logic for the meld-rs diff and merge
//! tool, including file comparison, directory comparison, version control integration,
//! and 3-way merge support.
//!
//! When compiled with the `gui` feature (default), the GTK4 UI is available.

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
