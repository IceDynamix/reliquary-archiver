//! High-level UI components.
//!
//! This module contains complex, reusable components that are composed
//! from the lower-level kit primitives:
//!
//! - [`file_download`]: File download button with tooltip
//! - [`log_view`]: Scrollable log viewer with selection support
//! - [`settings_modal`]: Application settings panel
//! - [`update`]: Update notification modal

pub mod file_download;
pub mod log_view;
pub mod settings_modal;
pub mod update;
