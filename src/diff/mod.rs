//! Diff engine and comparison logic.
//!
//! This module contains the core diffing algorithms (line-level and word-level),
//! file comparison logic, and directory comparison logic.

pub mod engine;
pub mod matchers;

/// Cross-line similarity matching for non-aligned changes.
pub mod similarity;

/// Moved code block detection for relocated content.
pub mod movement;

/// LRU cache for inline (character-level) diff results.
pub mod inline_cache;

/// File-level comparison logic (stat, shallow comparison, content comparison).
pub mod file_compare;

#[cfg(test)]
mod fuzz_tests;

#[cfg(feature = "gui")]
/// Background diff computation state management.
pub mod diff_state;

#[cfg(feature = "gui")]
/// Lazy-cached line access to GtkSourceBuffer.
pub mod buffer_lines;

#[cfg(feature = "gui")]
pub mod dirdiff;

#[cfg(feature = "gui")]
pub mod filediff;
