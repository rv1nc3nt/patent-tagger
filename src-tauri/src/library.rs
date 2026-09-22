//! Tauri commands for the Library screen (SPEC section 8): search,
//! similarity, and the CSV/JSON/per-tag-list/document exports it triggers.
//! Full-text/drawings availability filters and bulk retrieval aren't
//! included yet - they need M8's retrieval pipeline.

use crate::db::Db;
use crate::model::Model;
use core_lib::{documents, export, library, tags};
use embed_lib::Embedder;
use tauri::State;

#[tauri::command]
pub fn library_search(
    state: State<Db>,
    filters: library::LibraryFilters,
) -> Result<Vec<library::LibraryRow>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    library::search(&conn, &filters).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn similar_documents(
    state: State<Db>,
    model: State<Model>,
    doc_id: i64,
    limit: usize,
) -> Result<Vec<library::SimilarDocument>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    library::similar_to(&conn, doc_id, model.0.model_id(), limit).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn export_csv(state: State<Db>, dest_path: String) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let rows = export::export_rows(&conn).map_err(|e| e.to_string())?;
    std::fs::write(&dest_path, export::to_csv(&rows)).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn export_json(state: State<Db>, dest_path: String) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let rows = export::export_rows(&conn).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?;
    std::fs::write(&dest_path, json).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn export_tag_list(state: State<Db>, tag_id: i64, dest_path: String) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let pub_keys = export::pub_keys_for_tag(&conn, tag_id).map_err(|e| e.to_string())?;
    std::fs::write(&dest_path, pub_keys.join("\n")).map_err(|e| e.to_string())
}

/// One folder per document under `dest_dir`, each with a `.txt` file in
/// SPEC section 8's export format (drawings subfolders come with M8).
#[tauri::command]
pub fn export_documents(state: State<Db>, doc_ids: Vec<i64>, dest_dir: String) -> Result<usize, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let dest_dir = std::path::Path::new(&dest_dir);
    let mut exported = 0;
    for doc_id in doc_ids {
        let Some(detail) = documents::get_full(&conn, doc_id).map_err(|e| e.to_string())? else {
            continue;
        };
        let positive_tag_ids =
            core_lib::labels::all_positive_tag_ids(&conn, doc_id).map_err(|e| e.to_string())?;
        let tag_names: Vec<String> = tags::list_active(&conn)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|t| positive_tag_ids.contains(&t.id))
            .map(|t| t.name)
            .collect();

        let folder = dest_dir.join(&detail.pub_key);
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let text = export::document_txt(&detail, &tag_names);
        std::fs::write(folder.join(format!("{}.txt", detail.pub_key)), text).map_err(|e| e.to_string())?;
        exported += 1;
    }
    Ok(exported)
}
