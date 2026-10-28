//! Response caches with request coalescing (docs/phase1/spec/catalog.md §2.17): raw upstream
//! payloads, never annotated results, kept for 18 hours. Identical concurrent requests share one
//! upstream call and its outcome; errors and empty result lists are never cached, so a
//! lucky-empty answer cannot freeze for 18 hours.

use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;

use moka::future::Cache;

/// How long a cached upstream answer is served (Python `_CACHE_TTL`).
pub const CACHE_TTL: Duration = Duration::from_secs(18 * 3600);

/// Why an upstream call gave no usable answer. Shared by coalesced callers, so it is `Clone`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetch {
    /// The local throttle, a cooldown, or an upstream 429.
    Throttled,
    /// Upstream 404.
    Missing,
    /// Upstream 422 (Open Library: "query too short").
    Unprocessable,
    /// Any other failure: transport, another status, an undecodable body.
    Failed(String),
}

impl From<crate::outbound::UpstreamError> for Fetch {
    fn from(error: crate::outbound::UpstreamError) -> Self {
        use crate::outbound::UpstreamError;
        match error {
            UpstreamError::Throttled => Fetch::Throttled,
            UpstreamError::Status(404) => Fetch::Missing,
            UpstreamError::Status(422) => Fetch::Unprocessable,
            other => Fetch::Failed(other.to_string()),
        }
    }
}

/// What the init future of a coalesced call settles to when it is not cached.
#[derive(Debug)]
enum Uncached<V> {
    /// A good answer that must not be cached (an empty result list): handed to every waiter.
    Value(V),
    Failed(Fetch),
}

/// A bounded, expiring cache of upstream answers keyed by a normalised request key.
pub struct ResponseCache<K, V> {
    cache: Cache<K, V>,
}

impl<K, V> ResponseCache<K, V>
where
    K: Hash + Eq + Send + Sync + Clone + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// A cache of at most `capacity` entries (approximately: eviction runs in the background)
    /// that expire `ttl` after they were written.
    pub fn new(capacity: u64, ttl: Duration) -> Self {
        Self {
            cache: Cache::builder()
                .max_capacity(capacity)
                .time_to_live(ttl)
                .build(),
        }
    }

    /// The cached answer for `key`, or the outcome of `fetch`. Concurrent calls for one key
    /// share one `fetch` and its outcome, error included. An answer is stored only when
    /// `cacheable` says so. With `bypass` the cache is not read, but a cacheable answer is still
    /// written (a refresh replaces the stored answer).
    ///
    /// When the caller that runs `fetch` is dropped (its client went away), a waiting caller
    /// runs `fetch` itself instead of failing; nothing ever waits forever.
    pub async fn get_or_fetch<F, Fut>(
        &self,
        key: K,
        bypass: bool,
        cacheable: impl Fn(&V) -> bool,
        fetch: F,
    ) -> Result<V, Fetch>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<V, Fetch>>,
    {
        if bypass {
            let value = fetch().await?;
            if cacheable(&value) {
                self.cache.insert(key, value.clone()).await;
            }
            return Ok(value);
        }
        let outcome = self
            .cache
            .try_get_with(key, async {
                match fetch().await {
                    Ok(value) if cacheable(&value) => Ok(value),
                    Ok(value) => Err(Uncached::Value(value)),
                    Err(error) => Err(Uncached::Failed(error)),
                }
            })
            .await;
        match outcome {
            Ok(value) => Ok(value),
            Err(shared) => match &*shared {
                Uncached::Value(value) => Ok(value.clone()),
                Uncached::Failed(error) => Err(error.clone()),
            },
        }
    }

    /// The cached answer, if any (tests).
    pub async fn get(&self, key: &K) -> Option<V> {
        self.cache.get(key).await
    }

    /// Stores an answer (tests).
    pub async fn insert(&self, key: K, value: V) {
        self.cache.insert(key, value).await;
    }

    /// Runs the pending evictions and returns the number of entries (tests).
    pub async fn entry_count(&self) -> u64 {
        self.cache.run_pending_tasks().await;
        self.cache.entry_count()
    }

    /// Drops every entry (tests: "clear the RAM caches, reads stay local").
    pub fn clear(&self) {
        self.cache.invalidate_all();
    }
}

