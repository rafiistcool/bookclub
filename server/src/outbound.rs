//! Outbound HTTP to Open Library and the DNB: one client, per-host throttles, cooldown after an
//! upstream 429, and one retry on 502/503/504 (docs/phase1/conventions.md §10.3).

use std::sync::Mutex;
use std::time::Duration;

use anyhow::Context;
use axum::http::{HeaderMap, StatusCode};
use bytes::Bytes;
use reqwest::header::RETRY_AFTER;
use tokio::time::Instant;
use url::Url;

use crate::config::Config;
use crate::error::AppError;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const READ_TIMEOUT: Duration = Duration::from_secs(15);
/// Cooldown after an upstream 429 without a usable `Retry-After`.
const DEFAULT_COOLDOWN: Duration = Duration::from_secs(60);
/// Longest `Retry-After` wait on a 503 (HAR).
const MAX_503_WAIT: Duration = Duration::from_secs(300);
/// How often a 503 with `Retry-After` is retried (HAR).
const MAX_503_RETRIES: u32 = 3;

/// Base URLs of the upstreams; tests point them at wiremock.
#[derive(Debug, Clone)]
pub struct UpstreamUrls {
    pub openlibrary: Url,
    pub openlibrary_covers: Url,
    pub dnb_sru: Url,
    pub dnb_oai: Url,
}

/// The shared outbound client and the per-host throttles.
pub struct Outbound {
    /// One client for connection reuse; connect 8 s, read 15 s; rustls.
    pub client: reqwest::Client,
    /// openlibrary.org: 3 requests per second.
    pub ol_api: HostThrottle,
    /// covers.openlibrary.org: 5 requests per second.
    pub ol_covers: HostThrottle,
    /// DNB SRU: 1 request per second.
    pub dnb_sru: HostThrottle,
    /// DNB OAI-PMH: 1 request per second.
    pub dnb_oai: HostThrottle,
    pub urls: UpstreamUrls,
}

impl Outbound {
    /// The production client: User-Agent `Bookclub/1.0 (+{PUBLIC_URL}; {CONTACT_EMAIL})`.
    pub fn new(config: &Config) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(user_agent(config))
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .build()
            .context("cannot build the outbound HTTP client")?;
        Ok(Self {
            client,
            ol_api: HostThrottle::per_second(3.0),
            ol_covers: HostThrottle::per_second(5.0),
            dnb_sru: HostThrottle::per_second(1.0),
            dnb_oai: HostThrottle::per_second(1.0),
            urls: UpstreamUrls {
                openlibrary: config.openlibrary_url.clone(),
                openlibrary_covers: config.openlibrary_covers_url.clone(),
                dnb_sru: config.dnb_sru_url.clone(),
                dnb_oai: config.dnb_oai_url.clone(),
            },
        })
    }
}

/// The User-Agent with contact details that Open Library and the DNB ask for.
pub fn user_agent(config: &Config) -> String {
    match &config.contact_email {
        Some(contact) => format!("Bookclub/1.0 (+{}; {contact})", config.public_url),
        None => format!("Bookclub/1.0 (+{})", config.public_url),
    }
}

/// The local throttle (or an upstream cooldown) refused the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Throttled;

/// A per-host rate limit: requests are spaced evenly, and an upstream 429 stops every request
/// to the host for the cooldown.
pub struct HostThrottle {
    interval: Duration,
    state: Mutex<ThrottleState>,
}

struct ThrottleState {
    /// The earliest moment the next request may start.
    next_slot: Instant,
    cooldown_until: Option<Instant>,
}

impl HostThrottle {
    /// At most `rate` requests per second, evenly spaced.
    pub fn per_second(rate: f64) -> Self {
        Self {
            interval: Duration::from_secs_f64(1.0 / rate),
            state: Mutex::new(ThrottleState {
                next_slot: Instant::now(),
                cooldown_until: None,
            }),
        }
    }

