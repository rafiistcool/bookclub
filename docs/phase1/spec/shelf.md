# Port spec: personal shelf

Area: personal shelf. It covers statuses, ratings, takes and DNF reasons, reading progress, reading dates, the free shelf order, Goodreads CSV import, spine colours, favourites, and the physical shelf layout (one per user).

Python sources:
- `backend/app/routers/shelf.py`, `backend/app/shelf_ops.py`, `backend/app/goodreads.py`, `backend/app/spine_color.py`, `backend/app/favorites.py`
- Favourites endpoints in `backend/app/routers/auth.py` (lines 110–142)
- Models `ShelfEntry`, `UserPhysicalShelfLayout`, `UserFavorite`, `Book`, `ShelfStatus` in `backend/app/models.py`
- Schemas in `backend/app/schemas.py`: `ShelfAddIn`, `ShelfPatchIn`, `ShelfItemOut`, `ShelfListOut`, `BookOut`, `UserOut`, `FavoriteOut`, `FavoritesIn`, `FavoritesOut`, `PhysicalSpine`, `PhysicalDecoration`, `PhysicalShelfLayout`, `SpineColorsOut`, `GoodreadsSkipOut`, `GoodreadsImportOut`
- Helpers in `backend/app/serialize.py`, `covers.py`, `works.py`, `timezone.py`, `activity.py`

Behaviour marked **(probed)** was confirmed by running the Python app on 2026-10-27. The probe scripts were throwaway session files and are not in the repo.

---

## 0. Conventions that apply to every endpoint

- **Auth:** every endpoint below needs a signed-in session. Without one, the response is `401 {"detail": "Not signed in"}`. This also happens when the session's user no longer exists (the session is cleared).
- **Validation errors:** these return **400**, not 422. The body is `{"detail": "<msg1>; <msg2>…"}`: the Pydantic messages joined with `"; "`, with any `"Value error, "` prefix stripped. Custom validator messages (listed per endpoint) must be copied **verbatim**. Generic Pydantic messages (e.g. `"Input should be less than or equal to 5"`) are listed for reference. The frontend does not map them, so the port may word them differently, but it must keep status 400.
- **Path params** typed `int`: a value that isn't an integer gives 400 (`"Input should be a valid integer, unable to parse string as an integer"`).
- **Datetimes in responses:** Python emits naive ISO strings with no offset, which are implicitly UTC. Microseconds appear when non-zero, e.g. `"2026-10-27T10:46:20.818621"` and `"2024-02-03T12:00:00"` **(probed)**. The frontend (`shelf.ts:clubDate`) adds `Z` when no offset is present, so the Rust server may (and should) emit RFC 3339 with `Z`.
- **Instance timezone:** `resolved_timezone(BOOKCLUB_TZ)` falls back to `"UTC"` when unset or not a valid IANA name. `club_zone()` returns the matching `ZoneInfo`. See §5 for what replaces it.
- **Error strings mapped in `frontend/src/i18n/apiErrors.ts`** (exact match; changing any of them means updating the mapping):
  - `"Reading date requires a finished book"`
  - `"Reading date cannot be in the future"`
  - `"That book is not in the club yet"`
  - `"Each book can only be a favourite once"`
  - `"At most 3 favourites"`
  - `"You already have 3 favourites"`
  - `"Not signed in"`

### Shared response objects

`UserOut`:

| field | type | notes |
|---|---|---|
| `id` | int | |
| `username` | string | |
| `theme` | string | `paper` \| `slate` \| `forest` \| `ink` |
| `color_mode` | string | `light` \| `dark` \| `system` |
| `locale` | string | `en` \| `de` |
| `avatar_url` | string \| null | `"/api/members/{username}/avatar?v={unix seconds of avatar_updated_at}"`, or null when the user has no avatar |

`BookOut`:

| field | type | notes |
|---|---|---|
| `id` | int | books.id |
| `ol_work_key` | string | `"/works/<work id>"`, e.g. `/works/OL1W`, `/works/ISBN9780316769488`, `/works/GBxyz`, `/works/BCdeadbeef01` |
| `title` | string | |
| `authors` | string | comma-separated display string; may be `""` |
| `cover_id` | int \| null | Open Library cover id (>0) |
| `year` | int \| null | |
| `cover_url` | string \| null | see the rule below |
| `pages` | int \| null | |
| `spine_color` | string \| null | `"#rrggbb"`; the stored `""` ("no usable cover") is emitted as **null** |

`cover_url` rule (`serialize.cover_url`), first match wins:
1. `books.cover_image_url` if it passes `storable_cover_image_url`: https only, at most 512 chars, no userinfo, host is `books.google.com`, `*.books.google.com`, `googleusercontent.com` or `*.googleusercontent.com`, and never the OL CDN.
2. `https://covers.openlibrary.org/b/id/{cover_id}-L.jpg?default=false` if `cover_id > 0`.
3. If `GOOGLE_BOOKS_API_KEY` is set: null.
4. If the work id is `ISBN…`: `https://covers.openlibrary.org/b/isbn/{isbn}-L.jpg?default=false`.
5. Otherwise null.

`ShelfItemOut`:

| field | type | notes |
|---|---|---|
| `id` | int | shelf entry id |
| `status` | enum | `want_to_read` \| `currently_reading` \| `finished` \| `did_not_finish` |
| `position` | int | 0-based index inside the (user, status) column |
| `shelf_position` | int \| null | manual order across all statuses; null means "not arranged yet" |
| `updated_at` | datetime | |
| `started_at` | datetime \| null | |
| `finished_at` | datetime \| null | |
| `book` | BookOut | |
| `rating` | int \| null | 1–5, only ever non-null for `finished` |
| `take` | string | ≤140 chars, `""` when none; only non-empty for `finished` |
| `dnf_reason` | string | ≤200 chars, `""` when none; only non-empty for `did_not_finish` |
| `progress` | int \| null | percent 0–100, only ever non-null for `currently_reading` |

`FavoriteOut`: `{ "position": int (1..3), "book": BookOut }`

---

## 1. Endpoints (12)

### 1.1 `GET /api/shelf`

Returns a user's whole shelf plus their favourites.

**Query**
- `username` (optional string). When present and non-empty, look up `users.username == username.lower()`. The value is lowercased but **not trimmed**.
  - Unknown user: **404** `{"detail": "No member with that name"}`.
  - Missing or empty (`?username=`): the caller's own shelf **(probed)**.
  - Today **any** signed-in user can read any user's shelf (single club). See §5.

**Response 200** `ShelfListOut`:
```json
{
  "timezone": "UTC",
  "user": UserOut,
  "items": [ShelfItemOut],
  "favorites": [FavoriteOut]
}
```
- `timezone`: `resolved_timezone(BOOKCLUB_TZ)`.
- `user`: the **owner** of the shelf, not the viewer.
- `items`: every shelf entry of the owner, ordered `ORDER BY status, position, id`. `status` is a VARCHAR, so this is **alphabetical**: `currently_reading`, `did_not_finish`, `finished`, `want_to_read` **(probed)**. Entries whose book row is missing are skipped (defensive). No pagination.
- `favorites`: the owner's favourites, ordered by `position, id`. Rows with a missing book are skipped.

**Side effects:** none.

---

### 1.2 `POST /api/shelf`

Adds a book to the caller's shelf. Status code **201**.

**Body** `ShelfAddIn` (JSON). Unknown keys are ignored.

| field | type | default | validation (in order) |
|---|---|---|---|
| `ol_work_key` | string | required | trimmed; must start with `/works/`, else `"ol_work_key must be a /works/ key"` |
| `title` | string | required | trimmed; empty gives `"title is required"`; **no max length** |
| `authors` | string | `""` | **not trimmed**, no limit |
| `cover_id` | int \| null | null | any int; values ≤0 are treated as null in storage |
| `cover_url` | string \| null | null | trimmed, then `storable_cover_image_url`; anything other than a Google cover host silently becomes null |
| `year` | int \| null | null | any int |
| `status` | enum | `want_to_read` | invalid gives `"Input should be 'want_to_read', 'currently_reading', 'finished' or 'did_not_finish'"` |
| `rating` | int \| null | null | 1..5 (`"Input should be greater than or equal to 1"` / `"…less than or equal to 5"`) |
| `take` | string | `""` | trimmed; more than 140 **characters** gives `"Keep the take to 140 characters"` |
| `dnf_reason` | string | `""` | trimmed; more than 200 characters gives `"Keep the reason to 200 characters"` |
| `progress` | int \| null | null | 0..100 |

**Processing order**
1. `upsert_book(...)` with the payload's metadata (see §2.1). The work id is the **last path segment** of `ol_work_key`, canonicalised (ISBN `x` becomes `X`), and the stored key is `/works/<that>`. So `/works/a/b` is stored as `/works/b`, and the id format is **not** validated **(probed)**.
2. If the caller already has an entry for that book: return **409** with this body, and do **not** commit the metadata upsert (it is rolled back):
   ```json
   {"detail": "Already on your shelf", "item": ShelfItemOut}
   ```
   The frontend reads `err.item` in `HomePage.vue` and `BookDetailPage.vue`.
3. Create the entry with `status`, `position=0` and `shelf_position=NULL`.
4. `apply_finish_fields(entry, status, rating, take, dnf_reason)`. Values that don't apply to the status are **silently dropped** (§2.3), so a rating sent with `want_to_read` is stored as null **(probed)**.
5. `apply_progress(entry, status, progress)`. Progress is kept only for `currently_reading`.
6. `place_item(entry, status, 0)`: insert at the top of the status column, renumber the column 0..n-1, set `updated_at=now`, and stamp dates with `changed=False` (§2.4). The resulting dates:

   | status | `started_at` | `finished_at` |
   |---|---|---|
   | `want_to_read` | null | null |
   | `currently_reading` | now | null |
   | `finished` | now | now |
   | `did_not_finish` | now | null |

7. Record an activity event (§2.6), with `added=True`:

   | status | event |
   |---|---|
   | `finished` | `shelf_finished` `{rating?, take?}` |
   | `did_not_finish` | `shelf_dnf` `{reason?}` |
   | anything else | `shelf_added` `{status}` |

8. Commit, reload, and return `ShelfItemOut`. If the reload fails: **500** `"Could not save that book"`. This should be unreachable.

