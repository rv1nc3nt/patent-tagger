//! Tauri commands for the Tags screen (SPEC section 8): create, edit,
//! archive/restore, statistics, and stale-label discarding.

use crate::db::Db;
use crate::model::Model;
use core_lib::rusqlite::Connection;
use core_lib::tags::{self, TagFields, TagRow};
use embed_lib::Embedder;
use serde::Serialize;
use tauri::State;

/// Computes and stores the tag's `"{name}: {definition}"` embedding for
/// its current version (SPEC 7.2), so zero-shot scoring never has to fall
/// back to computing it lazily during review.
pub(crate) fn embed_tag(
    conn: &Connection,
    embedder: &impl Embedder,
    tag: &TagRow,
) -> Result<(), String> {
    let text = format!("{}: {}", tag.name, tag.definition);
    let vector = embedder
        .embed(&[text])
        .map_err(|e| e.to_string())?
        .pop()
        .ok_or("embedder returned no vector")?;
    core_lib::embeddings::store_tag_embedding(
        conn,
        tag.id,
        embedder.model_id(),
        tag.version,
        &vector,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_tags(state: State<Db>) -> Result<Vec<TagRow>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    tags::list_active(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_archived_tags(state: State<Db>) -> Result<Vec<TagRow>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    tags::list_archived(&conn).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct TagOverview {
    pub tag: TagRow,
    pub stats: tags::TagStats,
    pub eligibility: core_lib::predictions::Eligibility,
}

/// Every active tag with its label statistics and automatic-mode
/// eligibility (SPEC section 8: "Statistics per tag ... automatic-mode
/// toggle with eligibility status").
#[tauri::command]
pub fn tag_overview(state: State<Db>) -> Result<Vec<TagOverview>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let target_precision =
        core_lib::settings::target_precision(&conn).map_err(|e| e.to_string())?;
    tags::list_active(&conn)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|tag| {
            let stats = tags::stats(&conn, tag.id).map_err(|e| e.to_string())?;
            let eligibility =
                core_lib::predictions::tag_eligibility(&conn, tag.id, target_precision)
                    .map_err(|e| e.to_string())?;
            Ok(TagOverview {
                tag,
                stats,
                eligibility,
            })
        })
        .collect()
}

#[tauri::command]
pub fn create_tag(
    state: State<Db>,
    model: State<Model>,
    name: String,
    definition: String,
    parent_id: Option<i64>,
    color: Option<String>,
    hotkey: Option<String>,
) -> Result<TagRow, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let fields = TagFields {
        name: &name,
        definition: &definition,
        parent_id,
        color: color.as_deref(),
        hotkey: hotkey.as_deref(),
    };
    let id = tags::insert(&conn, fields, &crate::commands::current_timestamp())
        .map_err(|e| e.to_string())?;
    let tag = tags::get(&conn, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "tag vanished immediately after creation".to_string())?;
    embed_tag(&conn, &model.0, &tag)?;
    Ok(tag)
}

/// `bump_version` marks a material definition change (SPEC 7.1). The
/// embedding is recomputed whenever the name or definition changes.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn update_tag(
    state: State<Db>,
    model: State<Model>,
    tag_id: i64,
    name: String,
    definition: String,
    parent_id: Option<i64>,
    color: Option<String>,
    hotkey: Option<String>,
    bump_version: bool,
) -> Result<TagRow, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let fields = TagFields {
        name: &name,
        definition: &definition,
        parent_id,
        color: color.as_deref(),
        hotkey: hotkey.as_deref(),
    };
    let updated = tags::update(&conn, tag_id, fields, bump_version).map_err(|e| e.to_string())?;
    if updated.needs_embedding {
        embed_tag(&conn, &model.0, &updated.tag)?;
    }
    Ok(updated.tag)
}

#[tauri::command]
pub fn archive_tag(state: State<Db>, tag_id: i64) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    tags::archive(&conn, tag_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn unarchive_tag(state: State<Db>, tag_id: i64) -> Result<tags::Restored, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    tags::unarchive(&conn, tag_id).map_err(|e| e.to_string())
}

/// SPEC 7.1: stale labels "remain usable for training unless the user
/// discards them". Returns how many were discarded.
#[tauri::command]
pub fn discard_stale_labels(state: State<Db>, tag_id: i64) -> Result<usize, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let tag = tags::get(&conn, tag_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("tag {tag_id} not found"))?;
    core_lib::labels::discard_stale(&conn, &tag).map_err(|e| e.to_string())
}
