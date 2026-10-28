//! Search, trending and subjects (#41): the `test_search.py` parity ports, the search KEEP and
//! ADAPT tests of `test_google_books.py` (docs/phase1/spec/google.md §4) and the #41 acceptance
//! items on local-first results, deduplication and upstream errors.
//!
//! `FakeOpenLibrary` is a port of the Python `_FakeClient`: one wiremock responder that routes
//! Open Library paths to the same canned payloads, so the assertions on sent parameters and
//! call counts carry over.

mod common;

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use wiremock::{Mock, Request, Respond, ResponseTemplate};

fn search_docs() -> Value {
    json!([
        {
            "key": "/works/OL1W",
            "title": "Circe",
            "author_name": ["Madeline Miller"],
            "cover_i": 123,
            "first_publish_year": 2018,
            "number_of_pages_median": 393,
            "ratings_average": 4.27,
            "ratings_count": 812
        },
        {"key": "/books/OL9M", "title": "An edition, skip"},
        {"key": "/works/OL1W", "title": "Circe duplicate work"}
    ])
}

fn trending_works() -> Value {
    json!([
        {
            "key": "/works/OL7W",
            "title": "Atomic Habits",
            "author_name": ["James Clear"],
            "cover_i": 999,
            "first_publish_year": 2016
        },
        {"key": "/works/OL8W", "title": "No cover", "cover_i": -1}
    ])
}

/// subjects/*.json uses cover_id and authors[].name instead of cover_i/author_name.
fn subject_works() -> Value {
    json!([
        {
            "key": "/works/OL138052W",
            "title": "Alice's Adventures in Wonderland",
            "authors": [{"key": "/authors/OL22098A", "name": "Lewis Carroll"}],
            "cover_id": 10527843,
            "first_publish_year": 1865
        },
        {"key": "/authors/OL1A", "title": "Not a work, skip"}
    ])
}

#[derive(Default)]
struct FakeState {
    /// Path marker → status (with `Retry-After` when set).
    status_for: Vec<(String, u16)>,
    /// `search.json` whose `q` starts with the prefix → status.
    status_for_q: Vec<(String, u16)>,
    retry_after: Option<String>,
    /// Path markers answered 404.
    missing: Vec<String>,
    /// One-shot answers: the first queued entry whose marker is in the path answers once.
    once: VecDeque<(String, u16, Value)>,
    delay: Option<Duration>,
}

/// The Python `_FakeClient`, as a wiremock responder.
#[derive(Clone, Default)]
struct FakeOpenLibrary {
    state: Arc<Mutex<FakeState>>,
}

impl FakeOpenLibrary {
    async fn mount(app: &TestApp) -> Self {
        let fake = Self::default();
        Mock::given(wiremock::matchers::any())
            .respond_with(fake.clone())
            .mount(&app.upstream)
            .await;
        fake
    }

    fn status_for(&self, marker: &str, status: u16) {
        let mut state = self.state.lock().unwrap();
        state.status_for.retain(|(m, _)| m != marker);
        state.status_for.push((marker.to_owned(), status));
    }

    /// `search.json` requests whose `q` starts with `prefix` answer `status`.
    fn status_for_q(&self, prefix: &str, status: u16) {
        let mut state = self.state.lock().unwrap();
        state.status_for_q.retain(|(p, _)| p != prefix);
        state.status_for_q.push((prefix.to_owned(), status));
    }

    fn clear_status(&self) {
        self.state.lock().unwrap().status_for.clear();
    }

    fn retry_after(&self, value: &str) {
        self.state.lock().unwrap().retry_after = Some(value.to_owned());
    }

    fn missing(&self, marker: &str) {
        self.state.lock().unwrap().missing.push(marker.to_owned());
    }

    fn once(&self, marker: &str, status: u16, body: Value) {
        self.state
            .lock()
            .unwrap()
            .once
            .push_back((marker.to_owned(), status, body));
    }

    fn delay(&self, delay: Duration) {
        self.state.lock().unwrap().delay = Some(delay);
    }
}

fn answer(status: u16, body: Value) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(body)
}

