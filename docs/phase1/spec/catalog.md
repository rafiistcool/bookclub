# Port spec: catalog (search, details, work ids, Open Library, covers, local-first, caches, book serialization)

Sources read in full: `backend/app/routers/books.py`, `backend/app/works.py`, `backend/app/openlibrary.py`,
`backend/app/covers.py`, `backend/app/serialize.py`, `backend/app/shelf_ops.py` (`upsert_book`,
`apply_book_details`), `backend/app/branding.py` (`open_library_ua`), `backend/app/favorites.py`
(`favorite_position_for`), `backend/app/pick_ops.py` (`current_pick`), `Book` and related models in
`backend/app/models.py`, catalog schemas in `backend/app/schemas.py`, `COLUMN_MIGRATIONS` in `backend/app/db.py`,
the validation handler in `backend/app/main.py`, and the tests `test_search.py`, `test_book_details.py`,
`test_local_reads.py`. Frontend callers checked: `frontend/src/api/client.ts`, `constants.ts`,
`components/BookCover.vue`, `pages/DiscoverPage.vue`, `pages/BookDetailPage.vue`, `i18n/apiErrors.ts`.

Behaviour marked **(verified)** was confirmed by running the Python app with a faked `httpx` client,
not only by reading code. The probe scripts were throwaway session files and are not in the repo.

Google Books code paths (`googlebooks.py`, `fetch_google_catalog`, `_load_google_catalog`,
`_import_google_details`, `_refresh_google_catalog`, the Google branch of `/isbn`) are **not** ported. They are
described only where they shaped the data (ISBN…/GB… keys, `cover_image_url`) or produce a status the
non-Google path also reaches.

---

## 1. Endpoints

### 1.0 Conventions shared by every catalog endpoint

- **Prefix:** `/api/books`. Router tag `books`.
- **Auth:** every endpoint requires a signed-in user (`deps.get_current_user`, cookie session `user_id`).
  Missing or stale session: `401 {"detail": "Not signed in"}` (stale session also clears the cookie
  session). No role checks, no membership checks (single club).
- **Error envelope:** `{"detail": "<English string>"}`.
- **Validation errors** (query/path/body type, range, enum, missing field) are flattened by the app-wide
  handler into **400** with `detail` = each pydantic error `msg`, with a leading `"Value error, "` stripped,
  joined by `"; "`. Exact strings that catalog endpoints can produce **(verified)**:
  - `"Input should be greater than or equal to 1"` (page < 1, limit < 1, year < 1)
  - `"Input should be less than or equal to 40"` (limit > 40)
  - `"Input should be less than or equal to 3000"` (custom year > 3000)
  - `"Input should be a valid integer, unable to parse string as an integer"` (non-numeric page/limit)
  - `"Input should be 'readinglog', 'new', 'title' or 'relevance'"` (bad sort, including `sort=`)
  - `"Field required"` (custom book without `title`)
  - `"Input should be a valid string"` (non-string title/authors/description)
  - `"A title is required"` (custom title blank after strip; comes from a `ValueError`)
- **Unhandled:** a non-JSON body from Open Library raises `JSONDecodeError`, which nothing catches, so the
  endpoint answers **500 "Internal Server Error"** (plain text) **(verified)**. See risk R14.
- **Trailing slash:** FastAPI redirects `/api/books/works/OL1W/` to the slash-less path (307). The frontend
  never sends one.
- The frontend currently calls only: `/search`, `/trending`, `/subjects/{subject}`, `/works/{id}`,
  `/works/{id}/refresh`, `/custom`. `/work/{id}`, `/work/{id}/refresh` and `/isbn/{isbn}` are tested in the
  backend but have **no frontend caller**.
- The Discover page classifies errors **by status only**: 400 → "short query" UI, 429 → "rate limited" UI,
  502 → "unavailable" UI, anything else → generic. The status codes are therefore load-bearing; the detail
  strings are shown raw (none of the catalog strings is mapped in `apiErrors.ts`).

### 1.1 `GET /api/books/search` → `SearchPage`

Query params:

| name | type | default | rules |
|---|---|---|---|
| `q` | string | `""` | Whitespace-collapsed: `" ".join(q.split())` (Python `str.split()` = any Unicode whitespace). After collapsing, `len(query) > 200` code points → **400 "Search is too long"**. Exactly 200 is accepted **(verified)**. |
| `subject` | string | `""` | Normalized by `_normalize_subject` (§2.3). An invalid subject is **silently dropped** (becomes `""`), never an error here **(verified)**. |
| `sort` | enum `readinglog`\|`new`\|`title`\|`relevance` | none | Anything else → 400 (enum message above). |
| `page` | int | 1 | `>= 1`, **no upper bound**. |
| `limit` | int | 24 | `1..=40`. |

Algorithm (order matters):

1. Collapse `q`; length check (400 "Search is too long").
2. Normalize `subject`.
3. **Empty box:** if `query == ""` and `subject == ""` → return **trending** (`fetch_trending(limit)`,
   §1.2 behaviour, including its cache key `trending|{limit}` with the *search* limit, default 24),
   annotated. `sort` and `page` are ignored; the response has `page: 1, has_more: false` even if `page=2` was
   requested **(verified)**.
4. **Short query:** if `query` non-empty and `len(query) < 3` and `extract_isbn(query)` is None and
   `subject == ""` → **400 "Need at least 3 characters. Add the author, or paste an ISBN."** No outbound call.
   A short query **with** a subject is allowed (`q=it&subject=fantasy` sends `it subject_key:fantasy`)
   **(verified)**.
5. `resolved_sort = sort or ("relevance" if query else "readinglog")`.
6. **Local hits:** if `query` non-empty **and** `page == 1` → `_local_catalog_hits(query, limit)` (§2.6).
   Otherwise `[]`.
7. **Remote:** `fetch_open_library(query, subject, resolved_sort, page, limit)` (§2.4–2.5).
8. If the remote call raises any `HTTPException` (400/404/429/502) **and** local hits exist → return
   `SearchPage(items=local, page=page, has_more=false)`, annotated. With no local hits the error propagates.
9. Merge local + remote (§2.7), annotate (§2.8), return.

Response `SearchPage` (always all fields; see §1.10 for `SearchHit`):

```json
{ "items": [SearchHit...], "page": <int, echoes request page except trending branch>, "has_more": <bool> }
```

Status codes: 200; 400 (too long, too short, validation, or upstream 422 mapped to the short-query string);
401; 429 (upstream 429); 502 (upstream failure); 500 (upstream non-JSON). Upstream 404 on `search.json` is
a **502** with the search string (`test_upstream_failure_is_a_502`).

Error strings: `"Search is too long"`, `"Need at least 3 characters. Add the author, or paste an ISBN."`,
`"Could not search the library right now. Try again."`, `"The library is busy. Wait a few seconds and try again."`,
`"The library is busy. Try again in {seconds} seconds."`.

Side effects: outbound GETs to `https://openlibrary.org/search.json` (1 primary, plus up to 2 concurrent
fallbacks on a page-1 miss), or `trending/daily.json` for the empty box. Reads `books`, `shelf`,
`club_picks`. **No DB writes.** Fills the search cache (§2.17). No rate limit.

Ordering: local hits first (by `books.id` ascending), then remote hits in Open Library's order, deduped by
`ol_work_key`. Pagination is Open Library's page/limit; `has_more = page * limit < num_found` of the batch
actually used. Local hits only ever appear on page 1.

### 1.2 `GET /api/books/trending` → `SearchPage`

Query: `limit` int, default **12**, `1..=40`.

- `GET https://openlibrary.org/trending/daily.json?limit={limit}`; cache key `trending|{limit}`;
  empty results **are** cached.
- Items = `map_open_library_docs(payload["works"] or [])` (§2.9). `page: 1`, `has_more: false` always.
- Annotated (§2.8). No local hits.
- Errors: classified as in §2.16 with detail `"Could not load the library right now. Try again."`. Upstream
  404 → **502** (no `missing_detail`) **(verified)**; 429 → 429 with retry hint; 422 → 400 short-query string.
- No DB writes.

### 1.3 `GET /api/books/subjects/{subject}` → `SearchPage`

Path `subject`: normalized by `_normalize_subject`; empty result → **400 "That is not a valid subject."**
with no outbound call (`%21%21%21`, `sci-fi`, non-ASCII all fail) **(verified)**.
Query: `page` int default 1 (`>= 1`, no max); `limit` int default **12**, `1..=40`.

- `offset = (page - 1) * limit`.
- `GET https://openlibrary.org/subjects/{key}.json?limit={limit}&offset={offset}&details=false`;
  cache key `subject|{key}|{page}|{limit}`; empty results cached.
- Items = `map_subject_works(payload["works"] or [])` (§2.9). `page` echoes request.
  `has_more = offset + limit < work_count` where `work_count = int(payload.work_count or 0)`, non-numeric → 0.
- Errors: upstream 404 → **404 "No such subject."**; 429 → 429 hint; 422 → 400 short-query string; other →
  **502 "Could not load the library right now. Try again."** **(verified)**.
- Annotated. No DB writes.

### 1.4 `POST /api/books/custom` → `BookDetailOut`, **201**

Body `CustomBookIn` (JSON):

| field | type | default | rules |
|---|---|---|---|
| `title` | string | required | Stripped; empty → 400 "A title is required"; then **silently truncated** to 500 code points. Missing → 400 "Field required". |
| `authors` | string | `""` | Stripped, silently truncated to 300 code points. |
| `year` | int \| null | null | `1..=3000` (pydantic lax mode also accepts `"2024"` and `2024.0`). |
| `description` | string | `""` | Stripped, silently truncated to 4000 code points. |

- Generates `work_id = "BC" + secrets.token_hex(5)` (10 lowercase hex chars) and inserts
  `books(ol_work_key="/works/BC…", title, authors, year, description)` with every other column at its
  default (`cover_id` NULL, `subjects` `"[]"`, `pages`/`ol_rating`/`details_fetched_at`/`spine_color` NULL).
  No collision check (a collision would be a unique-constraint 500).
- Never contacts Open Library. Does not put the book on any shelf, does not log an event, records **no
  creator**.
