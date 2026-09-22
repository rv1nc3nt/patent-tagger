//! Export data assembly (SPEC section 8): CSV/JSON rows, a per-tag
//! publication-number list, and the per-document `.txt` format. Pure
//! data-gathering here; writing files (and any drawing images once M8
//! exists) is `src-tauri`'s job.

use crate::storage::StorageError;
use rusqlite::{params, Connection};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ExportRow {
    pub pub_key: String,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub sources: Vec<String>,
}

/// Every fetched document (SPEC 8 Export: CSV/JSON cover the library, not
/// just validated documents), with its current tags (any label source -
/// both a human decision and a standing automatic one count as "tagged").
pub fn export_rows(conn: &Connection) -> Result<Vec<ExportRow>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT id, pub_key, title, abstract_source FROM documents
         WHERE fetch_status = 'fetched' ORDER BY id ASC",
    )?;
    let docs: Vec<(i64, String, Option<String>, Option<String>)> = stmt
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut tag_stmt = conn.prepare(
        "SELECT t.name FROM labels l JOIN tags t ON t.id = l.tag_id
         WHERE l.doc_id = ?1 AND l.state = 'pos' ORDER BY t.name ASC",
    )?;

    let mut rows = Vec::with_capacity(docs.len());
    for (doc_id, pub_key, title, abstract_source) in docs {
        let tags = tag_stmt
            .query_map(params![doc_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let sources = abstract_source
            .map(|s| vec![format!("abstract {s}")])
            .unwrap_or_default();
        rows.push(ExportRow { pub_key, title, tags, sources });
    }
    Ok(rows)
}

pub fn csv_field(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

pub fn to_csv(rows: &[ExportRow]) -> String {
    let mut out = String::from("publication_number,title,tags,sources\n");
    for row in rows {
        out.push_str(&csv_field(&row.pub_key));
        out.push(',');
        out.push_str(&csv_field(row.title.as_deref().unwrap_or("")));
        out.push(',');
        out.push_str(&csv_field(&row.tags.join("; ")));
        out.push(',');
        out.push_str(&csv_field(&row.sources.join("; ")));
        out.push('\n');
    }
    out
}

/// Publication numbers with a `pos` label (any source) for `tag_id` - SPEC
/// 8's "one .txt list of publication numbers per tag".
pub fn pub_keys_for_tag(conn: &Connection, tag_id: i64) -> Result<Vec<String>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT d.pub_key FROM labels l JOIN documents d ON d.id = l.doc_id
         WHERE l.tag_id = ?1 AND l.state = 'pos' ORDER BY d.pub_key ASC",
    )?;
    let rows = stmt
        .query_map(params![tag_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The per-document `.txt` export format (SPEC section 8's Export
/// subsection). Full text/drawings don't exist until M8, so those
/// sections explicitly say so rather than being silently omitted, per
/// "Missing parts are stated explicitly...never omitted silently".
pub fn document_txt(detail: &crate::documents::DocumentDetail, tags: &[String]) -> String {
    let mut out = String::new();
    let kind_codes = if detail.kind_codes.is_empty() {
        String::new()
    } else {
        format!(" ({})", detail.kind_codes.join(", "))
    };
    out.push_str(&format!("Publication: {}{kind_codes}\n", detail.pub_key));
    out.push_str(&format!("Title: {}\n", detail.title.as_deref().unwrap_or("Not available")));
    out.push_str(&format!(
        "Applicants: {}\n",
        if detail.applicants.is_empty() { "Not available".to_string() } else { detail.applicants.join("; ") }
    ));
    out.push_str(&format!(
        "Publication date: {}\n",
        detail.publication_date.as_deref().unwrap_or("Not available")
    ));
    out.push_str(&format!("CPC: {}\n", if detail.cpc.is_empty() { "Not available".to_string() } else { detail.cpc.join(", ") }));
    out.push_str(&format!("Tags: {}\n", if tags.is_empty() { "None".to_string() } else { tags.join("; ") }));
    out.push('\n');
    out.push_str("===== ABSTRACT =====\n");
    out.push_str(detail.abstract_text.as_deref().unwrap_or("Not available"));
    out.push('\n');
    out.push_str("===== DESCRIPTION =====\n");
    out.push_str("Full text not retrieved.\n");
    out.push_str("===== CLAIMS =====\n");
    out.push_str("Full text not retrieved.\n");
    out.push_str("===== DRAWINGS =====\n");
    out.push_str("Not retrieved.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{documents, labels, storage, tags};
    use std::collections::HashSet;

    const NOW: &str = "2026-01-01T00:00:00Z";

    #[test]
    fn to_csv_escapes_commas_and_quotes() {
        let rows = vec![ExportRow {
            pub_key: "EP1234567".to_string(),
            title: Some("A \"gadget\", improved".to_string()),
            tags: vec!["Battery".to_string()],
            sources: vec!["abstract EP.1234567.A1".to_string()],
        }];
        let csv = to_csv(&rows);
        assert!(csv.contains("\"A \"\"gadget\"\", improved\""));
    }

    #[test]
    fn export_rows_include_tags_and_sources() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();

        documents::insert_pending(&conn, "EP1234567", "EP1234567", NOW).unwrap();
        let doc = documents::find_by_pub_key(&conn, "EP1234567").unwrap().unwrap();
        documents::store_fetched(
            &conn,
            doc.id,
            &documents::FetchedData {
                title: Some("A gadget".to_string()),
                abstract_text: Some("An abstract".to_string()),
                abstract_source: Some("EP.1234567.A1".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        labels::validate_document(&conn, doc.id, std::slice::from_ref(&tag), &[tag_id].into_iter().collect::<HashSet<_>>(), NOW).unwrap();

        let rows = export_rows(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].pub_key, "EP1234567");
        assert_eq!(rows[0].tags, vec!["Battery"]);
        assert_eq!(rows[0].sources, vec!["abstract EP.1234567.A1"]);
    }

    #[test]
    fn pub_keys_for_tag_only_returns_positively_labelled_documents() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();

        documents::insert_pending(&conn, "EP1111111", "EP1111111", NOW).unwrap();
        let yes = documents::find_by_pub_key(&conn, "EP1111111").unwrap().unwrap();
        documents::insert_pending(&conn, "EP2222222", "EP2222222", NOW).unwrap();
        let no = documents::find_by_pub_key(&conn, "EP2222222").unwrap().unwrap();

        labels::validate_document(&conn, yes.id, std::slice::from_ref(&tag), &[tag_id].into_iter().collect::<HashSet<_>>(), NOW).unwrap();
        labels::validate_document(&conn, no.id, std::slice::from_ref(&tag), &HashSet::new(), NOW).unwrap();

        assert_eq!(pub_keys_for_tag(&conn, tag_id).unwrap(), vec!["EP1111111"]);
    }

    #[test]
    fn document_txt_states_missing_parts_explicitly() {
        let detail = documents::DocumentDetail {
            id: 1,
            pub_key: "EP1234567".to_string(),
            title: Some("A gadget".to_string()),
            abstract_text: Some("An abstract".to_string()),
            kind_codes: vec!["A1".to_string()],
            ..Default::default()
        };
        let text = document_txt(&detail, &["Battery".to_string()]);
        assert!(text.starts_with("Publication: EP1234567 (A1)\n"));
        assert!(text.contains("Applicants: Not available"));
        assert!(text.contains("===== ABSTRACT =====\nAn abstract"));
        assert!(text.contains("===== DESCRIPTION =====\nFull text not retrieved."));
        assert!(text.contains("===== DRAWINGS =====\nNot retrieved."));
    }
}
