//! Orbit's native services. Secrets never appear in its public IPC DTOs.
pub const SCHEMA_VERSION: u32 = 1;
pub mod claude_config;
pub mod clipboard;
pub mod codex_binding;
pub mod codex_catalog;
#[cfg(all(test, target_os = "macos"))]
mod codex_runtime;
pub mod config;
pub mod dto;
pub mod error;
pub mod helper_artifact;
pub mod identity;
pub mod images;
pub mod network;
pub(crate) mod policy;
pub mod runtime_lock;
pub mod service;
pub mod store;
pub mod system;
pub mod update_recovery;
pub mod updates;
pub mod vault;
