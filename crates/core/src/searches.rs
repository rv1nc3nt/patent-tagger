//! Saved OPS searches by applicant (SPEC 5.6): the CQL query built from
//! the search fields, the batch position, and the choice of one document
//! per DOCDB family. Sending the query is `crates/ops`' job; importing the
//! chosen documents is the pipeline's.

use crate::storage::StorageError;
use rusqlite::{params, Connection, OptionalExtension};

/// Results read per batch: one OPS search page.
pub const PAGE_SIZE: i64 = 100;
/// OPS returns at most the first 2,000 results of a query.
pub const MAX_RESULTS: i64 = 2000;

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("search name must not be empty")]
    EmptyName,
    #[error("a search named \"{0}\" already exists")]
    DuplicateName(String),
    #[error("no search named \"{0}\"")]
    NotFound(String),
    #[error("applicant must not be empty")]
    EmptyApplicant,
    #[error("country must be a two-letter code, e.g. EP")]
    InvalidCountry,
    #[error("years must have four digits, and the start year must not be after the end year")]
    InvalidYears,
}

impl From<rusqlite::Error> for SearchError {
    fn from(e: rusqlite::Error) -> Self {
        SearchError::Storage(StorageError::from(e))
    }
}

/// The fields a search is created from (SPEC 5.6).
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct SearchFields {
    pub name: String,
    /// One or more applicant names separated by `;`.
    pub applicant: String,
    pub country: Option<String>,
    pub year_from: Option<i64>,
    pub year_to: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SavedSearch {
    pub id: i64,
    pub name: String,
    pub applicant: String,
    pub country: Option<String>,
    pub year_from: Option<i64>,
    pub year_to: Option<i64>,
    pub query: String,
    /// `None` until the first batch has run.
    pub total_results: Option<i64>,
    /// 1-based position of the next batch's first result.
    pub next_start: i64,
    pub imported: i64,
    pub created_at: String,
    pub last_run_at: Option<String>,
    /// Every reachable result has been read (see [`next_range`]).
    pub exhausted: bool,
    /// Results read so far in this pass.
    pub results_read: i64,
    /// More results match than OPS lets a client read ([`MAX_RESULTS`]).
    pub capped: bool,
}

/// The applicant names in `applicant`, split on `;`, trimmed, with the
/// characters that would break a CQL quoted term (`"` and `\`) removed.
fn applicant_names(applicant: &str) -> Vec<String> {
    applicant
        .split(';')
        .map(|name| {
            name.chars()
                .filter(|c| *c != '"' && *c != '\\')
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|name| !name.is_empty())
        .collect()
}

fn normalised_country(country: Option<&str>) -> Result<Option<String>, SearchError> {
    let Some(country) = country.map(str::trim).filter(|c| !c.is_empty()) else {
        return Ok(None);
    };
    if country.len() != 2 || !country.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(SearchError::InvalidCountry);
    }
    Ok(Some(country.to_ascii_uppercase()))
}

fn check_years(year_from: Option<i64>, year_to: Option<i64>) -> Result<(), SearchError> {
    let valid = |y: Option<i64>| y.is_none_or(|y| (1000..=9999).contains(&y));
    if !valid(year_from) || !valid(year_to) {
        return Err(SearchError::InvalidYears);
    }
    if let (Some(from), Some(to)) = (year_from, year_to) {
        if from > to {
            return Err(SearchError::InvalidYears);
        }
    }
    Ok(())
}

/// The CQL query for `fields` (SPEC 5.6), e.g. `(pa="Siemens" or
/// pa="Siemens Healthineers") and pn=EP and pd within "2020 2024"`.
pub fn build_query(fields: &SearchFields) -> Result<String, SearchError> {
    let names = applicant_names(&fields.applicant);
    if names.is_empty() {
        return Err(SearchError::EmptyApplicant);
    }
    let country = normalised_country(fields.country.as_deref())?;
    check_years(fields.year_from, fields.year_to)?;

    let terms: Vec<String> = names.iter().map(|n| format!("pa=\"{n}\"")).collect();
    let mut query = if terms.len() == 1 {
        terms[0].clone()
    } else {
        format!("({})", terms.join(" or "))
    };
    if let Some(country) = country {
        query.push_str(&format!(" and pn={country}"));
    }
    match (fields.year_from, fields.year_to) {
        (Some(from), Some(to)) => query.push_str(&format!(" and pd within \"{from} {to}\"")),
        (Some(from), None) => query.push_str(&format!(" and pd>={from}")),
        (None, Some(to)) => query.push_str(&format!(" and pd<={to}")),
        (None, None) => {}
    }
    Ok(query)
}

const SELECT_COLUMNS: &str = "id, name, applicant, country, year_from, year_to, query,
     total_results, next_start, imported, created_at, last_run_at";

fn row_to_search(row: &rusqlite::Row) -> rusqlite::Result<SavedSearch> {
    let mut search = SavedSearch {
        id: row.get(0)?,
        name: row.get(1)?,
        applicant: row.get(2)?,
        country: row.get(3)?,
        year_from: row.get(4)?,
        year_to: row.get(5)?,
        query: row.get(6)?,
        total_results: row.get(7)?,
        next_start: row.get(8)?,
        imported: row.get(9)?,
        created_at: row.get(10)?,
        last_run_at: row.get(11)?,
        exhausted: false,
        results_read: 0,
        capped: false,
    };
    search.exhausted = next_range(&search).is_none();
    search.results_read = search
        .total_results
        .map_or(0, |total| (search.next_start - 1).min(total));
    search.capped = search
        .total_results
        .is_some_and(|total| total > MAX_RESULTS);
    Ok(search)
}

/// Saves a new search. Its query is built from `fields`; the fields are
/// stored as entered (country upper-cased) for display.
pub fn create(
    conn: &Connection,
    fields: &SearchFields,
    now: &str,
) -> Result<SavedSearch, SearchError> {
    let name = fields.name.trim();
    if name.is_empty() {
        return Err(SearchError::EmptyName);
    }
    let query = build_query(fields)?;
    if get_by_name(conn, name)?.is_some() {
        return Err(SearchError::DuplicateName(name.to_string()));
    }
    let country = normalised_country(fields.country.as_deref())?;
    conn.execute(
        "INSERT INTO saved_searches (name, applicant, country, year_from, year_to, query, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            name,
            fields.applicant.trim(),
            country,
            fields.year_from,
            fields.year_to,
            query,
            now
        ],
    )?;
    get_by_name(conn, name)?.ok_or_else(|| SearchError::NotFound(name.to_string()))
}