impl Respond for FakeOpenLibrary {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let mut state = self.state.lock().unwrap();
        let path = request.url.path().to_owned();
        let delay = state.delay;
        let delayed = |template: ResponseTemplate| match delay {
            Some(delay) => template.set_delay(delay),
            None => template,
        };
        if let Some(i) = state
            .once
            .iter()
            .position(|(m, _, _)| path.contains(m.as_str()))
        {
            let (_, status, body) = state.once.remove(i).unwrap();
            return delayed(answer(status, body));
        }
        if let Some((_, status)) = state
            .status_for
            .iter()
            .find(|(m, _)| path.contains(m.as_str()))
        {
            let mut template = ResponseTemplate::new(*status);
            if let Some(retry_after) = &state.retry_after {
                template = template.insert_header("Retry-After", retry_after.as_str());
            }
            return template;
        }
        if state.missing.iter().any(|m| path.contains(m.as_str())) {
            return ResponseTemplate::new(404);
        }
        let q: String = request
            .url
            .query_pairs()
            .find(|(k, _)| k == "q")
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default();
        if path.contains("search.json")
            && let Some((_, status)) = state
                .status_for_q
                .iter()
                .find(|(prefix, _)| q.starts_with(prefix.as_str()))
        {
            return ResponseTemplate::new(*status);
        }
        let body = if path.contains("search.json") {
            if q == "zzzzempty" || q.starts_with("zzzzempty ") || q.ends_with(":zzzzempty") {
                json!({"numFound": 0, "docs": []})
            } else if q.starts_with("title:obscurexyz") || q.starts_with("author:obscurexyz") {
                json!({"numFound": 1, "docs": [search_docs()[0]]})
            } else if q == "obscurexyz" || q.starts_with("obscurexyz ") {
                json!({"numFound": 0, "docs": []})
            } else if q.contains("isbn:") {
                let mut doc = search_docs()[0].clone();
                doc["isbn"] = json!(["9780316769488"]);
                doc["cover_edition_key"] = json!("OL1M");
                json!({"numFound": 1, "docs": [doc]})
            } else if q.contains("title:\"Stand\"") && q.contains("author:\"Me\"") {
                // Adapted: "OLJUNKW" is no valid Open Library id, so the junk hit is OL66W.
                json!({"numFound": 6, "docs": [{
                    "key": "/works/OL66W",
                    "title": "Nothing Stands Between a Girl and Her Vegas",
                    "author_name": ["Raised Me"],
                    "cover_i": 1,
                    "first_publish_year": 2020
                }]})
            } else if q == "Stand by Me" || q.starts_with("Stand by Me ") {
                // Adapted: "OLSTANDW" is no valid Open Library id, so the hit is OL77W.
                json!({"numFound": 1, "docs": [{
                    "key": "/works/OL77W",
                    "title": "Stand by Me",
                    "author_name": ["Ben E. King"],
                    "cover_i": 2,
                    "first_publish_year": 1986
                }]})
            } else {
                json!({"numFound": 50, "docs": search_docs()})
            }
        } else if path.contains("trending") {
            json!({"works": trending_works()})
        } else if path.contains("/subjects/") {
            json!({"work_count": 40, "works": subject_works()})
        } else {
            return ResponseTemplate::new(404);
        };
        delayed(answer(200, body))
    }
}

/// The query parameters of every upstream request whose path contains `needle`, oldest first.
/// Every request must carry the Bookclub User-Agent, as the Python fake asserted.
async fn params_for(app: &TestApp, needle: &str) -> Vec<HashMap<String, String>> {
    let requests = app.upstream_requests().await;
    for request in &requests {
        let agent = request
            .headers
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(agent.contains("Bookclub/1.0"), "user agent {agent:?}");
    }
    requests
        .iter()
        .filter(|r| r.url.path().contains(needle))
        .map(|r| r.url.query_pairs().into_owned().collect())
        .collect()
}

async fn paths(app: &TestApp) -> Vec<String> {
    app.upstream_requests()
        .await
        .iter()
        .map(|r| r.url.path().to_owned())
        .collect()
}

