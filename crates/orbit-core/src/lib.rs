//! Orbit's native services. Secrets never appear in its public IPC DTOs.
pub const SCHEMA_VERSION: u32 = 1;
pub mod config;
pub mod dto;
pub mod error;
pub mod helper_artifact;
pub mod identity;
pub mod images;
pub mod network;
pub mod service;
pub mod store;
pub mod system;
pub mod updates;
pub mod vault;
