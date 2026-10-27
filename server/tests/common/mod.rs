//! Test harness (docs/phase1/conventions.md §17.2). Frozen after Wave 1: packages put extra
//! helpers in their own test files.
//!
//! - [`TestApp`]: an `AppState` over the `#[sqlx::test]` database with cheap argon2,
//!   [`CapturingEmail`], [`FakeCatalog`] as `lookup`, a `MemMediaStore`, and every upstream URL
//!   pointed at one wiremock server. Fixture helpers create users, clubs, works and editions.
//! - [`TestClient`]: requests through the router with a cookie jar, an optional bearer token
//!   and an optional client IP.
//! - [`visibility`]: the template for `tests/<pkg>_visibility.rs`.
//! - Low-level helpers for tests that need no database or their own configuration:
//!   [`test_config`], [`test_state`], [`test_state_with`], [`unreachable_db`], [`send`], [`get`].
#![allow(dead_code)]

mod catalog;
mod client;
mod email;
pub mod visibility;

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use bookclub_server::auth::password::Passwords;
use bookclub_server::auth::random;
use bookclub_server::auth::session::{self, SessionKind};
use bookclub_server::catalog::CatalogService;
use bookclub_server::catalog::shared::ingest::{
    self, AuthorRecord, EditionRecord, Mode, Source, WorkGrouping, WorkRecord,
};
use bookclub_server::catalog::shared::isbn::parse_isbn;
use bookclub_server::config::Config;
use bookclub_server::email::Email;
use bookclub_server::media::MediaService;
use bookclub_server::media::shared::MemMediaStore;
use bookclub_server::outbound::{HostThrottle, Outbound};
use bookclub_server::ratelimit::RateLimits;
use bookclub_server::{AppState, app};
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use sqlx::{AssertSqlSafe, PgPool};
use tower::ServiceExt;
use wiremock::MockServer;

// Not every test file uses every re-export.
#[allow(unused_imports)]
pub use catalog::{FakeCatalog, FakeError, FakeHit};
#[allow(unused_imports)]
pub use client::{Part, TestClient};
pub use email::CapturingEmail;

/// The password of every user the fixtures create.
pub const PASSWORD: &str = "password1";

/// The application under test, its state and its fakes.
pub struct TestApp {
    pub db: PgPool,
    pub state: AppState,
    /// Every upstream URL (Open Library, its covers, DNB SRU and OAI) points here. An unmatched
    /// request answers 404 and is recorded.
    pub upstream: MockServer,
    pub email: Arc<CapturingEmail>,
    /// `state.lookup`, unless built with [`TestApp::with_real_lookup`].
    pub catalog: Arc<FakeCatalog>,
    pub media: Arc<MemMediaStore>,
    router: Router,
    /// References to the state while no request or background task runs (see `settle`).
    idle_refs: usize,
}

/// A club made by [`TestApp::club`].
#[derive(Debug, Clone)]
pub struct TestClub {
    pub id: i64,
    pub slug: String,
    pub name: String,
}

/// A work made by the fixtures.
#[derive(Debug, Clone)]
pub struct TestWork {
    pub id: i64,
    pub title: String,
}

/// An edition made by the fixtures.
#[derive(Debug, Clone)]
pub struct TestEdition {
    pub id: i64,
    pub work_id: i64,
    pub isbn13: String,
}

impl TestApp {
    /// A DEV_MODE configuration (no Secure cookie, the Vite origin allowed, `SIGNUP_MODE`
    /// invite), cheap argon2, `MemMediaStore`, `CapturingEmail`, `FakeCatalog` as `lookup`, and
    /// every upstream URL pointed at [`TestApp::upstream`]. The outbound throttles allow 1000
    /// requests a second, so tests do not wait for the production spacing.
    pub async fn new(db: PgPool) -> Self {
        Self::build(db, &[], false).await
    }

    /// [`TestApp::new`] with extra environment variables (later entries win), e.g.
    /// `("SIGNUP_MODE", "open")` or a production-like `("DEV_MODE", "false")` set.
    pub async fn with_env(db: PgPool, env: &[(&str, &str)]) -> Self {
        Self::build(db, env, false).await
    }

    /// [`TestApp::new`], but `lookup` is `state.catalog`: the real resolution chain against
    /// [`TestApp::upstream`].
    pub async fn with_real_lookup(db: PgPool) -> Self {
        Self::build(db, &[], true).await
    }