fn enc(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

async fn search(client: &TestClient, q: &str) -> TestResponse {
    client.get(&format!("/api/books/search?q={}", enc(q))).await
}

async fn work_id_of(app: &TestApp, ol_work_id: &str) -> i64 {
    sqlx::query_scalar("SELECT id FROM works WHERE ol_work_id = $1")
        .bind(ol_work_id)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// A shelf entry inserted directly (SHELF owns `POST /api/shelf`).
async fn shelve(app: &TestApp, who: &TestClient, work_id: i64, status: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO shelf_entries (user_id, work_id, status) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(who.id())
    .bind(work_id)
    .bind(status)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

/// A current pick inserted directly (CLUB owns the pick routes).
async fn pick(app: &TestApp, club: &TestClub, work_id: i64) {
    sqlx::query("INSERT INTO club_picks (club_id, work_id) VALUES ($1, $2)")
        .bind(club.id)
        .bind(work_id)
        .execute(&app.db)
        .await
        .unwrap();
}

async fn setup(db: PgPool) -> (TestApp, FakeOpenLibrary, TestClient) {
    let app = TestApp::with_real_lookup(db).await;
    let fake = FakeOpenLibrary::mount(&app).await;
    let ada = app.user("ada").await;
    (app, fake, ada)
}

fn titles(body: &Value) -> Vec<String> {
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["title"].as_str().unwrap().to_owned())
        .collect()
}

/// parity: test_search.py::test_empty_search_serves_trending
/// Adapted: `cover_id` is gone; "no cover" is `cover_url: null`, a cover is same-origin.
/// parity: test_google_books.py::test_browse_and_empty_search_use_google_not_open_library
/// (KEEP: the empty box serves the trending list.)
/// parity: test_google_books.py::test_cover_url_from_image_links_https_and_zoom
/// (KEEP: a cover is never an upstream URL; it is served from our own `/media/` path.)
#[sqlx::test]
async fn empty_search_serves_trending(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    let blank = search(&ada, "   ").await;
    blank.assert_status(200);
    let missing = ada.get("/api/books/search").await;
    missing.assert_status(200);
    let body = missing.json();
    let keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys.len(), 3);
    assert!(keys.contains(&"items") && keys.contains(&"page") && keys.contains(&"has_more"));
    assert_eq!(body["page"], 1);
    assert_eq!(body["has_more"], false);
    assert_eq!(titles(&body), ["Atomic Habits", "No cover"]);
    assert!(body["items"][1]["cover_url"].is_null());
    let cover = body["items"][0]["cover_url"].as_str().unwrap();
    let thumb = body["items"][0]["cover_thumb_url"].as_str().unwrap();
    let public_id = body["items"][0]["cover_public_id"].as_str().unwrap();
    assert_eq!(cover, format!("/media/{public_id}/l"));
    assert_eq!(thumb, format!("/media/{public_id}/s"));
    assert!(params_for(&app, "search.json").await.is_empty());
    assert!(!params_for(&app, "trending").await.is_empty());
}

/// parity: test_search.py::test_open_library_never_receives_a_too_short_query
#[sqlx::test]
async fn open_library_never_receives_a_too_short_query(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    ada.get("/api/books/search").await;
    ada.get("/api/books/search?q=").await;
    ada.get("/api/books/search?q=%20%20").await;
    ada.get("/api/books/search?subject=fantasy").await;
    ada.get("/api/books/search?q=circe").await;
    ada.get("/api/books/trending").await;
    ada.get("/api/books/subjects/fantasy").await;
    let sent = params_for(&app, "search.json").await;
    assert!(!sent.is_empty());
    for params in sent {
        assert_ne!(params["q"], "*");
        assert!(params["q"].chars().count() >= 3, "{params:?}");
    }
}

/// parity: test_search.py::test_search_maps_and_caches
/// Adapted: `on_shelf` is a boolean next to `shelf_id`, and `club_pick` became `club_picks`:
/// the caller's clubs whose current pick the work is (a club the caller is not in never shows).
/// parity: test_google_books.py::test_search_prefers_google_books
/// (KEEP: the search cache key ignores case and spacing.)
#[sqlx::test]
async fn search_maps_and_caches(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    let first = search(&ada, "circe").await;
    first.assert_status(200);
    let body = first.json();
    assert_eq!(body["page"], 1);
    assert_eq!(body["has_more"], true);
    let hits = body["items"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["ol_work_key"], "/works/OL1W");
    assert_eq!(hits[0]["title"], "Circe");
    assert_eq!(hits[0]["authors"], "Madeline Miller");
    assert_eq!(hits[0]["year"], 2018);
    assert_eq!(hits[0]["source"], "openlibrary");
    assert_eq!(hits[0]["custom"], false);
    assert_eq!(hits[0]["on_shelf"], false);
    assert!(hits[0]["shelf_id"].is_null());
    assert_eq!(hits[0]["club_picks"], json!([]));
    let sent = params_for(&app, "search.json").await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["q"], "circe");
    assert!(!sent[0].contains_key("sort"));
    let fields: Vec<&str> = sent[0]["fields"].split(',').collect();
    assert!(fields.contains(&"cover_edition_key"));
    assert!(!fields.contains(&"isbn"));

    search(&ada, "circe").await.assert_status(200);
    assert_eq!(params_for(&app, "search.json").await.len(), 1);
    // Case and extra spaces share the cache key.
    search(&ada, "Circe").await.assert_status(200);
    search(&ada, "  CIRCE  ").await.assert_status(200);
    assert_eq!(params_for(&app, "search.json").await.len(), 1);

    let circe = work_id_of(&app, "OL1W").await;
    assert_eq!(hits[0]["id"], circe);
    let entry = shelve(&app, &ada, circe, "currently_reading").await;
    let marked = search(&ada, "circe").await.json();
    assert_eq!(marked["items"][0]["on_shelf"], true);
    assert_eq!(marked["items"][0]["shelf_id"], entry);
    assert_eq!(marked["items"][0]["club_picks"], json!([]));

    let readers = app.club(&ada, "Readers").await;
    pick(&app, &readers, circe).await;
    let grace = app.user("grace").await;
    let elsewhere = app.club(&grace, "Elsewhere").await;
    pick(&app, &elsewhere, circe).await;
    let picked = search(&ada, "circe").await.json();
    assert_eq!(
        picked["items"][0]["club_picks"],
        json!([{"slug": readers.slug, "name": "Readers"}])
    );
}

/// parity: test_search.py::test_search_forwards_page_subject_sort
#[sqlx::test]
async fn search_forwards_page_subject_sort(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    ada.get("/api/books/search?q=circe&subject=fantasy&sort=new&page=2&limit=12")
        .await
        .assert_status(200);
    let sent = params_for(&app, "search.json").await.pop().unwrap();
    assert_eq!(sent["q"], "circe subject_key:fantasy");
    assert!(!sent.contains_key("subject"));
    assert_eq!(sent["sort"], "new");
    assert_eq!(sent["page"], "2");
    assert_eq!(sent["limit"], "12");
    assert!(!sent.contains_key("lang"));

    ada.get("/api/books/search?subject=science_fiction")
        .await
        .assert_status(200);
    let sent = params_for(&app, "search.json").await.pop().unwrap();
    assert_eq!(sent["q"], "subject_key:science_fiction");
    assert_eq!(sent["lang"], "en");
    assert_eq!(sent["sort"], "readinglog");
}

/// parity: test_search.py::test_search_cache_keyed_by_page
#[sqlx::test]
async fn search_cache_keyed_by_page(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    search(&ada, "circe").await.assert_status(200);
    assert_eq!(params_for(&app, "search.json").await.len(), 1);
    search(&ada, "circe").await.assert_status(200);
    assert_eq!(params_for(&app, "search.json").await.len(), 1);
    ada.get("/api/books/search?q=circe&page=2")
        .await
        .assert_status(200);
    let sent = params_for(&app, "search.json").await;
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1]["page"], "2");
}

/// parity: test_search.py::test_trending_annotates_and_caches
#[sqlx::test]
async fn trending_annotates_and_caches(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    let habits = app.ol_work("OL7W", "Atomic Habits", "James Clear").await;
    shelve(&app, &ada, habits.id, "want_to_read").await;
    let body = ada.get("/api/books/trending?limit=5").await.json();
    assert_eq!(body["items"][0]["id"], habits.id);
    assert_eq!(body["items"][0]["on_shelf"], true);
    assert_eq!(params_for(&app, "trending").await[0]["limit"], "5");

    ada.get("/api/books/trending?limit=5")
        .await
        .assert_status(200);
    assert_eq!(params_for(&app, "trending").await.len(), 1);
}

