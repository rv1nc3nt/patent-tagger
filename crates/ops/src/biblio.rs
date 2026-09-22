//! Parses `published-data/publication/.../biblio,abstract` and
//! `family/publication/.../biblio` responses (SPEC 5.3-5.4) into
//! [`Publication`] values. XML element/attribute names verified against
//! real recorded responses in `tests/fixtures/ops/` (see docs/DECISIONS.md).

use crate::error::OpsError;
use roxmltree::{Document, Node};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publication {
    /// `{country}.{doc_number}.{kind}`, e.g. `"EP.1000000.A1"` - the docdb
    /// id this publication was fetched/cited by (SPEC 4.2 `abstract_source`
    /// / `source` columns).
    pub docdb_id: String,
    pub country: String,
    pub doc_number: String,
    pub kind: String,
    pub family_id: Option<String>,
    pub application_number: Option<String>,
    /// ISO 8601 (`YYYY-MM-DD`), from the publication-reference's date.
    pub publication_date: Option<String>,
    pub titles: BTreeMap<String, String>,
    pub abstracts: BTreeMap<String, String>,
    pub applicants: Vec<String>,
    pub ipc: Vec<String>,
    pub cpc: Vec<String>,
}

impl Publication {
    pub fn title(&self, lang: &str) -> Option<&str> {
        self.titles.get(lang).map(String::as_str)
    }

    pub fn abstract_text(&self, lang: &str) -> Option<&str> {
        self.abstracts.get(lang).map(String::as_str)
    }
}

/// Parses every `exchange-document` in the response. A `published-data`
/// response for a single number has exactly one; a `family` response has
/// one per family member.
pub fn parse(xml: &str) -> Result<Vec<Publication>, OpsError> {
    let doc = Document::parse(xml).map_err(|e| OpsError::Parse(format!("invalid XML: {e}")))?;
    doc.descendants()
        .filter(|n| n.has_tag_name("exchange-document"))
        .map(parse_exchange_document)
        .collect()
}

fn parse_exchange_document(node: Node) -> Result<Publication, OpsError> {
    let country = attr(node, "country")?;
    let doc_number = attr(node, "doc-number")?;
    let kind = attr(node, "kind")?;
    let family_id = node.attribute("family-id").map(str::to_string);
    let docdb_id = format!("{country}.{doc_number}.{kind}");

    let biblio = child(node, "bibliographic-data");

    let application_number = biblio
        .and_then(|b| child(b, "application-reference"))
        .and_then(application_number_from);

    let publication_date = biblio
        .and_then(|b| child(b, "publication-reference"))
        .and_then(docdb_document_id)
        .and_then(|d| child(d, "date"))
        .and_then(|d| d.text())
        .and_then(format_ops_date);

    let titles = biblio
        .map(|b| {
            b.children()
                .filter(|n| n.has_tag_name("invention-title"))
                .filter_map(|n| Some((n.attribute("lang")?.to_string(), text_of(n))))
                .collect()
        })
        .unwrap_or_default();

    let abstracts = node
        .children()
        .filter(|n| n.has_tag_name("abstract"))
        .filter_map(|n| Some((n.attribute("lang")?.to_string(), joined_paragraph_text(n))))
        .collect();

    let applicants = biblio
        .map(|b| {
            b.descendants()
                .filter(|n| {
                    n.has_tag_name("applicant") && n.attribute("data-format") == Some("epodoc")
                })
                .filter_map(|n| child(n, "applicant-name"))
                .filter_map(|n| child(n, "name"))
                .map(text_of)
                .collect()
        })
        .unwrap_or_default();

    let ipc = biblio
        .and_then(|b| child(b, "classification-ipc"))
        .map(|c| {
            c.children()
                .filter(|n| n.has_tag_name("text"))
                .map(text_of)
                .collect()
        })
        .unwrap_or_default();

    let cpc = biblio
        .and_then(|b| child(b, "patent-classifications"))
        .map(|pc| {
            pc.children()
                .filter(|n| n.has_tag_name("patent-classification"))
                .map(cpc_symbol)
                .collect()
        })
        .unwrap_or_default();

    Ok(Publication {
        docdb_id,
        country,
        doc_number,
        kind,
        family_id,
        application_number,
        publication_date,
        titles,
        abstracts,
        applicants,
        ipc,
        cpc,
    })
}

fn application_number_from(app_ref: Node) -> Option<String> {
    let epodoc = app_ref
        .children()
        .find(|n| n.has_tag_name("document-id") && n.attribute("document-id-type") == Some("epodoc"));
    if let Some(n) = epodoc {
        if let Some(number) = child(n, "doc-number").map(text_of) {
            return Some(number);
        }
    }
    let docdb = docdb_document_id(app_ref)?;
    let country = child(docdb, "country").map(text_of)?;
    let doc_number = child(docdb, "doc-number").map(text_of)?;
    Some(format!("{country}{doc_number}"))
}

fn docdb_document_id<'a, 'input>(parent: Node<'a, 'input>) -> Option<Node<'a, 'input>> {
    parent
        .children()
        .find(|n| n.has_tag_name("document-id") && n.attribute("document-id-type") == Some("docdb"))
}

/// `"20000517"` -> `"2000-05-17"`.
fn format_ops_date(raw: &str) -> Option<String> {
    if raw.len() != 8 || !raw.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(format!("{}-{}-{}", &raw[0..4], &raw[4..6], &raw[6..8]))
}

