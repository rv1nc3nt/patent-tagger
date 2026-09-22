//! OAuth2 client-credentials authentication (SPEC 5.4): `POST
//! {base_url}/auth/accesstoken` with HTTP Basic auth, caching the token and
//! renewing it before its ~20 minute expiry or on an invalid-token response.

use crate::error::{from_error_body, OpsError};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Renew this long before the token's stated expiry, so a request started
/// just before expiry doesn't race it.
const EXPIRY_SAFETY_MARGIN: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
struct Token {
    access_token: String,
    expires_at: Instant,
}

impl Token {
    fn is_valid(&self) -> bool {
        Instant::now() < self.expires_at
    }
}

/// Caches the current access token, fetching a new one on first use, on
/// expiry, or after an explicit [`TokenCache::invalidate`] (called when a
/// request comes back with an invalid-token error, per SPEC 5.4).
pub struct TokenCache {
    current: Mutex<Option<Token>>,
}

impl Default for TokenCache {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenCache {
    pub fn new() -> Self {
        Self {
            current: Mutex::new(None),
        }
    }

    pub async fn invalidate(&self) {
        *self.current.lock().await = None;
    }

    /// Returns a valid access token, fetching or renewing it as needed.
    /// `auth_url` is the full token-endpoint URL (not a base to append to —
    /// it is a sibling of the REST services base, not nested under it).
    pub async fn get(
        &self,
        http: &reqwest::Client,
        auth_url: &str,
        consumer_key: &str,
        consumer_secret: &str,
    ) -> Result<String, OpsError> {
        let mut guard = self.current.lock().await;
        if let Some(token) = guard.as_ref() {
            if token.is_valid() {
                return Ok(token.access_token.clone());
            }
        }
        let token = fetch_token(http, auth_url, consumer_key, consumer_secret).await?;
        let access_token = token.access_token.clone();
        *guard = Some(token);
        Ok(access_token)
    }
}

async fn fetch_token(
    http: &reqwest::Client,
    auth_url: &str,
    consumer_key: &str,
    consumer_secret: &str,
) -> Result<Token, OpsError> {
    let response = http
        .post(auth_url)
        .basic_auth(consumer_key, Some(consumer_secret))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("grant_type=client_credentials")
        .send()
        .await?;

    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(from_error_body(status.as_u16(), &body));
    }
    parse_token_response(&body)
}

#[derive(serde::Deserialize)]
struct TokenResponseRaw {
    access_token: String,
    #[serde(deserialize_with = "expires_in_as_u64")]
    expires_in: u64,
}

fn expires_in_as_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;

    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum StrOrNum {
        Str(String),
        Num(u64),
    }
    match StrOrNum::deserialize(deserializer)? {
        StrOrNum::Str(s) => s.parse().map_err(serde::de::Error::custom),
        StrOrNum::Num(n) => Ok(n),
    }
}

fn parse_token_response(body: &str) -> Result<Token, OpsError> {
    let raw: TokenResponseRaw =
        serde_json::from_str(body).map_err(|e| OpsError::Parse(format!("token response: {e}")))?;
    let ttl = Duration::from_secs(raw.expires_in).saturating_sub(EXPIRY_SAFETY_MARGIN);
    Ok(Token {
        access_token: raw.access_token,
        expires_at: Instant::now() + ttl,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_token_response_with_string_expires_in() {
        let token =
            parse_token_response(r#"{"access_token":"abc123","expires_in":"1199","token_type":"Bearer"}"#)
                .expect("should parse");
        assert_eq!(token.access_token, "abc123");
        assert!(token.is_valid());
    }

    #[test]
    fn parses_token_response_with_numeric_expires_in() {
        let token = parse_token_response(r#"{"access_token":"abc123","expires_in":1199}"#)
            .expect("should parse");
        assert_eq!(token.access_token, "abc123");
    }

    #[test]
    fn a_token_with_expiry_already_inside_the_safety_margin_is_not_valid() {
        let token = parse_token_response(r#"{"access_token":"abc123","expires_in":"30"}"#)
            .expect("should parse");
        assert!(!token.is_valid());
    }

    #[tokio::test]
    async fn cache_returns_cached_token_without_refetching() {
        let cache = TokenCache::new();
        *cache.current.lock().await = Some(Token {
            access_token: "cached-token".to_string(),
            expires_at: Instant::now() + Duration::from_secs(600),
        });

        // A request would fail (no real server), but we never get there
        // because the cached token is still valid.
        let http = reqwest::Client::new();
        let token = cache
            .get(&http, "http://127.0.0.1:0", "key", "secret")
            .await
            .expect("cached token should be returned without a network call");
        assert_eq!(token, "cached-token");
    }

    #[tokio::test]
    async fn invalidate_clears_the_cached_token() {
        let cache = TokenCache::new();
        *cache.current.lock().await = Some(Token {
            access_token: "cached-token".to_string(),
            expires_at: Instant::now() + Duration::from_secs(600),
        });
        cache.invalidate().await;
        assert!(cache.current.lock().await.is_none());
    }
}