    async fn build(db: PgPool, env: &[(&str, &str)], real_lookup: bool) -> Self {
        let upstream = MockServer::start().await;
        let uri = upstream.uri();
        let sru = format!("{uri}/sru/dnb");
        let oai = format!("{uri}/oai/repository");
        let mut vars: Vec<(&str, &str)> = vec![
            ("OPENLIBRARY_URL", &uri),
            ("OPENLIBRARY_COVERS_URL", &uri),
            ("DNB_SRU_URL", &sru),
            ("DNB_OAI_URL", &oai),
        ];
        vars.extend_from_slice(env);
        let config = Arc::new(test_config(&vars));

        let mut outbound = Outbound::new(&config).expect("outbound client");
        outbound.ol_api = HostThrottle::per_second(1000.0);
        outbound.ol_covers = HostThrottle::per_second(1000.0);
        outbound.dnb_sru = HostThrottle::per_second(1000.0);
        outbound.dnb_oai = HostThrottle::per_second(1000.0);
        let outbound = Arc::new(outbound);
        let email = Arc::new(CapturingEmail::default());
        let media = Arc::new(MemMediaStore::new());
        let fake = Arc::new(FakeCatalog::default());
        let catalog = Arc::new(CatalogService::new(&config, outbound.clone()));
        let media_service = Arc::new(MediaService::new(&config, media.clone(), outbound.clone()));
        let state = AppState {
            db: db.clone(),
            config,
            limits: Arc::new(RateLimits::new()),
            outbound,
            passwords: Arc::new(Passwords::for_tests()),
            email: email.clone(),
            media: media.clone(),
            lookup: if real_lookup {
                catalog.clone()
            } else {
                fake.clone()
            },
            catalog,
            media_service,
        };
        let router = app(state.clone());
        TestApp {
            idle_refs: Arc::strong_count(&state.limits),
            router,
            db,
            state,
            upstream,
            email,
            catalog: fake,
            media,
        }
    }

    pub fn config(&self) -> &Config {
        &self.state.config
    }

    /// A client without a session.
    pub fn anon(&self) -> TestClient {
        TestClient::new(self.router.clone(), None, None)
    }

    /// A signed-in user with the verified address `<username>@example.test` and the password
    /// [`PASSWORD`]. The session is stored directly, so no login limit is touched.
    pub async fn user(&self, username: &str) -> TestClient {
        self.make_user(username, true, false).await
    }

    /// [`TestApp::user`] whose address is not verified.
    pub async fn user_unverified(&self, username: &str) -> TestClient {
        self.make_user(username, false, false).await
    }

    /// [`TestApp::user`] who is an instance admin.
    pub async fn admin(&self, username: &str) -> TestClient {
        self.make_user(username, true, true).await
    }

