# Parity checklist (#49)

Every test function of the Python backend (`backend/tests/test_*.py`: 29 files, 292 functions, 358 cases with parametrisation) is listed here with the Rust test that ports it, or the reason it has none. Acceptance (#49): every Python test is checked off or reasoned. All 292 are.

## How this list is built and kept

- **Sources.**
  - The `/// parity: <file>::<test>` doc comments on Rust tests in `server/src` (unit tests) and `server/tests` (`grep -rn "/// parity:" server/src server/tests`).
  - `docs/phase1/parity-notes.md`: the reasoned drops, replacements and moves, one line per Python test.
  - Each package's notes, `docs/phase1/notes/*.md`, for the reasoned changes behind each port.
  - `docs/phase1/spec/google.md` §4 for the KEEP, ADAPT, DROP and INVERT verdict of each `test_google_books.py` test.
- **Statuses.**
  - **ported**: the Rust test checks the same behaviour. The mechanics may differ in the ways listed under "Changes every port shares".
  - **changed**: ported, but the behaviour changed on purpose. The reason follows on the line.
  - **replaced**: the feature is gone and another one takes its place. The Rust test covers the replacement.
  - **dropped**: the behaviour is gone and nothing replaces it. The reason follows. A Rust test named on such a line covers what is left.
- **References.** A Rust test is `` `tests/<file>.rs::<fn>` `` (integration) or `` `src/<path>.rs::<fn>` `` (unit test in that module). Several tests on one line all carry the parity comment, or cover a part of the Python test that the line names.
- **The check.** `server/tests/parity_checklist.rs` (`cargo test --test parity_checklist`) fails when:
  - a Python test function has no line here, or a section's count is wrong;
  - a line is not checked off, has no status, or a ported, changed or replaced line names no Rust test;
  - a `/// parity:` comment sits on a Rust test that the line of its Python test does not name;
  - a Rust test named anywhere in this file does not exist, or names a function without a `#[test]`, `#[sqlx::test]` or `#[tokio::test]` attribute;
  - a parametrised test's line does not give its case count "(N cases)", N is not the product of the lengths of its `parametrize` lists, or a line of a test that is not parametrised gives one;
  - a row of the summary table disagrees with its section: the tests, the cases (the sum of the lines' N, 1 for a line without one) and the statuses; the total row is not the sum of the rows, or the opening paragraph's numbers are not the total row's;
  - a Python test listed in `parity-notes.md` has no line.

  So a new parity comment, a renamed Rust test or a new Python test needs an edit here. Lines for Python tests that no longer exist are allowed: #48 deletes `test_api_error_copy.py`, and the cutover (#52) deletes `backend/`, after which the check is deleted and this file stays as the record.
- **Cross-package tests (PAR).** `server/tests/parity_ports.rs` holds the Python tests that no Wave 2 package ported in full: the handoffs of `docs/phase1/notes/cat.md` "Left for other packages, or PAR", and halves that a package stood in for with SQL because the route belonged to another package (the shelf → pick readers, overlap, suggestions and stats; every emitter → the activity feeds, quotes included; favourites → the book page; the pick and milestone routes → the pick threads). They run end to end through the routes of several packages, against the real `CatalogService` (`TestApp::with_real_lookup`) where a lookup is involved, and wiremock for every upstream. `server/tests/parity_importer.rs` runs the legacy importer `--offline` and `rematch-legacy` against the real lookup. `server/tests/parity_visibility.rs` is the #46 matrix completeness check (below). `server/tests/parity_schema.rs` checks the #37 schema criterion below.

## Summary

| File | Tests | Cases | ported | changed | replaced | dropped |
|---|---:|---:|---:|---:|---:|---:|
| `test_activity.py` | 3 | 3 | 2 | 1 | 0 | 0 |
| `test_api_error_copy.py` | 1 | 1 | 1 | 0 | 0 | 0 |
| `test_auth.py` | 22 | 22 | 19 | 2 | 1 | 0 |
| `test_avatars.py` | 8 | 8 | 5 | 2 | 0 | 1 |
| `test_backup.py` | 2 | 2 | 0 | 0 | 2 | 0 |
| `test_book_details.py` | 8 | 8 | 3 | 5 | 0 | 0 |
| `test_diary.py` | 15 | 15 | 10 | 5 | 0 | 0 |
| `test_favorites.py` | 12 | 12 | 8 | 4 | 0 | 0 |
| `test_google_books.py` | 51 | 78 | 11 | 28 | 0 | 12 |
| `test_import.py` | 8 | 8 | 6 | 2 | 0 | 0 |
| `test_local_reads.py` | 2 | 2 | 1 | 1 | 0 | 0 |
| `test_members.py` | 1 | 1 | 0 | 1 | 0 | 0 |
| `test_migrations.py` | 2 | 2 | 0 | 0 | 2 | 0 |
| `test_notes_and_meeting.py` | 11 | 11 | 9 | 2 | 0 | 0 |
| `test_overlap.py` | 4 | 4 | 4 | 0 | 0 | 0 |
| `test_physical_shelf.py` | 14 | 50 | 9 | 5 | 0 | 0 |
| `test_pick.py` | 8 | 8 | 3 | 5 | 0 | 0 |
| `test_pick_posts.py` | 5 | 5 | 4 | 1 | 0 | 0 |
| `test_push.py` | 10 | 10 | 0 | 0 | 0 | 10 |
| `test_reading_dates.py` | 12 | 12 | 10 | 2 | 0 | 0 |
| `test_search.py` | 35 | 35 | 26 | 9 | 0 | 0 |
| `test_search_history.py` | 3 | 3 | 3 | 0 | 0 | 0 |
| `test_selfhost.py` | 13 | 13 | 1 | 7 | 1 | 4 |
| `test_shelf.py` | 16 | 16 | 15 | 1 | 0 | 0 |
| `test_spa_cache.py` | 1 | 4 | 1 | 0 | 0 | 0 |
| `test_spine_color.py` | 9 | 9 | 7 | 2 | 0 | 0 |
| `test_stats_milestones_quotes.py` | 4 | 4 | 1 | 3 | 0 | 0 |
| `test_vote.py` | 7 | 7 | 7 | 0 | 0 | 0 |
| `test_vote_deadline.py` | 5 | 5 | 4 | 1 | 0 | 0 |
| **Total** | **292** | **358** | **170** | **89** | **6** | **27** |

## Known intentional changes

These changes run through many lines below. Each was decided in the design (work-plan appendix, endpoint-map, conventions) and is tested where named.

| Change | What it means for the ports | Tested in |
|---|---|---|
| Backup → export | The instance backup (`GET /api/backup`, the SQLite CLI) is gone. Each member downloads a ZIP of their own data; the operator backs up Postgres (#51). | `tests/auth_export.rs::the_export_needs_a_session`, `tests/auth_export.rs::a_member_downloads_a_zip_of_their_data` |
| Google removed | No Google Books provider, keys, ids, covers or sort mapping. The provider-agnostic rules of `test_google_books.py` are ported against Open Library, the DNB and the local catalog (google.md §4); the Google-only tests are dropped. Legacy `GB…` keys resolve through `legacy_work_keys`, stored hashed. | `tests/catalog_no_google.rs::no_google_books_code_or_data_in_the_server` |
| Single club → club-scoped | Every club route is under `/api/clubs/{slug}` (picks, votes, posts, milestones, overlap, members, invites, activity, stats). A port registers a club and joins members to it; the legacy "everyone is in the club" is "these users share a club". Non-members get 404 `No such club` on every club route. | `tests/clubs_non_member.rs::non_members_get_no_such_club_on_every_club_route` |
| Push in Phase 4 | `/api/push/*`, the VAPID key, notifications and the meeting reminder are dropped until Phase 4. The events stay; `notify_*` prefs and `club_picks.reminder_sent_at` are kept. | `test_push.py` below (10 dropped) |
| `/work/{id}` merged | `GET/POST /api/books/work/{id}[/refresh]` is gone; `/api/books/works/{work_id}` answers the merged `BookDetailOut` (`members` → `readers`, `club_pick` → `club_picks`), and accepts numeric ids, `OL…W` and legacy keys. | `tests/catalog_detail.rs::work_details_for_unknown_book` |
| Book identity and client metadata | Books are numeric work ids from the catalog. Writes (shelf, pick, nomination, diary, quote) take a `work_id` and never accept client book metadata or cover URLs; an unknown work is 404 `No such book.`. | `tests/parity_ports.rs::a_shelf_add_never_stores_a_client_cover`, `tests/parity_ports.rs::a_pick_never_stores_a_client_cover` |
| Strict JSON types | Bodies are typed: a string where a number belongs, or `1` for a boolean, is 400 with pydantic's wording; query booleans stay lax (`extract::parse_lax_bool`). | `src/extract.rs::json_type_errors_use_pydantic_wording`, `tests/auth_account.rs::notification_prefs_take_strict_booleans`, `tests/club_overlap.rs::include_reading_is_a_lax_boolean` |
| Auth before body | A signed-out request is 401 before its body is validated (FastAPI validated the body first). | `tests/auth_login.rs::auth_comes_before_the_body`, `tests/auth_required.rs::anonymous_callers_get_401_outside_the_public_allow_list` |
| RFC 3339 `Z` | Every timestamp is RFC 3339 in UTC with `Z` (Python wrote naive ISO strings). | `tests/club_picks.rs::an_empty_club_has_no_pick_until_an_admin_sets_one`, `tests/club_activity.rs::the_feed_tells_the_club_story_newest_first` |
| Health shape | `GET /api/health` is `{"status": "ok"}` (legacy `{"ok": true}`), and 503 when the database does not answer. | `tests/health.rs::health_is_ok_when_the_database_answers`, `tests/health.rs::health_is_unavailable_when_the_database_is_down` |
| No "email already in use" | Sign-up and `PUT /api/auth/me/email` answer the same for a free address and for one a verified account holds; that account gets a notice, not a link. | `tests/auth_email.rs::a_taken_address_answers_like_a_free_one`, `tests/auth_register.rs::an_address_of_a_verified_account_gets_a_notice_not_a_link` |
| Tokens in the link fragment | Verification and reset links carry the token after `#token=`, so it never reaches a server log or a `Referer`. | `tests/auth_register.rs::sign_up_sends_a_verification_link_in_the_fragment`, `tests/auth_reset.rs::a_reset_link_sets_the_password_and_signs_everyone_out` |
| Old zone names stored as current | Python's `ZoneInfo` took a tzdb link such as `Asia/Calcutta` (in `BOOKCLUB_TZ`) and kept the name as given. The server accepts the same names, case-sensitively, and stores the current one (`Asia/Kolkata`): at sign-up, in the account's and a club's zone, and for the importer's `--timezone` (conventions §12, `docs/phase1/notes/fup.md` item B). | `src/tz.rs::old_names_resolve_to_the_current_name`, `tests/auth_register.rs::an_old_zone_name_is_stored_as_the_current_one`, `tests/auth_account.rs::an_old_zone_name_is_stored_as_the_current_one`, `tests/clubs_manage.rs::an_old_zone_name_is_stored_as_the_current_one`, `tests/importer_run.rs::an_old_zone_name_is_imported_as_the_current_one` |

### Changes every port shares

These are not repeated on each line:

- Paths are the endpoint-map paths (club-scoped, `/api/posts/{post_id}` for both post kinds, numeric ids).
- Users are made by the harness (`TestApp::user`, verified, with a session) instead of `register` with an invite, unless the port tests sign-up itself. Members join a club with `TestApp::join` or an invite.
- Works come from the catalog fixtures (`TestApp::ol_work`, `TestApp::work`, a stored search hit), not from the client metadata of the first write.
- When a test needs a row whose route belongs to another package, a package's own port often inserts it with SQL. Where that left the cross-package path untested, `tests/parity_ports.rs` runs it end to end (named on the line).
- Upstream calls go to wiremock (`TestApp::upstream`) or `FakeCatalog`; no test leaves the machine.
- Pictures are stored and served by MED (conventions §8): an avatar is a 256×256 WebP at `/media/{public_id}/a` (legacy: a JPEG at `/api/members/{name}/avatar?v=`), covers are WebP `l` and `s` variants under `/media/`, and a cover is fetched from Open Library as `-L.jpg` (legacy `-M.jpg`; one fetch gives both sizes, `docs/phase1/notes/med.md` deviation 8). Ports that compared the legacy JPEG bytes or cover URL compare these instead.

## #37 schema acceptance criterion, as built

#37: "Club-scoped tables carry club_id with FK, personal tables don't." This holds with two reasoned deviations, recorded in the header of `server/migrations/0001_init.sql` and here. Two classifications are this checklist's own and not in that header: `club_invites` (club or instance) and `club_invite_uses` (deviation 2's pattern). #37 itself has no note on any of this yet; `docs/phase1/notes/par.md` asks the integrator for one.

Two tests check the table below:
- `tests/parity_schema.rs::club_scoped_tables_carry_club_id_and_personal_tables_do_not` checks each class against the live schema. A table of the schema that the test does not classify, classifies twice, or classifies although it is gone fails it.
- `tests/parity_schema.rs::the_checklist_classifies_every_table_like_the_schema_test` compares the Tables column below with the test's classes, so a table classified in the test but not here, here but not in the test, or in another class fails it.

| Class | Tables | As built |
|---|---|---|
| Club data | `club_memberships`, `club_removals`, `club_picks`, `next_up_votes`, `milestones`, `next_up_nominations`, `next_up_ballots` | `club_id NOT NULL`, with a foreign key from `club_id` alone to `clubs`, or a composite key with every column NOT NULL into another club table: `milestones` (`club_id`, `pick_id`) → `club_picks`, `next_up_nominations` and `next_up_ballots` (`club_id`, `vote_id`) → `next_up_votes`. The composite keys mean a child row can never point into another club. |
| Club or instance | `club_invites` | A nullable `club_id`, with a foreign key from `club_id` alone to `clubs`. NULL for a sign-up invite an instance admin mints for the closed beta (no club to join). |
| Deviation 1: both scopes | `posts`, `events` | A nullable `club_id`, with a foreign key from `club_id` alone to `clubs`; NULL means personal. A composite key such as `events_pick_fk` does not count: it is MATCH SIMPLE and skipped when `pick_id` is NULL. |
| Deviation 2: inherited scope | `post_reactions`, `club_invite_uses` | No `club_id`. A NOT NULL foreign key to the parent with ON DELETE CASCADE: `post_id` → `posts`, `invite_id` → `club_invites`. |
| Personal data | `shelf_entries`, `physical_shelf_layouts`, `user_favorites`, `search_history`, `quotes`, `import_jobs` | No `club_id`. A NOT NULL foreign key `user_id` → `users`. Other users read it through the visibility policy (`bc_visible_shelf`, `bc_visible_quotes`, conventions §6). |
| Neither | `users`, `sessions`, `email_tokens`, `clubs`, `authors`, `works`, `work_authors`, `editions`, `edition_contributors`, `legacy_work_keys`, `catalog_edit_requests`, `harvest_state`, `media`, `media_removed_hashes`, `reports`, `admin_notify_state` | No `club_id`: accounts, the clubs themselves, the shared catalog and media, and the admin notification job's state. |

- **Deviation 1, both scopes in one table.** The test also checks the constraints that keep the scopes apart:
  - `posts`: a club post has `club_id` and `pick_id` together (`posts_club_needs_pick`). `posts_parent_fk` on `(parent_id, work_id, scope_key)`, with `scope_key = coalesce(club_id, 0)`, ties a reply to its parent's work and scope. Only a personal root may be `only_me` (`posts_visibility_scope`).
  - `events`: the kind decides the scope (`events_scope`: the `shelf_*`, `progress` and `quote_added` kinds are personal). `events_personal_no_pick` keeps a pick out of personal events, since `events_pick_fk` is MATCH SIMPLE and skipped when `club_id` is NULL.
  - Why: a diary entry and a pick post have the same shape, replies, reactions and tombstones, and one feed reads both (work-plan appendix, diary 1); a personal event is stored once and shown in every club of its actor from `joined_at` on (club 5).
- **Deviation 2, inherited scope.** A reaction inherits its scope through its post, so it can never outlive or leave its post. `club_invite_uses`, the redemption log of an invite, follows the same pattern; the schema header names only `post_reactions`.

## #46 visibility matrix, as built

Conventions §6.5: every Wave 2 package covers the personal data kinds it serves across self, co-member, stranger and unknown name, and "#49 checks that the matrix is complete". `tests/parity_visibility.rs` does that in three tests:

- `tests/parity_visibility.rs::every_personal_column_route_and_rating_field_is_in_the_matrix` reads the live schema and the OpenAPI document. Every foreign key to `users` (28 columns), every operation with a `username` parameter (9) and every schema with a rating or take field (11) must be listed, either with the kinds that serve it or with the reason it is not personal data (account, club, invite, catalog or moderation data, or a request body). A new personal table, `?username=` route or rating field fails until it is listed in the test.
- `tests/parity_visibility.rs::every_personal_data_kind_has_its_four_viewer_cases` runs every kind those lists name through the routes, in one world written through the routes (`common::visibility::Viewers`: the owner, a co-member, and as strangers a member of another club, someone in no club and a former co-member). A kind cannot be added without its four cases: the kinds are an enum matched exhaustively, each kind's check matches all four cases, and a kind no list names is dead code that fails clippy.
- `tests/parity_visibility.rs::the_checklist_has_a_row_per_kind` compares the kinds those lists name with the rows of the table below (the Kind column up to its parenthesis), so a new kind also needs its row here.

| Kind | Self | Co-member | Stranger | Unknown name | Package tests |
|---|---|---|---|---|---|
| Shelf (ratings, takes, progress, `dnf_reason`) | all, `only_me` included | `only_me` rating and take redacted | 404 like an unknown name | 404 `No member with that name` | `tests/shelf_visibility.rs::shelf_is_visible_to_the_audience_with_only_me_ratings_redacted` |
| Favourites | own list, book page position | on the shelf; own list and position stay theirs | 404 like an unknown name | 404 | `tests/shelf_visibility.rs::favourites_are_shown_to_the_audience_only` |
| Physical layout | GET | GET; PUT is 404 | GET and PUT 404 like an unknown name | 404 | `tests/shelf_visibility.rs::layout_is_visible_to_the_audience_and_saved_by_its_owner_only` |
| Quotes | all | `audience` only | 200 and empty, like an unknown name | 200 and empty | `tests/shelf_visibility.rs::quotes_are_visible_to_the_audience_without_only_me_ones` |
| Stats | `only_me` counted | `only_me` left out | 404 like an unknown name | 404 | `tests/shelf_visibility.rs::personal_stats_leave_only_me_ratings_and_quotes_out_for_others` |
| Diary (entries, replies, reactions, `author_rating`) | all; own rating | `audience` roots; `author_rating` redacted | nothing; reacting or editing 404 | an unknown post id answers like a hidden one | `tests/posts_visibility.rs` |
| Activity (personal events) | all, with payloads | from `joined_at`; `shelf_finished` rating and `only_me` `quote_added` hidden; not in a club the owner is not in | `?username=` 200 and empty, like an unknown name | 200 and empty | `tests/club_visibility.rs`, `tests/club_activity.rs::a_user_in_two_clubs_sees_separate_feeds` |
| Book page (readers, ratings, quote count) | own rating and take | owner as a reader, `only_me` rating left out of reader, mean and count | no reader, rating or quote of the owner | not applicable: no name to pass | `tests/catalog_visibility.rs` |
| Pick readers | own rating and take | club members only, `only_me` redacted | 404 `No such club`; another club's readers leave the owner out | not applicable | `tests/club_visibility.rs::pick_readers_are_club_members_with_only_me_ratings_redacted` |
| Club shelves (overlap, suggestions, members' reading, club stats) | own data | members only, `only_me` left out | 404 `No such club`; another club's views leave the owner out | not applicable | `tests/shelf_visibility.rs::club_stats_are_for_members_with_only_me_ratings_left_out`, `tests/club_overlap.rs::overlap_covers_club_members_only`, `tests/club_suggestions.rs::suggestions_cover_club_members_only` |
| Search history | own | own only; deleting the owner's entry 404 | the same | an unknown id answers like the owner's | `tests/shelf_search_history.rs::history_is_private_and_delete_is_scoped` |
| Import jobs | own | 404 `No such import` | 404 | an unknown id answers like the owner's | `tests/shelf_import.rs::jobs_are_private` |

## test_activity.py (3)

- [x] `test_feed_requires_auth`: **ported**. `tests/club_activity.rs::feeds_require_a_session` (the club feed and the home feed).
- [x] `test_feed_records_the_club_story_newest_first`: **changed**. `tests/club_activity.rs::the_feed_tells_the_club_story_newest_first`, `tests/parity_ports.rs::the_club_story_is_recorded_by_every_route`. Joining a club records `member_joined`, not registering; the story is told by the club feed and the home feed. CLUB's port inserts the shelf, post and reaction events with SQL; the PAR port records every event through its own route (SHELF, CLUB, POSTS), including the unchanged progress and the repeated ballot that record nothing.
- [x] `test_feed_paginates_and_filters_by_member`: **ported**. `tests/club_activity.rs::the_feed_pages_and_filters_by_member` (both feeds).

## test_api_error_copy.py (1)

- [x] `test_mapped_english_api_errors_still_exist_in_backend`: **ported**. `tests/api_error_copy.rs::mapped_english_api_errors_exist_in_the_server` checks that every string `apiErrors.ts` maps is a string literal in `server/src`. #48 deletes the Python test and empties the `LEGACY_ONLY` allow-list.

## test_auth.py (22)

- [x] `test_health`: **changed**. `tests/health.rs::health_is_ok_when_the_database_answers`. The body is `{"status": "ok"}`, and 503 when the database is down (Health shape).
- [x] `test_register_consumes_invite_and_sets_session`: **changed**. `tests/auth_register.rs::register_consumes_the_invite_and_signs_in`. Sign-up needs an email; `UserOut` gained `email`, `email_verified`, `pending_email`, `needs_email_prompt`, `timezone`, `is_admin` and `clubs`.
- [x] `test_invalid_invite_does_not_create_user`: **ported**. `tests/auth_register.rs::an_invalid_invite_creates_nothing`.
- [x] `test_invite_cannot_be_reused`: **ported**. `tests/auth_register.rs::a_single_use_invite_cannot_be_reused`.
- [x] `test_invite_cannot_be_used_twice_concurrently`: **ported**. `tests/auth_register.rs::a_single_use_invite_admits_one_of_two_concurrent_sign_ups`.
- [x] `test_username_taken`: **ported**. `tests/auth_register.rs::a_taken_username_is_409_before_the_invite_is_checked`.
- [x] `test_bad_username_and_short_password`: **ported**. `tests/auth_register.rs::bad_usernames_and_short_passwords_are_400`.
- [x] `test_me_requires_session`: **ported**. `tests/auth_login.rs::me_requires_a_session`.
- [x] `test_login_and_logout`: **ported**. `tests/auth_login.rs::login_and_logout`.
- [x] `test_login_throttle`: **ported**. `tests/auth_login.rs::ten_failures_block_the_account`.
- [x] `test_me_carries_default_preferences`: **ported**. `tests/auth_account.rs::me_carries_default_preferences`.
- [x] `test_update_theme_alone_leaves_color_mode`: **ported**. `tests/auth_account.rs::updating_the_theme_alone_leaves_color_mode`.
- [x] `test_update_color_mode_alone_leaves_theme`: **ported**. `tests/auth_account.rs::updating_color_mode_alone_leaves_the_theme`.
- [x] `test_update_both_preferences`: **ported**. `tests/auth_account.rs::updating_both_preferences`.
- [x] `test_empty_preferences_payload_is_a_no_op`: **ported**. `tests/auth_account.rs::an_empty_preferences_payload_is_a_no_op`.
- [x] `test_preferences_persist_across_sessions`: **ported**. `tests/auth_account.rs::preferences_persist_across_sessions`.
- [x] `test_invalid_theme_rejected`: **ported**. `tests/auth_account.rs::an_invalid_theme_is_rejected`.
- [x] `test_invalid_color_mode_rejected`: **ported**. `tests/auth_account.rs::an_invalid_color_mode_is_rejected`.
- [x] `test_preferences_require_a_session`: **ported**. `tests/auth_account.rs::preferences_require_a_session`.
- [x] `test_update_locale`: **ported**. `tests/auth_account.rs::updating_the_locale`.
- [x] `test_invalid_locale_rejected`: **ported**. `tests/auth_account.rs::an_invalid_locale_is_rejected`.
- [x] `test_preference_columns_backfill_pre_existing_users`: **replaced**. `tests/importer_run.rs::a_database_from_before_every_column_migration_imports`. There is no in-place schema upgrade any more; the importer (IMP) reads a legacy users table without the preference columns and gives those users the defaults the backfill gave (paper, system, en, every notification on).

## test_avatars.py (8)

- [x] `test_upload_serves_and_deletes_avatar`: **ported**. `tests/media_avatars.rs::upload_serves_and_deletes_avatar`.
- [x] `test_other_members_see_avatar_url`: **ported**. `tests/media_avatars.rs::other_members_see_avatar_url` (the members list is under the club).
- [x] `test_rejects_non_image_and_oversize`: **ported**. `tests/media_avatars.rs::rejects_non_image_and_oversize`.
- [x] `test_rejects_small_file_with_too_many_pixels`: **ported**. `tests/media_avatars.rs::rejects_small_file_with_too_many_pixels`.
- [x] `test_oversize_content_length_is_rejected_before_read`: **ported**. `src/media/upload.rs::content_length_over_the_limit_plus_slack_is_oversize` (the unit half), `tests/media_avatars.rs::oversize_content_length_is_rejected_before_read` (the HTTP half).
- [x] `test_avatar_etag_and_nosniff`: **changed**. `tests/media_avatars.rs::avatar_urls_are_immutable_long_cached_and_nosniff`. The picture URL is immutable (`/media/{public_id}/a`), so it is cached for a year instead of an ETag and 304; a new upload gets a new URL. `nosniff` stays.
- [x] `test_avatar_blob_stays_deferred_when_serializing`: **dropped**. There is no avatar blob in `users` any more (#43): pictures are `media` rows with their bytes in the `MediaStore`, and no user query can load them.
- [x] `test_avatar_requires_session`: **changed**. `tests/media_avatars.rs::avatar_writes_require_a_session_and_the_url_is_public`. The writes need a session; the picture URL is public and unguessable, like every `/media` URL.

## test_backup.py (2)

- [x] `test_backup_requires_auth`: **replaced**. `tests/auth_export.rs::the_export_needs_a_session`. Backup → export: the personal export needs a session.
- [x] `test_backup_returns_sqlite_for_member`: **replaced**. `tests/auth_export.rs::a_member_downloads_a_zip_of_their_data`. A member gets a ZIP of their own data (`data.json` and their pictures, conventions §5.7), not the club's SQLite file.

## test_book_details.py (8)

- [x] `test_work_details_for_unknown_book`: **changed**. `tests/catalog_detail.rs::work_details_for_unknown_book`. `/work/{id}` merged into `/works/{id}`; `members` → `readers`, `club_pick` → `club_picks`; the cover is a same-origin `/media/` URL.
- [x] `test_work_details_persist_on_known_book_and_list_members`: **changed**. `tests/catalog_detail.rs::work_details_persist_on_known_book_and_list_members`. The viewer's own row is the top-level shelf state and `readers` lists the others in the viewer's audience.
- [x] `test_work_details_rejects_bad_ids_and_upstream_failure`: **changed**. `tests/catalog_detail.rs::work_details_rejects_bad_ids_and_upstream_failure`. `/works/not-a-key` is 404 (the dropped `/work/` route answered 400); an upstream failure on the import is 502.
- [x] `test_isbn_lookup`: **changed**. `tests/catalog_isbn.rs::isbn_lookup`. The hit is stored, so the repeat is local; the answer carries `work_id` and `edition_id`, and `on_shelf` is a boolean next to `shelf_id`.
- [x] `test_isbn10_with_x_check_digit_is_accepted`: **ported**. `tests/catalog_isbn.rs::isbn10_with_x_check_digit_is_accepted`.
- [x] `test_work_details_skips_leading_negative_cover`: **ported**. `tests/catalog_detail.rs::work_details_skips_leading_negative_cover`.
- [x] `test_positive_cover_id_rejects_placeholders`: **ported**. `src/catalog/openlibrary.rs::cover_ids_must_be_positive`.
- [x] `test_cancelled_work_details_unblocks_coalesced_waiters`: **changed**. `tests/catalog_detail.rs::cancelled_work_details_unblock_coalesced_waiters`. When the caller running a coalesced fetch is cancelled, a waiter runs the fetch itself instead of failing with 502 (work-plan catalog 16); it never hangs.

## test_diary.py (15)

- [x] `test_requires_auth_and_valid_work_id`: **ported**. `tests/posts_diary.rs::requires_auth_and_a_valid_work_id`.
- [x] `test_diary_on_an_unknown_book_is_empty_not_missing`: **changed**. `tests/posts_diary.rs::a_work_without_entries_has_an_empty_diary`. A catalog work nobody wrote about is an empty diary; an id the catalog does not know is 404 `No such book.` (work-plan diary 13).
- [x] `test_first_entry_creates_the_book_when_given_its_details`: **changed**. `tests/posts_diary.rs::the_first_entry_needs_a_catalog_work`. No client book metadata: an entry needs a catalog work, and a `book` key is ignored.
- [x] `test_entry_snapshots_where_the_author_was`: **ported**. `tests/posts_diary.rs::an_entry_snapshots_where_the_author_was`.
- [x] `test_viewer_position_drives_the_spoiler_shield_fields`: **ported**. `tests/posts_diary.rs::the_viewer_position_drives_the_spoiler_shield_fields`.
- [x] `test_replies_are_one_level_and_stay_on_the_book`: **ported**. `tests/posts_diary.rs::replies_are_one_level_and_stay_on_the_work`.
- [x] `test_only_the_author_edits_or_deletes`: **ported**. `tests/posts_diary.rs::only_the_author_edits_or_deletes`.
- [x] `test_deleting_an_entry_with_replies_leaves_a_tombstone`: **ported**. `tests/posts_diary.rs::deleting_an_entry_with_replies_leaves_a_tombstone`.
- [x] `test_reactions_toggle_and_aggregate`: **ported**. `tests/posts_diary.rs::reactions_toggle_and_aggregate`.
- [x] `test_entry_on_the_current_pick_records_the_pick`: **changed**. `tests/posts_picks.rs::a_club_post_records_its_pick`, `tests/parity_ports.rs::old_pick_threads_survive_a_new_pick_and_an_applied_vote` (end to end: the pick set with CLUB's route). `pick_id` survives only on club posts: a pick post records its pick, a diary entry on the pick's work is personal and has none (work-plan diary 1).
- [x] `test_feed_exposes_viewer_position_for_spoiler_shield`: **ported**. `tests/posts_diary.rs::the_feed_exposes_the_viewer_position_for_the_spoiler_shield`.
- [x] `test_feed_is_newest_first_across_books_and_pages`: **ported**. `tests/posts_diary.rs::the_feed_is_newest_first_across_works_and_pages`.
- [x] `test_legacy_pick_notes_are_imported_once_with_reactions`: **changed**. `tests/importer_posts.rs::legacy_pick_notes_are_imported_once_with_reactions`. Moved to IMP: the boot-time copy of `club_pick_posts` is the importer's job; a mirrored and a never-mirrored note are each imported once, with their reaction.
- [x] `test_club_rating_averages_finished_readers`: **ported**. `tests/posts_diary.rs::the_club_rating_averages_finished_readers` (on CAT's book page, the audience-wide `club_rating`).
- [x] `test_custom_book_has_a_diary_without_open_library`: **changed**. `tests/posts_diary.rs::a_user_added_work_has_a_diary_without_open_library`. The custom book is a user-added work, reached by id and by its legacy `BC…` key.

## test_favorites.py (12)

- [x] `test_replace_order_clear_and_eligibility`: **ported**. `tests/shelf_favorites.rs::replace_order_clear_and_eligibility`, `tests/parity_ports.rs::favourite_positions_show_on_the_book_page`. SHELF ports the favourites half; the PAR port checks `favorite_position` on CAT's book page after the PUT and the clear.
- [x] `test_unknown_and_duplicate_book_ids_are_rejected`: **changed**. `tests/shelf_favorites.rs::unknown_and_duplicate_work_ids_are_rejected`. The string is `That book is not in the catalog` (was `…not in the club yet`); the body takes `work_ids`.
- [x] `test_favorites_in_length_check_is_the_translatable_validator`: **changed**. `tests/shelf_favorites.rs::the_length_check_is_the_translatable_message`. The legacy test inspected pydantic's error list; the port checks the answer: exactly `At most 3 favourites`, alone.
- [x] `test_favourite_does_not_require_own_shelf`: **ported**. `tests/shelf_favorites.rs::favourite_does_not_require_own_shelf`.
- [x] `test_add_helper_appends_and_rejects_a_fourth`: **ported**. `tests/shelf_favorites.rs::add_appends_and_rejects_a_fourth`.
- [x] `test_remove_helper_compacts_positions`: **ported**. `tests/shelf_favorites.rs::remove_compacts_positions`.
- [x] `test_member_shelf_shows_their_favourites_not_yours`: **ported**. `tests/shelf_favorites.rs::member_shelf_shows_their_favourites_not_yours`.
- [x] `test_omitted_book_ids_clears_favourites`: **ported**. `tests/shelf_favorites.rs::omitted_work_ids_clears_favourites`.
- [x] `test_stale_last_slot_insert_maps_integrity_error_to_409`: **changed**. `tests/shelf_favorites.rs::a_full_list_written_elsewhere_still_refuses_a_fourth`. The stale read cannot happen under the user lock; the same setup (a third favourite written behind the API's back) gets the same 409, and a gap in the positions is closed first.
- [x] `test_favorite_relationships_use_delete_cascade`: **changed**. `tests/shelf_favorites.rs::favourites_go_with_their_user`. ORM introspection becomes a `pg_constraint` check: `user_id` cascades, `work_id` is RESTRICT because works are never deleted (merges re-point favourites).
- [x] `test_concurrent_add_for_last_slot_is_409_not_500`: **ported**. `tests/shelf_favorites.rs::concurrent_add_for_last_slot_is_409_not_500`.
- [x] `test_favorites_require_a_session`: **ported**. `tests/shelf_favorites.rs::favorites_require_a_session`.

## test_google_books.py (51)

Verdicts from `docs/phase1/spec/google.md` §4: KEEP → ported, ADAPT and INVERT → changed, DROP → dropped (Google removed, #41). Tests that are part DROP and part KEEP are changed, with the dropped part named.

- [x] `test_google_books_disabled_without_key`: **dropped**. Google Books removed (#41): there is no provider switch.
- [x] `test_google_catalog_ids`: **changed**. `tests/catalog_isbn.rs::isbn10_with_x_check_digit_is_accepted`. The Google id formats are dropped; the ISBN normalisation half (hyphens, the case of `X`) is ported. Legacy key classification is IMP's (`legacy_work_keys`).
- [x] `test_map_volume_prefers_isbn_key`: **dropped**. Google Books removed (#41).
- [x] `test_map_volume_without_isbn_uses_volume_id`: **dropped**. Google Books removed (#41).
- [x] `test_map_volumes_skips_junk_and_duplicates`: **changed**. `src/catalog/openlibrary.rs::search_docs_skip_junk_and_duplicates`. ADAPT: the Open Library mappers skip records without an id or a title and dedupe a page by work.
- [x] `test_volume_details_strips_html`: **dropped**. Google Books removed (#41); Google and VLB descriptions are never stored.
- [x] `test_search_prefers_google_books`: **changed**. `tests/catalog_search.rs::search_maps_and_caches`. The provider is dropped; KEEP: the search cache key ignores case and spacing.
- [x] `test_search_empty_google_does_not_fall_back`: **changed**. `tests/catalog_isbn.rs::a_dnb_miss_falls_through_and_a_final_miss_is_404`. INVERT: a miss in one source falls through to the next.
- [x] `test_empty_google_response_does_not_freeze_retries` (8 cases): **ported**. `tests/catalog_search.rs::empty_responses_do_not_freeze_retries` (search, trending, subjects), `tests/catalog_isbn.rs::empty_isbn_answers_do_not_freeze_retries` (ISBN), `tests/catalog_isbn.rs::a_dnb_miss_falls_through_and_a_final_miss_is_404` (DNB SRU).
- [x] `test_search_google_429_does_not_fall_back`: **changed**. `tests/catalog_search.rs::a_429_cools_open_library_down`. ADAPT: an upstream 429 is 429 `The library is busy…`; the "no fallback" part is INVERT.
- [x] `test_empty_subject_preview_recovers_with_google_topic_search` (4 cases): **changed**. `tests/catalog_detail.rs::a_displayed_hit_opens_during_a_503_outage`. The Google topic fallback is dropped; ADAPT: a displayed browse result opens from the stored hit.
- [x] `test_empty_explicit_search_or_later_page_is_not_broadened` (2 cases): **ported**. `tests/catalog_search.rs::empty_explicit_search_or_later_page_is_not_broadened`.
- [x] `test_subject_fallback_keeps_provider_errors_visible` (4 cases): **changed**. `tests/catalog_search.rs::subject_errors_stay_visible`, `tests/catalog_search.rs::fallback_errors_surface_only_without_hits`. ADAPT: an upstream error surfaces classified (429 or 502), never as an empty page; a 500 is not retried.
- [x] `test_search_google_500_does_not_fall_back`: **changed**. `tests/catalog_search.rs::a_500_without_local_hits_is_a_502`, `tests/catalog_search.rs::local_catalog_hits_survive_open_library_outage`. ADAPT: a 5xx without local hits is 502; local hits are still served.
- [x] `test_browse_and_empty_search_use_google_not_open_library`: **changed**. `tests/catalog_search.rs::empty_search_serves_trending`. The provider is dropped; KEEP: the empty search box serves the trending list.
- [x] `test_isbn_search_uses_google_isbn_field`: **changed**. `tests/catalog_search.rs::isbn_query_uses_the_isbn_lookup`, `tests/catalog_isbn.rs::a_german_isbn_search_goes_through_the_dnb`. ADAPT: an ISBN-shaped query goes to the ISBN lookup (local, DNB for 978-3, Open Library).
- [x] `test_detail_imports_from_google_not_open_library`: **changed**. `tests/catalog_detail.rs::book_detail_resolves_description_subjects_and_authors`. The Google import is dropped; KEEP: the second open of a known book makes no upstream call.
- [x] `test_opening_discovered_book_does_not_call_google` (12 cases): **changed**. `tests/catalog_search.rs::browsing_stores_every_hit`, `tests/catalog_detail.rs::a_displayed_hit_opens_during_a_503_outage`. INVERT: every browsed hit is stored. KEEP: opening a displayed hit works during an outage; its one-time enrichment is attempted and fails, and a 503 trips no cooldown. The "exactly one call on the next search" part is reasoned away by CAT (a 429 during the enrichment trips the cooldown).
- [x] `test_title_hit_opens_even_when_google_isbn_lookup_misses` (2 cases): **changed**. `tests/catalog_detail.rs::a_search_hit_opens_when_the_second_lookup_misses`. KEEP: opening a search hit does not depend on a second lookup; the stored hit opens although its record is gone upstream. The legacy "zero calls" does not hold: the first view of a stored search stub runs its one-time enrichment (one Open Library request, which misses), as in `test_opening_discovered_book_does_not_call_google` (PAR review, Part 2).
- [x] `test_google_transient_503_is_retried_once`: **changed**. `tests/catalog_search.rs::a_503_is_retried_once`. ADAPT: one retry on 502, 503 or 504 after 300 ms (work-plan google 8).
- [x] `test_google_persistent_503_stops_after_one_retry_and_can_recover`: **changed**. `tests/catalog_search.rs::a_503_is_retried_once`. ADAPT: a persistent 503 is 502 after two requests, trips no cooldown, and the next request recovers.
- [x] `test_google_failure_does_not_leave_unobserved_future`: **changed**. `src/catalog/cache.rs::concurrent_calls_share_one_fetch_and_its_outcome`. ADAPT: a failing coalesced fetch hands its error to every waiter and leaves nothing in flight.
- [x] `test_discovered_book_opens_during_existing_google_cooldown` (2 cases): **changed**. `tests/catalog_detail.rs::a_discovered_book_opens_during_a_cooldown`. ADAPT: opening is local, and during a cooldown the enrichment fails at once without a request. The `/work/` case is dropped with the route.
- [x] `test_discovered_book_refresh_still_fetches_fresh_metadata`: **changed**. `tests/catalog_detail.rs::refresh_fetches_fresh_metadata`. ADAPT: a refresh always goes to the source and bypasses the caches; it fetches the work JSON and its search doc, not exactly one call.
- [x] `test_expired_discovery_does_not_supply_book_details`: **dropped**. The discovery cache is replaced by persisting every hit on search (#41), so nothing expires into an upstream call.
- [x] `test_volume_key_detail_and_refresh`: **dropped**. Google Books removed (#41).
- [x] `test_known_google_book_skips_upstream`: **ported**. `tests/parity_ports.rs::a_known_legacy_isbn_book_skips_upstream`. KEEP: a book imported under a legacy `ISBN…` key opens by that key, is shelved, picked and given a diary entry, with no upstream request. (CAT handoff, split across SHELF, CLUB and POSTS; PAR runs all three.)
- [x] `test_refresh_updates_google_isbn_book`: **changed**. `tests/catalog_detail.rs::book_detail_refresh_updates_local_row`. The Google part is dropped and shelving needs an existing work; ADAPT: refreshing a bare record from a CC0 source fills its missing facts.
- [x] `test_isbn_lookup_prefers_google`: **changed**. `tests/catalog_isbn.rs::an_english_isbn_asks_open_library_only`. The provider is dropped; KEEP: an ISBN lookup returns title, authors, year, pages and cover.
- [x] `test_club_custom_book_still_skips_catalogs`: **ported**. `tests/catalog_custom.rs::custom_books_skip_the_catalogs` (the refresh refusal is `This book was added by a reader, not Open Library.`).
- [x] `test_cover_url_from_image_links_https_and_zoom`: **changed**. `tests/catalog_search.rs::empty_search_serves_trending`. The Google URL rules are dropped; KEEP: a cover is never an upstream URL, it is served from `/media/`.
- [x] `test_isbn_cover_url_from_work_key`: **dropped**. There are no client-facing Open Library cover URLs any more (#43); covers are `/media/…`.
- [x] `test_google_cover_wins_over_open_library_isbn_cdn`: **dropped**. Google Books removed (#41).
- [x] `test_isbn_cdn_skipped_when_google_key_is_set`: **dropped**. Google Books removed (#41).
- [x] `test_later_google_page_does_not_mix_open_library`: **ported**. `tests/catalog_search.rs::an_empty_later_page_ends_pagination`.
- [x] `test_title_and_popular_sorts_stay_on_google`: **dropped**. The Google sort mapping is removed (#41). Local sort semantics: `sort=title` also orders local hits (CAT deviation 14).
- [x] `test_isbn_refresh_keeps_description_when_google_misses`: **ported**. `tests/catalog_detail.rs::a_refresh_miss_never_wipes_the_record`.
- [x] `test_isbn_refresh_without_key_keeps_description`: **changed**. `tests/catalog_detail.rs::a_refresh_without_a_description_keeps_it`. ADAPT: a source record without a description does not wipe the stored one; its other values still update.
- [x] `test_isbn_x_and_X_are_the_same_row`: **ported**. `tests/catalog_isbn.rs::isbn10_with_x_check_digit_is_accepted` (the lookup), `tests/catalog_detail.rs::legacy_keys_resolve_through_the_key_map` (the legacy key).
- [x] `test_isbn_lookup_miss_does_not_use_open_library`: **changed**. `tests/catalog_isbn.rs::a_dnb_miss_falls_through_and_a_final_miss_is_404`. INVERT: a miss in the DNB falls through to Open Library; only a final miss is 404 `No book found for that ISBN`.
- [x] `test_shelf_add_persists_google_cover_url`: **dropped**. Client cover URLs are no longer accepted (#43) and Google covers are dropped (#41); the inverse rule, that a client-sent cover is never stored, is `test_pick_does_not_persist_ol_cdn_cover_url` and `test_shelf_rejects_non_google_cover_url` below.
- [x] `test_volume_key_without_google_key`: **dropped**. Google Books removed (#41); imported `GB…` keys resolve through `legacy_work_keys` (`tests/catalog_detail.rs::legacy_keys_resolve_through_the_key_map`).
- [x] `test_volume_refresh_429_is_classified`: **changed**. `tests/catalog_detail.rs::a_429_during_refresh_is_busy`. ADAPT: an upstream 429 during a refresh is 429 `The library is busy…`.
- [x] `test_goodreads_import_prefers_google`: **changed**. `tests/parity_ports.rs::the_goodreads_fixture_imports_through_the_real_lookup`, `tests/shelf_import.rs::import_matches_isbn_and_title`. ADAPT: the same fixture CSV imports through the real lookup chain (Open Library by ISBN with the recorded answer, by title and author, a definite miss); a second export shows a 978-3 ISBN going to the DNB and a known ISBN resolving locally. SHELF's test runs the job with `FakeCatalog`.
- [x] `test_pick_does_not_persist_ol_cdn_cover_url`: **ported**. `tests/parity_ports.rs::a_pick_never_stores_a_client_cover`. KEEP (generalised): `cover_id` and `cover_url` in the pick body are ignored; the pick shows the work's own cover or none.
- [x] `test_shelf_rejects_non_google_cover_url`: **ported**. `tests/parity_ports.rs::a_shelf_add_never_stores_a_client_cover`. KEEP (security): a tracking-pixel URL in a shelf add is never stored, served or fetched.
- [x] `test_shelf_isbn_x_and_X_are_the_same_row`: **ported**. `tests/parity_ports.rs::isbn_x_and_x_are_one_shelf_entry`. KEEP: both spellings reach one edition (through the ISBN lookup and the legacy key map), and the second shelf add is 409 `Already on your shelf`.
- [x] `test_google_403_access_not_configured_is_502`: **changed**. `tests/catalog_search.rs::a_403_is_a_502_without_cooldown`. ADAPT: only a 429 trips the cooldown; a 403 is 502.
- [x] `test_google_403_quota_cools_down`: **changed**. `tests/catalog_search.rs::a_429_cools_open_library_down`. ADAPT: a rate-limit answer (Open Library's 429) cools the host down, and the next search fails fast without a request.
- [x] `test_goodreads_import_while_cooling_down`: **changed**. `tests/parity_ports.rs::a_goodreads_import_during_a_cooldown_skips_without_asking`, `tests/shelf_import.rs::a_long_cooldown_skips_at_once`, `tests/shelf_import.rs::throttled_lookups_skip_with_catalog_unavailable`. ADAPT (work-plan google 9): the import is accepted (202, not the legacy 429); each throttled row is skipped with `catalog_unavailable`, the job finishes, and Open Library gets no request.
- [x] `test_later_empty_google_page_has_more_false_when_total_stays_high`: **ported**. `tests/catalog_search.rs::an_empty_later_page_ends_pagination`.

## test_import.py (8)

- [x] `test_clean_isbn_unwraps_goodreads_formula`: **ported**. `tests/shelf_import.rs::clean_isbn_unwraps_goodreads_formula`.
- [x] `test_parse_fixture_maps_shelves_and_skips`: **changed**. `tests/shelf_import.rs::parse_fixture_maps_shelves_and_skips`. A row without an ISBN has `isbn13: None` (was `""`), and `read` rows carry `My Rating` and `Date Read`, which are imported now.
- [x] `test_import_requires_auth`: **ported**. `tests/shelf_import.rs::import_requires_auth`.
- [x] `test_import_matches_isbn_and_title`: **changed**. `tests/shelf_import.rs::import_matches_isbn_and_title`, `tests/parity_ports.rs::the_goodreads_fixture_imports_through_the_real_lookup`. The import is a background job: 202, then the job's final state; skips carry `code`. The PAR port runs it against the real lookup.
- [x] `test_import_skips_books_already_on_shelf`: **ported**. `tests/shelf_import.rs::import_skips_books_already_on_shelf`.
- [x] `test_import_stays_on_signed_in_shelf`: **ported**. `tests/shelf_import.rs::import_stays_on_signed_in_shelf`.
- [x] `test_import_rejects_non_goodreads_csv`: **ported**. `tests/shelf_import.rs::import_rejects_non_goodreads_csv`.
- [x] `test_import_rejects_oversized_file`: **ported**. `tests/shelf_import.rs::import_rejects_oversized_file` (also a body over the route's 2 MiB limit).

## test_local_reads.py (2)

- [x] `test_known_book_reads_never_touch_open_library`: **ported**. `tests/catalog_detail.rs::known_book_reads_never_touch_open_library`, `tests/parity_ports.rs::known_book_reads_never_touch_open_library`. CAT covers the catalog reads with SQL-made rows; the PAR port writes the shelf entry, pick, diary entry, pick post, quote and nomination through their routes and reads every legacy path (club-scoped) plus the club and home variants, against the real lookup, with no upstream request. `/api/books/work/OL1W` is merged into `/works/`.
- [x] `test_shelving_persists_search_metadata_for_later_local_reads`: **changed**. `tests/catalog_detail.rs::search_metadata_serves_later_local_reads`, `tests/parity_ports.rs::shelving_persists_search_metadata_for_later_local_reads`. The search stores the metadata, not the shelf add (which takes a work id only); the shelf add's `BookOut` and the book page serve it. The first detail view of a search stub tries its one-time enrichment, so with Open Library down the stored hit is served.

## test_members.py (1)

- [x] `test_members_hide_self_and_preview_reading`: **changed**. `tests/clubs_members.rs::members_list_includes_the_caller_and_previews_reading`. The club roster includes the caller (`you: true`) for the role and transfer UI; the reading preview is kept.

## test_migrations.py (2)

- [x] `test_legacy_database_is_upgraded_in_place`: **replaced**. `tests/importer_run.rs::migrated_databases_import_with_the_defaults`, `tests/importer_run.rs::a_database_from_before_every_column_migration_imports`. No in-place SQLite upgrade: the importer (#38) reads the migrated shape and the never-booted legacy schema with the column defaults.
- [x] `test_fresh_database_matches_migration_list`: **replaced**. `tests/importer_run.rs::the_fresh_fixture_imports_with_matching_counts`. There is no hand-rolled column list (sqlx migrations, `cargo sqlx prepare --check`); the importer reads a fresh-shape database with every `COLUMN_MIGRATIONS` column.

## test_notes_and_meeting.py (11)

- [x] `test_post_carries_spoiler_flag_and_viewer_progress`: **ported**. `tests/posts_picks.rs::a_post_carries_its_spoiler_flag_and_the_viewer_progress`.
- [x] `test_spoiler_flag_must_be_a_percentage`: **ported**. `tests/posts_picks.rs::the_spoiler_flag_must_be_a_percentage`.
- [x] `test_author_can_edit_and_delete_own_note`: **changed**. `tests/posts_picks.rs::the_author_edits_and_deletes_their_post`. The `/api/pick/posts/{post_id}` routes are unified with the diary's `/api/posts/{post_id}`, with the diary's strings.
- [x] `test_other_members_cannot_edit_or_delete`: **ported**. `tests/posts_picks.rs::other_members_cannot_edit_or_delete`.
- [x] `test_reactions_toggle_and_aggregate`: **ported**. `tests/posts_picks.rs::reactions_on_club_posts_toggle_and_record_events`.
- [x] `test_deleting_a_note_removes_its_reactions`: **ported**. `tests/posts_picks.rs::deleting_a_post_removes_its_reactions`.
- [x] `test_change_password`: **ported**. `tests/auth_account.rs::change_password`.
- [x] `test_change_password_requires_auth`: **ported**. `tests/auth_account.rs::change_password_requires_auth`.
- [x] `test_notification_prefs_default_on_and_patch`: **ported**. `tests/auth_account.rs::notification_prefs_default_on_and_patch`.
- [x] `test_meeting_ics_download`: **changed**. `tests/club_picks.rs::the_meeting_downloads_as_an_ics_file`. The entry names the club (`PRODID`, `SUMMARY`) and links the book's and the club's pages instead of openlibrary.org.
- [x] `test_ics_folds_long_lines_without_breaking_utf8`: **ported**. `src/picks/ics.rs::folds_long_lines_without_breaking_utf8`.

## test_overlap.py (4)

- [x] `test_overlap_requires_auth`: **ported**. `tests/club_overlap.rs::overlap_requires_a_session`.
- [x] `test_two_want_same_book_shows_one_does_not`: **ported**. `tests/club_overlap.rs::a_book_two_members_want_is_shown`, `tests/parity_ports.rs::overlap_follows_the_shelf_routes` (end to end: the shelf entries from SHELF's routes, the member joined by invite).
- [x] `test_finished_and_solo_want_are_excluded`: **ported**. `tests/club_overlap.rs::finished_books_and_solo_wants_are_left_out`, `tests/parity_ports.rs::overlap_follows_the_shelf_routes` (end to end).
- [x] `test_include_reading_counts_want_plus_reading`: **ported**. `tests/club_overlap.rs::include_reading_counts_reading_too`, `tests/parity_ports.rs::overlap_follows_the_shelf_routes` (end to end).

## test_physical_shelf.py (14)

- [x] `test_layout_requires_authentication`: **ported**. `tests/shelf_layout.rs::layout_requires_authentication`.
- [x] `test_layout_persists_on_reload_and_restart_without_changing_reading_data`: **changed**. `tests/shelf_layout.rs::layout_persists_on_reload_and_restart_without_changing_reading_data`. "Restart" is a second `TestApp` on the same database, with a login through the API.
- [x] `test_members_can_view_a_layout_but_only_its_owner_can_change_it`: **ported**. `tests/shelf_layout.rs::members_can_view_a_layout_but_only_its_owner_can_change_it`.
- [x] `test_status_changes_do_not_move_books_and_deleted_entries_are_pruned`: **ported**. `tests/shelf_layout.rs::status_changes_do_not_move_books_and_deleted_entries_are_pruned`.
- [x] `test_deleted_spine_does_not_leak_to_a_book_with_a_reused_id`: **changed**. `tests/shelf_layout.rs::deleted_spine_does_not_leak_to_a_book_with_a_reused_id`. Postgres never reuses an id: the test reads the stored row after the DELETE, then inserts an entry with the old id by hand.
- [x] `test_get_prunes_entries_removed_outside_the_shelf_endpoint`: **ported**. `tests/shelf_layout.rs::get_prunes_entries_removed_outside_the_shelf_endpoint`.
- [x] `test_invalid_layout_does_not_replace_previous_layout` (35 cases): **changed**. `tests/shelf_layout.rs::invalid_layout_does_not_replace_previous_layout`. All 35 cases; the cases that hard-coded entry id 1 use the caller's real entry id, so each 400 comes from the rule the case breaks.
- [x] `test_legacy_stored_layout_still_loads_and_prunes_missing_books`: **ported**. `tests/shelf_layout.rs::legacy_stored_layout_still_loads_and_prunes_missing_books`.
- [x] `test_decorations_can_be_saved_without_books_and_removed`: **ported**. `tests/shelf_layout.rs::decorations_can_be_saved_without_books_and_removed`.
- [x] `test_mixed_layout_limits_books_and_decorations_separately`: **changed**. `src/shelf/layout.rs::books_and_decorations_are_limited_separately` (the schema half), `tests/shelf_layout.rs::mixed_layout_limits_books_and_decorations_separately` (the API half: the 2100 limit, and 2000 books plus 100 decorations pass the schema).
- [x] `test_shelf_book_exposes_known_page_count`: **ported**. `tests/shelf_layout.rs::shelf_book_exposes_known_page_count`.
- [x] `test_spines_can_follow_the_book_and_choose_how_it_stands`: **ported**. `tests/shelf_layout.rs::spines_can_follow_the_book_and_choose_how_it_stands`.
- [x] `test_new_layout_options_are_validated` (3 cases): **changed**. `tests/shelf_layout.rs::new_layout_options_are_validated`. All 3 cases, with the caller's real entry id on the row.
- [x] `test_new_decorations_are_accepted`: **ported**. `tests/shelf_layout.rs::new_decorations_are_accepted`.

## test_pick.py (8)

- [x] `test_pick_requires_auth`: **ported**. `tests/club_picks.rs::pick_routes_require_a_session` (also PATCH, the history and the `.ics`).
- [x] `test_empty_pick_then_set_and_read`: **changed**. `tests/club_picks.rs::an_empty_club_has_no_pick_until_an_admin_sets_one`. `/api/config` has no `timezone` any more; the pick answer carries the club's.
- [x] `test_same_book_updates_meeting_without_history`: **ported**. `tests/club_picks.rs::the_same_book_updates_the_meeting_without_history`.
- [x] `test_replacing_pick_archives_previous_with_meeting`: **ported**. `tests/club_picks.rs::replacing_the_pick_archives_the_previous_one_with_its_meeting`.
- [x] `test_reading_vs_finished_and_clear`: **changed**. `tests/club_picks.rs::readers_split_into_reading_and_finished_and_an_admin_clears_the_pick`, `tests/parity_ports.rs::the_pick_readers_come_from_the_shelf_routes` (end to end: the shelf entries from SHELF's routes, the member joined by invite). Clearing the pick is admin-only now; a member's DELETE is 403.
- [x] `test_meeting_uses_club_timezone`: **changed**. `tests/club_picks.rs::the_meeting_uses_the_club_time_zone`. The zone is the club's, not `BOOKCLUB_TZ`.
- [x] `test_invalid_timezone_falls_back_to_utc`: **changed**. `tests/club_picks.rs::an_invalid_zone_is_refused_and_a_corrupt_one_reads_as_utc`. An invalid zone is refused when the club is updated (400 `That is not a valid time zone`); a corrupt stored value still reads as UTC.
- [x] `test_invalid_pick_payload`: **changed**. `tests/club_picks.rs::invalid_pick_payloads_are_refused`. The body names a catalog work: the blank-title and key checks are gone, an unknown work is 404 `No such book.`, a missing one 400 `Field required`.

## test_pick_posts.py (5)

- [x] `test_posts_require_auth_and_a_pick`: **ported**. `tests/posts_picks.rs::posts_require_auth_and_a_pick`.
- [x] `test_members_can_post_and_read_oldest_first`: **ported**. `tests/posts_picks.rs::members_post_and_read_oldest_first`.
- [x] `test_old_thread_stays_when_pick_changes`: **ported**. `tests/posts_picks.rs::the_old_thread_stays_when_the_pick_changes`, `tests/parity_ports.rs::old_pick_threads_survive_a_new_pick_and_an_applied_vote` (end to end: the picks set with CLUB's routes).
- [x] `test_post_time_uses_club_timezone`: **changed**. `tests/posts_picks.rs::post_times_use_the_club_timezone`. `BOOKCLUB_TZ` is the club's zone; a club post's label is in its club's zone wherever it appears (POSTS deviation 4).
- [x] `test_invalid_post_body`: **ported**. `tests/posts_picks.rs::invalid_post_bodies`.

## test_push.py (10)

Web Push is dropped until Phase 4 (endpoint-map §12, work-plan appendix club 15). The events stay.

- [x] `test_vapid_key_is_generated_and_stable`: **dropped**. No Web Push in Phase 1, so no VAPID key (Phase 4).
- [x] `test_subscribe_list_unsubscribe`: **dropped**. The `/api/push/*` routes are dropped (Phase 4).
- [x] `test_new_pick_notifies_other_members_who_opted_in`: **dropped**. No push on a new pick; `pick_set` is recorded (Phase 4).
- [x] `test_new_note_notifies_except_author_and_respects_pref`: **dropped**. No push on a post; `note_posted` is recorded (Phase 4).
- [x] `test_dead_subscriptions_are_pruned`: **dropped**. No push subscriptions (Phase 4).
- [x] `test_push_copy_catalog_covers_every_locale`: **dropped**. The push copy (`backend/app/i18n.py`) is not ported (Phase 4).
- [x] `test_push_copy_follows_recipient_locale`: **dropped**. The push copy is not ported (Phase 4).
- [x] `test_meeting_reminder_sent_once_inside_24h`: **dropped**. `GET pick` has no side effects; the reminder becomes a Phase 4 job (`club_picks.reminder_sent_at` is kept).
- [x] `test_meeting_reminder_not_sent_far_ahead_or_when_opted_out`: **dropped**. As above, the reminder is a Phase 4 job.
- [x] `test_vote_close_notifies_new_pick`: **dropped**. Apply and the auto-close record `pick_set` and `vote_closed` but send no push (Phase 4).

## test_reading_dates.py (12)

- [x] `test_want_to_read_has_no_dates`: **ported**. `tests/shelf_reading_dates.rs::want_to_read_has_no_dates`.
- [x] `test_reading_sets_started_and_finish_sets_finished`: **ported**. `tests/shelf_reading_dates.rs::reading_sets_started_and_finish_sets_finished`.
- [x] `test_added_directly_as_finished_stamps_both`: **ported**. `tests/shelf_reading_dates.rs::added_directly_as_finished_stamps_both`.
- [x] `test_reorder_within_finished_keeps_finished_at`: **ported**. `tests/shelf_reading_dates.rs::reorder_within_finished_keeps_finished_at`.
- [x] `test_leaving_finished_clears_finished_at_but_keeps_start`: **ported**. `tests/shelf_reading_dates.rs::leaving_finished_clears_finished_at_but_keeps_start`.
- [x] `test_back_to_want_to_read_resets_dates`: **ported**. `tests/shelf_reading_dates.rs::back_to_want_to_read_resets_dates`.
- [x] `test_dnf_has_start_but_no_finish`: **ported**. `tests/shelf_reading_dates.rs::dnf_has_start_but_no_finish`.
- [x] `test_edit_and_clear_finish_date_survive_reordering`: **changed**. `tests/shelf_reading_dates.rs::edit_and_clear_finish_date_survive_reordering`. The instance zone `BOOKCLUB_TZ` is now the owner's `users.timezone`.
- [x] `test_invalid_finish_date_does_not_modify_entry`: **ported**. `tests/shelf_reading_dates.rs::invalid_finish_date_does_not_modify_entry`.
- [x] `test_reread_replaces_completion_without_duplicate`: **ported**. `tests/shelf_reading_dates.rs::reread_replaces_completion_without_duplicate`.
- [x] `test_move_between_months_saves_order_and_date_together`: **changed**. `tests/shelf_reading_dates.rs::move_between_months_saves_order_and_date_together`. As above, the owner's zone.
- [x] `test_cannot_change_another_members_completion_month`: **ported**. `tests/shelf_reading_dates.rs::cannot_change_another_members_completion_month`.

## test_search.py (35)

- [x] `test_empty_search_serves_trending`: **changed**. `tests/catalog_search.rs::empty_search_serves_trending`. `cover_id` is gone: "no cover" is `cover_url: null`, a cover is same-origin.
- [x] `test_open_library_never_receives_a_too_short_query`: **ported**. `tests/catalog_search.rs::open_library_never_receives_a_too_short_query`.
- [x] `test_search_maps_and_caches`: **changed**. `tests/catalog_search.rs::search_maps_and_caches`. `on_shelf` is a boolean next to `shelf_id`; `club_pick` is `club_picks`, the caller's clubs only.
- [x] `test_search_forwards_page_subject_sort`: **ported**. `tests/catalog_search.rs::search_forwards_page_subject_sort`.
- [x] `test_search_cache_keyed_by_page`: **ported**. `tests/catalog_search.rs::search_cache_keyed_by_page`.
- [x] `test_trending_annotates_and_caches`: **ported**. `tests/catalog_search.rs::trending_annotates_and_caches`.
- [x] `test_subject_maps_its_own_payload_shape`: **ported**. `tests/catalog_search.rs::subject_maps_its_own_payload_shape`.
- [x] `test_subject_rejects_invalid_key`: **ported**. `tests/catalog_search.rs::subject_rejects_invalid_key`.
- [x] `test_first_import_skips_author_api_when_search_has_names`: **ported**. `tests/catalog_detail.rs::first_import_skips_author_api_when_search_has_names`.
- [x] `test_book_detail_resolves_description_subjects_and_authors`: **changed**. `tests/catalog_detail.rs::book_detail_resolves_description_subjects_and_authors`. `cover_id` is gone; the cover behind `cover_public_id` is Open Library cover 555.
- [x] `test_book_detail_accepts_dict_description`: **ported**. `tests/catalog_detail.rs::book_detail_accepts_dict_description`.
- [x] `test_book_detail_flattens_markdown_in_description`: **ported**. `tests/catalog_detail.rs::book_detail_flattens_markdown_in_description`.
- [x] `test_book_detail_includes_own_shelf_state_and_other_readers`: **changed**. `tests/catalog_detail.rs::book_detail_includes_own_shelf_state_and_other_readers`. The readers are the viewer's audience (a shared club, #46); a stranger's entry is left out.
- [x] `test_book_detail_rejects_malformed_work_id`: **ported**. `tests/catalog_detail.rs::book_detail_rejects_malformed_work_id`.
- [x] `test_book_detail_propagates_not_found`: **ported**. `tests/catalog_detail.rs::book_detail_propagates_not_found`.
- [x] `test_upstream_failure_is_a_502`: **ported**. `tests/catalog_search.rs::upstream_failure_is_a_502`.
- [x] `test_detail_survives_author_lookup_failure`: **ported**. `tests/catalog_detail.rs::detail_survives_author_lookup_failure`.
- [x] `test_book_detail_refresh_updates_local_row`: **changed**. `tests/catalog_detail.rs::book_detail_refresh_updates_local_row`. The title-only work is a known Open Library work (shelving needs an existing work), and `/work/OL1W` is merged into `/works/`.
- [x] `test_refresh_bypasses_author_cache`: **ported**. `tests/catalog_detail.rs::refresh_bypasses_author_cache`.
- [x] `test_search_cache_is_bounded`: **ported**. `src/catalog/cache.rs::the_cache_is_bounded` (bounded by entry count; moka evicts approximately, not in strict LRU order).
- [x] `test_identical_inflight_searches_share_one_request`: **ported**. `src/catalog/cache.rs::concurrent_calls_share_one_fetch_and_its_outcome`, `tests/catalog_search.rs::identical_inflight_searches_share_one_request`.
- [x] `test_search_cache_ttl_is_at_least_half_a_day`: **ported**. `src/catalog/cache.rs::the_ttl_is_at_least_half_a_day`.
- [x] `test_open_library_connect_timeout_is_short`: **changed**. `tests/catalog_search.rs::request_policy_is_short`. The 8 s connect and 15 s read timeouts are `outbound.rs` constants (W1); the test checks the request policy CAT uses (15 s per attempt, 2 s throttle wait).
- [x] `test_cancelled_fetch_unblocks_coalesced_waiters`: **changed**. `src/catalog/cache.rs::a_dropped_owner_never_leaves_a_waiter_hanging`. A waiter runs the fetch itself when its owner is dropped, instead of failing with 502 (work-plan catalog 16).
- [x] `test_short_query_is_a_400_and_never_hits_open_library`: **ported**. `tests/catalog_search.rs::short_query_is_a_400_and_never_hits_open_library`.
- [x] `test_upstream_422_is_a_helpful_400`: **ported**. `tests/catalog_search.rs::upstream_422_is_a_helpful_400`.
- [x] `test_upstream_429_is_a_429_with_retry_hint`: **ported**. `tests/catalog_search.rs::upstream_429_is_a_429_with_retry_hint`.
- [x] `test_isbn_query_uses_isbn_field`: **changed**. `tests/catalog_search.rs::isbn_query_uses_the_isbn_lookup`. An ISBN query goes through the ISBN lookup chain, whose Open Library step is the `isbn:` search.
- [x] `test_empty_search_pages_are_not_cached`: **ported**. `tests/catalog_search.rs::empty_search_pages_are_not_cached`.
- [x] `test_title_fallback_when_raw_query_misses`: **ported**. `tests/catalog_search.rs::title_fallback_when_raw_query_misses`.
- [x] `test_search_candidates_keep_raw_query_before_by_split`: **ported**. `src/catalog/search.rs::candidates_keep_the_raw_query_first`.
- [x] `test_title_containing_by_uses_raw_query_first`: **ported**. `tests/catalog_search.rs::title_containing_by_uses_raw_query_first` (the fake ids are valid Open Library ids).
- [x] `test_local_catalog_is_searched_without_open_library_for_custom_books`: **changed**. `tests/catalog_custom.rs::local_catalog_is_searched_without_open_library_for_custom_books`. A user-added work has a numeric id and no `BC…` key; the catalog's own book comes first.
- [x] `test_local_catalog_hits_survive_open_library_outage`: **ported**. `tests/catalog_search.rs::local_catalog_hits_survive_open_library_outage`.
- [x] `test_custom_book_requires_a_title`: **ported**. `tests/catalog_custom.rs::custom_book_requires_a_title`.

## test_search_history.py (3)

- [x] `test_history_requires_login`: **ported**. `tests/shelf_search_history.rs::history_requires_login`.
- [x] `test_history_deduplicates_orders_and_caps`: **ported**. `tests/shelf_search_history.rs::history_deduplicates_orders_and_caps`.
- [x] `test_history_is_private_and_delete_is_scoped`: **ported**. `tests/shelf_search_history.rs::history_is_private_and_delete_is_scoped`.

## test_selfhost.py (13)

Self-hosting is removed: one hosted service, configured from the environment only.

- [x] `test_placeholder_detection`: **dropped**. There are no placeholder secrets or bootstrap invites to detect. The unsafe production settings that remain are refused at startup: `tests/auth_config.rs::production_startup_refuses_unsafe_settings`.
- [x] `test_production_generates_and_reuses_secrets`: **dropped**. Sessions and invites are database rows, so no `.secret_key` or `.bootstrap_invite` is generated or reused (marked on `tests/auth_config.rs::production_startup_refuses_unsafe_settings`).
- [x] `test_production_app_registers_and_hides_docs`: **changed**. `tests/auth_config.rs::a_production_app_registers` (health, sign-up, the document), `tests/openapi_contract.rs::no_docs_ui_is_served_and_the_document_is_public` (no docs UI), `tests/platform.rs::production_cors_only_allows_configured_origins` (CORS). Health shape; `GET /api/openapi.json` is public in every mode and no docs UI is served in any; sign-up needs an email and an invite from the database.
- [x] `test_session_cookie_not_secure_on_plain_http`: **changed**. `tests/auth_sessions.rs::dev_mode_cookies_are_not_secure`. `Secure` comes from `COOKIE_SECURE` (default: on outside DEV_MODE), not from the request scheme.
- [x] `test_session_cookie_secure_on_https_and_forwarded_proto`: **changed**. `tests/auth_sessions.rs::production_cookies_are_secure`. As above: production cookies are `Secure`, `HttpOnly` and `SameSite=Lax` whatever the scheme.
- [x] `test_backup_copies_sqlite`: **replaced**. `tests/auth_export.rs::a_member_downloads_a_zip_of_their_data`. Backup → export; the operator backs up Postgres (#51).
- [x] `test_production_refuses_demo_secret_and_invite`: **changed**. `tests/auth_config.rs::production_startup_refuses_unsafe_settings`. No demo secret or invite exists; startup refuses a production configuration without `TRUSTED_PROXIES` or with the log mail transport.
- [x] `test_public_config_and_cors_origins`: **changed**. `tests/auth_config.rs::public_url_is_an_origin`. `PUBLIC_URL` is reduced to its origin, goes into the outbound User-Agent, and must be an http(s) URL with a host; the SPA is same-origin, so it needs no CORS entry.
- [x] `test_branded_app_config_cors_and_user_agent`: **changed**. `tests/auth_config.rs::config_follows_the_service_settings`. The name comes from `SERVICE_NAME`, the accent colours are fixed, the manifest is a static file, and the public origin needs no CORS. The `/api/docs` assertion is `tests/openapi_contract.rs::no_docs_ui_is_served_and_the_document_is_public`.
- [x] `test_dotenv_only_production_boot`: **dropped**. Configuration comes from the environment only, without a `.env` file. The production boot from the documented variables is `tests/auth_config.rs::a_production_app_registers`, and its `/api/docs` assertion `tests/openapi_contract.rs::no_docs_ui_is_served_and_the_document_is_public`.
- [x] `test_trusted_proxy_denies_forwarded_proto`: **changed**. `tests/auth_sessions.rs::forwarded_proto_does_not_make_cookies_secure`. `X-Forwarded-Proto` never decides `Secure`, not even from a trusted proxy.
- [x] `test_invalid_theme_warns_and_falls_back`: **dropped**. The accent colours are fixed (the legacy defaults), so there is no theme setting to fall back from (marked on `tests/auth_config.rs::config_follows_the_service_settings`).
- [x] `test_debug_cors_keeps_vite`: **ported**. `tests/auth_config.rs::dev_mode_cors_keeps_vite`, `tests/platform.rs::dev_mode_cors_allows_the_vite_origin`.

## test_shelf.py (16)

- [x] `test_shelving_without_authors_does_not_clear_existing_authors`: **changed**. `tests/shelf_entries.rs::shelving_never_changes_the_catalog_row`. Holds by construction: a shelf add carries no book metadata, so it can never edit the catalog row.
- [x] `test_add_move_remove_and_unique`: **ported**. `tests/shelf_entries.rs::add_move_remove_and_unique`.
- [x] `test_cannot_edit_someone_elses_row`: **ported**. `tests/shelf_entries.rs::cannot_edit_someone_elses_row`.
- [x] `test_reorder_within_and_across_columns`: **ported**. `tests/shelf_entries.rs::reorder_within_and_across_columns`.
- [x] `test_unknown_member_shelf_is_404`: **ported**. `tests/shelf_entries.rs::unknown_member_shelf_is_404`.
- [x] `test_finish_rating_and_take_show_on_friend_shelf`: **ported**. `tests/shelf_entries.rs::finish_rating_and_take_show_on_friend_shelf`.
- [x] `test_dnf_reason_shows_on_friend_shelf`: **ported**. `tests/shelf_entries.rs::dnf_reason_shows_on_friend_shelf`.
- [x] `test_dnf_reason_is_optional`: **ported**. `tests/shelf_entries.rs::dnf_reason_is_optional`.
- [x] `test_moving_off_finished_clears_notes`: **ported**. `tests/shelf_entries.rs::moving_off_finished_clears_notes`.
- [x] `test_finish_notes_appear_on_club_pick`: **ported**. `tests/shelf_entries.rs::finish_notes_reach_the_club_readers`, `tests/parity_ports.rs::finish_notes_appear_on_the_club_pick`. SHELF checks what the pick reads (`bc_visible_shelf` for a co-member); the PAR port writes with SHELF's PATCH and reads CLUB's pick.
- [x] `test_reading_progress_shows_on_friend_shelf_and_pick`: **ported**. `tests/shelf_entries.rs::reading_progress_shows_on_friend_shelf_and_to_club_readers`, `tests/parity_ports.rs::reading_progress_shows_on_a_friends_shelf_and_the_pick`. As above, end to end in the PAR port.
- [x] `test_progress_is_optional_and_clears_when_leaving_reading`: **ported**. `tests/shelf_entries.rs::progress_is_optional_and_clears_when_leaving_reading`.
- [x] `test_invalid_progress`: **ported**. `tests/shelf_entries.rs::invalid_progress`.
- [x] `test_invalid_finish_fields`: **ported**. `tests/shelf_entries.rs::invalid_finish_fields`.
- [x] `test_free_shelf_order_persists_without_changing_reading_metadata`: **ported**. `tests/shelf_entries.rs::free_shelf_order_persists_without_changing_reading_metadata`.
- [x] `test_free_order_is_private_and_survives_status_changes_and_new_books`: **ported**. `tests/shelf_entries.rs::free_order_is_private_and_survives_status_changes_and_new_books`.

## test_spa_cache.py (1)

- [x] `test_app_html_is_not_cached_across_releases` (4 cases): **ported**. `tests/platform.rs::the_spa_shell_is_never_cached` (all four paths, `/book/ISBN9781408857892` included, plus a nested and a traversal path).

## test_spine_color.py (9)

- [x] `test_dominant_color_reads_the_cover_and_ignores_its_border`: **ported**. `src/media/color.rs::reads_the_cover_and_ignores_its_border`.
- [x] `test_dominant_color_prefers_a_vivid_colour_over_a_slightly_larger_pale_one`: **ported**. `src/media/color.rs::prefers_a_vivid_colour_over_a_slightly_larger_pale_one`.
- [x] `test_dominant_color_keeps_extremes_printable_and_rejects_junk`: **ported**. `src/media/color.rs::keeps_extremes_printable_and_rejects_junk`.
- [x] `test_spine_colors_are_sampled_once_and_served_with_the_book`: **ported**. `tests/media_spine.rs::spine_colors_are_sampled_once_and_served_with_the_book`.
- [x] `test_spine_colors_retry_after_network_trouble_but_not_after_a_missing_cover`: **ported**. `tests/media_spine.rs::spine_colors_retry_after_network_trouble_but_not_after_a_missing_cover`.
- [x] `test_a_new_cover_is_sampled_again`: **changed**. `tests/media_spine.rs::a_new_cover_is_sampled_again`. The colour belongs to the picture (`media.dominant_color`), so a new cover is a new, unsampled picture.
- [x] `test_spine_colors_only_cover_the_callers_shelf_and_need_a_login`: **ported**. `tests/media_spine.rs::spine_colors_only_cover_the_callers_shelf_and_need_a_login`.
- [x] `test_only_cover_hosts_are_ever_requested`: **changed**. `tests/media_spine.rs::only_cover_hosts_are_ever_requested`. Applied to the cover fetcher's redirects (`outbound::get_bytes`), with the Google hosts removed; a refused hop settles as "no cover".
- [x] `test_one_broken_cover_does_not_block_the_others`: **ported**. `tests/media_spine.rs::one_broken_cover_does_not_block_the_others`.

## test_stats_milestones_quotes.py (4)

- [x] `test_stats_year_in_review`: **changed**. `tests/shelf_stats.rs::stats_year_in_review`, `tests/parity_ports.rs::stats_count_what_the_routes_wrote` (end to end: the current pick set with CLUB's route, the reading date with `finished_on`). The club-wide `/api/stats` is split: personal numbers from `GET /api/stats`, the members, picks and club totals from `GET /api/clubs/{slug}/stats`.
- [x] `test_stats_with_empty_club`: **ported**. `tests/shelf_stats.rs::stats_with_empty_club` (both endpoints).
- [x] `test_milestones_crud_and_threads`: **changed**. `tests/club_milestones.rs::milestones_crud_and_threads`, `tests/parity_ports.rs::milestone_threads_run_through_the_post_routes` (end to end: the post on a milestone through POSTS' route, the 400 for a missing milestone, the detached post in the thread). PATCH is partial (clearing `due_at` takes an explicit `null`), and the chapter fields are `unit` with `range_from`/`range_to`; CLUB's port inserts the posts on a milestone with SQL, PAR's goes through the routes.
- [x] `test_quotes_crud_and_filters`: **changed**. `tests/shelf_quotes.rs::quotes_crud_and_filters`, `tests/parity_ports.rs::a_quote_reaches_the_feeds` (end to end: `quote_added` in the feeds, for the author after the delete, not for a co-member). Quotes name a catalog work: an unknown work is 404 `No such book.` and `?work=` takes the work id; another member sees a quote because they share a club.

## test_vote.py (7)

- [x] `test_vote_requires_auth`: **ported**. `tests/club_votes.rs::vote_routes_require_a_session`.
- [x] `test_empty_vote_then_nominate_and_count`: **ported**. `tests/club_votes.rs::an_empty_vote_opens_with_the_first_nomination`.
- [x] `test_members_can_vote_and_change_vote`: **ported**. `tests/club_votes.rs::members_vote_and_change_their_vote`.
- [x] `test_cannot_nominate_current_pick_or_duplicate`: **ported**. `tests/club_votes.rs::the_current_pick_and_duplicates_cannot_be_nominated`.
- [x] `test_nomination_limit`: **ported**. `tests/club_votes.rs::a_vote_takes_six_nominations`.
- [x] `test_apply_winner_uses_club_pick_flow`: **ported**. `tests/club_votes.rs::applying_the_winner_makes_it_the_pick` (the old pick's post is a row made with SQL), `tests/parity_ports.rs::old_pick_threads_survive_a_new_pick_and_an_applied_vote` (end to end: the old pick's note through POSTS' route).
- [x] `test_apply_requires_an_open_nomination`: **ported**. `tests/club_votes.rs::apply_needs_an_open_nomination`.

## test_vote_deadline.py (5)

- [x] `test_deadline_and_turnout_reporting`: **ported**. `tests/club_votes.rs::the_deadline_and_turnout_are_reported`.
- [x] `test_deadline_auto_applies_the_leader`: **ported**. `tests/club_votes.rs::the_deadline_applies_the_leader`.
- [x] `test_deadline_with_no_ballots_reopens_without_deadline`: **ported**. `tests/club_votes.rs::a_deadline_without_ballots_is_cleared`.
- [x] `test_tie_goes_to_earliest_nomination`: **ported**. `tests/club_votes.rs::a_tie_goes_to_the_earliest_nomination`.
- [x] `test_suggestions_rank_shared_tbr_and_subject_affinity`: **changed**. `tests/club_suggestions.rs::suggestions_rank_shared_wants_and_subject_affinity`, `tests/parity_ports.rs::suggestions_follow_the_shelf_and_pick_routes` (end to end: the shelf entries from SHELF's routes, the member joined by invite). Subjects and the Open Library rating are work columns (`works.subjects`, `works.ol_rating`).
