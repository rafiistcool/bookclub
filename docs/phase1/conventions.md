# Server conventions (Phase 1)

Every implementer of `server/` follows this file. It fixes the architecture, the shared interfaces and the coding rules, so that work packages can be built in parallel without stepping on each other. The contracts live in three companion files:
- `endpoint-map.md`: the HTTP surface.
- `server/migrations/0001_init.sql`: the schema.
- `legacy-mapping.md`: the importer.

The area specs these documents cite (`auth`, `catalog`, `club`, `diary`, `frontend`, `google`, `platform`, `shelf`) are normative too. Wave 1 copies them into the repo as `docs/phase1/spec/<area>.md`, and every reference in the design documents uses that path.

## 1. Principles

1. **Correctness first, boring code.** Plain SQL with sqlx macros, plain axum handlers, no ORM, no macros of our own, no generic repository layers.
2. **One rule, one place.**
   - Visibility of personal data is `crate::visibility` plus the SQL policy functions (§6.2).
   - Club access is the `ClubMember`/`ClubAdmin`/`ClubOwner` extractors, and for club rows reached by a global id or a query parameter, `clubs::extract::role_in`/`member_by_slug` and `bc_member_clubs` (§5.3).
   - Instance-admin access is one route layer on `/api/admin/*` (§2).
   - Error shape is `crate::error`.
   - Catalog writes are `crate::catalog::shared::ingest` (§7), with field-level provenance.
