# Parity notes: reasoned drops

PAR (#49) builds the parity checklist from two sources: the `/// parity: <file>::<test>` doc comments on Rust tests (ports, and reasoned changes tested by the Rust test they sit on), and this file. This file has one line per Python test that has **no Rust port of its behaviour**: it was dropped, replaced by a different feature, or moved to another package. Each line is `file::test -> reason`. Tests listed here may also carry a `/// parity:` line on the Rust test that covers the replacement; the line here says why the old behaviour is gone.

Packages add their own section when they drop a test.

## Wave 1: auth (#39)

Scope: `test_auth.py`, `test_selfhost.py`, `test_backup.py`, `test_notes_and_meeting.py::test_change_password*` and `::test_notification_prefs_default_on_and_patch` (work-plan.md, Wave 1 deliverable 4). Every other test of that scope is ported; its Rust test carries the `/// parity:` line.

- `test_auth.py::test_preference_columns_backfill_pre_existing_users` -> moved to IMP: there is no in-place schema upgrade any more; the importer fills the preference defaults (legacy-mapping.md §17 test 2, work-plan.md IMP parity list).
- `test_selfhost.py::test_placeholder_detection` -> dropped, self-hosting is removed: there are no placeholder secrets or bootstrap invites to detect. The unsafe production settings that remain are refused at startup (`tests/auth_config.rs::production_startup_refuses_unsafe_settings`).
- `test_selfhost.py::test_production_generates_and_reuses_secrets` -> dropped, self-hosting is removed: sessions and invites are database rows, so no `.secret_key` or `.bootstrap_invite` is generated or reused (marked on `tests/auth_config.rs::production_startup_refuses_unsafe_settings`).
- `test_selfhost.py::test_dotenv_only_production_boot` -> dropped: configuration comes from the environment only, without a `.env` file; the production boot itself is `tests/auth_config.rs::a_production_app_registers`.
- `test_selfhost.py::test_invalid_theme_warns_and_falls_back` -> dropped: the accent colours are fixed (the legacy defaults), so there is no theme setting to fall back from (marked on `tests/auth_config.rs::config_follows_the_service_settings`).
- `test_selfhost.py::test_backup_copies_sqlite` -> replaced: the SQLite backup CLI is gone; the operator backs up Postgres (#51), and each member downloads their own data (`tests/auth_export.rs::a_member_downloads_a_zip_of_their_data`).
- `test_backup.py::test_backup_requires_auth` -> replaced by the personal export, which needs a session too (`tests/auth_export.rs::the_export_needs_a_session`).
- `test_backup.py::test_backup_returns_sqlite_for_member` -> replaced: a member gets a ZIP of their own data (`data.json` plus their uploaded pictures, conventions.md §5.7), not the club's SQLite file (`tests/auth_export.rs::a_member_downloads_a_zip_of_their_data`).

## Wave 1: clubs (#40)

Scope: `test_members.py` (work-plan.md, Wave 1 deliverable 5). Its one test is ported with a reasoned change, so it has no line of its own here: `tests/clubs_members.rs::members_list_includes_the_caller_and_previews_reading` carries the `/// parity:` line and keeps the reading preview, while the "hide self" half is reversed on purpose (the club roster lists the caller with `you: true`, endpoint-map §3).