/// parity: test_search.py::test_subject_maps_its_own_payload_shape
#[sqlx::test]
async fn subject_maps_its_own_payload_shape(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    let body = ada
        .get("/api/books/subjects/Science%20Fiction?limit=10")
        .await
        .json();
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    let hit = &body["items"][0];
    assert_eq!(hit["title"], "Alice's Adventures in Wonderland");
    assert_eq!(hit["authors"], "Lewis Carroll");
    assert!(hit["cover_url"].as_str().unwrap().starts_with("/media/"));
    assert_eq!(hit["year"], 1865);
    let subject_paths: Vec<String> = paths(&app)
        .await
        .into_iter()
        .filter(|p| p.contains("/subjects/"))
        .collect();
    assert!(subject_paths[0].ends_with("/subjects/science_fiction.json"));
    assert_eq!(body["has_more"], true);

    let page_four = ada
        .get("/api/books/subjects/fantasy?page=4&limit=10")
        .await
        .json();
    assert_eq!(page_four["has_more"], false);
    assert_eq!(page_four["page"], 4);
    assert_eq!(
        params_for(&app, "/subjects/").await.pop().unwrap()["offset"],
        "30"
    );
}

/// parity: test_search.py::test_subject_rejects_invalid_key
#[sqlx::test]
async fn subject_rejects_invalid_key(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    ada.get("/api/books/subjects/%21%21%21")
        .await
        .assert_detail(400, "That is not a valid subject.");
    ada.get("/api/books/subjects/sci-fi")
        .await
        .assert_detail(400, "That is not a valid subject.");
    assert!(params_for(&app, "/subjects/").await.is_empty());
}

/// parity: test_search.py::test_upstream_failure_is_a_502
#[sqlx::test]
async fn upstream_failure_is_a_502(db: PgPool) {
    let (_app, fake, ada) = setup(db).await;
    fake.missing("search.json");
    search(&ada, "circe")
        .await
        .assert_detail(502, "Could not search the library right now. Try again.");
}

/// parity: test_search.py::test_identical_inflight_searches_share_one_request
/// (through the API: `circe` and `CIRCE` at the same time make one upstream request)
#[sqlx::test]
async fn identical_inflight_searches_share_one_request(db: PgPool) {
    let (app, fake, ada) = setup(db).await;
    fake.delay(Duration::from_millis(300));
    let (first, second) = tokio::join!(search(&ada, "circe"), search(&ada, "CIRCE"));
    assert_eq!(titles(&first.json()), ["Circe"]);
    assert_eq!(titles(&second.json()), ["Circe"]);
    assert_eq!(params_for(&app, "search.json").await.len(), 1);
}

/// parity: test_search.py::test_open_library_connect_timeout_is_short
/// Reasoned: the connect (8 s) and read (15 s) timeouts are `outbound.rs` constants (W1); what
/// CAT controls is the request policy: one attempt is bounded by 15 s and the throttle wait by
/// 2 s.
#[test]
fn request_policy_is_short() {
    use bookclub_server::outbound::RetryPolicy;
    assert!(RetryPolicy::REQUEST.attempt_timeout <= Duration::from_secs(15));
    assert_eq!(RetryPolicy::REQUEST.max_wait, Duration::from_secs(2));
}

/// parity: test_search.py::test_short_query_is_a_400_and_never_hits_open_library
#[sqlx::test]
async fn short_query_is_a_400_and_never_hits_open_library(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    let it = search(&ada, "it").await;
    it.assert_status(400);
    assert!(it.detail().contains("3 characters"));
    search(&ada, "ab").await.assert_detail(
        400,
        "Need at least 3 characters. Add the author, or paste an ISBN.",
    );
    assert!(params_for(&app, "search.json").await.is_empty());
    // A short query with a subject is allowed.
    ada.get("/api/books/search?q=it&subject=fantasy")
        .await
        .assert_status(200);
    assert_eq!(
        params_for(&app, "search.json").await[0]["q"],
        "it subject_key:fantasy"
    );
}

/// parity: test_search.py::test_upstream_422_is_a_helpful_400
#[sqlx::test]
async fn upstream_422_is_a_helpful_400(db: PgPool) {
    let (_app, fake, ada) = setup(db).await;
    fake.status_for("search.json", 422);
    let res = search(&ada, "circe").await;
    res.assert_status(400);
    assert!(res.detail().contains("3 characters"));
}

/// parity: test_search.py::test_upstream_429_is_a_429_with_retry_hint
#[sqlx::test]
async fn upstream_429_is_a_429_with_retry_hint(db: PgPool) {
    let (_app, fake, ada) = setup(db).await;
    fake.status_for("search.json", 429);
    fake.retry_after("20");
    let res = search(&ada, "circe").await;
    res.assert_detail(429, "The library is busy. Try again in 20 seconds.");
    assert_eq!(res.header("retry-after"), Some("20"));
}

/// parity: test_search.py::test_isbn_query_uses_isbn_field
/// Adapted: an ISBN query goes through the ISBN lookup chain, whose Open Library step is the
/// `isbn:` search; `cover_edition_key` is gone, `isbn` is the normalised ISBN.
/// parity: test_google_books.py::test_isbn_search_uses_google_isbn_field
/// (ADAPT: an ISBN-shaped query, hyphens allowed, is routed to the ISBN lookup, not to text
/// search.)
#[sqlx::test]
async fn isbn_query_uses_the_isbn_lookup(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    let res = search(&ada, "978-0-316-76948-8").await;
    res.assert_status(200);
    let sent = params_for(&app, "search.json").await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["q"], "isbn:9780316769488");
    let hit = &res.json()["items"][0];
    assert_eq!(hit["title"], "Circe");
    assert_eq!(hit["isbn"], "9780316769488");
    // An English ISBN is never asked at the DNB.
    assert!(params_for(&app, "/sru/").await.is_empty());
    // The edition is stored: the same search is now local.
    let again = search(&ada, "9780316769488").await;
    assert_eq!(again.json()["items"][0]["id"], hit["id"]);
    assert_eq!(params_for(&app, "search.json").await.len(), 1);
}

