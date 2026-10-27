# IMP (#38): legacy importer notes

Implementation notes for `server/src/importer/**`, `server/tests/importer_*.rs` and
`server/tests/fixtures/legacy/**`. The design is docs/phase1/legacy-mapping.md; this file records
where the code departs from it or adds detail, and what other packages need to know.

## Status

Part 1 (this commit series) implements legacy-mapping.md in section order up to and including §3:

- §0 frame: flag checks, the read-only reader for both legacy shapes (missing tables and columns
  read as the `COLUMN_MIGRATIONS` defaults), the one transaction, the target-not-empty refusal,
  `--dry-run`, `--offline`, `--allow-unmatched`, the lookup-error policy (`--retry-wait-ms`), the
  normalisation counters and the report; `setval` for `users` and `club_invites`.
- §1 users and avatars, §2 invites (with the club and memberships of §0/§15), §3 books → works,
  editions, covers and `legacy_work_keys` (classification, ISBN re-match, merges of rows that
  reach one work, the year/pages differences of matched rows).

Not yet (part 2): §4–§14 (shelf, layouts, favourites, picks, milestones, posts and reactions,
quotes, votes, events, search history, push subscriptions in the report), the remaining `setval`
calls and the insert-order details they need (`setval` on posts mid-way, the vote winner
`UPDATE`), the `merge_rules`-based collisions of §3.4 (shelf, favourites, nominations), and
`rematch-legacy` (`importer::rematch` is still the W1 stub).

## Deviations and added detail

- **Orphaned avatars are removed.** legacy-mapping.md §0 lets avatar files written before a failure
  stay behind and be listed. The importer instead deletes every storage prefix it wrote when the
  transaction rolls back after an error or the commit fails (best effort, a failure is logged with
  `tracing::warn`). Nothing references them after the rollback, so this is safe.
- **The report's ISBN list names the matched work's title, not its id.** Postgres sequences advance
  in a dry run's rolled-back transaction, so a work the lookup created would get another id in the
  real run and the two reports would never be equal. `Report::isbns` (structured) still carries the
  work id.
- **`Report` has structured fields** next to `lines`: `dry_run`, `entities`, `normalised`, `merges`,
  `differences` (matched ISBN rows whose `BookOut` year or pages differ, legacy-mapping.md §17 AC 11),
  `isbns`, `lookup_wait` and `books` (legacy book id → work, edition, match; part 2 maps the other
  tables through it). `Display` prints a `DRY RUN: …` header from `dry_run`, so `lines` of a dry
  run and the real run are equal (when `--club-slug` is given; the default slug is random).
- **`clubs.created_at`** is the earliest legacy `users.created_at` (the design does not say): the
  club is as old as its first member, and every membership's `joined_at` is that member's
  `created_at`.
- **The users table is locked** (`LOCK TABLE users IN EXCLUSIVE MODE`) at the start of the
  transaction, before the emptiness check, so a sign-up on a running server cannot slip in.
- **Matched ISBN rows** go through `ingest::upsert_edition(matched work, EditionRecord { source:
  Legacy, isbn13, pages, publication_year, ol_cover_id })` with `Mode::Refresh`: it finds the
  edition with that ISBN (the one the lookup persisted) and fills only its empty fields, then the
  work's NULL fields through the §7.3 fill. When the lookup returned a work without an edition
  (`FakeHit::Existing { edition_id: None }`), this creates a `legacy` edition with the ISBN under the
  matched work, and `legacy_work_keys.edition_id` points at it. If the ISBN's edition belongs to
  another work than the lookup named, the row keeps the lookup's work and gets no edition.
- **Covers of ISBN rows** go on the edition record; the work gets the cover through the §7.3 NULL
  fill. For a matched row the cover id is passed only when the matched work has no cover (§3.2).
  The spine colour is written to that Open Library media row's `dominant_color` only while it is
  NULL; a colour that is not `#rrggbb` (after lowercasing) or `''` becomes NULL and is counted as
  `books.spine_color`.
