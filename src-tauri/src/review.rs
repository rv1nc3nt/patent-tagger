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
    pub color: Option<String>,
    pub hotkey: Option<String>,
    pub score: Option<f32>,
    /// `"blend" | "lr" | "knn" | "zero_shot"`, absent when there's no score.
    pub source: Option<String>,
    /// A string identifying the embedding model and (when LR contributed)
    /// the classifier's training timestamp (SPEC 7.3's `model_version`),
    /// recorded alongside the score in `predictions` at validation time.
    pub model_version: String,
    /// Pre-checked in the UI: an existing human `pos` label, or a score at
    /// /above 0.5 from any source *except* zero-shot (SPEC 7.5's fallback
    /// threshold - no calibrated per-tag threshold until M6 - and 7.2:
    /// zero-shot suggestions are "shown as weak" and "never pre-checked").
    pub suggested: bool,
}

const DEFAULT_SUGGESTION_THRESHOLD: f32 = 0.5;

pub fn score_document(
    conn: &Connection,
    embedder: &impl Embedder,
    active_tags: &[TagRow],
    doc_id: i64,
) -> Result<Vec<TagScore>, storage::StorageError> {
    let doc_vector = embeddings::get_document_embedding(conn, doc_id, embedder.model_id())?;
    let already_positive = labels::positive_tag_ids(conn, doc_id)?;

    // Computed lazily and only once, since k-NN's candidate pool doesn't
    // depend on the tag.
    let mut validated_neighbours = None;

    let mut scores = Vec::with_capacity(active_tags.len());
    for tag in active_tags {
        let (score, source, model_version) = match &doc_vector {
            None => (None, None, embedder.model_id().to_string()),
            Some(doc_vector) => {
                let n_pos = core_lib::tags::count_human_positives(conn, tag.id)?;

                let zero_shot = if n_pos < scoring::ZERO_SHOT_POSITIVE_CEILING {
                    embeddings::get_tag_embedding(conn, tag.id, embedder.model_id(), tag.version)?
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

                let model_version = match trained_classifier.as_ref().and_then(|c| c.trained_at.as_deref()) {
                    Some(trained_at) => format!("{}+lr@{trained_at}", embedder.model_id()),
                    None => embedder.model_id().to_string(),
                };

                match scoring::blend_scores(lr, knn, zero_shot, n_pos) {
                    Some((score, source)) => (Some(score), Some(source), model_version),
                    None => (None, None, model_version),
                }
            }
        };

        let suggested = already_positive.contains(&tag.id)
            || (source != Some("zero_shot") && score.is_some_and(|s| s >= DEFAULT_SUGGESTION_THRESHOLD));

        scores.push(TagScore {
            tag_id: tag.id,
            name: tag.name.clone(),
            color: tag.color.clone(),
            hotkey: tag.hotkey.clone(),
            score,
            source: source.map(str::to_string),
            model_version,
            suggested,
        });
    }

    Ok(scores)
}

#[derive(Debug, Clone, Serialize)]
pub struct DocumentView {
    #[serde(flatten)]
    pub detail: documents::DocumentDetail,
    pub tags: Vec<TagScore>,
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
    Ok(Some(DocumentView { detail, tags }))
}
