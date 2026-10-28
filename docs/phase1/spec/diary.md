# Port spec: diary (reading diary, reactions, quotes, stats, milestones, search history)

**Sources read in full:**
- Routers: `backend/app/routers/{diary,quotes,stats,milestones,search_history}.py`.
- Helpers: `backend/app/{models,schemas,serialize,shelf_ops,pick_ops,timezone,works,covers,activity,db,deps,main}.py`.
- Related code: the pick-post section of `routers/pick.py`, `routers/activity.py`, the `quote_count` lines of `routers/books.py`, and `update_shelf_item` in `routers/shelf.py`.
- Tests: `backend/tests/{test_diary,test_stats_milestones_quotes,test_search_history,conftest}.py`, plus the parts of `test_local_reads.py` and `test_migrations.py` that touch this area.
- Frontend: `frontend/src/{api/client.ts,types.ts,diary.ts,i18n/apiErrors.ts,i18n/dates.ts}`.
- Plan: `plan.md`, `phase1-issues.md`, and the bodies of Forgejo #37, #44, #45 and #46.

Behaviour marked **(probed)** was confirmed by running the Python app against a scratch database, not only read from the source. The app ran on FastAPI 0.162, pydantic 2.14, SQLModel 0.0.48 and Starlette 1.10. The probe scripts were throwaway session files and are not in the repo.

