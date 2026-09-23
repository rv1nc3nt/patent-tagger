//! Parses OPS full-text inquiry/description/claims responses (SPEC 5.5).
//! XML shapes verified against real recorded responses in
//! `tests/fixtures/ops/` (see docs/DECISIONS.md).

use crate::error::OpsError;
use roxmltree::{Document, Node};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FulltextPart {
    Description,
    Claims,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FulltextInstance {
    pub lang: String,
    pub part: FulltextPart,
}

/// Which parts exist for a publication, and in which language(s) - SPEC
/// 5.5: "the code must not assume availability: it relies on the inquiry
/// result."
pub fn parse_inquiry(xml: &str) -> Result<Vec<FulltextInstance>, OpsError> {
    let doc = Document::parse(xml).map_err(|e| OpsError::Parse(format!("invalid XML: {e}")))?;
    let instances = doc
        .descendants()
        .filter(|n| n.has_tag_name("fulltext-instance"))
        .filter_map(|n| {
            let lang = n.attribute("lang")?.to_string();
            let part = match n.attribute("desc")? {
                "description" => FulltextPart::Description,
                "claims" => FulltextPart::Claims,
                _ => return None,
            };
            Some(FulltextInstance { lang, part })
        })
        .collect();
    Ok(instances)
}

/// The description's text, converted to plain text (SPEC 5.5): paragraph
/// numbers kept as `[0001]` (already literal text in OPS's own `<p>`
/// content - verified against `ep1000000_a1_description.xml`), one
/// paragraph per line, entities decoded (roxmltree does this
/// automatically), whitespace normalised, non-text content (tables,
/// chemistry, maths) replaced with a placeholder rather than garbled text.
///
/// `lang` selects among multiple `<description>` elements when the
/// response bundles more than one language (mirrors [`parse_claims`],
/// where a real EP B1 response bundles DE/FR/EN in one call - see
/// docs/DECISIONS.md). Case-insensitive, matching OPS's own attribute
/// casing (`"EN"`).
pub fn parse_description(xml: &str, lang: &str) -> Result<Option<(String, String)>, OpsError> {
    let doc = Document::parse(xml).map_err(|e| OpsError::Parse(format!("invalid XML: {e}")))?;
    let Some(description) = doc
        .descendants()
        .find(|n| n.has_tag_name("description") && n.attribute("lang").is_some_and(|l| l.eq_ignore_ascii_case(lang)))
    else {
        return Ok(None);
    };
    let lang = description.attribute("lang").unwrap_or("").to_string();

    let mut paragraph_number = 0;
    let text = description
        .children()
        .filter(|n| n.has_tag_name("p"))
        .map(|p| {
            paragraph_number += 1;
            paragraph_text(p, paragraph_number)
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Some((lang, text)))
}

/// The claims' text, converted to plain text: claim numbers kept as
/// literal text (already embedded in `<claim-text>` content - verified
/// against `ep1000000_a1_claims.xml`), one claim per line. Iterates every
/// `<claim-text>` regardless of how many `<claim>` wrapper elements
/// surround them, since a single real response bundled all claims under
/// one `<claim>`.
///
/// `lang` selects among multiple `<claims>` elements - a real EP B1
/// response bundles all of DE/FR/EN in one call (see
/// `ep1000000_b1_claims_trilingual.xml`), so the caller must pick.
/// Case-insensitive, matching OPS's own attribute casing (`"EN"`).
pub fn parse_claims(xml: &str, lang: &str) -> Result<Option<(String, String)>, OpsError> {
    let doc = Document::parse(xml).map_err(|e| OpsError::Parse(format!("invalid XML: {e}")))?;
    let Some(claims) = doc
        .descendants()
        .find(|n| n.has_tag_name("claims") && n.attribute("lang").is_some_and(|l| l.eq_ignore_ascii_case(lang)))
    else {
        return Ok(None);
    };
    let lang = claims.attribute("lang").unwrap_or("").to_string();

    let mut placeholder_index = 0;
    let text = claims
        .descendants()
        .filter(|n| n.has_tag_name("claim-text"))
        .map(|n| plain_text_of(n, &mut placeholder_index))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Some((lang, text)))
}

/// Local names of elements known to hold non-plain-text content (tables,
/// chemical/mathematical formulae) that SPEC 5.5 says to replace with a
/// placeholder rather than reproduce. Not verified against a real example
/// (none of this project's recorded fixtures contain one - see
/// docs/DECISIONS.md); based on the element names OPS's fulltext DTD
/// documents for this content.
const NON_TEXT_ELEMENTS: &[&str] = &["tables", "maths", "chemistry"];

fn paragraph_text(p: Node, paragraph_number: usize) -> String {
    if let Some(placeholder) = non_text_placeholder(p, paragraph_number) {
        return placeholder;
    }
    normalise_whitespace(&text_content(p))
}

fn plain_text_of(node: Node, placeholder_index: &mut usize) -> String {
    if let Some(kind) = node.descendants().find_map(|n| non_text_kind(n)) {
        *placeholder_index += 1;
        return format!("[{kind} not reproduced]");
    }
    normalise_whitespace(&text_content(node))
}

fn non_text_placeholder(node: Node, index: usize) -> Option<String> {
    let kind = node.descendants().find_map(non_text_kind)?;
    Some(format!("[{kind} {index} not reproduced]"))
}

fn non_text_kind(node: Node) -> Option<&'static str> {
    match node.tag_name().name() {
        "tables" => Some("Table"),
        "maths" => Some("Formula"),
        "chemistry" => Some("Chemical structure"),
        _ if NON_TEXT_ELEMENTS.contains(&node.tag_name().name()) => Some("Content"),
        _ => None,
    }
}