pub fn list(conn: &Connection) -> Result<Vec<SavedSearch>, SearchError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLUMNS} FROM saved_searches ORDER BY name COLLATE NOCASE ASC"
    ))?;
    let rows = stmt
        .query_map([], row_to_search)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<SavedSearch>, SearchError> {
    Ok(conn
        .query_row(
            &format!("SELECT {SELECT_COLUMNS} FROM saved_searches WHERE id = ?1"),
            params![id],
            row_to_search,
        )
        .optional()?)
}

pub fn get_by_name(conn: &Connection, name: &str) -> Result<Option<SavedSearch>, SearchError> {
    Ok(conn
        .query_row(
            &format!("SELECT {SELECT_COLUMNS} FROM saved_searches WHERE name = ?1"),
            params![name.trim()],
            row_to_search,
        )
        .optional()?)
}

/// Deletes the search. Documents it imported stay.
pub fn delete(conn: &Connection, id: i64) -> Result<(), SearchError> {
    conn.execute("DELETE FROM saved_searches WHERE id = ?1", params![id])?;
    Ok(())
}

/// Starts the search over from its first result (SPEC 5.6), to pick up
/// new publications. Families already imported are skipped again.
pub fn restart(conn: &Connection, id: i64) -> Result<(), SearchError> {
    conn.execute(
        "UPDATE saved_searches SET next_start = 1, total_results = NULL WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

/// The 1-based, inclusive range of results the next batch reads, or `None`
/// when every reachable result (at most [`MAX_RESULTS`]) has been read.
/// Before the first batch the total is unknown, so a full page is asked
/// for.
pub fn next_range(search: &SavedSearch) -> Option<(i64, i64)> {
    let reachable = search
        .total_results
        .map_or(MAX_RESULTS, |total| total.min(MAX_RESULTS));
    let start = search.next_start;
    if start > reachable {
        return None;
    }
    Some((start, (start + PAGE_SIZE - 1).min(reachable)))
}

/// Records a batch that read results up to `range_end` out of
/// `total_results` and imported `imported` documents.
pub fn record_batch(
    conn: &Connection,
    id: i64,
    total_results: i64,
    range_end: i64,
    imported: i64,
    now: &str,
) -> Result<(), SearchError> {
    conn.execute(
        "UPDATE saved_searches
         SET total_results = ?2, next_start = ?3, imported = imported + ?4, last_run_at = ?5
         WHERE id = ?1",
        params![id, total_results, range_end + 1, imported, now],
    )?;
    Ok(())
}

/// One search result, as far as choosing a family's document needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub country: String,
    pub doc_number: String,
    pub kind: String,
    pub family_id: Option<String>,
    /// ISO 8601 (`YYYY-MM-DD`).
    pub publication_date: Option<String>,
    pub has_english_abstract: bool,
}

impl SearchHit {
    /// SPEC 4.2 `pub_key`: country plus number, without kind code.
    pub fn pub_key(&self) -> String {
        format!("{}{}", self.country, self.doc_number)
    }

    /// The docdb id, e.g. `EP.1234567.A1`, kept as the document's raw
    /// input.
    pub fn docdb_id(&self) -> String {
        format!("{}.{}.{}", self.country, self.doc_number, self.kind)
    }
}

/// One hit per family (SPEC 5.6), in order of each family's first
/// appearance: the earliest publication with an English abstract, or the
/// earliest publication when none has one. Hits without a family id are
/// grouped by `pub_key`. A missing date sorts last; ties keep page order.
pub fn one_per_family(hits: &[SearchHit]) -> Vec<&SearchHit> {
    let mut groups: Vec<(String, Vec<&SearchHit>)> = Vec::new();
    for hit in hits {
        let key = hit
            .family_id
            .clone()
            .unwrap_or_else(|| format!("pub:{}", hit.pub_key()));
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, members)) => members.push(hit),
            None => groups.push((key, vec![hit])),
        }
    }
    groups
        .into_iter()
        .filter_map(|(_, members)| {
            members
                .iter()
                .enumerate()
                .min_by_key(|(i, hit)| {
                    (
                        !hit.has_english_abstract,
                        hit.publication_date.is_none(),
                        hit.publication_date.clone(),
                        *i,
                    )
                })
                .map(|(_, hit)| *hit)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;

    const NOW: &str = "2026-01-01T00:00:00Z";

    fn fields(applicant: &str) -> SearchFields {
        SearchFields {
            name: "S".to_string(),
            applicant: applicant.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn build_query_with_one_applicant() {
        assert_eq!(
            build_query(&fields("Siemens AG")).unwrap(),
            "pa=\"Siemens AG\""
        );
    }

    #[test]
    fn build_query_combines_name_variants_and_optional_filters() {
        let f = SearchFields {
            country: Some(" ep ".to_string()),
            year_from: Some(2020),
            year_to: Some(2024),
            ..fields("Siemens; Siemens  Healthineers ;")
        };
        assert_eq!(
            build_query(&f).unwrap(),
            "(pa=\"Siemens\" or pa=\"Siemens Healthineers\") and pn=EP and pd within \"2020 2024\""
        );
    }

    #[test]
    fn build_query_with_an_open_year_range() {
        let from = SearchFields {
            year_from: Some(2020),
            ..fields("X")
        };
        let to = SearchFields {
            year_to: Some(2024),
            ..fields("X")
        };
        assert_eq!(build_query(&from).unwrap(), "pa=\"X\" and pd>=2020");
        assert_eq!(build_query(&to).unwrap(), "pa=\"X\" and pd<=2024");
    }

    #[test]
    fn build_query_strips_characters_that_break_a_quoted_term() {
        assert_eq!(build_query(&fields("A \"B\" \\C")).unwrap(), "pa=\"A B C\"");
    }

    #[test]
    fn build_query_rejects_invalid_fields() {
        assert!(matches!(
            build_query(&fields(" ; ")),
            Err(SearchError::EmptyApplicant)
        ));
        let country = SearchFields {
            country: Some("EPO".to_string()),
            ..fields("X")
        };
        assert!(matches!(
            build_query(&country),
            Err(SearchError::InvalidCountry)
        ));
        let years = SearchFields {
            year_from: Some(2024),
            year_to: Some(2020),
            ..fields("X")
        };
        assert!(matches!(
            build_query(&years),
            Err(SearchError::InvalidYears)
        ));
        let short = SearchFields {
            year_from: Some(99),
            ..fields("X")
        };
        assert!(matches!(
            build_query(&short),
            Err(SearchError::InvalidYears)
        ));
    }

    #[test]
    fn create_stores_the_query_and_rejects_a_duplicate_name() {
        let conn = storage::open_in_memory().unwrap();
        let f = SearchFields {
            country: Some("ep".to_string()),
            ..fields("Siemens")
        };
        let search = create(&conn, &f, NOW).unwrap();
        assert_eq!(search.query, "pa=\"Siemens\" and pn=EP");
        assert_eq!(search.country.as_deref(), Some("EP"));
        assert_eq!(
            (search.next_start, search.imported, search.total_results),
            (1, 0, None)
        );
        assert!(!search.exhausted);
        assert!(matches!(
            create(&conn, &f, NOW),
            Err(SearchError::DuplicateName(_))
        ));
        let unnamed = SearchFields {
            name: " ".to_string(),
            ..fields("X")
        };
        assert!(matches!(
            create(&conn, &unnamed, NOW),
            Err(SearchError::EmptyName)
        ));
    }

    #[test]
    fn batches_advance_until_the_reachable_results_are_read_and_restart_resets() {
        let conn = storage::open_in_memory().unwrap();
        let search = create(&conn, &fields("X"), NOW).unwrap();
        assert_eq!(next_range(&search), Some((1, 100)));

        record_batch(&conn, search.id, 150, 100, 40, NOW).unwrap();
        let search = get(&conn, search.id).unwrap().unwrap();
        assert_eq!(next_range(&search), Some((101, 150)));
        assert_eq!(search.imported, 40);
        assert_eq!(search.results_read, 100);
        assert!(!search.capped);

        record_batch(&conn, search.id, 150, 150, 10, NOW).unwrap();
        let search = get(&conn, search.id).unwrap().unwrap();
        assert_eq!(next_range(&search), None);
        assert!(search.exhausted);
        assert_eq!(search.imported, 50);

        restart(&conn, search.id).unwrap();
        let search = get(&conn, search.id).unwrap().unwrap();
        assert_eq!(next_range(&search), Some((1, 100)));
        assert_eq!(search.imported, 50);
    }

    #[test]
    fn next_range_stops_at_the_ops_result_cap() {
        let conn = storage::open_in_memory().unwrap();
        let search = create(&conn, &fields("X"), NOW).unwrap();
        record_batch(&conn, search.id, 5000, 1900, 0, NOW).unwrap();
        let search = get(&conn, search.id).unwrap().unwrap();
        assert_eq!(next_range(&search), Some((1901, 2000)));
        record_batch(&conn, search.id, 5000, 2000, 0, NOW).unwrap();
        let search = get(&conn, search.id).unwrap().unwrap();
        assert!(search.exhausted);
        assert!(search.capped);
        assert_eq!(search.results_read, 2000);
    }

    #[test]
    fn delete_removes_the_search() {
        let conn = storage::open_in_memory().unwrap();
        let search = create(&conn, &fields("X"), NOW).unwrap();
        delete(&conn, search.id).unwrap();
        assert!(list(&conn).unwrap().is_empty());
    }

    fn hit(
        number: &str,
        kind: &str,
        family: Option<&str>,
        date: Option<&str>,
        en: bool,
    ) -> SearchHit {
        SearchHit {
            country: "EP".to_string(),
            doc_number: number.to_string(),
            kind: kind.to_string(),
            family_id: family.map(str::to_string),
            publication_date: date.map(str::to_string),
            has_english_abstract: en,
        }
    }

    #[test]
    fn one_per_family_keeps_the_earliest_english_publication() {
        let hits = vec![
            hit("3", "B1", Some("f1"), Some("2022-01-01"), true),
            hit("1", "A1", Some("f1"), Some("2019-01-01"), false),
            hit("2", "A1", Some("f1"), Some("2020-01-01"), true),
            hit("9", "A1", Some("f2"), Some("2021-01-01"), true),
        ];
        let chosen: Vec<String> = one_per_family(&hits).iter().map(|h| h.docdb_id()).collect();
        assert_eq!(chosen, vec!["EP.2.A1", "EP.9.A1"]);
    }

    #[test]
    fn one_per_family_falls_back_to_the_earliest_publication_without_english() {
        let hits = vec![
            hit("2", "A1", Some("f1"), None, false),
            hit("1", "A1", Some("f1"), Some("2020-01-01"), false),
        ];
        assert_eq!(one_per_family(&hits)[0].docdb_id(), "EP.1.A1");
    }

    #[test]
    fn one_per_family_groups_hits_without_a_family_id_by_pub_key() {
        let hits = vec![
            hit("1", "B1", None, Some("2022-01-01"), true),
            hit("1", "A1", None, Some("2020-01-01"), true),
            hit("2", "A1", None, Some("2020-01-01"), true),
        ];
        let chosen: Vec<String> = one_per_family(&hits).iter().map(|h| h.docdb_id()).collect();
        assert_eq!(chosen, vec!["EP.1.A1", "EP.2.A1"]);
    }
}
