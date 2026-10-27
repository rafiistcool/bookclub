//! In-process sliding-window rate limits (docs/phase1/conventions.md §10.1).

use std::collections::VecDeque;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::error::AppError;

/// Keys per limit before the least recently used ones are dropped.
const MAX_KEYS: u64 = 100_000;
const MINUTE: Duration = Duration::from_secs(60);
const HOUR: Duration = Duration::from_secs(3600);
const DAY: Duration = Duration::from_secs(86_400);

/// Seconds until the oldest hit leaves the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryAfter(pub u64);

impl RetryAfter {
    /// 429 with `msg` (`TOO_MANY_ATTEMPTS` for auth limits, `SLOW_DOWN` for the others) and
    /// `Retry-After`.
    pub fn into_error(self, msg: &'static str) -> AppError {
        AppError::too_many(msg, Some(self.0))
    }
}

/// At most `limit` hits per key within any `window`, on the monotonic clock.
pub struct SlidingWindow {
    limit: u32,
    window: Duration,
    hits: moka::sync::Cache<String, Arc<Mutex<VecDeque<Instant>>>>,
}

impl SlidingWindow {
    pub fn new(limit: u32, window: Duration) -> Self {
        Self {
            limit,
            window,
            hits: moka::sync::Cache::builder()
                .max_capacity(MAX_KEYS)
                .time_to_idle(window)
                .build(),
        }
    }

    /// True when `key` already has `limit` hits within the window.
    pub fn is_limited(&self, key: &str) -> bool {
        self.check(key).is_err()
    }

    /// Like [`SlidingWindow::is_limited`], with the wait. Records nothing, so it cannot guard
    /// work that runs concurrently: every request that checks before the first `hit` lands passes.
    /// Login and password change therefore use [`SlidingWindow::check_and_hit`] before verifying
    /// and [`SlidingWindow::clear`] on success.
    pub fn check(&self, key: &str) -> Result<(), RetryAfter> {
        self.check_at(key, Instant::now())
    }

    /// Records one hit.
    pub fn hit(&self, key: &str) {
        self.hit_at(key, Instant::now());
    }

    /// Records a hit unless the key is already limited; a refused attempt records nothing.
    /// Atomic per key: of N concurrent calls on a fresh key, exactly `limit` pass.
    pub fn check_and_hit(&self, key: &str) -> Result<(), RetryAfter> {
        self.check_and_hit_at(key, Instant::now())
    }

    /// Forgets every hit of `key` (a successful login clears its failures).
    pub fn clear(&self, key: &str) {
        self.hits.invalidate(key);
    }

    fn entry(&self, key: &str) -> Arc<Mutex<VecDeque<Instant>>> {
        self.hits.get_with_by_ref(key, Default::default)
    }

    fn check_at(&self, key: &str, now: Instant) -> Result<(), RetryAfter> {
        let Some(entry) = self.hits.get(key) else {
            return Ok(());
        };
        let mut hits = lock(&entry);
        self.admit(&mut hits, now)
    }

    fn hit_at(&self, key: &str, now: Instant) {
        let entry = self.entry(key);
        let mut hits = lock(&entry);
        self.prune(&mut hits, now);
        hits.push_back(now);
    }

    fn check_and_hit_at(&self, key: &str, now: Instant) -> Result<(), RetryAfter> {
        let entry = self.entry(key);
        let mut hits = lock(&entry);
        self.admit(&mut hits, now)?;
        hits.push_back(now);
        Ok(())
    }

    fn prune(&self, hits: &mut VecDeque<Instant>, now: Instant) {
        while hits
            .front()
            .is_some_and(|oldest| now.duration_since(*oldest) >= self.window)
        {
            hits.pop_front();
        }
    }

    fn admit(&self, hits: &mut VecDeque<Instant>, now: Instant) -> Result<(), RetryAfter> {
        self.prune(hits, now);
        if hits.len() < self.limit as usize {
            return Ok(());
        }
        let oldest = hits.front().copied().unwrap_or(now);
        let wait = self.window.saturating_sub(now.duration_since(oldest));
        Err(RetryAfter(wait.as_secs_f64().ceil().max(1.0) as u64))
    }
}

fn lock(entry: &Mutex<VecDeque<Instant>>) -> std::sync::MutexGuard<'_, VecDeque<Instant>> {
    // A panic while holding the lock cannot leave the deque inconsistent.
    entry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Every limit of §10.1. Each test gets a fresh set, because the harness builds a fresh `AppState`.
