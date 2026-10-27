# CAT (#41) implementation notes

Owner: CAT. These notes record what the catalog package built, where it deviates from the design documents, and what FE, PAR and the integrator need to know. The design documents themselves are unchanged.

## Status

**Part 1 (done): `CatalogService` and search.**

- `catalog/cache.rs`: the response caches (moka, 18 h, bounded) with request coalescing. Errors and empty result lists are never cached. Coalesced callers share one fetch and its outcome.
- `catalog/openlibrary.rs`: the Open Library client. It covers search pages, trending, subjects, works, authors, search by work key, ISBN search, text search for `resolve_text`, and the `/isbn/{isbn}.json` cover check. It also holds the lenient payload mappers and `ol_work_record`, the one normaliser for a work's first import and its refresh, which Part 2 uses.
- `catalog/dnb_sru.rs`: the DNB SRU client (`query=num={isbn}`, `recordSchema=MARC21-xml`, at most 5 records) through `outbound::get_bytes` and the 1 req/s DNB SRU throttle. It reads the SRU envelope only; records go to `dnb_marc::map_record`.
- `catalog/service.rs`: `CatalogLookup` with the registration-group routing of conventions §7.4.
  - Local edition first.
  - 978-3: DNB SRU, then Open Library.
  - Every other group: Open Library only.
  - A DNB hit is ingested with `upsert_dnb_record(…, Mode::Replace)`, followed by one Open Library cover check.
  - An Open Library hit is ingested with `upsert_edition_with_work(…, NewWork, Mode::Refresh)`.
  - The ingest runs in its own (nested) transaction and is retried once after a deadlock.
  - Without a definite answer the result is `Err`, never `Ok(None)`.
- `catalog/hits.rs`: persisting remote hits (one batched insert-only `upsert_work_stubs`, sorted by `ol_work_id`), queueing their Open Library covers with `MediaService::prefetch`, dedupe and cap, and the annotation: `on_shelf`/`shelf_id` from the caller's shelf, and `club_picks` through `bc_member_clubs`.
- `catalog/search.rs`: `GET /api/books/search`, `/trending`, `/subjects/{subject}`.
- `catalog/isbn_lookup.rs`: `GET /api/books/isbn/{isbn}` (`IsbnHitOut`).
- Tests:
  - `catalog_search.rs` (31)
  - `catalog_isbn.rs` (14)
  - `catalog_client.rs` (6)
  - `catalog_no_google.rs` (2)
  - unit tests in `catalog::{cache, openlibrary, dnb_sru, hits, search}` (17)
- Fixtures, recorded on 2026-10-27:
  - `tests/fixtures/dnb_sru/`: an SRU hit for 9783694050797 and an SRU miss.
  - `tests/fixtures/openlibrary/`: the ISBN search and edition JSON of 9780316556347.

**Part 2 (done): detail, refresh, user-added books, editions, requests, the admin queue and merges.**

- `catalog/detail.rs`:
  - `GET /api/books/works/{work_id}` answers `BookDetailOut`. A known work comes from the catalog.
  - An Open Library stub (`details_fetched_at` NULL) is enriched once on its first view, through `ol_work_record` and `upsert_work(…, Mode::Refresh)`.
  - A DNB work gets its one `/isbn/{isbn}.json` cover check on its first view, through the first edition with an ISBN.
  - If either fails, the stored row is served with 200 and the check stays open.
  - An unknown `OL…W` is imported.
  - `book_detail(conn, viewer, work_id)` is the loader. It reads readers, the rating mean and count through `bc_visible_shelf`, and the quote count through `bc_visible_quotes`.
  - `POST …/refresh` is limited to Open Library works, `refresh_user` (10/h), and uses `Mode::Refresh` with the caches bypassed.
