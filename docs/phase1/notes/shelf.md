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
- `test_physical_shelf.py::test_new_layout_options_are_validated` and `::test_invalid_layout_does_not_replace_previous_layout` -> the legacy cases hard-code entry id 1; the ports use the caller's real entry id, so each 400 comes from the case itself.
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

### Part 2 (not in this part)

The Goodreads import job (`POST /api/shelf/import`, `GET /api/shelf/import/{job_id}`, `shelf::goodreads::resume`), quotes (`/api/quotes*`), personal stats (`GET /api/stats`) and club stats (`GET /api/clubs/{slug}/stats`, `MemberYear`). Their parity tests are `test_import.py` (8) and `test_stats_milestones_quotes.py::{test_stats_year_in_review, test_stats_with_empty_club, test_quotes_crud_and_filters}`. Their visibility tests (quotes, stats) also belong to part 2.