pub struct RateLimits {
    /// Login failures per account and identifier kind: `u:<id>` for the username (shared with
    /// the current-password check of password change, email change and account deletion),
    /// `u:<id>:email` for the verified address. Cleared on success.
    pub login_failures: SlidingWindow,
    /// Login failures per identifier that names no account, keyed by
    /// [`unknown_identifier_key`]. Its own cache, so a flood of made-up names cannot evict the
    /// keys of real accounts.
    pub login_unknown: SlidingWindow,
    /// Login attempts per [`ip_key`].
    pub login_ip: SlidingWindow,
    pub signup_ip: SlidingWindow,
    pub reset_ip: SlidingWindow,
    /// Silent: the response is still 204.
    pub reset_account: SlidingWindow,
    /// Verification resend and `PUT /api/auth/me/email`, per account.
    pub email_send_user: SlidingWindow,
    /// Any mail to one case-folded address across all accounts. Silent: the mail is not sent.
    pub email_target: SlidingWindow,
    pub invite_preview_ip: SlidingWindow,
    /// `POST /api/invites/{code}/accept` per account, valid code or not.
    pub invite_accept_user: SlidingWindow,
    pub refresh_user: SlidingWindow,
    pub upload_user: SlidingWindow,
    pub report_user: SlidingWindow,
    pub edit_request_user: SlidingWindow,
    pub custom_book_user: SlidingWindow,
    pub export_user: SlidingWindow,
}

impl RateLimits {
    /// The thresholds of §10.1.
    pub fn new() -> Self {
        Self {
            login_failures: SlidingWindow::new(10, 10 * MINUTE),
            login_unknown: SlidingWindow::new(10, 10 * MINUTE),
            login_ip: SlidingWindow::new(30, 10 * MINUTE),
            signup_ip: SlidingWindow::new(5, HOUR),
            reset_ip: SlidingWindow::new(5, HOUR),
            reset_account: SlidingWindow::new(3, HOUR),
            email_send_user: SlidingWindow::new(3, HOUR),
            email_target: SlidingWindow::new(6, DAY),
            invite_preview_ip: SlidingWindow::new(60, HOUR),
            invite_accept_user: SlidingWindow::new(20, HOUR),
            refresh_user: SlidingWindow::new(10, HOUR),
            upload_user: SlidingWindow::new(30, DAY),
            report_user: SlidingWindow::new(20, DAY),
            edit_request_user: SlidingWindow::new(50, DAY),
            custom_book_user: SlidingWindow::new(30, DAY),
            export_user: SlidingWindow::new(5, HOUR),
        }
    }
}

impl Default for RateLimits {
    fn default() -> Self {
        Self::new()
    }
}

/// Rate-limit key for a client address: an IPv4 address as is, an IPv6 address by its /64
/// prefix (a client with a /64 can otherwise mint a fresh key per request).
pub fn ip_key(ip: IpAddr) -> String {
    match ip.to_canonical() {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => {
            let s = v6.segments();
            format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
        }
    }
}