/// parity: test_search.py::test_empty_search_pages_are_not_cached
#[sqlx::test]
async fn empty_search_pages_are_not_cached(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    let first = search(&ada, "zzzzempty").await;
    first.assert_status(200);
    assert_eq!(first.json()["items"], json!([]));
    let first_calls = params_for(&app, "search.json").await.len();
    assert!(first_calls >= 1);
    search(&ada, "zzzzempty").await.assert_status(200);
    assert!(params_for(&app, "search.json").await.len() > first_calls);
}

/// parity: test_search.py::test_title_fallback_when_raw_query_misses
#[sqlx::test]
async fn title_fallback_when_raw_query_misses(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    let res = search(&ada, "obscurexyz").await;
    res.assert_status(200);
    let qs: Vec<String> = params_for(&app, "search.json")
        .await
        .into_iter()
        .map(|p| p["q"].clone())
        .collect();
    assert!(qs.contains(&"obscurexyz".to_owned()));
    assert!(qs.iter().any(|q| q.starts_with("title:obscurexyz")));
    assert!(qs.iter().any(|q| q.starts_with("author:obscurexyz")));
    assert_eq!(res.json()["items"][0]["title"], "Circe");
}

/// parity: test_search.py::test_title_containing_by_uses_raw_query_first
/// Adapted: the fake's work ids are valid Open Library ids (OL77W, OL66W).
#[sqlx::test]
async fn title_containing_by_uses_raw_query_first(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    let res = search(&ada, "Stand by Me").await;
    res.assert_status(200);
    let qs: Vec<String> = params_for(&app, "search.json")
        .await
        .into_iter()
        .map(|p| p["q"].clone())
        .collect();
    assert_eq!(qs[0], "Stand by Me");
    assert!(!qs.iter().any(|q| q.contains("title:\"Stand\"")));
    assert_eq!(res.json()["items"][0]["title"], "Stand by Me");
    assert_eq!(res.json()["items"][0]["ol_work_key"], "/works/OL77W");
}

/// parity: test_search.py::test_local_catalog_hits_survive_open_library_outage
/// Adapted: the user-added work is made by the fixture (POST /api/books/custom is tested with
/// the custom-book routes).
/// parity: test_google_books.py::test_search_google_500_does_not_fall_back
/// (ADAPT: an upstream failure serves what the catalog knows.)
#[sqlx::test]
async fn local_catalog_hits_survive_open_library_outage(db: PgPool) {
    let (app, fake, ada) = setup(db).await;
    let zine = app.work("My Zine").await;
    fake.status_for("search.json", 503);
    let res = search(&ada, "zine").await;
    res.assert_status(200);
    let body = res.json();
    assert_eq!(body["items"][0]["title"], "My Zine");
    assert_eq!(body["items"][0]["custom"], true);
    assert_eq!(body["items"][0]["source"], "user");
    assert_eq!(body["items"][0]["id"], zine.id);
    assert!(body["items"][0]["ol_work_key"].is_null());
    assert_eq!(body["has_more"], false);
}

/// #41 acceptance: local results come first, remote hits are merged without duplicates (by work,
/// so a remote hit of a stored Open Library work is that work), and the page is capped at
/// `limit` (the legacy `limit + 1` merge is fixed).
#[sqlx::test]
async fn local_hits_first_without_duplicates_and_capped(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    let notes = app.work("Circe Reading Notes").await;
    let hit_ids = |body: &Value| -> Vec<i64> {
        body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|hit| hit["id"].as_i64().unwrap())
            .collect()
    };
    // Open Library answers Circe for "notes", which is no local hit for that query: the local
    // hit comes first, and a page of one holds only it.
    let body = search(&ada, "notes").await.json();
    let circe = work_id_of(&app, "OL1W").await;
    assert_eq!(hit_ids(&body), [notes.id, circe], "{body}");
    assert_eq!(body["has_more"], true);
    let capped = ada.get("/api/books/search?q=notes&limit=1").await.json();
    assert_eq!(hit_ids(&capped), [notes.id]);

    // Now Circe is stored: it is a local hit too, and its remote hit is the same work.
    let body = search(&ada, "circe").await.json();
    let ids = hit_ids(&body);
    assert_eq!(ids.len(), 2, "{body}");
    assert!(ids.contains(&notes.id) && ids.contains(&circe));

    // A merged-away work never appears; its survivor does.
    let survivor = app.work("Circe (survivor)").await;
    app.exec(&format!(
        "UPDATE works SET merged_into_id = {} WHERE id = {}",
        survivor.id, notes.id
    ))
    .await;
    let after = search(&ada, "circe").await.json();
    let ids: Vec<i64> = after["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["id"].as_i64().unwrap())
        .collect();
    assert!(!ids.contains(&notes.id));
    assert!(ids.contains(&survivor.id));
}

