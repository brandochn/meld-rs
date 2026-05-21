//! Diff engine and comparison logic.
//!
//! This module contains the core diffing algorithms (line-level and word-level),
//! file comparison logic, and directory comparison logic.

pub mod engine;
pub mod matchers;

/// LRU cache for inline (character-level) diff results.
pub mod inline_cache;

#[cfg(feature = "gui")]
/// Lazy-cached line access to GtkSourceBuffer.
pub mod buffer_lines;

#[cfg(feature = "gui")]
pub mod dirdiff;

#[cfg(feature = "gui")]
pub mod filediff;
