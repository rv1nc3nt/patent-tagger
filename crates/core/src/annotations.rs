//! Highlighted passages with optional comments, made on the View screen.
//!
//! A highlight covers `start..end` of one section's stored text, in UTF-16
//! code units (see `outline`). Its `quote` is kept so that the passage can
//! be found again when the stored text changes, for example after the full
//! text is retrieved again: [`list`] resolves each highlight against the
//! current text, and marks it `detached` when the quote is gone.

use crate::outline::{self, BlockKind};
use crate::storage::StorageError;
use rusqlite::{params, Connection, OptionalExtension};

pub const SECTIONS: &[&str] = &["title", "abstract", "description", "claims"];
pub const COLORS: &[&str] = &["yellow", "green", "blue", "pink"];

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Annotation {
    pub id: i64,
    pub doc_id: i64,
    pub section: String,
    /// Current position in the section's text; the stored one when
    /// `detached`.
    pub start: usize,
    pub end: usize,
    pub quote: String,
    pub comment: Option<String>,
    pub color: String,
    pub created_at: String,
    pub updated_at: String,
    /// The quote no longer appears in the section's text, so the
    /// highlight cannot be drawn. It is kept, with its quote and comment.
    pub detached: bool,
    /// Where the highlight is, for lists and exports: `Abstract`,
    /// `Description [0012]`, `Claim 3`.
    pub location: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AnnotationError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("document {0} not found")]
    DocumentNotFound(i64),
    #[error("highlight {0} not found")]
    NotFound(i64),
    #[error("unknown section \"{0}\"")]
    UnknownSection(String),
    #[error("unknown highlight colour \"{0}\"")]
    UnknownColor(String),
    #[error("this document has no {0} text")]
    NoText(String),
    #[error("the selection is outside the text or empty")]
    InvalidRange,
}

impl From<rusqlite::Error> for AnnotationError {
    fn from(e: rusqlite::Error) -> Self {
        AnnotationError::Storage(StorageError::from(e))
    }
}

/// The stored text of each highlightable section of a document.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SectionTexts {
    pub title: Option<String>,
    pub abstract_text: Option<String>,
    pub description: Option<String>,
    pub claims: Option<String>,
}

impl SectionTexts {
    pub fn load(conn: &Connection, doc_id: i64) -> Result<Option<Self>, StorageError> {
        let Some(detail) = crate::documents::get_full(conn, doc_id)? else {
            return Ok(None);
        };
        let fulltext = crate::fulltext::get(conn, doc_id)?;
        let (description, claims) = fulltext
            .map(|f| (f.description, f.claims))
            .unwrap_or_default();
        Ok(Some(SectionTexts {
            title: detail.title,
            abstract_text: detail.abstract_text,
            description,
            claims,
        }))
    }

    pub fn get(&self, section: &str) -> Option<&str> {
        match section {
            "title" => self.title.as_deref(),
            "abstract" => self.abstract_text.as_deref(),
            "description" => self.description.as_deref(),
            "claims" => self.claims.as_deref(),
            _ => None,
        }
    }
}

/// Highlights `start..end` of `section`. The comment is optional; a blank
/// one is stored as none.
#[allow(clippy::too_many_arguments)]
pub fn create(
    conn: &Connection,
    doc_id: i64,
    section: &str,
    start: usize,
    end: usize,
    comment: Option<&str>,
    color: &str,
    now: &str,
) -> Result<Annotation, AnnotationError> {
    if !SECTIONS.contains(&section) {
        return Err(AnnotationError::UnknownSection(section.to_string()));
    }
    check_color(color)?;
    let texts =
        SectionTexts::load(conn, doc_id)?.ok_or(AnnotationError::DocumentNotFound(doc_id))?;
    let text = texts
        .get(section)
        .ok_or_else(|| AnnotationError::NoText(section.to_string()))?;
    let quote = utf16_slice(text, start, end).ok_or(AnnotationError::InvalidRange)?;
    if quote.trim().is_empty() {
        return Err(AnnotationError::InvalidRange);
    }
    conn.execute(
        "INSERT INTO annotations (doc_id, section, start, \"end\", quote, comment, color, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![
            doc_id,
            section,
            start as i64,
            end as i64,
            quote,
            clean_comment(comment),
            color,
            now
        ],
    )?;
    let id = conn.last_insert_rowid();
    get(conn, id)?.ok_or(AnnotationError::NotFound(id))
}

