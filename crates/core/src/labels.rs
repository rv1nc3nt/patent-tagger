//! Persistence for `labels` and `label_history` (SPEC section 4.2, 7.1).
//! Validating a document means every non-archived tag has been considered:
//! checked tags become `pos`, unchecked become `neg`, both `source =
//! "human"`.

use crate::storage::StorageError;
use crate::tags::TagRow;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelState {
    Pos,
    Neg,
}

impl LabelState {
    fn as_str(self) -> &'static str {
        match self {
            LabelState::Pos => "pos",
            LabelState::Neg => "neg",
        }
    }
}

/// Writes a human `pos`/`neg` label for every tag in `active_tags`
/// (checked tags in `checked_tag_ids` get `pos`, the rest `neg`), appends
/// each to `label_history`, and sets the document to `review_state =
/// "validated"`.
pub fn validate_document(
    conn: &Connection,
    doc_id: i64,
    active_tags: &[TagRow],
    checked_tag_ids: &HashSet<i64>,
    now: &str,
) -> Result<(), StorageError> {
    for tag in active_tags {
        let state = if checked_tag_ids.contains(&tag.id) {
            LabelState::Pos
        } else {
            LabelState::Neg
        };
        conn.execute(
            "INSERT INTO labels (doc_id, tag_id, state, source, tag_version, created_at)
             VALUES (?1, ?2, ?3, 'human', ?4, ?5)
             ON CONFLICT (doc_id, tag_id) DO UPDATE SET
                state = excluded.state, source = excluded.source,
                tag_version = excluded.tag_version, created_at = excluded.created_at",
            params![doc_id, tag.id, state.as_str(), tag.version, now],
        )?;
        conn.execute(
            "INSERT INTO label_history (doc_id, tag_id, state, source, tag_version, created_at)
             VALUES (?1, ?2, ?3, 'human', ?4, ?5)",
            params![doc_id, tag.id, state.as_str(), tag.version, now],
        )?;
    }

    conn.execute(
        "UPDATE documents SET review_state = 'validated', validated_at = ?1 WHERE id = ?2",
        params![now, doc_id],
    )?;
    Ok(())
}

/// Writes an automatic `pos` label (SPEC 7.5: "writes pos labels with
/// source = auto, the confidence, and the model_version, for scores >=
/// threshold") - but never overwrites an existing human decision (the
/// conditional `WHERE labels.source != 'human'` on the upsert), and skips
/// `label_history` when that happens, since nothing actually changed.
/// Returns whether it actually wrote anything.
pub fn write_automatic_label(
    conn: &Connection,
    doc_id: i64,
    tag_id: i64,
    confidence: f32,
    model_version: &str,
    tag_version: i64,
    now: &str,
) -> Result<bool, StorageError> {
    write_automatic(
        conn,
        doc_id,
        tag_id,
        LabelState::Pos,
        confidence,
        model_version,
        tag_version,
        now,
    )
}

/// Writes an automatic `neg` label (SPEC 7.6: "confident absence" - a
/// score below `neg_threshold`). Only ever called under full automation
/// (SPEC 7.5: "outside full automation...the system never writes
/// automatic negatives") - that gate is the caller's job, same as
/// `write_automatic_label`'s pos side never checking a tag's automatic
/// mode itself.
pub fn write_automatic_neg_label(
    conn: &Connection,
    doc_id: i64,
    tag_id: i64,
    confidence: f32,
    model_version: &str,
    tag_version: i64,
    now: &str,
) -> Result<bool, StorageError> {
    write_automatic(
        conn,
        doc_id,
        tag_id,
        LabelState::Neg,
        confidence,
        model_version,
        tag_version,
        now,
    )
}

