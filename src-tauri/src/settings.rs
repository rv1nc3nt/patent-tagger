//! Tauri commands for the Settings screen (SPEC section 8): OPS
//! credentials live in `commands.rs` (M2); this is everything else -
//! target precision/recall/audit rate/full-automation switch, retrieval
//! policies, the data folder display, backup/restore, and tag-schema
//! export/import.

use crate::db::Db;
use crate::model::Model;
use core_lib::{settings, tags};
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsView {
    pub target_precision: f32,
    pub target_recall: f32,
    pub audit_rate: f32,
    pub full_automation_enabled: bool,
    pub fulltext_policy: String,
    pub drawings_policy: String,
    /// Consulted only when the matching policy is
    /// `"after_tagging_selected_tags"` (SPEC 5.5).
    pub fulltext_policy_tag_ids: Vec<i64>,
    pub drawings_policy_tag_ids: Vec<i64>,
}

#[tauri::command(async)]
pub fn get_settings(state: State<Db>) -> Result<SettingsView, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    Ok(SettingsView {
        target_precision: settings::target_precision(&conn).map_err(|e| e.to_string())?,
        target_recall: settings::target_recall(&conn).map_err(|e| e.to_string())?,
        audit_rate: settings::audit_rate(&conn).map_err(|e| e.to_string())?,
        full_automation_enabled: settings::full_automation_enabled(&conn)
            .map_err(|e| e.to_string())?,
        fulltext_policy: settings::fulltext_policy(&conn).map_err(|e| e.to_string())?,
        drawings_policy: settings::drawings_policy(&conn).map_err(|e| e.to_string())?,
        fulltext_policy_tag_ids: settings::fulltext_policy_tag_ids(&conn)
            .map_err(|e| e.to_string())?,
        drawings_policy_tag_ids: settings::drawings_policy_tag_ids(&conn)
            .map_err(|e| e.to_string())?,
    })
}

/// SPEC 7.6: full automation "can be turned on only when at least one tag
/// is in automatic mode" - enforced here, even though flipping it has no
/// behavioural effect until M9 builds full automation itself.
#[tauri::command(async)]
pub fn update_settings(state: State<Db>, view: SettingsView) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;

    if view.full_automation_enabled {
        let any_auto_enabled = tags::list_active(&conn)
            .map_err(|e| e.to_string())?
            .iter()
            .any(|t| t.auto_enabled);
        if !any_auto_enabled {
            return Err(
                "full automation can only be turned on once at least one tag is in automatic mode"
                    .to_string(),
            );
        }
    }

    settings::set(
        &conn,
        settings::TARGET_PRECISION_KEY,
        &view.target_precision.to_string(),
    )
    .map_err(|e| e.to_string())?;
    settings::set(
        &conn,
        settings::TARGET_RECALL_KEY,
        &view.target_recall.to_string(),
    )
    .map_err(|e| e.to_string())?;
    settings::set(
        &conn,
        settings::AUDIT_RATE_KEY,
        &view.audit_rate.to_string(),
    )
    .map_err(|e| e.to_string())?;
    settings::set(
        &conn,
        settings::FULL_AUTOMATION_ENABLED_KEY,
        if view.full_automation_enabled {
            "true"
        } else {
            "false"
        },
    )
    .map_err(|e| e.to_string())?;
    settings::set(&conn, settings::FULLTEXT_POLICY_KEY, &view.fulltext_policy)
        .map_err(|e| e.to_string())?;
    settings::set(&conn, settings::DRAWINGS_POLICY_KEY, &view.drawings_policy)
        .map_err(|e| e.to_string())?;
    settings::set_fulltext_policy_tag_ids(&conn, &view.fulltext_policy_tag_ids)
        .map_err(|e| e.to_string())?;
    settings::set_drawings_policy_tag_ids(&conn, &view.drawings_policy_tag_ids)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command(async)]
pub fn data_directory(state: State<Db>) -> String {
    state.data_dir.display().to_string()
}

#[tauri::command(async)]
pub fn create_backup(state: State<Db>, dest_path: String) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    crate::backup::create_backup(&state.data_dir, &conn, std::path::Path::new(&dest_path))
        .map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn restore_backup(state: State<Db>, src_path: String) -> Result<(), String> {
    let mut conn = state.conn.lock().map_err(|e| e.to_string())?;
    crate::backup::restore_backup(&state.data_dir, &mut conn, std::path::Path::new(&src_path))
        .map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn export_tag_schema(state: State<Db>, dest_path: String) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let schema = tags::export_schema(&conn).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&schema).map_err(|e| e.to_string())?;
    std::fs::write(&dest_path, json).map_err(|e| e.to_string())
}

/// Add-only (SPEC section 8): see `tags::import_schema`. Each created tag
/// gets its zero-shot embedding immediately, as in `create_tag`.
#[tauri::command(async)]
pub fn import_tag_schema(
    state: State<Db>,
    model: State<Model>,
    src_path: String,
) -> Result<tags::SchemaImport, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let json = std::fs::read_to_string(&src_path).map_err(|e| e.to_string())?;
    let schema: Vec<tags::TagSchema> = serde_json::from_str(&json).map_err(|e| e.to_string())?;

    let now = crate::commands::current_timestamp();
    let outcome = tags::import_schema(&conn, &schema, &now).map_err(|e| e.to_string())?;
    for &tag_id in &outcome.created {
        if let Some(tag) = tags::get(&conn, tag_id).map_err(|e| e.to_string())? {
            crate::tag_screen::embed_saved_tag(&conn, &model.0, &tag);
        }
    }
    Ok(outcome)
}