    async fn make_user(&self, username: &str, verified: bool, admin: bool) -> TestClient {
        let hash = self.state.passwords.hash(PASSWORD).await.unwrap();
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO users (username, email, email_verified_at, password_hash, is_admin)
             VALUES ($1, $2, CASE WHEN $3 THEN now() END, $4, $5)
             RETURNING id",
        )
        .bind(username)
        .bind(format!("{username}@example.test"))
        .bind(verified)
        .bind(hash)
        .bind(admin)
        .fetch_one(&self.db)
        .await
        .unwrap_or_else(|error| panic!("cannot create user {username}: {error}"));
        let mut conn = self.db.acquire().await.unwrap();
        let session = session::create(&mut conn, id, SessionKind::Cookie, "test")
            .await
            .unwrap();
        TestClient::new(
            self.router.clone(),
            Some((id, username.to_owned())),
            Some(session.secret),
        )
    }

    /// A club owned by `owner`, with the slug `<slugified name>-<4 random characters>` and the
    /// time zone UTC.
    ///
    /// Wave 1 step 3 inserts the club and the owner membership directly; step 5 routes it
    /// through `POST /api/clubs` (same signature).
    pub async fn club(&self, owner: &TestClient, name: &str) -> TestClub {
        let slug = format!("{}-{}", slugify(name), random_suffix());
        let mut tx = self.db.begin().await.unwrap();
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO clubs (slug, name, created_by) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(&slug)
        .bind(name)
        .bind(owner.id())
        .fetch_one(&mut *tx)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO club_memberships (club_id, user_id, role) VALUES ($1, $2, 'owner')",
        )
        .bind(id)
        .bind(owner.id())
        .execute(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
        TestClub {
            id,
            slug,
            name: name.to_owned(),
        }
    }

    /// Adds `who` to `club` directly, with `role` `"admin"` or `"member"`.
    pub async fn join(&self, club: &TestClub, who: &TestClient, role: &str) {
        assert!(matches!(role, "admin" | "member"), "role {role}");
        sqlx::query("INSERT INTO club_memberships (club_id, user_id, role) VALUES ($1, $2, $3)")
            .bind(club.id)
            .bind(who.id())
            .bind(role)
            .execute(&self.db)
            .await
            .unwrap();
    }

    /// A user-added work without authors (`ingest::upsert_work`, source `user`).
    pub async fn work(&self, title: &str) -> TestWork {
        let mut conn = self.db.acquire().await.unwrap();
        let id = ingest::upsert_work(
            &mut conn,
            &WorkRecord::new(Source::User, title),
            Mode::Refresh,
        )
        .await
        .unwrap();
        TestWork {
            id,
            title: title.to_owned(),
        }
    }

    /// An Open Library work `ol_id` (`OL…W`) with `authors` (", "-separated).
    pub async fn ol_work(&self, ol_id: &str, title: &str, authors: &str) -> TestWork {
        let record = WorkRecord {
            ol_work_id: Some(ol_id.to_owned()),
            ..work_record(Source::OpenLibrary, title, authors)
        };
        let mut conn = self.db.acquire().await.unwrap();
        let id = ingest::upsert_work(&mut conn, &record, Mode::Refresh)
            .await
            .unwrap();
        TestWork {
            id,
            title: title.to_owned(),
        }
    }

    /// A DNB work with one edition `isbn`, through `upsert_edition_with_work`
    /// (`WorkGrouping::DnbTitleAuthor`, `Mode::Replace`).
    pub async fn dnb_work(
        &self,
        title: &str,
        authors: &str,
        isbn: &str,
    ) -> (TestWork, TestEdition) {
        let isbn13 = parse_isbn(isbn).unwrap_or_else(|| panic!("invalid ISBN {isbn}"));
        let edition = EditionRecord {
            isbn13: Some(isbn13.clone()),
            ..EditionRecord::new(Source::Dnb)
        };
        let mut conn = self.db.acquire().await.unwrap();
        let (work_id, edition_id) = ingest::upsert_edition_with_work(
            &mut conn,
            &work_record(Source::Dnb, title, authors),
            &edition,
            WorkGrouping::DnbTitleAuthor,
            Mode::Replace,
        )
        .await
        .unwrap();
        (
            TestWork {
                id: work_id,
                title: title.to_owned(),
            },
            TestEdition {
                id: edition_id,
                work_id,
                isbn13: isbn13.as_str().to_owned(),
            },
        )
    }

    /// A user-added edition `isbn` of `work` (`ingest::upsert_edition`).
    pub async fn edition(&self, work: &TestWork, isbn: &str) -> TestEdition {
        let isbn13 = parse_isbn(isbn).unwrap_or_else(|| panic!("invalid ISBN {isbn}"));
        let record = EditionRecord {
            isbn13: Some(isbn13.clone()),
            ..EditionRecord::new(Source::User)
        };
        let mut conn = self.db.acquire().await.unwrap();
        let (work_id, id) = ingest::upsert_edition(&mut conn, work.id, &record, Mode::Refresh)
            .await
            .unwrap();
        TestEdition {
            id,
            work_id,
            isbn13: isbn13.as_str().to_owned(),
        }
    }

    /// Every mail sent so far, oldest first.
    pub fn sent_emails(&self) -> Vec<Email> {
        self.email.sent()
    }

    /// The token of the newest mail to `to` (case-insensitive) that carries a `#token=` link.
    /// Panics when there is none.
    pub fn last_link_token(&self, to: &str) -> String {
        self.sent_emails()
            .iter()
            .rev()
            .filter(|mail| mail.to.eq_ignore_ascii_case(to))
            .find_map(|mail| {
                let (_, rest) = mail.text.split_once("#token=")?;
                Some(
                    rest.split_whitespace()
                        .next()
                        .unwrap_or_default()
                        .to_owned(),
                )
            })
            .unwrap_or_else(|| panic!("no mail with a token link to {to}"))
    }

    /// Waits until no background task holds the app state any more: the mail of
    /// `POST /api/auth/password-reset`, which is sent after the answer, or a Goodreads import
    /// job. Every task holds a clone of `AppState`, so the reference count of its shared parts
    /// returns to its idle value. Panics after 10 seconds.
    pub async fn settle(&self) {
        for _ in 0..1000 {
            if Arc::strong_count(&self.state.limits) <= self.idle_refs {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("a background task still runs after 10 seconds");
    }

    /// Every request the upstream server received, e.g. to assert "no outbound call".
    pub async fn upstream_requests(&self) -> Vec<wiremock::Request> {
        self.upstream.received_requests().await.unwrap_or_default()
    }

    /// Runs raw SQL (one or more statements): move time, corrupt rows. Panics on an error.
    pub async fn exec(&self, sql: &str) {
        sqlx::raw_sql(AssertSqlSafe(sql.to_owned()))
            .execute(&self.db)
            .await
            .unwrap_or_else(|error| panic!("{sql}: {error}"));
    }

    /// The single `bigint` a query returns, e.g. `SELECT count(*) FROM users`.
    pub async fn scalar_i64(&self, sql: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.db)
            .await
            .unwrap_or_else(|error| panic!("{sql}: {error}"))
    }
}

