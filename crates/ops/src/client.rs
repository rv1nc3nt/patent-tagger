//! Ties authentication, throttling and retry together into the requests
//! the rest of the crate issues (SPEC 5.4). At most 2 concurrent requests,
//! enforced with a semaphore.

use crate::auth::TokenCache;
use crate::error::{from_error_body, OpsError};
use crate::retry::{backoff_delay, should_retry, MAX_ATTEMPTS};
use crate::throttle::{self, Action, ThrottleControl};
use rand::RngExt;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::Semaphore;

const MAX_CONCURRENT_REQUESTS: usize = 2;
const BACKOFF_BASE: Duration = Duration::from_millis(500);
const BACKOFF_CAP: Duration = Duration::from_secs(30);

/// The REST services base (verified against the live host; SPEC 5.4). Note
/// this is *not* an ancestor of the auth URL below — they're siblings under
/// `/3.2/`, not nested.
pub const OPS_REST_BASE_URL: &str = "https://ops.epo.org/3.2/rest-services";
/// The OAuth2 token endpoint (verified against the live host; SPEC 5.4).
pub const OPS_AUTH_URL: &str = "https://ops.epo.org/3.2/auth/accesstoken";

pub struct OpsClient {
    http: reqwest::Client,
    rest_base_url: String,
    auth_url: String,
    consumer_key: String,
    consumer_secret: String,
    tokens: TokenCache,
    concurrency: Semaphore,
    /// The most recently observed throttling state, consulted before each
    /// request so a red/black category is honoured even for the very next
    /// call, not just retroactively.
    last_throttle: Mutex<Option<ThrottleControl>>,
}

impl OpsClient {
    pub fn new(consumer_key: String, consumer_secret: String) -> Self {
        Self::with_urls(OPS_REST_BASE_URL, OPS_AUTH_URL, consumer_key, consumer_secret)
    }

    /// For tests or an alternate deployment; production code should use
    /// [`OpsClient::new`].
    pub fn with_urls(
        rest_base_url: impl Into<String>,
        auth_url: impl Into<String>,
        consumer_key: String,
        consumer_secret: String,
    ) -> Self {
        Self {
            http: reqwest::Client::new(),
            rest_base_url: rest_base_url.into(),
            auth_url: auth_url.into(),
            consumer_key,
            consumer_secret,
            tokens: TokenCache::new(),
            concurrency: Semaphore::new(MAX_CONCURRENT_REQUESTS),
            last_throttle: Mutex::new(None),
        }
    }

    /// `GET {base_url}{path}`, authenticated, throttle-aware and retried on
    /// 5xx/network errors. `path` starts with `/`, e.g.
    /// `/published-data/publication/docdb/EP.1234567.A1/biblio,abstract`.
    pub async fn get(&self, path: &str) -> Result<String, OpsError> {
        self.wait_for_throttle_clearance().await?;
        let _permit = self
            .concurrency
            .acquire()
            .await
            .map_err(|_| OpsError::Parse("concurrency semaphore closed".to_string()))?;

        let mut last_err = None;
        for attempt in 0..MAX_ATTEMPTS {
            match self.try_get(path).await {
                Ok(body) => return Ok(body),
                Err(err) => {
                    let status = match &err {
                        OpsError::Api { status, .. } => Some(*status),
                        _ => None,
                    };
                    if !should_retry(status) || attempt + 1 == MAX_ATTEMPTS {
                        return Err(err);
                    }
                    last_err = Some(err);
                    let jitter: f64 = rand::rng().random();
                    let delay = backoff_delay(attempt, BACKOFF_BASE, BACKOFF_CAP, jitter);
                    tokio::time::sleep(delay).await;
                }
            }
        }
        Err(last_err.unwrap_or(OpsError::Parse("retry loop exited without a result".to_string())))
    }

