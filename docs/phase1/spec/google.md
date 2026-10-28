# Port spec: Google Books integration (DROPPED)

Area key: `google`. Status in the new server: **removed**. Plan: "Google Books is dropped. Its terms forbid building databases or permanent copies." #41 AC: "no Google Books code or data in server/". #38: "Google ISBN…/GB… re-matched by ISBN to DNB/Open Library, unmatched keep bare facts (title, authors, ISBN, year, pages) without Google description/cover."

Nothing in this area is ported as Google code. This document exists for three reasons:

1. **Legacy importer (#38).** It records exactly which Google-derived values are in the SQLite `books` table, how to recognise them, and what to keep or drop.
2. **Parity checklist (#49).** Every test in `backend/tests/test_google_books.py` is listed with a verdict: DROP (Google-only), KEEP (a provider-agnostic rule that the OL + DNB + local catalog must also satisfy), ADAPT (keep the intent, change the mechanics) or INVERT (the new plan says the opposite).
3. **Frontend (#48) and catalog (#41).** It names the Google behaviour that leaks into shared endpoints, response fields and UI.

Sources read in full: `backend/app/googlebooks.py`, `backend/app/works.py`, `backend/app/routers/books.py`, `backend/tests/test_google_books.py`. Also read: `covers.py`, `serialize.py` (`cover_url`), `shelf_ops.py` (`upsert_book`, `apply_book_details`, `_set_cover`), `goodreads.py`, `openlibrary.py`, `schemas.py` (book DTOs), `models.py` (`Book` and the tables that reference it), `db.py` (`COLUMN_MIGRATIONS`), `branding.py` (UA), `config.py`, `frontend/src/constants.ts`, `tests/fixtures/goodreads.csv`, and the git history of `covers.py`/`googlebooks.py`.

---

## 0. Where Google touches the legacy system

| Place | What it does |
|---|---|
| `config.Settings.google_books_api_key` (env `GOOGLE_BOOKS_API_KEY`, default `""`) | Provider switch. `google_books_enabled()` is `bool(key.strip())`. Read on each call through `get_settings()`, so tests flip it at runtime. |
| `main.create_app` | Logs `catalog: Google Books` or `catalog: Open Library` at startup. |
| `branding.google_books_ua` | `"{token}/1.0 ({contact}; catalog search via Google Books)"`. `token` is the club name with everything except `[A-Za-z0-9._-]` removed (fallback `Bookclub`). `contact` is the public origin or `private book club`. |
| `googlebooks.py` | HTTP client, cache, cooldown, mapping of volumes to `SearchHit`/`VolumeDetails`. |
| `works.py` | `ISBN…` and `GB…` work-id formats and their derivation. |
| `covers.py` | Google cover-URL selection, normalisation and host allowlist. |
| `serialize.cover_url` | Render priority: Google URL > OL cover id > OL ISBN CDN, and the ISBN CDN is suppressed in Google mode. |
| `shelf_ops.upsert_book` / `apply_book_details` / `_set_cover` | Persist `cover_image_url`, canonicalise ISBN keys, reset `spine_color`. |
| `routers/books.py` | Provider branching in search, trending, subjects, works detail and refresh, work detail and refresh, and ISBN lookup. |
| `goodreads.py` | Goodreads CSV import resolves rows through Google when enabled. |
| `schemas.optional_cover_url` | Applies `storable_cover_image_url` to client `cover_url` on shelf add, club pick, diary book, vote nomination. `QuoteIn.cover_url` is raw but filtered later in `upsert_book`. |
| `spine_color._fetchable` | Allows fetching cover bytes from Google cover hosts. |
| Frontend `constants.ts` | `GOOGLE_WORK_ID_RE`, `isGoogleWorkId`, `isGoogleCatalogWorkKey`, `isbnFromWorkKey`, `googleBooksUrl`, `catalogUrl`, `remoteCoverUrl` + `isAllowedCoverHost`. |
| Frontend `BookCover.vue`, `BookDetailPage.vue` | Use those helpers: no OL ISBN cover for Google keys, a "Google Books" outbound link. |
| Docs | `.env.example`, `SELFHOST.md`, `README.md` mention the key. |

---

## 1. Endpoints

Rules that apply to every endpoint below:

- **Auth.** All require a signed-in session (`bookclub_session` cookie, `user_id`). Otherwise the response is **401** `{"detail": "Not signed in"}`.
- **Validation errors.** Query, path and body validation failures return **400**. The `detail` is every pydantic message joined with `"; "`, with a leading `"Value error, "` stripped.
- **No rate limit.** There is no per-user rate limit on these endpoints. The only throttle is Google's own quota and the global cooldown in §2.9.
- **Error mapping.** Every Google failure goes through `google_http_exception(exc, detail=…)`:

  | `GoogleBooksError` | HTTP status | `detail` |
  |---|---|---|
  | `status == 404` | 404 | `No such book.` |
  | `quota` true, or `status == 429` | 429 | `The library is busy. Wait a few seconds and try again.` |
  | anything else (5xx, non-quota 403, transport error with `status=None`, bad JSON, non-object JSON) | 502 | the caller's `detail` (listed per endpoint) |

  The 404 row applies to every call, search included. A Google 404 on a search request would surface as `No such book.`.

**Google outbound requests.** These are only made when the key is set.

- **Search and ISBN:** `GET https://www.googleapis.com/books/v1/volumes`.
- **Volume:** `GET https://www.googleapis.com/books/v1/volumes/{volume_id}`.
- **Headers:** `User-Agent: <google_books_ua>` and `X-Goog-Api-Key: <key>`. The key is never sent as a `key` query parameter (tests assert this).
- **Timeout:** connect 5 s, read 10 s, write 5 s, pool 5 s.
- **`VOLUME_FIELDS`:** `id,volumeInfo(title,authors,publishedDate,description,pageCount,categories,averageRating,ratingsCount,industryIdentifiers,imageLinks)`
- **`SEARCH_FIELDS`:** `totalItems,items(<VOLUME_FIELDS>)`

The endpoints follow, in Google mode (key set). OL-mode behaviour is described only where it touches `ISBN…`/`GB…` keys; the OL catalog itself belongs to the `books`/`catalog` area spec.

### 1.1 `GET /api/books/search` → `SearchPage`

**Query params:**

| Param | Type | Default | Rules |
|---|---|---|---|
| `q` | str | `""` | Whitespace collapsed (`" ".join(q.split())`). Longer than 200 chars after collapsing → 400 `Search is too long`. |
| `subject` | str | `""` | Normalised: lowercased, whitespace runs → `_`, must match `^[a-z0-9_]+$`. Otherwise it silently becomes `""` (no error). |
| `sort` | `readinglog` \| `new` \| `title` \| `relevance` \| null | null | Resolved to `sort or ("relevance" if q else "readinglog")`. Google maps `new` → `orderBy=newest` and every other value → no `orderBy` (Google relevance). |
| `page` | int ≥ 1 | 1 | |
| `limit` | int 1..40 | 24 | |

**Flow:**

1. **Empty `q` and empty `subject`** → trending (§1.2 with this `limit`). `page` is ignored and the response has `page: 1`.
2. **Short-query check.** `len(q) < 3`, not ISBN-shaped, no subject → 400 `Need at least 3 characters. Add the author, or paste an ISBN.`. This runs **only in OL mode**: in Google mode 1–2-character queries go to Google.
3. **Local hits.** `local = _local_catalog_hits(q, limit)` when `q` is non-empty and `page == 1`. It does a case-folded substring match of the needle against `"{title} {authors}"` over **all** `books` rows in `id` order, needs a needle of at least 2 chars, and stops at `limit`. Each hit gets `isbn = isbn_from_work_key(key)` (ISBN rows only), `custom` = key is `BC…`, and `cover_url = book_cover_url(book)`.
4. **Google search** (`fetch_google_catalog` → `search_volumes`):
   - **`q` construction** (`google_search_query`): if `extract_isbn(query)` matches, `q = "isbn:{ISBN}"` and the **subject is dropped**. Otherwise `q = query` plus, if a subject is set, ` subject:"{subject with _ → space}"`. An empty `q` returns `([], 0)` with no request.
   - **Params:** `q`, `maxResults=limit`, `startIndex=(page-1)*limit`, `fields=SEARCH_FIELDS`, plus `orderBy=newest` only for sort `new`.
   - **Cache key:** `gb|search|{q.casefold()}|{new|relevance}|{page}|{limit}`.
   - **Topic fallback.** When the mapped items are empty, `query == ""`, the subject is non-empty and `page == 1`, a second search runs with `query = subject.replace("_"," ")`, no subject, the same sort, page 1 and the same limit. It logs WARNING `google books empty subject shelf subject=<s> fallback_items=<n>` and marks `topic_fallback`.
   - **`has_more`** = `not topic_fallback and bool(items) and page*limit < totalItems`.
5. **Upstream errors** map as in the table above with detail `Could not search the library right now. Try again.`. If `local` is non-empty, any upstream `HTTPException` is swallowed and the response is `SearchPage(items=local, page=page, has_more=False)`.
6. **Merge** (`_merge_local_hits`): local hits first, then remote hits whose `ol_work_key` is not already present, appending until the list reaches `limit`. `has_more` stays as the remote page computed it.
7. **Annotate:**
   - **Shelf:** for each hit whose `ol_work_key` matches a `books` row the viewer has shelved, set `on_shelf` = status and `shelf_id` = entry id.
   - **Club pick:** `club_pick = (hit.ol_work_key == current pick's key)`, where the current pick is the newest `club_picks` row with `ended_at IS NULL`. When there is no current pick the field keeps its default `false`.

**Response `SearchPage`:**

- `items`: list of `SearchHit`
- `page`: int
- `has_more`: bool

**`SearchHit`:**

| Field | Type | Value from Google |
|---|---|---|
| `ol_work_key` | str | `/works/ISBN…` or `/works/GB…` |
| `title` | str | |
| `authors` | str | comma-joined |
| `cover_id` | int\|null | always null |
| `year` | int\|null | |
| `on_shelf` | `want_to_read` \| `currently_reading` \| `finished` \| `did_not_finish` \| null | |
| `shelf_id` | int\|null | |
| `club_pick` | bool | |
| `cover_edition_key` | str\|null | always null |
| `isbn` | str\|null | |
| `custom` | bool | false |
| `cover_url` | str\|null | Google https URL |

**Side effects:** no DB writes. Search does **not** import results; a test asserts no `books` row after browsing. The only writes are to the in-process cache and the cooldown.

**Status codes:** 200, 400, 401, 404 (Google 404), 429, 502.

### 1.2 `GET /api/books/trending` → `SearchPage`

- **Params:** `limit` int 1..40, default 12.
- **Google mode:** `fetch_google_catalog("", subject="fiction", sort="new", page=1, limit, detail="Could not load the library right now. Try again.")`.
  - This sends `q = subject:"fiction"` with `orderBy=newest`. On an empty result the topic fallback sends `q = fiction` with `orderBy=newest`.
  - `has_more` follows §1.1 (false after a fallback).
  - It never calls OL `trending/daily.json`.
- **Annotations:** same as search.
- **Errors:** 429, 502 (`Could not load the library right now. Try again.`), 404 (`No such book.`).

### 1.3 `GET /api/books/subjects/{subject}` → `SearchPage`

- **Params:**
  - `subject` path param, normalised as in §1.1. Empty after normalising → 400 `That is not a valid subject.`
  - `page` ≥ 1, default 1.
  - `limit` 1..40, default 12.
- **Google mode:** `fetch_google_catalog("", subject=key, sort="relevance", page, limit, detail="Could not load the library right now. Try again.")`.
  - This sends `q = subject:"{key with _→space}"` with no `orderBy`. The topic fallback (page 1 only) sends `q = {key with _→space}`.
  - It never calls OL `/subjects/`.
- **OL mode only:** 404 `No such subject.`.

### 1.4 `GET /api/books/works/{work_id}` → `BookDetailOut`

1. `work_id = canonical_work_id(work_id)` (§2.2). If `not is_work_id` → 404 `No such book.`
2. If a local row exists for `/works/{work_id}` → return it from the DB with **no outbound call**, in either mode.
3. `BC…` with no row → 404 `No such book.`
4. `ISBN…` or `GB…` with no row → `_load_google_catalog(work_id)` (§2.8), then `_import_google_details` (§2.10), then return from the DB.
5. Otherwise OL import (other spec).

**`_load_google_catalog` errors, Google mode:**

- **GB key:** `fetch_volume` errors map with detail `Could not load that book right now. Try again.`. A volume that maps to nothing → 404 `No such book.`.
- **ISBN key:** errors map with the same detail. No item → 404 `No such book.`.

**OL mode (no key):**

- **GB key:** 404 `No such book.`.
- **ISBN key:** OL `search.json?q=isbn:{isbn}`.
  - Miss → 404 `No such book.`.
  - Transport or status error → 502 `Could not look that ISBN up right now. Try again.`.
  - Hit → `VolumeDetails` with `description=None`, `subjects=None`, `cover_id` = OL cover id, `pages` = OL `number_of_pages_median`. The row is stored under the ISBN key.

The save can fail with 500 `Could not save that book`.

**Response `BookDetailOut`:**

| Field | Type / meaning |
|---|---|
| `id` | int |
| `ol_work_key` | str |
| `title` | str |
| `authors` | str, default `""` |
| `cover_id` | int\|null |
| `cover_url` | str\|null, from §2.4 |
| `year` | int\|null |
| `description` | str, default `""` |
| `subjects` | list[str], parsed from the JSON column |
| `on_shelf` | status\|null, viewer's |
| `shelf_id` | int\|null |
| `club_pick` | bool |
| `rating` | int\|null, viewer's |
| `take` | str |
| `dnf_reason` | str |
| `progress` | int\|null |
| `readers` | list of `{username, status, rating, progress, avatar_url}`; every **other** user with this book shelved, sorted by username |
| `club_rating` | float\|null; mean of all finished entries' ratings (viewer included), 1 dp |
| `rating_count` | int |
| `custom` | bool; key is `BC…` |
| `favorite_position` | int\|null, 1–3 |

**Side effects:** an INSERT or UPDATE of one `books` row on the first open. Google results enter the cache.

### 1.5 `POST /api/books/works/{work_id}/refresh` → `BookDetailOut`

No body.

1. `canonical_work_id`.
2. `BC…` → 400 `This book was added by the club, not Open Library.`
3. `ISBN…`/`GB…` → `_refresh_google_catalog`:
   - Calls `_load_google_catalog(work_id, bypass_cache=True)`. This skips both the discovery cache and the response cache, but the fresh response is still written to the cache.
   - If the load raises `HTTPException` with status **404** and a local row exists → return the existing row **unchanged** (no write, `details_fetched_at` not touched).
   - Any other error propagates: 429 busy, 502 `Could not load that book right now. Try again.`, and 400 below.
   - On success → `_import_google_details`. A row is created if none existed.
4. **GB key in OL mode** → **400** `This book was added from Google Books. Set GOOGLE_BOOKS_API_KEY to refresh.`. This happens **even if a local row exists**: 400 is not 404, so the keep-existing branch does not apply.
5. **ISBN key in OL mode** → OL ISBN lookup with `bypass_cache=True`.
   - Miss with an existing row → row unchanged.
   - Hit → title, authors, year and pages are overwritten from OL and `cover_id` is set to the OL cover. Description, subjects and `cover_image_url` are **kept**, because None does not wipe.
6. Not an OL work id → 404 `No such book.`
7. Otherwise OL refresh.

### 1.6 `GET /api/books/work/{work_id}` → `BookDetailsOut`

1. `canonical_work_id`.
2. `BC…`: local row or 404 `No such book.`.
3. `ISBN…`/`GB…`: a local row is returned with **no outbound call**. Otherwise load and import as in §1.4. The response then carries `ol_rating_count = details.rating_count` (Google `ratingsCount`). It is only returned on that first-import response and **never persisted**: later loads return null.
4. Anything else that is not `OL\d+W` → 400 `That is not an Open Library work id`. This route does not call `is_work_id` first.

**Response `BookDetailsOut`:**

| Field | Type / meaning |
|---|---|
| `ol_work_key` | str |
| `title` | str |
| `authors` | str |
| `cover_id` | int\|null |
| `year` | int\|null |
| `cover_url` | str\|null |
| `description` | str |
| `pages` | int\|null |
| `subjects` | list[str] |
| `ol_rating` | float\|null; Google `averageRating` on Google rows |
| `ol_rating_count` | int\|null |
| `on_shelf` | status\|null |
| `shelf_id` | int\|null |
| `club_pick` | bool; current pick's `book_id == book.id` |
| `members` | list of `{username, status, rating, take, progress, finished_at}`; every shelf entry, viewer first, then by username |
| `quote_count` | int; all quotes on the book |

### 1.7 `POST /api/books/work/{work_id}/refresh` → `BookDetailsOut`

- `BC…` → 400 `This book was added by the club, not Open Library.`
- `ISBN…`/`GB…` → `_refresh_google_catalog`, same as §1.5. The response does not include `ol_rating_count`.
- Not OL → 400 `That is not an Open Library work id`.

### 1.8 `GET /api/books/isbn/{isbn}` → `IsbnHitOut`

1. `clean = normalize_isbn(isbn)`: strip whitespace and `-`, uppercase, must match `^(?:\d{9}[\dXx]|\d{13})$`. Otherwise 400 `That is not a valid ISBN-10 or ISBN-13`.
   - **No checksum validation.**
2. **Google mode:** `lookup_isbn(clean)`.
   - **Request:** `q=isbn:{clean}`, `maxResults=1`, `fields=SEARCH_FIELDS`, no `startIndex`.
   - **Cache key:** `gb|isbn|{clean}`.
   - **Empty results are not cached.** The first item goes through `volume_details`.
   - Error → mapped with detail `Could not look that ISBN up right now. Try again.`.
   - No item, or the item does not map → 404 `No book found for that ISBN`.
3. **Response:**

   | Field | Value |
   |---|---|
   | `isbn` | `clean` (the **requested** ISBN) |
   | `ol_work_key` | the volume's **derived** key; can differ from `ISBN{clean}`, see §2.11 |
   | `title` | |
   | `authors` | |
   | `cover_id` | always null |
   | `year` | |
   | `pages` | |
   | `cover_url` | Google URL |
   | `on_shelf`, `shelf_id` | set if a `books` row with that derived key exists and the viewer shelved it |

4. **No DB write.**
5. **OL mode:** OL lookup. The response key is an OL work key.

### 1.9 `POST /api/shelf/import` (Goodreads CSV, multipart `file`) → `GoodreadsImportOut`

Only the Google branch is covered here. Parsing belongs to the shelf area.

- **Fail-fast cooldown.** If Google is enabled and cooling down and there is at least one parsed row → **429** `The library is busy. Wait a few seconds and try again.` before any lookup.
- **Lookups.** Two concurrent lookups at a time (semaphore of 2). Queries are tried in order and the first hit wins:
  1. `isbn:{isbn}` when the CSV has an ISBN. `ISBN13` is preferred over `ISBN`. `="…"` Excel wrappers are stripped, digits and X are kept, and the value is uppercased; the length must be 10 or 13.
  2. `"{title} {authors}"`, stripped.
- **Google request.** Each query goes through `search_volumes(query, page=1, limit=5)`, with cache key `gb|search|{query.casefold()}|relevance|1|5`.
  - `google_search_query` does **not** recognise the literal `isbn:…` prefix as an ISBN, so the string is sent as is (Google interprets it).
  - The first mapped hit wins. Google mode never falls back to OL.
- **Per-row errors.** A `GoogleBooksError` (including a cooldown tripped mid-import) → that row is skipped with reason `Catalog unavailable`. No hit → `No match`.
- **Upsert.** Each hit is saved with `upsert_book(ol_work_key=hit.ol_work_key, title, authors, cover_id=None, year, cover_image_url=hit.cover_url)`. The **stored key is Google's derived key, not the CSV's ISBN.** Description, pages, subjects and rating are **not** fetched.
  - Already shelved → skip with `Already on your shelf`.
- **Response:**
  - `imported`: int
  - `skipped`: int, equal to `len(skips)`
  - `skips`: up to 25 `{title, reason}` objects

### 1.10 Endpoints that persist client-supplied Google covers or accept Google keys

These create or update `books` rows through `upsert_book(..., cover_image_url=payload.cover_url)`. The `cover_url` passes through `storable_cover_image_url` (§2.4). Non-Google, OL CDN and malformed URLs are **silently dropped** (`null`), never rejected:

- `POST /api/shelf` (`ShelfAddIn`): `ol_work_key` must start with `/works/`. A duplicate returns 409 `{"detail": "Already on your shelf", "item": …}`.
- `PUT /api/pick` (`ClubPickSetIn` via `pick_ops.apply_club_pick`).
- `POST /api/vote/nominations` (`VoteNominateIn`).
- `POST /api/quotes` (`QuoteIn`; `cover_url` is not validated in the schema but is filtered in `upsert_book`). The book is looked up by the **raw** key first.
- `POST /api/books/works/{work_id}/posts` (`DiaryEntryIn.book.cover_url`). The path key goes through `canonical_work_id` and `is_work_id`; failure → 404 `No such book.`.

Two more endpoints are relevant:

- `GET /api/books/works/{work_id}/posts` accepts `ISBN…`/`GB…` keys (`is_work_id`).
- `POST /api/books/custom` (`CustomBookIn`) creates `BC{10 hex}` books that never call any catalog. A test checks this in Google mode.

**Key normalisation.** `upsert_book` reduces the key to `/works/{last path segment}` and canonicalises an `ISBN…x` key to `X`. It does **not** validate the id format, so any `/works/<segment>` can be stored (§6, risk 14).

---

## 2. Domain rules

### 2.1 Provider selection

- **Key set (Google mode):** search, browse, trending, ISBN lookup, Goodreads, and the detail and refresh of `ISBN…`/`GB…` keys use **only Google**. An empty result stays empty, with no OL fallback, and the short-query check is skipped. OL-keyed rows (`OL…W`) still load and refresh from OL.
- **Key empty (OL mode):** Google is never called.
  - `ISBN…` keys still resolve and refresh through the OL ISBN lookup.
  - `GB…` keys: 404 on open, 400 on refresh.
- **Local first.** A local row is always served first, in both modes, with no outbound call.

### 2.2 Work-id formats (`works.py`)

The exact regexes:

```
OL_WORK_ID_RE      ^OL\d+W$
CLUB_WORK_ID_RE    ^BC[a-f0-9]{10}$
ISBN_WORK_ID_RE    ^ISBN(?:\d{9}[\dXx]|\d{13})$
GOOGLE_VOLUME_ID_RE ^[A-Za-z0-9_-]{1,40}$
GOOGLE_WORK_ID_RE  ^GB[A-Za-z0-9_-]{1,40}$
WORK_ID_RE         ^(?:OL\d+W|BC[a-f0-9]{10}|ISBN(?:\d{9}[\dXx]|\d{13})|GB[A-Za-z0-9_-]{1,40})$
```

**`canonical_work_id(id)`.** If `len(id) > 4` and `id[:4].upper() == "ISBN"`, it builds `"ISBN" + id[4:].upper()`. If that matches `ISBN_WORK_ID_RE` it is returned; otherwise the id comes back unchanged. Examples:

- `ISBN080442957x` → `ISBN080442957X`
- `isbn9780316769488` → `ISBN9780316769488`
- `ISBN12345` → unchanged

`GB` ids are **case-sensitive** and never altered.

**Helper functions:**

- `work_key(id)` = `"/works/" + canonical_work_id(id)`
- `work_id_from_key(key)` = the last `/` segment
- `isbn_from_work_id(id)`: the canonical id minus the `ISBN` prefix, or None
- `google_volume_id(id)`: the id minus the `GB` prefix, for `GB` ids only
- `is_google_catalog_id(id)`: `is_isbn_work_id or is_google_work_id`. These predicates do **not** canonicalise; the routes call `canonical_work_id` first.

**`google_catalog_work_id(isbn, volume_id)`.** This is how Google keys are minted:

1. `cleaned = re.sub(r"[\s-]", "", isbn or "").upper()`. If `ISBN{cleaned}` matches `ISBN_WORK_ID_RE` → return `ISBN{cleaned}`.
2. Else `volume = volume_id.strip()`. If it matches `GOOGLE_VOLUME_ID_RE` → return `GB{volume}`.
3. Else → None, and the volume is dropped from results.

**The `isbn` passed in comes from `_volume_isbn(volumeInfo)`:**

- It iterates `industryIdentifiers` in order and skips non-dicts.
- For each entry: `isbn = extract_isbn(identifier)` (strip whitespace and `-`, uppercase, must match the 10/13 shape; **no checksum**).
- If `type == "ISBN_13"` **or** `len(isbn) == 13` → **return it immediately**.
- If `type == "ISBN_10"` **or** `len(isbn) == 10` → remember it. A later ISBN-10 overwrites an earlier one.
- At the end it returns the remembered ISBN-10, or None.

Consequences:

- An ISBN-13 always wins if present.
- With several ISBN-10s and no ISBN-13, the **last** ISBN-10 wins.
- An identifier of type `OTHER` whose value happens to be 10 or 13 digits (or 9 digits + X) is accepted as an ISBN.

Verified examples:

| `industryIdentifiers` | Result |
|---|---|
| `[ISBN_10 0316769487, OTHER UOM:39015, ISBN_13 9780316769488]` | `9780316769488` |
| `[OTHER 1234567890]` | `1234567890` |
| `[ISBN_10 0316769487, ISBN_10 0062060627]` | `0062060627` |

So a Google work id is `ISBN` + ISBN-13 (preferred) or ISBN-10, uppercase X, no hyphens. Failing that it is `GB` + the raw volume id (1–40 chars of `[A-Za-z0-9_-]`).

### 2.3 Volume mapping

**`map_volume(volume)` → `SearchHit` or None:**

- It requires a dict with a dict `volumeInfo` and a non-empty stripped `title`, and `google_catalog_work_id` must not be None.

| `SearchHit` field | Value |
|---|---|
| `ol_work_key` | `work_key(id)` |
| `title` | stripped |
| `authors` | `", ".join(str(n) for n in authors if n)`; a non-list gives `""` |
| `cover_id` | None |
| `year` | `_year(publishedDate)` |
| `isbn` | from `_volume_isbn` |
| `cover_url` | `cover_url_from_image_links(imageLinks)` |

- **`_year`:** `str(value).strip()`. It needs at least 4 chars and the first 4 must be digits. The year is `int(first 4)` and must fall in 1..3000, else None. Examples: `"2018-04-10"`→2018, `"2024"`→2024, `"201"`→None, `"0000"`→None, `"c2018"`→None.

**`map_volumes(items)`.** Maps in order, skips None and skips any repeat of an `ol_work_key` already seen (first wins). Two different volume ids with the same ISBN collapse to one hit.

**`volume_details(volume)` → `VolumeDetails`.** It is `map_volume` plus the fields below.

| Field | Value |
|---|---|
| `work_key` | as above |
| `volume_id` | stripped `id` |
| `title`, `authors`, `year`, `isbn` | as above |
| `description` | `_clean_description(description)`: html-unescape → `<br\s*/?>` (case-insensitive) → `\n` → `</p>` (case-insensitive) → `\n\n` → strip every `<…>` tag → collapse 3+ newlines to `\n\n` → strip. **Always a `str`**, `""` when missing. |
| `pages` | `int(pageCount)` or None |
| `subjects` | `_clean_subjects(categories)`: non-list → `[]`. Drop empty entries and entries over 40 chars, dedupe case-insensitively (first casing wins), keep at most 12. **Always a `list`.** Unlike OL cleaning, `:` and `,` are allowed, so BISAC-like `"Fiction / Fantasy / Epic"` survives. |
| `rating` | `round(float(averageRating), 2)` or None |
| `rating_count` | `int(ratingsCount)` or None |
| `cover_id` | always None |
| `cover_image_url` | the hit's `cover_url` |

### 2.4 Cover URLs

**`normalize_cover_image_url(v)`:**

- `str(v or "").strip()`; empty → None.
- A leading `http://` is rewritten to `https://`.
- The scheme must be `https` with a non-empty netloc.
- `len ≤ 512` (`COVER_URL_MAX`).
- No userinfo.

**`storable_cover_image_url(v)`.** It normalises the URL, then takes the host (netloc lowercased, port stripped).

- Rejected if the host is `covers.openlibrary.org` or `*.covers.openlibrary.org`.
- Rejected unless the host is `books.google.com`, `*.books.google.com`, `googleusercontent.com` or `*.googleusercontent.com`.

So **only Google hosts are ever stored**.

**`cover_url_from_image_links(links)`:**

- A non-dict gives None.
- It checks keys in the order `extraLarge, large, medium, small, thumbnail, smallThumbnail`.
- On the **first** key whose value normalises, it returns `storable_cover_image_url(_prefer_larger_google_thumb(url))` and stops. If that first normalisable URL has a non-Google host, the result is **None**: later keys are not tried (verified).
- **`_prefer_larger_google_thumb`:** parses the query (keeping blank values), drops `edge=curl` and rewrites `zoom=1` to `zoom=0`. If anything changed, the query is re-encoded with `urlencode`, which quote-plus-encodes (a space becomes `+`). Otherwise the URL is returned byte-for-byte.
- Example: `http://books.google.com/books/content?id=zyTCAlFPjgYC&printsec=frontcover&img=1&zoom=1&source=gbs_api` → `https://books.google.com/books/content?id=zyTCAlFPjgYC&printsec=frontcover&img=1&zoom=0&source=gbs_api`.

**Render (`serialize.cover_url(cover_id, ol_work_key, cover_image_url)`).** `book_cover_url(book)` is used for every `cover_url` in responses (`BookOut`, details, local hits, member previews). The first matching case wins:

1. `storable_cover_image_url(cover_image_url)` (re-validated at read time).
2. `cover_id > 0` → `https://covers.openlibrary.org/b/id/{cover_id}-L.jpg?default=false`.
3. **Google mode** → None. The OL ISBN CDN is never invented.
4. An `ISBN…` key → `https://covers.openlibrary.org/b/isbn/{isbn}-L.jpg?default=false`.
5. None.

`GB…` keys without a stored cover → None.

**`_set_cover(book, cover_id, cover_image_url)`:**

- Only non-None values are applied, so **a Google cover URL is never cleared once stored.**
- `cover_id` passes through `_usable_cover` (must be > 0).
- If `(cover_id, cover_image_url)` changed, `spine_color = None`, so the colour is sampled again later.

### 2.5 Query construction (`google_search_query`)

- An ISBN-shaped query → `isbn:{ISBN}`, ignoring the subject.
- Otherwise the query, a space, and `subject:"{label}"` with `_` → space. Either part may be absent.
- Examples:
  - `978-0-316-76948-8` → `isbn:9780316769488`
  - (`circe`, `science_fiction`) → `circe subject:"science fiction"`
  - (`""`, `science_fiction`) → `subject:"science fiction"`
  - (`978-0-316-76948-8`, `fiction`) → `isbn:9780316769488`
  - `isbn:9780316556347` → unchanged (the literal prefix is not recognised)

### 2.6 Pagination and the topic fallback

- `startIndex = (page-1)*limit`.
- `has_more` requires that mapped items exist. An empty later page reports `has_more=false` even when `totalItems` stays high.
- **Topic fallback conditions:** Google mode, empty query, non-empty subject, page 1, and zero mapped items.
- **The fallback never runs** for an explicit query (with or without a subject) or for any page > 1.
- **After the fallback** `has_more` is forced false, so the client does not append subject page 2 to topic results.
- **Errors in the fallback** surface with the same classification as the primary request: 429→429, other→502. They are not swallowed.

### 2.7 Response cache (Google)

- **Shape:** one process-wide `OrderedDict` LRU used for search, ISBN and volume responses.
- **TTL:** 18 h, set at write time (`time()` wall clock).
- **Size cap:** 256 keys. Every write purges expired entries, then evicts the oldest beyond 256.
- **Read:** an expired entry is popped on read. A hit moves the entry to the end (LRU).
- **Keys:** `gb|search|{q.casefold()}|{sort}|{page}|{limit}`, `gb|isbn|{isbn}`, `gb|volume|{volume_id}`.
  - The search endpoint has already collapsed whitespace and the key is case-folded, so `circe`, `Circe` and `"  CIRCE  "` share one entry.
  - The ISBN key is already uppercase.
- **Negative caching.** Search and ISBN calls use `cache_empty=False`. A 200 whose `items` is missing, not a list or empty is **not cached**, so the next request retries upstream. Volume fetches cache any successful payload.
- **In-flight coalescing.** Identical concurrent requests (same cache key, not bypassing) await one shared future (`asyncio.shield`).
  - The owner resolves the future with the result or the exception.
  - Cancellation resolves waiters with `GoogleBooksError()`.
  - The entry is removed in `finally`.
  - A done-callback retrieves the exception so no "Future exception was never retrieved" warning is logged.
- **`bypass_cache=True`** (refresh) skips both the cache read and joining an in-flight request. The result is still written.
- **Reset.** `clear_google_cache()` clears the cache and the in-flight map and resets the cooldown to 0. The test fixture calls it.

### 2.8 Discovery shortcut (`cached_volume_details(work_id)`)

`_load_google_catalog` calls this before any network call, but only when Google is enabled and the call is not a refresh.

- It scans cache entries **newest first** and skips expired ones.
- Any payload with an `items` list is examined, so search **and** ISBN payloads count.
- For each item it runs `map_volume` and compares the derived key with `work_key(work_id)`.
- On a match it moves that cache entry to the end and returns `volume_details(volume)`.

Effect: opening any result that was displayed within the TTL needs **no Google call**, even during a cooldown or an outage, for both `ISBN…` and `GB…` keys. The book row is created at that moment.

After the TTL expires, opening calls Google:

- `ISBN…` → `lookup_isbn`
- `GB…` → `fetch_volume`

**`_load_google_catalog` full order:**

1. Discovery shortcut (Google mode, not refresh).
2. **GB key.**
   - Google mode → `fetch_volume`.
   - OL mode → 400 `This book was added from Google Books. Set GOOGLE_BOOKS_API_KEY to refresh.` on refresh, else 404 `No such book.`.
3. **ISBN key.**
   - Google mode → `lookup_isbn`; None → 404 `No such book.`.
   - OL mode → OL `lookup_isbn` (its own 64-key, 18 h cache; bypassed on refresh); None → 404.

### 2.9 Errors, retry, cooldown

**`_fetch_json_uncached`:**

- If cooling down → raise `GoogleBooksError(status=429, quota=True)` **without any HTTP request**.
- **Status ≥ 400:** parse `{"error": {"message", "errors": [{"reason"}]}}`, ignoring malformed bodies.
  - `quota = status == 429 or (status == 403 and reason in {rateLimitExceeded, dailyLimitExceeded, userRateLimitExceeded})`.
  - If `quota` → trip the cooldown.
  - Log WARNING `google books request failed url=… status=… latency_ms=… reason=… message=…`.
  - Raise `GoogleBooksError(status, reason, quota)`.
- **`httpx.HTTPError`** (transport): status comes from `exc.response` if present, the same quota check applies, and it raises `GoogleBooksError(status=None or code)`.
- **Invalid JSON or a non-dict** → `GoogleBooksError()` with status None, which becomes 502.
- **Success** → log INFO `google books ok url=… latency_ms=…`.

**Cooldown:**

- 120 s (`_COOLDOWN_SEC`), measured on the wall clock, process-wide.
- It only trips for quota errors. A 403 `accessNotConfigured` → 502 and **no** cooldown.
- A 503 never trips it.
- While active, every non-cached Google call fails fast with 429, including Goodreads (§1.9).

**Retry:**

- Only `status in {502, 503, 504}`, **once**, after `asyncio.sleep(0.3)`. One retry is shared by all coalesced waiters.
- A 500 is **not** retried. Quota and auth errors are not retried, and transport errors (status None) are not retried.
- A persistent 503 → 2 upstream calls → 502 to the client, no cooldown. The next request tries again.

### 2.10 Import and upsert of Google details (`_import_google_details(session, work_id, details)`)

1. **Key.** The key is `work_key(work_id)`, i.e. the **requested** id, not `details.work_key` (§2.11).
2. **New row.** If no row exists → `upsert_book(key, title=details.title or work_id, authors, cover_id=None, cover_image_url, year)`.
3. **`apply_book_details`.** On a new or existing row:

   | Column | Rule |
   |---|---|
   | `title` | overwritten if non-empty |
   | `authors` | overwritten if non-empty |
   | `cover_image_url` | set if storable and non-None (`_set_cover`) |
   | `year` | set if not None |
   | `description` | set if not None. Google always passes a `str`, so a successful refresh of a volume **without** a description **wipes** it to `""`. |
   | `pages` | set if not None |
   | `subjects` | set to `json.dumps(subjects, ensure_ascii=False)` if not None. Google always passes a list, so a missing `categories` **wipes** it to `"[]"`. |
   | `ol_rating` | set to Google `averageRating` if not None |
   | `details_fetched_at` | `utcnow()` |

4. **Commit and reload.** If the reload fails → 500 `Could not save that book`.

**Refresh semantics:**

- A miss (404) keeps the row byte-for-byte.
- A hit overwrites the fields as in step 3.
- An OL-mode ISBN refresh passes description and subjects as None, so they are kept, and it also passes no cover URL.

### 2.11 Requested key versus derived key

These can differ:

- **`/works/ISBN{x}`** where Google's `isbn:` search returns a volume whose primary ISBN differs: an ISBN-10 requested and an ISBN-13 volume returned, or another edition.
  - The row is stored under **`ISBN{x}`** with that volume's data.
  - Test `test_isbn_x_and_X_are_the_same_row`: requesting `ISBN080442957X` imports Circe (`9780316769488`) data under `/works/ISBN080442957X`.
- **`/works/GB{id}`** for a volume that does have an ISBN: the row is stored as `GB{id}` and the ISBN is **not stored anywhere**.
- **Goodreads**: the stored key is Google's derived key, not the CSV ISBN. In the fixture, CSV ISBN13 `9780316556347` becomes `/works/ISBN9780316769488`.
- **`GET /api/books/isbn/{isbn}`**: returns the requested `isbn` and the derived `ol_work_key`.

### 2.12 Other behaviour visible in Google mode (mostly provider-agnostic)

- **Opening a known book never calls upstream.** This holds for `ISBN…`, `GB…`, `OL…` and `BC…` keys alike.
- **Club custom books (`BC…`).** Created locally, never call a catalog. Refresh → 400 `This book was added by the club, not Open Library.`.
- **ISBN `x`/`X`.** Both refer to the same row on detail, on shelf add (the second add → 409) and in any `upsert_book`.
- **Shelf add with an `ISBN…` key and no metadata** creates a bare row with `description ""` and `subjects "[]"`. A later refresh fills it from Google.

---

## 3. Data

### 3.1 `books` table (SQLite, `models.Book`)

The provenance column says what each column holds on Google-derived rows. "ISBN row" means `ol_work_key ~ ^/works/ISBN(\d{9}[\dX]|\d{13})$` and "GB row" means `^/works/GB[A-Za-z0-9_-]{1,40}$`.

| Column | Type / constraint | Meaning | Google provenance | Importer action |
|---|---|---|---|---|
| `id` | int PK | | | Map legacy id → new work id (+ edition id). |
| `ol_work_key` | str, unique, indexed, max 64 | `/works/{id}`; the prefix gives the source (§2.2). | `ISBN…` / `GB…` minted by `google_catalog_work_id`, or the **requested** id (§2.11). ISBN rows can also be created in OL mode or by any client. | Classify; ISBN → re-match; keep a legacy-key → work mapping. |
| `title` | str, NOT NULL | Display title. **Overwritten by every `upsert_book`** (shelf add, pick, nomination, diary, quote, Goodreads) with the client-sent title. | Google `volumeInfo.title`, or a client echo of it, or the OL title after an OL-mode ISBN refresh. | Keep as a bare fact if unmatched. |
| `authors` | str, default `""` | Comma-joined names. | Google `authors` joined with `", "`. | Keep as a bare fact; split on `", "` into author records (lossy: names with commas). |
| `cover_id` | int\|null | OL cover id, >0 or null. | **Never Google.** Always None from Google. On ISBN rows it can be an OL cover (OL-mode lookup or a client-sent value). | Keep: an OL cover id is a valid source for the media store. |
| `cover_image_url` | str\|null, max 512 (model) / `VARCHAR` (DDL) | Remote cover override; **wins over `cover_id`** at render time. | **Always Google** (books.google.com / googleusercontent.com, https, zoom=0). It can be on **any** key type, including `OL…`/`BC…`, through client `cover_url`. Never cleared once set. | **Drop on every row** (plan). The visible cover then falls back to the OL `cover_id`, or none. |
| `spine_color` | str\|null, max 7 | `#rrggbb` dominant cover colour; null = not sampled; `""` = no usable cover. | Sampled from whatever cover rendered, which is the Google cover when `cover_image_url` is set. | Reset to null when the cover source changes (mirrors `_set_cover`). See §6, risk 28. |
| `year` | int\|null | Google rows: the **volume's** `publishedDate` year (edition year, often a reprint). OL rows: `first_publish_year`. | Google `publishedDate[:4]` (1..3000). | Keep as a bare fact; put it on the edition. Do not assume it is the first-publication year. |
| `description` | str NOT NULL default `""` | Plain-text synopsis. | On ISBN/GB rows **any non-empty value is Google's** HTML-stripped description (OL-mode ISBN imports pass None). A successful refresh can wipe it to `""`. | **Drop for ISBN/GB rows.** |
| `pages` | int\|null | Page count. | Google `pageCount` (edition-specific), or OL `number_of_pages_median` on OL-mode ISBN rows. **Read by stats** (`stats.py`: pages read, longest book) and `BookOut.pages` (physical shelf spine). | Keep as a bare fact on the edition. Stats parity depends on it. |
| `subjects` | str (JSON list) NOT NULL default `"[]"` | Up to 12 labels. | ISBN/GB rows: Google `categories` (BISAC-like, may contain `/`, `:`, `,`). | **Drop for ISBN/GB rows.** |
| `ol_rating` | float\|null | Labelled "Open Library rating". | ISBN/GB rows: **Google `averageRating`**, rounded to 2 dp. | **Drop for ISBN/GB rows.** |
| `details_fetched_at` | datetime(tz)\|null | Last successful `apply_book_details`. | Set on Google import or refresh. Null for rows created only by shelf add or Goodreads. | Drop. |

**Not stored anywhere:**

- The Google volume id of an ISBN row.
- The ISBN of a GB row.
- `ratingsCount` (only returned once, in `BookDetailsOut.ol_rating_count`).
- The other ISBN of a volume (its ISBN-10 sibling).
- Google's `publishedDate` beyond the year.

### 3.2 Migrations in `db.py` touching this area

- `COLUMN_MIGRATIONS["books"]` adds these columns to old databases:

  | Column | DDL |
  |---|---|
  | `description` | `VARCHAR NOT NULL DEFAULT ''` |
  | `pages` | `INTEGER` |
  | `subjects` | `VARCHAR NOT NULL DEFAULT '[]'` |
  | `ol_rating` | `FLOAT` |
  | `details_fetched_at` | `DATETIME` |
  | `cover_image_url` | `VARCHAR` |
  | `spine_color` | `VARCHAR` |

- No data migration rewrites Google data. `import_legacy_pick_posts` is unrelated.
- **Canonical uppercase `X` on write** and the **Google-host allowlist** shipped in the same squash commit (`51c35df`) that introduced `cover_image_url`. A production DB therefore should not contain lowercase-`x` ISBN keys or non-Google `cover_image_url` values. The importer must still canonicalise and validate defensively.

### 3.3 Tables that reference `books.id`

Re-matching can merge several legacy book rows into one new work: an ISBN-10 row, an ISBN-13 row, an OL row and other-edition rows of the same work. These references must be re-pointed:

| Table | Column(s) | Constraint that can collide on merge |
|---|---|---|
| `shelf` | `user_id, book_id` | **UNIQUE(user_id, book_id)** |
| `user_favorites` | `user_id, book_id, position` | **UNIQUE(user_id, book_id)**, UNIQUE(user_id, position) |
| `club_picks` | `book_id` | none |
| `book_posts` (diary) | `book_id` | none |
| `quotes` | `book_id` (+ `page` 0..20000, edition-relative) | none |
| `events` | `book_id` nullable | none |
| `next_up_nominations` | `vote_id, book_id` | **UNIQUE(vote_id, book_id)**; ballots reference nomination ids |
| `physical_shelf_layouts` | `layout_json`: shelf **entry ids** + per-entry spine appearances | Merged or removed shelf entries must be rewritten in the JSON. |

### 3.4 Importer algorithm for Google-derived rows (recommended, #38)

1. **Classify** each `books` row by `canonical_work_id(work_id_from_key(ol_work_key))`:
   - `OL` (`^OL\d+W$`)
   - `BC` (`^BC[a-f0-9]{10}$`)
   - `ISBN` (`^ISBN(\d{9}[\dX]|\d{13})$`)
   - `GB` (`^GB[A-Za-z0-9_-]{1,40}$`)
   - `OTHER`: any other `/works/<segment>`, possible because shelf, pick, vote and quote only check the `/works/` prefix

   If canonicalisation produces a key that collides with another row, merge them.
2. **ISBN rows.** Take `raw = id[4:]`.
   - If the length is 10, validate the ISBN-10 checksum (mod 11, `X` = 10). If valid, convert: `isbn13 = "978" + raw[:9] + check13`.
   - If the length is 13, validate the ISBN-13 checksum (mod 10) and require a `978`/`979` prefix.
   - If invalid, treat the row as **no ISBN**: bare facts, no edition ISBN. Record the raw value in the import report.
3. **Lookup** `isbn13` in order:
   1. The local catalog (DNB-harvested editions).
   2. DNB SRU, if the importer is allowed to call it, with the throttle from #41.
   3. Open Library by ISBN (UA with contact, ≤ 3 req/s).

   - **Match** → map the legacy book to that work. The edition for that ISBN exists or is created from the CC0 source.
   - **No match** → create a work + edition with **bare facts only**: title, authors, ISBN-13 (+ ISBN-10 when 978), year (edition), pages (edition), and `cover_id` if present (OL cover, fetched into the media store). **Never** description, subjects, `ol_rating`, `cover_image_url`, `details_fetched_at`, or a `spine_color` sampled from a Google cover.
4. **GB rows.** There is no ISBN to re-match with, so they always get bare facts: title, authors, year, pages, OL `cover_id` if any. The work has no ISBN edition. Title/author fuzzy matching is an open question.
5. **Every row.** Drop `cover_image_url`. Reset `spine_color` where the rendered cover changes.
6. **Merge collisions** (§3.3):
   - Duplicate shelf entries of one user on one work: choose a winner by a documented rule (e.g. the most advanced status, then the latest `updated_at`) and keep the loser's rating, take and dates where the winner has none.
   - Duplicate favourites: keep the lower position and compact the positions.
   - Duplicate nominations in one vote: merge their ballots.
   - Rewrite the physical layout JSON.
7. **Keep a `legacy_work_key → work_id` mapping** in a table, to redirect old `/book/ISBN…`, `/book/GB…` and `/book/OL…` URLs (#48) and to make the dry-run report auditable.
8. **Dry-run report.** Books do not map 1:1 to works, so report `books: N source → W works (m merged; i ISBN-matched; u ISBN-unmatched bare; g GB bare; x invalid-ISBN bare; o OTHER)`, and report shelf entries, favourites and nominations dropped or merged by collision.

---

## 4. Tests: `backend/tests/test_google_books.py`

There are 51 test functions and 78 collected cases with parametrisation. Each line ends with a verdict:

- **DROP:** Google-only; record "Google Books removed (#41)" as the reason.
- **KEEP:** a provider-agnostic rule; port it against OL/DNB/local.
- **ADAPT:** keep the intent, change the mechanics.
- **INVERT:** the plan requires the opposite.

**Fixtures:**

- `client`: OL mode, no key.
- `gb`: `GOOGLE_BOOKS_API_KEY=test-gb-key`, all caches reset, a fake `httpx.AsyncClient` patched into `routers.books`, `googlebooks` and `openlibrary`, and user `ada` registered. The fake asserts `"Bookclub/1.0"` in the UA, that `key` is absent from Google params, and `X-Goog-Api-Key == "test-gb-key"`.
- Fake data:
  - `CIRCE_VOLUME`: id `zyTCAlFPjgYC`, ISBN-13 `9780316769488` + ISBN-10, `<p>` description, 393 pages, categories Fiction/Mythology, rating 4.5/88, http thumbnail with zoom=1.
  - `NO_ISBN_VOLUME`: id `abcVolumeId1`.
  - `ACHILLES_VOLUME`: ISBN-13 `9780062060624`.
  - The fake returns empty items for `q` starting `isbn:0000000000` or `isbn:9780000000002`, or `q == "zzzzempty"`. `"Song of Achilles"` in `q` → Achilles. Otherwise it returns `gb_items`.

**Tests:**

1. `test_google_books.py::test_google_books_disabled_without_key`: with no key set (`client` fixture), `google_books_enabled()` is False. **DROP**
2. `test_google_books.py::test_google_catalog_ids`: `is_work_id` accepts `ISBN9780316769488` and `GBzyTCAlFPjgYC`. `is_google_catalog_id` is true for both and false for `OL1W` and `BCdeadbeef01`. `isbn_from_work_key("/works/ISBN9780316769488")=="9780316769488"`, `google_volume_id("GBzyTCAlFPjgYC")=="zyTCAlFPjgYC"`, `google_catalog_work_id(isbn="978-0-316-76948-8")=="ISBN9780316769488"`, `google_catalog_work_id(isbn=None, volume_id="zyTCAlFPjgYC")=="GBzyTCAlFPjgYC"`, `google_search_query("978-0-316-76948-8")=="isbn:9780316769488"`, `google_search_query("circe","science_fiction")=='circe subject:"science fiction"'` and `canonical_work_id("ISBN080442957x")=="ISBN080442957X"`. **DROP** the id formats. **KEEP** as ISBN normalisation tests in #41 (hyphen stripping, uppercase X) and in importer key classification (#38).
3. `test_google_books.py::test_map_volume_prefers_isbn_key`: Circe maps to `/works/ISBN9780316769488`, title, authors `Madeline Miller`, year 2018, isbn, `cover_id` None, `cover_url` = https zoom=0 URL. **DROP**
4. `test_google_books.py::test_map_volume_without_isbn_uses_volume_id`: a volume without an ISBN maps to `/works/GBabcVolumeId1` with `isbn` None. **DROP**
5. `test_google_books.py::test_map_volumes_skips_junk_and_duplicates`: `[Circe, {"id":"x"}, Circe-with-other-id, NoIsbn]` → keys `[ISBN9780316769488, GBabcVolumeId1]`. Junk without `volumeInfo` is skipped, and the same ISBN under another volume id is deduped with the first one winning. **ADAPT**: OL/DNB result mappers must skip records without a title or id and dedupe one page by work.
6. `test_google_books.py::test_volume_details_strips_html`: `<p>…</p>` becomes `A witch on an island.`, pages 393, subjects `[Fiction, Mythology]`, rating 4.5, cover zoom=0. **DROP**: Google descriptions are not stored and DNB/VLB blurbs are never stored.
7. `test_google_books.py::test_search_prefers_google_books`: `q=circe` → one Google call with `q=circe`, `fields` containing `imageLinks`, the key in the header and not in params, no OL call. The hit has the ISBN key, title, isbn, year 2018 and cover. Then `Circe` and `"  CIRCE  "` are served from cache (still one call). **DROP** the provider. **KEEP** "search cache key is case- and whitespace-insensitive" for the remote lookup cache in #41.
8. `test_google_books.py::test_search_empty_google_does_not_fall_back`: empty Google items → 200 `items: []`, no OL call. **INVERT**: the new chain is local → DNB → OL, so a miss in one source falls through to the next.
9. `test_google_books.py::test_empty_google_response_does_not_freeze_retries` [8 cases: payload `{"totalItems":0}` or `{"totalItems":0,"items":[]}` × search `q=circe` / trending / `subjects/fiction` / `isbn/9780316769488`]:
   - The first request gets an empty upstream: search/browse → 200 `items: []`, ISBN → 404. Trending and subjects also make an empty topic-fallback call.
   - The retry reaches upstream again and returns Circe: 2 calls in total, 3 for trending/subjects.
   - A third identical request is served from cache with no new call and an identical body. No OL call.
   - **KEEP**: never negatively cache an empty remote result (missing or empty list), for OL search, OL ISBN and DNB SRU. Non-empty results are cached.
10. `test_google_books.py::test_search_google_429_does_not_fall_back`: Google 429 → 429 with `detail` containing "busy", no OL. **ADAPT**: an upstream 429 → 429 `The library is busy…` when no source answered. The "no fallback" part is INVERT.
11. `test_google_books.py::test_empty_subject_preview_recovers_with_google_topic_search` [4 cases: trending limit 14 / `subjects/fiction` 14 / `subjects/science_fiction` 14 / `search?subject=fantasy` default 24]:
    - Primary `q = subject:"<topic>"` returns 0 items, then the fallback `q = <topic>` is sent. `orderBy` is `newest` only for trending; `maxResults` equals the limit.
    - The result shows Circe with `has_more=false` despite `totalItems=1000`.
    - The log contains `fallback_items=1`. No OL.
    - Then `GET /api/books/works/ISBN9780316769488` → 200 with no new call (discovery cache).
    - **DROP** (Google-specific workaround). The "open a displayed result without another upstream call" part becomes **ADAPT** (persist on search).
12. `test_google_books.py::test_empty_explicit_search_or_later_page_is_not_broadened` [2: `q=nonexistent&subject=fiction`; `subject=fiction&page=2`]: empty items → 200 `items: []`, `has_more=false`, exactly one Google call. **KEEP** the principle: an explicit query or a later page is never broadened or replaced by suggestions.
13. `test_google_books.py::test_subject_fallback_keeps_provider_errors_visible` [4: status 429→429 or 500→502 × primary fails, or primary empty then fallback fails]: `/subjects/fiction` returns the classified error. Upstream calls: 1 if the primary fails, 2 if the fallback fails. No OL. **ADAPT**: an upstream error must surface as classified (429/502) and not be masked as an empty or suggestion result. A 500 is not retried.
14. `test_google_books.py::test_search_google_500_does_not_fall_back`: Google 500 → 502, no OL. **ADAPT**: 5xx → 502 `Could not search the library right now. Try again.` when no source answers. Fallback per the new chain.
15. `test_google_books.py::test_browse_and_empty_search_use_google_not_open_library`:
    - `search?subject=fantasy` → Google `q` contains `subject:"fantasy"`.
    - Trending → `q == 'subject:"fiction"'`, `orderBy=newest`, cover URL, no OL trending.
    - An empty search serves trending through Google (Circe).
    - `subjects/science_fiction` → `subject:"science fiction"`, no OL `/subjects/`.
    - **DROP** the provider. **KEEP** "empty search box serves the trending/new-releases list" (cf. `test_search.py::test_empty_search_serves_trending`).
16. `test_google_books.py::test_isbn_search_uses_google_isbn_field`: `q=978-0-316-76948-8` → upstream `q == "isbn:9780316769488"`, hit key `/works/ISBN9780316769488`. **ADAPT**: an ISBN-shaped search query (hyphens and spaces allowed) is routed to ISBN lookup (local edition, then DNB, then OL), not to text search.
17. `test_google_books.py::test_detail_imports_from_google_not_open_library`: `GET works/ISBN9780316769488` → title, description, `custom=false` and cover from Google, no OL. A second GET makes zero upstream calls. **DROP** the Google import. **KEEP** "second open of a known book is served locally with zero upstream calls".
18. `test_google_books.py::test_opening_discovered_book_does_not_call_google` [12: search / trending 14 / subjects 14 × Circe or no-ISBN volume × upstream 429 or 503]:
    - The browse returns 200 and **no books row exists afterwards**.
    - Then, with upstream failing, opening `works/<id>` → 200 with the title and description, a row now exists, zero upstream calls, and no cooldown tripped.
    - A further search succeeds with exactly one call.
    - **INVERT**: the plan copies everything searched into the catalog, so a row **should** exist after search. **KEEP**: opening a displayed result never needs an upstream call and works during an outage.
19. `test_google_books.py::test_title_hit_opens_even_when_google_isbn_lookup_misses` [2: `9781408857892`, `9788835713951`]: a text search finds the volume by title. With items then emptied, `GET works/ISBN<isbn>` → 200 with title `A Court of Mist and Fury` and zero calls. **KEEP**: opening a search result must not depend on a second (ISBN) lookup succeeding.
20. `test_google_books.py::test_google_transient_503_is_retried_once`: the first call returns 503, the second succeeds → trending 200 Circe in 2 calls. The repeat is cached (still 2). **ADAPT**: decide whether the OL/DNB clients retry 502/503/504 once after ~0.3 s. The current OL client does not.
21. `test_google_books.py::test_google_persistent_503_stops_after_one_retry_and_can_recover`: a persistent 503 → 502 after exactly 2 calls, no cooldown. After recovery the next request makes 1 call (3 in total) and returns 200. **ADAPT** (same decision as 20): at most one retry, and a 503 does not open a circuit breaker.
22. `test_google_books.py::test_google_failure_does_not_leave_unobserved_future`: a failing `_fetch_json` with no waiters logs no asyncio "exception never retrieved" and removes the in-flight entry. **ADAPT**: Rust request coalescing (e.g. a shared future or a `tokio::sync` map) must remove the in-flight entry on error or cancellation and wake waiters with an error.
23. `test_google_books.py::test_discovered_book_opens_during_existing_google_cooldown` [2: `works`, `work`]: trending cached, then a 429 on an uncached search trips the cooldown. Opening `works/ISBN…` or `work/ISBN…` → 200 with the description and zero calls. **ADAPT**: with persist-on-search, opening is purely local, so it is trivially independent of the upstream throttle state.
24. `test_google_books.py::test_discovered_book_refresh_still_fetches_fresh_metadata`: after a trending discovery, `POST works/ISBN…/refresh` bypasses the discovery and response caches and makes exactly 1 call, returning `Updated synopsis.`. **ADAPT** if refresh survives: refresh always goes to the source and bypasses caches. The description field itself is DROP.
25. `test_google_books.py::test_expired_discovery_does_not_supply_book_details`: after `time()` moves past the 18 h TTL, opening the ISBN key makes exactly 1 Google call. **DROP**: discovery is cache-based; the new design persists.
26. `test_google_books.py::test_volume_key_detail_and_refresh`: `GET works/GBabcVolumeId1` → key, title `Local Zine`, cover `https://books.google.com/books/content?id=abcVolumeId1&printsec=frontcover&img=1&zoom=0&source=gbs_api`, a call to `/volumes/abcVolumeId1`. A refresh after the volume changes → `Updated copy.`. **DROP**
27. `test_google_books.py::test_known_google_book_skips_upstream`:
    - `POST /api/shelf` with an ISBN key, then `GET works/ISBN…` → `on_shelf=currently_reading` with zero calls.
    - `PUT /api/pick` with an ISBN key → 200.
    - `GET /api/books/works/ISBN…/posts` → 200 echoing the `ol_work_key`.
    - **KEEP**: a known book is served locally, can be picked and has a diary, whatever its source. Legacy ISBN-keyed books after import must also support shelf, pick and diary.
28. `test_google_books.py::test_refresh_updates_google_isbn_book`: shelf add with a bare title → detail `description==""`. Refresh → Google fills `A witch on an island.`, with no OL `/works/OL` call. **DROP** (Google). **ADAPT**: refreshing a bare user-created record from a CC0 source fills missing facts.
29. `test_google_books.py::test_isbn_lookup_prefers_google`: `GET /api/books/isbn/9780316769488` → ISBN key, title Circe, Google cover, no OL. **DROP** the provider. **KEEP** "ISBN lookup returns title, authors, year, pages and cover for a known ISBN" (#41 AC, with German and English ISBNs).
30. `test_google_books.py::test_club_custom_book_still_skips_catalogs`: `POST /api/books/custom` → 201. `GET works/BC…` → `custom=true` with zero calls. Refresh → 400. **KEEP**: user-added books never call a catalog, and refreshing them is rejected (new message TBD).
31. `test_google_books.py::test_cover_url_from_image_links_https_and_zoom`:
    - Circe `imageLinks` → https zoom=0 URL; `None` → None; `javascript:alert(1)` → None.
    - `normalize_cover_image_url` rewrites http→https.
    - `storable_cover_image_url` keeps the Google URL and rejects the OL CDN URL and `https://attacker.example/pixel.gif`.
    - **DROP** the Google parts. **KEEP**: the server never stores a cover URL supplied by a client or an upstream; covers are fetched server-side and served from our domain (#43).
32. `test_google_books.py::test_isbn_cover_url_from_work_key` (no key):
    - `cover_url(None, ISBN key)` → `https://covers.openlibrary.org/b/isbn/9780316769488-L.jpg?default=false`.
    - `cover_url(123, ISBN key)` ends with `/123-L.jpg?default=false`.
    - A GB key with no URL → None.
    - A GB key with an http Google URL → the https URL.
    - **DROP**: there are no client-facing OL URLs in the new design (#43).
33. `test_google_books.py::test_google_cover_wins_over_open_library_isbn_cdn`: `cover_url(123, ISBN key, CIRCE_COVER)` → CIRCE_COVER. **DROP**
34. `test_google_books.py::test_isbn_cdn_skipped_when_google_key_is_set`: with the key set, `cover_url(None, ISBN key)` → None and `cover_url(555, "/works/OL1W")` → the OL id CDN URL. **DROP**
35. `test_google_books.py::test_later_google_page_does_not_mix_open_library`: `totalItems=50`. Page 1 has `has_more=true`. Page 2 is empty → `items: []`, `has_more=false`, no OL search. **KEEP**: a later page never mixes in another source's results, and an empty page ends pagination.
36. `test_google_books.py::test_title_and_popular_sorts_stay_on_google`: `sort=title` → no `orderBy`. `sort=readinglog` → 200 with the cover, served from the shared relevance cache. No OL. **DROP** (the Google sort mapping). Sort semantics for the local catalog are decided in #41.
37. `test_google_books.py::test_isbn_refresh_keeps_description_when_google_misses`: after an import, a refresh that misses (empty items) → 200 with the description, subjects `[Fiction, Mythology]` and cover unchanged. No OL. **KEEP**: a refresh miss never wipes the local record.
38. `test_google_books.py::test_isbn_refresh_without_key_keeps_description`: imported via Google, then the key is removed. The refresh (via OL ISBN) → 200 with the description kept and no Google call. **ADAPT**: a refresh from a source without a description must not wipe existing fields (None ≠ empty). The Google part is DROP.
39. `test_google_books.py::test_isbn_x_and_X_are_the_same_row`: `GET works/ISBN080442957X` and `…080442957x` both → 200 with `ol_work_key /works/ISBN080442957X`. **KEEP**: ISBN-10 check digit `x`/`X` are the same edition (normalise to uppercase) on lookup and on legacy-key redirect.
40. `test_google_books.py::test_isbn_lookup_miss_does_not_use_open_library`: empty Google → `isbn/9780316769488` 404 with no OL. **INVERT**: an ISBN miss in local or DNB falls through to OL. A final miss is 404 `No book found for that ISBN`.
41. `test_google_books.py::test_shelf_add_persists_google_cover_url`: shelf add with Google `cover_url` → 201 with `book.cover_url` = it. Detail returns it with zero calls. **DROP/INVERT**: client cover URLs are no longer accepted.
42. `test_google_books.py::test_volume_key_without_google_key` (no key): `GET works/GBabcVolumeId1` → 404. Refresh → 400 whose detail contains `GOOGLE_BOOKS_API_KEY`. **DROP**. After import, `GB…` legacy keys resolve through the legacy-key map.
43. `test_google_books.py::test_volume_refresh_429_is_classified`: import a GB book, then a 429 on refresh → 429 with "busy" in the detail. **ADAPT**: an upstream 429 during refresh → 429 `The library is busy. Wait a few seconds and try again.`.
44. `test_google_books.py::test_goodreads_import_prefers_google`: the fixture CSV imports 2 rows. The shelf has keys `/works/ISBN9780316769488` and `/works/ISBN9780062060624`, no OL search, and the Circe cover is the Google URL. **ADAPT**: the Goodreads import of the same fixture must work against local + DNB + OL (#44 AC: "Goodreads import works with the same sample CSVs"). Expected counts depend on the faked responses.
45. `test_google_books.py::test_pick_does_not_persist_ol_cdn_cover_url` (no key): `PUT /api/pick` with OL1W, `cover_id` 123 and an OL CDN `cover_url` → 200. The `books` row has `cover_image_url` None and `cover_id` 123. **KEEP** (generalised): client-sent cover URLs are never persisted.
46. `test_google_books.py::test_shelf_rejects_non_google_cover_url` (no key): shelf add with `https://attacker.example/pixel.gif?u=1` → **201** (silently ignored). The row has `cover_image_url` None and `cover_id` 123. **KEEP** (generalised, security): a third-party tracking URL can never become a stored or served cover.
47. `test_google_books.py::test_shelf_isbn_x_and_X_are_the_same_row`: shelf add `ISBN080442957X` → 201. Adding `…x` → 409. The shelf has exactly one key, `/works/ISBN080442957X`. **KEEP**: adding the same edition under `x`/`X` spellings is a duplicate (409 `Already on your shelf`).
48. `test_google_books.py::test_google_403_access_not_configured_is_502`: 403 `accessNotConfigured` → 502 without "busy" in the detail. The next search reaches Google again (no cooldown). **ADAPT**: only real rate-limit signals open the breaker; config or auth errors are a 502.
49. `test_google_books.py::test_google_403_quota_cools_down`: 403 `rateLimitExceeded` → 429. The next, different search → 429 with **zero** Google calls (cooldown). **ADAPT**: when DNB SRU or OL signals a rate limit, the client backs off globally for a period and fails fast, or falls through to the next source without calling.
50. `test_google_books.py::test_goodreads_import_while_cooling_down`: trip the cooldown via a search 429, then the Goodreads import → 429 with "busy" in the detail and zero Google calls. **ADAPT**: decide whether the Goodreads import fails fast when every source is throttled, or skips rows with `Catalog unavailable`.
51. `test_google_books.py::test_later_empty_google_page_has_more_false_when_total_stays_high`: `totalItems=50`. Page 1 has `has_more=true`. An empty page 2 → `has_more=false` with no OL. **KEEP**: `has_more` is false whenever a page is empty, regardless of the upstream total.

Related tests elsewhere (other area specs own them):

- `test_book_details.py::test_isbn10_with_x_check_digit_is_accepted`: OL-mode ISBN-10 with X.
- `test_spine_color.py` line 154: `_fetchable` allows `books.google.com`. DROP that assertion.
- `test_spa_cache.py` parametrises `/book/ISBN9781408857892`. An old Google-keyed SPA URL must still work, as a redirect.
- `test_migrations.py` lists `cover_image_url` among the expected columns.
- Frontend: `constants.test.ts` (Google ids, `googleBooksUrl`, `remoteCoverUrl`, `isbnFromWorkKey`) and `BookCover.test.ts` (no OL ISBN CDN for Google keys, Google https cover). Both are DROP and REWRITE in #48.

---

## 5. Multi-club mapping

- **Google is removed entirely.**
  - No `GOOGLE_BOOKS_API_KEY`, no provider switch, no `googleapis.com` calls.
  - No `X-Goog-Api-Key`, Google UA, Google cover-host allowlist, `cover_image_url` column, or `GOOGLE_NO_REFRESH` error.
  - No "Google Books" outbound link (`googleBooksUrl`, `catalogUrl` fallback).
  - No `ISBN…`/`GB…` work ids minted by new code. #41 AC: no Google code or data in `server/`.
- **The catalog is global** (neither club-scoped nor personal). Works, editions and authors are shared by every user and club. Imported Google rows become ordinary global works and editions with bare facts. Caches, in-flight coalescing and upstream throttles or breakers are process-global per upstream (DNB SRU, OL), not per club or user.
- **Works versus editions.** A legacy `ISBN…` key conflated a work with one edition chosen by Google. In the new schema:
  - The ISBN becomes an **edition** (ISBN-13) attached to a work.
  - `pages` and the Google `year` are edition attributes.
  - A shelf entry points to the work. Whether to also set `edition_id` from the legacy ISBN is an open question: the user never chose that edition, Google did.
  - Club picks point to works.
  - Quote `page` numbers are edition-relative, and the legacy ISBN is the only edition hint.
- **Per-viewer annotations stay personal.** Search, ISBN and detail responses keep the viewer's own `on_shelf`/`shelf_id`.
- **Annotations that were instance-wide become club-scoped or audience-scoped:**
  - `club_pick` on search hits and details was "the instance's current pick". It now needs a club context (a `/c/<slug>` page) or a list of the viewer's clubs where the work is the current pick.
  - `readers`, `members`, `club_rating`, `rating_count` and `quote_count` on the detail endpoints listed **every user**. Under #46 they list only the audience (co-members of a shared club, later friends), honour per-item "only me" on ratings and quotes, and are probably computed per club on club pages.
- **Custom (`BC…`) books** are "club-added books" (#38) in a global catalog. Whether they are visible outside the club that added them is not decided here; see the `books`/`catalog` spec. Refresh stays rejected for user-added books.
- **Refresh** (`POST …/refresh`) was open to every member. In a hosted service, re-pulling CC0 data from a source is harmless but costs upstream quota. Decide whether it stays a user action (rate-limited) or becomes admin-only. User **edits** go to the admin queue (#41/#54).
- **Milestones** (legacy `chapter_from/chapter_to`) and progress (percent) do not depend on Google data. Page counts differing between editions is why picks are work-level. Stats parity (pages read, longest book) still reads `pages`, so it must be defined per edition, or as a work-level default edition.
- **No longer meaningful:**
  - The discovery cache shortcut (replaced by persisting on search).
  - The topic fallback for empty Google subject shelves.
  - The Google sort mapping (`new`→`newest`).
  - Skipping the short-query check in Google mode.
  - `ol_rating` holding Google ratings.
  - Client `cover_url` fields on shelf, pick, vote, quote and diary bodies (covers are server-owned media, #43).
  - The `ol_work_key` naming of non-OL ids (rename to a neutral `work_id` in the new API, #47).
- **Old URLs.** `/book/ISBN…` and `/book/GB…` SPA routes and `/api/books/works/{ISBN…|GB…}` callers must resolve through the importer's legacy-key map, case-normalising ISBN `x`→`X` and leaving GB ids case-sensitive (#48 "Old routes redirect").
- **Data protection.** Clients currently load covers straight from `books.google.com` (a third-party request per view). The new media store serves every cover from our own domain (#43), which removes that DSGVO issue.

---

## 6. Port risks

1. **The key ISBN is not always the data's ISBN.** Detail opens store the row under the **requested** id while the data comes from whatever volume Google's `isbn:` search returned first, possibly another edition or an ISBN-10/13 sibling. Goodreads rows are keyed by Google's ISBN, not the CSV's. When re-matching, trust the key's ISBN, but expect title and authors that belong to a sibling edition.
2. **No checksum validation anywhere** (`extract_isbn`, `normalize_isbn`, `_volume_isbn`). Legacy `ISBN…` keys can be checksum-invalid, non-978/979 13-digit numbers, or a Google `OTHER` identifier that happens to be 10 digits. Validate before any lookup and never mint an edition from an invalid ISBN.
3. **GB rows carry no ISBN.** The volume's ISBN, if it had one, was never stored. They cannot be re-matched by ISBN. Do not try to call Google to recover it (plan: Google is dropped).
4. **Provenance is per column, not per key.**
   - On ISBN/GB rows, non-empty `description`, non-`[]` `subjects` and any `ol_rating` are Google's.
   - `cover_id` is always OL (or client-sent) and never Google.
   - `cover_image_url` is always Google and can sit on `OL…` and `BC…` rows too.
   - `ISBN…` rows can be entirely OL-derived (created in OL mode) or entirely client-sent (shelf add).
5. **`cover_image_url` wins at render time.** Dropping it changes the visible cover for those books. The fallback is `cover_id` (OL), and in OL mode an OL ISBN CDN cover was shown for ISBN keys with no cover. Expect some imported books to lose their cover until the media store fetches an OL cover by ISBN-matched edition.
6. **`_set_cover` never clears `cover_image_url`.** OL refreshes could not remove a Google cover, so even OL-keyed rows refreshed from OL may still render a Google cover. Drop it regardless of key.
7. **The refresh asymmetry.** A Google refresh **hit** overwrites `description` with `""` and `subjects` with `"[]"` when the volume lacks them (not None), while a **miss** leaves the row untouched. The new refresh should treat "field absent upstream" as "keep" (tests 37 and 38 encode that intent).
8. **`ol_rating` mislabelled.** On Google rows it is Google's `averageRating` (2 dp). Importing it as an OL rating would be both wrong and Google data.
9. **Subject vocabularies differ.** Google categories (`"Fiction / Fantasy / Epic"`) pass `_clean_subjects` in `googlebooks.py`. OL subject cleaning rejects `:` and `,`, and `routers/books.py` also rejects machine tags. Do not carry Google categories into the subject/FTS index.
10. **`year` semantics differ.** Google gives the volume's `publishedDate` year; OL gives `first_publish_year`. A work-level "first published" taken from a Google row will often be a reprint year.
11. **`pages` feeds stats.** `stats.py` sums `book.pages` for pages read and picks the longest book. If the importer drops or moves `pages` to an edition the shelf entry does not reference, per-person stats change. Define which edition's pages count.
12. **Merging creates unique-constraint collisions.** These hit shelf (user, work), favourites (user, work and position), next-up nominations (vote, work) with their ballots, and the physical layout JSON referencing shelf entry ids. #38's "matching counts for every entity" AC cannot hold literally for books (or for merged shelf entries), so the dry run must report merges explicitly.
13. **Case rules.** `ISBN` canonicalisation also upper-cases a lowercase `isbn` prefix (`isbn978…` → `ISBN978…`). `GB` ids are case-sensitive (`GBabc` ≠ `GBABC`), and Postgres `=` is case-sensitive like SQLite's default, so do not case-fold GB keys in the legacy map.
14. **Arbitrary legacy keys.** Shelf add, pick, vote nominate and quote only check the `/works/` prefix, and `upsert_book` keeps the last path segment, so `books.ol_work_key` may hold ids matching none of the four formats. The importer needs an `OTHER` bucket (bare facts).
15. **Lowercase-`x` duplicates.** Canonical-on-write shipped together with the Google feature, so duplicates should not exist. Still canonicalise and merge, because a single collision would break a unique index in Postgres.
16. **Negative caching.** Treat both a missing `items` (OL: `docs`) key and an empty list as "do not cache". Porting only the empty-list case reintroduces the "lucky-empty 200 frozen for 18 h" bug.
17. **Retry scope.** The retry is exactly one, only for 502/503/504, after 300 ms, and shared by coalesced waiters. 500, 4xx, quota and transport errors are not retried. Copying a generic retry policy (retry on any error, or several times) would double the load on a throttling upstream such as DNB SRU.
18. **Breaker semantics.**
    - It trips only on 429, or on a 403 with quota reasons.
    - A 503 must not trip it, nor a config 403.
    - While open, calls fail fast with **no** network request, and cached results and local rows remain servable.
    - It is wall-clock and process-global.
    - Tests reset it between cases: any Rust global needs a reset hook for test isolation, mirroring `clear_google_cache()` in `conftest.py`.
19. **Error mapping quirk.** An upstream 404 on a search or browse call surfaced as 404 `No such book.`. Do not port this. Map a search-endpoint 404 to 502, or to an empty result.
20. **`has_more`** must be false when the mapped page is empty, even when the upstream total says otherwise. The topic fallback forced it false too.
21. **The short-query rule differed by mode.** OL mode returned 400 for queries under 3 chars that are not ISBNs and have no subject; Google mode allowed them. The local FTS/trigram search in #41 needs its own explicit minimum (the legacy local substring match needs ≥ 2 chars).
22. **An ISBN-shaped query ignores the subject filter**, and the literal `isbn:` prefix is not recognised as an ISBN by the query parser (Goodreads relied on Google interpreting it). A port that strips `isbn:` must do so deliberately.
23. **Local fallback on upstream failure.** When local hits exist, any upstream error is swallowed and the local hits are returned with `has_more=false`. This is provider-agnostic and easy to lose.
24. **Do not port "browsing does not import".** The Google tests assert that no row exists after browsing; the plan says everything searched is copied in. Porting those assertions literally would contradict #41.
25. **"Open a displayed result without upstream" must survive** without a discovery cache. Persist at search time (or at least on open, from data the client already has) so opening works during an outage, as tests 18, 19 and 23 require.
26. **The ISBN lookup response mixes the requested `isbn` with the derived key** and never writes a row. A new ISBN endpoint should return the edition and work it actually resolved, normalised to ISBN-13, and persist them (#41: "anything scanned is copied into the catalog").
27. **The frontend has its own copies of these rules** (`constants.ts` regexes, `remoteCoverUrl` host allowlist, `BookCover` skipping the OL ISBN CDN for Google keys, `googleBooksUrl`). They must be deleted together with the server changes. Otherwise the UI keeps hot-linking `books.google.com` for any lingering URL, or tries OL ISBN CDN URLs the new server never emits.
28. **`spine_color` was sampled from the Google image.** After the cover changes it no longer matches. Reset it to null for rows whose rendered cover changes. Any user-chosen per-entry spine appearance in `physical_shelf_layouts.layout_json` is personal data and must be kept.
29. **`ol_rating_count`** (Google `ratingsCount`) was returned once on first import and never persisted. Do not treat it as stored data in parity comparisons.
30. **Authors.** The `authors` string is `", "`-joined. Splitting it into author records on import is lossy for names that contain a comma (e.g. "Jr."). Prefer matching to the source's author records after an ISBN match and keep the raw string for unmatched rows.

---

## Open questions

1. Does the production SQLite DB actually contain Google rows? That depends on whether `GOOGLE_BOOKS_API_KEY` was ever set on the homelab instance. Count rows by key class (OL/BC/ISBN/GB/OTHER) and non-null `cover_image_url` on the production copy before finalising #38.
2. For ISBN rows that match DNB/OL, should the imported shelf entries get `edition_id` = the legacy ISBN's edition? Google chose that edition, not the user. What about quotes with `page` numbers on those books?
3. GB rows have no ISBN. Should the importer try a title + author match against DNB/OL (with a confidence threshold and a review list in the dry-run report), or always keep bare facts?
4. Should the importer call DNB SRU and OL during `--dry-run` (read-only network), or report only "would re-match N ISBNs" without lookups?
5. What is the merge rule when several legacy rows collapse into one work and the same user has shelf entries, favourites or nominations on several of them (status precedence, ratings, dates, favourite positions, ballots)?
6. Is the legacy `ol_work_key → work_id` map kept permanently (for `/book/ISBN…` and `/book/GB…` redirects and old links), or only during the cutover?
7. Does `POST …/refresh` survive in the new API (user action, rate-limited, or admin-only), and what is its error for user-added books now that `This book was added by the club, not Open Library.` no longer fits?
8. Do the OL/DNB clients adopt Google's "one retry on 502/503/504 after 300 ms" and the global quota breaker (120 s fail-fast), or does DNB SRU get its own throttle policy?
9. Should the Goodreads import fail fast with 429 when every catalog source is throttled (Google-mode behaviour) or skip rows with `Catalog unavailable`?
10. After a Google cover is dropped, should the importer reset `spine_color` (so it is resampled from the new cover) or keep the Google-derived colour as a harmless fact?
11. For unmatched Google rows, which `year` goes on the work versus the edition? Google's year is the volume's publication year.
