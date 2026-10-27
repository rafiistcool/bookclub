# Phase 1 work plan: waves and work packages

This plan splits Phase 1 (#37–#49) into three waves:

- **Wave 1 (Foundation):** one implementer, done serially on branch `phase-1`.
- **Wave 2:** runs in parallel, one git worktree per package.
- **Wave 3:** runs after Wave 2 merges. Its two packages run in parallel with each other.

The contracts every package builds against are in four files:
- `conventions.md`: architecture, interfaces and rules
- `endpoint-map.md`: the HTTP surface and its owners
- `0001_init.sql`: the schema
- `legacy-mapping.md`: the importer

They cite the area specs (`auth`, `catalog`, `club`, `diary`, `frontend`, `google`, `platform`, `shelf`), which are normative too. Wave 1 copies the design documents into the repo as `docs/phase1/` and the specs as `docs/phase1/spec/`, so every worktree has them; every spec reference in the design uses the `docs/phase1/spec/…` path. Nothing a package needs lives only in a session scratchpad.

```
Wave 1  W1 Foundation (#36 done, #37, #39, #40, #46 core, #47 wiring)          serial
          │
Wave 2  CAT #41 ─ HAR #42 ─ MED #43 ─ SHELF #44a ─ POSTS #44b ─ CLUB #45 ─ IMP #38 ─ OAPI #47   parallel
          │
Wave 3  FE #48   ─   PAR #49                                                      parallel
```

## Ground rules for parallel work

1. **Ownership.** Every file belongs to exactly one package, as listed below. A package edits only its own files.
   - If a package needs a change in a file it does not own, especially a **frozen interface**, it stops and asks the integrator. It does not edit the file.
   - Wave 1 freezes the interfaces in `conventions.md` §3 (constructors and entry points), §5.3, §6.2, §7, §8, §10, §15 and §20, plus `AppState`, `error.rs` (including every `error::msg` string of endpoint-map §11.1), `extract.rs`, `dto.rs` and `tests/common/`.
2. **No new crates.** Wave 1 adds every dependency (conventions.md §18), so `Cargo.toml` and `Cargo.lock` are not touched in Wave 2. If a package needs a crate that isn't there, the integrator adds it on `phase-1` and every package rebases.
3. **No new migrations,** normally. `0001_init.sql` is the complete schema. If a package needs one, it uses its reserved version block (conventions.md §13).
4. **Paths.** Each HTTP path belongs to one package (endpoint-map.md), and path parameter names are fixed.
5. **Worktrees, databases and build resources.**
   - Each package runs in `git worktree add ../bookclub-<pkg> -b phase1/<pkg> phase-1`, on disk next to the repo, never under `/tmp` (a 3.9 GiB tmpfs).
   - **Each package runs its own Postgres server** (conventions.md §13): `#[sqlx::test]` names test databases after the test path only, so worktrees sharing one server drop each other's databases. Ports:

     | Package | Port | | Package | Port |
     |---|---|---|---|---|
     | CAT | 5441 | | CLUB | 5446 |
     | HAR | 5442 | | IMP | 5447 |
     | MED | 5443 | | OAPI | 5448 |
     | SHELF | 5444 | | PAR | 5449 |
     | POSTS | 5445 | | integrator, W1, FE | 5432 (compose) |

     `export DATABASE_URL=postgres://bookclub:bookclub@127.0.0.1:<port>/bookclub`.
   - **At most 3 packages build or test at the same time** (the VM has 6 CPUs and 7.8 GiB RAM; every worktree has its own `target/` of the full dependency tree). Each runs with `CARGO_BUILD_JOBS=2` in its environment. The `line-tables-only` debug profile (conventions.md §18) keeps each `target/` small. The orchestrator queues the others.
   - Each package commits its `.sqlx/` files.
6. **Done** means all of these pass in the worktree:
   - `cargo fmt --check`
   - `cargo clippy --all-targets --locked -- -D warnings`
   - `cargo test --locked`
   - `cargo sqlx prepare --check -- --all-targets`

   The parity tests the package owns are ported (`/// parity:` comments), or reasoned in the PR description.
7. **Integration.**
   - Every package opens a PR into `phase-1` on Forgejo (`git.tillhome.de`, with the `forgejo` MCP tools; never GitHub).
   - The integrator merges one PR at a time.
   - After each merge, the integrator:
     1. runs `cargo sqlx prepare -- --all-targets` and commits the regenerated `.sqlx/`
     2. regenerates `frontend/src/api/generated.ts`, once OAPI has merged
     3. runs the full suite
   - Parallel packages do not need to rebase unless a frozen interface changed.

---

## Wave 1: Foundation (W1), serial, one implementer

**Issues:** #37 (schema), #39 (auth), #40 (clubs), the core of #46 (visibility), and the wiring part of #47.

Wave 1 also registers a stub for every later module, so that `lib.rs` never changes again.

### Files W1 creates or changes (W1 owns them; marked **frozen** when Wave 2 may only call them)

| File | Notes |
|---|---|
| `server/Cargo.toml`, `server/Cargo.lock` | All Phase 1 dependencies and the `line-tables-only` debug profile (conventions.md §18). **Frozen.** |
| `server/migrations/0001_init.sql` | The design schema, committed with the design documents. **Frozen.** |
| `server/.sqlx/**` | Initial offline data (shared; every package adds files). |
| `server/Dockerfile` | Adds `ENV SQLX_OFFLINE=true` in the build stage. |
| `server/src/main.rs` | clap CLI: `serve`, `import-legacy` (flattens `importer::Args`, calls `importer::run`, a stub until IMP; `TargetNotEmpty` → exit 2), `rematch-legacy` (`importer::rematch`), `make-admin`, `beta-invite`, `openapi`. Migrations at startup, connect retry, job spawning, `ConnectInfo`. |
| `server/src/lib.rs` | `AppState` (**frozen**), `app()`, `openapi()`, layers (incl. `Referrer-Policy`), fallbacks, module registration, the `/api/admin` router with its `InstanceAdmin` route layer. |
| `server/src/{config,error,extract,dto,text,tz,ratelimit,client_ip,outbound,openapi,spa,health,jobs,visibility}.rs` | **Frozen** APIs per conventions.md (`dto.rs`: §20; `outbound.rs`: `get_bytes`/`get_json`/`get_text`, `RetryPolicy`, `UpstreamError`). `spa.rs` serves `STATIC_DIR`. `openapi.rs` registers every §0 DTO. |
| `server/src/auth/**` | All of #39 except the avatar endpoints (MED) and favourites (SHELF), plus `auth::user_out` and `auth::random` (**frozen**). |
| `server/src/clubs/**` | All of #40, including the `ClubMember`/`ClubAdmin`/`ClubOwner` extractors and `member_by_slug`, `role_in`, `club_refs` (**frozen**), and `clubs/invites.rs` (`normalize_code`, `consume`, `join`: the redemption seam sign-up and invite accept share, conventions.md §5.5). |
| `server/src/admin/{mod.rs,invites.rs}` | The admin router, sign-up invites. |
| `server/src/email/**` | `EmailSender` (**frozen**), `LogEmail`, `SmtpEmail`, templates. |
| `server/src/activity/{mod.rs,record.rs}` | `record()` (**frozen**). `mod.rs` routes to `feed::routes()`. |
| `server/src/activity/feed.rs` | **Stub** with empty `routes()`. Owned by CLUB from Wave 2 on. |
| `server/src/catalog/shared/**` | books, work_ref, isbn, ingest (with provenance), lookup, `dnb_marc` (the one DNB MARC21 mapping), `merge_rules` (**frozen**; conventions.md §7), fully implemented. |
| `server/src/catalog/{mod.rs,service.rs}` | **Stubs.** `CatalogService::new(&Config, Arc<Outbound>)` implements `CatalogLookup` by returning `Unavailable`; `routes()` and `admin_routes()` are empty. Owned by CAT from Wave 2 on. |
| `server/src/media/shared/**` | `MediaStore` trait, `FsMediaStore`, `MemMediaStore`, urls (`storage_prefix`, `object_key`), `register`, `is_removed_hash` (**frozen**; conventions.md §8). |
| `server/src/media/{mod.rs,service.rs}` | **Stubs.** `MediaService::new(&Config, Arc<dyn MediaStore>, Arc<Outbound>)`, a no-op `prefetch`, empty `routes()` and `admin_routes()`. Owned by MED. |
| `server/src/{harvest,shelf,posts,picks,votes,importer}/mod.rs`, `server/src/overlap.rs` | **Stubs** with the frozen signatures of conventions.md §3: empty `routes()`; `harvest::run_forever(state, shutdown)` returns immediately; `shelf::goodreads::resume(state)` is a no-op; `importer::run`/`rematch` return `ImportError::Other("not implemented")`; `importer::Args`/`RematchArgs` carry the flags of legacy-mapping.md §0. Owned by the respective packages from Wave 2 on. |
| `server/tests/common/**` | Harness (conventions.md §17.2), including `FakeCatalog`, `CapturingEmail`, `visibility.rs` template helpers. **Frozen.** |
| `server/tests/{health,platform,jobs,db_helpers,ingest,harness,auth_*,clubs_*,visibility_policy,api_error_copy,config,openapi_smoke,auth_required,admin_guard,clubs_non_member,dnb_marc}.rs` | Wave 1 tests. `harness.rs` tests `tests/common` itself (fixtures, `TestClient` incl. multipart, `FakeCatalog`, the wiremock wiring). The auth tests so far (step 3): `auth_register.rs` (sign-up, invites, the verification mail, the "address in use" notice, the sign-up and per-address limits), `auth_login.rs` (login, logout, the failure and IP limits, concurrent guesses, the legacy hash and rehash), `auth_account.rs` (me, preferences, notification preferences, password change), `auth_sessions.rs` (cookies, sliding expiry, bearer tokens, the session list, the origin check with a session), `auth_config.rs` (`GET /api/config`, the self-hosting ports, the rate-limit keys, unsafe production settings). Step 4 adds `auth_email.rs` (verification links, resend, `PUT /api/auth/me/email`, the email prompt, and the email security tests of deliverable 4), `auth_reset.rs` (password reset and the links' invalidation), `auth_delete.rs` (account deletion: personal data and every avatar, tombstones, owned clubs, the records that outlive the account) and `auth_export.rs` (the export, its sections and media, and the `pg_constraint` completeness check). `health.rs` is updated to build `AppState` through the harness. `platform.rs` covers the fallbacks, the SPA, the origin check, CORS and the response headers. `jobs.rs` covers `jobs::run_locked`, `auth::cleanup` and the shutdown of `spawn_all`; `db_helpers.rs` covers `admin::make_admin`, the `dto` loaders, `media::shared::register`/`is_removed_hash` and `activity::record`. `ingest.rs` covers `catalog::shared` against Postgres (provenance rank pairs, modes, grouping, merges, the legacy upgrade, covers, admin edits, loaders, `WorkRef`, `local_isbn`) and its concurrency: two transactions grouping one DNB title, racing inserts of one ISBN, a deadlock between page transactions, and a takedown waiting for an ingest that attaches its cover; `dnb_marc.rs` maps the MARC fixtures and ingests them through `upsert_dnb_record`. |
| `server/tests/fixtures/dnb_marc/**` | MARC21-xml records for the `dnb_marc` mapping tests (conventions.md §7.5), copied from recorded DNB reiheN pages. |
| `.forgejo/workflows/ci.yml`, `.github/workflows/ci.yml` | Server job: `cargo sqlx migrate run` plus `cargo sqlx prepare --check -- --all-targets` before the tests. |
| `docs/phase1/{conventions,endpoint-map,legacy-mapping,work-plan}.md` | Copies of the design. |
| `docs/phase1/parity-notes.md` | One line per Python test that has no Rust port (dropped, replaced or moved), with the reason; packages add their own section. PAR builds the checklist from it and the `/// parity:` comments. |
| `docs/phase1/spec/{auth,catalog,club,diary,frontend,google,platform,shelf}.md` | Copies of the area specs the design cites. |
| `CLAUDE.md` (root) | Adds the server's build, test and sqlx commands, the per-worktree Postgres server rule (one server per worktree, port table) and the build-concurrency rule. |

### Wave 1 deliverables and order

1. **Dependencies and CLI.** Add the dependencies; wire `main.rs` (clap); extend config; startup migration with connect retry. The existing scaffold tests stay green.
2. **Platform pieces.** Write `error.rs`, `extract.rs`, `text.rs`, `tz.rs`, `outbound.rs`, `ratelimit.rs`, `client_ip.rs`, `spa.rs` and the fallbacks, with unit tests: tz vectors from docs/phase1/spec/club.md §2.1; text helpers; JSON rejection mapping; lax bool; 404/405 JSON.
3. **Migration and harness.** Add `0001_init.sql`, then the `tests/common` harness, then `tests/visibility_policy.rs` (the SQL policy functions across the viewer matrix, including the deleted-account tombstone and NULL owners, conventions.md §6.5).
4. **Auth (#39).**
   - **Endpoints:** register, login, logout, me, preferences, password, notifications, verify, resend, email change, prompt dismiss, reset, tokens, sessions, account deletion, export.
   - **Mechanisms:** sessions, origin check, rate limits, argon2 with legacy hash verification and rehash.
   - **Tests:** sign-up, verification, login, logout, reset, change, tokens, rate limiting, the legacy hash, deletion (personal data and media gone, including a user who replaced their avatar twice; club posts handled per conventions.md §5.6; another user's reaction event shows `author: null` afterwards), and export completeness (the `pg_constraint` check of §5.7).
   - **Email security tests** (conventions.md §5.5): an old verification link dies when the address changes or a new link is sent; a reset link dies on password change, reset, email change and verification; reset confirm refuses a token whose address is no longer the verified one; register and `PUT /me/email` answer the same for a free address and for one a verified account holds (and that account gets the notice, not a link); an unverified address does not block another account from verifying it; `PUT /me/email` is limited (429) and the per-address limit is silent; links carry the token in the fragment.
   - **Rate-limit keys:** two IPv6 addresses in one /64 share a bucket; unknown identifiers land in at most 65,536 buckets; startup fails without `TRUSTED_PROXIES` outside DEV_MODE.
   - **Concurrent guesses:** N parallel wrong logins (and wrong current passwords on `PATCH /api/auth/password`) for one account verify at most 10 passwords; login uses `login_failures.check_and_hit` before verifying (conventions.md §10.1).
   - **Parity tests:** `test_auth.py` (22: health and register become reasoned changes; the backfill test moves to IMP), `test_selfhost.py` (13: mostly reasoned as "self-hosting removed"; the cookie Secure/HttpOnly/SameSite, trusted-proxy and CORS behaviours are ported to the new config), `test_backup.py` (2, replaced by the export tests), `test_notes_and_meeting.py::test_change_password*` and `::test_notification_prefs_default_on_and_patch`. Step 3 ported or marked all of them except the three backup tests, which step 4 marks on its export tests, and the backfill test (IMP); review-log.md "Wave 1 auth step" has the table. Every reasoned drop and move is listed in `docs/phase1/parity-notes.md`, which PAR reads together with the `/// parity:` comments.
5. **Clubs (#40).**
   - **Endpoints:** clubs CRUD, members list, role change, removal, leave, transfer, invites (create/list/revoke), preview, accept, sign-up invites.
   - **Tests:** both limits (10 clubs created, 20 members); every role permission row; ownership transfer; invite expiry, revocation and reuse (a link invite is redeemable while it has no `max_uses`; a member who left can rejoin with the same link; a removed member cannot rejoin with an invite created before the removal, but can with a newer one); concurrent redemption of a single-use invite (exactly one success).
   - **Owner races:** transfer concurrent with removal of the new owner, and transfer concurrent with the new owner leaving, end with exactly one owner (the in-statement guards); a raw `DELETE` of the owner membership fails at COMMIT (the deferred trigger).
   - **Route-wide guards** (conventions.md §17.7): `auth_required.rs` (401 for anonymous callers outside the public allow-list), `admin_guard.rs` (404 for signed-in non-admins on every `/api/admin/*`), and `clubs_non_member.rs`, which walks **every route in the OpenAPI document** whose path starts with `/api/clubs/{slug}` as a signed-in non-member (404 `No such club`) and also covers club rows reached by a global id or query parameter (`/api/posts/{post_id}…`, the diary reply route, `/api/diary?club=`, and the absence of club A ids in the home feeds). The tests are generic, so they cover Wave 2 routes automatically once they merge.
   - **Parity:** `test_members.py::test_members_hide_self_and_preview_reading` (reasoned: the caller is now included, as `you`).
6. **Shared interfaces.** Implement `catalog/shared` (including `dnb_marc` and `merge_rules`), `media/shared`, `dto.rs`, the `outbound` functions and `activity::record`, with unit tests: ISBN conversion and checksums, `is_german_group`; `WorkRef` parsing incl. hashed GB keys; ingest InsertOnly vs Refresh vs Replace; **one test per provenance rank pair** (a source never overwrites `user`/`admin`, overwrites `legacy` and equal rank, fills empty fields); creating DNB and Legacy works with authors through `upsert_edition_with_work`; the NULL fill of work pages/year/cover from editions; `search_extra` and `match_key`; `ol_cover_media` returning None for a removed row; `set_cover` with `only_if_empty`; the `dnb_marc` fixtures of conventions.md §7.5 (formats, multi-ISBN records, inverted names, translators with and without GND, 263, no 520/856); the `merge_rules` functions.
7. **Stubs.** Add every stub module with the frozen signatures of conventions.md §3; `openapi()` builds; `tests/openapi_smoke.rs` asserts unique operation ids of the form `<module>_<action>`, a tag and responses on every operation, unique `ToSchema` names (source scan), and that every §0 DTO is in the document.
8. **API error copy.** Define every endpoint-map §11.1 string as an `error::msg` constant (including SHELF's and MED's), then add `tests/api_error_copy.rs` with the `LEGACY_ONLY` allow-list. It passes from now on in every worktree.
9. **CI and docs.** Update CI and the Dockerfile; commit `.sqlx/`; copy the design docs to `docs/phase1/` and the specs to `docs/phase1/spec/`; update `CLAUDE.md`; tag `phase1-wave1`.

**Exit criteria:**
- Everything is green.
- `bookclub-server openapi` prints valid JSON.
- A signed-in user can create a club, invite a second user, and both see each other's empty shelves through the visibility policy. (The shelf endpoint comes in Wave 2, so the Wave 1 test calls the policy function directly.)

---

## Wave 2: parallel packages

Every package below may call (but not change):
- `error.rs` (incl. `error::msg`), `extract.rs`, `dto.rs`, `text.rs`, `tz.rs`, `outbound.rs`, `ratelimit.rs`, `client_ip.rs`
- `visibility.rs` and the SQL policy functions
- `auth::extract`, `auth::user_out`, `auth::random`, and `clubs::extract` (`member_by_slug`, `role_in`, `club_refs`)
- `catalog::shared` (incl. `dnb_marc`, `merge_rules`), `media::shared`, `activity::record`
- `AppState` fields and the frozen constructors and entry points of conventions.md §3 (e.g. CAT calls `state.media_service.prefetch`)
- `tests/common`

Each package owns `server/tests/<prefix>_*.rs` for its tests and `server/tests/fixtures/<dir>/**` for its fixtures.

### CAT: catalog core (#41)

- **Owns:**
  - `server/src/catalog/**` except `catalog/shared/**` (this includes `catalog/mod.rs` and `catalog/service.rs`)
  - `server/tests/catalog_*.rs`
  - `server/tests/fixtures/openlibrary/**` and `server/tests/fixtures/dnb_sru/**`
- **Builds:**
  - **`CatalogService`:**
    - Open Library client: search, trending, subjects, work, authors, search-by-key, ISBN search, `/isbn/{isbn}.json` (cover check).
    - DNB SRU client (fallback, 1 req/s, cached), through `outbound::get_bytes`; records are mapped by `catalog::shared::dnb_marc` (never a second MARC parser) and ingested with `ingest::upsert_dnb_record(…, Mode::Replace)`; a lookup aborted by a deadlock is retried once.
    - Response caches and coalescing (moka, 18 h, empty search results not cached).
    - `CatalogLookup` implementation, including `resolve_isbn_remote`, with the routing by registration group of conventions.md §7.4 (978-3 → DNB SRU → Open Library; others → Open Library only; fall-through when DNB is cooling down; Err rather than Ok(None) without a definite answer), and the one Open Library cover check after a DNB hit and on a DNB work's first detail view.
  - **Search pipeline:**
    - Local FTS and trigram search.
    - Remote candidates, with the legacy candidate rules (raw query first, `X by Y` split, title/author fallbacks).
    - Persisting hits, then `state.media_service.prefetch` for their covers.
    - Merge and dedupe, capped at `limit`; merged-away works excluded.
    - Annotation: `on_shelf`, `shelf_id`, `club_picks` (via `bc_member_clubs`).
  - **Detail** (`BookDetailOut`; readers, rating and quote count through `bc_visible_shelf`/`bc_visible_quotes`), with one-time enrichment of stubs.
  - **Refresh:** rate-limited, `Mode::Refresh` under the provenance rules (never overwrites `user`/`admin` fields or covers).
  - **ISBN lookup:** the full chain, persisted.
  - **Custom books:** user-added works and editions, provenance `user`; with an ISBN, `resolve_isbn_remote` first (a hit is 409).
  - **Editions:** list, add (same ISBN rule), get (`edition_outs`).
  - **Edit and merge requests**, plus the admin routes in `catalog::admin_routes()`: approve applies fields through `ingest::apply_admin_fields`; `cover_replace` through `ingest::set_cover(…, Admin)`; `work_merge` and the direct admin merge through `catalog::merge_works` (`catalog/merge.rs`, conventions.md §7.6).
- **Endpoints** (endpoint-map §4): everything under `/api/books/*` except `…/posts`, `…/cover` and `/new-releases`; plus `/api/admin/edit-requests*` and `/api/admin/works/{work_id}/merge`.
- **Parity:**
  - `test_search.py` (35)
  - `test_book_details.py` (8)
  - the catalog half of `test_local_reads.py`
  - the KEEP and ADAPT tests of `test_google_books.py` (docs/phase1/spec/google.md §4). DROP and INVERT tests are listed in the PR with their reason.
- **Acceptance (#41):** local results first; remote merged without duplicates; ISBN lookup for German and English test ISBNs with faked HTTP (an English ISBN makes **no** SRU request; a German one falls through to Open Library while DNB cools down); client-side rate limits enforced; no Google code or data under `server/` (`tests/catalog_no_google.rs`, with the importer carve-out of legacy-mapping.md §0).
- **Merge tests:** `merge_works` with users who have both works on their shelf and as favourites (and in their layouts), and a vote that nominated both.
- **Provenance tests:** a refresh after a user upload or an approved edit leaves the cover and the fields alone; a legacy work is upgraded by a source record for its ISBN.
- **Visibility tests:** `tests/catalog_visibility.rs` covers book-detail readers, rating and quote count across viewer types, with `only_me` ratings **and `only_me` quotes**.

### HAR: DNB harvester (#42)

- **Owns:**
  - `server/src/harvest/**`
  - `server/tests/harvest_*.rs`
  - `server/tests/fixtures/dnb_oai/**` (recorded `ListRecords` pages, including one resumption token, a `badResumptionToken` error, a deleted header, a not-yet-published title and the same IDN re-delivered with a final title)
- **Builds:**
  - **OAI-PMH `ListRecords` client** through `outbound::get_bytes` with `RetryPolicy::JOB` (`metadataPrefix=MARC21-xml`, set, `from`, resumption tokens; ≥ 1 s between pages; obeys 503 `Retry-After`).
  - **Envelope handling only.** Each `<record>` goes to the frozen W1 mapping `catalog::shared::dnb_marc::map_record` (conventions.md §7.5: the CC0 field list, format, multi-ISBN records, natural-order names, translators). HAR never parses MARC fields itself, and so never stores 520, 856 or any VLB text.
  - **Deleted headers** (`<header status="deleted">`): `ingest::withdraw_dnb_edition(idn)`; the metadata is never mapped.
  - **Ingest:** `ingest::upsert_dnb_record(record, Mode::Replace)` per record (the first edition with `WorkGrouping::DnbTitleAuthor`, the others into its work; conventions.md §7.2). A page transaction that Postgres aborts with a deadlock (`error::is_deadlock`) is rolled back and retried (conventions.md §7.2 "Deadlocks"). Replace means DNB-owned fields are replaced as a whole (None clears them), user and admin fields are never touched, a re-delivered record with a final title retitles a single-edition DNB work, and `planned_publication_date` only changes when the record carries 263. Grouping: an existing edition by ISBN or IDN first, else the `match_key` index among `source='dnb'` works.
  - **Sets:** every entry of `DNB_HARVEST_SETS` (default `dnb-all:reiheN,dnb:reiheA`), each with its own `harvest_state` row (`dnb:<setSpec>`). A reiheA record for a title harvested from reiheN finds the edition by ISBN or IDN and updates it.
  - **Cursor** in `harvest_state`:
    - The first run of a set starts at now − `DNB_HARVEST_START_DAYS` (default 90); bulk history is #63.
    - Each page's upserts, the next `resumption_token` (with `token_issued_at`) and `records_seen` are written **in one transaction**, so a crash resumes after the last committed page and never skips one.
    - On `badResumptionToken`, or a stored token older than 6 h, the token is cleared and the run restarts from `run_from`.
    - `harvested_until` = the run start minus 1 h, written when the run completes.
  - **`run_forever(state, shutdown)`:** every `DNB_HARVEST_INTERVAL_MINUTES`, never more often than every 30 min, under an advisory lock.
  - **`GET /api/books/new-releases`** with the date window of endpoint-map §5, `SearchHit.planned_publication_date` filled.
- **Acceptance (#42):** OAI envelope tests on the recorded fixtures (the MARC mapping tests are W1's); a harvest resumes from its cursor after a simulated crash (the task is aborted between pages) and restarts from `run_from` after a rejected token; a deleted record withdraws its edition and drops it from new-releases; a retitled record replaces the provisional title; a not-yet-published fixture title becomes searchable through the local catalog and resolvable through `CatalogLookup::resolve_isbn`'s local step (`ingest`).

### MED: media store (#43)

- **Owns:**
  - `server/src/media/**` except `media/shared/**`
  - `server/tests/media_*.rs`
  - `server/tests/fixtures/images/**`, including a JPEG with GPS EXIF
- **Builds:**
  - The image pipeline (conventions.md §8).
  - Avatar `PUT`/`DELETE /api/auth/me/avatar`, with the `error::msg` picture strings and limits; `PUT` deletes the previous avatar row and its bytes; both return `auth::user_out`.
  - `GET /media/{public_id}/{variant}`: `removed_at` checked first; lazy Open Library cover fetch by cover id through `outbound::get_bytes` (`RetryPolicy::MEDIA`, up to 10 s), coalesced per media id; `Cache-Control: no-store` on every non-200.
  - `MediaService::prefetch` and its bounded background worker.
  - Dominant colour: a port of `spine_color.py` (median cut, scoring, lightness clamp).
  - `POST /api/shelf/spine-colors` (backfill; never fetches a removed row).
  - Cover upload `POST /api/books/works/{work_id}/cover`: `ingest::set_cover(…, User, only_if_empty = true)`; when that returns false, a `cover_replace` row in `catalog_edit_requests`, which CAT's approve applies. An upload matching a removed row's hash is refused.
  - Reports: `POST /api/media/{public_id}/reports`; the admin routes in `media::admin_routes()`: `GET /api/admin/reports`, `POST …/resolve`, `DELETE /api/admin/media/{public_id}`. A takedown deletes the bytes, sets `removed_at` and calls `ingest::clear_media`.
- **Parity:**
  - `test_avatars.py` (8). The ETag and `?v=` tests become "immutable URL, long cache, nosniff"; the deferred-blob test is reasoned away.
  - `test_spine_color.py` (9). The SSRF allow-list is `outbound`'s redirect check (conventions.md §10.3, tested in W1); MED ports the allow-list tests against `get_bytes` redirects, with Google hosts removed. A refused redirect (`UpstreamError::Redirect`) and an oversize body (`TooLarge`) settle as `""`.
- **Acceptance (#43):** an uploaded JPEG with GPS EXIF comes back without metadata; a cover renders with no client request to openlibrary.org (the response URL is same-origin and the bytes come from the wiremock covers host exactly once); reporting creates a queue item, and an admin can remove the cover; **after a takedown**, a refresh, an enrichment and a re-upload of the same file leave `cover_media_id` NULL and make no outbound request; two concurrent first uploads give one applied cover and one `cover_replace` request.

### SHELF: personal shelf and data (#44a)

- **Owns:**
  - `server/src/shelf/**`: shelf entries, ordering, finish fields, reading dates, favourites, physical layout, the Goodreads parser and job, quotes, search history, personal stats **and club stats** (`GET /api/clubs/{slug}/stats`: both are built on one `MemberYear` function, so the rules for pages, `only_me` exclusion and rounding exist once)
  - `server/tests/shelf_*.rs`
  - `server/tests/fixtures/goodreads/**` (a copy of `backend/tests/fixtures/goodreads.csv`)
- **Builds:**
  - The endpoint-map §7 routes, the favourites routes from §2 and `GET /api/clubs/{slug}/stats` from §9. Reads of other users' shelves, ratings and quotes go through `bc_visible_shelf`/`bc_visible_quotes`; the mapped detail strings come from `error::msg`.
  - Ordering and date semantics are byte-faithful to docs/phase1/spec/shelf.md (`place_item`, `place_on_shelf`, `apply_finish_fields`, `apply_reading_dates`, the tri-state PATCH, the "Nothing to update" check before the 404).
  - Per-user locking.
  - Rating visibility.
  - The Goodreads background job through `state.lookup` (FakeCatalog in tests), with skip codes, rating and date import.
  - Personal stats, with `round_half_even`.
  - Events through `activity::record`.
- **Parity:**
  - `test_shelf.py` (16)
  - `test_reading_dates.py` (12)
  - `test_physical_shelf.py` (14, with all 35 parametrised cases)
  - `test_favorites.py` (12)
  - `test_import.py` (8)
  - `test_search_history.py` (3)
  - `test_stats_milestones_quotes.py::{test_stats_year_in_review, test_stats_with_empty_club, test_quotes_crud_and_filters}`: the personal-stats, club-stats and quotes parts.
- **Acceptance (#44, its share):** every listed Python test has a Rust counterpart or a recorded reason; the Goodreads import works with the same sample CSV; visibility tests (`tests/shelf_visibility.rs`) cover the shelf, favourites, layout (GET and the PUT→404 acceptance criterion), quotes and stats.

### POSTS: diary entries and pick discussions (#44b, also the pick-post part of #45)

- **Owns:**
  - `server/src/posts/**`
  - `server/tests/posts_*.rs`
- **Builds:**
  - The endpoint-map §8 routes, with the two scopes in one table (conventions.md §6.3). Every read goes through `bc_visible_posts`; every write and reaction on `/api/posts/{post_id}` goes through `visibility::post_access` (404 for missing, tombstoned or invisible posts; moderation by the role in **that** post's club).
  - Position snapshots, the live `author_rating` (redacted for `only_me`), tombstones, the atomic reaction toggle, `note_posted` and `reaction` events for club posts (`reaction` with `author_id`).
  - The combined feed and the per-club feed (`?club=` via `clubs::extract::member_by_slug`).
  - Milestone validation on club posts (SQL on `milestones`).
  - Closed-pick checks (SQL on `club_picks`).
- **Parity:**
  - `test_diary.py` (15). `test_legacy_pick_notes_are_imported_once_with_reactions` moves to IMP; `test_entry_on_the_current_pick_records_the_pick` becomes "a club post records its pick".
  - `test_pick_posts.py` (5)
  - `test_notes_and_meeting.py::{test_post_carries_spoiler_flag_and_viewer_progress, test_spoiler_flag_must_be_a_percentage, test_author_can_edit_and_delete_own_note, test_other_members_cannot_edit_or_delete, test_reactions_toggle_and_aggregate, test_deleting_a_note_removes_its_reactions}`
- **Visibility tests:** `tests/posts_visibility.rs` covers diary roots (`audience`/`only_me`), replies by strangers, reaction user lists, `parent_author`, club posts across clubs, **reacting to an invisible post (404, body not returned)**, a club admin of club B unable to delete a club A post, and the deleted-account tombstone whose replies stay visible to their audience.

### CLUB: club features (#45)

- **Owns:**
  - `server/src/picks/**`: pick, history, PATCH meeting, ICS, milestones
  - `server/src/votes/**`: vote, deadline, nominations, cast, apply, auto-close, suggestions
  - `server/src/overlap.rs`, `server/src/activity/feed.rs`
  - `server/tests/club_*.rs`
- **Builds:**
  - The endpoint-map §9 routes, with the role table.
  - Lazy auto-close under a lock on the vote row.
  - Activity feeds: club and home (conventions.md §6.4), with `reaction` payloads served as `author` (resolved from `author_id`).
  - The ICS golden output.
  - Readers, overlap and suggestions read ratings through `bc_visible_shelf` restricted to club members.
- **Parity:**
  - `test_pick.py` (8): the timezone tests become club-timezone tests, and the invalid-timezone test becomes "400 on club update".
  - `test_vote.py` (7)
  - `test_vote_deadline.py` (5)
  - `test_overlap.py` (4)
  - `test_activity.py` (3)
  - `test_stats_milestones_quotes.py::test_milestones_crud_and_threads` (the club-stats assertions are SHELF's)
  - `test_notes_and_meeting.py::{test_meeting_ics_download, test_ics_folds_long_lines_without_breaking_utf8}`
  - `test_push.py` (10): **reasoned** (Phase 4)
- **Acceptance (#45):** a user in two clubs sees separate picks, votes, milestones, threads and activity; ids from club A under club B's path give 404; every role row has an allowed and a denied case; concurrency tests show that two first nominations create one vote, a concurrent auto-close applies once, and two concurrent `PUT pick` calls leave one current pick.
- **Visibility tests:** `tests/club_visibility.rs` covers reader rating redaction, `shelf_finished` payload redaction, hidden `quote_added` events, and `joined_at` filtering.

### IMP: legacy importer (#38)

- **Owns:**
  - `server/src/importer/**`
  - `server/tests/importer_*.rs`
  - `server/tests/fixtures/legacy/**`: `fresh.sql`, `migrated.sql`, `generate.py`, and a `README.md` on how to regenerate them
- **Builds:** everything in `legacy-mapping.md`: `importer::Args`/`RematchArgs` and the frozen `run`/`rematch` entry points (conventions.md §3), the single transaction, the dry run, `--offline`, `--allow-unmatched`, the lookup-error policy, id preservation and the `setval` order, the normalisation of out-of-range values, the books→works classification and ISBN re-match through the injected `CatalogLookup`, merges through `catalog::shared::merge_rules`, the layout remap, the posts scope rule (only mirror copies and new pick notes are club posts) and mirror handling, event payload remaps (`author_id`, `quote_id` by time), the report, and `rematch-legacy`.

  The importer writes the target tables **directly with SQL**; it is the only package allowed to insert explicit ids. Works and editions still go through `catalog::shared::ingest` (`Source::Legacy`, provenance `legacy`). Media goes through `media::shared` (`register`, `object_key`).
- **Parity:**
  - `test_migrations.py` (2): replaced by tests on the migrated-shape fixture
  - `test_auth.py::test_preference_columns_backfill_pre_existing_users`
  - `test_diary.py::test_legacy_pick_notes_are_imported_once_with_reactions`
- **Acceptance (#38):** the tests in legacy-mapping.md §17. The dry run and the real run on a copy of the production DB are a **manual step with Rafael**, and #38 may stay open until then.

### OAPI: OpenAPI types and CI (#47)

- **Owns:**
  - `frontend/scripts/gen-api-types.mjs` (new)
  - `frontend/src/api/generated.ts` (new; generated)
  - `frontend/package.json`: the `openapi-typescript` devDependency and the `gen:api` and `check:api` scripts
  - `frontend/package-lock.json`: the lockfile change that devDependency brings (`npm install --save-dev`), so `npm ci` keeps working
  - the CI step in `.forgejo/workflows/ci.yml` and `.github/workflows/ci.yml`
  - `server/tests/openapi_contract.rs`
- **Builds:**
  - `npm run gen:api` runs `cargo run --quiet --manifest-path ../server/Cargo.toml -- openapi > openapi.json` (needs no database: `SQLX_OFFLINE=true`), then `openapi-typescript`, writing `src/api/generated.ts`.
  - CI regenerates the file and fails on `git diff --exit-code`.
  - `openapi_contract.rs` checks that every response DTO name in `endpoint-map.md` §0 exists in the document, so a renamed DTO is caught. W1 registers all of them in `openapi::ApiDoc` (conventions.md §14), so the test passes in OAPI's worktree before the other packages merge.
- **Acceptance (#47):** changing a DTO without regenerating fails CI; the frontend type-checks against the generated types. (The step-by-step replacement of `types.ts` is FE's job.)
- **Concurrency:** it runs in parallel with the other Wave 2 packages. Until they merge, the generated file only covers what has merged; the integrator regenerates it after each merge.

---

## Wave 3: runs after every Wave 2 package has merged

### FE: adapt the Vue app (#48)

- **Owns:**
  - `frontend/**`, except `src/api/generated.ts`, which only the script writes
  - deleting `backend/tests/test_api_error_copy.py`
  - emptying the `LEGACY_ONLY` list in `server/tests/api_error_copy.rs` (a one-line handover from W1)
- **Builds** (docs/phase1/spec/frontend.md §5 is the detailed checklist):
  - **Build and dev setup:** `vite.config.ts` uses `outDir` `dist` and proxies to `127.0.0.1:8080`.
  - **API client:** `client.ts` gets one place for future headers and global 401 handling (redirect to `/login?next=`).
  - **Types:** `types.ts` is replaced by aliases of the generated types, step by step.
  - **Club context:** a `useClubs` store (from `/api/auth/me` `clubs` and `/api/clubs`), a club switcher, and `/c/:slug/…` routes for pick, vote, members and invites, overlap, discussions (club posts, reusing `BookDiary`/`DiaryEntry`) and events (the activity feed).
  - **Home:** club cards, the combined activity feed and the combined diary.
  - **Solo mode:** no club calls when the user has no clubs.
  - **Join and create:** `/join/:code` and `/clubs/new`.
  - **Auth pages:** sign-up (email, timezone from the browser), verify-email and reset-password (they read the token from the URL **fragment**, `#token=…`, and POST it), forgot-password, the one-time email prompt, and the pending address (`UserOut.pending_email`). Register and email change never show "already in use" (the server does not say).
  - **Settings:** email, password, sessions, notification prefs, export, delete account; invites move to the club page.
  - **Visibility toggles:** on diary entries, quotes and ratings.
  - **Quotes:** a `/quotes` page.
  - **Covers:** `BookCover.vue` is rewritten to use same-origin `cover_url`/`cover_thumb_url` only (one retry after a short delay when a cover fails to load), with a minimal "Report cover" sheet that posts to `/api/media/{cover_public_id}/reports`.
  - **Book identity:** books are identified by numeric `id`; `/book/:id`. Legacy `/book/OL…W|BC…|ISBN…|GB…` routes resolve through `GET /api/books/works/{legacy}` and then `router.replace` to `/book/{id}`, keeping the `#entry-` hash.
  - **Meeting edit:** uses `PATCH /api/clubs/{slug}/pick`.
  - **Goodreads import:** a job with polling and skip codes translated.
  - **Translations:** every new detail string in endpoint-map.md §11 is mapped in `apiErrors.ts`, with en/de keys.
  - **Old routes** redirect.
- **Optional** (not required by #48): the milestones UI, the stats pages, an ICS download link, an edition picker, cover photo upload, "these are the same book" merge requests.
- **Acceptance (#48):** `npm run typecheck && npm test && npm run build` pass; a user in two clubs can switch between them; a user with no clubs can use the shelf, diary and quotes; i18n key parity holds.

### PAR: parity checklist (#49)

- **Owns:**
  - `docs/phase1/parity-checklist.md`
  - `server/tests/parity_*.rs` (cross-package end-to-end tests)
  - fixes in `server/src/**` for bugs it finds. FE does not touch the server, so these edits do not overlap; each fix goes to the integrator as a separate PR.
- **Builds:**
  - **The checklist.** It is generated by grepping the `/// parity:` comments and reading `docs/phase1/parity-notes.md` (the reasoned drops, replacements and moves, one line per Python test). There is one section per `backend/tests` file and one line per Python test, each mapped to its Rust test or given a reason. The known intentional changes are recorded: backup → export; Google removed; single club → club-scoped; push in Phase 4; `/work/{id}` merged; strict JSON types; auth before body; RFC 3339 `Z`; health shape; no "email already in use" on register and email change; tokens in the link fragment.
  - **The #37 schema AC, as built.** "Club-scoped tables carry club_id with FK, personal tables don't" holds with two recorded, reasoned deviations (also noted on #37 and in the schema header): `posts` and `events` hold both scopes in one table (club_id NULL = personal; `posts_parent_fk` ties a reply to its parent's work and scope, `events_personal_no_pick` keeps a pick out of personal events), and `post_reactions` has no `club_id` because it inherits its scope through `post_id`.
  - **End-to-end tests:**
    - `test_local_reads.py` in full, against the real `CatalogService` with no upstream calls
    - the Goodreads import against `with_real_lookup` and wiremock Open Library
    - the importer run with the real lookup, offline
    - the #46 matrix completeness check: every personal data kind has self, co-member, stranger and unknown-name cases
- **Acceptance (#49):** every Python test is checked off or reasoned.

---

## Dependencies and concurrency summary

| Package | Hard dependencies | Runs alongside | Notes |
|---|---|---|---|
| W1 | #36 | none | serial |
| CAT | W1 | all of Wave 2 | Its real `CatalogLookup` replaces the stub; nobody else depends on its internals. |
| HAR | W1 (`ingest`, `dnb_marc`) | all of Wave 2 | The MARC mapping is W1's, so HAR and CAT's SRU client share it without waiting for each other. |
| MED | W1 (`media::shared`, `ingest::set_cover`/`clear_media`) | all of Wave 2 | `cover_replace` approval is CAT's; MED's tests check that the request row is created. CAT calls MED's `prefetch` through its frozen signature (a no-op until MED merges). |
| SHELF | W1 | all of Wave 2 | Its tests use `FakeCatalog`; the end-to-end test is in PAR. Owns personal and club stats. |
| POSTS | W1 | all of Wave 2 | It reads `club_picks` and `milestones` with SQL only. |
| CLUB | W1 | all of Wave 2 | It reads `shelf_entries`, `posts` and `quotes` with SQL only. |
| IMP | W1 (`ingest`, `merge_rules`, `media::shared`) | all of Wave 2 | Its tests use `FakeCatalog` and `MemMediaStore`. |
| OAPI | W1 | all of Wave 2 | The integrator regenerates `generated.ts` after each merge. Every §0 DTO is registered by W1, so its contract test passes alone. |
| FE | all of Wave 2 merged (a stable OpenAPI) | PAR | Frontend only. |
| PAR | all of Wave 2 merged | FE | Server tests and fixes only. |

**Suggested merge order in Wave 2:** CAT, MED, SHELF, POSTS, CLUB, HAR, IMP, then OAPI last, with one final regeneration. Any order works. At most 3 packages build at a time (ground rule 5); a sensible batching is {CAT, MED, SHELF}, then {POSTS, CLUB, HAR}, then {IMP, OAPI}.

---

## Appendix: the readers' open questions, resolved

Each line gives the decision and where it is specified. Items marked **[Rafael]** need his decision or input; everything else is decided by this design.

### auth

1. **Closed-beta gate:** `SIGNUP_MODE=invite` (the default until #55). Sign-up needs a club invite (it joins the club) or a club-less sign-up invite minted by an instance admin (`POST /api/admin/invites`, or the CLI `beta-invite`). → endpoint-map §2, §3.
2. **Login identifier:** username **or** email (it contains `@`).
3. **Sessions:** server-side, with sliding idle expiry: 30 days for cookies, 90 days for bearer tokens. The cookie's Max-Age is 400 days. → conventions.md §5.1.
4. **Password change** revokes every other session and token. A reset revokes all of them.
5. **Invites** have an optional expiry plus an optional `max_uses`, and at least one must be set. There are two presets: `single_use` (1 use, 30 days) and `link` (7 days, unlimited uses). Admins can revoke them.
6. **Invite visibility:** admins see every invite of the club.
7. **Invite errors:** unknown, expired, revoked and used-up all keep `That invite code is not valid`. New strings: `This club is full`, `You are already in this club`, and the two role 403s. The favourites string becomes `That book is not in the catalog`.
8. **Legacy invites:** unused ones are imported **revoked**; `joined_at` = `users.created_at`.
9. **Member roster:** includes the caller (`you: true`), ordered owner, admins, members, then username.
10. **Avatars:** WebP 256×256, public unguessable immutable URL (`/media/{public_id}/a`). Legacy JPEG blobs are imported as they are.
11. **Timezone:** both. `clubs.timezone` for club times, `users.timezone` for personal dates.
12. **Docs and config:** `/api/openapi.json` is public. `/api/config` = `{name, theme, theme_dark, public_url, signup}`; the name is `SERVICE_NAME` ("Bookclub" until #50), and the theme defaults are unchanged.
13. **Account deletion:** club posts are deleted, or become tombstones if they have replies. Invites the user created get `created_by` NULL. Events where the user is the actor are deleted. → conventions.md §5.6.
14. **Rate limits:**
    - per-IP limits for login, sign-up, reset and invite preview, with the thresholds in conventions.md §10.1
    - `TRUSTED_PROXIES` CIDRs, required outside DEV_MODE (`none` for a server without a proxy); IPv6 keyed by /64
16. **Email addresses:** only verified addresses are unique and usable for login and reset; register and email change never reveal whether an address is taken; tokens travel in the link fragment and die on the events listed in conventions.md §5.5.
17. **Removed members** cannot rejoin with an invite created before their removal; a new invite works. → endpoint-map §3.
15. **Notification prefs:** global per user. Per-club mutes are revisited in Phase 4.

### catalog

1. **Work ids:** a numeric work id, with routes at `/book/{id}`. `{work_id}` accepts a numeric id, `OL…W` or a legacy key. `legacy_work_keys` is permanent.
2. **Club picks on search and detail:** `club_picks: ClubRef[]`, the viewer's clubs whose current pick is this work.
3. **Ratings and readers:** one audience-wide value (the viewer plus their audience, `only_me` excluded). The field names `club_rating`/`rating_count` stay.
4. **Search hits:** every remote hit is persisted (insert-only stub).
5. **User-added works** are searchable by everyone at once. Legacy `BC` rows have `created_by` NULL.
6. **Detail DTOs:** merged into `/works/{id}`. `/work/{id}` and its refresh are dropped; `/isbn` stays and persists its hits.
7. **Open Library data:** descriptions, subjects, rating and rating count are stored (OL publishes its data as CC0). **[Rafael]** Confirm descriptions legally: some are publisher blurbs.
8. **Refresh:** any signed-in user, 10 per hour, Open Library works only. Field-level provenance (conventions.md §7.2): it never overwrites what a reader entered or an admin approved, including covers; fields absent upstream keep their value.
9. **Unknown works:** writes need an existing work id; no client stubs, never an overwrite.
10. **Local ranking:**
    - `ts_rank` plus trigram similarity; local hits first on page 1
    - `sort` applies to remote results (and `title` sorts local ones too)
    - dedupe by work id, ISBN and OL id
    - a DNB-only work and an OL hit without an ISBN may both appear, until a merge request is approved (`POST /api/books/works/{work_id}/merge-requests`)
    - edition titles are searchable through `works.search_extra`
11. **Python quirks:**
    - the `limit + 1` merge is fixed
    - an OL 422 on a work becomes 502
    - a failing fallback no longer fails the whole search
    - half-to-even rounding is kept (cheap)
12. **Enrichment:** only once, on the first detail view of an OL stub (and one Open Library cover check by ISBN for a DNB work). Covers of persisted hits are prefetched in the background (MED); there is no other background enrichment.
17. **ISBN routing:** 978-3 → DNB SRU → Open Library; other groups → Open Library only. Adding a book or edition with an ISBN asks the sources first.
13. **Saturated limiter:** wait up to 2 s, then 429 `The library is busy. Wait a few seconds and try again.`
14. **Subject browse:** keep OL `lang=en`, and add a DNB new-releases row for German users (HAR).
15. **Detail strings:** keep the English strings; #48 may map them (optional).
16. **Coalescing:** a follower that retries after its owner was cancelled is acceptable (moka).

### google

1. **[Rafael]** Count the production rows by key class and the non-null `cover_image_url` values on a copy of the database:

   ```sql
   SELECT CASE WHEN ol_work_key GLOB '/works/OL*W' THEN 'OL'
               WHEN ol_work_key GLOB '/works/BC*' THEN 'BC'
               WHEN ol_work_key GLOB '/works/ISBN*' THEN 'ISBN'
               WHEN ol_work_key GLOB '/works/GB*' THEN 'GB'
               ELSE 'OTHER' END AS k,
          count(*), sum(cover_image_url IS NOT NULL)
   FROM books GROUP BY k;
   ```

   The importer handles every class either way.
2. **Legacy ISBN editions:** shelf entries and quotes of an ISBN-keyed legacy book get the edition that ISBN resolved to (the legacy row identified that edition), so page counts and covers stay as users saw them. → legacy-mapping.md §3.3.
3. **GB rows:** no fuzzy matching; bare facts; the volume id is stored only as a hash.
4. **Dry run:** does read-only lookups; `--offline` skips them.
5. **Merges:** legacy-mapping.md §3.4.
6. **The legacy key map** is permanent.
7. **Refresh** stays (rate-limited). Its error for user-added works is `This book was added by a reader, not Open Library.`
8. **Retries:** one retry on 502/503/504 after 300 ms. A 429 opens a per-host cooldown (`Retry-After`, or 60 s). DNB SRU has its own 1 req/s throttle.
9. **Goodreads when throttled:** the row is skipped with code `catalog_unavailable`, and the job continues.
10. **Spine colour:** reset where a Google cover was dropped.
11. **Unmatched rows:** the year and pages go on the edition and on the work built from it, so `BookOut` shows what the legacy app showed. `rematch-legacy` upgrades them later. → legacy-mapping.md §3.2–§3.3.

### shelf

1. **Rating visibility** is per entry and also hides `take` and the `shelf_finished` payload.
2. **Favourites** have the same audience as the shelf.
3. **Timezone:** `users.timezone` (default UTC, set from the browser at sign-up and by the importer from `--timezone`). `ShelfListOut.timezone` is the owner's zone.
4. **Adding to the shelf:** `POST /api/shelf {work_id, edition_id?, …}`. Remote hits are already ingested at search time.
5. **Goodreads columns:** the CSV ISBN sets `edition_id` when it resolves. `My Rating` and `Date Read` are imported (a reasoned improvement). Custom shelves stay `Unknown shelf`.
6. **Goodreads execution:** a background job with progress.
7. **Spine colour:** computed at cover ingest and stored in `media.dominant_color`; the endpoint is kept as a backfill.
8. **Shelf events:** stored once (`club_id` NULL) and filtered at read time. A new member's older events are hidden (only events from `joined_at` on).
9. **Editing another user's layout:** `PUT /api/shelf/physical-layout?username=<other>` → 404.
10. **Importer merges:** legacy-mapping.md §3.4.
11. **Types:** `status` is text with a CHECK (it sorts alphabetically); ids are bigint identity values.
12. **Wording:** keep `/api/auth/me/favorites`. `That book is not in the catalog` replaces the old string; `No member with that name` is kept.

### diary

1. **Pick discussion:** club posts and diary entries share one `posts` table with two scopes. Import: only real pick notes become club posts: a legacy `book_posts` root with `legacy_post_id` (the mirror copy of a pick note) and genuinely new unmirrored `club_pick_posts`, with their replies. Every other root stays a personal diary entry, even if the legacy app stamped the current pick on it, so a member who leaves keeps their diary. → legacy-mapping.md §9.
2. **Replies:** anyone who can see the root may reply. A reply has no visibility of its own. It is shown only to viewers who can see its author (and always to the root's author).
3. **Reaction users:** the list is filtered to the viewer's audience, club members and the viewer; `count` includes everyone.
4. **`pick_id`:** survives only as the pick of a club post.
5. **Feeds:** both. `/api/diary` is the combined feed; `?club=` filters to one club.
6. **Quotes** get an optional `edition_id` (default: the user's shelf edition).
7. **Personal events** are stored once and shown in club feeds (current members, from `joined_at`) and in the home feed. `quote_added` is hidden from others when the quote is `only_me` or gone.
8. **Stats pages:** the edition's pages, else `works.pages`, else 0.
9. **Milestones:** admins and the owner only; 403 `Only club admins can do that`.
10. **Milestone shape:** title-only milestones stay (`unit` NULL). PATCH becomes partial, with the backwards-range check on edit too; the closed-pick check stays create-only.
11. **Timezone for labels:** the club's zone for club labels, the viewer's zone for personal ones. The labels are kept as the frontend fallback.
12. **Stats endpoints:** `/api/stats?username=` (personal) and `/api/clubs/{slug}/stats` (club; readers = club members). Year boundaries stay UTC.
13. **Unknown work id:** 404.
14. **Deleted accounts:** entries with replies become tombstones, the rest are deleted, and the user's reactions are deleted.
15. **Phase 1 UI:** quotes yes; stats and milestones are optional (their backends are ported).

### club

1. **Vote roles:** members nominate and cast. Admins set the deadline, apply the winner, and set or clear the pick and the meeting.
2. **Moderation:** admins of the post's own club may delete (not edit) other members' club posts. The 403 strings are in endpoint-map.md §11.
3. **"Events"** means the activity log. The meeting stays one optional `meeting_at` per pick, edited with `PATCH /api/clubs/{slug}/pick`.
4. **Timezone:** per club and per user. Home-feed labels use the item's club zone for club items and the viewer's zone for personal ones.
5. **Activity storage:** a nullable `club_id`, joined at read time. Legacy `quote_added` events get a `quote_id` at import. `shelf_finished` is redacted by the current entry's visibility.
6. **New members:** their older personal activity is not shown (from `joined_at` on).
7. **Milestone model:** `unit` (percent, chapter or NULL), `range_from`, `range_to`; a NULL `range_from` means "up to `range_to`". The title stays required.
8. **Legacy pick posts:** see diary 1. `pick_id` survives on club posts.
9. **Auto-close setter:** credit the winning nominator while their account exists, even if they left the club. For a deleted account, the actor and `set_by` are NULL.
10. **Suggestions:** subjects and `ol_rating` are kept on works, so both reasons survive.
11. **Reader redaction:** `only_me` hides the rating and take; `dnf_reason` and `progress` stay visible.
12. **Book DTO:** `BookOut` (endpoint-map.md §0).
13. **Importer collisions:** nominations merge (the earliest one is kept and the ballots move to it). Two picks of the same work are fine.
14. **Endpoints with no UI:** all backend endpoints are ported in Phase 1. #48 builds the required pages; the rest are optional.
15. **Auto-close and reminders:** lazy on read, under row locks (parity). The reminder becomes a Phase 4 job.

### platform

1. **SPA serving:** axum serves it (`STATIC_DIR` = `frontend/dist`), with immutable caching on `/assets`.
2. **Legacy `book_posts` with `pick_id`:** see diary 1.
3. **Ids:** legacy ids are preserved (the list is in legacy-mapping.md §0); work ids are not.
4. **Phase 4 fields:** `notify_*` and `reminder_sent_at` are imported and kept; push subscriptions are dropped.
5. **Event scope:** club kinds carry `club_id`, personal kinds have NULL. Historical events are imported.
6. **Error contract:**
   - pydantic's generic messages are kept for the common cases
   - a 500 becomes JSON
   - auth runs before body validation (a reasoned change)
7. **OpenAPI** is public.
8. **`/api/config`:** see auth 12. The manifest becomes a static file.
9. **Unused legacy invites** are imported revoked.
10. **The #38 fixture** is committed as SQL text dumps plus the generator script; CI needs no Python.
11. **`test_api_error_copy.py`:** its Rust twin (with a temporary allow-list) lands in W1; #48 deletes the Python test.
12. **Phase 4:** a scheduled reminder job, and per-club mutes are decided then.

### frontend

1. **Club home** shows the pick's club thread with the diary components. A reply belongs to its author and inherits the root's scope. Legacy rows map as in diary 1.
2. **Home feed:** club cards (`/api/clubs`), the combined activity feed (`/api/activity`) and the combined diary (`/api/diary`).
3. **"Events"** is the activity log.
4. **Club addressing:** by slug. Slugs are immutable, so no redirects are needed.
5. **Missing role:** 403 with mapped strings. Members can nominate and vote.
6. **Other users' pages:** `/u/:username`. A stranger gets a 404 identical to an unknown name.
7. **Work ids:** numeric. Legacy links are resolved by the works endpoint, then `router.replace`, keeping the hash.
8. **Editions:** an edition picker is optional. The shelf and the physical shelf use the edition's pages and cover when the entry has an edition.
9. **Covers:** a "Report cover" sheet is in #48. Photo upload is optional (it must exist before the Phase 2 launch).
10. **Timezones:** a personal timezone preference (from the browser at sign-up, editable in settings). A new club defaults to its creator's zone.
11. **Before verification** the user is logged in immediately; only club creation needs a verified email.
12. **CSRF:** no token. `SameSite=Lax`, JSON-only bodies and the server-side origin check cover it.
13. **`/api/config`** stays, as service branding.
14. **Search history** uses casefold (the Rust `caseless` crate).
15. **Book page rating:** audience-wide.
16. **Discover:** trending and the subject rows stay (live OL, cached, hits persisted), and a DNB new-releases row is added.
17. **Built SPA:** `frontend/dist`, served by axum. The dev proxy targets port 8080.
18. **Goodreads skips:** `code` is added to each skip.
19. **Feed:** other users' `only_me` entries are never returned (enforced server-side). There is no spoiler-sensitivity setting.
20. **Meeting edit:** a dedicated `PATCH /api/clubs/{slug}/pick`, admin only.

### Needs Rafael

- **Transactional email provider** (EU-hosted, with a DPA). The server speaks SMTP through lettre, so any provider fits. Record the choice in #39.
- **Production DB facts:** the Google row counts (query above), and a copy of the database for #38's dry and real runs.
- **Open Library descriptions:** confirm we store them under the CC0-only rule.
- **Optional overrides of defaults this design set:**
  - `SIGNUP_MODE=invite` until #55
  - club posts are deleted on account deletion
  - Goodreads ratings and dates are imported
  - user-added works are searchable by everyone
  - a removed member cannot rejoin with an invite created before the removal (`club_removals`)
  - legacy diary entries written on the current pick stay personal diary entries; only real pick notes become club posts
  - imported shelf entries of ISBN-keyed books get the edition that ISBN names