**Frontend status (important for #48).** Only the diary and search history have a UI today.
- The editorial-redesign merge (`273f7c0`, PR #3) removed every frontend call to `/api/quotes`, `/api/stats` and `/api/pick/{id}/milestones`.
- CLAUDE.md's "Backend features without a frontend" list does not mention this; it names only push and the password change.
- These three features are therefore backend-only today, but their tests still pass.
- #48 lists `/quotes` as a personal page, so a quotes UI must be built from scratch.

---

## 1. Endpoints

### 1.0 Conventions shared by every endpoint in this area

These match `auth.md` §1.0. Only what matters here is repeated.

**Error body**
- Every error is `{"detail": "<English string>"}`.
- Only one string in this area is mapped in `frontend/src/i18n/apiErrors.ts`: `"Search history entry not found"` → `errors.historyNotFound`. It must stay byte-identical.

**Validation errors**
- They return **400** (not 422).
- Messages appear in field-declaration order, with any leading `"Value error, "` stripped, joined with `"; "`.
- Example **(probed)**: `POST /api/books/works/OL1W/posts {"body":"","spoiler_upto":101,"book":{"title":""}}` → 400 `"Write a short take, quote, or meeting note; Input should be less than or equal to 100; A title is required"`.

Generic pydantic messages seen in this area **(probed)**:

| Situation | Message |
|---|---|
| Missing field | `"Field required"` |
| Wrong type where a string is expected | `"Input should be a valid string"` |
| Non-integer path or query param, or a non-numeric string for an int field | `"Input should be a valid integer, unable to parse string as an integer"` |
| Float with a fraction for an int field | `"Input should be a valid integer, got a number with a fractional part"` |
| Below a `ge` bound | `"Input should be greater than or equal to N"` |
| Above an `le` bound | `"Input should be less than or equal to N"` |

**Lax integer coercion (probed)**
- pydantic accepts `"50"` (a numeric string), `true` (→ 1) and `50.0` for int fields.
- `spoiler_upto: true` was stored as `1`.
- serde rejects all three by default. See risk R14.

**Order of checks**
1. JSON decode.
2. Auth: 401 `"Not signed in"`.
3. Body, path and query validation: 400.
4. Handler logic.

So a signed-out request with an invalid body gets **401** (probed: `POST /api/books/works/nope/posts {"body":""}` signed out → 401). A signed-in request with an invalid body *and* an invalid work id gets **400** for the body, because validation runs before the handler's work-id check (probed).

**Auth:** every endpoint here requires a signed-in user (`deps.get_current_user`). There are no role checks anywhere in this area: any signed-in user may do anything that is not author-restricted.

**Timestamps**
- They are written as aware UTC. SQLite stores them as naive text (`'2026-10-27 10:58:49.471065'`) and returns naive datetimes.
- They are therefore serialized as **naive ISO strings with microseconds and no offset**, e.g. `"2026-10-27T10:58:18.198238"` (probed).
- **Exception:** `MilestoneOut.due_at` is made aware before serialization and comes out as `"2030-07-05T18:00:00Z"` (probed).

**Timezone**
- `resolved_timezone(BOOKCLUB_TZ)` is the club zone: an empty or unknown name becomes `"UTC"`.
- Responses that carry `timezone` echo it.

**`created_label` / `due_label`**
- `meeting_label(dt, tz)`: treat a naive `dt` as UTC, convert to the club zone, format with `strftime("%a %d %b %Y, %H:%M %Z")` in the C locale.
- Probed examples: `"Tue 27 Oct 2026, 10:58 UTC"`, `"Fri 05 Jul 2030, 20:00 CEST"`, `"Sun 05 Jan 2020, 16:00 CET"`.
- The day is zero-padded.
- `created_label` falls back to `""` (it can't be None, because created_at is never null).

**`due_local`:** `strftime("%Y-%m-%dT%H:%M")` in the club zone.

**`BookOut`** (from `serialize.book_out`). Field order is `id, ol_work_key, title, authors, cover_id, year, cover_url, pages, spine_color`.
- `id`: int, `books.id`.
- `ol_work_key`: str, e.g. `"/works/OL1W"`.
- `title`: str.
- `authors`: str, a single comma-joined string, stored verbatim.
- `cover_id`: int|null.
- `year`: int|null.
- `cover_url`: str|null, chosen in this order:
  1. the stored Google URL, if it is on an allowed host;
  2. else `https://covers.openlibrary.org/b/id/{cover_id}-L.jpg?default=false` if cover_id > 0;
  3. else, when no Google key is set and the key is `ISBN…`, `https://covers.openlibrary.org/b/isbn/{isbn}-L.jpg?default=false`;
  4. else null.
- `pages`: int|null.
- `spine_color`: str|null; an empty string becomes null.

**`author_avatar_url`:** `avatar_url_for(user)` returns `"/api/members/{username}/avatar?v={epoch_seconds_of_avatar_updated_at}"`, or null when there is no avatar.

**`ShelfStatus` values:** `want_to_read`, `currently_reading`, `finished`, `did_not_finish`.

**`ReactionOut`:** `{emoji: str, count: int, mine: bool, users: [str]}`.

**`DiaryEntryOut`.** Field order is `id, author, author_avatar_url, mine, body, deleted, spoiler_upto, progress_at, status_at, author_rating, parent_id, created_at, created_label, edited, reactions, replies`.

| Field | Type | Value |
|---|---|---|
| `id` | int | |
| `author` | str | Author's username, `"unknown"` if missing (unreachable) |
| `author_avatar_url` | str\|null | |
| `mine` | bool | `author_id == viewer.id` |
| `body` | str | `""` when deleted |
| `deleted` | bool | `deleted_at IS NOT NULL` |
| `spoiler_upto` | int\|null | null when deleted |
| `progress_at` | int\|null | Snapshot. **Not** cleared on delete |
| `status_at` | ShelfStatus\|null | Snapshot. **Not** cleared on delete |
| `author_rating` | int\|null | The author's *current* shelf rating for this book, only if their *current* status is `finished`; else null. **Computed even for tombstones** |
| `parent_id` | int\|null | |
| `created_at` | naive datetime | |
| `created_label` | str | |
| `edited` | bool | `updated_at IS NOT NULL AND NOT deleted` |
| `reactions` | [ReactionOut] | `[]` when deleted |
| `replies` | [DiaryEntryOut] | See each endpoint |

Replies nest only one level. A reply's own `replies` is always `[]`.

**Reaction aggregation** (`_reactions_out`):
- Group reaction rows by emoji. `users` is the usernames sorted ascending. `count = len(users)`. `mine = viewer.username in users`.
- Groups are sorted by `(-count, emoji)`; emoji strings compare by code point.
- Probed order for counts 2,1,1,1: U+1F525 (count 2), then U+2764 U+FE0F, U+1F44D, U+1F4DA.

**Position helper** `_position(shelf_entry)` returns `(progress, status)`:
- no shelf entry → `(null, null)`;
- `finished` → `(100, "finished")`, whatever `shelf.progress` holds;
- `currently_reading` → `(shelf.progress, "currently_reading")`; progress may be null;
- `want_to_read` → `(null, "want_to_read")`;
- `did_not_finish` → `(null, "did_not_finish")`.

**Work id in diary paths** (`diary._work_key`):
1. Canonicalize: if the id starts with `ISBN` case-insensitively and `"ISBN" + rest.upper()` matches `^ISBN(?:\d{9}[\dXx]|\d{13})$`, use that uppercase form.
2. Match `^(?:OL\d+W|BC[a-f0-9]{10}|ISBN(?:\d{9}[\dXx]|\d{13})|GB[A-Za-z0-9_-]{1,40})$`.
3. If it doesn't match: 404 `"No such book."` (**note the trailing period**).
4. The work key is `"/works/" + canonical_id`.

Probed:
- `ol1w` → 404 and `BCDEADBEEF01` → 404 (both prefixes are case-sensitive).
- `isbn030640615x` → 200 with `ol_work_key "/works/ISBN030640615X"`.
- `GBabc_-1` → 200.
- `OL1` → 404.
- `%2Fworks%2FOL1W` → no route matches, so the framework's 404 `"Not Found"`.

**Reaction set.** Exactly these six strings after `strip()`, compared by exact code points:
1. U+2764 U+FE0F (two code points)
2. U+1F44D
3. U+1F602
4. U+1F62E
5. U+1F525
6. U+1F4DA

Anything else → 400 `"Pick one of the club reactions"`. A bare U+2764 without U+FE0F is rejected (probed), and `" <fire> "` with spaces is accepted (probed). The same list is `REACTIONS` in `frontend/src/diary.ts`.

**Post body rule** (`_post_body_ok`, shared by diary entries and legacy pick posts):
1. `strip()`.
2. If empty → `"Write a short take, quote, or meeting note"`.
3. If more than 1000 **code points** → `"Keep it to 1000 characters"` (probed: 1000 × `é` is accepted, 1001 is rejected).
4. Store the stripped value.

---

### 1.1 `GET /api/books/works/{work_id}/posts`: one book's diary

- **Auth:** signed in.
- **Path:** `work_id` (str), validated as in §1.0; failure → 404 `"No such book."`.
- **Response 200** `DiaryOut`, field order `ol_work_key, book_id, my_progress, my_status, items, timezone`:
  - `ol_work_key`: str, the canonical `"/works/<id>"`.
  - `book_id`: int|null; null when no `books` row has this key.
  - `my_progress`, `my_status`: `_position(viewer's shelf entry for this book)`.
  - `items`: top-level entries (parent_id null), ordered **`created_at ASC, id ASC`**. Each carries `replies`: its replies, also ordered `created_at ASC, id ASC`. Tombstones are **included**.
  - `timezone`: str.
- **Unknown but valid work id:** 200 `{"ol_work_key": "/works/OL1W", "book_id": null, "my_progress": null, "my_status": null, "items": [], "timezone": "UTC"}`. It is never 404 and never contacts any catalog (test 2).
- **Visibility:** none. Every user sees every entry, including other people's spoiler-flagged bodies in full. The spoiler shield is **client-side only** (`diary.ts: isShielded`).
- **Side effects:** none. No outbound HTTP.

### 1.2 `POST /api/books/works/{work_id}/posts`: write an entry or a reply

- **Auth:** signed in.
- **Path:** `work_id` as in §1.0; invalid → 404 `"No such book."`. This check runs after body validation.
- **Body** `DiaryEntryIn`; unknown keys are ignored:
  - `body` (str, required): post body rule (§1.0).
  - `spoiler_upto` (int|null, default null): 0 ≤ x ≤ 100. The percent of the book the entry is safe up to; null means no spoiler flag.
  - `parent_id` (int|null, default null).
  - `book` (`DiaryBookIn`|null, default null), used **only** when no `books` row exists for the key:
    - `title` (str, required): stripped. Empty → `"A title is required"`. Then **silently truncated to 500 code points**.
    - `authors` (str, default `""`): not stripped.
    - `cover_id` (int|null): a value ≤ 0 is stored as null by `upsert_book`.
    - `cover_url` (str|null): stripped, then `storable_cover_image_url`. Only https URLs (http is rewritten to https) on `books.google.com`, `*.books.google.com`, `googleusercontent.com` or `*.googleusercontent.com` are kept, with no userinfo and ≤ 512 chars. Anything else becomes null **silently**.
    - `year` (int|null): no range.
- **Logic, in order:**
  1. Look up the book by `ol_work_key == work_key`.
  2. If it is missing:
     - and `book` is null → 400 `"Nobody has this book yet; send its title to start the diary"`;
     - otherwise `upsert_book(...)` inserts a `books` row and flushes it (not committed yet).
  3. If the book **exists**, the `book` payload is **ignored entirely**: no title, author or cover update (probed: title stays "Circe" after posting `book.title "CHANGED"`).
  4. If `parent_id` is set:
     - the parent is missing, or its `book_id` differs → 400 `"That entry is not on this book"`;
     - the parent itself has a `parent_id` → 400 `"Reply to the entry itself, not to a reply"`;
     - the parent has `deleted_at` set → 400 `"That entry was deleted"`.
  5. Snapshot the author's position: `progress_at, status_at = _position(author's shelf entry for this book)`.
  6. `pick_id` = the current pick's id **if** a current pick exists (newest `club_picks` row with `ended_at IS NULL`) **and** its `book_id` is this book; else null.
  7. Insert `book_posts` and commit. A failed parent check rolls back the just-created book too (probed: `OL2W` stays `book_id: null` after a rejected reply).
- **Response 201** `DiaryEntryOut` for the new entry, with `replies: []`. `author_rating` is the author's current rating if they are finished.
- **Errors:**
  - 400 validation messages;
  - 400 `"Nobody has this book yet; send its title to start the diary"`;
  - 400 `"That entry is not on this book"`, `"Reply to the entry itself, not to a reply"`, `"That entry was deleted"`;
  - 404 `"No such book."`;
  - 500 `"Could not save that entry"` (unreachable).
- **Side effects:**
  - It may insert a `books` row.
  - It does **not** add the book to the writer's shelf (test 3).
  - **No activity event.** `note_posted` is only emitted by legacy pick posts.
  - No push. No outbound HTTP, even for `BC…` books (test 15).

### 1.3 `PATCH /api/posts/{post_id}`: edit your own entry or reply

- **Auth:** signed in.
- **Path:** `post_id` (int).
- **Body** `DiaryEntryPatchIn`:
  - `body` (str|null, default null). Null means "do not change". When present it follows the post body rule.
  - `spoiler_upto` (int|null, 0..100). Tri-state: **absent** = keep, **explicit null** = clear, int = set.
- **Logic, in order:**
  1. Load the post. If it is missing **or a tombstone** → 404 `"No entry with that id"`.
  2. If `author_id != me` → 403 `"You can only edit your own entries"`. This runs before the empty-body check, so a non-author's `{}` gets 403.
  3. If `body` is null and `spoiler_upto` is absent → 400 `"Nothing to update"`. This includes `{"body": null}` (probed).
  4. Apply the fields. Set `updated_at = now` **unconditionally**, even when the values did not change. Commit.
     - **Phase 1:** this holds for `body` and `spoiler_upto`. The new `visibility` (§5.1) alone does not move `updated_at`, so switching an entry to "Only me" and back does not mark it edited (`docs/phase1/notes/fup.md` item C).
- **Response 200** `DiaryEntryOut`. For a top-level entry it includes the full `replies` list (`created_at, id` ascending); for a reply, `replies: []`.
- **Not changed:** progress_at, status_at, pick_id. There is no activity event.

### 1.4 `DELETE /api/posts/{post_id}`

- **Auth:** signed in.
- **Logic:**
  1. Missing or already a tombstone → 404 `"No entry with that id"`.
  2. Not the author → 403 `"You can only delete your own entries"`.
  3. Delete **all** `book_post_reactions` of the post.
  4. If any row has `parent_id == post_id`, keep a **tombstone**: `body = ""`, `spoiler_upto = NULL`, `deleted_at = now`. `progress_at`, `status_at`, `updated_at`, `pick_id` and `created_at` are kept. Otherwise hard-delete the row.
  5. Commit.
- **Response:** 204 with an empty body.
- **Consequences (probed):**
  - A tombstone stays in the book diary even after its last reply is deleted: `deleted: true, replies: []`.
  - A tombstone can never be removed through the API; a second DELETE gives 404.
  - Deleting a reply is always a hard delete, because replies have no replies.

### 1.5 `POST /api/posts/{post_id}/reactions`: toggle a reaction

- **Auth:** signed in.
- **Body** `ReactionIn`: `{emoji: str}`, required, from the reaction set (§1.0). A missing body field → 400 `"Field required"` even for a missing post (probed).
- **Logic:**
  1. Missing post or tombstone → 404 `"No entry with that id"`.
  2. If a row `(post_id, me, emoji)` exists, delete it; otherwise insert one.
  3. Commit.
- **Permissions:** anyone signed in may react to any live entry, their own included, top-level or reply.
- **Response 200** `DiaryEntryOut` of the reacted post: replies included for top-level entries, `[]` for replies.
- **Other effects:** none. `updated_at` and `edited` are untouched, and there is no activity event.
- **Constraint:** `UNIQUE(post_id, user_id, emoji)`. Two concurrent identical toggles can raise an IntegrityError (500) in Python.

### 1.6 `GET /api/diary`: newest entries across every book (Club page "recently written")

- **Auth:** signed in.
- **Query:**
  - `before` (int|null): ≥ 1; it is an entry id. `0` → 400 `"Input should be greater than or equal to 1"`.
  - `limit` (int, default **10**): 1..**50**. 0 → 400 `"Input should be greater than or equal to 1"`; 51 → 400 `"Input should be less than or equal to 50"`. The frontend sends `limit=8`.
- **Query logic:**
  - Select `book_posts WHERE deleted_at IS NULL` (tombstones are excluded; their replies are not), `[AND id < before]`, `ORDER BY id DESC LIMIT limit+1`.
  - `has_more = rows > limit`; keep the first `limit` rows.
- **Response 200** `DiaryFeedOut {items, has_more, timezone}`. Each item is `DiaryFeedItemOut {entry, book, parent_author, my_progress, my_status}`:
  - `entry`: DiaryEntryOut with **`replies: []` always**, even for top-level entries with replies.
  - `book`: BookOut.
  - `parent_author`: str|null. The parent's author username when the entry is a reply, **even when the parent is a tombstone** (probed); null for top-level entries.
  - `my_progress`, `my_status`: `_position(viewer's shelf entry for that item's book)`.
- An item whose book is missing is skipped after `has_more` has been computed (unreachable with the FK).
- **Ordering quirk:** the feed orders by **id**, while the book diary orders by **created_at**. Legacy-imported posts carry an old `created_at` but a newer id.

---

### 1.7 `GET /api/quotes`

- **Auth:** signed in.
- **Query:**
  - `work` (str|null). Exact match against `books.ol_work_key`, with **no canonicalization**: `work=OL5W` matches nothing (probed). An empty string is ignored. An unknown key → `{"items": []}`.
  - `username` (str|null). Lowercased, **not stripped**. Empty is ignored. An unknown user → `{"items": []}`.
  - `limit` (int, default **100**): 1..**500**; the bounds give the generic messages.
- **Filters** combine with AND. **Order:** `quotes.id DESC`, then `LIMIT`. There is no pagination cursor.
- **Response 200** `QuoteListOut {items: [QuoteOut]}`. `QuoteOut` field order is `id, book, author, mine, body, page, created_at, created_label`:
  - `id`: int.
  - `book`: BookOut.
  - `author`: username of the quote's owner.
  - `mine`: `user_id == viewer.id`.
  - `body`: str.
  - `page`: int|null.
  - `created_at`: naive datetime.
  - `created_label`: str.
- **Visibility:** none. Everyone sees every member's quotes.
- **Error:** 500 `"Quote is missing its book"` (unreachable).

### 1.8 `POST /api/quotes`

- **Auth:** signed in.
- **Body** `QuoteIn`, declaration order `ol_work_key, title, authors, cover_id, cover_url, year, body, page`:
  - `ol_work_key` (str, required): stripped; must start with `"/works/"`, else `"ol_work_key must be an Open Library work key"`. **Nothing else is validated**: `/works/hello` and `/works/a/OL5W` pass.
  - `title` (str, default `""`).
  - `authors` (str, default `""`): not stripped, stored verbatim (probed `" A "`).
  - `cover_id` (int|null).
  - `cover_url` (str|null): no schema validator, but `upsert_book` applies `storable_cover_image_url`, so non-Google URLs are dropped (probed: an OL cover URL became null).
  - `year` (int|null).
  - `body` (str, required): stripped. Empty → `"Paste the line you want to keep"`; more than 1000 code points → `"Keep it to 1000 characters"`.
  - `page` (int|null): 0..20000 (`"Input should be greater than or equal to 0"` / `"Input should be less than or equal to 20000"`).
- **Logic:**
  1. Look up the book by `ol_work_key == <stripped key>`.
  2. If it is missing:
     - and `title.strip()` is empty → 400 `"title is required for a new book"`;
     - otherwise call `upsert_book` with the title stripped. That function **re-normalizes** the key to `"/works/" + canonical(last path segment)` (probed: `" /works/hello/OL5W "` created `/works/OL5W`). If that normalized key already exists, `upsert_book` **updates** that book's title, authors, cover and year instead of inserting.
  3. If the book exists under the exact key, the metadata is ignored.
  4. Insert the quote and flush.
  5. Record the activity event `quote_added`: `book_id` = the book, payload `{"page": page}` (omitted when null) plus `"excerpt": body[:80]` (the first 80 code points).
  6. Commit.
- **Response 201** QuoteOut.
- **Errors:**
  - 400 validation messages;
  - 400 `"title is required for a new book"`;
  - 500 `"Could not save that quote"` (unreachable).

### 1.9 `PATCH /api/quotes/{quote_id}`

- **Body** `QuotePatchIn`:
  - `body` (str|null): null means keep. When present, the same rule and messages as POST.
  - `page` (int|null, 0..20000): tri-state; absent = keep, explicit null = clear.
- **Logic:**
  1. Missing → 404 `"No quote with that id"`.
  2. Not the owner → 403 `"You can only edit your own quotes"`.
  3. `body` null and `page` absent → 400 `"Nothing to update"`, including `{"body": null}` (probed).
  4. Apply the fields and commit.
- There is no `updated_at` and no edited flag. There is no activity event.
- **Response 200** QuoteOut.

### 1.10 `DELETE /api/quotes/{quote_id}`

1. Missing → 404 `"No quote with that id"`.
2. Not the owner → 403 `"You can only delete your own quotes"`.
3. Delete the quote.

The response is **204**. The `quote_added` event stays in `events`.

---

### 1.11 `GET /api/stats`: year in review, per member and per pick

- **Auth:** signed in. The output is the same for every viewer (it is club-wide).
- **Query:** `year` (int|null): 2000..2100 (`"Input should be greater than or equal to 2000"` / `"Input should be less than or equal to 2100"`). A year with no data is accepted and returns zeros (probed: `year=2050`).
- **Response 200** `StatsOut`, field order `year, years, members, by_month, picks, club_finished, club_pages, club_average_rating, best_pick`:
  - `year`: int, the chosen year.
  - `years`: [int] descending. The distinct `year_of(e)` over every finished or DNF shelf entry, ∪ {the current UTC year}.
  - `members`: [MemberYear]. **Every user** in the DB, including those with zero activity, sorted by `(-finished, username)`.
  - `by_month`: [{`month`: "YYYY-MM", `finished`: int}]. Over all members' finished entries in the chosen year, keyed by the UTC month of `finished_at` (or `updated_at` when that is null), sorted ascending by month. Months with zero are omitted.
  - `picks`: [PickStat] for **every** club pick, ordered `id DESC`. **Not year-filtered.**
  - `club_finished`: int, the sum of the members' `finished`.
  - `club_pages`: int, the sum of the members' `pages`.
  - `club_average_rating`: float|null, the rounded mean of every member's counted ratings in the year.
  - `best_pick`: PickStat|null.
- **`MemberYear`**, field order `username, finished, dnf, pages, average_rating, five_stars, top_book, longest_book, quotes`. Only this user's finished and DNF entries whose `year_of == year` count:
  - `finished`: count with status `finished`.
  - `dnf`: count with status `did_not_finish`.
  - `pages`: Σ `book.pages or 0` over finished.
  - `average_rating`: `round(mean(ratings), 2)` or null. `ratings` = the finished entries' `rating`, keeping truthy values only (so nulls are dropped).
  - `five_stars`: count of ratings == 5.
  - `top_book`: BookOut|null. The finished entry with a truthy rating that maximizes `(rating, shelf.id)`, so a tie goes to the **highest shelf entry id**.
  - `longest_book`: BookOut|null. The finished entry with truthy `book.pages` that maximizes pages; a tie goes to the **first** in iteration order, i.e. the lowest shelf id (probed).
  - `quotes`: count of this user's quotes with UTC `created_at.year == year`.
- **`PickStat`**, field order `pick_id, book, set_by, started_at, ended_at, readers, finished, dnf, average_rating, ratings`:
  - `set_by`: the setter's username or `"unknown"`.
  - `started_at`, `ended_at`: naive datetimes.
  - `readers`: count of **all** shelf entries for the pick's book, in any status, `want_to_read` included, from any year.
  - `finished`, `dnf`: all-time counts for that book.
  - `ratings`: the finished entries' truthy ratings, sorted **descending**.
  - `average_rating`: rounded mean or null.
  - A book picked twice yields two identical-looking rows.
- **`best_pick`:** among picks with a non-null `average_rating`, the maximum by `(average_rating, len(ratings))`. On a full tie the first in `id DESC` order wins, i.e. the newest pick.
- **`year_of(entry)`:** the UTC year of `finished_at`, else of `updated_at` (NOT NULL, so the `now()` fallback is dead code).
  - DNF entries always have `finished_at = NULL`, so they count by `updated_at`.
  - `updated_at` is bumped by almost every shelf PATCH, through `place_item`; only a pure `shelf_position` change skips it (Phase 1: and a pure `rating_visibility` change, `docs/phase1/notes/fup.md` decision 8).
  - So editing an old DNF entry's reason moves it into the current year.
- **Rounding:** Python's `round(x, 2)` rounds half to even on the binary value (probed: 33/8 → `4.12`, 37/8 → `4.62`, 35/8 → `4.38`). JSON prints `5.0` for an integral float.
- **Side effects:** none.

---

### 1.12 `GET /api/pick/{pick_id}/milestones`

- **Auth:** signed in.
- **Path:** `pick_id` (int); a non-int → 400 with the generic message.
- **Errors:** no such pick → 404 `"No club pick with that id"`. Closed picks are allowed.
- **Response 200** `MilestoneListOut {pick_id: int, items: [MilestoneOut], timezone: str}`; items are ordered `position ASC, id ASC`.
- **`MilestoneOut`**, field order `id, pick_id, title, chapter_from, chapter_to, due_at, due_local, due_label, position, note_count, passed`:
  - `due_at`: aware UTC, serialized with `Z`, or null.
  - `due_local`: `"YYYY-MM-DDTHH:MM"` in the club zone, or null.
  - `due_label`: `meeting_label` or null.
  - `position`: int.
  - `note_count`: the number of **`club_pick_posts`** rows with `milestone_id == id`. These are legacy pick notes, **not** diary entries.
  - `passed`: `due_at IS NOT NULL AND due_at <= now()` in UTC.
- Test fixture: an empty list is exactly `{"pick_id": <id>, "items": [], "timezone": "UTC"}`.

### 1.13 `POST /api/pick/{pick_id}/milestones`

- **Body** `MilestoneIn`:
  - `title` (str, required): stripped. Empty → `"Give the milestone a title"`; more than 120 code points → `"Keep the title to 120 characters"`.
  - `chapter_from` (int|null, ≥ 0).
  - `chapter_to` (int|null, ≥ 0).
  - `due_at` (str|null).
- **Logic, in order (probed):**
  1. No such pick → 404 `"No club pick with that id"`.
  2. `pick.ended_at` is set → 400 `"That pick is closed"`.
  3. Both chapters set and `chapter_to < chapter_from` → 400 `"Chapter range is backwards"`. Equal values are fine.
  4. Parse `due_at` with `parse_meeting`. A failure → 400 **`"meeting_at must be a date and time"`**, which names the wrong field (it is copied from picks).
  5. `position = count of the pick's existing milestones`, not max+1.
  6. Insert with `created_by_id = me`.
  7. Record the activity event `milestone_added`: `book_id` = the pick's book, `pick_id` = the pick, payload `{"title": title, "due_at": due.isoformat()}`. `due_at` is omitted when null; it looks like `"2030-07-05T18:00:00+00:00"` (probed).
  8. Commit.
- **Response 201** MilestoneOut.
- **`parse_meeting(value, tz)`** (probed):
  - null, empty or whitespace → null;
  - strip; a trailing `Z` becomes `+00:00`;
  - parse with Python 3.12 `datetime.fromisoformat`, which accepts `2030-01-05`, `2030-01-05T20`, `2030-01-05T20:00`, `2030-01-05 20:00`, `20300105T2000`, seconds and fractions, and `±HH:MM` offsets;
  - a naive result gets the club zone; the result is converted to UTC.
  - Example: `"2030-07-05T20:00"` in Europe/Berlin → `2030-07-05T18:00:00Z`.

### 1.14 `PATCH /api/pick/{pick_id}/milestones/{milestone_id}`

- **Body:** the **same `MilestoneIn`**, so `title` is required and this is a **full replace**.
- **Logic:**
  1. No such pick → 404 `"No club pick with that id"`. This is checked first, even when the milestone exists on another pick (probed with pick 999).
  2. The milestone is missing, or `milestone.pick_id != pick_id` → 404 `"No milestone with that id"`.
  3. Parse `due_at`; an error → 400 `"meeting_at must be a date and time"`.
  4. Set title, chapter_from, chapter_to and due_at from the payload. **Omitted fields become null** (test: an edit without `due_at` clears it).
  5. Commit.
- **Not done here (probed):**
  - **no** closed-pick check (editing on a closed pick works);
  - **no** backwards-range check (`9..1` is accepted);
  - position is unchanged; there is no activity event.
- **Response 200** MilestoneOut.

### 1.15 `DELETE /api/pick/{pick_id}/milestones/{milestone_id}`

1. Pick 404 → milestone 404, with the same strings as PATCH.
2. Set `milestone_id = NULL` on every `club_pick_posts` row that pointed at it; the notes survive.
3. Delete the milestone and flush.
4. Renumber the pick's remaining milestones (ordered `position, id`) to `0..n-1`.
5. Commit.

The response is **204**. There is no closed-pick check and no activity event.

---

### 1.16 `GET /api/search-history`

- **Auth:** signed in.
- **Response 200:** a **bare JSON array** of `SearchHistoryOut {id: int, query: str, last_used_at: naive datetime}`, holding only the caller's rows, ordered `last_used_at DESC, id DESC`. It has no limit; the 20 cap is enforced at write time.

### 1.17 `POST /api/search-history`: remember a query

- **Auth:** signed in.
- **Body** `SearchHistoryIn {query: str}`:
  - The value is `" ".join(query.split())`: Python splits on every `str.isspace` character (NBSP, U+2003, and also U+001C..U+001F), and runs collapse to one ASCII space.
  - After collapsing, the length must be 1..300 **code points**, else 400 `"Search must contain between 1 and 300 characters"`.
- **Logic (one transaction):**
  1. Upsert:
     ```sql
     INSERT (user_id, query, normalized_query = query.casefold(), last_used_at = now)
     ON CONFLICT (user_id, normalized_query) DO UPDATE SET query = <new display form>, last_used_at = now
     ```
     The `id` is preserved on conflict.
  2. Load all of the user's rows ordered `last_used_at DESC, id DESC`.
  3. Delete every row past the 20th.
  4. Commit.
- **Response 200** (not 201): a bare array of the first 20 rows, in the same order as GET.
- **casefold (probed):** `"Straße Nord"` and `"STRASSE NORD"` both normalize to `strasse nord`. The second post kept id 1 and changed the display to `"STRASSE NORD"`.
- The server never records searches on its own; the frontend calls this endpoint after a submitted Discover search.

### 1.18 `DELETE /api/search-history`: clear all

- **Auth:** signed in.
- Deletes all of the caller's rows. Always **204** with an empty body, even when there was nothing to delete.

### 1.19 `DELETE /api/search-history/{entry_id}`

- **Auth:** signed in.
- **Path:** `entry_id` (int); a non-int → 400 generic.
- A missing row, **or another user's row**, → 404 `"Search history entry not found"`. This string is mapped in `apiErrors.ts`.
- Otherwise delete it → **204**.

---

### 1.20 Adjacent endpoints in other areas that read or write this area's tables

- `GET /api/books/work/{work_id}` (books area): `BookDetailsOut.quote_count` is the count of **all** quotes on the book (no user filter). `test_quotes_crud_and_filters` asserts it.
- `GET /api/books/works/{work_id}` (books area): `club_rating`/`rating_count`. `test_diary.py::test_club_rating_averages_finished_readers` tests them.
- `POST /api/pick/posts` and `POST /api/pick/{pick_id}/posts` (pick area) accept `milestone_id`:
  - a missing milestone, or one on another pick, → 400 `"That milestone is not on this pick"`;
  - the post's `milestone_id` feeds `note_count`;
  - `GET /api/pick/posts` returns `milestone_id`.
  - `test_milestones_crud_and_threads` uses these.
- `GET /api/activity` (activity area) renders the `quote_added` and `milestone_added` events written here.

---

## 2. Domain rules

### 2.1 Diary model

- **Entries hang off a book, not a pick.** Anyone can write about any book. The Home pick discussion is "the current pick's book's diary". `pick_id` is only a context marker, set when the entry was written while that book was the current pick. No endpoint reads it back; only the DB test checks it.
- **Position snapshot.**
  - At creation, the entry stores where the author was: `progress_at`/`status_at` from `_position`. Finished is 100, reading is the progress (possibly null), want_to_read and DNF are null progress with their status.
  - The snapshot never updates.
  - `author_rating`, by contrast, is **live**: the author's current rating, shown only while their current status is finished. An old "at 40%" entry shows the author's current stars after they finish (test 4).
- **Spoiler shield** (client-side, `frontend/src/diary.ts`):
  - An entry is shielded when **all** of these hold:
    - it is not deleted;
    - it is not mine;
    - `spoiler_upto != null`;
    - the viewer's status is neither finished nor DNF;
    - `(my_progress ?? 0) < spoiler_upto`.
  - The server's only job is to supply `spoiler_upto`, `mine`, `deleted`, `my_progress` and `my_status`, per diary and per feed item.
  - The server **never hides bodies**.
  - The composer's default `spoiler_upto` is 100 when finished, the progress when reading, else null.
- **Replies:**
  - Exactly one level.
  - The parent must be top-level, live, and on the same book.
  - Replies have their own spoiler flag, reactions and position snapshot.
- **Deletion:** a hard delete when the entry has no replies; a tombstone otherwise.
  - Tombstones are excluded from the feed but shown in the book diary.
  - A tombstone cannot be edited, reacted to, replied to or deleted again; all of these give 404, or 400 for a reply.
  - Reactions are removed in both cases.
- **Edited flag:** set by any PATCH (even a no-op); never shown on tombstones; reactions do not affect it.
- **Counting** (frontend `countEntries`): tombstones count as 0 and replies count as 1 each.

### 2.2 Feed pagination

- Keyset on the integer id: `id < before`, `ORDER BY id DESC`, `limit+1` to compute `has_more`.
- The frontend passes the last item's `entry.id` as `before`.

### 2.3 Quotes

- They are personal rows but visible to everyone in the instance.
- No dedupe: the same body can be saved twice.
- `page` is a free integer with no edition context.
- Creating a quote for an unknown book creates the book from the client's metadata.

### 2.4 Stats

- Everything is computed in Python in memory on each request. There is no cache; it loads all finished and DNF shelf entries, all users and all quotes.
- Year boundaries are **UTC**, not the club timezone. For example, a book finished at 00:30 on 1 January Berlin time counts in the previous year.
- Means are `round(sum/len, 2)` with banker's rounding on the binary float.
- Only truthy ratings count: ratings are validated 1..5 elsewhere, so this just drops nulls.

### 2.5 Milestones

- They form an ordered reading schedule per pick.
- `position` is 0-based. A new milestone gets the current count, and a delete renumbers to stay dense. There is no reorder endpoint.
- Chapters are free integers ≥ 0; either or both may be null.
- `due_at` is entered in club-local time; a naive input is interpreted in `BOOKCLUB_TZ`, and offsets are honored.
- `passed` is computed at read time.
- POST is forbidden on closed picks, but PATCH and DELETE are not.
- Milestone threads use the **legacy pick posts** (`club_pick_posts.milestone_id`). Diary entries cannot be attached to a milestone.

### 2.6 Search history

- Private per user, with at most 20 rows, LRU by `last_used_at`.
- Dedupe is case-insensitive via **Unicode casefold** after whitespace collapsing.
- The latest display form wins.
- `id` is stable across re-use, so the frontend can delete by id.

### 2.7 Activity events written by this area

`events` rows are append-only; nothing here deletes them.

| Kind | Columns | Payload |
|---|---|---|
| `quote_added` | `actor_id`, `book_id` | `{"page"?: int, "excerpt": str (≤ 80 code points)}` |
| `milestone_added` | `actor_id`, `book_id`, `pick_id` | `{"title": str, "due_at"?: "<isoformat +00:00>"}` |

- Null payload values are dropped (`activity.record`).
- The payload is JSON with `ensure_ascii=False`.
- Diary entries and reactions write **no** events.

### 2.8 Caching, rate limits, outbound HTTP

- None in this area: no module-level caches, no rate limits and no outbound HTTP.
- `test_local_reads` asserts that the diary GET/POST, the feed, quotes and stats never contact Open Library.

---

## 3. Data

The SQLite DDL is exactly what `create_all` produces (probed via `sqlite_master`). All `DATETIME` columns hold naive UTC text such as `'2026-10-27 10:58:49.471065'`. `PRAGMA foreign_keys=ON`. **No `ON DELETE` actions** on any table here.

### 3.1 `book_posts` (diary entries; read and written)

```
id INTEGER PK
book_id INTEGER NOT NULL → books.id         (ix_book_posts_book_id)
author_id INTEGER NOT NULL → users.id       (ix_book_posts_author_id)
parent_id INTEGER NULL → book_posts.id      (ix_book_posts_parent_id)   NULL = top-level
body VARCHAR(1000) NOT NULL                 "" for tombstones
spoiler_upto INTEGER NULL                   percent 0..100; NULL = no spoiler flag (and always NULL on tombstones)
progress_at INTEGER NULL                    percent snapshot; 100 when author had finished
status_at VARCHAR(17) NULL                  ShelfStatus name (== value) snapshot; NULL = author had no shelf entry
pick_id INTEGER NULL → club_picks.id        the current pick at write time if it was this book
legacy_post_id INTEGER NULL UNIQUE          club_pick_posts.id this row was imported from; NULL for native entries
deleted_at DATETIME NULL                    tombstone marker
created_at DATETIME NOT NULL                (ix_book_posts_created_at); legacy imports copy the old created_at
updated_at DATETIME NULL                    set by every PATCH; legacy imports copy the old updated_at
```

The model's invariants are enforced in code only:
- `parent_id` always points at a top-level row on the same `book_id`.
- A row with replies is never hard-deleted.

### 3.2 `book_post_reactions` (read and written)

```
id INTEGER PK, post_id → book_posts.id (indexed), user_id → users.id,
emoji VARCHAR(16) NOT NULL, created_at DATETIME NOT NULL,
UNIQUE (post_id, user_id, emoji) "uq_book_reaction_post_user_emoji"
```

### 3.3 `quotes` (read and written)

```
id INTEGER PK, user_id → users.id (indexed), book_id → books.id (indexed),
body VARCHAR(1000) NOT NULL (stripped, 1..1000 code points), page INTEGER NULL (0..20000; NULL = no page),
created_at DATETIME NOT NULL
```

There is no updated_at.

### 3.4 `search_history` (read and written)

```
id INTEGER PK, user_id → users.id (indexed), "query" VARCHAR NOT NULL (display form, whitespace-collapsed),
normalized_query VARCHAR NOT NULL (casefold of query), last_used_at DATETIME NOT NULL,
UNIQUE (user_id, normalized_query) "uq_search_history_user_query"
```

There is no created_at. `test_migrations` asserts the table exists, with these columns, on an upgraded legacy DB.

### 3.5 `pick_milestones` (read and written)

```
id INTEGER PK, pick_id → club_picks.id (indexed), title VARCHAR(120) NOT NULL (stripped),
chapter_from INTEGER NULL, chapter_to INTEGER NULL (both ≥0; NULL = open/unspecified),
due_at DATETIME NULL (UTC), position INTEGER NOT NULL (0-based dense), created_by_id → users.id NOT NULL,
created_at DATETIME NOT NULL
```

### 3.6 Read or written, but owned by other areas

| Table | Columns used | How |
|---|---|---|
| `club_pick_posts` | `milestone_id` (written by milestone delete, set to NULL; counted for `note_count`); the full row (`pick_id, author_id, body, spoiler_upto, created_at, updated_at`) | Read by the legacy import |
| `post_reactions` | `post_id, user_id, emoji, created_at` | Read by the legacy import |
| `shelf` | `id, user_id, book_id, status, progress, rating, finished_at, updated_at` | Read for positions, author_rating and stats. Unique `(user_id, book_id)`, so one shelf entry per user per book |
| `books` | `id, ol_work_key, title, authors, cover_id, cover_image_url, year, pages, spine_color` | Read. `upsert_book` inserts, or updates title / non-empty authors / covers / non-null year and resets `spine_color` to NULL when the cover changed |
| `users` | `id, username, avatar_updated_at` | Read. Stats list **all** users |
| `club_picks` | `id, book_id, set_by_id, started_at, ended_at` | Read. Current pick = newest `ended_at IS NULL` by id |
| `events` | `actor_id, kind, book_id, pick_id, payload` (JSON text), `created_at` | Written (§2.7) |

### 3.7 Data migrations in `db.py` touching this area

**`import_legacy_pick_posts(engine)`** runs on **every boot**, after `create_all` and `COLUMN_MIGRATIONS`.
1. It reads every `club_pick_posts` row joined to its `club_picks` row, ordered `created_at, id`.
2. It skips rows whose id already appears in `book_posts.legacy_post_id`.
3. Every other row becomes a top-level `book_posts` row:
   - `book_id` = the pick's book;
   - `author_id`, `body`, `spoiler_upto`, `created_at`, `updated_at` copied;
   - `pick_id` = the pick;
   - `legacy_post_id` = the old id;
   - `progress_at` and `status_at` NULL.
4. Its `post_reactions` are copied into `book_post_reactions` with the same `created_at`.
5. It returns the number imported; it is idempotent (test 13).

Consequences:
- Legacy `club_pick_posts.milestone_id` is **not** carried to the diary.
- A legacy note written *after* the last boot (through the still-existing `/api/pick/posts`, which the frontend no longer calls) has no diary copy until the next boot.
- Edits or deletes of a legacy pick post after import do not propagate to its diary copy.

**`COLUMN_MIGRATIONS`**:
- `club_pick_posts` gets `spoiler_upto`, `milestone_id` and `updated_at`.
- `book_posts`, `book_post_reactions`, `quotes`, `search_history` and `pick_milestones` have no column migrations; they shipped whole.

---

## 4. Tests (parity checklist; 22 tests)

### backend/tests/test_diary.py (15)

- `test_diary.py::test_requires_auth_and_valid_work_id`: signed-out GET of `/api/books/works/OL1W/posts` → 401. Signed in, GET and POST (`{"body":"x"}`) on `/api/books/works/nope/posts` → 404.
- `test_diary.py::test_diary_on_an_unknown_book_is_empty_not_missing`: GET on a valid id with no book row → 200 with `ol_work_key "/works/OL1W"`, `book_id` null, `items []`, `my_progress` null, `my_status` null.
- `test_diary.py::test_first_entry_creates_the_book_when_given_its_details`:
  - POST with no `book` on an unknown book → 400;
  - with `book` → 201, the body stripped (`"  Loved the opening  "` → `"Loved the opening"`), `author "ada"`, `mine` true, `progress_at`/`status_at` null, `created_label` non-empty;
  - the diary then has a `book_id` and that one item;
  - the writer's `/api/shelf` stays empty.
- `test_diary.py::test_entry_snapshots_where_the_author_was`:
  - Shelved reading at 40% → the entry has `progress_at 40`, `status_at currently_reading`, `spoiler_upto 40`, `author_rating` null.
  - After finishing with rating 4 → a new entry has `progress_at 100`, `status_at finished`, `author_rating 4`.
  - In the diary the first entry keeps `progress_at 40` but shows the live `author_rating 4`; `my_progress 100`, `my_status finished`.
- `test_diary.py::test_viewer_position_drives_the_spoiler_shield_fields`: ada (finished, rated 5) posts with `spoiler_upto 100`. Grace, reading at 25%, sees `my_progress 25`, `my_status currently_reading`, `items[0].spoiler_upto 100`, `mine` false.
- `test_diary.py::test_replies_are_one_level_and_stay_on_the_book`:
  - grace's reply → 201 with `parent_id` = root;
  - a reply to the reply → 400;
  - a reply to the root posted under `/works/OL2W/posts` (with a `book` payload) → 400;
  - the diary has 1 top-level item whose replies are `["Yes!"]`, authored by `"grace"`.
- `test_diary.py::test_only_the_author_edits_or_deletes`:
  - a non-author's PATCH and DELETE → 403;
  - the author's PATCH `{}` → 400;
  - PATCH `{body:"Final", spoiler_upto:60}` → body, `spoiler 60`, `edited` true;
  - PATCH `{spoiler_upto:null}` → spoiler null;
  - DELETE → 204 and the diary is empty (a hard delete);
  - a second DELETE → 404.
- `test_diary.py::test_deleting_an_entry_with_replies_leaves_a_tombstone`:
  - The root has `spoiler 50` and a reaction; grace replies; the author deletes the root → 204.
  - The diary item has `deleted` true, `body ""`, `spoiler_upto` null, `reactions []`, replies `["Reply"]`.
  - PATCH and react on the tombstone → 404; a new reply to it → 400.
  - `/api/diary` lists only `"Reply"`.
- `test_diary.py::test_reactions_toggle_and_aggregate`:
  - an emoji outside the set → 400;
  - the first toggle → `[{"emoji":"<U+2764 U+FE0F>","count":1,"mine":true,"users":["ada"]}]`;
  - grace toggles the same emoji → count 2, `mine` true, `users ["ada","grace"]`;
  - grace toggles again → count 1, `mine` false, `users ["ada"]`.
- `test_diary.py::test_entry_on_the_current_pick_records_the_pick`: with Circe as the current pick, a Circe entry stores `pick_id` = that pick (checked in the DB); an entry on another book stores `pick_id` NULL.
- `test_diary.py::test_feed_exposes_viewer_position_for_spoiler_shield`:
  - Each feed item carries the viewer's own position on that item's book.
  - ada, reading at 80 → `80/currently_reading`, `mine` true, the spoiler body returned unredacted.
  - An unshelved book → `null/null`.
  - grace, unshelved → `null/null`, `mine` false, `spoiler_upto 70`.
  - grace reading at 25 → 25; then finished → `100/finished`; then DNF → `null/did_not_finish`.
- `test_diary.py::test_feed_is_newest_first_across_books_and_pages`:
  - `limit=2` → bodies `["Circe reply","Achilles 1"]`, books `["Circe","The Song of Achilles"]`, `parent_author` `"ada"` then null, `has_more` true.
  - `before=<last id>` → `["Circe 1"]` with `has_more` false.
- `test_diary.py::test_legacy_pick_notes_are_imported_once_with_reactions`:
  - A legacy `/api/pick/posts` note (`spoiler 30`) with a U+1F525 reaction.
  - `import_legacy_pick_posts` returns 1, then 0.
  - The diary shows it with `spoiler 30`, `author ada` and the reaction `{"emoji":"<U+1F525>","count":1,"mine":true,"users":["ada"]}`.
  - The DB has exactly one BookPost (with `pick_id` and `legacy_post_id` set) and one BookPostReaction.
- `test_diary.py::test_club_rating_averages_finished_readers` (really the book-details area):
  - `GET /api/books/works/OL1W` gives `club_rating 4.5` and `rating_count 2` for ratings 5 and 4; a reader with no rating does not count.
  - A fresh book `OL2W` (OL faked) gives `club_rating` null and `rating_count 0`.
- `test_diary.py::test_custom_book_has_a_diary_without_open_library`:
  - A `POST /api/books/custom` book (`BC…` id) has an empty diary whose `ol_work_key` ends with the id.
  - A POST without a `book` payload → 201, since the book row exists.
  - The entry is listed. `httpx.AsyncClient` is never constructed.

### backend/tests/test_stats_milestones_quotes.py (4)

- `test_stats_milestones_quotes.py::test_stats_year_in_review`. Setup:
  - ada finishes Circe (rating 5, 393 pages) and Achilles (rating 3, 352 pages, `finished_at` moved to last year), and DNFs Dune.
  - Circe is set as the pick, and ada quotes Circe.

  The default year (this year) gives:
  - `years [this, this-1]`;
  - members[0] = ada with finished 1, dnf 1, pages 393, avg 5.0, five_stars 1, top/longest Circe, quotes 1;
  - club_finished 1, club_pages 393, club_avg 5.0;
  - one `by_month` row with finished 1;
  - `picks[0]` = Circe with finished 1, avg 5.0, `ratings [5]`;
  - `best_pick` Circe.

  `year=this-1` gives finished 1, top_book Achilles, club_pages 352.
- `test_stats_milestones_quotes.py::test_stats_with_empty_club`: with one user and no data, `members[0]` is exactly `{"username":"ada","finished":0,"dnf":0,"pages":0,"average_rating":null,"five_stars":0,"top_book":null,"longest_book":null,"quotes":0}`, `best_pick` null, `by_month []`.
- `test_stats_milestones_quotes.py::test_milestones_crud_and_threads`:
  - The empty list is exactly `{"pick_id":id,"items":[],"timezone":"UTC"}`.
  - Create "Part one" (ch 1–8, due `2030-01-05T20:00`) → 201, position 0, `due_local "2030-01-05T20:00"`, `passed` false, `note_count 0`.
  - Create "Part two" (9–16) → position 1, `due_at` null.
  - An empty title → 400; backwards 9→2 → 400.
  - A pick post with `milestone_id` = first → 201 echoing `milestone_id`; with `milestone_id 9999` → 400; the list shows `note_count 1`.
  - PATCH the first with title "Part 1" and no `due_at` → 200, title changed, `due_at` null.
  - DELETE the first → 204; the remaining `["Part two"]` is at position 0; the pick post now has `milestone_id` null.
  - `GET /api/pick/999/milestones` → 404; DELETE of an unknown milestone → 404.
  - After a new pick closes Circe, POST → 400.
  - The activity kinds include `milestone_added`.
- `test_stats_milestones_quotes.py::test_quotes_crud_and_filters`:
  - An unknown book without a title → 400.
  - Create (`"  Humbling women.  "`, page 12) → 201 with stripped body, page 12, `mine` true, `author ada`, `book.title Circe`.
  - A second quote on Achilles; the list has 2.
  - `work=/works/OL1W` → only Circe; `work=/works/OL99W` → `[]`.
  - PATCH `{page:null}` → 200 with page null; PATCH `{}` → 400.
  - `GET /api/books/work/OL1W` → `quote_count 1`.
  - As grace: `username=ada` items have `mine` false; PATCH and DELETE of ada's quote → 403; `username=grace` → `[]`.
  - As ada: DELETE → 204, then 404.
  - The activity kinds include `quote_added`.

### backend/tests/test_search_history.py (3)

- `test_search_history.py::test_history_requires_login`: signed out, GET, POST (`{"query":"Dune"}`) and DELETE `/api/search-history` all → 401.
- `test_search_history.py::test_history_deduplicates_orders_and_caps`:
  - 22 posts `"Book 0".."Book 21"`, each 200.
  - GET has 20, newest first (`"Book 21"`), last `"Book 2"`.
  - Re-posting `"  BOOK   2  "` → the response's first item has the **same id** as `"Book 2"`, `query "BOOK 2"`, and the length is still 20.
  - `"  "` → 400; 301 × `"x"` → 400.
- `test_search_history.py::test_history_is_private_and_delete_is_scoped`:
  - ada saves "Circe"; bob registers (now signed in as bob).
  - bob: GET → `[]`; DELETE of ada's entry id → 404; posts "Dune"; clear-all → 204; GET → `[]`.
  - Logged in as ada: "Circe" is still first; deleting it by id → 204; GET → `[]`.

### Related tests elsewhere (owned by other areas, listed so nothing is lost)

- `test_local_reads.py::test_known_book_reads_never_touch_open_library` covers diary POST/GET, `/api/diary`, `/api/stats` and `/api/quotes` (POST and GET) without OL contact.
- `test_migrations.py::test_legacy_database_is_upgraded_in_place` covers the existence of the `search_history`, `quotes` and `pick_milestones` tables and the `club_pick_posts.milestone_id` column.

---

## 5. Multi-club mapping

Labels:
- **[plan]**: decided in plan.md or the issues (#37, #38, #44, #45, #46, #48);
- **[proposal]**: a recommendation for the implementer;
- **[open]**: not decided; see §7.

### 5.1 Personal data (no club_id; visibility through the central policy of #46)

**Diary entries (`book_posts` → `diary_entries`)**

Ownership and keys:
- Personal [plan #37/#44].
- They carry `visibility ∈ {audience, only_me}` [plan #37/#46]; the default is `audience` [proposal].
- They point to a **work**, not an edition [plan: works have many editions]. Percent positions stay meaningful across editions.

Reading a work's diary:
- The diary returns the viewer's own entries plus entries whose author is in the viewer's audience and whose visibility is `audience`.
- The audience is the members of any club shared with the author; friends join in Phase 3 [plan #46].
- Strangers' entries are never returned.

Replies [open]:
- A reply should be visible only when the viewer can see **both** the root and the reply's author [proposal].
- Whether a reply has its own visibility or inherits the root's is open.
- Replying requires being able to see the root. Otherwise the parent check should look like "not found" (`"That entry is not on this book"` today), never reveal existence [proposal].

Reactions:
- Anyone who can see the entry may react [proposal].
- Reaction `users` lists must not leak strangers' usernames: a co-member of the author in another club may be a stranger to the viewer.
- So count every reaction but list only usernames in the viewer's audience, or self [proposal; open]. `mine` stays exact.

Fields that change:
- `author_rating` must respect rating visibility. If the author's rating is `only_me` and the viewer is not the author, return null [plan #46: ratings can be only me].
- `pick_id` stops making sense on a personal entry: a user can be in several clubs with the same current pick. Either drop it, or keep a nullable `club_pick_id` that is shown only inside that club [open].
- Python never reads `pick_id` back through the API, so dropping it breaks no endpoint, only `test_entry_on_the_current_pick_records_the_pick` (to be reasoned).

Errors and the feed:
- Author-only edit and delete: keep 403 for an entry the viewer can see but did not write. Return **404** for an entry the viewer cannot see, so existence isn't revealed [proposal, consistent with #40's "404 never 403" for club resources].
- `GET /api/diary` becomes the **combined home feed** [plan #48: "Home: one feed across all the user's clubs"]: entries visible to the viewer, newest first, same keyset paging.
  - A per-club variant, limited to authors who are members of that club, is a natural fit for the club page's "recently written" [proposal; open].
  - Tombstones stay excluded.

**Reactions on diary entries:** personal side table (no club_id).

**Quotes**
- Personal, with `visibility` [plan #37/#46].
- `GET /api/quotes` returns only quotes the viewer may see.
- The `username` filter returns `{"items": []}` for unknown users **and** for strangers, so membership can't be probed [proposal]; Python already returns an empty list for unknown users.
- `quote_count` on book details counts only quotes visible to the viewer [proposal].
- `page` depends on the edition. Either quotes get an optional `edition_id`, defaulting to the user's shelf edition for that work, or `page` stays a free int with no edition semantics [open].
- The `quote_added` activity event is a club event in Python. Under the plan, events are club data [plan], and a personal quote belongs to no club [open]: either no event, fan-out to each of the user's clubs (with the visibility check at write time), or a derived "personal activity" in the home feed.

**Search history**
- Personal and unchanged [plan #37/#44].
- It is part of the DSGVO export and is deleted with the account [plan #39].
- It is never visible to anyone else (it is not in the #46 audience list).

**Per-person stats**
- Personal [plan #44], and visible to the audience [plan #46].
- Proposed shape: `GET /api/users/{username}/stats?year=` (or `/api/me/stats`), returning the `MemberYear` fields plus `years` and `by_month` for that one user.
- For a viewer other than the owner:
  - exclude `only_me` ratings from `average_rating`, `five_stars` and `top_book`;
  - exclude `only_me` quotes from `quotes` [proposal; required by #46 to avoid leaks].
- A stranger gets 404 [proposal].

### 5.2 Club-scoped data (club_id FK; non-members get 404 [plan #40])

**Milestones (`pick_milestones` → `milestones`, via `pick_id` → club pick)**
- Club data [plan].
- They are **percent or chapter** instead of chapter ranges [plan #37/#45]. Proposed columns: `unit ∈ {percent, chapter}` (nullable for title-only milestones), `range_from`, `range_to` (≥ 0; ≤ 100 when the unit is percent), `title`, `due_at`, `position`, `created_by`.
- The importer maps legacy `chapter_from/chapter_to` to `unit = chapter`. A row where both are null gets a null unit [plan #45 "Legacy milestones are mapped by the importer"; detail is a proposal].
- Path [proposal]: `/api/clubs/{slug}/picks/{pick_id}/milestones[/{milestone_id}]`.
  - A pick from another club → 404 `"No club pick with that id"`.
  - A non-member → 404 on everything.
- Roles [open]: Python lets any member create, edit or delete. #40 gives admins "picks", which suggests admin or owner for milestone writes and members for reads. Decide and add a 403 string if needed.
- `due_at` timezone: the club's timezone setting replaces `BOOKCLUB_TZ` [open in auth.md §5.4: per-club or per-user].
- `note_count` counts the club's pick posts with that `milestone_id`, unchanged in meaning.
- `milestone_added` is a club event (club_id).

**Club stats**
- [plan #45: "club stats, computed per club"]
- Proposed path: `GET /api/clubs/{slug}/stats?year=`.
- `members` = the club's members, not all users.
- `picks` = that club's picks.
- `readers`/`finished`/`dnf`/`ratings` for a pick count only **club members'** shelf entries for the pick's **work**, and only ratings visible to the audience (club members are always audience, but `only_me` ratings must be excluded) [proposal].
- `club_*` totals sum over members.
- `best_pick` is computed within the club.

**Pick discussion vs. the diary [open, important]**
- Python merged the pick discussion into the per-book diary (`import_legacy_pick_posts`, "The current pick's discussion on Home is simply that book's diary").
- The plan lists both "diary entries + reactions" (personal) and "pick posts + reactions" (club) [plan #37/#38/#45].
- Decide whether the club's pick discussion is:
  - (a) club-scoped pick posts, separate from personal diary entries; or
  - (b) personal diary entries on the pick's work, filtered to the club's members.

### 5.3 Works vs editions, the catalog, and Google removal

- **Work id:** the `/api/books/works/{work_id}/posts` path and `ol_work_key` fields carry the legacy key formats (`OL…W`, `ISBN…`, `GB…`, `BC…`). The new catalog has its own work ids [plan #41].
  - `DiaryOut.ol_work_key`, `book_id`, `QuoteIn.ol_work_key` and `GET /api/quotes?work=` all change to the new id [proposal: `work_id`].
  - `canonical_work_id` and the regex only matter to redirect old routes [plan #48 "Old routes redirect"].
- **Creating books from client metadata goes away.**
  - Today the diary's `book` payload and the quote's `title/authors/cover_id/cover_url/year` insert or update `books` rows from whatever the client sends.
  - Under the own catalog, works enter through search, scan or shelving and the user-add flow [plan #41]. Diary and quote creation should require an existing work id, else 404 [proposal].
  - The strings `"Nobody has this book yet; send its title to start the diary"`, `"title is required for a new book"` and `"ol_work_key must be an Open Library work key"` disappear, along with the Google `cover_url` acceptance [plan: Google Books dropped].
- **Pages:** stats `pages` and `longest_book` use `books.pages`. Pages now live on **editions**.
  - Per-person stats should use the shelf entry's edition pages when set, else a work-level fallback (e.g. the most common edition's pages) or zero [open].
- **BookOut** in feed items, quotes and stats becomes the new work DTO. Its `cover_url` points at our own media domain, never `covers.openlibrary.org` [plan #43].
- **Spoiler and progress percent:** unchanged; percent is edition-independent.

### 5.4 Importer (#38) items for this area

| Legacy | New |
|---|---|
| `book_posts` (all, including tombstones) | `diary_entries` with visibility `audience`. Keep `deleted_at`, `parent_id` (mapped), `progress_at`, `status_at`, `spoiler_upto`, `created_at`, `updated_at`. Map `pick_id` to the imported club's pick (if kept). Insert in **id order** so new ids preserve the feed order, which is id-based |
| `book_post_reactions` | Diary reactions |
| `quotes` | `quotes` with visibility `audience` |
| `search_history` | As is, `normalized_query` kept |
| `pick_milestones` | Club milestones (chapter unit) |
| `events` with kind `quote_added`/`milestone_added` | The imported club's events |

- **Do not double-import pick notes.** Every `club_pick_posts` row with a `book_posts.legacy_post_id` match is already a diary entry.
  - If the importer also creates club pick posts from `club_pick_posts` [plan #38 lists "pick posts + reactions"], the same text appears twice [open; see 5.2].
  - The importer should run the equivalent of `import_legacy_pick_posts` itself for any `club_pick_posts` row without a diary copy, or import those rows only as pick posts.
- **Google-keyed books** (`ISBN…`/`GB…`) referenced by diary entries and quotes must resolve to the re-matched work, or to the bare-facts work [plan #38].

### 5.5 What no longer makes sense (record a reason in #49)

- Instance-wide visibility of every diary entry, quote and stat ("everyone in the DB is the club").
- `/api/stats` listing **all** users.
- Creating or updating books from client-supplied metadata in diary and quote writes.
- Google cover URLs.
- `BOOKCLUB_TZ` as the one timezone for labels and milestone parsing.
- `book_posts.pick_id` as a single-club context.
- `test_diary_on_an_unknown_book_is_empty_not_missing`: the parity can stay for works that exist in the catalog but have no entries. A truly unknown work id should become 404 [proposal].
- `test_legacy_pick_notes_are_imported_once_with_reactions`: becomes an importer test, since the boot-time data migration disappears with SQLite.
- `test_club_rating_averages_finished_readers`: belongs to book details; becomes club- or audience-scoped there.
- `test_custom_book_has_a_diary_without_open_library`: `BC…` books become user-added catalog works [plan #38/#41]. The assertion "no outbound HTTP for a known work" carries over.

---

## 6. Port risks

**R1. Two different orderings.**
- The book diary orders top-level entries and replies by `created_at ASC, id ASC`. Legacy-imported rows have old `created_at` values and newer ids.
- The feed orders by `id DESC` and pages with `id < before`.
- Don't unify them.

**R2. Tombstone shape.**
- A tombstone keeps `author`, `author_avatar_url`, `mine`, `progress_at`, `status_at`, `parent_id`, `created_at` and `created_label`.
- `author_rating` is still computed live for it.
- It forces `body ""`, `spoiler_upto null`, `reactions []`, `edited false`.
- In the book diary it still carries its `replies`.
- It is excluded from the feed, but its replies are not, and they show `parent_author` = the tombstone's author.
- **It stays forever** after its last reply is deleted (probed).
- Every write against it gives 404 (edit, react, delete) or 400 `"That entry was deleted"` (reply).

**R3. Delete decides between hard delete and tombstone** by "does any row have `parent_id = id`".
- It deletes the post's reactions first in both cases.
- In Postgres, give `parent_id` no `ON DELETE CASCADE`, or a reply race could remove replies [proposal: FK `ON DELETE RESTRICT`].
- Do the existence check and the delete in one transaction.

**R4. Order of checks on POST entry.**
1. Body validation (400).
2. Work-id regex (404 `"No such book."`, with the period).
3. Book lookup or create (400 when `book` is missing).
4. Parent checks, in the exact order: missing or other book → not a top-level → deleted.

The created book must roll back when a later check fails.

**R5. An existing book ignores the `book` payload** (diary).
- For quotes the lookup uses the stripped raw key, but `upsert_book` re-normalizes it to `/works/<last segment canonical>`. A non-canonical key can therefore **update** an existing book's metadata.
- With the new catalog this path disappears (§5.3). For strict parity tests, replicate it.

**R6. Order of checks on PATCH entry and quote.**
1. Validation.
2. 404 (missing or tombstone).
3. 403 (not the author).
4. 400 `"Nothing to update"`.

A non-author sending `{}` gets **403**, not 400.

**R7. Tri-state fields.**
- `spoiler_upto` (entry PATCH) and `page` (quote PATCH): absent = keep, explicit `null` = clear.
- `body: null` counts as absent.
- In serde use `Option<Option<i32>>` with `#[serde(default, deserialize_with = …)]` (or `serde_with::rust::double_option`). Plain `Option` loses the distinction, and test 7 (`{"spoiler_upto": null}` clears) and the quotes test (`{"page": null}` clears) fail.

**R8. PATCH entry bumps `updated_at` unconditionally**, so `edited` becomes true even when nothing changed. Reactions never touch `updated_at`.
- **Phase 1:** R8 holds for every PATCH that sends `body` or `spoiler_upto`. A PATCH with only `visibility` (new in Phase 1) leaves `updated_at` as it is (`docs/phase1/notes/fup.md` item C).

**R9. Response shape differs by endpoint.**
- PATCH and reaction on a **top-level** entry return its full `replies`; on a reply, `[]`.
- POST returns `replies: []`.
- Feed entries always have `replies: []`.

**R10. `_position` mapping.**
- `finished` → 100, even when `shelf.progress` holds something else.
- `currently_reading` → the raw progress, possibly null.
- `want_to_read` and `did_not_finish` → null progress with the status kept.
- No shelf entry → both null.
- The snapshot is taken from the **author's** shelf at write time. `my_*` is the **viewer's** shelf at read time.

**R11. `author_rating` is live and only for finished authors**:
- null while the author is reading or DNF, even if they once rated it;
- a want_to_read entry with a leftover rating can't occur, because `apply_finish_fields` clears it.

**R12. Reaction details.**
- The emoji is stripped, then compared exactly. U+2764 alone is invalid; U+2764 U+FE0F is valid.
- Groups sort by `(-count, emoji)`. Rust `String` `Ord` (byte-wise UTF-8) equals code-point order, so `sort_by(|a, b| b.count.cmp(&a.count).then(a.emoji.cmp(&b.emoji)))`.
- `users` are sorted ascending.
- Make the toggle race-safe in Postgres, since Python raises a 500 on a concurrent duplicate: `DELETE … RETURNING`, and if nothing was deleted, `INSERT … ON CONFLICT DO NOTHING`.

**R13. Length and whitespace semantics.**
- Python `len()` counts code points: use `chars().count()`, never `len()`. This applies to the 1000-character post and quote bodies, the 120-character milestone titles, and the 300-character search queries.
- Python's `strip()` and `split()` treat U+001C..U+001F as whitespace; Rust's `trim()` and `split_whitespace()` do not (probed). The difference is negligible, but write a helper if exact parity is wanted.
- `DiaryBookIn.title` is silently truncated to 500 code points.
- The `quote_added` excerpt is `body[:80]` in code points: `chars().take(80)`.

**R14. Lax numbers.** pydantic accepts `"50"`, `true` and `50.0` for ints (probed). serde does not. The frontend always sends real numbers, so record strict parsing as an intentional change in #49 rather than emulate it.

**R15. Framework-default errors must be remapped to the Python strings and status 400:**
- axum's `Path`/`Query`/`Json` rejections: `"Input should be a valid integer, unable to parse string as an integer"`, `"Input should be greater than or equal to 1"`, `"Input should be less than or equal to 50"`, `"Field required"`, …
- Validators must run in field-declaration order and join with `"; "` (probed multi-error strings in §1.0 and §1.8).
- Body validation errors must win over the handler's 404 for a bad work id.

**R16. Timestamps.**
- Python emits **naive** ISO strings without offset, e.g. `"2026-10-27T10:58:18.198238"`.
- The frontend's `formatClubDate` does `new Date(iso)`. Per ECMAScript, that string is parsed as **local** time, so non-UTC browsers already show diary times shifted: a latent bug.
- Emit RFC 3339 UTC with `Z` from Rust [proposal]. That is an intentional, beneficial change; record it in #49.
- `MilestoneOut.due_at` already uses `Z`.
- Keep microsecond precision: Postgres `timestamptz` has microseconds.

**R17. `created_label`/`due_label` formatting** needs IANA tz data and `%Z` abbreviations (`UTC`, `CET`, `CEST`, or numeric like `-03` for zones without letters).
- sqlx is built with the `time` feature, and `time` has no tz database. Use `chrono-tz` (or `jiff`) for conversion and the abbreviation.
- Format `%a %d %b %Y, %H:%M %Z` with English names regardless of locale, and a zero-padded day.

**R18. `parse_meeting` edge cases.**
- It accepts the full Python 3.12 `fromisoformat` grammar (date only → midnight, hour only, space separator, basic format, `Z`, offsets). The frontend sends `YYYY-MM-DDTHH:MM` from `datetime-local` inputs, so support at least that, date-only, and RFC 3339 with offset or `Z` [proposal: document the narrowing].
- A naive local time inside a DST **gap** or **overlap**: Python's zoneinfo uses `fold=0`, which takes the offset **before** the transition. chrono's `from_local_datetime` returns `None` or `Ambiguous` and must be resolved the same way. Example (probed): in Europe/Berlin, `2030-03-31T02:30` becomes `01:30Z`, and `2030-10-27T02:30` becomes `00:30Z`, the first, CEST, occurrence.
- The error string is `"meeting_at must be a date and time"` even for milestones.

**R19. Milestone asymmetries.**
- POST checks closed pick, then backwards range, then the due date.
- PATCH checks none of the first two, and it is a **full replace**: omitted `due_at` and chapters become null, and `title` is required.
- DELETE detaches pick posts and renumbers to a dense 0..n-1.
- A new position = **count**, not max+1.
- The 404 order is pick first (`"No club pick with that id"`), then milestone (`"No milestone with that id"`).
- `passed` uses `<=` now.

**R20. `note_count` counts pick posts (`club_pick_posts`), not diary entries.** Under the new model it must count whatever table holds club pick posts with `milestone_id`.

**R21. Stats rounding.**
- Python's `round(x, 2)` rounds half to even on the binary double: 4.125 → 4.12, 4.625 → 4.62, 4.375 → 4.38 (probed).
- `(x * 100.0).round() / 100.0` gives 4.13 and 4.63 (probed in Rust 1.99), which is **wrong**.
- `format!("{:.2}", x).parse::<f64>()` matches Python (probed).
- **Don't** round in SQL: Postgres `round(numeric, 2)` rounds half away from zero on exact decimals.
- Serialize integral means as `5.0`; serde_json prints `5.0` for an `f64` 5.0.

**R22. Stats year attribution.**
- The **UTC** year of `finished_at`, falling back to `updated_at`. DNF always uses `updated_at`, which moves with every shelf edit.
- `years` always includes the current UTC year and is sorted descending.
- The default `year` is `years[0]`.
- An explicit year outside the list is allowed, within 2000..2100.

**R23. Stats tie-breaks and scopes.**
- `top_book`: max `(rating, shelf.id)`.
- `longest_book`: first max by pages in shelf-id order. Add an explicit `ORDER BY shelf.id` because Python relied on SQLite rowid order.
- Members sort by `(-finished, username)`.
- Pick stats are **all-time**. `readers` counts any status, including want_to_read.
- `best_pick` is the max `(avg, len(ratings))`, with the first in `id DESC` order winning a tie.
- `by_month` is club-wide and sorted ascending.
- Only truthy ratings count.

**R24. Quotes list.**
- `work` is an exact key match, with no canonicalization.
- `username` is lowercased but not stripped.
- Empty strings are ignored.
- An unknown user or work gives an empty list, never 404.
- Default limit 100, max 500; order `id DESC`.

**R25. Search history.**
- Normalize with `" ".join(q.split())`, then check 1..300 **code points**.
- Dedupe on Unicode **casefold**. Rust `to_lowercase()` keeps `ß` (probed), so use a full case-fold (e.g. the `caseless` crate's `default_case_fold_str`), or `straße`/`STRASSE` duplicates appear.
- Postgres `lower()` is not casefold either.
- The upsert, the prune beyond 20 (order `last_used_at DESC, id DESC`) and the response must happen in one transaction. Use `INSERT … ON CONFLICT (user_id, normalized_query) DO UPDATE SET query = EXCLUDED.query, last_used_at = EXCLUDED.last_used_at` so parallel tabs don't conflict.
- POST returns **200** and a **bare array**.
- Deleting another user's id → 404 with the mapped string `"Search history entry not found"`. Clear-all is always 204.

**R26. Auth before 404.** Every endpoint returns 401 before any 404 or 400 handler logic. With a JSON body, a JSON parse error comes before 401 (see auth.md).

**R27. Visibility leaks once multi-club lands.**
- Every read here (book diary, feed, reply lists, reaction user lists, `author_rating`, quotes, `quote_count`, stats averages, `top_book`) touches other users' data. Each must go through the single #46 policy function.
- The easy-to-miss leaks:
  - **reaction `users`**: stranger usernames;
  - **`parent_author`** in the feed, when the parent is hidden from the viewer;
  - **`author_rating`** with an `only_me` rating;
  - **stats aggregates** over `only_me` ratings.

**R28. ID strategy.** Feed paging relies on monotonically increasing integer ids. If the schema uses UUIDs, switch to keyset paging on `(created_at, id)` and change the `before` parameter, a frontend change. Otherwise keep `bigint` identity ids [proposal].

**R29. Activity payload format.**
- `milestone_added.due_at` is Python's `isoformat()` with `+00:00`, not `Z`. `quote_added.excerpt` is 80 code points.
- Null values are dropped from payloads.
- If events stay JSON, keep the key names (`title`, `due_at`, `page`, `excerpt`); the activity feed and frontend read them.

---

## 7. Open questions (not settled by plan.md or the issues)

1. **Pick discussion vs. diary.** Is a club's pick discussion club-scoped pick posts, or personal diary entries on the pick's work filtered to club members? Python merged them into the diary. Which legacy table does the importer treat as the source, so content isn't duplicated (`club_pick_posts` rows are already mirrored into `book_posts` via `legacy_post_id`)?
2. **Replies.** Who may reply to whom? Does a reply have its own visibility or inherit the root's? How is a reply shown when its author is outside the viewer's audience: hidden, or as "someone"?
3. **Reactions.** Should reaction user lists omit or anonymize users who are strangers to the viewer?
4. **`book_posts.pick_id`.** Drop it, or keep it as a per-club context marker?
5. **Diary feed scope.** Is `/api/diary` the combined home feed (everything visible), a per-club feed (authors who are club members), or both?
6. **Quote page numbers.** Do quotes carry an optional `edition_id`, so `page` has meaning, or stay work-level with a free `page`?
7. **Personal events.** Do `quote_added` (and other personal events) go into club activity feeds (fan-out to which clubs?), into a personal or home feed, or nowhere?
8. **Stats pages.** With editions, where do stats `pages` and `longest_book` come from when the shelf entry has no edition?
9. **Milestone permissions.** Who may create, edit or delete milestones: any member (Python parity) or admin and owner only? What is the new 403 string?
10. **Milestone shape.** Keep a title-only milestone (no unit)? Keep PATCH as a full replace with no backwards and no closed-pick check, or fix those asymmetries?
11. **Timezone source.** Is it per club or per user for `created_label`, `due_local`, `due_label` and milestone parsing? Should the labels be dropped, given the frontend formats `created_at` itself and uses the label only as a fallback?
12. **Stats endpoint shapes.** What are the personal stats (path, fields, `only_me` filtering) and the club stats (whose shelves count toward pick `readers`)? Do the year boundaries stay UTC?
13. **Unknown work ids.** Does `GET /api/books/works/{id}/posts` on an id the catalog doesn't know keep returning an empty 200 diary, or become 404?
14. **Account deletion (#39).** What happens to a deleted user's diary entries that have replies from others: tombstone (Python's model) or anonymise? What happens to their reactions on others' entries?
15. **Quotes UI.** The quotes, stats and milestones UIs were removed in PR #3. #48 mentions `/quotes`; are stats and milestones UIs in Phase 1 scope?
