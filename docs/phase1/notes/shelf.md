# SHELF (#44a) implementation notes

Owner: SHELF. These notes record what the code does where the design left room. They also list the deviations, the Python tests that are reasoned rather than ported, and what FE, PAR and the other packages need to know. The design documents themselves are unchanged.

## Part 1: shelf entries, favourites, physical layout, search history

### Modules (`server/src/shelf/`)

| File | Contents |
|---|---|
| `mod.rs` | `routes()`, `OwnerQuery` (`?username=`), `resolve_owner` (through `visibility::visible_user`), `lock_user` (`SELECT … FROM users WHERE id = $1 FOR UPDATE`, 401 when the account is gone) |
| `ordering.rs` | Pure rules with unit tests: `apply_finish_fields`, `apply_progress`, `apply_reading_dates`, `column_order` (the `position` half of `place_item`), `shelf_order` (`place_on_shelf`), `declaration_index` |
| `entries.rs` | `GET/POST /api/shelf`, `PATCH/DELETE /api/shelf/{entry_id}`, `ShelfItemOut`, `ShelfListOut`, `ShelfAddIn`, `ShelfPatchIn`, `item_outs` (reads through `bc_visible_shelf`), the shelf events |
| `favorites.rs` | `GET/PUT /api/auth/me/favorites`, `POST/DELETE /api/auth/me/favorites/{work_id}`, `FavoriteOut`, `FavoritesOut`, `FavoritesIn`, `favorite_outs` |
| `layout.rs` | `GET/PUT /api/shelf/physical-layout`, `PhysicalShelfLayout` and its parts, the validator, read-time pruning, `remove_from_layout` (used by DELETE) |
| `search_history.rs` | `GET/POST/DELETE /api/search-history`, `DELETE /api/search-history/{entry_id}` |

Every operation has tag `shelf`, including the favourites under `/api/auth/me/…`. Operation ids: `shelf_list`, `shelf_add`, `shelf_update`, `shelf_remove`, `shelf_get_layout`, `shelf_save_layout`, `shelf_list_favorites`, `shelf_replace_favorites`, `shelf_add_favorite`, `shelf_remove_favorite`, `shelf_list_search_history`, `shelf_remember_search`, `shelf_clear_search_history`, `shelf_delete_search`.

Schema names: `ShelfItemOut`, `ShelfListOut`, `ShelfAddIn`, `ShelfPatchIn`, `FavoriteOut`, `FavoritesOut`, `FavoritesIn`, `PhysicalShelfLayout`, `PhysicalShelfItem`, `PhysicalSpine`, `PhysicalDecoration`, `ShelfMaterial`, `SpineDisplay`, `DecorationKind`, `SearchHistoryIn`, `SearchHistoryOut`. Other packages must not reuse them.

### How the writes work