#[allow(clippy::too_many_arguments)]
fn write_automatic(
    conn: &Connection,
    doc_id: i64,
    tag_id: i64,
    state: LabelState,
    confidence: f32,
    model_version: &str,
    tag_version: i64,
    now: &str,
) -> Result<bool, StorageError> {
    let changed = conn.execute(
        "INSERT INTO labels (doc_id, tag_id, state, source, confidence, model_version, tag_version, created_at)
         VALUES (?1, ?2, ?3, 'auto', ?4, ?5, ?6, ?7)
         ON CONFLICT (doc_id, tag_id) DO UPDATE SET
            state = excluded.state, source = excluded.source, confidence = excluded.confidence,
            model_version = excluded.model_version, tag_version = excluded.tag_version,
            created_at = excluded.created_at
         WHERE labels.source != 'human'",
        params![doc_id, tag_id, state.as_str(), confidence, model_version, tag_version, now],
    )? > 0;

    if changed {
        conn.execute(
            "INSERT INTO label_history (doc_id, tag_id, state, source, confidence, model_version, tag_version, created_at)
             VALUES (?1, ?2, ?3, 'auto', ?4, ?5, ?6, ?7)",
            params![doc_id, tag_id, state.as_str(), confidence, model_version, tag_version, now],
        )?;
    }
    Ok(changed)
}

pub fn skip_document(conn: &Connection, doc_id: i64) -> Result<(), StorageError> {
    conn.execute(
        "UPDATE documents SET review_state = 'skipped' WHERE id = ?1",
        params![doc_id],
    )?;
    Ok(())
}

/// Every document's human label for `tag_id` (SPEC 7.2: the k-NN pool of
/// "neighbours with a human label for t").
pub fn human_labels_for_tag(
    conn: &Connection,
    tag_id: i64,
) -> Result<HashMap<i64, LabelState>, StorageError> {
    let mut stmt =
        conn.prepare("SELECT doc_id, state FROM labels WHERE tag_id = ?1 AND source = 'human'")?;
    let rows = stmt
        .query_map(params![tag_id], |row| {
            let doc_id: i64 = row.get(0)?;
            let state: String = row.get(1)?;
            Ok((doc_id, state))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .map(|(doc_id, state)| {
            let state = if state == "pos" {
                LabelState::Pos
            } else {
                LabelState::Neg
            };
            (doc_id, state)
        })
        .collect())
}

/// The tag IDs this document has a human `pos` label for, e.g. to know
/// which checkboxes to pre-check when re-opening an already-validated
/// document.
pub fn positive_tag_ids(conn: &Connection, doc_id: i64) -> Result<HashSet<i64>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT tag_id FROM labels WHERE doc_id = ?1 AND state = 'pos' AND source = 'human'",
    )?;
    let rows = stmt
        .query_map(params![doc_id], |row| row.get::<_, i64>(0))?
        .collect::<Result<HashSet<_>, _>>()?;
    Ok(rows)
}

/// Whether `(doc_id, tag_id)` already has a label at all, regardless of
/// state or source - used to decide whether an automatic decision still
/// needs to be made (SPEC 7.5), since a document only reaches this check
/// while still in the review queue (never yet human-validated), so any
/// existing label there can only be a prior automatic one.
pub fn has_any_label(conn: &Connection, doc_id: i64, tag_id: i64) -> Result<bool, StorageError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM labels WHERE doc_id = ?1 AND tag_id = ?2)",
        params![doc_id, tag_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|n| n != 0)
    .map_err(StorageError::from)
}

/// The current `(state, confidence, model_version)` of `(doc_id,
/// tag_id)`'s label, if any - used by full automation (SPEC 7.6) to reuse
/// an already-decided tag's state/score without re-scoring it.
pub fn get_label(
    conn: &Connection,
    doc_id: i64,
    tag_id: i64,
) -> Result<Option<(LabelState, Option<f32>, Option<String>)>, StorageError> {
    conn.query_row(
        "SELECT state, confidence, model_version FROM labels WHERE doc_id = ?1 AND tag_id = ?2",
        params![doc_id, tag_id],
        |row| {
            let state: String = row.get(0)?;
            let confidence: Option<f64> = row.get(1)?;
            let model_version: Option<String> = row.get(2)?;
            Ok((
                if state == "pos" {
                    LabelState::Pos
                } else {
                    LabelState::Neg
                },
                confidence.map(|c| c as f32),
                model_version,
            ))
        },
    )
    .optional()
    .map_err(StorageError::from)
}

/// Tag IDs this document currently has an *automatic* label for - called
/// right before `validate_document` overwrites them, to know which tags
/// this validation is about to audit (SPEC 7.5).
pub fn auto_labelled_tag_ids(conn: &Connection, doc_id: i64) -> Result<HashSet<i64>, StorageError> {
    let mut stmt =
        conn.prepare("SELECT tag_id FROM labels WHERE doc_id = ?1 AND source = 'auto'")?;
    let rows = stmt
        .query_map(params![doc_id], |row| row.get::<_, i64>(0))?
        .collect::<Result<HashSet<_>, _>>()?;
    Ok(rows)
}