/// Local search matches substrings, accents and edition titles, and `sort=title` orders the
/// local hits by title.
#[sqlx::test]
async fn local_search_matches_words_substrings_and_accents(db: PgPool) {
    let (app, fake, ada) = setup(db).await;
    fake.status_for("search.json", 500);
    let b = app.work("Brücke der Zeit").await;
    let a = app.work("Amulett der Brücken").await;
    let body = search(&ada, "brucke").await.json();
    let ids: Vec<i64> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["id"].as_i64().unwrap())
        .collect();
    assert!(ids.contains(&a.id) && ids.contains(&b.id), "{body}");
    let sorted = ada
        .get("/api/books/search?q=brucke&sort=title")
        .await
        .json();
    assert_eq!(titles(&sorted), ["Amulett der Brücken", "Brücke der Zeit"]);
    // A two-character needle with a subject still searches the catalog.
    let short = ada.get("/api/books/search?q=Ze&subject=history").await;
    assert_eq!(titles(&short.json()), ["Brücke der Zeit"]);
}

/// Search parameters keep the legacy validation strings.
#[sqlx::test]
async fn search_parameters_are_validated(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    ada.get("/api/books/search?q=circe&sort=")
        .await
        .assert_detail(
            400,
            "Input should be 'readinglog', 'new', 'title' or 'relevance'",
        );
    ada.get("/api/books/search?q=circe&page=0")
        .await
        .assert_detail(400, "Input should be greater than or equal to 1");
    ada.get("/api/books/search?q=circe&limit=41")
        .await
        .assert_detail(400, "Input should be less than or equal to 40");
    ada.get("/api/books/search?q=circe&page=x")
        .await
        .assert_detail(
            400,
            "Input should be a valid integer, unable to parse string as an integer",
        );
    let long = "a".repeat(201);
    search(&ada, &long)
        .await
        .assert_detail(400, "Search is too long");
    search(&ada, &"é".repeat(200)).await.assert_status(200);
    ada.get("/api/books/trending?limit=0")
        .await
        .assert_detail(400, "Input should be greater than or equal to 1");
    ada.get("/api/books/subjects/fantasy?limit=41")
        .await
        .assert_detail(400, "Input should be less than or equal to 40");
    app.anon()
        .get("/api/books/search?q=circe")
        .await
        .assert_detail(401, "Not signed in");
}

/// parity: test_google_books.py::test_empty_google_response_does_not_freeze_retries
/// KEEP: an empty answer (an empty or a missing list) is never cached; the retry reaches
/// Open Library again and a non-empty answer is cached. Search, trending and subjects here; the
/// ISBN case is in catalog_isbn.rs.
#[sqlx::test]
async fn empty_responses_do_not_freeze_retries(db: PgPool) {
    for empty in [
        json!({"numFound": 0}),
        json!({"numFound": 0, "docs": [], "works": []}),
    ] {
        let app = TestApp::with_real_lookup(db.clone()).await;
        // Each round starts with an empty catalog, so the search has no local hits.
        app.exec("DELETE FROM works").await;
        let fake = FakeOpenLibrary::mount(&app).await;
        let ada = app
            .user(&format!("ada{}", empty.as_object().unwrap().len()))
            .await;
        for (path, marker) in [
            ("/api/books/search?q=circe", "search.json"),
            ("/api/books/trending", "trending"),
            ("/api/books/subjects/fiction", "/subjects/"),
        ] {
            for _ in 0..3 {
                fake.once(marker, 200, empty.clone());
            }
            let first = ada.get(path).await;
            first.assert_status(200);
            assert_eq!(first.json()["items"], json!([]), "{path}");
            assert_eq!(first.json()["has_more"], false, "{path}");
            let before = paths(&app).await.len();
            fake.state.lock().unwrap().once.clear();
            let second = ada.get(path).await.json();
            assert!(
                paths(&app).await.len() > before,
                "{path}: the retry went upstream"
            );
            assert!(!second["items"].as_array().unwrap().is_empty(), "{path}");
            let calls = paths(&app).await.len();
            let third = ada.get(path).await.json();
            assert_eq!(
                paths(&app).await.len(),
                calls,
                "{path}: served from the cache"
            );
            assert_eq!(third, second, "{path}");
        }
    }
}

/// parity: test_google_books.py::test_empty_explicit_search_or_later_page_is_not_broadened
/// KEEP: an explicit query or a later page is never replaced by suggestions; only the legacy
/// title and author fallbacks of the query itself run, and only on page 1.
#[sqlx::test]
async fn empty_explicit_search_or_later_page_is_not_broadened(db: PgPool) {
    let (app, fake, ada) = setup(db).await;
    for _ in 0..3 {
        fake.once("search.json", 200, json!({"numFound": 1000, "docs": []}));
    }
    let res = ada
        .get("/api/books/search?q=nonexistent&subject=fiction")
        .await
        .json();
    assert_eq!(res["items"], json!([]));
    assert_eq!(res["has_more"], false);
    let qs: Vec<String> = params_for(&app, "search.json")
        .await
        .into_iter()
        .map(|p| p["q"].clone())
        .collect();
    assert_eq!(
        qs,
        [
            "nonexistent subject_key:fiction",
            "title:nonexistent subject_key:fiction",
            "author:nonexistent subject_key:fiction"
        ]
    );
    assert!(params_for(&app, "trending").await.is_empty());
    assert!(params_for(&app, "/subjects/").await.is_empty());

    fake.once("search.json", 200, json!({"numFound": 1000, "docs": []}));
    let later = ada
        .get("/api/books/search?subject=fiction&page=2")
        .await
        .json();
    assert_eq!(later["items"], json!([]));
    assert_eq!(later["has_more"], false);
    assert_eq!(params_for(&app, "search.json").await.len(), 4);
}

