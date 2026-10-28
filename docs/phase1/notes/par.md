# PAR (#49) implementation notes

Owner: PAR. These notes record what the parity package built, the decisions it had to make, and what the integrator needs to know. The design documents themselves are unchanged.

## Part 1: the checklist (done)

### Files

| File | Contents |
|---|---|
| `docs/phase1/parity-checklist.md` | One section per `backend/tests` file and one line per Python test function (292 functions, 358 cases), each with a status and its Rust test(s) or reason; a summary table; the known intentional changes with the tests that pin them; the #37 schema acceptance criterion as built. |
| `docs/phase1/parity-notes.md` | The Wave 1 sections, plus Wave 2 sections copied from the package notes (CAT's reasoned-away lines and the reasoned changes it asked for, MED's avatar-blob line, CLUB's ten `test_push.py` lines, the POSTS and IMP moves and replacements). The two Wave 1 "moved to IMP" lines now name IMP's ports. |
| `server/tests/parity_checklist.rs` | The staleness check (below). No database. |
| `server/tests/parity_ports.rs` | 12 end-to-end ports (below). |
| `server/tests/parity_schema.rs` | The #37 schema check (below). |

No file in `server/src` changed: Part 1 found no server bug. No migration, no new crate, no frozen file touched, no `.sqlx/` change (the new tests use runtime `sqlx::query`, like the other test files).

### The staleness check

`cargo test --test parity_checklist` reads `backend/tests/test_*.py`, every `/// parity:` comment in `server/src` and `server/tests`, `docs/phase1/parity-notes.md` and the checklist. It fails when:

- a Python test function has no line, or a section's count differs from its file;
- a line is not `- [x]`, has no status, or a **ported**, **changed** or **replaced** line names no Rust test, or a **dropped** line has no reason;
- a parity comment sits on a Rust test that its Python test's line does not name;
- the checklist names a Rust test (anywhere in the file) that does not exist;
- a parametrised Python test's line has no "(N cases)";
- the summary table disagrees with the sections (test count and the four statuses per file, and the total row);
- a Python test listed in `parity-notes.md` has no line.

Mutation-checked: deleting a line, unchecking one, removing a status, renaming a referenced Rust test, changing a summary number and adding a parity comment to another Rust test each fail with a message that names the line.

Decisions:
- **A Rust test, not a Python script.** CI runs it with `cargo test`, and CI's server job has the whole checkout. It finds the repository through `CARGO_MANIFEST_DIR/..`.
- **Lines for deleted Python tests are allowed.** FE (#48) deletes `backend/tests/test_api_error_copy.py`; its line stays. A typo in a test name is still caught, because the real name then has no line.
- **Python test discovery is textual:** `def test_*` at column 0, with decorators collected to see `parametrize`. A test class or a nested `def test_*` makes the check fail with a message, so it cannot silently miss tests.
- **At the cutover (#52)** `backend/` goes away: the check then fails with "delete this test". The checklist stays as the record.

### Statuses

Each line carries one of four statuses (checklist "How this list is built and kept"): **ported**, **changed** (ported, behaviour changed on purpose, reason on the line), **replaced** (another feature took its place), **dropped** (gone, reason on the line). The verdicts of `test_google_books.py` follow `docs/phase1/spec/google.md` §4: KEEP is ported, ADAPT and INVERT are changed, DROP is dropped, and mixed tests are changed with the dropped part named. Totals: 171 ported, 88 changed, 6 replaced, 27 dropped.

Status choices where the package notes left room:
- `test_auth.py::test_preference_columns_backfill_pre_existing_users` and both `test_migrations.py` tests are **replaced** (the in-place upgrade became the importer), not "moved".
- `test_diary.py::test_legacy_pick_notes_are_imported_once_with_reactions` is **changed** (moved to IMP, same behaviour at import time).
- `test_selfhost.py::test_dotenv_only_production_boot` is **dropped** (as parity-notes.md says), although `auth_config.rs::a_production_app_registers` carries its parity line; that test is named on the line as what is left.

### End-to-end ports (`tests/parity_ports.rs`)

Each has a `/// parity:` line. They run through the routes of several packages, against the real `CatalogService` (`TestApp::with_real_lookup`) where a lookup is involved, with wiremock for every upstream.

| Rust test | Python test | Why PAR |
|---|---|---|
| `known_book_reads_never_touch_open_library` | `test_local_reads.py::test_known_book_reads_never_touch_open_library` | CAT ported the catalog reads with SQL-made rows. This writes the shelf entry, pick, diary entry, pick post, quote and nomination through their routes, then reads 25 paths (every legacy one, club-scoped, plus the club and home variants): 200 each, zero upstream requests. |
| `shelving_persists_search_metadata_for_later_local_reads` | `test_local_reads.py::test_shelving_persists_search_metadata_for_later_local_reads` | The shelf add's `BookOut` half (SHELF) after a real search; the book page and the shelf serve the stored hit with Open Library down. |
| `a_known_legacy_isbn_book_skips_upstream` | `test_google_books.py::test_known_google_book_skips_upstream` | CAT handoff, split across SHELF, CLUB and POSTS: a book under a legacy `ISBN…` key is opened by that key, shelved, picked and given a diary, with no upstream request. |
| `the_goodreads_fixture_imports_through_the_real_lookup` | `test_google_books.py::test_goodreads_import_prefers_google` | CAT handoff and work-plan PAR: the fixture CSV against the real lookup (Open Library by ISBN with CAT's recorded answer, by title and author, a definite miss), then a second export: a 978-3 ISBN goes to the DNB (CAT's recorded SRU record) and never to Open Library's search, and a known ISBN resolves locally. |
| `a_goodreads_import_during_a_cooldown_skips_without_asking` | `test_google_books.py::test_goodreads_import_while_cooling_down` | CAT handoff: after a 429 with `Retry-After: 3600`, the import is 202, each row is `catalog_unavailable` at once, and Open Library gets no request. |
| `a_pick_never_stores_a_client_cover` | `test_google_books.py::test_pick_does_not_persist_ol_cdn_cover_url` | CAT handoff to CLUB. |
| `a_shelf_add_never_stores_a_client_cover` | `test_google_books.py::test_shelf_rejects_non_google_cover_url` | CAT handoff to SHELF. |
| `isbn_x_and_x_are_one_shelf_entry` | `test_google_books.py::test_shelf_isbn_x_and_X_are_the_same_row` | CAT handoff to SHELF: through the ISBN lookup and the legacy key map. |
| `finish_notes_appear_on_the_club_pick` | `test_shelf.py::test_finish_notes_appear_on_club_pick` | SHELF checked `bc_visible_shelf`, CLUB used SQL shelf rows; this goes PATCH → pick readers. |
| `reading_progress_shows_on_a_friends_shelf_and_the_pick` | `test_shelf.py::test_reading_progress_shows_on_friend_shelf_and_pick` | As above, plus a co-member who joined by invite. |
| `the_club_story_is_recorded_by_every_route` | `test_activity.py::test_feed_records_the_club_story_newest_first` | CLUB inserted the shelf, post and reaction events with SQL; here every event comes from its own route (SHELF, CLUB, POSTS), in both feeds. |
| `favourite_positions_show_on_the_book_page` | `test_favorites.py::test_replace_order_clear_and_eligibility` | SHELF left the `favorite_position` assertions to CAT's book page; here PUT favourites → book page, and clear. |

Findings while writing them (no server change needed):
- The book page of a stored search stub runs its one-time enrichment, so "no upstream request" holds only for known works (`details_fetched_at` set) or works of another source. The ports count requests only where that holds.
- A 409 from `POST /api/shelf` carries `item` next to `detail`, so `assert_detail` (exact body) does not fit; the ports check the status and `detail()`.

### The #37 schema acceptance criterion (`tests/parity_schema.rs`)

`club_scoped_tables_carry_club_id_and_personal_tables_do_not` classifies every table of the live schema and checks the class:
- club data: `club_id NOT NULL` with a foreign key over `club_id` to `clubs` or to another club table (the composite keys);
- `club_invites`: nullable `club_id` → `clubs` (NULL = a sign-up invite);
- deviation 1 (`posts`, `events`): nullable `club_id` → `clubs`, plus `posts_parent_fk` on `(parent_id, work_id, scope_key)`, `posts_club_needs_pick`, `posts_visibility_scope`, `events_scope`, `events_personal_no_pick`;
- deviation 2 (`post_reactions`, and `club_invite_uses` by the same pattern): no `club_id`, a NOT NULL foreign key to the parent with ON DELETE CASCADE;
- personal data: no `club_id`, `user_id` → `users`;
- everything else (accounts, `clubs`, catalog, media, harvest state): no `club_id`.

`club_invite_uses` is not one of the two deviations #37 records. It is a redemption log under an invite, so it inherits its scope like `post_reactions`; the checklist names it under deviation 2. **For the integrator:** add it to the #37 note and the schema header comment if you agree, or tell PAR to classify it otherwise.

## For the integrator

- **A new table fails `parity_schema.rs`** until it is added to one of its classes and to the checklist's #37 section. INT's `server/migrations/12*.sql` may add one; if so, classify it in the same merge (PAR owns the file, but a one-line addition to a list during the merge is the expected fix).
- **A new or renamed parity comment, or a renamed Rust test named in the checklist, fails `parity_checklist.rs`.** The message names the line to edit.
- **#48** deletes `test_api_error_copy.py`; nothing in the checklist needs to change for that.
- No `blocked_on_frozen` item from Part 1. HAR's open point (a deleted multi-ISBN record withdraws only its first edition, `docs/phase1/notes/har.md` "Open points") is in INT's files now; PAR did not touch it.

## Left for Part 2

- The end-to-end tests of work-plan PAR not yet written: the importer run with the real lookup, offline (IMP's note: `--offline` checks only `lookup::local_isbn`, so it must make no upstream request; `rematch-legacy` calls `resolve_isbn_remote` once per legacy edition with an ISBN), and the #46 matrix completeness check (every personal data kind has self, co-member, stranger and unknown-name cases; SHELF's `tests/shelf_visibility.rs` covers quotes and stats).
- Other seams where a package's port stands in for another package's route with SQL, still unported end to end: overlap and suggestions on shelf rows (`club_overlap.rs`, `club_suggestions.rs`), the pick readers in `club_picks.rs::readers_split_into_reading_and_finished_and_an_admin_clears_the_pick`, the milestone threads (`club_milestones.rs::milestones_crud_and_threads` inserts the posts), the old pick's thread after `vote/apply` (`club_votes.rs::applying_the_winner_makes_it_the_pick`), the pick in the stats (`shelf_stats.rs::stats_year_in_review`), the `quote_added` event in the feed (`shelf_quotes.rs::quotes_crud_and_filters` reads the `events` row), and the picks and milestones of `posts_picks.rs`.
- A review of the parity comments themselves (does each Rust test check what its Python test checked?). Part 1 took the packages' comments and notes as given, apart from the end-to-end gaps above.