- `catalog/custom.rs`: `POST /api/books/custom`, and `claim_isbn`, the ISBN rule that books and editions share: a local edition is 409, then `resolve_isbn_remote`.
- `catalog/editions.rs`: `GET/POST /api/books/works/{work_id}/editions` and `GET /api/books/editions/{edition_id}`.
- `catalog/requests.rs`: `POST /api/books/works/{work_id}/edit-requests`, `POST /api/books/editions/{edition_id}/edit-requests` and `POST /api/books/works/{work_id}/merge-requests`, sharing `edit_request_user` (50/day).
- `catalog/admin.rs`: `GET /api/admin/edit-requests` and `POST …/{request_id}/approve`. Approve applies a request through `apply_admin_fields` for edits, `set_cover(…, Admin)` for `cover_replace` and `merge_works` for `work_merge`. Also `POST …/{request_id}/reject` and `POST /api/admin/works/{work_id}/merge`.
- `catalog/merge.rs`: `catalog::merge_works(conn, loser, winner) -> sqlx::Result<MergeReport>` (conventions §7.6), described under deviation 26.
- `catalog/service.rs`: `attach_checked_cover`, the bookkeeping of the cover check, shared by the SRU lookup and the first detail view.
- Tests:
  - `catalog_detail.rs` (31)
  - `catalog_custom.rs` (9)
  - `catalog_requests.rs` (8)
  - `catalog_merge.rs` (3)
  - `catalog_provenance.rs` (3)
  - `catalog_visibility.rs` (2)
- No new fixtures; the Part 1 recordings are reused.

## Deviations and decisions

1. **`on_shelf` is a boolean.** W1's frozen `SearchHit` has `on_shelf: bool` next to `shelf_id`. The legacy value was the shelf status. `IsbnHitOut.on_shelf` is a boolean too. FE: Discover must stop reading a status from `on_shelf`.
2. **Remote docs whose key is not `/works/OL<digits>W` are skipped.** Such a doc cannot be stored as an Open Library work: ingest would drop the id and create a new work on every search. The parity fake ids `OLSTANDW` and `OLJUNKW` became `OL77W` and `OL66W`.
3. **Empty trending and subject lists are not cached either.** The legacy app cached them; only empty search pages were skipped. The google spec KEEP test 9 covers trending and subjects as well, and a lucky-empty browse row should not freeze for 18 hours.
4. **Fallbacks:**
   - A failing fallback no longer fails a search when another fallback has hits (work-plan catalog decision 11).
   - When no fallback has hits, the first fallback error surfaces, so an outage is not shown as "nothing found".
5. **The 429 hint.** While an upstream cooldown runs, the answer is `The library is busy. Try again in {n} seconds.` with `Retry-After: n`. That includes the 60 s default cooldown after a 429 without `Retry-After`, where Python said `The library is busy. Wait a few seconds and try again.` The plain `Wait a few seconds` string remains for a full local throttle (2 s wait, decision 13).
6. **Hits come from stored rows.** Every hit is persisted first, then built from `works` through `books::book_outs`, so a stored work's fields win over the search doc. Stubs are insert-only (`upsert_work_stubs`), so a work stored without a cover does not take the cover of a later search doc; the first detail view or a refresh fills it.
7. **ISBN-shaped search queries:**
   - They go through `state.lookup.resolve_isbn`, ignore the subject, and return at most one hit on page 1 and an empty page after that.
   - A query with an ISBN's shape but a failing checksum matches nothing and makes no outbound call. Python sent `isbn:<digits>` to Open Library.
   - `SearchHit.isbn` is the normalised ISBN-13.
8. **The Open Library ISBN step** is the legacy `search.json?q=isbn:…` (fields without `isbn`, `limit=1`). The hit is stored as an Open Library edition carrying only `isbn13`, because the doc describes the work. `number_of_pages_median` goes on the work.
9. **The DNB cover check:**
   - After a DNB hit, `/isbn/{isbn}.json` is asked once. Its first positive `covers` id is attached to the edition through `ol_cover_media` and `set_cover(…, Source::OpenLibrary, false)`, and ingest's NULL fill then gives the work the cover.
   - When Open Library answered (a cover, no cover, or 404), `works.details_fetched_at` is set directly with `UPDATE works SET details_fetched_at = now() WHERE id = $1 AND source = 'dnb' AND details_fetched_at IS NULL`. This is bookkeeping, not a provenance field, and no ingest entry point sets it for a work without `ol_work_id`. When Open Library did not answer, it stays NULL and the first detail view (Part 2) checks again.