/// parity: test_google_books.py::test_later_google_page_does_not_mix_open_library
/// parity: test_google_books.py::test_later_empty_google_page_has_more_false_when_total_stays_high
/// KEEP: a later page never mixes in other results, and an empty page ends pagination whatever
/// the upstream total says.
#[sqlx::test]
async fn an_empty_later_page_ends_pagination(db: PgPool) {
    let (app, fake, ada) = setup(db).await;
    app.work("Circe Reading Notes").await;
    let first = search(&ada, "circe").await.json();
    assert_eq!(first["has_more"], true);
    fake.once("search.json", 200, json!({"numFound": 50, "docs": []}));
    let second = ada.get("/api/books/search?q=circe&page=2").await.json();
    assert_eq!(second["items"], json!([]));
    assert_eq!(second["has_more"], false);
    assert_eq!(second["page"], 2);
    assert_eq!(params_for(&app, "search.json").await.len(), 2);
}

/// parity: test_google_books.py::test_subject_fallback_keeps_provider_errors_visible
/// ADAPT: an upstream error surfaces classified (429 / 502), never as an empty page; a 500 is
/// not retried.
#[sqlx::test]
async fn subject_errors_stay_visible(db: PgPool) {
    for (status, expected) in [(429, 429), (500, 502)] {
        let app = TestApp::with_real_lookup(db.clone()).await;
        let fake = FakeOpenLibrary::mount(&app).await;
        let ada = app.user(&format!("ada{status}")).await;
        fake.status_for("/subjects/", status);
        let res = ada.get("/api/books/subjects/fiction").await;
        res.assert_status(expected);
        if expected == 502 {
            assert_eq!(
                res.detail(),
                "Could not load the library right now. Try again."
            );
        } else {
            assert!(res.detail().contains("busy"));
        }
        assert_eq!(params_for(&app, "/subjects/").await.len(), 1);
    }
}

/// Decision 11 and deviation 4: after an empty primary query, a failing `title:` or `author:`
/// fallback is ignored when another fallback has hits; when none has hits, the first fallback
/// error surfaces (429 or 502), so an outage is not shown as "nothing found".
/// parity: test_google_books.py::test_subject_fallback_keeps_provider_errors_visible
/// (ADAPT, the "primary empty, then the fallback fails" cells, for the search fallbacks.)
#[sqlx::test]
async fn fallback_errors_surface_only_without_hits(db: PgPool) {
    // `obscurexyz`: the primary query is empty, both fallbacks find Circe. `zzzzempty`: every
    // query is empty.
    type Case<'a> = (&'a str, &'a [(&'a str, u16)], u16);
    let cases: [Case; 3] = [
        ("obscurexyz", &[("title:", 500)], 200),
        ("obscurexyz", &[("title:", 500), ("author:", 500)], 502),
        ("zzzzempty", &[("title:", 429)], 429),
    ];
    for (round, (q, failing, expected)) in cases.into_iter().enumerate() {
        let app = TestApp::with_real_lookup(db.clone()).await;
        let fake = FakeOpenLibrary::mount(&app).await;
        let ada = app.user(&format!("ada{round}")).await;
        for (prefix, status) in failing {
            fake.status_for_q(prefix, *status);
        }
        let res = search(&ada, q).await;
        res.assert_status(expected);
        match expected {
            200 => assert_eq!(titles(&res.json()), ["Circe"]),
            502 => assert_eq!(
                res.detail(),
                "Could not search the library right now. Try again."
            ),
            _ => assert!(
                res.detail().starts_with("The library is busy."),
                "{}",
                res.detail()
            ),
        }
        let sent: Vec<String> = params_for(&app, "search.json")
            .await
            .into_iter()
            .map(|params| params["q"].clone())
            .collect();
        assert_eq!(sent[0], q);
        if expected != 429 {
            // A 429 cools Open Library down, which may stop the other fallback before it is
            // sent; a 500 is not retried.
            assert_eq!(sent.len(), 3, "{sent:?}");
        }
    }
}

/// parity: test_google_books.py::test_search_google_429_does_not_fall_back
/// parity: test_google_books.py::test_google_403_quota_cools_down
/// ADAPT: an upstream 429 answers 429 `The library is busy…`, and Open Library is not asked
/// again while it cools down: a different search fails fast without a request.
#[sqlx::test]
async fn a_429_cools_open_library_down(db: PgPool) {
    let (app, fake, ada) = setup(db).await;
    fake.status_for("search.json", 429);
    let res = search(&ada, "circe").await;
    res.assert_status(429);
    assert!(res.detail().contains("busy"));
    assert_eq!(
        res.detail(),
        "The library is busy. Try again in 60 seconds."
    );
    fake.clear_status();
    let other = search(&ada, "madeline").await;
    other.assert_status(429);
    assert!(
        other
            .detail()
            .starts_with("The library is busy. Try again in")
    );
    assert_eq!(params_for(&app, "search.json").await.len(), 1);
    // Browsing fails fast too.
    ada.get("/api/books/trending").await.assert_status(429);
    assert!(params_for(&app, "trending").await.is_empty());
}

/// parity: test_google_books.py::test_google_403_access_not_configured_is_502
/// ADAPT: only a 429 trips the cooldown; a 403 is a 502 and the next search asks again.
#[sqlx::test]
async fn a_403_is_a_502_without_cooldown(db: PgPool) {
    let (app, fake, ada) = setup(db).await;
    fake.once("search.json", 403, json!({}));
    let res = search(&ada, "circe").await;
    res.assert_detail(502, "Could not search the library right now. Try again.");
    assert!(!res.detail().contains("busy"));
    search(&ada, "circe").await.assert_status(200);
    assert_eq!(params_for(&app, "search.json").await.len(), 2);
}