/// Changes a highlight's comment and colour. Its passage stays the same.
pub fn update(
    conn: &Connection,
    id: i64,
    comment: Option<&str>,
    color: &str,
    now: &str,
) -> Result<Annotation, AnnotationError> {
    check_color(color)?;
    let changed = conn.execute(
        "UPDATE annotations SET comment = ?2, color = ?3, updated_at = ?4 WHERE id = ?1",
        params![id, clean_comment(comment), color, now],
    )?;
    if changed == 0 {
        return Err(AnnotationError::NotFound(id));
    }
    get(conn, id)?.ok_or(AnnotationError::NotFound(id))
}

pub fn delete(conn: &Connection, id: i64) -> Result<(), AnnotationError> {
    let changed = conn.execute("DELETE FROM annotations WHERE id = ?1", params![id])?;
    if changed == 0 {
        return Err(AnnotationError::NotFound(id));
    }
    Ok(())
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<Annotation>, StorageError> {
    let Some(stored) = conn
        .query_row(
            &format!("{SELECT} WHERE id = ?1"),
            params![id],
            row_to_stored,
        )
        .optional()?
    else {
        return Ok(None);
    };
    let texts = SectionTexts::load(conn, stored.doc_id)?.unwrap_or_default();
    Ok(Some(resolve(stored, &texts)))
}

/// A document's highlights in reading order: title, abstract, description,
/// claims, then by position. Detached highlights keep their stored
/// position for ordering.
pub fn list(conn: &Connection, doc_id: i64) -> Result<Vec<Annotation>, StorageError> {
    let mut stmt = conn.prepare(&format!("{SELECT} WHERE doc_id = ?1"))?;
    let stored = stmt
        .query_map(params![doc_id], row_to_stored)?
        .collect::<Result<Vec<_>, _>>()?;
    let texts = SectionTexts::load(conn, doc_id)?.unwrap_or_default();
    let mut out: Vec<Annotation> = stored.into_iter().map(|s| resolve(s, &texts)).collect();
    out.sort_by_key(|a| (SECTIONS.iter().position(|s| *s == a.section), a.start, a.id));
    Ok(out)
}

const SELECT: &str =
    "SELECT id, doc_id, section, start, \"end\", quote, comment, color, created_at, updated_at
                      FROM annotations";

struct Stored {
    id: i64,
    doc_id: i64,
    section: String,
    start: usize,
    end: usize,
    quote: String,
    comment: Option<String>,
    color: String,
    created_at: String,
    updated_at: String,
}

fn row_to_stored(row: &rusqlite::Row) -> rusqlite::Result<Stored> {
    Ok(Stored {
        id: row.get(0)?,
        doc_id: row.get(1)?,
        section: row.get(2)?,
        start: row.get::<_, i64>(3)?.max(0) as usize,
        end: row.get::<_, i64>(4)?.max(0) as usize,
        quote: row.get(5)?,
        comment: row.get(6)?,
        color: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn resolve(stored: Stored, texts: &SectionTexts) -> Annotation {
    let text = texts.get(&stored.section);
    let found = text.and_then(|t| locate(t, &stored.quote, stored.start, stored.end));
    let (start, end) = found.unwrap_or((stored.start, stored.end));
    let location = location(&stored.section, text.filter(|_| found.is_some()), start);
    Annotation {
        id: stored.id,
        doc_id: stored.doc_id,
        section: stored.section,
        start,
        end,
        quote: stored.quote,
        comment: stored.comment,
        color: stored.color,
        created_at: stored.created_at,
        updated_at: stored.updated_at,
        detached: found.is_none(),
        location,
    }
}

/// Where `quote` is in `text`: at the stored position if it is still
/// there, else the occurrence closest to it.
fn locate(text: &str, quote: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    if utf16_slice(text, start, end).as_deref() == Some(quote) {
        return Some((start, end));
    }
    let len = outline::utf16_len(quote);
    text.match_indices(quote)
        .map(|(byte, _)| outline::utf16_len(&text[..byte]))
        .min_by_key(|&pos| pos.abs_diff(start))
        .map(|pos| (pos, pos + len))
}

/// `Title`, `Abstract`, `Description [0012]` or `Claim 3`, from the block
/// holding `start`. Without the text (detached), only the section.
pub fn location(section: &str, text: Option<&str>, start: usize) -> String {
    let blocks = match (section, text) {
        ("description", Some(t)) => outline::description_blocks(t),
        ("claims", Some(t)) => outline::claim_blocks(t),
        _ => Vec::new(),
    };
    let block = blocks.iter().find(|b| start >= b.start && start < b.end);
    match (section, block) {
        ("title", _) => "Title".to_string(),
        ("abstract", _) => "Abstract".to_string(),
        ("description", Some(b)) if b.kind == BlockKind::Heading => {
            "Description, heading".to_string()
        }
        ("description", Some(b)) => match &b.label {
            Some(n) => format!("Description [{n}]"),
            None => "Description".to_string(),
        },
        ("claims", Some(b)) => match &b.label {
            Some(n) => format!("Claim {n}"),
            None => "Claims".to_string(),
        },
        ("claims", None) => "Claims".to_string(),
        _ => "Description".to_string(),
    }
}

/// `text[start..end]` in UTF-16 units; `None` for an empty or
/// out-of-range span, or one that splits a surrogate pair.
fn utf16_slice(text: &str, start: usize, end: usize) -> Option<String> {
    if start >= end {
        return None;
    }
    let units: Vec<u16> = text.encode_utf16().collect();
    let span = units.get(start..end)?;
    String::from_utf16(span).ok()
}

fn check_color(color: &str) -> Result<(), AnnotationError> {
    if COLORS.contains(&color) {
        Ok(())
    } else {
        Err(AnnotationError::UnknownColor(color.to_string()))
    }
}

fn clean_comment(comment: Option<&str>) -> Option<&str> {
    comment.map(str::trim).filter(|c| !c.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::open_in_memory;

    const NOW: &str = "2026-09-25T10:00:00Z";

    fn setup() -> (Connection, i64) {
        let conn = open_in_memory().unwrap();
        conn.execute(
            "INSERT INTO documents (pub_key, input_raw, title, \"abstract\", fetch_status, review_state, imported_at)
             VALUES ('EP1', 'EP1', 'A brick press', 'An apparatus for bricks.', 'fetched', 'queued', ?1)",
            params![NOW],
        )
        .unwrap();
        let doc_id = conn.last_insert_rowid();
        crate::fulltext::store_fetched(
            &conn,
            doc_id,
            Some("[0001] The invention relates to bricks.\n[0002] A mould is filled."),
            Some("1. Apparatus for bricks.\n2. Apparatus as claimed in claim 1."),
            "EN",
            "EP.1.A1",
            false,
            NOW,
        )
        .unwrap();
        (conn, doc_id)
    }

    #[test]
    fn create_stores_the_quote_and_location() {
        let (conn, doc_id) = setup();
        // "mould" in paragraph [0002].
        let start = "[0001] The invention relates to bricks.\n[0002] A ".len();
        let a = create(
            &conn,
            doc_id,
            "description",
            start,
            start + 5,
            Some("  key part  "),
            "yellow",
            NOW,
        )
        .unwrap();
        assert_eq!(a.quote, "mould");
        assert_eq!(a.comment.as_deref(), Some("key part"));
        assert_eq!(a.location, "Description [0002]");
        assert!(!a.detached);

        let claim = create(&conn, doc_id, "claims", 28, 37, None, "blue", NOW).unwrap();
        assert_eq!(claim.quote, "Apparatus");
        assert_eq!(claim.location, "Claim 2");
        assert_eq!(claim.comment, None);
    }

    #[test]
    fn create_rejects_bad_input() {
        let (conn, doc_id) = setup();
        let err = |r: Result<Annotation, AnnotationError>| r.unwrap_err().to_string();
        assert!(
            err(create(&conn, doc_id, "summary", 0, 1, None, "yellow", NOW)).contains("section")
        );
        assert!(err(create(&conn, doc_id, "title", 0, 1, None, "red", NOW)).contains("colour"));
        assert!(
            err(create(&conn, doc_id, "title", 3, 3, None, "yellow", NOW)).contains("selection")
        );
        assert!(
            err(create(&conn, doc_id, "title", 0, 999, None, "yellow", NOW)).contains("selection")
        );
        assert!(
            err(create(&conn, doc_id, "title", 1, 2, None, "yellow", NOW)).contains("selection")
        );
        assert!(err(create(&conn, 999, "title", 0, 1, None, "yellow", NOW)).contains("not found"));
    }

    #[test]
    fn update_and_delete() {
        let (conn, doc_id) = setup();
        let a = create(&conn, doc_id, "abstract", 3, 12, None, "yellow", NOW).unwrap();
        let later = "2026-09-25T11:00:00Z";
        let b = update(&conn, a.id, Some("note"), "green", later).unwrap();
        assert_eq!(b.comment.as_deref(), Some("note"));
        assert_eq!(b.color, "green");
        assert_eq!(b.updated_at, later);
        assert_eq!(b.quote, a.quote);
        assert_eq!(
            update(&conn, a.id, Some(" "), "green", later)
                .unwrap()
                .comment,
            None
        );

        delete(&conn, a.id).unwrap();
        assert!(list(&conn, doc_id).unwrap().is_empty());
        assert!(matches!(
            delete(&conn, a.id),
            Err(AnnotationError::NotFound(_))
        ));
    }

    #[test]
    fn list_is_in_reading_order() {
        let (conn, doc_id) = setup();
        create(&conn, doc_id, "claims", 3, 12, None, "yellow", NOW).unwrap();
        create(&conn, doc_id, "description", 11, 19, None, "yellow", NOW).unwrap();
        create(&conn, doc_id, "abstract", 3, 12, None, "yellow", NOW).unwrap();
        create(&conn, doc_id, "description", 0, 6, None, "yellow", NOW).unwrap();
        let sections: Vec<_> = list(&conn, doc_id)
            .unwrap()
            .into_iter()
            .map(|a| (a.section, a.start))
            .collect();
        assert_eq!(
            sections,
            [
                ("abstract".to_string(), 3),
                ("description".to_string(), 0),
                ("description".to_string(), 11),
                ("claims".to_string(), 3),
            ]
        );
    }

    #[test]
    fn highlight_follows_its_quote_when_the_text_changes() {
        let (conn, doc_id) = setup();
        let start = "[0001] The invention relates to bricks.\n[0002] A ".len();
        let a = create(
            &conn,
            doc_id,
            "description",
            start,
            start + 5,
            None,
            "pink",
            NOW,
        )
        .unwrap();

        // Retrieved again, with a paragraph inserted before.
        crate::fulltext::store_fetched(
            &conn,
            doc_id,
            Some(
                "[0001] Field.\n[0002] The invention relates to bricks.\n[0003] A mould is filled.",
            ),
            None,
            "EN",
            "EP.1.B1",
            false,
            NOW,
        )
        .unwrap();
        let moved = get(&conn, a.id).unwrap().unwrap();
        assert!(!moved.detached);
        assert_eq!(moved.location, "Description [0003]");
        assert_eq!(
            moved.start,
            "[0001] Field.\n[0002] The invention relates to bricks.\n[0003] A ".len()
        );

        // The passage is gone.
        crate::fulltext::store_fetched(
            &conn,
            doc_id,
            Some("[0001] Other text."),
            None,
            "EN",
            "EP.1.B1",
            false,
            NOW,
        )
        .unwrap();
        let gone = get(&conn, a.id).unwrap().unwrap();
        assert!(gone.detached);
        assert_eq!(gone.quote, "mould");
        assert_eq!(gone.location, "Description");
    }

    #[test]
    fn locate_prefers_the_nearest_occurrence() {
        let text = "brick one, brick two, brick three";
        assert_eq!(locate(text, "brick", 13, 18), Some((11, 16)));
        assert_eq!(locate(text, "brick", 30, 35), Some((22, 27)));
        assert_eq!(locate(text, "clay", 0, 4), None);
    }
}
