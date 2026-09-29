pub mod app;
pub mod config;
pub mod dev_tools;
pub mod diagnostics;
pub mod fixture;
pub mod framework;
pub mod hotkey;
pub mod image_tools;
pub mod intake;
pub mod integrations;
pub mod network_tools;
pub mod plugin_ui;
pub mod plugins;
pub mod preferences;
pub mod prompt_library;
pub mod prompt_template;
pub mod service;
pub mod tools;
pub mod tools_advanced;
pub mod tools_extra;
pub mod tray;
pub mod workbench;

pub mod tasks;

pub mod credentials;

pub mod chat_attachments;
pub mod chat_library;
pub mod chat_stream;
pub mod conversation;
pub mod mcp;
pub mod mcp_ui;
pub mod model_discovery;
pub mod plugin_settings;
#[cfg(windows)]
pub mod recorder;
#[cfg(windows)]
pub mod recorder_ui;