10. **The DNB SRU query** uses the `num=` index (every number field). Only records that have an edition with the requested ISBN after `map_record` count as a hit, and the first such record wins. A response without `numberOfRecords` (an SRU diagnostic) counts as no answer, not as a miss.
11. **`resolve_text`:**
    - **Local match:** a current work whose `lower(bc_unaccent(title))` equals the given title's and whose `search_vector` holds every word of title and authors. This is stricter than ranked FTS, so that "It" does not resolve to another book by the same author.
    - **Remote:** the first Open Library hit of `q="{title} {authors}"` with `limit=5`, stored as a stub.
    - **Errors:** `Throttled`, or `Unavailable`. The legacy Goodreads lookup turned these into "No match".
12. **Error mapping goes through `catalog::browse_error` and `catalog::busy`,** not through `UpstreamError::to_app_error`. The catalog needs 422 → 400 `Need at least 3 characters…` and the cooldown hint, which `to_app_error` cannot express.
13. **Query parameters:**
    - `page` and `limit` are `i32`, so a `page` above 2147483647 is 400 `Input should be a valid integer, unable to parse string as an integer` (risk R19).
    - `sort=` (empty) is 400 with the pydantic enum string, as before.
14. **Local search:**
    - It uses full-text search (`plainto_tsquery('simple', bc_unaccent(q))`) or a case- and accent-insensitive substring match on `bc_search_text(…)` (the trigram index). It is ranked by `ts_rank` plus trigram similarity, and by title first for `sort=title`.
    - It runs on page 1 for needles of at least 2 characters. The subject filter does not apply to local hits (parity).
    - Merged-away works never match.
15. **Coalescing after cancellation:** when the caller running a fetch is dropped, a waiting caller runs the fetch itself instead of failing with a 502 (decision 16). `test_cancelled_fetch_unblocks_coalesced_waiters` asserts that.
16. **Cache sizes:** 1024 Open Library answers and 256 DNB SRU answers. Bounded by entry count, which replaces strict LRU order (spec §5.8).

### Part 2

17. **`BookDetailOut.on_shelf` is the viewer's `ShelfStatus` or null, not a boolean** (unlike `SearchHit`). The DTO has no other status field, and the detail page needs the status.
18. **`BookDetailOut` details:**
    - `subtitle` is a string, `""` when there is none, like `description`.
    - `BookReader` keeps the legacy fields `{username, status, rating, progress, avatar_url}`, and `avatar_url` is `/media/{public_id}/a`.
    - The detail shows the work without an edition context. The viewer's shelf edition is `edition_id`.
    - `can_refresh` is `source == "openlibrary"`.
19. **Enrichment and the DNB cover check have no negative cache.**
    - A failed attempt serves the stored row, and the next view tries again.
    - During an Open Library cooldown the attempt fails at once, without a request.
    - An upstream 422 on a work import is 502 `Could not load the library right now. Try again.` (decision 11).
20. **Refresh:**
    - Every stored work that is not an Open Library work is 400 `This book was added by a reader, not Open Library.` (`user`, `dnb`, `legacy`).
    - A `{work_id}` that names no work is 404 `No such book.`. The legacy app answered 400 for a missing `BC…` key.
    - The 10-per-hour limit counts only refreshes that reach Open Library, after the 400 and 404 checks.
21. **User-added books (`POST /custom`):**
    - **Edition:** an edition is created when `isbn`, `publisher`, `pages` or `language` is given. `year` stays on the work.
    - **Validation:**
      - `language` must be three letters, else 400 with the pydantic pattern message `String should match pattern '^[a-z]{3}$'`.
      - `pages` must be 1 to 100000.
      - An invalid ISBN is 400 `That is not a valid ISBN-10 or ISBN-13`.
    - **Rate limit:** it counts after validation, so invalid bodies do not count, and before the ISBN lookup, so a 409 counts.
    - **409 body:** it includes `work_id` and `edition_id`.
    - **A concurrent claim:** when the ISBN appears under another work between the check and the insert, the transaction is rolled back and the answer is 409.
