//! Tauri commands for the Library screen (SPEC section 8): search,
//! similarity, and the CSV/JSON/per-tag-list/document exports it triggers,
//! plus the bulk full-text/drawings retrieval action.

use crate::db::Db;
use crate::model::Model;
use core_lib::rusqlite::Connection;
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

/// Document export grouped by tag: one folder per active tag under
/// `dest_dir` (named by [`export::tag_folder_names`]), each holding the
/// folder of every selected document carrying that tag. A document with
/// several tags is copied into each of their folders; one with none goes
/// into [`export::UNTAGGED_FOLDER`]. Returns the number of documents
/// exported (not of folders written).
#[tauri::command]
pub fn export_documents(
    state: State<Db>,
    doc_ids: Vec<i64>,
    dest_dir: String,
) -> Result<usize, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let dest_dir = std::path::Path::new(&dest_dir);
    let active_tags = tags::list_active(&conn).map_err(|e| e.to_string())?;
    let folder_names =
        export::tag_folder_names(active_tags.iter().map(|t| (t.id, t.name.as_str())));
    let mut exported = 0;
    for doc_id in doc_ids {
        let positive_tag_ids =
            core_lib::labels::all_positive_tag_ids(&conn, doc_id).map_err(|e| e.to_string())?;
        let mut tag_dirs: Vec<std::path::PathBuf> = active_tags
            .iter()
            .filter(|t| positive_tag_ids.contains(&t.id))
            .filter_map(|t| folder_names.get(&t.id))
            .map(|name| dest_dir.join(name))
            .collect();
        if tag_dirs.is_empty() {
            tag_dirs.push(dest_dir.join(export::UNTAGGED_FOLDER));
        }
        let mut found = false;
        for tag_dir in &tag_dirs {
            found = export_document_folder(&conn, &state.data_dir, doc_id, tag_dir)
                .map_err(|e| e.to_string())?;
            if !found {
                break;
            }
        }
        if found {
            exported += 1;
        }
    }
    Ok(exported)
}

/// `tag_id`'s folder for a by-tag export under `dest_dir`. Naming is
/// shared with [`export_documents`], so the CLI and the GUI put a tag's
/// documents in the same place.
pub(crate) fn tag_export_dir(
    conn: &Connection,
    tag_id: i64,
    dest_dir: &std::path::Path,
) -> anyhow::Result<std::path::PathBuf> {
    let active_tags = tags::list_active(conn)?;
    let folder_names =
        export::tag_folder_names(active_tags.iter().map(|t| (t.id, t.name.as_str())));
    let name = folder_names
        .get(&tag_id)
        .ok_or_else(|| anyhow::anyhow!("tag {tag_id} is not active"))?;
    Ok(dest_dir.join(name))
}

/// One document's export folder (SPEC section 8): `<dest_dir>/<pub_key>/`
/// with the `.txt` file and, when drawings were retrieved, copied PNG
/// pages. `dest_dir` is the tag folder chosen by the caller: the
/// [`export_documents`] Tauri command and the CLI's `export --format txt`
/// (`cli.rs`). Returns `Ok(false)` when `doc_id` doesn't exist rather than
/// erroring, matching `export_documents`'s original best-effort behaviour
/// over a batch.
pub(crate) fn export_document_folder(
    conn: &Connection,
    data_dir: &std::path::Path,
    doc_id: i64,
    dest_dir: &std::path::Path,
) -> anyhow::Result<bool> {
    let Some(detail) = documents::get_full(conn, doc_id)? else {
        return Ok(false);
    };
    let positive_tag_ids = core_lib::labels::all_positive_tag_ids(conn, doc_id)?;
    let tag_names: Vec<String> = tags::list_active(conn)?
        .into_iter()
        .filter(|t| positive_tag_ids.contains(&t.id))
        .map(|t| t.name)
        .collect();

    let folder = dest_dir.join(&detail.pub_key);
    std::fs::create_dir_all(&folder)?;

    let fulltext_row = core_lib::fulltext::get(conn, doc_id)?;
    let fulltext_export = fulltext_row.map(|f| export::DocumentExportFulltext {
        status: f.status,
        description: f.description,
        claims: f.claims,
        lang: f.lang,
        source: f.source,
    });

    let drawings_status = core_lib::drawings::get_status(conn, doc_id)?;
    let pages = core_lib::drawings::list_pages(conn, doc_id)?;
    let drawings_export = drawings_status.map(|s| {
        let mut page_paths = Vec::new();
        if s.status == "fetched" {
            for page in pages.iter().filter(|p| p.page >= 1) {
                let file_name = format!("{:03}.png", page.page);
                let source_path = data_dir.join(&page.path);
                let dest_path = folder.join("drawings").join(&file_name);
                if std::fs::create_dir_all(folder.join("drawings")).is_ok()
                    && std::fs::copy(&source_path, &dest_path).is_ok()
                {
                    page_paths.push(format!("drawings/{file_name}"));
                }
            }
        }
        export::DocumentExportDrawings {
            status: s.status,
            page_paths,
            source: s.source,
        }
    });

    let text = export::document_txt(
        &detail,
        &tag_names,
        fulltext_export.as_ref(),
        drawings_export.as_ref(),
    );
    std::fs::write(folder.join(format!("{}.txt", detail.pub_key)), text)?;
    Ok(true)
}

/// SPEC section 8's Library "bulk action: retrieve full text or drawings
/// for the current selection". Enqueues a job per document (skipping any
/// already-successful ones - same rule as after-tagging enqueueing) then
/// drains the queue once, synchronously, so the caller's promise resolves
/// once the batch is done.
#[tauri::command]
pub async fn bulk_retrieve(
    state: State<'_, Db>,
    doc_ids: Vec<i64>,
    kind: String,
) -> Result<(), String> {
    let _lock = crate::lock::PipelineLock::acquire(&state.data_dir).map_err(|e| e.to_string())?;
    let now = crate::commands::current_timestamp();
    {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        for doc_id in &doc_ids {
            match kind.as_str() {
                "fulltext" => {
                    let already_done = core_lib::fulltext::get(&conn, *doc_id)
                        .ok()
                        .flatten()
                        .is_some_and(|f| {
                            matches!(f.status.as_str(), "fetched" | "non_english_only")
                        });
                    if !already_done {
                        let _ = crate::retrieval_worker::enqueue_fulltext(&conn, *doc_id, &now);
                    }
                }
                "drawings" => {
                    let already_done = core_lib::drawings::get_status(&conn, *doc_id)
                        .ok()
                        .flatten()
                        .is_some_and(|s| s.status == "fetched");
                    if !already_done {
                        let _ = crate::retrieval_worker::enqueue_drawings(&conn, *doc_id, &now);
                    }
                }
                _ => return Err(format!("unknown retrieval kind: {kind}")),
            }
        }
    }

    let creds = crate::platform::credentials::load(&state.data_dir)
        .map_err(|e| e.to_string())?
        .ok_or("no OPS credentials saved yet")?;
    let client = ops_lib::client::OpsClient::new(creds.consumer_key, creds.consumer_secret);
    match kind.as_str() {
        "fulltext" => crate::retrieval_worker::run_fulltext(&state.conn, &client, |_| {}).await,
        "drawings" => {
            crate::retrieval_worker::run_drawings(&state.conn, &client, &state.data_dir, |_| {})
                .await
        }
        _ => {}
    }
    Ok(())
}
