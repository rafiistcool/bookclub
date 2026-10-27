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
    importer/…                    # IMP
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

Modules with instance-admin routes (`admin::invites`, `catalog`, `media`) expose them separately, never inside `routes()`:

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
}
fn admin_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(admin::invites::admin_routes()).merge(catalog::admin_routes()).merge(media::admin_routes())
}
pub fn openapi() -> utoipa::openapi::OpenApi { api_router().merge(admin_router()).split_for_parts().1 }
pub fn app(state: AppState) -> axum::Router {
    let (api, _) = api_router().split_for_parts();
    let (admin, _) = admin_router().split_for_parts();
    // One guard for every admin route: a handler that forgets its InstanceAdmin argument is still guarded.
    // Applied only when `admin.has_routes()`: axum panics on a route layer over an empty router.
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
1. tracing
2. `Config::from_env`
3. connect the pool, retrying every 2 s for up to 60 s, then exit non-zero
4. `sqlx::migrate!("./migrations").run(&pool)`
5. build `AppState`
6. `jobs::spawn_all(state.clone(), shutdown.clone())`
7. bind
8. `axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())` with graceful shutdown, which also cancels the jobs

**Layers, outermost first:**
1. tower-http `SetRequestId` / `PropagateRequestId` (`x-request-id`)
2. `SetResponseHeader` `Referrer-Policy: same-origin` on every response (no URL of ours leaks to a third party). It sits outside the layers below so their own answers (the panic 500, a CORS preflight, the origin check's 403) carry it too.
3. `TraceLayer`
4. `CatchPanicLayer`, answering 500 `{"detail":"Internal Server Error"}`
5. CORS: only when `CORS_ORIGINS` is set (Phase 4 apps), or for the Vite origin in DEV_MODE
6. the origin check (§5.8)
7. `DefaultBodyLimit::max(1 MiB)`; upload routes override it per route (§4.6)

There are no global timeouts; handlers bound their outbound calls instead (§10.3).

**Fallbacks:**
- An unknown `/api/…` path (and `/api` itself) or `/media/…` path gets 404 `{"detail":"Not found"}`, whatever the method.
- A wrong method gets `method_not_allowed_fallback` → 405 `{"detail":"Method Not Allowed"}`.
- Every other GET or HEAD gets the SPA shell when `STATIC_DIR` is set (`spa.rs`: `ServeDir` for files; `index.html` with `Cache-Control: no-store`; `/assets/*` with `public, max-age=31536000, immutable`; a missing asset is a JSON 404, never the shell). Other methods on those paths get the JSON 405; without `STATIC_DIR` they are all JSON 404s.

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
2. The `Content-Type` must be `application/json` (parameters allowed). Otherwise 400 `Input should be a valid dictionary or object to extract fields from`. Unparsable JSON gives 400 `JSON decode error`. A non-object gives 400 `Input should be a valid dictionary or object to extract fields from`.
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
  - The cookie is `bookclub_session=<value>; Path=/; HttpOnly; SameSite=Lax; Max-Age=34560000`, plus `Secure` unless `COOKIE_SECURE=false` (the DEV_MODE default).
- **Idle expiry slides.** On every authenticated request, when `last_used_at < now - 1 hour`, the extractor sets `last_used_at = now` and `expires_at = now + 30 days` (cookie) or `+ 90 days` (bearer). The cookie's Max-Age is fixed at 400 days, the browser maximum; the server decides validity.
- **Logout** deletes the row and sends a clearing cookie, but only when a valid session existed (parity).
- **A cookie that does not resolve** (unknown or expired session, deleted user) gets 401 plus a clearing `Set-Cookie`.
- **Revocation:**
  - A password change deletes every other session and token of the user.
  - A password reset deletes all of them.
  - Account deletion cascades.
- An hourly job (§16) deletes expired sessions and used or expired email tokens.

### 5.2 Bearer tokens

- `POST /api/auth/tokens` returns `bc_<base64url(32 bytes)>`, stored in the same table with `kind='bearer'`.
- The extractor reads `Authorization: Bearer bc_…` **before** the cookie. A bearer request never needs the origin check (§5.8).
- Phase 4 device tokens (APNs/FCM) will reference `sessions.id` from a new table.

### 5.3 Extractors

All of them live in `auth::extract` and `clubs::extract` (W1, frozen).

```rust
pub struct CurrentUser {
    pub id: i64, pub username: String, pub is_admin: bool, pub email_verified: bool,
    pub locale: String, pub timezone: String, pub session_id: i64,
}
pub struct MaybeUser(pub Option<CurrentUser>);
pub struct VerifiedUser(pub CurrentUser);   // 403 "Verify your email address first"
pub struct InstanceAdmin(pub CurrentUser);  // 404 "Not found" for everyone else

pub enum Role { Member, Admin, Owner }       // ordered: Member < Admin < Owner
pub struct ClubCtx { pub id: i64, pub slug: String, pub name: String, pub timezone: String }
pub struct ClubMember { pub user: CurrentUser, pub club: ClubCtx, pub role: Role }
pub struct ClubAdmin(pub ClubMember);        // role >= Admin, else 403 ONLY_ADMINS
pub struct ClubOwner(pub ClubMember);        // role == Owner, else 403 ONLY_OWNER
```

- One query joins `sessions` and `users` for `CurrentUser`.
- The club extractors read the `slug` path parameter through `axum::extract::RawPathParams` and call `member_by_slug`. They return 404 `No such club` for a missing club **and** for a non-member. A path with any parameter that is not valid UTF-8 after percent-decoding (`/api/clubs/%ff/…`, `/api/clubs/{slug}/picks/%ff`) is also 404 `No such club`: axum then reads none of the parameters, and such a path names nothing the caller can see.

**Club helpers** (`clubs::extract`, W1, frozen; the extractors are built on them, so there is one implementation):

```rust
/// The caller's membership of the club with this slug. 404 NO_SUCH_CLUB for an unknown slug or a non-member.
/// For `?club=<slug>` query parameters (GET /api/diary?club=), which RawPathParams cannot read.
pub async fn member_by_slug(conn: &mut PgConnection, user: &CurrentUser, slug: &str) -> ApiResult<ClubMember>;
/// The user's role in a club, None for a non-member. For club rows reached by a global id
/// (/api/posts/{post_id} on a club post, a club reply created through the diary route).
pub async fn role_in(conn: &mut PgConnection, user_id: i64, club_id: i64) -> sqlx::Result<Option<Role>>;
/// The ClubRef list of the viewer's clubs (slug, name), for DTO annotation.
pub async fn club_refs(conn: &mut PgConnection, club_ids: &[i64]) -> sqlx::Result<HashMap<i64, ClubRef>>;
```

`auth::user_out(conn: &mut PgConnection, user_id: i64) -> ApiResult<UserOut>` (W1, frozen) builds the full `UserOut` including `clubs`; MED's avatar endpoints return it.

- **Club data rule:**
  - On `/api/clubs/{slug}/…` routes, every query on club data takes `club_id` **only** from the `ClubMember` extractor.
  - On routes that reach a club row by a global id (`/api/posts/{post_id}`, a reply whose `parent_id` is a club post), `club_id` comes **from the row**, and the caller must pass `role_in(conn, me, club_id)`; `None` is that route's 404 (`No entry with that id` for posts). Moderation checks use the role **in that row's club**, never "admin anywhere".
  - On routes with a `?club=<slug>` parameter, the club comes from `member_by_slug`.
  - List queries that mix clubs (the home feeds, `club_picks` on search and detail) restrict club rows with `JOIN bc_member_clubs($viewer) AS mc ON mc.club_id = x.club_id`.
  - Every child id from the path or the body is resolved with `WHERE club_id = $1 AND id = $2`. An id from another club is then indistinguishable from a missing one.
  - Never accept a `club_id` from a request.
- **Membership writes** (remove a member, change a role, leave, transfer, join, accept an invite) never trust the role the extractor read before the transaction. Each one:
  1. opens a transaction and locks the club row (`SELECT … FROM clubs WHERE id = $1 FOR UPDATE`);
  2. re-reads the caller's role **and** the target's role inside the transaction (a caller who lost the needed role meanwhile gets the 403 or 404 that applies now);
  3. puts the guard into the write itself, and treats 0 affected rows as the documented 403/404:
     - remove by an admin: `DELETE FROM club_memberships WHERE club_id = $1 AND user_id = $2 AND role = 'member'`; by the owner: `… AND role <> 'owner'`;
     - role change: `UPDATE … SET role = $3 WHERE club_id = $1 AND user_id = $2 AND role <> 'owner'`;
     - leave: `DELETE … WHERE club_id = $1 AND user_id = $me AND role <> 'owner'` (0 rows and the caller is the owner → 409 `Transfer ownership before leaving`);
     - transfer: demote the caller (`… AND role = 'owner'`, 0 rows → 403 ONLY_OWNER), then promote the target (`… AND user_id = $2`, 0 rows → 404 NO_MEMBER).
  
  The deferred constraint triggers `clubs_have_owner`/`club_memberships_keep_owner` are the backstop: a transaction that would leave an existing club without an owner fails at COMMIT. Tests that create clubs with raw SQL must insert the owner row in the same transaction.

### 5.4 Passwords

- **Hashing.** The `argon2` crate (0.6) uses `Argon2id`. Production parameters are the crate default (`m = 19456 KiB, t = 2, p = 1`, OWASP).
- **Verification** parses the PHC string with `argon2::password_hash::phc::PasswordHash::new` (argon2 0.6; the old `password_hash::PasswordHash` path is a deprecated alias and fails clippy `-D warnings`) and verifies with the parameters stored in it, so legacy argon2-cffi hashes (`m=65536,t=3,p=4`) verify unchanged. An unparsable hash counts as a mismatch, never a 500. Hashing is `Argon2::default().hash_password(pw.as_bytes())`: argon2 0.6 generates the salt itself.
- **Rehash on login.** After a successful login, a hash whose parameters differ from the current ones is re-hashed and stored, inside the login transaction.
- **Blocking work** (hash and verify) runs in `tokio::task::spawn_blocking`, behind a `tokio::sync::Semaphore` of 4 permits held by `Passwords`. That bounds memory: legacy hashes use 64 MiB each.
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
- **Mail is sent after the transaction commits.** A send failure is logged and does not fail the request. Resend exists for verification, and reset can be requested again.

**Addresses and uniqueness.** Only **verified** addresses are unique (`users_verified_email_key`), and only verified addresses are used for login by email and for reset. An unverified address is just what someone typed; it cannot squat anyone's address.
- `users.email` is the account address. On an account with no verified address, a sign-up or `PUT /api/auth/me/email` writes the new address there at once (unverified). On an account with a verified address, the new address lives only on the open `verify_email` token until it is verified; `UserOut.pending_email` shows it.
- **No enumeration.** Register and `PUT /api/auth/me/email` never reveal whether an address is in use: they answer exactly as for a free address. When the address already belongs to a **verified** account, no verification link is sent; that address gets the "someone tried to use your address" notice instead (§9). The new account keeps an unverified address it can never verify.

**Token invalidation** (each in the same transaction as the event):

| Event | Effect on open tokens of the user |
|---|---|
| A new `verify_email` token is issued (register, `PUT /me/email`, resend) | Every older open `verify_email` token is marked used: at most one is open. |
| `PUT /me/email` | Also every open `reset_password` token is marked used. |
| Verification succeeds | Every other open `verify_email` and every open `reset_password` token is marked used. |
| Password change, password reset | Every open `reset_password` token is marked used (plus the session revocation of §5.1). |

- **Verify** accepts a token only when it is open (unused, unexpired), is the user's newest `verify_email` token, and its `email` is the address still pending (`users.email` for an unverified account, else the token address itself). Success sets `users.email = token.email` and `email_verified_at`. If another account verified the same address first, it answers 409 `That email is already in use` (the unique index decides; mapped with `is_unique_violation`). Only the holder of a link can see that, so it does not enumerate.
- **Reset confirm** requires `token.email = users.email AND users.email_verified_at IS NOT NULL`; anything else is 400 `That link has expired or was already used`.
- **Rate limits** (§10.1): `PUT /me/email` and resend share the per-account `email_send_user` bucket (3 per hour, 429 `Slow down a little. Try again later.`). Every mail to an address (verification link, notice, reset) also counts against `email_target` (6 per day per case-folded address across all accounts); over it, the mail is silently not sent and the response is unchanged.
- **Sign-up gate:** `SIGNUP_MODE=invite` is the Phase 1/2 default (a closed beta until #55). Sign-up then requires a valid invite: a club invite (it also joins the club) or a sign-up invite minted by an instance admin (club_id NULL).
- **Before verification,** a user can log in and use everything personal, join clubs by invite, and post. Creating a club requires a verified email (`VerifiedUser`). That limits abuse once sign-up is open.
- **Imported users** have `email` NULL. `UserOut.needs_email_prompt = email IS NULL AND email_prompt_dismissed_at IS NULL`.

### 5.6 Account deletion (`DELETE /api/auth/me`)

Everything below runs in one transaction, after the password is verified:

1. **Clubs the user owns.** If any of them has other members: 409 `Transfer or delete your clubs first`. Clubs where the user is the sole member are deleted. (Each owned club row is locked first, so a concurrent join cannot slip in.)
2. **Posts:**
   - The user's posts that have replies by others become tombstones (body `''`, spoiler NULL, `deleted_at` now).
   - All other posts of the user are deleted.
   - Their reactions cascade.
   - `author_id` then becomes NULL through the foreign key. Live posts can't exist at this point; the `posts_live_has_author` check enforces it.
   - A diary tombstone with `author_id` NULL stays visible to whoever can see the author of one of its replies, so the other users' replies stay visible to the same people as before (`bc_visible_posts`).
3. **Media:** delete **every** avatar media row the user uploaded (`media WHERE kind = 'avatar' AND uploaded_by = $user`, not only the current one) and remember their storage prefixes. Cover photos the user uploaded are catalog contributions: they stay, with `uploaded_by` set to NULL by the FK.
4. **Delete the user.** This cascades:
   - sessions and email tokens
   - shelf, layout, favourites, quotes, search history, import jobs
   - memberships, removals, ballots, reactions
   - every event whose actor is the user (`ON DELETE CASCADE`)

   These references are set to NULL instead: picks (`set_by`), nominations (`nominated_by`), milestones (`created_by`), invites (`created_by`), invite uses, `invited_by`, clubs (`created_by`), works and editions (`created_by`), edit requests and reports.

   Other users' `reaction` events keep only `author_id` (a user id, never a username, §15). Identity ids are never reused, so after the deletion the feed shows the author as null; no later account can inherit the name.
5. **After commit:** delete the remembered media bytes, and clear the cookie.

**The decision #39 asks for:** club posts are **deleted**; they are only kept as an anonymous tombstone when others replied to them. Activity events are deleted. Structural club records (picks, nominations) stay, without the name.

**Replacing an avatar** (`PUT /api/auth/me/avatar`, MED) deletes the previous avatar row in the same transaction and its bytes after commit, so normally a user has at most one avatar row; step 3 also catches rows left behind by a crash.

### 5.7 DSGVO export (`GET /api/auth/me/export`)

The response is a ZIP built in memory: personal data is small, and 50 MB is the upper bound. It contains `data.json` and `media/`.

**`data.json`** holds one key per foreign-key column that references `users(id)`:

| Key | Source (`table.column`) |
|---|---|
| `profile` | `users` row minus `password_hash`, plus `pending_email` |
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
| `edit_requests` (filed / decided) | `catalog_edit_requests.created_by`, `catalog_edit_requests.decided_by` |
| `reports` (filed / resolved) | `reports.created_by`, `reports.resolved_by` |
| `uploaded_media` (avatars and cover photos) | `media.uploaded_by` |

**`media/`** holds the avatar file and the cover photos the user uploaded.

**Exempt, with the reason in the test's allow-list:** `email_tokens.user_id` (one-time secrets that expire within 48 h; the only personal datum on them, a pending address, is exported as `profile.pending_email`).

**Completeness test.** It reads `pg_constraint` for every foreign-key column that references `users(id)` and asserts that each `table.column` is either a source in the table above (the test holds the same list) or in the allow-list. A new column that points at users fails it until it is exported or exempted with a reason. (Matching on column names would also hit `work_authors.author_id`, which references `authors`.)

### 5.8 Cross-site requests (CSRF)

The cookie is `SameSite=Lax`, so browsers don't attach it to cross-site POST, PUT, PATCH or DELETE. JSON endpoints also require `Content-Type: application/json`, which a plain HTML form cannot send.

On top of that, the **origin check layer** answers 403 `{"detail":"Cross-site request refused"}` to any unsafe-method request that:
- may authenticate by cookie: it has no bearer token and carries a `Cookie` header, whatever cookies it holds, and
- carries an `Origin` header that is not `PUBLIC_URL`'s origin, not in `CORS_ORIGINS`, and not the Vite origin in DEV_MODE.

Any cookie counts, not only a literal `bookclub_session`: the check fails closed instead of depending on how the session extractor parses cookie names (axum-extra's `CookieJar` percent-decodes them, so `bookclub%5Fsession` is the session cookie to it). Bearer-token requests skip the check. **The frontend sends nothing extra.** It is `auth::origin::check` (W1, implemented in step 1); the cookie name is `auth::SESSION_COOKIE`.

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
- **Tombstone of a deleted account** (`author_id` NULL, §5.6): the root is visible to every viewer who can see the author of at least one of its replies, and each reply keeps its own rule (visible when the viewer can see the reply's author). The other users' replies therefore stay visible to the same people as before the deletion.

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
- **Names at read time:** `reaction` payloads store `author_id`; the feed replaces it with `"author": <username>`, or `null` when the account is gone, so the API keeps the legacy payload shape.

### 6.5 The #46 test matrix

`tests/visibility_policy.rs` (W1) covers the SQL functions and `Relation` for:
- viewer types: self, co-member, member of another club, stranger, and "former co-member" (left the club)
- data: `audience` and `only_me` items, and a diary tombstone of a deleted account with replies by a co-member and by a stranger
- the scalar functions with a NULL owner (they return false, never NULL)

Every Wave 2 package adds `tests/<pkg>_visibility.rs`, following the template in `tests/common/visibility.rs`. It covers each personal data kind it serves (shelf, layout incl. the PUT→404 AC, favourites, stats, diary, quotes, ratings in book detail/pick readers/activity) across viewer types (self, co-member, stranger, unknown name) and item visibility. #49 checks that the matrix is complete.

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

Every catalog field of `works`, `editions`, `authors`, `work_authors` and `edition_contributors` is written by `ingest`. Three narrow exceptions write through dedicated functions with the same rules: covers (`set_cover`/`clear_media`, used by MED and CAT), admin edits (`apply_admin_fields`, CAT's approve) and merges (`catalog::merge_works`, CAT). The importer may also set `works.details_fetched_at` directly (the legacy value).

**Provenance.** `works.field_sources` and `editions.field_sources` record, per field that holds a value, which source wrote it. Ranks: `admin` (an approved edit request) > `user` (entered by a reader) > `dnb` = `openlibrary` > `legacy` (bare facts from the legacy import).
- An empty field (NULL, `''`, `{}`) may always be filled; the new provenance is the writer's.
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
    pub dnb_idn: Option<String>, pub title: Option<String>, pub subtitle: Option<String>, pub publisher: Option<String>,
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
    /// Attach to this existing work.
    Into(i64),
    /// DNB: an existing non-merged `source='dnb'` work with the same match_key, else a new work.
    DnbTitleAuthor,
}

/// A work identified by ol_work_id is found and updated per `mode`. With ol_work_id None, a new work is
/// ALWAYS created, for any source (no deduplication; the caller groups). Source::OpenLibrary requires
/// ol_work_id (debug_assert; Err(Protocol) in release). Sets details_fetched_at when full_details.
pub async fn upsert_work(conn: &mut PgConnection, rec: &WorkRecord, mode: Mode) -> sqlx::Result<i64>;
/// One batched INSERT … ON CONFLICT DO NOTHING for search pages; returns ids in input order.
pub async fn upsert_work_stubs(conn: &mut PgConnection, recs: &[WorkRecord]) -> sqlx::Result<Vec<i64>>;
/// Finds the edition by isbn13, then dnb_idn, then ol_edition_id.
/// - Found: updates it per `mode` and the provenance rules (dnb_idn takes the newest record's IDN, moving it from
///   another edition; a NULL isbn13/ol_edition_id is filled when free, an existing one is kept; a withdrawn
///   edition is un-withdrawn). Then the parent work's title, subtitle, author_names and work_authors are updated
///   from `work` under the same rules when they are `legacy`-ranked, or when they are `dnb`-ranked and this is the
///   work's only edition (a provisional VLB title gets the final one). Precisely: the three fields merge normally
///   (empty ones filled) when the work's source is `legacy` (bare facts being upgraded) or when the record is DNB,
///   the title's provenance is `dnb` and the work has one edition; otherwise only `legacy`-ranked values are
///   replaced. So an OL record never retitles a DNB work and a DNB record never retitles an OL work.
/// - Not found: resolves the work by `grouping` (creating it from `work` when needed) and inserts the edition.
/// In both cases NULL work fields are filled from the edition (§7.3). Returns (work_id, edition_id).
/// InsertOnly returns a found edition untouched. A `legacy` edition becomes `dnb`/`openlibrary` like a work.
pub async fn upsert_edition_with_work(conn: &mut PgConnection, work: &WorkRecord, ed: &EditionRecord,
    grouping: WorkGrouping, mode: Mode) -> sqlx::Result<(i64, i64)>;
/// An edition of a known work (OL editions list, a user adding an edition). Same find-or-insert as above,
/// but never touches the work's own fields except the NULL fill of §7.3. A found edition of another work keeps
/// its work: the returned work_id says so (CAT answers 409).
pub async fn upsert_edition(conn: &mut PgConnection, work_id: i64, rec: &EditionRecord, mode: Mode)
    -> sqlx::Result<(i64, i64)>;
/// DNB delivered status="deleted" for this IDN: sets editions.withdrawn_at when the edition exists. Never inserts.
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
/// language, format, pages, publication_year, planned_publication_date ("YYYY-MM-DD"). There is no `isbn` key
/// (identity, not an editable field). Null clears a field (not a work title) and drops its provenance, so a
/// source may fill it again. Changing `authors` drops the work_authors links. 404 NO_SUCH_BOOK for a missing
/// target; 400 `Unknown field: <key>` / `Invalid value for <key>` (admin-only strings, not mapped by the app).
pub async fn apply_admin_fields(conn: &mut PgConnection, target: CoverTarget, fields: &serde_json::Map<String, Value>)
    -> ApiResult<()>;
// Pure rules, public for CAT and tests:
pub fn clean_description(raw: &str) -> String;  pub fn clean_subjects(values: &[String]) -> Vec<String>;
pub fn name_key(name: &str) -> String;  pub fn author_surname(name: &str, sort_name: Option<&str>) -> String;
pub fn match_key(title: &str, surname: &str) -> String;  pub fn search_extra(work_title: &str, edition_titles: &[String]) -> String;
```

**How ingest writes** (W1 implementation notes that callers rely on):
- Each write loads the row `FOR UPDATE`, merges the record field by field, and writes it once. Records are sanitised first: values outside the schema's CHECKs are dropped (pages, years, ratings, cover ids <= 0, malformed OL ids, overlong identifiers) or truncated by characters (titles, names, publisher, description); text is stored as Unicode NFC.
- **Concurrency:** writes of an edition take transaction-level advisory locks on its ISBN, IDN and OL edition id, and `DnbTitleAuthor` on the match key (the two-key lock space `(0x62630100, hash)`, separate from `jobs::lock_ids`). Inserts also use `ON CONFLICT DO NOTHING` and merge into the winner. Callers run a multi-statement ingest in one transaction (HAR per page, CAT per lookup), or the locks end with each statement.
- `upsert_work_stubs` writes `author_names` only (author records come with the full record) and inserts records without an `ol_work_id` one by one.

### 7.3 Rules ingest maintains

- **Covers never come back after a takedown.** `ol_cover_media` returns None for a removed row; a record whose `ol_cover_id` is removed sets no cover; `set_cover` refuses a removed row.
- **Covers from sources never replace a user's or an admin's cover.** A cover set by a first upload has provenance `user`; an approved `cover_replace` has `admin`. A Refresh only sets `cover_media_id` when it is NULL or has source rank (an Open Library cover may replace an older Open Library cover).
- **NULL work fields are filled from editions.** After any edition write, a NULL `works.pages`, `first_publish_year` (from `publication_year`, else the year of `planned_publication_date`) and `cover_media_id` are filled from that edition, with the edition's provenance. So a work built from DNB or legacy facts, or a work whose only cover is a user photo of one edition, shows year, pages and cover outside an edition context too.
- **Search sees edition titles.** `works.search_extra` is rebuilt from the work's edition titles that differ (by case fold) from the work title.
- **`works.match_key`** = `casefold(collapse_ws(title))` + U+001F + `casefold(surname of the first author)`, where the surname is the part before the comma of an inverted name (`authors.sort_name`), else the last word of the name (up to its first comma). A work without author records uses the first name in `author_names`. Rust computes it; Postgres `lower()` is not a case fold. `search_extra` joins the distinct edition titles with " / " (at most 5000 characters).
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
- **ISBNs:** the ISBN-10 and ISBN-13 of one book (DNB lists both) are one edition. Only the first edition carries `dnb_idn` (`editions.dnb_idn` is unique), so a deleted header withdraws the first ISBN's edition.
- **Imprint:** 264 with ind2 `1` first, then other 264s, then 260. An edition whose 020 `$q` names a 264 `$b` (co-publishers) gets that publisher, else the first. The year is the first four-digit number of `$c` ("September 2026", "[2026]"); 2099 and later are DNB placeholders and count as missing. Language: the first valid 041 `$a`, else 008/35-37. Pages: the number before "Seiten"/"S."/"pages"/"p." in 300 `$a`.
- **Work facts** (for `upsert_edition_with_work`): title and subtitle from 245; authors from the `aut` names; `first_publish_year` = the edition's `publication_year`, else the year of the planned date; `pages` = the edition's pages.
- **Several ISBNs in one record:** one edition per ISBN in the registration group of the first 020 ISBN (foreign co-publisher ISBNs are dropped), all under the same work.
- **Format**, per edition: with one ISBN, 653 `(Produktform)` first (`Hardback` → hardcover; `Paperback / softback` → paperback; `Digital…`/`E-book` → ebook; `Audio…` → audiobook; anything else, e.g. `Calendar`, `Multiple-item retail product` → other), then the 020 $c binding, then 337/338. With several ISBNs, each edition's own 020 $c binding first (`Gb.`, `geb.`, `Gebunden`, `Gewebe`, `Hldr.`, `Pp.`, `Hperg.`, `Festeinband`, `Leinen`, `Ln.`, `Lw.`, `Hardcover` → hardcover; `kart.`, `KT`, `Broschur`, `brosch.`, `Broschiert`, `Klappenbroschur`, `Pb.`, `Paperback`, `Taschenbuch` → paperback; matched as words, case-insensitively, before the first `:`), then a single 653 value, then 337/338 (`cr` → ebook; audio carriers such as `sd` → audiobook; 337 `s` → audiobook, `c` → ebook). Nothing usable → `other`, so a DNB edition always has a format.
- **Deleted records:** the OAI header `status="deleted"` is the caller's to detect; it calls `ingest::withdraw_dnb_edition` and never maps the metadata.
- **Deleted records still carry metadata** in DNB's OAI responses, so the caller must check the header before mapping.
- Tests (W1): `tests/fixtures/dnb_marc/` holds single records copied from DNB responses recorded on 2026-10-27 (reiheN and reiheA OAI pages, one SRU search): a single-ISBN hardback with 263, an inverted multi-author name list, translators and editors with GND ids, a translator without one, a multi-ISBN co-publisher record, a record with 856 links plus a hand-added 520 (DNB's MARC carries blurbs only as 856 links; no recorded record had a 520), and a deleted-header record. No recorded record combined kart./Gewebe/Hperg. bindings, so `dnb_marc.rs` unit tests cover the binding vocabulary, foreign co-publisher groups and the carrier fallbacks with small inline records. `tests/dnb_marc.rs` maps and ingests every fixture and checks that no VLB text or URL is stored.

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
- Reports and takedown: a takedown deletes the bytes, sets `removed_at`, and calls `ingest::clear_media`.
- Avatar endpoints (`PUT` deletes the previous avatar row in the same transaction and its bytes after commit; both return `auth::user_out`).
- Spine-colour backfill.

## 9. Email (W1)

```rust
#[derive(Clone, Debug)] pub struct Email { pub to: String, pub subject: String, pub text: String }
#[async_trait::async_trait]
pub trait EmailSender: Send + Sync { async fn send(&self, msg: Email) -> anyhow::Result<()>; }
pub struct LogEmail;        // logs to and subject, plus the full body, at INFO: the dev transport (EMAIL_TRANSPORT=log, DEV_MODE only)
pub struct SmtpEmail { … }  // lettre AsyncSmtpTransport<Tokio1Executor>, rustls, pooled; from SMTP_URL
// tests: CapturingEmail (tests/common) keeps every message
```

- **Templates** are plain text only, in English and German by `users.locale`, in `email/templates.rs`: verify, reset, email changed (to the old address), and "someone tried to use your address" (sent to a verified address instead of a verification link when a sign-up or an email change named it, §5.5; it says what was tried and links to login and password reset, and contains no link that changes anything).
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
pub struct RateLimits { pub login_failures, pub login_ip, pub signup_ip, pub reset_ip, pub reset_account,
    pub email_send_user, pub email_target, pub invite_preview_ip, pub refresh_user, pub upload_user,
    pub report_user, pub edit_request_user, pub custom_book_user, pub export_user: SlidingWindow }
/// Rate-limit key for a client address: an IPv4 address as is, an IPv6 address by its /64 prefix
/// (a client with a /64 can otherwise mint a fresh key per request).
pub fn ip_key(ip: IpAddr) -> String;
/// Key for an identifier that names no account: "n:" + the first 4 hex digits of its SHA-256
/// (65,536 buckets at most, so attacker-chosen names cannot flush real buckets).
pub fn unknown_identifier_key(normalised: &str) -> String;
```

- **Login and password change** call `login_failures.check_and_hit(key)` **before** verifying the password and `clear(key)` on success. `check` followed by `hit` after a failure is not enough: the verify awaits the password semaphore, so concurrent attempts would all pass `check` before the first `hit` lands, and thousands of guesses could reach one account. With `check_and_hit` the counting is unchanged (10 failures, then 429; a success clears), and of N concurrent attempts at most 10 are verified. A refused attempt records nothing, so the right password is refused while the key is blocked, as in Python.
- **Bounded memory:** each limit has its own cache, `max_capacity` 100,000 keys, with `time_to_idle` = the window. Account keys (`u:<id>`, at most one per account) and hashed unknown-identifier keys are bounded by construction.
- **Every test gets a fresh `RateLimits`**, because the harness builds a fresh `AppState`.

| Limit | Threshold |
|---|---|
| Login failures (per account key `u:<id>`, or `unknown_identifier_key` when the account is unknown) | 10 per 600 s, cleared on success, shared with password change |
| Login attempts per IP (`ip_key`) | 30 per 600 s |
| Sign-up per IP | 5 per hour |
| Reset per IP | 5 per hour |
| Reset per account | 3 per hour, silent (the response is still 204) |
| `email_send_user`: verification resend and `PUT /api/auth/me/email`, per account | 3 per hour |
| `email_target`: any mail to one address (case-folded), across all accounts | 6 per day, silent (the mail is not sent; the response is unchanged) |
| Invite preview per IP | 60 per hour |
| Refresh | 10 per hour per user |
| Uploads | 30 per day |
| Reports | 20 per day |
| Edit requests | 50 per day |
| Custom books | 30 per day |
| Export | 5 per hour |

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
| `BIND_ADDR` | `127.0.0.1:8080` | listen address (the Docker image sets `0.0.0.0:8080`) |
| `PUBLIC_URL` | required unless `DEV_MODE` (then `http://localhost:5173`) | origin for email and invite links, the UA contact, the origin check and ICS; reduced to scheme://host[:port] |
| `SERVICE_NAME` | `Bookclub` | `/api/config.name`, email copy |
| `DEV_MODE` | `false` | `COOKIE_SECURE` defaults to false, `EMAIL_TRANSPORT` to `log` (and allows it); the Vite origin is allowed for CORS and the origin check |
| `COOKIE_SECURE` | `!DEV_MODE` | the `Secure` cookie attribute |
| `SIGNUP_MODE` | `invite` | `invite` (closed beta) or `open` (#55) |
| `TRUSTED_PROXIES` | required unless `DEV_MODE` (then empty) | comma-separated CIDRs whose `X-Forwarded-For` is trusted, or `none` (§10.2) |
| `CORS_ORIGINS` | empty | extra allowed origins (Phase 4 Capacitor origins) |
| `STATIC_DIR` | unset | built SPA (`frontend/dist`). When unset, no SPA routes are served |
| `MEDIA_DIR` | `./data/media` | `FsMediaStore` root |
| `EMAIL_TRANSPORT` | `log` if `DEV_MODE`, else `smtp` | `log` is refused at startup without `DEV_MODE`: `LogEmail` writes every mail, reset links included, to the log (§19) |
| `SMTP_URL` | – | e.g. `smtps://user:pass@smtp.provider.eu:465` |
| `EMAIL_FROM` | – | `Bookclub <noreply@…>` |
| `CONTACT_EMAIL` | required unless `DEV_MODE` | contact in the outbound User-Agent |
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

**Time zones** (`tz.rs`, `jiff` with `tzdb-bundle-always`, so the slim Docker image needs no tzdata):

```rust
pub fn parse_zone(name: &str) -> Result<jiff::tz::TimeZone, InvalidZone>; // validate IANA names on write; only the
    // canonical, case-sensitive spelling passes ("Europe/Berlin", not "europe/berlin"), like Python's ZoneInfo
pub fn zone_or_utc(name: &str) -> jiff::tz::TimeZone;                     // corrupt stored value → UTC
pub fn parse_meeting(value: Option<&str>, zone: &TimeZone) -> Result<Option<OffsetDateTime>, ()>;
    // None/""/whitespace → None; trailing Z; accepts date-only (midnight), YYYY-MM-DD[T| ]HH[:MM[:SS[.f]]],
    // lowercase t, ±HH:MM/±HHMM/Z; naive values use Disambiguation::Compatible (= Python fold=0); → UTC.
    // Error → caller returns 400 error::msg::MEETING_AT_FORMAT (also for closes_at and due_at). Fractions may use
    // '.' or ','. ISO week dates and the basic format (20260822T1900) are rejected. The function carries
    // #[allow(clippy::result_unit_err)].
pub fn local_minute(dt: OffsetDateTime, zone: &TimeZone) -> String;  // "%Y-%m-%dT%H:%M"
pub fn label(dt: OffsetDateTime, zone: &TimeZone) -> String;         // "%a %d %b %Y, %H:%M %Z", English, zero-padded day
pub fn today(zone: &TimeZone) -> time::Date;
pub fn noon_local_utc(date: time::Date, zone: &TimeZone) -> OffsetDateTime;
```

**Which zone applies:**

| Use | Zone |
|---|---|
| Club times (meeting, deadline, milestone due, club feed and thread labels) | `clubs.timezone` |
| Personal dates (`finished_on`, shelf "today", personal labels) | `users.timezone` of the owner |
| `created_label` on diary and home-feed items | the viewer's zone |

Legacy tests with fixed zones (Kiritimati, Berlin, New York) become tests that set `users.timezone` or `clubs.timezone`.

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
  - `SELECT … FROM clubs WHERE id = $1 FOR UPDATE`: **every membership write** (§5.3): join and invite accept (with the 20-member count), member removal, role change, leave, transfer, club delete, and the owned-club check of account deletion
  - `SELECT … FROM next_up_votes WHERE id = $1 FOR UPDATE`: nominate, cast, apply, auto-close
- **Concurrency outcomes** come from atomic statements:
  - invite consume: `UPDATE club_invites SET use_count = use_count + 1 WHERE code = $1 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now()) AND (max_uses IS NULL OR use_count < max_uses) RETURNING …` (link invites have `max_uses` NULL)
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
  - Before Phase 2 (#52) the migrations may be squashed back into one file, since no production database exists yet.

## 14. OpenAPI (utoipa 6 / utoipa-axum 0.3)

- **DTOs:** every request and response type derives `utoipa::ToSchema`; query structs derive `IntoParams`. Schema names are the TypeScript type names the frontend will import (`BookOut`, `ShelfItemOut`, …), so keep them stable and **unique crate-wide**: utoipa silently keeps only one of two same-named schemas. The shared DTOs live only in `crate::dto` (§20); a package never defines its own copy.
- **Operation ids:** every `#[utoipa::path]` sets `operation_id = "<module>_<action>"` (e.g. `shelf_list_entries`, `catalog_get_work`); utoipa would otherwise use the bare handler name, and two packages' `list` handlers would collide only after both merge.
- **Handlers** carry `#[utoipa::path(method, path = "/api/…", operation_id = "…", tag = "<module>", params(…), request_body = …, responses((status = 200, body = X), (status = 4xx, body = ErrorBody, description = "…")))]`. Document every status the handler can return. `ErrorBody { detail: String }` is shared from `error.rs`.
- **Security schemes** are registered in `openapi::ApiDoc`: cookie `bookclub_session` and `bearer`.
- **Every §0 DTO is registered** in `openapi::ApiDoc` (`components(schemas(BookOut, EditionOut, ContributorOut, SearchHit, SearchPage, UserOut, UserSummary, ClubRef, ClubSummary, ShelfStatus, Visibility, CatalogSource, ReactionOut, ErrorBody))`), so the document contains them from Wave 1 on, whether or not a merged handler uses them yet.
- **W1 checks** (`tests/openapi_smoke.rs`): operation ids unique; every operation has a tag and responses; every operation id matches `^[a-z]+_[a-z0-9_]+$`; and a source scan of `server/src/**/*.rs` finds every `derive(… ToSchema …)` struct or enum name at most once.
- **`Patch<T>` fields** use `#[schema(value_type = Option<T>)]`. Timestamps use the utoipa `time` feature.
- **Serving:** `GET /api/openapi.json` serves the built document. `bookclub-server openapi` prints it without a database; #47's script turns it into `frontend/src/api/generated.ts` (openapi-typescript), and CI fails when the file is stale.

## 15. Activity events (W1 `activity::record`; CLUB owns the feeds)

```rust
pub enum EventKind { MemberJoined, PickSet, PickCleared, NotePosted, Reaction, Nominated, Voted, VoteClosed,
    MilestoneAdded, ShelfAdded, ShelfMoved, ShelfFinished, ShelfDnf, ShelfRemoved, Progress, QuoteAdded }
impl EventKind { pub fn is_personal(self) -> bool; }
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
  - `auth::cleanup` (hourly)
  - `harvest::run_forever` (only when `DNB_HARVEST`)
  - `shelf::goodreads::resume` (once at startup: re-queues `queued`/`running` jobs)
  - the media prefetch worker (`MediaService`, §8)
- Each periodic job:
  - takes `pg_try_advisory_lock(<job id>)` for the run, which is safe if a second instance ever runs (`jobs::run_locked(db, lock_id, name, job)` does it, logs and never panics: a panicking job is caught, logged at ERROR and counts as failed, and its connection is closed rather than returned to the pool, so the session-level lock dies with it; the keys are `jobs::lock_ids`, e.g. `DNB_HARVEST` for HAR)
  - logs start, end and duration
  - never panics (errors are logged)
- Jobs are **not** started in tests. Tests call the job function directly.
- **Goodreads jobs** run as `tokio::spawn` tasks started by the POST handler. The handler commits the job row first, so a crash is resumed at startup.
- **The CLI** (clap, `main.rs`, W1) has `serve` (default), `import-legacy` (IMP fills `importer::run`), `rematch-legacy` (IMP fills `importer::rematch`: re-resolves legacy bare-facts editions by ISBN, legacy-mapping.md §3.3), `make-admin`, `beta-invite` and `openapi`. `import-legacy` and `rematch-legacy` connect and migrate like `serve`, build the production `AppState`, and print `importer::Report` with its `Display` impl (IMP keeps that impl).

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
    pub async fn with_real_lookup(db: PgPool) -> Self; // lookup = state.catalog (real chain against `upstream`)
    pub fn config(&self) -> &Config;
    pub fn anon(&self) -> TestClient;
    pub async fn user(&self, username: &str) -> TestClient;          // verified email <username>@example.test,
                                                                     // password "password1", signed in
    pub async fn user_unverified(&self, username: &str) -> TestClient;
    pub async fn admin(&self, username: &str) -> TestClient;         // is_admin
    pub async fn club(&self, owner: &TestClient, name: &str) -> TestClub;            // via POST /api/clubs
    pub async fn join(&self, club: &TestClub, who: &TestClient, role: &str);       // direct insert ("admin"|"member")
    pub async fn work(&self, title: &str) -> TestWork;               // ingest::upsert_work (source user, no authors)
    pub async fn ol_work(&self, ol_id: &str, title: &str, authors: &str) -> TestWork;
    pub async fn dnb_work(&self, title: &str, authors: &str, isbn: &str) -> (TestWork, TestEdition); // via upsert_edition_with_work
    pub async fn edition(&self, work: &TestWork, isbn: &str) -> TestEdition;
    pub fn sent_emails(&self) -> Vec<Email>;
    pub fn last_link_token(&self, to: &str) -> String;               // reads the #token= fragment
    pub async fn upstream_requests(&self) -> Vec<wiremock::Request>; // assert "no outbound call"
    pub async fn exec(&self, sql: &str);                              // raw SQL (move time, corrupt rows); panics on error
    pub async fn scalar_i64(&self, sql: &str) -> i64;                 // e.g. "SELECT count(*) FROM users"
}
pub struct TestClub { pub id: i64, pub slug: String, pub name: String }
pub struct TestWork { pub id: i64, pub title: String }
pub struct TestEdition { pub id: i64, pub work_id: i64, pub isbn13: String }

pub struct TestClient { /* Router clone, cookie jar for bookclub_session, optional bearer, client ip */ }
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
    pub fn id(&self) -> i64; pub fn username(&self) -> &str;
}
pub enum Part { File { name: String, filename: String, content_type: String, bytes: Vec<u8> }, Text { name: String, value: String } }
impl Part { pub fn file(filename: &str, content_type: &str, bytes: Vec<u8>) -> Part; /* name "file" */ pub fn text(name: &str, value: &str) -> Part; }
pub struct TestResponse { pub status: StatusCode, pub headers: HeaderMap, pub body: Bytes }
impl TestResponse { pub fn json(&self) -> Value; pub fn detail(&self) -> String;
    pub fn assert_status(&self, s: u16) -> &Self; pub fn assert_detail(&self, s: u16, d: &str) -> &Self; }

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
```

### 17.3 Faking outbound HTTP

- `TestApp::new` starts one `wiremock::MockServer` and points every upstream URL at it.
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
- `tests/auth_required.rs`: an anonymous caller gets **401** on every operation except the public allow-list: `GET /api/health`, `GET /api/config`, `GET /api/openapi.json`, `POST /api/auth/register`, `POST /api/auth/login`, `POST /api/auth/logout`, `POST /api/auth/tokens`, `POST /api/auth/verify-email`, `POST /api/auth/password-reset`, `POST /api/auth/password-reset/confirm`, `GET /api/invites/{code}`, `GET /media/{public_id}/{variant}`.
- `tests/admin_guard.rs`: a signed-in non-admin gets **404 `Not found`** on every `/api/admin/*` operation.
- `tests/clubs_non_member.rs`: a signed-in non-member gets **404 `No such club`** on every `/api/clubs/{slug}/…` operation. It also seeds, in club A, a pick, a club post and a reply, and as a member of club B and as a clubless user calls every `/api/posts/{post_id}…` operation on them (404 `No entry with that id`), `POST /api/books/works/{work_id}/posts` with `parent_id` = the club post (400 `That entry is not on this book`) and `GET /api/diary?club=<A>` (404 `No such club`), and checks that `GET /api/books/works/{work_id}/posts`, `GET /api/diary` and `GET /api/activity` contain none of club A's post or event ids. Until POSTS and CLUB merge, those routes are absent from the document and the walker skips them.

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
- **Never log:** passwords, tokens, cookies, email tokens, the email body (except in `LogEmail` in dev) or full email addresses at INFO.
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
    pub pending_email: Option<String>,   // the address a verification link was last sent to, when not yet verified
    pub needs_email_prompt: bool, pub theme: String, pub color_mode: String, pub locale: String,
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

// Loaders that are not catalog-specific (catalog loaders are in catalog::shared::books, §7.1):
pub async fn user_summaries(conn: &mut PgConnection, ids: &[i64]) -> sqlx::Result<HashMap<i64, UserSummary>>;
```

`ErrorBody { detail: String }` stays in `error.rs`. `MemberYear` (one user's year in review) is SHELF's: it backs both `GET /api/stats` and `GET /api/clubs/{slug}/stats`, which SHELF owns (work-plan).
