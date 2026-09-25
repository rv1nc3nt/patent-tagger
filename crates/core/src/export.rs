//! Export data assembly (SPEC section 8): CSV/JSON rows, a per-tag
//! publication-number list, and the per-document `.txt` format. Pure
//! data-gathering here; writing files (folders, `.txt`, drawing images) is
//! `src-tauri`'s job.

use crate::storage::StorageError;
use rusqlite::{params, Connection};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ExportRow {
    pub pub_key: String,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub sources: Vec<String>,
    /// View-screen highlights, in reading order. Not part of the CSV.
    pub highlights: Vec<ExportHighlight>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ExportHighlight {
    pub section: String,
    pub location: String,
    pub quote: String,
    pub comment: Option<String>,
    pub color: String,
    /// The quote is no longer in the stored text (see `annotations`).
    pub detached: bool,
    pub created_at: String,
}

impl From<crate::annotations::Annotation> for ExportHighlight {
    fn from(a: crate::annotations::Annotation) -> Self {
        ExportHighlight {
            section: a.section,
            location: a.location,
            quote: a.quote,
            comment: a.comment,
            color: a.color,
            detached: a.detached,
            created_at: a.created_at,
        }
    }
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
        let highlights = crate::annotations::list(conn, doc_id)?
            .into_iter()
            .map(ExportHighlight::from)
            .collect();
        rows.push(ExportRow {
            pub_key,
            title,
            tags,
            sources,
            highlights,
        });
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

/// Full text and drawings data for one document's export (SPEC section
/// 8). `None` means the retrieval simply hasn't happened yet (`fulltext`/
/// `drawings_status` has no row, or `status = "pending"`) as opposed to
/// having tried and found nothing (`not_available`) - both are stated
/// explicitly in [`document_txt`], but with different wording.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DocumentExportFulltext {
    pub status: String,
    pub description: Option<String>,
    pub claims: Option<String>,
    pub lang: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DocumentExportDrawings {
    pub status: String,
    /// Relative paths as written under the export folder, e.g.
    /// `drawings/001.png`, in page order.
    pub page_paths: Vec<String>,
    pub source: Option<String>,
}

/// The per-document `.txt` export format (SPEC section 8's Export
/// subsection). `fulltext`/`drawings` are `None` when retrieval was never
/// attempted (no row in `fulltext`/`drawings_status` yet, e.g. the policy
/// is "never" or "on demand" and nobody asked); "Missing parts are stated
/// explicitly...never omitted silently" applies to that case too.
///
/// A `HIGHLIGHTS` section follows `DRAWINGS` when the document has any
/// View-screen highlights. It is left out when there are none, since it is
/// the user's work rather than a part of the publication.
pub fn document_txt(
    detail: &crate::documents::DocumentDetail,
    tags: &[String],
    fulltext: Option<&DocumentExportFulltext>,
    drawings: Option<&DocumentExportDrawings>,
    highlights: &[crate::annotations::Annotation],
) -> String {
    let mut out = String::new();
    let kind_codes = if detail.kind_codes.is_empty() {
        String::new()
    } else {
        format!(" ({})", detail.kind_codes.join(", "))
    };
    out.push_str(&format!("Publication: {}{kind_codes}\n", detail.pub_key));
    out.push_str(&format!(
        "Title: {}\n",
        detail.title.as_deref().unwrap_or("Not available")
    ));
    out.push_str(&format!(
        "Applicants: {}\n",
        if detail.applicants.is_empty() {
            "Not available".to_string()
        } else {
            detail.applicants.join("; ")
        }
    ));
    out.push_str(&format!(
        "Publication date: {}\n",
        detail
            .publication_date
            .as_deref()
            .unwrap_or("Not available")
    ));
    out.push_str(&format!(
        "CPC: {}\n",
        if detail.cpc.is_empty() {
            "Not available".to_string()
        } else {
            detail.cpc.join(", ")
        }
    ));
    out.push_str(&format!(
        "Tags: {}\n",
        if tags.is_empty() {
            "None".to_string()
        } else {
            tags.join("; ")
        }
    ));

    let mut sources = Vec::new();
    if let Some(s) = &detail.abstract_source {
        sources.push(format!("abstract {s}"));
    }
    if let Some(ft) = fulltext {
        if let Some(s) = &ft.source {
            sources.push(format!("full text {s}"));
        }
    }
    if let Some(dr) = drawings {
        if let Some(s) = &dr.source {
            sources.push(format!("drawings {s}"));
        }
    }
    out.push_str(&format!(
        "Sources: {}\n",
        if sources.is_empty() {
            "None".to_string()
        } else {
            sources.join("; ")
        }
    ));

    out.push('\n');
    out.push_str("===== ABSTRACT =====\n");
    out.push_str(detail.abstract_text.as_deref().unwrap_or("Not available"));
    out.push('\n');

    out.push_str("===== DESCRIPTION =====\n");
    out.push_str(&fulltext_part_text(
        fulltext,
        |ft| ft.description.as_deref(),
        "Description",
    ));
    out.push_str("===== CLAIMS =====\n");
    out.push_str(&fulltext_part_text(
        fulltext,
        |ft| ft.claims.as_deref(),
        "Claims",
    ));

    out.push_str("===== DRAWINGS =====\n");
    match drawings {
        None => out.push_str("Drawings not retrieved.\n"),
        Some(dr) => match dr.status.as_str() {
            "fetched" if !dr.page_paths.is_empty() => {
                for path in &dr.page_paths {
                    out.push_str(path);
                    out.push('\n');
                }
            }
            "not_available" => out.push_str("This publication has no drawings.\n"),
            "error" => out.push_str("Drawings retrieval failed.\n"),
            _ => out.push_str("Drawings not retrieved.\n"),
        },
    }

    if !highlights.is_empty() {
        out.push_str("===== HIGHLIGHTS =====\n");
        for h in highlights {
            out.push_str(&highlight_txt(h));
        }
    }
    out
}

/// `Claim 1 (yellow): "quote"`, then the comment indented below it. Line
/// breaks inside the quote become spaces.
fn highlight_txt(h: &crate::annotations::Annotation) -> String {
    let quote = h.quote.split_whitespace().collect::<Vec<_>>().join(" ");
    let state = if h.detached {
        ", passage no longer in the text"
    } else {
        ""
    };
    let mut out = format!("{} ({}{state}): \"{quote}\"\n", h.location, h.color);
    if let Some(comment) = &h.comment {
        for (i, line) in comment.lines().enumerate() {
            let prefix = if i == 0 { "  Comment: " } else { "           " };
            out.push_str(prefix);
            out.push_str(line.trim_end());
            out.push('\n');
        }
    }
    out
}

/// `part` picks `description` or `claims` off a fetched row; `label`
/// names the part for the "not available"/"pending" wording.
fn fulltext_part_text(
    fulltext: Option<&DocumentExportFulltext>,
    part: impl Fn(&DocumentExportFulltext) -> Option<&str>,
    label: &str,
) -> String {
    let mut out = String::new();
    match fulltext {
        None => out.push_str(&format!("{label} not retrieved.\n")),
        Some(ft) => match ft.status.as_str() {
            "not_available" => out.push_str("Full text not available in OPS.\n"),
            "error" => out.push_str("Full text retrieval failed.\n"),
            "fetched" | "non_english_only" => match part(ft) {
                Some(text) => {
                    if ft.status == "non_english_only" {
                        let lang = ft.lang.as_deref().unwrap_or("unknown");
                        out.push_str(&format!("[Non-English text, language: {lang}]\n"));
                    }
                    out.push_str(text);
                    out.push('\n');
                }
                None => out.push_str(&format!("{label} not available for this publication.\n")),
            },
            _ => out.push_str(&format!("{label} not retrieved.\n")),
        },
    }
    out
}

/// Document-export folder for selected documents with no active positive
/// tag. The leading underscore keeps it apart from tag folders; a tag that
/// happens to sanitise to the same name is disambiguated instead.
pub const UNTAGGED_FOLDER: &str = "_untagged";

/// A tag name made safe as a folder name on both Windows and Linux:
/// characters Windows forbids (`<>:"/\|?*` and control characters) become
/// `_`, trailing dots and spaces (which Windows strips) are removed, and
/// reserved device names (`CON`, `COM1`, … also with an extension) get a
/// trailing `_`. Never empty.
pub fn sanitize_folder_name(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = replaced.trim().trim_end_matches(['.', ' ']);
    if trimmed.is_empty() {
        return "_".to_string();
    }
    let stem = trimmed.split('.').next().unwrap_or(trimmed).trim_end();
    let reserved = matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON" | "PRN" | "AUX" | "NUL"
    ) || {
        let upper = stem.to_ascii_uppercase();
        (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.len() == 4
            && upper.as_bytes()[3].is_ascii_digit()
            && upper.as_bytes()[3] != b'0'
    };
    if reserved {
        format!("{trimmed}_")
    } else {
        trimmed.to_string()
    }
}

/// One folder name per tag for the by-tag document export, keyed by tag
/// id. Names are sanitised with [`sanitize_folder_name`] and are unique
/// case-insensitively (Windows file names are), also against
/// [`UNTAGGED_FOLDER`]. On a clash the tag with the lower id keeps the plain
/// name and later ones get ` (2)`, ` (3)`, …, so names are stable across
/// exports as long as tags are not renamed.
pub fn tag_folder_names<'a>(
    tags: impl IntoIterator<Item = (i64, &'a str)>,
) -> std::collections::HashMap<i64, String> {
    let mut tags: Vec<(i64, &str)> = tags.into_iter().collect();
    tags.sort_by_key(|(id, _)| *id);
    let mut taken: std::collections::HashSet<String> =
        std::iter::once(UNTAGGED_FOLDER.to_lowercase()).collect();
    let mut names = std::collections::HashMap::new();
    for (id, name) in tags {
        let base = sanitize_folder_name(name);
        let mut candidate = base.clone();
        let mut n = 2;
        while !taken.insert(candidate.to_lowercase()) {
            candidate = format!("{base} ({n})");
            n += 1;
        }
        names.insert(id, candidate);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{documents, labels, storage, tags};
    use std::collections::HashSet;

    #[test]
    fn sanitize_folder_name_replaces_characters_windows_forbids() {
        assert_eq!(sanitize_folder_name("A/B: C?"), "A_B_ C_");
        assert_eq!(sanitize_folder_name("x\\y*z|\"<>"), "x_y_z____");
        assert_eq!(sanitize_folder_name("tab\there"), "tab_here");
        assert_eq!(sanitize_folder_name("Batteries"), "Batteries");
        assert_eq!(
            sanitize_folder_name("Électrodes – solides"),
            "Électrodes – solides"
        );
    }

    #[test]
    fn sanitize_folder_name_trims_trailing_dots_and_spaces_and_is_never_empty() {
        assert_eq!(sanitize_folder_name("  name. . "), "name");
        assert_eq!(sanitize_folder_name(".."), "_");
        assert_eq!(sanitize_folder_name("   "), "_");
    }

    #[test]
    fn sanitize_folder_name_suffixes_reserved_device_names() {
        assert_eq!(sanitize_folder_name("con"), "con_");
        assert_eq!(sanitize_folder_name("COM1"), "COM1_");
        assert_eq!(sanitize_folder_name("lpt9.txt"), "lpt9.txt_");
        assert_eq!(sanitize_folder_name("COM0"), "COM0");
        assert_eq!(sanitize_folder_name("CONSOLE"), "CONSOLE");
    }

    #[test]
    fn tag_folder_names_disambiguate_case_insensitive_clashes_by_id() {
        let names = tag_folder_names([(3, "a/b"), (1, "A_B"), (2, "_Untagged"), (4, "other")]);
        assert_eq!(names[&1], "A_B");
        assert_eq!(names[&2], "_Untagged (2)");
        assert_eq!(names[&3], "a_b (2)");
        assert_eq!(names[&4], "other");
    }

    const NOW: &str = "2026-01-01T00:00:00Z";

    #[test]
    fn to_csv_escapes_commas_and_quotes() {
        let rows = vec![ExportRow {
            pub_key: "EP1234567".to_string(),
            title: Some("A \"gadget\", improved".to_string()),
            tags: vec!["Battery".to_string()],
            sources: vec!["abstract EP.1234567.A1".to_string()],
            highlights: Vec::new(),
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
        let doc = documents::find_by_pub_key(&conn, "EP1234567")
            .unwrap()
            .unwrap();
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
        labels::validate_document(
            &conn,
            doc.id,
            std::slice::from_ref(&tag),
            &[tag_id].into_iter().collect::<HashSet<_>>(),
            NOW,
        )
        .unwrap();

        let rows = export_rows(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].pub_key, "EP1234567");
        assert_eq!(rows[0].tags, vec!["Battery"]);
        assert_eq!(rows[0].sources, vec!["abstract EP.1234567.A1"]);
        assert!(rows[0].highlights.is_empty());

        crate::annotations::create(&conn, doc.id, "abstract", 3, 11, Some("why"), "green", NOW)
            .unwrap();
        let rows = export_rows(&conn).unwrap();
        assert_eq!(rows[0].highlights.len(), 1);
        assert_eq!(rows[0].highlights[0].quote, "abstract");
        assert_eq!(rows[0].highlights[0].location, "Abstract");
        assert_eq!(rows[0].highlights[0].comment.as_deref(), Some("why"));
        assert_eq!(
            to_csv(&rows).lines().count(),
            2,
            "CSV has no highlights column"
        );
    }

    #[test]
    fn pub_keys_for_tag_only_returns_positively_labelled_documents() {
        let conn = storage::open_in_memory().expect("in-memory db");
        let tag_id = tags::create(&conn, "Battery", "About batteries", None, None, NOW).unwrap();
        let tag = tags::get(&conn, tag_id).unwrap().unwrap();

        documents::insert_pending(&conn, "EP1111111", "EP1111111", NOW).unwrap();
        let yes = documents::find_by_pub_key(&conn, "EP1111111")
            .unwrap()
            .unwrap();
        documents::insert_pending(&conn, "EP2222222", "EP2222222", NOW).unwrap();
        let no = documents::find_by_pub_key(&conn, "EP2222222")
            .unwrap()
            .unwrap();

        labels::validate_document(
            &conn,
            yes.id,
            std::slice::from_ref(&tag),
            &[tag_id].into_iter().collect::<HashSet<_>>(),
            NOW,
        )
        .unwrap();
        labels::validate_document(
            &conn,
            no.id,
            std::slice::from_ref(&tag),
            &HashSet::new(),
            NOW,
        )
        .unwrap();

        assert_eq!(pub_keys_for_tag(&conn, tag_id).unwrap(), vec!["EP1111111"]);
    }

    fn base_detail() -> documents::DocumentDetail {
        documents::DocumentDetail {
            id: 1,
            pub_key: "EP1234567".to_string(),
            title: Some("A gadget".to_string()),
            abstract_text: Some("An abstract".to_string()),
            abstract_source: Some("EP.1234567.A1".to_string()),
            kind_codes: vec!["A1".to_string()],
            ..Default::default()
        }
    }

    #[test]
    fn document_txt_states_missing_parts_explicitly_when_never_retrieved() {
        let text = document_txt(&base_detail(), &["Battery".to_string()], None, None, &[]);
        assert!(text.starts_with("Publication: EP1234567 (A1)\n"));
        assert!(text.contains("Applicants: Not available"));
        assert!(text.contains("Sources: abstract EP.1234567.A1\n"));
        assert!(text.contains("===== ABSTRACT =====\nAn abstract"));
        assert!(text.contains("===== DESCRIPTION =====\nDescription not retrieved."));
        assert!(text.contains("===== CLAIMS =====\nClaims not retrieved."));
        assert!(text.contains("===== DRAWINGS =====\nDrawings not retrieved."));
    }

    #[test]
    fn document_txt_reports_full_text_and_drawings_when_fetched() {
        let fulltext = DocumentExportFulltext {
            status: "fetched".to_string(),
            description: Some("[0001] A widget.".to_string()),
            claims: Some("1. A widget.".to_string()),
            lang: Some("EN".to_string()),
            source: Some("EP.1234567.B1".to_string()),
        };
        let drawings = DocumentExportDrawings {
            status: "fetched".to_string(),
            page_paths: vec![
                "drawings/001.png".to_string(),
                "drawings/002.png".to_string(),
            ],
            source: Some("EP.1234567.A1".to_string()),
        };
        let text = document_txt(&base_detail(), &[], Some(&fulltext), Some(&drawings), &[]);
        assert!(text.contains(
            "Sources: abstract EP.1234567.A1; full text EP.1234567.B1; drawings EP.1234567.A1\n"
        ));
        assert!(text.contains("===== DESCRIPTION =====\n[0001] A widget.\n"));
        assert!(text.contains("===== CLAIMS =====\n1. A widget.\n"));
        assert!(text.contains("===== DRAWINGS =====\ndrawings/001.png\ndrawings/002.png\n"));
    }

    #[test]
    fn document_txt_marks_non_english_text_with_its_language() {
        let fulltext = DocumentExportFulltext {
            status: "non_english_only".to_string(),
            description: Some("[0001] Nur Deutsch.".to_string()),
            claims: None,
            lang: Some("DE".to_string()),
            source: Some("EP.1234567.A1".to_string()),
        };
        let text = document_txt(&base_detail(), &[], Some(&fulltext), None, &[]);
        assert!(text.contains(
            "===== DESCRIPTION =====\n[Non-English text, language: DE]\n[0001] Nur Deutsch.\n"
        ));
        assert!(text.contains("===== CLAIMS =====\nClaims not available for this publication.\n"));
    }

    /// SPEC 10's M8 acceptance criterion: "exported .txt matches a
    /// reference file" - a fully-populated document (title, abstract,
    /// description, claims and drawings all present) compared byte-for-byte
    /// against `tests/fixtures/export/document_reference.txt`.
    #[test]
    fn document_txt_matches_the_reference_file_for_a_fully_populated_document() {
        let detail = documents::DocumentDetail {
            id: 1,
            pub_key: "EP1234567".to_string(),
            title: Some("Apparatus for manufacturing green bricks".to_string()),
            abstract_text: Some(
                "An apparatus for manufacturing green bricks from clay.".to_string(),
            ),
            abstract_source: Some("EP.1234567.A1".to_string()),
            applicants: vec!["ACME Corp".to_string(), "Foo Industries".to_string()],
            publication_date: Some("2020-05-14".to_string()),
            cpc: vec!["B28B1/29".to_string(), "B28B7/00".to_string()],
            kind_codes: vec!["A1".to_string(), "B1".to_string()],
            ..Default::default()
        };
        let fulltext = DocumentExportFulltext {
            status: "fetched".to_string(),
            description: Some(
                "[0001] The invention relates to an apparatus for manufacturing green bricks.\n[0002] Further details follow.".to_string(),
            ),
            claims: Some("1. Apparatus for manufacturing green bricks.\n2. Apparatus as claimed in claim 1.".to_string()),
            lang: Some("EN".to_string()),
            source: Some("EP.1234567.B1".to_string()),
        };
        let drawings = DocumentExportDrawings {
            status: "fetched".to_string(),
            page_paths: vec![
                "drawings/001.png".to_string(),
                "drawings/002.png".to_string(),
            ],
            source: Some("EP.1234567.A1".to_string()),
        };

        let text = document_txt(
            &detail,
            &["Battery".to_string(), "Ceramics".to_string()],
            Some(&fulltext),
            Some(&drawings),
            &[],
        );

        let reference = std::fs::read_to_string(format!(
            "{}/../../tests/fixtures/export/document_reference.txt",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("reading the reference export file");
        assert_eq!(text, reference);
    }

    #[test]
    fn document_txt_states_not_available_distinctly_from_not_retrieved() {
        let fulltext = DocumentExportFulltext {
            status: "not_available".to_string(),
            ..Default::default()
        };
        let drawings = DocumentExportDrawings {
            status: "not_available".to_string(),
            ..Default::default()
        };
        let text = document_txt(&base_detail(), &[], Some(&fulltext), Some(&drawings), &[]);
        assert!(text.contains("===== DESCRIPTION =====\nFull text not available in OPS.\n"));
        assert!(text.contains("===== DRAWINGS =====\nThis publication has no drawings.\n"));
    }

    #[test]
    fn document_txt_lists_highlights_after_drawings() {
        let highlight = |location: &str, quote: &str, comment: Option<&str>, detached: bool| {
            crate::annotations::Annotation {
                id: 1,
                doc_id: 1,
                section: "claims".to_string(),
                start: 0,
                end: 1,
                quote: quote.to_string(),
                comment: comment.map(str::to_string),
                color: "yellow".to_string(),
                created_at: NOW.to_string(),
                updated_at: NOW.to_string(),
                detached,
                location: location.to_string(),
            }
        };
        let highlights = [
            highlight(
                "Claim 1",
                "a mould\ncontainer",
                Some("Key feature.\nSee also D1."),
                false,
            ),
            highlight("Description", "old text", None, true),
        ];
        let text = document_txt(&base_detail(), &[], None, None, &highlights);
        assert!(text.ends_with(
            "===== DRAWINGS =====\nDrawings not retrieved.\n\
             ===== HIGHLIGHTS =====\n\
             Claim 1 (yellow): \"a mould container\"\n  Comment: Key feature.\n           See also D1.\n\
             Description (yellow, passage no longer in the text): \"old text\"\n"
        ));

        let without = document_txt(&base_detail(), &[], None, None, &[]);
        assert!(!without.contains("HIGHLIGHTS"));
    }
}