/// Shared JSON payloads.
pub type JsonCache = ResponseCache<String, Arc<serde_json::Value>>;

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::sync::Notify;

    use super::*;

    fn cache(capacity: u64) -> ResponseCache<String, Arc<String>> {
        ResponseCache::new(capacity, CACHE_TTL)
    }

    /// parity: test_search.py::test_search_cache_is_bounded
    /// The cache is bounded by entry count (moka evicts approximately, not in strict LRU order).
    #[tokio::test]
    async fn the_cache_is_bounded() {
        let cache = cache(2);
        for key in ["a", "b", "c", "d"] {
            cache.insert(key.into(), Arc::new(key.into())).await;
        }
        assert!(cache.entry_count().await <= 2);
    }

    /// parity: test_search.py::test_search_cache_ttl_is_at_least_half_a_day
    #[test]
    fn the_ttl_is_at_least_half_a_day() {
        assert!(CACHE_TTL >= Duration::from_secs(12 * 3600));
    }

    #[tokio::test]
    async fn answers_are_cached_but_errors_and_uncacheable_answers_are_not() {
        let cache = cache(10);
        let calls = AtomicUsize::new(0);
        let fetch = |answer: Result<&'static str, Fetch>| {
            calls.fetch_add(1, Ordering::SeqCst);
            async move { answer.map(|s| Arc::new(s.to_owned())) }
        };
        let never = |_: &Arc<String>| false;
        let always = |_: &Arc<String>| true;

        let failed = cache
            .get_or_fetch("k".into(), false, always, || fetch(Err(Fetch::Throttled)))
            .await;
        assert_eq!(failed, Err(Fetch::Throttled));
        let empty = cache
            .get_or_fetch("k".into(), false, never, || fetch(Ok("")))
            .await;
        assert_eq!(empty.unwrap().as_str(), "");
        let first = cache
            .get_or_fetch("k".into(), false, always, || fetch(Ok("x")))
            .await;
        assert_eq!(first.unwrap().as_str(), "x");
        let cached = cache
            .get_or_fetch("k".into(), false, always, || fetch(Ok("y")))
            .await;
        assert_eq!(cached.unwrap().as_str(), "x");
        assert_eq!(calls.load(Ordering::SeqCst), 3);

        // A bypass skips the read but writes the fresh answer.
        let fresh = cache
            .get_or_fetch("k".into(), true, always, || fetch(Ok("z")))
            .await;
        assert_eq!(fresh.unwrap().as_str(), "z");
        assert_eq!(cache.get(&"k".to_owned()).await.unwrap().as_str(), "z");
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    /// parity: test_search.py::test_identical_inflight_searches_share_one_request
    /// parity: test_google_books.py::test_google_failure_does_not_leave_unobserved_future
    /// (ADAPT: a failing coalesced fetch hands its error to every waiter and leaves nothing
    /// behind.)
    #[tokio::test]
    async fn concurrent_calls_share_one_fetch_and_its_outcome() {
        for outcome in [Ok("circe"), Err(Fetch::Failed("boom".into()))] {
            let cache = Arc::new(cache(10));
            let calls = Arc::new(AtomicUsize::new(0));
            let release = Arc::new(Notify::new());
            let run = |cache: Arc<ResponseCache<String, Arc<String>>>| {
                let calls = calls.clone();
                let release = release.clone();
                let outcome = outcome.clone();
                tokio::spawn(async move {
                    cache
                        .get_or_fetch(
                            "ol|search|circe".into(),
                            false,
                            |_| true,
                            || async move {
                                calls.fetch_add(1, Ordering::SeqCst);
                                release.notified().await;
                                outcome.map(|s| Arc::new(s.to_owned()))
                            },
                        )
                        .await
                })
            };
            let first = run(cache.clone());
            while calls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
            let second = run(cache.clone());
            tokio::time::sleep(Duration::from_millis(50)).await;
            release.notify_waiters();
            let (a, b) = (first.await.unwrap(), second.await.unwrap());
            assert_eq!(a, b);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            if a.is_err() {
                // The error was not cached: the next call fetches again.
                let again = cache
                    .get_or_fetch(
                        "ol|search|circe".into(),
                        false,
                        |_| true,
                        || async { Ok(Arc::new("later".to_owned())) },
                    )
                    .await;
                assert_eq!(again.unwrap().as_str(), "later");
            }
        }
    }

    /// parity: test_search.py::test_cancelled_fetch_unblocks_coalesced_waiters
    /// Reasoned change (work-plan catalog decision 16): when the caller running the fetch is
    /// dropped, a waiting caller runs the fetch itself instead of failing with a 502. It never
    /// hangs, and nothing stays in flight.
    #[tokio::test]
    async fn a_dropped_owner_never_leaves_a_waiter_hanging() {
        let cache = Arc::new(cache(10));
        let calls = Arc::new(AtomicUsize::new(0));
        let owner = {
            let (cache, calls) = (cache.clone(), calls.clone());
            tokio::spawn(async move {
                cache
                    .get_or_fetch(
                        "k".into(),
                        false,
                        |_| true,
                        || async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            std::future::pending::<Result<Arc<String>, Fetch>>().await
                        },
                    )
                    .await
            })
        };
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        let follower = {
            let (cache, calls) = (cache.clone(), calls.clone());
            tokio::spawn(async move {
                cache
                    .get_or_fetch(
                        "k".into(),
                        false,
                        |_| true,
                        || async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            Ok(Arc::new("circe".to_owned()))
                        },
                    )
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        owner.abort();
        let answer = tokio::time::timeout(Duration::from_secs(5), follower)
            .await
            .expect("the follower must not hang")
            .unwrap();
        assert_eq!(answer.unwrap().as_str(), "circe");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
