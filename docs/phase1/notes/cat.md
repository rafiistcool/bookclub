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

**Part 2 (open):** detail (`GET /api/books/works/{work_id}` with the one-time enrichment of stubs and the DNB cover check on the first view), refresh, custom books, editions, edit and merge requests, the admin routes, `catalog/merge.rs`, `tests/catalog_visibility.rs`, and the remaining parity tests (listed below).

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

## Strings

No new detail strings. The catalog answers with the legacy strings of spec/catalog.md §1.11 and endpoint-map §4, byte for byte. They are consts in `catalog/mod.rs`. None of them is in `apiErrors.ts`, and #48 may map them (optional). The 429 hint is a template, `The library is busy. Try again in {n} seconds.`; the frontend must match it by prefix if it maps it.

## Python tests: ported in Part 1

- **`test_search.py` (23 of 35):**
  - In `tests/catalog_search.rs`: every search, trending, subject, cache and error test.
  - Unit tests in `catalog::cache`: `test_search_cache_is_bounded`, `test_search_cache_ttl_is_at_least_half_a_day`, `test_identical_inflight_searches_share_one_request` (also in `catalog_search.rs`, through the API) and `test_cancelled_fetch_unblocks_coalesced_waiters` (reasoned change: the follower retries).
  - A unit test in `catalog::search`: `test_search_candidates_keep_raw_query_before_by_split`.
- **`test_book_details.py` (3 of 8):** `test_isbn_lookup` and `test_isbn10_with_x_check_digit_is_accepted` in `catalog_isbn.rs`, and `test_positive_cover_id_rejects_placeholders` in `catalog::openlibrary`.
- **`test_google_books.py` (search and lookup KEEP and ADAPT):** tests 2 (ISBN half), 5, 7, 9 (search, trending, subjects, ISBN, DNB SRU), 10, 12, 13, 14, 15, 16, 18 (browse half, INVERT), 20, 21, 22, 29, 31, 35, 39 (lookup half), 40 (INVERT), 48, 49 and 51. Test 8 (INVERT) is ported as "a miss falls through to the next source".

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

## Left for Part 2, other packages, or PAR

- **Part 2:**
  - `test_search.py` (12): `test_first_import_skips_author_api_when_search_has_names`, `test_book_detail_resolves_description_subjects_and_authors`, `test_book_detail_accepts_dict_description`, `test_book_detail_flattens_markdown_in_description`, `test_book_detail_includes_own_shelf_state_and_other_readers`, `test_book_detail_rejects_malformed_work_id`, `test_book_detail_propagates_not_found`, `test_detail_survives_author_lookup_failure`, `test_book_detail_refresh_updates_local_row`, `test_refresh_bypasses_author_cache`, `test_local_catalog_is_searched_without_open_library_for_custom_books`, `test_custom_book_requires_a_title`.
  - `test_book_details.py` (5): `test_work_details_for_unknown_book`, `test_work_details_persist_on_known_book_and_list_members`, `test_work_details_rejects_bad_ids_and_upstream_failure`, `test_work_details_skips_leading_negative_cover`, `test_cancelled_work_details_unblocks_coalesced_waiters`.
  - The catalog half of `test_local_reads.py`.
  - `test_google_books.py` tests 11 (the open half), 17, 18 (the open half), 19, 23, 24, 28, 30, 37, 38, 39 (the legacy-key half) and 43.
  - `catalog_client.rs` already checks `ol_work_record` (description and subject cleaning, cover order, the author rules, refresh bypass, 404) at the service level, so the Part 2 ports only need the endpoints.
- **SHELF, CLUB and POSTS:** `test_google_books.py` tests 27, 41, 44, 45, 46, 47 and 50.

## For FE and PAR

- `SearchHit.cover_url` and `cover_thumb_url` are same-origin `/media/{public_id}/l` and `/s`, or null. The client never needs Open Library URLs.
- `club_picks` lists only the caller's clubs (from `bc_member_clubs`), newest membership first.
- `GET /api/books/isbn/{isbn}` answers `IsbnHitOut {isbn, work_id, edition_id, ol_work_key, title, authors, year, pages, cover_url, cover_thumb_url, cover_public_id, on_shelf, shelf_id}`. The title and pages are the edition's when it has its own.
- `CatalogService::clear_caches()` exists for tests that need "the RAM caches are gone, reads stay local".
- Prefetch: `hits::prefetch_covers` queues only Open Library media rows that were never fetched (`storage_key IS NULL`) and not removed. MED's worker decides about `fetch_failed_at`.

## For the integrator

- `tests/catalog_no_google.rs` greps `server/{src,tests,migrations}`, `Cargo.toml`, `Dockerfile` and `compose.yaml` for Google Books hosts and settings. Exempt are `src/importer/**` and `tests/fixtures/legacy/**` (legacy-mapping.md §0), and the scanner file itself. One W1 line is allow-listed: the negative assertion `"https://books.google.com/books/content?id=x",` in `src/outbound.rs`'s redirect-guard test. If that test switches to another non-upstream host, the allow-list entry can go.
- No migration, no new crate, no frozen file changed.