**Concurrency:** two concurrent adds of the same book violate `UNIQUE(user_id, book_id)`. Python does not catch this, so it returns a 500. The port should answer 409 with the existing item.

---

### 1.3 `POST /api/shelf/import`

Imports a Goodreads CSV export. Status code **200**.

**Body:** `multipart/form-data` with field **`file`**, which is required. A missing field gives 400 `"Field required"`.

**Processing**
1. Read at most `MAX_IMPORT_BYTES + 1` = 1,500,001 bytes of the file.
2. `parse_goodreads_csv(raw)` (§2.7). It raises `ValueError`, which becomes **400** `{"detail": str(exc)}`. Exact strings:
   - `"That file is too large. Keep the export under 1.5 MB."` when the file is more than 1,500,000 bytes.
   - `"The file must be a UTF-8 Goodreads CSV export"` when the bytes aren't valid UTF-8 (a BOM is allowed).
   - `"That does not look like a Goodreads export"` when there's no header row (empty file) or the headers (trimmed, case-insensitive) lack `exclusive shelf` or `title`.
3. If Google Books is enabled and in its 120 s cooldown: **429** `"The library is busy. Wait a few seconds and try again."`, before any lookup. This only happens when there is at least one parseable row (the function returns early on an empty row list).
4. `lookup_catalog(rows)` runs at most **2 lookups at a time** (semaphore) and preserves CSV order (§2.7).
5. For each `(row, hit, reason)` in CSV order:
   - `hit is None`: add skip `(row.title, reason or "No match")`. The reason is `"No match"`, or `"Catalog unavailable"` when a `GoogleBooksError` was raised.
   - Otherwise `upsert_book` with the **hit's** metadata (OL title, authors, cover, year), not the CSV's.
   - If the caller already has that book: skip `(row.title, "Already on your shelf")`. This includes a book added by an **earlier row of the same file**, because inserts are flushed **(probed)**.
   - Otherwise create an entry with `status=row.status` and `position=0`, then `place_item(entry, row.status, 0)`. This inserts at the top of the column, so **within a status the CSV order ends up reversed** (the last row is on top) **(probed)**. Dates are stamped as in 1.2, so an imported `read` book gets `started_at = finished_at = import time`.
   - No rating, take, DNF reason or progress is set.
   - **No activity events** are recorded **(probed)**.
6. Commit once at the end. A crash anywhere loses the whole import.

**Response 200** `GoodreadsImportOut`:
```json
{
  "imported": 2,
  "skipped": 3,
  "skips": [{"title": "Custom Shelf Book", "reason": "Unknown shelf"}]
}
```
- `imported`: number of entries created.
- `skipped`: the **total** number of skips.
- `skips`: the **first 25** (`SKIP_LIST_LIMIT`).
- Skip order: parse-time skips in CSV order, then lookup-time skips in CSV order **(probed)**.

Skip reasons (verbatim):
- `"Stopped after 400 rows"`, with title `""`
- `"No title"`, with title `"Untitled"`
- `"Unknown shelf"`
- `"No match"`
- `"Catalog unavailable"` (Google only)
- `"Already on your shelf"`

The UI renders `skip.title || "Untitled"`, and the reasons are shown untranslated.

**Outbound HTTP:**
- `GET https://openlibrary.org/search.json` with params:
  - `q=<query>`
  - `limit=5`
  - `fields=key,title,author_name,cover_i,first_publish_year`
- Header `User-Agent: "{token}/1.0 ({contact}; catalog browse via Open Library)"`, e.g. `"Bookclub/1.0 (private book club; catalog browse via Open Library)"` **(probed)**.
- Timeout: connect 8 s, read 15 s, write 5 s, pool 5 s.
- One `AsyncClient` is shared for the whole import.
- In Google mode, Google `search_volumes(query, page=1, limit=5)` is used instead.

---

### 1.4 `GET /api/shelf/physical-layout`

Returns a user's physical shelf arrangement.

**Query:** `username` (optional), same lookup and 404 as 1.1: `"No member with that name"`, lowercased, empty means self.

**Response 200** `PhysicalShelfLayout`:
```json
{
  "rows": [[123, "decor:plant"], [], []],
  "material": "oak",
  "lights": false,
  "spines": {"123": {"color": "#ab3c45", "width": 37, "display": "spine"}},
  "decorations": {"decor:plant": {"kind": "plant", "color": "#abcdef"}}
}
```
- With no stored row, the default is `{"rows": [[], [], []], "material": "oak", "lights": false, "spines": {}, "decorations": {}}`.
- Each spine object always has all three keys: `color` (string or null), `width` (int or null) and `display`.

**Read-time pruning:** this is not written back. The stored JSON is parsed with the **current** schema; an invalid stored layout gives a 500 in Python, and the port must not do that. Then:
- Each row keeps every string item (decoration) and every int item that is a current shelf-entry id of the **owner**.
- Row boundaries are kept, even when a row ends up empty.
- `spines` keeps only keys whose `int(key)` is an owned entry id.
- `decorations` is not pruned.

**Side effects:** none.

---

### 1.5 `PUT /api/shelf/physical-layout`

Replaces the **caller's own** layout. There is no username param.

**Body** `PhysicalShelfLayout`. Every field is optional and missing fields take their defaults, so `{}` resets to the default layout. Unknown keys are ignored **(probed)**.

| field | type | default | rules |
|---|---|---|---|
| `rows` | list of lists of item | `[[],[],[]]` | 1..12 rows; see the item rules below |
| `material` | enum | `"oak"` | `oak` \| `walnut` \| `white` \| `black` |
| `lights` | bool | false | **lax** bool: `true`/`false`, `1`/`0`, `"true"`, `"yes"`… are accepted; `"sometimes"` is rejected **(probed)** |
| `spines` | object string → `PhysicalSpine` | `{}` | at most 2000 entries |
| `decorations` | object `DecorationId` → `PhysicalDecoration` | `{}` | at most 100 entries; must be an object (a list is rejected) |

Row item rules:
- An item is either a **strict** int greater than 0 (a shelf entry id; bools, floats such as `1.5` and numeric strings such as `"1"` are rejected), or a **strict** string matching `^decor:[a-zA-Z0-9_-]{1,64}$`.
- Field validator on `rows`, run after the items are validated, in this order:
  1. More than 2100 items in total: `"At most 2100 items fit on the physical shelf"`
  2. More than 2000 int items: `"At most 2000 books fit on the physical shelf"`
  3. Any repeated item across all rows: `"Each item can only appear once on the physical shelf"`
- Row count errors (Pydantic):
  - `"List should have at least 1 item after validation, not 0"`
  - `"List should have at most 12 items after validation, not 13"`

`PhysicalSpine`:
- `color`: string or null, pattern `^#[0-9a-fA-F]{6}$`, **lowercased**.
- `width`: **strict** int or null, 26..64. `30.5` and `"30"` are rejected.
- `display`: `"spine"` \| `"flat"` \| `"cover"`, default `"spine"`.
- Missing `color` and `width` mean "follow the book" (cover colour and page count).
- Spine keys must fully match `[1-9][0-9]{0,18}`, else `"Spine appearances must refer to shelf entry IDs"`. `"01"`, `"foo"` and a 5000-digit key all fail.

`PhysicalDecoration`:
- `kind` (required): `plant` \| `vase` \| `candle` \| `frame` \| `bookend` \| `globe`.
- `color` (required): pattern `^#[0-9a-fA-F]{6}$`, lowercased.

Model validator: the set of string ids in `rows` must **equal** the key set of `decorations`, else `"Each decoration must appear exactly once on the physical shelf"`. This catches orphan decorations, decorations missing from rows, and mismatched ids. A decoration placed twice fails the earlier "appear once" rule.

**Ownership check**, run after schema validation: the referenced ids are the int items in `rows` ∪ `{int(k) for k in spines}`. They must all be shelf-entry ids of the **caller**, else **400** `"Physical shelf books must be on your shelf"`. Spines don't have to correspond to placed items; they only need to be owned.

**Write:** an atomic upsert `INSERT … ON CONFLICT(user_id) DO UPDATE SET layout_json=excluded.layout_json`, so two devices saving their first layout at the same moment don't collide. The stored value is `payload.model_dump_json()`: compact JSON with defaults filled and colours lowercased. It never touches reading status or column order.

**Response 200:** the normalised payload echoed back. Colours are lowercased and defaults filled (`lights:false`, and per spine `color:null`, `width:null`, `display:"spine"`).

**Errors:**
- 400 for any validation failure (the previous layout is unchanged).
- 400 `"Physical shelf books must be on your shelf"`.
- 401.

---

### 1.6 `POST /api/shelf/spine-colors`

Samples cover colours for the caller's books that don't have one yet, one batch per call.

**Body:** none.

**Processing**
1. `pending` = books joined to the caller's shelf where `books.spine_color IS NULL`, ordered by `books.id`. This is only the caller's books, never another user's **(test)**.
2. `batch = pending[:12]` (`SPINE_COLOR_BATCH = 12`). All 12 are resolved **concurrently** (`asyncio.gather`).
3. `_resolve_spine_color(book)` returns one of three outcomes:
   - `""` means settled with no usable cover:
     - no cover URL;
     - `fetch_cover` returned None or empty bytes;
     - `fetch_cover` raised a **non-`httpx.HTTPError`** exception (e.g. `ValueError`, `InvalidURL`);
     - `dominant_color` returned None or raised.
   - `None` means retry later: `fetch_cover` raised `httpx.HTTPError`. That covers network errors, timeouts (including the 15 s overall deadline) and upstream status codes other than 200/3xx/404/410.
   - `"#rrggbb"` is a colour.
4. Each non-None result is stored on `books.spine_color`. That includes `""`, which settles the book for good. Colours (not `""`) are added to `colors[book.id]`.
5. `remaining = 0` if **any** result in the batch was None, otherwise `len(pending) - len(batch)`. This stops clients polling while the cover host is down; the next page visit tries again.

**Response 200** `SpineColorsOut`:
```json
{"colors": {"<book id as JSON string key>": "#rrggbb"}, "remaining": 0}
```
The frontend loops while `remaining > 0`.

