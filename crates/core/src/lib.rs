//! Domain types, SQLite storage and learning. No Tauri, no network.

pub mod audit;
pub mod classifier;
pub mod documents;
pub mod drawings;
pub mod embeddings;
pub mod export;
pub mod full_automation;
pub mod fulltext;
pub mod jobs;
pub mod labels;
pub mod library;
pub mod number;
pub mod predictions;
pub mod retrieval_policy;
pub mod scoring;
pub mod selection;
pub mod settings;
pub mod storage;
pub mod tags;
pub mod time;

// Re-exported so downstream crates (src-tauri) can name `Connection` without
// duplicating the rusqlite dependency/version themselves.
pub use rusqlite;
