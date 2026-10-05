pub mod agent;
pub mod agent_record;
pub mod agent_record_compare;
pub mod agent_record_library;
pub mod agent_record_ui;
pub mod agent_ui;
pub mod app;
pub mod calculator;
pub mod clock;
pub mod commands;
pub mod config;
pub mod dev_tools;
pub mod diagnostics;
pub mod directory_compare;
pub mod disk_inspector;
pub mod document_ingestion;
pub mod duplicate_finder;
pub mod embedding;
pub mod file_encoding;
pub mod fixture;
pub mod framework;
pub mod hotkey;
pub mod hybrid_search;
pub mod image_tools;
pub mod intake;
pub mod integrations;
pub mod knowledge_answer;
pub mod knowledge_capture;
pub mod knowledge_eval;
pub mod knowledge_index;
pub mod knowledge_mcp;
pub mod knowledge_search;
pub mod knowledge_sources;
pub mod markdown_preview;
pub mod network_tools;
pub mod planner;
pub mod plugin_ui;
pub mod plugins;
pub mod preferences;
pub mod prompt_library;
pub mod prompt_template;
pub mod service;
pub mod sqlite_browser;
pub mod tools;
pub mod tools_advanced;
pub mod tools_extra;
pub mod tray;
pub mod vector_index;
pub mod workbench;

pub mod tasks;

pub mod credentials;

pub mod character_tools;
pub mod chat_attachments;
pub mod chat_library;
pub mod chat_stream;
pub mod checksum_manifest;
pub mod conversation;
pub mod mcp;
pub mod mcp_access;
pub mod mcp_export;
pub mod mcp_http;
pub mod mcp_ui;
pub mod model_discovery;
pub mod plugin_settings;
#[cfg(windows)]
pub mod recorder;
#[cfg(windows)]
pub mod recorder_ui;

pub mod mcp_oauth;

pub mod mcp_oauth_ui;

pub mod mcp_oauth_callback;
#[cfg(test)]
mod mcp_oauth_flow_tests;
pub mod mcp_oauth_login;
pub mod mcp_oauth_registration;
#[cfg(test)]
mod mcp_oauth_tls_tests;
pub mod mcp_oauth_token;
pub mod workspace_store;