/// parity: test_google_books.py::test_search_google_500_does_not_fall_back
/// ADAPT: a 5xx with no local hits is 502 `Could not search the library right now. Try again.`;
/// a 500 is not retried.
#[sqlx::test]
async fn a_500_without_local_hits_is_a_502(db: PgPool) {
    let (app, fake, ada) = setup(db).await;
    fake.status_for("search.json", 500);
    search(&ada, "circe")
        .await
        .assert_detail(502, "Could not search the library right now. Try again.");
    assert_eq!(params_for(&app, "search.json").await.len(), 1);
}

/// parity: test_google_books.py::test_google_transient_503_is_retried_once
/// parity: test_google_books.py::test_google_persistent_503_stops_after_one_retry_and_can_recover
/// ADAPT: one retry on 502/503/504 (300 ms later); a persistent 503 is a 502 after two
/// requests, trips no cooldown, and the next request recovers.
#[sqlx::test]
async fn a_503_is_retried_once(db: PgPool) {
    let (app, fake, ada) = setup(db).await;
    fake.once("trending", 503, json!({}));
    let body = ada.get("/api/books/trending").await.json();
    assert_eq!(titles(&body)[0], "Atomic Habits");
    assert_eq!(params_for(&app, "trending").await.len(), 2);
    ada.get("/api/books/trending").await.assert_status(200);
    assert_eq!(params_for(&app, "trending").await.len(), 2);

    fake.status_for("/subjects/", 503);
    ada.get("/api/books/subjects/fiction")
        .await
        .assert_detail(502, "Could not load the library right now. Try again.");
    assert_eq!(params_for(&app, "/subjects/").await.len(), 2);
    fake.clear_status();
    ada.get("/api/books/subjects/fiction")
        .await
        .assert_status(200);
    assert_eq!(params_for(&app, "/subjects/").await.len(), 3);
}

/// parity: test_google_books.py::test_opening_discovered_book_does_not_call_google
/// INVERT (the browse half): every remote hit of a search, trending or subject page is stored,
/// with its cover reference, so a displayed result has a work id. (Opening it is local; the
/// detail tests cover that.)
#[sqlx::test]
async fn browsing_stores_every_hit(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    search(&ada, "circe").await.assert_status(200);
    ada.get("/api/books/trending").await.assert_status(200);
    ada.get("/api/books/subjects/fiction")
        .await
        .assert_status(200);
    for ol in ["OL1W", "OL7W", "OL8W", "OL138052W"] {
        let row = sqlx::query_as::<_, (String, Option<i64>)>(
            "SELECT w.source, m.ol_cover_id FROM works w
             LEFT JOIN media m ON m.id = w.cover_media_id WHERE w.ol_work_id = $1",
        )
        .bind(ol)
        .fetch_one(&app.db)
        .await
        .unwrap_or_else(|e| panic!("{ol}: {e}"));
        assert_eq!(row.0, "openlibrary");
        let expected = match ol {
            "OL1W" => Some(123),
            "OL7W" => Some(999),
            "OL138052W" => Some(10527843),
            _ => None,
        };
        assert_eq!(row.1, expected, "{ol}");
    }
    // Stubs are insert-only: a later page never overwrites a stored work.
    app.exec("UPDATE works SET title = 'Circe (edited)' WHERE ol_work_id = 'OL1W'")
        .await;
    ada.get("/api/books/search?q=circe&page=2")
        .await
        .assert_status(200);
    let title: String = sqlx::query_scalar("SELECT title FROM works WHERE ol_work_id = 'OL1W'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(title, "Circe (edited)");
}

/// Security review 2026-10-28 #5: every user shares Open Library's 3 requests a second, so one
/// account gets 60 catalog requests a minute (search, trending, subjects); another account is
/// not affected.
#[sqlx::test]
async fn catalog_requests_are_limited_per_account(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    let long = "x".repeat(201);
    for _ in 0..60 {
        search(&ada, &long)
            .await
            .assert_detail(400, "Search is too long");
    }
    for path in [
        "/api/books/search?q=circe",
        "/api/books/trending",
        "/api/books/subjects/fiction",
    ] {
        let refused = ada.get(path).await;
        refused.assert_detail(429, "Slow down a little. Try again later.");
        assert!(refused.header("retry-after").is_some());
    }
    let grace = app.user("grace").await;
    search(&grace, "circe").await.assert_status(200);
}

/// Two catalog requests per account at a time; a third waits for a turn instead of claiming
/// more of the shared throttle's slots.
#[sqlx::test]
async fn an_account_has_two_catalog_requests_at_a_time(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    let key = format!("u:{}", ada.id());
    let first = app.state.limits.catalog_gate.enter(&key).await;
    let _second = app.state.limits.catalog_gate.enter(&key).await;
    let waiting = tokio::spawn({
        let ada = ada.clone();
        async move { search(&ada, "circe").await }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!waiting.is_finished(), "the third request waits for a turn");
    let grace = app.user("grace").await;
    search(&grace, "circe").await.assert_status(200);
    drop(first);
    tokio::time::timeout(Duration::from_secs(10), waiting)
        .await
        .unwrap()
        .unwrap()
        .assert_status(200);
}

/// Pages go up to 100: each page is its own upstream call and cache entry.
#[sqlx::test]
async fn pages_stop_at_100(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    for path in [
        "/api/books/search?q=circe&page=101",
        "/api/books/search?q=circe&page=2147483647",
        "/api/books/subjects/fiction?page=101",
    ] {
        ada.get(path)
            .await
            .assert_detail(400, "Input should be less than or equal to 100");
    }
    assert!(paths(&app).await.is_empty());
    ada.get("/api/books/subjects/fiction?page=100")
        .await
        .assert_status(200);
}
