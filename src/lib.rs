//! omarchyuu: a native remote workspace for Omarchy, compatible with UU clients.

pub const APP_NAME: &str = "omarchyuu";
pub(crate) const PROJECT_URL: Option<&str> = option_env!("OMARCHYUU_PROJECT_URL");
pub(crate) const VIEWER_TITLE_PREFIX: &str = "omarchyuu — ";

pub mod account;
pub mod application;
pub mod diagnostics;
pub mod features;
mod lifecycle;
pub mod media;
pub mod protocol;
pub mod session;
pub mod transport;

// The plugin execution/rendering backend is Windows-only.
#[cfg(windows)]
pub mod plugins;
mod ui;

mod platform;