/// CPC symbol as `{section}{class}{subclass}{main-group}/{subgroup}`, e.g.
/// `"B28B1/29"`.
fn cpc_symbol(node: Node) -> String {
    let part = |tag: &str| child(node, tag).map(text_of).unwrap_or_default();
    format!(
        "{}{}{}{}/{}",
        part("section"),
        part("class"),
        part("subclass"),
        part("main-group"),
        part("subgroup"),
    )
}

fn joined_paragraph_text(node: Node) -> String {
    node.children()
        .filter(|n| n.has_tag_name("p"))
        .map(text_of)
        .collect::<Vec<_>>()
        .join("\n")
}

fn text_of(node: Node) -> String {
    node.text().unwrap_or_default().trim().to_string()
}

fn child<'a, 'input>(node: Node<'a, 'input>, tag: &str) -> Option<Node<'a, 'input>> {
    node.children().find(|n| n.has_tag_name(tag))
}

fn attr(node: Node, name: &str) -> Result<String, OpsError> {
    node.attribute(name)
        .map(str::to_string)
        .ok_or_else(|| OpsError::Parse(format!("missing @{name} on <exchange-document>")))
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
    fn ep_a1_with_english_abstract() {
        let pubs = parse(&fixture("ep1000000_a1_biblio_abstract.xml")).expect("should parse");
        assert_eq!(pubs.len(), 1);
        let p = &pubs[0];
        assert_eq!(p.docdb_id, "EP.1000000.A1");
        assert_eq!(p.family_id.as_deref(), Some("19768124"));
        assert_eq!(p.application_number.as_deref(), Some("EP19990203729"));
        assert_eq!(p.publication_date.as_deref(), Some("2000-05-17"));
        assert_eq!(
            p.title("en"),
            Some("Apparatus for manufacturing green bricks for the brick manufacturing industry")
        );
        assert!(p.title("de").is_some());
        assert!(p.title("fr").is_some());
        assert!(p.abstract_text("en").unwrap().starts_with("[0001]"));
        assert!(!p.applicants.is_empty());
        assert!(p.ipc.contains(&"B28B1/29".to_string()));
        assert!(p.cpc.contains(&"B28B1/29".to_string()));
    }

    #[test]
    fn ep_b1_grant_has_an_abstract_in_this_real_case() {
        // Contrary to the SPEC's example case, this real B1 does carry an
        // English abstract; see synthetic_ep_b1_no_abstract.xml for the
        // no-abstract-at-all case, which needed a synthetic fixture.
        let pubs = parse(&fixture("ep1000000_b1_biblio_abstract.xml")).expect("should parse");
        assert_eq!(pubs[0].kind, "B1");
        assert!(pubs[0].abstract_text("en").is_some());
    }

    #[test]
    fn synthetic_ep_b1_has_no_abstract_at_all() {
        let pubs = parse(&fixture("synthetic_ep_b1_no_abstract.xml")).expect("should parse");
        assert!(pubs[0].abstracts.is_empty());
        assert_eq!(pubs[0].title("en"), Some("Apparatus for manufacturing green bricks"));
    }

    #[test]
    fn synthetic_ep_a1_has_only_french_and_german() {
        let pubs =
            parse(&fixture("synthetic_ep_fr_de_only_a1_biblio_abstract.xml")).expect("should parse");
        let p = &pubs[0];
        assert!(p.title("en").is_none());
        assert!(p.abstract_text("en").is_none());
        assert!(p.title("de").is_some());
        assert!(p.abstract_text("de").is_some());
    }

    #[test]
    fn synthetic_family_has_an_english_wo_member() {
        let pubs = parse(&fixture("synthetic_family_with_english_wo_member.xml")).expect("should parse");
        assert_eq!(pubs.len(), 2);
        let wo = pubs.iter().find(|p| p.country == "WO").expect("WO member present");
        assert_eq!(wo.abstract_text("en"), Some("The invention relates to a data processing apparatus comprising a processor and a memory."));
    }

    #[test]
    fn us_grant_pre_2001_kind_code() {
        let pubs = parse(&fixture("us_5960411_grant_biblio_abstract.xml")).expect("should parse");
        assert_eq!(pubs[0].country, "US");
        assert_eq!(pubs[0].kind, "A");
        assert!(pubs[0].abstract_text("en").is_some());
    }

    #[test]
    fn us_b1_grant() {
        let pubs = parse(&fixture("us_6285999_b1_biblio_abstract.xml")).expect("should parse");
        assert_eq!(pubs[0].kind, "B1");
    }

    #[test]
    fn wo_publication() {
        let pubs = parse(&fixture("wo_2019123456_a1_biblio_abstract.xml")).expect("should parse");
        assert_eq!(pubs[0].country, "WO");
        assert_eq!(pubs[0].doc_number, "2019123456");
    }

    #[test]
    fn family_response_has_multiple_members() {
        let pubs = parse(&fixture("ep1000000_family_biblio.xml")).expect("should parse");
        assert_eq!(pubs.len(), 5);
        assert!(pubs.iter().any(|p| p.country == "US" && p.doc_number == "6093011"));
    }

    #[test]
    fn not_found_fault_is_not_valid_exchange_data() {
        let pubs = parse(&fixture("not_found_fault.xml")).expect("a fault has no exchange-document, which is not itself an error");
        assert!(pubs.is_empty());
    }
}
