//! Utility helpers.
//!
//! Provides encoding detection, file operations, and background task scheduling.

pub mod encoding;
pub mod file_utils;

#[cfg(feature = "gui")]
pub mod task;
