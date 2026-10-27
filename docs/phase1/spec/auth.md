# Port spec: auth (accounts, sessions, invites, members, avatars, settings, health)

Sources read in full: `backend/app/routers/{auth,invites,members}.py`, `backend/app/{deps,security,avatars,config,runtime,http,healthcheck,main,db,create_invite,branding,themes,i18n,timezone,serialize,activity,favorites,backup}.py`, `User`/`Invite`/`Event`/`UserFavorite` in `models.py`, the auth schemas in `schemas.py`, `backend/tests/{conftest,test_auth,test_members,test_avatars,test_selfhost}.py`, `frontend/src/i18n/apiErrors.ts`, `frontend/src/api/client.ts`.

Behaviour marked **(probed)** was confirmed by running the Python app (FastAPI 0.162, Starlette 1.10, argon2-cffi 25.1, Pillow 13) against a scratch database, not just read from source.

Library versions matter here: Starlette 1.10's `SessionMiddleware` only re-sends the cookie when the session is modified (see 2.1).

---

## 1. Endpoints

### 1.0 Conventions shared by every endpoint in this area

**Error body.** Every error is `{"detail": "<English string>"}`. The frontend (`api/client.ts`) reads `detail`. When it is a string, `translateApiError()` maps the **exact** string to an i18n key. Strings in the mapping below must stay byte-identical, or `apiErrors.ts` must change with them. `backend/tests/test_api_error_copy.py` enforces this today, and the Rust side needs an equivalent check.

Mapped strings in this area (from `apiErrors.ts`):
`"Not signed in"`, `"That username is already taken"`, `"That invite code is not valid"`, `"Too many attempts. Wait a few minutes."`, `"Wrong username or password"`, `"Current password is wrong"`, `"Pick a different password"`, `"That picture is too large (50 MB max)"`, `"That picture has too many pixels"`, `"Use a JPEG, PNG, WebP, or GIF"`, `"That file is not a picture"`, `"No profile picture"`, `"That book is not in the club yet"`, `"Each book can only be a favourite once"`, `"At most 3 favourites"`, `"You already have 3 favourites"`.

**Validation errors** (`main.py: validation_handler`) return status **400**, never 422. Every pydantic error message is collected in field-declaration order. A leading `"Value error, "` is stripped, and the messages are joined with `"; "`. Generic pydantic messages seen in this area **(probed)**:
- missing field: `"Field required"` (one per missing field, e.g. `"Field required; Field required; Field required"` for an empty register body)
- request with no body at all where a body model is required: `"Field required"`
- malformed JSON: `"JSON decode error"`
- non-string where a string is expected: `"Input should be a valid string"`
- body is a JSON array instead of an object: `"Input should be a valid dictionary or object to extract fields from"`
- non-boolean where a bool is expected: `"Input should be a valid boolean, unable to interpret input"`
- non-integer path param: `"Input should be a valid integer, unable to parse string as an integer"`

Unknown JSON keys are ignored (pydantic default `extra="ignore"`).

**Order of checks on an authenticated JSON endpoint (probed):**
1. JSON parse error, which returns 400 `"JSON decode error"` even when the request is not signed in.
2. Auth (`get_current_user`): 401 `"Not signed in"`.
3. Field validation: 400.
4. Handler logic.

So a signed-out request with a valid-JSON but invalid body gets **401**, not 400.

**Auth dependency (`deps.get_current_user`):**
- It reads `user_id` from the signed session cookie. Missing or falsy means 401 `"Not signed in"`.
- It loads the user by id. If the row is gone (user deleted), it clears the session, so the response carries a deletion `Set-Cookie`, and returns 401 `"Not signed in"` **(probed)**.
- A tampered or expired cookie fails the signature check. The session is then treated as empty: 401, with **no** `Set-Cookie` **(probed)**.
- The avatar blob is never loaded with the user (`defer(User.avatar)`). It is a performance detail only.

**UserOut** (returned by register, me, preferences, avatar PUT/DELETE, and embedded as `user` in `GET /api/shelf?username=`):
```json
{"id": int, "username": str, "theme": str, "color_mode": str, "locale": str, "avatar_url": str|null}
```
- Field order is as shown.
- `locale` is `user.locale or "en"`.
- `avatar_url` follows the rule in 2.6.
- The response has no `avatar`, `password_hash`, `created_at` or notification fields. A test asserts `"avatar" not in body`.

---

### 1.1 `POST /api/auth/register`: create an account with an invite code

- **Auth:** none. Calling it while signed in is allowed: it creates the new user and **switches** the session to that user **(probed)**.
- **Body (`RegisterIn`)**: `{"username": str, "password": str, "invite_code": str}`. All three are required strings.
  - `username`: `value.strip().lower()`, then it must fully match `^[a-z0-9_]{2,32}$`. Otherwise the error is `"Username must be 2–32 characters: lowercase letters, numbers, underscore"`. The character between 2 and 32 is an **EN DASH U+2013**, not a hyphen.
  - `password`: `len(value) >= 8`, counted in **Unicode code points** (`"pässwörd"` is accepted). Otherwise the error is `"Password must be at least 8 characters"`. There is no maximum, and the password is not stripped.
  - `invite_code`: any string; it is not validated.
  - Errors from username and password are joined: `"Username must be 2–32 characters: lowercase letters, numbers, underscore; Password must be at least 8 characters"` **(probed)**.
- **Logic (order matters):**
  1. If a user with the normalized username exists, return **409** `"That username is already taken"`. This runs **before** the invite check, so a taken name with a bad invite gives 409 **(probed)**.
  2. `code = normalize_invite_code(invite_code)`: remove every whitespace character (Python `\s`, Unicode-aware) and every `-`, then `.upper()`.
  3. Insert `users` row: username, `password_hash = argon2id(password)`, defaults theme `paper`, color_mode `system`, locale `en`, notify_* true, created_at now (UTC). Flush to get the id.
  4. Run an atomic conditional update: `UPDATE invites SET used_by_id=:new_id, used_at=now WHERE code=:code AND used_by_id IS NULL`. If the rowcount is not exactly 1, **roll back** (no user row survives) and return **400** `"That invite code is not valid"`. This covers both an unknown code and an already-used one.
  5. Insert `events` row `kind="member_joined"`, `actor_id=new_id`, `book_id=NULL`, `pick_id=NULL`, `payload="{}"` (the same transaction).
  6. Commit.
  7. Session: clear, then set `{"user_id": id, "username": username}`, which issues the `Set-Cookie` (see 2.1).
- **Response:** **201** UserOut, e.g. `{"id":1,"username":"ada","theme":"paper","color_mode":"system","locale":"en","avatar_url":null}`.
- **Concurrency:** two simultaneous registrations with one invite give exactly one 201 and one 400, and only one user exists afterwards. A concurrent registration of the **same username** with two valid invites hits the unique index on flush. Python returns an unhandled IntegrityError (500) there, and the port should return 409.
- **Not done:** no rate limit on register, no email, no auto-login throttle reset.

### 1.2 `POST /api/auth/login`

- **Auth:** none.
- **Body (`LoginIn`)**: `{"username": str, "password": str}`, both required.
  - `username` is `strip().lower()` with **no** regex.
  - `password` has no rules.
- **Logic:**
  1. If `too_many_failures(username)`, return **429** `"Too many attempts. Wait a few minutes."`. This is checked **before** verifying, so the right password is also refused while blocked **(probed)**.
  2. Look up the user by normalized username. If there is no user, or `verify_password` is false (including an unparsable hash), call `record_failure(username)` and return **401** `"Wrong username or password"`. Unknown usernames also accumulate failures **(probed)**.
  3. On success, `clear_failures(username)` and set the session (clear, then `user_id` and `username`).
- **Response:** **204**, empty body, with `Set-Cookie: bookclub_session=...`.

### 1.3 `POST /api/auth/logout`

- **Auth:** none.
- **Body:** none.
- **Logic:** `request.session.clear()`.
- **Response:** **204**.
  - If the request carried a valid session cookie, the response includes `Set-Cookie: bookclub_session=null; path=/; expires=Thu, 01 Jan 1970 00:00:00 GMT; httponly; samesite=lax` (plus `; Secure` per 2.1).
  - If there was no valid session, there is no `Set-Cookie` **(probed)**.

### 1.4 `GET /api/auth/me`

- **Auth:** required.
- **Response:** 200 UserOut.
- **Errors:** 401 `"Not signed in"`.

### 1.5 `PATCH /api/auth/me/preferences`

- **Auth:** required (401 comes before validation, **probed**).
- **Body (`PreferencesIn`)** is **required**. A request with no body returns 400 `"Field required"` **(probed)**, while `{}` is a valid no-op. All fields are optional and nullable; `null` or absent means unchanged.
  - `theme`: `strip().lower()`, must be in `("paper","slate","forest","ink")`. Error: `"theme must be one of: paper, slate, forest, ink"`.
  - `color_mode`: `strip().lower()`, must be in `("light","dark","system")`. Error: `"color_mode must be one of: light, dark, system"`.
  - `locale`: `strip().lower()`, must be in `("en","de")`. Error: `"locale must be one of: en, de"`.
  - Multiple invalid fields are joined with `"; "` in the order theme, color_mode, locale **(probed)**.
  - A non-string value gives `"Input should be a valid string"`.