- Response: `_book_detail_from_local` (§2.19) → `custom: true`, `readers: []`, `club_pick: false`,
  `favorite_position: null`, `cover_url: null`.

### 1.5 `GET /api/books/works/{work_id}` → `BookDetailOut` (the detail endpoint the UI uses)

Path `work_id` (no slashes). Steps:

1. `work_id = canonical_work_id(work_id)` (§2.1).
2. `is_work_id(work_id)` false → **404 "No such book."**, no outbound call (`not-a-key`, `OL1M`, `foo`).
3. **Local first:** a `books` row with `ol_work_key == "/works/{work_id}"` → `_book_detail_from_local`.
   **No outbound call, even if the row is a stub without description** (`test_book_detail_includes_own_shelf_state_and_other_readers`).
4. `BC…` with no row → 404 "No such book.".
5. `ISBN…`/`GB…` (Google catalog ids) with no row: without a Google key, `GB…` → 404 "No such book."
   **(verified)**; `ISBN…` → Open Library ISBN lookup (§2.10 C), 404 "No such book." on a miss, 502
   "Could not look that ISBN up right now. Try again." on failure; a hit is stored under the `ISBN…` key with
   OL title/authors/year/pages/cover_id and no description/subjects. Not ported (legacy only, see §5).
6. Otherwise (`OL…W`): **import** via pipeline A (§2.10 A) → `_book_detail_from_local`.

Status: 200; 401; 404 `"No such book."` (bad id, BC missing, OL 404 on the work JSON); 429 (OL 429 on the
work JSON, hint string); 400 short-query string (OL 422 on the work JSON, nonsensical but real
**(verified)**); 502 `"Could not load the library right now. Try again."`; 500 `"Could not save that book"`
(row vanished after commit).

Side effects: **a GET writes the DB** on first view (insert `books` row + details). Outbound: work JSON,
search doc, and maybe up to 3 author JSONs. Fills cache keys `work|{id}`, `work-search|{id}`,
`author|{id}`.

### 1.6 `POST /api/books/works/{work_id}/refresh` → `BookDetailOut`

1. Canonicalize.
2. `BC…` → **400 "This book was added by the club, not Open Library."** (checked before existence; also for
   ids that have no row).
3. `ISBN…`/`GB…` → Google refresh path. Without a Google key: `GB…` → **400 "This book was added from Google
   Books. Set GOOGLE_BOOKS_API_KEY to refresh."** **(verified)**; `ISBN…` → OL ISBN lookup with cache
   bypass, miss keeps the local row. Not ported.
4. Not `OL…W` (any malformed id) → **404 "No such book."**.
5. Pipeline A with `bypass_cache=True` (§2.10 A). Works even when no local row exists (it imports).
   An OL 404 → 404 "No such book." even though a local row exists (the local row is left untouched).

Body: none. Status/error strings as §1.5 plus the 400 above. Side effects: DB update of the `books` row
(overwrites description and subjects, see §2.15), cache refresh of `work|`, `work-search|`, `author|` keys.
No rate limit, no permission check: any signed-in user can refresh any book.

### 1.7 `GET /api/books/work/{work_id}` → `BookDetailsOut` (richer DTO; no frontend caller)

1. Canonicalize.
2. `BC…` → local row or **404 "No such book."**.
3. `ISBN…`/`GB…` → local row, else Google/ISBN import. Without a Google key `GB…` → 404 "No such book."
   **(verified)**. Not ported.
4. Not `OL…W` → **400 "That is not an Open Library work id"** (note: 400 here, 404 on `/works/`).
5. Local row → `_work_details_from_local` with `ol_rating_count: null`. **No outbound call.**
6. Else `fetch_work_details("/works/{id}")` (pipeline B, §2.10 B) → `_import_work_details` →
   `_work_details_from_local(..., ol_rating_count=details.ol_rating_count)`.

Errors from pipeline B: **every** transport or HTTP error (including 404 and 429) → **502 "Could not load
that book right now. Try again."** **(verified)**. 500 "Could not save that book".

### 1.8 `POST /api/books/work/{work_id}/refresh` → `BookDetailsOut`

1. Canonicalize. 2. `BC…` → 400 "This book was added by the club, not Open Library.". 3. Google ids → Google
refresh (not ported). 4. Not `OL…W` → 400 "That is not an Open Library work id". 5.
`fetch_work_details(bypass_cache=True)` → import (overwrite) → response with `ol_rating_count`.

### 1.9 `GET /api/books/isbn/{isbn}` → `IsbnHitOut` (no frontend caller)

- `normalize_isbn`: remove all whitespace and `-`, uppercase; must match `^(?:\d{9}[\dXx]|\d{13})$`, else
  **400 "That is not a valid ISBN-10 or ISBN-13"**. **No checksum validation**: `0000000000` is accepted
  (and is also checksum-valid). `080442957X` and `978-0-316-55634-7` are accepted.
- `lookup_isbn(clean)` (§2.10 C): miss → **404 "No book found for that ISBN"**; any transport/HTTP error
  (incl. 404/429) → **502 "Could not look that ISBN up right now. Try again."** **(verified)**.
- Response `isbn` = the normalized input (not from OL), OL work key/title/authors/cover_id/year/pages,
  `cover_url: null`. Annotated with the viewer's shelf entry for that work key (`on_shelf`, `shelf_id`).
- **No DB write**: an ISBN lookup does not import the book.

### 1.10 DTOs (JSON field names, types, nullability; every field is always present in responses)

`ShelfStatus` enum (string): `want_to_read`, `currently_reading`, `finished`, `did_not_finish`.

**SearchHit**

| field | type | notes |
|---|---|---|
| `ol_work_key` | string | `/works/{id}` |
| `title` | string | non-empty |
| `authors` | string | names joined with `", "`; may be `""` |
| `cover_id` | int \| null | positive OL cover id only |
| `year` | int \| null | OL `first_publish_year`, or local `books.year` |
| `on_shelf` | ShelfStatus \| null | viewer's shelf status |
| `shelf_id` | int \| null | viewer's shelf entry id |
| `club_pick` | bool | default false; true only for the current pick's key |
| `cover_edition_key` | string \| null | OL edition OLID (`OL…M`) from search/trending docs; null for subject hits and local hits |
| `isbn` | string \| null | remote: first valid ISBN in doc `isbn` (only present if OL returns it, see R9); local: ISBN from an `ISBN…` key |
| `custom` | bool | true for `BC…` keys (local hits only) |
| `cover_url` | string \| null | remote hits: always null; local hits: `book_cover_url(book)` |

**SearchPage**: `items: SearchHit[]`, `page: int`, `has_more: bool`.

**BookDetailOut** (`/works/…`, `/custom`)

| field | type | source |
|---|---|---|
| `id` | int | `books.id` |
| `ol_work_key` | string | |
| `title` | string | |
| `authors` | string | |
| `cover_id` | int \| null | |
| `cover_url` | string \| null | `book_cover_url` (§2.13) |
| `year` | int \| null | |
| `description` | string | `""` when unknown |
| `subjects` | string[] | parsed from JSON column; invalid JSON or non-list → `[]`; items coerced with `str()` |
| `on_shelf` | ShelfStatus \| null | viewer |
| `shelf_id` | int \| null | viewer |
| `club_pick` | bool | current pick's `ol_work_key == book.ol_work_key` |
| `rating` | int \| null | viewer's rating (1–5; only non-null when finished) |
| `take` | string | viewer's take (≤140) |
| `dnf_reason` | string | viewer's DNF reason (≤200) |
| `progress` | int \| null | viewer's progress percent 0–100 (only while currently reading) |
| `readers` | BookReader[] | every **other** user with a shelf entry on this book (any status), sorted by username ascending |
| `club_rating` | float \| null | mean of all finished entries with a rating (viewer included), Python `round(x, 1)` |
| `rating_count` | int | number of ratings in that mean; 0 when none |
| `custom` | bool | `BC…` key |
| `favorite_position` | int \| null | viewer's favourite slot 1–3 for this book |

**BookReader**: `username: string`, `status: ShelfStatus`, `rating: int|null`, `progress: int|null`,
`avatar_url: string|null` (`/api/members/{username}/avatar?v={unix_ts}` when the user has an avatar, else
null).

**BookDetailsOut** (`/work/…`)

| field | type | notes |
|---|---|---|
| `ol_work_key`, `title`, `authors` | string | |
| `cover_id` | int \| null | |
| `year` | int \| null | |
| `cover_url` | string \| null | |
| `description` | string | |
| `pages` | int \| null | `books.pages` (OL `number_of_pages_median`) |
| `subjects` | string[] | |
| `ol_rating` | float \| null | `books.ol_rating` |
| `ol_rating_count` | int \| null | **only on the response that imported/refreshed from OL**; never stored, so later local reads return null |
| `on_shelf`, `shelf_id` | as above | viewer |
| `club_pick` | bool | current pick's `book_id == book.id` |
| `members` | BookMember[] | **every** user with an entry, viewer included, viewer first, then username ascending |
| `quote_count` | int | count of all quotes on the book by anyone |

No `id`, `custom`, `favorite_position`, `readers`, `club_rating` here.

**BookMember**: `username: string`, `status: ShelfStatus`, `rating: int|null`, `take: string`,
`progress: int|null`, `finished_at: datetime|null` (SQLite returns naive UTC; Python serializes it
**without** an offset, e.g. `"2026-10-27T10:48:05.057186"`).

**IsbnHitOut**: `isbn: string`, `ol_work_key: string`, `title: string`, `authors: string`,
`cover_id: int|null`, `year: int|null`, `pages: int|null`, `cover_url: string|null` (always null on the OL
path), `on_shelf: ShelfStatus|null`, `shelf_id: int|null`.

**BookOut** (shared book serializer used by shelf, picks, favourites, overlap, stats, activity, vote,
diary): `id: int`, `ol_work_key: string`, `title: string`, `authors: string`, `cover_id: int|null`,
`year: int|null`, `cover_url: string|null` (`book_cover_url`), `pages: int|null`,
`spine_color: string|null` (`books.spine_color or None`, so `""` → null).

### 1.11 Every catalog `detail` string (verbatim)

