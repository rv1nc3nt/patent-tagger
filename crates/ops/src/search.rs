//! `published-data/search/biblio` (SPEC 5.6): a CQL query answered with
//! one page of results and their bibliographic data, abstracts included.
//! Shapes verified against recorded live responses in
//! `tests/fixtures/ops/search_biblio_*.xml` (see docs/DECISIONS.md).

use crate::biblio::{self, Publication};
use crate::client::OpsClient;
use crate::error::OpsError;
use roxmltree::Document;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPage {
    /// Results matching the query, including those beyond the 2,000 OPS
    /// lets a client read.
    pub total_results: i64,
    pub publications: Vec<Publication>,
}

/// The request path for results `begin..=end` (1-based) of `query`. The
/// range goes in the `Range` query parameter.
pub fn path(query: &str, begin: i64, end: i64) -> String {
    format!(
        "/published-data/search/biblio?q={}&Range={begin}-{end}",
        percent_encode(query)
    )
}

/// Percent-encodes everything but RFC 3986 unreserved characters.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

pub fn parse(xml: &str) -> Result<SearchPage, OpsError> {
    let doc = Document::parse(xml).map_err(|e| OpsError::Parse(format!("invalid XML: {e}")))?;
    let total_results = doc
        .descendants()
        .find(|n| n.has_tag_name("biblio-search"))
        .and_then(|n| n.attribute("total-result-count"))
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| OpsError::Parse("search response without total-result-count".to_string()))?;
    Ok(SearchPage {
        total_results,
        publications: biblio::parse(xml)?,
    })
}

/// Runs `query` for results `begin..=end`. A query without any match is an
/// empty page, not an error (OPS answers it with a 404 fault).
pub async fn search(
    client: &OpsClient,
    query: &str,
    begin: i64,
    end: i64,
) -> Result<SearchPage, OpsError> {
    match client.get(&path(query, begin, end)).await {
        Ok(body) => parse(&body),
        Err(OpsError::Api { status: 404, .. }) => Ok(SearchPage {
            total_results: 0,
            publications: Vec::new(),
        }),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture(name: &str) -> String {
        fs::read_to_string(format!(
            "{}/../../tests/fixtures/ops/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap_or_else(|e| panic!("reading fixture {name}: {e}"))
    }

    #[test]
    fn path_encodes_the_query_and_puts_the_range_in_a_parameter() {
        assert_eq!(
            path("pa=\"Siemens AG\" and pd>=2023", 101, 200),
            "/published-data/search/biblio?q=pa%3D%22Siemens%20AG%22%20and%20pd%3E%3D2023&Range=101-200"
        );
    }

    #[test]
    fn parses_total_count_and_every_result_with_family_date_and_abstract() {
        let page = parse(&fixture("search_biblio_siemens_healthineers_2021_1-10.xml")).unwrap();
        assert_eq!(page.total_results, 170);
        assert_eq!(page.publications.len(), 10);

        let first = &page.publications[0];
        assert_eq!(first.docdb_id, "US.2021405700.A1");
        assert_eq!(first.family_id.as_deref(), Some("78826798"));
        assert_eq!(first.publication_date.as_deref(), Some("2021-12-30"));
        assert!(first.abstract_text("en").is_some());

        let german = &page.publications[2];
        assert_eq!(german.docdb_id, "DE.102020208000.A1");
        assert!(german.abstract_text("en").is_none());
        assert!(german.abstract_text("de").is_some());
    }

    #[test]
    fn total_count_may_exceed_the_readable_results() {
        let page = parse(&fixture("search_biblio_open_year_range_1-1.xml")).unwrap();
        assert_eq!(page.total_results, 2009);
        assert_eq!(page.publications.len(), 1);
    }

    #[test]
    fn a_response_without_a_total_count_is_a_parse_error() {
        assert!(matches!(
            parse(&fixture("search_biblio_beyond_2000_fault.xml")),
            Err(OpsError::Parse(_))
        ));
    }
}
