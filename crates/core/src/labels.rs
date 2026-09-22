//! Persistence for `labels` and `label_history` (SPEC section 4.2, 7.1).
//! Validating a document means every non-archived tag has been considered:
//! checked tags become `pos`, unchecked become `neg`, both `source =
//! "human"`.

use crate::storage::StorageError;
use crate::tags::TagRow;
use rusqlite::{params, Connection};
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
    let mut stmt = conn.prepare(
        "SELECT doc_id, state FROM labels WHERE tag_id = ?1 AND source = 'human'",
    )?;
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
            let state = if state == "pos" { LabelState::Pos } else { LabelState::Neg };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{documents, storage, tags};

    const NOW: &str = "2026-01-01T00:00:00Z";

    fn setup(conn: &Connection) -> (i64, TagRow, TagRow) {
        documents::insert_pending(conn, "EP1234567", "EP1234567", NOW).unwrap();
        let doc = documents::find_by_pub_key(conn, "EP1234567").unwrap().unwrap();
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
        validate_document(&conn, doc_id, &[battery.clone(), solar.clone()], &checked, NOW).unwrap();

        let battery_labels = human_labels_for_tag(&conn, battery.id).unwrap();
        assert_eq!(battery_labels.get(&doc_id), Some(&LabelState::Pos));

        let solar_labels = human_labels_for_tag(&conn, solar.id).unwrap();
        assert_eq!(solar_labels.get(&doc_id), Some(&LabelState::Neg));
    }

    #[test]
    fn checked_tags_become_positive_unchecked_become_negative() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let (doc_id, battery, solar) = setup(&conn);
        let checked: HashSet<i64> = [battery.id].into_iter().collect();

        validate_document(&conn, doc_id, &[battery.clone(), solar.clone()], &checked, NOW).unwrap();

        let positives = positive_tag_ids(&conn, doc_id).unwrap();
        assert!(positives.contains(&battery.id));
        assert!(!positives.contains(&solar.id));

        let doc = documents::find_by_pub_key(&conn, "EP1234567").unwrap().unwrap();
        assert_eq!(doc.review_state, "validated");
    }

    #[test]
    fn validating_twice_updates_rather_than_duplicating() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let (doc_id, battery, solar) = setup(&conn);

        let first_checked: HashSet<i64> = [battery.id].into_iter().collect();
        validate_document(&conn, doc_id, &[battery.clone(), solar.clone()], &first_checked, NOW).unwrap();

        let second_checked: HashSet<i64> = [solar.id].into_iter().collect();
        validate_document(&conn, doc_id, &[battery.clone(), solar.clone()], &second_checked, NOW).unwrap();

        let positives = positive_tag_ids(&conn, doc_id).unwrap();
        assert!(!positives.contains(&battery.id));
        assert!(positives.contains(&solar.id));

        let count: i64 = conn
            .query_row("SELECT count(*) FROM labels WHERE doc_id = ?1", params![doc_id], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2, "labels should be updated in place, not duplicated");

        let history_count: i64 = conn
            .query_row("SELECT count(*) FROM label_history WHERE doc_id = ?1", params![doc_id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(history_count, 4, "label_history is append-only across both validations");
    }

    #[test]
    fn skip_sets_review_state_without_writing_labels() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let (doc_id, _battery, _solar) = setup(&conn);

        skip_document(&conn, doc_id).unwrap();

        let doc = documents::find_by_pub_key(&conn, "EP1234567").unwrap().unwrap();
        assert_eq!(doc.review_state, "skipped");
        assert!(positive_tag_ids(&conn, doc_id).unwrap().is_empty());
    }
}
