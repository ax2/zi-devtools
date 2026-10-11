//! Shared Shell behavior without the desktop UI, tools, services or updater runtime.
pub mod explorer;
pub mod file_actions;
#[cfg(windows)]
pub mod msi;
pub mod paths;
mod quote;
pub use quote::quote;