    async fn try_get(&self, path: &str) -> Result<String, OpsError> {
        let token = self
            .tokens
            .get(&self.http, &self.auth_url, &self.consumer_key, &self.consumer_secret)
            .await?;

        let response = self
            .http
            .get(format!("{}{path}", self.rest_base_url))
            .bearer_auth(&token)
            .header("Accept", "application/xml")
            .send()
            .await?;

        if let Some(header) = response
            .headers()
            .get("X-Throttling-Control")
            .and_then(|v| v.to_str().ok())
        {
            if let Ok(parsed) = throttle::parse(header) {
                *self.last_throttle.lock().expect("throttle mutex poisoned") = Some(parsed);
            }
        }

        let status = response.status();
        let body = response.text().await?;

        if status == reqwest::StatusCode::UNAUTHORIZED {
            // Per SPEC 5.4: renew on an invalid-token response, then let the
            // retry loop's next attempt fetch a fresh one.
            self.tokens.invalidate().await;
            return Err(from_error_body(status.as_u16(), &body));
        }
        if !status.is_success() {
            return Err(from_error_body(status.as_u16(), &body));
        }
        Ok(body)
    }

    /// `GET {base_url}{path}` with an extra request header (SPEC 5.4's
    /// `X-OPS-Range` for a specific drawing page - verified against the
    /// live host, see docs/DECISIONS.md), returning the raw response body
    /// bytes (a TIFF image) rather than assuming XML text. Same auth/
    /// throttle/retry behaviour as [`OpsClient::get`].
    pub async fn get_bytes(&self, path: &str, extra_header: (&str, &str)) -> Result<Vec<u8>, OpsError> {
        self.wait_for_throttle_clearance().await?;
        let _permit = self
            .concurrency
            .acquire()
            .await
            .map_err(|_| OpsError::Parse("concurrency semaphore closed".to_string()))?;

        let mut last_err = None;
        for attempt in 0..MAX_ATTEMPTS {
            match self.try_get_bytes(path, extra_header).await {
                Ok(body) => return Ok(body),
                Err(err) => {
                    let status = match &err {
                        OpsError::Api { status, .. } => Some(*status),
                        _ => None,
                    };
                    if !should_retry(status) || attempt + 1 == MAX_ATTEMPTS {
                        return Err(err);
                    }
                    last_err = Some(err);
                    let jitter: f64 = rand::rng().random();
                    let delay = backoff_delay(attempt, BACKOFF_BASE, BACKOFF_CAP, jitter);
                    tokio::time::sleep(delay).await;
                }
            }
        }
        Err(last_err.unwrap_or(OpsError::Parse("retry loop exited without a result".to_string())))
    }

    async fn try_get_bytes(&self, path: &str, extra_header: (&str, &str)) -> Result<Vec<u8>, OpsError> {
        let token = self
            .tokens
            .get(&self.http, &self.auth_url, &self.consumer_key, &self.consumer_secret)
            .await?;

        let response = self
            .http
            .get(format!("{}{path}", self.rest_base_url))
            .bearer_auth(&token)
            .header("Accept", "application/tiff")
            .header(extra_header.0, extra_header.1)
            .send()
            .await?;

        if let Some(header) = response.headers().get("X-Throttling-Control").and_then(|v| v.to_str().ok()) {
            if let Ok(parsed) = throttle::parse(header) {
                *self.last_throttle.lock().expect("throttle mutex poisoned") = Some(parsed);
            }
        }

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            self.tokens.invalidate().await;
            let body = response.text().await.unwrap_or_default();
            return Err(from_error_body(status.as_u16(), &body));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(from_error_body(status.as_u16(), &body));
        }
        Ok(response.bytes().await?.to_vec())
    }

    /// Blocks until the last observed throttle state allows another
    /// request: no delay on green, a short sleep on yellow, a longer one on
    /// red, and an immediate error on black (SPEC 5.4: "stop and report").
    async fn wait_for_throttle_clearance(&self) -> Result<(), OpsError> {
        let action_and_delay = {
            let guard = self.last_throttle.lock().expect("throttle mutex poisoned");
            guard.as_ref().map(|tc| (tc.strictest_action(), tc.recommended_delay()))
        };
        match action_and_delay {
            Some((Action::Stop, _)) => Err(OpsError::Blocked("black".to_string())),
            Some((_, Some(delay))) => {
                tokio::time::sleep(delay).await;
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
