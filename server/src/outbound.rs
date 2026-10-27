//! Outbound HTTP to Open Library and the DNB: one client, per-host throttles, cooldown after an
//! upstream 429, one retry on 502/503/504, checked redirects and capped bodies
//! (docs/phase1/conventions.md §10.3).

use std::sync::Mutex;
use std::time::Duration;

use anyhow::Context;
use axum::http::{HeaderMap, StatusCode};
use bytes::{Bytes, BytesMut};
use reqwest::header::{LOCATION, RETRY_AFTER};
use tokio::time::Instant;
use url::{Host, Url};

use crate::config::Config;
use crate::error::AppError;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const READ_TIMEOUT: Duration = Duration::from_secs(15);
/// Cooldown after an upstream 429 without a usable `Retry-After`.
const DEFAULT_COOLDOWN: Duration = Duration::from_secs(60);
/// Longest cooldown, whatever an upstream's `Retry-After` asks for.
const MAX_COOLDOWN: Duration = Duration::from_secs(3600);
/// Longest `Retry-After` wait on a 503 (HAR).
const MAX_503_WAIT: Duration = Duration::from_secs(300);
/// How often a 503 with `Retry-After` is retried (HAR).
const MAX_503_RETRIES: u32 = 3;
/// Redirects one request may follow. An Open Library cover takes two: archive.org, then the
/// storage node that holds the image.
const MAX_REDIRECTS: usize = 3;
/// Domains a redirect may lead to besides the configured upstreams (and their subdomains):
/// Open Library serves cover images from the Internet Archive.
const REDIRECT_DOMAINS: [&str; 1] = ["archive.org"];

/// Base URLs of the upstreams; tests point them at wiremock.
#[derive(Debug, Clone)]
pub struct UpstreamUrls {
    pub openlibrary: Url,
    pub openlibrary_covers: Url,
    pub dnb_sru: Url,
    pub dnb_oai: Url,
}

impl UpstreamUrls {
    fn all(&self) -> [&Url; 4] {
        [
            &self.openlibrary,
            &self.openlibrary_covers,
            &self.dnb_sru,
            &self.dnb_oai,
        ]
    }
}

/// The shared outbound client and the per-host throttles.
pub struct Outbound {
    /// One client for connection reuse; connect 8 s, read 15 s; rustls; follows no redirects
    /// itself ([`get_bytes`] checks each one).
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
            .redirect(reqwest::redirect::Policy::none())
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

    /// Refuses every request for `duration`, capped at one hour (extends, never shortens, a
    /// running cooldown). The cap keeps an absurd upstream `Retry-After` from blocking the host
    /// until the next restart.
    pub fn cool_down(&self, duration: Duration) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let until = Instant::now() + duration.min(MAX_COOLDOWN);
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

/// How long to wait for the throttle, which failures to retry, and how much to accept.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Throttle wait: 2 s on request paths, 10 s for `/media`, 60 s for jobs.
    pub max_wait: Duration,
    /// `Some(300 ms)`: one retry on 502/503/504.
    pub retry_5xx_once_after: Option<Duration>,
    /// HAR: on 503 wait `Retry-After` (capped at 5 min) and retry, up to 3 times.
    pub honour_retry_after_on_503: bool,
    /// Deadline for one attempt: the request, its redirects and the whole body. A slow-drip
    /// upstream cannot hold a call open past it.
    pub attempt_timeout: Duration,
    /// A 2xx body is read until it passes this many bytes, then refused
    /// ([`UpstreamError::TooLarge`]).
    pub max_body_bytes: usize,
}

