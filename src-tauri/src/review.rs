//! Review screen support (SPEC section 8): scoring every active tag for a
//! document by blending whichever of LR/k-NN/zero-shot apply (SPEC 7.3).

use core_lib::rusqlite::Connection;
use core_lib::tags::TagRow;
use core_lib::{classifier, documents, embeddings, labels, scoring, storage};
use embed_lib::Embedder;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct TagScore {
    pub tag_id: i64,
    pub name: String,
    /// Organisational only: lets the Review screen show tags as a tree.
    pub parent_id: Option<i64>,
    pub color: Option<String>,
    pub hotkey: Option<String>,
    pub score: Option<f32>,
    /// `"blend" | "lr" | "knn" | "zero_shot"`, absent when there's no score.
    pub source: Option<String>,
    /// A string identifying the embedding model and (when LR contributed)
    /// the classifier's training timestamp (SPEC 7.3's `model_version`),
    /// recorded alongside the score in `predictions` at validation time.
    pub model_version: String,
    /// Pre-checked in the UI: an existing `pos` label (human or automatic -
    /// either represents a decision already made, worth showing as such),
    /// or a score at/above the tag's calibrated threshold (SPEC 7.5, 0.5
    /// if not yet calibrated) from any source *except* zero-shot (SPEC
    /// 7.2: zero-shot suggestions are "shown as weak" and "never
    /// pre-checked").
    pub suggested: bool,
    /// SPEC 8: "in focused review, confident automatic tags are shown as
    /// filled" - true when this tag already carries a `source = auto`
    /// label for this document (pos or neg), regardless of the *current*
    /// score (which may have moved since the label was written).
    pub automatic: bool,
    /// SPEC 8: "uncertain tags...are highlighted" - true when the tag is
    /// in automatic mode but neither confidently decided (no existing
    /// automatic label) nor a plain suggestion: its score falls between
    /// `neg_threshold` and `threshold`, or one/both aren't calibrated yet.
    pub uncertain: bool,
}

/// One tag's blended score for one document's embedding (SPEC 7.3). Pulled
/// out of `score_document` so automatic-label application (`automation.rs`)
/// can reuse the exact same scoring path instead of a parallel copy.
pub fn score_one_tag(
    conn: &Connection,
    embedder: &impl Embedder,
    tag: &TagRow,
    doc_vector: &[f32],
    validated_neighbours: &mut Option<Vec<(i64, Vec<f32>)>>,
) -> Result<(Option<f32>, Option<&'static str>, String), storage::StorageError> {
    let n_pos = core_lib::tags::count_human_positives(conn, tag.id)?;

    let zero_shot = if n_pos < scoring::ZERO_SHOT_POSITIVE_CEILING {
        tag_embedding(conn, embedder, tag)?
            .map(|tag_vector| scoring::zero_shot_score(doc_vector, &tag_vector))
    } else {
        None
    };

    let neighbours = validated_neighbours.get_or_insert_with(|| {
        embeddings::list_validated_document_embeddings(conn, embedder.model_id())
            .unwrap_or_default()
    });
    let labels_for_tag = labels::human_labels_for_tag(conn, tag.id)?;
    let knn = scoring::knn_score(doc_vector, neighbours, &labels_for_tag);

    let trained_classifier = classifier::load(conn, tag.id, embedder.model_id())?;
    let lr = trained_classifier.as_ref().map(|c| c.predict(doc_vector));

    let model_version = match trained_classifier
        .as_ref()
        .and_then(|c| c.trained_at.as_deref())
    {
        Some(trained_at) => format!("{}+lr@{trained_at}", embedder.model_id()),
        None => embedder.model_id().to_string(),
    };

    match scoring::blend_scores(lr, knn, zero_shot, n_pos) {
        Some((score, source)) => Ok((Some(score), Some(source), model_version)),
        None => Ok((None, None, model_version)),
    }
}

/// The tag's embedding for its current version. One is missing when
/// embedding failed after the tag was saved, or for a new embedding model:
/// it is computed and stored now. If the embedder fails again, the tag has
/// no zero-shot score this time.
fn tag_embedding(
    conn: &Connection,
    embedder: &impl Embedder,
    tag: &TagRow,
) -> Result<Option<Vec<f32>>, storage::StorageError> {
    if let Some(vector) =
        embeddings::get_tag_embedding(conn, tag.id, embedder.model_id(), tag.version)?
    {
        return Ok(Some(vector));
    }
    let Some(vector) = crate::tag_screen::compute_tag_embedding(embedder, tag) else {
        return Ok(None);
    };
    embeddings::store_tag_embedding(conn, tag.id, embedder.model_id(), tag.version, &vector)?;
    Ok(Some(vector))
}

/// Whether a score alone pre-checks the tag: at or above its calibrated
/// threshold (SPEC 7.5, 0.5 if not yet calibrated), from any source except
/// zero-shot, whose suggestions are "never pre-checked" (SPEC 7.2).
pub fn score_suggests(tag: &TagRow, score: Option<f32>, source: Option<&str>) -> bool {
    let effective_threshold = tag.threshold.unwrap_or(0.5);
    source != Some("zero_shot") && score.is_some_and(|s| s >= effective_threshold)
}