fn text_content(node: Node) -> String {
    node.descendants().filter_map(|n| n.text()).collect::<Vec<_>>().join("")
}

fn normalise_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture(name: &str) -> String {
        fs::read_to_string(format!("{}/../../tests/fixtures/ops/{name}", env!("CARGO_MANIFEST_DIR")))
            .unwrap_or_else(|e| panic!("reading fixture {name}: {e}"))
    }

    #[test]
    fn inquiry_lists_available_parts_and_languages() {
        let instances = parse_inquiry(&fixture("ep1000000_a1_fulltext_inquiry.xml")).unwrap();
        assert_eq!(
            instances,
            vec![
                FulltextInstance { lang: "EN".to_string(), part: FulltextPart::Description },
                FulltextInstance { lang: "EN".to_string(), part: FulltextPart::Claims },
            ]
        );
    }

    #[test]
    fn description_keeps_paragraph_numbers_and_normalises_whitespace() {
        let (lang, text) =
            parse_description(&fixture("ep1000000_a1_description.xml"), "en").unwrap().unwrap();
        assert_eq!(lang, "EN");
        assert!(text.starts_with("[0001] The invention relates to an apparatus"));
        assert!(text.contains("[0022]"));
        assert!(!text.contains("  "), "whitespace should be normalised to single spaces");
    }

    #[test]
    fn claims_keeps_claim_numbers_one_per_line() {
        let (lang, text) = parse_claims(&fixture("ep1000000_a1_claims.xml"), "en").unwrap().unwrap();
        assert_eq!(lang, "EN");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 11);
        assert!(lines[0].starts_with("1. Apparatus for manufacturing"));
        assert!(lines[10].starts_with("11. Apparatus as claimed in claim 10"));
    }

    #[test]
    fn b1_claims_are_genuinely_trilingual_and_parsing_picks_out_english_only() {
        let xml = fixture("ep1000000_b1_claims_trilingual.xml");
        let doc = Document::parse(&xml).unwrap();
        let langs: Vec<&str> =
            doc.descendants().filter(|n| n.has_tag_name("claims")).filter_map(|n| n.attribute("lang")).collect();
        assert_eq!(langs, vec!["DE", "FR", "EN"]);

        // SPEC 5.5: "for an EP B1, take the English claims" - the cascade
        // decision itself is core_lib's job; this confirms parse_claims
        // can target the specific English element rather than picking
        // document order (which would silently return German).
        let (lang, text) = parse_claims(&xml, "en").unwrap().unwrap();
        assert_eq!(lang, "EN");
        assert!(text.starts_with("1. Apparatus for manufacturing"));

        let (german_lang, german_text) = parse_claims(&xml, "de").unwrap().unwrap();
        assert_eq!(german_lang, "DE");
        assert!(german_text.starts_with("1. Vorrichtung zum Herstellen"));
    }

    #[test]
    fn german_description_parses_the_same_way_as_english() {
        let (lang, text) =
            parse_description(&fixture("synthetic_ep_fr_de_only_description.xml"), "de").unwrap().unwrap();
        assert_eq!(lang, "DE");
        assert!(text.starts_with("[0001] Die Erfindung betrifft"));
    }

    #[test]
    fn requesting_a_language_that_is_not_present_returns_none() {
        assert_eq!(parse_description(&fixture("ep1000000_a1_description.xml"), "de").unwrap(), None);
        assert_eq!(parse_claims(&fixture("ep1000000_a1_claims.xml"), "de").unwrap(), None);
    }

    #[test]
    fn not_found_fault_has_no_fulltext_instances() {
        let instances = parse_inquiry(&fixture("us5960411_fulltext_not_available_404.xml")).unwrap();
        assert!(instances.is_empty());
    }
}
