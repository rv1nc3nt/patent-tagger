//! Domain types, SQLite storage and learning. No Tauri, no network.

pub mod documents;
pub mod embeddings;
pub mod jobs;
pub mod labels;
pub mod number;
pub mod scoring;
pub mod selection;
pub mod storage;
pub mod tags;
pub mod time;

// Re-exported so downstream crates (src-tauri) can name `Connection` without
// duplicating the rusqlite dependency/version themselves.
pub use rusqlite;
