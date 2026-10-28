# Parity notes: reasoned drops

PAR (#49) builds the parity checklist from two sources: the `/// parity: <file>::<test>` doc comments on Rust tests (ports, and reasoned changes tested by the Rust test they sit on), and this file. This file has one line per Python test that has **no Rust port of its behaviour**: it was dropped, replaced by a different feature, or moved to another package. Each line is `file::test -> reason`. Tests listed here may also carry a `/// parity:` line on the Rust test that covers the replacement; the line here says why the old behaviour is gone.

Packages add their own section when they drop a test. Wave 2 packages could not edit this file, so they listed their drops in `docs/phase1/notes/<pkg>.md`; PAR (#49) copied those lines into the Wave 2 sections below. The complete mapping of every Python test, ports included, is `docs/phase1/parity-checklist.md`, which `server/tests/parity_checklist.rs` keeps in step with this file's sources.

## Wave 1: auth (#39)

Scope: `test_auth.py`, `test_selfhost.py`, `test_backup.py`, `test_notes_and_meeting.py::test_change_password*` and `::test_notification_prefs_default_on_and_patch` (work-plan.md, Wave 1 deliverable 4). Every other test of that scope is ported; its Rust test carries the `/// parity:` line.

- `test_auth.py::test_preference_columns_backfill_pre_existing_users` -> moved to IMP: there is no in-place schema upgrade any more; the importer fills the preference defaults (legacy-mapping.md §17 test 2, work-plan.md IMP parity list). IMP ported it on the shape the backfill handled: `tests/importer_run.rs::a_database_from_before_every_column_migration_imports`.
- `test_selfhost.py::test_placeholder_detection` -> dropped, self-hosting is removed: there are no placeholder secrets or bootstrap invites to detect. The unsafe production settings that remain are refused at startup (`tests/auth_config.rs::production_startup_refuses_unsafe_settings`).
- `test_selfhost.py::test_production_generates_and_reuses_secrets` -> dropped, self-hosting is removed: sessions and invites are database rows, so no `.secret_key` or `.bootstrap_invite` is generated or reused (marked on `tests/auth_config.rs::production_startup_refuses_unsafe_settings`).
- `test_selfhost.py::test_dotenv_only_production_boot` -> dropped: configuration comes from the environment only, without a `.env` file; the production boot itself is `tests/auth_config.rs::a_production_app_registers`.
- `test_selfhost.py::test_invalid_theme_warns_and_falls_back` -> dropped: the accent colours are fixed (the legacy defaults), so there is no theme setting to fall back from (marked on `tests/auth_config.rs::config_follows_the_service_settings`).
- `test_selfhost.py::test_backup_copies_sqlite` -> replaced: the SQLite backup CLI is gone; the operator backs up Postgres (#51), and each member downloads their own data (`tests/auth_export.rs::a_member_downloads_a_zip_of_their_data`).
- `test_backup.py::test_backup_requires_auth` -> replaced by the personal export, which needs a session too (`tests/auth_export.rs::the_export_needs_a_session`).
- `test_backup.py::test_backup_returns_sqlite_for_member` -> replaced: a member gets a ZIP of their own data (`data.json` plus their uploaded pictures, conventions.md §5.7), not the club's SQLite file (`tests/auth_export.rs::a_member_downloads_a_zip_of_their_data`).

## Wave 1: clubs (#40)

Scope: `test_members.py` (work-plan.md, Wave 1 deliverable 5). Its one test is ported with a reasoned change, so it has no line of its own here: `tests/clubs_members.rs::members_list_includes_the_caller_and_previews_reading` carries the `/// parity:` line and keeps the reading preview, while the "hide self" half is reversed on purpose (the club roster lists the caller with `you: true`, endpoint-map §3).

## Wave 2: catalog (#41)

From `docs/phase1/notes/cat.md`, "Python tests reasoned away". The verdicts are those of `docs/phase1/spec/google.md` §4.

- `test_search.py::test_open_library_connect_timeout_is_short` -> partly reasoned: the 8 s connect and 15 s read timeouts are `outbound.rs` constants (W1). `tests/catalog_search.rs::request_policy_is_short` checks the request policy CAT uses (15 s per attempt, 2 s throttle wait).
- `test_google_books.py::test_google_books_disabled_without_key` -> Google Books removed (#41).
- `test_google_books.py::test_google_catalog_ids` -> the Google id formats are dropped with Google Books (#41). The ISBN normalisation half is ported (`tests/catalog_isbn.rs::isbn10_with_x_check_digit_is_accepted`), and the legacy key classification belongs to IMP.
- `test_google_books.py::test_map_volume_prefers_isbn_key` -> Google Books removed (#41).
- `test_google_books.py::test_map_volume_without_isbn_uses_volume_id` -> Google Books removed (#41).
- `test_google_books.py::test_volume_details_strips_html` -> Google Books removed (#41). Google and VLB descriptions are never stored.
- `test_google_books.py::test_empty_subject_preview_recovers_with_google_topic_search` -> the Google topic fallback is removed (#41). The "open a displayed result without another call" part is ported (`tests/catalog_detail.rs::a_displayed_hit_opens_during_a_503_outage`).
- `test_google_books.py::test_expired_discovery_does_not_supply_book_details` -> the discovery cache is replaced by persisting on search (#41).
- `test_google_books.py::test_volume_key_detail_and_refresh` -> Google Books removed (#41).
- `test_google_books.py::test_isbn_cover_url_from_work_key` -> there are no client-facing Open Library cover URLs any more (#43). Covers are `/media/…`.
- `test_google_books.py::test_google_cover_wins_over_open_library_isbn_cdn` -> Google Books removed (#41).
- `test_google_books.py::test_isbn_cdn_skipped_when_google_key_is_set` -> Google Books removed (#41).
- `test_google_books.py::test_title_and_popular_sorts_stay_on_google` -> the Google sort mapping is removed (#41). Local sort semantics: `title` also orders local hits.
- `test_google_books.py::test_volume_key_without_google_key` -> Google Books removed (#41). GB keys resolve through `legacy_work_keys` (`tests/catalog_detail.rs::legacy_keys_resolve_through_the_key_map`).
- `test_google_books.py::test_opening_discovered_book_does_not_call_google` (the "no row after browsing" and "a further search makes exactly one call" parts) -> INVERT: browsing stores every hit, and opening a stored stub tries its one-time enrichment, which after an upstream 429 trips the Open Library cooldown. A 503 trips none (tested). The rest is ported.
- `test_google_books.py::test_discovered_book_opens_during_existing_google_cooldown` (the `/work/ISBN…` case) -> `/work/{id}` is dropped (endpoint-map §4); the `/works/` case is ported.
- `test_google_books.py::test_discovered_book_refresh_still_fetches_fresh_metadata` ("exactly 1 call") -> a refresh fetches the work JSON and its search doc. The port asserts that the work JSON is fetched again despite the cache.
- `test_google_books.py::test_refresh_updates_google_isbn_book` (the Google and shelf-stub parts) -> Google Books removed (#41), and shelving needs an existing work. The "refresh fills a bare record" part is ported.
- `test_google_books.py::test_shelf_add_persists_google_cover_url` -> DROP/INVERT: client cover URLs are no longer accepted (#43), and Google covers are dropped with Google Books (#41). The inverse rule, "a client-sent cover is never stored", is ported in `tests/parity_ports.rs::a_shelf_add_never_stores_a_client_cover` and `::a_pick_never_stores_a_client_cover`.

Reasoned changes inside CAT's ports (the behaviour changed on purpose; the Rust test checks the new one):

- `test_search.py::test_cancelled_fetch_unblocks_coalesced_waiters` and `test_book_details.py::test_cancelled_work_details_unblocks_coalesced_waiters` -> when the caller running a coalesced fetch is cancelled, a waiter runs the fetch itself instead of failing with 502 (work-plan catalog 16).
- `test_book_details.py::test_work_details_rejects_bad_ids_and_upstream_failure` -> `/works/not-a-key` is 404; the dropped `/work/` route answered 400.

## Wave 2: media (#43)

From `docs/phase1/notes/med.md`, "Reasoned away".

- `test_avatars.py::test_avatar_blob_stays_deferred_when_serializing` -> there is no avatar blob in `users` any more (#43): pictures are `media` rows with bytes in the `MediaStore`, and no user query can load them.

## Wave 2: club features (#45)

From `docs/phase1/notes/club.md`, "Parity (part 2)". Web Push is dropped until Phase 4 (endpoint-map §12 `/api/push/*`, work-plan appendix club 15); the activity events stay.

- `test_push.py::test_vapid_key_is_generated_and_stable` -> no Web Push in Phase 1 (Phase 4).
- `test_push.py::test_subscribe_list_unsubscribe` -> the `/api/push/*` routes are dropped (Phase 4).
- `test_push.py::test_new_pick_notifies_other_members_who_opted_in` -> no push on a new pick; `pick_set` is recorded (Phase 4).
- `test_push.py::test_new_note_notifies_except_author_and_respects_pref` -> no push on a post (Phase 4).
- `test_push.py::test_dead_subscriptions_are_pruned` -> no push subscriptions (Phase 4).
- `test_push.py::test_push_copy_catalog_covers_every_locale` -> push copy (`backend/app/i18n.py`) is not ported (Phase 4).
- `test_push.py::test_push_copy_follows_recipient_locale` -> push copy is not ported (Phase 4).
- `test_push.py::test_meeting_reminder_sent_once_inside_24h` -> `GET pick` has no side effects; the reminder becomes a Phase 4 job (`club_picks.reminder_sent_at` is kept).
- `test_push.py::test_meeting_reminder_not_sent_far_ahead_or_when_opted_out` -> as above (Phase 4).
- `test_push.py::test_vote_close_notifies_new_pick` -> apply and auto-close record `pick_set` and `vote_closed` but send no push (Phase 4).

## Wave 2: posts (#44b) and the importer (#38)

From `docs/phase1/notes/posts.md` and `docs/phase1/notes/imp.md`.

- `test_diary.py::test_legacy_pick_notes_are_imported_once_with_reactions` -> moved to IMP: the boot-time copy of `club_pick_posts` is the importer's job. IMP ported it: `tests/importer_posts.rs::legacy_pick_notes_are_imported_once_with_reactions`.
- `test_migrations.py::test_legacy_database_is_upgraded_in_place` -> replaced: there is no in-place SQLite upgrade; the importer reads the migrated shape and the never-booted legacy schema with the column defaults (`tests/importer_run.rs::migrated_databases_import_with_the_defaults`, `::a_database_from_before_every_column_migration_imports`).
- `test_migrations.py::test_fresh_database_matches_migration_list` -> replaced: there is no hand-rolled column list any more (sqlx migrations, `cargo sqlx prepare --check`); the importer reads a fresh-shape database with every `COLUMN_MIGRATIONS` column (`tests/importer_run.rs::the_fresh_fixture_imports_with_matching_counts`).

## Wave 2: SHELF (#44a), OAPI (#47), HAR (#42)

No drops. SHELF's reasoned changes are on its tests' doc comments and in `docs/phase1/notes/shelf.md`; OAPI ported the `/api/docs` half of `test_selfhost.py::test_production_app_registers_and_hides_docs`; the Python app has no harvester.
