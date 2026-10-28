# Port spec: platform (app assembly, errors, SPA, DB init and migrations, models, Web Push, backup, branding, seed_demo)

**Sources read in full:**
- `backend/app/{main,db,models,push,i18n,backup,branding,seed_demo}.py`
- `backend/app/routers/{push,backup}.py`
- `backend/tests/conftest.py`
- `backend/tests/{test_push,test_backup,test_migrations,test_spa_cache,test_api_error_copy}.py`

**Also read for context:**
- `backend/app/{config,runtime,http,deps,security,timezone,themes,healthcheck,create_invite,activity,works,pick_ops}.py`
- The push call sites in `routers/{pick,vote,auth}.py`.
- `frontend/src/{i18n/apiErrors.ts,api/client.ts,brand.ts,stores/club.ts,shelf.ts,i18n/dates.ts}`, `frontend/index.html`, `frontend/public/manifest.webmanifest`, `frontend/vite.config.ts`.
- The old `frontend/public/sw.js` from commit `3d0b27d`.
- The Rust scaffold: `server/src/*.rs`, `server/tests/health.rs`, `server/Cargo.toml`, `server/compose.yaml`, `server/Dockerfile`, `.forgejo/workflows/ci.yml`.

**Probed behaviour.** Anything marked **(probed)** was confirmed by running the Python app (FastAPI 0.162.0, Starlette 1.10.0, pydantic 2.14.1, SQLModel 0.0.48, SQLAlchemy 2.1.6) against scratch databases under the session scratchpad. Nothing in the repo was touched. The probe scripts were throwaway session files and are not in the repo.

**Overlap with sibling specs.** `spec/auth.md` already covers these in depth; this document summarises them and does not contradict it:
- `GET /api/health`, `GET /api/config`, `GET /manifest.webmanifest` and the debug docs
- settings and `prepare_environment`
- the session cookie and the middleware stack
- `ensure_bootstrap_invite`, `create_invite` and `healthcheck`
- `test_selfhost.py`

`spec/shelf.md` and `spec/catalog.md` own the semantics of the shelf and catalog tables. Here those tables are described at the schema and migration level only.

---

## 1. Endpoints

### 1.0 Cross-cutting conventions (`main.py`: error handling)

The frontend depends on these for every endpoint, so they are part of the platform contract.

**Error body.**
- Every handled error is JSON `{"detail": "<English string>"}` with `Content-Type: application/json`.
- `HTTPException(status, detail)` passes the status and detail through unchanged.
- `frontend/src/api/client.ts` reads `data.detail`:
  - If it is a string, `translateApiError(detail)` maps the **exact** string through `API_ERROR_KEYS` in `frontend/src/i18n/apiErrors.ts`. Unmapped strings are shown verbatim.
  - If it is not a string, the generic `errors.generic` message is shown.

**Validation errors become 400, never 422** (`validation_handler`):
```python
messages = []
for error in exc.errors():               # pydantic order: body/query/path fields in declaration order
    msg = str(error.get("msg", "Invalid value"))
    if msg.startswith("Value error, "):  # only this prefix is stripped ("Assertion failed, " is not)
        msg = msg[len("Value error, "):]
    messages.append(msg)
return JSONResponse(status_code=400, content={"detail": "; ".join(messages)})
```
- The `loc` (field name) is **not** included, so messages carry no field name unless the validator text names it.
- Several errors are joined with `"; "`.

**Probed examples of the flattened detail:**

| Request | Status | `detail` |
|---|---|---|
| `POST /api/push/subscriptions` `{"endpoint":"http://insecure","keys":{...}}` | 400 | `Push endpoint must be an https URL` |
| same, with `keys` also missing | 400 | `Push endpoint must be an https URL; Field required` |
| `{}` | 400 | `Field required; Field required` |
| no body at all | 400 | `Field required` |
| `Content-Type: application/json` with an empty body | 400 | `Field required` |
| `{"endpoint":"https://x/z","keys":{}}` | 400 | `Field required; Field required` |
| `keys: {"p256dh": 1, "auth": null}` | 400 | `Input should be a valid string; Input should be a valid string` |
| `user_agent: null` | 400 | `Input should be a valid string` |
| body `[1]` (JSON array) | 400 | `Input should be a valid dictionary or object to extract fields from` |
| valid JSON sent as `Content-Type: text/plain`, or with an empty `Content-Type` | 400 | `Input should be a valid dictionary or object to extract fields from` (the body is parsed as JSON **only** when the content type is JSON) |
| malformed JSON `{nope` | 400 | `JSON decode error` |
| `PATCH /api/auth/notifications {"notify_pick":"maybe"}` | 400 | `Input should be a valid boolean, unable to interpret input` |
| `GET /api/books/search?limit=abc` (query) or `GET /api/pick/abc/posts` (path) | 400 | `Input should be a valid integer, unable to parse string as an integer` |
| `PUT /api/pick` with `meeting_at: "nope"` (a handler-raised `HTTPException`, not validation) | 400 | `meeting_at must be a date and time` |

**Order of checks on an authenticated JSON endpoint (probed):**
1. JSON decode. A malformed body returns 400 `JSON decode error` **even when signed out**.
2. Auth (`deps.get_current_user`). Returns 401 `Not signed in`. A signed-out request with well-formed but invalid JSON gets **401**, not 400.
3. Field validation: 400.
4. The handler.

**Unknown JSON keys** are ignored (pydantic `extra="ignore"`).

**Unhandled exceptions (probed):**
- 500 with `Content-Type: text/plain; charset=utf-8` and body `Internal Server Error`. This is Starlette's `ServerErrorMiddleware`. FastAPI is not created with `debug=True`, so no traceback is sent.
- **The frontend's `request()` then throws a `SyntaxError`** from `JSON.parse("Internal Server Error")` instead of an `ApiError`. This is a latent frontend bug (see §6).

**Not-found and method errors (probed).** These depend on whether the SPA catch-all is mounted (§1.6):

| Request | With `STATIC_DIR` present (production image) | Without `STATIC_DIR` (dev, tests) |
|---|---|---|
| `GET /api/nope` | 404 `{"detail":"Not found"}` (lower-case `f`, from the catch-all) | 404 `{"detail":"Not Found"}` (framework) |
| `GET /api/` | 404 `{"detail":"Not found"}` | 404 `{"detail":"Not Found"}` |
| `GET /api` (no slash) | **200, SPA shell** (`"api"` does not start with `"api/"`) | 404 `{"detail":"Not Found"}` |
| `GET /apix` | 200, SPA shell | 404 `Not Found` |
| `GET /api/push/test` (a POST-only route) | 404 `{"detail":"Not found"}`: the catch-all is a full match for GET | 405 `{"detail":"Method Not Allowed"}`, `Allow: POST` |
| `POST /api/nope` | **405** `{"detail":"Method Not Allowed"}`, `Allow: GET` (the catch-all path matched, the method did not) | 404 `{"detail":"Not Found"}` |
| `POST /discover` | 405, `Allow: GET` | 404 |
| `HEAD /` | **405** `Allow: GET`. FastAPI GET routes do not answer HEAD | 404 |
| `OPTIONS /` (not a CORS preflight) | 405 `{"detail":"Method Not Allowed"}` | 404 |

**Datetime serialization in responses.**
- SQLite returns naive datetimes, so most response timestamps have **no offset**, e.g. `"2026-10-27T11:00:28.235126"` (probed on `POST /api/push/subscriptions` and `GET /api/push/subscriptions`).
- The values are UTC wall-clock.
- Values built in memory and never reloaded serialize **with** `+00:00`, for example the `meeting_at` inside a `pick_set` event payload, which comes from `isoformat()`.
- Frontend behaviour:
  - `shelf.ts: clubDate()` appends `Z` when no offset is present.
  - `i18n/dates.ts: formatClubDate()` and `constants.ts: relativeDay()` call `new Date(iso)` directly, so offset-less values are read as **device-local time**. That is an existing bug, and a port that always emits `Z` fixes it (§6).

### 1.1 App assembly: `create_app()` (`main.py`)

This is not an endpoint, but the steps run in this order. Module import also runs `app = create_app()` (`uvicorn app.main:app`), and each test calls `create_app()` again.

1. `prepare_environment()`: in production, generates or validates `SECRET_KEY` and the bootstrap invite under the data dir. See auth.md §2.8.
2. `get_settings.cache_clear()`, then `settings = get_settings()`. This reads the env plus the dotenv file.
3. `club_name = sanitize_name(settings.bookclub_name)`.
4. `warn_invalid_theme(settings)` logs a warning for an invalid `BOOKCLUB_THEME` or `BOOKCLUB_THEME_DARK`.
5. Logs `catalog: Google Books` if `GOOGLE_BOOKS_API_KEY.strip()` is non-empty, otherwise `catalog: Open Library`.
6. `engine = init_db(settings.database_path)` (§2.2).
7. `ensure_bootstrap_invite(session, settings)`: inserts the bootstrap invite only when `invites` has zero rows (auth.md §1.23).
8. `FastAPI(title=club_name, docs_url="/api/docs" if debug else None, redoc_url=None, openapi_url="/api/openapi.json" if debug else None)`.
9. Sets `app.state.engine` and `app.state.settings`. Handlers reach the engine through `request.app.state.engine`.
10. Middleware, outermost first, as executed (auth.md §2.9):
    1. `CORSMiddleware`, only if `cors_origins()` is non-empty.
    2. `ProxyHeadersMiddleware(trusted_hosts=...)`.
    3. `AutoSecureCookieMiddleware`, only when `https_mode(BOOKCLUB_HTTPS) == "auto"`.
    4. `SessionMiddleware`: cookie `bookclub_session`, `max_age` 30 days, `same_site="lax"`, `https_only` only when `always`.
    5. The routes.
11. The `RequestValidationError` handler (§1.0).
12. Inline routes `GET /api/health` and `GET /api/config`.
13. Routers in this order: auth, invites, books, diary, shelf, search_history, members, pick, overlap, vote, **backup**, activity, stats, milestones, quotes, **push**.
14. `GET /manifest.webmanifest`. It is registered before the SPA catch-all, so it **shadows** the static `manifest.webmanifest` that Vite copies from `frontend/public/` (probed: the dynamic JSON is returned).
15. Only if `STATIC_DIR` (`backend/app/static`) is a directory **when `create_app()` runs**:
    - mount `StaticFiles` at `/assets`, but only if `static/assets` exists at that moment;
    - add the GET catch-all `/{full_path:path}` (§1.6).

### 1.2 `GET /api/health` (summary; full text in auth.md §1.19)

- **Auth:** none.
- **Logic:** runs `SELECT 1` in a new session. No timeout.
- **Responses:**
  - 200 `{"ok": true}`.
  - On any exception: 503 `{"detail": "database unavailable"}`, logged as `health check failed` with the traceback.
- **Rust scaffold:** already returns 200 `{"status":"ok"}` or 503 `{"status":"unavailable"}` with a 2 s timeout. This is a deliberate change. The Python Docker healthcheck (`app.healthcheck`, which expects the bytes `"ok"`) does not carry over.

### 1.3 `GET /api/config` (summary; full text in auth.md §1.20)

- **Auth:** none.
- **Response:** 200 `{"name": str, "theme": str, "theme_dark": str, "public_url": str, "timezone": str}`.
- Example (probed, with `BOOKCLUB_NAME="Thursday <Readers>"` and `BOOKCLUB_THEME="#336699"`): `{"name":"Thursday Readers","theme":"#336699","theme_dark":"#274f77","public_url":"","timezone":"UTC"}`.
- Reads `get_settings()` on every request. The value is cached, so it is effectively per process.
- **Frontend consumers:** `stores/club.ts` loads it without blocking and falls back to `{name:"Bookclub", theme:"#b44a2a", theme_dark:"#8e361c", public_url:"", timezone:"UTC"}` on error.
  - `brand.ts: applyBrand` sets `document.title` and the `apple-mobile-web-app-title` meta.
  - It also **replaces the manifest link with a Blob URL** built on the client.
  - `applyClubAccent` sets `--accent` and `--accent-dark` on the Paper theme only.
  - `timezone` is used by the club and diary date formatting.

### 1.4 `GET /manifest.webmanifest` (summary; full text in auth.md §1.21)

- **Auth:** none.
- **Response:** `Content-Type: application/manifest+json`, body `json.dumps(manifest_payload(settings))`.
- `settings` is the object captured at `create_app` time.
- Fields: name, short_name (`name[:32]`), description, start_url, scope, display, orientation, background_color, theme_color, and the 3 icons.