/// Like [`positive_tag_ids`], but any source - an automatic `pos` decision
/// (SPEC 7.5) is still a decision already on record, worth pre-checking in
/// the Review screen just like a human one.
pub fn all_positive_tag_ids(conn: &Connection, doc_id: i64) -> Result<HashSet<i64>, StorageError> {
    let mut stmt = conn.prepare("SELECT tag_id FROM labels WHERE doc_id = ?1 AND state = 'pos'")?;
    let rows = stmt
        .query_map(params![doc_id], |row| row.get::<_, i64>(0))?
        .collect::<Result<HashSet<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{documents, storage, tags};

    const NOW: &str = "2026-01-01T00:00:00Z";

    fn setup(conn: &Connection) -> (i64, TagRow, TagRow) {
        documents::insert_pending(conn, "EP1234567", "EP1234567", NOW).unwrap();
        let doc = documents::find_by_pub_key(conn, "EP1234567")
            .unwrap()
            .unwrap();
        let battery_id = tags::create(conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let solar_id = tags::create(conn, "Solar", "About solar power", None, None, NOW).unwrap();
        (
            doc.id,
            tags::get(conn, battery_id).unwrap().unwrap(),
            tags::get(conn, solar_id).unwrap().unwrap(),
        )
    }

    #[test]
    fn human_labels_for_tag_reports_both_pos_and_neg() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let (doc_id, battery, solar) = setup(&conn);
        let checked: HashSet<i64> = [battery.id].into_iter().collect();
        validate_document(
            &conn,
            doc_id,
            &[battery.clone(), solar.clone()],
            &checked,
            NOW,
        )
        .unwrap();

        let battery_labels = human_labels_for_tag(&conn, battery.id).unwrap();
        assert_eq!(battery_labels.get(&doc_id), Some(&LabelState::Pos));

        let solar_labels = human_labels_for_tag(&conn, solar.id).unwrap();
        assert_eq!(solar_labels.get(&doc_id), Some(&LabelState::Neg));
    }

    #[test]
    fn auto_labelled_tag_ids_reflects_only_automatic_labels() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let (doc_id, battery, solar) = setup(&conn);

        write_automatic_label(&conn, doc_id, battery.id, 0.9, "model@v1", 1, NOW).unwrap();
        let checked: HashSet<i64> = [solar.id].into_iter().collect();
        validate_document(&conn, doc_id, std::slice::from_ref(&solar), &checked, NOW).unwrap();

        let auto_tags = auto_labelled_tag_ids(&conn, doc_id).unwrap();
        assert!(auto_tags.contains(&battery.id));
        assert!(
            !auto_tags.contains(&solar.id),
            "solar was labelled by a human, not automatically"
        );
    }

    #[test]
    fn all_positive_tag_ids_includes_automatic_labels() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let (doc_id, battery, solar) = setup(&conn);

        write_automatic_label(&conn, doc_id, battery.id, 0.9, "model@v1", 1, NOW).unwrap();
        let checked: HashSet<i64> = [solar.id].into_iter().collect();
        validate_document(&conn, doc_id, std::slice::from_ref(&solar), &checked, NOW).unwrap();

        let positives = all_positive_tag_ids(&conn, doc_id).unwrap();
        assert!(
            positives.contains(&battery.id),
            "the automatic label should be included"
        );
        assert!(
            positives.contains(&solar.id),
            "the human label should be included"
        );
    }

    #[test]
    fn checked_tags_become_positive_unchecked_become_negative() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let (doc_id, battery, solar) = setup(&conn);
        let checked: HashSet<i64> = [battery.id].into_iter().collect();

        validate_document(
            &conn,
            doc_id,
            &[battery.clone(), solar.clone()],
            &checked,
            NOW,
        )
        .unwrap();

        let positives = positive_tag_ids(&conn, doc_id).unwrap();
        assert!(positives.contains(&battery.id));
        assert!(!positives.contains(&solar.id));

        let doc = documents::find_by_pub_key(&conn, "EP1234567")
            .unwrap()
            .unwrap();
        assert_eq!(doc.review_state, "validated");
    }

    #[test]
    fn validating_twice_updates_rather_than_duplicating() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let (doc_id, battery, solar) = setup(&conn);

        let first_checked: HashSet<i64> = [battery.id].into_iter().collect();
        validate_document(
            &conn,
            doc_id,
            &[battery.clone(), solar.clone()],
            &first_checked,
            NOW,
        )
        .unwrap();

        let second_checked: HashSet<i64> = [solar.id].into_iter().collect();
        validate_document(
            &conn,
            doc_id,
            &[battery.clone(), solar.clone()],
            &second_checked,
            NOW,
        )
        .unwrap();

        let positives = positive_tag_ids(&conn, doc_id).unwrap();
        assert!(!positives.contains(&battery.id));
        assert!(positives.contains(&solar.id));

        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM labels WHERE doc_id = ?1",
                params![doc_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 2,
            "labels should be updated in place, not duplicated"
        );

        let history_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM label_history WHERE doc_id = ?1",
                params![doc_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            history_count, 4,
            "label_history is append-only across both validations"
        );
    }

    #[test]
    fn skip_sets_review_state_without_writing_labels() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let (doc_id, _battery, _solar) = setup(&conn);

        skip_document(&conn, doc_id).unwrap();

        let doc = documents::find_by_pub_key(&conn, "EP1234567")
            .unwrap()
            .unwrap();
        assert_eq!(doc.review_state, "skipped");
        assert!(positive_tag_ids(&conn, doc_id).unwrap().is_empty());
    }

    #[test]
    fn write_automatic_label_sets_pos_with_confidence_and_model_version() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let (doc_id, battery, _solar) = setup(&conn);

        let changed =
            write_automatic_label(&conn, doc_id, battery.id, 0.87, "model@v1", 1, NOW).unwrap();
        assert!(changed);

        let (state, source, confidence): (String, String, f64) = conn
            .query_row(
                "SELECT state, source, confidence FROM labels WHERE doc_id = ?1 AND tag_id = ?2",
                params![doc_id, battery.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(state, "pos");
        assert_eq!(source, "auto");
        assert!((confidence - 0.87).abs() < 1e-6);
    }

    #[test]
    fn get_label_returns_state_and_confidence_or_none() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let (doc_id, battery, _solar) = setup(&conn);

        assert_eq!(get_label(&conn, doc_id, battery.id).unwrap(), None);

        write_automatic_label(&conn, doc_id, battery.id, 0.87, "model@v1", 1, NOW).unwrap();
        let (state, confidence, model_version) =
            get_label(&conn, doc_id, battery.id).unwrap().unwrap();
        assert_eq!(state, LabelState::Pos);
        assert!((confidence.unwrap() - 0.87).abs() < 1e-6);
        assert_eq!(model_version.as_deref(), Some("model@v1"));
    }

    #[test]
    fn write_automatic_neg_label_sets_neg_with_confidence_and_model_version() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let (doc_id, battery, _solar) = setup(&conn);

        let changed =
            write_automatic_neg_label(&conn, doc_id, battery.id, 0.92, "model@v1", 1, NOW).unwrap();
        assert!(changed);

        let (state, source): (String, String) = conn
            .query_row(
                "SELECT state, source FROM labels WHERE doc_id = ?1 AND tag_id = ?2",
                params![doc_id, battery.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, "neg");
        assert_eq!(source, "auto");
    }

    #[test]
    fn write_automatic_label_never_overwrites_an_existing_human_label() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let (doc_id, battery, solar) = setup(&conn);

        // Human explicitly said "no" to battery.
        let checked: HashSet<i64> = [solar.id].into_iter().collect();
        validate_document(
            &conn,
            doc_id,
            &[battery.clone(), solar.clone()],
            &checked,
            NOW,
        )
        .unwrap();

        let changed =
            write_automatic_label(&conn, doc_id, battery.id, 0.99, "model@v1", 1, NOW).unwrap();
        assert!(
            !changed,
            "an automatic label must never overwrite a human decision"
        );

        let state: String = conn
            .query_row(
                "SELECT state FROM labels WHERE doc_id = ?1 AND tag_id = ?2",
                params![doc_id, battery.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "neg", "the human's neg decision must survive");
    }
}
