#[derive(Debug, thiserror::Error)]
pub enum OpsError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("OPS returned {status}: {message}")]
    Api { status: u16, message: String },
    #[error("could not parse OPS response: {0}")]
    Parse(String),
    #[error("OPS service is overloaded or blocked (state: {0}); stopping")]
    Blocked(String),
    #[error("OPS quota exhausted")]
    QuotaExhausted,
}

/// OPS error responses are `<error><code>...</code><message>...</message></error>`
/// (verified against the live host, see docs/DECISIONS.md). Falls back to the
/// raw body when it doesn't match that shape.
pub(crate) fn from_error_body(status: u16, body: &str) -> OpsError {
    let message = roxmltree::Document::parse(body)
        .ok()
        .and_then(|doc| {
            doc.descendants()
                .find(|n| n.has_tag_name("message"))
                .and_then(|n| n.text())
                .map(str::trim)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.trim().to_string());
    OpsError::Api { status, message }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_message_from_error_xml() {
        let err = from_error_body(
            401,
            "<error><code>401</code><message>ClientId is Invalid</message></error>",
        );
        match err {
            OpsError::Api { status, message } => {
                assert_eq!(status, 401);
                assert_eq!(message, "ClientId is Invalid");
            }
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    #[test]
    fn falls_back_to_raw_body_when_not_xml() {
        let err = from_error_body(500, "internal server error");
        match err {
            OpsError::Api { status, message } => {
                assert_eq!(status, 500);
                assert_eq!(message, "internal server error");
            }
            other => panic!("expected Api error, got {other:?}"),
        }
    }
}
