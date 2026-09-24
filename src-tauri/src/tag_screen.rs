//! Tauri commands for the Tags screen (SPEC section 8): create, edit,
//! archive/restore, statistics, stale-label discarding, and the per-tag
//! review queue (SPEC 7.1).

use crate::db::Db;
use crate::model::Model;
use core_lib::labels::{self, LabelState, TagReviewCandidate};
use core_lib::rusqlite::Connection;
use core_lib::tags::{self, TagFields, TagRow};
use embed_lib::Embedder;
use serde::Serialize;
use std::collections::HashSet;
use tauri::State;

/// Computes the tag's `"{name}: {definition}"` embedding (SPEC 7.2), or
/// `None` if the embedder fails.
pub(crate) fn compute_tag_embedding(embedder: &impl Embedder, tag: &TagRow) -> Option<Vec<f32>> {
    let text = format!("{}: {}", tag.name, tag.definition);
    embedder.embed(&[text]).ok()?.pop()
}

/// Computes and stores the tag's embedding for its current version right
/// after the tag was saved, so zero-shot scoring rarely has to compute it
/// during review. The tag is already saved, so a failure here is not
/// reported as a failed save: `review::score_one_tag` computes a missing
/// embedding the next time the tag is scored.
pub(crate) fn embed_saved_tag(conn: &Connection, embedder: &impl Embedder, tag: &TagRow) {
    if let Some(vector) = compute_tag_embedding(embedder, tag) {
        let _ = core_lib::embeddings::store_tag_embedding(
            conn,
            tag.id,
            embedder.model_id(),
            tag.version,
            &vector,
        );
    }
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
    embed_saved_tag(&conn, &model.0, &tag);
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
        embed_saved_tag(&conn, &model.0, &updated.tag);
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

#[derive(Debug, Clone, Serialize)]
pub struct TagReviewItem {
    #[serde(flatten)]
    pub candidate: TagReviewCandidate,
    pub score: Option<f32>,
}

fn active_tag(conn: &Connection, tag_id: i64) -> Result<TagRow, String> {
    tags::get(conn, tag_id)
        .map_err(|e| e.to_string())?
        .filter(|t| !t.archived)
        .ok_or_else(|| format!("tag {tag_id} not found or archived"))
}

/// SPEC 7.1: "review this tag against existing documents", sorted by
/// descending score; unscored documents come last.
#[tauri::command]
pub fn tag_review_queue(
    state: State<Db>,
    model: State<Model>,
    tag_id: i64,
) -> Result<Vec<TagReviewItem>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    review_queue_for(&conn, &model.0, tag_id)
}

