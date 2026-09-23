//! The English title/abstract fallback cascade (SPEC section 5.3):
//! 1. the requested publication's own English text;
//! 2. another publication of the same application;
//! 3. a family member, preferring WO, then US, GB, EP, then any other;
//! 4. otherwise none (caller sets `fetch_status = no_english_abstract`).
//!
//! Deliberately decoupled from `crates/ops`'s response types, so this
//! stays part of core's network-free, easily-tested domain logic; callers
//! (src-tauri) convert `ops::biblio::Publication` into [`Candidate`].

use std::collections::{BTreeMap, BTreeSet};

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

/// A publication considered as a full-text source (SPEC 5.5). `langs` is
/// the union of languages available for *either* description or claims -
/// a single publication can offer them in different language sets (e.g.
/// description German-only, claims trilingual), so the coarse cascade
/// below only decides *which publication* wins; the caller then fetches
/// whichever parts that publication actually has in the chosen language,
/// leaving the other part absent rather than mixing languages within one
/// document (see docs/DECISIONS.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FulltextCandidate {
    pub docdb_id: String,
    pub country: String,
    pub application_number: Option<String>,
    pub langs: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FulltextSelected {
    pub source_docdb_id: String,
    pub lang: String,
    /// SPEC 5.5 step 4: set when no English text exists anywhere in the
    /// cascade, so the caller stores `status = non_english_only`.
    pub non_english_only: bool,
}

/// SPEC 5.5's full-text selection cascade: the requested publication's own
/// English text, then another publication of the same application, then
/// an English family member (WO, then US, GB, EP, then any other); if none
/// of those has English at all, the same cascade is retried accepting any
/// language (picking the alphabetically-first one available at whichever
/// publication wins, for determinism - SPEC doesn't specify a tie-break
/// among non-English languages).
pub fn select_fulltext(
    requested: &FulltextCandidate,
    family_members: &[FulltextCandidate],
) -> Option<FulltextSelected> {
    if let Some(selected) = fulltext_cascade(requested, family_members, |c| {
        c.langs.iter().find(|l| l.eq_ignore_ascii_case("en")).cloned()
    }) {
        return Some(FulltextSelected { non_english_only: false, ..selected });
    }
    fulltext_cascade(requested, family_members, |c| c.langs.iter().next().cloned())
        .map(|selected| FulltextSelected { non_english_only: true, ..selected })
}

fn fulltext_cascade(
    requested: &FulltextCandidate,
    family_members: &[FulltextCandidate],
    pick_lang: impl Fn(&FulltextCandidate) -> Option<String>,
) -> Option<FulltextSelected> {
    if let Some(lang) = pick_lang(requested) {
        return Some(FulltextSelected {
            source_docdb_id: requested.docdb_id.clone(),
            lang,
            non_english_only: false,
        });
    }

    let others = || family_members.iter().filter(|c| c.docdb_id != requested.docdb_id);

    if let Some(app_num) = requested.application_number.as_deref() {
        if let Some((c, lang)) = others()
            .filter(|c| c.application_number.as_deref() == Some(app_num))
            .find_map(|c| pick_lang(c).map(|lang| (c, lang)))
        {
            return Some(FulltextSelected { source_docdb_id: c.docdb_id.clone(), lang, non_english_only: false });
        }
    }

    others()
        .filter_map(|c| pick_lang(c).map(|lang| (c, lang)))
        .min_by_key(|(c, _)| family_country_priority(&c.country))
        .map(|(c, lang)| FulltextSelected { source_docdb_id: c.docdb_id.clone(), lang, non_english_only: false })
}

/// A publication considered as a drawings source (SPEC 5.5). Drawings are
/// language-neutral, so only availability matters, not language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawingsCandidate {
    pub docdb_id: String,
    pub country: String,
    pub application_number: Option<String>,
    pub has_drawings: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawingsSelected {
    pub source_docdb_id: String,
}