pub fn score_document(
    conn: &Connection,
    embedder: &impl Embedder,
    active_tags: &[TagRow],
    doc_id: i64,
) -> Result<Vec<TagScore>, storage::StorageError> {
    let doc_vector = embeddings::get_document_embedding(conn, doc_id, embedder.model_id())?;
    // Any existing pos label (human or automatic) - either represents a
    // decision already on record, worth pre-checking in the UI.
    let already_positive = labels::all_positive_tag_ids(conn, doc_id)?;
    // SPEC 8: "confident automatic tags are shown as filled" - tags with
    // an existing `source = auto` label (pos or neg) for this document.
    let auto_labelled = labels::auto_labelled_tag_ids(conn, doc_id)?;

    let mut validated_neighbours = None;

    let mut scores = Vec::with_capacity(active_tags.len());
    for tag in active_tags {
        let (score, source, model_version) = match &doc_vector {
            None => (None, None, embedder.model_id().to_string()),
            Some(doc_vector) => {
                score_one_tag(conn, embedder, tag, doc_vector, &mut validated_neighbours)?
            }
        };

        let suggested = already_positive.contains(&tag.id) || score_suggests(tag, score, source);

        let automatic = auto_labelled.contains(&tag.id);
        let uncertain = !automatic
            && matches!(
                core_lib::full_automation::decide_tag(tag, score),
                core_lib::full_automation::TagDecision::Uncertain
            );

        scores.push(TagScore {
            tag_id: tag.id,
            name: tag.name.clone(),
            parent_id: tag.parent_id,
            color: tag.color.clone(),
            hotkey: tag.hotkey.clone(),
            score,
            source: source.map(str::to_string),
            model_version,
            suggested,
            automatic,
            uncertain,
        });
    }

    Ok(scores)
}

#[derive(Debug, Clone, Serialize)]
pub struct DocumentView {
    #[serde(flatten)]
    pub detail: documents::DocumentDetail,
    pub tags: Vec<TagScore>,
    /// `None` when full text was never even attempted (SPEC 5.5's "never"
    /// policy and nobody has asked on demand yet) - the Description/Claims
    /// tabs only appear "when retrieved" (SPEC section 8).
    pub fulltext: Option<core_lib::fulltext::FulltextRow>,
    pub drawings_status: Option<core_lib::drawings::DrawingsStatusRow>,
    pub drawing_pages: Vec<core_lib::drawings::DrawingPage>,
}

pub fn document_view(
    conn: &Connection,
    embedder: &impl Embedder,
    doc_id: i64,
) -> Result<Option<DocumentView>, storage::StorageError> {
    let Some(detail) = documents::get_full(conn, doc_id)? else {
        return Ok(None);
    };
    let active_tags = core_lib::tags::list_active(conn)?;
    let tags = score_document(conn, embedder, &active_tags, doc_id)?;
    let fulltext = core_lib::fulltext::get(conn, doc_id)?;
    let drawings_status = core_lib::drawings::get_status(conn, doc_id)?;
    let drawing_pages = core_lib::drawings::list_pages(conn, doc_id)?;
    Ok(Some(DocumentView {
        detail,
        tags,
        fulltext,
        drawings_status,
        drawing_pages,
    }))
}

/// Reads one converted drawing page (or the `FirstPageClipping` thumbnail
/// at page 0) off disk for display in the webview. Returned as raw bytes
/// rather than a file path/URL, since the data directory isn't exposed
/// through the asset protocol (SPEC 4.2: the app "writes nowhere else",
/// and scoping the asset protocol to a data directory that moves with
/// portable mode adds complexity this doesn't need yet) - the frontend
/// builds a `Blob` from the bytes directly.
pub fn read_drawing_page(
    conn: &Connection,
    data_dir: &std::path::Path,
    doc_id: i64,
    page: i64,
) -> anyhow::Result<Vec<u8>> {
    let pages = core_lib::drawings::list_pages(conn, doc_id)?;
    let page_row = pages
        .into_iter()
        .find(|p| p.page == page)
        .ok_or_else(|| anyhow::anyhow!("no drawing page {page} for document {doc_id}"))?;
    Ok(std::fs::read(data_dir.join(&page_row.path))?)
}

/// SPEC section 8: the queue can be ordered by import order (the default,
/// `documents::list_queue`'s own order) or "most uncertain first (scores
/// closest to their thresholds)" - now meaningful once M6 gives every tag
/// a real calibrated threshold to be close to, rather than the constant
/// 0.5 stand-in M4 deferred this behind.
///
/// A document's "uncertainty" is the smallest `|score - threshold|` across
/// its tags with a defined score (the single most undecided tag drives
/// whether a human needs to look at it at all); documents with no scored
/// tags yet sort last, after everything with a real signal.
pub fn queue_ordered_by_uncertainty(
    conn: &Connection,
    embedder: &impl Embedder,
) -> Result<Vec<documents::QueueEntry>, storage::StorageError> {
    let active_tags = core_lib::tags::list_active(conn)?;
    let mut entries = documents::list_queue(conn)?;

    let mut distances = Vec::with_capacity(entries.len());
    for entry in &entries {
        let scores = score_document(conn, embedder, &active_tags, entry.id)?;
        let distance = scores
            .iter()
            .filter_map(|s| {
                s.score.map(|score| {
                    (score
                        - active_tags
                            .iter()
                            .find(|t| t.id == s.tag_id)
                            .and_then(|t| t.threshold)
                            .unwrap_or(0.5))
                    .abs()
                })
            })
            .fold(f32::INFINITY, f32::min);
        distances.push(distance);
    }

    let mut indices: Vec<usize> = (0..entries.len()).collect();
    indices.sort_by(|&a, &b| distances[a].total_cmp(&distances[b]));
    entries = indices.into_iter().map(|i| entries[i].clone()).collect();
    Ok(entries)
}