fn review_queue_for(
    conn: &Connection,
    embedder: &impl Embedder,
    tag_id: i64,
) -> Result<Vec<TagReviewItem>, String> {
    let tag = active_tag(conn, tag_id)?;
    let mut neighbours = None;
    let mut items = labels::tag_review_candidates(conn, &tag)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|candidate| {
            let score = score_for(conn, embedder, &tag, candidate.doc_id, &mut neighbours)?
                .map(|(score, _, _)| score);
            Ok(TagReviewItem { candidate, score })
        })
        .collect::<Result<Vec<_>, String>>()?;
    items.sort_by(|a, b| match (a.score, b.score) {
        (Some(x), Some(y)) => y.total_cmp(&x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    Ok(items)
}

type ScoreResult = Option<(f32, Option<&'static str>, String)>;

fn score_for(
    conn: &Connection,
    embedder: &impl Embedder,
    tag: &TagRow,
    doc_id: i64,
    neighbours: &mut Option<Vec<(i64, Vec<f32>)>>,
) -> Result<ScoreResult, String> {
    let Some(doc_vector) =
        core_lib::embeddings::get_document_embedding(conn, doc_id, embedder.model_id())
            .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    let (score, source, model_version) =
        crate::review::score_one_tag(conn, embedder, tag, &doc_vector, neighbours)
            .map_err(|e| e.to_string())?;
    Ok(score.map(|s| (s, source, model_version)))
}

/// Confirms `pos`/`neg` for one tag on an already-reviewed document (SPEC
/// 7.1). When the document was unknown for the tag, the score is recorded
/// first as a prediction (SPEC 7.4), just as validation does. A stale
/// label's document was part of the training data, so its score is not
/// out-of-sample and is not recorded.
#[tauri::command]
pub fn label_single_tag(
    state: State<Db>,
    model: State<Model>,
    doc_id: i64,
    tag_id: i64,
    positive: bool,
) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let now = crate::commands::current_timestamp();
    label_one(&conn, &model.0, doc_id, tag_id, positive, &now)
}

fn label_one(
    conn: &Connection,
    embedder: &impl Embedder,
    doc_id: i64,
    tag_id: i64,
    positive: bool,
    now: &str,
) -> Result<(), String> {
    let tag = active_tag(conn, tag_id)?;

    let unknown = !labels::has_any_label(conn, doc_id, tag.id).map_err(|e| e.to_string())?;
    if unknown {
        if let Some((score, source, model_version)) =
            score_for(conn, embedder, &tag, doc_id, &mut None)?
        {
            let suggested = crate::review::score_suggests(&tag, Some(score), source);
            core_lib::predictions::record(
                conn,
                doc_id,
                tag.id,
                &model_version,
                score,
                suggested,
                now,
            )
            .map_err(|e| e.to_string())?;
        }
    }

    let state = if positive {
        LabelState::Pos
    } else {
        LabelState::Neg
    };
    labels::write_human_label(conn, doc_id, &tag, state, now).map_err(|e| e.to_string())?;
    if positive {
        let checked: HashSet<i64> = [tag.id].into_iter().collect();
        crate::commands::enqueue_retrieval_after_tagging(conn, doc_id, &checked, now)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_lib::{documents, embeddings, storage};

    const NOW: &str = "2026-01-01T00:00:00Z";

    struct DirectionEmbedder;
    impl Embedder for DirectionEmbedder {
        fn model_id(&self) -> &str {
            "direction-model@v1"
        }
        fn dim(&self) -> usize {
            2
        }
        fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, embed_lib::EmbedError> {
            unreachable!("these tests only use stored embeddings")
        }
    }

    fn validated_doc(conn: &Connection, pub_key: &str, vector: &[f32]) -> i64 {
        documents::insert_pending(conn, pub_key, pub_key, NOW).unwrap();
        let doc = documents::find_by_pub_key(conn, pub_key).unwrap().unwrap();
        documents::store_fetched(
            conn,
            doc.id,
            &documents::FetchedData {
                title: Some("A gadget".to_string()),
                abstract_text: Some("An abstract".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        conn.execute(
            "UPDATE documents SET review_state = 'validated' WHERE id = ?1",
            [doc.id],
        )
        .unwrap();
        embeddings::store_document_embedding(conn, doc.id, "direction-model@v1", vector).unwrap();
        doc.id
    }

    fn tag_pointing_up(conn: &Connection) -> i64 {
        let tag_id = tags::create(conn, "Battery", "About batteries", None, None, NOW).unwrap();
        embeddings::store_tag_embedding(conn, tag_id, "direction-model@v1", 1, &[0.0, 1.0])
            .unwrap();
        tag_id
    }

    fn prediction_count(conn: &Connection, doc_id: i64) -> i64 {
        conn.query_row(
            "SELECT count(*) FROM predictions WHERE doc_id = ?1",
            [doc_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn review_queue_is_sorted_by_descending_score() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tag_pointing_up(&conn);
        let away = validated_doc(&conn, "EP1", &[0.0, -1.0]);
        let toward = validated_doc(&conn, "EP2", &[0.0, 1.0]);
        let sideways = validated_doc(&conn, "EP3", &[1.0, 0.0]);

        let queue = review_queue_for(&conn, &DirectionEmbedder, tag_id).unwrap();
        let order: Vec<i64> = queue.iter().map(|i| i.candidate.doc_id).collect();
        assert_eq!(order, vec![toward, sideways, away]);
        assert!(queue.iter().all(|i| i.score.is_some()));
    }

    #[test]
    fn labelling_an_unknown_document_records_a_prediction_first() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tag_pointing_up(&conn);
        let doc_id = validated_doc(&conn, "EP1", &[0.0, 1.0]);

        label_one(&conn, &DirectionEmbedder, doc_id, tag_id, true, NOW).unwrap();

        assert_eq!(prediction_count(&conn, doc_id), 1);
        let (state, _, _) = labels::get_label(&conn, doc_id, tag_id).unwrap().unwrap();
        assert_eq!(state, LabelState::Pos);
        assert!(review_queue_for(&conn, &DirectionEmbedder, tag_id)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn relabelling_a_stale_label_records_no_prediction() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tag_pointing_up(&conn);
        let doc_id = validated_doc(&conn, "EP1", &[0.0, 1.0]);
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();
        labels::write_human_label(&conn, doc_id, &tag, LabelState::Neg, NOW).unwrap();
        let tag = tags::update(
            &conn,
            tag_id,
            tags::TagFields {
                name: "Battery",
                definition: "About rechargeable batteries",
                parent_id: None,
                color: None,
                hotkey: None,
            },
            true,
        )
        .unwrap()
        .tag;
        embeddings::store_tag_embedding(
            &conn,
            tag_id,
            "direction-model@v1",
            tag.version,
            &[0.0, 1.0],
        )
        .unwrap();

        label_one(&conn, &DirectionEmbedder, doc_id, tag_id, true, NOW).unwrap();

        assert_eq!(prediction_count(&conn, doc_id), 0);
        let (state, _, _) = labels::get_label(&conn, doc_id, tag_id).unwrap().unwrap();
        assert_eq!(state, LabelState::Pos);
    }

    /// Embeds every text as "up", or fails when `fail` is set.
    struct UpEmbedder {
        fail: bool,
    }
    impl Embedder for UpEmbedder {
        fn model_id(&self) -> &str {
            "direction-model@v1"
        }
        fn dim(&self) -> usize {
            2
        }
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, embed_lib::EmbedError> {
            if self.fail {
                return Err(embed_lib::EmbedError::Tokenizer("unavailable".to_string()));
            }
            Ok(texts.iter().map(|_| vec![0.0, 1.0]).collect())
        }
    }

    #[test]
    fn scoring_computes_and_stores_a_missing_tag_embedding() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        validated_doc(&conn, "EP1", &[0.0, 1.0]);

        let queue = review_queue_for(&conn, &UpEmbedder { fail: false }, tag_id).unwrap();

        assert!(queue[0].score.is_some());
        assert_eq!(
            embeddings::get_tag_embedding(&conn, tag_id, "direction-model@v1", 1).unwrap(),
            Some(vec![0.0, 1.0])
        );
    }

    #[test]
    fn scoring_without_a_tag_embedding_survives_an_embedder_failure() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        validated_doc(&conn, "EP1", &[0.0, 1.0]);

        let queue = review_queue_for(&conn, &UpEmbedder { fail: true }, tag_id).unwrap();

        assert_eq!(queue[0].score, None);
        assert_eq!(
            embeddings::get_tag_embedding(&conn, tag_id, "direction-model@v1", 1).unwrap(),
            None
        );
    }

    #[test]
    fn archived_tags_have_no_review_queue() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tag_pointing_up(&conn);
        tags::archive(&conn, tag_id).unwrap();
        assert!(review_queue_for(&conn, &DirectionEmbedder, tag_id).is_err());
    }
}
