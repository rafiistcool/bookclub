# IMP (#38): legacy importer notes

Implementation notes for `server/src/importer/**`, `server/tests/importer_*.rs` and
`server/tests/fixtures/legacy/**`. The design is docs/phase1/legacy-mapping.md; this file records
where the code departs from it or adds detail, and what other packages need to know.

## Status

legacy-mapping.md is implemented in full, in two commits:

- Part 1: §0 frame (flag checks, the read-only reader for both legacy shapes, the one
  transaction, the target-not-empty refusal, `--dry-run`, `--offline`, `--allow-unmatched`, the
  lookup-error policy, the normalisation counters, the report), §1 users and avatars, §2 invites
  with the club and memberships, §3 books → works, editions, covers and `legacy_work_keys`.
- Part 2: §4 shelf entries, §5 layouts, §6 favourites, §7 picks, §8 milestones, §9 posts and both
  reaction tables, §10 quotes, §11 votes, nominations and ballots, §12 events, §13 search
  history, §14 push subscriptions (a DROPPED report line), the `merge_rules` collisions of §3.4,
  the full insert order and every `setval` of §0, and `rematch-legacy` (§3.3 step 5).

Every acceptance test of legacy-mapping.md §17 has a test (AC 3 is
`importer_run.rs::a_legacy_user_logs_in_with_the_old_password`, which now also reads the
imported club's members over HTTP). The dry run and the real run on a copy of the production
database stay a manual step with Rafael (#38 may stay open until then): no copy is on the build
machine, and it is never committed. The runbook is under "Manual step" below; its commands were
rehearsed through the binary on `fresh.sql` (dry run and real run identical apart from the
`DRY RUN` header, `rematch-legacy --dry-run`, the second run refused with exit 2, a `beta-invite`
before the import refused with exit 1).

Module map (`server/src/importer/`): `mod.rs` (frame, insert order, sequences), `source.rs`
(the reader), `users.rs`, `club.rs`, `books.rs`, `shelf.rs` (§4–§6), `layout.rs` (the layout
reader), `picks.rs` (§7, §8, §11), `posts.rs` (§9), `personal.rs` (§10, §13), `events.rs`
(§12), `rematch.rs`, `report.rs`.

## Deviations and added detail

### Part 1

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
- **The users and invites tables are locked** (`LOCK TABLE users, club_invites IN EXCLUSIVE
  MODE`) at the start of the transaction, before the emptiness checks, so a sign-up or a
  `beta-invite` on a running server cannot slip in.
- **A target with invites is refused too.** Invites keep their legacy ids and codes, and a
  `beta-invite` sign-up invite needs no user, so `users` being empty is not enough: an invite
  minted before the import would take id 1 and make the import fail on `club_invites_pkey` after
  every lookup ran. `run` checks `club_invites` right after `users`, before the first write and
  the first lookup, and fails with `ImportError::Other` (exit 1, not 2: the frozen
  `TargetNotEmpty` message names users) and `importer::TARGET_HAS_INVITES`: "target database
  already has invites (minted with beta-invite?); import into an empty database and mint invites
  after import-legacy". **Mint beta invites only after `import-legacy`.**
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
- **The books line counts every row in its class.** Each source row counts under the match of
  the work it maps to, a merged row too (an ISBN row that resolved to an OL row's work is `isbn
  matched`; an x/X twin or the second key of an ISBN-10/ISBN-13 pair counts in its first row's
  class), so the classes add up to the source count, as in the §0 example; `N merged` is the
  separate figure for rows that landed on an earlier row's work. The `editions` line counts
  distinct editions (the two keys of one ISBN identify one).
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

### Part 2

- **Over-long `take` and `dnf_reason` are cut, not refused.** legacy-mapping.md §4 says a take over
  140 characters aborts; §0's normalisation table lists `take` 140 and `dnf_reason` 200 among the
  values cut by characters. The importer follows §0 (counted as `shelf.take` /
  `shelf.dnf_reason`): only identifying values abort.
- **Dangling references.** SQLAlchemy never turned on SQLite's foreign keys, so a legacy row can
  name a user, book, pick, vote, nomination or post that does not exist. Such a row is not
  imported; the entity line counts it ("with a dangling reference") and `notes` lists it
  (`<table> <id>: <what> does not exist; not imported`). Optional references (`set_by`,
  `created_by`, `nominated_by`) become NULL instead; a tombstone whose author is gone keeps
  `author_id` NULL; an event's unknown book or pick becomes NULL (counted as `events.book_id` /
  `events.pick_id`); an event whose actor is gone is not imported (the target deletes an
  account's events with it).
- **The layout reader is the importer's own** (`importer/layout.rs`): SHELF's GET reader is not in
  this worktree. It applies the `PhysicalShelfLayout` rules of docs/phase1/spec/shelf.md §1.5
  (rows 1..12 of strict positive ints and `decor:` ids, at most 2100 items and 2000 books, no
  repeats, material, lax `lights`, spine keys, colours, widths 26..64, display, decoration kinds,
  decorations placed exactly once), fills every default, lowercases colours and drops unknown
  keys, so the stored value is canonical. Order: `merge_rules::remap_layout` for each merged
  entry, then the schema check, then the pruning of ids the user does not own (rows and spine
  keys), as the GET endpoint checks before it prunes. A layout that is not JSON or that the schema
  refuses becomes the default layout as a whole, like SHELF's GET (counted as
  `physical_shelf_layouts.layout_json`, listed with the reason); one bad value is never repaired
  field by field. Pruned ids are counted in the entity line.
- **Shelf merges** keep the `merge_rules::shelf_winner`; the losers are merged into it with
  `merge_shelf_fields` in winner order (the next best fills first). Each row is normalised for its
  own status first (rating only on finished 1..5, take only on finished, dnf_reason only on DNF,
  progress only on currently_reading 0..100, a negative `shelf_position` NULL; each counted), so a
  loser only contributes values it could hold. Positions are renumbered per (user, status) after
  the merges (not counted: every entry goes through it).
- **Favourites:** a changed position is counted as `user_favorites.position`; a fourth favourite
  after the merges (impossible: legacy positions are 1..3) would not be imported.
- **Extra open picks and votes** end at the `started_at` of the next row by id (open or not), else
  at the import time; each is counted (`club_picks.ended_at` / `next_up_votes.ended_at`) and
  listed.
- **Milestones:** the title is trimmed; an empty one becomes `Untitled` and a negative chapter
  NULL (both counted); a swapped range is counted as `pick_milestones.chapter_range`.
- **Posts:**
  - The parent check compares the mapped works, so a reply stays a reply when its legacy book and
    its parent's book became one work. A reply to a reply, a self-parent or a missing parent
    becomes a top-level post (counted `book_posts.parent_id`, listed).
  - A mirror copy whose pick was not imported becomes a diary entry (listed).
  - A live post with an empty body (impossible through the API) becomes a tombstone at its
    `updated_at`, else `created_at` (counted `book_posts.body`); a tombstone that still had a body
    or spoiler is cleared (counted the same); a `status_at` outside the four statuses becomes NULL
    (counted).
  - New pick notes are inserted in Python's mirror order, `(created_at, id)`, after the `setval`;
    a note with an empty body is not imported. The `club_pick_posts` line lists the skipped and
    new note ids (legacy ids, never the new post ids, so the dry run and the real run print the
    same).
  - `post_reactions` on mirrored notes are counted in two groups: those the diary copy also has
    (duplicates) and those it does not (dropped: removed in the diary or added to the old note
    after mirroring).
- **Votes:** a winner that is not a nomination of its vote stays NULL (counted
  `next_up_votes.winner_nomination_id`, listed); a ballot moved by a nomination merge is listed.
- **Events:** null payload values are dropped, as `activity::record` does. An unknown or skipped
  `post_id` is removed (counted `events.payload.post_id`); an unknown `author` is removed without
  an `author_id` (counted `events.payload.author`). `quote_added` matching is greedy over all
  candidate pairs, ordered by distance, then a matching excerpt, then the event and quote ids.
- **Search history:** a query over 300 characters is cut and its `normalized_query` recomputed
  with `text::casefold` (the copied one would no longer match it); an empty or over-long
  `normalized_query` is recomputed too (counted). Two queries of one user that then normalise
  alike keep the most recently used row (counted as merged); an empty query is not imported.
- **Quotes** with an empty body are not imported (the target needs 1..1000 characters).
- **Report:** entity lines use the legacy table names (`shelf`, `physical_shelf_layouts`,
  `user_favorites`, `club_picks`, `pick_milestones`, `book_posts`, `club_pick_posts`,
  `book_post_reactions`, `post_reactions`, `quotes`, `next_up_votes`, `next_up_nominations`,
  `next_up_ballots`, `events`, `search_history`, `push_subscriptions`), so the name column is 22
  wide. Merge lines name legacy ids only: part 1's `both are work <id>` became `both resolve to one
  work`, because work ids differ between a dry run and the real run. `Report` gained `notes`
  (the per-row list the rendered report prints under `notes:`).
- **`rematch-legacy` commits per edition.** It runs on the live server, so each legacy edition
  gets its own short transaction (lookup, ingest's upgrade, the `legacy_work_keys` update, then
  commit, or rollback under `--dry-run`): ingest's advisory and row locks are never held across
  another edition's lookup, and a member's lookup of an ISBN already processed does not wait for
  the whole run. A throttled lookup rolls its transaction back before the wait (`--retry-wait-ms`,
  hidden, as for `import-legacy`) and retries in a new one; an unavailable source does not abort
  (it is a retry tool: the edition stays as it is and is reported). A deadlock (40P01,
  `error::is_deadlock`) retries the edition's transaction up to twice (conventions.md §7.2); any
  other database error rolls back that edition only, is reported (`error: database: …`) and the
  run continues. Only a failure to reach the database at all (pool or I/O errors) aborts; the
  editions already upgraded stay upgraded, and a rerun continues with the rest (they are no
  longer `legacy`). A hit that leaves the edition `legacy` (the lookup knew the ISBN under another
  existing work without touching this edition) is reported as "under another work" for a merge
  through the queue. `legacy_work_keys.match` becomes `isbn_matched` only on
  `isbn_unmatched`/`isbn_error` rows of an upgraded edition. The report has one `legacy editions`
  line (`N → M upgraded (… unmatched, … with the source unavailable, … under another work, … with
  a database error)`) and one note per ISBN. Because a dry run rolls back each edition, two legacy
  ISBNs whose hits land on one new work can report differently in the dry run and the real run
  (rare; the real run's report is the record).
- **Fixture changes** (`generate.py`, regenerated; two runs give the same bytes):
  - The pick note whose diary copy the author deletes is now created before the note with the
    milestone. Its id (2) lies below a mirrored id (3), as §9.2 assumes; before, it was the
    highest mirrored id, so after the deletion it looked genuinely new.
  - Demo's legacy spine `width` is 30: 3 was never valid (26..64), and the whole layout would
    have become the default.
  - anna has both merge rows (and one more book) as favourites, so the favourites merge is in the
    merge fixture.

## Manual step: the dry run and the real run on a copy of the production database (with Rafael)

#38 AC: the importer runs once on a copy of the production `bookclub.db` before the cutover (#52).
The copy is never committed and never leaves Rafael's machines.

1. **Take the copy** with the legacy app's online backup (SELFHOST.md §4): `docker compose exec
   bookclub python -m app.backup /data/bookclub-backup.db`, then copy
   `./data/bookclub-backup.db` to the machine that runs the import. Read the legacy settings
   from its `.env`: `BOOKCLUB_NAME` (→ `--club-name`) and `BOOKCLUB_TZ` (→ `--timezone`), and
   pick the owner's username (→ `--owner`).
2. **An empty target.** A new Postgres database; `import-legacy` applies the migrations itself.
   Set `DATABASE_URL`, `MEDIA_DIR` and the rest of the `serve` configuration. Do **not** run
   `beta-invite` (or let anyone sign up) before the import: users or invites in the target make
   it refuse (exit 2 for users, exit 1 for invites, nothing written).
3. **Dry run**, with network lookups (only then are the ISBN outcomes real), and a fixed slug so
   the two reports compare line by line:

   ```
   bookclub-server import-legacy bookclub-backup.db --owner <username> \
       --club-name "<BOOKCLUB_NAME>" --timezone <BOOKCLUB_TZ> --club-slug <slug> \
       --dry-run > dry.txt
   ```

   Exit 1 with "a source is unavailable" means DNB or Open Library could not answer: retry later,
   or add `--allow-unmatched` (those rows become bare facts, counted as `isbn error`, and
   `rematch-legacy` retries them later). `--offline` skips every lookup (all ISBN rows bare facts
   unless the target's catalog already knows the ISBN).
4. **Read `dry.txt` together:** every entity line (source → target; the differences must be the
   listed merges and skips), `normalised` (repaired values per column), `merges`, `differences`
   (matched ISBN rows whose year or pages changed against what the legacy app showed), `isbn
   keys` (each ISBN with matched/unmatched/error) and `notes` (dangling rows, layouts imported as
   the default, skipped pick notes). Anything unexpected: stop, fix, rerun the dry run.
5. **Real run**: the same command without `--dry-run`, into `real.txt`. Then
   `diff <(tail -n +2 dry.txt) real.txt`: only the `lookup wait` line, or an ISBN whose upstream
   record changed in between, may differ.
6. **After the import:** `bookclub-server make-admin <username>`; then `beta-invite` if needed;
   if `isbn unmatched` or `isbn error` is not 0, `bookclub-server rematch-legacy --dry-run`, then
   `rematch-legacy` (safe on the live server: it commits per edition). Spot-check in the browser:
   a legacy user signs in with the old password and gets the email prompt, the club, shelves,
   layouts, pick notes and the activity feed are there, old `/works/…` links resolve.
7. Attach the (non-personal) report counts to #38 and close it.

## New detail strings

None. The importer is a CLI; it adds no HTTP endpoint and no `error::msg` string.

## Python tests

- `test_migrations.py::test_legacy_database_is_upgraded_in_place` -> ported on the migrated-shape
  fixture (`tests/importer_run.rs::migrated_databases_import_with_the_defaults`) and the
  never-booted `LEGACY_SCHEMA` (`a_database_from_before_every_column_migration_imports`).
- `test_auth.py::test_preference_columns_backfill_pre_existing_users` -> ported on the shape it
  builds, a users table without the preference columns
  (`tests/importer_run.rs::a_database_from_before_every_column_migration_imports`: the reader's
  defaults paper/system/en and every notification on). migrated.sql already holds the backfilled
  values, so the migrated-shape test does not exercise the defaults.
- `test_migrations.py::test_fresh_database_matches_migration_list` -> replaced: there is no
  hand-rolled column list any more (sqlx migrations, `cargo sqlx prepare --check`); the importer
  reads a fresh-shape database with every `COLUMN_MIGRATIONS` column
  (`tests/importer_run.rs::the_fresh_fixture_imports_with_matching_counts`).
- `test_diary.py::test_legacy_pick_notes_are_imported_once_with_reactions` -> ported
  (`tests/importer_posts.rs::legacy_pick_notes_are_imported_once_with_reactions`): on
  migrated.sql, a mirrored note and a never-mirrored one are each imported once, with their
  reaction.

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
  makes no upstream request at all. `rematch-legacy` calls `resolve_isbn_remote` once per legacy
  edition with an ISBN.
- **SHELF:** imported layouts are stored canonical (every default filled, colours lowercased,
  unknown keys dropped) and already pruned to the owner's entries, so the GET reader reads them
  unchanged. `importer/layout.rs` repeats the `PhysicalShelfLayout` rules; a change to them should
  be mirrored there. Imported `rating_visibility` is `audience` everywhere.
- **POSTS:** every imported diary entry and reply is `audience`. Club posts are the mirror copies
  (with `milestone_id` when the legacy note had one on the same pick) and their replies, plus the
  pick notes written after the last mirroring, whose ids lie above every kept id (they sort as the
  newest in id-ordered feeds).
- **CLUB:** imported `reaction` payloads carry `author_id`, never `author`; `note_posted` and
  `reaction` may lack `post_id` (a skipped note), `quote_added` may lack `quote_id` (a deleted
  quote: the feed shows it to its actor only). Personal events never carry a pick. Imported
  milestones have `unit` `chapter`, or NULL for a title-only checkpoint, never `percent`.