/// SPEC 5.5: "prefer the drawings of the publication used for the full
/// text" - callers pass that publication (or, if no full text was
/// retrieved, the abstract's source) as `requested` - "else any
/// publication of the same application, else an English family member".
/// The last step reuses the same WO/US/GB/EP/other country priority as
/// [`select_fulltext`]/[`select_abstract`]; "English" there just means
/// "the same family member set already known to carry English text" -
/// callers build `family_members` from that same candidate set, since
/// drawings themselves carry no language attribute to filter on.
pub fn select_drawings(
    requested: &DrawingsCandidate,
    family_members: &[DrawingsCandidate],
) -> Option<DrawingsSelected> {
    if requested.has_drawings {
        return Some(DrawingsSelected { source_docdb_id: requested.docdb_id.clone() });
    }

    let others = || family_members.iter().filter(|c| c.docdb_id != requested.docdb_id);

    if let Some(app_num) = requested.application_number.as_deref() {
        if let Some(c) =
            others().find(|c| c.application_number.as_deref() == Some(app_num) && c.has_drawings)
        {
            return Some(DrawingsSelected { source_docdb_id: c.docdb_id.clone() });
        }
    }

    others()
        .filter(|c| c.has_drawings)
        .min_by_key(|c| family_country_priority(&c.country))
        .map(|c| DrawingsSelected { source_docdb_id: c.docdb_id.clone() })
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

    fn fulltext_candidate(
        docdb_id: &str,
        country: &str,
        app_num: Option<&str>,
        langs: &[&str],
    ) -> FulltextCandidate {
        FulltextCandidate {
            docdb_id: docdb_id.to_string(),
            country: country.to_string(),
            application_number: app_num.map(str::to_string),
            langs: langs.iter().map(|l| l.to_string()).collect(),
        }
    }

    #[test]
    fn fulltext_prefers_the_requested_publications_own_english_text() {
        let requested = fulltext_candidate("EP.1.A1", "EP", Some("EP1"), &["EN"]);
        let selected = select_fulltext(&requested, &[]).unwrap();
        assert_eq!(selected.source_docdb_id, "EP.1.A1");
        assert_eq!(selected.lang, "EN");
        assert!(!selected.non_english_only);
    }

    #[test]
    fn fulltext_falls_back_to_another_publication_of_the_same_application() {
        let requested = fulltext_candidate("EP.1.B1", "EP", Some("EP1"), &[]);
        let family = vec![
            fulltext_candidate("EP.1.A1", "EP", Some("EP1"), &["EN"]),
            fulltext_candidate("WO.1.A1", "WO", Some("WO1"), &["EN"]),
        ];
        let selected = select_fulltext(&requested, &family).unwrap();
        assert_eq!(selected.source_docdb_id, "EP.1.A1");
    }

    #[test]
    fn fulltext_falls_back_to_family_member_preferring_wo_over_us_over_gb_over_ep() {
        let requested = fulltext_candidate("EP.1.A1", "EP", Some("EP1"), &[]);
        let family = vec![
            fulltext_candidate("DE.1.A1", "DE", Some("DE1"), &["EN"]),
            fulltext_candidate("US.1.A1", "US", Some("US1"), &["EN"]),
            fulltext_candidate("WO.1.A1", "WO", Some("WO1"), &["EN"]),
        ];
        let selected = select_fulltext(&requested, &family).unwrap();
        assert_eq!(selected.source_docdb_id, "WO.1.A1");
    }

    #[test]
    fn fulltext_falls_back_to_non_english_when_no_english_exists_anywhere() {
        let requested = fulltext_candidate("EP.1.A1", "EP", Some("EP1"), &["DE"]);
        let family = vec![fulltext_candidate("US.1.A1", "US", Some("US-continuation"), &["DE", "FR"])];
        let selected = select_fulltext(&requested, &family).unwrap();
        assert_eq!(selected.source_docdb_id, "EP.1.A1", "own non-English text still beats a family member's");
        assert_eq!(selected.lang, "DE");
        assert!(selected.non_english_only);
    }

    #[test]
    fn fulltext_returns_none_when_nothing_is_available_anywhere() {
        let requested = fulltext_candidate("EP.1.A1", "EP", Some("EP1"), &[]);
        assert_eq!(select_fulltext(&requested, &[]), None);
    }

    fn drawings_candidate(
        docdb_id: &str,
        country: &str,
        app_num: Option<&str>,
        has_drawings: bool,
    ) -> DrawingsCandidate {
        DrawingsCandidate {
            docdb_id: docdb_id.to_string(),
            country: country.to_string(),
            application_number: app_num.map(str::to_string),
            has_drawings,
        }
    }

    #[test]
    fn drawings_prefer_the_requested_publication() {
        let requested = drawings_candidate("EP.1.B1", "EP", Some("EP1"), true);
        let selected = select_drawings(&requested, &[]).unwrap();
        assert_eq!(selected.source_docdb_id, "EP.1.B1");
    }

    #[test]
    fn drawings_fall_back_to_the_same_application_then_family_priority() {
        let requested = drawings_candidate("EP.1.B1", "EP", Some("EP1"), false);
        let family = vec![
            drawings_candidate("EP.1.A1", "EP", Some("EP1"), true),
            drawings_candidate("WO.1.A1", "WO", Some("WO1"), true),
        ];
        let selected = select_drawings(&requested, &family).unwrap();
        assert_eq!(selected.source_docdb_id, "EP.1.A1", "same application should win over a family member");

        let requested_no_same_app = drawings_candidate("EP.1.B1", "EP", Some("EP1"), false);
        let family_only = vec![
            drawings_candidate("US.1.A1", "US", Some("US1"), true),
            drawings_candidate("WO.1.A1", "WO", Some("WO1"), true),
        ];
        let selected = select_drawings(&requested_no_same_app, &family_only).unwrap();
        assert_eq!(selected.source_docdb_id, "WO.1.A1");
    }

    #[test]
    fn drawings_none_when_no_publication_has_any() {
        let requested = drawings_candidate("EP.1.A1", "EP", Some("EP1"), false);
        let family = vec![drawings_candidate("US.1.A1", "US", Some("US1"), false)];
        assert_eq!(select_drawings(&requested, &family), None);
    }
}
