//! UI building blocks and reusable widget primitives.
//!
//! This module contains low-level UI components that are used throughout
//! the application:
//!
//! - [`icons`]: SVG icon components (refresh, close, cog, social buttons)
//! - [`modal`]: Modal backdrop and card components
//! - [`togglegroup`]: Radio-button-style toggle group widget
//! - [`tooltip`]: Hover tooltip components with theme support

pub mod icons;
pub mod modal;
pub mod togglegroup;
pub mod tooltip;

/// Formats a byte size to a human-readable string (B, KB, MB).
pub fn format_byte_size(size: usize) -> String {
    let size_f = size as f64;
    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.2} KB", size_f / 1024.0)
    } else {
        format!("{:.2} MB", size_f / 1024.0 / 1024.0)
    }
}
