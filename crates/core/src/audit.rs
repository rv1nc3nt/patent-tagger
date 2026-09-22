//! Auditing automatic decisions (SPEC 7.5's safeguard): "if audited
//! precision over the last 50 audited documents falls below the target,
//! automatic mode for that tag is disabled." Reuses `label_history` rather
//! than a dedicated audit table - every automatic `pos` label is already
//! recorded there, and so is the human decision that later confirms or
//! overturns it at validation time (see docs/DECISIONS.md for why M6
//! treats every such pair as "audited" rather than sampling a fraction,
//! which only has meaning once documents can skip review entirely, at M9).

use crate::storage::StorageError;
use rusqlite::{params, Connection};

const AUDIT_WINDOW: i64 = 50;

/// Fraction of the last (up to) 50 automatic `pos` decisions for `tag_id`
/// that a human later confirmed as `pos` too, paired with how many such
/// audited decisions exist. `None` precision when there are none yet.
pub fn audited_precision(conn: &Connection, tag_id: i64) -> Result<(Option<f32>, i64), StorageError> {
    // For each `auto`/`pos` label_history row, find the next `human` row
    // for the same document+tag (the validation that audited it).
    let mut stmt = conn.prepare(
        "SELECT (
            SELECT h.state FROM label_history h
            WHERE h.doc_id = a.doc_id AND h.tag_id = a.tag_id
              AND h.source = 'human' AND h.id > a.id
            ORDER BY h.id ASC LIMIT 1
         ) AS human_state
         FROM label_history a
         WHERE a.tag_id = ?1 AND a.source = 'auto' AND a.state = 'pos'
           AND EXISTS (
             SELECT 1 FROM label_history h
             WHERE h.doc_id = a.doc_id AND h.tag_id = a.tag_id
               AND h.source = 'human' AND h.id > a.id
           )
         ORDER BY a.id DESC
         LIMIT ?2",
    )?;
    let states: Vec<String> = stmt
        .query_map(params![tag_id, AUDIT_WINDOW], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    let n = states.len() as i64;
    if n == 0 {
        return Ok((None, 0));
    }
    let confirmed = states.iter().filter(|s| s.as_str() == "pos").count() as f32;
    Ok((Some(confirmed / n as f32), n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{documents, labels, storage, tags};
    use std::collections::HashSet;

    const NOW: &str = "2026-01-01T00:00:00Z";

    fn auto_then_human(conn: &Connection, pub_key: &str, tag: &tags::TagRow, human_confirms: bool) {
        documents::insert_pending(conn, pub_key, pub_key, NOW).unwrap();
        let doc = documents::find_by_pub_key(conn, pub_key).unwrap().unwrap();
        labels::write_automatic_label(conn, doc.id, tag.id, 0.9, "model@v1", 1, NOW).unwrap();
        let checked: HashSet<i64> =
            if human_confirms { [tag.id].into_iter().collect() } else { HashSet::new() };
        labels::validate_document(conn, doc.id, std::slice::from_ref(tag), &checked, NOW).unwrap();
    }

    #[test]
    fn no_audited_decisions_yet_is_none() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        assert_eq!(audited_precision(&conn, tag_id).unwrap(), (None, 0));
    }

    #[test]
    fn precision_reflects_confirmed_vs_overturned_auto_decisions() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();

        auto_then_human(&conn, "EP0000001", &tag, true);
        auto_then_human(&conn, "EP0000002", &tag, true);
        auto_then_human(&conn, "EP0000003", &tag, true);
        auto_then_human(&conn, "EP0000004", &tag, false); // overturned

        let (precision, n) = audited_precision(&conn, tag_id).unwrap();
        assert_eq!(n, 4);
        assert_eq!(precision, Some(0.75));
    }

    #[test]
    fn an_auto_label_not_yet_validated_by_a_human_is_not_counted() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();

        documents::insert_pending(&conn, "EP0000001", "EP0000001", NOW).unwrap();
        let doc = documents::find_by_pub_key(&conn, "EP0000001").unwrap().unwrap();
        labels::write_automatic_label(&conn, doc.id, tag.id, 0.9, "model@v1", 1, NOW).unwrap();

        assert_eq!(audited_precision(&conn, tag_id).unwrap(), (None, 0));
    }

    #[test]
    fn window_keeps_only_the_last_50() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();

        for i in 0..55 {
            auto_then_human(&conn, &format!("EP{i:07}"), &tag, true);
        }

        let (_, n) = audited_precision(&conn, tag_id).unwrap();
        assert_eq!(n, 50);
    }
}
