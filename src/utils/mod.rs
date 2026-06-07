//! Utility helpers.
//!
//! Provides encoding detection, file operations, text filtering, and background task scheduling.

pub mod encoding;
pub mod file_utils;
pub mod text_filter;

#[cfg(feature = "gui")]
pub mod task;