| string | status | where |
|---|---|---|
| `Not signed in` | 401 | all |
| `Search is too long` | 400 | search |
| `Need at least 3 characters. Add the author, or paste an ISBN.` | 400 | search short query; any OL 422 via `books.py` fetcher (search, trending, subjects, `/works/` import) |
| `Could not search the library right now. Try again.` | 502 | search |
| `Could not load the library right now. Try again.` | 502 | trending, subjects, `/works/` import (work JSON); also the internal detail of swallowed author/search-doc failures |
| `The library is busy. Wait a few seconds and try again.` | 429 | OL 429 with no usable `Retry-After` |
| `The library is busy. Try again in {seconds} seconds.` | 429 | OL 429 with integer `Retry-After > 0` |
| `No such subject.` | 404 | subjects (OL 404) |
| `That is not a valid subject.` | 400 | subjects (bad key) |
| `No such book.` | 404 | `/works/` bad id, BC missing, OL 404; `/work/` BC missing; `/works/…/refresh` non-OL id |
| `This book was added by the club, not Open Library.` | 400 | both refresh endpoints for `BC…` |
| `This book was added from Google Books. Set GOOGLE_BOOKS_API_KEY to refresh.` | 400 | Google only; **drop** |
| `Could not save that book` | 500 | import commit vanished |
| `That is not an Open Library work id` | 400 | `/work/` and `/work/…/refresh` |
| `Could not load that book right now. Try again.` | 502 | `/work/` pipeline B, any failure; coalesced-waiter cancel |
| `That is not a valid ISBN-10 or ISBN-13` | 400 | `/isbn/` |
| `No book found for that ISBN` | 404 | `/isbn/` miss |
| `Could not look that ISBN up right now. Try again.` | 502 | `/isbn/` failure; `/works/ISBN…` lookup failure |
| `A title is required` | 400 | custom |

---

## 2. Domain rules

### 2.1 Work ids and keys (`works.py`)