    /// Waits for a slot for up to `max_wait`, else `Err(Throttled)`. While a cooldown is active
    /// (after an upstream 429: `Retry-After` or 60 s), fails at once.
    pub async fn acquire(&self, max_wait: Duration) -> Result<(), Throttled> {
        let slot = {
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            let now = Instant::now();
            if state.cooldown_until.is_some_and(|until| until > now) {
                return Err(Throttled);
            }
            let slot = state.next_slot.max(now);
            if slot.duration_since(now) > max_wait {
                return Err(Throttled);
            }
            state.next_slot = slot + self.interval;
            slot
        };
        tokio::time::sleep_until(slot).await;
        Ok(())
    }

    /// Refuses every request for `duration` (extends, never shortens, a running cooldown).
    pub fn cool_down(&self, duration: Duration) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let until = Instant::now() + duration;
        state.cooldown_until = Some(
            state
                .cooldown_until
                .map_or(until, |current| current.max(until)),
        );
    }

    /// Time left in the current cooldown, for messages such as "Try again in 20 seconds."
    pub fn cooldown_remaining(&self) -> Option<Duration> {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        state
            .cooldown_until
            .filter(|until| *until > now)
            .map(|until| until.duration_since(now))
    }
}

/// How long to wait for the throttle and which failures to retry.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Throttle wait: 2 s on request paths, 10 s for `/media`, 60 s for jobs.
    pub max_wait: Duration,
    /// `Some(300 ms)`: one retry on 502/503/504.
    pub retry_5xx_once_after: Option<Duration>,
    /// HAR: on 503 wait `Retry-After` (capped at 5 min) and retry, up to 3 times.
    pub honour_retry_after_on_503: bool,
}

impl RetryPolicy {
    /// Request paths (search, lookups).
    pub const REQUEST: RetryPolicy = RetryPolicy {
        max_wait: Duration::from_secs(2),
        retry_5xx_once_after: Some(Duration::from_millis(300)),
        honour_retry_after_on_503: false,
    };
    /// `/media` cover fetches.
    pub const MEDIA: RetryPolicy = RetryPolicy {
        max_wait: Duration::from_secs(10),
        retry_5xx_once_after: Some(Duration::from_millis(300)),
        honour_retry_after_on_503: false,
    };
    /// Background jobs (harvester, prefetch).
    pub const JOB: RetryPolicy = RetryPolicy {
        max_wait: Duration::from_secs(60),
        retry_5xx_once_after: Some(Duration::from_millis(300)),
        honour_retry_after_on_503: true,
    };
}