/// Key for an identifier that names no account: `"n:"` plus the hex SHA-256 of the normalised
/// identifier. Every name has its own bucket, like an account: a shared bucket would let
/// colliding names fill it and tell an unknown address from a known one by the 429.
pub fn unknown_identifier_key(normalised: &str) -> String {
    let digest = Sha256::digest(normalised.as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("n:{hex}")
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::http::StatusCode;

    use super::*;
    use crate::error::msg;

    #[test]
    fn allows_up_to_the_limit_then_refuses() {
        let limit = SlidingWindow::new(3, HOUR);
        for _ in 0..3 {
            assert!(limit.check_and_hit("ip").is_ok());
        }
        let RetryAfter(wait) = limit.check_and_hit("ip").unwrap_err();
        assert!((3590..=3600).contains(&wait), "{wait}");
        assert!(limit.is_limited("ip"));
        assert!(!limit.is_limited("other"));
        assert!(limit.check_and_hit("other").is_ok());
    }

    #[test]
    fn a_refused_attempt_is_not_recorded() {
        // A long window: neither the window nor moka's idle expiry can hide a recorded refusal.
        let limit = SlidingWindow::new(1, HOUR);
        assert!(limit.check_and_hit("k").is_ok());
        for _ in 0..5 {
            assert!(limit.check_and_hit("k").is_err());
        }
        assert_eq!(limit.hits.get("k").unwrap().lock().unwrap().len(), 1);
    }

    #[test]
    fn concurrent_attempts_cannot_pass_together() {
        // 64 logins for one account at once: exactly the 10 the limit allows get to verify.
        let limit = SlidingWindow::new(10, 10 * MINUTE);
        let start = Barrier::new(64);
        let admitted = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..64 {
                scope.spawn(|| {
                    start.wait();
                    if limit.check_and_hit("u:1").is_ok() {
                        admitted.fetch_add(1, Ordering::SeqCst);
                    }
                });
            }
        });
        assert_eq!(admitted.load(Ordering::SeqCst), 10);
        assert_eq!(limit.hits.get("u:1").unwrap().lock().unwrap().len(), 10);
    }

    #[test]
    fn failures_counted_with_hit_and_cleared_on_success() {
        let failures = SlidingWindow::new(2, HOUR);
        assert!(failures.check("u:1").is_ok());
        failures.hit("u:1");
        failures.hit("u:1");
        assert!(failures.check("u:1").is_err());
        failures.clear("u:1");
        assert!(failures.check("u:1").is_ok());
        assert!(!failures.is_limited("u:1"));
    }

    #[test]
    fn the_window_slides() {
        let limit = SlidingWindow::new(2, Duration::from_secs(300));
        let t0 = Instant::now();
        let at = |seconds: u64| t0 + Duration::from_secs(seconds);
        limit.hit_at("k", t0);
        limit.hit_at("k", at(200));
        // Full: the first hit leaves the window at 300 s.
        assert_eq!(limit.check_at("k", at(200)), Err(RetryAfter(100)));
        assert_eq!(limit.check_and_hit_at("k", at(299)), Err(RetryAfter(1)));
        // At 350 s the first hit has left; the second (until 500 s) has not.
        assert_eq!(limit.check_and_hit_at("k", at(350)), Ok(()));
        assert_eq!(limit.check_at("k", at(350)), Err(RetryAfter(150)));
        // Both remaining hits leave at 500 and 650 s.
        assert_eq!(limit.check_at("k", at(500)), Ok(()));
        assert_eq!(limit.check_and_hit_at("k", at(650)), Ok(()));
        assert_eq!(limit.hits.get("k").unwrap().lock().unwrap().len(), 1);
    }

    #[test]
    fn retry_after_becomes_a_429() {
        let error = RetryAfter(30).into_error(msg::TOO_MANY_ATTEMPTS);
        assert_eq!(error.status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(error.detail, msg::TOO_MANY_ATTEMPTS);
        assert_eq!(error.retry_after, Some(30));
    }

    #[test]
    fn ipv6_addresses_in_one_slash_64_share_a_bucket() {
        let a: IpAddr = "2001:db8:1:2:aaaa::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2:ffff:ffff:ffff:ffff".parse().unwrap();
        let c: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        assert_eq!(ip_key(a), ip_key(b));
        assert_eq!(ip_key(a), "2001:db8:1:2::/64");
        assert_ne!(ip_key(a), ip_key(c));
        let limits = RateLimits::new();
        for _ in 0..5 {
            limits.signup_ip.hit(&ip_key(a));
        }
        assert!(limits.signup_ip.is_limited(&ip_key(b)));
        assert!(!limits.signup_ip.is_limited(&ip_key(c)));
    }

    #[test]
    fn ipv4_keys_are_the_address() {
        assert_eq!(ip_key("203.0.113.7".parse().unwrap()), "203.0.113.7");
        assert_eq!(ip_key("::ffff:203.0.113.7".parse().unwrap()), "203.0.113.7");
    }

    #[test]
    fn every_unknown_identifier_has_its_own_key() {
        assert_eq!(
            unknown_identifier_key("nobody"),
            "n:6382b3cc881412b77bfcaeed026001c00d9e3025e66c20f6e7e92f079851462a"
        );
        let keys: HashSet<String> = (0..100_000)
            .map(|i| unknown_identifier_key(&format!("user{i}")))
            .collect();
        assert_eq!(keys.len(), 100_000);
    }

    #[test]
    fn thresholds_match_the_table() {
        let limits = RateLimits::new();
        let cases = [
            (&limits.login_failures, 10, 600),
            (&limits.login_unknown, 10, 600),
            (&limits.login_ip, 30, 600),
            (&limits.signup_ip, 5, 3600),
            (&limits.reset_ip, 5, 3600),
            (&limits.reset_account, 3, 3600),
            (&limits.email_send_user, 3, 3600),
            (&limits.email_target, 6, 86_400),
            (&limits.invite_preview_ip, 60, 3600),
            (&limits.invite_accept_user, 20, 3600),
            (&limits.refresh_user, 10, 3600),
            (&limits.upload_user, 30, 86_400),
            (&limits.report_user, 20, 86_400),
            (&limits.edit_request_user, 50, 86_400),
            (&limits.custom_book_user, 30, 86_400),
            (&limits.export_user, 5, 3600),
        ];
        for (limit, count, seconds) in cases {
            assert_eq!(limit.limit, count);
            assert_eq!(limit.window, Duration::from_secs(seconds));
        }
    }
}