- **Logic:** set each non-null field, then commit. Validation failure changes nothing.
- **Response:** 200 UserOut.

### 1.6 `PUT /api/auth/me/avatar`: upload a profile picture

- **Auth:** required. Auth is checked before everything below.
- **Request:** `multipart/form-data` with a **file** part named `file`.
- **Logic, in order:**
  1. If the `Content-Length` header parses as an int and is greater than `50*1024*1024 + 4096` (52,432,896), return **413** `"That picture is too large (50 MB max)"` **before reading the body**. Exactly 52,432,896 is allowed. A missing or unparsable header skips this check.
  2. Parse the form. If `file` is missing or is a plain text field rather than an uploaded file, return **400** `"Use a JPEG, PNG, WebP, or GIF"`. A JSON body also gives this message **(probed)**.
  3. Read up to `MAX_UPLOAD_BYTES + 1` bytes (MAX = 52,428,800).
  4. Run `process_avatar(raw)` (see 2.5). Any `ValueError` becomes **400** with its message:
     - `"That picture is too large (50 MB max)"`: the file part is over 50 MiB. This is a **400** here, not 413.
     - `"Use a JPEG, PNG, WebP, or GIF"`: under 24 bytes, or magic bytes not JPEG/PNG/GIF/WebP. An SVG gets this message.
     - `"That file is not a picture"`: magic bytes OK but the image is undecodable (e.g. a truncated PNG, **probed**).
     - `"That picture has too many pixels"`: width×height is over 25,000,000, or Pillow raises a decompression-bomb error.
  5. Store `users.avatar = <jpeg bytes>`, `avatar_mime = "image/jpeg"`, `avatar_updated_at = now (UTC)`, then commit.
- **Response:** 200 UserOut whose `avatar_url` is `"/api/members/<username>/avatar?v=<epoch seconds>"`.
- **Failure leaves the old avatar untouched.**

### 1.7 `DELETE /api/auth/me/avatar`

- **Auth:** required.
- **Logic:** set `avatar`, `avatar_mime` and `avatar_updated_at` to NULL, then commit. This is idempotent: it returns 200 even if there was no avatar.
- **Response:** 200 UserOut with `avatar_url: null`.

### 1.8 `PATCH /api/auth/password`: change the password

