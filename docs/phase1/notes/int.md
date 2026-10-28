# INT (Wave 3, #42 #45): integrator follow-ups

Owner: INT. INT does what the Wave 2 notes (`docs/phase1/notes/*.md`) left "for the integrator", unless it belongs to FE (#48) or to PAR (#49, parity lines). These notes record what was built, the decisions behind it, and every other integrator item: done, or left with a reason. The other packages' notes are unchanged (they are theirs), so where they still describe a gap that INT closed, this file is the record.

## What was built

### (a) Docs: time zones and locks

- **conventions.md §12 "Which zone applies".** The `created_label` row now reads: "the viewer's zone for personal items, the item's club zone for club items" (work-plan appendix diary 11 and club 4). This is how POSTS (`notes/posts.md` deviation 4) and CLUB (`notes/club.md` "Activity feeds") built it, and the integrator kept it. A paragraph below the table says where a club item's label differs from the response's `timezone`. endpoint-map.md §8 (the work thread, `/api/diary`) and §9 (`/api/activity`) now say the same.
- **conventions.md §13 "Locks".**
  - New row: the club row `FOR NO KEY UPDATE` for every pick, milestone and vote write (nominate, cast, deadline, apply, auto-close). It comes before the vote row, and the vote row's line says "after the club row".
  - New bullet, "Lock order of a club write". It gives the order of pick and vote writes (work, club, users, vote, pick) and the order of club posts, replies and reactions (club, work, milestone, parent or post). It also explains why the two never block each other, and why both agree with `catalog::merge_works` and with the membership writes.
  - New bullet: CAT's rule for writes that store a `work_id`, with the harvest's variant and CLUB's accepted race.
  - Sources: `notes/club.md` ("Locks and concurrency", "Votes", "Work merges", "Docs (integrator)"), `notes/posts.md` ("Lock order", "Work lock") and `notes/cat.md` ("For SHELF, POSTS and CLUB").
- **conventions.md §13 "Migrations"** lists the migrations after `0001_init.sql`: MED's `1300_media_removed_hashes.sql` and INT's `1200_editions_dnb_record_idn.sql`.
- **conventions.md §7.2 and §7.5:**
  - `EditionRecord` gets `dnb_record_idn`.
  - `upsert_edition_with_work` writes it.
  - `withdraw_dnb_edition` has its new rule (the signature is unchanged).
  - The "ISBNs" and "Deleted records" bullets of the mapping are updated.
- **work-plan.md:** HAR's "Deleted headers" bullet, a short INT section under Wave 3, and INT in the dependency table.

### (b) A deleted multi-ISBN DNB record withdraws every edition (HAR's open point)

HAR's open point (`notes/har.md` "Open points", "Blocked on a frozen interface"): `map_record` gave only the first ISBN's edition the record's IDN (`editions.dnb_idn` is unique). `withdraw_dnb_edition` therefore withdrew only that edition, and the record's other editions stayed live. HAR hid them in new-releases with a sibling clause (`notes/har.md` decision 15).

The fix:

- **Migration `server/migrations/1200_editions_dnb_record_idn.sql`** (HAR's block). It adds `editions.dnb_record_idn text COLLATE "C" NULL CHECK (char_length BETWEEN 1 AND 32)`, not unique. It also adds the partial index `editions_dnb_record_idn_idx` and a backfill `dnb_record_idn = dnb_idn` for rows written before it. It updates the comments on `dnb_record_idn` and `withdrawn_at`.
- **`EditionRecord.dnb_record_idn: Option<String>`.** `EditionRecord::new` sets it to `None`, and every caller uses struct update syntax, so no caller changed.
- **`dnb_marc::map_record`** sets `dnb_record_idn: Some(idn)` on every edition of the record. `dnb_idn` stays on the first edition only.
- **Ingest (`CleanEdition`):**
  - Cleaning: `dnb_record_idn` is cleaned like `dnb_idn` (dropped, never truncated, when it breaks the limits). When the record has none, it falls back to the cleaned `dnb_idn`: an edition that carries an IDN was delivered by that record.
  - Writes: `insert_edition` writes the column, and `find_edition` reads it.
  - Updates: `update_edition` replaces it whenever the record carries a different one (the newest delivering record wins). Since the column is not unique, nothing moves off other editions.
  - Lookup: editions are still found by ISBN, then `dnb_idn`, then OL id, never by the record IDN.
- **`withdraw_dnb_edition(conn, idn)`** now runs `UPDATE editions SET withdrawn_at = now(), updated_at = now() WHERE dnb_record_idn = $1 AND withdrawn_at IS NULL`. The signature and the "never inserts" rule are unchanged.
- **`harvest::new_releases`** drops the sibling clause of HAR's decision 15. Its two limitations go with it:
  - another ISBN of the record that ingest found under a different work is withdrawn now
  - an IDN-less DNB edition of another record of the same work, with the same date, is no longer hidden
- CAT's SRU ingest, IMP's importer and HAR's harvester are otherwise unchanged. They call the same functions; the importer's records carry no IDN.

### (c) One annotation for search hits and new-releases

`catalog::hits::annotated_hits(conn, viewer, &[ListedHit])` builds the `SearchHit`s of a list.

- **Input:** each `ListedHit` is a `BookRef`, which may carry an edition context, plus the hit's `isbn` and `planned_publication_date`.
- **Annotation:** `on_shelf`, `shelf_id` and `club_picks` come from the viewer, through `hits::shelf_entries` and `hits::club_picks`.
- **Callers:**
  - `search_hits` (CAT's search) is a thin wrapper: the work without an edition context, and no date.
  - `harvest::new_releases_page` calls it with the edition context and the planned date. Its private copies of `shelf_entries` and `club_picks` are gone.
- **Unchanged:**
  - A repeated `BookRef` is still dropped, as both copies did before.
  - The book detail (`detail.rs`) and the ISBN lookup (`isbn_lookup.rs`) keep calling `club_picks` and `shelf_entries` for their own DTOs.

### (d) The other integrator items

Done:

- POSTS deviation 4 and CLUB "Docs (integrator)": (a) above.
- HAR decision 15 ("the integrator may point both at one function"): (c) above.
- MED "For PAR and the integrator": the migration is listed in conventions §13 (the schema overview's "one addition"). The other lines need nothing: `run_prefetch` is started as before, and `.sqlx/` was regenerated here too.

Left, with the reason:

- **CAT "For the integrator", parity lists** ("add the handoff lines under 'Left for other packages, or PAR' to the SHELF, CLUB, POSTS and PAR parity lists"): these are parity lines, PAR's. The same goes for MED's "copy the reasoned-away line into `parity-notes.md`", HAR's "For PAR" and OAPI's "For PAR".
- **CAT, `catalog_no_google.rs` allow-list:** nothing to do. The allow-list entry can go only once W1's redirect-guard test in `src/outbound.rs` uses another host, and nobody changed it.
- **CAT, "No migration…", "Part 2 routes", "`.sqlx/`":** information for the merges, already acted on when CAT merged.
- **OAPI, "run `npm run gen:api` after each merge":** this is the integrator's merge routine, and `frontend/src/api/generated.ts` is written only by the script, which runs in FE's tree. INT changes no DTO. It does change one operation description, the doc comment of `harvest_new_releases` (`GET /api/books/new-releases`; the old text named the sibling rule), so **`generated.ts` must be regenerated after merging INT**, or CI's `check:api` fails.
- **SHELF, "the integrator can make the layout reader public if IMP should share it":** not done.
  - It changes SHELF's and IMP's files (`shelf/layout.rs`, `importer/layout.rs`), which are outside INT's ownership and are not callers of a function INT changed.
  - It is not needed: IMP stores layouts already canonical and pruned to the owner's entries (`notes/imp.md` "For other packages", SHELF), so the reader reads them unchanged.
  - IMP's reader also does more than SHELF's: it prunes, and it reports what it dropped. Sharing would mean a new common function, not making the existing one public.
  - The existing rule stays: a change to the `PhysicalShelfLayout` rules is mirrored in `importer/layout.rs`.
- **CLUB, "SHELF / integrator", the flaky `shelf_import.rs::two_runs_of_one_job_handle_each_row_once`:** already fixed by 9b03a09 (the fake catalog ingests each hit in one transaction). It did not fail in INT's runs.

Not an integrator item, but the same kind as (c), for a later clean-up: IMP builds club slugs with its own copy of `clubs::manage::slug_base` (`notes/imp.md` Part 1, "The slug").

## Decisions and deviations

1. **The withdraw rule is "the newest delivering record", not HAR's proposed `dnb_idn = $1 OR dnb_record_idn = $1`.**
   - `dnb_idn` can lag behind. Take an edition first delivered as the first ISBN of record R1 and later as a second ISBN of record R2: it keeps `dnb_idn` R1, because only a record that names the edition first moves `dnb_idn`.
   - With the `OR`, deleting the old duplicate R1 would withdraw an edition that R2 still lists. With `dnb_record_idn = $1` it stays live.
   - Every edition with a `dnb_idn` also has a `dnb_record_idn`, through ingest's fallback and the migration's backfill. So the single predicate covers what the `OR` covered.
   - `tests/ingest.rs::a_deleted_record_withdraws_the_editions_it_delivered_last` pins this.
2. **Only the newest delivering record is tracked,** as with `dnb_idn`.
   - If two live records list the same ISBN and the newer one is deleted, the edition is withdrawn, although the older record still lists it. A later delivery of the older record brings it back.
   - A record delivered again without one of its earlier ISBNs leaves that edition's `dnb_record_idn` as it was. Deleting the record later withdraws that edition too, since the record was the last to deliver it. That is usually a corrected ISBN typo.
   - No statement clears stale record IDNs: the extra write per record would buy little.
3. **The backfill in migration 1200** keeps databases migrated before it consistent. That covers dev databases and the integrator's compose server; no production database exists yet.
4. **`dnb_record_idn` is not an advisory-lock key.** It is not unique and identifies no edition. Ingest's row lock on the edition (`find_edition … FOR UPDATE`) covers its write, and `withdraw_dnb_edition` stays a single `UPDATE`, as before.
5. **Docs changed beyond the asked §12 and §13 rows:**
   - conventions §7.2 and §7.5 (the frozen `EditionRecord`, `withdraw_dnb_edition` and the mapping notes now describe the new field and rule)
   - endpoint-map §5 (the new-releases row), §8 and §9 (the zones)
   - work-plan HAR and Wave 3

No new user-visible string, no new crate, no new route, no DTO change.

## For the integrator

- **After merging `phase1/int`:**
  1. Run `cargo sqlx migrate run` against the compose server. 1200 is applied even though 1300 already is: sqlx applies missing lower versions, as checked here on a database migrated to 1300 first.
  2. Run `cargo sqlx prepare -- --all-targets`.
  3. Run `npm run gen:api`: the `harvest_new_releases` description changed.
- **`notes/har.md` is stale:** its "Open points" item and decision 15's last bullets still describe the gap and the workaround. That file is HAR's, so it was not edited. This file records the fix.
- **`.sqlx/`:** six files were replaced, because six queries changed:
  - `find_edition`, `insert_edition`, `update_edition` and `withdraw_dnb_edition` in ingest
  - `new_releases_page`, without the sibling clause
  - `tests/harvest_catalog.rs::glitzer_editions`, which reads the new column

  The two private queries removed from `new_releases.rs` had the same text as `hits::shelf_entries` and `hits::club_picks`, so they shared those files and nothing else went.

## For FE (#48)

- No API shape change. `GET /api/books/new-releases` no longer lists a work whose DNB announcement was deleted, whichever of its ISBNs is in the window. Before, it hid only editions with the first edition's date.

## For PAR (#49)

- No Python test is involved: the Python app has no DNB harvester.

## Tests

The server suite has 1147 tests (phase1-green: 1145).

- `tests/harvest_catalog.rs` (15, one rewritten): `a_deleted_multi_isbn_record_leaves_new_releases` now covers:
  - both editions carry `dnb_record_idn`
  - with only the second ISBN's edition in the window, that edition lists the work
  - the deleted header withdraws both editions (`[true, true]`, before `[true, false]`)
  - neither edition lists the work afterwards, whatever its date; another title stays
  - the record harvested twice more keeps the same two editions (one per ISBN, two on the work), live again, and back in the list
- `tests/ingest.rs` (+1): `a_deleted_record_withdraws_the_editions_it_delivered_last`.
  - Record 100 delivers A and B.
  - Record 200 then delivers B and a new C: B's `dnb_idn` and record IDN become 200, and C's record IDN is 200.
  - Deleting 100 withdraws only A. Deleting 200 withdraws B and C.
  - Record 100 again: A and B live, B's `dnb_idn` stays 200 and its record IDN is 100, C stays withdrawn. Three editions in all.
- `tests/dnb_marc.rs` (+1, one extended):
  - `a_multi_isbn_record_gives_one_edition_per_isbn` checks the record IDN on both editions.
  - New: `a_deleted_multi_isbn_record_withdraws_every_edition` runs `multi_isbn.xml` through ingest. The withdraw takes both ISBNs, and two more ingests keep two editions, both live.
- Unit tests, extended:
  - `catalog::shared::dnb_marc` checks the record IDN on all three editions of the inline multi-ISBN record and on a record without an ISBN.
  - `catalog::shared::ingest` checks the cleaning of `dnb_record_idn` and its fallback to `dnb_idn`.
- (c) is covered by the existing tests: `catalog_search.rs` (shelf and club-pick annotation of search hits) and the new-releases tests in `harvest_catalog.rs` (the viewer's shelf and club picks, the edition's own title and ISBN).
- Mutation check: with `withdraw_dnb_edition` back on `WHERE dnb_idn = $1`, all three new or rewritten withdraw tests fail. Those are `harvest_catalog.rs::a_deleted_multi_isbn_record_leaves_new_releases`, `ingest.rs::a_deleted_record_withdraws_the_editions_it_delivered_last` and `dnb_marc.rs::a_deleted_multi_isbn_record_withdraws_every_edition`.