3. **404, not 403, for anything the caller may not know exists.** That covers other clubs, other users' private items and strangers. 403 is only for "you can see it but your role or authorship does not allow this action".
4. **Never trust client book metadata.** Requests reference works and editions by id; catalog rows change only through the catalog module or the admin queue.
5. **Parity is the default.** Behaviour, status codes and English detail strings match the Python app unless `endpoint-map.md` lists a change. The Python test suite is the checklist (#49).

## 2. Module layout and router

```
server/
  Cargo.toml                      # every Phase 1 dependency is added in Wave 1 (see §18)
  migrations/0001_init.sql        # the whole Phase 1 schema
  .sqlx/                          # committed offline query data (§13)
  src/
    main.rs                       # clap CLI: serve | import-legacy | rematch-legacy | make-admin | beta-invite | openapi
                                  #   | healthcheck | send-test-mail (§16)
    healthcheck.rs                # `bookclub-server healthcheck` (DEP #51; `mod healthcheck;` in main.rs, the binary crate)
    lib.rs                        # AppState, app(state), openapi(): router assembly only
    config.rs                     # Config::from_env (§11)
    error.rs                      # AppError, ApiResult, ErrorBody, error::msg (§4)
    extract.rs                    # ApiJson, ApiQuery, ApiPath, ApiMultipart, Validate, Patch<T> (§4)
    dto.rs                        # the shared DTOs of endpoint-map §0 (§20), frozen
    text.rs                       # Python-compatible strip/split/len, casefold, excerpt, round_half_even (§12)
    tz.rs                         # jiff wrappers: zones, parse_meeting, labels, local dates (§12)
    ratelimit.rs  client_ip.rs    # §10
    outbound.rs                   # reqwest client, per-host throttles, cooldown, retry (§10)
    openapi.rs  spa.rs  health.rs
    serve.rs                      # the server loop (header read timeout) and the request deadline (§3)
    jobs.rs                       # background job runner (§16)
    visibility.rs                 # #46 policy API (§6)
    activity/{mod.rs, record.rs, feed.rs}         # record() is W1, feed.rs is CLUB
    auth/…                        # #39: sessions, tokens, passwords, email flows, account delete/export
    clubs/…                       # #40: clubs, memberships, roles, invites, join/leave/transfer
    email/…                       # EmailSender trait, LogEmail, SmtpEmail, templates (§9)
    catalog/
      shared/{mod,books,ingest,isbn,lookup,work_ref,dnb_marc,merge_rules}.rs   # frozen shared interfaces (W1, §7)
      …                           # everything else: CAT (incl. merge.rs: merge_works)
    media/
      shared/{mod,store,urls}.rs  # frozen shared interfaces (W1, §8)
      …                           # everything else: MED
    harvest/…                     # HAR
    shelf/…                       # SHELF: shelf, favourites, layout, Goodreads job, quotes, search history, personal and club stats
    posts/…                       # POSTS: diary entries and pick discussions, reactions, feeds
    picks/… votes/… overlap.rs    # CLUB
    admin/{mod.rs, invites.rs}    # W1: the /api/admin router and its guard, sign-up invites
    admin/{queue,summary,notify}.rs  # ADM (#54): queue paging, GET /api/admin/summary, the admin notification job (§16)
    admin/{users,notices}.rs      # ADM2 (#54, #55): suspension (§5.3), the decision notices (§9)
    catalog/reports.rs            # ADM2: reports on works and editions readers added, and their decision (§7.2)
    legal.rs                      # LEG (#53): LEGAL_DIR, GET /api/legal/{doc}, terms acceptance (§5.9)
    importer/…                    # IMP
  legal-placeholder/              # LEG: the DEV_MODE legal texts (compiled in with include_str!) and a manifest.json
  tests/
    common/…                      # test harness (W1, frozen, §17)
    <area>_*.rs                   # one or more integration test files per package
    fixtures/…                    # per package: openlibrary/, dnb_sru/, dnb_oai/, dnb_marc/ (W1), images/, legacy/, goodreads/
```

**Router rule.** Every feature module exposes:

```rust
pub fn routes() -> utoipa_axum::router::OpenApiRouter<crate::AppState>
```

This is utoipa-axum's wrapper around `axum::Router<AppState>`. It collects the OpenAPI paths of the handlers it routes. Every routed handler carries `#[utoipa::path]` (§14), so no route can go undocumented.

Modules with instance-admin routes (`admin`, `catalog`, `media`) expose them separately, never inside `routes()`. `admin::admin_routes()` merges `admin::invites` and `admin::summary` (ADM #54), and `admin::users` (ADM2):

```rust
pub fn admin_routes() -> OpenApiRouter<crate::AppState>   // every path starts with /api/admin/
```

`lib.rs` merges all modules once (Wave 1 registers every module, at first with an empty stub `routes()`/`admin_routes()`):

```rust
fn api_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::with_openapi(openapi::ApiDoc::openapi())
        .merge(health::routes()).merge(openapi::routes())     // GET /api/health, GET /api/openapi.json
        .merge(auth::routes()).merge(clubs::routes())
        .merge(catalog::routes()).merge(harvest::routes()).merge(media::routes())
        .merge(shelf::routes()).merge(posts::routes())
        .merge(picks::routes()).merge(votes::routes()).merge(overlap::routes())
        .merge(activity::routes())
        .merge(legal::routes())                               // LEG (#53): GET /api/legal/{doc}
}
fn admin_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(admin::admin_routes()).merge(catalog::admin_routes()).merge(media::admin_routes())
}
pub fn openapi() -> utoipa::openapi::OpenApi { api_router().merge(admin_router()).split_for_parts().1 }
pub fn app(state: AppState) -> axum::Router {
    let (api, _) = api_router().split_for_parts();
    let (admin, _) = admin_router().split_for_parts();
    // One guard for every admin route: a handler that forgets its InstanceAdmin argument is still guarded.
    // axum panics on a route layer over an empty router; admin::admin_routes always has routes.
    let admin = admin.route_layer(axum::middleware::from_extractor_with_state::<InstanceAdmin, AppState>(state.clone()));
    /* api.merge(admin), the SPA/404/405 fallbacks (spa::fallback, spa::method_not_allowed) and the layers of §3,
       then .with_state(state) */
}
```

Admin handlers still take `InstanceAdmin` as an argument (they need the admin's id); the route layer is the guarantee.

`GET /api/health` (`health.rs`) and `GET /api/openapi.json` (`openapi.rs`) are documented routes too, with tag `platform` and operation ids `platform_health` and `platform_openapi`. Two generic W1 tests walk the OpenAPI document (§17.7): anonymous callers get 401 everywhere except a fixed public allow-list, and signed-in non-admins get 404 on every `/api/admin/*` operation.

**Path ownership.**
- Each path string belongs to one package (`endpoint-map.md`).
- Path parameters use the fixed names listed there.
- Handlers in one `routes!(…)` call share one path; that is utoipa-axum's grouping.
- Axum 0.8 path syntax is `/{param}`.

**Inside a module.**
- Handlers are thin. They extract, call an operation function, and map the result to a DTO.
- Operation functions take `&mut PgConnection` (works with a pooled connection or a transaction) and return `ApiResult<T>` or `sqlx::Result<T>`.
- Handlers never call other handlers. Cross-module reuse goes through public operation functions; Wave 2 packages only call the frozen interfaces of other packages (`work-plan.md`).

## 3. AppState, startup, layers

```rust
#[derive(Clone)]
pub struct AppState {
    pub db: sqlx::PgPool,
    pub config: Arc<Config>,
    pub limits: Arc<ratelimit::RateLimits>,
    pub outbound: Arc<outbound::Outbound>,
    pub passwords: Arc<auth::password::Passwords>,   // argon2 params + concurrency semaphore
    pub email: Arc<dyn email::EmailSender>,
    pub media: Arc<dyn media::shared::MediaStore>,
    pub catalog: Arc<catalog::CatalogService>,       // struct lives in a CAT-owned file; CAT adds fields freely
    pub lookup: Arc<dyn catalog::shared::lookup::CatalogLookup>, // = catalog in prod; FakeCatalog in tests
    pub media_service: Arc<media::MediaService>,     // MED-owned state (fetch coalescing, image limits)
}
```

`AppState::new(db: PgPool, config: Config) -> anyhow::Result<AppState>` builds the production state (`FsMediaStore` on `MEDIA_DIR`, `LogEmail` or `SmtpEmail` by `EMAIL_TRANSPORT`, `Passwords::new()`, `lookup` = `catalog`). Tests build theirs through the harness (§17).

The fields are frozen after Wave 1. Packages keep their private state inside `CatalogService` / `MediaService`, or in function-local statics where that is trivially safe. They never add fields.

**Frozen constructors and entry points** (W1 writes the signatures and stubs; the owning package fills the bodies and may add private fields and helpers, never change a signature):

```rust
// catalog/service.rs (CAT)
impl CatalogService { pub fn new(config: &Config, outbound: Arc<Outbound>) -> Self; }   // implements CatalogLookup
// media/service.rs (MED)
impl MediaService {
    pub fn new(config: &Config, store: Arc<dyn MediaStore>, outbound: Arc<Outbound>) -> Self;
    /// Queue Open Library covers for background fetching (bounded queue, drops when full). Called by CAT
    /// after it persisted search, trending or subject hits. W1 stub: no-op.
    pub fn prefetch(&self, media_ids: &[i64]);
}
/// The prefetch worker draining that queue until `shutdown`; started by jobs::spawn_all. W1 stub: returns.
pub async fn media::service::run_prefetch(state: AppState, shutdown: tokio_util::sync::CancellationToken);
// harvest/mod.rs (HAR)
pub async fn run_forever(state: AppState, shutdown: tokio_util::sync::CancellationToken);
// shelf/goodreads.rs (SHELF)
pub async fn resume(state: AppState);
// importer/mod.rs (IMP); `Args` derives clap::Args and is flattened into the CLI by main.rs
pub async fn run(db: PgPool, lookup: Arc<dyn CatalogLookup>, media: Arc<dyn MediaStore>, args: importer::Args)
    -> Result<importer::Report, importer::ImportError>;
pub async fn rematch(db: PgPool, lookup: Arc<dyn CatalogLookup>, args: importer::RematchArgs)
    -> Result<importer::Report, importer::ImportError>;
pub enum ImportError { TargetNotEmpty, Source(String), Upstream(String), Db(sqlx::Error), Other(anyhow::Error) }
```

`main.rs` maps `ImportError::TargetNotEmpty` to exit code 2 (message `target database already has users`) and every other error to exit code 1. Because `Args` and `RematchArgs` are flattened, IMP adds flags without touching `main.rs`.

**Startup (`serve`):**
1. tracing, on stderr (§19)
2. `Config::from_env` (it also reads `LEGAL_DIR`, §5.9)
2a. `FsMediaStore::new(media_dir).check_writable()` (DEP #51): create `MEDIA_DIR`, write and remove a probe file `.write-probe.<16 hex>`, else exit non-zero with `MEDIA_DIR <path> is not writable: <cause>`, so a wrong mount fails before the 60 s connect retry. `import-legacy` runs it too; `rematch-legacy`, `make-admin` and `beta-invite` store no media and skip it.
3. connect the pool, retrying every 2 s for up to 60 s, then exit non-zero
4. `sqlx::migrate!("./migrations").run(&pool)`
5. build `AppState`
6. `jobs::spawn_all(state.clone(), shutdown.clone())`
7. bind
8. `serve::run(listener, app(state), serve::HEADER_READ_TIMEOUT, shutdown)`: what `axum::serve(…into_make_service_with_connect_info::<SocketAddr>())` with graceful shutdown did (every request carries `ConnectInfo<SocketAddr>`; shutdown lets open requests finish and also cancels the jobs), plus hyper's header read timeout of 30 s, which hyper enforces only with a timer and `axum::serve` gives it none. A connection that has not sent a complete request head within 30 s is closed (slowloris). `hyper` and `hyper-util` are direct dependencies for it (§18).

**Layers, outermost first:**
1. tower-http `SetRequestId` / `PropagateRequestId` (`x-request-id`)
2. `SetResponseHeader` `Referrer-Policy: same-origin` on every response (no URL of ours leaks to a third party), and, overriding too, `X-Frame-Options: DENY` and `X-Content-Type-Options: nosniff`; `Content-Security-Policy: frame-ancestors 'none'` only where the answer has no policy yet (`if_not_present`): the SPA fallback sets the full shell policy (`spa::SHELL_CSP`, GATE #55, see "Fallbacks"), which also forbids framing (security review 2026-10-28 #10: a same-site page, such as a sibling subdomain behind the shared proxy, would get the Lax session cookie into a frame of ours and could trick a click on a one-tap action like joining a club). They sit outside the layers below so their own answers (the panic 500, a CORS preflight, the origin check's 403) carry them too. HSTS and `Permissions-Policy` are the reverse proxy's (#51, `deploy/hosted/Caddyfile.example`), which terminates TLS.
3. `TraceLayer`
4. `CatchPanicLayer`, answering 500 `{"detail":"Internal Server Error"}`
4a. the request deadline (`serve::deadline`, `serve::REQUEST_DEADLINE` = 600 s): a request that has not produced its response within 10 minutes, reading its body included, answers 408 `{"detail":"The request took too long"}` and its handler is dropped (the transaction rolls back). Long enough for a 50 MB upload over a slow uplink; a streamed response body (the export) does not count. A client that sends its body one byte at a time holds a task for 10 minutes, not for days.
4b. `json_server_errors`: a plain-text 500 (axum's answer when a response body fails to serialise) becomes the JSON 500, logged at ERROR with axum's message.
5. CORS: only when `CORS_ORIGINS` is set (Phase 4 apps), or for the Vite origin in DEV_MODE
6. the origin check (§5.8)
7. `auth::session::maintain_cookie`: the session extractor cannot touch the response, so it leaves a note in a per-request slot that this layer reads: a cookie that resolved to no live session gets a clearing `Set-Cookie` on the 401, and a cookie session that slid gets its cookie again (§5.1). A handler that sets the session cookie itself (login, logout) wins.
7a. `legal::terms_scope` (LEG #53): runs each request with the configuration's terms version in a tokio task-local, so the frozen `auth::user_out` can fill `UserOut.needs_terms` (§5.9). It only wraps the handlers; its place among the inner layers does not matter.
8. `DefaultBodyLimit::max(1 MiB)`; upload routes override it per route (§4.6)

A `route_layer` on the API and admin routes (so not the SPA fallback) adds `Cache-Control: no-store` to every answer whose handler set none (security review 2026-10-28 #11): API answers carry personal data (the account, shelves, a club's invite codes) and set no `Vary: Cookie`, so a caching proxy keyed without the cookie would hand one user's answer to the next, and a browser would keep them on disk after a logout. `/media` and the export set their own and keep them.

Apart from the header read timeout and the request deadline, there are no global timeouts; handlers bound their outbound calls (§10.3). Per-IP connection caps and tighter timeouts belong to the reverse proxy (#51); a server that faces clients directly (`TRUSTED_PROXIES=none`) has only the two bounds above.

**Fallbacks:**
- An unknown `/api/…` path (and `/api` itself) or `/media/…` path gets 404 `{"detail":"Not found"}`, whatever the method.
- A wrong method gets `method_not_allowed_fallback` → 405 `{"detail":"Method Not Allowed"}`.
- Every other GET or HEAD gets the SPA shell when `STATIC_DIR` is set (`spa.rs`: `ServeDir` for files; `index.html` with `Cache-Control: no-store`; `/assets/*` with `public, max-age=31536000, immutable`; a missing asset is a JSON 404, never the shell). Other methods on those paths get the JSON 405; without `STATIC_DIR` they are all JSON 404s.
- Every answer of the SPA fallback (the shell, `/assets/*`, the other built files, a missing asset's JSON 404) carries `Content-Security-Policy: <spa::SHELL_CSP>` (GATE #55): `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; manifest-src 'self' blob:; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'`. Scripts, styles, pictures, fonts, fetches and the manifest come from the own origin only; `data:` and `blob:` pictures, the `blob:` manifest of `frontend/src/brand.ts` and inline styles are allowed; no plugins, no framing. So `frontend/index.html` has no inline script (the theme boot script is `frontend/public/theme-init.js`), and `frontend/src/thirdParty.test.ts` keeps third-party loads and inline scripts out of the sources. API, `/media` and other JSON answers keep `frame-ancestors 'none'` (layer 2); `tests/platform.rs::the_spa_carries_the_shell_policy` checks both.

## 4. Errors, extraction, validation

### 4.1 The error type

```rust
pub type ApiResult<T> = Result<T, AppError>;

#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    pub detail: Cow<'static, str>,
    pub extra: Option<serde_json::Map<String, serde_json::Value>>, // e.g. 409 Already on your shelf + "item"
    pub retry_after: Option<u64>,                                   // sets Retry-After on 429
}
impl AppError {
    pub fn bad_request(d: impl Into<Cow<'static, str>>) -> Self;
    pub fn unauthorized() -> Self;                 // 401 "Not signed in"
    pub fn forbidden(d: …) -> Self;
    pub fn not_found(d: …) -> Self;
    pub fn conflict(d: …) -> Self;
    pub fn payload_too_large(d: …) -> Self;
    pub fn too_many(d: …, retry_after: Option<u64>) -> Self;
    pub fn bad_gateway(d: …) -> Self;
    pub fn internal(e: impl std::fmt::Display) -> Self;   // logs e at ERROR; detail "Internal Server Error"
    pub fn new(status: StatusCode, d: …) -> Self;          // any other status (405, …)
    pub fn with(mut self, key: &str, value: impl Serialize) -> Self;  // adds an extra top-level field
}
impl IntoResponse for AppError { /* JSON {"detail": …, …extra}; Content-Type application/json */ }
impl From<sqlx::Error> for AppError { /* internal */ }
impl From<anyhow::Error> for AppError { /* internal */ }

/// True when `e` is a unique violation of the named constraint or index (SQLSTATE 23505).
pub fn is_unique_violation(e: &sqlx::Error, constraint: &str) -> bool;
/// True when Postgres aborted the transaction to break a deadlock (SQLSTATE 40P01); see §7.2 "Concurrency".
pub fn is_deadlock(e: &sqlx::Error) -> bool;
```

**Contract.**
- Every non-2xx response is JSON `{"detail": "<English>"}`, except 204/304 with no body.
- `detail` is always a string.
- Never return axum's plain-text rejections or empty 405/413 bodies; the frontend `JSON.parse`s every error.

**Detail strings.**
- Strings shared across modules live in `error::msg`, for example:
  - `NOT_SIGNED_IN`
  - `NO_SUCH_CLUB = "No such club"`
  - `ONLY_ADMINS = "Only club admins can do that"`
  - `ONLY_OWNER = "Only the club owner can do that"`
  - `TOO_MANY_ATTEMPTS = "Too many attempts. Wait a few minutes."`
  - `SLOW_DOWN = "Slow down a little. Try again later."`
  - `NO_MEMBER = "No member with that name"`
  - `NO_SUCH_BOOK = "No such book."`
  - `BOOK_NOT_IN_CATALOG = "That book is not in the catalog"`
  - `NO_ENTRY = "No entry with that id"`
  - `NOT_FOUND = "Not found"`
- The other §11.1 constants are `USERNAME_TAKEN`, `INVITE_INVALID`, `WRONG_CREDENTIALS`, `CURRENT_PASSWORD_WRONG` and `PICK_DIFFERENT_PASSWORD`. W1 also defines the strings several modules return: `INTERNAL` (`Internal Server Error`), `METHOD_NOT_ALLOWED`, `CROSS_SITE_REFUSED`, `VERIFY_EMAIL_FIRST`, `INVALID_TIME_ZONE` (`That is not a valid time zone`), `CLUB_FULL` (`This club is full`), `EDITION_OF_ANOTHER_BOOK` and `MEETING_AT_FORMAT` (`meeting_at must be a date and time`).
- **Every string of endpoint-map §11.1 is an `error::msg` constant from Wave 1 on**, whichever package returns it, so `tests/api_error_copy.rs` passes in every worktree. That includes the strings SHELF and MED return: `PICTURE_TOO_LARGE`, `PICTURE_TOO_MANY_PIXELS`, `PICTURE_BAD_TYPE`, `PICTURE_NOT_AN_IMAGE`, `FAVOURITE_TWICE`, `AT_MOST_3_FAVOURITES`, `ALREADY_3_FAVOURITES`, `READING_DATE_NEEDS_FINISHED`, `READING_DATE_IN_FUTURE`, `SEARCH_HISTORY_NOT_FOUND`. SHELF and MED use these constants; they never define their own copy.
- Other strings used by one module are `const`s at the top of that module.
- Strings are copied **byte for byte** from `endpoint-map.md` and the specs, including the EN DASH in the username message and the trailing period in `No such book.`.
- `tests/api_error_copy.rs` checks that every string mapped in `frontend/src/i18n/apiErrors.ts` appears in `server/src` (§17.6).

### 4.2 Order of checks (every authenticated endpoint)

1. **Auth.** `CurrentUser` (or a club extractor) comes **first** in the handler's argument list. axum runs `FromRequestParts` extractors before the body extractor, so a signed-out request is always 401, even with malformed JSON. This is a reasoned change: Python returned 400 for malformed JSON before checking auth.
2. Club membership (404), then role (403), for club routes.
3. Path, query and body extraction and validation (400).
4. Handler logic in the documented order (existence 404 → ownership 403/404 → state 400/409).

The specs list several endpoints where validation runs before a 404. Keep those: the handler validates the body before it loads anything.

### 4.3 `ApiJson<T>`

`ApiJson<T>` is the only JSON body extractor. Never use `axum::Json` for input; `Json` for output is fine.

1. An empty body gives 400 `Field required`, whatever the content type (checked first, parity with FastAPI). A body over the route's `DefaultBodyLimit` gives 413 `Request body is too large`.
2. The `Content-Type` must be `application/json` (parameters allowed). Otherwise 400 `Input should be a valid dictionary or object to extract fields from`. Unparsable JSON gives 400 `JSON decode error`. A non-object gives 400 `Input should be a valid dictionary or object to extract fields from`. A string anywhere in the body, object keys and unknown keys included, that contains U+0000 gives 400 `Invalid value`: Postgres `text` and `jsonb` cannot store it, and the statement would fail with a 500 (a map-typed field, such as an edit request's `fields`, stores its keys too).
3. The value is deserialised with `serde_path_to_error`. The first serde error maps to a pydantic-style message:

   | serde error | detail |
   |---|---|
   | missing field | `Field required` |
   | expected a string | `Input should be a valid string` |
   | expected an integer, or got a float with a fraction | `Input should be a valid integer` |
   | expected a bool | `Input should be a valid boolean` |
   | expected a sequence | `Input should be a valid list` |
   | anything else | `Invalid value` |

   serde stops at the first type error, so only one is reported. That is a reasoned change from pydantic, which reported them all; custom validation messages are still all collected (step 4).
4. `T: Validate` runs. It normalises in place (trim, collapse) and collects **every** custom message in **field declaration order**. If there are any, the response is 400 with the messages joined by `"; "`.
5. Unknown keys are ignored (no `deny_unknown_fields`).

```rust
pub trait Validate { fn validate(&mut self, v: &mut Validator) {} }   // default: nothing to check
pub struct Validator { messages: Vec<String> }
impl Validator {
    pub fn new() -> Self; pub fn messages(&self) -> &[String];
    pub fn into_result(self) -> ApiResult<()>;                          // 400, messages joined by "; "
    pub fn fail(&mut self, msg: impl Into<String>);
    pub fn check(&mut self, ok: bool, msg: &str);
    pub fn range_i64(&mut self, value: Option<i64>, min: i64, max: i64);  // pydantic wording:
    // "Input should be greater than or equal to {min}" / "Input should be less than or equal to {max}"
}
```

Strict JSON numbers and booleans are a reasoned change: pydantic accepted `"50"`, `true` and `50.0`. Status codes stay 400.

A route whose JSON body is optional (ADM's `DELETE /api/admin/media/{public_id}`, #54) uses `media::reports::OptionalJson<T>`: an empty body or the JSON `null` is `None`, anything else goes through `ApiJson` with its 400s. It may move to `extract.rs` when a second route needs it.

### 4.4 Tri-state fields

PATCH bodies tell absent from `null` apart with `extract::Patch<T>`:

```rust
#[derive(Default)] pub enum Patch<T> { #[default] Missing, Null, Value(T) }
// Deserialize: Option<T> → Null | Value. Use with #[serde(default)].
// impl Patch<T> { pub fn is_missing(&self) -> bool; pub fn into_option(self) -> Option<Option<T>>; pub fn as_option(&self) -> Option<Option<&T>>; }
// utoipa: #[schema(value_type = Option<T>)]  (document as nullable, optional)
```

Use it exactly where the specs say presence matters:
- shelf PATCH: `rating`, `take`, `dnf_reason`, `progress`, `finished_on`, `edition_id`
- post PATCH: `spoiler_upto`
- quote PATCH: `page`
- pick PUT/PATCH: `meeting_at`, `note`
- vote PATCH: `closes_at` (absent = null = clear, parity)
- milestone PATCH: every field

Where the specs say `null` means "unchanged", use a plain `Option<T>`.

### 4.5 Query and path

- **`ApiQuery<T>`** (`T: DeserializeOwned + Validate`) maps rejections the same way: a missing field is `Field required`, a non-integer `Input should be a valid integer, unable to parse string as an integer`, a bad boolean `Input should be a valid boolean, unable to interpret input`. Boolean query parameters use `#[serde(default, deserialize_with = "crate::extract::lax_bool")]`, which accepts `true/false/1/0/yes/no/on/off/t/f/y/n` in any case (parity for `include_reading`). Integer bounds are checked in `Validate` with the pydantic wording. The message strings are `pub const`s in `extract.rs` (`FIELD_REQUIRED`, `INTEGER_PARSE`, …).
- **`ApiPath<T>`:** a non-numeric id gives 400 `Input should be a valid integer, unable to parse string as an integer`. A numeric id that does not exist is a 404 from the handler. A numeric id that overflows its type is rejected by `ApiPath` itself with 404 `Not found` (the extractor cannot know the handler's string; no such row can exist).
- **NUL characters:** a query key or value, or a string path value, that decodes to a string with U+0000 (`%00`) gives 400 `Invalid value` from `ApiQuery`/`ApiPath`, as in a JSON body (§4.3). The club extractors answer a `{slug}` with U+0000 with 404 `No such club`, and `visibility::visible_user` treats such a name as unknown (404 `No member with that name`). Multipart text fields and filenames are not checked: an upload handler that stores one must reject U+0000 itself.

### 4.6 Multipart and body limits

- Upload routes set `DefaultBodyLimit::max(50 * 1024 * 1024 + 64 * 1024)` for the avatar and cover routes, and `2 * 1024 * 1024` for the Goodreads import.
- Avatar and cover handlers first check `Content-Length`: above 52,432,896 bytes gives 413 `That picture is too large (50 MB max)`.
- An oversize part found while reading gives 400 with the same detail (parity).
- Use `ApiMultipart`, which maps every multipart rejection to a 400 with the route's documented string.
- The field name is always `file`.

```rust
// extract.rs (W1, frozen)
pub struct ApiMultipart(pub axum::extract::Multipart);   // its own rejection: 400 "Field required"
pub struct UploadedFile { pub filename: String, pub content_type: Option<String>, pub bytes: bytes::Bytes }
pub struct FormFields(pub HashMap<String, String>);      // the other text fields, e.g. the cover's edition_id
impl ApiMultipart {
    /// Reads every part. Returns the `file` part plus the text fields. A missing `file` part, a malformed
    /// body or a part over `max_bytes` → 400 with `bad_upload` (the route's documented string, e.g.
    /// error::msg::PICTURE_TOO_LARGE for images, the Goodreads parse string for CSV).
    pub async fn file(self, max_bytes: usize, bad_upload: &'static str) -> ApiResult<(UploadedFile, FormFields)>;
}
```

## 5. Auth (#39)

### 5.1 Cookie sessions

- **Login** (and register, and token creation) generates 32 random bytes and stores `sha256(bytes)` in `sessions.token_hash`. Randomness everywhere (session and email tokens, public ids, invite codes, slugs) comes from one W1 helper, `auth::random::fill(&mut [u8])`, which uses `rand::TryRng::try_fill_bytes(&mut rand::rngs::SysRng, …)` (rand 0.10 has no `OsRng`; `SysRng` is the OS source and is fallible: `fill` panics on an OS failure, which the `CatchPanicLayer` turns into a 500).
  - The cookie value is `base64url_nopad(bytes)`.
  - The cookie is `__Host-bookclub_session=<value>; Path=/; HttpOnly; SameSite=Lax; Max-Age=34560000; Secure`. With `COOKIE_SECURE=false` (the DEV_MODE default) it is `bookclub_session=<value>` without `Secure`, because browsers accept the `__Host-` prefix only on a `Secure` cookie. `session::cookie_name(config)` picks the name; `auth::SESSION_COOKIE` and `auth::SECURE_SESSION_COOKIE` are the two.
  - **Cookie tossing.** A page on a sibling subdomain can set a cookie for the parent domain, which the browser then sends next to ours, and a last-wins parser would sign the victim in as the attacker. The `__Host-` prefix makes the browser refuse any such cookie under that name (it must come from this host, with `Path=/` and no `Domain`). `session::presented` therefore reads the cookie by its **raw** name (no percent-decoding: the browser checks the prefix on the raw name, so `%5F%5FHost-bookclub_session` is another cookie to it), and a request that carries the session cookie more than once presents a malformed cookie (401 and a clearing cookie, never "the last one wins").
- **Idle expiry slides.** On every authenticated request, when `last_used_at < now - 1 hour`, the extractor sets `last_used_at = now` and `expires_at = now + 30 days` (cookie) or `+ 90 days` (bearer). The cookie's Max-Age is fixed at 400 days, the browser maximum; the server decides validity. When a cookie session slides, the response carries the same cookie again (`maintain_cookie`, §3), so the browser's 400 days restart too.
- **Logout** deletes the row and sends a clearing cookie, but only when a valid session existed (parity). A logout that presents a bearer token deletes that token's session (no cookie is sent).
- **Login, register and token creation are the only places that create sessions,** apart from a password change, which replaces the caller's cookie session (see Revocation). Login and register called with a cookie session delete that session (they switch the browser to the new one, parity with the legacy app); token creation leaves the caller's cookie alone.
- **A cookie that does not resolve** (unknown or expired session, deleted user, a value that is not base64url of 32 bytes) gets 401 plus a clearing `Set-Cookie` (through `maintain_cookie`). Only a 401 clears it: a public route that reads the user optionally (`MaybeUser`) answers as usual.
- **Mechanics** live in `auth::session` (W1):

  ```rust
  pub enum SessionKind { Cookie, Bearer }            // sessions.kind; serialised "cookie" | "bearer"
  pub struct NewSession { pub id: i64, pub secret: String /* cookie value or "bc_…" */, pub label: String,
                          pub created_at: OffsetDateTime, pub last_used_at: OffsetDateTime }
  pub enum Presented { Nothing, Bearer(Option<[u8; 32]>), Cookie(Option<[u8; 32]>) }  // None = malformed
  pub fn cookie_name(config: &Config) -> &'static str;     // __Host-bookclub_session when Secure, else bookclub_session
  pub fn presented(config: &Config, headers: &HeaderMap) -> Presented;  // bearer header first, then the cookie by its raw name;
                                                            // the cookie twice → Cookie(None)
  pub async fn create(conn, user_id: i64, kind: SessionKind, label: &str) -> sqlx::Result<NewSession>;
  pub async fn delete_by_secret(conn, secret: &[u8; 32], kind: SessionKind) -> sqlx::Result<bool>; // true: it was live
  pub fn token_hash(secret: &[u8; 32]) -> Vec<u8>;         // sessions.token_hash
  pub fn set_cookie(config: &Config, value: &str) -> HeaderValue;
  pub fn clear_cookie(config: &Config) -> HeaderValue;
  pub fn device_label(headers: &HeaderMap) -> String;      // the User-Agent, the label of a cookie session
  pub async fn maintain_cookie(State<AppState>, Request, Next) -> Response;   // the layer of §3
  ```

  Labels are cut to 200 characters. Tests create signed-in users with `session::create` (§17.2), so no login limit is touched.
- **Revocation:**
  - A password change deletes every other session and token of the user. A cookie session that changes the password is replaced too: the transaction deletes it and creates a new one with the same label (`session::create`), and the answer carries its `Set-Cookie`, so a copy of the old cookie (another browser profile, an extension) stops working; that copy shares the row, so the session list could never show it. A bearer token that changes the password keeps its secret, because the app holds it (security review 2026-10-28 #7).
  - A password reset deletes all of them.
  - A suspension deletes all of them (ADM2, §5.3).
  - Account deletion cascades.
- An hourly job (§16) deletes expired sessions and used or expired email tokens.
- **A session that ends before it expires** (a logout, a revoked session, a password change or reset) leaves its `last_used_at` on the account as `users.last_seen_at` (LEG2, migration 2201): the trigger `sessions_keep_last_use` (`AFTER DELETE ON sessions`, only when `expires_at > now()`) writes it when it is newer. Its `UPDATE` takes the users row `FOR NO KEY UPDATE SKIP LOCKED`, so a logout never waits for a users row that a password change, a reset, an account deletion or the purge holds, and cannot deadlock with them; a transaction that holds the row itself (a password change deleting the other sessions) is not skipped. An expired session writes nothing, so the hourly cleanup stays a plain delete. Code that deletes sessions needs nothing new. The purge of empty sign-ups (§16) counts `last_seen_at` as a sign-in.

### 5.2 Bearer tokens

- `POST /api/auth/tokens` returns `bc_<base64url(32 bytes)>`, stored in the same table with `kind='bearer'` (the hash is of the 32 bytes, as for cookies). Its response `{token, session: SessionOut}` has `session.current = false`: the request that created it did not use it.
- A request that presents a bearer header is decided by it alone: a malformed or unknown token is 401 even next to a valid cookie, and that cookie is not cleared. A cookie value is never accepted as a bearer token and vice versa (`kind` is part of the lookup).
- `GET /api/auth/sessions` lists the caller's live sessions (`expires_at > now`), most recently used first (`last_used_at DESC, id DESC`); `current` marks the one that authenticated the request.
- The extractor reads `Authorization: Bearer bc_…` **before** the cookie. A bearer request never needs the origin check (§5.8).
- Phase 4 device tokens (APNs/FCM) will reference `sessions.id` from a new table.

### 5.3 Extractors

All of them live in `auth::extract` and `clubs::extract` (W1, frozen).

```rust
pub struct CurrentUser {
    pub id: i64, pub username: String, pub is_admin: bool, pub email_verified: bool,
    pub locale: String, pub timezone: String, pub session_id: i64,
}
pub struct MaybeUser(pub Option<CurrentUser>);  // None when signed out or for a dead cookie (not cleared);
                                                // a failed session lookup (500) rejects instead of None
pub struct VerifiedUser(pub CurrentUser);   // 403 "Verify your email address first"
pub struct InstanceAdmin(pub CurrentUser);  // 404 "Not found" for everyone else

pub enum Role { Member, Admin, Owner }       // ordered: Member < Admin < Owner
pub struct ClubCtx { pub id: i64, pub slug: String, pub name: String, pub timezone: String }
pub struct ClubMember { pub user: CurrentUser, pub club: ClubCtx, pub role: Role }
pub struct ClubAdmin(pub ClubMember);        // role >= Admin, else 403 ONLY_ADMINS
pub struct ClubOwner(pub ClubMember);        // role == Owner, else 403 ONLY_OWNER
```

- One query joins `sessions` and `users` for `CurrentUser`.
- **A suspended account has no live session** (ADM2, #55, Q15; migration 2102). That query takes only accounts with `users.suspended_at IS NULL`, so a cookie or bearer session of a suspended account is 401 `Not signed in`, a cookie with the clearing `Set-Cookie` like a dead one, and `MaybeUser` treats it as signed out. The suspension itself deletes every session and bearer token of the account (`admin::users::suspend`, `POST /api/admin/users/{username}/suspend`): one transaction that locks the user row `FOR NO KEY UPDATE` (the lock `auth::login::confirm` takes, §5.4), sets `suspended_at = coalesce(suspended_at, now())` (a second suspension keeps the first time) and deletes the account's `sessions` rows. A login that committed first loses its session there; one that commits later waits for the row lock and is refused with 403 (§5.4). The extractor's check is the backstop for a session that slipped through anyway. A suspended instance admin is refused by every admin route like anyone signed out, and `admin::notify` mails only admins who are not suspended (§16). An admin cannot suspend their own account (400 `You cannot suspend your own account`); `unsuspend` clears the column and, when the account was suspended, sets `users.unsuspended_at` to now (migration 2900). The content of a suspended account stays as it is, and the empty sign-up purge never deletes it; it counts the lifting of a suspension as a sign-in, since the account could not sign in before (§16).
- The club extractors read the `slug` path parameter through `axum::extract::RawPathParams` and call `member_by_slug`. They return 404 `No such club` for a missing club **and** for a non-member. A path with any parameter that is not valid UTF-8 after percent-decoding (`/api/clubs/%ff/…`, `/api/clubs/{slug}/picks/%ff`) is also 404 `No such club`: axum then reads none of the parameters, and such a path names nothing the caller can see.

**Club helpers** (`clubs::extract`, W1, frozen; the extractors are built on them, so there is one implementation):

```rust
/// The caller's membership of the club with this slug. 404 NO_SUCH_CLUB for an unknown slug or a non-member.
/// For `?club=<slug>` query parameters (GET /api/diary?club=), which RawPathParams cannot read.
pub async fn member_by_slug(conn: &mut PgConnection, user: &CurrentUser, slug: &str) -> ApiResult<ClubMember>;
/// The user's role in a club, None for a non-member. For club rows reached by a global id
/// (/api/posts/{post_id} on a club post, a club reply created through the diary route).
pub async fn role_in(conn: &mut PgConnection, user_id: i64, club_id: i64) -> sqlx::Result<Option<Role>>;
/// The ClubRef (slug, name) of each existing club id, for DTO annotation. Checks nothing: callers pass only
/// club ids already restricted to the viewer (`ClubMember`, `member_by_slug`, `role_in`, or `JOIN bc_member_clubs($viewer)`).
pub async fn club_refs(conn: &mut PgConnection, club_ids: &[i64]) -> sqlx::Result<HashMap<i64, ClubRef>>;
```

`auth::user_out(conn: &mut PgConnection, user_id: i64) -> ApiResult<UserOut>` (W1, frozen) builds the full `UserOut` including `clubs`; MED's avatar endpoints return it. `pending_email` is the address of the newest open verification link (highest `id`, §5.5) of an account whose own address is verified (an email change waiting for confirmation), else null; `clubs` are ordered newest join first; a user that no longer exists is 401 `Not signed in`. `needs_terms` (LEG #53) is true while the operator's terms are configured and `users.terms_version` is not their current version (imported accounts, sign-ups from before the terms, everyone after a new version); `user_out` has no configuration, so it reads the version through `legal::needs_terms` from the task-local that `legal::terms_scope` sets per request (§3 layer 7a; false outside a request, §5.9).

- **Club data rule:**
  - On `/api/clubs/{slug}/…` routes, every query on club data takes `club_id` **only** from the `ClubMember` extractor.
  - On routes that reach a club row by a global id (`/api/posts/{post_id}`, a reply whose `parent_id` is a club post), `club_id` comes **from the row**, and the caller must pass `role_in(conn, me, club_id)`; `None` is that route's 404 (`No entry with that id` for posts). Moderation checks use the role **in that row's club**, never "admin anywhere".
  - On routes with a `?club=<slug>` parameter, the club comes from `member_by_slug`.
  - List queries that mix clubs (the home feeds, `club_picks` on search and detail) restrict club rows with `JOIN bc_member_clubs($viewer) AS mc ON mc.club_id = x.club_id`.
  - Every child id from the path or the body is resolved with `WHERE club_id = $1 AND id = $2`. An id from another club is then indistinguishable from a missing one.
  - Never accept a `club_id` from a request.
- **Membership writes** (remove a member, change a role, leave, transfer, join, accept an invite) and the invite writes (create, revoke) never trust the role the extractor read before the transaction. Each one:
  1. opens a transaction and locks the club row (`SELECT … FROM clubs WHERE id = $1 FOR UPDATE`);
  2. re-reads the caller's role **and** the target's role inside the transaction (a caller who lost the needed role meanwhile gets the 403 or 404 that applies now);
  3. puts the guard into the write itself, and treats 0 affected rows as the documented 403/404:
     - remove by an admin: `DELETE FROM club_memberships WHERE club_id = $1 AND user_id = $2 AND role = 'member'`; by the owner: `… AND role <> 'owner'`;
     - role change: `UPDATE … SET role = $3 WHERE club_id = $1 AND user_id = $2 AND role <> 'owner'`;
     - leave: `DELETE … WHERE club_id = $1 AND user_id = $me AND role <> 'owner'` (0 rows and the caller is the owner → 409 `Transfer ownership before leaving`);
     - transfer: demote the caller (`… AND role = 'owner'`, 0 rows → 403 ONLY_OWNER), then promote the target (`… AND user_id = $2`, 0 rows → 404 NO_MEMBER).
  
  The deferred constraint triggers `clubs_have_owner`/`club_memberships_keep_owner` are the backstop: a transaction that would leave an existing club without an owner fails at COMMIT. Tests that create clubs with raw SQL must insert the owner row in the same transaction.

  **Wave 1 step 5 implementation** (`clubs::{manage, members, invites, redeem, out}`):
  - `clubs::lock_club(conn, club_id) -> sqlx::Result<bool>` takes the club-row lock (false: the club is gone); `clubs::locked_role(conn, user_id, club_id) -> ApiResult<Role>` takes it and re-reads the caller's role; `clubs::member_named(conn, club_id, username)` reads a member's id and role inside the transaction. Inside the lock, a caller who is no longer a member gets 404 `No such club`.
  - Removal: the target is looked up first (404 `No member with that name` for an unknown name or a non-member); removing yourself is 400 `Use leave to leave the club`; when the in-statement guard matches no row (an admin removing an admin or the owner) the answer is 403 `Only the club owner can do that`. A removal upserts `club_removals` with `removed_at = clock_timestamp()`, the time under the club lock: `now()` is the start of the transaction, which can precede an invite that took the lock first. Under the same lock it revokes (`revoked_at = clock_timestamp()`) every **active** invite of the club the person knows, so a second account cannot walk back in with one: the invites they created or redeemed (`club_invite_uses`), and, when the target is an admin (admins list every invite), all of them. A member who was demoted from admin also knows every invite created up to the demotion: a demotion of an admin sets `club_memberships.admin_until = clock_timestamp()` (under the club lock, so it orders with `insert_invite`'s dates), and the removal (`DELETE … RETURNING admin_until`) revokes every active invite with `created_at <= admin_until` too (security review 2026-10-28 #9; the residual of Wave 1 security #5). Expired and used-up invites keep their state. What the server cannot know stays: a link someone forwarded to a plain member, and the codes an admin saw before leaving the club and coming back (the membership row, and with it `admin_until`, goes on leave); the admins revoke those (they see every invite with its creator). A role change revokes nothing: a demoted admin's links stay the club's invites, which the other admins see and can revoke.
  - Invite create and revoke (`clubs::invites`) take the club lock through `locked_role` before they write: 404 `No such club` for a caller removed meanwhile, 403 `Only club admins can do that` for one demoted. `insert_invite` dates the invite with `clock_timestamp()`. A removal and a club invite are therefore dated in the order they held the lock, which is what `join` compares: an admin removed while their invite request waited gets no code to walk back in with, and an invite created after a removal works for the removed person.
  - Role change: `{"role": "owner"}` and a target who is the owner (the caller) are 400 `Use transfer to change the owner`; any other role than `admin`/`member` is 400 `Invalid value`.
  - Transfer to yourself is a harmless no-op (demote, then promote the same row).
  - `PATCH /api/clubs/{slug}` and `DELETE /api/clubs/{slug}` also put the role check into the statement (`AND EXISTS (… role IN ('owner','admin'))`, resp. `role = 'owner'`); the delete runs under the club lock. 0 rows → 404 `No such club` or the 403 that applies now (`clubs::manage::refusal`).

### 5.4 Passwords

- **Hashing.** The `argon2` crate (0.6) uses `Argon2id`. Production parameters are the crate default (`m = 19456 KiB, t = 2, p = 1`, OWASP).
- **Verification** parses the PHC string with `argon2::password_hash::phc::PasswordHash::new` (argon2 0.6; the old `password_hash::PasswordHash` path is a deprecated alias and fails clippy `-D warnings`) and verifies with the parameters stored in it, so legacy argon2-cffi hashes (`m=65536,t=3,p=4`) verify unchanged. An unparsable hash counts as a mismatch, never a 500. Hashing is `Argon2::default().hash_password(pw.as_bytes())`: argon2 0.6 generates the salt itself.
- **Rehash on login.** After a successful login, a hash whose parameters differ from the current ones is re-hashed and stored, inside the login transaction.
- **Blocking work** (hash and verify) runs in `tokio::task::spawn_blocking`, behind a `tokio::sync::Semaphore` of 4 permits held by `Passwords`. That bounds memory: legacy hashes use 64 MiB each. The semaphore is an `Arc<Semaphore>` and each operation moves an owned permit into its blocking task, so a request cancelled by a disconnecting client cannot free its permit while the hash still runs. An operation waits at most 10 s for a permit and then fails with `password::Busy`, which `AppError` answers with 503 `The server is busy. Try again in a moment.` (`msg::SERVER_BUSY`; from `hash`'s `anyhow::Error` too, by downcast), so a flood cannot hold every login of the instance in an unbounded queue (security review 2026-10-28 #1).
- **API** (`auth::password`, W1):

  ```rust
  pub struct Passwords { pub params: argon2::Params, pub permits: Arc<Semaphore>, /* the dummy hash */ }
  pub enum Verdict { Mismatch, Match { rehash: bool } }    // rehash: not argon2id v19 with params' m, t, p
  impl Passwords {
      pub fn new() -> Self; pub fn for_tests() -> Self;
      pub async fn hash(&self, password: &str) -> anyhow::Result<String>;            // a PHC string; Busy inside the error
      /// None = unknown account: verifies against a dummy hash made with `params`, always Mismatch.
      pub async fn verify(&self, password: &str, stored: Option<&str>) -> Result<Verdict, Busy>;
  }
  ```
- **Login order** (`auth::login::authenticate`, shared by login and token creation): `login_ip.check_and_hit(ip_key)` → look the account up → `check_and_hit` of the identifier's failure key (`login_failures`: `u:<id>` for a username, `u:<id>:email` for a verified address; `login_unknown`: `unknown_identifier_key` for an identifier that names no account) → verify → on success `clear` the key and, when `rehash`, hash again. The transaction that creates the session starts with `auth::login::confirm`: it locks the user row (`SELECT password_hash … FOR NO KEY UPDATE`; not `FOR SHARE`, so two rehashing logins of one account cannot deadlock), answers 401 `Wrong username or password` when the hash is no longer the one verified, answers 403 `This account is suspended` (`msg::ACCOUNT_SUSPENDED`, ADM2) when `users.suspended_at` is set, and stores the rehash. The suspension check comes only after the password matched, so a wrong password stays 401 and the answer tells a guesser nothing; the login limits are unchanged (Q17 A). A suspension that committed first is seen there; one that commits later waits for the row lock and then deletes the new session (§5.3). A password change or reset that committed during the verify therefore wins, and one that commits later waits for the login, so its session revocation deletes the new session too. Identifiers are stripped and lowercased; one containing `@` is looked up as a **verified** address (`lower(email) = lower($1)`). **Every identifier has a bucket of its own**, known or not, so a full or fresh bucket never tells a verified address from a free one, and ten failures of a username never show which address is its (§10.1). The price: 10 guesses per identifier, so 20 per account (name and address) per 10 minutes.
- **The current-password check** (`auth::account::verify_current_password`, shared by password change, email change and account deletion): read the stored hash → `password_check_session.check_and_hit(s:<session_id>)` (10 per 10 minutes) → `password_check_user.check_and_hit(u:<id>)` (30 per hour) → verify (400 `Current password is wrong`). Over either bucket it is 429 `Too many attempts. Wait a few minutes.` Every check counts, right or wrong, and nothing clears the buckets (security review 2026-10-28 #1): when a right password cleared its bucket, one session could send its own password without end and keep the four password permits busy for every login of the instance. The buckets are not the login's (`login_failures`), so failed logins by name, which anyone can send, never block a signed-in owner's password change, address change or account deletion, and wrong current passwords no longer block the login (#3). A session thief gets 10 guesses per 10 minutes, and new sessions cost a login each.
- **Password change**: the current-password check → 400 `Pick a different password` when the new one equals it → hash → one transaction: store it only over the verified hash (`UPDATE … WHERE id = $1 AND password_hash = <verified>`; 0 rows, i.e. a change or reset committed meanwhile, is 400 `Current password is wrong`), bump `password_changed_at`, delete every other session and token (and replace a cookie session, §5.1), void every open reset link and, on an account whose own address is verified, the open verification link (a pending address change, §5.5).
- **Password reset confirm** (§5.5) stores the new hash and deletes the sessions in one transaction; `confirm` and the guarded change UPDATE above then keep a racing login or change from outliving it. It also clears the account's `login_failures` keys (`u:<id>` and `u:<id>:email`).
- **Email change and account deletion** check the password with the same check (`auth::account::verify_current_password`, §5.5) and guard their transaction by the verified hash: they lock the user row and answer 400 `Current password is wrong` when it changed since the check.
- **Unknown accounts** run a verify against a fixed dummy hash, so their timing matches.
- **Rules:**
  - Length ≥ 8 is counted in **characters** (`chars().count()`), with no maximum.
  - Passwords are never trimmed.
  - Usernames are `trim` plus lowercase before the regex and before every lookup. Rust's `to_lowercase` matches Python's `lower()` for the Kelvin sign case.
- **Tests** use `Passwords::for_tests()` (`m = 8, t = 1, p = 1`), so a test run doesn't hash at production cost.

### 5.5 Email flows

- **Verification links:** 48 h.
- **Reset links:** 1 h, sent only to verified addresses.
- **Token storage:** the token is 32 random bytes; the URL carries base64url and the database keeps the SHA-256.
- **Links** carry the token in the **fragment**, which browsers never send to a server, a proxy log or a `Referer`. The SPA reads it and POSTs it:
  - `{PUBLIC_URL}/verify-email#token=…`
  - `{PUBLIC_URL}/reset-password#token=…`
  - `{PUBLIC_URL}/join/{code}` (invite links; the code is meant to be shared)
- **Using a token** marks it used, in the same transaction as its effect.
- **Mail is sent after the transaction commits.** A send failure is logged and does not fail the request. Resend exists for verification, and reset can be requested again. The handler awaits the send (`auth::mail::deliver`, at most 15 s, then it logs and gives up) before it answers, so tests see the mail at once. **Two exceptions answer first and send from a background task**, because only some answers send a mail and the time a send takes would reveal whether an account holds the address: the reset request (below) also does its lookup in the task, and resend spawns the delivery of its link. Tests wait for both with `TestApp::settle` (§17.2).
- **API** (`auth::mail`, W1; the email flows of Wave 1 step 4 build on it):

  ```rust
  pub enum TokenPurpose { VerifyEmail, ResetPassword }                     // email_tokens.purpose
  pub struct Holder { pub user_id: i64, pub username: String, pub locale: String, pub email: String }
  /// Marks every older open verify_email token of the user used, then issues one (48 h). Returns the link token.
  pub async fn issue_verify_token(conn, user_id: i64, email: &str) -> sqlx::Result<String>;
  pub async fn issue_reset_token(conn, user_id: i64, email: &str) -> sqlx::Result<String>;   // 1 h
  pub async fn void_open_tokens(conn, user_id: i64, purpose: TokenPurpose) -> sqlx::Result<()>;
  pub fn link_token_hash(token: &str) -> Option<Vec<u8>>;   // None: not base64url of 32 bytes
  pub fn verify_link(config: &Config, token: &str) -> String; pub fn reset_link(config: &Config, token: &str) -> String;
  pub fn site(config: &Config) -> email::templates::Site<'_>;
  /// The verified account holding an address (`lower(email) = lower($1)`), for the "address in use" notice.
  pub async fn verified_holder(conn, email: &str) -> sqlx::Result<Option<Holder>>;
  pub enum MailKind { Link, EmailChanged, AddressInUse }   // which per-address bucket a mail counts in
  /// After commit: the per-address limit of `kind` (silent when over), then send; failures logged. True when sent.
  pub async fn deliver(state: &AppState, email: Email, kind: MailKind) -> bool;
  pub async fn void_token(conn, token: &str) -> sqlx::Result<()>;   // marks a link used, e.g. when its mail was not sent
  pub const LINK_EXPIRED: &str = "That link has expired or was already used";   // verify and reset confirm
  ```

- **Handlers** (Wave 1 step 4): `auth::address` (`verify_email`, `resend_verification`, `change_email`, `dismiss_email_prompt`; strings `EMAIL_IN_USE`, `ADD_EMAIL_FIRST`), `auth::reset` (`request_reset`, `confirm_reset`), `auth::deletion` (`delete_account`, `delete_user`, `TRANSFER_CLUBS_FIRST`, §5.6) and `auth::export` (`export_me`, §5.7). `auth::account::verify_current_password(state, user: &CurrentUser, password) -> ApiResult<String>` is the password check that password change, email change and account deletion share (read the stored hash → `password_check_session` and `password_check_user`, never cleared → verify, 400 `Current password is wrong`; §5.4); it returns the verified hash, which the caller's write is guarded by.
- **Lock order:** every email flow locks the `users` row (`FOR UPDATE`) before it reads or writes that user's `email_tokens` rows, so two flows of one account cannot deadlock.

**Addresses and uniqueness.** Only **verified** addresses are unique (`users_verified_email_key`), and only verified addresses are used for login by email and for reset. An unverified address is just what someone typed; it cannot squat anyone's address.
- **Accepted addresses** (`auth::rules::valid_email`, 400 `Enter a valid email address`): at most 254 characters, a plain `local@domain` with an RFC 5322 dot-atom local part (atext plus non-ASCII letters; no quotes, comments, angle brackets, commas, whitespace, control or format characters) and a domain of at least two labels of letters, digits and inner hyphens, which lettre also parses as an `Address` unchanged. Anything a mail library could read as another mailbox (`"a"@b.de`, `(c)a@b.de`, `x<a@b.de>`) is refused, so `verified_holder`, `users_verified_email_key` and the `email_target` limit always see the address the mail goes to. `SmtpEmail` sends to `Mailbox::new(None, address)`, never to a parsed mailbox string. Provider aliases (`a+tag@`, Gmail dots) remain distinct addresses.
- `users.email` is the account address. On an account with no verified address, a sign-up or `PUT /api/auth/me/email` writes the new address there at once (unverified). On an account with a verified address, the new address lives only on the open `verify_email` token until it is verified; `UserOut.pending_email` shows it.
- **No enumeration.** Register and `PUT /api/auth/me/email` never reveal whether an address is in use: they answer exactly as for a free address. When the address already belongs to a **verified** account, no verification link is sent; that address gets the "someone tried to use your address" notice instead (§9). The new account keeps an unverified address it can never verify. `PUT /me/email` on a verified account still issues the (unsent) verification token in that case, so `UserOut.pending_email` shows the address exactly as for a free one, and the old address gets the same "email changed" notice. The reset request answers 204 before it looks anything up, and resend answers before its link is sent (it sends nothing when a verified account holds the address). The stored state differs (sign-up issues no token for a held address), so nothing may show an unverified account's open link: `UserOut.pending_email` and the export's `profile.pending_email` are null unless the account's own address is verified, and the login failure buckets are per identifier (§5.4).

**Token invalidation** (each in the same transaction as the event):

| Event | Effect on open tokens of the user |
|---|---|
| A new `verify_email` token is issued (register, `PUT /me/email`, resend) | Every older open `verify_email` token is marked used: at most one is open. |
| `PUT /me/email` | Also every open `reset_password` token is marked used. |
| Verification succeeds | Every other open `verify_email` and every open `reset_password` token is marked used. |
| Password change, password reset | Every open `reset_password` token is marked used (plus the session revocation of §5.1). On an account whose own address is verified (always, for a reset), every open `verify_email` token too: it is a pending address change, which whoever knew the old password may have started, and its link would later take the account's address. The owner starts a change of their own again. An unverified account's link verifies its own address and stays. |

- **Newest link** means the highest `id`, never `created_at`. Every flow that issues a verification token holds the `users` row lock (sign-up creates the row in the same transaction), and identity values are taken at insert, so ids follow issue order. `created_at` is the start of the issuing transaction: a request that began first but waited for the lock issues the later link with the earlier time. Verify, `UserOut.pending_email`, the email flows and the export's `pending_email` all order by `id`.
- **Preview** (`POST /api/auth/verify-email/preview`, `{token}` → `{username, email}`) answers for the same tokens as verify, read-only, and 400 `That link has expired or was already used` otherwise. The verify page shows it ("{email} becomes the email address of the account {username}") and verifies only when the reader taps the button: a mail scanner that runs the page's JavaScript, or a curious click on a mail someone else's sign-up sent, never hands the address to an account its owner does not know (security review 2026-10-28 #8; `/join` asks for its tap for the same reason).
- **Verify** accepts a token only when it is open (unused, unexpired), is the user's newest `verify_email` token, and its `email` is the address still pending (`users.email` for an unverified account, else the token address itself). Success sets `users.email = token.email` and `email_verified_at`. If another account verified the same address first, it answers 409 `That email is already in use` (the unique index decides; mapped with `is_unique_violation`); the link stays unused. Only the holder of a link can see that, so it does not enumerate. Verify needs no session and signs nobody in.
- **Resend** (`POST /api/auth/verify-email/resend`) renews the link of the pending address: the account's own address while it is unverified, else the address of its open email change. An account without an address is 400 `Add an email address first`. A verified account with no change waiting has nothing to verify: 204, nothing is sent and the limit is not touched. Otherwise `email_send_user` counts (429), a new token voids the old one, and the link is sent unless a verified account holds the address (then nothing is sent: the holder had its notice when the address was entered). The answer comes right after the commit in both cases; the link goes out in a spawned task.
- **`PUT /api/auth/me/email`** (`{email, current_password}`): body validation (400 `Enter a valid email address`) → `email_send_user.check` (429 `Slow down a little. Try again later.`; records nothing, so a refused change costs no argon2 work and a mistyped password uses up no change) → `verify_current_password` (400 `Current password is wrong`; 429 `Too many attempts. Wait a few minutes.`) → `email_send_user.check_and_hit` (429 `Slow down a little. Try again later.`) → one transaction: lock the user row, 400 `Current password is wrong` when the hash changed since the check, void every open reset link; then
  - the account's own verified address (compared with `lower()`, like the unique index) cancels a pending change: every open verification link is voided, nothing is sent;
  - otherwise an unverified (or missing) address is replaced at once, and a verification token is issued for the new address in every case (it voids the older one) → commit → the "email changed" notice to the old address when it is verified, then the link to the new address, or the "address in use" notice (`Attempt::EmailChange`) to the verified account that holds it.
- **Reset request** (`POST /api/auth/password-reset`, `{email}` stripped, not validated): `reset_ip.check_and_hit(ip_key)` (429 `Too many attempts. Wait a few minutes.`) → 204 at once. A spawned task then looks the address up among verified accounts (`verified_holder`), locks the user row, and sends nothing (silently) while the account has an **open reset link**, when the address's `email_target` bucket is full (the mail would be dropped), or over `reset_account` (3 an hour). Otherwise it issues the reset token for the stored address and delivers the link; a link whose mail was not sent (limit, transport failure, timeout) is voided (`void_token`), so it never holds back the next request. One open link at a time (each valid an hour) means reset requests, which anyone can send, cannot use up the address's daily mail limit faster than its owner can use a link, and the owner always holds the open one (security review 2026-10-28 #14). An unknown or unverified address does nothing.
- **Reset confirm** requires `token.email = users.email AND users.email_verified_at IS NOT NULL`; anything else is 400 `That link has expired or was already used`. Order: body validation (400 `New password must be at least 8 characters`) → the token is checked read-only first, so a made-up token costs no argon2 work → hash → one transaction: lock the user row, mark the token used (re-checked), check the address rule, store the hash and bump `password_changed_at`, delete every session and token of the user, void every open reset link and the open verification link (a pending address change) → commit → `login_failures.clear` of `u:<id>` and `u:<id>:email` (the link proves the mailbox, so someone else's failed guesses no longer lock the owner out). A refused link stays unused.
- **Rate limits** (§10.1): `PUT /me/email` and resend share the per-account `email_send_user` bucket (3 per hour, 429 `Slow down a little. Try again later.`). Every mail to an address also counts against a per-address limit of its kind (6 per day per case-folded address across all accounts): verification and reset links in `email_target`, the "email changed" notice and the "address in use" notice each in `notice_target` under its own prefix (`changed:`, `in_use:`). Over it, the mail is silently not sent and the response is unchanged. The kinds have buckets of their own so a flood of one (reset requests, which anyone can send; sign-ups with someone's address) cannot silence the notice that an account's address is being changed (security review 2026-10-28 #14).
- **Sign-up gate:** `SIGNUP_MODE=invite` is the Phase 1/2 default (a closed beta until #55). Sign-up then requires a valid invite: a club invite (it also joins the club) or a sign-up invite minted by an instance admin (club_id NULL). With `SIGNUP_MODE=open` a code is optional, and a given code must still be valid.
- **Sign-up order** (`POST /api/auth/register`): body validation (400, every message in field order: username, email, password, timezone, locale) → `signup_ip.check_and_hit(ip_key)` (429; every validated attempt counts) → username taken (409) → no code in invite mode (400 `Sign-up needs an invite code right now`; a code that normalises to nothing counts as none) → hash → one transaction: insert the user (a concurrent same-name sign-up is 409 through `users_username_key`), consume the invite (400 `That invite code is not valid`), join its club, issue the verification token **or**, when a verified account holds the address, nothing; create the session; delete the caller's previous cookie session → commit → send the link or the notice → 201 `UserOut` with `Set-Cookie`. Any failure rolls the account and the invite use back.
- **Invite seam** (`clubs::invites`, W1): `normalize_code(raw)` (whitespace and `-` removed, uppercased), `consume(conn, code, user_id) -> Option<ConsumedInvite>` (the atomic consume of §13 plus the `club_invite_uses` row) and `join(conn, &invite, club_id, user_id) -> ApiResult<()>`. `ConsumedInvite { id, club_id: Option<i64>, created_by: Option<i64>, created_at }`. Sign-up calls `join` for a club invite; `POST /api/invites/{code}/accept` reuses `consume`/`join`.
  - `join`: the club-row lock; 400 `That invite code is not valid` when the user was removed from the club at or after the invite's `created_at` (`created_at <= removed_at`); 409 `This club is full` at 20 members, counted under the lock; the membership (`role = 'member'`, `invited_by` = the invite's `created_by`; a duplicate is 409 `You are already in this club`); deleting the removal row; `member_joined` (club-scoped, actor = the new member, payload `{}`).
  - **Lock order of a join:** the club row, then the inviter's `users` row (`FOR KEY SHARE`), then the invite row. `consume` reads the invite's `club_id` and `created_by` without a lock, takes the first two locks, and only then runs the atomic `UPDATE`. The membership's `invited_by` foreign key needs the inviter's row, and an account deletion holds that row `FOR UPDATE` before its cascade clears the invite's `created_by`; a join that held the invite row first would deadlock with it. The deletion takes the club row before the inviter's row (§5.6), also when the inviter created the club and left it. A deleted inviter leaves `invited_by` NULL.
  - Accept order: the invite's club is read (unknown code, or a sign-up invite: 400 `That invite code is not valid`, nothing used) → the club-row lock → "already in this club" (409 `You are already in this club`, nothing used) → `consume` (400 when revoked, expired or used up) → `join` → `ClubOut` (200).
  - `clubs::invites::insert_invite(conn, club_id, created_by, max_uses, expires_in_days)` mints an invite under a fresh 10-character Crockford code (`new_code`, retried on a collision), with `created_at = clock_timestamp()` and the expiry counted from it; `admin::invites::mint_signup_invite` and the club route use it.
- **Before verification,** a user can log in and use everything personal, join clubs by invite, and post. Creating a club requires a verified email (`VerifiedUser`). That limits abuse once sign-up is open.
- **Imported users** have `email` NULL. `UserOut.needs_email_prompt = email IS NULL AND email_prompt_dismissed_at IS NULL`.

### 5.6 Account deletion (`DELETE /api/auth/me`)

Body `{password}`. The password is checked with `auth::account::verify_current_password` (§5.4): 400 `Current password is wrong`; 429 `Too many attempts. Wait a few minutes.` over the current-password limits. Everything below then runs in one transaction (`auth::deletion::delete_user`). **Lock order:** the rows of every club the deletion writes in (in id order), then the user row, then the user's posts (in id order). The clubs it writes in are every club that holds a row of the user or a reference to them, which the cascade deletes or clears: memberships (also those the user invited, `invited_by`), removals, clubs the user created (`clubs.created_by`), invites and invite uses, picks, milestones, posts, reactions, nominations, ballots and events. A user who left a club (after a transfer, too) can still have such rows in it. Step 1's owned clubs are read in a second statement after the locks, restricted to the locked clubs. That is the order of every membership write (§5.3): a join through the user's invite locks the club and then needs the inviter's row for `invited_by`, and a removal or a club delete locks the club and then reaches rows of the user in it, which the deletion's cascade also writes; taking the user row first would deadlock with them. A new table that references both a club and a user must be added to `auth::deletion::lock_clubs`; `tests/auth_delete.rs::the_deletion_locks_every_club_it_writes_in_first` has one case per kind of row. (Wave 1 step 5 widened the lock from the owned clubs to every club of the user, and its review to every club the deletion writes in.) After locking the user row `FOR UPDATE`, the deletion answers 400 `Current password is wrong` when the hash changed since the check. The user-row lock makes every later write that references the user (a new post, reaction or shelf entry) wait and then fail, instead of slipping in after the deletes.

1. **Clubs the user owns.** If any of them has other members: 409 `Transfer or delete your clubs first`. Clubs where the user is the sole member are deleted. (Each owned club row is locked first, in id order, and the members are counted in a new statement after the locks, so a concurrent join cannot slip in. A transfer that makes the user an owner between the club locks and the count is caught at COMMIT, as below. A membership write that makes the user owner of another club meanwhile is caught by the deferred `clubs_have_owner` check at COMMIT, which is also answered 409 `Transfer or delete your clubs first`.)
2. **Posts:**
   - First, every post of the user is locked (`SELECT id FROM posts WHERE author_id = $1 ORDER BY id FOR UPDATE`). A reply's foreign-key check holds its parent `FOR KEY SHARE`, which conflicts: a reply in flight commits before the lock is granted, and one that comes later waits for the deletion. Without the lock, a reply committed while the DELETE below waited for its parent would fail the DELETE at `posts_parent_fk` (a 500).
   - Then the user's posts that nobody else replied to are deleted (with the user's own replies under them, in one statement). It is a new statement, so it sees every reply committed before the locks were granted.
   - Then every post of the user that is left (others replied to it) becomes a tombstone (body `''`, spoiler NULL, `deleted_at` kept or now). `progress_at` and `status_at` are cleared too: they are the deleted user's reading state. (A tombstone of a post its author deleted keeps them, parity: spec/diary.md.) Tombstoning "everything left" instead of re-checking the replies means a reply deleted between the two statements cannot leave a live post without an author.
   - Their reactions cascade.
   - `author_id` then becomes NULL through the foreign key. Live posts can't exist at this point; the `posts_live_has_author` check enforces it.
   - A diary tombstone with `author_id` NULL stays visible to whoever can see the author of one of its replies (`bc_visible_posts`, §6.3), unless the author had made it `only_me`: then it stays hidden with its replies, as before the deletion. The audience of an `audience` thread changes: the root's author no longer limits it, so each reply reaches its own author's audience (people who never saw the thread may now see the anonymous root and those replies), and people who saw the thread only through the root's author lose it. Replies are always `audience`, so no reply reaches anyone its author did not allow.
3. **Media:** delete **every** avatar media row the user uploaded (`media WHERE kind = 'avatar' AND (uploaded_by = $user OR id = users.avatar_media_id)`, not only the current one) and remember their storage prefixes (`storage_key`). Cover photos the user uploaded are catalog contributions: they stay, with `uploaded_by` set to NULL by the FK.
4. **Delete the user.** This cascades:
   - sessions and email tokens
   - shelf, layout, favourites, quotes, search history, import jobs
   - memberships, removals, ballots, reactions
   - every event whose actor is the user (`ON DELETE CASCADE`)

   These references are set to NULL instead: picks (`set_by`), nominations (`nominated_by`), milestones (`created_by`), invites (`created_by`), invite uses, `invited_by`, clubs (`created_by`), works and editions (`created_by`), edit requests and reports (`created_by`, `resolved_by`, and `target_user_id`, whose profile picture was reported).

   Other users' `reaction` events keep only `author_id` (a user id, never a username, §15). Identity ids are never reused, so after the deletion the feed shows the author as null; no later account can inherit the name.
5. **After commit:** delete the remembered media bytes (`MediaStore::delete_prefix`; a failure is logged), and answer 204 with a clearing `Set-Cookie`, also to a bearer-token request.

**The decision #39 asks for:** club posts are **deleted**; they are only kept as an anonymous tombstone when others replied to them. Activity events are deleted. Structural club records (picks, nominations) stay, without the name.

**A second caller: the purge of empty sign-ups** (`auth::purge_empty_sign_ups`, LEG2, §16) runs `delete_user` in a transaction of its own per account, with the password hash it read under its lock. Its lock order is the user row, then the user's sessions (both `FOR UPDATE`), as in a password change and a reset; it takes no club row first, because an empty account has no row in any club and none can arrive while its row is locked, so `lock_clubs` locks nothing (`delete_user`'s doc comment says so). A failing `delete_user` is rolled back and logged (`deleting an empty sign-up failed; kept`, no account named), and the purge goes on with the next account.

**Replacing an avatar** (`PUT /api/auth/me/avatar`, MED) deletes the previous avatar row in the same transaction and its bytes after commit, so normally a user has at most one avatar row; step 3 also catches rows left behind by a crash.

**A reported avatar is held** (security review, Phase 2 Wave 2, merged with ADM2): when its owner replaces or deletes a profile picture that has an open report, `media::avatar::delete_avatars` keeps its row and files for the admin's decision instead of deleting them. Under the user-row lock it locks the user's avatar rows (`FOR UPDATE`, id order), then reads which have an open report in a new statement. Every report, takedown and dismissal of a profile picture locks the owner's row `FOR KEY SHARE` before the picture (§7.2), so one that started first has committed and is seen, and a later one waits for the upload or deletion. Those leave the profile and get a new public id, so the old URL answers 404 like a deleted picture's and tells the owner nothing; only the admin queue shows the new one. The other rows are deleted as before. The decision then takes the held picture down or deletes it (§7.2, endpoint-map §6), and the export leaves it out meanwhile (§5.7). Step 3 deletes held avatars too: their reports then become `gone`.

### 5.7 DSGVO export (`GET /api/auth/me/export`)

The response is a ZIP that contains `data.json` and `media/`. It is `auth::export::export_me`:

- **Streamed, bounded memory.** The rows are read first; then a spawned task writes the ZIP into the response body through a channel of one chunk: each picture is read from the media store, written (stored, not recompressed) and sent before the next one is read, and `data.json` (deflated) and the central directory come last. `data.json` is serialised once (pretty), which drops the rows' `serde_json::Value`, and is deflated and sent in 256 KiB slices. An export holds at most one picture and `data.json` in memory however many pictures the user uploaded (cover uploads have a daily limit but no total), and the producer waits for a slow client instead of reading ahead.
- **Bounded in number and time** (security review 2026-10-28 #6). `data.json` grows with what the user wrote (posts and quotes have no count limit), so: two exports stream at once on the instance (`RateLimits::exports`, `ratelimit::EXPORTS_AT_ONCE`) and one per account (`export_gate`), both taken before the rows are read and held until the stream ends, else 429 `Slow down a little. Try again later.` (checked before `export_user`, so a refused export does not count). The response body does not count against the request deadline (§3) and hyper has no write timeout, so the producer bounds itself: a chunk the client does not take within 60 s ends the export (`send_chunk`), and so does a download still running after an hour; either drops the rows and frees the slots. The entries carry their sizes in data descriptors (the `zip` crate's stream mode), which unzip tools read from the central directory. A media-store error after the answer started aborts the download (logged); the archive then has no central directory, so no reader takes it for complete.

- 5 exports per user and hour (`export_user`, 429 `Slow down a little. Try again later.`).
- Every section is read in one `REPEATABLE READ, READ ONLY` transaction, so the sections agree with each other.
- The response carries `Content-Type: application/zip`, `Content-Disposition: attachment; filename="bookclub-export-<username>-<YYYYMMDD>.zip"` (today in the user's time zone) and `Cache-Control: no-store`.
- Most rows are the table row as Postgres renders it (`to_jsonb(row)`) minus the reference to the user, plus readable names (`work_title`, `club_name`), so a column added later is exported without a code change. Sessions use an allow-list (never `token_hash`); the profile drops `password_hash`.
- Timestamps are RFC 3339 in UTC with `Z`, like the API (§12): the string value of every key ending in `_at` that parses as RFC 3339 is rewritten (`posts.progress_at` is a number and `posts.status_at` a status, so they stay).
- `catalog_contributions` is `{works, editions}`, `edit_requests` `{filed, decided}`, `reports` `{filed, resolved, about_me}`, `physical_shelf_layout` an object or null; every other key is an array. `invite_uses` names the club and the time, never the code (the invite belongs to whoever created it).
- **Clubs the user no longer belongs to** (left or removed): the export must not be a window into them (DSGVO Art. 15(4): the copy must not touch the rights of others). Their rows keep only what the user wrote and nobody can change since, plus the club's current name:
  - `members_invited`: only clubs the user is a member of (`bc_is_member`); other people's memberships are theirs.
  - `clubs_created`: `{id, name, created_at}`, not slug and time zone.
  - `picks_set`: `{id, club_id, club_name, work_id, work_title, started_at}`, not the note, meeting, end or reminder, which other admins edit.
  - `milestones_created`: `{id, club_id, club_name, pick_id, created_at}`, not the title, range or due date.
  - `invites_created`: the whole row (with `code`, `use_count`, `revoked_at`) only while the user may list the invite (an admin or owner of its club; an instance admin for a sign-up invite), else `{id, club_id, club_name, created_at, expires_at, max_uses}`. The code is a credential of the club.
  
  Club names are the current ones in every section (a rename after the user left shows; the name is what makes the rows readable).

**`data.json`** holds one key per foreign-key column that references `users(id)`:

| Key | Source (`table.column`) |
|---|---|
| `profile` | `users` row minus `password_hash` (so `terms_version` and `terms_accepted_at` too, without code: `the_export_holds_the_accepted_terms`, and ADM2's `suspended_at` and the integrator's `unsuspended_at`), plus `pending_email` (as in `UserOut`: null unless the account's own address is verified, §5.5 "No enumeration") |
| `sessions` | `sessions.user_id`: label, kind, created, last used (never `token_hash`) |
| `shelf_entries` (with work titles) | `shelf_entries.user_id` |
| `physical_shelf_layout` | `physical_shelf_layouts.user_id` |
| `favorites` | `user_favorites.user_id` |
| `search_history` | `search_history.user_id` |
| `quotes` | `quotes.user_id` |
| `posts` (both scopes, with club names) | `posts.author_id` |
| `reactions` | `post_reactions.user_id` |
| `memberships` (club name, role, joined) | `club_memberships.user_id` |
| `members_invited` (club name, username) | `club_memberships.invited_by` |
| `club_removals` (club name, date) | `club_removals.user_id` |
| `clubs_created` | `clubs.created_by` |
| `invites_created` | `club_invites.created_by` |
| `invite_uses` | `club_invite_uses.user_id` |
| `picks_set` | `club_picks.set_by` |
| `milestones_created` | `milestones.created_by` |
| `nominations` | `next_up_nominations.nominated_by` |
| `ballots` | `next_up_ballots.user_id` |
| `events` | `events.actor_id` |
| `import_jobs` | `import_jobs.user_id` |
| `catalog_contributions` (works and editions the user added) | `works.created_by`, `editions.created_by` |
| `edit_requests` (filed / decided) | `catalog_edit_requests.created_by`, `catalog_edit_requests.decided_by` (the rows carry `notified_at`, migration 2100: when the admins were first told; no user reference) |
| `reports` (filed / resolved / about_me) | `reports.created_by`, `reports.resolved_by`, `reports.target_user_id` (the rows carry `notified_at`, as above) |
| `uploaded_media` (avatars and cover photos) | `media.uploaded_by` |

**`reports`** (`auth::export`, the reports section; ADM2 and the Phase 2 Wave 2 security review):
- `filed`: the reports the user filed, each row minus `created_by`, `resolved_by` and `target_user_id` (another account); it keeps `target_work_id` (a work id), `via` and `closed_on_arrival`.
- `resolved`: the reports the user decided as an admin, `{id, target_kind, media_id, reason, status, resolved_at}`.
- `about_me`: the **removals** of the user's content after a report, one entry per decision: reports of their profile picture (`target_user_id`), of pictures they uploaded (`media.uploaded_by`), and of works and editions they added (`created_by`; a work report by `target_work_id`, so after a merge it stays with the reader whose book was reported). Only `status = 'removed'` and not `closed_on_arrival`, grouped by target and `resolved_at`, as `{target_kind, media_id, work_id, edition_id, reason, status, resolved_at}` with the reason of the decision's oldest report. Open and dismissed reports, reports closed on arrival, how many reports a decision closed, `via`, the reports' own times, the reporter and `details` are withheld (Art. 15 (4) GDPR, § 29 (1) BDSG): only club mates can report a profile picture, so in a club of 2 to 5 any of them can name the reporter. The statement of reasons (§9) already told the owner of every removal. Tests: `auth_export.rs::about_me_holds_only_removals_never_who_reported_or_when`, `every_section_holds_the_users_rows`.

**`media/`** holds the avatar file and the cover photos the user uploaded: every stored variant of every `uploaded_media` row (`uploaded_by` = the user, plus the current avatar) that was not taken down, as `media/<public_id>/<variant>.<webp|jpg>`, stored without recompression. Each `uploaded_media` entry lists its paths in `files`; a removed picture is listed with `files: []`. A **held** profile picture (§5.6: replaced or deleted while reported, neither the current avatar nor taken down) is left out of `uploaded_media` and `media/`, since listing it would tell the owner of the report; once taken down it is listed as removed (`a_held_avatar_is_left_out_of_the_export_until_it_is_decided`).

**Exempt, with the reason in the test's allow-list:** `email_tokens.user_id` (one-time secrets that expire within 48 h; the only personal datum on them, a pending address, is exported as `profile.pending_email`).

**Completeness test** (`tests/auth_export.rs`, `SOURCES` and `EXEMPT`). It reads `pg_constraint` for every foreign-key column that references `users(id)` and asserts that each `table.column` is either a source in the table above (the test holds the same list) or in the allow-list, and that `data.json` has exactly the keys of the table. A second test gives one user a row behind every one of these columns and checks that each section holds it and nothing of another user. A new column that points at users fails it until it is exported or exempted with a reason. The test also pins the number of such columns (29 since ADM2's `reports.target_user_id`), so the package that adds one changes the count (work plan amendment 19). (Matching on column names would also hit `work_authors.author_id`, which references `authors`.)

**The purge of empty sign-ups has the same kind of check** (LEG2, `tests/jobs.rs`; it reads only the schema):
- A new foreign key to `users` also needs a `NOT EXISTS` in `auth::empty_sign_ups` and a row in `DATA` (SQL that gives an account a row there), or a row in `TRACES` with the reason when it is a trace of signing up or in (today `sessions.user_id`, `email_tokens.user_id`, and `club_invite_uses.user_id` of a sign-up invite). `every_reference_to_an_account_is_data_or_a_trace_of_signing_in` fails otherwise.
- A new column of `users` needs a row in `USERS_COLUMNS` with how the purge reads it (`every_column_of_users_is_considered_by_the_purge`), and a condition in `empty_sign_ups` when it records a sign-in or keeps the account.
- A new foreign key to `media` needs a check in `auth::delete_rejected_cover_photos` and a row in `MEDIA_REFERENCES` (`rejected_cover_photos_go_with_their_request`).

### 5.8 Cross-site requests (CSRF)

The cookie is `SameSite=Lax`, so browsers don't attach it to cross-site POST, PUT, PATCH or DELETE. JSON endpoints also require `Content-Type: application/json`, which a plain HTML form cannot send.

On top of that, the **origin check layer** answers 403 `{"detail":"Cross-site request refused"}` to any unsafe-method request that:
- may authenticate by cookie: it has no bearer token and carries a `Cookie` header, whatever cookies it holds, and
- carries an `Origin` header that is not `PUBLIC_URL`'s origin, not in `CORS_ORIGINS`, and not the Vite origin in DEV_MODE.

Any cookie counts, not only a literal session cookie: the check fails closed instead of depending on how the session extractor parses cookie names (it reads the raw name, §5.1, but a check that never parses names cannot drift from it). Bearer-token requests skip the check. **The frontend sends nothing extra.** It is `auth::origin::check` (W1, implemented in step 1); the cookie name is `session::cookie_name(config)` (§5.1).

### 5.9 Legal texts and terms acceptance (LEG #53, `legal.rs`)

- **The texts.** The operator keeps six HTML fragments in `LEGAL_DIR` (§11): `{imprint,privacy,terms}.{de,en}.html`, as a generator exports them, UTF-8, non-empty, at most 512 KiB each, plus `manifest.json`: `{"terms_version": "2026-11-01", "updated": {"imprint": "2026-11-01", "privacy": …, "terms": …}}`. `LegalTexts::configure`, called from `Config::from_lookup` (not `main.rs`), reads them once at startup; a change needs a restart.
  - The manifest is required once a text file exists, and unknown keys are refused. `terms_version` is 1 to 64 of `[A-Za-z0-9._-]` (the column check `users_terms_version_format`) and required once a terms file exists; a new value makes every account accept again. `updated` is optional per document (`YYYY-MM-DD`).
  - **An absent or empty `LEGAL_DIR`** (a path that does not exist, or a directory with neither the manifest nor any of the six texts) serves no texts and logs a warning, in DEV_MODE too. The hosted compose file mounts the empty `/srv/bookclub/legal` at `/app/legal` before any text exists, so the first deploy relies on this. Still errors: a path that is not a directory, texts without a manifest, an invalid manifest, a bad file. Errors name `LEGAL_DIR` and the file.
  - **Outside DEV_MODE**, startup refuses a file carrying the placeholder marker (`data-legal-placeholder`), and refuses `SIGNUP_MODE=open` unless all six files are there (the error lists the missing ones). `SIGNUP_MODE=invite` starts with any subset.
  - A document missing in one language falls back to the other (`LegalDocumentOut.fallback`).
- **Placeholders.** DEV_MODE without `LEGAL_DIR` serves the six marked files of `server/legal-placeholder/`, compiled into the binary (`include_str!`). They carry **no** terms version, so development sign-ups (and every test) need no acceptance. `LEGAL_DIR=legal-placeholder` (from `server/`) loads the same files with their manifest (terms version `placeholder`) to try the acceptance.
- **Terms acceptance.** `legal::accepted_terms(config, given)` decides for sign-up and `POST /api/auth/terms`: without configured terms nothing is stored (a given version is ignored on sign-up, refused by the accept route); with terms, `given` must equal the current version, else 400 `msg::TERMS_CHANGED`. Sign-up stores `users.terms_version` and `terms_accepted_at = now()` in the same insert (migration 2200; `users_terms_accepted_together`: both NULL or both set). Accepting the version already accepted keeps the first time. `UserOut.needs_terms` is §5.3's rule.
- **Serving.** `GET /api/legal/{doc}` is public (§17.7) and carries the API's `Cache-Control: no-store`. `/api/config` lists the served documents (`legal`), the current `terms_version` and `contact_email` (`CONTACT_EMAIL`, the public contact and takedown address).
- **The SPA** shows the texts at `/imprint`, `/privacy` and `/terms` (top-level routes outside the AppShell) and puts the operator's HTML through `sanitizeLegalHtml` (`frontend/src/pages/LegalPage.vue`, an allow-list of text elements and attributes; no picture, script, style or handler survives) before `v-html`. Any other place that renders server HTML must use it too.

## 6. Visibility policy (#46)

### 6.1 Rules

**Relation of a viewer to the owner of personal data:**

| Relation | Meaning |
|---|---|
| `Myself` | the viewer is the owner |
| `Audience` | they share at least one club (Phase 3: or are friends) |
| `Stranger` | anyone else |

- **Audience sees** shelf, ratings and takes, currently reading, stats, diary entries, quotes, favourites and the physical layout (read-only).
- **`only_me` items** (diary roots, quotes, and ratings via `shelf_entries.rating_visibility`, which also hides `take`) are visible to `Myself` only.
- **Strangers see nothing.**
- **Unknown usernames** produce exactly the same 404 `No member with that name` as strangers.
- **Club data is visible to the club's members only** (§5.3). Inside a club, readers, overlap, suggestions, members previews and the activity feed show **club members only**. Those are a subset of the viewer's audience, but `only_me` ratings and takes are still redacted.
- **Rating redaction:**
  - For `shelf_entries` with `only_me`, every non-owner sees `rating: null`, `take: ""` and `rating_visibility: null`.
  - The redaction also applies to `author_rating` on posts, to `club_rating`/`rating_count`, to stats, and to `shelf_finished` event payloads.
  - It does not apply to `dnf_reason` or `progress`: they are shelf state, which the plan makes visible to the audience.

### 6.2 API (W1, frozen)

```rust
// crate::visibility
#[derive(Clone, Copy, PartialEq, Eq)] pub enum Relation { Myself, Audience, Stranger }
// Visibility is defined in crate::dto (§20) and re-exported here.

impl Relation { pub fn sees(self, item: Visibility) -> bool; pub fn sees_profile(self) -> bool; }
pub async fn relation(conn: &mut PgConnection, viewer: i64, owner: i64) -> sqlx::Result<Relation>;
/// Resolve `?username=` for personal reads: 404 NO_MEMBER when unknown or a stranger.
pub async fn visible_user(conn: &mut PgConnection, viewer: i64, username: &str) -> ApiResult<UserSummaryRow>;
/// For one post id: Ok(PostAccess) when the viewer may see it, else 404 NO_ENTRY. Tombstones count as
/// missing unless `allow_tombstone`. `club_role` is the viewer's role in the post's club (club posts only).
pub async fn post_access(conn: &mut PgConnection, viewer: i64, post_id: i64, allow_tombstone: bool)
    -> ApiResult<PostAccess>;
pub struct PostAccess { pub post_id: i64, pub work_id: i64, pub author_id: Option<i64>, pub parent_id: Option<i64>,
    pub club_id: Option<i64>, pub pick_id: Option<i64>, pub club_role: Option<Role>, pub deleted: bool }
pub fn redact_rating(is_owner: bool, vis: Visibility, rating: Option<i32>, take: String)
    -> (Option<i32>, String);
```

`UserSummaryRow { id: i64, username: String, avatar_public_id: Option<String> }` is in `crate::dto` (§20).

**SQL side (in the migration).** Set-returning functions are for lists and aggregates; they are single SQL statements, so the planner inlines them into the calling query (checked with EXPLAIN on the design schema). Scalar functions are for single checks and never return NULL.

| Function | Returns | Use |
|---|---|---|
| `bc_visible_owners(viewer)` | `(user_id, is_self)`: the viewer **and** everyone in their audience | "whose data may I see"; "other readers" add `WHERE NOT is_self` |
| `bc_visible_shelf(viewer)` | shelf entry rows of visible owners, with `rating`/`take`/`rating_visibility` already redacted for others' `only_me` entries, plus `is_self` | readers, rating means and counts, stats, overlap, suggestions, members previews |
| `bc_visible_quotes(viewer)` | quote rows the viewer may see (own, plus `audience` quotes of the audience), plus `is_self` | quote lists, `quote_count`, stats quote counts |
| `bc_visible_posts(viewer)` | ids of posts of both scopes the viewer may see, tombstones included (§6.3) | threads, feeds (add `deleted_at IS NULL`), reply checks |
| `bc_member_clubs(viewer)` | `(club_id, role, joined_at)` | club rows reached without a `ClubMember` extractor |
| `bc_can_see_user(viewer, owner)` | boolean | single checks |
| `bc_can_see_item(viewer, owner, visibility)` | boolean | single diary-root, quote or rating checks |
| `bc_can_see_post(viewer, post_id)` | boolean | single post checks (`visibility::post_access` uses it) |
| `bc_is_member(viewer, club_id)` | boolean | single club checks in SQL |

sqlx infers every column of a function result as nullable; use `AS "x!"` overrides for the non-null ones.

**Review rule:** every query that returns rows owned by a user other than the caller, or club rows, must get them in one of these ways, and nothing else may decide visibility:
- through one of the functions above (lists: `bc_visible_*`/`bc_member_clubs`; single checks: the scalar ones or `crate::visibility`), or
- as a club query whose `club_id` came from `ClubMember` (or `member_by_slug`/`role_in`, §5.3) and that is restricted to members of that club (`JOIN club_memberships m ON m.club_id = $club AND m.user_id = x.user_id`). When such a query shows ratings, takes or quotes it still reads them through `bc_visible_shelf`/`bc_visible_quotes`, so `only_me` items stay redacted inside clubs too.

In particular, `quote_count`, the stats quote counts and every rating aggregate exclude other users' `only_me` items because they read through these functions, not because each query remembers a `visibility` filter.

### 6.3 Posts: two scopes in one table

**Diary entry** (`club_id IS NULL`):
- A root is visible when `bc_can_see_item(viewer, author, visibility)`.
- A reply is visible when its root is visible **and** the viewer is its author, the root's author, or in the reply author's audience.
- Replies are always `audience`. The author of an entry sees every reply to it.
- **Tombstone of a deleted account** (`author_id` NULL, §5.6): an `audience` root is visible to every viewer who can see the author of at least one of its replies, and each reply keeps its own rule (visible when the viewer can see the reply's author). An `only_me` root stays hidden, with its replies, from everyone. The audience is therefore not the same as before the deletion: each reply reaches its own author's audience, no longer limited by the root's author (§5.6).

**Club post** (`club_id` set):
- Visible to members of that club. Replies inherit the scope; the schema enforces it (`posts_parent_fk` ties a reply to its parent's work and scope).
- Any member may reply.
- Admins **of that post's club** may delete others' club posts (moderation, checked with `PostAccess.club_role`); edits stay author-only.

**Writes** (all through `visibility::post_access`):
- Replying needs a visible root. An invisible root gives 400 `That entry is not on this book`, as for a missing one.
- Editing, deleting **or reacting to** a missing, tombstoned or invisible post (another club's post, a stranger's or someone else's `only_me` diary entry) gives 404 `No entry with that id`. The reaction endpoint returns the full `PostOut`, so this check is what keeps other people's bodies private.
- Editing a visible post by someone else gives 403.

**Reaction lists:**
- `count` counts everyone.
- `users` lists only the viewer, the viewer's audience and, on club posts, club members. Strangers' names never leak.
- `mine` is exact.

**`parent_author` in feeds** is null when the parent's author is not visible to the viewer.

### 6.4 Activity events

- **Club feed** = events with `club_id = $club`, plus personal events (`club_id IS NULL`) whose actor is a **current** member of the club with `created_at >= joined_at`.
- **Home feed** = the union of the viewer's club feeds (clubs from `bc_member_clubs`), plus the viewer's own personal events, deduplicated by id.
- **Redaction at read time,** for viewers other than the actor:
  - `shelf_finished`: drop `rating` and `take` when the actor's current shelf entry for the work is `only_me` or missing.
  - `quote_added`: drop the event when `payload.quote_id` is missing, the quote is gone, or it is `only_me`.
- **Stored payloads are sealed too** (security review 2026-10-28 #2). A payload outlives its entry, so read-time redaction alone let an old rating come back: an only-me rating after the work was removed and added again (audience by default, as "start reading" on the club page does), a take written while only-me after the entry was rewritten and shared, the event of an only-me entry that a work merge moved onto an audience entry. So a `shelf_finished` payload's `rating` and `take` are **sealed** to the actor (`"private": true`, `activity::PRIVATE_KEY`) unless its actor's entry for the work exists and is `audience`; a sealed rating and take are dropped for every other viewer for good, whatever the entry becomes, and the actor keeps seeing their own. The feeds never serve the key itself.
  - `status_event` seals them when it writes them for an `only_me` entry;
  - `activity::seal_finished_ratings(conn, actor?, work_ids?)` seals every `shelf_finished` event with a rating or take whose actor has no `audience` entry for the event's work. The shelf calls it when an entry is removed or saved as `only_me`, the work merge before it merges the shelves (on both works) and after it moved the references (on the winner), and the importer once over all events (legacy payloads were redacted only at read time; the report counts them as `events.payload.rating_take`).
  - A `quote_added` item shows the quote as it is now: the feeds join the quote and serve `excerpt` (its first 80 characters) and `page` from it, never the text it was added with, which may have been only-me. The stored excerpt is shown only to the actor, for a quote they deleted.
- **Names at read time:** `reaction` payloads store `author_id`; the feed replaces it with `"author": <username>`, or `null` when the account is gone, so the API keeps the legacy payload shape.

### 6.5 The #46 test matrix

`tests/visibility_policy.rs` (W1) covers the SQL functions and `Relation` for:
- viewer types: self, co-member, member of another club, stranger, and "former co-member" (left the club)
- data: `audience` and `only_me` items, and a diary tombstone of a deleted account with replies by a co-member and by a stranger
- the scalar functions with a NULL owner (they return false, never NULL)

Every Wave 2 package adds `tests/<pkg>_visibility.rs`, following the template in `tests/common/visibility.rs`. It covers each personal data kind it serves (shelf, layout incl. the PUT→404 AC, favourites, stats, diary, quotes, ratings in book detail/pick readers/activity) across viewer types (self, co-member, stranger, unknown name) and item visibility. #49 checks that the matrix is complete: `tests/parity_visibility.rs` (`every_personal_column_route_and_rating_field_is_in_the_matrix`) reads the live schema and the OpenAPI document, and every foreign key to `users`, every operation with a `username` parameter and every schema with a rating or take field must have a row there, with the kinds that serve it or the reason it is not personal data. The package that adds one adds its row (work plan amendment 27; ADM2 added `reports.target_user_id` and the two suspension routes).

## 7. Catalog interfaces (W1, frozen; `catalog/shared/`)

### 7.1 Loaders, references, ISBNs

```rust
// books.rs: the shared DTO loaders (DTOs in crate::dto, §20)
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)] pub struct BookRef { pub work_id: i64, pub edition_id: Option<i64> }
impl BookRef { pub fn work(work_id: i64) -> BookRef; }  // no edition context
/// With an edition of that work: the edition's title (when it has its own), pages, cover and spine colour win; an
/// edition of another work is ignored. Missing works are left out. A removed cover gives no URLs; spine '' → null.
pub async fn book_outs(conn: &mut PgConnection, refs: &[BookRef]) -> sqlx::Result<HashMap<BookRef, BookOut>>;
pub async fn book_out(conn: &mut PgConnection, r: BookRef) -> sqlx::Result<Option<BookOut>>;
/// Missing ids are left out; contributors in record order; isbn10 derived.
pub async fn edition_outs(conn: &mut PgConnection, ids: &[i64]) -> sqlx::Result<HashMap<i64, EditionOut>>;
/// The work id after following works.merged_into_id; None when it does not exist.
pub async fn follow_merges(conn: &mut PgConnection, work_id: i64) -> sqlx::Result<Option<i64>>;
/// follow_merges for many ids in one query: the current work per requested id; missing ids are left out.
pub async fn follow_merges_many(conn: &mut PgConnection, work_ids: &[i64]) -> sqlx::Result<HashMap<i64, i64>>;
/// Follows works.merged_into_id. 404 NO_SUCH_BOOK when the work does not exist.
pub async fn require_work(conn: &mut PgConnection, work_id: i64) -> ApiResult<i64>;
/// 400 "That edition belongs to another book" unless the edition is of that work (a missing edition too).
pub async fn require_edition_of(conn: &mut PgConnection, work_id: i64, edition_id: i64) -> ApiResult<()>;

// work_ref.rs: `{work_id}` path parameter
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkRef { Id(i64), OpenLibrary(String /* OL…W */), Legacy(String /* legacy_work_keys.legacy_key */) }
/// `^[0-9]+$` that fits i64 → Id; `^OL[0-9]+W$` → OpenLibrary; BC…/ISBN…/GB… (legacy regexes, catalog spec §2.1) →
/// Legacy("/works/<canonical id>"), canonicalising ISBN x→X; a GB key becomes
/// "sha256:<hex of SHA-256 of "/works/GB…">" (case-sensitive). Anything else (also legacy OTHER keys) → None.
impl WorkRef { pub fn parse(raw: &str) -> Option<WorkRef>; }
/// Follows merges, like require_work.
pub async fn resolve_local(conn: &mut PgConnection, r: &WorkRef) -> sqlx::Result<Option<i64>>;
pub fn canonical_work_id(work_id: &str) -> String;     // the legacy canonical_work_id
/// The legacy_work_keys.legacy_key for a legacy work id ("/works/<canonical>", GB ids hashed). IMP stores keys
/// through this, so they always match WorkRef::parse.
pub fn legacy_key(work_id: &str) -> String;

// isbn.rs
pub struct Isbn13(pub(crate) String);                  // impl Display; as_str()
pub fn parse_isbn(raw: &str) -> Option<Isbn13>;       // strips spaces/'-', uppercases, checks checksum, 10→978-13
pub fn isbn10_of(isbn: &Isbn13) -> Option<String>;     // None for 979; EditionOut.isbn10 is always derived
pub fn looks_like_isbn(q: &str) -> bool;               // shape only, for routing a search query
pub fn is_german_group(isbn: &Isbn13) -> bool;         // 978-3: the DNB route of §7.4
pub fn registration_group(isbn: &Isbn13) -> &str;      // prefix + group, e.g. "9783", "97888", "97910" (range table)
```

### 7.2 Ingest: the only writer of catalog fields, with field-level provenance

Every catalog field of `works`, `editions`, `authors`, `work_authors` and `edition_contributors` is written by `ingest`. Three narrow exceptions write through dedicated functions with the same rules: covers (`set_cover`/`clear_media`, used by MED and CAT), admin edits (`apply_admin_fields`: CAT's approve, and ADM's direct `PATCH /api/admin/works/{work_id}` and `PATCH /api/admin/editions/{edition_id}`, #54) and merges (`catalog::merge_works`, CAT). The importer may also set `works.details_fetched_at` directly (the legacy value).

**Provenance.** `works.field_sources` and `editions.field_sources` record, per field that holds a value, which source wrote it. Ranks: `admin` (an approved edit request or a direct admin edit) > `user` (entered by a reader) > `dnb` = `openlibrary` > `legacy` (bare facts from the legacy import).
- An empty field (NULL, `''`, `{}`) may be filled; the new provenance is the writer's.
- **An admin's cleared field stays empty** (Phase 2, ADM #54, Q18): `apply_admin_fields` records `admin` for a field it clears too, and no source fills an empty field whose provenance is `admin`; only another admin edit sets it again. That is how a description is removed on notice (`docs/legal/takedown.md`). Two writers outside `decide` can still fill such a field: `fill_work_from_edition` (a work's NULL `pages` and `first_publish_year`, from an edition) and a merge (`catalog::merge_works` takes the loser's value into an empty winner field).
- A source may overwrite a field whose provenance rank is **equal or lower** than its own; never a `user` or `admin` field.
- `Mode` decides what a record does to fields it has no value for (see below).
- `works.source`/`editions.source` is the row's origin. A `legacy` work becomes `dnb` or `openlibrary` the first time a record of that source is ingested into it (an ISBN hit on its edition, or a merge target); `user` rows keep `user`.

```rust
pub enum Source { OpenLibrary, Dnb, User, Legacy, Admin }   // Admin: field provenance only, never a row source
impl Source { pub fn rank(self) -> u8; pub fn as_str(self) -> &'static str; pub fn parse(s: &str) -> Option<Source>; }
                                                             // Legacy 1, OpenLibrary/Dnb 2, User 3, Admin 4
pub enum ContributorRole { Translator, Illustrator, Editor } // as_str()
pub struct AuthorRecord { pub name: String /* natural order */, pub sort_name: Option<String>,
    pub ol_author_id: Option<String>, pub gnd_id: Option<String> }
pub struct WorkRecord { pub source: Source, pub ol_work_id: Option<String>, pub title: String,
    pub subtitle: Option<String>, pub author_names: String, pub authors: Vec<AuthorRecord>,
    pub first_publish_year: Option<i32>, pub description: Option<String>, pub subjects: Option<Vec<String>>,
    pub pages: Option<i32>, pub ol_rating: Option<f64>, pub ol_rating_count: Option<i32>,
    pub ol_cover_id: Option<i64>, pub full_details: bool, pub created_by: Option<i64> }
pub struct EditionRecord { pub source: Source, pub isbn13: Option<Isbn13>, pub ol_edition_id: Option<String>,
    pub dnb_idn: Option<String>, pub dnb_record_idn: Option<String> /* None = dnb_idn; added by INT, migration 1200 */,
    pub title: Option<String>, pub subtitle: Option<String>, pub publisher: Option<String>,
    pub language: Option<String>, pub format: Option<Format>, pub pages: Option<i32>,
    pub publication_year: Option<i32>, pub planned_publication_date: Patch<time::Date>,
    pub ol_cover_id: Option<i64>, pub contributors: Vec<(AuthorRecord, ContributorRole)>, pub created_by: Option<i64> }
pub enum Format { Hardcover, Paperback, Ebook, Audiobook, Other }   // as_str(), parse()
// Constructors with every other field None/empty, for struct update syntax:
impl AuthorRecord { pub fn named(name: impl Into<String>) -> AuthorRecord; }
impl WorkRecord { pub fn new(source: Source, title: impl Into<String>) -> WorkRecord; }   // author_names "" = derived from authors
impl EditionRecord { pub fn new(source: Source) -> EditionRecord; }                      // planned date Patch::Missing
pub enum Mode {
    /// Existing rows are returned untouched (search stubs, one batched statement).
    InsertOnly,
    /// Some values fill empty fields and overwrite fields of equal or lower rank; None keeps the old value.
    Refresh,
    /// A complete record from an authoritative feed (DNB OAI and SRU): as Refresh, and None also clears a
    /// field whose provenance is this same source. planned_publication_date follows its Patch
    /// (Missing keeps, Null clears, Value sets): the MARC mapping never sends Null, so the date survives
    /// a later record without 263. A record without a cover never clears one.
    Replace,
}
pub enum WorkGrouping {
    /// Create a new work from the WorkRecord (no deduplication; the caller decided). A record with an
    /// ol_work_id still finds that work (upsert_work).
    NewWork,
    /// Attach to this existing work (following merges; Err for a missing work).
    Into(i64),
    /// DNB: an existing non-merged `source='dnb'` work with the same match_key, else a new work. A record
    /// without an author always gets a new work: the title alone would group unrelated anthologies,
    /// calendars and yearbooks.
    DnbTitleAuthor,
}

/// A work identified by ol_work_id is found and updated per `mode`. With ol_work_id None, a new work is
/// ALWAYS created, for any source (no deduplication; the caller groups). Source::OpenLibrary requires
/// ol_work_id (debug_assert; Err(Protocol) in release). Sets details_fetched_at when full_details.
/// A work found by ol_work_id that was merged away (it keeps its id when the winner has its own, §7.6)
/// resolves to the surviving work, which is returned untouched: it has its own Open Library record, and
/// the merged row is never written again. So NewWork grouping attaches editions to the survivor.
pub async fn upsert_work(conn: &mut PgConnection, rec: &WorkRecord, mode: Mode) -> sqlx::Result<i64>;
/// One batched INSERT … ON CONFLICT DO NOTHING for search pages; returns ids in input order, each
/// resolved through merges like upsert_work.
pub async fn upsert_work_stubs(conn: &mut PgConnection, recs: &[WorkRecord]) -> sqlx::Result<Vec<i64>>;
/// Finds the edition by isbn13, then dnb_idn, then ol_edition_id.
/// - Found: updates it per `mode` and the provenance rules (dnb_idn takes the newest record's IDN, moving it from
///   another edition; dnb_record_idn takes the newest delivering record's IDN, which is not unique and moves
///   nothing; a NULL isbn13/ol_edition_id is filled when free, an existing one is kept; a withdrawn
///   edition is un-withdrawn). Then the parent work's title, subtitle, author_names and work_authors are updated
///   from `work` under the same rules when they are `legacy`-ranked, or when they are `dnb`-ranked and this is the
///   work's only edition (a provisional VLB title gets the final one). Precisely: the three fields merge normally
///   (empty ones filled) when the work's source is `legacy` (bare facts being upgraded) or when the record is DNB,
///   the title's provenance is `dnb` and the work has one edition; otherwise only `legacy`-ranked values are
///   replaced. So an OL record never retitles a DNB work and a DNB record never retitles an OL work.
/// - Not found: resolves the work by `grouping` (creating it from `work` when needed) and inserts the edition.
/// In both cases NULL work fields are filled from the edition (§7.3). Returns (work_id, edition_id).
/// InsertOnly returns a found edition untouched. A `legacy` edition becomes `dnb`/`openlibrary` like a work.
/// An `openlibrary` work always has an ol_work_id: a `legacy` work upgraded by an Open Library record takes
/// the record's ol_work_id (details_fetched_at stays NULL, so CAT's first detail view fetches the full
/// record). When another work (merged away or not) already holds that id, the work stays `legacy`, a
/// visible merge candidate, though its legacy-ranked title, subtitle and authors are still upgraded.
pub async fn upsert_edition_with_work(conn: &mut PgConnection, work: &WorkRecord, ed: &EditionRecord,
    grouping: WorkGrouping, mode: Mode) -> sqlx::Result<(i64, i64)>;
/// One mapped DNB record (§7.5), for HAR and CAT's SRU lookup; use it instead of calling
/// upsert_edition_with_work per edition. Takes the advisory locks of every edition and of the match key in
/// one call, ingests the first edition with DnbTitleAuthor and the others with Into(that edition's work),
/// so a record's co-publisher editions never split across works (e.g. when its first ISBN already belongs
/// to an Open Library work). Returns (work_id, edition_id) per edition, in record order; an edition found
/// under another work keeps it.
pub async fn upsert_dnb_record(conn: &mut PgConnection, rec: &DnbRecord, mode: Mode)
    -> sqlx::Result<Vec<(i64, i64)>>;
/// An edition of a known work (OL editions list, a user adding an edition). Same find-or-insert as above,
/// but never touches the work's own fields except the NULL fill of §7.3. A found edition of another work keeps
/// its work: the returned work_id says so (CAT answers 409).
pub async fn upsert_edition(conn: &mut PgConnection, work_id: i64, rec: &EditionRecord, mode: Mode)
    -> sqlx::Result<(i64, i64)>;
/// DNB delivered status="deleted" for this IDN: sets editions.withdrawn_at on every edition whose newest
/// delivering record it is (editions.dnb_record_idn), so a multi-ISBN record is withdrawn with all its ISBNs and
/// an edition a newer record delivered again stays live. Never inserts. (INT changed the rule from `dnb_idn = $1`,
/// which withdrew only the first ISBN's edition; the signature is unchanged.)
pub async fn withdraw_dnb_edition(conn: &mut PgConnection, dnb_idn: &str) -> sqlx::Result<()>;
/// The media row for an Open Library cover id (insert-only). None when that row was removed by a takedown,
/// so no caller can attach it again, and for ids <= 0.
pub async fn ol_cover_media(conn: &mut PgConnection, ol_cover_id: i64) -> sqlx::Result<Option<i64>>;
pub enum CoverTarget { Work(i64), Edition(i64) }
/// Sets the cover under the provenance rules (`by` = the writer). With `only_if_empty` the UPDATE carries
/// `AND cover_media_id IS NULL` (a first user upload; false = someone else was first). Refuses a removed
/// media row (and a non-cover row). Returns whether the cover changed (false for a missing target or the same
/// media id). Also fills a NULL work cover from an edition cover.
pub async fn set_cover(conn: &mut PgConnection, target: CoverTarget, media_id: i64, by: Source,
    only_if_empty: bool) -> sqlx::Result<bool>;
/// Takedown: clears every works/editions cover_media_id that points at the media row (and its provenance).
pub async fn clear_media(conn: &mut PgConnection, media_id: i64) -> sqlx::Result<u64>;
/// An approved edit request: writes the fields and marks them "admin". `fields` uses the edit-request keys:
/// works title, subtitle, authors, year, description, subjects, pages; editions title, subtitle, publisher,
/// language, format, pages, publication_year, planned_publication_date ("YYYY-MM-DD", year -5000..=3000 like
/// the work year it may fill). There is no `isbn` key
/// (identity, not an editable field). Null clears a field (not a work title) and records `admin` for it, so no
/// source fills it again (§7.2 "Provenance"). Changing `authors` drops the work_authors links. Every key is checked before
/// anything is written. 404 NO_SUCH_BOOK for a missing
/// target; 400 `Unknown field: <key>` / `Invalid value for <key>` (admin-only strings, not mapped by the app).
pub async fn apply_admin_fields(conn: &mut PgConnection, target: CoverTarget, fields: &serde_json::Map<String, Value>)
    -> ApiResult<()>;
// Pure rules, public for CAT and tests:
pub fn clean_description(raw: &str) -> String;  pub fn clean_subjects(values: &[String]) -> Vec<String>;
pub fn name_key(name: &str) -> String;  pub fn author_surname(name: &str, sort_name: Option<&str>) -> String;
pub fn match_key(title: &str, surname: &str) -> String;  pub fn search_extra(work_title: &str, edition_titles: &[String]) -> String;
```

**How ingest writes** (W1 implementation notes that callers rely on):
- Each write loads the row `FOR UPDATE`, merges the record field by field, and writes it once. Records are sanitised first: values outside the schema's CHECKs are dropped (pages, years, ratings, cover ids <= 0, malformed OL ids, overlong identifiers, a planned date whose year is outside -5000..=3000) or truncated by characters (titles, names, publisher, description); text is stored as Unicode NFC without control characters (U+0000, which Postgres `text` refuses and Open Library JSON sometimes carries, and every other one except tab and line breaks). Admin edit values get the same NFC and control-character cleaning.
- **Concurrency:** writes of an edition take transaction-level advisory locks on its ISBN, IDN and OL edition id, and `DnbTitleAuthor` on the match key (the two-key lock space `(0x62630100, hash)`, separate from `jobs::lock_ids`); `upsert_dnb_record` takes all of a record's keys at once. Inserts also use `ON CONFLICT DO NOTHING` and merge into the winner. Callers run a multi-statement ingest in one transaction (HAR per page, CAT per lookup), or the locks end with each statement.
- **Deadlocks:** the advisory locks of one call are taken in a fixed order before its row locks, so concurrent single-record ingests serialise on their identifiers and deadlock only in rare cases (an IDN moving between two editions of one work while the other edition is re-ingested). Locks are not ordered across calls: a transaction that ingests several records (HAR's page) holds the advisory and row locks of earlier records while it asks for the next ones, and a concurrent ingest (a CAT lookup, another page) can hold them the other way round. Postgres then aborts one transaction with SQLSTATE 40P01 after `deadlock_timeout`. Every ingest caller is ready for that: the aborted transaction is rolled back and retried as a whole (HAR retries the page a few times, then fails the run; CAT retries the lookup once). Test with `error::is_deadlock`. HAR may instead commit per record or small batch, as long as the cursor is written with the last one (work-plan HAR).
- **Covers and takedowns:** the media rows a write attaches are locked `FOR SHARE` (`ol_cover_media`, `set_cover`) before any catalog row is locked, and the lock lasts until the caller's transaction ends. A takedown's `UPDATE media SET removed_at` then waits for an attaching transaction, and its `clear_media` sees the new reference; MED's takedown sets `removed_at` before it calls `clear_media`, in one transaction.
  - **Lock order around a picture** (ADM #54): everything that touches a picture's reports or cover requests locks its media row first, so takedowns, reports, dismissals and cover approvals take turns instead of deadlocking:
    - **Takedown** (`media::reports::take_down_locked`, for the direct takedown and `remove`): media `FOR UPDATE` → works and editions (`clear_media`) → reports → `catalog_edit_requests` (it rejects the picture's open `cover_replace` requests with the note `media::reports::TAKEN_DOWN_NOTE`). **A profile picture** (ADM2, security review Phase 2 Wave 2) then takes down the owner's other live profile pictures with the same `sha256` as received (the same file uploaded again while the first was held, §5.6) the same way: they are locked `FOR UPDATE` in id order after the first, and their open reports are resolved too. An avatar replace locks the owner's avatar rows in id order (§5.6), so a copy with a lower id than the picture taken down is locked in the other order; the owner's row, which both lock first (below), keeps them apart.
    - **Cover approval** (`approve_edit_request`): the request's picture `FOR SHARE` (`lock_request_media`, before the request row) → the request row `FOR UPDATE` → catalog rows.
    - **Report:** a repeat (the reader's open report, or their `removed` one of a picture that is down) is answered from one plain read before the transaction, with no lock. A new report takes `pg_advisory_xact_lock(0x6263_0200, hashtext('<user id>:<media id>'))` (the two-key space for one reader's reports of one picture, beside ingest's `0x6263_0100` and `jobs::lock_ids`) → for a profile picture, the owner's `users` row `FOR KEY SHARE` → media `FOR SHARE` → the reports row. The reader's turn comes before the picture, so of one reader's requests for one picture only the one whose turn it is holds the picture, and repeats hold nothing: otherwise one account could keep the picture share-locked without a break and a takedown would wait until the request deadline (security review, Phase 2 Wave 1 #1). A profile picture (ADM2) is reported the same way, with the same key. **A report of a work or edition a reader added** (ADM2, `catalog::reports`) takes the same space with the key `<user id>:work:<work id>` or `<user id>:edition:<edition id>`, then writes the reports row; it locks no catalog row. No other path takes that advisory lock, so it cannot form a cycle with the media row.
    - **Dismiss:** media `FOR NO KEY UPDATE` → reports (for a profile picture, the owner's row `FOR KEY SHARE` first, as below). A held profile picture (§5.6) whose reports are dismissed is then deleted with its row (`delete_held_avatar`, unless a report of it is still open), its files after the commit.
    - **Avatar replace or delete** (`media::avatar`, which reads reports and writes none): the user row `FOR UPDATE` → the user's avatar rows `FOR UPDATE` in id order → a new statement reads which have an open report (§5.6). An account deletion (§5.6) locks the user row `FOR UPDATE` and then deletes the avatar rows.
    - **The owner of a profile picture before the picture** (`media::reports::lock_owner`): a report, a takedown (direct or `remove`) and a dismissal of a profile picture lock the owner's `users` row `FOR KEY SHARE` before the media row. A report's row and a notice's name the owner (`target_user_id`), whose foreign key takes that lock anyway, and a takedown locks the owner's copies of the picture; taken only after the picture, each could wait for the owner's row while an avatar upload or an account deletion that held it waited for the picture, and Postgres aborted one of them (40P01). An account deleted meanwhile leaves no row, and its pictures are gone too: the report is 404, the takedown 404, the decision finds the picture gone. Race tests: `media_reports.rs::a_report_racing_the_owners_avatar_upload_does_not_deadlock`, `a_report_racing_the_owners_account_deletion_does_not_deadlock`, `a_takedown_with_a_notice_waits_for_the_owners_avatar_upload`, `removing_an_avatar_with_an_older_copy_waits_for_the_owners_upload`.
    - Each lock has a race test that fails without it (`catalog_requests.rs::a_takedown_rejects_a_cover_request_an_admin_is_approving`, `media_reports.rs::a_report_filed_during_a_takedown_sees_the_removal`, `a_dismissal_waits_for_a_report_being_filed_and_closes_it_too`, `two_reports_at_once_by_one_reader_are_one_report`), and so does the report's order (`repeat_reports_take_no_lock`, `a_report_waiting_for_its_turn_does_not_hold_off_a_takedown`).
  - **Lock order of a catalog decision** (ADM2, `catalog::reports::decide`, the resolve of a `work` or `edition` report): the work row `FOR UPDATE` (for an edition, its work as it is now, since a merge moves editions) → the edition row `FOR UPDATE` → on "remove", `ingest::apply_admin_fields` → the reports rows. That is ingest's and the direct admin edit's order. `decide` reads `merged_into_id` under the work's lock, so a merge that commits between the report read and the decision is refused (409) too (`catalog_reports.rs::a_merge_after_the_report_never_hands_the_survivor_to_remove`, `two_reports_of_a_book_at_once_by_one_reader_are_one_report` for the advisory keys).
- `upsert_work_stubs` writes `author_names` only (author records come with the full record) and inserts records without an `ol_work_id` one by one.

### 7.3 Rules ingest maintains

- **Covers never come back after a takedown.** `ol_cover_media` returns None for a removed row; a record whose `ol_cover_id` is removed sets no cover; `set_cover` refuses a removed row.
- **Covers from sources never replace a user's or an admin's cover.** A cover set by a first upload has provenance `user`; an approved `cover_replace` has `admin`. A Refresh only sets `cover_media_id` when it is NULL or has source rank (an Open Library cover may replace an older Open Library cover).
- **NULL work fields are filled from editions.** After any edition write, a NULL `works.pages`, `first_publish_year` (from `publication_year`, else the year of `planned_publication_date` when it is within -5000..=3000) and `cover_media_id` are filled from that edition, with the edition's provenance. So a work built from DNB or legacy facts, or a work whose only cover is a user photo of one edition, shows year, pages and cover outside an edition context too.
- **Search sees edition titles.** `works.search_extra` is rebuilt from the work's edition titles that differ (by case fold) from the work title.
- **`works.match_key`** = `casefold(collapse_ws(title))` + U+001F + `casefold(surname of the first author)`, where the surname is the part before the comma of an inverted name (`authors.sort_name`), else the last word of the name (up to its first comma). A work without author records uses the first name in `author_names`. A record without any author still gets a match key, but `DnbTitleAuthor` never groups by it (each such record makes its own work; its other ISBNs join through `upsert_dnb_record`). Rust computes it; Postgres `lower()` is not a case fold. `search_extra` joins the distinct edition titles with " / " (at most 5000 characters).
- **Authors without an external id** (no OL id, no GND id) are deduplicated by `authors.name_key` (`casefold(collapse_ws(name))`); rows with an id are deduplicated by the id.
- **Never overwrite from client input.** Client requests reference works and editions by id; the only client-entered values are user-added works and editions (provenance `user`) and edit requests (provenance `admin` once approved).
- **Description cleaning** is the union of the two legacy cleaners: trailer removal plus markdown flattening (docs/phase1/spec/catalog.md §5.7). It applies to Open Library and DNB records; text a reader, an admin or the legacy import entered is stored as given (trimmed, length-capped).
- **Subject cleaning:** drop blanks and entries that contain `:` or `,` or exceed 40 characters, dedupe case-insensitively, keep at most 12. Machine tags `^[a-z_]+:\S` are dropped too. Same scope as description cleaning; other sources keep at most 50 non-blank subjects.
- **Rounding** of rating means uses `text::round_half_even`.

### 7.4 Remote resolution (`lookup.rs`)

```rust
#[async_trait::async_trait]
pub trait CatalogLookup: Send + Sync {
    /// Local editions first; then the remote route below; persists hits through ingest. Ok(None) = a definite
    /// miss (every source answered). Err(Throttled/Unavailable) = no definite answer.
    async fn resolve_isbn(&self, conn: &mut PgConnection, isbn: &Isbn13) -> Result<Option<Resolved>, LookupError>;
    /// The remote route only, skipping the local step; persists the hit, which upgrades an existing bare-facts
    /// or user edition with that ISBN under the provenance rules. Used by "add a book or edition with an ISBN"
    /// (a hit means 409) and by `rematch-legacy`.
    async fn resolve_isbn_remote(&self, conn: &mut PgConnection, isbn: &Isbn13) -> Result<Option<Resolved>, LookupError>;
    /// local FTS → Open Library search (limit 5, first hit); persists the hit.
    async fn resolve_text(&self, conn: &mut PgConnection, title: &str, authors: &str) -> Result<Option<Resolved>, LookupError>;
}
pub struct Resolved { pub work_id: i64, pub edition_id: Option<i64> }
#[derive(Debug, thiserror::Error)] pub enum LookupError { #[error("throttled")] Throttled, #[error("unavailable")] Unavailable, #[error(transparent)] Db(#[from] sqlx::Error) }
/// The local step every resolution starts with: the edition with this ISBN (a withdrawn one too) and its current
/// work. CAT's resolve_isbn, FakeCatalog and IMP's --offline mode share it.
pub async fn local_isbn(conn: &mut PgConnection, isbn: &Isbn13) -> sqlx::Result<Option<Resolved>>;
```

**Remote route by registration group** (the plan: DNB for German books, Open Library for English ones; DNB SRU is a rate-limited fallback):
- `978-3…` (German-language group): DNB SRU, then Open Library. When DNB is throttled or unavailable, the lookup falls through to Open Library; an Open Library hit is persisted with `openlibrary` provenance, and a later DNB record for that ISBN (harvest or SRU) overwrites it at equal rank.
- Every other group: Open Library only. DNB SRU is never called for them.
- After a DNB hit, one Open Library `/isbn/{isbn13}.json` call attaches `covers[0]` through `ol_cover_media` (a cover id, so the ISBN-cover limit does not apply) and sets the work's `details_fetched_at`. Harvested DNB works get the same one call on their first detail view (`details_fetched_at` NULL).
- When no source gave a definite answer (a remote step was throttled or unavailable and nothing hit), the result is `Err(Throttled)` or `Err(Unavailable)`, never `Ok(None)`, so callers do not mistake it for a miss.

### 7.5 DNB MARC mapping (`dnb_marc.rs`, W1, frozen and fully implemented)

One mapping for both DNB channels: HAR's OAI-PMH harvest and CAT's SRU lookup. Each feeds it one MARC21-xml `<record>` element; envelopes (OAI headers, resumption tokens, SRU wrappers) stay in the packages.

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct DnbRecord { pub idn: String, pub work: WorkRecord, pub editions: Vec<EditionRecord> }
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MarcError { #[error("malformed MARC21-xml: {0}")] Xml(String), #[error("the record has no 001 control field")] NoIdn,
                     #[error("the record has no usable title")] NoTitle }
/// Err for a record without 001 or without a usable title. The parser only collects the fields below, so 520
/// and 856 never even reach memory; elements around the MARC record (an OAI header) are ignored.
pub fn map_record(record_xml: &str) -> Result<DnbRecord, MarcError>;
```

- **CC0 fields only:** 001 IDN; 020 ISBNs (with the $c binding); 245 $a/$b title and subtitle; 100/700 names with $4 relator (`aut` → authors, `trl` → translator, `ill` → illustrator, `edt` → editor) and $0 GND ids; 264/260 publisher and year; 041/008 language; 300 pages; 263 planned date (`YYYYMM` → first of the month); 653 `(Produktform)…`; 337/338. **Never** 520, 856 or any other VLB text or URL.
- **Text:** DNB's non-filing markers (U+0098 … U+009C around "Die", "The") are dropped, and the decomposed Unicode DNB delivers ("o" + U+0308) is composed to NFC; trailing ISBD separators (` /`, ` :`, ` ;`, ` =`, `,`) are trimmed.
- **Title:** 245 `$a`, plus `$n` (". " + part number) and `$p` (". " + part name, ", " after a `$n`), so the volumes of a series ("Gesammelte Werke. Bd. 21.") never share a match key; `$b` is the subtitle.
- **Names** come in natural order: "Paulitz, Udo" → `name` "Udo Paulitz", `sort_name` "Paulitz, Udo". `author_names` joins the `aut` names in natural order with ", ". A 100 without `$4` counts as `aut`; other relators (`aui`, `waw`, `ctb`, …) and a 700 without `$4` are not mapped; each person appears once per role. The GND id is the `$0` with prefix `(DE-588)`.
- **ISBNs:** the ISBN-10 and ISBN-13 of one book (DNB lists both) are one edition. Only the first edition carries `dnb_idn` (`editions.dnb_idn` is unique); every edition carries the record's IDN as `dnb_record_idn` (`editions.dnb_record_idn`, not unique, migration 1200), so a deleted header withdraws every ISBN of the record.
- **Imprint:** 264 with ind2 `1` first, then other 264s, then 260. An edition whose 020 `$q` names a 264 `$b` (co-publishers) gets that publisher, else the first. The year is the first four-digit number of `$c` ("September 2026", "[2026]").
- **Placeholder years:** DNB announces books a year or two ahead and uses far-future years for "not known yet" (recorded records carry 2040/204001, 2050/205004 and 9999). A year more than five years after the current year counts as missing, in 260/264 `$c` and in 263 alike, so `map_record` depends on the clock. Language: the first valid 041 `$a`, else 008/35-37. Pages: the number before "Seiten"/"S."/"pages"/"p." in 300 `$a`.
- **Work facts** (for `upsert_edition_with_work`): title and subtitle from 245; authors from the `aut` names; `first_publish_year` = the edition's `publication_year`, else the year of the planned date; `pages` = the edition's pages.
- **Several ISBNs in one record:** one edition per ISBN in the registration group of the first 020 ISBN (foreign co-publisher ISBNs are dropped), all under the same work. Callers ingest a record with `ingest::upsert_dnb_record` (§7.2), which keeps them on one work; calling `upsert_edition_with_work(…, DnbTitleAuthor, …)` per edition can split them when the first ISBN already belongs to a non-DNB work.
- **Format**, per edition: with one ISBN, 653 `(Produktform)` first (`Hardback` → hardcover; `Paperback / softback` → paperback; `Digital…`/`E-book` → ebook; `Audio…` → audiobook; anything else, e.g. `Calendar`, `Multiple-item retail product` → other), then the 020 $c binding, then 337/338. With several ISBNs, each edition's own 020 $c binding first (`Gb.`, `geb.`, `Gebunden`, `Gewebe`, `Hldr.`, `Pp.`, `Hperg.`, `Festeinband`, `Leinen`, `Ln.`, `Lw.`, `Hardcover` → hardcover; `kart.`, `KT`, `Broschur`, `brosch.`, `Broschiert`, `Klappenbroschur`, `Pb.`, `Paperback`, `Taschenbuch` → paperback; matched as words, case-insensitively, before the first `:`), then a single 653 value, then 337/338 (`cr` → ebook; audio carriers such as `sd` → audiobook; 337 `s` → audiobook, `c` → ebook). Nothing usable → `other`, so a DNB edition always has a format.
- **Deleted records:** the OAI header `status="deleted"` is the caller's to detect; it calls `ingest::withdraw_dnb_edition` with the header's IDN and never maps the metadata. That withdraws the editions the record delivered last (`dnb_record_idn`), all its ISBNs included.
- **Deleted records still carry metadata** in DNB's OAI responses, so the caller must check the header before mapping.
- Tests (W1): `tests/fixtures/dnb_marc/` holds single records copied from DNB responses recorded on 2026-10-27 (reiheN and reiheA OAI pages, one SRU search): a single-ISBN hardback with 263, an inverted multi-author name list, translators and editors with GND ids, a translator without one, a multi-ISBN co-publisher record, a record with 856 links plus a hand-added 520 (DNB's MARC carries blurbs only as 856 links; no recorded record had a 520), and a deleted-header record. No recorded record combined kart./Gewebe/Hperg. bindings, so `dnb_marc.rs` unit tests cover the binding vocabulary, foreign co-publisher groups and the carrier fallbacks with small inline records. `tests/dnb_marc.rs` maps and ingests every fixture through `upsert_dnb_record` and checks that no VLB text or URL is stored.

### 7.6 Merge rules (`merge_rules.rs`, W1, frozen; pure functions)

The collision rules for two works becoming one, used by IMP while importing and by CAT's `merge_works` (live data):

```rust
pub struct ShelfFacts { pub id: i64, pub status: ShelfStatus, pub updated_at: OffsetDateTime, pub rating: Option<i32>,
    pub rating_visibility: Visibility, pub take: String, pub dnf_reason: String, pub progress: Option<i32>,
    pub started_at: Option<OffsetDateTime>, pub finished_at: Option<OffsetDateTime>, pub edition_id: Option<i64> }
pub fn shelf_status_rank(s: ShelfStatus) -> u8;   // finished 4 > did_not_finish 3 > currently_reading 2 > want_to_read 1
/// Winner: higher status rank, then newer updated_at, then lower id. `Greater` means `a` wins, so
/// `entries.iter().max_by(|a, b| shelf_winner(a, b))` is the winner.
pub fn shelf_winner(a: &ShelfFacts, b: &ShelfFacts) -> std::cmp::Ordering;
/// Copies the loser's rating, take, dnf_reason, progress, started_at, finished_at, edition_id into the
/// winner where the winner's value is NULL/'' and the loser's value is allowed for the winner's status
/// (rating/take: finished; dnf_reason: did_not_finish; progress: currently_reading; started_at: any but
/// want_to_read; finished_at: finished; edition_id: any). A copied rating or take from an `only_me` loser makes
/// the winner `only_me`; a copied start is clamped to the finish.
pub fn merge_shelf_fields(winner: &mut ShelfFacts, loser: &ShelfFacts);
/// Replaces loser entry ids by the winner's in a layout's rows and spine keys (dropping the loser where the
/// winner is already placed or has a spine). Other shapes are left alone.
pub fn remap_layout(layout: &mut serde_json::Value, loser_entry: i64, winner_entry: i64);
```

`catalog::merge_works(conn, loser: i64, winner: i64) -> sqlx::Result<MergeReport>` (CAT, in `catalog/merge.rs`) applies them to live data in one transaction, locking both users' rows that are affected: shelf entries (winner kept, loser fields merged, layouts remapped, positions renumbered per (user, status)), favourites (keep the lower position, compact 1..n), nominations (keep the earliest per vote, move ballots, re-point `winner_nomination_id`), editions and `legacy_work_keys` moved to the winner, posts/quotes/events/picks/reports re-pointed, open edit requests on the loser rejected with a note, `ol_work_id` moved to the winner when it has none (cleared on the loser first), `merged_into_id` set. Tests: users who have both works on their shelf and as favourites, and a vote that nominated both.

## 8. Media store (W1 interface, frozen; MED implements the rest)

```rust
// media/shared/store.rs
#[async_trait::async_trait]
pub trait MediaStore: Send + Sync {
    async fn put(&self, key: &str, bytes: bytes::Bytes, content_type: &str) -> anyhow::Result<()>;
    async fn get(&self, key: &str) -> anyhow::Result<Option<bytes::Bytes>>;
    async fn delete_prefix(&self, prefix: &str) -> anyhow::Result<()>;
}
pub struct FsMediaStore { root: PathBuf }   // writes to a temp file, then renames (atomic)
impl FsMediaStore { pub async fn check_writable(&self) -> anyhow::Result<()>; }  // DEP (#51): startup probe, §3 step 2a
pub struct MemMediaStore { … }             // tests: new(), keys() (sorted), content_type(key)
// Both refuse keys that are not relative paths of plain components ("..", absolute, empty).

// media/shared/urls.rs
pub fn new_public_id() -> String;                               // 26 chars [a-z0-9] from auth::random
pub fn storage_prefix(kind: &str, public_id: &str) -> String;  // "covers/<pp>/<public_id>" / "avatars/<pp>/<public_id>",
                                                                // <pp> = the first two characters of public_id
pub fn object_key(kind: &str, public_id: &str, variant: &str) -> String; // storage_prefix + "/" + variant
pub fn media_url(public_id: &str, variant: &str) -> String;     // "/media/{public_id}/{variant}"
pub fn cover_urls(public_id: Option<&str>) -> (Option<String>, Option<String>); // (l, s)
pub fn avatar_url(public_id: Option<&str>) -> Option<String>;   // variant "a"

// media/shared/mod.rs
pub struct NewMedia<'a> { pub kind: &'a str, pub source: &'a str, pub content_type: &'a str,
    pub variants: &'a [&'a str], pub width: i32, pub height: i32, pub byte_size: i64,
    pub sha256: [u8; 32] /* of the bytes as received (schema comment on media.sha256) */, pub dominant_color: Option<String>,
    pub uploaded_by: Option<i64>, pub created_at: Option<time::OffsetDateTime> /* None = now(); IMP passes avatar_updated_at */ }
/// Inserts the media row for bytes the caller has already stored under object_key(kind, public_id, variant)
/// for every variant. The caller generates public_id (new_public_id) first, because the keys contain it.
/// Sets storage_key = storage_prefix(kind, public_id). Returns the row id. (`media::shared` re-exports
/// FsMediaStore, MediaStore and MemMediaStore.)
pub async fn register(conn: &mut PgConnection, public_id: &str, m: &NewMedia<'_>) -> sqlx::Result<i64>;
/// True when a removed media row has this hash (uploads of a taken-down picture are refused).
pub async fn is_removed_hash(conn: &mut PgConnection, sha256: &[u8; 32]) -> sqlx::Result<bool>;
```

Open Library cover rows are created by `ingest::ol_cover_media` with `storage_key` NULL; MED fills `storage_key`, `content_type`, `variants`, size, hash and colour after the first fetch. The export (W1) and account deletion (W1) read bytes and delete prefixes with these functions, so W1, IMP and MED agree on the key layout.

**MED implements:**
- **The image pipeline** (`image` crate with decode `Limits`):
  - Sniff the magic bytes first.
  - Check pixels (≤ 25,000,000) before decoding.
  - Apply EXIF orientation via `ImageDecoder::orientation`.
  - Take the first frame of a GIF or animated WebP.
  - Composite alpha onto white.
  - Avatars: centre-crop and Lanczos3 resize to 256.
  - Covers: resize to ≤ 800 px wide (`l`) and ≤ 240 px wide (`s`).
  - Encode with `webp` (lossy, quality 82).
  - Metadata is dropped by construction: we never copy EXIF, ICC or XMP.
- `/media/…` serving. **`removed_at` is checked first** on every path (serve, fetch, colour backfill): a removed row answers 410 and is never fetched again, whatever `storage_key` says. Every non-200 answer (404, 410, 429, 502) carries `Cache-Control: no-store`; only a 200 is `immutable`.
- Fetching Open Library covers by cover id (through `outbound::get_bytes` with `RetryPolicy::MEDIA`, which checks every redirect and caps the body at 1.5 MB, §10.3). A request for a not-yet-fetched cover waits up to **10 s** for the `ol_covers` throttle (not the 2 s of other request paths), and concurrent requests for one media id are coalesced.
- **Prefetch:** `MediaService::prefetch` (§3) feeds a bounded queue (500 ids, dropped when full) that one background task drains through the same throttle, fetching the covers of persisted search, trending and subject hits before the browser asks. Started by `jobs::spawn_all`; not in tests.
- Dominant colour (the legacy median-cut algorithm, hand-written; `color_quant` and `imagequant` are not used).
- Cover upload: a first cover goes through `ingest::set_cover(…, Source::User, only_if_empty = true)`; when that returns false (a cover exists, or a concurrent upload won), the upload becomes a `cover_replace` request instead. An upload whose original-bytes hash matches a removed row is refused (400 `This picture was removed`).
- Reports and takedown: a takedown deletes the bytes and, in one transaction, sets `removed_at` and then calls `ingest::clear_media` (that order lets it wait for an ingest that is attaching the row, §7.2 "Covers and takedowns"). Since ADM2 a profile picture is reported and taken down like a cover; a removed one shows nowhere, because every query that builds an `avatar_url` joins `media … AND removed_at IS NULL` (`dto::user_summaries`, `visibility::visible_user`, `auth::user_out`, the club, post and pick DTOs), so `users.avatar_media_id` keeps pointing at the removed row (clearing it would lock the user row after the media row, against the avatar upload's order).
- Avatar endpoints (`PUT` deletes the previous avatar row in the same transaction and its bytes after commit, except a reported one, which is held for the decision, §5.6; both return `auth::user_out`).
- Spine-colour backfill.

## 9. Email (W1)

```rust
#[derive(Clone, Debug)] pub struct Email { pub to: String /* a bare address (§5.5), never "Name <addr>" */, pub subject: String, pub text: String }
#[async_trait::async_trait]
pub trait EmailSender: Send + Sync { async fn send(&self, msg: Email) -> anyhow::Result<()>; }
pub struct LogEmail;        // logs to and subject, plus the full body, at INFO: the dev transport (EMAIL_TRANSPORT=log, DEV_MODE only)
pub struct SmtpEmail { … }  // lettre AsyncSmtpTransport<Tokio1Executor>, rustls, pooled; from SMTP_URL;
                            // parses `to` as lettre::Address and sends to Mailbox::new(None, address)
// tests: CapturingEmail (tests/common) keeps every message
pub fn test_mail(config: &Config, to: &str) -> Email;   // DEP (#51): the `send-test-mail` message (§16)
```

- **Headers** (DEP #51): `SmtpEmail` sets `Message-ID: <uuid-v4@domain of EMAIL_FROM>` on every mail (the UUID from `auth::random`; lettre 0.11 adds none by itself), and lettre adds `Date`.
- **`email::test_mail`** is English: subject `<SERVICE_NAME> test mail`, a body naming the service and `PUBLIC_URL`, no link that does anything. It lives in `email/mod.rs`, not in `templates.rs`.

- **Templates** are plain text only, in English and German by `users.locale`, in `email/templates.rs`: verify, reset, email changed (to the old address), and "someone tried to use your address" (sent to a verified address instead of a verification link when a sign-up or an email change named it, §5.5; it says what was tried and links to login and password reset, and contains no link that changes anything), since ADM (#54) the two admin mails, and since ADM2 the two decision notices.

  ```rust
  pub struct Site<'a> { pub service: &'a str, pub public_url: &'a str }        // auth::mail::site(&config)
  pub struct Recipient<'a> { pub to: &'a str, pub username: &'a str, pub locale: &'a str }   // "de" = German
  pub enum Attempt { SignUp, EmailChange }
  pub fn verify_email(site: Site, to: Recipient, link: &str) -> Email;
  pub fn reset_password(site: Site, to: Recipient, link: &str) -> Email;
  pub fn email_changed(site: Site, to: Recipient, new_address: &str) -> Email;
  pub fn address_in_use(site: Site, to: Recipient, attempt: Attempt) -> Email;  // links: {PUBLIC_URL}/login, /forgot-password
  // ADM (#54), to instance admins (§16 "admin notification"): counts and the oldest item's age only,
  // never a reporter, another username or what a report says.
  pub struct QueueNews { pub new: i64, pub open: i64, pub oldest_hours: i64 }
  pub fn admin_reports(site: Site, to: Recipient, news: QueueNews) -> Email;          // link: {PUBLIC_URL}/admin?tab=reports
  pub fn admin_requests_digest(site: Site, to: Recipient, news: QueueNews) -> Email;  // link: {PUBLIC_URL}/admin?tab=requests
  // ADM2 (#54, Q12 A), the decision notices (`admin::notices`), named as reported, never the other side:
  pub enum Reported<'a> { Cover(Option<&'a str>) /* the book's title */, Avatar, Work(&'a str), Edition(&'a str) }
  pub enum Outcome { Removed, Kept, Gone }   // Gone: replaced or deleted (a picture), merged (a book)
  /// To a reporter (DSA Art. 16 (5)); Kept says where to object (`contact` = CONTACT_EMAIL, else "the admins of {service}").
  pub fn report_decision(site: Site, to: Recipient, what: Reported, outcome: Outcome, contact: Option<&str>) -> Email;
  /// To whoever uploaded or added what was removed (DSA Art. 17): what and what changes, the ground (`reason`,
  /// or "an admin found that it breaks the law or the terms of use"), that a person decided, how to object, {PUBLIC_URL}/terms.
  pub fn statement_of_reasons(site: Site, to: Recipient, what: Reported, reason: Option<&str>, contact: Option<&str>) -> Email;
  ```
- **The legal footer** (#53, ADM2): every template goes through the one helper `mail()`, which ends the text with a signature separator and `Imprint: {PUBLIC_URL}/imprint · Privacy: {PUBLIC_URL}/privacy` (German `Impressum: … · Datenschutz: …`). The unit test `every_template_ends_with_the_legal_footer` scans the file for every `pub fn … -> Email`, so a new template fails until it is covered. `email::test_mail` is not a template and has no footer.
- **Decision notices** (ADM2, Q12 A; `admin::notices`): `notices::decided(conn, config, admin_id, what, removal, gone)` builds them inside the decision's transaction, after its last update of `reports`: one `report_decision` to each reporter of the reports this admin resolved in this transaction (`resolved_by = admin AND resolved_at = now()`, so one decision mails each reporter once), and, when `removal` is given and names an owner, one `statement_of_reasons` to that owner (a reporter who is also the owner gets the statement only). Only verified addresses, in `users.locale`; a suspended account is still told; a notice recorded from an email (`via = 'notice'`) has no reporter account. `Notices::send` runs after the commit, so a rollback (a 409, an error) sends nothing: one mail after the other through `state.email` (15 s each, a failure logged; not `auth::mail::deliver`'s per-address limits), in a spawned task the handler awaits, so the admin's answer comes once the mails are handed to the transport and closing the page cancels nothing. Neither side is named to the other, and the report's `details` go to nobody.
- **Provider:** an EU-hosted transactional provider with a DPA, reached over SMTP. The choice is Rafael's decision; record it in #39. SMTP keeps any provider swappable.

## 10. Rate limiting, client IP, outbound HTTP

### 10.1 Rate limits

All rate limiting is in-process (one server, about 10k accounts).

```rust
pub struct SlidingWindow { limit: u32, window: Duration, hits: moka::sync::Cache<String, Arc<Mutex<VecDeque<Instant>>>> }
#[derive(Debug, Clone, Copy)] pub struct RetryAfter(pub u64);   // seconds until the oldest hit leaves the window
impl RetryAfter {
    /// 429 with `msg` (TOO_MANY_ATTEMPTS for auth limits, SLOW_DOWN for the others) and Retry-After.
    pub fn into_error(self, msg: &'static str) -> AppError;
}
impl SlidingWindow {
    pub fn new(limit: u32, window: Duration) -> Self;
    pub fn is_limited(&self, key: &str) -> bool;   // >= limit hits within window (monotonic clock)
    pub fn check(&self, key: &str) -> Result<(), RetryAfter>;   // is_limited with the wait, records nothing
    pub fn hit(&self, key: &str);                  // record one
    pub fn check_and_hit(&self, key: &str) -> Result<(), RetryAfter>;   // atomic per key; a refusal records nothing
    pub fn clear(&self, key: &str);
}
pub struct RateLimits { pub login_failures, pub password_check_session, pub password_check_user, pub login_unknown,
    pub login_ip, pub signup_ip, pub reset_ip, pub reset_account, pub email_send_user, pub email_target, pub notice_target,
    pub invite_preview_ip, pub invite_accept_user, pub refresh_user, pub upload_user, pub report_user, pub report_request_user, pub edit_request_user,
    pub custom_book_user, pub export_user, pub avatar_user, pub catalog_user: SlidingWindow,
    pub upload_gate, pub catalog_gate, pub export_gate: KeyedGate,   // at most N at once per key (`u:<id>`)
    pub exports: Arc<Semaphore> }                                    // exports streaming on the instance
/// At most `permits` operations at once per key: `try_enter` refuses (None) when they are taken, `enter` waits.
/// The GatePass gives the turn back on drop; a key's entry lives only while someone holds or waits for it.
impl KeyedGate { pub fn new(permits: usize) -> Self; pub fn try_enter(&self, key: &str) -> Option<GatePass>;
                 pub async fn enter(&self, key: &str) -> GatePass; }
/// Rate-limit key for a client address: an IPv4 address as is, an IPv6 address by its /64 prefix
/// (a client with a /64 can otherwise mint a fresh key per request).
pub fn ip_key(ip: IpAddr) -> String;
/// Key for an identifier that names no account (in `login_unknown`): "n:" + the hex SHA-256 of the normalised
/// identifier. One bucket per name, like an account: shared buckets let colliding names fill an address's bucket,
/// and the 429 then told a free address from a verified one.
pub fn unknown_identifier_key(normalised: &str) -> String;
```

- **Login** calls `login_failures.check_and_hit(key)` **before** verifying the password and `clear(key)` on success. The current-password check counts in `password_check_session` and `password_check_user` the same way, before verifying, but nothing clears them (§5.4). `check` followed by `hit` after a failure is not enough: the verify awaits the password semaphore, so concurrent attempts would all pass `check` before the first `hit` lands, and thousands of guesses could reach one account. With `check_and_hit` the counting is unchanged (10 failures, then 429; a success clears), and of N concurrent attempts at most 10 are verified. A refused attempt records nothing, so the right password is refused while the key is blocked, as in Python.
- **Bounded memory:** each limit has its own cache, `max_capacity` 100,000 keys, with `time_to_idle` = the window. Account keys (`u:<id>` and `u:<id>:email`, at most two per account) are bounded by construction. Unknown identifiers have their own cache, `login_unknown`, so a flood of made-up names evicts only other made-up names, never an account's failures.
- **Every test gets a fresh `RateLimits`**, because the harness builds a fresh `AppState`.

| Limit | Threshold |
|---|---|
| `login_failures`: login failures per account and identifier kind (`u:<id>` for the username, `u:<id>:email` for the verified address) | 10 per 600 s, cleared on success |
| `password_check_session`: current-password checks (password change, email change, account deletion) per session (`s:<session_id>`), right or wrong | 10 per 600 s, never cleared |
| `password_check_user`: the same checks per account (`u:<id>`), across its sessions | 30 per hour, never cleared |
| `login_unknown`: login failures per identifier that names no account (`unknown_identifier_key`) | 10 per 600 s |
| Login attempts per IP (`ip_key`) | 30 per 600 s |
| Sign-up per IP | 5 per hour |
| Reset per IP | 5 per hour |
| Reset per account | 3 per hour, silent (the response is still 204); and nothing while a reset link of the account is open (§5.5) |
| `email_send_user`: verification resend and `PUT /api/auth/me/email`, per account | 3 per hour |
| `email_target`: verification and reset links to one address (case-folded), across all accounts | 6 per day, silent (the mail is not sent; the response is unchanged) |
| `notice_target`: the "email changed" and "address in use" notices to one address, each kind its own key (`changed:<address>`, `in_use:<address>`) | 6 per day per kind, silent |
| Invite preview per IP | 60 per hour |
| `invite_accept_user`: `POST /api/invites/{code}/accept` per account (`u:<id>`), valid code or not | 20 per hour (429 `Slow down a little. Try again later.`, checked before the transaction) |
| Refresh | 10 per hour per user |
| Uploads | 30 per day |
| `report_user`: new reports per account, every kind together (a repeat, answered with the earlier report, is none) | 20 per day |
| `report_request_user`: `POST /api/media/{public_id}/reports`, and since ADM2 `POST /api/books/works/{work_id}/reports` and `POST /api/books/editions/{edition_id}/reports`, per account (`u:<id>`), repeats and refusals included | 30 per minute, shared (checked first) |
| Edit requests | 50 per day |
| Custom books | 30 per day |
| Export | 5 per hour; and 2 at once on the instance (`exports`), 1 per account (`export_gate`), held while the download streams (§5.7) |
| `avatar_user`: `PUT /api/auth/me/avatar` per account | 10 per hour |
| `upload_gate`: picture uploads (avatar and cover) per account at once | 1, refused (429) before the body is read |
| `catalog_user`: catalog requests that may call Open Library (search, trending, subjects, an unknown `OL…W` import, a stub's first view) per account | 60 per minute |
| `catalog_gate`: the same requests per account at once | 2; further ones wait for a turn |

- **Auth limits** (login, sign-up, reset) answer 429 `Too many attempts. Wait a few minutes.` with `Retry-After`.
- **Other limits** answer 429 `Slow down a little. Try again later.`
- **The reverse proxy** adds coarse per-IP limits in front (#51).

### 10.2 Client IP (`ClientIp` extractor)

```rust
pub struct ClientIp(pub IpAddr);   // FromRequestParts<AppState>; never rejects
```

- The IP is the socket peer from `ConnectInfo<SocketAddr>`, or `0.0.0.0` when it is absent (tests that don't set it).
- `X-Forwarded-For` is honoured only when the peer is inside a `TRUSTED_PROXIES` CIDR (`ipnet`). The extractor then takes the right-most address that is not itself trusted.
- The legacy `*` default is not carried over. Outside `DEV_MODE`, `TRUSTED_PROXIES` **must be set**: a list of CIDRs, or the literal `none` for a server that faces clients directly. Startup fails otherwise, because behind a proxy an empty list would put every user into the proxy's one IP bucket, and one client could lock everyone out of sign-up and reset.

### 10.3 Outbound HTTP (`outbound.rs`, W1)

```rust
pub struct Outbound {
    pub client: reqwest::Client,          // one client, connection reuse; connect 8 s, read 15 s; rustls
    pub ol_api: HostThrottle,             // openlibrary.org: 3 req/s
    pub ol_covers: HostThrottle,          // covers.openlibrary.org: 5 req/s
    pub dnb_sru: HostThrottle,            // 1 req/s
    pub dnb_oai: HostThrottle,            // 1 req/s
    pub urls: UpstreamUrls,               // from config, overridden in tests
}
impl Outbound { pub fn new(config: &Config) -> anyhow::Result<Self>; }   // fields are pub: tests swap in fast throttles
pub fn user_agent(config: &Config) -> String;
#[derive(Debug, Clone, Copy)] pub struct Throttled;
impl HostThrottle {
    pub fn per_second(rate: f64) -> Self;          // evenly spaced slots, no burst
    /// Time left in the cooldown, e.g. for CAT's "The library is busy. Try again in {n} seconds."
    pub fn cooldown_remaining(&self) -> Option<Duration>;
    /// Waits for a token for up to `max_wait`, else Err(Throttled).
    /// While a cooldown is active (after an upstream 429: Retry-After or 60 s), fails at once.
    pub async fn acquire(&self, max_wait: Duration) -> Result<(), Throttled>;
    pub fn cool_down(&self, for_: Duration);       // capped at 1 h, so an absurd Retry-After cannot block a host until restart
}
pub struct RetryPolicy {
    pub max_wait: Duration,                       // throttle wait: 2 s on request paths, 10 s for /media, 60 s for jobs
    pub retry_5xx_once_after: Option<Duration>,   // Some(300 ms): one retry on 502/503/504
    pub honour_retry_after_on_503: bool,          // HAR: wait Retry-After (capped at 5 min) and retry, up to 3 times
    pub attempt_timeout: Duration,                // one attempt: request, redirects and whole body; 15 s (REQUEST, MEDIA), 60 s (JOB)
    pub max_body_bytes: usize,                    // a larger 2xx body is refused while streaming: 4 MiB, 1,500,000 (MEDIA), 32 MiB (JOB)
}
impl RetryPolicy { pub const REQUEST: RetryPolicy; pub const MEDIA: RetryPolicy; pub const JOB: RetryPolicy; }
pub struct Upstream { pub status: StatusCode, pub headers: HeaderMap, pub body: bytes::Bytes }
#[derive(Debug, thiserror::Error)]
pub enum UpstreamError {
    #[error("throttled")] Throttled,                // local throttle, cooldown, or upstream 429 (cooldown tripped)
    #[error("upstream status {0}")] Status(u16),    // any other non-2xx after retries (404 included)
    #[error("transport: {0}")] Transport(String),   // connect, timeout (incl. attempt_timeout), TLS, a broken body
    #[error("decode: {0}")] Decode(String),         // body is not the expected JSON/text
    #[error("redirect refused: {0}")] Redirect(String),   // a redirect off the allow-list, or more than 3; never followed
    #[error("body larger than {0} bytes")] TooLarge(usize), // the 2xx body passed max_body_bytes
}
impl UpstreamError {
    /// Throttled → 429 with `busy_msg`; Status(404) → 404 with `missing_msg` when Some; everything else → 502
    /// with `unavailable_msg`. Logs at WARN.
    pub fn to_app_error(&self, busy_msg: &'static str, missing_msg: Option<&'static str>, unavailable_msg: &'static str) -> AppError;
}
/// GET with the host throttle, cooldown and retry policy; 2xx returns the raw response.
/// Used for XML (DNB SRU and OAI-PMH) and image bytes (Open Library covers).
pub async fn get_bytes(o: &Outbound, t: &HostThrottle, url: Url, policy: &RetryPolicy) -> Result<Upstream, UpstreamError>;
pub async fn get_json(o: &Outbound, t: &HostThrottle, url: Url, policy: &RetryPolicy) -> Result<serde_json::Value, UpstreamError>;
pub async fn get_text(o: &Outbound, t: &HostThrottle, url: Url, policy: &RetryPolicy) -> Result<String, UpstreamError>;
```

- `get_json` and `get_text` are wrappers around `get_bytes`, so every package shares one implementation of the politeness rules. Coalesced callers share one retry.
- **Redirects** (the SSRF guard of docs/phase1/spec/shelf.md §6 risk 16): the client follows none itself (`redirect::Policy::none()`). `get_bytes` follows at most 3 (so up to 4 requests; an Open Library cover takes two, to `archive.org` and on to a storage node such as `ia802809.us.archive.org`) and checks each target before requesting it: no username or password; any URL on the origin of a configured `UpstreamUrls` base (which keeps wiremock bases working in tests); otherwise `https`, no explicit port, and a domain that is a configured upstream's or `archive.org`, or a subdomain of one matched with a leading dot (`archive.org.example.com` fails). IP addresses pass only as a configured origin. A refused or fourth redirect is `UpstreamError::Redirect`. The first URL is the caller's, built from `Outbound::urls`; redirect hops take no extra throttle slot.
- **Bodies** are streamed and refused with `TooLarge` as soon as they pass `max_body_bytes`; each attempt (request, redirects, body) is bounded by `attempt_timeout`, so a slow-drip upstream cannot hold a call open. Non-2xx bodies are not read.
- **Cover fetches** (`/media` and the prefetch worker) use `RetryPolicy::MEDIA`; the prefetch worker may lengthen `max_wait` with struct update syntax, but keeps the 1.5 MB cap. MED settles `Redirect`, `TooLarge` and `Status(404 | 410)` as "no cover" and keeps transport errors, timeouts and other statuses as "retry later" (docs/phase1/spec/shelf.md §6 risk 15).
- **User-Agent:** `Bookclub/1.0 (+{PUBLIC_URL}; {CONTACT_EMAIL})` (plan #41: a UA with contact details).
- **Error mapping** to HTTP status goes through `UpstreamError::to_app_error`: upstream 429 or throttled → 429; upstream 404 → the caller's "missing" string where a spec says so; everything else, including an undecodable body → 502 with the caller's string.

## 11. Configuration (environment)

| Variable | Default | Meaning |
|---|---|---|
| `DATABASE_URL` | required | Postgres URL |
| `BIND_ADDR` | `127.0.0.1:8080` | listen address (the root `Dockerfile`'s image sets `0.0.0.0:8080`); `healthcheck` reads only this (§16) |
| `PUBLIC_URL` | required unless `DEV_MODE` (then `http://localhost:5173`) | origin for email and invite links, the UA contact, the origin check and ICS; reduced to scheme://host[:port] |
| `SERVICE_NAME` | `Bookclub` | `/api/config.name`, email copy |
| `DEV_MODE` | `false` | `COOKIE_SECURE` defaults to false, `EMAIL_TRANSPORT` to `log` (and allows it); the Vite origin is allowed for CORS and the origin check |
| `COOKIE_SECURE` | `!DEV_MODE` | the `Secure` cookie attribute |
| `SIGNUP_MODE` | `invite` | `invite` (closed beta) or `open` (#55) |
| `TRUSTED_PROXIES` | required unless `DEV_MODE` (then empty) | comma-separated CIDRs whose `X-Forwarded-For` is trusted, or `none` (§10.2) |
| `CORS_ORIGINS` | empty | extra allowed origins (Phase 4 Capacitor origins) |
| `STATIC_DIR` | unset | built SPA (`frontend/dist`; the image sets `/app/static`). When unset, no SPA routes are served |
| `MEDIA_DIR` | `./data/media` | `FsMediaStore` root (the image sets `/data/media`, owned by uid 1000). `serve` and `import-legacy` refuse to start when they cannot write a probe file there (§3 step 2a) |
| `LEGAL_DIR` | unset | the operator's legal texts and `manifest.json` (LEG #53, §5.9); read once at startup. Unset: DEV_MODE serves the placeholders, production none. A path that does not exist or holds no text and no manifest serves none (with a warning). Outside DEV_MODE, `SIGNUP_MODE=open` needs all six files. The hosted setup sets `/app/legal` |
| `EMAIL_TRANSPORT` | `log` if `DEV_MODE`, else `smtp` | `log` is refused at startup without `DEV_MODE`: `LogEmail` writes every mail, reset links included, to the log (§19) |
| `SMTP_URL` | – | e.g. `smtps://user:pass@smtp.provider.eu:465`. Without `DEV_MODE` it must be encrypted: `smtps://` or `smtp://…?tls=required` (STARTTLS that must succeed). Plain `smtp://` (unencrypted in lettre) and `?tls=opportunistic` (strippable by an on-path attacker) are refused at startup unless the host is `localhost` or a loopback address (a relay on the same host), because the mail carries reset links and the URL the SMTP password (security review 2026-10-28 #13) |
| `EMAIL_FROM` | – | `Bookclub <noreply@…>` |
| `CONTACT_EMAIL` | required unless `DEV_MODE` | contact in the outbound User-Agent, and the public contact and takedown address (`/api/config.contact_email`, LEG #53) |
| `OPENLIBRARY_URL` / `OPENLIBRARY_COVERS_URL` | `https://openlibrary.org` / `https://covers.openlibrary.org` | tests point these at wiremock |
| `DNB_SRU_URL` / `DNB_OAI_URL` | `https://services.dnb.de/sru/dnb` / `https://services.dnb.de/oai/repository` | |
| `DNB_HARVEST` | `false` | enable the harvester job (enable it after the legacy import) |
| `DNB_HARVEST_INTERVAL_MINUTES` | `240` | values below 30 are rejected at startup |
| `DNB_HARVEST_SETS` | `dnb-all:reiheN,dnb:reiheA` | comma list of OAI setSpecs (checked against ListSets on 2026-10-27): the VLB announcements and the published titles of the book trade (Reihe A) |
| `DNB_HARVEST_START_DAYS` | `90` | the first run of a set (no `harvested_until` yet) starts this many days back; bulk history is #63 |
| `RUST_LOG` | `info` | |

`Config::from_lookup` (public) stays testable (the scaffold pattern). Invalid values fail startup with the variable name in the message. Details fixed in Wave 1:
- Empty values count as unset. Booleans accept `true/false/1/0/yes/no/on/off`.
- `SMTP_URL` and `EMAIL_FROM` are required when the transport is `smtp`. `STATIC_DIR` must be an existing directory.
- `TRUSTED_PROXIES` entries may also be bare IP addresses (a /32 or /128).
- `CORS_ORIGINS` entries are reduced to `scheme://host[:port]` and may use any scheme with a host (`capacitor://localhost`).
- `Config::allowed_origins()` (origin check) and `Config::cors_allowed_origins()` (CORS) apply the DEV_MODE Vite origins `http://localhost:5173` and `http://127.0.0.1:5173`.
- `deploy/hosted/env.example` (DEP #51) names every variable the server reads, plus the compose variables (`BOOKCLUB_TAG`, `BOOKCLUB_LISTEN`, `BOOKCLUB_PORT`, `BOOKCLUB_DATA_DIR`, `BOOKCLUB_ENV_FILE`, `BOOKCLUB_RESTART`) and `POSTGRES_PASSWORD`; `tests/deploy_env.rs` fails when a variable is added to `config.rs` (or read with `env::var` anywhere in `server/src`) without a line there, and checks that the filled-in file and `deploy/hosted/ci.env` are production configurations.

## 12. JSON, time and text

**JSON:**
- Field names are `snake_case`, exactly as in the legacy API unless `endpoint-map.md` renames them.
- Every field is always present; `Option` serialises as `null`. Never use `skip_serializing_if` on response DTOs.
- Ids are JSON numbers (`i64` identity values stay well below 2^53).
- Enums are lowercase snake strings (`#[serde(rename_all = "snake_case")]`), with `#[derive(sqlx::Type)] #[sqlx(type_name = "text", rename_all = "snake_case")]` for the text-plus-CHECK columns.

**Timestamps:**
- Stored as `timestamptz` and loaded as `time::OffsetDateTime`.
- Serialised with `#[serde(with = "time::serde::rfc3339")]` (or `::option`), always UTC with `Z`. This is a reasoned change that fixes the legacy offset-less strings.
- Dates are `time::Date`, `YYYY-MM-DD`. They need no attribute: the `time` crate's `serde-human-readable` feature (§18) serialises `Date` as `YYYY-MM-DD` in JSON. Timestamps still need `time::serde::rfc3339`.

**Time zones** (`tz.rs`, `jiff` with `tzdb-bundle-always`, so the server needs no tzdata. jiff's `TimeZone::get` still reads the host's tzdb first when there is one: the server image's `debian:trixie-slim` base has one, without the old names that `tzdata-legacy` holds. So writes validate against the bundled tzdb explicitly):

```rust
pub fn parse_zone(name: &str) -> Result<jiff::tz::TimeZone, InvalidZone>; // validate IANA names on write against
    // jiff's bundled tzdb (TimeZoneDatabase::bundled(), not the host's). Every name it knows passes, in its exact,
    // case-sensitive spelling only ("Europe/Berlin", not "europe/berlin"), like Python's ZoneInfo. An old name (a
    // link in tz::LINKS, e.g. "Asia/Calcutta") passes too and returns the zone it links to, whose iana_name() is
    // the current name ("Asia/Kolkata")
pub fn canonical_zone_name(name: &str) -> Result<String, InvalidZone>;     // parse_zone(name)?.iana_name(): the
    // name every writer stores (sign-up, PATCH /api/auth/me/preferences, club create/PATCH, importer --timezone),
    // so stored names are always current ones
pub fn zone_or_utc(name: &str) -> jiff::tz::TimeZone;                     // the host's tzdb, else the bundled
    // one (which knows every name parse_zone accepts); corrupt stored value → UTC
pub fn parse_meeting(value: Option<&str>, zone: &TimeZone) -> Result<Option<OffsetDateTime>, ()>;
    // None/""/whitespace → None; trailing Z; accepts date-only (midnight), YYYY-MM-DD[T| ]HH[:MM[:SS[.f]]],
    // lowercase t, ±HH:MM/±HHMM/Z; naive values use Disambiguation::Compatible (= Python fold=0); → UTC.
    // Error → caller returns 400 error::msg::MEETING_AT_FORMAT (also for closes_at and due_at). Fractions may use
    // '.' or ','. ISO week dates and the basic format (20260822T1900) are rejected. Year 0000, and any value whose
    // UTC instant falls outside the years 1..=9999, is an error too (Python's datetime range; time's RFC 3339
    // serialiser writes no other year, and axum would answer a plain-text 500 for a stored one). The function
    // carries #[allow(clippy::result_unit_err)].
pub fn local_minute(dt: OffsetDateTime, zone: &TimeZone) -> String;  // "%Y-%m-%dT%H:%M"
pub fn label(dt: OffsetDateTime, zone: &TimeZone) -> String;         // "%a %d %b %Y, %H:%M %Z", English, zero-padded day
pub fn today(zone: &TimeZone) -> time::Date;
pub fn noon_local_utc(date: time::Date, zone: &TimeZone) -> OffsetDateTime;  // callers check personal_date_in_range first
pub fn personal_date_in_range(date: time::Date) -> bool;  // 0001-01-02..=9999-12-30: noon there is in the UTC years
    // 1..=9999 in every zone. A `time::Date` body field also accepts year 0 and negative years; every personal date
    // (finished_on, Goodreads "Date Read") outside the range is 400 `Invalid value` before it is stored.
```

**Which zone applies:**

| Use | Zone |
|---|---|
| Club times (meeting, deadline, milestone due, club feed and thread labels) | `clubs.timezone` |
| Personal dates (`finished_on`, shelf "today", personal labels) | `users.timezone` of the owner |
| `created_label` on diary and home-feed items | the viewer's zone for personal items, the item's club zone for club items (work-plan appendix diary 11, club 4) |

So a club post's label is in its club's zone wherever it appears (pick thread, work thread, `/api/diary`, PATCH and reaction answers), and a club event's in the home feed too. It can differ from the response's `timezone` (the viewer's zone in the work thread, `/api/diary` and the home feed); a client that formats `created_at` itself uses the club's zone for club items (`docs/phase1/notes/posts.md` deviation 4, `notes/club.md` "Activity feeds").

Legacy tests with fixed zones (Kiritimati, Berlin, New York) become tests that set `users.timezone` or `clubs.timezone`.

**Old zone names** (`tz::LINKS`, 150 entries, `docs/phase1/notes/fup.md` decision 2): the `Link` lines of the bundled tzdb's release (2026e) as built with `backzone` for `zone.tab` (Debian's `tzdata.zi`), less the 11 links listed in `zone.tab` (a country's own name such as `Europe/Bratislava`, which browsers report) and less `UTC` (the column default). So `Asia/Calcutta`, `Europe/Kiev`, `America/Buenos_Aires` and `US/Eastern` are stored as `Asia/Kolkata`, `Europe/Kyiv`, `America/Argentina/Buenos_Aires` and `America/New_York`; `Europe/Oslo`, `Pacific/Chuuk` and `UTC` stay. jiff's own `iana_name()` returns a link's name unresolved, so the table is needed. After a jiff-tzdb update, compare it with the new release: a test pins the bundled release's names (count and hash) and fails until then. Every name in the table that ICU browsers report (`Asia/Calcutta`, `America/Coral_Harbour`, `CST6CDT`, …) must also be in the SPA's `currentZoneName` map (`frontend/src/timezone.ts`, whose test reads the table), so that the SPA sends and compares the stored name.

**Text** (`text.rs`). Python semantics are reproduced where the specs say they matter:

```rust
pub fn py_strip(s: &str) -> &str;          // also strips U+001C..U+001F like str.strip()
pub fn collapse_ws(s: &str) -> String;     // " ".join(s.split())
pub fn char_len(s: &str) -> usize;         // code points; never use str::len() for limits
pub fn truncate_chars(s: &str, n: usize) -> &str;   // never slice bytes
pub fn casefold(s: &str) -> String;        // caseless::default_case_fold_str ("Straße" → "strasse")
pub fn excerpt(s: &str, max: usize) -> String;
pub fn round_half_even(x: f64, digits: usize) -> f64;  // via format!("{:.*}") — matches Python round()
```

**Sorting:** sort usernames in SQL with the `COLLATE "C"` columns, or in Rust (byte order). Python `str.lower()` titles use `to_lowercase()` in Rust.

## 13. Database and sqlx

- **Queries** use the compile-time macros `sqlx::query!`, `query_as!` and `query_scalar!`. Dynamic SQL is allowed only for search with optional filters, through `sqlx::QueryBuilder` with bound parameters, and needs tests. Never `format!` SQL.
- **Type overrides** go in the macros (`status AS "status: ShelfStatus"`, `x AS "x!"` for non-null aggregates), and are kept minimal.
- **Data-access functions** take `conn: &mut PgConnection`. Handlers open a transaction (`state.db.begin()`) for any multi-statement write and pass `&mut *tx`. Background jobs and the importer do the same.
- **Locks**, used exactly where the specs found races:
  - `SELECT … FROM users WHERE id = $1 FOR UPDATE`: shelf add/patch/delete/import, favourites, physical layout, and the 10-club creation count
  - `SELECT … FROM clubs WHERE id = $1 FOR UPDATE`: **every membership write** (§5.3): join and invite accept (with the 20-member count), member removal, role change, leave, transfer, club delete, invite create and revoke, and account deletion (every club the deletion writes in, before the user row; §5.6)
  - `SELECT … FROM clubs WHERE id = $1 FOR NO KEY UPDATE`: **every pick, milestone and vote write** (`picks::lock_club_writes`, `picks::lock_as_admin`, `votes::lock_as_member`): `PUT`/`PATCH`/`DELETE` pick, milestone create, update and delete, and the vote writes nominate, cast, deadline (`PATCH vote`), apply and auto-close, each with the caller's membership or role read again under the lock. It serialises the pick and vote writes of one club and conflicts with the membership writes' `FOR UPDATE`, but not with the `FOR KEY SHARE` of foreign-key checks, so club posts and events are not blocked. It comes **before** the vote row: a first nomination has no vote row to lock yet, and a club delete holds the club row and then reaches the vote rows through its cascade (`docs/phase1/notes/club.md` "Locks and concurrency", "Votes")
  - `SELECT … FROM next_up_votes WHERE id = $1 FOR UPDATE`: nominate, cast, deadline (`PATCH vote`), apply, auto-close (after the club row)
- **Lock order of a club write:**
  - **Pick and vote writes** (CLUB; `docs/phase1/notes/club.md` "Locks and concurrency", "Work merges"): the work the write stores (`FOR SHARE`, CAT's rule below; `PUT pick` and nominate only), then the club row (`FOR NO KEY UPDATE`), then the user rows it references (`FOR KEY SHARE`, in id order: the caller, and on auto-close every nominator of the open vote), then the vote row (`FOR UPDATE`), then the current pick's row (its `UPDATE`).
  - **Club posts, replies and reactions** (POSTS; `notes/posts.md` "Lock order", "Work lock"): the club row (`FOR KEY SHARE`, with the membership read again under it), then the work (`FOR SHARE`), then the milestone (`FOR KEY SHARE`) and the parent or the post; the user rows only through foreign keys.
  - The two orders never deadlock. They do not conflict on the club and work rows (`FOR SHARE` against `FOR SHARE` on the work, `FOR NO KEY UPDATE` against `FOR KEY SHARE` on the club) and meet only on a milestone row: a club post or reply that names a milestone holds it `FOR KEY SHARE`, and a milestone delete (a pick-side write, after the club row) deletes it, so whichever starts second waits for the other (a post after a delete is then refused with `That milestone is not on this pick`; a delete after a post detaches it through `posts_milestone_fk`, a no-key update that the `FOR KEY SHARE` of replies and reactions on that post does not block). Neither holds anything else the other needs (`tests/posts_races.rs`; `notes/posts.md` "For other packages"). Both take the work before users, votes and posts, as `catalog::merge_works` does (the cover media rows, the two works `FOR NO KEY UPDATE`, the affected users `FOR UPDATE`, the affected votes, then the rows it re-points), and the club row before the user rows, as the membership writes and account deletion do (§5.3, §5.6). So a club write waits for a merge, a club delete or a removal instead of deadlocking with it.
- **Writes that store a `work_id`** lock the work against a concurrent merge (CAT's rule, `notes/cat.md` "For SHELF, POSTS and CLUB"): `SELECT id FROM works WHERE id = $1 AND merged_into_id IS NULL FOR SHARE`; no row means the work was merged away meanwhile, so the write follows `merged_into_id` (`books::require_work`) and locks again. The lock comes before the write's user and vote locks (works → users → votes, the merge's order). CAT's own edition adds and the DNB harvest lock the work after ingest's advisory and edition locks instead (`FOR NO KEY UPDATE` in the harvest, `notes/har.md` decision 11). A write that skips the rule documents the race: `pick_cleared` and `milestone_added` events may keep a merged-away `work_id` (`notes/club.md` "Accepted race").
- **Concurrency outcomes** come from atomic statements:
  - invite consume: `UPDATE club_invites SET use_count = use_count + 1 WHERE code = $1 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now()) AND (max_uses IS NULL OR use_count < max_uses) RETURNING …` (link invites have `max_uses` NULL), after the club-row lock and the inviter's row lock (`FOR KEY SHARE`), in that order (§5.5 "Lock order of a join")
  - first cover upload: `UPDATE … SET cover_media_id = $1 WHERE id = $2 AND cover_media_id IS NULL RETURNING` (through `ingest::set_cover`)
  - reaction toggle: `DELETE … RETURNING`, then `INSERT … ON CONFLICT DO NOTHING`
  - ballot: `INSERT … ON CONFLICT … DO UPDATE … RETURNING (xmax = 0) AS inserted`
  - search history: upsert, then prune, in one transaction
- **A unique violation** that can happen under a race is mapped explicitly (`error::is_unique_violation`) to the documented 409 or 400. It never surfaces as a 500.
- **Offline data:**
  - `server/.sqlx/` is committed.
  - The Dockerfile builds with `SQLX_OFFLINE=true`.
  - CI runs `cargo sqlx migrate run`, then `cargo sqlx prepare --check -- --all-targets` against its Postgres service (added in Wave 1).
- **One Postgres server per worktree.** `#[sqlx::test]` names each test database after the test's module path only (`_sqlx_test_<hash of the path>`, sqlx-core 0.9 `testing/mod.rs`) and drops and recreates it server-wide. Every worktree inherits the same W1 tests, so two worktrees on one server drop each other's databases mid-test. A separate database name per worktree does **not** help; a separate server does.
- **How an agent prepares queries:**
  1. Start the worktree's own server on its port from the work plan (`<port>` below; the integrator on `phase-1` uses the compose server on 5432):
     `docker run -d --name bookclub-pg-<pkg> -p 127.0.0.1:<port>:5432 -e POSTGRES_USER=bookclub -e POSTGRES_PASSWORD=bookclub -e POSTGRES_DB=bookclub postgres:17 -c max_connections=200 -c shared_buffers=64MB`
  2. `export DATABASE_URL=postgres://bookclub:bookclub@127.0.0.1:<port>/bookclub` and `cargo sqlx migrate run`.
  3. Write the queries, then `cargo test`.
  4. `cargo sqlx prepare -- --all-targets`, and commit `.sqlx/`.
  5. Remove the container when the package is merged (`docker rm -f bookclub-pg-<pkg>`).

  `.sqlx` holds one file per query hash, so parallel branches add files without conflicts. The integrator runs `cargo sqlx prepare` once more after merging, to drop stale files.
- **Migrations:**
  - `0001_init.sql` is the whole Phase 1 schema, so Wave 2 packages normally need no migration.
  - If one does, it uses its **reserved version block**, never the next free number: CAT 1100–1199, HAR 1200–1299, MED 1300–1399, SHELF 1400–1499, POSTS 1500–1599, CLUB 1600–1699, IMP 1700–1799. It also notes the migration in its PR.
  - The migrations after `0001_init.sql` so far:
    - `1200_editions_dnb_record_idn.sql` (HAR's block, added by the integrator's follow-up package INT): `editions.dnb_record_idn`, the IDN of the newest DNB record that delivered an edition, on every edition of a multi-ISBN record, with its index; `withdraw_dnb_edition` withdraws by it (§7.2, §7.5). Its backfill gives rows harvested before it the record IDN (the other ISBNs of a record by HAR's sibling heuristic, the same work and DNB planned date) and withdraws the editions HAR's removed new-releases clause hid (`docs/phase1/notes/int.md` decision 3).
    - `1300_media_removed_hashes.sql` (MED): `media_removed_hashes`, the hashes a taken-down picture must never come back under.
    - `1900_club_memberships_admin_until.sql` (the integrator's block, 1900–1999, security review 2026-10-28 #9): `club_memberships.admin_until`, when a member last stopped being an admin, so their removal revokes the invites they could list (§5.3).
    - Phase 2 blocks (`docs/phase2/work-plan.md` §2.1 rule 4): ADM and ADM2 2100–2199, LEG and LEG2 2200–2299, the integrator 2900–2999.
    - `2100_admin_notify_state.sql` (ADM): `reports.notified_at` and `catalog_edit_requests.notified_at`, the state of the admin mails (§16).
    - `2101_reports_targets.sql` (ADM2): `reports.target_kind` takes `cover`, `avatar`, `work` and `edition` (the constraint keeps its name `reports_target_kind_check`). New columns: `target_user_id` (`REFERENCES users ON DELETE SET NULL`: whose profile picture was reported, so the report keeps the owner when the picture row goes; the 29th foreign key to `users`, §5.7), `target_work_id` (`REFERENCES works ON DELETE SET NULL`: the work as reported, which a merge leaves in place while it re-points `work_id` to the survivor), `via` (`app` | `notice`, default `app`) and `closed_on_arrival` (only with `status = 'removed'`). Three checks keep the targets apart: `media_id` only on picture reports, `target_user_id` only on avatar reports, `target_work_id` only on work reports. The targets per kind: `cover` `media_id` with the book that showed it (`work_id`, `edition_id`); `avatar` `media_id` and `target_user_id`; `work` `target_work_id` and `work_id`; `edition` `edition_id` and its `work_id`. Older rows are backfilled by the old rules (closed on arrival by the three-column test, `via = 'notice'` for `created_by IS NULL AND status = 'removed'`). Indexes: `reports_target_user_id_idx`, and `reports_open_work_idx` and `reports_open_edition_idx` for the open reports a decision closes together.
    - `2102_users_suspended_at.sql` (ADM2): `users.suspended_at` (no foreign key) and a partial index for the list of suspended accounts (§5.3).
    - `2200_users_terms.sql` (LEG): `users.terms_version` and `terms_accepted_at` (§5.9).
    - `2201_users_last_seen.sql` (LEG2): `users.last_seen_at` and the trigger `sessions_keep_last_use` (§5.1).
    - `2900_users_unsuspended_at.sql` (the integrator, after ADM2 and LEG2): `users.unsuspended_at`, when an admin last lifted a suspension, which the purge of empty sign-ups counts as a sign-in (§5.3, §16).
  - sqlx applies a lower version that is missing even after a higher one was applied (1200 after 1300), so a database migrated before 1200 existed only needs `cargo sqlx migrate run` again.
  - There is no squash (work plan default D7): a migration merged into `phase-2` is never edited.

## 14. OpenAPI (utoipa 6 / utoipa-axum 0.3)

- **DTOs:** every request and response type derives `utoipa::ToSchema`; query structs derive `IntoParams`. Schema names are the TypeScript type names the frontend will import (`BookOut`, `ShelfItemOut`, …), so keep them stable and **unique crate-wide**: utoipa silently keeps only one of two same-named schemas. The shared DTOs live only in `crate::dto` (§20); a package never defines its own copy.
- **Operation ids:** every `#[utoipa::path]` sets `operation_id = "<module>_<action>"` (e.g. `shelf_list_entries`, `catalog_get_work`); utoipa would otherwise use the bare handler name, and two packages' `list` handlers would collide only after both merge.
- **Handlers** carry `#[utoipa::path(method, path = "/api/…", operation_id = "…", tag = "<module>", params(…), request_body = …, responses((status = 200, body = X), (status = 4xx, body = ErrorBody, description = "…")))]`. Document every status the handler can return. `ErrorBody { detail: String }` is shared from `error.rs`.
- **Security schemes** are registered in `openapi::ApiDoc`: cookie `bookclub_session` (its description says the cookie is `__Host-bookclub_session` when `Secure`, §5.1; the scheme name stays) and `bearer`.
- **Every §0 DTO is registered** in `openapi::ApiDoc` (`components(schemas(BookOut, EditionOut, ContributorOut, SearchHit, SearchPage, UserOut, UserSummary, ClubRef, ClubSummary, ShelfStatus, Visibility, CatalogSource, ReactionOut, ErrorBody))`), so the document contains them from Wave 1 on, whether or not a merged handler uses them yet.
- **W1 checks** (`tests/openapi_smoke.rs`): operation ids unique; every operation has a tag and responses; every operation id matches `^[a-z]+_[a-z0-9_]+$`; and a source scan of `server/src/**/*.rs` finds every `derive(… ToSchema …)` struct or enum name at most once.
- **`Patch<T>` fields** use `#[schema(value_type = Option<T>)]`. Timestamps use the utoipa `time` feature.
- **Serving:** `GET /api/openapi.json` serves the built document. `bookclub-server openapi` prints it without a database; #47's script turns it into `frontend/src/api/generated.ts` (openapi-typescript), and CI fails when the file is stale.

## 15. Activity events (W1 `activity::record`; CLUB owns the feeds)

```rust
pub enum EventKind { MemberJoined, PickSet, PickCleared, NotePosted, Reaction, Nominated, Voted, VoteClosed,
    MilestoneAdded, ShelfAdded, ShelfMoved, ShelfFinished, ShelfDnf, ShelfRemoved, Progress, QuoteAdded }
impl EventKind { pub const ALL: [EventKind; 16]; pub fn is_personal(self) -> bool; }
pub struct NewEvent { pub club_id: Option<i64>, pub actor_id: Option<i64>, pub kind: EventKind,
    pub work_id: Option<i64>, pub pick_id: Option<i64>, pub payload: serde_json::Map<String, Value> }
/// Drops null payload values (top level); panics unless club_id.is_none() == kind.is_personal() (a programming
/// error; the events_scope CHECK enforces the same); same transaction as the caller. Also `EventKind::as_str`.
pub async fn record(conn: &mut PgConnection, ev: NewEvent) -> sqlx::Result<i64>;
```

- The **kinds and payload keys** are the legacy ones (docs/phase1/spec/club.md §2.11), plus `quote_id` on `quote_added` and `post_id` = `posts.id` on `note_posted`/`reaction`, with one change: **`reaction` stores `author_id`** (the post author's user id) instead of the legacy `author` username. A username in a stored payload would survive the account's deletion and would then name whoever registers the freed username next. The feed resolves `author_id` to `"author": <username>|null` at read time (§6.4), so the API payload keeps the legacy key. The importer converts legacy `author` names to ids.
- **Emitters are unchanged.** The legacy kinds that have no emitter yet: diary entries record nothing, the Goodreads import records nothing, and invite joins record `member_joined` (club-scoped).

## 16. Background jobs and CLI

- `jobs::spawn_all(state, shutdown) -> tokio::task::JoinSet<()>` starts (signatures in §3); `serve` cancels `shutdown` on SIGTERM/Ctrl+C and waits up to 10 s for the set:
  - `auth::cleanup`, then `auth::delete_rejected_cover_photos` (`jobs::auth_cleanup_loop`, hourly, the first tick at startup, one `run_locked` run under `AUTH_CLEANUP`). The retention periods are Q14 A (LEG2, `docs/legal/privacy-facts.md` §7); none is a setting:
    - `cleanup` deletes expired sessions and used or expired email tokens; `reports` with `status <> 'open'` 3 years after `resolved_at`, and `catalog_edit_requests` with `status <> 'open'` 3 years after `decided_at` (open ones stay however old they are; the period runs from the decision, not the filing), except a rejected `cover_replace` whose photo is still stored; then up to `auth::PURGE_BATCH` (500) empty sign-ups (`auth::purge_empty_sign_ups`). It logs counts only: `deleted expired sessions and email tokens, old decisions and empty sign-ups` with `sessions`, `tokens`, `reports`, `edit_requests`, `sign_ups`.
    - **An empty sign-up** (`auth::empty_sign_ups`, re-checked per account by `auth::is_empty_sign_up`): an address it never verified (`email IS NOT NULL AND email_verified_at IS NULL`; an imported account without an address is never purged), not an instance admin, not suspended (`suspended_at IS NULL`: a suspension is a moderation record, and purging would free the username), no sign-in for 30 days (`greatest(created_at, password_changed_at, terms_accepted_at, last_seen_at, unsuspended_at)`: lifting a suspension counts, as the account could not sign in before; no session used and no email token created within them), and no data (no avatar, and a `NOT EXISTS` for every foreign key to `users` except the traces of signing up and in, §5.7). Each account goes in a transaction of its own: the user row, then its sessions, both `FOR UPDATE`, then `is_empty_sign_up` again in a new statement (a write that gave it data or used a session meanwhile has committed and is seen, or waits and fails), then `auth::deletion::delete_user` (§5.6), so a sign-up invite keeps its use with `user_id` NULL. 500 a run; a spam wave goes over several hours.
    - `delete_rejected_cover_photos(conn, store)` deletes the photos of rejected `cover_replace` requests decided more than 3 years ago and not taken down, lowest media id first, at most `PURGE_BATCH`, each in a transaction of its own: the media row `FOR UPDATE` (media rows first, §7.2), a second look in a new statement (the photo stays, with its request, while a work or an edition has it as its cover, an account as its picture, a report or a removed hash names it, or a request that is not due references it), `DELETE FROM media` (the request goes with it through `ON DELETE CASCADE`), then after the commit the files (`media::delete_prefixes`, a failure is logged). It logs `deleted the photos of rejected cover requests` with `photos`.
  - `admin::notify::run` (ADM #54; `jobs::admin_notify_loop`, every 5 minutes, `admin::notify::RUN_EVERY`, the first tick at startup): per queue, when the queue's last mail (its newest `notified_at`, migration 2100) is at least the interval old (1 hour for reports, 24 hours for catalog requests) and open items without `notified_at` exist, it mails every user with `is_admin` and a verified address who is not suspended (ADM2), in `users.locale` (§9), through `state.email` directly (15 s each, not `auth::mail::deliver`'s per-address limits), then stamps `notified_at = now()` on the items it counted, only when at least one mail went out. No setting; with `EMAIL_TRANSPORT=log` the mails go to the log.
  - `harvest::run_forever` (only when `DNB_HARVEST`)
  - `shelf::goodreads::resume` (once at startup: re-queues `queued`/`running` jobs)
  - the media prefetch worker (`MediaService`, §8)
- Each periodic job:
  - takes `pg_try_advisory_lock(<job id>)` for the run, which is safe if a second instance ever runs (`jobs::run_locked(db, lock_id, name, job)` does it, logs and never panics: a panicking job is caught, logged at ERROR and counts as failed, and its connection is closed rather than returned to the pool, so the session-level lock dies with it; the keys are `jobs::lock_ids`: `AUTH_CLEANUP = 0x6263_0001`, `DNB_HARVEST = 0x6263_0002`, `ADMIN_NOTIFY = 0x6263_0003`)
  - logs start, end and duration
  - never panics (errors are logged)
- Jobs are **not** started in tests. Tests call the job function directly.
- **Goodreads jobs** run as `tokio::spawn` tasks started by the POST handler. The handler commits the job row first, so a crash is resumed at startup.
- **The CLI** (clap, `main.rs`, W1) has `serve` (default), `import-legacy` (IMP fills `importer::run`), `rematch-legacy` (IMP fills `importer::rematch`: re-resolves legacy bare-facts editions by ISBN, legacy-mapping.md §3.3), `make-admin`, `beta-invite` and `openapi`. `import-legacy` and `rematch-legacy` connect and migrate like `serve`, build the production `AppState`, and print `importer::Report` with its `Display` impl (IMP keeps that impl). DEP (#51) added two that never touch the database:
  - `healthcheck` (`healthcheck.rs`, the image's `HEALTHCHECK`): reads only `BIND_ADDR` (`0.0.0.0` becomes `127.0.0.1`, `[::]` becomes `[::1]`), never `Config::from_env`, and GETs `/api/health` with reqwest, 3 s for the whole request and no proxy. Exit 0 and `GET <url> answered 200 OK` on stdout only on 200; otherwise exit 1 with `error: GET <url> answered …` (or the connection error) on stderr.
  - `send-test-mail <address>`: the full configuration as for `serve`; the address must pass `auth::rules::valid_email`. It sends `email::test_mail` (§9) through the configured transport (`LogEmail` in DEV_MODE) and exits 1 with `error: SMTP send failed: …` when the server refuses.
- **The legacy importer** (`importer/source.rs`, CUT #52) never opens the legacy file itself: it resolves a symlink, copies the file and its `-wal` and `-journal` into a private directory `$TMPDIR/bookclub-legacy-<16 hex>` (0700, files 0600), opens that copy with `SQLITE_OPEN_READ_ONLY` and removes it afterwards, so a WAL database on a read-only mount imports with its committed `-wal` rows (never `immutable=1`). A hot journal is refused with its reason. The hosted container's `/tmp` is a 64 MB tmpfs; `TMPDIR` moves the copy (`deploy/hosted/README.md`, "The legacy import").

## 17. Testing

### 17.1 Integration tests

Integration tests live in `server/tests/*.rs`. Every DB test is `#[sqlx::test]`: it gets a fresh database with all migrations applied, and runs in parallel with the others.

```rust
mod common;
use common::*;

#[sqlx::test]
async fn add_move_remove_and_unique(db: PgPool) {   // parity: test_shelf.py::test_add_move_remove_and_unique
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let circe = app.work("Circe").await;
    let res = ada.post("/api/shelf", json!({"work_id": circe.id})).await;
    res.assert_status(201);
    assert_eq!(res.json()["status"], "want_to_read");
    let dup = ada.post("/api/shelf", json!({"work_id": circe.id})).await;
    dup.assert_detail(409, "Already on your shelf");
}
```

### 17.2 Harness (`tests/common/`, W1, frozen; packages put extra helpers in their own test files)

```rust
pub struct TestApp { pub db: PgPool, pub state: AppState, pub upstream: wiremock::MockServer,
                     pub email: Arc<CapturingEmail>, pub catalog: Arc<FakeCatalog>, pub media: Arc<MemMediaStore> }
impl TestApp {
    pub async fn new(db: PgPool) -> Self;            // DEV_MODE-like config, cheap argon2, MemMediaStore,
                                                     // CapturingEmail, FakeCatalog as `lookup`, every
                                                     // upstream URL pointed at `upstream`
    pub async fn with_env(db: PgPool, env: &[(&str, &str)]) -> Self; // new() plus env vars (later win), e.g.
                                                     // ("SIGNUP_MODE", "open") or &PRODUCTION_ENV
    pub async fn with_real_lookup(db: PgPool) -> Self; // lookup = state.catalog (real chain against `upstream`)
    pub fn config(&self) -> &Config;
    pub fn cookie_name(&self) -> &'static str;      // session::cookie_name(config): __Host-bookclub_session under PRODUCTION_ENV
    pub fn anon(&self) -> TestClient;
    pub async fn user(&self, username: &str) -> TestClient;          // verified email <username>@example.test,
                                                                     // password PASSWORD ("password1"), signed in
                                                                     // (session row created directly: no limits)
    pub async fn user_unverified(&self, username: &str) -> TestClient;
    pub async fn admin(&self, username: &str) -> TestClient;         // is_admin
    pub async fn club(&self, owner: &TestClient, name: &str) -> TestClub;            // via POST /api/clubs: the owner must
                                                                     // be verified and below the 10-club limit
    pub async fn join(&self, club: &TestClub, who: &TestClient, role: &str);       // direct insert ("admin"|"member")
    pub async fn work(&self, title: &str) -> TestWork;               // ingest::upsert_work (source user, no authors)
    pub async fn ol_work(&self, ol_id: &str, title: &str, authors: &str) -> TestWork;
    pub async fn dnb_work(&self, title: &str, authors: &str, isbn: &str) -> (TestWork, TestEdition); // via upsert_edition_with_work
    pub async fn edition(&self, work: &TestWork, isbn: &str) -> TestEdition;
    pub fn sent_emails(&self) -> Vec<Email>;
    pub fn last_link_token(&self, to: &str) -> String;               // reads the #token= fragment
    pub async fn upstream_requests(&self) -> Vec<wiremock::Request>; // assert "no outbound call"
    pub async fn settle(&self);                                      // waits (≤ 10 s) until no background task
                                                                     // holds the state: the reset or resend mail, a Goodreads job
    pub async fn exec(&self, sql: &str);                              // raw SQL (move time, corrupt rows); panics on error
    pub async fn scalar_i64(&self, sql: &str) -> i64;                 // e.g. "SELECT count(*) FROM users"
}
pub struct TestClub { pub id: i64, pub slug: String, pub name: String }
pub struct TestWork { pub id: i64, pub title: String }
pub struct TestEdition { pub id: i64, pub work_id: i64, pub isbn13: String }

pub struct TestClient { /* Router clone, cookie jar for the session cookie (TestApp::cookie_name()), optional bearer, client ip */ }
impl TestClient {
    pub async fn get(&self, path: &str) -> TestResponse;
    pub async fn post(&self, path: &str, body: Value) -> TestResponse;
    pub async fn put(&self, path: &str, body: Value) -> TestResponse;
    pub async fn patch(&self, path: &str, body: Value) -> TestResponse;
    pub async fn delete(&self, path: &str) -> TestResponse;
    pub async fn delete_json(&self, path: &str, body: Value) -> TestResponse;   // DELETE /api/auth/me {password}
    /// Any method, any content type, raw body: malformed JSON, wrong content types, HEAD, 405 checks.
    pub async fn request(&self, method: Method, path: &str, content_type: Option<&str>, body: Vec<u8>) -> TestResponse;
    pub async fn post_multipart(&self, path: &str, parts: Vec<Part>) -> TestResponse;
    pub async fn put_multipart(&self, path: &str, parts: Vec<Part>) -> TestResponse;
    pub fn with_ip(self, ip: IpAddr) -> Self; pub fn with_header(self, k: &str, v: &str) -> Self;
    pub fn with_bearer(self, token: &str) -> Self;
    pub fn id(&self) -> i64; pub fn username(&self) -> &str;           // panic for an anonymous client
    pub fn for_router(router: Router) -> TestClient;                     // anonymous, any router (extractor tests)
    pub fn cookie(&self) -> Option<String>; pub fn set_cookie(&self, value: Option<&str>);   // the jar
}
// Clones share the cookie jar. A Set-Cookie for the session cookie (bookclub_session, or __Host-bookclub_session
// under a Secure configuration such as PRODUCTION_ENV) stores its value; an empty value or Max-Age=0
// empties the jar. Without with_ip the server sees 0.0.0.0.
pub enum Part { File { name: String, filename: String, content_type: String, bytes: Vec<u8> }, Text { name: String, value: String } }
impl Part { pub fn file(filename: &str, content_type: &str, bytes: Vec<u8>) -> Part; /* name "file" */ pub fn text(name: &str, value: &str) -> Part; }
pub struct TestResponse { pub status: StatusCode, pub headers: HeaderMap, pub body: Bytes }
impl TestResponse { pub fn json(&self) -> Value; pub fn detail(&self) -> String;
    pub fn text(&self) -> String; pub fn header(&self, name: &str) -> Option<&str>; pub fn set_cookies(&self) -> Vec<String>;
    pub fn assert_status(&self, s: u16) -> &Self; pub fn assert_detail(&self, s: u16, d: &str) -> &Self; }
pub struct CapturingEmail { /* Mutex<Vec<Email>> */ }   // impl EmailSender
impl CapturingEmail { pub fn sent(&self) -> Vec<Email>; pub fn sent_to(&self, to: &str) -> Vec<Email>; }  // case-insensitive

pub const PASSWORD: &str = "password1";
pub const PRODUCTION_ENV: [(&str, &str); 6];   // DEV_MODE=false plus PUBLIC_URL https://books.example.com,
                                               // TRUSTED_PROXIES none, CONTACT_EMAIL, SMTP_URL, EMAIL_FROM
// Low-level helpers for tests that need no database or build their own state:
pub fn test_config(extra: &[(&str, &str)]) -> Config;      // DEV_MODE plus extra
pub fn test_state(db: PgPool) -> AppState;                  // AppState::new with test_config, cheap argon2
pub fn test_state_with(db: PgPool, config: Config) -> AppState;
pub fn unreachable_db() -> PgPool;                          // never connects
pub async fn send(app: Router, request: Request<Body>) -> TestResponse;
pub async fn get(app: Router, path: &str) -> TestResponse;

/// A programmable CatalogLookup. Unprogrammed ISBNs and texts are Ok(None).
pub struct FakeCatalog { /* Mutex<…>; creates works and editions through ingest */ }
pub enum FakeHit {
    /// Resolve to an existing work (and optionally edition).
    Existing { work_id: i64, edition_id: Option<i64> },
    /// Create (once) a work through ingest and resolve to it; with an ISBN query, an edition with that ISBN too.
    New { title: String, authors: String, ol_work_id: Option<String>, source: Source },
}
impl FakeCatalog {
    pub fn on_isbn(&self, isbn13: &str, hit: FakeHit);           // resolve_isbn and resolve_isbn_remote
    pub fn on_text(&self, title: &str, authors: &str, hit: FakeHit);
    pub fn fail_all(&self, err: FakeError);                      // FakeError::{Throttled, Unavailable}
    pub fn fail_isbn(&self, isbn13: &str, err: FakeError, times: Option<usize>);   // None = always; Some(n) = the next n calls
    pub fn calls(&self) -> Vec<String>;                          // "isbn:978…", "isbn_remote:978…", "text:<title>|<authors>"
}
// resolve_isbn runs lookup::local_isbn first, like CAT; failures and programmed hits stand for the remote step.
// resolve_isbn_remote and resolve_text have no local step. A FakeHit::New with an ISBN is created with
// upsert_edition_with_work(WorkGrouping::NewWork, Mode::Refresh), without one with upsert_work.
```

`TestApp::settle` counts the references to the state's shared parts (every request and every background task holds a clone of `AppState`) and returns once the count is back to its value when the app was built. A test that keeps its own clone of `app.state` must drop it before it calls `settle`.

**`tests/common/visibility.rs`** is the template for `tests/<pkg>_visibility.rs` (§6.5). `Viewers::new(&app)` returns

```rust
pub struct Viewers {
    pub owner: TestClient,       // user `owner`: the data belongs to them
    pub co_member: TestClient,   // user `comember`: in `shared` with the owner, and in `other` with `otherclub`
    pub other_club: TestClient,  // user `otherclub`: owns `other`, a club the owner is not in
    pub stranger: TestClient,    // user `stranger`: in no club
    pub former: TestClient,      // user `former`: was in `shared` and left
    pub shared: TestClub,        // the owner's club ("Shared")
    pub other: TestClub,         // otherclub's club ("Other")
}
```

**`common::legal_dir(terms_version) -> tempfile::TempDir`** (LEG #53, added at the end of `tests/common/mod.rs`): a `LEGAL_DIR` with all six texts (`<p>{doc} {locale}</p>`) and a manifest with that terms version; pass it as `("LEGAL_DIR", dir.path().to_str().unwrap())`. Phase 2 packages only add helpers to `tests/common/**` and record any change to an existing one in their notes.

**`tests/common/routes.rs`** lists the documented operations for the route-wide guards (§17.7): `operations() -> Vec<Operation>` (from `bookclub_server::openapi()`), `find(&Method, path)`, and `outcome(&TestResponse) -> (u16, Value)` (the body as JSON, `null` when it is not). `Operation { method, path, params: Vec<(name, is_integer)>, has_body, tag }` has `label()` (`"GET /api/clubs"`), `fill(|name, is_integer| value)`, `fill_dummy(slug)` (§17.7's dummy values) and `send(&client, path, &body)` (the body as JSON when the operation takes one).

`audience()` (`[&co_member]`) and `strangers()` (`[&other_club, &stranger, &former]`) list the viewers; `assert_hidden_like_unknown(viewer, |name| path, "owner")` asserts that a stranger's answer is a 404 identical to the one for the unknown name `visibility::UNKNOWN`. Paths use the usernames (`?username=owner`), code uses the fields (`v.co_member`). `tests/harness.rs` tests the harness itself.

### 17.3 Faking outbound HTTP

- `TestApp::new` starts one `wiremock::MockServer` and points every upstream URL at it, keeping the real paths: `OPENLIBRARY_URL` and `OPENLIBRARY_COVERS_URL` are the server's root (API paths like `/works/…` and cover paths like `/b/id/…` do not collide), `DNB_SRU_URL` is `{uri}/sru/dnb` and `DNB_OAI_URL` is `{uri}/oai/repository`.
- Its `Outbound` throttles allow 1000 requests a second, so tests do not wait for the production spacing. A test of the throttle itself builds its own `HostThrottle`.
- Tests mount mocks for Open Library, cover, DNB SRU and DNB OAI paths, and assert on the received requests: query parameters, User-Agent and call counts.
- An unmatched request answers 404 and is recorded. `upstream_requests()` then shows that it was sent.
- The local-reads parity tests assert `upstream_requests().is_empty()`.
- **Time-dependent behaviour** (vote deadlines, session expiry, token expiry) is tested by updating rows with SQL, exactly as the Python tests did. There is no clock injection.

### 17.4 Parity naming

Every port of a Python test carries a `/// parity: <file>::<test>` doc comment. #49 generates its checklist by grepping these comments.

### 17.5 Unit tests

Unit tests stay next to the code (`#[cfg(test)]`). They cover pure functions: ISBN, text helpers, tz parsing and labels, ICS folding, description and subject cleaning, MARC parsing, dominant colour, the layout validator, the Goodreads CSV parser.

### 17.6 `tests/api_error_copy.rs` (W1)

- Reads `../frontend/src/i18n/apiErrors.ts` with the regex `"([^"]+)":\s*"errors\.`.
- Requires at least 10 keys.
- Asserts that each key appears inside a string literal in `server/src/**/*.rs`.
- Has a temporary `LEGACY_ONLY` allow-list (`No profile picture`, `That book is not in the club yet`). #48 empties it.
- Passes from Wave 1 on in every worktree, because every other mapped string is an `error::msg` constant defined by W1 (§4.1), including the ones SHELF and MED return.

### 17.7 Route-wide guards (W1, generic, cover Wave 2 routes once they merge)

They walk the OpenAPI document (`bookclub_server::openapi()`), fill path parameters with dummy values (`{slug}` of a real club, numeric ids `1`, other strings `x`) and send a body `{}` where one is expected:
- `tests/auth_required.rs`: an anonymous caller gets **401** on every operation except the public allow-list: `GET /api/health`, `GET /api/config`, `GET /api/openapi.json`, `POST /api/auth/register`, `POST /api/auth/login`, `POST /api/auth/logout`, `POST /api/auth/tokens`, `POST /api/auth/verify-email`, `POST /api/auth/verify-email/preview`, `POST /api/auth/password-reset`, `POST /api/auth/password-reset/confirm`, `GET /api/invites/{code}`, `GET /media/{public_id}/{variant}`, `GET /api/legal/{doc}` (LEG #53).
- `tests/admin_guard.rs`: a signed-in non-admin gets **404 `Not found`** on every `/api/admin/*` operation. Since ADM (#54) it asserts the exact list of admin operations (`ADMIN_OPERATIONS`, 15 since ADM2's three user routes), so a new one fails until it is added there with real ids and a valid body, and the walk uses real seeded ids so an unguarded handler would act (ADM2: per-operation path values in `Seeded.op_ids`, an account to suspend and a suspended one to unsuspend, with `users` and `sessions` watched).
- `tests/clubs_non_member.rs`: a signed-in non-member gets **404 `No such club`** on every `/api/clubs/{slug}/…` operation. It also seeds, in club A, a pick, a club post and a reply, and as a member of club B and as a clubless user calls every `/api/posts/{post_id}…` operation on them (404 `No entry with that id`), `POST /api/books/works/{work_id}/posts` with `parent_id` = the club post (400 `That entry is not on this book`) and `GET /api/diary?club=<A>` (404 `No such club`), and checks that `GET /api/books/works/{work_id}/posts`, `GET /api/diary` and `GET /api/activity` contain none of club A's post or event ids. Until POSTS and CLUB merge, those routes are absent from the document and the walker skips them.

Details of the W1 implementation:
- Each walker asserts that the W1 operations it guards are in the walk, so it can never pass vacuously. `auth_required.rs` also asserts that every allow-listed operation that exists answers an anonymous caller with something other than 401, and that a dead session cookie gets the same 401 as no cookie. `admin_guard.rs` walks as a club owner and as an unverified user and checks that nothing was written; a positive control shows an instance admin passing.
- `clubs_non_member.rs` walks as a member of another club and as a clubless user, with club A's real slug and with an unknown slug, filling `{username}` with a real member of A, `{invite_id}` with a real invite of A and every other parameter with the dummies. Every request carries a body that would act if a guard were missing (`{"name", "timezone", "role", "username": <caller>, "kind": "link"}`; unknown keys are ignored), and a snapshot of club A must be unchanged afterwards.
- For the `/api/posts/{post_id}…` operations the body is `{"body": "…", "emoji": "❤️"}`, so a handler that validates before it checks access (§4.2) still reaches the visibility check. The feed checks have positive controls: a member of club A sees A's post in the work thread and in `GET /api/diary` (`items[*].entry.id`) and A's events in `GET /api/activity` (`items[*].id`); the work thread's ids are `items[*].id` and `items[*].replies[*].id`. The seeded rows (a pick, a club post, a reply by another member, a `pick_set` and a `note_posted` event) are also checked against `bc_can_see_post` directly, so the test means something before POSTS and CLUB merge.

### 17.8 Required checks before merge (every package)

```
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo sqlx prepare --check -- --all-targets
```

## 18. Dependencies (added once, in Wave 1)

They are compatible with the existing axum 0.8.9, sqlx 0.9.0, tokio 1.54 and Rust 1.99. The versions were checked against crates.io on 2026-10-27.

```toml
[dependencies]
anyhow = "1.0.104"
argon2 = "0.6.0"                                   # default: alloc, getrandom, password-hash
async-trait = "0.1.92"
axum = { version = "0.8.9", features = ["multipart", "macros"] }
axum-extra = { version = "0.12.6", features = ["cookie", "typed-header"] }
base64 = "0.23.1"
bytes = "1"
caseless = "0.2.2"                                 # Unicode case folding (= Python casefold)
clap = { version = "4.6.7", features = ["derive", "env"] }
csv = "1.4.0"                                      # Goodreads import (flexible rows)
futures = "0.3.34"
hyper = "1.12.0"                                   # serve.rs names hyper's request body (Wave 1 security review)
hyper-util = { version = "0.1.21", features = ["server-auto", "server-graceful", "service", "tokio"] }  # serve.rs: header read timeout
image = { version = "0.25.10", default-features = false, features = ["jpeg", "png", "gif", "webp"] }
ipnet = "2.12.2"
jiff = { version = "0.2.38", features = ["tzdb-bundle-always"] }
lettre = { version = "0.11.23", default-features = false, features = ["builder", "hostname", "smtp-transport", "pool", "tokio1", "tokio1-rustls", "ring", "webpki-roots"] }
moka = { version = "0.12.16", features = ["future", "sync"] }
quick-xml = "0.42.0"                               # OAI-PMH + MARC21-xml, DNB SRU
rand = "0.10.3"
regex = "1.13.1"
reqwest = { version = "0.13.5", default-features = false, features = ["rustls", "json", "query", "http2", "charset", "stream"] }
rusqlite = { version = "0.39.0", features = ["bundled"] }   # legacy importer only (read-only); see the note below
serde = { version = "1.0.229", features = ["derive"] }
serde_json = "1.0.154"
serde_path_to_error = "0.1.20"
sha2 = "0.11.1"
sqlx = { version = "0.9.0", default-features = false, features = ["runtime-tokio", "tls-rustls", "postgres", "macros", "migrate", "uuid", "time", "json"] }
thiserror = "2.0.21"
time = { version = "0.3.55", features = ["serde", "serde-human-readable", "formatting", "parsing", "macros"] }
tokio = { version = "1.54.1", features = ["full"] }
tokio-util = "0.7.20"                              # CancellationToken for graceful job shutdown
tower = { version = "0.5.3", features = ["util"] }
tower-http = { version = "0.7.1", features = ["fs", "set-header", "trace", "request-id", "cors", "catch-panic", "limit", "util"] }
tracing = "0.1.44"
tracing-subscriber = { version = "0.3.23", features = ["env-filter"] }
unicode-normalization = "0.1.26"                   # NFC for catalog text (DNB MARC is decomposed); already in the tree via caseless
url = "2.5.8"
utoipa = { version = "6.0.0", features = ["macros", "axum_extras", "time", "preserve_order"] }
utoipa-axum = "0.3.0"
webp = "0.3.1"                                     # lossy WebP encoding (bundled libwebp)
zip = { version = "9.1.2", default-features = false, features = ["deflate"] }

[dev-dependencies]
http-body-util = "0.1.5"
tempfile = "3.27.0"
wiremock = "0.6.5"
```

**Notes:**
- `sqlx` gains `json` for `jsonb`.
- `time` gains `serde-human-readable` (Wave 1): without it `time::Date` serialises as a `(year, ordinal)` tuple.
- `tower` moves from dev-dependencies to dependencies.
- `hyper` and `hyper-util` (Wave 1 security review): the server loop of §3, because `axum::serve` gives hyper no timer and so no header read timeout. Both were already locked as dependencies of axum, with these features; they add no crate.
- `unicode-normalization` (Wave 1 step 2): DNB delivers decomposed umlauts; ingest and the MARC mapping store NFC. It was already locked as a dependency of `caseless`, so it adds no crate.
- **Not used:**
  - `utoipa-scalar`: no docs UI is needed, and its optional `time < 0.3.52` pin would clash.
  - `governor`/`tower_governor`: the sliding-window counter is simpler and does the job.
  - `imagequant`: GPL.
  - `chrono`: one date library (`time`) plus `jiff` for zones.
- `webp` and `rusqlite` compile C code; the `rust:slim` builder image has `cc`.
- **rusqlite stays on 0.39** (libsqlite3-sys 0.37). sqlx 0.9's optional `sqlx-sqlite` pins `libsqlite3-sys >=0.30.1, <0.38.0`, and Cargo checks `links = "sqlite3"` across optional dependencies even with the sqlx `sqlite` feature off, so rusqlite 0.40/0.41 (libsqlite3-sys 0.38/0.39) do not resolve. Bump rusqlite and sqlx together. The set above resolves to 371 packages with a single axum 0.8.9 / axum-core 0.5.6 (checked with `cargo generate-lockfile` and a compile probe on 2026-10-27).
- **Build profile** (in `Cargo.toml`, W1), to keep 8 parallel worktrees within the VM's disk and memory:

  ```toml
  [profile.dev]
  debug = "line-tables-only"
  [profile.test]
  debug = "line-tables-only"
  ```

## 19. Logging and operational rules

- Use `tracing` with fields, not string formatting: `tracing::warn!(url = %u, status, latency_ms, "open library request failed")`.
- **The log goes to stderr** (DEP #51), with ANSI colours only when stderr is a terminal. stdout carries only a command's result (`beta-invite`'s code and link, `openapi`'s JSON, the importer's report), so `docker compose exec app bookclub-server beta-invite | head -1` is the code and Docker's log files hold plain text.
- **Never log:** passwords, tokens, cookies, email tokens, invite codes, the email body (except in `LogEmail` in dev) or full email addresses at INFO. The access log's `path` (the `TraceLayer` span in `lib.rs`) replaces the code in `/api/invites/{code}…` and `/join/{code}` with `{code}`.
- `AppError::internal` logs the cause at ERROR with the request id.
- **Outbound calls** log URL, status and latency: INFO on success, WARN on failure.
- **The health check** stays as scaffolded.

## 20. Shared DTOs (`crate::dto`, W1, frozen)

Every type of endpoint-map §0 has exactly one Rust definition, here. Packages import them; they never define a type with the same schema name (§14). Timestamps serialise as RFC 3339 UTC (`time::serde::rfc3339`), dates as `YYYY-MM-DD`.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, sqlx::Type, Serialize, Deserialize, ToSchema)]
#[sqlx(type_name = "text", rename_all = "snake_case")] #[serde(rename_all = "snake_case")]
pub enum ShelfStatus { WantToRead, CurrentlyReading, Finished, DidNotFinish }

#[derive(Clone, Copy, Debug, PartialEq, Eq, sqlx::Type, Serialize, Deserialize, ToSchema)]
#[sqlx(type_name = "text", rename_all = "snake_case")] #[serde(rename_all = "snake_case")]
pub enum Visibility { Audience, OnlyMe }

#[derive(Clone, Copy, Debug, PartialEq, Eq, sqlx::Type, Serialize, Deserialize, ToSchema)]
#[sqlx(type_name = "text", rename_all = "snake_case")] #[serde(rename_all = "snake_case")]
pub enum CatalogSource { Openlibrary, Dnb, User, Legacy }      // serialises "openlibrary", …

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct BookOut {
    pub id: i64, pub ol_work_key: Option<String>, pub title: String, pub authors: String, pub year: Option<i32>,
    pub cover_url: Option<String>, pub cover_thumb_url: Option<String>, pub cover_public_id: Option<String>,
    pub pages: Option<i32>, pub spine_color: Option<String>, pub source: CatalogSource, pub custom: bool,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ContributorOut { pub name: String, pub role: String /* "translator"|"illustrator"|"editor" */ }

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct EditionOut {
    pub id: i64, pub work_id: i64, pub isbn13: Option<String>, pub isbn10: Option<String> /* derived */,
    pub title: Option<String>, pub subtitle: Option<String>, pub publisher: String, pub language: Option<String>,
    pub format: Option<String>, pub pages: Option<i32>, pub publication_year: Option<i32>,
    pub planned_publication_date: Option<time::Date>, pub contributors: Vec<ContributorOut>,
    pub cover_url: Option<String>, pub cover_thumb_url: Option<String>, pub cover_public_id: Option<String>,
    pub source: CatalogSource,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct SearchHit {
    pub id: i64, pub ol_work_key: Option<String>, pub title: String, pub authors: String, pub year: Option<i32>,
    pub cover_url: Option<String>, pub cover_thumb_url: Option<String>, pub cover_public_id: Option<String>,
    pub isbn: Option<String>, pub source: CatalogSource, pub custom: bool, pub on_shelf: bool,
    pub shelf_id: Option<i64>, pub club_picks: Vec<ClubRef>,
    pub planned_publication_date: Option<time::Date>,   // new-releases hits (HAR); null elsewhere
}
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct SearchPage { pub items: Vec<SearchHit>, pub page: i32, pub has_more: bool }

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct UserOut {
    pub id: i64, pub username: String, pub email: Option<String>, pub email_verified: bool,
    pub pending_email: Option<String>,   // an email change waiting for confirmation: the newest open verification link's address, only while the account's own address is verified; else null
    pub needs_email_prompt: bool,
    pub needs_terms: bool,               // LEG (#53): the operator's terms are configured and not accepted in their current version (§5.3)
    pub theme: String, pub color_mode: String, pub locale: String,
    pub timezone: String, pub avatar_url: Option<String>, pub is_admin: bool, pub clubs: Vec<ClubSummary>,
}
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct UserSummary { pub id: i64, pub username: String, pub avatar_url: Option<String> }
/// Row form for loaders (not a schema): visible_user, user_summaries.
pub struct UserSummaryRow { pub id: i64, pub username: String, pub avatar_public_id: Option<String> }
impl From<UserSummaryRow> for UserSummary { /* avatar_url = media::shared::urls::avatar_url */ }

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ClubRef { pub slug: String, pub name: String }
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ClubSummary { pub slug: String, pub name: String, pub role: String /* owner|admin|member */,
    pub timezone: String, pub member_count: i64 }

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ReactionOut { pub emoji: String, pub count: i64, pub mine: bool, pub users: Vec<String> }

// Loaders that are not catalog-specific (catalog loaders are in catalog::shared::books, §7.1).
// Checks nothing: it names any existing user id. Callers pass only ids the viewer may see (§6.3: reaction users,
// parent authors and reader lists come from bc_visible_owners or the club's members, never from the row alone).
pub async fn user_summaries(conn: &mut PgConnection, ids: &[i64]) -> sqlx::Result<HashMap<i64, UserSummary>>;
```

`ErrorBody { detail: String }` stays in `error.rs`. `MemberYear` (one user's year in review) is SHELF's: it backs both `GET /api/stats` and `GET /api/clubs/{slug}/stats`, which SHELF owns (work-plan).