- **Lock.** Add, PATCH, DELETE, every favourites write, the layout PUT and the search-history POST lock the caller's `users` row first. The writes of one user therefore run one after the other. Tests show that 8 parallel reorders keep the column and the free order dense, and that 30 parallel searches leave exactly 20 rows.
- **One statement per entry.** PATCH computes the entry's new state in Rust with the `ordering` rules and writes the whole row in one `UPDATE`, so the status CHECKs of `shelf_entries` always see a consistent row. The other entries of the touched columns are renumbered with one `UPDATE … FROM unnest(…)` each (only rows whose value changes are written).
- **Order of checks for PATCH** (spec §1.7): validation (400) → `Nothing to update` (400, before the lookup) → user lock → the entry (404 `Book not on your shelf`, also for another user's entry) → `finished_on` against the resulting status, then against today in the caller's zone → the edition (400) → changes. Nothing is written on an error.
- **Order of checks for POST:** validation (400) → `require_work` (404 `No such book.`; a merged work resolves to its target) → `require_edition_of` (400 `That edition belongs to another book`) → user lock → `INSERT … ON CONFLICT ON CONSTRAINT shelf_entries_user_work_key DO NOTHING`. A conflict rolls back and answers 409 `{"detail": "Already on your shelf", "item": ShelfItemOut}`, also for two concurrent adds (tested).
- **Dates:** `finished_on` uses `CurrentUser.timezone` (the owner, since only the owner can PATCH): `tz::today` for the future check and `tz::noon_local_utc` for the stored value. `now` is one `OffsetDateTime::now_utc()` per request, so an entry added as finished has `started_at == finished_at`.
- **Events** go through `activity::record` with `club_id` NULL and `work_id` set. The kinds and payloads are those of spec §2.6. `shelf_finished` stores the rating even for an `only_me` entry; the feed redacts it at read time (conventions §6.4, CLUB). A same-status PATCH records `progress` only when progress was sent, is not null and changed. Rating and take edits record nothing.
- **DELETE** records `shelf_removed {status}`, then prunes the entry from the stored layout in the same transaction, deletes it and renumbers its column. `shelf_position` is not compacted, and favourites are not touched.
- **Favourites:** PUT deletes all of them and inserts positions 1..n in one transaction. POST compacts the positions before it appends at n+1. A unique violation on insert (unreachable under the lock) still answers like the legacy race: 200 when the work is a favourite now, else 409.
- **Layout PUT:** the body is validated, then `?username=` (if given) must name the caller, then every entry on the rows and every spine key must be one of the caller's entries. The stored JSON is the normalised layout (`serde_json::to_value`), written with `INSERT … ON CONFLICT (user_id) DO UPDATE`.
- **Layout GET:** the stored JSON is parsed and validated with the current schema. If that fails, the answer is the default layout; this never causes a 500. Entries the owner no longer has are then pruned from the rows and spines, keeping row boundaries and decorations. The pruned result is not written back.

### Deviations and decisions (beyond endpoint-map §2 and §7)

1. **POST check order:** 404 `No such book.` and 400 for a foreign edition come before the 409. The legacy app had no such checks.
2. **`Nothing to update` counts the new fields:** a present `edition_id` (tri-state, so `null` counts too) and a non-null `rating_visibility` count as something to update. A PATCH with only `rating_visibility` behaves like a rating-only PATCH: the column is re-sequenced and `updated_at` moves. A status change keeps `rating_visibility`.
3. **`finished_on` range:** a date outside 0001-01-02..9999-12-30 (year 0 or negative years, which `time::Date` accepts) is 400 `Invalid value` (`tz::personal_date_in_range`). An invalid date string is 400 `Invalid value`.
4. **Layout PUT with `?username=`:** validation (400) comes before the 404 for another user's name (conventions §4.2: body validation, then handler logic). `?username=` naming the caller in any case, or an empty value, saves the caller's layout. Any other name is 404 `No member with that name`, the same body as for an unknown name.
5. **Layout validation messages.** The custom strings are verbatim. The generic ones follow pydantic's wording where one exists: `List should have at least 1 item after validation, not 0`, `List should have at most 12 items after validation, not 13`, `Dictionary should have at most N items after validation, not M`, `Input should be greater than 0`, `String should match pattern '^#[0-9a-fA-F]{6}$'`, `String should match pattern '^decor:[a-zA-Z0-9_-]{1,64}$'`, `Input should be greater than or equal to 26` / `less than or equal to 64`. Type errors (`true`, `1.5`, `"1"` as an entry, a list for `decorations`, an unknown enum value) are serde errors, so the answer is `Invalid value` or `Input should be a valid list`. All of them are 400 and none is mapped in the frontend.
6. **Layout row items are strict:** a JSON integer is an entry and a string is a decoration id; `1.0` is refused as well as `1.5`. `lights` accepts JSON booleans, the integers 0 and 1, and the strings of `extract::parse_lax_bool`; floats are refused. pydantic also took 0.0 and 1.0, which nothing sends.
7. **Layout spine keys** must match `[1-9][0-9]{0,18}`. A key that matches but exceeds `i64::MAX` (a 19-digit key from 9223372036854775808) is no one's entry: 400 `Physical shelf books must be on your shelf`, never a 500.
8. **Stored layouts that fail the schema** read as the default (Python answered 500). DELETE prunes the raw JSON (`strip_entry`), so it also cleans a layout that no longer validates.
9. **Favourites follow merges:** PUT, POST and DELETE resolve a merged-away work id to its target (`follow_merges`, `follow_merges_many`). If two ids of one PUT resolve to the same work, the answer is 400 `Each book can only be a favourite once`. If an id does not exist, PUT and POST answer 400 `That book is not in the catalog`; DELETE answers 200 with the list.
10. **Favourites positions are compacted before an add.** This fixes the latent gap bug of spec §3: a gap left by a merge no longer causes a false 409.
11. **`ShelfListOut.items` for another viewer** come from `bc_visible_shelf(viewer)`. `rating_visibility` is null for every non-owner (endpoint-map §7), and `only_me` entries show `rating: null` and `take: ""`.
12. **Search history:** POST takes the user lock (see above). GET still has no limit (parity). `normalized_query` is `text::casefold` of the collapsed query.
13. **Path ids:** `{entry_id}` and the favourites' `{work_id}` go through `ApiPath<i64>`. A non-numeric id is 400 `Input should be a valid integer, unable to parse string as an integer`, and an id that overflows `i64` is 404 `Not found` (W1 extractor rule). The favourites path takes numeric work ids only (endpoint-map §2).

### Detail strings

No new strings. The strings below are kept verbatim from the legacy app and live as `const`s in their module; the frontend does not map them today:
- `Already on your shelf`, `Book not on your shelf`, `Nothing to update`
- `Keep the take to 140 characters`, `Keep the reason to 200 characters`
- the layout strings: `At most 2100 items fit on the physical shelf`, `At most 2000 books fit on the physical shelf`, `Each item can only appear once on the physical shelf`, `Spine appearances must refer to shelf entry IDs`, `Each decoration must appear exactly once on the physical shelf`, `Physical shelf books must be on your shelf`
- `Search must contain between 1 and 300 characters`

The mapped strings come from `error::msg`: `FAVOURITE_TWICE`, `AT_MOST_3_FAVOURITES`, `ALREADY_3_FAVOURITES`, `READING_DATE_NEEDS_FINISHED`, `READING_DATE_IN_FUTURE`, `SEARCH_HISTORY_NOT_FOUND`. Two `error::msg` strings that #48 maps per endpoint-map §11.3 are used as well: `BOOK_NOT_IN_CATALOG` (replaces `That book is not in the club yet`) and `EDITION_OF_ANOTHER_BOOK`.

### Python tests reasoned rather than ported one to one

Every test of `test_shelf.py` (16), `test_reading_dates.py` (12), `test_physical_shelf.py` (14 functions, all 35 + 3 parametrised cases) and `test_favorites.py` (12) and `test_search_history.py` (3) has a Rust test with a `/// parity:` line. These are ported with a reasoned change:

- `test_shelf.py::test_shelving_without_authors_does_not_clear_existing_authors` -> holds by construction: a shelf add sends no book metadata (the legacy keys are ignored), so it can never edit the catalog row (spec §6 risk 23). `shelf_entries.rs::shelving_never_changes_the_catalog_row`.
- `test_shelf.py::test_finish_notes_appear_on_club_pick` -> the pick's reader list is CLUB's (`picks`). The shelf half is ported; the reader half checks what the pick reads, `bc_visible_shelf(co-member)`. `shelf_entries.rs::finish_notes_reach_the_club_readers`.
- `test_shelf.py::test_reading_progress_shows_on_friend_shelf_and_pick` -> same split. `shelf_entries.rs::reading_progress_shows_on_friend_shelf_and_to_club_readers`.
- `test_reading_dates.py::test_edit_and_clear_finish_date_survive_reordering` and `::test_move_between_months_saves_order_and_date_together` -> the instance zone `BOOKCLUB_TZ` is now the owner's `users.timezone` (set by SQL in the test).
- `test_physical_shelf.py::test_deleted_spine_does_not_leak_to_a_book_with_a_reused_id` -> Postgres never reuses an id. The test reads the stored row after the DELETE, then inserts an entry with the old id by hand and shows that nothing of the old one attaches to it.
- `test_physical_shelf.py::test_layout_persists_on_reload_and_restart_without_changing_reading_data` -> "restart" is a second `TestApp` on the same database, with a login through the API.
- `test_physical_shelf.py::test_mixed_layout_limits_books_and_decorations_separately` -> the schema half is a unit test in `shelf::layout`; the API half shows the 2100 limit and that 2000 books plus 100 decorations pass the schema.
- `test_physical_shelf.py::test_new_layout_options_are_validated` and `::test_invalid_layout_does_not_replace_previous_layout` -> the legacy cases hard-code entry id 1 for the caller's entry. The ports use the caller's real entry id instead (the duplicate-row case and the five spine-value cases of the matrix), so each 400 comes from the rule the case breaks and not from the ownership check. A control PUT shows that the same spine with valid values is accepted. `9999` and the 5000-digit key stay as the entries that are not the caller's.
- `test_favorites.py::test_replace_order_clear_and_eligibility` -> the book-detail `favorite_position` assertions belong to CAT's `BookDetailOut`; the favourites half is ported.
- `test_favorites.py::test_favorites_in_length_check_is_the_translatable_validator` -> it inspected pydantic's error list. The port checks the HTTP answer: exactly `At most 3 favourites`, alone, also when the ids are invalid too.
- `test_favorites.py::test_stale_last_slot_insert_maps_integrity_error_to_409` -> it monkeypatched `list_favorites` to simulate a stale read, which the user lock now rules out. The port keeps the setup (a third favourite written behind the API's back) and the answer (409, list unchanged), and adds the gap case.
- `test_favorites.py::test_favorite_relationships_use_delete_cascade` -> ORM introspection becomes a `pg_constraint` check: `user_id` is ON DELETE CASCADE (tested by deleting the user), and `work_id` is RESTRICT by design, because works are never deleted and merges re-point favourites.

### For FE (#48)

- `POST /api/shelf` takes `{work_id, edition_id?, status?, rating?, rating_visibility?, take?, dnf_reason?, progress?}`; the legacy metadata keys are ignored. A 409 carries `item`.
- `ShelfItemOut` adds `edition` (an `EditionOut` or null) and `rating_visibility` (null for anyone but the owner). Timestamps end in `Z`. `book` is edition-aware.
- `ShelfListOut.user` is a `UserSummary` (`id`, `username`, `avatar_url`), and `timezone` is the owner's zone.
- Favourites take `{work_ids}` (legacy `book_ids`).
- The layout PUT accepts `?username=` naming the caller only. The layout shape is unchanged.
- The search history is unchanged (bare arrays); `last_used_at` ends in `Z`.

### For other packages

- **CAT:** `BookDetailOut.favorite_position` is `SELECT position FROM user_favorites WHERE user_id = $viewer AND work_id = $work`. `merge_works` must keep favourites at positions 1..n; the SHELF add compacts anyway.
- **CLUB:** the shelf events are personal (`club_id` NULL) with `work_id`. `shelf_finished` keeps `rating`/`take` in the payload even for `only_me` entries; the feed redacts them (conventions §6.4).
- **MED:** `POST /api/shelf/spine-colors` is MED's; SHELF registers no route on that path.
- **IMP:** the stored layout is the normalised JSON of `PhysicalShelfLayout` (legacy-mapping.md §5). The GET reader (`shelf::layout`, private `stored_layout`) fills `lights`, `decorations` and spine `display` and lowercases colours, but answers the default layout for anything the current schema refuses (for example a decoration on the rows without its entry in `decorations`). An imported layout must therefore pass the full validation, or the member sees the default. After Wave 2 the integrator can make the reader public if IMP should share it.

## Part 2: the Goodreads import, quotes, personal and club stats

### Modules (`server/src/shelf/`)

| File | Contents |
|---|---|
| `goodreads.rs` | `routes()` (the upload route with a 2 MiB body limit), `POST /api/shelf/import`, `GET /api/shelf/import/{job_id}`, the parser (`parse_goodreads_csv`, `clean_isbn`, `GoodreadsRow`, `ParsedExport`), the job (`resume`, private `run`/`process`/`find`/`import_row`/`shelve`), `ImportJobOut`, `ImportSkipOut`, `ImportSkipCode`, `ImportStatus`, `GoodreadsUpload` (the multipart schema) |
| `quotes.rs` | `GET/POST /api/quotes`, `PATCH/DELETE /api/quotes/{quote_id}`, `QuoteOut`, `QuoteListOut`, `QuoteIn`, `QuotePatchIn`, `QuoteQuery` |
| `stats.rs` | `GET /api/stats`, `GET /api/clubs/{slug}/stats`, the one rule set (`ReadingLog`, `year_totals`, `member_years`, `mean`), `PersonalStatsOut`, `ClubStatsOut`, `MemberYear`, `MonthCount`, `PickStat`, `StatsQuery`, `ClubStatsQuery` |
| `entries.rs` | gained `NewEntry` and `insert_entry` (insert at the top of the column and renumber, no event), shared by `POST /api/shelf` and the import |

Operation ids (tag `shelf`): `shelf_start_import`, `shelf_get_import`, `shelf_list_quotes`, `shelf_add_quote`, `shelf_update_quote`, `shelf_delete_quote`, `shelf_personal_stats`, `shelf_club_stats`. Schema names: `ImportJobOut`, `ImportSkipOut`, `ImportSkipCode`, `ImportStatus`, `GoodreadsUpload`, `QuoteOut`, `QuoteListOut`, `QuoteIn`, `QuotePatchIn`, `PersonalStatsOut`, `ClubStatsOut`, `MemberYear`, `MonthCount`, `PickStat`. Other packages must not reuse them.

### How the import works

- **Upload.** `CurrentUser` first (401 before the body is read), then the `file` part. The route raises the body limit to 2 MiB (conventions §4.6), so a file between the global 1 MiB limit and 1.5 MB is accepted (tested). The reader stops just after 1,500,000 bytes. A body over the 2 MiB limit gives the same 400 `That file is too large. Keep the export under 1.5 MB.` as a large file, never a 413. A missing `file` part, or a body that is not multipart, gives 400 `Field required`.
- **Parse** (synchronous, legacy strings): the size, UTF-8 (a BOM is allowed), then the headers (`title` and `exclusive shelf`, trimmed and case-insensitive). Rows are mapped as in spec §2.7, with the 400-row cap (one `stopped_after_400` skip), `no_title` (`Untitled`) and `unknown_shelf`. Extra cells are ignored, which avoids the Python 500 (spec §6 risk 18). `Title` and `Author` are cut to 300 characters (`MAX_LOOKUP_CHARS`, the search box's limit) and trimmed again, so the lookup's full-text step never gets an unbounded string from the file. The skip labels use the cut title.
- **Job.** The parsed rows, `total` (the rows to look up), `skipped` and the first 25 parse skips are stored in `import_jobs`. The answer is **202** `ImportJobOut` (`status: queued`), and a `tokio::spawn` task runs the job. The `import_jobs_one_active` unique index decides "one active job per user", so two concurrent uploads give one 202 and one 409 (tested).
- **Cancellation.** The handler runs the INSERT and the spawn of the job's task inside one spawned task and awaits it. A client that goes away drops the handler, but not that task, so a committed job row always gets its task. Before, a disconnect while the INSERT's reply was pending left a `queued` row without a task, and every later upload of that user answered 409 until the next restart (tested by dropping the request while the INSERT waits for a lock).
- **One row at a time.** For each row from `processed` on:
  1. The row is looked up on a pool connection, outside any transaction: `resolve_isbn` (when the row has a valid ISBN), then `resolve_text(title, authors)`. The first hit wins. The lookup holds one of two process-wide slots (`LOOKUP_SLOTS`, a `tokio::sync::Semaphore`) and its pool connection only while the lookups run. So all jobs together hold at most two connections through remote calls, and the API keeps the rest of the pool (sqlx's default of 10). The slot is released while a row waits (below) and while the row commits.
  2. **Throttled rows wait.** When a lookup was throttled and nothing hit, the row is tried again instead of being skipped at once. While Open Library's throttle has a cooldown running (after an upstream 429), the row waits for the cooldown plus 100 ms. Otherwise (the throttle's queue was full) it waits 250 ms, at most twice. All waits of one row add up to at most 60 s (`RetryPolicy::JOB.max_wait`, the jobs' throttle wait of conventions §10.3), so a cooldown longer than that is not waited for. When the tries run out, the row is skipped as `catalog_unavailable`. `CatalogLookup` has no wait parameter, so these retries stand in for the jobs' longer throttle wait.
  3. One transaction locks the user row, then the job row (`FOR UPDATE`, checking that `processed` is still this row's index). It inserts the entry at the top of its column (`insert_entry`), or records the skip. It then advances `processed`, `imported`/`skipped` and `skips` (appended while it has fewer than 25), and commits.
  4. When every row is done, the job becomes `done`, `rows` is emptied and `finished_at` is set.
- **Errors.** A database error or a panic marks the job `failed`, with `error` = `The import stopped unexpectedly. Try again.` (the cause is logged). `rows` is emptied, `finished_at` is set, and the rows already committed stay on the shelf; the row that failed is rolled back with its progress (tested with a trigger that refuses the second row's insert).
- **Resume.** `resume` (once at startup) spawns a run for every `queued` or `running` job. A run claims its job with `UPDATE … SET status = 'running' … RETURNING`. Because each row commits with its progress, a resumed job never looks up or shelves a committed row again. Two runs on one job (resume racing a fresh spawn) are safe: the `processed` check makes the loser stop (tested with two `resume` calls on one queued job).
- **No events** are recorded (parity, conventions §15).

### Deviations and decisions (part 2)

**Import:**
1. **Background job instead of one request.** The legacy single transaction is replaced by one transaction per row (above), so progress is visible and a restart loses nothing. Lookups run one after another per job, not two at a time: Open Library's global 3 req/s limit is the bottleneck either way. Jobs of different users run concurrently, but share two lookup slots (above), so a third job's rows wait for a slot.
2. **`total`** counts the rows to look up. Rows skipped while parsing are in `skipped` (and `skips`) from the start, so a finished job has `imported + skipped = total + <parse skips>`.
3. **Lookup outcome.** `no_match` only when every lookup gave a definite answer. When a lookup was throttled or unavailable (`LookupError::Throttled`/`Unavailable`) and nothing hit, the code is `catalog_unavailable` and the job goes on. A throttled row is first tried again within its 60 s budget (above); an unavailable one is not. A throttled ISBN lookup still tries title and author (the local full-text step can hit).
4. **Editions.** Only an ISBN hit sets `edition_id` (the edition must belong to the work after following merges). A title-and-author hit names no edition, because it does not identify the reader's copy. A hit on a merged-away work shelves the work it was merged into.
5. **ISBNs** go through `parse_isbn`, so an ISBN with a bad checksum is dropped and the row is looked up by title and author. ISBN-10s become ISBN-13s. (Python sent any 10- or 13-character string to Open Library.) `clean_isbn` itself is unchanged.
6. **`My Rating`** (an integer 1–5; Goodreads writes 0 for "no rating") is imported for `read` rows, with `rating_visibility` `audience`. **`Date Read`** (`YYYY/MM/DD` or `YYYY-MM-DD`) of a `read` row becomes `finished_at` at noon in the user's zone, and `started_at` is clamped to it, like a `finished_on` PATCH. A date outside the personal date range, or in the future in the user's zone, is ignored: the entry gets the import time, as before.
7. **The 409** `An import is already running` also carries the active job as `job` (an `ImportJobOut`), so a reloaded page can resume polling. `GET /api/shelf/import/{job_id}` of another user's job is 404 `No such import`, like a missing one.
8. **CSV edge cases.** A blank line before the header row is skipped (Python saw an empty header and answered `That does not look like a Goodreads export`). With duplicate header names the last column wins (Python's row dict).

**Quotes:**
9. **Requests.** `POST` takes `{work_id, edition_id?, body, page?, visibility?}`; the legacy book metadata (`ol_work_key`, `title`, …) is gone. An unknown work is 404 `No such book.` (it replaces `title is required for a new book`), and a merged work resolves to its target. `edition_id` must be an edition of the work (400 `That edition belongs to another book`, a missing edition too). Without it, the quote takes the caller's shelf edition of the work at creation time, else null.
10. **`PATCH`** takes `{body?, page?, visibility?}`. `body: null` and `visibility: null` count as absent, `page: null` clears the page. Check order (diary R6): validation (400) → not visible (404 `No quote with that id`) → someone else's (403) → `Nothing to update` (400). `DELETE` checks the same way. A quote the caller cannot see (a stranger's, or someone else's `only_me` one) is 404, never 403. `updated_at` is set on every PATCH; it is not in `QuoteOut`.
11. **`GET /api/quotes`.** `work` is a numeric work id (a merged id finds its target; an unknown id gives `{"items": []}`; `OL1W` is 400 `Input should be a valid integer, unable to parse string as an integer`). An empty `work=` is no filter, like an absent one (legacy `if work:`, diary R24), so `?work=&username=ada` is `?username=ada`. A value of only spaces is not empty and is 400. `username` goes through `visibility::visible_user`: an unknown name and a stranger both give `{"items": []}`. That function also trims the name (Python only lowercased it). `limit` is 1..500 (default 100) with pydantic's messages. The order is `id DESC`.
12. **`QuoteOut`** = `{id, book, edition_id, author, mine, body, page, visibility, created_at, created_label}`. `book` uses the quote's edition as context. `created_label` is `tz::label` in the viewer's zone.
13. **`quote_added`** is personal (`club_id` NULL, `work_id` set), with payload `{page?, excerpt, quote_id}`. `excerpt` is the first 80 characters of the trimmed body. Deleting a quote keeps the event.

**Stats:**
14. **Personal stats** `GET /api/stats?username=&year=` → `PersonalStatsOut {username, year, years, finished, dnf, pages, average_rating, five_stars, top_book, longest_book, quotes, by_month}`: the legacy `MemberYear` fields of one user, plus `years` (that user's years ∪ the current UTC year, newest first) and `by_month` (that user's finished books per UTC month). `?username=` follows `resolve_owner` (404 `No member with that name` for strangers and unknown names).
15. **Club stats** `GET /api/clubs/{slug}/stats?year=` → `ClubStatsOut` (the legacy `StatsOut` shape, renamed so the schema name says what it is):
    - `members` are the club's current members, sorted by `(-finished, username)`.
    - `years` and `by_month` cover those members.
    - `picks` are the club's picks, all time, `id DESC`. `readers`, `finished`, `dnf` and `ratings` count the current members' entries for the pick's work, any year (`readers`: any status).
    - `set_by` is `unknown` when the setter's account is gone; a setter who left the club still shows their name.
    - `best_pick`: the max `(average_rating, number of ratings)`; on a full tie the newest pick wins.
16. **One rule set** (`member_years`):
    - An entry counts in the UTC year of `finished_at`, else of `updated_at`.
    - `pages` are the entry edition's, else the work's, else 0, and `top_book`/`longest_book` are `BookOut`s with the entry's edition as context. Pick books have no edition context.
    - Ties: `top_book` takes the max `(rating, entry id)`; `longest_book` takes the first max in entry-id order.
    - Means use `text::round_half_even(…, 2)`.
    - Counts and pages are JSON integers (`i64`).
    - The default `year` is the newest of `years`; any year in 2000..2100 is accepted.
17. **`only_me`.** Every shelf and quote read goes through `bc_visible_shelf(viewer)` and `bc_visible_quotes(viewer)`. For another viewer, an `only_me` entry still counts as finished (shelf state), but its rating is not in `average_rating`, `five_stars`, `top_book`, the pick `ratings` or the club mean, and `only_me` quotes are not in `quotes`. The viewer's own `only_me` ratings and quotes count for that viewer, so the club stats a member sees can differ from another member's by that member's own `only_me` items.

### Detail strings (part 2)

From endpoint-map §11.3, which FE must map: `An import is already running`, `No such import`.

New, not in the design. It is the `error` field of a failed job, not a `detail`, so FE shows it or maps it: `The import stopped unexpectedly. Try again.`

Kept verbatim from the legacy app, as `const`s in their module. The frontend does not map them today:
- `That file is too large. Keep the export under 1.5 MB.`
- `The file must be a UTF-8 Goodreads CSV export`
- `That does not look like a Goodreads export`
- `Paste the line you want to keep`
- `Keep it to 1000 characters`
- `No quote with that id`
- `You can only edit your own quotes`
- `You can only delete your own quotes`
- `Nothing to update`

The skip `reason`s stay the legacy English (`Stopped after 400 rows`, `No title`, `Unknown shelf`, `No match`, `Catalog unavailable`, `Already on your shelf`); FE translates by `code`.

From `error::msg`: `NO_SUCH_BOOK`, `EDITION_OF_ANOTHER_BOOK`, `NO_MEMBER`, `NO_SUCH_CLUB` (the club extractor).

### Python tests ported with a reasoned change (part 2)

All 8 tests of `test_import.py` and the three stats and quotes tests have a Rust test with a `/// parity:` line (`tests/shelf_import.rs`, `tests/shelf_quotes.rs`, `tests/shelf_stats.rs`). These carry a reasoned change:

- `test_import.py::test_parse_fixture_maps_shelves_and_skips` -> a row without an ISBN has `isbn13: None` (legacy `""`), and the rows also carry `rating` and `read_on`.
- `test_import.py::test_import_matches_isbn_and_title` -> the monkeypatched Open Library client becomes `FakeCatalog`, with Circe by ISBN and Achilles by title and author. The upload answers 202 and the test asserts the finished job, whose skips carry `code`. The calls `isbn:9780316556347` and `text:The Song of Achilles|Madeline Miller` are asserted.
- `test_import.py::test_import_skips_books_already_on_shelf` -> the ISBN resolves to the existing work (`FakeHit::Existing`).
- `test_import.py::test_import_rejects_oversized_file` -> also checks a body over the 2 MiB route limit (same 400). `import_accepts_a_file_over_the_global_body_limit` shows that a 1.4 MB export is accepted, so the route's own limit is in force.
- `test_stats_milestones_quotes.py::test_stats_year_in_review` -> the legacy club-wide answer is split: the member numbers come from `GET /api/stats` and from `members` of the club stats, and the picks and club totals from the club stats. The pick is inserted by SQL (picks are CLUB's).
- `test_stats_milestones_quotes.py::test_stats_with_empty_club` -> asserted on both endpoints.
- `test_stats_milestones_quotes.py::test_quotes_crud_and_filters`:
  - "unknown book needs a title" becomes 404 `No such book.` for an unknown work id, and `?work=` takes the work id.
  - The book detail's `quote_count` is CAT's (`BookDetailOut`); the test checks the count CAT reads, `bc_visible_quotes(viewer)` for the work.
  - The feed is CLUB's; the test checks the `quote_added` row in `events`.
  - grace sees ada's quote because they share a club (the legacy "everyone in the instance").

### For FE (#48), part 2

- **Import.** `POST /api/shelf/import` answers **202** with `ImportJobOut {id, status, total, processed, imported, skipped, skips: [{title, code, reason}], error}`. Poll `GET /api/shelf/import/{id}` until `status` is `done` or `failed`. Progress is `processed / total`. Show `skip.title || "Untitled"` and translate by `code`. A 409 carries the running job as `job`.
- **Quotes.** The quotes page sends `work_id` (and optionally `edition_id` and `visibility`). `?work=` takes a work id. `QuoteOut` adds `edition_id` and `visibility`.
- **Stats.** Stats are optional in Phase 1 (work-plan, diary 15). The personal stats are `PersonalStatsOut`, the club stats `ClubStatsOut` (the legacy `StatsOut` fields).

### For other packages, part 2

- **CLUB:** `quote_added` events are personal and carry `quote_id`. The feed hides them from others when the quote is gone or `only_me` (conventions §6.4). The import records no events. `GET /api/clubs/{slug}/stats` is SHELF's path.
- **CAT:** `BookDetailOut.quote_count` should count `bc_visible_quotes(viewer)` rows for the work, so other users' `only_me` quotes are not counted. The import calls `resolve_isbn` and `resolve_text` on a plain pool connection (autocommit, no transaction around them), one row at a time per job and at most two rows at a time across all jobs. Two lookups may therefore persist the same Open Library work concurrently. A throttled row is tried again after `outbound.ol_api.cooldown_remaining()` (or 250 ms when no cooldown runs), within 60 s per row. Titles and authors passed to `resolve_text` are at most 300 characters.
- **IMP:** quotes are imported with `visibility` `audience` and the `edition_id` of legacy-mapping.md. `import_jobs` has no legacy rows.
- **PAR:** the visibility matrix for quotes and stats is in `tests/shelf_visibility.rs`: the audience, strangers (another club, no club, a former co-member) and an unknown name, for the quote list, PATCH and DELETE, the personal stats, and the club stats (members only).