A **work id** is the last path segment; a **work key** is `"/works/" + canonical id`. Stored in
`books.ol_work_key` (the name is historical; it holds every source's key).

| kind | regex (fullmatch) | origin |
|---|---|---|
| Open Library | `^OL\d+W$` | OL work |
| Club ("custom") | `^BC[a-f0-9]{10}$` | `POST /custom` |
| ISBN | `^ISBN(?:\d{9}[\dXx]|\d{13})$` | Google volume with ISBN (or OL ISBN fallback) |
| Google volume | `^GB[A-Za-z0-9_-]{1,40}$` | Google volume without ISBN |
| any | `^(?:OL\d+W|BC[a-f0-9]{10}|ISBN(?:\d{9}[\dXx]|\d{13})|GB[A-Za-z0-9_-]{1,40})$` | `is_work_id` |

- `canonical_work_id(id)`: if `len(id) > 4` and `id[:4].upper() == "ISBN"`, candidate =
  `"ISBN" + id[4:].upper()`; if the candidate matches the ISBN regex return it, else return `id` unchanged.
  So `isbn080442957x` → `ISBN080442957X` **(verified)**. Nothing else is case-normalized (`ol1w` is invalid).
- `is_work_id(id)` canonicalizes first.
- `work_key(id) = "/works/" + canonical_work_id(id)`; `work_id_from_key(key) = key.rsplit("/", 1)[-1]`
  (last segment, so `/works/a/OL5W` → `OL5W`).
- `isbn_from_work_id` returns the ISBN part only for canonical ISBN ids.
- `new_club_work_id() = "BC" + token_hex(5)`.
- Frontend mirrors these regexes in `constants.ts` (`workId`, `isClubWorkId`, `isbnFromWorkKey`, …) and
  builds detail routes as `/book/{workId}`.

### 2.2 ISBN handling

- `extract_isbn(raw)`: `re.sub(r"[\s-]", "", raw).upper()`, then fullmatch `^(?:\d{9}[\dXx]|\d{13})$`
  (after `.upper()` only `X` can remain) → normalized string or None. Used for search detection and
  remote-doc ISBNs.
- `normalize_isbn(raw)`: same, raising 400 instead of returning None.
- No checksum check, no ISBN-10 → ISBN-13 conversion, no 978/979 handling. ISBN-10 and ISBN-13 of the same
  book are different cache keys and different `ISBN…` work ids.

### 2.3 Query and subject normalization

- Search box: `query = " ".join(q.split())` (case kept; sent to OL as typed).
- Cache key normalization: `normalize_search_query(s) = " ".join(s.casefold().split())`.
  **casefold**, not lowercase: `"Straße"` → `"strasse"` **(verified)**.
- Subject: `key = "_".join(subject.lower().split())`; valid iff non-empty and fullmatch `^[a-z0-9_]+$`;
  otherwise `""`. `"Science  Fiction"` → `science_fiction`; `"sci-fi"`, `"Ünicode"`, `"!!!"` → `""`.

### 2.4 Search flow against Open Library (`fetch_open_library`)

1. `candidates = _search_candidates(query, subject)` (§2.5). `lang_en = (query == "")`.
2. Primary page: `_search_ol_page(candidates[0], sort, page, limit, lang_en)`.
3. If the **mapped** primary items are empty (also when OL returned docs that were all filtered out) **and**
   `page == 1` **and** fallbacks exist → run **all** fallbacks **concurrently** (page 1); take the first
   non-empty batch **in candidate order** (title before author) and its `num_found`. If none is non-empty,
   keep the empty primary result. If **any** fallback raises, the whole search fails with that error
   (Python `asyncio.gather` without `return_exceptions`), even if another fallback had hits.
4. `has_more = page * limit < num_found`, where `num_found = int(payload.num_found ?? payload.numFound ?? 0)`,
   non-numeric → 0.
5. A primary failure is not retried with fallbacks; it propagates.

`_search_ol_page` request: `GET https://openlibrary.org/search.json` with params, in this order:
`q`, `page` (always sent, also 1), `limit`, `fields=key,title,author_name,cover_i,first_publish_year,cover_edition_key`,
then `lang=en` only when `lang_en`, then `sort={sort}` only when `sort != "relevance"`. `isbn` is deliberately
**not** in `fields` (OL returns every edition ISBN, ~10× payload). Cache key:
`ol|search|{normalize_search_query(q)}|{sort}|{page}|{limit}` with `cache_empty=False` (§2.17).

### 2.5 Candidate OL queries (`_search_candidates`)

- `isbn = extract_isbn(query)` → `[f"isbn:{isbn}"]` only (subject ignored, no fallbacks).
- `query == ""` → `[_open_library_q("", subject)]` = `["subject_key:{subject}"]`.
- `primary = _open_library_q(query, subject)` = `"{query} subject_key:{subject}"` if subject else `query`.
- If `query` fullmatches `^(.+?)\s+by\s+(.+)$` (case-insensitive, **lazy** title, so it splits at the
  **first** ` by `): `title, author` stripped; `fielded = f'title:"{title}" author:"{author}"'` (+
  `" subject_key:{subject}"`); return `[primary, fielded]`. No escaping of `"` inside the values.
  `"Stand by Me by Ben E. King"` → `['Stand by Me by Ben E. King', 'title:"Stand" author:"Me by Ben E. King"']`
  **(verified)**. The raw query must stay first (regression: fielded-first turned "Stand by Me" into junk).
- Else `[primary, f"title:{query}", f"author:{query}"]` (unquoted; each + `" subject_key:{subject}"` when a
  subject is set).

### 2.6 Local catalog search (`_local_catalog_hits`)

- Runs only for a non-empty `query` on page 1.
- `needle = query.casefold()`; `len(needle) < 2` → no local hits. (A 2-char query without subject is
  already rejected by the short-query 400, so local search only sees 2-char needles together with a subject.)
- Scans **all** `books` rows ordered by `id` ascending (in Python, in memory). Match iff
  `needle in f"{title} {authors}".casefold()` (substring over title + space + authors, so a needle can
  straddle title and authors).
- Stops at `limit` hits.
- Hit fields: `ol_work_key`, `title`, `authors`, `cover_id` (positive only), `year`,
  `isbn = isbn_from_work_key(key)`, `custom = is_club_work_key(key)`, `cover_url = book_cover_url(book)`,
  `cover_edition_key = null`, `on_shelf/shelf_id` filled later by annotation.
- Covers every book anyone ever added (shelved, picked, nominated, quoted, diaried, imported, custom), not
  only custom books.
- An ISBN query (e.g. `978-0-316-…`) is used verbatim as the needle, so it practically never matches
  locally.

### 2.7 Merge, dedupe, ranking (`_merge_local_hits`)

- No local hits → remote page unchanged.
- Otherwise: `merged = local` (dedupe set = local keys); for each remote hit whose key is not in the set:
  append, add key, **then** `if len(merged) >= limit: break`. Consequence: when local already has `limit`
  hits, **one** remote hit is still appended, so the page has `limit + 1` items **(verified:
  limit=2 → 3 items)**. See R5.
- On a key collision the **local** version wins (it has `cover_url`, `custom`, `isbn`; it loses the remote
  `cover_edition_key`).
- `has_more` is the remote value, unchanged. Remote hits displaced from page 1 by local hits are never
  shown on page 2 (page 2 starts at OL offset `limit`). See R6.
- Remote mapping also dedupes by key within one payload (`seen` set; first occurrence wins).
- There is **no other ranking**: local by insertion id, remote by OL (relevance or the chosen sort).

### 2.8 Annotation (`_annotate`) — applied to every `SearchPage` returned by search/trending/subjects, including the local-only outage fallback

- `_annotate_shelf`: keys of all hits → `books` rows with those `ol_work_key`s → the **viewer's** `shelf`
  rows for those book ids → `hit.on_shelf = entry.status`, `hit.shelf_id = entry.id`.
- `_annotate_club_pick`: current pick = latest `club_picks` row with `ended_at IS NULL` ordered by `id`
  desc (with its book). If none, nothing is set (field stays `false`). Otherwise
  `hit.club_pick = (hit.ol_work_key == pick.book.ol_work_key)` for every hit.
- Annotation is per-viewer and is **never cached**; caches hold raw OL payloads.

### 2.9 Mapping Open Library payloads

`map_open_library_docs(docs)` — `search.json` `docs` and `trending/*.json` `works`:

- Skip non-dicts; `key = str(doc.key or "")` must start with `/works/` (editions `/books/…`, authors
  skipped); skip a key already seen; `title = str(doc.title or "").strip()` must be non-empty.
- `authors = ", ".join(str(name) for name in (doc.author_name or []) if name)`. If `author_name` is a
  **string** Python iterates its characters (`"Ann"` → `"A, n, n"`) **(verified)**; see R13.
- `cover_id = positive_cover_id(doc.cover_i)`; `year = int(doc.first_publish_year)` or None on failure;
  `cover_edition_key = str(doc.cover_edition_key or "").strip() or None`; `isbn` = `extract_isbn` of
  `doc.isbn` when it is a string, else the first list item that yields a valid ISBN, else None.
- `on_shelf = None` initially.

`map_subject_works(works)` — `subjects/*.json` `works` (different shape):

- Same key/title/dedupe rules; `authors = ", ".join(entry.name for entry in works.authors if dict and name)`;
  `cover_id = positive_cover_id(work.cover_id)`; `year` from `first_publish_year`. No
  `cover_edition_key`, no `isbn` (even if present in the payload).

Number leniency (both mappers and the import pipelines): `int(value)` in Python accepts ints, floats
(truncates toward zero: `1999.7` → 1999, cover `12.9` → 12), numeric strings (`"1999"`), and bools; fails
(→ None) on `"4.5"`, `None`, objects **(verified)**. Ratings: `round(float(value), 2)`; `float` accepts
numeric strings.

### 2.10 Importing a work (three code paths)

**A. `books.py::_import_work_payload` (used by `GET/POST /works/{id}[/refresh]`)**

Concurrently:
- `GET https://openlibrary.org/works/{id}.json` via the cached fetcher, cache key `work|{id}`,
  detail `BROWSE_UNAVAILABLE`, `missing_detail="No such book."` → failures propagate (404 → 404,
  429 → 429, 422 → 400, other → 502).
- `_work_search_doc`: `GET search.json` with `q=key:/works/{id}`,
  `fields=key,title,author_name,cover_i,first_publish_year,number_of_pages_median,ratings_average,ratings_count,subject`,
  `limit=1`, cache key `work-search|{id}`; **any** failure → `{}` (best effort); `doc = docs[0]` if it is a
  dict, else `{}`.

Then:
- `title = str(work.title or doc.title or "").strip()`.
- `search_authors = ", ".join(non-empty doc.author_name)`.
- Authors: on a normal view `search_authors or await _author_names(work)`; on refresh
  (`bypass_cache=True`) `await _author_names(work, bypass) or search_authors` (author API wins on refresh;
  `test_refresh_bypasses_author_cache`).
- `_author_names`: ids from `work.authors[]`: each entry's `entry.author.key` (dict) or `entry.key`; must be
  a string; `id = key.rsplit("/",1)[-1]` must fullmatch `^OL\d+A$`; dedupe; stop after 3. Fetch
  `https://openlibrary.org/authors/{id}.json` concurrently (cache key `author|{id}`, detail
  BROWSE_UNAVAILABLE); a failure yields `""`; name = `author.name.strip()` if a string. Join non-empty names
  with `", "`.
- `cover_id = first_positive_cover(work.covers, doc.cover_i)` (**work covers first**).
- `year = int(doc.first_publish_year)`, `pages = int(doc.number_of_pages_median)`,
  `ol_rating = round(float(doc.ratings_average), 2)`. `ratings_count` is fetched but **discarded**.
- `subjects = _work_subjects(work)` or, if empty, `_work_subjects({"subjects": doc.subject or []})`.
- `description = _work_description(work)` (§2.11 A).
- Persist: if no row, `upsert_book(key, title=title or work_id, authors, cover_id, year)` (a work with no
  title anywhere is stored with title `OL777W` **(verified)**); then `apply_book_details(title, authors,
  cover_id, year, description, pages, subjects, ol_rating)`; commit; reload by key (500 "Could not save that
  book" if gone).

**B. `openlibrary.py::fetch_work_details` / `_load_work_details` (used by `GET/POST /work/{id}[/refresh]`)**

- One `httpx.AsyncClient`; concurrently `GET https://openlibrary.org/works/{id}.json` and `GET search.json`
  with the same `q=key:…`/fields/`limit=1` as A. Both use `raise_for_status`; **any** `httpx.HTTPError` in
  either → 502 "Could not load that book right now. Try again." (no 404/429 distinction, search failure is
  fatal). Non-dict JSON → `{}`.
- `doc = (search.docs or [{}])[0]` when `docs` is a list, else `{}`.
- `title = str(work.title or doc.title or "").strip()`; `authors` = doc `author_name` joined (**no author
  API**); `cover_id = first_positive_cover(doc.cover_i, work.covers)` (**search cover first**);
  `year`, `pages` via `int`; `description = openlibrary._clean_description(work.description)` (§2.11 B);
  `subjects = _clean_subjects(work.subjects) or _clean_subjects(doc.subject)` (§2.12 B);
  `ol_rating = round(float(ratings_average), 2)`; `ol_rating_count = int(ratings_count)`.
- Result cached in a separate details cache (§2.17). `_import_work_details` persists like A
  (`title or work_id`).

**C. `openlibrary.py::lookup_isbn` (used by `/isbn/` and legacy `ISBN…` ids)**

- Cache hit by normalized ISBN unless bypassed.
- `GET search.json` with `q=isbn:{isbn}`,
  `fields=key,title,author_name,cover_i,first_publish_year,number_of_pages_median`, `limit=1`.
- Any `httpx.HTTPError` → 502 "Could not look that ISBN up right now. Try again.".
- First doc with `/works/` key and non-empty stripped title → `IsbnHit(isbn, ol_work_key, title,
  authors, cover_id=positive_cover_id(cover_i), year, pages)`; cached. No doc → None (**misses are not
  cached**).

### 2.11 Description cleaning (two variants; tests pin both)

**A. `books.py::_clean_description`** (after `_work_description`: dict → `.value`; non-string → `""`):
in this order:
1. Remove reference definitions: `(?m)^[ \t]*\[[^\]]+\]:[ \t]*\S+.*$` → `""` (the line's newline stays).
2. Inline links `\[([^\]]+)\]\([^)]*\)` → `\1` (empty URL allowed).
3. Reference links `\[([^\]]+)\]\[[^\]]*\]` → `\1`.
4. Blockquote markers `(?m)^[ \t]*>[ \t]?` → `""`.
5. `\n{3,}` → `"\n\n"`.
6. `.strip()` (Unicode whitespace).
Does **not** remove a `----` source trailer.

**B. `openlibrary.py::_clean_description`**: dict → `.get("value", "")`; `text = str(value or "").strip()`
(a list becomes its `repr`); remove `\n+-{3,}[\s\S]*$` (from the first newline-run followed by 3+ dashes to
the end); inline links `\[([^\]]+)\]\([^)]+\)` → `\1` (non-empty URL); `.strip()`. Does **not** handle
blockquotes, reference links or reference definitions.

Fixtures: A turns
`"[Comment by Kim Stanley Robinson][1] and [a note](http://example.com):\n\n> One of my favorite novels.\n> Truly.\n\n\n\nThe end.\n\n[1]: http://example.com/robinson\n"`
into `"Comment by Kim Stanley Robinson and a note:\n\nOne of my favorite novels.\nTruly.\n\nThe end."`;
B turns `"A witch on an island.\n\n----------\n[Source](https://x)"` into `"A witch on an island."`.
A union of both rule sets satisfies both fixtures (see §5.7).

### 2.12 Subject cleaning (two variants)

**A. `_work_subjects(payload, limit=12)`**: iterate `payload.subjects`; skip non-strings; `label = strip`;
skip empty, skip if `label.lower()` already seen, skip machine tags `^[a-z_]+:\S` (case-sensitive: only
lowercase/underscore before the colon, and a non-space right after it; `award:hugo_award=1970` dropped,
`Fiction: x` kept); keep first 12. Long and comma-containing subjects are **kept**.

**B. `_clean_subjects(values)`**: non-list → `[]`; `name = str(item or "").strip()`; skip empty, length
> 40, containing `:` or `,`; dedupe by `lower()`; keep first 12 (`MAX_SUBJECTS`).

Fixtures: A: `["Mythology","mythology","Greek literature","","award:hugo_award=1970"]` →
`["Mythology","Greek literature"]`. B: `["Greek mythology","Fiction","Witches","Fiction, fantasy, general","x"*50]`
→ `["Greek mythology","Fiction","Witches"]`.

Stored as JSON text (`json.dumps(list, ensure_ascii=False)`).

### 2.13 Covers

- `positive_cover_id(v)`: `int(v)` (lenient), only `> 0` is usable; `-1`, `0`, None, junk → None.
- `first_positive_cover(*values)`: walk values in order; a list is walked item by item; first positive
  wins. `first_positive_cover(-1, [0, -1, 99], 12) == 99`.
- `cover_url(cover_id, ol_work_key, cover_image_url)` / `book_cover_url(book)`, in order:
  1. `storable_cover_image_url(cover_image_url)`: normalize (strip; `http://` → `https://`; must be https
     with a host, ≤512 chars, no userinfo) and keep only Google hosts (`books.google.com`,
     `*.books.google.com`, `googleusercontent.com`, `*.googleusercontent.com`); OL CDN hosts are rejected.
  2. `cover_id > 0` → `https://covers.openlibrary.org/b/id/{cover_id}-L.jpg?default=false`.
  3. Google key configured → None.
  4. ISBN work key → `https://covers.openlibrary.org/b/isbn/{isbn}-L.jpg?default=false`.
  5. None.
- Remote search hits carry `cover_url: null`; the **frontend builds OL CDN URLs itself** from `cover_id`
  (`/b/id/{id}-{S|M|L}.jpg`), then `cover_edition_key` (`/b/olid/{OLID}-…`), then ISBN
  (`/b/isbn/{isbn}-…`), with size tiers xs/sm → S, md/lg/tile → M, fluid → L, `?default=false`, and falls
  back on error or a ≤1px image. Clients therefore talk to `covers.openlibrary.org` directly today.
- Spine colours (`spine_color.py`, physical-shelf area) fetch the same cover server-side at `-M.jpg`.
- `_set_cover`: a non-None new cover id or image URL replaces the old; if `(cover_id, cover_image_url)`
  changed, `spine_color` is reset to NULL so it is sampled again.

### 2.14 Python-specific rounding

- `club_rating = round(sum / n, 1)` and `ol_rating = round(float(x), 2)` use Python's `round`, which rounds
  the exact binary value and breaks exact ties **half-to-even**: `round(1.25,1) = 1.2`,
  `round(2.25,1) = 2.2`, `round(1.75,1) = 1.8` **(verified)**. Means of 1–5 star ratings hit exact ties
  whenever n is a multiple of 4 (e.g. ratings [1,1,1,2] → 1.25 → **1.2**). See R10.

### 2.15 Catalog writes (`upsert_book`, `apply_book_details`)

`upsert_book(key, title, authors, cover_id, year, cover_image_url=None)` is the write path used by this
area **and** by shelf add, Goodreads import, pick set, vote nomination, quote create and diary create, all
with **client-supplied** metadata:
- `key = work_key(work_id_from_key(key))` (canonicalizes ISBN case; takes the last segment).
- `cover_id ≤ 0` → None; `cover_image_url` filtered by `storable_cover_image_url`.
- New row: insert with those values (flush).
- Existing row: **`title` is overwritten unconditionally**; `authors` only if non-empty; cover via
  `_set_cover` (only non-None values); `year` only if not None. Description/pages/subjects untouched.
  → Any user can rename a book for everyone by shelving it with another title. See R1.
- Callers validate the key only with `startswith("/works/")`, so rows like `/works/foo` exist and
  `GET /works/foo` then returns 404 **(verified)**.

`apply_book_details(book, title="", authors="", cover_id=None, year=None, description=None, pages=None,
subjects=None, ol_rating=None, cover_image_url=None)`:
- `title`/`authors` only if non-empty; cover via `_set_cover(positive id, storable url)`; `year`, `pages`,
  `ol_rating` only if not None; **`description` whenever not None, including `""`**; **`subjects` whenever
  not None, including `[]`** (both import pipelines always pass a string and a list, so a refresh wipes a
  description or subjects that OL no longer has); `details_fetched_at = now()` always.

### 2.16 Open Library HTTP client

- Base URLs: `https://openlibrary.org/search.json`, `/trending/daily.json`, `/subjects/{s}.json`,
  `/works/{id}.json`, `/authors/{id}.json`.
- Timeouts (`OL_TIMEOUT`): connect **8.0 s**, read **15.0 s**, write 5.0 s, pool 5.0 s. A new
  `httpx.AsyncClient` per request (no connection reuse).
- User-Agent: `"{token}/1.0 ({contact}; catalog browse via Open Library)"`; `token` = `BOOKCLUB_NAME`
  sanitized and stripped to `[A-Za-z0-9._-]` (default `Bookclub`); `contact` = origin of
  `BOOKCLUB_PUBLIC_URL` or `"private book club"`. Test default: `Bookclub/1.0 (private book club; catalog
  browse via Open Library)` **(verified)**. Tests assert `"Bookclub/1.0"` is in the header.
- Status classification in `books.py` (`_classify_open_library_status`), applied to every status ≥ 400:
  - 422 → 400 `QUERY_TOO_SHORT`;
  - 429 → 429; `Retry-After` parsed with `int()`; if `> 0` → `"The library is busy. Try again in {n} seconds."`,
    otherwise (missing, `"abc"`, `"0"`, HTTP-date) → `RATE_LIMITED` **(verified)**;
  - 404 with a `missing_detail` → 404 with that detail;
  - anything else → 502 with the call's `detail`.
  Transport errors (`httpx.HTTPError`: timeouts, connect errors) → 502 with the call's detail. Non-object
  JSON → 502. Non-JSON → unhandled 500.
- Logging: warning with url, status, latency_ms, error class on failure; info with url and latency on
  success.
- **No client-side rate limiting** and no retry.

### 2.17 Caches and request coalescing

| cache | module | key(s) | TTL | max keys | what is cached | not cached |
|---|---|---|---|---|---|---|
| `_cache` | `books.py` | `ol|search|{norm q}|{sort}|{page}|{limit}`, `trending|{limit}`, `subject|{key}|{page}|{limit}`, `work|{id}`, `work-search|{id}`, `author|{id}` | 18 h | 256 | raw OL JSON object | errors; **search pages whose `docs` is an empty list** (`cache_empty=False`); a payload without a `docs` key or whose docs are all filtered out **is** cached |
| `_details_cache` | `openlibrary.py` | `/works/{id}` | 18 h | 64 | `WorkDetails` | errors |
| `_isbn_cache` | `openlibrary.py` | normalized ISBN | 18 h | 64 | `IsbnHit` | misses, errors |

- All three are LRU (`OrderedDict`): a hit moves the key to the end; `_cache_set` first purges **every**
  expired entry, inserts/moves to end, then evicts from the front while over the cap. A get of an expired
  key deletes it and misses. Process-local (lost on restart; not shared across workers).
- `bypass_cache=True` (refresh paths) skips the read but still writes the fresh value.
- **In-flight coalescing** (`_inflight` for `_cache`, `_details_inflight` for details; not for ISBN): the
  first caller registers a future under the cache key; identical concurrent calls `await shield(future)`
  and receive the same payload or the same exception. A bypassing call also registers (overwriting any
  existing entry). In `finally`: if the owner was cancelled, the future is failed with a 502
  (`detail` of that call, or "Could not load that book right now. Try again." for details) so waiters
  never hang, and the map entry is removed only if it is still this future. Errors are not cached.
- `clear_search_cache()` clears `_cache` and `_inflight`; `clear_details_cache()` clears details, details
  in-flight and ISBN caches. The test fixture calls both before each test.
- The cache key for search pages uses the full OL `q` (including `subject_key:` and fielded prefixes),
  so `circe`, `Circe` and `  CIRCE  ` share one entry.

### 2.18 Local-first rule

- Any book already in `books` is served from the DB by `/works/{id}`, `/work/{id}` and `BookOut`
  consumers with **no outbound call**, even when it is a stub (no description, no pages).
- Only an explicit refresh re-fetches. Clearing every RAM cache does not cause refetches.
- `test_local_reads` asserts that shelf, pick, pick history, overlap, members, vote, vote suggestions,
  diary, activity, stats, quotes, book detail (both DTOs) and book diary reads never construct an HTTP
  client once the book is known.

### 2.19 Detail computations

`_book_detail_from_local(book, viewer)`:
- Base fields from the row; `subjects` parsed; `club_pick = current_pick_key == book.ol_work_key`;
  `custom = is BC key`; `favorite_position` = viewer's `user_favorites.position` for this book.
- `_annotate_book_detail`: load all `shelf` rows for the book (with users).
  - `ratings = [rating for entries if status == finished and rating is not None]`; if any:
    `club_rating = round(mean, 1)`, `rating_count = len`. Viewer included.
  - Viewer's entry → `on_shelf`, `shelf_id`, `rating`, `take`, `dnf_reason`, `progress`.
  - Every other entry → `BookReader` (all statuses), sorted by `username` (code-point order; usernames are
    `[a-z0-9_]`).

`_work_details_from_local(book, viewer, ol_rating_count=None)`:
- `members` = every entry (viewer included) as `BookMember`, sorted by `(username != viewer.username,
  username)` → viewer first, then alphabetical. (The SQL `ORDER BY status, id` is irrelevant after the sort.)
- `on_shelf`/`shelf_id` from the viewer's entry; `quote_count` = number of `quotes` rows for the book (all
  users); `club_pick = current pick's book_id == book.id`; `pages`, `ol_rating` from the row.

Shelf-field invariants these rely on (shelf area): `rating` is non-null only for `finished`; `take` only
for `finished`; `dnf_reason` only for `did_not_finish`; `progress` only for `currently_reading`.

---

## 3. Data

### 3.1 `books` (read and written by this area)

| column | type (SQLite) | semantics / quirks |
|---|---|---|
| `id` | INTEGER PK | exposed as `BookOut.id` / `BookDetailOut.id`; favourites reference it from the client (`book_ids`) |
| `ol_work_key` | VARCHAR(64) UNIQUE, indexed | `/works/{id}`; ids `OL…W`, `BC…`, `ISBN…` (uppercase `X`), `GB…`, **plus arbitrary legacy strings** because writers only check the `/works/` prefix (e.g. `/works/foo`) |
| `title` | VARCHAR NOT NULL | no length limit except custom (500); stub rows carry whatever the client sent last; a title-less OL import stores the work id |
| `authors` | VARCHAR NOT NULL DEFAULT `""` | names joined with `", "`; not structured; OL names may themselves contain commas (`"Tolkien, J. R. R."`), so splitting is ambiguous |
| `cover_id` | INTEGER NULL | positive OL cover id; NULL = none known |
| `cover_image_url` | VARCHAR(512) NULL | Google Books image URL only (Google hosts); preferred over `cover_id` for display. **Drop** (Google terms) |
| `spine_color` | VARCHAR(7) NULL | `#rrggbb`; NULL = not sampled yet; `""` = sampled, no usable cover; reset to NULL when the cover changes |
| `year` | INTEGER NULL | OL `first_publish_year` (work level), Google `publishedDate` year, custom year (1–3000), or an unvalidated client int |
| `description` | VARCHAR NOT NULL DEFAULT `""` | cleaned plain text; for `ISBN…`/`GB…` rows it is usually Google's description (must not be carried over) |
| `pages` | INTEGER NULL | OL `number_of_pages_median` (work-level median across editions) or Google `pageCount` |
| `subjects` | VARCHAR NOT NULL DEFAULT `'[]'` | JSON array of ≤12 strings; Google `categories` for Google rows |
| `ol_rating` | FLOAT NULL | OL `ratings_average` rounded to 2 dp; **Google `averageRating` for Google rows** despite the name |
| `details_fetched_at` | DATETIME NULL | set by every `apply_book_details`; NULL = stub (only search/shelf metadata). **Write-only**: nothing reads it (also set by `seed_demo`) |

`ol_rating_count` is **not** stored anywhere.

`COLUMN_MIGRATIONS["books"]` (added after first ship, idempotent `ALTER TABLE ADD COLUMN`):
`description VARCHAR NOT NULL DEFAULT ''`, `pages INTEGER`, `subjects VARCHAR NOT NULL DEFAULT '[]'`,
`ol_rating FLOAT`, `details_fetched_at DATETIME`, `cover_image_url VARCHAR`, `spine_color VARCHAR`. Old rows
therefore have `''`/`'[]'`/NULL in these columns. No data migration rewrites `books`;
`import_legacy_pick_posts` only reads `club_picks.book_id`.

### 3.2 Tables read (not written) by this area

| table | columns used | why |
|---|---|---|
| `shelf` | `id`, `user_id`, `book_id`, `status`, `rating`, `take`, `dnf_reason`, `progress`, `finished_at` | annotations, readers, members, club rating (unique `(user_id, book_id)`) |
| `users` | `id`, `username`, `avatar_updated_at` | readers/members names, avatar URL |
| `club_picks` | `id`, `book_id`, `ended_at` | current pick = `ended_at IS NULL` with highest `id` |
| `quotes` | `book_id` | `quote_count` |
| `user_favorites` | `user_id`, `book_id`, `position` (1–3) | `favorite_position` |

Other tables reference `books.id`: `user_favorites` (ON DELETE CASCADE), `club_picks`, `book_posts`,
`events`, `quotes`, `next_up_nominations`. Books are never deleted.

### 3.3 Legacy facts the importer (#38) must handle

- Key kinds: `OL…W` (keep), `BC…` (user/club-added; **no creator recorded**), `ISBN…` and `GB…` (Google;
  re-match by ISBN; `GB…` rows have no ISBN at all, so they can only be matched by title/author or kept as
  bare facts), and non-conforming keys from the loose `/works/` prefix check.
- For `ISBN…`/`GB…` rows, `description`, `subjects`, `ol_rating`, `cover_image_url` and possibly `pages`
  and `year` came from Google; when the instance ran without a Google key, `ISBN…` rows may instead carry OL
  data from the ISBN fallback. The DB does not record which; treat all of them as Google-derived.
- `spine_color` is cover-dependent; with new covers it may need resampling.

---

## 4. Tests (parity checklist; 45 tests)

### `backend/tests/test_search.py` (35)

Fixture `ol`: registers `ada`, resets a fake `httpx.AsyncClient` that records `(url, params)` and asserts
`"Bookclub/1.0"` in the User-Agent. Fake search returns 3 docs (`OL1W` Circe; `/books/OL9M` edition;
duplicate `OL1W`) with `numFound: 50`, trending 2 works (`OL7W`, `OL8W` with `cover_i: -1`), subjects
`work_count: 40` with one work plus an author entry.

- `test_search.py::test_empty_search_serves_trending`: `q="   "` and no `q` both return 200 trending; body keys exactly `{items,page,has_more}`, `page 1`, `has_more false`, titles `["Atomic Habits","No cover"]`, `cover_i -1` → `cover_id null`; no `search.json` call, one trending call.
- `test_search.py::test_open_library_never_receives_a_too_short_query`: across empty, blank, subject-only, `circe`, trending and subject requests, every `search.json` `q` is not `*` and has ≥3 characters.
- `test_search.py::test_search_maps_and_caches`: `q=circe` → one hit `OL1W` "Circe" (edition and duplicate dropped), `has_more true`, `on_shelf null`; sent `q="circe"`, no `sort`, `fields` contains `cover_edition_key` and not `isbn`; repeated, `Circe` and `  CIRCE  ` hit the cache (1 call total); after shelving `on_shelf == currently_reading` and `club_pick false`; after setting the pick `club_pick true`.
- `test_search.py::test_search_forwards_page_subject_sort`: `q=circe&subject=fantasy&sort=new&page=2&limit=12` sends `q="circe subject_key:fantasy"`, no `subject` param, `sort=new`, `page=2`, `limit=12`, no `lang`; `subject=science_fiction` alone sends `q="subject_key:science_fiction"`, `lang=en`, `sort=readinglog`.
- `test_search.py::test_search_cache_keyed_by_page`: page 1 twice → 1 call; page 2 → second call with `page=2`.
- `test_search.py::test_trending_annotates_and_caches`: shelved `OL7W` shows `on_shelf want_to_read` in trending; `limit=5` forwarded; a repeat is served from cache (1 trending call).
- `test_search.py::test_subject_maps_its_own_payload_shape`: `/subjects/Science Fiction?limit=10` maps `authors[].name`/`cover_id`/year (1 item, author entry skipped), URL ends `/subjects/science_fiction.json`, `has_more true`; `fantasy?page=4&limit=10` → `has_more false`, `offset=30`.
- `test_search.py::test_subject_rejects_invalid_key`: `/subjects/%21%21%21` → 400, no subject call.
- `test_search.py::test_first_import_skips_author_api_when_search_has_names`: `GET /works/OL1W` takes authors from the search doc and never calls `/authors/`.
- `test_search.py::test_book_detail_resolves_description_subjects_and_authors`: first view imports: description plain string, subjects deduped case-insensitively with blanks and machine tags dropped (`["Mythology","Greek literature"]`), `cover_id 555` (first positive of work covers `[-1,555]`), authors, `on_shelf null`, `readers []`; after `clear_search_cache()` a second view makes no outbound calls and returns the persisted description/subjects.
- `test_search.py::test_book_detail_accepts_dict_description`: `{"type","value"}` description is unwrapped and trimmed; no subjects → `[]`; no work covers → `cover_id` from search doc (123).
- `test_search.py::test_book_detail_flattens_markdown_in_description`: reference links, inline links, blockquote markers, extra blank lines and reference definitions are flattened to the exact expected text.
- `test_search.py::test_book_detail_includes_own_shelf_state_and_other_readers`: ada reading at 40%, grace finished with 5 stars and take; grace's view makes no OL calls, shows her own `on_shelf/rating/take`, `readers == [ada]` with status and progress 40, and local stub fields (`year 2018`, `cover_id 123`, `title`, `description ""`).
- `test_search.py::test_book_detail_rejects_malformed_work_id`: `/works/not-a-key` and `/works/OL1M` → 404 with no OL calls.
- `test_search.py::test_book_detail_propagates_not_found`: OL 404 on the work JSON → 404 `"No such book."`.
- `test_search.py::test_upstream_failure_is_a_502`: OL 404 on `search.json` → 502 with `SEARCH_UNAVAILABLE`.
- `test_search.py::test_detail_survives_author_lookup_failure`: with `/authors/` failing, detail still returns title and authors from the search doc.
- `test_search.py::test_book_detail_refresh_updates_local_row`: a title-only shelf stub reads locally (`description ""`, `year null`, no calls); refresh fetches work JSON and search, fills description, subjects, cover 555, year 2018; shelf book `pages 393`; `/work/OL1W` shows `pages 393`, `ol_rating 4.27`, `year 2018`; after clearing caches `/works/OL1W` still has the data with no calls.
- `test_search.py::test_refresh_bypasses_author_cache`: after a first view with "Madeline Miller", refresh returns the author API's new name "M. Miller".
- `test_search.py::test_search_cache_is_bounded`: with cap 2, inserting a, b, c evicts a and keeps b and c.
- `test_search.py::test_identical_inflight_searches_share_one_request`: concurrent `circe` and `CIRCE` `fetch_open_library` calls make one `search.json` request and both get Circe.
- `test_search.py::test_search_cache_ttl_is_at_least_half_a_day`: `_CACHE_TTL >= 12 h`.
- `test_search.py::test_open_library_connect_timeout_is_short`: connect timeout 8.0 s (< 10), read 15.0 s.
- `test_search.py::test_cancelled_fetch_unblocks_coalesced_waiters`: cancelling the owning fetch makes the coalesced follower fail with a 502 instead of hanging, and `_inflight` ends empty.
- `test_search.py::test_short_query_is_a_400_and_never_hits_open_library`: `q=it` and `q=ab` → 400 mentioning "3 characters", no search call.
- `test_search.py::test_upstream_422_is_a_helpful_400`: OL 422 → 400 mentioning "3 characters".
- `test_search.py::test_upstream_429_is_a_429_with_retry_hint`: OL 429 with `Retry-After: 20` → 429 whose detail contains "20 seconds".
- `test_search.py::test_isbn_query_uses_isbn_field`: `q=978-0-316-76948-8` sends `q=isbn:9780316769488`; hit has title Circe, `isbn 9780316769488`, `cover_edition_key OL1M`.
- `test_search.py::test_empty_search_pages_are_not_cached`: an empty `docs` result is fetched again on the next identical search.
- `test_search.py::test_title_fallback_when_raw_query_misses`: raw `obscurexyz` misses → `title:` and `author:` fallbacks are sent, result is Circe.
- `test_search.py::test_search_candidates_keep_raw_query_before_by_split`: unit: `"Stand by Me"` → `["Stand by Me", 'title:"Stand" author:"Me"']`; `"Circe by Madeline Miller"` → `[raw, 'title:"Circe" author:"Madeline Miller"']`.
- `test_search.py::test_title_containing_by_uses_raw_query_first`: `q="Stand by Me"` sends the raw query first, never the fielded split (primary hit), result `OLSTANDW`.
- `test_search.py::test_local_catalog_is_searched_without_open_library_for_custom_books`: `POST /custom` → 201, `custom true`, key `/works/BC…`; its detail makes no OL calls; search `zine` lists it first with `custom true`; refresh → 400 mentioning "Open Library".
- `test_search.py::test_local_catalog_hits_survive_open_library_outage`: with `search.json` 503, `q=zine` still returns 200 with the custom book first, `custom true`, `has_more false`.
- `test_search.py::test_custom_book_requires_a_title`: whitespace title → 400 (or 422).

### `backend/tests/test_book_details.py` (8)

Fixture `ol`: fake client in `openlibrary.py` returning `WORK_JSON` (dict description with `----` trailer,
5 subjects, covers `[123]`) and `SEARCH_JSON` (one doc with pages 393, rating 4.27/812); `isbn:` queries
containing `0000000000` return no docs.

- `test_book_details.py::test_work_details_for_unknown_book`: `/work/OL1W` → title, authors, `pages 393`, `ol_rating 4.27`, `ol_rating_count 812`, description without the source trailer, subjects `["Greek mythology","Fiction","Witches"]` (comma and >40-char entries dropped), `on_shelf null`, `members []`, `club_pick false`, `cover_url` ends `/123-L.jpg?default=false`; exactly 2 outbound calls; after `clear_details_cache()` a second read makes no calls.
- `test_book_details.py::test_work_details_persist_on_known_book_and_list_members`: ada finished (5, "Loved it"), grace reading at 40% and sets the pick; grace's `/work/OL1W` makes no calls, `club_pick true`, `on_shelf currently_reading`, `shelf_id` set, members `["grace","ada"]` (viewer first), ada's member object exactly `{username,status,rating,take,progress:null,finished_at}`, grace progress 40, `pages null`; refresh → `pages 393` and shelf book `pages 393`; after clearing caches a read makes no calls.
- `test_book_details.py::test_work_details_rejects_bad_ids_and_upstream_failure`: `/work/not-a-key` → 400; connect error → `/work/OL9W` 502.
- `test_book_details.py::test_isbn_lookup`: `/isbn/12345` → 400; `/isbn/0000000000` → 404; `978-0-316-55634-7` → 200 with `isbn 9780316556347`, `OL1W`, title, `pages 393`, `on_shelf null`; after shelving Circe the same lookup returns `on_shelf want_to_read` and a `shelf_id` with no new outbound call (ISBN cache).
- `test_book_details.py::test_isbn10_with_x_check_digit_is_accepted`: `/isbn/080442957X` → 200.
- `test_book_details.py::test_work_details_skips_leading_negative_cover`: search `cover_i -1`, work covers `[-1,0,555]` → `cover_id 555`, `cover_url` ends `/555-L.jpg?default=false`.
- `test_book_details.py::test_positive_cover_id_rejects_placeholders`: unit: `-1`, `0`, None → None; `first_positive_cover(-1,[0,-1,99],12) == 99`; `extract_isbn("978-0-316-76948-8") == "9780316769488"`, `extract_isbn("it") is None`.
- `test_book_details.py::test_cancelled_work_details_unblocks_coalesced_waiters`: cancelling the owning `fetch_work_details` fails the coalesced follower with 502 and leaves `_details_inflight` empty.

### `backend/tests/test_local_reads.py` (2)

- `test_local_reads.py::test_known_book_reads_never_touch_open_library`: after shelving, picking, a diary entry, a quote and a nomination for Circe, with every HTTP client constructor raising, `GET` shelf, pick, pick history, overlap, members, vote, vote suggestions, diary, activity, stats, quotes, `/books/works/OL1W`, `/books/work/OL1W` and `/books/works/OL1W/posts` all return 200.
- `test_local_reads.py::test_shelving_persists_search_metadata_for_later_local_reads`: shelf add returns the book with key, title, authors, `cover_id 123`, `year 2018`, `cover_url` ending `/123-L.jpg?default=false`; with HTTP blocked, `/books/works/OL1W` returns those fields and `on_shelf want_to_read`.

Related, out of scope for this checklist: `test_google_books.py` (51 tests) is dropped with Google Books
(#49 lists it as an intentional change). A few of its ideas stay provider-agnostic: an empty provider page
does not freeze in the cache, a later page never mixes providers, custom books skip catalogs, and ISBN `x`
and `X` are the same row.

---

## 5. Multi-club mapping

### 5.1 What is global, personal or club-scoped

| concern | Python today | new server |
|---|---|---|
| Catalog records (works, editions, authors, covers) | one `books` table per instance | **Global** catalog shared by every account and club (#41). Not club-scoped and not personal. |
| Search, trending, subjects, ISBN lookup, details | per instance | Global endpoints; no club in the path. |
| `on_shelf`, `shelf_id`, `rating`, `take`, `dnf_reason`, `progress`, `favorite_position` | viewer | Personal (viewer). Unchanged meaning. |
| `readers` / `members` / `club_rating` / `rating_count` / `quote_count` | everyone in the instance | **Audience-filtered** by the #46 policy: only users who share a club with the viewer (later friends), and only items they have not set to "only me" (ratings, quotes). Strangers never appear. Exactly one central policy function. |
| `club_pick` | the one current pick | **Club-scoped**: a work can be the current pick in several of the viewer's clubs. A single boolean is meaningless on a global endpoint; see open question Q2. |
| Custom books | club-typed, no creator | User-added works (and editions) with a `created_by` and provenance; see Q5. |
| Search history | personal | Personal (search-history area, not here). |

### 5.2 Works vs editions

- Python stores one row per *work key* and mixes work- and edition-level facts: `pages` is OL's median,
  `year` is first publication, ISBN only exists inside `ISBN…` keys, the cover is whatever came first.
- New: `works` (title, authors via a link table, first publish year, description, subjects, OL work id,
  OL rating, work cover) and `editions` (ISBN-13, publisher, language, pages, planned publication date,
  edition cover), per #37/#41. A shelf entry points at a work and optionally at an edition; club picks point
  at works.
- `pages` in `BookOut`/`BookDetailsOut` becomes either the edition's pages (when the shelf entry has an
  edition) or a work-level display value (median). Milestones no longer depend on it (percent/chapter).
- `/isbn/{isbn}` resolves to an **edition** and its work. ISBN-10 input is converted to ISBN-13; 979
  ISBNs have no ISBN-10 form; checksum validation becomes possible (the parity fixtures `0000000000`,
  `080442957X`, `9780316556347`, `9780316769488` are all checksum-valid, so the tests survive it).
- ISBN lookup chain per #41: local editions → DNB SRU (fallback only, cached and throttled) → Open
  Library. Python's lookup never writes; the plan requires that anything scanned is copied into the
  catalog, so the lookup now persists the edition and work.

### 5.3 Work identifiers in the API

- Today every client-facing book reference is `ol_work_key` (`/works/{id}`), and routes use the bare id
  (`/book/OL1W`, `/api/books/works/OL1W`). Shelf add, pick set, vote nomination, quote and diary create
  all send `ol_work_key` plus client metadata.
- New works that come only from DNB or from users have no OL id. The server needs a stable public work id
  that is not an OL id, and must still accept `OL…W` (for remote OL hits that are not yet in the catalog)
  and resolve legacy `BC…`/`ISBN…`/`GB…` ids from the importer's mapping, so old links keep working. See Q1.
- `custom: bool` becomes "user-added" (or a `source` enum: `openlibrary`, `dnb`, `user`).

### 5.4 Google removal

Drop: `GOOGLE_BOOKS_API_KEY`, provider selection, `googlebooks.py`, `fetch_google_catalog`, the Google
branches of every endpoint, `GOOGLE_NO_REFRESH`, `cover_image_url`, the Google cover host allow-list,
`ISBN…`/`GB…` as live id kinds (legacy lookup only), and the OL `b/isbn/` cover fallback in `cover_url`.
The importer re-matches Google rows by ISBN and drops Google description/subjects/rating/cover (#38). No
Google code or data may remain in `server/` (#41 AC).

### 5.5 Covers

- `cover_url` stops pointing at `covers.openlibrary.org`. It becomes a same-origin media URL (#43):
  OL covers are fetched once **by cover id** (the OL covers API limits ISBN/OLID lookups per IP, not
  cover-id lookups) and served from our domain; user photos are stored as WebP.
- Every hit and detail must carry a usable `cover_url` (today remote hits send null and the browser
  builds OL URLs from `cover_id`/`cover_edition_key`/ISBN). The frontend fallback chain in `BookCover.vue`
  must go (#48), because the AC is "no client request to openlibrary.org". The S/M/L size tiers the
  frontend uses need a server equivalent (sized variants or a size parameter).
- Three tests assert `cover_url` ends with `/123-L.jpg?default=false` or `/555-L.jpg?default=false`; their
  Rust counterparts assert a same-origin cover URL derived from that cover id.

### 5.6 Roles and permissions

- Catalog reads and custom-book creation need only a signed-in account (cookie or bearer token, #39).
  No club role is involved.
- Edits to existing records go to the admin queue (#54), not straight into the row. That forbids the
  Python `upsert_book` behaviour of overwriting `title`/`authors`/`year`/`cover` from client payloads in
  other areas (R1).
- Refresh-from-source is not a user edit but costs several outbound calls; it needs a per-user rate
  limit and must not overwrite admin-approved corrections (Q8).

### 5.7 Things that no longer make sense, or should be merged

- **Two import pipelines** (§2.10 A and B) with different cleaning, cover precedence, author handling and
  error mapping. Merge them into one normalizer. A single rule set satisfies every parity test: description
  = B's trailer removal plus A's markdown flattening; subjects = drop blanks, `:`-containing, `,`-containing,
  > 40 chars, dedupe case-insensitively, max 12; covers = work covers first, then search `cover_i`; authors =
  search names first, author API on miss, author API first on refresh; errors = 404 → "No such book.",
  429 → 429 hint, other → 502.
- **Two detail DTOs** (`BookDetailOut` on `/works/`, `BookDetailsOut` on `/work/`): the UI uses only the
  first. Either port both for parity or merge into one DTO carrying `pages`, `ol_rating` and `quote_count`
  too (Q6).
- `details_fetched_at` is write-only; keep it as a `synced_at` for staleness and background refresh, or
  drop it.
- The UA built from `BOOKCLUB_NAME`/`BOOKCLUB_PUBLIC_URL` (instance branding) becomes a fixed service UA
  with a contact address (#41).
- Process-local caches for **work details** and **ISBN hits** lose most of their value once everything is
  persisted locally first; the search/trending/subject page caches and in-flight coalescing remain useful.
- `lang=en` on subject browse assumed an English club; German users now get DNB-backed data (Q14).

### 5.8 Test mapping notes for #49

- Tests that need two users (`…includes_own_shelf_state_and_other_readers`,
  `…persist_on_known_book_and_list_members`) must put both users in one club first; add a stranger case
  that must not appear in `readers`/`members`.
- `club_pick` assertions (`test_search_maps_and_caches`, `test_work_details_persist_on_known_book_and_list_members`)
  become per-club assertions.
- `test_local_reads` endpoints move under `/api/clubs/{slug}/…` for pick, history, overlap, members, vote,
  suggestions, activity and stats.
- Python-internal unit tests (`_cache_set` bound, TTL constant, timeout constants, `_search_candidates`,
  `positive_cover_id`, coalescing and cancellation) become Rust unit tests on the cache and OL client
  modules; with an off-the-shelf cache (e.g. moka) "bounded" replaces "strict LRU order", and a cancelled
  owner may let a follower retry instead of failing with 502 (Q16).
- Outbound HTTP in tests: point a configurable OL base URL at a mock server (e.g. wiremock) so request
  params, call counts and the User-Agent can be asserted the same way the Python fakes do, and provide a
  "no network" client that fails the test on any call for the `test_local_reads` equivalents.

---

## 6. Port risks

- **R1. Client metadata overwrites the shared catalog.** `upsert_book` sets `title` unconditionally (and
  authors/year/cover when given) from whatever the client posts to shelf add, pick, vote, quote, diary
  and Goodreads import. In a global catalog this lets any user rename a book for everyone. The new
  server must create a stub only for an unknown work (ideally verified against the source) and never
  overwrite an existing record from client input. The parity test
  `test_shelving_persists_search_metadata_for_later_local_reads` only needs stub creation for an unknown
  work.
- **R2. Local-first means no remote call on read, even for stubs.** A stub row without description must
  be returned as-is (`description ""`), not enriched in the request path. Any "fetch if missing" logic
  breaks `test_book_detail_includes_own_shelf_state_and_other_readers` and the `test_local_reads` tests.
- **R3. GET writes.** `GET /works/{id}` and `GET /work/{id}` insert and commit on first view. Concurrent
  first views race on the unique key (Python would 500 on one of them). Use
  `INSERT … ON CONFLICT (…) DO UPDATE/NOTHING` and re-read.
- **R4. Status mapping differs by path.** `/works/` maps OL 404 → 404, 429 → 429, 422 → 400; `/work/` and
  `/isbn/` map everything to 502; trending and search map 404 → 502; subjects map 404 → 404 "No such
  subject.". Retry-After only counts as a positive integer. The frontend switches UI by status.
- **R5. Merge can return `limit + 1` items** (the length check runs after appending). Decide whether to
  keep this quirk; no test pins it, and capping at `limit` is the obvious fix.
- **R6. Pagination gap.** Local hits push remote hits off page 1, and page 2 starts at OL offset `limit`, so
  those remote hits are never shown; `has_more` ignores local hits.
- **R7. Fallback semantics.** Fallbacks run only on page 1 after a *mapped* empty primary, concurrently,
  first non-empty in candidate order wins, and one failing fallback fails the whole search. The raw query
  must stay first for "X by Y" (lazy split at the first " by ", values quoted without escaping).
- **R8. Empty-box and short-query rules.** Empty `q` and empty/invalid `subject` serve trending with
  `page: 1` regardless of `page`/`sort`, using the search `limit` (default 24) as the trending cache key.
  Invalid subjects are silently ignored in search but 400 on `/subjects/`. Length limits count code points
  (`chars().count()` in Rust, not bytes). A short query with a subject is allowed.
- **R9. Remote `isbn` on hits is effectively always null.** `fields` omits `isbn`, so real OL search docs
  have no ISBN; the test fake injects one. For an ISBN query the new server should set the hit's ISBN from
  the query (or from the edition it resolves), which still satisfies `test_isbn_query_uses_isbn_field`.
- **R10. Rounding.** Python `round()` breaks exact ties half-to-even (`[1,1,1,2]` → 1.2, not 1.3);
  Rust `f64::round` and Postgres `round(numeric)` round half away from zero. No test pins it; decide and
  document.
- **R11. casefold vs lowercase.** Cache keys and local matching use `casefold()` (`ß` → `ss`). Rust's
  `to_lowercase()` and Postgres `lower()` keep `ß`. Python `str.split()` also treats `\x1c`–`\x1f` as
  whitespace; Rust `split_whitespace` does not.
- **R12. Two description and subject cleaners.** Porting only one fails half of the tests (§2.11/§2.12).
  The regexes rely on Python `re` semantics (`MULTILINE` anchors, `[\s\S]`); Rust `regex` supports them but
  needs `(?m)` explicitly.
- **R13. Lenient JSON coercion.** Python `int()` accepts floats (truncating), numeric strings and bools;
  `float()` accepts numeric strings; a string `author_name` is iterated character by character
  (`"A, n, n"`). A strict serde model would drop whole docs or fail the page. Deserialize into
  `serde_json::Value` and coerce field by field; treat a string `author_name` as one name (an intentional
  fix).
- **R14. Non-JSON upstream bodies are a 500 in Python.** Map them to the call's 502 in Rust (intentional
  improvement).
- **R15. Caching details.** Raw payloads are cached, never annotated results. Empty search `docs` lists are
  not cached, but trending/subject/work-search empties are. Refresh bypasses reads but writes the cache.
  Expired entries are purged on every set. Errors are never cached. ISBN misses are not cached.
- **R16. Coalescing and cancellation.** Followers must never hang when the owner is dropped (axum drops
  handler futures when a client disconnects). With a hand-rolled map, use a drop guard that resolves
  waiters; with moka's `try_get_with`/`optionally_get_with` the init is retried by a waiter. Also note that
  Python keys coalescing by the normalized cache key, so `circe` and `CIRCE` share one request.
- **R17. Annotation scope.** Annotations are per viewer: shelf state by work key, current pick by highest
  open pick id. In multi-club, readers/members/ratings/quote counts need the #46 audience filter; porting
  the Python "everyone in the DB" queries leaks strangers' data.
- **R18. Sorting with Postgres collations.** `readers` sorts by username in code-point order; members put
  the viewer first. Use `COLLATE "C"` or sort in Rust; a locale collation can order `_` differently.
- **R19. Unbounded `page`.** `page` has no upper limit; `page * limit` and `(page - 1) * limit` overflow
  32-bit integers. Clamp or use checked i64 arithmetic.
- **R20. Unvalidated legacy keys and lengths.** Legacy `ol_work_key` values can be any `/works/…` string,
  titles have no length limit, and years come unvalidated from clients. The importer and the new write
  paths must validate (and the importer must map or quarantine non-conforming keys).
- **R21. Timestamps.** SQLite datetimes are naive UTC, and Python serializes `finished_at` without an
  offset; the frontend's `shelf.ts` appends `Z` when it is missing. Rust should emit RFC 3339 with an
  offset, which that helper accepts, but every other `new Date(iso)` call in the frontend should be checked.
- **R22. Outbound politeness is new work, not parity.** Python has no client-side rate limit, a new TLS
  connection per request (2–4.7 s handshakes measured), and no contact address in the UA. #41 requires
  ≤3 req/s to OL with a contact UA; reuse one `reqwest::Client` with connect 8 s / read 15 s timeouts.
  Decide what happens when the global limiter is saturated (queue briefly, then 429 with the existing
  `RATE_LIMITED` string).
- **R23. Persisting search results.** The plan copies "every book anyone searches" into the catalog. Doing
  that for every remote hit (up to 40 per page, plus trending/subjects) makes search a write-heavy path;
  batch the upserts and keep them off the response's critical path, without overwriting richer records
  (R1).

### Open questions (decisions this spec does not settle)

- **Q1.** Public work id and URL scheme: an opaque id plus an optional OL id, or keep `/works/OL…W` keys?
  How do legacy `BC…`/`ISBN…`/`GB…` ids and non-conforming keys resolve (redirect table from the importer)?
- **Q2.** `club_pick` on global search and detail: a list of the viewer's clubs whose current pick this is,
  a club query parameter, or drop it from global endpoints?
- **Q3.** `club_rating`, `readers`, `members`, `quote_count`: one audience-wide value, per club, or both?
  Rename `club_rating` accordingly.
- **Q4.** Persist every remote search, trending and subject hit, or only works that are opened, scanned or
  shelved?
- **Q5.** User-added ("custom") works: globally searchable, or visible only to the creator and their
  clubs until reviewed? Who owns legacy `BC…` rows that have no creator?
- **Q6.** Port `/work/{id}`, `/work/{id}/refresh` and `/isbn/{isbn}` as they are (no frontend caller), or
  merge the two detail DTOs into one endpoint and keep `/isbn` for scanning (#56)?
- **Q7.** May OL descriptions and ratings be stored under the CC0-only rule? (OL data is CC0, but many
  descriptions are publisher blurbs.) Should `ol_rating_count` be stored?
- **Q8.** Who may refresh a work, how often, and may a refresh overwrite admin-approved corrections (needs
  per-field provenance)?
- **Q9.** For shelf/pick/vote/quote/diary writes that reference a work unknown to the catalog: trust the
  client stub, or resolve it against OL first?
- **Q10.** Local search ranking and how `sort` (`readinglog`, `new`, `title`, `relevance`) applies to local
  results; how to dedupe DNB-only works against OL remote hits that carry no ISBN.
- **Q11.** Keep the Python quirks (merge returns `limit + 1`, `/works/` 422 → short-query 400, fallback
  failure fails the search, half-to-even rounding) or fix them and record the change in #49?
- **Q12.** Background enrichment of stub works (empty description) outside the request path: allowed, or
  strictly on refresh only?
- **Q13.** Behaviour when the global OL rate limiter is saturated: wait (how long) or 429?
- **Q14.** Subject browse: keep OL `lang=en`, follow the user's locale, or add DNB-backed browse rows for
  German users?
- **Q15.** Keep the exact English `detail` strings (none of them is mapped in `apiErrors.ts`, so German
  users see English), or add them to the frontend mapping as part of #48?
- **Q16.** Is it acceptable that a coalesced follower retries instead of getting a 502 when the owning
  request is cancelled (changes `test_cancelled_*` expectations)?