22. **Editions:**
    - **List:** local only (no Open Library editions fetch), oldest first.
    - **Add:**
      - It needs at least one field, else 400 `Field required`.
      - `format` gets the pydantic enum message `Input should be 'hardcover', 'paperback', 'ebook', 'audiobook' or 'other'`.
      - `publication_year` must be 1 to 3000.
      - A title equal to the work's is not stored (ingest).
    - **Unknown ids:** an unknown edition is 404 `No such book.`, and writes never import an unknown `OL…W`.
23. **Edit requests are checked by a dry run of their approval.** `apply_admin_fields` runs in a transaction that is rolled back.
    - The user-facing 400s are therefore `Unknown field: <key>` and `Invalid value for <key>`, plus `Field required` for an empty `fields`.
    - Notes are trimmed and cut at 2000 characters.
24. **Merge requests:**
    - `into_work_id` is a numeric id.
    - Both ids follow merges, and two ids that resolve to one work are 400 `A book cannot be merged into itself`.
25. **Admin queue:**
    - **Response:** `EditRequestOut {id, kind, status, work_id, edition_id, book: BookOut|null, payload, cover_url, cover_thumb_url, cover_public_id, note, created_by: UserSummary|null, created_at, decided_by, decided_at, decision_note}`. `book` is the work in the edition's context; the cover fields are the proposed `cover_replace` picture, null once it was taken down.
    - **Errors:**
      - An unknown request is 404 `Not found`.
      - A decided request is 409 `This request was already decided` (admin only).
      - A `cover_replace` whose picture was removed is 400 `This picture was removed`.
    - **Bodies:** reject needs a JSON object (`{}` is enough).
    - **Approving:**
      - A `cover_replace` sets the edition's cover when the request names an edition, else the work's.
      - The request is marked approved before it is applied, in the same transaction, so a merge does not reject its own request.