fn work_record(source: Source, title: &str, authors: &str) -> WorkRecord {
    WorkRecord {
        author_names: authors.to_owned(),
        authors: authors
            .split(", ")
            .filter(|name| !name.is_empty())
            .map(AuthorRecord::named)
            .collect(),
        ..WorkRecord::new(source, title)
    }
}

fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_end_matches('-').chars().take(50).collect();
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "club".to_owned()
    } else {
        slug.to_owned()
    }
}

fn random_suffix() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut bytes = [0u8; 4];
    random::fill(&mut bytes);
    bytes
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect()
}

/// A DEV_MODE configuration plus `extra` variables (later entries win).
pub fn test_config(extra: &[(&str, &str)]) -> Config {
    let mut pairs: Vec<(String, String)> = vec![
        ("DATABASE_URL".into(), "postgres://unused".into()),
        ("DEV_MODE".into(), "true".into()),
    ];
    pairs.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    Config::from_lookup(|key| {
        pairs
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    })
    .expect("valid test configuration")
}

/// A production-style configuration: what `DEV_MODE=false` requires, with `SMTP_URL` and
/// `EMAIL_FROM` so the smtp transport validates (tests replace the sender anyway).
pub const PRODUCTION_ENV: [(&str, &str); 6] = [
    ("DEV_MODE", "false"),
    ("PUBLIC_URL", "https://books.example.com"),
    ("TRUSTED_PROXIES", "none"),
    ("CONTACT_EMAIL", "ops@example.com"),
    ("SMTP_URL", "smtps://user:pass@smtp.example.eu:465"),
    ("EMAIL_FROM", "Bookclub <noreply@example.com>"),
];

/// The production `AppState` over `db` with `config` and cheap argon2 parameters.
pub fn test_state_with(db: PgPool, config: Config) -> AppState {
    let mut state = AppState::new(db, config).expect("test state");
    state.passwords = Arc::new(Passwords::for_tests());
    state
}

/// [`test_state_with`] and the default test configuration.
pub fn test_state(db: PgPool) -> AppState {
    test_state_with(db, test_config(&[]))
}

/// A pool that never connects, for tests that need no database.
pub fn unreachable_db() -> PgPool {
    PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(500))
        .connect_lazy("postgres://bookclub:bookclub@127.0.0.1:1/bookclub")
        .expect("valid URL")
}

/// A response with its body read.
#[derive(Debug, Clone)]
pub struct TestResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl TestResponse {
    /// The body as JSON; panics with the body when it is not JSON.
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|error| {
            panic!(
                "body is not JSON ({error}): {}",
                String::from_utf8_lossy(&self.body)
            )
        })
    }

    /// The `detail` string of an error body.
    pub fn detail(&self) -> String {
        match self.json().get("detail") {
            Some(Value::String(detail)) => detail.clone(),
            _ => panic!("no detail string in {}", self.text()),
        }
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    /// Every `Set-Cookie` value.
    pub fn set_cookies(&self) -> Vec<String> {
        self.headers
            .get_all("set-cookie")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .map(str::to_owned)
            .collect()
    }

    /// Asserts the status, showing the body on failure.
    #[track_caller]
    pub fn assert_status(&self, status: u16) -> &Self {
        assert_eq!(self.status.as_u16(), status, "body: {}", self.text());
        self
    }

    /// Asserts the status and the exact JSON `{"detail": …}` body.
    #[track_caller]
    pub fn assert_detail(&self, status: u16, detail: &str) -> &Self {
        assert_eq!(self.status.as_u16(), status, "body: {}", self.text());
        assert_eq!(self.json(), serde_json::json!({ "detail": detail }));
        self
    }
}

/// Sends one request through `app`.
pub async fn send(app: Router, request: Request<Body>) -> TestResponse {
    let response = app.oneshot(request).await.expect("infallible");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    TestResponse {
        status,
        headers,
        body,
    }
}

/// `GET path` through `app`.
pub async fn get(app: Router, path: &str) -> TestResponse {
    send(app, Request::get(path).body(Body::empty()).unwrap()).await
}
