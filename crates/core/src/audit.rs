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

/// SPEC 7.6's full-automation safeguard: "if audited precision falls below
/// the target precision, or audited recall below the target recall, over
/// the last 50 audits for a tag, automatic mode for that tag is
/// suspended." Both metrics are computed from *one* shared window - the
/// most recent 50 automatic decisions (`pos` or `neg`) for `tag_id` that a
/// human has since confirmed or overturned (whether via an audit sample,
/// SPEC 7.6, or a focused-review validation, SPEC 7.5 - both produce the
/// same `label_history` shape once validated, since validating always
/// converts every label to `source = human`). This differs from
/// [`audited_precision`] (SPEC 7.5's separate, simpler per-tag automatic-
/// mode safeguard, which only ever considers `pos` decisions and keeps
/// its own 50-item window) - see docs/DECISIONS.md.
pub fn auto_completion_audit(conn: &Connection, tag_id: i64) -> Result<(Option<f32>, Option<f32>, i64), StorageError> {
    let mut stmt = conn.prepare(
        "SELECT a.state, (
            SELECT h.state FROM label_history h
            WHERE h.doc_id = a.doc_id AND h.tag_id = a.tag_id
              AND h.source = 'human' AND h.id > a.id
            ORDER BY h.id ASC LIMIT 1
         ) AS human_state
         FROM label_history a
         WHERE a.tag_id = ?1 AND a.source = 'auto'
           AND EXISTS (
             SELECT 1 FROM label_history h
             WHERE h.doc_id = a.doc_id AND h.tag_id = a.tag_id
               AND h.source = 'human' AND h.id > a.id
           )
         ORDER BY a.id DESC
         LIMIT ?2",
    )?;
    let pairs: Vec<(String, String)> = stmt
        .query_map(params![tag_id, AUDIT_WINDOW], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;

    let n = pairs.len() as i64;
    let (mut tp, mut fp, mut fn_) = (0u32, 0u32, 0u32);
    for (auto_state, human_state) in &pairs {
        match (auto_state.as_str(), human_state.as_str()) {
            ("pos", "pos") => tp += 1,
            ("pos", _) => fp += 1,
            ("neg", "pos") => fn_ += 1,
            _ => {} // ("neg", "neg"): a confirmed true negative, affecting neither metric's numerator or denominator.
        }
    }
    let precision = if tp + fp > 0 { Some(tp as f32 / (tp + fp) as f32) } else { None };
    let recall = if tp + fn_ > 0 { Some(tp as f32 / (tp + fn_) as f32) } else { None };
    Ok((precision, recall, n))
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

    /// Like [`auto_then_human`] but the automatic decision is `neg`
    /// (SPEC 7.6's "confident absence" side) - `human_confirms` means the
    /// human later validated it as `neg` too (a true negative); `false`
    /// means the human overturned it to `pos` (a missed positive, `fn_`).
    fn auto_neg_then_human(conn: &Connection, pub_key: &str, tag: &tags::TagRow, human_confirms: bool) {
        documents::insert_pending(conn, pub_key, pub_key, NOW).unwrap();
        let doc = documents::find_by_pub_key(conn, pub_key).unwrap().unwrap();
        labels::write_automatic_neg_label(conn, doc.id, tag.id, 0.05, "model@v1", 1, NOW).unwrap();
        let checked: HashSet<i64> =
            if human_confirms { HashSet::new() } else { [tag.id].into_iter().collect() };
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

    #[test]
    fn auto_completion_audit_computes_precision_and_recall_from_one_shared_window() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();

        // 8 confirmed auto-pos (TP), 2 overturned auto-pos (FP):
        // precision = 8/10 = 0.8.
        for i in 0..8 {
            auto_then_human(&conn, &format!("EPTP{i:04}"), &tag, true);
        }
        for i in 0..2 {
            auto_then_human(&conn, &format!("EPFP{i:04}"), &tag, false);
        }
        // 15 confirmed auto-neg (true negatives, affect neither metric),
        // 2 overturned auto-neg (FN): recall = TP/(TP+FN) = 8/10 = 0.8.
        for i in 0..15 {
            auto_neg_then_human(&conn, &format!("EPTN{i:04}"), &tag, true);
        }
        for i in 0..2 {
            auto_neg_then_human(&conn, &format!("EPFN{i:04}"), &tag, false);
        }

        let (precision, recall, n) = auto_completion_audit(&conn, tag_id).unwrap();
        assert_eq!(n, 27);
        assert_eq!(precision, Some(0.8));
        assert_eq!(recall, Some(0.8));
    }

    #[test]
    fn auto_completion_audit_window_keeps_only_the_last_50_combined() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();

        for i in 0..30 {
            auto_then_human(&conn, &format!("EPP{i:04}"), &tag, true);
        }
        for i in 0..30 {
            auto_neg_then_human(&conn, &format!("EPN{i:04}"), &tag, true);
        }

        let (_, _, n) = auto_completion_audit(&conn, tag_id).unwrap();
        assert_eq!(n, 50, "the window is shared across pos and neg decisions, not 50 of each");
    }

    #[test]
    fn auto_completion_audit_is_none_without_any_audited_decisions() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        assert_eq!(auto_completion_audit(&conn, tag_id).unwrap(), (None, None, 0));
    }
}
