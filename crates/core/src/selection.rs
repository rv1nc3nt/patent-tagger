//! The English title/abstract fallback cascade (SPEC section 5.3):
//! 1. the requested publication's own English text;
//! 2. another publication of the same application;
//! 3. a family member, preferring WO, then US, GB, EP, then any other;
//! 4. otherwise none (caller sets `fetch_status = no_english_abstract`).
//!
//! Deliberately decoupled from `crates/ops`'s response types, so this
//! stays part of core's network-free, easily-tested domain logic; callers
//! (src-tauri) convert `ops::biblio::Publication` into [`Candidate`].

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub docdb_id: String,
    pub country: String,
    pub application_number: Option<String>,
    pub titles: BTreeMap<String, String>,
    pub abstracts: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selected {
    pub text: String,
    pub source_docdb_id: String,
}

pub fn select_abstract(requested: &Candidate, family_members: &[Candidate]) -> Option<Selected> {
    select_text(requested, family_members, |c| &c.abstracts)
}

pub fn select_title(requested: &Candidate, family_members: &[Candidate]) -> Option<Selected> {
    select_text(requested, family_members, |c| &c.titles)
}

/// Country preference for step 3 of the cascade: WO, then US, GB, EP, then
/// any other (lower is more preferred).
fn family_country_priority(country: &str) -> u8 {
    match country {
        "WO" => 0,
        "US" => 1,
        "GB" => 2,
        "EP" => 3,
        _ => 4,
    }
}

fn select_text(
    requested: &Candidate,
    family_members: &[Candidate],
    field: impl Fn(&Candidate) -> &BTreeMap<String, String>,
) -> Option<Selected> {
    if let Some(text) = field(requested).get("en") {
        return Some(Selected {
            text: text.clone(),
            source_docdb_id: requested.docdb_id.clone(),
        });
    }

    let others = || {
        family_members
            .iter()
            .filter(|c| c.docdb_id != requested.docdb_id)
    };

    if let Some(app_num) = requested.application_number.as_deref() {
        if let Some(c) = others().find(|c| {
            c.application_number.as_deref() == Some(app_num) && field(c).contains_key("en")
        }) {
            return Some(Selected {
                text: field(c)["en"].clone(),
                source_docdb_id: c.docdb_id.clone(),
            });
        }
    }

    others()
        .filter(|c| field(c).contains_key("en"))
        .min_by_key(|c| family_country_priority(&c.country))
        .map(|c| Selected {
            text: field(c)["en"].clone(),
            source_docdb_id: c.docdb_id.clone(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(docdb_id: &str, country: &str, app_num: Option<&str>, langs: &[(&str, &str)]) -> Candidate {
        Candidate {
            docdb_id: docdb_id.to_string(),
            country: country.to_string(),
            application_number: app_num.map(str::to_string),
            titles: BTreeMap::new(),
            abstracts: langs.iter().map(|(l, t)| (l.to_string(), t.to_string())).collect(),
        }
    }

    #[test]
    fn prefers_the_requested_publications_own_english_abstract() {
        let requested = candidate("EP.1.A1", "EP", Some("EP1"), &[("en", "own abstract")]);
        let family = vec![candidate("US.1.A", "US", Some("US1"), &[("en", "other abstract")])];
        let selected = select_abstract(&requested, &family).unwrap();
        assert_eq!(selected.text, "own abstract");
        assert_eq!(selected.source_docdb_id, "EP.1.A1");
    }

    #[test]
    fn falls_back_to_another_publication_of_the_same_application() {
        let requested = candidate("EP.1.B1", "EP", Some("EP1"), &[]);
        let family = vec![
            candidate("EP.1.A1", "EP", Some("EP1"), &[("en", "A1 abstract")]),
            candidate("WO.1.A1", "WO", Some("WO1"), &[("en", "WO abstract")]),
        ];
        let selected = select_abstract(&requested, &family).unwrap();
        assert_eq!(selected.text, "A1 abstract");
        assert_eq!(selected.source_docdb_id, "EP.1.A1");
    }

    #[test]
    fn falls_back_to_family_member_preferring_wo_over_us_over_gb_over_ep_over_other() {
        let requested = candidate("EP.1.A1", "EP", Some("EP1"), &[]);
        let family = vec![
            candidate("DE.1.A1", "DE", Some("DE1"), &[("en", "DE abstract")]),
            candidate("US.1.A1", "US", Some("US1"), &[("en", "US abstract")]),
            candidate("WO.1.A1", "WO", Some("WO1"), &[("en", "WO abstract")]),
            candidate("GB.1.A1", "GB", Some("GB1"), &[("en", "GB abstract")]),
        ];
        let selected = select_abstract(&requested, &family).unwrap();
        assert_eq!(selected.source_docdb_id, "WO.1.A1", "WO should win over US/GB/DE");
    }

    #[test]
    fn no_english_anywhere_yields_none() {
        let requested = candidate("EP.1.A1", "EP", Some("EP1"), &[("de", "nur deutsch")]);
        let family = vec![candidate("EP.1.B1", "EP", Some("EP1"), &[("fr", "seulement en français")])];
        assert_eq!(select_abstract(&requested, &family), None);
    }

    #[test]
    fn same_application_step_is_skipped_when_only_other_applications_share_the_family() {
        // A family member with English text but a *different* application
        // number should only be picked up by the family-fallback step, not
        // mistaken for "another publication of the same application".
        let requested = candidate("EP.1.A1", "EP", Some("EP1"), &[]);
        let family = vec![candidate("US.1.A1", "US", Some("US-continuation"), &[("en", "US abstract")])];
        let selected = select_abstract(&requested, &family).unwrap();
        assert_eq!(selected.source_docdb_id, "US.1.A1");
    }

    #[test]
    fn title_selection_follows_the_same_cascade() {
        let mut requested = candidate("EP.1.A1", "EP", Some("EP1"), &[]);
        requested.titles.insert("de".to_string(), "nur deutsch".to_string());
        let mut wo = candidate("WO.1.A1", "WO", Some("WO1"), &[]);
        wo.titles.insert("en".to_string(), "English title".to_string());
        let selected = select_title(&requested, &[wo]).unwrap();
        assert_eq!(selected.text, "English title");
    }
}