26. **`merge_works`:**
    - **Locks:** the two works `FOR NO KEY UPDATE`, which does not block the foreign-key checks of concurrent shelf writes, then the users with a shelf entry or favourite on the loser, then the affected votes, all `FOR UPDATE` and in id order.
    - **Fields:** the winner's record stays the authority.
      - Its empty fields are filled from the loser.
      - Its `legacy`-ranked values give way to better-ranked loser values.
      - An `admin` loser value wins over anything but an `admin` winner value.
      - A reader's value never replaces a source's. When the admin keeps an Open Library work, a user book merged into it does not retitle it.
      - `work_authors` move with the names, and `ol_rating` and `ol_rating_count` fill when empty.
    - **Upgrades:** a `legacy` winner becomes `openlibrary` when it receives the loser's `ol_work_id`, else `dnb` when the loser is DNB. `ol_work_id` moves when the winner has none; it is cleared on the loser first, and `details_fetched_at` comes along.
    - **Derived columns:** `match_key` and `search_extra` are recomputed.
    - **Redirects:** works merged into the loser earlier are re-pointed at the winner.
    - **Requests:**
      - Open requests on the loser's editions follow the editions.
      - Merge requests into the loser now name the winner.
      - Other open requests on the loser, and merge requests that became self-merges, are rejected with `decision_note` `Merged into book {winner}`.
    - **Atomicity:** it runs as a savepoint and returns `RowNotFound` or `Protocol` for an invalid pair. The handlers answer 404 or 400 before calling it.
    - **No deadlock retry:** a deadlock with a concurrent multi-record ingest (HAR's page) is possible but rare.

## Strings

The catalog answers with the legacy strings of spec/catalog.md §1.11 and endpoint-map §4, byte for byte. They are consts in `catalog/mod.rs`. None of them is in `apiErrors.ts`, and #48 may map them (optional). The 429 hint is a template, `The library is busy. Try again in {n} seconds.`; the frontend must match it by prefix if it maps it.

Part 2 returns the endpoint-map §11.3 strings `This book was added by a reader, not Open Library.`, `That ISBN is already in the catalog`, `A book cannot be merged into itself` and `This picture was removed`; #48 maps them as planned. Strings that are not in endpoint-map §11.3:

- `String should match pattern '^[a-z]{3}$'`: a custom book's or an edition's `language`.
- `Input should be 'hardcover', 'paperback', 'ebook', 'audiobook' or 'other'`: an edition's `format`.
- `Unknown field: <key>` and `Invalid value for <key>`: edit requests. W1 wrote them as admin-only strings, but the dry run makes them user-facing. They are templates; FE maps them by prefix, or by status.
- `Field required`: an edition or edit request with nothing in it. This is the existing `extract` string.
- `This request was already decided` (409): admin only, so FE does not map it.
- `A title is required`: the legacy string, kept.

## Python tests: ported in Part 1

- **`test_search.py` (23 of 35):**
  - In `tests/catalog_search.rs`: every search, trending, subject, cache and error test.
  - Unit tests in `catalog::cache`: `test_search_cache_is_bounded`, `test_search_cache_ttl_is_at_least_half_a_day`, `test_identical_inflight_searches_share_one_request` (also in `catalog_search.rs`, through the API) and `test_cancelled_fetch_unblocks_coalesced_waiters` (reasoned change: the follower retries).
  - A unit test in `catalog::search`: `test_search_candidates_keep_raw_query_before_by_split`.
- **`test_book_details.py` (3 of 8):** `test_isbn_lookup` and `test_isbn10_with_x_check_digit_is_accepted` in `catalog_isbn.rs`, and `test_positive_cover_id_rejects_placeholders` in `catalog::openlibrary`.
- **`test_google_books.py` (search and lookup KEEP and ADAPT):** tests 2 (ISBN half), 5, 7, 9 (search, trending, subjects, ISBN, DNB SRU), 10, 12, 13, 14, 15, 16, 18 (browse half, INVERT), 20, 21, 22, 29, 31, 35, 39 (lookup half), 40 (INVERT), 48, 49 and 51. Test 8 (INVERT) is ported as "a miss falls through to the next source".

## Python tests: ported in Part 2

- **`test_search.py` (12):** every detail, refresh and custom-book test.
  - In `catalog_detail.rs`: `test_first_import_skips_author_api_when_search_has_names`, `test_book_detail_resolves_description_subjects_and_authors`, `test_book_detail_accepts_dict_description`, `test_book_detail_flattens_markdown_in_description`, `test_book_detail_includes_own_shelf_state_and_other_readers` (adapted: a club, a stranger, a known work), `test_book_detail_rejects_malformed_work_id`, `test_book_detail_propagates_not_found`, `test_detail_survives_author_lookup_failure`, `test_book_detail_refresh_updates_local_row` (adapted: a known title-only work) and `test_refresh_bypasses_author_cache`.
  - In `catalog_custom.rs`: `test_local_catalog_is_searched_without_open_library_for_custom_books` and `test_custom_book_requires_a_title`.
- **`test_book_details.py` (5)**, all in `catalog_detail.rs`, with `/work/` ported to `/works/`:
  - `test_work_details_for_unknown_book`
  - `test_work_details_persist_on_known_book_and_list_members` (`members` → `readers`)
  - `test_work_details_rejects_bad_ids_and_upstream_failure` (`/works/not-a-key` is 404, not 400)
  - `test_work_details_skips_leading_negative_cover`
  - `test_cancelled_work_details_unblocks_coalesced_waiters` (reasoned change: the follower fetches itself)
- **`test_local_reads.py` (2, the catalog half)** in `catalog_detail.rs`. `known_book_reads_never_touch_open_library` covers the detail by id and by `OL…W`, the editions and the ISBN lookup. `search_metadata_serves_later_local_reads` is adapted: the search persists the metadata, and a failing enrichment serves it.
- **`test_google_books.py`:**
  - In `catalog_detail.rs`: tests 11 (the open half), 17, 18 (the open half), 19, 23, 24, 28 (ADAPT), 37, 38, 39 (the legacy-key half) and 43.
  - In `catalog_custom.rs`: test 30.
- **`tests/catalog_visibility.rs`:** readers, the rating mean and count and the quote count for every `Viewers` type, with `only_me` ratings and `only_me` quotes, plus the half-to-even rounding.
- **Provenance tests (`catalog_provenance.rs`):** a refresh after a user upload, a refresh after approved edits and an approved `cover_replace`, and the legacy upgrade by an Open Library record and by a DNB record for its ISBN.
- **Merge tests (`catalog_merge.rs`):**
  - Users with both works on their shelf, as favourites and in their layouts, two votes that nominated both, and every reference.
  - The field rules and the `ol_work_id` move.
  - Invalid pairs and chained redirects.

## Python tests reasoned away (one line each)

- `test_search.py::test_open_library_connect_timeout_is_short` → partly reasoned: the 8 s connect and 15 s read timeouts are `outbound.rs` constants (W1). `request_policy_is_short` checks the request policy CAT uses (15 s per attempt, 2 s throttle wait).
- `test_google_books.py::test_google_books_disabled_without_key` → Google Books removed (#41).
- `test_google_books.py::test_google_catalog_ids` → the Google id formats are dropped with Google Books (#41). The ISBN normalisation half is ported, and the legacy key classification belongs to IMP.
- `test_google_books.py::test_map_volume_prefers_isbn_key` → Google Books removed (#41).
- `test_google_books.py::test_map_volume_without_isbn_uses_volume_id` → Google Books removed (#41).
- `test_google_books.py::test_volume_details_strips_html` → Google Books removed (#41). Google and VLB descriptions are never stored.
- `test_google_books.py::test_empty_subject_preview_recovers_with_google_topic_search` → the Google topic fallback is removed (#41). The "open a displayed result without another call" part is Part 2 (detail).
- `test_google_books.py::test_expired_discovery_does_not_supply_book_details` → the discovery cache is replaced by persisting on search (#41).
- `test_google_books.py::test_volume_key_detail_and_refresh` → Google Books removed (#41).
- `test_google_books.py::test_isbn_cover_url_from_work_key` → there are no client-facing Open Library cover URLs any more (#43). Covers are `/media/…`.
- `test_google_books.py::test_google_cover_wins_over_open_library_isbn_cdn` → Google Books removed (#41).
- `test_google_books.py::test_isbn_cdn_skipped_when_google_key_is_set` → Google Books removed (#41).
- `test_google_books.py::test_title_and_popular_sorts_stay_on_google` → the Google sort mapping is removed (#41). Local sort semantics: `title` also orders local hits.
- `test_google_books.py::test_volume_key_without_google_key` → Google Books removed (#41). GB keys resolve through `legacy_work_keys` (Part 2 detail and IMP).
- `test_google_books.py::test_opening_discovered_book_does_not_call_google` (the "no row after browsing" and "a further search makes exactly one call" parts) → INVERT: browsing stores every hit (Part 1), and opening a stored stub tries its one-time enrichment, which after an upstream 429 trips the Open Library cooldown. A 503 trips none (tested).
- `test_google_books.py::test_discovered_book_opens_during_existing_google_cooldown` (the `/work/ISBN…` case) → `/work/{id}` is dropped (endpoint-map §4); the `/works/` case is ported.
- `test_google_books.py::test_discovered_book_refresh_still_fetches_fresh_metadata` ("exactly 1 call") → a refresh fetches the work JSON and its search doc. The test asserts that the work JSON is fetched again despite the cache.
- `test_google_books.py::test_refresh_updates_google_isbn_book` (the Google and shelf-stub parts) → Google Books removed (#41), and shelving needs an existing work. The "refresh fills a bare record" part is ported.

## Left for other packages, or PAR

- **SHELF, CLUB and POSTS:** `test_google_books.py` tests 27, 41, 44, 45, 46, 47 and 50.
- **The non-catalog half of `test_local_reads.py`** (shelf, pick, history, overlap, members, vote, suggestions, diary, activity, stats, quotes and book posts read locally) belongs to the packages that own those reads.

## For FE and PAR

- `SearchHit.cover_url` and `cover_thumb_url` are same-origin `/media/{public_id}/l` and `/s`, or null. The client never needs Open Library URLs.
- `club_picks` lists only the caller's clubs (from `bc_member_clubs`), newest membership first.
- `GET /api/books/isbn/{isbn}` answers `IsbnHitOut {isbn, work_id, edition_id, ol_work_key, title, authors, year, pages, cover_url, cover_thumb_url, cover_public_id, on_shelf, shelf_id}`. The title and pages are the edition's when it has its own.
- `CatalogService::clear_caches()` exists for tests that need "the RAM caches are gone, reads stay local".
- Prefetch: `hits::prefetch_covers` queues only Open Library media rows that were never fetched (`storage_key IS NULL`) and not removed. MED's worker decides about `fetch_failed_at`.
- **Book detail.** `GET /api/books/works/{work_id}` answers `BookDetailOut`, the endpoint-map §4 fields:
  - `on_shelf` is the viewer's status or null (deviation 17).
  - `club_picks` replaces `club_pick`.
  - `readers` lists only the viewer's audience, and an `only_me` rating is null in it.
  - `can_refresh` decides whether "Refresh details" is shown.
  - The page links to `/book/{id}`, with `id` from the answer (a legacy key or `OL…W` resolves to it).
  - `/api/books/work/{id}` and its refresh are gone.
- **Refresh errors:**
  - 400 `This book was added by a reader, not Open Library.`
  - 429 `Slow down a little. Try again later.` (10 per hour), with `Retry-After`.
  - 429 or 502 for a busy or unavailable Open Library.
- **`POST /api/books/custom`:**
  - It answers 201 `BookDetailOut`: use `id`, since there is no `BC…` key.
  - With an ISBN, a 409 carries `work_id` and `edition_id`, so the sheet can offer "open that book".
  - 429 and 502 mean the ISBN check could not answer.
  - The limit is 30 per day, shared with adding editions.
- **Edit and merge requests** answer 201 `{id, status: "open"}`, and their 400s are listed under Strings.
- **For MED:**
  - Approve expects a `cover_replace` row with `work_id` (the current work), an optional `edition_id`, the uploaded `media_id` (kind `cover`) and `payload` `{}`.
  - The approved cover gets provenance `admin`. It goes on the edition when the request names one, else on the work.
- **For SHELF, POSTS and CLUB:**
  - `merge_works` re-points shelf entries, favourites, layouts, quotes, posts, events, picks and nominations, and resolves their collisions (deviation 26).
  - It locks the affected users `FOR UPDATE` after the two works, so SHELF's user-first writes do not deadlock with it.

## For the integrator

- `tests/catalog_no_google.rs` greps `server/{src,tests,migrations}`, `Cargo.toml`, `Dockerfile` and `compose.yaml` for Google Books hosts and settings. Exempt are `src/importer/**` and `tests/fixtures/legacy/**` (legacy-mapping.md §0), and the scanner file itself. One W1 line is allow-listed: the negative assertion `"https://books.google.com/books/content?id=x",` in `src/outbound.rs`'s redirect-guard test. If that test switches to another non-upstream host, the allow-list entry can go.
- No migration, no new crate, no frozen file changed.
- **Part 2 routes:**
  - The admin routes carry tag `admin` and operation ids `catalog_admin_*`.
  - The `/api/books/*` routes carry tag `catalog`.
  - New schema names: `BookDetailOut`, `BookReader`, `CustomBookIn`, `NewEditionIn`, `EditionList`, `EditRequestIn`, `MergeRequestIn`, `EditRequestCreated`, `EditRequestOut`, `EditRequestList`, `EditRequestRejectIn` and `WorkMergeIn`.
- **`.sqlx/`:** Part 2 adds query files and replaces one Part 1 file, the cover-check `UPDATE`, which moved into `attach_checked_cover`.