The backend keeps this endpoint, but no frontend calls it any more (the form was removed in PR #3).

- **Auth:** required (401 comes before validation, **probed**).
- **Body (`PasswordChangeIn`)**: `{"current_password": str, "new_password": str}`, both required.
  - `new_password` must be at least 8 code points. Error: `"New password must be at least 8 characters"`.
  - `current_password` has no rules.
- **Logic, in order:**
  1. If `too_many_failures(user.username)`, return **429** `"Too many attempts. Wait a few minutes."`. This is the **same bucket as login**.
  2. If `verify_password(current, hash)` is false, call `record_failure(user.username)` and return **400** `"Current password is wrong"`. This takes precedence over the "same password" check **(probed)**.
  3. If `current_password == new_password`, return **400** `"Pick a different password"`. No failure is recorded and failures are not cleared.
  4. Call `clear_failures(user.username)`, set `password_hash = argon2id(new)`, then commit.
- **Response:** **204**. The session is not rotated and other sessions are not revoked; with a stateless cookie the server cannot revoke them.

### 1.9 `GET /api/auth/notifications`

- **Auth:** required.
- **Response:** 200 `{"notify_meeting": bool, "notify_pick": bool, "notify_note": bool}`.
- **Frontend:** no current caller (Web Push UI removed). `push.py` reads these columns through `PREF_FOR_KIND` (meeting/pick/note; `test` ignores prefs).

### 1.10 `PATCH /api/auth/notifications`

- **Auth:** required.
- **Body:** `{"notify_meeting"?: bool|null, "notify_pick"?: bool|null, "notify_note"?: bool|null}`. `null` or absent means unchanged.
- **Coercion:** pydantic lax bool, so `"no"`, `"YES"`, `0`, `1`, `"true"`, `"off"` and similar are accepted **(probed)**. Garbage gives 400 `"Input should be a valid boolean, unable to interpret input"`.
- **Response:** 200 with the same shape as 1.9, after the update.

### 1.11–1.14 Favourites (mounted in the auth router; logic in `favorites.py`)

These are personal features, and their tests live in `test_favorites.py`, which another spec owns. They are listed here because they share the `/api/auth/me` prefix. The routes:
- `GET /api/auth/me/favorites`
- `PUT /api/auth/me/favorites`
- `POST /api/auth/me/favorites/{book_id}`
- `DELETE /api/auth/me/favorites/{book_id}`

**Common to all four:**
- Auth required.
- Response 200 `{"items": [{"position": int, "book": BookOut}]}`, ordered by `(position, id)`.
- `book_id` is an int path param; a non-int gives 400 `"Input should be a valid integer, unable to parse string as an integer"`.

**PUT** body `{"book_ids": [int]}` (default `[]`):
- Validator errors:
  - more than 3 ids: `"At most 3 favourites"`
  - duplicates: `"Each book can only be a favourite once"`
  - any id ≤ 0: `"That book is not in the club yet"`
- Any id not in `books`: 400 `"That book is not in the club yet"`.
- The list then replaces all favourites, with positions 1..n in list order.

**POST:**
- Unknown book: 400 `"That book is not in the club yet"`.
- Already a favourite: a no-op that returns the current list.
- Already at 3 favourites: **409** `"You already have 3 favourites"`.
- Otherwise the book is appended at position n+1. A unique-constraint race is treated the same way.

**DELETE:**
- Not a favourite: a no-op.
- Otherwise the remaining favourites are renumbered 1..n in their existing order.

Favourites do not require the book to be on the user's own shelf.

### 1.15 `POST /api/invites`: mint an invite

- **Auth:** required. **Any** signed-in user may mint; there are no roles.
- **Body:** none.
- **Logic:** `code = generate_invite_code()` (10 chars from the Crockford alphabet `0123456789ABCDEFGHJKMNPQRSTVWXYZ`, using `secrets.choice`). Regenerate while the code already exists, then insert `invites(code, created_by_id=user.id, created_at=now)`.
- **Response:** **201** `{"code": "HT78ATRS04"}`.
- **Limits:** no limit on how many invites a user mints, no expiry, no revocation endpoint.

### 1.16 `GET /api/invites`: list my invites

- **Auth:** required.
- **Logic:** select invites `WHERE created_by_id = me ORDER BY created_at DESC`. Ties have no secondary key.
  - The bootstrap or CLI invite (`created_by_id NULL`) is never listed.
  - Usernames of redeemers are resolved in one batch.
- **Response:** 200 `[{"code": str, "used": bool, "used_by": str|null, "created_at": datetime}]`.
  - `used` is `used_by_id IS NOT NULL`.
  - `used_by` is the redeemer's username, or null when unused or the redeemer row is gone.
  - `created_at` is serialized **without an offset**, because SQLite returns naive datetimes: `"2026-10-27T10:46:19.453446"` **(probed)**. The value is UTC.
- **Not returned:** `used_at` is never returned.

### 1.17 `GET /api/members`: everyone else in the (single) club

- **Auth:** required.
- **Logic:** all users **except the caller**, `ORDER BY username` (SQLite BINARY, i.e. byte order). For each member, select `shelf` rows with `status='currently_reading'`, `ORDER BY position, id`.
  - `currently_reading_count` is the total count.
  - The preview is the first 3 of those rows whose book exists.
- **Response:** 200 list of:
```json
{"username": str,
 "currently_reading_count": int,
 "currently_reading_preview": [{"ol_work_key": str, "title": str, "cover_id": int|null, "cover_url": str|null}],
 "avatar_url": str|null}
```
- `ol_work_key` is the stored `/works/<id>` key, e.g. `/works/OL1W`.
- `cover_url` comes from `serialize.cover_url`:
  1. Google `cover_image_url` if storable.
  2. Otherwise, when `cover_id > 0`, `https://covers.openlibrary.org/b/id/<id>-L.jpg?default=false`.
  3. Otherwise, in Open Library mode only, `https://covers.openlibrary.org/b/isbn/<isbn>-L.jpg?default=false` for `ISBN…` keys.
  4. Otherwise null.
- **Pagination:** none. The Python implementation runs N+1 queries.

### 1.18 `GET /api/members/{username}/avatar`: serve a profile picture

- **Auth:** required. A signed-out request gets 401 even with a valid URL; the same URL returns 200 after login.
- **Path:** `username` is lowercased and not otherwise validated. Query `v` is ignored; it only busts caches.
- **Not found:** if the user does not exist, or the `avatar` blob is NULL or empty, return **404** `"No profile picture"`. Unknown user and no-avatar give the same response.
- **Headers on both 200 and 304:**
  - `Cache-Control: private, max-age=86400`
  - `X-Content-Type-Options: nosniff`
  - `ETag: "<int(avatar_updated_at.timestamp())>"` when `avatar_updated_at` is not null.
- **304:** if the `If-None-Match` header **exactly equals** the ETag string, the response is **304** with an empty body and the headers above. Weak tags and lists are not parsed.
- **200:** otherwise **200** with the raw bytes and `Content-Type: <avatar_mime or "image/jpeg">`. That is always `image/jpeg` in practice.
- **ETag quirk (probed):** the ETag is computed from a naive datetime, which Python interprets as **server local time**. The URL `v=` treats the same value as UTC. On a CET host they differ by 3600 (`v=1793097979`, `ETag "1793094379"`). Only self-consistency of the ETag matters.

### 1.19 `GET /api/health`

- **Auth:** none.
- **Logic:** run `SELECT 1` on a fresh session.
- **Responses:**
  - 200 `{"ok": true}`.
  - On any exception, **503** `{"detail": "database unavailable"}`, with the exception logged.
- **No timeout:** unlike the Rust scaffold, Python sets no timeout.
- **Rust scaffold status:** `server/src/health.rs` already returns 200 `{"status":"ok"}`, or 503 `{"status":"unavailable"}` with a 2 s timeout. That is a deliberate change from Python (see section 5).

### 1.20 `GET /api/config`: public branding

- **Auth:** none.
- **Response:** 200 `{"name": str, "theme": str, "theme_dark": str, "public_url": str, "timezone": str}`. The fields are built as follows:
  - `name`: `sanitize_name(BOOKCLUB_NAME)`. It strips, removes non-printable characters and `<` `>`, collapses whitespace, and truncates to 48 chars. If the result is empty it is `"Bookclub"`.
  - `theme`: `BOOKCLUB_THEME` if it matches `^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6})$`.
    - A 4-char form is lowercased; a 7-char form keeps its case.
    - Anything else falls back to `"#b44a2a"`, with a warning logged once at startup: `"Invalid BOOKCLUB_THEME %r; using default %s"`.
  - `theme_dark`: `BOOKCLUB_THEME_DARK` if valid. Otherwise `"#8e361c"` when the accent is the default, or else `darken_hex(accent, 0.78)`: each channel is multiplied by 0.78 and truncated with `int()`, giving lowercase hex. An invalid value logs `"Invalid BOOKCLUB_THEME_DARK %r; ignoring"`.
  - `public_url`: the scheme://netloc origin of `BOOKCLUB_PUBLIC_URL`, or `""`.
  - `timezone`: `BOOKCLUB_TZ` if it is a valid IANA zone, else `"UTC"`.
- **Example (probed):** `{"name":"Bookclub","theme":"#b44a2a","theme_dark":"#8e361c","public_url":"","timezone":"UTC"}`.

### 1.21 `GET /manifest.webmanifest`

- **Auth:** none.
- **Content-Type:** `application/manifest+json`.
- **Body:**
  - `name`: the config name.
  - `short_name`: `name[:32]`.
  - `description`: `"A tiny private bookclub: one pick, everyone's shelves, a vote for what's next."`
  - `start_url`: `"/"`.
  - `scope`: `"/"`.
  - `display`: `"standalone"`.
  - `orientation`: `"portrait"`.
  - `background_color`: `"#f6f1e8"`.
  - `theme_color`: the config theme.
  - `icons`: `[{src:"/icons/icon-192.png",sizes:"192x192",type:"image/png",purpose:"any"},{src:"/icons/icon-512.png",sizes:"512x512",...,purpose:"any"},{src:"/icons/icon-512.png",sizes:"512x512",...,purpose:"maskable"}]`.
- **Related branding:** `brand_index_html` rewrites `<title>Bookclub</title>`, the `apple-mobile-web-app-title` meta and `theme-color` meta in the served `index.html`.

### 1.22 `GET /api/docs`, `GET /api/openapi.json`

- Mounted only when `settings.debug`; otherwise 404. ReDoc is never mounted.
- The FastAPI title is the sanitized club name.

### 1.23 Non-HTTP entry points in this area

- **Startup (`create_app`)** runs these steps in order:
  1. `prepare_environment()` (2.8).
  2. `get_settings.cache_clear()`, then `get_settings()`.
  3. `warn_invalid_theme`.
  4. Log `"catalog: Google Books"` or `"catalog: Open Library"`.
  5. `init_db` (create tables, `COLUMN_MIGRATIONS`, legacy pick-post import).
  6. `ensure_bootstrap_invite`.
  7. Add the middleware (2.9).
  8. Include the routers.
- **`ensure_bootstrap_invite(session, settings)`:**
  - Only acts if the `invites` table has **zero rows**.
  - `code = normalize_invite_code(BOOKCLUB_BOOTSTRAP_INVITE)`. If it is empty, it raises `RuntimeError("BOOKCLUB_BOOTSTRAP_INVITE is empty")`. If not debug and the code is `"DEVONLY"`, it raises `RuntimeError("Set BOOKCLUB_BOOTSTRAP_INVITE to a secret code when DEBUG=0")`.
  - It inserts `Invite(code, created_by_id=NULL)`. In debug, `DEV-ONLY` is therefore stored as `DEVONLY` and can be redeemed as `dev-only`, `DEV ONLY` and so on.
- **`python -m app.create_invite`:** runs `prepare_environment`, then `init_db`, generates a unique 10-char code, inserts it with `created_by_id=NULL`, and prints it.
- **`python -m app.healthcheck`:** the Docker `HEALTHCHECK` (`gosu bookclub python -m app.healthcheck`, every 30 s, timeout 5 s, start 20 s, 3 retries). It GETs `http://127.0.0.1:8000/api/health` with a 4 s timeout and exits 0 only if the status is 200 **and** the body contains the bytes `"ok"` (with quotes). It exits 1 on any URL, timeout or OS error.
- **`python -m app.backup [dest]`:** this belongs to another area, but `test_selfhost.py` covers it. It runs the SQLite online backup into `dest`, or `<data>/bookclub-YYYYMMDDTHHMMSSZ.db` by default.

---

## 2. Domain rules

### 2.1 Session cookie (Starlette `SessionMiddleware` 1.10 plus `AutoSecureCookieMiddleware`)

- **Cookie name:** `bookclub_session`.
- **Value:** `base64(json.dumps(session))` signed with itsdangerous `TimestampSigner(SECRET_KEY)`, e.g. `eyJ1c2VyX2lkIjogMSwgInVzZXJuYW1lIjogImFkYV8xIn0=.auCA-w.<sig>`.
- **Payload:** `{"user_id": int, "username": str}`. The server only ever reads `user_id`.
- **Attributes (probed):** `path=/; Max-Age=2592000; httponly; samesite=lax`, plus `; Secure` under the rules below. No Domain, no Partitioned.
- **Lifetime:** 30 days, enforced twice: by the cookie `Max-Age` and server-side by `unsign(max_age=2592000)`.
- **Not sliding:** the cookie is **only re-issued when the session is modified**, which happens only at login, register and logout. A session therefore dies exactly 30 days after login, however active the user is.
- **Clearing:** a clear on a session that was non-empty on entry emits `bookclub_session=null; path=/; expires=Thu, 01 Jan 1970 00:00:00 GMT; httponly; samesite=lax`. A clear on an empty or invalid session emits nothing.
- **Side effects:** responses that touched the session add `Vary: Cookie`.
- **Secure flag (`BOOKCLUB_HTTPS`, `runtime.https_mode`):**
  - `always`: the middleware sets `https_only`, so every Set-Cookie carries `; secure`. Accepted values are `1/true/yes/on/always`, or a bool True.
  - `never`: never Secure. Accepted values are `0/false/no/off/never`, or a bool False.
  - `auto`: anything else; this is the default. `AutoSecureCookieMiddleware` appends `; Secure` to a `Set-Cookie` that starts with `bookclub_session=` when the **effective request scheme** is `https` and the header doesn't already contain `; secure` or end with `secure`. The scheme comes from uvicorn `ProxyHeadersMiddleware`, which applies `X-Forwarded-Proto` only when the client IP is in `BOOKCLUB_TRUSTED_PROXIES`.
- **No server-side session store:** logout, password change and deletion cannot revoke other copies of a cookie.

### 2.2 Password hashing

- **Hashing:** argon2-cffi `PasswordHasher()` defaults, i.e. **argon2id, v=19, m=65536 KiB, t=3, p=4, hash_len=32, salt_len=16**. The PHC string looks like `$argon2id$v=19$m=65536,t=3,p=4$<b64 salt>$<b64 hash>`.
- **Verification:**
  - Parameters and variant are read from the stored string.
  - `VerifyMismatchError`, `VerificationError` and `InvalidHashError` all map to "false"; garbage such as `'hash'` counts as a wrong password, not an error.
  - There is no rehash-on-login (`check_needs_rehash` is unused).
- **Timing leak:** login for an unknown username does **not** run a dummy hash, so the response time differs.

### 2.3 Failure rate limit (`security.py`)

- **Storage:** in-process `dict[str, list[float]]`, keyed by **normalized username**: the login form value after strip+lower, or `user.username` for a password change. Login and password change **share a bucket**.
- **Window:** 600 s, measured with `time.monotonic()`. The limit is 10.
- **`too_many_failures(key)`:** drops stamps 600 s or older, then returns `len(recent) >= 10`. The window is therefore sliding: one slot frees when the oldest failure ages out.
- **What records and clears:**
  - `record_failure` appends `now`. It is called only on a wrong login or a wrong current password. A 429 does **not** record.
  - `clear_failures` deletes the key on a successful login or a successful password change.
- **Result:** attempts 1–10 return 401 (or 400 for a password change), and attempt 11 returns 429.
- **Scope:**
  - Per process and lost on restart.
  - Unbounded keys: any string sent as a username creates an entry.
  - There is no per-IP limit. Anyone can lock any username out for 10 minutes.
- **Tests:** `reset_rate_limits()` clears it, and the `client` fixture resets it.
- **Not limited:** register, invite minting and avatar upload have no rate limits.

### 2.4 Invite codes

- **Generated codes:** 10 characters from Crockford base32 (`0-9A-Z` without `I L O U`), roughly 50 bits.
- **Normalization:** when stored (bootstrap only) and when redeemed, normalization strips whitespace and `-`, then uppercases. There is **no** Crockford decoding: `O` is not mapped to `0`, and `I`/`L` are not mapped to `1`.
- **Lifecycle:** single-use, by setting `used_by_id`/`used_at` atomically. There is no expiry, no revocation and no deletion.
- **Bootstrap:** exactly one bootstrap invite exists on a fresh DB (created_by NULL). Later invites come from users or the CLI.
- **Ownership:** any user can mint unlimited invites, and only their own minted invites appear in their list.

### 2.5 Avatar processing (`avatars.process_avatar`)

Constants:
- `MAX_UPLOAD_BYTES = 52_428_800`
- `UPLOAD_LENGTH_SLACK = 4096`
- `MAX_SOURCE_PIXELS = 25_000_000`
- `OUTPUT_SIZE = 256`
- `JPEG_QUALITY = 85`
- `AVATAR_MIME = "image/jpeg"`

Steps, in order:
1. If `len(data) > MAX_UPLOAD_BYTES`, fail with TOO_LARGE.
2. If `len(data) < 24` or the magic bytes don't match, fail with UNSUPPORTED_TYPE. Accepted magic:
   - JPEG `FF D8 FF`
   - PNG `89 50 4E 47 0D 0A 1A 0A`
   - `GIF87a` / `GIF89a`
   - WebP: `RIFF` at 0..4 and `WEBP` at 8..12
3. Open the image to read the header without decoding. A decompression-bomb error fails with TOO_MANY_PIXELS. Unidentified or OS errors fail with NOT_A_PICTURE.
4. If width or height is ≤ 0, fail with NOT_A_PICTURE. If `w*h > 25_000_000`, fail with TOO_MANY_PIXELS. This is checked **before** decoding: a 6000×6000 PNG is small on disk but rejected.
5. Decode fully (`load()`). A bomb error fails with TOO_MANY_PIXELS; unidentified, OS or ValueError fails with NOT_A_PICTURE.
6. Apply the EXIF orientation transpose.
7. For multi-frame images (animated GIF/WebP), use frame 0.
8. Convert colour modes:
   - Modes `P`, `RGBA`, `LA` and `PA`: convert to RGBA and composite over an opaque **white** background using alpha.
   - Anything else (including CMYK and 16-bit): `convert("RGB")`.
9. Centre-crop to a square: `side = min(w,h)`, `left = (w-side)//2`, `top = (h-side)//2`, using floor division.
10. Resize to 256×256 with **Lanczos**.
11. Encode as JPEG, quality 85, `optimize=True`.

No EXIF, ICC or other metadata is written to the output, so GPS is stripped implicitly.

The output is always a 256×256 baseline JPEG that starts with `FF D8 FF`.

### 2.6 Avatar URL

- `avatar_url_for(user)` is null if the username is empty or **`avatar_updated_at` is NULL**. This holds even if a blob exists; that can only happen through manual DB edits.
- Otherwise it is `"/api/members/{username}/avatar?v={int(epoch seconds of avatar_updated_at, naive treated as UTC)}"`.
- The URL is reused in UserOut, MemberOut, the diary (`author_avatar_url`), pick readers (`pick_ops`) and book readers (`books.py`).
- Re-uploading within the same second yields the same `v`, so the browser may show the stale image for up to the 1-day `max-age`.

### 2.7 Preferences allowlists

- `THEME_IDS = ("paper","slate","forest","ink")`, default `paper`.
- `COLOR_MODES = ("light","dark","system")`, default `system`.
- `LOCALES = ("en","de")`, default `en`.
- These are per-user UI preferences, unrelated to the instance accent colour `BOOKCLUB_THEME`.

### 2.8 Settings and runtime

**Environment variables** (pydantic-settings, case-insensitive, `extra="ignore"`):

| Var | Default | Meaning |
|---|---|---|
| `SECRET_KEY` | `dev-secret-change-me` | Session signing key |
| `DEBUG` | **true** | Docs, dev CORS, and skip production checks |
| `BOOKCLUB_HTTPS` | `auto` | Cookie Secure mode (2.1) |
| `BOOKCLUB_BOOTSTRAP_INVITE` | `DEV-ONLY` | First invite on an empty DB |
| `DATABASE_PATH` | `<repo>/data/bookclub.db` | SQLite file; its parent is the data dir |
| `BOOKCLUB_TZ` | `UTC` | Club timezone; invalid falls back to UTC |
| `BOOKCLUB_TRUSTED_PROXIES` | `*` | Comma list of hosts allowed to set `X-Forwarded-*`. `*` or `all` means everyone; an empty list also means `*` |
| `BOOKCLUB_NAME` | `Bookclub` | Branding |
| `BOOKCLUB_THEME` / `BOOKCLUB_THEME_DARK` | `#b44a2a` / `""` | Branding accent |
| `BOOKCLUB_PUBLIC_URL` | `""` | Validated: stripped; empty is allowed; otherwise it must be http(s) with a host, else `ValueError("BOOKCLUB_PUBLIC_URL must be an http(s) URL like https://books.example.com")`. Reduced to scheme://netloc (a path is dropped) |
| `VAPID_PRIVATE_KEY`, `VAPID_SUBJECT` | `""` | Web Push |
| `GOOGLE_BOOKS_API_KEY` | `""` | Catalog provider switch |

**Dotenv:**
- `apply_dotenv()` copies keys from `BOOKCLUB_ENV_FILE`, or `<repo>/.env` by default, into `os.environ` **only if unset**; the process env wins. A key with no value becomes `""`.
- `get_settings()` is `lru_cache`d and calls `apply_dotenv()` first.

**`DEBUG` is parsed twice:**
- `runtime.debug_from_env()`: unset means True; otherwise `strip().lower() in {"1","true","yes","on"}`.
- `Settings.debug`: pydantic bool.
- The two can disagree for values such as `t`/`y`, and garbage crashes pydantic.

**`prepare_environment()`** (called at app start and by the CLIs):
- If debug, it returns at once with `{"debug": True, "generated_secret": False, "generated_invite": False}`.
- **Data dir:** the parent of the resolved `DATABASE_PATH`, or `<repo>/data`.
- **`explicit_env(name)`:** None if the var is unset **or** whitespace-only.
- **Secret key:**
  - If `SECRET_KEY` is not explicit, it reads or creates `<data>/.secret_key` with `secrets.token_urlsafe(48)` (64 chars). If the content is a placeholder, it raises `RuntimeError("SECRET_KEY is missing or too weak. Set a long random SECRET_KEY (at least 32 characters) or make the data directory writable so one can be generated.")`. Otherwise it sets `os.environ["SECRET_KEY"]`.
  - If it is explicit but a placeholder, it raises `RuntimeError("SECRET_KEY must be a long random string when DEBUG=0 (at least 32 characters). Do not use the example value.")`.
- **Placeholder secret:** None, or stripped-lowercased in `{"", "dev-secret-change-me", "change-me-to-a-long-random-string", "changeme", "secret"}`, or stripped length < 32.
- **Bootstrap invite:**
  - If not explicit, it reads or creates `<data>/.bootstrap_invite` with `generate_invite_code()`. A placeholder content raises `RuntimeError("Set BOOKCLUB_BOOTSTRAP_INVITE to a secret code when DEBUG=0, or make the data directory writable so one can be generated.")`. Otherwise it sets the env var, and if the file was created it prints the banner and logs `"Generated bootstrap invite %s"`. The banner is:
    ```
    ============================================================
    <name> first-run invite (share once, then mint more):
      <code>
    ============================================================
    ```
  - If it is explicit but a placeholder, it raises `RuntimeError("BOOKCLUB_BOOTSTRAP_INVITE cannot be DEV-ONLY when DEBUG=0. Set a secret code, or leave it empty to generate one.")`.
- **Placeholder invite:** None, blank, or normalizes to `DEVONLY`.
- **`read_or_create_file`:**
  - Reads the stripped content if the file exists and is non-empty.
  - Otherwise it mkdirs the parent, writes `value + "\n"` and chmods 600. A chmod failure only logs a warning mentioning NFS/CIFS.
  - Returns `(value, created)`.

### 2.9 Middleware stack (outermost first)

1. `CORSMiddleware`, only if `cors_origins()` is non-empty.
   - Origins: in debug, `http://localhost:5173` and `http://127.0.0.1:5173`, plus the public origin if set.
   - `allow_credentials=True`, all methods, all headers.
   - Preflights from other origins get no `access-control-allow-origin`.
2. `ProxyHeadersMiddleware(trusted_hosts)`: applies `X-Forwarded-Proto` and `X-Forwarded-For` from trusted clients.
3. `AutoSecureCookieMiddleware`, only in `auto` mode.
4. `SessionMiddleware`.
5. The routes.

### 2.10 Open Library User-Agent (`branding.open_library_ua`)

- Format: `"{token}/1.0 ({contact}; catalog browse via Open Library)"`.
  - `token` is the sanitized name with `[^A-Za-z0-9._-]` removed, or `Bookclub` if that leaves nothing.
  - `contact` is the public origin, or `"private book club"`.
- Example: `ThursdayReaders/1.0 (https://books.example.com; catalog browse via Open Library)`.
- A Google variant (`catalog search via Google Books`) exists and is dropped by the plan.
- `app/http.py` holds only the ASGI cookie middleware; there is no outbound HTTP client helper in this area.

### 2.11 Time handling

- All timestamps are written as aware UTC (`utcnow()`). SQLite stores them as naive text and returns naive datetimes.
- Serialized datetimes therefore have **no offset** (`InviteOut.created_at`).
- Code that needs epoch seconds treats the naive value as UTC (`avatar_url_for`). The exception is the avatar ETag, which treats it as local time.

### 2.12 Activity

- `register` writes the only `member_joined` event.
- `seed_demo` also writes one per demo user.
- The activity feed (another area) shows it.

---

## 3. Data

### 3.1 `users` (SQLite DDL as created today)

| Column | Type | Semantics |
|---|---|---|
| `id` | INTEGER PK | Autoincrement. Referenced by almost every table (shelf, favourites, posts, reactions, events, picks, votes, ballots, quotes, push, search history, invites) |
| `username` | VARCHAR(32) NOT NULL, unique index `ix_users_username` | Always stored lowercase and matching `^[a-z0-9_]{2,32}$` (enforced only at register; SQLite doesn't enforce the length). The display name *and* the login identifier *and* the URL key for other users' data |
| `password_hash` | VARCHAR NOT NULL | argon2 PHC string (2.2). Test fixtures may contain non-PHC junk such as `'hash'` |
| `theme` | VARCHAR(32) NOT NULL DEFAULT 'paper' | One of THEME_IDS |
| `color_mode` | VARCHAR(16) NOT NULL DEFAULT 'system' | One of COLOR_MODES |
| `locale` | VARCHAR(8) NOT NULL DEFAULT 'en' | One of LOCALES. Read with `or "en"` |
| `created_at` | DATETIME NOT NULL | Naive UTC text, `YYYY-MM-DD HH:MM:SS[.ffffff]`. Never exposed by the API |
| `notify_meeting`, `notify_pick`, `notify_note` | BOOLEAN NOT NULL DEFAULT 1 | Web Push opt-outs per kind |
| `avatar` | BLOB NULL | 256×256 JPEG bytes. NULL means no avatar |
| `avatar_mime` | VARCHAR(32) NULL | Always `image/jpeg` when set |
| `avatar_updated_at` | DATETIME NULL | Naive UTC. Drives `avatar_url` and the ETag. NULL means no avatar_url |

**`COLUMN_MIGRATIONS["users"]`** (db.py) adds these to pre-existing tables with defaults that backfill old rows:
- `theme VARCHAR(32) NOT NULL DEFAULT 'paper'`
- `color_mode VARCHAR(16) NOT NULL DEFAULT 'system'`
- `locale VARCHAR(8) NOT NULL DEFAULT 'en'`
- `notify_meeting/notify_pick/notify_note BOOLEAN NOT NULL DEFAULT 1`
- `avatar BLOB`
- `avatar_mime VARCHAR(32)`
- `avatar_updated_at DATETIME`

A production DB that hasn't booted the latest Python may lack any of these. The importer must default missing columns the same way.

There is no user deletion anywhere in the Python app. There is no admin flag or role; all users are equal.

### 3.2 `invites`

| Column | Type | Semantics |
|---|---|---|
| `id` | INTEGER PK | |
| `code` | VARCHAR(32) NOT NULL, unique index `ix_invites_code` | Normalized uppercase. Generated codes are 10 Crockford chars. The bootstrap code is whatever the operator set, normalized, and may in theory exceed 32 chars because SQLite doesn't enforce it. Demo seed uses `DEMOTEST2026` |
| `created_by_id` | INTEGER NULL FK users.id | NULL for the bootstrap invite and for CLI-minted invites |
| `used_by_id` | INTEGER NULL FK users.id | NULL means unused. Set exactly once |
| `created_at` | DATETIME NOT NULL | Naive UTC |
| `used_at` | DATETIME NULL | Set together with `used_by_id`. Never exposed |

### 3.3 Other tables this area writes or reads

- **`events`** (written by register): `actor_id` is the new user, `kind='member_joined'`, `book_id`/`pick_id` are NULL, `payload='{}'`, `created_at` is now.
- **`user_favorites`** (written by the favourites endpoints): `user_id` (FK, ON DELETE CASCADE), `book_id` (FK books, ON DELETE CASCADE), `position` 1..3. Unique on `(user_id, book_id)` and `(user_id, position)`, with CHECK `position BETWEEN 1 AND 3`.
- **`shelf`** (read by the members list): `user_id`, `book_id`, `status` (`currently_reading`), `position`, `id`.
- **`books`** (read by the members preview): `ol_work_key` (`/works/<id>`), `title`, `cover_id`, `cover_image_url`.
- **Files under the data dir** (not tables): `.secret_key`, `.bootstrap_invite`, `.vapid_private.pem`.

### 3.4 Data migrations touching this area

There is no data migration for users or invites; only the `COLUMN_MIGRATIONS` above. `import_legacy_pick_posts` does not touch users.

---

## 4. Tests (parity checklist; 44 tests)

### backend/tests/test_auth.py (22)
- `test_auth.py::test_health`: `GET /api/health` returns exactly `{"ok": true}`.
- `test_auth.py::test_register_consumes_invite_and_sets_session`: registering ada with the bootstrap invite returns 201 and the body is exactly `{"id":1,"username":"ada","theme":"paper","color_mode":"system","locale":"en","avatar_url":null}`. The returned session cookie makes `GET /api/auth/me` return 200 with username ada.
- `test_auth.py::test_invalid_invite_does_not_create_user`: register with invite `NOPE` returns 400. Logging in as ada afterwards returns 401, so no user row is left behind.
- `test_auth.py::test_invite_cannot_be_reused`: after a successful register and a logout, a second register with the same invite returns 400 with `detail` containing "invite" (case-insensitive).
- `test_auth.py::test_invite_cannot_be_used_twice_concurrently`: two threads register ada and grace with the same invite at the same instant. The statuses are exactly {201, 400}, and exactly one of ada or grace can log in afterwards.
- `test_auth.py::test_username_taken`: ada registers and mints an invite (`POST /api/invites` returns 201). After a logout, registering `"Ada"` (normalized to ada) with the fresh invite returns 409.
- `test_auth.py::test_bad_username_and_short_password`: username `"Ada Lovelace"` returns 400, and password `"short"` returns 400.
- `test_auth.py::test_me_requires_session`: `GET /api/auth/me` without a cookie returns 401.
- `test_auth.py::test_login_and_logout`:
  - After logout, `/me` returns 401.
  - Login with the right password returns 204, and `/me` then shows ada.
  - Login with a wrong password returns 401.
- `test_auth.py::test_login_throttle`: 10 wrong-password logins each return 401, and the 11th attempt returns 429.
- `test_auth.py::test_me_carries_default_preferences`: a new user's `/me` has theme `paper` and color_mode `system`.
- `test_auth.py::test_update_theme_alone_leaves_color_mode`: PATCH preferences `{"theme":"forest"}` returns 200 with the exact full UserOut `{"id":1,"username":"ada","theme":"forest","color_mode":"system","locale":"en","avatar_url":null}`.
- `test_auth.py::test_update_color_mode_alone_leaves_theme`: PATCH `{"color_mode":"dark"}` returns 200 with theme still `paper` and color_mode `dark`.
- `test_auth.py::test_update_both_preferences`: PATCH `{"theme":"ink","color_mode":"light"}` returns 200 with both applied.
- `test_auth.py::test_empty_preferences_payload_is_a_no_op`: after setting `slate`, PATCH `{}` returns 200 and theme stays `slate`.
- `test_auth.py::test_preferences_persist_across_sessions`: set slate/dark, log out and log in again; `/me` still shows slate/dark.
- `test_auth.py::test_invalid_theme_rejected`: theme `neon` returns 400 with `detail` containing `"paper"` (the allowed list). The stored theme is unchanged.
- `test_auth.py::test_invalid_color_mode_rejected`: color_mode `sepia` returns 400 with `detail` containing `"color_mode"`. The value is unchanged (`system`).
- `test_auth.py::test_preferences_require_a_session`: PATCH preferences after logout returns 401.
- `test_auth.py::test_update_locale`: PATCH `{"locale":"de"}` returns 200 with locale `de` and theme still `paper`. It persists across logout and login.
- `test_auth.py::test_invalid_locale_rejected`: locale `fr` returns 400 with `detail` containing `"locale"`. The value stays `en`.
- `test_auth.py::test_preference_columns_backfill_pre_existing_users`: `init_db` on a legacy SQLite `users(id, username, password_hash, created_at)` table adds the missing columns. Running `init_db` a second time does not fail. The existing row reads back `('paper','system','en')`. Port this as an importer test: a legacy DB lacking these columns imports with the defaults.

### backend/tests/test_members.py (1)
- `test_members.py::test_members_hide_self_and_preview_reading`:
  - Setup: ada shelves Circe (`/works/OL1W`, cover_id 11) as `currently_reading` and mints an invite, then grace registers with it.
  - grace's `GET /api/members` has exactly one row, ada, with `currently_reading_count == 1`.
  - `currently_reading_preview[0].title == "Circe"`, and `preview[0].ol_work_key` starts with `/works/`.
  - grace never appears in her own list.

### backend/tests/test_avatars.py (8)
- `test_avatars.py::test_upload_serves_and_deletes_avatar`:
  - Uploading a 48×48 PNG returns 200 UserOut with username ada, `avatar_url` starting with `/api/members/ada/avatar?v=`, and no `avatar` key in the body.
  - GET of that URL returns 200 with `content-type: image/jpeg`, bytes starting `FF D8 FF`, and an image decoding to 256×256.
  - `/me.avatar_url` equals the upload response's URL.
  - DELETE returns 200 with `avatar_url` null. The old URL then returns 404, and `/me.avatar_url` is null.
- `test_avatars.py::test_other_members_see_avatar_url`: after ada uploads, grace (another user) sees ada's `avatar_url` in `/api/members` equal to ada's own URL. grace can GET it with 200. `GET /api/shelf?username=ada` has `user.avatar_url` equal to the same URL.
- `test_avatars.py::test_rejects_non_image_and_oversize`:
  - An SVG upload returns 400.
  - A JPEG-magic file of 50 MiB + 3 bytes returns 400 with `detail == "That picture is too large (50 MB max)"`.
  - `avatar_url` stays null.
- `test_avatars.py::test_rejects_small_file_with_too_many_pixels`: a 6000×6000 PNG, under 50 MiB on disk, returns 400 `"That picture has too many pixels"`. `avatar_url` stays null.
- `test_avatars.py::test_oversize_content_length_is_rejected_before_read`:
  - Unit: `oversize_content_length(None)` and `("100")` are False. `(50MiB+4096)` is False and `(50MiB+4097)` is True.
  - HTTP: a PUT with header `Content-Length: 50MiB+4097` and a tiny body returns 413 with `detail == "That picture is too large (50 MB max)"`.
- `test_avatars.py::test_avatar_etag_and_nosniff`:
  - GET of the avatar URL returns 200 with `x-content-type-options: nosniff` and a quoted `etag`.
  - A repeat GET with `If-None-Match: <etag>` returns 304 with an empty body, still carrying `nosniff` and the same `etag`.
- `test_avatars.py::test_avatar_blob_stays_deferred_when_serializing`: an ORM-level check that loading a `User` and computing `avatar_url_for` never loads the `avatar` blob column, and `avatar_updated_at` is not None. This is SQLAlchemy-specific. The Rust equivalent is that user queries never select the blob; under #43 there is no blob in `users` at all.
- `test_avatars.py::test_avatar_requires_session`:
  - After logout: avatar PUT returns 401, DELETE returns 401, and GET of the existing avatar URL returns 401.
  - After logging in again, the same GET returns 200.

### backend/tests/test_selfhost.py (13)
- `test_selfhost.py::test_placeholder_detection`:
  - `is_placeholder_secret` is true for None, `dev-secret-change-me`, `change-me-to-a-long-random-string` and `short`, and false for `"a"*32`.
  - `is_placeholder_invite` is true for None, `DEV-ONLY` and `dev only`, and false for `FRIENDS`.
  - `https_mode`: `"auto"` gives auto, `"1"` gives always, `"0"` gives never, True gives always.
- `test_selfhost.py::test_production_generates_and_reuses_secrets`:
  - With DEBUG=0 and empty SECRET_KEY and BOOTSTRAP_INVITE, the first `prepare_environment()` creates `.secret_key` (≥32 chars) and `.bootstrap_invite` (non-empty, not `DEVONLY`), with `generated_*` True.
  - A second run with empty vars reuses both files, with `generated_*` False and identical contents.
- `test_selfhost.py::test_production_app_registers_and_hides_docs`:
  - Production app (DEBUG=0, explicit 32-char secret, invite `SELFHOST1`): health returns `{"ok":true}`, and `/api/docs` returns 404.
  - A CORS preflight from `http://localhost:5173` gets no `access-control-allow-origin`.
  - Register with `SELFHOST1` returns 201 with a Set-Cookie containing `httponly` and `samesite=lax`, and `/me` works.
- `test_selfhost.py::test_session_cookie_not_secure_on_plain_http`: production app at `http://bookclub.test` with HTTPS=auto. The register cookie contains `bookclub_session=` and no `secure`.
- `test_selfhost.py::test_session_cookie_secure_on_https_and_forwarded_proto`: both cases have HTTPS=auto and set a `bookclub_session` cookie containing `secure`.
  - Base URL `https://bookclub.test`.
  - Plain http with `X-Forwarded-Proto: https`, where trusted proxies default to `*`. The register returns 201.
- `test_selfhost.py::test_backup_copies_sqlite`: `backup_database(src, dest)` produces a non-empty file after a register. Replaced by the plan (Postgres server backups plus DSGVO export).
- `test_selfhost.py::test_production_refuses_demo_secret_and_invite`: with DEBUG=0, `SECRET_KEY=dev-secret-change-me` raises a RuntimeError matching "SECRET_KEY". With a good secret and `BOOKCLUB_BOOTSTRAP_INVITE=DEV-ONLY`, it raises a RuntimeError matching "DEV-ONLY".
- `test_selfhost.py::test_public_config_and_cors_origins`:
  - `public_config` maps name `Thursday Readers`, theme `#224466`, `theme_dark` starting with `#`, and a public URL with path `/app` reduced to `https://books.example.com`.
  - Non-debug `cors_origins` is `["https://books.example.com"]`.
  - The OL UA contains `ThursdayReaders/1.0` and the origin.
  - Debug `cors_origins` includes `http://localhost:5173` and the public origin.
  - `Settings(bookclub_public_url="not-a-url")` raises a ValidationError.
- `test_selfhost.py::test_branded_app_config_cors_and_user_agent`:
  - Production app with name `Thursday Readers`, theme `#336699` and public URL: the app title is the name, `/api/docs` returns 404, and `/api/config` has name, theme and public_url.
  - `/manifest.webmanifest` has name and theme_color, plus icons including (192x192, any), (512x512, any) and (512x512, maskable).
  - A preflight from the public origin is allowed with an echoing `access-control-allow-origin`. One from `http://localhost:5173` is not allowed.
- `test_selfhost.py::test_dotenv_only_production_boot`:
  - All relevant env vars are unset, and `BOOKCLUB_ENV_FILE` points to a .env with `DEBUG=0`, empty SECRET_KEY and invite, DATABASE_PATH, name `Dotenv Club` and HTTPS=auto.
  - The app boots, the title is `Dotenv Club`, docs return 404, health is ok, and `/api/config` name is `Dotenv Club`.
  - The generated `.bootstrap_invite` file exists and registering with it returns 201.
- `test_selfhost.py::test_trusted_proxy_denies_forwarded_proto`: with `BOOKCLUB_TRUSTED_PROXIES=127.0.0.1` (the test client is not 127.0.0.1), `X-Forwarded-Proto: https` is ignored. Register returns 201 with a cookie that has **no** `secure`, but does have `httponly` and `samesite=lax`.
- `test_selfhost.py::test_invalid_theme_warns_and_falls_back`: `BOOKCLUB_THEME=not-a-color` makes `/api/config` theme `#b44a2a`, and the WARNING log contains `BOOKCLUB_THEME` and `not-a-color`.
- `test_selfhost.py::test_debug_cors_keeps_vite`: on the debug client, `/api/config` name is `Bookclub`. A preflight from `http://localhost:5173` gets `access-control-allow-origin: http://localhost:5173` and `access-control-allow-credentials: true`.

---

## 5. Multi-club mapping

Each point is labelled **[plan]** when it is decided in plan.md or the issues (including Forgejo #39, #40, #43, #46, #55, which I read for detail), and **[proposal]** when it is my recommendation for the implementer.

### 5.1 Personal (account-level, no club_id)

- **`users`** [plan #37/#39]
  - **Columns carried over:** username, password_hash (copied verbatim by the importer), theme, color_mode, locale and the notify_* prefs. These stay personal.
  - **Email:** new nullable column with a verified flag or `email_verified_at`. Imported users have NULL and are prompted once (skippable), so a `email_prompt_dismissed_at` [proposal] is needed. Password reset is impossible without an email.
  - **Avatar:** the blob moves to the media store [plan #43]. `users` gets an avatar media reference instead of `avatar/avatar_mime/avatar_updated_at`.
- **Sessions** [plan #39: cookie sessions for web, bearer tokens for apps]
  - Replace the stateless signed cookie with a server-side `sessions` table [proposal]: a random token stored hashed, user_id, created_at, expires_at, last_used_at, kind (`cookie`/`bearer`), user agent.
  - This enables real logout, "log out other devices" on password change, and session wipe on deletion.
  - Keep the cookie name `bookclub_session`, `HttpOnly`, `SameSite=Lax`, `Path=/` and a 30-day lifetime.
  - Legacy signed cookies will not verify, so everyone logs in once after cutover [plan: the domain changes anyway].
  - `SECRET_KEY` is then unnecessary for sessions.
- **Password change**
  - Keep `PATCH /api/auth/password` with the same strings.
  - Add revocation of other sessions [proposal], plus reset via email token [plan #39].
- **Preferences and notification prefs:** unchanged and personal. #48 lists notification prefs under account settings; push itself moves to Phase 4 [plan].
- **Favourites:** personal [plan]. `book_ids` become work ids.
  - The detail `"That book is not in the club yet"` no longer fits a global catalog. If it changes, update `apiErrors.ts` and the i18n keys too.
- **Account deletion and DSGVO export** [plan #39]. Both are new.
  - Deletion removes the user, sessions, tokens, avatar media, shelf, diary, quotes, favourites, physical layout and search history.
  - Club posts are deleted or anonymised; this is still to be decided. Invites they created keep `created_by` NULL [proposal]. A redeemer reference becomes NULL.
  - An owner must transfer ownership or delete the club first [plan #40 implies this].
- **Rate limits** [plan #39/#55: login, sign-up, reset]
  - Keep the parity behaviour for login (10 failures in 600 s per normalized username, then 429 with the same string, a success clears the bucket, password change shares it).
  - Add per-IP limits [proposal]. This needs the real client IP from a **trusted** proxy only; the Python default of trusting every proxy (`*`) must not carry over.
  - The proxy also rate-limits [plan].

### 5.2 Club-scoped (club_id FK; non-members get 404 [plan #40])

- **Invites** [plan #40]
  - **Lifecycle:** club-scoped codes or links that are single-use or expiring. Admins and owners create and revoke them; members cannot [plan].
    - Python lets any user mint, so `POST /api/invites` gains a role check and a club path. A plain member gets 403 with a new string; non-members get 404.
    - Proposed columns: `club_id`, `code` (globally unique, keeping the 10-char Crockford generator and normalization), `created_by_id` (nullable), `expires_at` (nullable), `max_uses` (1 for single-use, NULL for expiring multi-use), `revoked_at`, and redemptions (a separate table if multi-use).
  - **Listing:** Python lists only the caller's own invites; under roles, admins should see all invites of the club [proposal].
  - **Redemption:**
    - It joins the club as `member`. It must fail when the club has 20 members [plan], when the invite is expired, revoked or used up, or when the user is already a member.
    - Keep the atomic conditional consume in the same transaction as the membership insert. Keep the `"That invite code is not valid"` string for invalid, used, expired and revoked codes, unless distinct messages are wanted; those must be added to `apiErrors.ts`.
  - **Closed beta** [plan #55: "Until it closes, the instance is a closed beta: only invited people can sign up", via a sign-up flag]:
    - Phase 1 sign-up = username + email + password [+ invite code while the flag is off].
    - A valid club invite consumed at sign-up creates the account **and** the membership.
  - **Bootstrap invite:** the `BOOKCLUB_BOOTSTRAP_INVITE` env var, the `.bootstrap_invite` file and `app.create_invite` CLI have no meaning any more [plan: the importer's `--owner` makes the first owner]. Something must still let Rafael invite a beta user who wants no club (open question).
- **Members list:** `GET /api/members` becomes the club's member list under `/c/<slug>` [plan #48], e.g. `GET /api/clubs/{slug}/members`.
  - **Roles:** add `role` (owner/admin/member) per row.
  - **Self:** Python excludes the caller. A club roster probably should include self for the role and transfer UI [proposal]. If not, keep exclusion for parity.
  - **Reading preview:** the currently-reading count and preview are reads of another user's shelf, so they go through the central visibility policy [plan #46]. Co-members are the audience, so they still see it.
  - **Book fields:** `ol_work_key` and `cover_id` in the preview become the new work id and a cover URL from our own media domain [plan #41/#43]; no `covers.openlibrary.org` URLs reach clients.
- **`member_joined` event:** becomes a club activity event (club_id), emitted on joining a club rather than on account creation. Solo sign-ups emit none [proposal].

### 5.3 Avatars [plan #43]

- **Storage:** the media store with content-hashed URLs and long cache headers. That replaces `?v=<epoch>`, the ETag and the `private, max-age=86400` header.
- **URL:** `avatar_url` stays a nullable string on UserOut, MemberOut and the others, but its format changes. The frontend treats it as opaque.
- **Upload:** EXIF stripping is now an explicit requirement [plan #43 AC for photos]. Python's pipeline already drops all metadata.
- **Format:** Python stores a JPEG; whether avatars become WebP like cover photos is an open question. The importer copies existing JPEG blobs into the store.
- **Visibility:** Python requires any session to fetch any avatar. Under the plan strangers see nothing [#46], yet unguessable content-hashed URLs are often served without auth (open question).
- **Removed:** the `/api/members/{username}/avatar` endpoint and its 404 `"No profile picture"` go away, or redirect to the media URL.

### 5.4 Instance-level config that changes or disappears

- **Gone with self-hosting [plan]:**
  - `prepare_environment()`, placeholder-secret detection, generated `.secret_key` / `.bootstrap_invite`
  - `DEBUG` defaulting to true
  - `BOOKCLUB_ENV_FILE` dotenv handling
  - `BOOKCLUB_HTTPS=auto` per-request Secure (hosted is always HTTPS behind the proxy, so Secure always in prod and a dev flag for http)
  - `DATABASE_PATH`
  - the healthcheck module (the Rust image has no Python; uptime monitoring is external)
  - the backup endpoint and CLI (replaced by personal export and server backups [plan #49])
- **Branding:** `BOOKCLUB_NAME/THEME/THEME_DARK` were per-instance branding. The service has one public name, decided in #50 [plan].
  - Club names become per-club data.
  - `/api/config` and the manifest become service-level: name, public_url, and perhaps `signup_open` [proposal] so the frontend knows whether an invite is required.
- **Timezone:** `BOOKCLUB_TZ` (`/api/config.timezone`) was the club timezone for meetings. It must become per-club (a club setting) or per-user (open question).
- **Public URL:** `BOOKCLUB_PUBLIC_URL` is still needed: for email links, the OL User-Agent contact [plan #41: "User-Agent with contact"] and CORS. Keep the validation (http(s) with a host, reduced to origin).
- **CORS:** the web app is same-origin and Vite proxies `/api` in dev, so CORS is only needed later for Capacitor origins with bearer tokens (Phase 4). Make the origins explicit config, not tied to a debug flag [proposal].
- **Dropped [plan]:**
  - `GOOGLE_BOOKS_API_KEY`, Google UA and the Google cover-URL branch: Google Books is removed.
  - `VAPID_*`: push subscriptions are dropped; push comes back in Phase 4.
- **Docs:** `/api/docs` and `/api/openapi.json` were debug-only. #47 wants `GET /api/openapi.json` for type generation; whether it is public in prod is open.
- **Health:** the scaffold already decided `{"status":"ok"}` / 503 `{"status":"unavailable"}`. `test_auth.py::test_health` and the selfhost tests asserting `{"ok": true}` become reasoned changes. The old Docker probe's substring check `"ok"` would still match the new body.

### 5.5 What no longer makes sense

These items need a recorded reason in the #49 checklist:
- "everyone except me" as the definition of members
- any user minting invites
- `GET /api/invites` scoped to the creator
- the bootstrap invite
- placeholder-secret refusal
- the dotenv-only boot
- the HTTPS auto mode
- trusted-proxy `*`
- branding env vars
- the SQLite backup
- the deferred-blob ORM test
- the `COLUMN_MIGRATIONS` backfill test (replaced by an importer test on old-schema fixtures)

---

## 6. Port risks

1. **EN DASH** in `"Username must be 2–32 characters: lowercase letters, numbers, underscore"` (U+2013). A hyphen-minus breaks string equality.
2. **Length in code points, not bytes:** Python `len()` counts code points. Rust `str::len()` counts bytes, so use `chars().count()` for the ≥8 password rule (`"pässwörd"` passes). The username regex is ASCII-only after `.lower()`. Python lowercases the Kelvin sign U+212A to `k`, and Rust's `to_lowercase` does the same; keep strip+lowercase **before** the regex.
3. **Normalization before every lookup:** login also strips and lowercases (no regex). Register checks "taken" against the normalized name. The avatar path lowercases the username. The rate-limit key is the normalized username.
4. **Check order** (1.0, 1.1, 1.8):
   - JSON parse 400, then auth 401, then field validation 400, then logic.
   - Register: validation, then 409 taken, then 400 invite.
   - Password change: 429, then wrong current (records a failure), then same password (does not record).
   - Login: 429 is checked before verifying credentials.
5. **axum defaults differ from FastAPI:**
   - The `Json` extractor rejects with 415 or 422 and plain-text bodies, and `Path` rejections are plain text. Everything must map to **400 `{"detail": "..."}`**, with multiple messages joined by `"; "`.
   - Missing fields, wrong types and unknown keys (ignored) all need matching behaviour.
   - A missing body on `PATCH /me/preferences` is 400, while `{}` is a no-op.
6. **Body size limit:** axum's `DefaultBodyLimit` is 2 MB. The avatar route must allow 50 MiB + 4096 slack. The explicit `Content-Length` check must return **413 with the JSON detail**, while an oversize file part that slips through returns **400** with the same detail.
7. **Avatar decode safety:** read dimensions from the header and reject over 25,000,000 pixels **before** decoding (a 6000×6000 PNG test). Use the `image` crate `Limits`. Also apply EXIF orientation, use the first GIF/WebP frame, composite alpha onto white (not black), centre-crop with floor offsets, resize with Lanczos3 to 256, and encode JPEG q85. Keep the error message precedence of 2.5. The magic-byte sniff (≥24 bytes) runs before decoding: an SVG gets "Use a JPEG, PNG, WebP, or GIF", not "That file is not a picture".
8. **argon2 is blocking and expensive** (64 MiB, t=3, p=4): run hash and verify in `spawn_blocking` (or a bounded pool) or the async runtime stalls.
   - Verification must use the parameters from the PHC string; the Rust `argon2` crate does this via `PasswordHash::new` and `verify_password`.
   - An unparsable stored hash must be a mismatch (401), never a 500.
   - New hashes stay argon2id, with the same params unless deliberately changed.
   - Consider a dummy verify for unknown usernames to close the timing leak.
9. **Invite consumption must be atomic** and in the same transaction as the account or membership insert: `UPDATE … WHERE code=$1 AND used_by_id IS NULL RETURNING`, or a row lock. A failed consume must roll back the new user. A unique violation on username must map to 409, not 500. The concurrency test expects exactly one success.
10. **Session semantics:**
    - Python sessions are **not sliding** and die 30 days after login.
    - Logout always returns 204, needs no auth, and emits a deletion cookie only when a valid session existed.
    - Registering while signed in switches identity.
    - A deleted user's cookie gives 401 and clears the cookie.
    - Secure flag rules per environment.
    - `SameSite=Lax` is the only CSRF defence for cookie-auth JSON endpoints. Keep it, and don't add GET endpoints with side effects.
11. **Rate-limit details:**
    - The window is sliding over failure timestamps (not a fixed bucket).
    - The threshold is `>= 10` recorded failures before the attempt.
    - A 429 doesn't record.
    - Unknown usernames count.
    - Success clears.
    - Login and password change share a bucket.
    - Use a monotonic clock.
    - Bound memory with a TTL or LRU; the Python dict grows forever.
    - A test-reset hook is required; the Python `client` fixture resets it.
12. **Ordering and collation:** `ORDER BY username` was SQLite byte order, so digits < `_` < letters. Postgres with a glibc or ICU locale collation orders `_` differently, so use `COLLATE "C"`. `ORDER BY created_at DESC` for invites needs an `id DESC` tiebreak for determinism.
13. **Datetime serialization:**
    - Python emits naive ISO strings for DB-loaded datetimes (`"2026-10-27T10:46:19.453446"`, no offset). JavaScript `new Date()` then parses them as **local** time.
    - Rust will emit RFC 3339 with `Z`. That is correct, but it changes displayed times for non-UTC browsers.
    - The importer must parse naive SQLite text (`YYYY-MM-DD HH:MM:SS` with optional `.ffffff`) as UTC.
14. **Avatar URL and cache:**
    - `avatar_url` is null whenever `avatar_updated_at` is NULL.
    - `v` is UTC epoch seconds, while the Python ETag uses local time; do not copy that bug.
    - `If-None-Match` matching is exact string equality.
    - A 304 must still carry ETag, nosniff and Cache-Control.
    - Under #43 all of this is replaced by content-hashed media URLs; keep `avatar_url` nullable and opaque.
15. **Members list:**
    - The count covers **all** currently-reading entries, while the preview is capped at 3, ordered by `(position, id)` and skips entries without a book.
    - The caller is excluded.
    - Avoid the N+1 queries: a single query with a window function (`row_number() over (partition by user_id order by position, id)`).
16. **Lax booleans** on `PATCH /api/auth/notifications` accept `"no"`, `"YES"` and `1`; serde accepts only JSON booleans. No current frontend caller exists, so strict booleans are acceptable but should be recorded.
17. **Detail strings are a cross-repo contract:** every mapped string in 1.0 must survive verbatim, or be changed together with `apiErrors.ts` and both i18n files. Port `test_api_error_copy.py` so it scans `server/src` instead of `backend/app`.
18. **Health timeout:** the scaffold uses a 2 s timeout and returns `status` keys. Don't regress to Python's `{"ok":true}`; record it in the parity checklist instead.
19. **Importer** (affects auth data):
    - A legacy DB may lack the preference, notify or avatar columns; default them as `COLUMN_MIGRATIONS` does.
    - Copy password hashes byte-for-byte; a known-hash login test is required by #38.
    - Normalize theme, color_mode and locale values that aren't in the allowlists to defaults.
    - Bootstrap invite codes may exceed 32 chars, so use `TEXT` with no length cap.
    - Decide what happens to unused legacy invites.

---

## 7. Open questions (not settled by plan.md or the issues)

1. **Closed-beta sign-up gate:** until #55 turns on open sign-up, does sign-up require a *club* invite, or is there also a club-less "beta invite" (or admin CLI) so someone can join just to track their own reading?
2. **Login identifier:** username only (parity, and imported users have no email), or username *or* email?
3. **Session lifetime:** keep the fixed 30 days from login (parity), or switch to sliding expiry now that sessions are server-side? Lifetime of bearer tokens for the apps?
4. **Password change revocation:** should a password change revoke all other sessions and tokens, or all except the current one?
5. **Invite kinds:** what exactly are "single-use or expiring"? Two kinds (single-use with no expiry, and multi-use until `expires_at`), or every invite has an optional expiry plus a max-uses count? Default expiry?
6. **Invite listing scope:** do admins see every invite in the club, or only their own (as Python does)?
7. **Distinct error strings:** do expired, revoked and full-club invites get their own `detail` strings, or keep the single `"That invite code is not valid"`? Same question for the new 403 role-denied string, and for the stale `"That book is not in the club yet"` on favourites.
8. **Legacy invites in the importer:** import unused codes as club invites (single-use, no expiry), or drop them? Should a membership's `joined_at` come from `invites.used_at` or `users.created_at`?
9. **Members roster:** should the club member list include the caller (needed for the role UI), and which order: role then username, or username only?
10. **Avatars:** WebP or keep JPEG? Are content-hashed media URLs served without auth (unguessable), or behind the same visibility policy as other personal data? What size?
11. **Club timezone:** `BOOKCLUB_TZ` becomes a per-club setting, a per-user setting, or both?
12. **Service config:** is `/api/openapi.json` public in production? Should `/api/config` expose a `signup_open` flag? What service name and theme does it return before #50 decides the name?
13. **Account deletion and club content:** delete or anonymise the user's posts in clubs (#39 AC requires a decision)? What happens to invites they created, and to events where they are the actor?
14. **Rate limits:** per-IP limits on top of per-username, and what thresholds for sign-up and reset? Which trusted-proxy CIDRs feed the client IP?
15. **Notification prefs:** keep `notify_meeting/pick/note` global per user, or make them per club once push returns in Phase 4?