### 1.5 `GET /api/docs`, `GET /api/openapi.json`

- Only when `settings.debug` is true; otherwise 404 (`Not Found`, or `Not found` when the catch-all is mounted).
- ReDoc is never mounted.

### 1.6 SPA and static files (`main.py`, `test_spa_cache.py`)

These exist only when `STATIC_DIR` exists at startup. `npm run build` writes there (`vite.config.ts: build.outDir = "../backend/app/static"`, `emptyOutDir: true`).

**`GET /assets/{path}`** is a Starlette `StaticFiles(directory=static/assets)` mount.
- **Auth:** none.
- **Hit:** 200 with the file and a guessed `Content-Type` (`text/javascript; charset=utf-8` for `.js`), plus `ETag` (quoted md5-style) and `Last-Modified`.
- **No `Cache-Control` header (probed).** Hashed assets are therefore cached heuristically, not immutably.
- **Miss** (e.g. `/assets/missing.js`): 404 `{"detail":"Not Found"}` as JSON (probed). The SPA shell is **never** served for a missing asset, so a stale shell's missing chunk fails loudly.
- `/assets/` (a directory) returns 404 `Not Found`. `/assets` without the slash does not match the mount and falls through to the catch-all, which serves the shell.

**`GET /{full_path:path}`** is the catch-all. It is GET only, with no HEAD (probed 405).
- **Auth:** none.
- **Algorithm, in order:**
  1. If `full_path.startswith("api/")`: 404 `{"detail": "Not found"}`.
  2. If `full_path in {"", "index.html"}` and `static/index.html` is a file: serve `app_html(index)`.
  3. `candidate = (STATIC_DIR / full_path).resolve()`. If it is an existing **file** and `STATIC_DIR` is among its parents (path-traversal guard; symlinks are resolved first):
     - if `candidate.name == "index.html"`, serve `app_html(candidate)`;
     - otherwise `FileResponse(candidate)`: guessed type, `ETag`, `Last-Modified`, **no Cache-Control**.
  4. Otherwise, if `static/index.html` exists: serve `app_html(index)`. This is the SPA fallback for every client route.
  5. Otherwise 404 `{"detail": "Not found"}`.
- **`app_html(index)`:**
  - Reads the file **on every request**, so a static swap needs no restart.
  - Applies `brand_index_html(text, settings)` (§2.5).
  - Returns `HTMLResponse(..., headers={"Cache-Control": "no-store"})` with `Content-Type: text/html; charset=utf-8`.
- **Probed outcomes:**

| Path | Result |
|---|---|
| `/`, `/index.html`, `/discover`, `/book/ISBN9781408857892` | 200 branded shell, `cache-control: no-store` |
| `/nested/index.html` (file exists) | 200 that file, branded, `no-store` |
| `/nested` (a directory) | 200 root shell |
| `/icons/icon-192.png`, `/favicon.svg` | 200 file, ETag, Last-Modified, no Cache-Control |
| `/../../etc/passwd`, `/..%2f..%2fetc%2fpasswd` | 200 root shell; the traversal is neutralised |

### 1.7 `GET /api/push/vapid`

- **Auth:** session required; otherwise 401 `Not signed in` (probed).
- **Request:** none.
- **Response:** 200 `{"public_key": str}`.
  - The key is the VAPID public key as an **uncompressed P-256 point** (65 bytes, first byte `0x04`), base64url-encoded **without padding**, so exactly **87 characters**.
  - Probed example: `BIHYHcBPVChW2EbZUjnzStna5pHAGHoejEbFRv4qlrRvOKViNf00rRLUXghii89_IfsEOOz0Q4O3-YdWVqzuK7A`.
- **Side effects:** the first call in a process may **generate and persist** the key pair (§2.6.1). It is stable across calls and across `reset_keys()`, because it is re-read from the file.

### 1.8 `GET /api/push/subscriptions`

- **Auth:** session required.
- **Response:** 200 `{"items": [SubscriptionOut, ...]}`.
  - Only the caller's rows (`user_id == me.id`), ordered by `id` ascending.
  - **`SubscriptionOut`:**
    ```json
    {"id": int, "endpoint_tail": str, "user_agent": str, "created_at": "naive ISO datetime, no offset"}
    ```
  - `endpoint_tail = endpoint[-12:]`: the last 12 **characters**, or the whole endpoint if shorter.
- No pagination.

### 1.9 `POST /api/push/subscriptions`

- **Auth:** session required.
- **Body `SubscribeIn`:**
  - `endpoint: str`, required. The validator `strip()`s it, then requires it to **start with `https://`** (case-sensitive: `HTTPS://` is rejected, probed) and `len <= 1024` after stripping. Otherwise `ValueError("Push endpoint must be an https URL")`, which becomes 400 detail `Push endpoint must be an https URL`.
    - Probed: `"https://" + 1016×"a"` (1024 chars) gives 201; 1025 chars gives 400.
  - `keys: {"p256dh": str, "auth": str}`, both required, any string including `""`. **No length validation**, although the DB columns are `VARCHAR(256)` and `VARCHAR(128)`; SQLite does not enforce them.
  - `user_agent: str = ""`. Optional, must be a string (`null` gives 400 `Input should be a valid string`), and is **silently truncated** to 256 characters.
- **Logic:**
  - Look up a row by `endpoint` across **all users**, since `endpoint` is globally unique.
  - **If it exists,** it is *taken over*: `user_id = me.id`, `p256dh`, `auth` and `user_agent` are updated. `id` and `created_at` stay unchanged. The rationale is a device re-subscribing, or another account on the same browser.
  - **Otherwise** a new row is inserted with `created_at = utcnow()`.
- **Response:** **201 in both cases**, with a `SubscriptionOut` of the row. The row is refreshed from the DB, so `created_at` is naive.
- **Errors:**
  - 401 `Not signed in`.
  - 400 validation, as in §1.0.

### 1.10 `DELETE /api/push/subscriptions`

- **Auth:** session required.
- **Body `UnsubscribeIn`:** `{"endpoint": str}`, required. A DELETE with no body gives 400 `Field required` (probed). There is no https check here.
- **Logic:**
  - `endpoint.strip()` must match a row **owned by the caller**.
  - A row with that endpoint belonging to another user counts as not found.
- **Responses:**
  - 204, empty body.
  - 404 `No such subscription`.
  - 401 `Not signed in`.

### 1.11 `POST /api/push/test`

- **Auth:** session required.
- **Body:** none. Any body is ignored.
- **Logic:**
  - Load all of the caller's subscriptions.
  - If there are none: **400 `This device is not subscribed yet`**. The message says "this device", but the check is per account.
  - Otherwise build targets from the caller's rows: `{id, endpoint, p256dh, auth}`, with no locale.
  - Build the copy with `push_copy("test", me.locale, club=sanitize_name(BOOKCLUB_NAME))`.
  - Queue a background `push.deliver(engine, targets, payload)` with payload `{"title", "body", "url": "/settings", "tag": "test", "kind": "test"}`.
  - **Ignores `notify_*` prefs.** Only the caller's own devices are targeted.
- **Response:** 200 `{"sent_to": <number of the caller's subscriptions>}`.
  - The count is computed before delivery, so failures do not reduce it.
  - Probed: `{"sent_to":2}`, payload title `Bookclub: notifications are on`.
- **Side effects:**
  - Outbound Web Push requests after the response.
  - Subscriptions answering 404 or 410 are deleted.

### 1.12 Push side effects on endpoints owned by other areas

The endpoints are specified elsewhere. Their push behaviour belongs here. Every push goes through `push.schedule()` (§2.6.3) unless noted.

| Trigger | Condition | kind | tag | url | Recipients | Copy fields |
|---|---|---|---|---|---|---|
| `PUT /api/pick` | only when the current pick's id **after** differs from the id **before** (a first pick, or a different book; re-PUT of the same book just updates note/meeting and does **not** notify) | `pick` | `pick-{pick.id}` | `/` | all users with `notify_pick`, **except the actor** | `club` = sanitized name, `actor` = username, `title` = book title |
| `POST /api/vote/apply` | always after a successful apply | `pick` | `pick-{pick.id}` | `/` | `notify_pick`, except the applier | as above |
| `GET /api/vote` → `maybe_auto_close` | the deadline passed, a leader exists and the vote closes | `pick` | `pick-{id}` | `/` | `notify_pick`, except the **winning nominator** (recorded as actor and pick setter) | as above |
| `POST /api/vote/nominations`, `POST /api/vote/cast` → `maybe_auto_close(session)` | auto-close also runs here, but **no push** is sent (no `engine` is passed) | none | | | | |
| `POST /api/pick/posts`, `POST /api/pick/{pick_id}/posts` (legacy pick notes) | after the post is saved, if the pick has a book | `note` | `note-{pick.id}` | `/` | `notify_note`, except the author | `actor`, `title`, `excerpt` (below) |
| `GET /api/pick` → `maybe_schedule_meeting_reminder` | the current pick has `meeting_at`, `reminder_sent_at IS NULL`, a book, and `now < meeting_at <= now + 24h` | `meeting` | `meeting-{pick.id}` | `/` | `notify_meeting`, **nobody excluded** | `club`, `title` |
| `POST /api/push/test` | §1.11 | `test` | `test` | `/settings` | the caller only | `club` |

- **Excerpt rule:**
  - `excerpt = body if len(body) <= 90 else body[:87] + "…"`. The result is at most 88 characters and ends with U+2026.
  - `body` is the validated value (`_post_body_ok` strips it).
- **Diary entries (`book_posts`, `/api/books/works/{id}/posts`) never push.** The frontend uses only the diary, so note pushes cannot fire from the current UI.
- **The meeting reminder is one-shot:**
  - `reminder_sent_at = now` is committed **before** targets are collected. It is set even when nobody is subscribed or everyone opted out, so it is never retried.
  - Re-scheduling the meeting on the same pick does **not** reset `reminder_sent_at`, so a moved meeting gets no new reminder.
  - Only `GET /api/pick` triggers it; history and other reads do not.

### 1.13 `GET /api/backup`

- **Auth:** session required. **Any member** may download; there is no role check.
- **Request:** none.
- **Logic:**
  - If `settings.database_path` is not a file: **404 `No database file to export`**.
  - Otherwise create a temp file `bookclub-backup-*.db` (`delete=False`) and run the SQLite online backup from a **read-only** URI connection (`file:{path}?mode=ro`) into it.
- **Response:** 200 `FileResponse`.
  - `Content-Type: application/vnd.sqlite3`.
  - `Content-Disposition: attachment; filename="bookclub.db"` (probed exact).
  - The body starts with `SQLite format 3\x00`.
  - `Content-Length` is set; the probe DB was 258048 bytes.
- **Side effects:**
  - The temp file is unlinked after the response is sent (a `BackgroundTask`, `missing_ok=True`).
  - No rate limit and no size cap.
- **What it leaks:**
  - The whole instance: every user's **argon2 password hash**, avatar blobs, push subscription keys (`p256dh`/`auth`), search history and everything else.
  - The hosted service must not reproduce this (§5.7).
- **Frontend:** `SettingsPage.vue` has a plain `<a href="/api/backup">` labelled with the `settings.backup`, `settings.backupBlurb` and `settings.downloadDatabase` i18n keys.

### 1.14 Non-HTTP entry points in this area

- **`python -m app.backup [dest]`:**
  - Runs `prepare_environment()` and `get_settings()`.
  - `dest` is `argv[1]` (with `~` expanded), or `<db dir>/bookclub-YYYYMMDDTHHMMSSZ.db` (UTC).
  - `backup_database(source, dest)` mkdirs `dest.parent`. It raises `FileNotFoundError(f"Database not found: {source}")` if the source is missing, then does an online backup from a read-only connection.
  - Prints the destination path. Covered by `test_selfhost.py::test_backup_copies_sqlite` (auth.md).
- **`python -m app.seed_demo`:** see §2.9.
- `create_invite` and `healthcheck`: see auth.md §1.23.

---

## 2. Domain rules

### 2.1 Startup is idempotent

Every boot runs all of these, and each is safe to repeat:
- `create_all` (creates missing tables only)
- `apply_migrations` (adds missing columns only)
- `import_legacy_pick_posts` (skips rows already mirrored)
- `ensure_bootstrap_invite` (acts only on an empty `invites` table)

