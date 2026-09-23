//! Library search (SPEC section 8): full-text search over abstracts (FTS5)
//! combined with tag/label-source/review-state/full-text/drawings-
//! availability filters, plus "similar to this document" (embedding
//! search). Bulk retrieval of full text/drawings for a selection is
//! `src-tauri`'s job, driving this module's search only to pick the
//! selection.

use crate::storage::StorageError;
use rusqlite::{params, Connection};

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct LibraryFilters {
    /// FTS5 query over title + abstract; `None`/empty matches everything.
    pub query: Option<String>,
    /// Documents must have a `pos` label for *every* tag listed here.
    pub include_tag_ids: Vec<i64>,
    /// Documents must have a `pos` label for *none* of the tags listed here.
    pub exclude_tag_ids: Vec<i64>,
    /// `"human" | "auto"` - documents with at least one label from this
    /// source.
    pub label_source: Option<String>,
    /// `"validated" | "auto_completed" | "queued" | "skipped"`.
    pub review_state: Option<String>,
    /// `"available" | "unavailable"` - "available" means `fulltext.status`
    /// is `fetched` or `non_english_only` (SPEC 5.5: text was retrieved,
    /// even if not in English); a missing `fulltext` row counts the same
    /// as `unavailable`.
    pub fulltext_availability: Option<String>,
    /// `"available" | "unavailable"` - "available" means
    /// `drawings_status.status = 'fetched'`; a missing row counts the
    /// same as `unavailable`.
    pub drawings_availability: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct LibraryRow {
    pub id: i64,
    pub pub_key: String,
    pub title: Option<String>,
    pub review_state: String,
    pub tags: Vec<String>,
}

/// Applies `filters` and returns matching documents, newest first. The
/// base query (FTS5 text search, label source, review state) is built
/// with `?N` placeholders bound via `bind_values` - filter values are
/// never interpolated into the SQL string itself. Tag include/exclude
/// filtering happens afterward, in Rust, rather than as dynamic `EXISTS`
/// subqueries per tag - simpler at the personal-library scale this app
/// targets, and sidesteps building a variable-length `IN (...)` list.
pub fn search(conn: &Connection, filters: &LibraryFilters) -> Result<Vec<LibraryRow>, StorageError> {
    let mut sql = String::from(
        "SELECT DISTINCT d.id, d.pub_key, d.title, d.review_state FROM documents d",
    );
    let mut conditions: Vec<String> = vec!["d.fetch_status = 'fetched'".to_string()];
    // Bound in the same order the `?N` placeholders below are assigned -
    // never string-interpolated into the SQL itself.
    let mut bind_values: Vec<String> = Vec::new();

    if let Some(q) = filters.query.as_deref().filter(|q| !q.trim().is_empty()) {
        sql.push_str(" JOIN documents_fts fts ON fts.rowid = d.id");
        bind_values.push(q.to_string());
        conditions.push(format!("documents_fts MATCH ?{}", bind_values.len()));
    }
    if let Some(source) = &filters.label_source {
        bind_values.push(if source == "auto" { "auto".to_string() } else { "human".to_string() });
        conditions.push(format!(
            "EXISTS (SELECT 1 FROM labels l WHERE l.doc_id = d.id AND l.source = ?{})",
            bind_values.len()
        ));
    }
    if let Some(state) = &filters.review_state {
        bind_values.push(state.clone());
        conditions.push(format!("d.review_state = ?{}", bind_values.len()));
    }
    if let Some(availability) = &filters.fulltext_availability {
        let exists = "EXISTS (SELECT 1 FROM fulltext ft WHERE ft.doc_id = d.id AND ft.status IN ('fetched', 'non_english_only'))";
        conditions.push(if availability == "available" {
            exists.to_string()
        } else {
            format!("NOT {exists}")
        });
    }
    if let Some(availability) = &filters.drawings_availability {
        let exists =
            "EXISTS (SELECT 1 FROM drawings_status ds WHERE ds.doc_id = d.id AND ds.status = 'fetched')";
        conditions.push(if availability == "available" {
            exists.to_string()
        } else {
            format!("NOT {exists}")
        });
    }

    sql.push_str(" WHERE ");
    sql.push_str(&conditions.join(" AND "));
    sql.push_str(" ORDER BY d.id DESC");

    let mut stmt = conn.prepare(&sql)?;
    let mut rows: Vec<(i64, String, Option<String>, String)> = stmt
        .query_map(rusqlite::params_from_iter(bind_values.iter()), |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut tag_stmt = conn.prepare(
        "SELECT t.name FROM labels l JOIN tags t ON t.id = l.tag_id
         WHERE l.doc_id = ?1 AND l.state = 'pos' ORDER BY t.name ASC",
    )?;
    let mut pos_tag_ids_stmt =
        conn.prepare("SELECT tag_id FROM labels WHERE doc_id = ?1 AND state = 'pos'")?;

    let mut result = Vec::new();
    for (id, pub_key, title, review_state) in rows.drain(..) {
        let pos_tag_ids: std::collections::HashSet<i64> = pos_tag_ids_stmt
            .query_map(params![id], |row| row.get::<_, i64>(0))?
            .collect::<Result<_, _>>()?;

        if !filters.include_tag_ids.iter().all(|t| pos_tag_ids.contains(t)) {
            continue;
        }
        if filters.exclude_tag_ids.iter().any(|t| pos_tag_ids.contains(t)) {
            continue;
        }

        let tags = tag_stmt
            .query_map(params![id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        result.push(LibraryRow { id, pub_key, title, review_state, tags });
    }
    Ok(result)
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SimilarDocument {
    pub id: i64,
    pub pub_key: String,
    pub title: Option<String>,
    pub similarity: f32,
}

/// "Similar to this document" (SPEC section 8): every other fetched
/// document with an embedding, ranked by cosine similarity, most similar
/// first.
pub fn similar_to(
    conn: &Connection,
    doc_id: i64,
    model_id: &str,
    limit: usize,
) -> Result<Vec<SimilarDocument>, StorageError> {
    let Some(target) = crate::embeddings::get_document_embedding(conn, doc_id, model_id)? else {
        return Ok(vec![]);
    };

    let mut stmt = conn.prepare(
        "SELECT d.id, d.pub_key, d.title, e.vector FROM embeddings e
         JOIN documents d ON d.id = e.doc_id
         WHERE e.model_id = ?1 AND d.id != ?2",
    )?;
    let candidates: Vec<(i64, String, Option<String>, Vec<u8>)> = stmt
        .query_map(params![model_id, doc_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut scored: Vec<SimilarDocument> = candidates
        .into_iter()
        .map(|(id, pub_key, title, bytes)| {
            let vector: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes(c.try_into().expect("chunks_exact(4)")))
                .collect();
            let similarity = crate::scoring::cosine_similarity(&target, &vector);
            SimilarDocument { id, pub_key, title, similarity }
        })
        .collect();
    scored.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));
    scored.truncate(limit);
    Ok(scored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{documents, embeddings, labels, storage, tags};
    use std::collections::HashSet;

    const NOW: &str = "2026-01-01T00:00:00Z";
    const MODEL: &str = "test-model@abc";

    fn fetched_doc(conn: &Connection, pub_key: &str, title: &str, abstract_text: &str) -> i64 {
        documents::insert_pending(conn, pub_key, pub_key, NOW).unwrap();
        let doc = documents::find_by_pub_key(conn, pub_key).unwrap().unwrap();
        documents::store_fetched(
            conn,
            doc.id,
            &documents::FetchedData {
                title: Some(title.to_string()),
                abstract_text: Some(abstract_text.to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        doc.id
    }

    #[test]
    fn full_text_search_matches_title_and_abstract() {
        let conn = storage::open_in_memory().expect("in-memory db");
        fetched_doc(&conn, "EP1111111", "Apparatus for bricks", "About manufacturing bricks");
        fetched_doc(&conn, "EP2222222", "Telescope mount", "About observing stars");

        let filters = LibraryFilters { query: Some("bricks".to_string()), ..Default::default() };
        let rows = search(&conn, &filters).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].pub_key, "EP1111111");
    }

    #[test]
    fn include_tag_filter_requires_all_listed_tags() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let battery_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let solar_id = tags::create(&conn, "Solar", "About solar", None, None, NOW).unwrap();
        let battery = tags::get(&conn, battery_id).unwrap().unwrap();
        let solar = tags::get(&conn, solar_id).unwrap().unwrap();

        let both_id = fetched_doc(&conn, "EP1111111", "A", "a");
        labels::validate_document(&conn, both_id, &[battery.clone(), solar.clone()], &[battery_id, solar_id].into_iter().collect::<HashSet<_>>(), NOW).unwrap();

        let only_battery_id = fetched_doc(&conn, "EP2222222", "B", "b");
        labels::validate_document(&conn, only_battery_id, &[battery.clone(), solar.clone()], &[battery_id].into_iter().collect::<HashSet<_>>(), NOW).unwrap();

        let filters = LibraryFilters { include_tag_ids: vec![battery_id, solar_id], ..Default::default() };
        let rows = search(&conn, &filters).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].pub_key, "EP1111111");
    }

    #[test]
    fn exclude_tag_filter_removes_any_listed_tag() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let battery_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let battery = tags::get(&conn, battery_id).unwrap().unwrap();

        let tagged_id = fetched_doc(&conn, "EP1111111", "A", "a");
        labels::validate_document(&conn, tagged_id, std::slice::from_ref(&battery), &[battery_id].into_iter().collect::<HashSet<_>>(), NOW).unwrap();
        fetched_doc(&conn, "EP2222222", "B", "b");

        let filters = LibraryFilters { exclude_tag_ids: vec![battery_id], ..Default::default() };
        let rows = search(&conn, &filters).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].pub_key, "EP2222222");
    }

    #[test]
    fn fulltext_availability_filter_treats_non_english_only_as_available() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let with_text = fetched_doc(&conn, "EP1111111", "A", "a");
        fetched_doc(&conn, "EP2222222", "B", "b");
        crate::fulltext::store_fetched(&conn, with_text, Some("[0001] text"), None, "DE", "EP.1.A1", true, NOW)
            .unwrap();

        let available = search(
            &conn,
            &LibraryFilters { fulltext_availability: Some("available".to_string()), ..Default::default() },
        )
        .unwrap();
        assert_eq!(available.len(), 1);
        assert_eq!(available[0].pub_key, "EP1111111");

        let unavailable = search(
            &conn,
            &LibraryFilters { fulltext_availability: Some("unavailable".to_string()), ..Default::default() },
        )
        .unwrap();
        assert_eq!(unavailable.len(), 1);
        assert_eq!(unavailable[0].pub_key, "EP2222222");
    }

    #[test]
    fn drawings_availability_filter_requires_fetched_status() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let with_drawings = fetched_doc(&conn, "EP1111111", "A", "a");
        let pending = fetched_doc(&conn, "EP2222222", "B", "b");
        crate::drawings::store_fetched_status(&conn, with_drawings, 1, "EP.1.A1", NOW).unwrap();
        crate::drawings::store_not_available(&conn, pending, NOW).unwrap();

        let available = search(
            &conn,
            &LibraryFilters { drawings_availability: Some("available".to_string()), ..Default::default() },
        )
        .unwrap();
        assert_eq!(available.len(), 1);
        assert_eq!(available[0].pub_key, "EP1111111");
    }

    #[test]
    fn similar_to_ranks_by_cosine_similarity_excluding_self() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let target_id = fetched_doc(&conn, "EP1111111", "Target", "t");
        let close_id = fetched_doc(&conn, "EP2222222", "Close", "c");
        let far_id = fetched_doc(&conn, "EP3333333", "Far", "f");

        embeddings::store_document_embedding(&conn, target_id, MODEL, &[1.0, 0.0]).unwrap();
        embeddings::store_document_embedding(&conn, close_id, MODEL, &[0.9, 0.1]).unwrap();
        embeddings::store_document_embedding(&conn, far_id, MODEL, &[0.0, 1.0]).unwrap();

        let results = similar_to(&conn, target_id, MODEL, 10).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].pub_key, "EP2222222");
        assert_eq!(results[1].pub_key, "EP3333333");
        assert!(results[0].similarity > results[1].similarity);
    }
}
