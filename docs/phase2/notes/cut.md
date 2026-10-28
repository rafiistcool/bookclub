# CUT (#52, #38): cutover preparation notes

Notes for the integrator. The package owns `server/src/importer/source.rs`,
`server/tests/importer_run.rs`, `server/tests/cutover_counts.rs`,
`server/tests/fixtures/legacy/README.md`, `deploy/cutover/**`, `deploy/legacy-origin/**` and
`docs/phase2/cutover.md`. It touched no shared file, no migration, no DTO and no frontend file, so
`server/.sqlx/` and `frontend/src/api/generated.ts` are unchanged.

## What was built

- **WAL-safe legacy reader** (`server/src/importer/source.rs::read`). The importer no longer opens
  the legacy file. It copies `<db>` and, when they exist, `<db>-wal` and `<db>-journal` into a new
  directory `$TMPDIR/bookclub-legacy-<16 hex>` (mode 0700, files 0600), opens that copy with
  `SQLITE_OPEN_READ_ONLY`, and removes the directory when the read ends (also on an error).
  - Why: a WAL-mode database (production, and every `app.backup` copy, whose header keeps WAL
    mode) cannot be opened read-only where SQLite cannot create `-wal`/`-shm`. From the `:ro`
    import mount this failed with `attempt to write a readonly database` (a backup copy without
    `-wal`) or `unable to open database file` (a `-wal` without `-shm`). `immutable=1` was not
    used: it ignores committed `-wal` content.
  - `-shm` is never copied (SQLite rebuilds it from the `-wal`). A hot `-journal` is copied so the
    read-only open refuses it (`SQLITE_READONLY_ROLLBACK`) instead of reading a half-written file;
    the importer then fails with `ImportError::Source`: "`<path>` has a hot journal (an
    interrupted write): open it once with sqlite3 or the legacy app to roll it back, then import
    it again".
  - New error texts (CLI only, `ImportError::Source`, exit 1): `cannot create a private copy of
    <db> in <tmp>: …`, `cannot copy <file> into <dir>: …`, `cannot read <db>: …`, and the hot
    journal one above. A missing source file now fails at the copy (`cannot copy … No such file or
    directory`) instead of in SQLite.
  - The random directory name comes from `auth::random::fill` (conventions §5.1). No new crate.
- **Independent count check** (`deploy/cutover/`):
  - `legacy-counts.sql` (sqlite3) and `target-counts.sql` (psql) print `count|<entity>|<rows>`
    lines named after the import report's entity lines (`a+b` on the target side where two report
    lines land in one table: `book_posts+club_pick_posts`, `book_post_reactions+post_reactions`),
    `user|<id>|<username>|<what>|<rows>` lines (shelf, favorites, quotes, posts, reactions,
    searches, ballots, events, layout, avatar) and `hash|<id>|<username>|<password_hash>` lines.
    The report's `covers` target has no target query (lookups add cover media to the same table).
  - `compare.sh [--report real.txt] LEGACY_DB [-- PSQL_COMMAND...]` copies the legacy file (+
    `-wal`, `-journal`) into a private temp dir, runs both files, prints the counts side by side
    and per user, and compares the account triples without printing a hash. With `--report` every
    legacy count must equal the report's source column and every target count its target column.
    Exit 0/1/2 (match / difference / usage or query error). It refuses a dry-run report, and it
    strips CR line endings from a report saved through a TTY.
  - `server/tests/cutover_counts.rs` runs both SQL files (without their CLI settings lines) around
    the `fresh.sql` and `migrated.sql` imports and checks them against the report exactly as
    `compare.sh --report` does, plus the expected per-user differences of `fresh.sql` and the
    account triples (equal after the import, different for the one user after a login).
- **Old domain** (`deploy/legacy-origin/`): `Caddyfile.example` (301 to the new host keeping path
  and query, inside `handle`; `/sw.js` served from `/srv/bookclub-legacy-origin` with a JavaScript
  type and `Cache-Control: no-store`) and `sw.js` (skipWaiting, claim, delete every cache,
  unregister, reload the open windows; no fetch handler). Host names are `{$BOOKCLUB_OLD_HOST:…}`
  and `{$BOOKCLUB_NEW_HOST:…}` placeholders so `caddy validate` runs on the file as it is.
- **Runbook** `docs/phase2/cutover.md`: checklist, DE/EN announcement (plus a short "it is live"
  message), cutover-day commands, rollback window, afterwards, and §6 with the `legacy-python`
  branch and tag commands for Q6.
- **Fixture README**: how to regenerate the fixtures from the `legacy-python` branch once
  `backend/` is gone, and `--test cutover_counts` in the test command.

## Tests

- `cargo test --test importer_run wal`: 2 new tests.
  - `a_wal_database_with_uncheckpointed_rows_imports_from_a_read_only_directory`: `fresh.sql` in
    WAL mode plus a user, a quote and a theme change left in the `-wal` (no checkpoint on close,
    `-shm` removed), directory `chmod 555`: users 6 → 6, quotes 4 → 4, the rows and the change
    arrive, and the directory's files and bytes are unchanged.
  - `a_wal_backup_copy_without_its_wal_file_imports_from_a_read_only_directory`: a checkpointed
    WAL file (header bytes 18/19 = 2, no `-wal`) in a `chmod 555` directory imports with the
    fixture's counts.
  - Both failed before the fix (checked by running them against the old `read`): "attempt to write
    a readonly database" and "unable to open database file".
  - `chmod 555` binds only a non-root user. If a CI job runs the tests as root, they still pass but
    no longer tell the fix apart; on the `agents` VM they run as uid 1000.