/// A 2xx upstream response.
#[derive(Debug, Clone)]
pub struct Upstream {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

/// Why an outbound request gave no usable answer.
#[derive(Debug, thiserror::Error)]
pub enum UpstreamError {
    /// Local throttle, cooldown, or an upstream 429 (which starts a cooldown).
    #[error("throttled")]
    Throttled,
    /// Any other non-2xx status after retries (404 included).
    #[error("upstream status {0}")]
    Status(u16),
    /// Connect, timeout, TLS.
    #[error("transport: {0}")]
    Transport(String),
    /// The body is not the expected JSON or text.
    #[error("decode: {0}")]
    Decode(String),
}

impl UpstreamError {
    /// `Throttled` → 429 with `busy_msg`; `Status(404)` → 404 with `missing_msg` when given;
    /// everything else → 502 with `unavailable_msg`. Logs at WARN.
    pub fn to_app_error(
        &self,
        busy_msg: &'static str,
        missing_msg: Option<&'static str>,
        unavailable_msg: &'static str,
    ) -> AppError {
        tracing::warn!(error = %self, "upstream request failed");
        match (self, missing_msg) {
            (UpstreamError::Throttled, _) => AppError::too_many(busy_msg, None),
            (UpstreamError::Status(404), Some(missing)) => AppError::not_found(missing),
            _ => AppError::bad_gateway(unavailable_msg),
        }
    }
}

/// GET with the host throttle, cooldown and retry policy; a 2xx returns the raw response.
/// Used for XML (DNB SRU and OAI-PMH) and image bytes (Open Library covers).
pub async fn get_bytes(
    o: &Outbound,
    t: &HostThrottle,
    url: Url,
    policy: &RetryPolicy,
) -> Result<Upstream, UpstreamError> {
    let mut retried_5xx = false;
    let mut retries_503 = 0;
    loop {
        t.acquire(policy.max_wait)
            .await
            .map_err(|Throttled| UpstreamError::Throttled)?;
        let started = Instant::now();
        let response = match o.client.get(url.clone()).send().await {
            Ok(response) => response,
            Err(error) => {
                let latency_ms = started.elapsed().as_millis() as u64;
                tracing::warn!(url = %url, latency_ms, %error, "outbound request failed");
                return Err(UpstreamError::Transport(error.to_string()));
            }
        };
        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .bytes()
            .await
            .map_err(|error| UpstreamError::Transport(error.to_string()));
        let latency_ms = started.elapsed().as_millis() as u64;

        if status.is_success() {
            let body = body.inspect_err(|error| {
                tracing::warn!(url = %url, status = status.as_u16(), latency_ms, %error, "outbound body failed");
            })?;
            tracing::info!(url = %url, status = status.as_u16(), latency_ms, "outbound request");
            return Ok(Upstream {
                status,
                headers,
                body,
            });
        }
        tracing::warn!(url = %url, status = status.as_u16(), latency_ms, "outbound request failed");

        if status == StatusCode::TOO_MANY_REQUESTS {
            t.cool_down(retry_after(&headers).unwrap_or(DEFAULT_COOLDOWN));
            return Err(UpstreamError::Throttled);
        }
        if status == StatusCode::SERVICE_UNAVAILABLE
            && policy.honour_retry_after_on_503
            && retries_503 < MAX_503_RETRIES
            && let Some(wait) = retry_after(&headers)
        {
            retries_503 += 1;
            tokio::time::sleep(wait.min(MAX_503_WAIT)).await;
            continue;
        }
        if matches!(status.as_u16(), 502..=504)
            && !retried_5xx
            && let Some(pause) = policy.retry_5xx_once_after
        {
            retried_5xx = true;
            tokio::time::sleep(pause).await;
            continue;
        }
        return Err(UpstreamError::Status(status.as_u16()));
    }
}

/// [`get_bytes`], then the body as JSON.
pub async fn get_json(
    o: &Outbound,
    t: &HostThrottle,
    url: Url,
    policy: &RetryPolicy,
) -> Result<serde_json::Value, UpstreamError> {
    let upstream = get_bytes(o, t, url, policy).await?;
    serde_json::from_slice(&upstream.body).map_err(|error| UpstreamError::Decode(error.to_string()))
}

/// [`get_bytes`], then the body as UTF-8 text.
pub async fn get_text(
    o: &Outbound,
    t: &HostThrottle,
    url: Url,
    policy: &RetryPolicy,
) -> Result<String, UpstreamError> {
    let upstream = get_bytes(o, t, url, policy).await?;
    String::from_utf8(upstream.body.to_vec())
        .map_err(|error| UpstreamError::Decode(error.to_string()))
}

/// A positive integer `Retry-After` in seconds; anything else counts as absent.
fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn config(server: &MockServer) -> Config {
        let uri = server.uri();
        let pairs = [
            ("DATABASE_URL", "postgres://x".to_owned()),
            ("DEV_MODE", "true".to_owned()),
            ("PUBLIC_URL", "https://books.example.com".to_owned()),
            ("CONTACT_EMAIL", "ops@example.com".to_owned()),
            ("OPENLIBRARY_URL", uri.clone()),
            ("OPENLIBRARY_COVERS_URL", uri.clone()),
            ("DNB_SRU_URL", uri.clone()),
            ("DNB_OAI_URL", uri),
        ];
        Config::from_lookup(|key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone())
        })
        .unwrap()
    }

    /// An outbound client with fast throttles, so tests do not wait.
    fn outbound(server: &MockServer) -> Outbound {
        let mut outbound = Outbound::new(&config(server)).unwrap();
        outbound.ol_api = HostThrottle::per_second(1000.0);
        outbound
    }

    fn url(server: &MockServer, p: &str) -> Url {
        Url::parse(&format!("{}{p}", server.uri())).unwrap()
    }

    const FAST: RetryPolicy = RetryPolicy {
        max_wait: Duration::from_secs(2),
        retry_5xx_once_after: Some(Duration::from_millis(1)),
        honour_retry_after_on_503: false,
    };

    #[tokio::test]
    async fn sends_the_contact_user_agent() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/works/OL1W.json"))
            .and(header(
                "user-agent",
                "Bookclub/1.0 (+https://books.example.com; ops@example.com)",
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"title": "Circe"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let o = outbound(&server);
        let value = get_json(&o, &o.ol_api, url(&server, "/works/OL1W.json"), &FAST)
            .await
            .unwrap();
        assert_eq!(value["title"], "Circe");
    }

    #[tokio::test]
    async fn retries_a_5xx_once() {
        let server = MockServer::start().await;
        Mock::given(path("/flaky"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(path("/flaky"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;
        let o = outbound(&server);
        let text = get_text(&o, &o.ol_api, url(&server, "/flaky"), &FAST)
            .await
            .unwrap();
        assert_eq!(text, "ok");

        Mock::given(path("/down"))
            .respond_with(ResponseTemplate::new(502))
            .expect(2)
            .mount(&server)
            .await;
        let error = get_bytes(&o, &o.ol_api, url(&server, "/down"), &FAST)
            .await
            .unwrap_err();
        assert!(matches!(error, UpstreamError::Status(502)), "{error:?}");
    }

    #[tokio::test]
    async fn a_404_is_not_retried() {
        let server = MockServer::start().await;
        Mock::given(path("/missing"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;
        let o = outbound(&server);
        let error = get_bytes(&o, &o.ol_api, url(&server, "/missing"), &FAST)
            .await
            .unwrap_err();
        assert!(matches!(error, UpstreamError::Status(404)));
        let app_error = error.to_app_error("busy", Some("No such book."), "down");
        assert_eq!(app_error.status, StatusCode::NOT_FOUND);
        assert_eq!(app_error.detail, "No such book.");
        let app_error = error.to_app_error("busy", None, "down");
        assert_eq!(app_error.status, StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn a_429_starts_a_cooldown_that_blocks_the_host() {
        let server = MockServer::start().await;
        Mock::given(path("/busy"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "20"))
            .expect(1)
            .mount(&server)
            .await;
        let o = outbound(&server);
        let error = get_bytes(&o, &o.ol_api, url(&server, "/busy"), &FAST)
            .await
            .unwrap_err();
        assert!(matches!(error, UpstreamError::Throttled));
        let remaining = o.ol_api.cooldown_remaining().unwrap();
        assert!(remaining > Duration::from_secs(19) && remaining <= Duration::from_secs(20));
        // The cooldown answers at once, without a request (expect(1) above).
        let error = get_bytes(&o, &o.ol_api, url(&server, "/busy"), &FAST)
            .await
            .unwrap_err();
        assert!(matches!(error, UpstreamError::Throttled));
        let app_error = error.to_app_error("The library is busy.", None, "down");
        assert_eq!(app_error.status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(app_error.detail, "The library is busy.");
        // Other hosts are not affected.
        assert!(o.ol_covers.cooldown_remaining().is_none());
    }

    #[tokio::test]
    async fn a_429_without_retry_after_cools_down_for_a_minute() {
        let server = MockServer::start().await;
        Mock::given(path("/busy"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;
        let o = outbound(&server);
        let _ = get_bytes(&o, &o.ol_api, url(&server, "/busy"), &FAST).await;
        let remaining = o.ol_api.cooldown_remaining().unwrap();
        assert!(remaining > Duration::from_secs(59));
    }

    #[tokio::test]
    async fn jobs_wait_out_a_503_retry_after() {
        let server = MockServer::start().await;
        Mock::given(path("/oai"))
            .respond_with(ResponseTemplate::new(503).insert_header("retry-after", "1"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(path("/oai"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<OAI-PMH/>"))
            .mount(&server)
            .await;
        let o = outbound(&server);
        let policy = RetryPolicy {
            retry_5xx_once_after: None,
            ..RetryPolicy::JOB
        };
        let started = Instant::now();
        let text = get_text(&o, &o.ol_api, url(&server, "/oai"), &policy)
            .await
            .unwrap();
        assert_eq!(text, "<OAI-PMH/>");
        assert!(started.elapsed() >= Duration::from_millis(900));
    }

    #[tokio::test]
    async fn undecodable_bodies_are_decode_errors() {
        let server = MockServer::start().await;
        Mock::given(path("/garbage"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0xff, 0xfe]))
            .mount(&server)
            .await;
        let o = outbound(&server);
        let error = get_json(&o, &o.ol_api, url(&server, "/garbage"), &FAST)
            .await
            .unwrap_err();
        assert!(matches!(error, UpstreamError::Decode(_)));
        let error = get_text(&o, &o.ol_api, url(&server, "/garbage"), &FAST)
            .await
            .unwrap_err();
        assert!(matches!(error, UpstreamError::Decode(_)));
        assert_eq!(
            error.to_app_error("busy", None, "down").status,
            StatusCode::BAD_GATEWAY
        );
    }

    #[tokio::test]
    async fn transport_errors_are_reported() {
        let server = MockServer::start().await;
        let o = outbound(&server);
        let dead = Url::parse("http://127.0.0.1:1/").unwrap();
        let error = get_bytes(&o, &o.ol_api, dead, &FAST).await.unwrap_err();
        assert!(matches!(error, UpstreamError::Transport(_)), "{error:?}");
    }

    #[tokio::test]
    async fn the_throttle_spaces_requests_and_gives_up_after_max_wait() {
        let throttle = HostThrottle::per_second(20.0);
        let started = Instant::now();
        throttle.acquire(Duration::from_secs(1)).await.unwrap();
        throttle.acquire(Duration::from_secs(1)).await.unwrap();
        throttle.acquire(Duration::from_secs(1)).await.unwrap();
        assert!(started.elapsed() >= Duration::from_millis(90));

        let slow = HostThrottle::per_second(1.0);
        slow.acquire(Duration::ZERO).await.unwrap();
        assert_eq!(
            slow.acquire(Duration::from_millis(10)).await,
            Err(Throttled)
        );
    }

    #[tokio::test]
    async fn a_cooldown_fails_at_once_and_never_shrinks() {
        let throttle = HostThrottle::per_second(1000.0);
        throttle.cool_down(Duration::from_secs(30));
        throttle.cool_down(Duration::from_secs(5));
        assert!(throttle.cooldown_remaining().unwrap() > Duration::from_secs(29));
        assert_eq!(
            throttle.acquire(Duration::from_secs(60)).await,
            Err(Throttled)
        );
    }

    #[test]
    fn user_agent_without_contact_in_dev() {
        let pairs = [("DATABASE_URL", "postgres://x"), ("DEV_MODE", "true")];
        let config = Config::from_lookup(|key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        })
        .unwrap();
        assert_eq!(user_agent(&config), "Bookclub/1.0 (+http://localhost:5173)");
    }
}
