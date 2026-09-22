//! Local parsing of patent/publication numbers (SPEC section 5.1-5.2).
//!
//! The parser handles common EP, WO and US forms locally. Anything else that
//! still looks like a publication number is left for the OPS number-service
//! to normalise (section 5.2); anything that doesn't look like a number at
//! all is reported as unparseable.

const SEPARATORS: [char; 4] = [' ', '.', '/', ','];
const LOCALLY_HANDLED_COUNTRIES: [&str; 3] = ["EP", "US", "WO"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedNumber {
    pub country: String,
    pub number: String,
    pub kind_code: Option<String>,
}

impl ParsedNumber {
    /// Country code plus number, without kind code (SPEC section 4.2 `pub_key`).
    pub fn pub_key(&self) -> String {
        format!("{}{}", self.country, self.number)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseOutcome {
    Parsed(ParsedNumber),
    /// Looks like a publication number but not one of the forms this local
    /// parser confidently handles; needs the OPS number-service (M2).
    NeedsNormalisation(String),
    /// Does not look like a publication number at all.
    Unparseable(String),
}

pub fn parse(raw: &str) -> ParseOutcome {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ParseOutcome::Unparseable(trimmed.to_string());
    }

    let country_len = trimmed
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .count();
    if country_len == 0 {
        return ParseOutcome::Unparseable(trimmed.to_string());
    }
    let country = trimmed[..country_len].to_ascii_uppercase();
    let rest = &trimmed[country_len..];

    let compact: String = rest.chars().filter(|c| !SEPARATORS.contains(c)).collect();
    if compact.is_empty() || !compact.chars().all(|c| c.is_ascii_alphanumeric()) {
        return ParseOutcome::Unparseable(trimmed.to_string());
    }

    let Some((number, kind_code)) = split_number_and_kind_code(&compact) else {
        return ParseOutcome::Unparseable(trimmed.to_string());
    };

    if !LOCALLY_HANDLED_COUNTRIES.contains(&country.as_str()) {
        return ParseOutcome::NeedsNormalisation(trimmed.to_string());
    }

    ParseOutcome::Parsed(ParsedNumber {
        country,
        number,
        kind_code,
    })
}

/// Splits a compact (separator-free) number+kind-code string, e.g.
/// `"1234567B1"` -> `("1234567", Some("B1"))`, `"1234567"` -> `("1234567", None)`.
///
/// A kind code is exactly one letter followed by zero or more digits, at the
/// very end of the string, immediately after an all-digit number. Returns
/// `None` when the string doesn't fit that shape (e.g. digits interleaved
/// with letters elsewhere).
fn split_number_and_kind_code(compact: &str) -> Option<(String, Option<String>)> {
    let trailing_digits_start = compact
        .char_indices()
        .rev()
        .find(|(_, c)| !c.is_ascii_digit())
        .map_or(0, |(i, c)| i + c.len_utf8());
    let (before, trailing_digits) = compact.split_at(trailing_digits_start);

    if before.is_empty() {
        return if trailing_digits.is_empty() {
            None
        } else {
            Some((trailing_digits.to_string(), None))
        };
    }

    let last_char = before.chars().next_back()?;
    if !last_char.is_ascii_alphabetic() {
        return None;
    }
    let letter_start = before.len() - last_char.len_utf8();
    let digits_before = &before[..letter_start];
    if digits_before.is_empty() || !digits_before.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    let kind_code = format!("{}{trailing_digits}", &before[letter_start..]).to_ascii_uppercase();
    Some((digits_before.to_string(), Some(kind_code)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(country: &str, number: &str, kind_code: Option<&str>) -> ParseOutcome {
        ParseOutcome::Parsed(ParsedNumber {
            country: country.to_string(),
            number: number.to_string(),
            kind_code: kind_code.map(str::to_string),
        })
    }

    #[test]
    fn ep_with_spaces_and_kind_code() {
        assert_eq!(
            parse("EP 1 234 567 B1"),
            parsed("EP", "1234567", Some("B1"))
        );
    }

    #[test]
    fn ep_compact_without_kind_code() {
        assert_eq!(parse("EP1234567"), parsed("EP", "1234567", None));
    }

    #[test]
    fn us_pre_grant_publication_with_slash() {
        assert_eq!(
            parse("US 2020/0123456 A1"),
            parsed("US", "20200123456", Some("A1"))
        );
    }

    #[test]
    fn us_grant_compact() {
        assert_eq!(parse("US11000000B2"), parsed("US", "11000000", Some("B2")));
    }

    #[test]
    fn wo_compact_with_slash_no_kind_code() {
        assert_eq!(parse("WO2019/123456"), parsed("WO", "2019123456", None));
    }

    #[test]
    fn wo_with_spaces_and_kind_code() {
        assert_eq!(
            parse("WO 2019123456 A1"),
            parsed("WO", "2019123456", Some("A1"))
        );
    }

    #[test]
    fn tolerates_dots_and_commas() {
        assert_eq!(
            parse("EP.1.234.567.B1"),
            parsed("EP", "1234567", Some("B1"))
        );
        assert_eq!(
            parse("EP,1,234,567,B1"),
            parsed("EP", "1234567", Some("B1"))
        );
    }

    #[test]
    fn lowercase_country_and_kind_code_are_normalised() {
        assert_eq!(parse("ep1234567b1"), parsed("EP", "1234567", Some("B1")));
    }

    #[test]
    fn unknown_country_needs_ops_normalisation() {
        assert_eq!(
            parse("GB2345678A"),
            ParseOutcome::NeedsNormalisation("GB2345678A".to_string())
        );
    }

    #[test]
    fn empty_input_is_unparseable() {
        assert_eq!(parse(""), ParseOutcome::Unparseable("".to_string()));
        assert_eq!(parse("   "), ParseOutcome::Unparseable("".to_string()));
    }

    #[test]
    fn garbage_is_unparseable() {
        assert_eq!(
            parse("not a number"),
            ParseOutcome::Unparseable("not a number".to_string())
        );
        assert_eq!(
            parse("EP-!?"),
            ParseOutcome::Unparseable("EP-!?".to_string())
        );
    }

    #[test]
    fn pub_key_drops_kind_code() {
        let parsed = ParsedNumber {
            country: "EP".to_string(),
            number: "1234567".to_string(),
            kind_code: Some("B1".to_string()),
        };
        assert_eq!(parsed.pub_key(), "EP1234567");
    }
}
