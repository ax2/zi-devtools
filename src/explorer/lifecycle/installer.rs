//! Bind shared manual maintenance to the desktop product version.
use anyhow::Result;
pub use core::{Request, Ticket, apply, ensure_portable_origin};
use std::path::Path;
use zi_shell_core::explorer::lifecycle::installer as core;

pub fn preview(config: &Path, request: Request) -> Result<Ticket> {
    core::preview(config, request, env!("CARGO_PKG_VERSION"))
}

#[cfg(feature = "ui-preview")]
pub(crate) fn fixture_preview(config: &Path) -> Result<Ticket> {
    core::fixture_preview(config, env!("CARGO_PKG_VERSION"))
}