impl RetryPolicy {
    /// Request paths (search, lookups): Open Library JSON and DNB SRU pages.
    pub const REQUEST: RetryPolicy = RetryPolicy {
        max_wait: Duration::from_secs(2),
        retry_5xx_once_after: Some(Duration::from_millis(300)),
        honour_retry_after_on_503: false,
        attempt_timeout: Duration::from_secs(15),
        max_body_bytes: 4 * 1024 * 1024,
    };
    /// `/media` cover fetches: 1.5 MB at most (spec/shelf.md §2.8). The prefetch worker uses it
    /// too, with a longer `max_wait`.
    pub const MEDIA: RetryPolicy = RetryPolicy {
        max_wait: Duration::from_secs(10),
        retry_5xx_once_after: Some(Duration::from_millis(300)),
        honour_retry_after_on_503: false,
        attempt_timeout: Duration::from_secs(15),
        max_body_bytes: 1_500_000,
    };
    /// Background jobs (the harvester's OAI-PMH pages).
    pub const JOB: RetryPolicy = RetryPolicy {
        max_wait: Duration::from_secs(60),
        retry_5xx_once_after: Some(Duration::from_millis(300)),
        honour_retry_after_on_503: true,
        attempt_timeout: Duration::from_secs(60),
        max_body_bytes: 32 * 1024 * 1024,
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
    /// Connect, timeout (including the attempt deadline), TLS, a broken body.
    #[error("transport: {0}")]
    Transport(String),
    /// The body is not the expected JSON or text.
    #[error("decode: {0}")]
    Decode(String),
    /// A redirect to a URL that is not allowed, or more than 3 redirects. Never followed.
    #[error("redirect refused: {0}")]
    Redirect(String),
    /// The 2xx body passed the policy's `max_body_bytes` (the value given).
    #[error("body larger than {0} bytes")]
    TooLarge(usize),
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
///
/// `url` is the caller's, built from [`Outbound::urls`]. Redirects are followed here, at most 3,
/// and only to a URL `may_follow` allows; they take no extra throttle slot.
pub async fn get_bytes(
    o: &Outbound,
    t: &HostThrottle,
    url: Url,
    policy: &RetryPolicy,
) -> Result<Upstream, UpstreamError> {
    let mut retries = Retries::default();
    loop {
        t.acquire(policy.max_wait)
            .await
            .map_err(|Throttled| UpstreamError::Throttled)?;
        let started = Instant::now();
        let answer = tokio::time::timeout(policy.attempt_timeout, attempt(o, &url, policy))
            .await
            .unwrap_or_else(|_| {
                Err(UpstreamError::Transport(format!(
                    "no complete answer within {} s",
                    policy.attempt_timeout.as_secs_f64()
                )))
            });
        let latency_ms = started.elapsed().as_millis() as u64;
        let answer = answer.inspect_err(|error| {
            tracing::warn!(url = %url, latency_ms, %error, "outbound request failed");
        })?;

        let status = answer.status;
        if status.is_success() {
            tracing::info!(url = %url, status = status.as_u16(), latency_ms, "outbound request");
            return Ok(answer);
        }
        tracing::warn!(url = %url, status = status.as_u16(), latency_ms, "outbound request failed");

        if status == StatusCode::TOO_MANY_REQUESTS {
            t.cool_down(retry_after(&answer.headers).unwrap_or(DEFAULT_COOLDOWN));
            return Err(UpstreamError::Throttled);
        }
        match retries.pause_after(status, &answer.headers, policy) {
            Some(pause) => tokio::time::sleep(pause).await,
            None => return Err(UpstreamError::Status(status.as_u16())),
        }
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

/// One attempt: the request, its checked redirects, and a 2xx body read up to the policy's cap.
/// A non-2xx answer comes back with an empty body.
async fn attempt(o: &Outbound, url: &Url, policy: &RetryPolicy) -> Result<Upstream, UpstreamError> {
    let mut url = url.clone();
    let mut redirects = 0;
    loop {
        let response = o.client.get(url.clone()).send().await.map_err(transport)?;
        let status = response.status();
        if status.is_redirection()
            && let Some(location) = response.headers().get(LOCATION)
        {
            let next = location
                .to_str()
                .ok()
                .and_then(|location| url.join(location).ok())
                .ok_or_else(|| {
                    UpstreamError::Redirect(format!("unreadable Location from {url}"))
                })?;
            if redirects == MAX_REDIRECTS {
                return Err(UpstreamError::Redirect(format!(
                    "more than {MAX_REDIRECTS} redirects, the last to {next}"
                )));
            }
            if !may_follow(&o.urls, &next) {
                return Err(UpstreamError::Redirect(format!(
                    "{url} redirects to {next}"
                )));
            }
            redirects += 1;
            url = next;
            continue;
        }
        let headers = response.headers().clone();
        let body = if status.is_success() {
            read_capped(response, policy.max_body_bytes).await?
        } else {
            Bytes::new()
        };
        return Ok(Upstream {
            status,
            headers,
            body,
        });
    }
}

/// Whether a redirect may lead to `url`, the SSRF guard of spec/shelf.md §6 risk 16:
/// - never with a username or password;
/// - any URL on the origin of a configured upstream (which keeps wiremock bases working in tests);
/// - otherwise https, no explicit port, and a domain that is a configured upstream's or
///   `archive.org`, or a subdomain of one (`archive.org.example.com` is not).
///
/// IP addresses pass only as a configured origin.
fn may_follow(urls: &UpstreamUrls, url: &Url) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let upstreams = urls.all();
    if upstreams.iter().any(|base| base.origin() == url.origin()) {
        return true;
    }
    let Some(Host::Domain(host)) = url.host() else {
        return false;
    };
    let mut domains = upstreams
        .iter()
        .filter_map(|base| base.domain())
        .chain(REDIRECT_DOMAINS);
    url.scheme() == "https"
        && url.port().is_none()
        && domains.any(|domain| {
            host == domain
                || host
                    .strip_suffix(domain)
                    .is_some_and(|rest| rest.ends_with('.'))
        })
}

/// The body, refused as soon as it passes `max` bytes, so an endless body never fills memory.
async fn read_capped(mut response: reqwest::Response, max: usize) -> Result<Bytes, UpstreamError> {
    let mut body = BytesMut::new();
    while let Some(chunk) = response.chunk().await.map_err(transport)? {
        if body.len() + chunk.len() > max {
            return Err(UpstreamError::TooLarge(max));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.freeze())
}

fn transport(error: reqwest::Error) -> UpstreamError {
    UpstreamError::Transport(error.to_string())
}

/// The retries one [`get_bytes`] call has used.
#[derive(Debug, Default)]
struct Retries {
    after_5xx: bool,
    after_503: u32,
}

impl Retries {
    /// The pause before retrying a non-2xx answer other than 429, or `None` to give up.
    /// - A 503 with `Retry-After`, under a policy that honours it: that wait, capped at 5 min, up
    ///   to 3 times.
    /// - Then one retry on 502/503/504 when the policy has `retry_5xx_once_after`.
    fn pause_after(
        &mut self,
        status: StatusCode,
        headers: &HeaderMap,
        policy: &RetryPolicy,
    ) -> Option<Duration> {
        if status == StatusCode::SERVICE_UNAVAILABLE
            && policy.honour_retry_after_on_503
            && self.after_503 < MAX_503_RETRIES
            && let Some(wait) = retry_after(headers)
        {
            self.after_503 += 1;
            return Some(wait.min(MAX_503_WAIT));
        }
        if matches!(status.as_u16(), 502..=504)
            && !self.after_5xx
            && let Some(pause) = policy.retry_5xx_once_after
        {
            self.after_5xx = true;
            return Some(pause);
        }
        None
    }
}

/// A positive integer `Retry-After` in seconds; anything else counts as absent. Callers cap it.
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
    use axum::http::HeaderValue;
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
        attempt_timeout: Duration::from_secs(15),
        max_body_bytes: 1000,
    };

    fn redirect_to(location: &str) -> ResponseTemplate {
        ResponseTemplate::new(302).insert_header("location", location)
    }

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

        for status in [502, 504] {
            let p = format!("/down/{status}");
            Mock::given(path(p.as_str()))
                .respond_with(ResponseTemplate::new(status))
                .expect(2)
                .mount(&server)
                .await;
            let error = get_bytes(&o, &o.ol_api, url(&server, &p), &FAST)
                .await
                .unwrap_err();
            assert!(
                matches!(error, UpstreamError::Status(s) if s == status),
                "{error:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_500_is_not_retried() {
        let server = MockServer::start().await;
        Mock::given(path("/broken"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;
        let o = outbound(&server);
        let error = get_bytes(&o, &o.ol_api, url(&server, "/broken"), &FAST)
            .await
            .unwrap_err();
        assert!(matches!(error, UpstreamError::Status(500)), "{error:?}");
    }

    /// REQUEST and MEDIA retry a 503 once after 300 ms and ignore its `Retry-After`.
    #[tokio::test]
    async fn request_paths_ignore_a_503_retry_after() {
        let server = MockServer::start().await;
        Mock::given(path("/busy"))
            .respond_with(ResponseTemplate::new(503).insert_header("retry-after", "30"))
            .expect(4)
            .mount(&server)
            .await;
        let o = outbound(&server);
        for policy in [RetryPolicy::REQUEST, RetryPolicy::MEDIA] {
            let started = Instant::now();
            let error = get_bytes(&o, &o.ol_api, url(&server, "/busy"), &policy)
                .await
                .unwrap_err();
            assert!(matches!(error, UpstreamError::Status(503)), "{error:?}");
            let elapsed = started.elapsed();
            assert!(
                elapsed >= Duration::from_millis(300) && elapsed < Duration::from_secs(5),
                "{elapsed:?}"
            );
        }
    }

    #[test]
    fn jobs_wait_out_at_most_three_503s_then_retry_once_more() {
        let mut retry_after_30 = HeaderMap::new();
        retry_after_30.insert(RETRY_AFTER, HeaderValue::from_static("30"));
        let mut retries = Retries::default();
        let unavailable = StatusCode::SERVICE_UNAVAILABLE;
        let job = RetryPolicy::JOB;
        for _ in 0..3 {
            assert_eq!(
                retries.pause_after(unavailable, &retry_after_30, &job),
                Some(Duration::from_secs(30))
            );
        }
        assert_eq!(
            retries.pause_after(unavailable, &retry_after_30, &job),
            Some(Duration::from_millis(300))
        );
        assert_eq!(
            retries.pause_after(unavailable, &retry_after_30, &job),
            None
        );

        // A long Retry-After is capped at 5 minutes.
        let mut retry_after_hour = HeaderMap::new();
        retry_after_hour.insert(RETRY_AFTER, HeaderValue::from_static("3600"));
        assert_eq!(
            Retries::default().pause_after(unavailable, &retry_after_hour, &job),
            Some(Duration::from_secs(300))
        );
        // Without Retry-After a 503 gets the one 5xx retry only.
        let mut retries = Retries::default();
        assert_eq!(
            retries.pause_after(unavailable, &HeaderMap::new(), &job),
            Some(Duration::from_millis(300))
        );
        assert_eq!(
            retries.pause_after(unavailable, &HeaderMap::new(), &job),
            None
        );
        // Request paths never honour Retry-After.
        let mut retries = Retries::default();
        let request = RetryPolicy::REQUEST;
        assert_eq!(
            retries.pause_after(unavailable, &retry_after_30, &request),
            Some(Duration::from_millis(300))
        );
        assert_eq!(
            retries.pause_after(unavailable, &retry_after_30, &request),
            None
        );
        // Other statuses are never retried.
        for status in [400, 404, 410, 500, 501] {
            let status = StatusCode::from_u16(status).unwrap();
            assert_eq!(
                Retries::default().pause_after(status, &retry_after_30, &job),
                None
            );
        }
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
    async fn a_huge_retry_after_cools_down_for_at_most_an_hour() {
        let server = MockServer::start().await;
        Mock::given(path("/busy"))
            .respond_with(
                ResponseTemplate::new(429).insert_header("retry-after", "18446744073709551615"),
            )
            .mount(&server)
            .await;
        let o = outbound(&server);
        let error = get_bytes(&o, &o.ol_api, url(&server, "/busy"), &FAST)
            .await
            .unwrap_err();
        assert!(matches!(error, UpstreamError::Throttled));
        let remaining = o.ol_api.cooldown_remaining().unwrap();
        assert!(
            remaining > Duration::from_secs(3590) && remaining <= MAX_COOLDOWN,
            "{remaining:?}"
        );

        let throttle = HostThrottle::per_second(1.0);
        throttle.cool_down(Duration::MAX);
        assert!(throttle.cooldown_remaining().unwrap() <= MAX_COOLDOWN);
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
    async fn a_slow_upstream_hits_the_attempt_deadline() {
        let server = MockServer::start().await;
        Mock::given(path("/slow"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("late")
                    .set_delay(Duration::from_secs(5)),
            )
            .mount(&server)
            .await;
        let o = outbound(&server);
        let policy = RetryPolicy {
            attempt_timeout: Duration::from_millis(200),
            ..FAST
        };
        let started = Instant::now();
        let error = get_bytes(&o, &o.ol_api, url(&server, "/slow"), &policy)
            .await
            .unwrap_err();
        assert!(matches!(error, UpstreamError::Transport(_)), "{error:?}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn bodies_over_the_cap_are_refused() {
        let server = MockServer::start().await;
        Mock::given(path("/fits"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; 1000]))
            .mount(&server)
            .await;
        Mock::given(path("/huge"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; 1001]))
            .mount(&server)
            .await;
        let o = outbound(&server);
        let fits = get_bytes(&o, &o.ol_api, url(&server, "/fits"), &FAST)
            .await
            .unwrap();
        assert_eq!(fits.body.len(), 1000);
        let error = get_bytes(&o, &o.ol_api, url(&server, "/huge"), &FAST)
            .await
            .unwrap_err();
        assert!(matches!(error, UpstreamError::TooLarge(1000)), "{error:?}");
        assert_eq!(
            error.to_app_error("busy", Some("missing"), "down").status,
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(RetryPolicy::MEDIA.max_body_bytes, 1_500_000);
    }

    #[tokio::test]
    async fn redirects_on_the_upstream_origin_are_followed_up_to_three() {
        let server = MockServer::start().await;
        Mock::given(path("/isbn/9780140328721.json"))
            .respond_with(redirect_to("/books/OL7353617M.json"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/books/OL7353617M.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .expect(1)
            .mount(&server)
            .await;
        let o = outbound(&server);
        let found = get_text(
            &o,
            &o.ol_api,
            url(&server, "/isbn/9780140328721.json"),
            &FAST,
        )
        .await
        .unwrap();
        assert_eq!(found, "{}");

        // Three redirects (four requests) are fine.
        for hop in 0..3 {
            Mock::given(path(format!("/three/{hop}")))
                .respond_with(redirect_to(&format!("/three/{}", hop + 1)))
                .expect(1)
                .mount(&server)
                .await;
        }
        Mock::given(path("/three/3"))
            .respond_with(ResponseTemplate::new(200).set_body_string("done"))
            .expect(1)
            .mount(&server)
            .await;
        let done = get_text(&o, &o.ol_api, url(&server, "/three/0"), &FAST)
            .await
            .unwrap();
        assert_eq!(done, "done");
    }

    #[tokio::test]
    async fn a_fourth_redirect_is_refused() {
        let server = MockServer::start().await;
        for hop in 0..4 {
            Mock::given(path(format!("/hop/{hop}")))
                .respond_with(redirect_to(&format!("/hop/{}", hop + 1)))
                .expect(1)
                .mount(&server)
                .await;
        }
        Mock::given(path("/hop/4"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let o = outbound(&server);
        let error = get_bytes(&o, &o.ol_api, url(&server, "/hop/0"), &FAST)
            .await
            .unwrap_err();
        assert!(matches!(error, UpstreamError::Redirect(_)), "{error:?}");
        assert_eq!(
            error.to_app_error("busy", Some("missing"), "down").status,
            StatusCode::BAD_GATEWAY
        );
    }

    #[tokio::test]
    async fn redirects_off_the_allow_list_are_never_requested() {
        let server = MockServer::start().await;
        let port = server.address().port();
        // Same server, other origin: `localhost` is not the configured `127.0.0.1`.
        let elsewhere = format!("http://localhost:{port}/internal");
        for (from, to) in [
            ("/to-internal", elsewhere.as_str()),
            ("/to-lookalike", "https://archive.org.example.com/cover.jpg"),
            ("/to-metadata", "http://169.254.169.254/latest/meta-data/"),
            ("/to-nowhere", "http://[::1"),
        ] {
            Mock::given(path(from))
                .respond_with(redirect_to(to))
                .expect(1)
                .mount(&server)
                .await;
        }
        Mock::given(path("/internal"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let o = outbound(&server);
        for from in [
            "/to-internal",
            "/to-lookalike",
            "/to-metadata",
            "/to-nowhere",
        ] {
            let error = get_bytes(&o, &o.ol_api, url(&server, from), &FAST)
                .await
                .unwrap_err();
            assert!(
                matches!(error, UpstreamError::Redirect(_)),
                "{from}: {error:?}"
            );
        }
    }

    #[test]
    fn the_redirect_allow_list() {
        let pairs = [("DATABASE_URL", "postgres://x"), ("DEV_MODE", "true")];
        let config = Config::from_lookup(|key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        })
        .unwrap();
        let urls = Outbound::new(&config).unwrap().urls;
        let allowed = |raw: &str| may_follow(&urls, &Url::parse(raw).unwrap());
        for good in [
            "https://covers.openlibrary.org/b/id/8739161-M.jpg",
            "https://openlibrary.org/books/OL7353617M.json",
            "https://archive.org/download/l_covers_0008/l_covers_0008_73.zip/0008739161-L.jpg",
            "https://ia802809.us.archive.org/view_archive.php?archive=x",
            "https://ARCHIVE.org/x",
            "https://archive.org:443/x",
            "https://services.dnb.de/sru/dnb?query=x",
        ] {
            assert!(allowed(good), "{good}");
        }
        for bad in [
            "http://archive.org/x",
            "https://archive.org:8443/x",
            "https://user@archive.org/x",
            "https://user:secret@covers.openlibrary.org/b/id/1-M.jpg",
            "https://127.0.0.1/x",
            "https://[::1]/x",
            "https://example.com/x",
            "https://archive.org.example.com/x",
            "https://notarchive.org/x",
            "http://covers.openlibrary.org/b/id/1-M.jpg",
            "https://books.google.com/books/content?id=x",
            "ftp://archive.org/x",
        ] {
            assert!(!allowed(bad), "{bad}");
        }
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

    #[tokio::test]
    async fn the_host_opens_again_after_the_cooldown() {
        let throttle = HostThrottle::per_second(1000.0);
        throttle.cool_down(Duration::from_millis(300));
        assert_eq!(throttle.acquire(Duration::ZERO).await, Err(Throttled));
        // A late wakeup only makes the cooldown more over.
        tokio::time::sleep(Duration::from_millis(350)).await;
        assert_eq!(throttle.cooldown_remaining(), None);
        assert_eq!(throttle.acquire(Duration::ZERO).await, Ok(()));
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
