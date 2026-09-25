//! Tauri commands for the View screen: one document's content with its
//! outline, and its highlights.

use crate::db::Db;
use core_lib::annotations::{self, Annotation};
use core_lib::documents::{self, QueueEntry};
use core_lib::outline::{self, Block};
use serde::Serialize;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
pub struct ViewDocument {
    #[serde(flatten)]
    pub detail: documents::DocumentDetail,
    /// Active tags with a positive label, any source.
    pub tags: Vec<String>,
    pub fulltext: Option<core_lib::fulltext::FulltextRow>,
    pub description_blocks: Vec<Block>,
    pub claim_blocks: Vec<Block>,
    pub drawings_status: Option<core_lib::drawings::DrawingsStatusRow>,
    pub drawing_pages: Vec<core_lib::drawings::DrawingPage>,
    pub annotations: Vec<Annotation>,
}

#[tauri::command(async)]
pub fn find_documents(
    state: State<Db>,
    query: String,
    limit: usize,
) -> Result<Vec<QueueEntry>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    documents::find(&conn, &query, limit).map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn view_document(state: State<Db>, doc_id: i64) -> Result<Option<ViewDocument>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    load(&conn, doc_id).map_err(|e| e.to_string())
}

fn load(
    conn: &core_lib::rusqlite::Connection,
    doc_id: i64,
) -> Result<Option<ViewDocument>, core_lib::storage::StorageError> {
    let Some(detail) = documents::get_full(conn, doc_id)? else {
        return Ok(None);
    };
    let positive = core_lib::labels::all_positive_tag_ids(conn, doc_id)?;
    let tags = core_lib::tags::list_active(conn)?
        .into_iter()
        .filter(|t| positive.contains(&t.id))
        .map(|t| t.name)
        .collect();
    let fulltext = core_lib::fulltext::get(conn, doc_id)?;
    let (description_blocks, claim_blocks) = match &fulltext {
        Some(f) => (
            f.description
                .as_deref()
                .map(outline::description_blocks)
                .unwrap_or_default(),
            f.claims
                .as_deref()
                .map(outline::claim_blocks)
                .unwrap_or_default(),
        ),
        None => Default::default(),
    };
    Ok(Some(ViewDocument {
        detail,
        tags,
        fulltext,
        description_blocks,
        claim_blocks,
        drawings_status: core_lib::drawings::get_status(conn, doc_id)?,
        drawing_pages: core_lib::drawings::list_pages(conn, doc_id)?,
        annotations: annotations::list(conn, doc_id)?,
    }))
}

/// Returns the document's highlights after the change, in reading order.
#[tauri::command(async)]
pub fn create_annotation(
    state: State<Db>,
    doc_id: i64,
    section: String,
    start: usize,
    end: usize,
    comment: Option<String>,
    color: String,
) -> Result<Vec<Annotation>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let now = crate::commands::current_timestamp();
    annotations::create(
        &conn,
        doc_id,
        &section,
        start,
        end,
        comment.as_deref(),
        &color,
        &now,
    )
    .map_err(|e| e.to_string())?;
    annotations::list(&conn, doc_id).map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn update_annotation(
    state: State<Db>,
    id: i64,
    comment: Option<String>,
    color: String,
) -> Result<Vec<Annotation>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let now = crate::commands::current_timestamp();
    let updated = annotations::update(&conn, id, comment.as_deref(), &color, &now)
        .map_err(|e| e.to_string())?;
    annotations::list(&conn, updated.doc_id).map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn delete_annotation(
    state: State<Db>,
    doc_id: i64,
    id: i64,
) -> Result<Vec<Annotation>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    annotations::delete(&conn, id).map_err(|e| e.to_string())?;
    annotations::list(&conn, doc_id).map_err(|e| e.to_string())
}
