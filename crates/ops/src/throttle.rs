//! Parsing and interpretation of the `X-Throttling-Control` response header
//! (SPEC section 5.4): overall service state plus a colour per category,
//! e.g. `idle (retrieval=green:200, search=yellow:20, inpadoc=red:30,
//! images=green:200, other=green:1000)`.

use crate::error::OpsError;
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    Idle,
    Busy,
    Overloaded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colour {
    Green,
    Yellow,
    Red,
    Black,
}

/// What a caller should do before its next request to a category at this
/// colour, per SPEC 5.4: "normal on green, slow down on yellow, pause on
/// red, stop and report on black".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Continue,
    SlowDown,
    Pause,
    Stop,
}

impl Colour {
    pub fn action(self) -> Action {
        match self {
            Colour::Green => Action::Continue,
            Colour::Yellow => Action::SlowDown,
            Colour::Red => Action::Pause,
            Colour::Black => Action::Stop,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CategoryStatus {
    pub colour: Colour,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThrottleControl {
    pub state: ServiceState,
    pub categories: BTreeMap<String, CategoryStatus>,
}

impl ThrottleControl {
    /// The most restrictive action across every reported category, since a
    /// single client should honour the strictest one that applies to any
    /// service it might call next.
    pub fn strictest_action(&self) -> Action {
        self.categories
            .values()
            .map(|c| c.colour.action())
            .max_by_key(|a| match a {
                Action::Continue => 0,
                Action::SlowDown => 1,
                Action::Pause => 2,
                Action::Stop => 3,
            })
            .unwrap_or(Action::Continue)
    }

    /// A conservative pause duration for [`Action::SlowDown`] or
    /// [`Action::Pause`]. The SPEC doesn't mandate an exact value; these are
    /// deliberately generous so a misbehaving client backs off rather than
    /// hammering a throttled service.
    pub fn recommended_delay(&self) -> Option<Duration> {
        match self.strictest_action() {
            Action::Continue => None,
            Action::SlowDown => Some(Duration::from_secs(2)),
            Action::Pause => Some(Duration::from_secs(30)),
            Action::Stop => None,
        }
    }
}

pub fn parse(header_value: &str) -> Result<ThrottleControl, OpsError> {
    let value = header_value.trim();
    let open = value
        .find('(')
        .ok_or_else(|| OpsError::Parse(format!("no '(' in throttling header: {value}")))?;
    let close = value
        .rfind(')')
        .ok_or_else(|| OpsError::Parse(format!("no ')' in throttling header: {value}")))?;
    if close < open {
        return Err(OpsError::Parse(format!(
            "malformed throttling header: {value}"
        )));
    }

    let state = match value[..open].trim() {
        "idle" => ServiceState::Idle,
        "busy" => ServiceState::Busy,
        "overloaded" => ServiceState::Overloaded,
        other => return Err(OpsError::Parse(format!("unknown service state: {other}"))),
    };

    let mut categories = BTreeMap::new();
    for entry in value[open + 1..close].split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let (name, rest) = entry
            .split_once('=')
            .ok_or_else(|| OpsError::Parse(format!("malformed category entry: {entry}")))?;
        let (colour, limit) = rest
            .split_once(':')
            .ok_or_else(|| OpsError::Parse(format!("malformed category entry: {entry}")))?;
        let colour = match colour.trim() {
            "green" => Colour::Green,
            "yellow" => Colour::Yellow,
            "red" => Colour::Red,
            "black" => Colour::Black,
            other => return Err(OpsError::Parse(format!("unknown colour: {other}"))),
        };
        let limit: u32 = limit
            .trim()
            .parse()
            .map_err(|_| OpsError::Parse(format!("non-numeric limit in: {entry}")))?;
        categories.insert(name.trim().to_string(), CategoryStatus { colour, limit });
    }

    Ok(ThrottleControl { state, categories })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_documented_example() {
        let tc =
            parse("idle (retrieval=green:200, search=yellow:20, inpadoc=red:30, images=green:200, other=green:1000)")
                .expect("should parse");
        assert_eq!(tc.state, ServiceState::Idle);
        assert_eq!(tc.categories.len(), 5);
        assert_eq!(
            tc.categories["retrieval"],
            CategoryStatus {
                colour: Colour::Green,
                limit: 200
            }
        );
        assert_eq!(
            tc.categories["search"],
            CategoryStatus {
                colour: Colour::Yellow,
                limit: 20
            }
        );
        assert_eq!(
            tc.categories["inpadoc"],
            CategoryStatus {
                colour: Colour::Red,
                limit: 30
            }
        );
    }

    #[test]
    fn strictest_action_is_the_worst_colour_present() {
        let tc = parse("busy (retrieval=green:200, search=red:5)").expect("should parse");
        assert_eq!(tc.strictest_action(), Action::Pause);
    }

    #[test]
    fn black_means_stop() {
        let tc = parse("overloaded (retrieval=black:0)").expect("should parse");
        assert_eq!(tc.strictest_action(), Action::Stop);
        assert_eq!(tc.recommended_delay(), None);
    }

    #[test]
    fn all_green_means_continue_with_no_delay() {
        let tc = parse("idle (retrieval=green:200)").expect("should parse");
        assert_eq!(tc.strictest_action(), Action::Continue);
        assert_eq!(tc.recommended_delay(), None);
    }

    #[test]
    fn rejects_malformed_input() {
        assert!(parse("not a valid header").is_err());
        assert!(parse("idle (retrieval)").is_err());
        assert!(parse("idle (retrieval=purple:1)").is_err());
    }
}