**Outbound HTTP:** `fetch_cover(url)` (§2.8). The URL is `spine_cover_url(book)`: the BookOut `cover_url`, with `-L.jpg` replaced by `-M.jpg` when it contains `covers.openlibrary.org`. Example: `https://covers.openlibrary.org/b/id/111-M.jpg?default=false` **(test)**.

**Side effects:** writes `books.spine_color`. This is **global** catalog data shared by every user.

---

### 1.7 `PATCH /api/shelf/{entry_id}`

Updates one of the caller's shelf entries.

**Path:** `entry_id` int.

**Body** `ShelfPatchIn`. The distinction between **absent** and **null** matters (Pydantic `model_fields_set`).

| field | type | validation | absent | explicit null |
|---|---|---|---|---|
| `status` | enum \| null | as in 1.2 | no change | **same as absent** |
| `position` | int \| null | ≥0 (`"Input should be greater than or equal to 0"`) | no change | same as absent |
| `shelf_position` | int \| null | ≥0 | no change | same as absent |
| `rating` | int \| null | 1..5 | keep the current value | **clear** it (null) |
| `take` | string \| null | trimmed; >140 chars gives `"Keep the take to 140 characters"` | keep | clear (`""`) |
| `dnf_reason` | string \| null | trimmed; >200 chars gives `"Keep the reason to 200 characters"` | keep | clear (`""`) |
| `progress` | int \| null | 0..100 | keep | clear (null) |
| `finished_on` | date `YYYY-MM-DD` \| null | must be a valid date; `"invalid"` gives 400 | no change | **clear `finished_at`** |

**Processing order** (exact):
1. If `status`, `position` and `shelf_position` are all null or absent, **and** none of `rating`, `take`, `dnf_reason`, `progress`, `finished_on` is *present*: **400** `"Nothing to update"`. This check runs **before** the ownership check, so `{}` against someone else's or a missing entry gives 400, not 404 **(probed)**.
2. Load the entry. If it is missing or `user_id != caller`: **404** `"Book not on your shelf"`.
3. `previous_status`, `previous_progress` = current values. `status` = payload status or the current one. `position` = payload position or the current one.
4. If `finished_on` is present:
   - If the resulting `status != finished`: **400** `"Reading date requires a finished book"`. The **resulting** status counts, so `{"status": "finished", "finished_on": …}` on a `want_to_read` entry is allowed.
   - If `finished_on` is non-null and later than **today in the instance zone** (`datetime.now(zone).date()`): **400** `"Reading date cannot be in the future"`. Today itself is allowed.
   - These checks come before any mutation, so the entry is unchanged on error **(test)**.
5. If `status` is present or any of `rating`/`take`/`dnf_reason` is present: `apply_finish_fields(entry, status, rating = payload if present else current, take = …, dnf_reason = …)` (§2.3).
6. If `status` is present or `progress` is present: `apply_progress(entry, status, progress = payload if present else current)`.
7. If `shelf_position` is not null: `place_on_shelf(entry, shelf_position)` (§2.5).
8. If `shelf_position` is null, **or** status is not null, **or** position is not null: `place_item(entry, status, position, stamp_dates = status != previous_status)`.
   - This means **every PATCH except a `shelf_position`-only one** (`shelf_position` possibly combined with rating/take/dnf_reason/progress/finished_on, but without status or position) re-sequences the column and **bumps `updated_at`**. A rating-only PATCH bumps `updated_at` **(probed)**. A `shelf_position`-only PATCH leaves `updated_at` and all reading metadata untouched **(test)**.
   - **Phase 1:** a PATCH with only the new `rating_visibility` skips `place_item` too, so it leaves the column and `updated_at` as they are (`docs/phase1/notes/fup.md` decision 8).
   - Dates are stamped only when the status actually changed. A same-status PATCH never re-stamps, so a cleared `finished_at` stays null **(test)**.
9. If `finished_on` is present:
   - `finished_at` = `datetime.combine(finished_on, 12:00, tz=zone)` converted to UTC, or null. Noon local time in the instance zone; for example Berlin 2025-08-01 gives `2025-08-01T10:00:00Z`, and Kiritimati (UTC+14) 2025-07-01 gives `2025-06-30T22:00:00Z`.
   - Then, if `started_at > finished_at`, set `started_at = finished_at`. A completion entered after the fact must not precede the start.
   - This runs **after** `place_item`, so it overrides any stamp.
10. Events:
    - If status changed: `_status_event(added=False)`: `shelf_finished` `{rating?, take?}`, `shelf_dnf` `{reason?}`, or `shelf_moved` `{status}`.
    - Else, if status is `currently_reading`, `progress` was present, the new progress is not null and it differs from `previous_progress`: a `progress` event `{progress}`.
    - Rating or take edits on an already-finished book produce **no** event.
11. Commit, reload, and return 200 `ShelfItemOut`. If the reload is missing: 404 `"Book not on your shelf"`.

**Errors:** 400 for the validation strings above, 400 `"Nothing to update"`, 400 for the reading-date strings, 404 `"Book not on your shelf"`, 401.

---

### 1.8 `DELETE /api/shelf/{entry_id}`

Removes one of the caller's shelf entries. Status code **204**, empty body.