- **Collisions:** an x/X pair of ISBN keys canonicalises to one `legacy_key` (the table's primary
  key), so only the first row gets a `legacy_work_keys` row; the second maps onto the same work and
  is listed as a merge. The ISBN-10 and ISBN-13 keys of one book are two keys: both get rows with
  the first one's work, edition and match, and the second is listed (`same ISBN as book N`) without
  a second lookup. An OTHER key with an empty last segment (`/works/`) gets no `legacy_work_keys`
  row (the CHECK needs a non-empty id) and is listed under `notes`.
- **Normalisation additions:** invite codes that `club_invites.code` refuses are normalised like
  typed codes (whitespace and `-` removed, uppercased, cut to 128; nothing left → `LEGACY<id>`) and
  counted as `invites.code` rather than aborting; a title over 1000 characters is cut and then
  trimmed (ingest collapses whitespace anyway); invalid `subjects` JSON is counted as
  `books.subjects`; theme, color_mode and locale outside the target's sets are counted as
  `users.<column>`.
- **Errors:** bad flags (`--club-name`, `--timezone`, `--club-slug`) are `ImportError::Other`; bad
  legacy data (usernames outside `^[a-z0-9_]{2,32}$`, all listed; an unknown `--owner`; unreadable
  values with table, id and column) is `ImportError::Source`; an unavailable source without
  `--allow-unmatched` is `ImportError::Upstream`. All exit with 1; only `TargetNotEmpty` exits with
  2. `--owner` is trimmed and lowercased.
- **Throttled lookups** wait `--retry-wait-ms`, default 60 s: `LookupError::Throttled` carries no
  Retry-After.
- **The slug** is built by the importer's own copy of the club-creation algorithm
  (`clubs::manage::slug_base` is not a frozen interface).
- **Fixtures:** besides `fresh.sql`, `migrated.sql`, `generate.py` and `README.md`, the directory
  holds `legacy_schema.sql` (LEGACY_SCHEMA never booted: the "very old database" of §0) and
  `harness.rs`, the helpers the importer test files share through `#[path]` (`tests/common` is
  frozen, and the helpers have to live in an IMP-owned path).

## New detail strings

None. The importer is a CLI; it adds no HTTP endpoint and no `error::msg` string.

## Python tests

- `test_migrations.py::test_legacy_database_is_upgraded_in_place` -> ported on the migrated-shape
  fixture (`tests/importer_run.rs::migrated_and_very_old_databases_import_with_the_defaults`).
- `test_auth.py::test_preference_columns_backfill_pre_existing_users` -> ported on the same test.
- `test_migrations.py::test_fresh_database_matches_migration_list` -> replaced: there is no
  hand-rolled column list any more (sqlx migrations, `cargo sqlx prepare --check`); the importer
  reads a fresh-shape database with every `COLUMN_MIGRATIONS` column
  (`tests/importer_run.rs::the_fresh_fixture_imports_with_matching_counts`).
- `test_diary.py::test_legacy_pick_notes_are_imported_once_with_reactions` -> part 2 (posts).

## For other packages

- **MED:** legacy avatars are `media` rows with `kind='avatar'`, `source='legacy'`,
  `content_type='image/jpeg'` (or `image/webp` if a blob was one), `variants='{a}'`, the size read
  from the header, `storage_key` set and `sha256` of the blob. The stored bytes are the legacy JPEG,
  not a WebP: `/media/{public_id}/a` must serve the row's `content_type`.
- **FE:** imported users sign in by username with their old password; they have no email, so
  `UserOut.needs_email_prompt` is true until they add one or dismiss the prompt.
- **CAT/FE:** old links keep working through `legacy_work_keys` (`WorkRef::parse` +
  `resolve_local`): `/works/OL…W`, `/works/BC…`, `/works/ISBN…` (also invalid ones and ISBN-10
  keys, by their own key) and GB keys hashed to `sha256:<hex>`.
- **PAR:** `--offline` checks only `lookup::local_isbn`, so an offline run with the real lookup
  makes no upstream request at all.