- `source.rs` unit tests: `a_hot_journal_is_refused_with_its_reason` and
  `the_private_copy_is_removed_and_a_missing_file_is_named`.
- `cargo test --test cutover_counts`: 2 tests.

## For the docs (integrator)

- `docs/phase1/legacy-mapping.md:37` ("Read-only source"): the file is copied (with `-wal` and
  `-journal`) into a private temporary directory under `$TMPDIR`, and that copy is opened with
  `SQLITE_OPEN_READ_ONLY`; the source file is never opened. Never `immutable=1`.
- `docs/phase1/conventions.md` §18 (rusqlite note at `:1697`): unchanged in substance; "read-only"
  still holds.
- `deploy/hosted/README.md` (DEP), import section:
  - run `docker compose run --rm -T …`: without `-T` the report gets CR line endings;
  - the import directory is readable by uid 1000 (`install -d -o 1000 -g 1000 -m 700`);
  - the container's `/tmp` (the tmpfs of the compose file) must hold a copy of the legacy
    database and its `-wal` during the import; `TMPDIR` moves the copy elsewhere.
  - Point at `docs/phase2/cutover.md` for the cutover itself, and align its compose directory and
    paths (`/srv/bookclub/import`, `/srv/bookclub/cutover`) with DEP's README after the merge.
- `docs/phase1/notes/imp.md` "Manual step" step 1 still says `app.backup`; that keeps working,
  and stopping the app and copying `bookclub.db` (plus `-wal`) works too now. The cutover uses the
  latter (`cutover.md` §3.1).
- `CLAUDE.md`: no change needed. If wanted, one line under the server checks: `cutover_counts.rs`
  reads `deploy/cutover/*.sql`.
- No new route, DTO, `error::msg` string, env variable (only the standard `TMPDIR`), CLI command
  or migration.

## Not done here (by instruction)

- No branch or tag pushed, no Forgejo issue touched: Q6 (`legacy-python`, commands in
  `cutover.md` §6) and D9 (#27, #29 now; #34, #35 at the cutover) wait for Rafael and the
  integrator.
- CUT-R (`cutover/remove-python`) is not opened; it comes after Wave 1 is in `main`.
- For CUT-R's grep (`backend/|uvicorn|pytest|app\.backup`): `server/src/importer/source.rs` (doc
  comment on `app.backup` copies), `server/tests/importer_run.rs` (a test comment on
  `python -m app.backup`), `server/tests/fixtures/legacy/README.md` and `generate.py` mention them
  as history. `deploy/cutover/` and `deploy/legacy-origin/` stay until after the cutover.

## For CUT2 (rehearsal)

- Use `docs/phase2/cutover.md` §3.3–3.4 on the rehearsal copy, with `compare.sh --report` and a
  psql command for port 5456, e.g. `-- psql postgres://bookclub:bookclub@127.0.0.1:5456/bookclub`
  (or `-- docker exec -i bookclub-pg-rehearsal psql -U bookclub -d bookclub`). The agents VM has
  no `sqlite3`: run compare.sh in a container that has it (e.g. `debian:trixie-slim` with
  `sqlite3 postgresql-client`, `--network host`), or set `SQLITE3` to a wrapper.
- Record the slug, the run time and the lookup wait in #38 for the cutover window.

## Verified by hand

- `compare.sh` (Debian trixie, mawk 1.3.4, sqlite3, psql 17) against a binary `import-legacy
  --offline` of `fresh.sql` on the CUT Postgres: every count matches the report, all 5 accounts
  match, exit 0. A changed report line → MISMATCH rows, exit 1. A changed hash → `< 4|lena` /
  `> 4|lena` only, exit 1. A dry-run report, a file that is no report, a wrong database and no
  arguments → exit 2 with a message. A WAL database with an uncheckpointed user on a `:ro` mount →
  legacy users 6, the extra account listed. On the host with `-- docker exec -i
  bookclub-pg-cut psql …` (the shape of `docker compose exec -T postgres psql`) → exit 0.
- `shellcheck` (koalaman/shellcheck:v0.10.0) on `compare.sh`: clean.
- `caddy validate` and `caddy fmt` (caddy:2.10) on `Caddyfile.example`: valid, already formatted.
- The block on `agents.lan:8085` (`BOOKCLUB_OLD_HOST=http://:8085`): `/shelf?x=1` → 301
  `https://bookclub.example/shelf?x=1`; `/`, `/works/OL45883W?tab=notes&x=%C3%A4`,
  `/api/auth/me` and a POST keep path and query; `/sw.js` and `/sw.js?v=2` → 200,
  `text/javascript; charset=utf-8`, `Cache-Control: no-store`.
- The worker in headless Chromium on `localhost:8085` (a secure context): the old app's `sw.js`
  from 3d0b27d installed and controlling with `bookclub-shell-v1`; after switching Caddy to the
  legacy-origin block, one navigation to `/shelf?x=1` landed on `https://example.com/shelf?x=1`,
  and the old origin then had no registration, no controller and no caches.