1. Load the entry. If it is missing or not the caller's: **404** `"Book not on your shelf"`.
2. Record a `shelf_removed` event `{status}` with `book_id`. This happens before the delete.
3. If the caller has a stored layout: remove `entry_id` from every row (row boundaries kept) and pop `spines[str(entry_id)]`, then write the layout back **in the same transaction**. SQLite can reuse the highest deleted id, so a stale layout would otherwise attach to the next new book.
4. Delete the entry, then renumber the remaining entries of the **same status** to 0..n-1 (`ORDER BY position, id`).
5. `shelf_position` is **not** compacted, so gaps can appear. Favourites are **not** touched (they're keyed by book).

---

### 1.9 `GET /api/auth/me/favorites`

Returns the caller's favourites.

**Response 200** `FavoritesOut`: `{"items": [FavoriteOut]}`, ordered by `position, id`. Rows with a missing book are skipped.

---

### 1.10 `PUT /api/auth/me/favorites`

Replaces the caller's favourites with an ordered list.

**Body** `FavoritesIn`: `{"book_ids": [int]}`.
- The default is `[]`, so an omitted key **clears** the favourites **(test)**.
- `null` gives 400 `"Input should be a valid list"`.

Validator, in order. The first failure wins, and these are custom `ValueError`s, so the strings are verbatim. There is **no** Pydantic `max_length`, so the length message is translatable **(test)**.
1. `len > 3`: `"At most 3 favourites"`
2. Duplicates: `"Each book can only be a favourite once"`
3. Any id ≤ 0: `"That book is not in the club yet"`

**Processing** (`replace_favorites`):
1. Length check again (unreachable): 400 `"At most 3 favourites"`.
2. `_require_club_books`: every id must exist in `books`, else **400** `"That book is not in the club yet"`. Nothing is changed. The book does **not** need to be on the caller's shelf.
3. Delete all of the caller's favourites, flush, insert positions 1..n in list order, then commit.

**Response 200** `FavoritesOut`.

---

### 1.11 `POST /api/auth/me/favorites/{book_id}`

Appends one book to the caller's favourites.

**Path:** `book_id` int.
1. `_require_club_books([book_id])`: an unknown id (including 0 and negative) gives **400** `"That book is not in the club yet"` **(probed)**.
2. If the book is already a favourite: return the current list with **200**. This is idempotent.
3. If the caller already has 3 or more: **409** `"You already have 3 favourites"`.
4. Insert at `position = len(current) + 1` and commit.
   - On `IntegrityError` (a concurrent add or a stale read): roll back and re-list. If the book is now present, return 200 with the list; otherwise **409** `"You already have 3 favourites"`.
   - Concurrent adds for the last slot give exactly one 200 and one 409, never a 500 **(test)**.

**Response 200** `FavoritesOut`.

---

### 1.12 `DELETE /api/auth/me/favorites/{book_id}`

Removes one book from the caller's favourites.

- If the book isn't among the caller's favourites: **200** with the current list. There is no 404 and **no** check that the book exists **(probed)**.
- Otherwise call `replace_favorites` with the remaining ids, which **compacts positions** to 1..n.

**Response 200** `FavoritesOut`.

---

## 2. Domain rules

### 2.1 `upsert_book` (shared with the pick, vote, diary, quotes and books routers)
- Key: `work_key(work_id_from_key(ol_work_key))`.
- `cover_id`: values ≤0 or null become null.
- `cover_image_url`: only storable Google URLs survive.
- **New book:** insert with title, authors, cover_id, cover_image_url and year.
- **Existing book:**
  - `title` is **always overwritten** with the caller's title.
  - `authors` is overwritten only when non-empty **(test: shelving without authors keeps them)**.
  - Cover fields are overwritten only when not null.
  - `year` is overwritten only when not null.
- `_set_cover`: if `(cover_id, cover_image_url)` changed, reset `spine_color = NULL` so the colour is sampled again. `apply_book_details` (catalog refresh) does the same **(test: a new cover is sampled again)**.

### 2.2 Shelf status
- Enum `ShelfStatus`, declared in this order: `want_to_read`, `currently_reading`, `finished`, `did_not_finish`.
- The declaration order is used by `place_on_shelf` for unarranged rows, and by the frontend's `STATUSES.indexOf`.
- The alphabetical text order is used by `GET /api/shelf` and by other routers' `order_by(ShelfEntry.status)`.
- Any status can move to any other status; there is no state machine beyond §2.3 and §2.4.

### 2.3 Finish fields (`apply_finish_fields`)

| status | rating | take | dnf_reason |
|---|---|---|---|
| `finished` | the given rating (may be null) | `(take or "").strip()` | `""` |
| `did_not_finish` | null | `""` | `(dnf_reason or "").strip()` |
| `want_to_read` / `currently_reading` | null | `""` | `""` |

Consequences:
- Moving off `finished` clears the rating and take **(test)**.
- A rating sent with a non-finished status is silently ignored.
- `progress` (`apply_progress`) is kept only for `currently_reading` and set to null otherwise, so moving `currently_reading` → `finished` clears it **(test)**.

### 2.4 Reading dates (`apply_reading_dates`)

Called through `place_item` when `stamp_dates` is true. `changed` = source status ≠ target status.

| target status | rule |
|---|---|
| `want_to_read` | if `changed`: clear `started_at` and `finished_at`; otherwise leave them |
| any other status | if `started_at` is null, set it to now |
| `finished` | if `changed` or `finished_at` is null, set `finished_at` to now |
| `currently_reading` / `did_not_finish` | set `finished_at` to null |

Consequences:
- A re-read (`finished` → `currently_reading` → `finished`) keeps the original `started_at` and gets a **new** `finished_at`. There is no second row **(test)**.
- POST and import always stamp, with `changed=False`. PATCH stamps only when the status changed.
- A `finished` entry can legitimately have `finished_at = null` after `finished_on: null`. It stays null across same-status PATCHes **(test)**.

### 2.5 Ordering

There are two independent orders.

- **`position` (per-status column order).**
  - 0-based, contiguous per (user, status). Ties are broken by `id`.
  - `place_item(item, status, position)` takes the siblings in the target status (excluding the item) ordered by `(position, id)`, inserts the item at `clamp(position, 0, len(siblings))`, and renumbers 0..n.
  - If the status changed, the source column is renumbered too.
  - It also sets `item.status` and `item.updated_at = now`. Siblings' `updated_at` is untouched.
  - New and imported entries go to position 0, the top.
- **`shelf_position` (manual order across all statuses; private to the owner, but visible in the owner's shelf JSON to other viewers).**
  - `place_on_shelf(item, p)` loads **all** the user's entries and sorts them by:
    `(shelf_position IS NOT NULL, shelf_position if set else status_declaration_index, position, id)`
  - So unarranged entries come first in enum order (want, reading, finished, DNF), then arranged entries by `shelf_position`.
  - Then it removes the item, inserts it at `min(p, len)`, and **renumbers every entry 0..n-1**. After the first manual move, all of the user's entries have a `shelf_position`.
  - Entries added later have null and sort first.
  - A status change keeps `shelf_position` **(test)**.
  - It never touches status, dates, reviews, position or `updated_at`.
  - The frontend `orderedShelf` reproduces this sort.

### 2.6 Activity events written by this area (table `events`)

Every event has `actor_id` = caller, `book_id` = the book, and `payload` = a JSON object with **null values dropped**, so a finish without a rating stores `{}`.

| kind | when | payload |
|---|---|---|
| `shelf_added` | POST, status `want_to_read` or `currently_reading` | `{"status": s}` |
| `shelf_finished` | POST or status-changing PATCH to `finished` | `{"rating": r?, "take": t?}`; `take` only if non-empty |
| `shelf_dnf` | POST or status-changing PATCH to `did_not_finish` | `{"reason": r?}`; only if non-empty |
| `shelf_moved` | status-changing PATCH to `want_to_read` or `currently_reading` | `{"status": s}` |
| `progress` | same-status PATCH on `currently_reading` where progress was present, is non-null and changed | `{"progress": p}` |
| `shelf_removed` | DELETE | `{"status": s}`, the status before deletion |

The Goodreads import, spine colours, favourites and the physical layout record **no** events. The feed (`GET /api/activity`) is a different area.

### 2.7 Goodreads parsing and lookup

Constants:

| name | value |
|---|---|
| `MAX_IMPORT_BYTES` | 1,500,000 |
| `MAX_IMPORT_ROWS` | 400 |
| `SKIP_LIST_LIMIT` | 25 |
| `LOOKUP_CONCURRENCY` | 2 |

Parsing (`parse_goodreads_csv`):
1. Decode with `utf-8-sig`, which strips a leading BOM.
2. `csv.DictReader` with the Python `excel` dialect.
   - An unquoted field such as `="0316556343"` keeps its quotes literally.
   - Blank lines are skipped and don't count as rows.
   - Short rows fill the missing keys with None.
   - Long rows put the extra cells under key `None`. **Python bug:** `_cell()` then crashes with a 500 (`AttributeError: 'NoneType' object has no attribute 'lower'`) whenever it falls back to case-insensitive lookup, e.g. when the file has no `ISBN13` column **(probed)**. Do not port this; ignore extra cells.
3. Header check: the trimmed, lowercased header set must contain `exclusive shelf` and `title`.
4. Cell lookup `_cell(row, name)`: an exact key first, then a case-insensitive match on the **untrimmed** key. The value is `str(value).strip()`, and missing or None gives `""`.
5. Rows are enumerated after blank lines are dropped. At index ≥ 400, add **one** skip `("", "Stopped after 400 rows")` and stop. Everything left is dropped uncounted, so 405 rows give imported 400 and skipped 1 **(probed)**.
6. Per row:
   - `title = Title`, `authors = Author` (only that column; *Additional Authors* is ignored).
   - `shelf = Exclusive Shelf`, lowercased.
   - `isbn = clean_isbn(ISBN13) or clean_isbn(ISBN)`.
   - An empty title gives skip `("Untitled", "No title")`.
   - The shelf is mapped as follows; any other value (e.g. `dnf`, custom exclusive shelves) gives skip `(title, "Unknown shelf")`.

     | Goodreads shelf | status |
     |---|---|
     | `to-read` | `want_to_read` |
     | `currently-reading` | `currently_reading` |
     | `read` | `finished` |

   - Ignored columns: My Rating, Date Read, Date Added, My Review, Read Count, Owned Copies, Bookshelves.

`clean_isbn(value)`:
1. Trim, then strip `"` and `'` characters from both ends.
2. If the value starts with `=`, drop the `=` and trim and strip quotes again.
3. Remove every character that isn't `[0-9Xx]`.
4. Return the result uppercased if its length is 10 or 13, else `""`.

There is no checksum validation, and an `X` is accepted at any position.

Examples **(test)**:

| input | output |
|---|---|
| `="9780316556347"` | `9780316556347` |
| `="0316556343"` | `0316556343` |
| `""` | `""` |
| `=""` | `""` |

`lookup_work(isbn, title, authors)`:
1. `queries = ["isbn:<isbn>"]` if there is an ISBN.
2. Then `" ".join(non-empty of (title, authors)).strip()`, appended if it isn't already in the list.
3. Run the queries in order and return the first hit.
4. OL path: the first item of `map_open_library_docs(payload["docs"])`. Docs are skipped when their key doesn't start with `/works/` or their title is empty.
   - Authors are joined with `", "`.
   - `cover_i` ≤0 or invalid becomes null.
   - `first_publish_year` goes through `int()`, so a string `"1999"` becomes 1999 **(probed)**.
5. Any `httpx.HTTPError`, `ValueError` or `TypeError` gives "no hit", so an **OL outage turns every row into `No match`**, not an error.
6. Google path: the first item of `search_volumes`. A `GoogleBooksError` gives `"Catalog unavailable"`.

### 2.8 Spine colour sampling

`fetch_cover(url)`:
- Overall deadline 15 s; when it expires, raise `httpx.TimeoutException`, which counts as retry later.
- Per-request timeout 8 s.
- Redirects are followed **manually**: at most 3 redirects, so up to 4 requests.
- **Every hop** is checked with `_fetchable` before it is requested. A disallowed hop returns None, which settles the book as `""`.
- `_fetchable(url)`: scheme must be `https`, **no explicit port**, no username or password, and the host (lowercased) is one of:
  - `covers.openlibrary.org` or `*.covers.openlibrary.org`
  - `books.google.com` or `*.books.google.com`
  - `googleusercontent.com` or `*.googleusercontent.com`
  - `archive.org` or `*.archive.org`

  Rejected **(test)**: `http://…`, an explicit `:8443`, `user@…`, `127.0.0.1`, `example.com`, `archive.org.example.com`.
- Response handling:

  | response | result |
  |---|---|
  | 3xx with `Location` | `urljoin` and continue |
  | 404 or 410 | None |
  | any other status ≠ 200 | raise `HTTPStatusError` (retry later) |
  | 200 | stream the body; return None as soon as it exceeds 1,500,000 bytes |

  If the loop is exhausted: None.
- Request header: the OL `User-Agent`.

`dominant_color(data)` returns `"#rrggbb"` or None:
1. Return None if `data` is empty or longer than 1,500,000 bytes.
2. Decode the image (Pillow: any format). Return None if `min(w,h) < 8` (Open Library's 1×1 "no cover" pixel) or `w*h > 4,000,000`. Decode errors give None.
3. Convert to RGB.
4. Crop an 8% inset: `ix = int(w*0.08)`, `iy = int(h*0.08)`, box `(ix, iy, w-ix, h-iy)`. This drops publisher borders and scan edges.
5. `thumbnail((48,48))`: fit within 48×48, keep the aspect ratio, never upscale.
6. Quantize to **6 colours** with **median cut**.
7. For each palette colour with pixel count `count`: `S` is the HSV saturation of the 0..1 RGB, and `score = (count/total) * (0.35 + S)`. The highest score wins, and the first one wins a tie (strict `>`). Area decides, but a vivid colour beats an equally large grey.
8. Convert to HLS, clamp lightness to `[0.17, 0.86]`, and convert back to RGB.
9. Format as `"#%02x%02x%02x"` with `round(c*255)`, lowercase. Python's `round` is banker's rounding.

### 2.9 Caches and rate limits

- This area has no in-process caches.
- There are no rate limits, apart from the import's Google cooldown check.
- The OL search client used by the import does **not** go through the books router's search cache or in-flight dedupe.

---

## 3. Data

### `shelf` (ShelfEntry): personal

| column | type | semantics |
|---|---|---|
| `id` | INTEGER PK | Referenced by `physical_shelf_layouts.layout_json` (rows and spine keys). SQLite may **reuse** the highest deleted id. |
| `user_id` | FK users.id, index | No ON DELETE CASCADE. |
| `book_id` | FK books.id | `UNIQUE(user_id, book_id)` (`uq_shelf_user_book`). |
| `status` | VARCHAR(17), index | Stores enum values as text. |
| `position` | INTEGER NOT NULL, default 0 | 0-based within (user, status). |
| `shelf_position` | INTEGER NULL | Manual cross-status order; NULL means not arranged since the last renumber. **Migration column**, NULL for old rows. |
| `rating` | INTEGER NULL | 1–5 (not enforced in the DB). Only set for `finished`. **Migration column.** |
| `take` | VARCHAR(140) | `""` means none. Fresh DBs: NOT NULL. **Migrated DBs: `VARCHAR DEFAULT ''`, nullable.** The importer should COALESCE to `''`. |
| `dnf_reason` | VARCHAR(200) | Same nullability quirk as `take`. |
| `progress` | INTEGER NULL | Percent 0–100, only for `currently_reading`. NULL means not tracked. **Migration column.** |
| `updated_at` | DATETIME NOT NULL | Naive UTC, stored as `"YYYY-MM-DD HH:MM:SS.ffffff"` **(probed)**. |
| `started_at` | DATETIME NULL | NULL for `want_to_read`, **and for legacy rows that predate the column** whatever their status. |
| `finished_at` | DATETIME NULL | Only meaningful for `finished`. NULL if the date was cleared or the row is legacy. |

### `books` (Book): global catalog

Columns this area reads or writes:

| column | semantics |
|---|---|
| `id` | |
| `ol_work_key` | UNIQUE; `/works/<id>`; the prefix gives the source: `OL…W` Open Library, `ISBN…` Google with ISBN, `GB…` Google volume, `BC…` club-typed |
| `title`, `authors`, `cover_id`, `year` | written by `upsert_book` |
| `cover_image_url` | Google URL only, ≤512 chars |
| `spine_color` | VARCHAR(7). **NULL** = not sampled yet; **`""`** = settled, no usable cover; **`"#rrggbb"`** = colour. Reset to NULL when the cover changes. Migration column. |
| `pages` | int or NULL; read for BookOut and spine width |

`description`, `subjects`, `ol_rating` and `details_fetched_at` are not used here, except that `apply_book_details` resets `spine_color`.

### `physical_shelf_layouts` (UserPhysicalShelfLayout): personal

| column | semantics |
|---|---|
| `user_id` | PK, FK users ON DELETE CASCADE |
| `layout_json` | VARCHAR NOT NULL: compact JSON as dumped by `PhysicalShelfLayout`. The model default `'{"rows":[[],[],[]],"material":"oak","spines":{},"decorations":{}}'` is never actually written by the API. |

**Legacy layouts** may lack `decorations`, `lights` and spine `display`. In that case `color` and `width` may both be present (older format) or absent. They must still load, with defaults filled and missing books pruned **(test)**.

### `user_favorites` (UserFavorite): personal

| column | semantics |
|---|---|
| `id` | PK |
| `user_id` | FK users ON DELETE CASCADE, index |
| `book_id` | FK books ON DELETE CASCADE, index |
| `position` | 1..3 |

Constraints:
- `UNIQUE(user_id, book_id)` (`uq_favorite_user_book`)
- `UNIQUE(user_id, position)` (`uq_favorite_user_position`)
- `CHECK (position >= 1 AND position <= 3)` (`ck_favorite_position`)
- The ORM relationships `User.favorites` and `Book.favorites` have cascade delete and delete-orphan **(test)**.
- `FAVORITE_LIMIT = 3`.

A book cascade-delete can leave a **gap** (e.g. positions 1 and 3). `add_favorite` then tries `len+1 = 3`, which hits the unique constraint and returns 409 even though only 2 favourites exist. This is a latent bug.

### `events`: written only (§2.6)

`actor_id`, `kind` (VARCHAR(32)), `book_id`, `pick_id` (always null here), `payload` (JSON text, `ensure_ascii=False`), `created_at`.

### `users`: read only

`id`, `username` (lowercase, unique), `theme`, `color_mode`, `locale`, `avatar_updated_at` (for `UserOut`).

### Migrations in `db.py` that touch this area

`COLUMN_MIGRATIONS` adds:
- On `shelf`: `shelf_position INTEGER`, `rating INTEGER`, `take VARCHAR DEFAULT ''`, `dnf_reason VARCHAR DEFAULT ''`, `progress INTEGER`, `started_at DATETIME`, `finished_at DATETIME`.
- On `books`: `description`, `pages`, `subjects`, `ol_rating`, `details_fetched_at`, `cover_image_url VARCHAR`, `spine_color VARCHAR`.

No data migration touches shelf, layout or favourites (`import_legacy_pick_posts` only touches posts). `test_migrations.py::test_legacy_database_is_upgraded_in_place` (another area) asserts the shelf columns exist and that `shelf_position` is NULL for an old row.

### Other areas that read `shelf`

They must apply the new visibility policy and club filtering:
- members (currently-reading preview)
- overlap
- stats (finished and DNF per year, pick stats)
- pick (readers, viewer entry)
- vote (want-to-read suggestions)
- diary (`my_progress` / `my_status`, `progress_at` snapshots)
- books (`on_shelf` / `shelf_id` on search hits and details, `readers`, `club_rating`, `favorite_position` via `favorite_position_for`)

---

## 4. Tests: parity checklist

There are 71 test functions; parametrised cases are listed separately, for 107 cases in total. "Pick" and "book detail" tests also touch other areas, as noted.

### `backend/tests/test_shelf.py` (16)
- `test_shelf.py::test_shelving_without_authors_does_not_clear_existing_authors`: ada shelves OL1W with authors. grace shelves the same work with `authors=""` (201) and the response's book authors are still "Madeline Miller". ada's shelf (viewed by grace) shows the same.
- `test_shelf.py::test_add_move_remove_and_unique`:
  - POST returns 201 with status `want_to_read`, position 0 and the book title.
  - A duplicate POST returns 409, and `json.item.id` is the existing id.
  - PATCH status `currently_reading` returns 200.
  - GET shows 1 item in `currently_reading`.
  - DELETE returns 204, and the shelf is then empty.
- `test_shelf.py::test_cannot_edit_someone_elses_row`: grace PATCHes and DELETEs ada's entry (404 for both). grace can GET `?username=ada` (200, 1 item).
- `test_shelf.py::test_reorder_within_and_across_columns`:
  - The newest add is at the top (`["Song of Achilles", "Circe"]`, positions 0 and 1).
  - PATCH `{position: 0, status: want_to_read}` on Circe reorders the column.
  - Moving Song to `currently_reading` at position 0 leaves want = [Circe @0] and reading = [Song].
- `test_shelf.py::test_unknown_member_shelf_is_404`: `GET /api/shelf?username=nobody` returns 404.
- `test_shelf.py::test_finish_rating_and_take_show_on_friend_shelf`: PATCH to finished with rating 4 and take `"  Witchy and sad  "` gives a trimmed take, rating 4 and `dnf_reason ""`. grace sees the rating and take on ada's shelf.
- `test_shelf.py::test_dnf_reason_shows_on_friend_shelf`: PATCH to DNF with reason `"  Too grim  "` gives a trimmed reason, rating null and take `""`. grace sees the reason and a null rating.
- `test_shelf.py::test_dnf_reason_is_optional`: PATCH to DNF without a reason returns 200 with `dnf_reason ""`.
- `test_shelf.py::test_moving_off_finished_clears_notes`: added as finished with rating 5 and take; moving to `currently_reading` clears the rating, take and `dnf_reason`.
- `test_shelf.py::test_finish_notes_appear_on_club_pick`: with the pick set to the book, finishing it with rating 5 and take puts `pick.finished == ["ada"]`, and the reader row has rating 5 and the take. *(pick area)*
- `test_shelf.py::test_reading_progress_shows_on_friend_shelf_and_pick`: a `currently_reading` entry PATCHed with progress 35 returns 200 with status unchanged. `pick.reading == ["ada"]` and the reader's progress is 35. grace sees progress 35 on ada's shelf and on the pick. *(pick area)*
- `test_shelf.py::test_progress_is_optional_and_clears_when_leaving_reading`: an added `currently_reading` entry has null progress. PATCH progress 80, then status finished, leaves progress null.
- `test_shelf.py::test_invalid_progress`: PATCH progress −1 returns 400; progress 101 returns 400.
- `test_shelf.py::test_invalid_finish_fields`: PATCH finished with rating 0 returns 400, rating 6 returns 400, a 141-char take returns 400, and DNF with a 201-char reason returns 400.
- `test_shelf.py::test_free_shelf_order_persists_without_changing_reading_metadata`:
  - Want, reading (35%) and finished (5, take, `finished_on` 2025-04-12).
  - PATCH `shelf_position: 0` on finished gives the order [finished, want, reading]. status, position, rating, take, progress, `started_at`, `finished_at` and `updated_at` are unchanged for every row.
  - `shelf_position: 99` clamps, giving [want, reading, finished] with positions [0, 1, 2].
  - `shelf_position: -1` returns 400.
- `test_shelf.py::test_free_order_is_private_and_survives_status_changes_and_new_books`:
  - Moving the finished entry to `shelf_position` 0 makes the want entry 1.
  - Changing the want entry's status keeps `shelf_position` 1.
  - A new add has `shelf_position` null. PATCH it to 2 gives [second, first, newest].
  - grace's PATCH of ada's `shelf_position` returns 404, and ada's shelf viewed by grace equals `before` exactly.

### `backend/tests/test_import.py` (8)
- `test_import.py::test_clean_isbn_unwraps_goodreads_formula`: `="9780316556347"` gives `9780316556347`, `="0316556343"` gives `0316556343`, `""` gives `""`, and `=""` gives `""`.
- `test_import.py::test_parse_fixture_maps_shelves_and_skips`: parsing the fixture `tests/fixtures/goodreads.csv` gives these rows:

  | title | status | isbn |
  |---|---|---|
  | Circe | `want_to_read` | 9780316556347 |
  | The Song of Achilles | `currently_reading` | `""` |
  | Unmatched Novel | `finished` | 9780000000002 |

  Skips: "Custom Shelf Book" with "Unknown shelf", and "Untitled" with "No title".
- `test_import.py::test_import_requires_auth`: an unauthenticated upload returns 401.
- `test_import.py::test_import_matches_isbn_and_title`:
  - With faked OL, the response is 200 with `imported 2` and `skipped 3`.
  - Skip titles: {Unmatched Novel, Custom Shelf Book, Untitled}, and one reason is "No match".
  - The `isbn:9780316556347` query was sent, and a query containing "Song of Achilles" was sent.
  - The shelf has Circe (`want_to_read`, `/works/OL1W`) and The Song of Achilles (`currently_reading`, `/works/OL2W`).
- `test_import.py::test_import_skips_books_already_on_shelf`: with Circe already shelved as finished, the import gives `imported 1` and the skip `{"title": "Circe", "reason": "Already on your shelf"}`. Circe stays finished.
- `test_import.py::test_import_stays_on_signed_in_shelf`: grace imports (`imported 2`), her shelf has 2 items, and ada's shelf is empty.
- `test_import.py::test_import_rejects_non_goodreads_csv`: `hello,world\n1,2\n` returns 400, and the detail contains "Goodreads".
- `test_import.py::test_import_rejects_oversized_file`: a header line plus 1,500,001 `x` bytes returns 400, and the detail contains "too large".

### `backend/tests/test_spine_color.py` (9)
- `test_spine_color.py::test_dominant_color_reads_the_cover_and_ignores_its_border`: a (30, 90, 160) JPEG with a white border gives blue > green > red and |blue − 160| < 20.
- `test_spine_color.py::test_dominant_color_prefers_a_vivid_colour_over_a_slightly_larger_pale_one`: a PNG that is 120×180 light grey (205, 205, 200) with the top 80 px deep green (20, 95, 40) gives green > red+40 and green > blue+30.
- `test_spine_color.py::test_dominant_color_keeps_extremes_printable_and_rejects_junk`:
  - Pure black gives R > 30; pure white gives R < 235.
  - `b""` gives None, text bytes give None, and a 1×1 JPEG gives None.
- `test_spine_color.py::test_spine_colors_are_sampled_once_and_served_with_the_book`:
  - Book 1 has `cover_id` 111 and book 2 has no cover. Before sampling, the book's `spine_color` is null.
  - POST returns `remaining 0`, `colors` keys = [book1 id], and exactly one fetch of `https://covers.openlibrary.org/b/id/111-M.jpg?default=false`.
  - The shelf then shows the colour on book 1 and null on book 2 (`""` is emitted as null).
  - A second POST returns `{"colors": {}, "remaining": 0}` with no new fetch.
- `test_spine_color.py::test_spine_colors_retry_after_network_trouble_but_not_after_a_missing_cover`:
  - When the fetch raises `httpx.ConnectError`, the response is `{"colors": {}, "remaining": 0}` and the DB `spine_color` stays NULL.
  - When the fetch returns None, the response is the same and the DB `spine_color` becomes `""`.
- `test_spine_color.py::test_a_new_cover_is_sampled_again`: after the first sampling, `apply_book_details(book, cover_id=222)` resets the colour. The next POST returns a non-empty, different colours map.
- `test_spine_color.py::test_spine_colors_only_cover_the_callers_shelf_and_need_a_login`: unauthenticated POST returns 401. grace's POST never fetches ada's books and returns `{"colors": {}, "remaining": 0}`.
- `test_spine_color.py::test_only_cover_hosts_are_ever_requested`: `_fetchable` allows the OL covers, `ia800100.us.archive.org` and `books.google.com` URLs. It rejects http, an explicit port, userinfo, an IP, `example.com` and `archive.org.example.com`.
- `test_spine_color.py::test_one_broken_cover_does_not_block_the_others`: the fetch raises `ValueError` for 111 and returns an image for 222. The response is 200 with colours only for 222, and the next POST returns `{"colors": {}, "remaining": 0}` because the broken one is settled.

### `backend/tests/test_physical_shelf.py` (14 functions, 35 + 3 parametrised cases)
- `test_physical_shelf.py::test_layout_requires_authentication`: GET and PUT without a session return 401.
- `test_physical_shelf.py::test_layout_persists_on_reload_and_restart_without_changing_reading_data`:
  - The default GET equals the default layout.
  - A PUT with decorations, mixed-case colours and a spine `{color, width: 37}` returns 200 with colours lowercased and defaults filled.
  - GET equals what was saved, and the shelf items are unchanged.
  - A new app instance on the same DB (after logging in again) GETs the same layout. A replacement PUT there is visible from the original client.
  - The DB row exists.
- `test_physical_shelf.py::test_members_can_view_a_layout_but_only_its_owner_can_change_it`:
  - grace sees ada's layout via `?username=ada` and `?username=ADA`. `?username=nobody` returns 404.
  - grace's own GET returns the default.
  - grace PUTting ada's layout returns 400. A row with grace's **book id** (which is ada's entry id) returns 400. A spine for ada's entry returns 400.
  - grace's own layout saves (200). Each user then GETs their own layout.
- `test_physical_shelf.py::test_status_changes_do_not_move_books_and_deleted_entries_are_pruned`:
  - A status PATCH leaves the layout identical.
  - A new add doesn't appear in the rows.
  - Deleting an entry removes it from its row (boundaries kept) and drops its spine.
- `test_physical_shelf.py::test_deleted_spine_does_not_leak_to_a_book_with_a_reused_id`: an entry with a spine is deleted and a new add **reuses the same id** (SQLite). GET then returns rows `[[], []]` with no spines. *(The Postgres port can't reproduce the id reuse; see §6.)*
- `test_physical_shelf.py::test_get_prunes_entries_removed_outside_the_shelf_endpoint`: the entry is deleted directly in the DB. GET drops it from the rows and spines and keeps the decorations and three rows.
- `test_physical_shelf.py::test_invalid_layout_does_not_replace_previous_layout`: each case below gets **400**, and the previously saved layout `{rows: [[id]], material: walnut}` is unchanged. The cases:
  1. `{"rows": []}`
  2. 13 rows
  3. `[[1],[1]]` (duplicate)
  4. a single row of 1..2001 (2001 books)
  5. two rows totalling 2001 books
  6. `[[0]]`
  7. `[[true]]`
  8. `[["1"]]`
  9. `[[1.5]]`
  10. `[[9999]]` (not owned)
  11. `material: "pine"`
  12. spine color `"red"`
  13. spine color `"#abcd"`
  14. spine width 25
  15. spine width 65
  16. spine width 30.5
  17. spine key `"01"`
  18. spine key `"foo"`
  19. spine key `"9999"` (not owned)
  20. a 5000-digit spine key
  21. rows `["decor:missing"]` with no decoration
  22. an orphan decoration not in the rows
  23. a decoration placed in two rows
  24. a row decor id that doesn't match the decoration key
  25. `"decor:"` with an empty suffix
  26. `"plant"` without the prefix
  27. `"decor:space name"`
  28. `"decor:"` + 65 chars
  29. decoration kind `"unknown"`
  30. decoration color `"red"`
  31. decoration color `"#abcd"`
  32. decoration without color
  33. decoration without kind
  34. `decorations: []`
  35. 101 decorations
- `test_physical_shelf.py::test_legacy_stored_layout_still_loads_and_prunes_missing_books`:
  - A raw legacy JSON (no decorations or lights, and a spine for a missing id 9999) loads with 9999 pruned from the rows and spines.
  - After deleting the entry, GET returns the default rows with `material: walnut`.
- `test_physical_shelf.py::test_decorations_can_be_saved_without_books_and_removed`: a decoration id of max length (`decor:` + 64 chars from `[A-Za-z0-9_-]`) saves and reads back. A PUT of the default then clears it.
- `test_physical_shelf.py::test_mixed_layout_limits_books_and_decorations_separately`: schema-level checks.
  - 2000 books plus 100 decorations validate.
  - 2001 books give "At most 2000 books".
  - 2001 books plus 100 decorations give "At most 2100 items". The 2100 check runs before the books check.
- `test_physical_shelf.py::test_shelf_book_exposes_known_page_count`: a new entry has `book.pages` null. After setting `pages=480` in the DB, the shelf shows 480.
- `test_physical_shelf.py::test_spines_can_follow_the_book_and_choose_how_it_stands`: material black and `lights: true`, with these spines:
  - `{display: flat}` comes back as `{display: flat, color: null, width: null}`.
  - `{display: cover, color: "#ABCDEF"}` comes back lowercased with width null.
  - `{width: 40}` comes back as `{display: spine, color: null, width: 40}`.

  The PUT response and the GET both equal the expected layout.
- `test_physical_shelf.py::test_new_layout_options_are_validated`: each case gets 400, all with `rows: [[1]]`.
  1. spine display `"upside-down"`
  2. `lights: "sometimes"`
  3. `material: "pine"`
- `test_physical_shelf.py::test_new_decorations_are_accepted`: `bookend` and `globe` decorations save and echo back.

### `backend/tests/test_favorites.py` (12)
- `test_favorites.py::test_replace_order_clear_and_eligibility`:
  - The empty GET returns `{"items": []}` and the shelf favourites are `[]`.
  - PUT [song, circe] gives that order with positions [1, 2]. PUT [circe, galatea, song] reorders to positions [1, 2, 3].
  - `/api/shelf` favourites follow the same order, and each book has `cover_url`.
  - Book detail `favorite_position` is 1 for Circe, 3 for Song and 2 for Galatea.
  - PUT [] clears everything, and detail `favorite_position` becomes null. *(books area for detail)*
- `test_favorites.py::test_unknown_and_duplicate_book_ids_are_rejected`:
  - `[circe, 9999]` returns 400 with "not in the club" and leaves nothing saved.
  - `[circe, circe]` returns 400 with "once".
  - 4 ids return 400 with exactly `"At most 3 favourites"`, and nothing is saved.
- `test_favorites.py::test_favorites_in_length_check_is_the_translatable_validator`: `FavoritesIn(book_ids=[1, 2, 3, 4])` raises a `value_error` with msg `"Value error, At most 3 favourites"`, and there is no `too_long` error.
- `test_favorites.py::test_favourite_does_not_require_own_shelf`: grace POSTs ada's book (200, `["Circe"]`). grace's shelf items are empty but her favourites list Circe. ada's favourites are empty.
- `test_favorites.py::test_add_helper_appends_and_rejects_a_fourth`:
  - POST returns 200; POSTing the same book again returns 200 with one item.
  - Adding up to 3 works; the 4th returns 409 with "3 favourites", and the order stays [Circe, Song, Galatea].
  - POST 9999 returns 400 with "not in the club".
- `test_favorites.py::test_remove_helper_compacts_positions`: DELETE of the middle favourite leaves [Circe, Galatea] at positions [1, 2]. A repeated DELETE returns 200 with the same list.
- `test_favorites.py::test_member_shelf_shows_their_favourites_not_yours`: `/api/shelf` shows the caller's favourites, and `?username=ada` shows ada's.
- `test_favorites.py::test_omitted_book_ids_clears_favourites`: PUT `{}` returns 200 with `{"items": []}`.
- `test_favorites.py::test_stale_last_slot_insert_maps_integrity_error_to_409`:
  - Setup: 2 favourites via PUT, then a 3rd inserted directly at position 3. `list_favorites` is monkeypatched to hide position 3 on its first call.
  - `add_favorite(leftover)` raises an HTTPException with 409 and `FAVOURITES_FULL`.
  - The list stays [Circe, Song, Galatea].
- `test_favorites.py::test_favorite_relationships_use_delete_cascade`: ORM introspection. The `User.favorites` and `Book.favorites` cascades include delete and delete-orphan. Both FKs have `ondelete == "CASCADE"`.
- `test_favorites.py::test_concurrent_add_for_last_slot_is_409_not_500`: with 2 favourites, two threads POST different books at once. The statuses sort to [200, 409] with no 500, and 3 favourites end up stored.
- `test_favorites.py::test_favorites_require_a_session`: GET, PUT, POST `/1` and DELETE `/1` without a session all return 401.

### `backend/tests/test_reading_dates.py` (12)
- `test_reading_dates.py::test_want_to_read_has_no_dates`: added as want, `started_at` and `finished_at` are both null.
- `test_reading_dates.py::test_reading_sets_started_and_finish_sets_finished`:
  - want → reading (position 0) sets `started_at` and leaves `finished_at` null.
  - Then → finished (rating 4) keeps the same `started_at`, sets `finished_at`, and `finished_at` ≥ `started_at` (compared as strings).
- `test_reading_dates.py::test_added_directly_as_finished_stamps_both`: added as finished, both dates are set.
- `test_reading_dates.py::test_reorder_within_finished_keeps_finished_at`: PATCH `{position: 0}` leaves `finished_at` unchanged.
- `test_reading_dates.py::test_leaving_finished_clears_finished_at_but_keeps_start`: finished → reading gives `finished_at` null and `started_at` unchanged.
- `test_reading_dates.py::test_back_to_want_to_read_resets_dates`: finished → want gives both dates null.
- `test_reading_dates.py::test_dnf_has_start_but_no_finish`: added as DNF, `started_at` is set and `finished_at` is null.
- `test_reading_dates.py::test_edit_and_clear_finish_date_survive_reordering`: with `BOOKCLUB_TZ=Pacific/Kiritimati`:
  - `finished_on 2025-07-01` gives `finished_at` starting `2025-06-30T22:00:00`, and `started_at ≤ finished_at`.
  - The shelf `timezone` is `"Pacific/Kiritimati"`.
  - PATCH `{position: 0, status: finished}` keeps the date.
  - `finished_on: null` makes it null. It stays null after a same-status reorder and after a rating-only PATCH.
- `test_reading_dates.py::test_invalid_finish_date_does_not_modify_entry`:
  - On a want entry, `finished_on` returns 400.
  - `{status: finished, finished_on: 2999-01-01}` returns 400 and the status stays want.
  - `finished_on: "invalid"` returns 400.
- `test_reading_dates.py::test_reread_replaces_completion_without_duplicate`: `finished_on` 2020-07-01, then → reading, then → finished, gives a `finished_at` that no longer starts with `2020-07`, and the shelf still has one item.
- `test_reading_dates.py::test_move_between_months_saves_order_and_date_together`: with `Europe/Berlin`:
  - Set `finished_on` 2025-09-23. Then PATCH `{shelf_position: 1, finished_on: 2025-08-01}` gives `shelf_position` 1, `finished_at` starting `2025-08-01T10:00:00`, status finished, and the rating and take kept.
  - The value persists, and the other entry gets `shelf_position` 0.
  - A `shelf_position: 0` reorder keeps the date.
  - `{shelf_position: 1, finished_on: 2999-01-01}` returns 400 and the shelf is unchanged (deep-equal).
- `test_reading_dates.py::test_cannot_change_another_members_completion_month`: grace PATCHes ada's entry with `{shelf_position: 0, finished_on: 2025-08-01}` and gets 404.

### Related tests in other files (owned by other areas, listed for cross-reference)
- `test_google_books.py`:
  - `test_shelf_add_persists_google_cover_url`, `test_goodreads_import_prefers_google`, `test_shelf_isbn_x_and_X_are_the_same_row`, `test_goodreads_import_while_cooling_down`: **drop**, because Google is removed.
  - `test_shelf_rejects_non_google_cover_url`: drop or reword; no client cover URLs are accepted.
- `test_activity.py::test_feed_records_the_club_story_newest_first`:
  - Shelf events in order: `shelf_added`, `shelf_moved`, `progress` (one only; an unchanged progress emits nothing), `shelf_finished` with payload `{"rating": 4, "take": "Great"}`, and `shelf_removed`.
  - The `progress` payload is `{"progress": 50}`.
- `test_migrations.py::test_legacy_database_is_upgraded_in_place`, `test_local_reads.py::test_shelving_persists_search_metadata_for_later_local_reads`, `test_members.py::test_members_hide_self_and_preview_reading`.

---

## 5. Multi-club mapping (per `plan.md` and #37/#41/#43/#44/#46)

### Personal vs club-scoped

**Everything in this area is personal.** No table gets a `club_id`:
- shelf entries, with status, rating, take, DNF reason, progress, dates, `position` and `shelf_position`
- favourites
- the physical layout

Spine colour is catalog data and becomes a property of a cover or media object.

### Reading another user's data (#46)
- `GET /api/shelf?username=` and `GET /api/shelf/physical-layout?username=` go through the central visibility policy.
- **Allowed:** self, members of any club the viewer shares with the owner, and friends (Phase 3).
- **Everyone else**, including nonexistent users, gets **404 `"No member with that name"`**. The two cases must be indistinguishable.
- Single-club "any signed-in user can view" no longer holds.
- Favourites in the shelf response follow the same audience. The plan lists favourites as personal data but not explicitly in the visibility list; this needs confirming.
- `user` in `ShelfListOut` currently leaks the owner's theme, colour mode and locale. Trim it to public fields (id, username, avatar_url) for non-self viewers, or confirm this is fine.

### Ratings set to "only me"
- Per-entry visibility (`rating_visibility`: `audience` | `only_me`).
- When another viewer looks, emit `rating: null` (and probably `take: ""`; this needs deciding).
- The same filter applies to every other reader of `shelf.rating`:
  - `shelf_finished` event payloads
  - pick readers
  - book detail `readers` and `club_rating` / `rating_count`
  - stats
- Self always sees their own rating.

### Writes
- PATCH and DELETE of someone else's entry stay **404** `"Book not on your shelf"`.
- The physical layout PUT only ever writes the caller's own layout. #46's AC "editing someone else's physical layout returns 404" needs an addressable target. Either add `?username=` or a path form to PUT and return 404 for anyone but the owner, or record that the AC is met because PUT can't target others and foreign entry ids give 400.

### Works vs editions (#37, #41)
- A shelf entry gets `work_id NOT NULL` plus `edition_id NULL` (the edition the user owns).
- Uniqueness becomes **`UNIQUE(user_id, work_id)`**, so adding a second edition of the same work gives 409 with the existing item.
- `POST /api/shelf` should take a catalog work reference (and an optional edition) instead of `ol_work_key + title/authors/cover/year`.
- **Client-supplied metadata must not overwrite catalog records.** Today `upsert_book` always overwrites the title, and authors, cover and year when non-empty. Under the plan, edits to existing records go to the admin queue. Remote search hits are ingested into the catalog by the catalog layer (#41) before or while shelving.
- `BookOut` for a shelf item should use the owned edition's cover and page count when set, falling back to the work's. The physical spine's default width "follows the page count", so it then follows the edition's pages.
- `progress` stays percent 0–100 and is independent of club milestones (percent or chapter).
- Favourites point to works.
- The physical layout still references shelf-entry ids and needs no change.

### Google removal
- Drop the Google branch of the Goodreads import: the 429 cooldown (`"The library is busy…"`) and the `"Catalog unavailable"` skip.
- Drop `cover_url` on `ShelfAddIn`, the Google hosts in `_fetchable`, `books.cover_image_url`, and `ISBN…`/`GB…` keys.
- Legacy Google books are re-matched by the importer (#38).

### Goodreads lookup under the new catalog
1. Local catalog, by ISBN and then FTS on title and author.
2. DNB SRU, as a fallback for German ISBNs (cached and throttled).
3. Open Library (User-Agent with contact, **≤3 req/s globally**).

Matched books are copied into the catalog. A Goodreads ISBN could set `edition_id`; this needs deciding. At 400 rows × up to 2 remote queries under ≤3 req/s, a synchronous request can run for minutes, so it likely needs a background job with progress (§6).

### Covers and spine colours (#43)
- Covers live in our media store and are served from our domain, so `BookOut.cover_url` becomes our URL.
- Spine sampling should read the stored bytes and make **no outbound HTTP**. Better still, compute the colour once when a cover is ingested and store it on the media or cover row.
- The `POST /api/shelf/spine-colors` endpoint can remain as a backfill with the same contract, since the frontend calls it.
- A cover replacement (user photo, admin decision) must reset the colour.

### Timezone
- There is no instance `BOOKCLUB_TZ` any more. `ShelfListOut.timezone` and the "today" and "noon local" rules for `finished_on` need a per-user timezone preference (IANA, validated, `UTC` fallback).
- The legacy importer should set it from the old instance's `BOOKCLUB_TZ`.
- Clubs may have their own zone for meetings, but that doesn't apply to the personal shelf.

### Activity events
- Shelf events are personal actions. Decide whether they're stored without `club_id` and shown in each club feed (and the combined home feed) where the viewer shares a club with the actor (filtered by the visibility policy at read time), or fanned out per club at write time.
- Either way, rating payloads must respect `only_me`.
- The import keeps emitting no events.

### Wording that no longer fits
- `"That book is not in the club yet"` refers to "the club" while favourites now reference the global catalog. Reword it (e.g. "That book is not in the catalog yet") and update `apiErrors.ts` and the i18n keys.
- `"No member with that name"` could also be reworded, since it isn't "member" without a club. It is currently unmapped.

### What no longer makes sense
- SQLite id reuse. Postgres sequences never reuse ids, but keep prune-on-delete so stored layouts stay clean.
- The instance-wide `timezone`.
- Google cover URL persistence.
- The "single club" assumptions behind `?username=`.
- Roles have no effect in this area.

### Legacy importer (#38) duties for this area
- **Shelf rows:**
  - Copy them, converting naive UTC strings to `timestamptz`.
  - COALESCE `take` and `dnf_reason` to `''`.
  - Keep `position` and `shelf_position`.
  - Keep NULL dates as NULL.
- **Merges:** map `book_id` to `work_id`. Two legacy books (e.g. an `ISBN…` Google row and an `OL…W` row) can re-match to the **same work**, so one user may end up with two entries for one work. That collides with `UNIQUE(user_id, work_id)`. The importer needs a merge rule (e.g. keep the more advanced status, or the one with a rating, take or dates) and must re-sequence `position`.
- **Layouts:** rewrite `layout_json`:
  - Remap every entry id in `rows` and `spines` keys to the new ids.
  - Drop ids that were merged away (the "appear once" rule would otherwise fail).
  - Validate the result with the current schema.
- **Favourites:** map to works. Drop duplicates created by merges and compact positions to 1..n.
- **Spine colours:** these can be dropped and resampled from the new covers, or copied onto the matching cover.
- **Events:** copy `shelf_*` and `progress` events if the activity history is kept.

---

## 6. Port risks: things a reimplementation would likely get wrong

1. **Absent vs null in PATCH.**
   - `rating`, `take`, `dnf_reason`, `progress` and `finished_on` need tri-state deserialisation (`Option<Option<T>>` with `#[serde(default, deserialize_with=…)]`): absent means keep, null means clear.
   - `status`, `position` and `shelf_position` treat null as absent.
   - The "Nothing to update" check uses presence, not value, for the five tri-state fields, and runs **before** the 404 ownership check.
2. **When `place_item` runs.** It runs on every PATCH unless `shelf_position` is set and both `status` and `position` are null or absent (Phase 1: or the PATCH sets nothing but `rating_visibility`). It bumps `updated_at` and renumbers the column. A `shelf_position`-only PATCH must leave `updated_at` byte-identical (the test deep-compares items).
3. **Date stamping.**
   - PATCH stamps only on an actual status change. POST and import always stamp with `changed=False`.
   - `finished_on` is applied **after** stamping and then clamps `started_at`.
   - Validation uses the **resulting** status and "today" in the configured zone, with noon local time converted to UTC.
   - A `finished` entry with null `finished_at` must stay null across same-status edits.
4. **Ordering of `GET /api/shelf`.** It sorts by status **text** alphabetically, then position, then id. A Postgres `ENUM` sorts by declaration order, so use `status::text` (or store text with a CHECK) if parity matters. Tests compare whole item lists for equality, so the order must be deterministic.
5. **`place_on_shelf`.**
   - Unarranged entries sort first by **declaration** order (want, reading, finished, DNF).
   - Every row is renumbered.
   - The position is clamped.
   - DELETE does not compact `shelf_position`.
6. **Character limits.** `take` (140) and `dnf_reason` (200) count Unicode **characters after trimming**, not bytes. Use `.chars().count()`. Python's `str.strip()` whitespace set is close to Rust's `trim()`; edge cases like `\x1c`–`\x1f` differ but don't matter.
7. **Silently dropped fields.** Rating, take, progress and DNF reason are silently dropped or cleared for statuses that don't carry them. A rating sent while `want_to_read` is not an error.
8. **POST 409 body.** It must include `item` (the frontend uses it). The metadata upsert must not be committed on 409. The concurrent-duplicate race should also give 409, not 500.
9. **Events.**
   - Exact kinds and payloads, with null keys dropped and `take`/`reason` only when non-empty.
   - The `progress` event fires only on a real change.
   - Import writes no events.
   - DELETE records `shelf_removed` with the old status.
10. **Physical layout validation strictness.**
    - Row ids and `width` are **strict** ints: reject `true`, `1.5`, `1.0`, `"1"`. A serde untagged `enum Item { Id(i64), Decor(String) }` plus a range check gets close.
    - `lights` is lax in Python; being stricter is acceptable (only `"sometimes"` is tested), but don't reject `true`/`false`.
    - Unknown top-level keys are ignored; don't `deny_unknown_fields`.
    - The 2100-items check comes before the 2000-books check.
    - Decorations must match the row strings **exactly**.
    - Spine-key regex `[1-9][0-9]{0,18}`: a 19-digit key can exceed `i64::MAX`. Parse it safely and return 400 `"Physical shelf books must be on your shelf"`, never a 500.
    - Ownership covers row ids ∪ spine keys.
11. **Layout echo shape.** Colours are lowercased. Every spine always has `color`, `width` and `display`, and `lights` is always present. JSONB reorders object keys; that's harmless because `rows` carries the placement.
12. **Layout read path.** It prunes missing entries but keeps row boundaries, doesn't write back, and must **not 500** on a stored layout that fails the current schema (degrade gracefully). Legacy layouts without `decorations`, `lights` or `display` must load.
13. **Prune on DELETE in the same transaction.** `test_deleted_spine_does_not_leak_to_a_book_with_a_reused_id` relies on SQLite id reuse, which Postgres never does. Port it as "after DELETE, the stored layout JSON no longer contains the id", by reading the row directly, rather than asserting id reuse.
14. **Favourites.**
    - Validator order: >3, then duplicates, then ≤0. All are custom strings. Don't use a generic length error.
    - PUT is delete-then-insert in one transaction.
    - POST is idempotent for an existing favourite. A full list gives 409. A unique violation (SQLSTATE 23505) on the race gives re-read, then 200 or 409.
    - DELETE of a non-favourite or unknown id gives 200 with the current list.
    - DELETE compacts positions.
    - Pick the next slot as the lowest free position, or compact, to avoid the latent gap bug (§3).
    - Locking the user (e.g. `SELECT … FOR UPDATE` on the user row, or an advisory lock) makes the concurrent test deterministic.
15. **Spine colour retry semantics.**
    - Settle as `""`: no URL, 404/410, too large, disallowed redirect, more than 3 redirects, undecodable image, tiny image, or a non-network error.
    - Leave NULL and force `remaining: 0`: transport errors, timeouts (8 s per request, 15 s overall), and other non-200 statuses including 429/403/5xx.
    - Batch of 12 ordered by book id, resolved concurrently.
    - `colors` keys are book ids as JSON strings.
    - Emit `""` as null in `BookOut`.
16. **SSRF guard on cover fetches.** Do not use the HTTP client's automatic redirects. Validate **every** hop: https, no port, no userinfo, host allowlist matched by suffix with a leading dot (`archive.org.example.com` must fail). Stream with a 1.5 MB cap.
17. **`dominant_color` fidelity.**
    - Crop 8% with int truncation, `thumbnail` 48 (no upscaling), median cut to 6 colours, score `(count/total)*(0.35 + HSV saturation)`, the first maximum wins, HLS lightness clamped to [0.17, 0.86], lowercase hex.
    - Rust has no drop-in Pillow MEDIANCUT. Implement median cut by hand rather than using NeuQuant (`color_quant`), which behaves differently. Avoid `imagequant` (GPL-3.0).
    - Tests assert only qualitative properties, but keep the border-ignoring and vivid-beats-grey behaviour.
    - The pixel limit (4M) and minimum edge (8) are part of the contract.
18. **Goodreads CSV quirks.**
    - Strip the BOM.
    - Header detection is trimmed and case-insensitive.
    - Values like `="…"` are unquoted fields with literal quotes. The `csv` crate treats quotes as special only at field start, which matches.
    - Enable flexible row lengths, treat missing cells as `""` and ignore extra cells. Python 500s here; that is a bug, don't port it.
    - Skip blank lines before counting.
    - One `"Stopped after 400 rows"` skip (title `""`).
    - Skip order: parse skips, then lookup skips.
    - `skipped` counts all skips, while `skips` is capped at 25.
    - The `"Untitled"` label applies only to "No title".
    - The ISBN13 column is preferred.
19. **Import lookup semantics.**
    - Two queries (ISBN, then `"title authors"`), first hit wins, using the **hit's** metadata.
    - Remote failure gives "No match", not an error.
    - Within-file duplicates give "Already on your shelf". The second row must see the first row's insert in the same transaction.
    - Each import goes to position 0, so CSV order is reversed per status.
    - Dates are stamped at import time.
    - It's all one transaction.
    - Under the new ≤3 req/s OL limit this can exceed proxy timeouts; decide on a job or streaming before porting.
20. **Upload size.**
    - Python reads up to 1.5 MB + 1 and returns 400 `"That file is too large. Keep the export under 1.5 MB."`.
    - axum's default body limit is 2 MB and returns **413 with a plain-text body** for larger uploads. Set an explicit limit on this route (e.g. 2 MB) and map oversize to the same 400 JSON detail, or accept 413 for very large files and say so in the parity notes.
    - A missing `file` field gives 400.
21. **Datetime format.** Python emits naive strings; Rust should emit RFC 3339 with `Z`. Ported tests must parse datetimes before comparing them; don't compare strings. For example, `finished_at >= started_at` and `startswith("2025-08-01T10:00:00")` are both fragile once fractional digits or an offset suffix appear.
22. **`username` query handling.** It is lowercased, not trimmed, and an empty value means self. With the visibility policy, "unknown" and "not visible" must return the identical 404 body.
23. **Catalog mutation via shelf add.** Python silently renames shared books on every shelf add (the title is always overwritten). The port must not let shelf actions edit catalog rows. `test_shelving_without_authors_does_not_clear_existing_authors` then holds trivially.
24. **Concurrent reorders.** Python renumbers in application code without locks. In Postgres, take a per-user lock (advisory lock on `user_id`, or `SELECT … FOR UPDATE` on the user's shelf rows) for add, patch, delete and import, so concurrent edits from two devices can't produce duplicate positions.
25. **Cascades.**
    - Favourites and layouts cascade on user delete. `shelf` does not in Python; account deletion (#39) must remove shelf rows, so use ON DELETE CASCADE in the new schema.
    - Favourites also cascade on work delete. Decide what happens to shelf entries if a work is merged or deleted in the admin queue (re-point them rather than deleting).