### 2.2 DB init (`db.py: init_db(path)`)

1. `path.parent.mkdir(parents=True, exist_ok=True)`.
2. `create_engine("sqlite:///{path}", connect_args={"check_same_thread": False})`.
3. On every new DBAPI connection: `PRAGMA journal_mode=WAL` and `PRAGMA foreign_keys=ON`. **FKs are enforced**, and `ON DELETE` is `NO ACTION` unless stated in §3.
4. `SQLModel.metadata.create_all(engine)` creates **missing tables** with the full current model. Existing tables are never altered.
5. `apply_migrations(engine)`: in one transaction, for each `(table, columns)` in `COLUMN_MIGRATIONS`:
   - `PRAGMA table_info(table)`. If it returns no rows (the table doesn't exist), skip.
   - For each column not present, run `ALTER TABLE {table} ADD COLUMN {name} {ddl}`.
   - Dict order is irrelevant.
6. `import_legacy_pick_posts(engine)` (§2.3).
7. Returns the engine.

**`missing_columns(engine) -> dict[str, list[str]]`:**
- Lists the `COLUMN_MIGRATIONS` columns absent from existing tables. Absent tables are skipped.
- Used only by tests. It is useful for the importer as a pre-flight check (§6).

**`COLUMN_MIGRATIONS` (verbatim DDL):**

| table | column | DDL |
|---|---|---|
| users | theme | `VARCHAR(32) NOT NULL DEFAULT 'paper'` |
| users | color_mode | `VARCHAR(16) NOT NULL DEFAULT 'system'` |
| users | locale | `VARCHAR(8) NOT NULL DEFAULT 'en'` |
| users | notify_meeting | `BOOLEAN NOT NULL DEFAULT 1` |
| users | notify_pick | `BOOLEAN NOT NULL DEFAULT 1` |
| users | notify_note | `BOOLEAN NOT NULL DEFAULT 1` |
| users | avatar | `BLOB` |
| users | avatar_mime | `VARCHAR(32)` |
| users | avatar_updated_at | `DATETIME` |
| books | description | `VARCHAR NOT NULL DEFAULT ''` |
| books | pages | `INTEGER` |
| books | subjects | `VARCHAR NOT NULL DEFAULT '[]'` |
| books | ol_rating | `FLOAT` |
| books | details_fetched_at | `DATETIME` |
| books | cover_image_url | `VARCHAR` |
| books | spine_color | `VARCHAR` |
| shelf | shelf_position | `INTEGER` |
| shelf | rating | `INTEGER` |
| shelf | take | `VARCHAR DEFAULT ''` (**nullable**) |
| shelf | dnf_reason | `VARCHAR DEFAULT ''` (**nullable**) |
| shelf | progress | `INTEGER` |
| shelf | started_at | `DATETIME` |
| shelf | finished_at | `DATETIME` |
| club_picks | meeting_at | `DATETIME` |
| club_picks | reminder_sent_at | `DATETIME` |
| club_pick_posts | spoiler_upto | `INTEGER` |
| club_pick_posts | milestone_id | `INTEGER REFERENCES pick_milestones(id)` |
| club_pick_posts | updated_at | `DATETIME` |
| next_up_votes | closes_at | `DATETIME` |

**A migrated legacy DB and a freshly created DB differ.** The production DB has been migrated over time, so the importer must accept both shapes:

| column | fresh (`create_all`) | migrated (`ALTER`) |
|---|---|---|
| `users.theme` / `color_mode` / `locale` | `NOT NULL`, **no DB default** (the ORM fills it) | `NOT NULL DEFAULT 'paper'/'system'/'en'` |
| `users.notify_*` | `BOOLEAN NOT NULL`, no default | `NOT NULL DEFAULT 1` |
| `books.cover_image_url` | `VARCHAR(512)` | `VARCHAR` |
| `books.spine_color` | `VARCHAR(7)` | `VARCHAR` |
| `shelf.take` | `VARCHAR(140) NOT NULL` | `VARCHAR DEFAULT ''`, **nullable** |
| `shelf.dnf_reason` | `VARCHAR(200) NOT NULL` | `VARCHAR DEFAULT ''`, **nullable** |
| `club_pick_posts.milestone_id` | FK in the table constraint | FK in the column clause |
| column order | model order | legacy columns first, migrated columns appended |

Tables created after their first release by `create_all` alone never had columns added. They are absent from `COLUMN_MIGRATIONS`:
- `invites`
- `physical_shelf_layouts`
- `user_favorites`
- `pick_milestones`
- `post_reactions`
- `book_posts`
- `book_post_reactions`
- `events`
- `push_subscriptions`
- `quotes`
- `next_up_nominations`
- `next_up_ballots`
- `search_history`

### 2.3 Data migration: `import_legacy_pick_posts(engine) -> int`

This copies pre-diary club notes (`club_pick_posts`) into the per-book diary (`book_posts`). It runs **on every boot**.

1. `done` = the set of all non-NULL `book_posts.legacy_post_id`.
2. `pending` = `club_pick_posts JOIN club_picks ON club_picks.id = club_pick_posts.pick_id`, ordered by `club_pick_posts.created_at, club_pick_posts.id`.
3. For each `(legacy, pick)` whose `legacy.id` is not in `done`:
   - Insert `BookPost` with:
     - `book_id = pick.book_id`
     - `author_id = legacy.author_id`
     - `body = legacy.body`
     - `spoiler_upto = legacy.spoiler_upto`
     - `pick_id = pick.id`
     - `legacy_post_id = legacy.id`
     - `created_at = legacy.created_at`
     - `updated_at = legacy.updated_at`
     - `parent_id = NULL`, `progress_at = NULL`, `status_at = NULL`, `deleted_at = NULL`
   - `flush` to get the new id.
   - For each `post_reactions` row with `post_id = legacy.id`, insert `BookPostReaction(post_id=new.id, user_id, emoji, created_at)`.
   - Count it.
4. One `commit` at the end. Return the count.

**Quirks the port and the importer must know:**
- **`club_pick_posts.milestone_id` is not copied.** The milestone link exists only in the legacy table.
- **The legacy table is never emptied.** After the import, the same note exists in **both** tables.
  - The legacy `/api/pick/posts*` endpoints are still mounted and keep reading and writing `club_pick_posts`.
  - A note written there today appears in the diary only **after the next boot**.
  - Edits (`PATCH /api/pick/posts/{id}`) and deletes on a legacy row after mirroring are **not** propagated.
  - Reactions added to a legacy row after mirroring are **not** copied, because the reaction copy happens only once, at mirror time.
- **SQLite id reuse.** `club_pick_posts` has no `AUTOINCREMENT`. If the highest legacy row is deleted and a new one is created, it can get the same id, which is already in `done`. That row is then **never** mirrored.
- `test_diary.py::test_legacy_pick_notes_are_imported_once_with_reactions` (owned by the diary spec) asserts:
  - the first call returns 1 and the second 0;
  - the diary entry keeps `spoiler_upto` (30) and the author, and its reactions are `[{"emoji":"🔥","count":1,"mine":true,"users":["ada"]}]`;
  - `pick_id` and `legacy_post_id` are set.

### 2.4 Error-handling rules to keep in the port

- One error shape for every 4xx and 5xx: `{"detail": "<English string>"}`. The frontend's string mapping depends on it.
- Request validation failures return **400**, never 422.
- Several validation messages join with `"; "` in field order. Python's custom validator messages are the important strings (they are what the frontend shows). Pydantic's generic texts (`Field required` and so on) are only shown verbatim and are not mapped.
- `apiErrors.ts` (19 entries today) maps exact strings. A port must keep every mapped string byte-identical, or change `apiErrors.ts` and the i18n keys with it (§2.10, R10).
- **The current mapping contains no string from this area.** The push, backup, health and SPA strings are all unmapped.

### 2.5 Branding (`branding.py`)

The full rules are in auth.md §1.20, §1.21 and §2.10. What matters here:
- **`sanitize_name(value)`:**
  - strip;
  - drop characters where `not ch.isprintable()` or `ch in "<>"`;
  - collapse whitespace runs to single spaces;
  - truncate to **48 characters**;
  - fall back to `"Bookclub"` if empty.
  - Probed: `"Thursday <Readers>"` becomes `"Thursday Readers"`.
- **`sanitize_theme(value)`:**
  - must match `^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{6})$` after strip;
  - the 4-char form is lowercased;
  - the 7-char form keeps its case;
  - otherwise `""`.
- **`resolve_theme(theme, theme_dark)`:**
  - accent = sanitized theme or `#b44a2a`;
  - dark = sanitized dark, else `#8e361c` if `accent.lower() == "#b44a2a"`, else `darken_hex(accent, 0.78)`.
  - `darken_hex` expands `#abc` to `#aabbcc`, applies `int(channel * 0.78)` clamped to 0–255, and returns lowercase hex.
  - Probed: `#336699` gives `#274f77`.
- **`public_origin(value)`:** the `scheme://netloc` of an http(s) URL with a host, else `""`.
- **`cors_origins(settings)`:** in debug, `http://localhost:5173` and `http://127.0.0.1:5173`, then the public origin if set and not already listed.
- **`brand_index_html(html, settings)`** does three **exact-substring** replacements. The name is `html.escape(name, quote=True)`:
  - `<title>Bookclub</title>` becomes `<title>{name}</title>`.
  - `name="apple-mobile-web-app-title" content="Bookclub"` becomes `... content="{name}"`.
  - `name="theme-color" content="#b44a2a"` becomes `... content="{theme}"`.
    - **This is a no-op today.** `frontend/index.html` ships `<meta name="theme-color" content="#f7f4ec" />`, and `brand.ts: applyThemeColor()` sets the meta at runtime from the palette's `--bg`.
- **`manifest_payload(settings)`:**
  - `description` is `"A tiny private bookclub: one pick, everyone's shelves, a vote for what's next."`
  - `background_color` is `"#f6f1e8"`.
  - Icons, in order:
    1. `/icons/icon-192.png` 192x192 png `any`
    2. `/icons/icon-512.png` 512x512 png `any`
    3. `/icons/icon-512.png` 512x512 png `maskable`
  - The frontend overrides the manifest at runtime with a Blob URL (`brand.ts`), so the server manifest matters only for the first install prompt and for crawlers.
- **User-Agent strings:**
  - `open_library_ua`: `"{token}/1.0 ({contact}; catalog browse via Open Library)"`.
  - `google_books_ua`: `"{token}/1.0 ({contact}; catalog search via Google Books)"`. Dropped by the plan.
  - `token` is the sanitized name with `[^A-Za-z0-9._-]+` removed (or `Bookclub`). `contact` is the public origin, or `"private book club"`.

### 2.6 Web Push (`push.py`)

#### 2.6.1 Keys and subject

- **`private_key_pem()`** is cached per process with `lru_cache`.
  - If `VAPID_PRIVATE_KEY.strip()` is non-empty, return it with literal `\n` sequences replaced by newlines. This allows single-line env values.
  - Otherwise `read_or_create_file(data_dir_from_env() / ".vapid_private.pem", generate)`. The data dir is the parent of `DATABASE_PATH` from the **env**, not from settings.
    - `generate` is `py_vapid.Vapid().generate_keys()`, giving a P-256 private key as PEM, stripped. The file is written with a trailing newline and `chmod 600`.
    - Logs `Generated VAPID key pair for Web Push` when it was created.
    - The test asserts `(tmp_path / ".vapid_private.pem").is_file()`.
- **`reset_keys()`** clears the cache. Tests use it; the result stays stable because the file is re-read.
- **`public_key_b64url()`:** parse the PEM, export the public key as X9.62 **uncompressed point** (65 bytes), base64url, strip `=`. Always 87 characters.
- **`vapid_subject()`:**
  - `VAPID_SUBJECT.strip()` if non-empty;
  - else `BOOKCLUB_PUBLIC_URL` (already reduced to the origin);
  - else `"mailto:bookclub@localhost"`.
- `ensure_keys_at_startup` exists, but **nothing calls it**. Keys are generated lazily on the first `GET /api/push/vapid` or the first delivery.

#### 2.6.2 Delivery: `send_to_subscription(target, payload) -> True | False | None`

- Calls `pywebpush.webpush` with:
  - `subscription_info={"endpoint", "keys": {"p256dh", "auth"}}`
  - `data=json.dumps(payload, ensure_ascii=False)`
  - `vapid_private_key=PEM`
  - `vapid_claims={"sub": vapid_subject()}`
  - `ttl=86400` (24 h)
  - `timeout=8.0`
- **Return values:**
  - `True`: success.
  - `False`: `WebPushException` whose response status is **404 or 410**. The subscription is dead.
  - `None`: any other failure. Logs `push failed status=%s err=%s` (status and exception class name), or `push failed err=%s` for non-WebPush exceptions.
- No retries.
- Tests monkeypatch this function: a fake returns `False` for endpoints ending in `/dead`, otherwise `True`, and records `(target, payload)`.

**`deliver(engine, targets, payload)`:**
- Sends to each target **sequentially**, collecting the ids that returned `False`.
- If there are any, it opens a **new** session, deletes those `push_subscriptions` rows and commits.

#### 2.6.3 Targeting: `schedule(background, engine, session, *, kind, title, body, url="/", tag=None, exclude_user_id=None, copy=None) -> int`

1. **`collect_targets(session, kind, exclude_user_id)`:**
   - `pref = {"meeting": "notify_meeting", "pick": "notify_pick", "note": "notify_note", "test": None}[kind]`.
   - Allowed users are **all users in the instance** with `id != exclude_user_id` and (`pref is None` or `user.<pref>` is true).
   - If nobody is allowed, return `[]`.
   - Otherwise load every `push_subscriptions` row whose `user_id` is allowed. There is **no ORDER BY**.
   - Each target is `{id, endpoint, p256dh, auth, locale}`, with `locale = normalize_locale(user.locale)`.
2. If there are no targets, return 0.
3. Group targets by `normalize_locale(target.locale or "en")`.
4. Per group:
   - `(title, body) = copy[locale]` if `copy` has that locale, else the passed `title` and `body` (the callers pass the English copy).
   - The payload is exactly `{"title", "body", "url", "tag": tag or kind, "kind": kind}`.
   - Add a background task `deliver(engine, group, payload)`. With `background=None`, deliver synchronously; no current caller does that.
5. Return `len(targets)`: devices, not users.

**Timing.**
- FastAPI `BackgroundTasks` run **after the response is sent**, in the same process.
- `TestClient` runs them before the call returns, which is why the tests can inspect `sent` right away.
- A crash during delivery is invisible to the client.

#### 2.6.4 Meeting reminder: `maybe_schedule_meeting_reminder(background, engine, session, pick, club_name) -> bool`

- Returns False when any of these holds:
  - `pick` is None;
  - `meeting_at` is None;
  - `reminder_sent_at` is not None;
  - `pick.book` is None.
- A naive `meeting_at` is treated as UTC.
- Sends only if `now < meeting <= now + 24h`. **Exactly 24 h away still sends**, because the check is `meeting - now > 24h`, so equality passes.
- Then:
  1. Set `reminder_sent_at = now` and commit.
  2. Build `copy = push_copy_map("meeting", club=club_name, title=book.title)`.
  3. Call `schedule(kind="meeting", url="/", tag=f"meeting-{pick.id}", copy=copy)`, using the English copy as the default title and body.
- Returns True.

### 2.7 i18n for push (`i18n.py`)

- `LOCALES = ("en", "de")`; `DEFAULT_LOCALE = "en"`.
- `is_locale(v)` is `v in LOCALES`. It is case-sensitive; the preferences validator in the auth area uses it.
- `normalize_locale(v)`: `(v or "").strip().lower()`, returned if it is in `LOCALES`, else `"en"`.
- `push_copy(kind, locale, **fields)`:
  - looks up `PUSH_COPY[kind]` (an unknown kind raises `KeyError`);
  - uses the normalized locale's pair, falling back to `en`;
  - applies `str.format(**fields)` to the title and body. Extra fields are ignored; a missing field raises `KeyError`.
  - User text is substituted as **values**, never as templates, so braces in a book title are safe.
- `push_copy_map(kind, **fields)` returns `{loc: push_copy(kind, loc, **fields) for loc in LOCALES}`.

**`PUSH_COPY`, verbatim** (`—` is U+2014):

| kind | en title | en body | de title | de body |
|---|---|---|---|---|
| `pick` | `{club}: new club pick` | `{actor} chose {title}.` | `{club}: neues Club-Buch` | `{actor} hat {title} gewählt.` |
| `note` | `{actor} on {title}` | `{excerpt}` | `{actor} zu {title}` | `{excerpt}` |
| `meeting` | `{club}: meeting tomorrow` | `{title} — see you at the meeting.` | `{club}: Treffen morgen` | `{title} — bis zum Treffen.` |
| `test` | `{club}: notifications are on` | `You will hear about new picks, notes, and meeting reminders.` | `{club}: Benachrichtigungen sind an` | `Du hörst von neuen Club-Büchern, Notizen und Treffen.` |

Web Push copy is the only backend-side UI copy. All other UI strings live in the frontend `en.json` and `de.json`.

### 2.8 Backup semantics

- The SQLite online backup API (`src.backup(dst)`) gives a consistent snapshot while the app writes, and works with WAL. The source is opened read-only.
- There is no restore endpoint. Restoring means replacing files, as described in SELFHOST.md §4.

### 2.9 `seed_demo` (`python -m app.seed_demo`)

- **Target:** a fixed path, `DEMO_DB = <repo>/data/demo.db`. It is **not configurable**: no env var and no argument. A wrapper must assign `app.seed_demo.DEMO_DB` before calling `main()`.
- **Setup:** `init_db(DEMO_DB)` runs the full schema, migrations and legacy import. It does **not** call `ensure_bootstrap_invite`. Booting the app on this DB later creates no bootstrap invite either, because `invites` is non-empty.
- **Idempotent:** if any user exists, it prints `Demo already exists; preserving edits: {DEMO_DB}` and returns.
- **Output:** `Demo created: {DEMO_DB}`, then `Login: demo / DemoBuecher2026!`.
- **Timestamps:** everything is relative to `now = utcnow()`, so the output is **not deterministic**. Password hashes use random salts.

**The data it creates (book `bN` = `books[N-1]`):**

- **users:** `demo`, `anna`, `max`.
  - Password `DemoBuecher2026!` (argon2-cffi defaults: `$argon2id$v=19$m=65536,t=3,p=4$…`).
  - `locale="de"`, `created_at = now − 45d`.
  - Default theme `paper`, `color_mode` `system`, all `notify_*` true, no avatar.
- **events:** one `member_joined` per user, `payload "{}"`.
- **books b1–b8:**
  - `ol_work_key = f"/works/BC{n:010x}"`, i.e. `/works/BC0000000001` … `/works/BC0000000008`. These match `CLUB_WORK_ID_RE ^BC[a-f0-9]{10}$`, so they are club-only books.
  - Each has title, authors (one name), year, pages, a German `description`, `subjects = json.dumps([subject], ensure_ascii=False)`, and `details_fetched_at = now`.
  - `cover_id`, `cover_image_url`, `spine_color` and `ol_rating` are NULL.

  | b | title | author | year | pages | subject |
  |---|---|---|---|---|---|
  | 1 | Die Bibliothek am Meer | Mara Winter | 2025 | 320 | Roman |
  | 2 | Zwischen den Sternen | Jonas Berg | 2024 | 384 | Science Fiction |
  | 3 | Ein Sommer in Lissabon | Clara Sommer | 2023 | 272 | Roman |
  | 4 | Der Garten der Zeit | Emil Falk | 2022 | 296 | Fantasy |
  | 5 | Kleine Wege, große Welten | Lea Neumann | 2025 | 224 | Sachbuch |
  | 6 | Nachtzug nach Norden | Finn Hansen | 2024 | 352 | Krimi |
  | 7 | Das Haus der leisen Stimmen | Sophie Adler | 2021 | 304 | Roman |
  | 8 | Kaffee mit dem Morgen | Paul Richter | 2025 | 192 | Erzählungen |

- **shelf (18 rows).**
  - `position` is the 0-based index within that user's list.
  - `shelf_position` is NULL.
  - `updated_at = now − position h`.
  - `started_at = now − (12 + bookIndex0) d` unless `want_to_read`. This includes DNF and currently reading.
  - `finished_at = now − (bookIndex0 + 2) d` only when finished.
  - `take = "Eine Geschichte, die noch lange nachklingt."` iff a rating is set, else `""`.
  - `dnf_reason = "Diesmal war es nicht das richtige Buch für mich."` iff DNF, else `""`.
  - Rows per user:
    - demo: b1 reading 45%, b2 want, b3 finished 100% ★5, b4 want, b5 finished 100% ★4, b6 want, b7 DNF (progress NULL), b8 finished 100% ★4.
    - anna: b1 reading 70%, b2 want, b3 finished ★4, b4 finished ★5, b6 want.
    - max: b1 reading 25%, b2 want, b4 want, b5 finished ★5, b6 finished ★4.
  - Each row also records a `shelf_added` event with payload `{"status": <status>, "rating": <n>}` (rating omitted when NULL). **This is a quirk:** the real app records `shelf_finished` or `shelf_dnf` for those statuses.
- **user_favorites:** demo b3 at position 1, demo b5 at position 2.
- **club_picks:**
  - **Previous pick:** b3, set by anna, note `Unsere letzte gemeinsame Lektüre.`, started `now − 40d`, ended `now − 15d`, no meeting.
  - **Current pick:** b1, set by demo, note `Willkommen im Demo-Buchclub! Beim nächsten Treffen sprechen wir über die Briefe und Noras Neuanfang.`, started `now − 8d`, `meeting_at = now + 7d`, `reminder_sent_at` NULL.
  - Event `pick_set` (actor demo, book b1, the current pick), payload `{}`. The seed passes no `meeting_at`.
- **pick_milestones** (current pick, created by demo):

  | position | title | chapters | due |
  |---|---|---|---|
  | 0 | `Ankommen an der Küste` | 1–8 | now + 2d |
  | 1 | `Die Briefe in den Büchern` | 9–16 | now + 5d |
  | 2 | `Das Ende und unser Club-Treffen` | 17–24 | now + 7d |

- **book_posts** (all on b1, `pick_id` = current pick, `status_at = currently_reading`):

  | # | author | body | spoiler_upto | progress_at | created |
  |---|---|---|---|---|---|
  | 1 | anna | `Die Atmosphäre am Meer gefällt mir sehr. Ich würde sofort in dieser Bibliothek stöbern!` | NULL | 20 | now − 18h |
  | 2 | demo | `Die Briefe sind mein Lieblingsdetail bisher. Bin gespannt, wer sie hinterlassen hat.` | 40 | 45 | now − 15h |
  | 3 | max | `Ich bin gerade bei Kapitel 6. Die Figuren wachsen mir langsam ans Herz.` | 25 | 25 | now − 12h |
  | 4 | anna | `Ab der Mitte bekommen die Briefe eine neue Bedeutung. Darüber müssen wir beim Treffen sprechen!` | 65 | 70 | now − 9h |
  | reply to #1 | demo | `Ja! Ein perfekter Ort für unser nächstes Treffen.` | NULL | 45 | now − 2h |

- **book_post_reactions:** post #1 by demo `❤️` (U+2764 U+FE0F); post #2 by anna `👍`.
- **quotes:** demo on b1: `Manche Geschichten finden uns genau dann, wenn wir sie brauchen.`, page 84.
- **next_up_votes:** one open vote, `closes_at = now + 4d`.
  - Nominations: b2 by demo, b4 by anna, b6 by max.
  - Ballots: anna voted for the b2 nomination; max voted for the b4 nomination.
- **invites:** `DEMOTEST2026`, created by demo, unused.

**Row counts:**

| table | rows |
|---|---|
| users | 3 |
| books | 8 |
| shelf | 18 |
| user_favorites | 2 |
| club_picks | 2 |
| pick_milestones | 3 |
| book_posts | 5 |
| book_post_reactions | 2 |
| quotes | 1 |
| next_up_votes | 1 |
| next_up_nominations | 3 |
| next_up_ballots | 2 |
| invites | 1 |
| events | 22 (3 + 18 + 1) |
| club_pick_posts, post_reactions, push_subscriptions, search_history, physical_shelf_layouts | 0 |

There are also no avatars, no OL or Google books, and no tombstones.

### 2.10 API-error-copy drift check (`test_api_error_copy.py`)

- Reads `frontend/src/i18n/apiErrors.ts` and extracts every key with the regex `"([^"]+)":\s*"errors\.`.
- Requires **at least 10** keys.
- Asserts each key string appears somewhere in the concatenated text of `backend/app/**/*.py`. This is a plain substring check, so a string in a comment or docstring also passes.

### 2.11 Test harness (`conftest.py`), which the Rust harness must replace

**The `client` fixture, per test:**
- Removes `BOOKCLUB_ENV_FILE`.
- Sets:
  - `DATABASE_PATH=<tmp>/bookclub.db`
  - `SECRET_KEY=test-secret-key-which-is-long-enough`
  - `BOOKCLUB_BOOTSTRAP_INVITE=TESTINVITE`
  - `DEBUG=true`
  - `BOOKCLUB_NAME=Bookclub`
  - `BOOKCLUB_THEME=#b44a2a`
  - `BOOKCLUB_PUBLIC_URL=""`
  - `BOOKCLUB_TRUSTED_PROXIES=*`
  - `BOOKCLUB_TZ=UTC`
- Removes `GOOGLE_BOOKS_API_KEY`.
- Clears the settings cache and resets the module-level state:
  - `security.reset_rate_limits()`
  - `books.clear_search_cache()`
  - `openlibrary.clear_details_cache()`
  - `googlebooks.clear_google_cache()`
- Yields `TestClient(create_app())` with `raise_server_exceptions` at its default (True).

**Side effects of that setup:**
- The temp dir is also the data dir, so the VAPID key lands in `tmp_path`.
- The VAPID `lru_cache` is **not** reset by the fixture; `test_push` resets it explicitly.

**Helpers:**
- `register(client, username, password="password1", invite="TESTINVITE")` posts `/api/auth/register`. A successful register also signs the client in.
- `login(client, username, password="password1")`.

**Multi-member pattern** (`test_push._add_member`):
1. The signed-in user mints an invite with `POST /api/invites`.
2. Log out.
3. Register the new user, who is then signed in.
4. **Every later call is made as the newest user.**

---

## 3. Data: the complete `models.py`

### 3.0 Storage conventions (SQLite, as found in a real DB file)

- **Primary keys:** `INTEGER NOT NULL ... PRIMARY KEY (id)` with no `AUTOINCREMENT`, so SQLite **may reuse** the id of a deleted max row.
- **Datetimes:**
  - `DATETIME` columns hold **TEXT** `YYYY-MM-DD HH:MM:SS.ffffff`, naive, UTC wall clock (probed: `'2026-10-27 11:00:28.202443'`).
  - Rows written by hand or by old code may have no fraction (`'2024-01-01 00:00:00'`, as in the migration test).
  - The model declares `DateTime(timezone=True)`, but SQLite drops the offset.
  - **All writes use `utcnow()` (aware UTC)** or values converted to UTC. `parse_meeting` converts local club time to UTC.
- **Booleans** are stored as INTEGER `0`/`1`.
- **Enums:**
  - `ShelfStatus` is stored as `VARCHAR(17)` holding the **name**, which equals the value: `want_to_read`, `currently_reading`, `finished`, `did_not_finish`.
  - `book_posts.status_at` uses the same enum and is nullable.
- **JSON-in-TEXT columns:** `books.subjects`, `events.payload`, `physical_shelf_layouts.layout_json`.
- **Foreign keys** are enforced (`PRAGMA foreign_keys=ON`). Only `user_favorites.{user_id,book_id}` and `physical_shelf_layouts.user_id` have `ON DELETE CASCADE`. All others are `NO ACTION`. No route deletes users or books today.
- **ORM-level cascade:** `User.favorites` and `Book.favorites` use `cascade_delete=True`.
- **ORM hook `_defer_avatar_blob`:** a `do_orm_execute` listener adds `defer(User.avatar)` to every entity SELECT that loads `User`, except column refreshes. The avatar blob is therefore never loaded with a user row. This is a performance detail only.
- **Module constants:**
  - `FAVORITE_LIMIT = 3`, mirrored in a CHECK constraint.
  - `NOMINATION_LIMIT = 6` (per vote; enforced in `routers/vote.py`).
  - `utcnow()` returns `datetime.now(timezone.utc)`.

### 3.1 `users` (model `User`)

| column | type | null | default | semantics |
|---|---|---|---|---|
| id | INTEGER PK | no | | |
| username | VARCHAR(32) | no | | unique (`ix_users_username`). Lower-cased by the auth validators |
| password_hash | VARCHAR | no | | argon2 PHC string from argon2-cffi `PasswordHasher()` defaults (argon2id) |
| theme | VARCHAR(32) | no | `paper` | palette id: `paper`, `slate`, `forest` or `ink` (`themes.py`) |
| color_mode | VARCHAR(16) | no | `system` | `light`, `dark` or `system` |
| locale | VARCHAR(8) | no | `en` | `en` or `de` |
| created_at | DATETIME | no | utcnow | |
| notify_meeting | BOOLEAN | no | 1 | Web Push opt-in for the meeting reminder |
| notify_pick | BOOLEAN | no | 1 | opt-in for a new club pick |
| notify_note | BOOLEAN | no | 1 | opt-in for legacy pick notes |
| avatar | BLOB | yes | NULL | processed JPEG bytes. NULL means no picture |
| avatar_mime | VARCHAR(32) | yes | NULL | |
| avatar_updated_at | DATETIME | yes | NULL | used for the cache-busting avatar URL |

- **Relationships:** `shelf_entries`, and `favorites` (cascade).
- **Legacy:** pre-migration rows got the DDL defaults (`paper`, `system`, `en`, `1`, `1`, `1`), as asserted by the migration test.

### 3.2 `invites` (model `Invite`)

| column | type | null | semantics |
|---|---|---|---|
| id | INTEGER PK | no | |
| code | VARCHAR(32) | no | unique. Normalized: whitespace and `-` removed, upper-cased. Generated codes are 10 Crockford base32 characters |
| created_by_id | INTEGER FK users | yes | NULL means the bootstrap invite or one from the `create_invite` CLI |
| used_by_id | INTEGER FK users | yes | NULL means unused |
| created_at | DATETIME | no | |
| used_at | DATETIME | yes | |

### 3.3 `books` (model `Book`): the per-instance catalog

| column | type | null | default | semantics |
|---|---|---|---|---|
| id | INTEGER PK | no | | |
| ol_work_key | VARCHAR(64) | no | | unique. **Always `/works/<id>`**, where `<id>` is `OL\d+W` (Open Library), `BC[a-f0-9]{10}` (club-added), `ISBN(\d{9}[\dX]\|\d{13})` (Google with ISBN) or `GB[A-Za-z0-9_-]{1,40}` (Google volume) |
| title | VARCHAR | no | | |
| authors | VARCHAR | no | `''` | display string; several authors are comma-joined |
| cover_id | INTEGER | yes | | Open Library cover id |
| cover_image_url | VARCHAR(512) | yes | | remote cover (Google `imageLinks`), preferred over `cover_id` |
| spine_color | VARCHAR(7) | yes | | `#rrggbb`. **NULL means not sampled yet; `''` means sampled with no usable cover** |
| year | INTEGER | yes | | first publication year |
| description | VARCHAR | no | `''` | |
| pages | INTEGER | yes | | Open Library median page count or Google pageCount |
| subjects | VARCHAR | no | `'[]'` | JSON array of subject strings |
| ol_rating | FLOAT | yes | | Open Library average rating |
| details_fetched_at | DATETIME | yes | | when the details were last pulled from the provider. NULL means never |

### 3.4 `shelf` (model `ShelfEntry`)

`UNIQUE(user_id, book_id)` (`uq_shelf_user_book`); index on `user_id` and on `status`.

| column | type | null | semantics |
|---|---|---|---|
| id | INTEGER PK | no | referenced by `physical_shelf_layouts.layout_json` |
| user_id | FK users | no | |
| book_id | FK books | no | |
| status | VARCHAR(17) | no | `ShelfStatus` |
| position | INTEGER | no | default 0. Order within the status column |
| shelf_position | INTEGER | yes | optional manual order across all statuses. NULL means unset |
| rating | INTEGER | yes | 1–5, validated by the API |
| take | VARCHAR(140) | no (fresh) / **yes (migrated)** | default `''`. Short review |
| dnf_reason | VARCHAR(200) | no / **yes (migrated)** | default `''` |
| progress | INTEGER | yes | percent, 0–100 |
| updated_at | DATETIME | no | |
| started_at | DATETIME | yes | |
| finished_at | DATETIME | yes | |

The shelf spec covers the status-transition stamping and the reading-date rules.

### 3.5 `physical_shelf_layouts` (model `UserPhysicalShelfLayout`)

| column | type | semantics |
|---|---|---|
| user_id | INTEGER PK, FK users **ON DELETE CASCADE** | one layout per user |
| layout_json | VARCHAR NOT NULL | JSON. Model default: `{"rows":[[],[],[]],"material":"oak","spines":{},"decorations":{}}` |

**JSON contents** (see the API schema `PhysicalShelfLayout`):
- `rows`: 1–12 rows. Each row is a list of items: either a **shelf entry id** (an int > 0) or a decoration id (`decor:[a-zA-Z0-9_-]{1,64}`).
- `material`: `oak`, `walnut`, `white` or `black`.
- `lights`: bool. It may be absent in old rows.
- `spines`: keyed by **`str(shelf entry id)`**, values `{color?, width?, display}`.
- `decorations`: keyed by decoration id, values `{kind, color}`.
- **These ids are embedded in JSON**, so the importer must rewrite them if shelf ids change.

### 3.6 `user_favorites` (model `UserFavorite`)

- **Constraints:** `UNIQUE(user_id, book_id)`, `UNIQUE(user_id, position)`, `CHECK(position >= 1 AND position <= 3)`.
- **FKs:** both `ON DELETE CASCADE`. Indexes on `user_id` and `book_id`.
- **Columns:** `id`, `user_id`, `book_id`, `position` (1–3, ordered favourite slot).

### 3.7 `club_picks` (model `ClubPick`)

| column | type | null | semantics |
|---|---|---|---|
| id | PK | | |
| book_id | FK books (indexed) | no | |
| set_by_id | FK users | no | last setter. Re-PUT of the same book overwrites it. For an auto-closed vote it is the winning nominator |
| note | VARCHAR(280) | no | default `''` |
| started_at | DATETIME | no | |
| ended_at | DATETIME (indexed) | yes | **NULL means this is the current pick.** At most one is open by convention, not by constraint |
| meeting_at | DATETIME | yes | UTC; entered in the club zone (`BOOKCLUB_TZ`) |
| reminder_sent_at | DATETIME | yes | non-NULL means the 24 h push reminder has fired (§2.6.4) |

### 3.8 `pick_milestones` (model `PickMilestone`)

| column | type | null | semantics |
|---|---|---|---|
| id | PK | | |
| pick_id | FK club_picks (indexed) | no | |
| title | VARCHAR(120) | no | e.g. "Chapters 1–10" |
| chapter_from | INTEGER | yes | chapter range start |
| chapter_to | INTEGER | yes | chapter range end |
| due_at | DATETIME | yes | |
| position | INTEGER | no | default 0. Order |
| created_by_id | FK users | no | |
| created_at | DATETIME | no | |

### 3.9 `club_pick_posts` (model `ClubPickPost`): legacy pick notes

| column | type | null | semantics |
|---|---|---|---|
| id | PK | | |
| pick_id | FK club_picks (indexed) | no | |
| author_id | FK users | no | |
| body | VARCHAR(1000) | no | stripped, 1–1000 characters |
| spoiler_upto | INTEGER | yes | percent of the book the note is safe up to. NULL means no flag |
| milestone_id | FK pick_milestones | yes | **not copied to `book_posts`** |
| created_at | DATETIME | no | |
| updated_at | DATETIME | yes | NULL means never edited |

### 3.10 `post_reactions` (model `PostReaction`): reactions on legacy notes

- `UNIQUE(post_id, user_id, emoji)`.
- **Columns:**
  - `id`
  - `post_id` (FK club_pick_posts, indexed)
  - `user_id` (FK users)
  - `emoji` (VARCHAR(16); allowlisted in the API as `❤️ 👍 😂 😮 🔥 📚`)
  - `created_at`

### 3.11 `book_posts` (model `BookPost`): diary entries

These are visible to the whole club today.

| column | type | null | semantics |
|---|---|---|---|
| id | PK | | |
| book_id | FK books (indexed) | no | |
| author_id | FK users (indexed) | no | |
| parent_id | FK book_posts (indexed) | yes | replies are **one level deep**: this always points at a top-level entry |
| body | VARCHAR(1000) | no | cleared to `''` on a tombstone |
| spoiler_upto | INTEGER | yes | percent safe-up-to |
| progress_at | INTEGER | yes | the author's progress percent when writing (a snapshot) |
| status_at | VARCHAR(17) | yes | the author's shelf status when writing (a snapshot) |
| pick_id | FK club_picks | yes | the club pick that was running for this book when written |
| legacy_post_id | INTEGER **UNIQUE** | yes | set when mirrored from `club_pick_posts` (§2.3). No FK |
| deleted_at | DATETIME | yes | **tombstone**: an entry with replies is soft-deleted (body cleared) so the replies keep their anchor |
| created_at | DATETIME (indexed) | no | |
| updated_at | DATETIME | yes | |

### 3.12 `book_post_reactions` (model `BookPostReaction`)

- `UNIQUE(post_id, user_id, emoji)`.
- **Columns:**
  - `id`
  - `post_id` (FK book_posts, indexed)
  - `user_id`
  - `emoji` (VARCHAR(16))
  - `created_at`

### 3.13 `events` (model `Event`): append-only activity log

| column | type | null | semantics |
|---|---|---|---|
| id | PK | | |
| actor_id | FK users (indexed) | no | |
| kind | VARCHAR(32) (indexed) | no | one of `activity.KINDS`. Anything else raises `ValueError(f"Unknown event kind: {kind}")` |
| book_id | FK books | yes | |
| pick_id | FK club_picks | yes | |
| payload | VARCHAR | no | default `'{}'`. JSON object with `None` values dropped. Readers return `{}` on bad JSON or a non-dict |
| created_at | DATETIME (indexed) | no | the feed reads newest first |

**Kinds and payload keys, by writer:**

| kind | writer | payload keys |
|---|---|---|
| member_joined | register, seed | none |
| shelf_added | shelf add (non-finished, non-DNF) | `status` (seed also sends `rating`) |
| shelf_moved | shelf status change | `status` |
| shelf_finished | to finished | `rating?`, `take?` |
| shelf_dnf | to DNF | `reason?` |
| shelf_removed | shelf delete | `status` |
| progress | progress update | `progress` |
| pick_set | new pick | `meeting_at?` (ISO with `+00:00`) |
| pick_cleared | DELETE pick | none |
| note_posted | legacy pick note | `post_id` (a **club_pick_posts** id) |
| reaction | legacy pick-note reaction | `emoji`, `post_id` (a **club_pick_posts** id) |
| nominated | vote nomination | none |
| voted | ballot | none |
| vote_closed | apply or auto-close | `votes` (the winner's ballot count) |
| milestone_added | milestone create | `title`, `due_at?` |
| quote_added | quote create | `page?`, `excerpt` (the first 80 characters of the body) |

Diary entries and diary reactions record **no** events.

### 3.14 `push_subscriptions` (model `PushSubscription`)

| column | type | null | semantics |
|---|---|---|---|
| id | PK | | |
| user_id | FK users (indexed) | no | the current owner. It can move to another user on takeover |
| endpoint | VARCHAR(1024) **UNIQUE** | no | push-service URL, `https://` |
| p256dh | VARCHAR(256) | no | client ECDH public key, base64url |
| auth | VARCHAR(128) | no | client auth secret, base64url |
| user_agent | VARCHAR(256) | no | default `''`. Informational label for the settings list |
| created_at | DATETIME | no | first subscribe. Unchanged on takeover |

Rows are deleted by an explicit unsubscribe or by pruning when the push service answers 404 or 410.

### 3.15 `quotes` (model `Quote`)

- **Columns:**
  - `id`
  - `user_id` (FK, indexed)
  - `book_id` (FK, indexed)
  - `body` (VARCHAR(1000))
  - `page` (INTEGER, nullable)
  - `created_at`

### 3.16 `next_up_votes` (model `NextUpVote`)

| column | type | null | semantics |
|---|---|---|---|
| id | PK | | |
| started_at | DATETIME | no | |
| ended_at | DATETIME (indexed) | yes | NULL means the open vote. At most one by convention |
| closes_at | DATETIME | yes | auto-close deadline. Reset to NULL when it passes with no votes |
| winner_nomination_id | INTEGER | yes | **no FK** |

### 3.17 `next_up_nominations` (model `NextUpNomination`)

- `UNIQUE(vote_id, book_id)`.
- **Columns:**
  - `id`
  - `vote_id` (FK, indexed)
  - `book_id` (FK)
  - `nominated_by_id` (FK users)
  - `created_at`
- At most 6 per vote (`NOMINATION_LIMIT`).

### 3.18 `next_up_ballots` (model `NextUpBallot`)

- `UNIQUE(vote_id, user_id)`: one ballot per user per vote.
- **Columns:**
  - `id`
  - `vote_id` (FK, indexed)
  - `nomination_id` (FK, indexed)
  - `user_id`
  - `created_at`

### 3.19 `search_history` (model `SearchHistory`)

- `UNIQUE(user_id, normalized_query)`.
- **Columns:**
  - `id`
  - `user_id` (FK, indexed)
  - `query` (as typed)
  - `normalized_query` (`query.casefold()`)
  - `last_used_at`
- Upserted on conflict.

---

## 4. Tests (parity checklist: 16 test functions, 19 cases counting parametrisation)

Each line gives the behaviour asserted, then the proposed Rust disposition for #49 in brackets.

### backend/tests/test_push.py (10)

- `test_push.py::test_vapid_key_is_generated_and_stable`:
  - After register and `reset_keys()`, `GET /api/push/vapid` returns `public_key` of length 87.
  - Padded and base64url-decoded, it is 65 bytes starting with `0x04`.
  - `<tmp>/.vapid_private.pem` exists.
  - After another `reset_keys()`, the key is unchanged.
  - [Phase 4: push moves to native/Web Push (#60). No Phase 1 port.]
- `test_push.py::test_subscribe_list_unsubscribe`:
  - The list starts as `{"items": []}`.
  - `POST /api/push/test` with no subscription gives 400.
  - Subscribing an `http://` endpoint gives 400.
  - A valid subscribe gives 201 with `endpoint_tail == "abc123def456"` and `user_agent == "Test/1.0"`.
  - Re-subscribing the same endpoint gives 201 and the list still has 1 item.
  - `POST /api/push/test` gives 200 `{"sent_to": 1}`, and the last payload's `kind == "test"`.
  - DELETE of an unknown endpoint gives 404; DELETE of the real one gives 204; the list is empty again.
  - [Phase 4]
- `test_push.py::test_new_pick_notifies_other_members_who_opted_in`:
  - ada, grace and tom each subscribe; tom sets `notify_pick=false`; tom sets the pick.
  - The targets are exactly ada and grace (sorted).
  - The payload has `kind == "pick"`, a body containing `tom chose Circe`, and a title starting with `Bookclub`.
  - Re-PUT of the same book with only `meeting_at` changed sends nothing.
  - [Phase 4. Club-scoped: recipients are members of the pick's club.]
- `test_push.py::test_new_note_notifies_except_author_and_respects_pref`:
  - ada and grace subscribe; grace opts out of notes; grace sets the pick and posts a 170-character note (stripped to 169) via `POST /api/pick/posts`.
  - Only ada is targeted.
  - The payload has `kind == "note"`, `title == "grace on Circe"`, a body ending in `…` and a body length ≤ 90.
  - Then ada posts "Reply" and nothing is sent, because grace opted out and ada is the author.
  - [Phase 4. Decide which posts notify: diary entries never did.]
- `test_push.py::test_dead_subscriptions_are_pruned`:
  - ada subscribes `https://push.example/dead`; grace sets the pick.
  - The only target is the dead endpoint (the fake returns False).
  - ada's subscription list is then empty.
  - [Phase 4]
- `test_push.py::test_push_copy_catalog_covers_every_locale`:
  - `PUSH_COPY` kinds are exactly {pick, note, meeting, test}.
  - Each has exactly the `LOCALES` keys.
  - Formatting with {club, actor, title, excerpt} gives a non-empty title and body without `{`.
  - [Phase 4. Port as a unit test of the Rust copy table.]
- `test_push.py::test_push_copy_follows_recipient_locale`:
  - ada sets locale `de` and subscribes; grace subscribes; tom sets the pick.
  - ada's body contains `gewählt`; grace's body contains `chose Circe`.
  - [Phase 4]
- `test_push.py::test_meeting_reminder_sent_once_inside_24h`:
  - ada subscribes and sets the pick with a meeting in 5 h (naive local time in the UTC zone).
  - The first `GET /api/pick` sends 1 push with `kind == "meeting"` and a body containing `Circe`.
  - Two more GETs send nothing.
  - The DB has a non-NULL `reminder_sent_at` on the open pick.
  - [Phase 4. Keep `reminder_sent_at` in the schema now.]
- `test_push.py::test_meeting_reminder_not_sent_far_ahead_or_when_opted_out`:
  - A meeting 10 days ahead: GET sends nothing.
  - After `notify_meeting=false` and a meeting 2 h ahead on the same pick: GET sends nothing.
  - [Phase 4]
- `test_push.py::test_vote_close_notifies_new_pick`:
  - grace (as the newest user) subscribes; ada logs back in, nominates Achilles and applies the nomination.
  - The only target is grace's endpoint, with `kind == "pick"`.
  - [Phase 4]

### backend/tests/test_backup.py (2)

- `test_backup.py::test_backup_requires_auth`: `GET /api/backup` signed out gives 401. [Replaced: the DSGVO export endpoint (#39) needs auth, so 401 signed out.]
- `test_backup.py::test_backup_returns_sqlite_for_member`:
  - Signed in: 200, the content-type contains `sqlite`, the content-disposition contains `bookclub.db`, and the body starts with `SQLite format 3`.
  - [Replaced: the export returns only the caller's data (JSON plus media) with an attachment disposition. Instance backups become server-side `pg_dump` (#51).]

### backend/tests/test_migrations.py (2)

- `test_migrations.py::test_legacy_database_is_upgraded_in_place`:
  - Setup: a hand-written legacy schema (users, books, shelf, club_picks, club_pick_posts and next_up_votes with their original columns, plus one user, one book and one shelf row) goes through `init_db`.
  - Afterwards:
    - `missing_columns == {}`.
    - `search_history` exists with `user_id`, `query`, `normalized_query`, `last_used_at` and 0 rows.
    - shelf has `rating`, `take`, `dnf_reason`, `progress`, `started_at`, `finished_at`, `shelf_position`, and `shelf_position` is NULL for the old row.
    - books have `description`, `pages`, `subjects`, `ol_rating`, `details_fetched_at`, `cover_image_url`, `spine_color`.
    - users have `notify_*`, `locale`, `avatar`, `avatar_mime`, `avatar_updated_at`, and the old user's `locale == "en"`.
    - club_pick_posts have `spoiler_upto`, `milestone_id`, `updated_at`.
    - next_up_votes have `closes_at`.
  - The old row reads `status='finished'`, `rating NULL`, `take ''`, `subjects '[]'`, and `notify_*` = (1, 1, 1).
  - New tables exist: `events`, `post_reactions`, `push_subscriptions`, `pick_milestones`, `quotes`, `user_favorites`, `physical_shelf_layouts`.
  - `user_favorites` has ≥ 2 unique indexes and all its FKs are CASCADE.
  - `physical_shelf_layouts` columns are exactly {`user_id`, `layout_json`}, with a CASCADE FK.
  - A second `init_db` is a no-op.
  - [Replaced. The Rust schema uses numbered sqlx migrations (#37), checked by "migrations apply cleanly in CI". The importer (#38) must accept a legacy DB of **both** shapes (§2.2). Keep a Rust importer test with a migrated-shape fixture: nullable `take`/`dnf_reason`, timestamps without fractions.]
- `test_migrations.py::test_fresh_database_matches_migration_list`:
  - A fresh `init_db` has no missing columns.
  - Every `COLUMN_MIGRATIONS` column exists in `SQLModel.metadata`.
  - [Dropped: there is no hand-rolled column list any more. The sqlx migration checksum, plus `cargo sqlx prepare --check`, covers drift.]

### backend/tests/test_spa_cache.py (1 function, 4 parametrised cases)

- `test_spa_cache.py::test_app_html_is_not_cached_across_releases[/ | /index.html | /discover | /book/ISBN9781408857892]`:
  - Setup: with `STATIC_DIR` monkeypatched to a temp dir whose `index.html` references `/assets/current.js`, a new app serves each path.
  - Each gives 200, a body containing `current.js`, and `cache-control == "no-store"`.
  - [Port if the Rust server serves the SPA (open question). Otherwise make it a deploy or proxy check. Book URLs change with the new work ids, but any client route must still fall back to a no-store shell.]

### backend/tests/test_api_error_copy.py (1)

- `test_api_error_copy.py::test_mapped_english_api_errors_still_exist_in_backend`:
  - At least 10 details are mapped in `apiErrors.ts`.
  - Each appears as a substring somewhere in `backend/app/**/*.py`.
  - [Port as a Rust integration test that reads `../frontend/src/i18n/apiErrors.ts` and searches `server/src/**/*.rs`. Keep the ≥ 10 floor.]

### Platform behaviour asserted by tests owned by other specs, listed here for cross-reference

- `test_diary.py::test_legacy_pick_notes_are_imported_once_with_reactions`: the legacy pick-post mirror (§2.3). Owned by the diary spec.
- `test_auth.py::test_preference_columns_backfill_pre_existing_users`: `COLUMN_MIGRATIONS` backfill for user preferences. Owned by auth.md.
- `test_auth.py::test_health`, and all of `test_selfhost.py` (13: config, CORS, manifest, the backup CLI, cookies, dotenv, proxies): owned by auth.md.

---

## 5. Multi-club mapping

### 5.1 Every legacy table: where it goes

Tags: **G** global catalog, **P** personal (`user_id`, no `club_id`), **C** club-scoped (`club_id` FK, non-members get 404 [plan #40]), **X** dropped.

| legacy table | scope | new home and changes |
|---|---|---|
| users | P (global account) | `users`. username, password_hash (copied verbatim), theme, color_mode, locale, created_at and notify_* carry over. New: email (nullable) plus verification. The avatar blob moves to the media store [#43]. See auth.md §5.1 |
| invites | C | `club_invites` with club_id, role checks, expiry or single use, revocation [#40]. The bootstrap invite (`created_by_id NULL`) is meaningless. **Open:** whether to import unused legacy codes |
| books | G | `works` + `editions` + `authors` [#37/#41]. `OL…W` stays an OL work. `BC…` becomes a user-added work. `ISBN…`/`GB…` are re-matched by ISBN, and unmatched ones keep bare facts [#38]. `cover_image_url` and the Google description are dropped. `cover_id` goes to the media fetch [#43]. `spine_color` moves to the cover or media row (shelf.md §5). `subjects`, `ol_rating` and `description` become work-level. `pages` becomes edition-level. See catalog.md §5 |
| shelf | P | `shelf_entries(work_id, edition_id NULL, …)`, `UNIQUE(user_id, work_id)`, plus rating visibility [#46]. See shelf.md §5 |
| physical_shelf_layouts | P | Same JSON. Visible to the audience; only the owner edits [#46]. **Shelf ids inside the JSON must stay valid** (R6) |
| user_favorites | P | `work_id` instead of `book_id` |
| club_picks | C | `club_picks(club_id, work_id, …)`. Keep `meeting_at` and `reminder_sent_at` (the latter is needed again in Phase 4) |
| pick_milestones | C | `milestones` with a kind (`percent` or `chapter`). Legacy `chapter_from`/`chapter_to` become the chapter kind [#45] |
| club_pick_posts | C (legacy) | Already mirrored into `book_posts` (§2.3). **Do not import it as a second copy.** Use it only to (a) mirror rows that are not mirrored yet, (b) recover `milestone_id` through `legacy_post_id`, and (c) recover reactions added after mirroring (R5) |
| post_reactions | C (legacy) | Same treatment as above |
| book_posts | P or C, **open** | The plan lists both "diary entries" (personal, with an `only_me` setting) and "pick posts" (club). Today every `book_posts` row is club-visible, and rows with `pick_id` were written during a club pick. **Decide:** entries with `pick_id` become club pick posts and the others personal diary entries, or all become diary entries |
| book_post_reactions | follows its post | |
| events | split | Club kinds (`pick_set`, `pick_cleared`, `note_posted`, `reaction`, `nominated`, `voted`, `vote_closed`, `milestone_added`, `member_joined`) get `club_id`. Personal kinds (`shelf_*`, `progress`, `quote_added`) are either stored without a club and filtered by the audience at read time, or fanned out per club (shelf.md §5 leaves this open). `shelf_finished` rating payloads must honour `only_me` |
| push_subscriptions | X | Dropped [plan]: they are tied to the old origin's VAPID key and service worker |
| quotes | P | Plus visibility (`audience`/`only_me`) [#46] |
| next_up_votes / nominations / ballots | C | `club_id` on the vote; nominations point to `work_id`. `winner_nomination_id` should become a real FK |
| search_history | P | Unchanged |

New tables with no legacy counterpart: clubs, memberships (role), sessions and API tokens, media, editions, authors, edit requests, reports, the DNB harvest cursor.

### 5.2 App assembly in Rust (`create_app` becomes `lib.rs: app(state)` plus `main.rs`)

- **Startup order** (proposal):
  1. tracing;
  2. `Config::from_env()`;
  3. pool;
  4. **`sqlx::migrate!("./migrations").run(&pool)`** before binding [#37: "migrations run on startup"];
  5. start background jobs (DNB harvester, #42);
  6. bind;
  7. serve with graceful shutdown.
- **The scaffold uses `connect_lazy`.** Running migrations at startup forces a connection, so decide whether to retry with backoff until Postgres is up or to exit non-zero and let the supervisor restart (R13).
- **Gone with self-hosting** (auth.md §5.4):
  - `prepare_environment`;
  - generated secrets and the bootstrap invite;
  - `DEBUG` defaulting to true;
  - dotenv handling;
  - the per-request `Secure` decision;
  - the `catalog:` provider log (Google is removed).
- **`AppState`** should own everything that was module-level in Python: catalog caches, rate-limit buckets, the outbound HTTP clients (behind traits, so tests inject fakes), the media store and the email sender. Each test's `app(state)` then has fresh state, which replaces the conftest reset list.
- **`/api/docs` and `/api/openapi.json`:** #47 requires `GET /api/openapi.json` (utoipa). Python exposed it only in debug. Decide whether it is public in production (open question).
- **Middleware (tower layers), outermost first** (proposal):
  1. request id and tracing;
  2. CORS: an explicit allow-list from config, initially empty in production. Phase 4 adds the Capacitor origins `capacitor://localhost` and `http://localhost` with bearer tokens;
  3. a trusted-proxy real-IP extractor for per-IP rate limits;
  4. the session and bearer-token auth extractor (per route, not global);
  5. body-size limits per route (R9).

### 5.3 Error contract in Rust

- A single `ApiError { status, detail: String }` implementing `IntoResponse` as `{"detail": detail}`.
- **Map every framework rejection to it:**
  - `JsonRejection`, `QueryRejection`, `PathRejection` and missing or wrong content-type become **400**. axum's defaults are 400, 415 and 422 with **plain-text** bodies (R7).
  - Unknown `/api/*` routes: 404 `{"detail":"Not found"}`.
  - Wrong method on a known route: 405 `{"detail":"Method Not Allowed"}`.
  - Panics and `sqlx`/`anyhow` errors: 500 **JSON** `{"detail":"Internal Server Error"}`, logged. This deliberately improves on Python's text/plain, which the frontend's `JSON.parse` chokes on.
- **Multi-error messages:** keep `"; "` joining in field order where a DTO is validated by hand.
- **Auth before body validation** (proposal): put the auth extractor before `Json<T>` in handler arguments. A signed-out request then always gets 401, even for malformed JSON. Python returned 400 for malformed JSON before auth. No test depends on that.
- **Datetimes:** serialize every timestamp as **RFC 3339 UTC with `Z`** (`time::OffsetDateTime` via `serde(with = "time::serde::rfc3339")`). `clubDate()` already accepts it. It fixes `formatClubDate()` and `relativeDay()`, which today read offset-less strings as device-local time (R8).

### 5.4 SPA serving

The plan doesn't say whether the Rust server or the reverse proxy serves the built Vue app (open question). Either way, keep these contracts:
- `index.html` and every client-route fallback: `Cache-Control: no-store`.
- `/assets/*` (hashed): **`Cache-Control: public, max-age=31536000, immutable`**. This improves on Python, which sends no Cache-Control. A **miss returns 404 and never the shell.**
- `/api/*` never falls through to the shell. Python's quirk of `GET /api` serving the shell needn't be kept.
- GET **and HEAD** should be answered on SPA routes; Python's 405 on HEAD is not worth keeping.
- **If axum serves it:** use `tower-http` `ServeDir` with `fallback` to `index.html` and `SetResponseHeader` layers. The `fs` and `set-header` features are not in `Cargo.toml` yet. The Vite `outDir` must move off `backend/app/static`, because `backend/` is deleted at cutover [plan]. For example, use `frontend/dist`, copied into the image.
- `brand_index_html` goes away: per-instance branding no longer exists. Put the public name (#50) into `index.html` at build time. The `theme-color` replacement is already dead code.

### 5.5 `/api/config` and the manifest

- **Not club-scoped any more.** Branding becomes service-level: the name from #50, a fixed accent, `public_url`, and perhaps `signup_open` [auth.md proposal].
- **Timezone:** `timezone` (`BOOKCLUB_TZ`) splits in two:
  - meeting times use a **per-club** zone, which belongs in club pick and vote responses (they already carry `timezone`);
  - personal dates use a **per-user** zone (shelf.md §5).
  - The importer seeds both from the old instance's `BOOKCLUB_TZ`.
- **Manifest:** a static `frontend/public/manifest.webmanifest` already exists with the default values, and `brand.ts` rewrites the manifest at runtime. The dynamic `/manifest.webmanifest` route can be dropped once the name is fixed at build time.

### 5.6 Web Push: dropped in Phase 1, back in Phase 4

**In Phase 1:**
- No `/api/push/*` routes. The current frontend doesn't call them, since the service worker and push store were removed in PR #3.
- **Keep** `users.notify_meeting`, `notify_pick` and `notify_note`, and import them. #48 lists "notification prefs" in account settings, and `GET`/`PATCH /api/auth/notifications` are auth-area endpoints.
- **Keep** `club_picks.reminder_sent_at`, and import it. That way Phase 4 doesn't re-send reminders for meetings that already got one.
- **Drop** `push_subscriptions`, `VAPID_*`, `.vapid_private.pem`, `push.py` and `i18n.py`'s `PUSH_COPY` [plan].

**For Phase 4 (#60), recorded so the semantics survive:**
- Recipients become **members of the club** the event belongs to, minus the actor, filtered by the per-kind prefs. This replaces "every user in the instance".
- `{club}` becomes that club's name.
- Copy is per recipient locale (`en`/`de`), with English as the fallback.
- Keep:
  - the one-shot 24 h meeting reminder and its `now < meeting ≤ now+24h` window;
  - the excerpt rule (≤ 90 characters, else 87 + `…`);
  - pruning on 404/410 (and the APNs/FCM equivalents);
  - one push per device;
  - delivery off the request path.
- Run the meeting reminder as a **scheduled background job**, not only on `GET /pick` reads: with many clubs, a club nobody opens would never get a reminder.
- Per-club mute settings may be wanted (open question).

### 5.7 Backup becomes the personal export plus server backups

- `GET /api/backup` and the `app.backup` CLI are **removed** [plan, #49].
- **Replacements:**
  - **DSGVO export** [#39]: `GET` (path to be decided) returns the caller's own personal tables as JSON, plus their media. It needs auth and must never include other users' hashes or data. Club content they authored is presumably included; decide in #39.
  - **Server-side backups:** encrypted offsite `pg_dump` plus the media store [plan, #51]. These are an operator concern with no endpoint.
- **Frontend:** `SettingsPage.vue`'s `href="/api/backup"` link and the `settings.backup`, `settings.backupBlurb` and `settings.downloadDatabase` keys (en and de) become an "Export my data" action [#48].

### 5.8 `seed_demo` as the #38 fixture generator

`seed_demo` produces a realistic, fully synthetic legacy SQLite DB. It is a good base for the importer fixture [#38 AC: "fixture DB produced by the Python app imports with matching counts for every entity"]. It doesn't cover everything. A generator script (Python, run against the real `app` package with `DEMO_DB` overridden) should add:
- an **Open Library** book (`/works/OL…W` with `cover_id`), a **Google ISBN** book (`/works/ISBN978…`) and a **Google volume** book (`/works/GB…`, with `cover_image_url` and a description), so the re-match, keep-bare-facts and drop-Google paths are all exercised;
- `club_pick_posts` and `post_reactions` rows, some mirrored and **one deliberately not mirrored** (inserted after `init_db`), plus a reaction added after mirroring;
- one `milestone_id` on a legacy post;
- a `book_posts` **tombstone** with a reply;
- `search_history` rows;
- a `physical_shelf_layouts` row referencing shelf ids and a decoration;
- an avatar blob;
- a used invite and a bootstrap invite (`created_by_id NULL`);
- `push_subscriptions` rows, to prove they are dropped;
- an ended vote with `winner_nomination_id`;
- a `shelf_position` value;
- `spine_color` values `''` and `#rrggbb`;
- events of every kind;
- one user with `notify_*` = 0;
- one user with locale `en`;
- **a migrated-shape variant**: start from the `LEGACY_SCHEMA` of `test_migrations.py` and boot `init_db` on it, giving nullable `take` and timestamps without fractions.

**Determinism:**
- Pass explicit timestamps rather than relying on `now`. SQLModel `default_factory=utcnow` binds the function object, so patching `app.models.utcnow` does **not** change defaults.
- For the "legacy user logs in with the old password" test, embed a **fixed known argon2 hash** of a known password instead of hashing at generation time.

**Where the fixture lives** is an open question: a committed small `.db`, or generated in CI, which needs Python in the Rust CI job. The plan only forbids committing the production copy.

### 5.9 What no longer makes sense

- Instance branding (`BOOKCLUB_NAME/THEME/THEME_DARK`, `brand_index_html`, the dynamic manifest).
- `BOOKCLUB_TZ` as one instance zone.
- The bootstrap invite.
- `DEBUG`-gated docs.
- `PRAGMA`s, `COLUMN_MIGRATIONS`, `missing_columns`, `create_all`.
- The boot-time `import_legacy_pick_posts`: it becomes a one-off importer step, and the legacy `/api/pick/posts*` endpoints are not ported.
- Whole-database download.
- `GOOGLE_BOOKS_API_KEY` and the `catalog:` log line.
- Web Push in Phase 1.
- The SQLite id-reuse hazards (Postgres sequences never reuse ids).

---

## 6. Port risks

**R1. Validation must return 400 with a JSON `detail`.** axum's `Json` extractor returns 400, 415 or 422 with `text/plain` bodies by default. The frontend then shows `errors.generic` (a non-string detail), or throws on `JSON.parse` of plain text. Wrap every extractor.

**R2. Only `application/json` bodies are parsed in Python.** Valid JSON with `text/plain` or no content type gives 400 `Input should be a valid dictionary or object to extract fields from`. axum gives 415 `Expected request with Content-Type: application/json`. Map it to 400 JSON.

**R3. Character counts, not bytes.** `endpoint[-12:]`, `user_agent[:256]`, the 87/90 excerpt rule, `name[:48]` and `short_name[:32]` are code-point slices. Rust `&s[..n]` slices bytes and **panics** inside a multi-byte character. Use `chars()`.

**R4. Push upsert is global by endpoint and transfers ownership.** Unsubscribing another user's endpoint is a 404 for the caller. Subscribe returns 201 even for an update, and `created_at`/`id` stay unchanged. (Phase 4 only, but easy to get wrong when it returns.)

**R5. Legacy pick posts are duplicated in two tables.**
- After `import_legacy_pick_posts`, the same note exists in `club_pick_posts` and `book_posts`. Importing both doubles every old note.
- Conversely, a legacy note posted after the last Python boot is **only** in `club_pick_posts`, and so are reactions added to a legacy note after its mirror.
- The importer should:
  1. mirror unmirrored rows using exactly the §2.3 field mapping;
  2. union the reactions;
  3. carry `milestone_id` across through `legacy_post_id`;
  4. report all three counts in the dry run.

**R6. Ids embedded in JSON.**
- `physical_shelf_layouts.layout_json` holds shelf entry ids (in rows) and `str(id)` keys (in spines).
- `events.payload.post_id` holds `club_pick_posts` ids, not `book_posts` ids.
- If the importer does not **preserve legacy integer ids**, it must rewrite these JSON blobs. The target is guaranteed empty (#38 refuses a non-empty target), so preserving ids is possible: insert with explicit ids, then `setval` each sequence to `max(id)`.

**R7. 404/405 shapes.**
- Python's unknown `/api` GET returns `"Not found"` (lower-case f) when the SPA is mounted and `"Not Found"` otherwise.
- A wrong method can produce 404 instead of 405 because of the GET catch-all.
- Pick one consistent JSON behaviour in Rust and make sure **axum's default 405, which has an empty body,** is also JSON.

**R8. Timestamps.** Python emits naive UTC strings without an offset, except values never reloaded from SQLite, which carry `+00:00`. Emitting RFC 3339 `Z` everywhere changes the display of diary and club dates in non-UTC browsers. It fixes the current bug where `formatClubDate()` treats them as local time. Frontend tests that build fixtures with offset-less strings keep working, because `clubDate` handles both.

**R9. Body size limits.**
- axum's `DefaultBodyLimit` is **2 MB**. Python allows avatar uploads up to 50 MB (auth.md) and the Goodreads CSV has its own cap.
- Set limits per route, and map 413 to JSON `{"detail": ...}`. Keep `"That picture is too large (50 MB max)"` where it applies, because it is mapped in `apiErrors.ts`.

**R10. `test_api_error_copy.py` will break the Python CI during the transition.** It asserts that every `apiErrors.ts` key exists in `backend/app`. As soon as #48 maps a Rust-only string (for example sign-up or email errors), the Python test fails. Either retire it when the frontend switches to the Rust server, or point it at `server/src` and port it to Rust then.

**R11. Exact-substring copy checks are weak.** The drift check passes if the string appears in a comment. A Rust port could tighten it (for example, assert the string appears inside a string literal), but must not loosen it.

**R12. sqlx compile-time macros need a database or offline data.** The scaffold `Dockerfile` builds with no `DATABASE_URL`. Once `query!` is used, the image build fails unless `.sqlx/` (from `cargo sqlx prepare`) is committed and `SQLX_OFFLINE=true` is set. Add `cargo sqlx prepare --check` to CI so the data cannot go stale.

**R13. Startup migrations versus `connect_lazy`.** The scaffold starts without Postgres and serves a 503 health. With `migrate!().run()` at startup, the process cannot serve until Postgres is reachable. Choose retry-with-backoff, or crash-and-restart, deliberately. sqlx's Postgres migrator takes an advisory lock, so parallel instances are safe.

**R14. Test isolation.**
- Python relies on the fixture resetting module-level caches (`clear_search_cache`, `clear_details_cache`, `clear_google_cache`, `reset_rate_limits`) and the VAPID `lru_cache`. Any process-global `static`, `OnceLock` or `lazy_static` cache or limiter in Rust leaks between `#[sqlx::test]` tests that run in parallel. Keep all of it in `AppState`.
- `#[sqlx::test]` creates a database per test. That is why `server/compose.yaml` raises `max_connections` to 500.

**R15. Background work after the response.** Python's push delivery runs after the response is sent, and the tests see it synchronously only because `TestClient` runs background tasks inline. In Rust, `tokio::spawn` from a handler is truly asynchronous, so tests need an injectable sender with a channel or a "flush" hook. (Phase 4, and also email sending [#39].)

**R16. The meeting reminder's one-shot flag is set before targeting.** It is set even when nobody is subscribed or everyone opted out, and moving the meeting doesn't reset it. Both are surprising. Preserve them or change them consciously in Phase 4. The equality edge is that a meeting exactly 24 h away sends.

**R17. Auto-close pushes only from `GET /api/vote`.** Auto-close triggered by a nomination or ballot closes the vote silently, without a push. This is an existing inconsistency to fix (or keep) in Phase 4.

**R18. `brand_index_html` theme-color replacement is dead.** `index.html` now ships `#f7f4ec`. Don't port it as if it worked.

**R19. Migrated versus fresh legacy schema.** In a migrated production DB, `shelf.take` and `shelf.dnf_reason` can be NULL. VARCHAR lengths aren't enforced, so `books.spine_color` or `cover_image_url` can exceed their nominal lengths, and timestamps can lack fractional seconds.
- The importer's SQLite reader must use `Option<String>` with `''` coalescing, and a lenient datetime parser accepting `YYYY-MM-DD HH:MM:SS[.ffffff]` (and tolerating a `T` separator or an offset).
- Values may exceed the new Postgres `VARCHAR(n)` limits. Use `TEXT` with `CHECK`s, or truncate deliberately and report it in the dry run.

**R20. SQLite id reuse in legacy data.**
- `club_pick_posts`, `book_posts` and others can contain reused ids. Events, `legacy_post_id` and layout references to deleted rows may point at a **different, newer** row with the same id.
- `book_posts.legacy_post_id` and `next_up_votes.winner_nomination_id` have no FK, so dangling references are possible.
- The importer should validate these references and report rather than crash.

**R21. Whole-database backup leaks secrets.** Python lets any member download every user's password hash and push keys. The DSGVO export must be scoped to the caller. Do not port "download everything" under any name.

**R22. Old service worker on the legacy origin.**
- Users who installed the app before PR #3 may still run the old `sw.js`, which cached the shell and `/api/shelf`, `/api/pick` and `/api/config`.
- Today `/sw.js` returns the SPA shell (200 `text/html`), so the browser's update check fails and **the old worker stays installed**.
- At cutover, while the old domain redirects [plan], serve a self-unregistering `sw.js` (or a 404) at `/sw.js` on the old origin instead of redirecting it. A redirected SW script fails to update and keeps the stale worker.

**R23. Seed data quirks.** `seed_demo` records `shelf_added` for finished and DNF entries, whereas the real app records `shelf_finished` or `shelf_dnf`. Its `pick_set` event has no `meeting_at` even though the pick has one. Event-mapping code tested only against the seed would miss the real kinds; the fixture generator must add them (§5.8).

**R24. The VAPID key path comes from the env, not settings.** `data_dir_from_env()` reads `DATABASE_PATH` from `os.environ`, not `settings.database_path`. This is irrelevant to Phase 1, but in Phase 4 the key must live in configured, persistent, backed-up storage: losing it invalidates every Web Push subscription. Changing it does too (SELFHOST.md).

---

## 7. Open questions (not settled by plan.md or the Phase 1 issues)

1. **SPA serving.** Does the Rust server serve the built SPA (needs `tower-http` `fs`, a new Vite `outDir`, and a port of `test_spa_cache`), or does the reverse proxy? Who sets the immutable cache headers on `/assets`?
2. **`book_posts` scope.** Are entries with `pick_id` club "pick posts" or personal "diary entries"? Is every legacy `book_posts` row (all club-visible today) imported with the audience visibility?
3. **Legacy ids.** Does the importer preserve legacy integer ids (simplest for `layout_json`, event payloads and `legacy_post_id`), or does the new schema use different keys (UUIDs, remapping)?
4. **Notification state.** Are `notify_*` and `reminder_sent_at` imported and kept unused until Phase 4 (recommended), or dropped and re-added later?
5. **Event log.** Which kinds become club-scoped, and are the personal kinds stored once (with an audience filter) or per club? Are historical events imported at all? #38 lists "events", which may mean this log.
6. **Error contract details.** Keep pydantic's generic texts (`Field required`, …) for parity, or define new generic messages? Confirm 500 becomes JSON. Confirm auth runs before body validation.
7. **`/api/openapi.json`.** Public in production or only in dev? (#47 needs it for type generation in CI only.)
8. **`/api/config` for the hosted service.** Which fields remain, given that name, theme and theme_dark become service constants and `timezone` moves per club or per user? Is `/manifest.webmanifest` kept dynamic?
9. **Legacy invites.** Import unused codes into the imported club as single-use invites, or drop them all?
10. **Importer fixture.** Is the synthetic legacy fixture committed as a small `.db`, or generated in CI by a Python script (which adds Python to the Rust CI job)? Where does the generator live?
11. **`test_api_error_copy.py` during the transition.** When is the Python version retired or redirected (R10)?
12. **Phase 4 reminders.** Should meeting reminders run from a scheduled job (needed at multi-club scale), and are per-club notification mutes wanted?
