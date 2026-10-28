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

## Part 2: the end-to-end tests and the review (done)

### Files

| File | Contents |
|---|---|
| `server/tests/parity_importer.rs` | 2 tests: the legacy importer `--offline` and `rematch-legacy` against the real lookup (below). |
| `server/tests/parity_visibility.rs` | 2 tests: the #46 matrix completeness check (below). |
| `server/tests/parity_ports.rs` | 7 more end-to-end ports (19 in all): the seams Part 1 left open (below). |
| `docs/phase1/parity-checklist.md` | The new ports named on their lines, the "#46 visibility matrix, as built" section, one status corrected and one shared change recorded by the review (below), the summary updated (170 ported, 89 changed, 6 replaced, 27 dropped). |

No file in `server/src` changed: none of these tests found a server bug, so there is no fix and no `blocked_on_frozen` item. No migration, no new crate, no fixture file, no `.sqlx/` change (the tests use runtime `sqlx::query`). `parity-notes.md` is unchanged: the review found nothing to drop.

### The work-plan PAR end-to-end tests

All four are written:

- **`test_local_reads.py` in full** (Part 1): `parity_ports.rs::known_book_reads_never_touch_open_library` and `shelving_persists_search_metadata_for_later_local_reads`, against the real `CatalogService`.
- **The Goodreads import** (Part 1): `parity_ports.rs::the_goodreads_fixture_imports_through_the_real_lookup` covers the chain in order: a known ISBN resolves locally (the second export's Circe makes no second request), a 978-3 ISBN goes to the DNB SRU and never to Open Library's search, any other ISBN to Open Library, then the title and author search, and a definite miss.
- **The importer run with the real lookup, offline** (Part 2), `parity_importer.rs`:
  - `an_offline_import_with_the_real_lookup_asks_no_upstream`: fresh.sql (plus two ISBN books added to the temporary copy with `Legacy::exec`) is imported `--offline` into a target whose catalog already holds Middlemarch (`OL45883W`, full record) with the edition of the fixture's matched ISBN key. That key resolves locally and merges with book 9; the other four ISBN keys become bare facts. wiremock receives no request, neither during the import nor while two legacy users (signed in with their old password) make 24 reads across every package (shelf, layout, favourites, book pages by `BC…`, `ISBN…` and `OL…W` keys, the pick, history, posts, members, overlap, vote, suggestions, both feeds, stats, diary, quotes, search history).
  - `rematch_legacy_asks_each_legacy_isbn_once_along_the_real_route`: after the offline import, `rematch-legacy` asks once per legacy edition with an ISBN (4), along the real route: each 978-3 ISBN once to the DNB, then Open Library's search only after a DNB miss (a DNB hit makes the one Open Library cover check, `/isbn/{isbn}.json`, instead); the 978-0 ISBN to Open Library only; the locally matched key's edition (source `openlibrary`) not at all. 7 requests. The DNB hit and the Open Library hit upgrade their editions in place (`source` dnb and openlibrary, `match` `isbn_matched`), the shelf entries keep their work and edition, and the book page by legacy key and the shelf show the upgraded records without another request. A second run asks only about the two editions still bare facts (4 more requests).
  - Mutation-checked: without `offline` both tests fail (the import aborts on the unavailable source).
- **The #46 matrix completeness check** (Part 2): `parity_visibility.rs`, below.

### The #46 matrix completeness check (`tests/parity_visibility.rs`)

The checklist section "#46 visibility matrix, as built" has the table. How "complete" is enforced:

- `every_personal_column_route_and_rating_field_is_in_the_matrix` discovers, from the live schema and `bookclub_server::openapi()`, every foreign key to `users` (28 columns), every operation with a `username` parameter (9) and every schema with a `rating`, `take`, `ratings` or `*_rating` field other than `ol_*` (11). Each must be in the test's tables, with the kinds that serve it or the reason it is not personal data. Mutation-checked: dropping `GET /api/stats` or `PickStat` from the tables fails with a message naming it.
- `every_personal_data_kind_has_its_four_viewer_cases` builds one world through the routes (the owner's shelf with an `audience` and an `only_me` rating, favourites, a layout, an `audience` and an `only_me` quote and diary entry, reactions, a pick in each club, overlap and suggestion candidates, a search, an import job) and runs every kind the tables name across self, co-member, stranger (each of: a member of another club, someone in no club, a former co-member) and unknown name: 12 kinds, 48 cases, 45 checked and 3 not applicable.
- A kind cannot be added without its cases: `Kind` is matched exhaustively in `check`, each kind's function matches all four `Case`s, and a `Kind` variant no table names is never constructed, which `dead_code` turns into a clippy error. Only the unknown-name case may be "not applicable", and the test asserts that.

Decisions:
- **Unknown name where there is no name.** The book page, the pick readers and the club views (overlap, suggestions, members, club stats) take no username, so their unknown-name case is "not applicable", as `catalog_visibility.rs` already says; their stranger case checks that the owner is absent (and that another club's view leaves the owner out even for a co-member who can see the owner's shelf). For the diary, the search history and import jobs the unknown name is an unknown id, and the test compares the stranger's answer for the owner's item with the answer for an id that names nothing.
- **Owner-only kinds are in the matrix.** `search_history` and `import_jobs` are personal tables nobody else may read; the matrix checks that a co-member and strangers get 404 (`Search history entry not found`, `No such import`) like for an unknown id, and that the owner's row survives their attempts.
- **What is not personal data** (each with its reason in the test): sessions and email tokens (account), memberships, removals, invites, picks, milestones, nominations and ballots (club data, members only), `works.created_by`/`editions.created_by` (provenance, never served), catalog edit requests and reports (admin moderation), `media.uploaded_by` (never served; an avatar is served by its unguessable public id, MED), and the member-administration routes `PATCH`/`DELETE /api/clubs/{slug}/members/{username}`.
- As built by the packages, and confirmed here: for a stranger, `GET /api/quotes?username=` and `GET /api/activity?username=` answer 200 with an empty list, exactly like an unknown name; every other `?username=` route answers 404 `No member with that name` for both.

### The seams Part 1 left open (`tests/parity_ports.rs`)

Each carries the `/// parity:` lines of its Python tests and is named on their checklist lines.

| Rust test | Python test(s) | What runs end to end |
|---|---|---|
| `the_pick_readers_come_from_the_shelf_routes` | `test_pick.py::test_reading_vs_finished_and_clear` | SHELF's `POST /api/shelf` → CLUB's pick `reading`/`finished`/readers, a member joined by invite, admin-only clear, history. |
| `overlap_follows_the_shelf_routes` | `test_overlap.py::test_two_want_same_book_shows_one_does_not`, `::test_finished_and_solo_want_are_excluded`, `::test_include_reading_counts_want_plus_reading` | SHELF's POST and PATCH (want → reading → finished) → CLUB's overlap, with and without `include_reading`. |
| `suggestions_follow_the_shelf_and_pick_routes` | `test_vote_deadline.py::test_suggestions_rank_shared_tbr_and_subject_affinity` | SHELF's wants and CLUB's past pick → suggestions with scores and reasons; nominating and moving off the want shelf remove a book. |
| `milestone_threads_run_through_the_post_routes` | `test_stats_milestones_quotes.py::test_milestones_crud_and_threads` | CLUB's milestone → POSTS' pick post naming it (`note_count`), 400 `That milestone is not on this pick`, deleting the milestone detaches the post in the thread, `milestone_added` in the home feed. |
| `old_pick_threads_survive_a_new_pick_and_an_applied_vote` | `test_vote.py::test_apply_winner_uses_club_pick_flow`, `test_pick_posts.py::test_old_thread_stays_when_pick_changes`, `test_diary.py::test_entry_on_the_current_pick_records_the_pick` | CLUB's `PUT` pick and `vote/apply` → POSTS' threads: the note records its pick, old threads stay readable and closed, the new one starts empty and open. |
| `stats_count_what_the_routes_wrote` | `test_stats_milestones_quotes.py::test_stats_year_in_review` | CLUB's current pick (SHELF's port used an ended one made with SQL), SHELF's entries, `finished_on` of last year and a quote → personal and club stats for both years. |
| `a_quote_reaches_the_feeds` | `test_stats_milestones_quotes.py::test_quotes_crud_and_filters` | SHELF's quote → `quote_added` in CLUB's home and club feeds for the author and a co-member; after the delete it stays for the author only (§6.4). |

All passed against the merged packages: no seam was broken.

### The review of the parity comments

Method: a script (not committed) paired every Python test with the Rust tests its line and its parity comments name (259 ported or changed tests) and checked whether the string literals and status codes of the Python assertions appear in the Rust bodies. 43 pairs matched less than 60 % of them, and 44 ported pairs missed at least one. Those lists were screened, and 21 pairs (every ported one whose misses were not only legacy paths, helper code or Google payloads, and the weakest changed ones) were read side by side with their Rust tests. Most misses were legacy paths, helper functions or Google payloads that the ports replace on purpose. Findings:

- `test_google_books.py::test_title_hit_opens_even_when_google_isbn_lookup_misses` was **ported**; its Rust test asserts at least one upstream request where the Python test asserted none (the stored stub's one-time enrichment, CAT). Now **changed**, with the reason on the line.
- MED's picture changes ran through several "ported" lines without being listed: avatars are a 256×256 WebP at `/media/{id}/a` (legacy JPEG at `/api/members/{name}/avatar`), and the cover fetch asks for `-L.jpg` (legacy `-M.jpg`, med.md deviation 8). They are now under "Changes every port shares" in the checklist; the lines keep their status.
- No Rust test was found to check less than its line says otherwise. The search cache port checks only the bound, not which key is evicted; its line already says so (moka evicts approximately).

The review compared assertions; it did not mutation-test the 259 Rust tests.

## For the integrator

- **A new table fails `parity_schema.rs`** until it is added to one of its classes and to the checklist's #37 section. INT's `server/migrations/12*.sql` may add one; if so, classify it in the same merge (PAR owns the file, but a one-line addition to a list during the merge is the expected fix).
- **A new or renamed parity comment, or a renamed Rust test named in the checklist, fails `parity_checklist.rs`.** The message names the line to edit.
- **#48** deletes `test_api_error_copy.py`; nothing in the checklist needs to change for that.
- **A new foreign key to `users`, a new `username` parameter or a new rating/take field fails `parity_visibility.rs`** until it is listed in that test's tables (with its kinds and cases, or the reason it is not personal data) and in the checklist's #46 table. INT's `server/migrations/12*.sql` may add such a column; listing it is a one-line addition during the merge.
- No `blocked_on_frozen` item from Part 1 or Part 2. HAR's open point (a deleted multi-ISBN record withdraws only its first edition, `docs/phase1/notes/har.md` "Open points") is in INT's files now; PAR did not touch it.

## Left

Nothing of #49. The acceptance holds: every Python test is checked off or reasoned (`parity_checklist.rs` keeps it so).
