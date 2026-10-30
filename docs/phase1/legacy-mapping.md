# Legacy mapping: SQLite (Python app) → Postgres (Rust server)

This file maps every legacy table and column to its new home, or marks it **DROPPED** with the reason. It also specifies what the importer (#38, `bookclub-server import-legacy`) must transform. The target schema is `server/migrations/0001_init.sql`.

Sources: `backend/app/models.py`, `COLUMN_MIGRATIONS` and `import_legacy_pick_posts` in `backend/app/db.py`, and the specs in `docs/phase1/spec/` (auth, catalog, google, shelf, diary, club, platform).

## 0. Importer overview

### Command

```
bookclub-server import-legacy <bookclub.db>
    --owner <username>          # becomes club owner; everyone else becomes a member
    --club-name <name>          # legacy BOOKCLUB_NAME, e.g. "Thursday Readers"
    --timezone <IANA zone>      # legacy BOOKCLUB_TZ; used for the club and for every imported user
    [--club-slug <slug>]        # default: slugified club name + "-" + 4 random characters
    [--dry-run]                 # do everything inside one transaction, print the report, ROLLBACK
    [--offline]                 # no DNB/Open Library lookups; ISBN rows are kept as bare facts (rematch later)
    [--allow-unmatched]         # an unavailable source does not abort; those rows are kept as bare facts
```

`bookclub-server rematch-legacy [--dry-run]` re-resolves every `source='legacy'` edition that has an ISBN through `CatalogLookup::resolve_isbn_remote` and lets ingest upgrade it under the provenance rules (§3.3), then runs the cover search for the imported books still without a cover (§3.5). Rafael runs it after an `--offline` or `--allow-unmatched` import, after an import whose cover search stopped, and whenever he wants to retry.

`DATABASE_URL` and `MEDIA_DIR` come from the environment, as for `serve`.

### Guarantees

- **Refuses a non-empty target.** If `users` has any row, the importer aborts before writing anything, with exit code 2 and `target database already has users`. A catalog filled by the DNB harvester is fine, because work ids are not preserved.
- **Atomic.** The whole import runs in **one Postgres transaction**. A failure rolls back everything. Avatar files written to the media store before a failure stay behind as orphans; they are harmless, and the report lists them.
- **Dry run.** It runs the same code path inside the same kind of transaction, then rolls back.
  - Media bytes are **not** written, only counted.
  - Network lookups **do** run unless `--offline` is given. The lookups only read the network, and only then is the "matched / unmatched" report accurate.
- **Lookup errors** (an offline CLI can afford to wait):
  - `LookupError::Throttled`: wait out the host cooldown (`Retry-After` or 60 s; a hidden `--retry-wait-ms` flag sets it, and tests set 0) and retry the same ISBN, indefinitely; the report shows the total wait.
  - `LookupError::Unavailable`: abort the run (exit 1, nothing written), unless `--allow-unmatched` is given; then the row is kept as bare facts and counted as `isbn_error`, separately from `isbn_unmatched`.
  - Because a throttled lookup is retried rather than counted as a miss, the dry run and the real run resolve the same ISBNs the same way unless an upstream record changed in between. The report lists every ISBN with its outcome (matched work id / unmatched / error), so the two runs can be diffed.
- **Read-only source.** The importer never opens the legacy file itself (CUT #52, `importer/source.rs`): it resolves a symlink, copies the file and, when they exist, its `-wal` and `-journal` into a private directory under `$TMPDIR` (0700), opens that copy with `SQLITE_OPEN_READ_ONLY` through `rusqlite` (bundled) and removes it afterwards. So a WAL database on a read-only mount imports with its committed `-wal` rows; `immutable=1` is never used (it ignores them). A hot journal is refused with its reason.
- **Both legacy schema shapes are accepted** (docs/phase1/spec/platform.md §2.2): a fresh `create_all` DB, and a DB migrated over time with `ALTER TABLE`.
  - Columns from `COLUMN_MIGRATIONS` may be missing in a very old DB. They default exactly like the DDL there: theme `paper`, color_mode `system`, locale `en`, notify_* true, `subjects '[]'`, `description ''`, `take`/`dnf_reason ''`, everything else NULL.
  - `shelf.take` and `shelf.dnf_reason` may be NULL in migrated DBs; COALESCE them to `''`.
  - Presence is detected with `PRAGMA table_info`.
- **Timestamps.** SQLite DATETIME text (`YYYY-MM-DD HH:MM:SS` with an optional `.ffffff`) is naive UTC. It is parsed as UTC into `timestamptz`, and both forms must parse. An unparsable value aborts with the table, id and column.
- **Ids are preserved** for users, shelf entries, book_posts (as posts), quotes, club picks, milestones, votes, nominations, events, search history and invites.
  - The rows are inserted with explicit ids, then each identity sequence is advanced with `SELECT setval(pg_get_serial_sequence('<table>', 'id'), max(id))`.
  - This keeps `layout_json` shelf ids, `#entry-<id>` links and the id-ordered feeds valid.
  - Work ids are **not** preserved (§3).
- **Insert order** follows foreign keys: users → media (avatars) → club → memberships → invites → works/editions → shelf → layouts → favourites → picks → milestones → posts from `book_posts` (explicit ids) → **`setval` on posts** → posts from unmirrored `club_pick_posts` (default ids, §9.2) → reactions → quotes → votes (**with `winner_nomination_id` NULL**) → nominations (merged per §3.4) → **`UPDATE next_up_votes SET winner_nomination_id`** (the re-pointed winner) → ballots → events → search history → `setval` on every other kept-id table.
  - `next_up_votes_winner_fk` is an immediate composite FK to nominations, so a vote cannot carry its winner before the nominations exist.
  - `posts.id` is `GENERATED BY DEFAULT`; a default-id insert before the `setval` would take ids from 1 and collide with the kept `book_posts` ids.
- **The deferred club-owner check** (`clubs_have_owner`) runs at COMMIT: the club and its owner membership are inserted in the same transaction, so it passes.

### Normalising values the target refuses

The legacy columns have no length or range limits; the target has CHECKs. The importer never lets a CHECK abort the one-transaction import on a value it can repair. Every repair is counted per table and column in the report, so the dry run on the production copy shows them:

| Value | Repair |
|---|---|
| `books.cover_id` ≤ 0 (Open Library work JSON often has `covers[0] = -1`; the legacy app treated ≤ 0 as no cover, `serialize.cover_url`) | no cover |
| `books.pages` ≤ 0 (Google `pageCount` 0, an OL median of 0) or > 100000 | NULL |
| `books.year` outside -5000..3000 | NULL |
| text over its target limit (`title` 1000, `authors` 2000, `description` 50000, `posts.body`/`quotes.body` 1000, `take` 140, `dnf_reason` 200, `note` 280, milestone `title` 120, `search_history.query` 300) | truncated by characters (`text::truncate_chars`) |
| empty title (after trimming) | `Untitled` |
| `subjects` over 50 entries | the first 50 |
| `quotes.page` outside 0..20000, `spoiler_upto`/`progress_at`/`progress` outside 0..100 | NULL |

Only values that identify something abort the import, with the table, id and column: a username outside `^[a-z0-9_]{2,32}$`, an unknown shelf status, event kind or reaction emoji, and an unparsable timestamp.

### Report

The importer prints the report at the end of both a dry run and a real run. It has one line per entity: `source rows → target rows`, with merged, dropped and normalised counts. For example:

```
users                 3 → 3
avatars               1 → 1 media (jpeg, legacy)
club                  - → 1 "Thursday Readers" (thursday-readers-k3x9), owner demo, 2 members
invites               2 → 2 (1 used, 1 unused imported as revoked)
books                 11 → 10 works (OL 1, user 8, isbn matched 1, isbn unmatched 0, isbn invalid 0, google volume 1, other 0; 1 merged)
shelf                 18 → 18 (0 merged, 2 fields normalised)
...
push_subscriptions    2 → DROPPED (Phase 4)
```

The real run has to show the same numbers as the dry run (see "Lookup errors" above for the one way they can differ, and how to see it). If a book merge collapsed duplicate rows, the report says so per table. Merges and the documented skips (§9.2) are the only ways target counts can differ from source counts (#38 AC, "matching counts for every entity"), and each is listed.

**The cover search** (COVER #74, §3.5) adds a `cover search` line after `lookup wait` and a `without a cover` section after `isbn keys`:

```
cover search           20 searched: 12 attached, 8 still without a cover (3 suggested, 4 no match, 1 not searched); 2 books readers added left as they are; 9 imported cover ids not fetched yet (checked once the server runs)
...
without a cover:
  book 12 "<title>" by <authors>: suggested, review it on the admin page
  book 14 "<title>" by <authors>: no match on Open Library
  book 15 "<title>" by <authors>: not searched (Open Library unavailable)
  book 16 "<title>" by <authors>: not searched (stopped after an outage)
```

- "still without a cover" is counted in the database at the end of the step (`bc_cover_missing` over the imported works), not added up. `uncertain` and `blocked` outcomes count as "no match". "Not searched" names its cause: Open Library unavailable, Open Library busy (the throttle waits gave up), stopped after an outage, the time budget, or a database error.
- "imported cover ids not fetched yet" counts the imported works whose cover is an Open Library picture without bytes, those the search attached included: the importer has no media service, so the server's prefetch and dead check take them after the start.
- The section names the legacy book id (stable between the dry and the real run; work ids are not) and the title, like `isbn keys`: **only in the private report file**, never in #38.
- With `--offline`: `cover search           skipped (--offline): 20 without a cover; rematch-legacy searches them`.
- **Not an entity line:** its name has a space and it has no `→`, so `compare.sh` and `cutover_counts.rs` ignore it. The counts depend on Open Library's answers, not on the legacy data, so the dry and the real run may differ in this line and section (an answer, a throttle, an outage or the time budget at another book).
- `rematch-legacy` prints the same line and section for its pass. A dry rematch also prints a second header line under `DRY RUN` (`importer::REMATCH_DRY_RUN_COVERS`): its cover pass sees the books as they were before the ISBN pass, whose upgrades it rolled back, so it may search books the real run will not.

### Test fixtures (#38 AC)

- **Committed as SQL text, never as a binary DB:**
  - `server/tests/fixtures/legacy/fresh.sql`: a `sqlite3 .dump` of a database produced by the Python app.
  - `server/tests/fixtures/legacy/migrated.sql`: the hand-written `LEGACY_SCHEMA` from `test_migrations.py`, booted once through `init_db`, so it has nullable `take` and timestamps without fractions.
- **Loading:** the importer tests load them into a temp file with `rusqlite::Connection::execute_batch`. CI therefore needs no Python.
- **Generator:** `server/tests/fixtures/legacy/generate.py`. It runs against `backend/app` (`seed_demo` with `DEMO_DB` overridden, plus the extra rows docs/phase1/spec/platform.md §5.8 lists), uses fixed timestamps and embeds one **known argon2 hash** of a known password. It is run by hand whenever the fixture needs to change. The extra rows also cover every normalisation above: a book with `cover_id = -1` (in `migrated.sql` too), a Google ISBN row with `pages = 0`, a title over 1000 characters, a year of 99999, a diary entry stamped with the current pick, a mirrored pick note whose `book_posts` copy was deleted, a reaction removed after mirroring, an edited quote, and a `reaction` event.
- **No Google data:** the fixture's Google-shaped rows are synthetic (invented volume ids, a made-up `cover_image_url` and description). The #41 AC "no Google Books code or data in `server/`" is checked by `tests/catalog_no_google.rs` (CAT), which greps `server/src` and `server/tests` for `googleapis` and `books.google` and excludes exactly `server/src/importer/**` (code that recognises and drops Google fields) and `server/tests/fixtures/legacy/**` (the synthetic fixture).
- **The production database is never committed.** The dry run and the real run on a copy of it are manual steps (#38 may stay open until Rafael provides the copy).

---

## 1. `users` (personal; global account)

| Legacy column | New | Transform |
|---|---|---|
| `id` | `users.id` | kept |
| `username` | `users.username` | Copied. It must match `^[a-z0-9_]{2,32}$`; any other value aborts the import and lists the offending rows. |
| `password_hash` | `users.password_hash` | **Copied byte for byte.** argon2-cffi PHC strings verify in Rust with their stored parameters. Junk values such as test fixtures' `hash` are copied too; they simply never verify. |
| `theme` | `users.theme` | A value outside `paper/slate/forest/ink`, or a missing column, becomes `paper`. |
| `color_mode` | `users.color_mode` | A value outside `light/dark/system` becomes `system`. |
| `locale` | `users.locale` | A value outside `en/de` becomes `en`. |
| `created_at` | `users.created_at`, and also `password_changed_at` | Naive UTC is parsed as UTC. |
| `notify_meeting`, `notify_pick`, `notify_note` | same names | Copied (0/1 to bool). They are kept for Phase 4 push and are unused in Phase 1. |
| `avatar` (BLOB) | new `media` row + `users.avatar_media_id` | See §1.1. |
| `avatar_mime` | `media.content_type` | Always `image/jpeg` in practice. Anything else (or NULL with a blob present) is detected from the magic bytes; an unknown type gets no avatar, and the report counts it. |
| `avatar_updated_at` | `media.created_at` | When the blob is present but `avatar_updated_at` is NULL, Python showed no avatar (`avatar_url` was null). The importer still imports it, with `created_at` = import time: the user did upload it. |

New columns:
- `email` and `email_verified_at` are NULL. The app shows a one-time, skippable "add your email" prompt (`email_prompt_dismissed_at` is NULL).
- `timezone` = `--timezone`.
- `is_admin` = false. Rafael runs `bookclub-server make-admin <username>` afterwards.

### 1.1 Avatar blobs to the media store

- For each user with a non-empty `avatar` blob, generate a `public_id` (`media::shared::urls::new_public_id`), write the bytes unchanged under `object_key("avatar", public_id, "a")` (= `avatars/<pp>/<public_id>/a`), and insert the row with `media::shared::register`:
  - `kind='avatar'`, `source='legacy'`, `content_type='image/jpeg'`, `variants='{a}'`
  - `width=256`, `height=256` (read from the JPEG header, not assumed)
  - `byte_size`, `sha256`, `uploaded_by=user`, `created_at` = `avatar_updated_at` (`NewMedia.created_at`)
- The blobs are already 256×256 JPEGs with no metadata, so they are not re-encoded: no quality loss, and no dependency on the media package's image pipeline.
- They are served by the same `/media/{public_id}/a` route with the stored content type.
- In a dry run, no bytes are written; the report only counts them.

## 2. `invites` → `club_invites` + `club_invite_uses` (club-scoped)

| Legacy column | New | Transform |
|---|---|---|
| `id` | `club_invites.id` | kept |
| `code` | `club_invites.code` | Copied verbatim (already normalised uppercase). Codes longer than 32 characters (a bootstrap code) are allowed. |
| `created_by_id` | `club_invites.created_by` | NULL stays NULL: the bootstrap invite and CLI-minted invites. |
| `used_by_id` | `club_invite_uses.user_id` + `club_memberships.invited_by` | A used invite gets one `club_invite_uses` row, with `use_count = 1`. The redeemer's membership gets `invited_by = created_by_id`. |
| `created_at` | `club_invites.created_at` | |
| `used_at` | `club_invite_uses.used_at` | When NULL on a used invite: `users.created_at` of the redeemer. |

- Every imported invite gets `club_id` = the imported club, `max_uses = 1` and `expires_at = NULL`.
- **Unused invites are imported as revoked** (`revoked_at` = import time). They keep their history (who minted what), but they cannot be redeemed: the old domain and its links go away, and under the new roles members may no longer mint invites.

## 3. `books` → `works` + `editions` + `media` + `legacy_work_keys` (global catalog)

### 3.1 Classification

Each row is classified by its key. The id is `canonical_work_id(work_id_from_key(ol_work_key))`, which is the last path segment with `isbn`/`x` uppercased as in `works.py`.

| Class | Rule | Result |
|---|---|---|
| `OL` | `^OL\d+W$` | A work with `ol_work_id` and `source='openlibrary'` |
| `BC` | `^BC[a-f0-9]{10}$` | A user-added work: `source='user'`, `created_by` NULL (the legacy app recorded no creator), fields with provenance `user` |
| `ISBN` | `^ISBN(\d{9}[\dX]\|\d{13})$` | Re-matched by ISBN (§3.3) |
| `GB` | `^GB[A-Za-z0-9_-]{1,40}$` | Bare facts, `source='legacy'`, no edition. The key is stored hashed (`sha256:<hex>`, see `legacy_work_keys.legacy_key`): no Google volume id is kept. |
| `OTHER` | anything else under `/works/` (the loose prefix check let these in) | Bare facts, `source='legacy'`, no edition |

- **Processing order:** OL rows first, then BC, then ISBN, then GB and OTHER. A later row whose match resolves to a work created earlier is **merged** into it.
- **Canonicalisation collisions** (two rows whose keys canonicalise to one key, such as an `x`/`X` ISBN pair) also merge.

### 3.2 Columns

| Legacy column | OL and BC rows | ISBN (matched) | ISBN (unmatched), GB, OTHER |
|---|---|---|---|
| `id` | `legacy_work_keys.legacy_book_id` (work ids are new) | same | same |
| `ol_work_key` | `legacy_work_keys.legacy_key`; for OL also `works.ol_work_id` | `legacy_key` | `legacy_key` (hashed for GB) |
| `title` | `works.title` (empty → `Untitled`; over 1000 characters → truncated) | not copied; the matched record wins | `works.title` (as for OL) |
| `authors` | `works.author_names` (verbatim, truncated to 2000; no author records, since splitting on `", "` is lossy) | not copied | `works.author_names` |
| `cover_id` | a value > 0: `media` row through `ingest::ol_cover_media` (`source='openlibrary'`) + `works.cover_media_id`; bytes are fetched lazily. **≤ 0 or NULL: no cover** (§0 normalisation) | kept (> 0 only) if the matched work has no cover | as for OL (an OL cover id is CC0-sourced, not Google data) |
| `cover_image_url` | **DROPPED** (Google host, Google terms) | **DROPPED** | **DROPPED** |
| `spine_color` | `media.dominant_color` of the cover row. Reset to NULL when `cover_image_url` was set, because the colour came from the Google image. `''` is kept as `''`. | not copied | as for OL; NULL if there is no cover row |
| `year` (NULL outside -5000..3000) | `works.first_publish_year` | fills the matched edition's `publication_year` and the work's `first_publish_year` **only where they are NULL**, with provenance `legacy` (any later source overwrites it) | unmatched ISBN rows: `editions.publication_year` **and** `works.first_publish_year` (the work is built from the edition facts, conventions.md §7.3), so `BookOut.year` is unchanged. GB/OTHER: `works.first_publish_year` (no edition exists). |
| `description` | OL: `works.description`. BC: `works.description` (entered by a club member) | **DROPPED** (Google) | **DROPPED** (any non-empty value on ISBN/GB rows is Google's); OTHER: kept |
| `pages` (NULL when ≤ 0 or > 100000) | `works.pages` | fills the matched edition's `pages` and the work's `pages` **only where they are NULL**, provenance `legacy` | `editions.pages` and `works.pages` for unmatched ISBN rows; GB/OTHER: `works.pages` |
| `subjects` (JSON text) | `works.subjects` (parsed; invalid JSON or a non-list becomes `{}`; items are stringified) | **DROPPED** | **DROPPED** (Google categories) |
| `ol_rating` | OL: `works.ol_rating`. BC: NULL | **DROPPED** (Google `averageRating`) | **DROPPED** |
| `details_fetched_at` | OL: `works.details_fetched_at` (so imported OL works are not re-fetched on first view) | – | NULL |

### 3.3 ISBN re-matching (#38, docs/phase1/spec/google.md §3.4)

1. `raw = id[4:]`.
   - A 10-character value must pass the ISBN-10 checksum and is converted to 978-ISBN-13.
   - A 13-character value must pass the ISBN-13 checksum and start with 978 or 979.
   - Anything invalid is classed `isbn_invalid`: a bare-facts work with **no** edition ISBN. The raw value goes into the report.
2. Resolve with `CatalogLookup::resolve_isbn(conn, isbn13)`. The chain is local editions (including works harvested from DNB), then by registration group: `978-3` → DNB SRU → Open Library, every other group → Open Library only (conventions.md §7.4).
   - With `--offline`, only the local catalog is checked.
   - Errors: Throttled is waited out and retried; Unavailable aborts unless `--allow-unmatched` (§0).
   - A hit means `match='isbn_matched'`, and the legacy row maps onto that work. `legacy_work_keys.edition_id` is the resolved edition. The legacy `year` and `pages` fill only the NULL fields of that edition and work, with provenance `legacy` (§3.2).
   - A miss means `match='isbn_unmatched'`. The importer calls `ingest::upsert_edition_with_work(work, edition, WorkGrouping::NewWork, Mode::Refresh)` with `Source::Legacy` records: the work (title, `author_names`, `first_publish_year`, `pages`) and the edition (`isbn13`, `pages`, `publication_year`). `isbn10` is derived, never stored.
3. **Shelf entries and quotes of an ISBN-matched book get `edition_id` = `legacy_work_keys.edition_id`.** The legacy row identified exactly that edition by its ISBN, so its page count, cover and pagination stay what the user saw. (Unmatched rows have a single edition; their entries get it too.)
4. **GB rows are never fuzzy-matched by title or author.** They keep bare facts, and anyone can propose a merge through `POST /api/books/works/{work_id}/merge-requests`. The plan only asks for ISBN re-matching, and fuzzy matching would need a review loop. The cover search (§3.5) does search Open Library by title and author for such a book, but only for a **picture**: it never changes a work's facts, identity, `source` or `ol_work_id`, and never merges anything.
5. **Re-matching later.** Unmatched and error rows are not stuck: `rematch-legacy` (§0) calls `resolve_isbn_remote` for every `source='legacy'` edition with an ISBN. A hit is persisted through ingest, which finds the bare-facts edition by its ISBN and overwrites its `legacy`-ranked fields (and its work's title and authors) with the source record; the work's `source` becomes `dnb` or `openlibrary`, and `legacy_work_keys.match` becomes `isbn_matched`. Shelf entries keep pointing at the same work and edition. (When the source record belongs to a work that already exists, ingest's lookup by ISBN still returns the bare-facts edition, so nothing is duplicated; the two works can then be merged through the queue.)

### 3.4 Merge rules when several legacy books become one work

The rules are the pure functions of `catalog::shared::merge_rules` (conventions.md §7.6), which CAT's live `merge_works` uses too, so an import merge and an admin merge behave the same.

| Table | Collision | Rule |
|---|---|---|
| `shelf` | UNIQUE (user, work) | The winner is the entry with the highest status rank (finished > did_not_finish > currently_reading > want_to_read). Ties go to the newest `updated_at`, then the lowest id. Where the winner's `rating`, `take`, `dnf_reason`, `progress`, `started_at`, `finished_at` or `edition_id` is NULL or `''` and the loser's value is allowed for the winner's status, the loser's value is copied. The loser's id goes into the layout remap (§5). Positions are renumbered per (user, status). |
| `user_favorites` | (user, work) and (user, position) | Keep the lower position, drop the other, then compact to 1..n. |
| `next_up_nominations` | UNIQUE (vote, work) | Keep the earliest nomination and move the loser's ballots to it (ballots are one per user per vote, so no new collision can arise). A `winner_nomination_id` that pointed at the loser is re-pointed; the votes' winners are written in the `UPDATE` after the nominations (§0 insert order). |
| `club_picks`, `posts`, `quotes`, `events` | none | `work_id` is re-mapped. |

Every merge is listed in the report.

### 3.5 Cover search for imported books (COVER #74)

Google covers are dropped (§3.2), so an ISBN row unknown to the DNB and Open Library, and every GB or OTHER row without an Open Library `cover_id`, arrives as bare facts without a cover (in the #38 rehearsal, 15 of 51 legacy books were Google ISBN rows unknown to both). `import-legacy` therefore searches Open Library for a picture right after the books step, inside its transaction, for every imported work that is a cover-search candidate (conventions.md §7.3: not a reader's own `BC…` book, no cover), in legacy book id order. Each book runs in a savepoint: a database error in one book's step rolls back only that step and reports the book "not searched (database error)". The first unavailable answer, the first book whose throttle waits give up (3 waits of `--retry-wait-ms`, counted in `lookup wait`) and the 5-minute budget stop the search; the books after it are "not searched" and have no `cover_search` row, so the server's job takes them first after the start. Nothing in the cover step makes the import exit 1, and `--offline` skips it. A clear match puts the cover on the work with provenance `openlibrary`; an uncertain one becomes a suggestion on the admin page (Admin → Covers). The dry run reports what the real run will do unless Open Library answers, throttles or fails differently in between. `rematch-legacy` searches the imported books still without a cover after its ISBN pass, 2 s apart, each step committing on its own. The report lines are in §0 "Report".

## 4. `shelf` → `shelf_entries` (personal)

| Legacy column | New | Transform |
|---|---|---|
| `id` | `id` | kept (layout JSON references it) |
| `user_id` | `user_id` | |
| `book_id` | `work_id` | through the book→work map (§3) |
| – | `edition_id` | `legacy_work_keys.edition_id` for ISBN-keyed books (matched or unmatched, §3.3); NULL otherwise |
| `status` | `status` | Copied. An unknown value aborts. |
| `position` | `position` | Copied, then renumbered densely 0..n-1 per (user, status) in `(position, id)` order. That keeps the order and repairs gaps. |
| `shelf_position` | `shelf_position` | copied (NULL = not arranged) |
| `rating` | `rating` | Kept only when status = finished and the value is 1..5; otherwise NULL. Each normalisation is counted. |
| – | `rating_visibility` | `'audience'` (everyone imported shares the club, as before) |
| `take` | `take` | COALESCE `''`. Kept only for finished entries. Over 140 characters aborts (impossible through the API). |
| `dnf_reason` | `dnf_reason` | COALESCE `''`. Kept only for DNF entries. |
| `progress` | `progress` | Kept only for currently_reading and 0..100. |
| `updated_at`, `started_at`, `finished_at` | same | NULL stays NULL |

## 5. `physical_shelf_layouts` (personal)

| Legacy column | New | Transform |
|---|---|---|
| `user_id` | `user_id` | |
| `layout_json` (TEXT) | `layout` (jsonb) | Parse it, then normalise it with the same lenient reader the GET endpoint uses: fill `lights=false`, `decorations={}` and spine `display="spine"`, lowercase colours. Re-map the ids of shelf entries merged away to their winner; if the winner is already placed, drop the loser. Drop ids that do not belong to the user's imported entries. Invalid JSON becomes the default layout, and the report counts it. Shelf entry ids are otherwise unchanged (§0). |
| – | `updated_at` | import time |

## 6. `user_favorites` (personal)

| Legacy column | New | Transform |
|---|---|---|
| `id` | **DROPPED** | Never exposed. Order is `(position)`, unique per user. |
| `user_id` | `user_id` | |
| `book_id` | `work_id` | through the map; merge rule in §3.4 |
| `position` | `position` | Compacted to 1..n per user (also repairs the legacy "gap" bug). |

## 7. `club_picks` → `club_picks` (club)

| Legacy column | New | Transform |
|---|---|---|
| `id` | `id` | kept |
| – | `club_id` | the imported club |
| `book_id` | `work_id` | through the map |
| `set_by_id` | `set_by` | |
| `note` | `note` | |
| `started_at`, `ended_at`, `meeting_at` | same | If several picks are open (`ended_at IS NULL`), all but the highest id get `ended_at` = the next pick's `started_at` (or import time), and the report counts them. This satisfies the "one current pick" index. |
| `reminder_sent_at` | same | Kept, so Phase 4 never re-sends a reminder. |

## 8. `pick_milestones` → `milestones` (club)

| Legacy column | New | Transform |
|---|---|---|
| `id` | `id` | kept |
| – | `club_id` | the imported club |
| `pick_id` | `pick_id` | |
| `title` | `title` | copied (≤ 120 characters already) |
| `chapter_from` | `range_from` | `unit='chapter'` when either chapter is non-NULL; with both NULL, `unit` is NULL (a title-only checkpoint). |
| `chapter_to` | `range_to` | A backwards range (legacy PATCH allowed one) is **swapped**, and the report counts it. |
| `due_at` | `due_at` | |
| `position` | `position` | Renumbered densely 0..n-1 per pick in `(position, id)` order. |
| `created_by_id` | `created_by` | |
| `created_at` | `created_at` | |

## 9. Posts: `book_posts`, `club_pick_posts` → `posts` (one table, two scopes)

**Decision.**
- Diary entries are personal: `posts.club_id IS NULL`, visible to the author's audience, optionally only-me. They follow their author: if the author leaves the club, the entries stay theirs and stay visible to their audience; the author can set them to only-me.
- Pick discussions are club data: `posts.club_id` and `pick_id` are set, and only members can see them. They stay with the club.
- Python already merged every legacy pick post into `book_posts` (`import_legacy_pick_posts`), marking each copy with `legacy_post_id`. Separately, the legacy app stamped `pick_id` on **every** diary entry written while its book was the current pick (`diary.py` `add_entry`), as a context marker that no endpoint reads back (docs/phase1/spec/diary.md: "Entries hang off a book, not a pick").

**Import rule:** only the real pick posts become club posts.
- A `book_posts` root **with `legacy_post_id` set** (the mirror copy of a legacy pick note) becomes a **club post** of the imported club, with `pick_id` = its `pick_id`, together with all its replies.
- An unmirrored `club_pick_posts` row that is genuinely new (§9.2) becomes a club post.
- **Every other root**, including diary entries stamped with the current pick's `pick_id`, becomes a **diary entry** with visibility `audience`, together with its replies. The stamped `pick_id` is dropped (the report counts it); it never decided who could see the entry.

This is lossless and adds no duplicates:
- Every imported user is a member of the imported club, and diary entries are visible to the author's audience, so everyone still sees everything they saw before, on the book page and in the club's `GET /api/diary?club=` feed (which includes diary entries by current members).
- A member who later leaves keeps their diary, and owner-side club deletion removes only the pick discussion, not people's diaries.
- Club discussion stays inside the club, even for future friends or members of other clubs.

### 9.1 `book_posts` → `posts`

| Legacy column | New | Transform |
|---|---|---|
| `id` | `id` | kept (`#entry-<id>` links, feed order) |
| `book_id` | `work_id` | through the map |
| `author_id` | `author_id` | |
| `parent_id` | `parent_id` | Kept. The importer checks the invariants: the parent is top-level and on the same book. A violating row is re-attached as a root, and the report counts it. A reply takes its root's scope, so `posts_parent_fk` (same work, same scope) holds. |
| `body` | `body` | |
| `spoiler_upto`, `progress_at`, `status_at` | same | |
| `pick_id` | `club_id` + `pick_id` | Only for a root with `legacy_post_id` set and its replies (club scope, from the root's `pick_id`). On every other row it is dropped and counted (diary scope). |
| `legacy_post_id` | decides the scope (above) and sets `milestone_id` (§9.2), then **DROPPED** | It marks the mirror copy. |
| `deleted_at` | `deleted_at` | Tombstones are copied with body `''` and spoiler NULL. |
| `created_at`, `updated_at` | same | |
| – | `visibility` | `'audience'` |
| – | `milestone_id` | From `club_pick_posts.milestone_id` of the row named by `legacy_post_id`, if that milestone exists and is on the same pick. |

### 9.2 `club_pick_posts` and `post_reactions` (legacy pick notes)

| Legacy column | New | Transform |
|---|---|---|
| (mirrored rows: their id appears in `book_posts.legacy_post_id`) | **not imported again** | The `book_posts` copy is canonical: it is the row users have seen, edited, replied to and reacted on since the diary shipped. Only `milestone_id` is carried across. |
| (unmirrored rows with `id` ≤ max(`book_posts.legacy_post_id`)) | **skipped**, counted and listed by id in the report | In practice these are mirror copies that their author deleted in the diary: `delete_entry` hard-deletes a post without replies and leaves the `club_pick_posts` original behind (the legacy app itself would resurrect them on its next restart). Their reactions are skipped too, and `note_posted` events pointing at them lose `post_id` (§12). The frontend no longer calls `/api/pick/posts`, so no genuinely new note can have an id below a mirrored one. |
| (unmirrored rows with `id` > max(`book_posts.legacy_post_id`), or all rows when nothing was ever mirrored) | new `posts` row, club scope | Genuinely new (written through the legacy `/api/pick/posts` after the last boot). Exactly Python's mirror mapping: `work_id` = the pick's work, `club_id`/`pick_id`, `author_id`, `body`, `spoiler_upto`, `milestone_id`, `created_at`, `updated_at`; `progress_at`/`status_at` NULL. Inserted **after** the `setval` on posts (§0), so it gets a new id above every kept one; the id-ordered feeds therefore list these few notes as the newest, and the report says so. |
| `post_reactions.*` on a **mirrored** row | **DROPPED** for the mirror copy, which takes its reactions from `book_post_reactions` only | Legacy `post_reactions` is a snapshot taken at mirror time; the frontend toggles reactions only through `/api/posts/{id}/reactions`. A union would bring back every reaction a user removed after mirroring. The report counts the `post_reactions` rows that have no `book_post_reactions` twin. |
| `post_reactions.*` on an imported **unmirrored** row | `post_reactions` on the new row | copied, deduplicated on (post, user, emoji) |

### 9.3 `book_post_reactions` → `post_reactions`

| Legacy column | New | Transform |
|---|---|---|
| `id` | **DROPPED** | The primary key is (post, user, emoji). |
| `post_id`, `user_id`, `emoji`, `created_at` | same | An emoji outside the six allowed strings aborts (impossible through the API). |

## 10. `quotes` (personal)

| Legacy column | New | Transform |
|---|---|---|
| `id` | `id` | kept |
| `user_id` | `user_id` | |
| `book_id` | `work_id` | through the map |
| – | `edition_id` | `legacy_work_keys.edition_id` for ISBN-keyed books (§3.3); NULL otherwise (unknown edition) |
| `body` | `body` | |
| `page` | `page` | |
| `created_at` | `created_at` | `updated_at` NULL |
| – | `visibility` | `'audience'` |

## 11. Votes (club)

### `next_up_votes`

| Legacy column | New | Transform |
|---|---|---|
| `id` | `id` | kept |
| – | `club_id` | the imported club |
| `started_at`, `ended_at`, `closes_at` | same | Extra open votes are ended, as with picks. |
| `winner_nomination_id` | same, **written in a second pass** | Votes are inserted with NULL; after the nominations (and their merges) exist, `UPDATE next_up_votes SET winner_nomination_id = …` sets it (`next_up_votes_winner_fk` is immediate). It must point at a nomination of the same vote; otherwise it stays NULL, and the report counts it (the legacy column had no FK). Merges are re-pointed. |

### `next_up_nominations`

| Legacy column | New | Transform |
|---|---|---|
| `id` | `id` | kept |
| – | `club_id` | |
| `vote_id` | `vote_id` | |
| `book_id` | `work_id` | through the map; merge rule in §3.4 |
| `nominated_by_id` | `nominated_by` | |
| `created_at` | `created_at` | |

### `next_up_ballots`

| Legacy column | New | Transform |
|---|---|---|
| `id` | **DROPPED** | Never exposed. The primary key is (vote, user). |
| `vote_id`, `user_id`, `created_at` | same | plus `club_id` |
| `nomination_id` | `nomination_id` | re-pointed on a merge |

## 12. `events` → `events` (split scope)

| Legacy column | New | Transform |
|---|---|---|
| `id` | `id` | kept (feed order) |
| – | `club_id` | Club kinds (`member_joined`, `pick_set`, `pick_cleared`, `note_posted`, `reaction`, `nominated`, `voted`, `vote_closed`, `milestone_added`) get the imported club. Personal kinds (`shelf_*`, `progress`, `quote_added`) get NULL. |
| `actor_id` | `actor_id` | |
| `kind` | `kind` | An unknown kind aborts. |
| `book_id` | `work_id` | through the map |
| `pick_id` | `pick_id` | kept for club kinds; set to NULL on personal kinds |
| `payload` (TEXT) | `payload` (jsonb) | Parsed; invalid JSON or a non-object becomes `{}`. `note_posted` and `reaction`: `post_id` (a legacy `club_pick_posts` id) is re-mapped to the `posts` id of its mirror copy or new row; it is dropped when unknown or skipped (§9.2; rowid reuse made some wrong). `reaction`: the legacy `author` username is replaced by `author_id` (the user with that username; dropped when unknown), conventions.md §15. `quote_added`: `quote_id` is **added** by matching on (actor, book) and the **nearest `quotes.created_at` within 5 s of the event's `created_at`** (both were written in one request); `body[:80] == excerpt` only breaks ties, so an edited quote still matches; each quote matches at most one event. No match means the quote was deleted; the event is still imported, the read path hides it from everyone but its actor, and the report counts and lists these events. The seed quirk `shelf_added` with `rating` is kept as is. `shelf_finished`: after the import, every event with a `rating` or `take` whose actor's imported entry for the work is `only_me` or missing is sealed to its actor (`"private": true`; the legacy app redacted them only at read time; conventions.md §6.4; counted as `events.payload.rating_take`). |
| `created_at` | `created_at` | |

## 13. `search_history` (personal)

| Legacy column | New | Transform |
|---|---|---|
| `id` | `id` | kept |
| `user_id`, `query`, `last_used_at` | same | |
| `normalized_query` | same | Copied verbatim. It is already a Python casefold, which Rust's `caseless` reproduces. |

## 14. `push_subscriptions`: DROPPED

All columns (`id`, `user_id`, `endpoint`, `p256dh`, `auth`, `user_agent`, `created_at`) are dropped. They are bound to the old origin's VAPID key and service worker. Push returns in Phase 4 with native and Web Push device tokens (plan). The report counts them.

## 15. Not tables

| Legacy | Fate |
|---|---|
| `<data>/.secret_key` | DROPPED. Sessions are server-side; there is nothing to sign. |
| `<data>/.bootstrap_invite` | DROPPED. The bootstrap invite row is imported as revoked (§2). |
| `<data>/.vapid_private.pem` | DROPPED (Phase 4). |
| `BOOKCLUB_TZ` | `--timezone`: `clubs.timezone` and every imported `users.timezone`. |
| `BOOKCLUB_NAME` | `--club-name`: `clubs.name`. |
| `BOOKCLUB_THEME*`, `BOOKCLUB_PUBLIC_URL`, `BOOKCLUB_HTTPS`, `BOOKCLUB_TRUSTED_PROXIES`, `DEBUG`, `GOOGLE_BOOKS_API_KEY`, `VAPID_*` | DROPPED with self-hosting. Service-level replacements are listed in conventions.md §11. |
| Legacy session cookies | Not valid any more. Everyone logs in once after cutover (the domain changes anyway). |

## 16. Column checklist (every legacy column)

The checklist covers every table in `models.py`, including every column `COLUMN_MIGRATIONS` adds.

- **users:** id ✓, username ✓, password_hash ✓, theme ✓, color_mode ✓, locale ✓, created_at ✓, notify_meeting ✓, notify_pick ✓, notify_note ✓, avatar → media ✓, avatar_mime → media ✓, avatar_updated_at → media ✓
- **invites:** id ✓, code ✓, created_by_id ✓, used_by_id → uses ✓, created_at ✓, used_at → uses ✓
- **books:** id → legacy_work_keys ✓, ol_work_key → legacy_work_keys/ol_work_id ✓, title ✓, authors ✓, cover_id → media ✓, cover_image_url DROPPED ✓, spine_color → media.dominant_color ✓, year ✓, description ✓ (dropped for Google), pages ✓, subjects ✓ (dropped for Google), ol_rating ✓ (dropped for Google), details_fetched_at ✓
- **shelf:** id ✓, user_id ✓, book_id ✓, status ✓, position ✓, shelf_position ✓, rating ✓, take ✓, dnf_reason ✓, progress ✓, updated_at ✓, started_at ✓, finished_at ✓
- **physical_shelf_layouts:** user_id ✓, layout_json ✓
- **user_favorites:** id DROPPED ✓, user_id ✓, book_id ✓, position ✓
- **club_picks:** id ✓, book_id ✓, set_by_id ✓, note ✓, started_at ✓, ended_at ✓, meeting_at ✓, reminder_sent_at ✓
- **pick_milestones:** id ✓, pick_id ✓, title ✓, chapter_from ✓, chapter_to ✓, due_at ✓, position ✓, created_by_id ✓, created_at ✓
- **club_pick_posts:** id (mirror map) ✓, pick_id ✓, author_id ✓, body ✓, spoiler_upto ✓, milestone_id ✓, created_at ✓, updated_at ✓
- **post_reactions:** id DROPPED ✓, post_id ✓, user_id ✓, emoji ✓, created_at ✓
- **book_posts:** id ✓, book_id ✓, author_id ✓, parent_id ✓, body ✓, spoiler_upto ✓, progress_at ✓, status_at ✓, pick_id → club scope for mirror copies, dropped (counted) on diary entries ✓, legacy_post_id (decides scope, consumed) ✓, deleted_at ✓, created_at ✓, updated_at ✓
- **book_post_reactions:** id DROPPED ✓, post_id ✓, user_id ✓, emoji ✓, created_at ✓
- **events:** id ✓, actor_id ✓, kind ✓, book_id ✓, pick_id ✓, payload ✓, created_at ✓
- **push_subscriptions:** DROPPED ✓
- **quotes:** id ✓, user_id ✓, book_id ✓, body ✓, page ✓, created_at ✓
- **next_up_votes:** id ✓, started_at ✓, ended_at ✓, closes_at ✓, winner_nomination_id ✓
- **next_up_nominations:** id ✓, vote_id ✓, book_id ✓, nominated_by_id ✓, created_at ✓
- **next_up_ballots:** id DROPPED ✓, vote_id ✓, nomination_id ✓, user_id ✓, created_at ✓
- **search_history:** id ✓, user_id ✓, query ✓, normalized_query ✓, last_used_at ✓

New tables with no legacy source: `sessions`, `email_tokens`, `clubs`, `club_memberships` (built from users and `--owner`), `authors`, `work_authors`, `editions` (except unmatched ISBN rows), `edition_contributors`, `catalog_edit_requests`, `reports`, `harvest_state`, `import_jobs`.

## 17. Importer acceptance tests (#38)

1. `fresh.sql` imports, and per-entity target counts equal the source counts, apart from the merges and skips the report lists.
2. `migrated.sql` imports with the defaults filled: `take`/`dnf_reason` `''`, user prefs `paper`/`system`/`en`. This replaces `test_auth.py::test_preference_columns_backfill_pre_existing_users` and `test_migrations.py::test_legacy_database_is_upgraded_in_place`.
3. A legacy user logs in through `POST /api/auth/login` with the old password (the known argon2-cffi hash in the fixture).
4. A dry run prints the same report as the real run and leaves the target empty: `SELECT count(*) FROM users` is 0, and nothing is written under `MEDIA_DIR`.
5. A second run on the now non-empty target fails with exit code 2 and writes nothing (row counts are unchanged).
6. A merge fixture (an OL row plus an ISBN row that resolves to the same OL work through the FakeCatalog) produces one work, one merged shelf entry, a re-mapped layout and a merged nomination whose vote keeps its winner, and the report shows each merge.
7. Legacy pick notes: a mirrored note is not duplicated and keeps only its `book_post_reactions` (a reaction removed after mirroring stays removed); a genuinely new unmirrored note (id above every `legacy_post_id`) becomes a club post with a new id above every kept post id, with its `post_reactions`; an unmirrored note whose mirror copy was deleted (id below) is skipped and listed; a milestone link is carried across.
8. Scope: a diary entry stamped with the current pick's `pick_id` becomes a diary entry (club_id NULL, its replies too); a mirror copy becomes a club post with its replies.
9. Google rows: `cover_image_url`, description, subjects and rating are absent from the target, no GB volume id is stored (`legacy_work_keys` holds `sha256:` keys for them, and `GET /api/books/works/GB…` still resolves), and an ISBN-10 key is converted to ISBN-13.
10. Normalisation: `cover_id = -1` gives no cover, `pages = 0` gives NULL, a 99999 year gives NULL, an over-long title is truncated, and the report counts each; none of them aborts the import.
11. `BookOut` is unchanged for imported users: for every key class (OL, BC, ISBN matched, ISBN unmatched, GB, OTHER), each legacy book's `year` and `pages` as the legacy API showed them equal the imported `BookOut` (through the shelf entry, so with its edition), except where the matched record had its own value; those rows are listed in the report.
12. `layout_json` with a decoration, spines and a deleted entry id imports pruned and valid.
13. Event payloads: `note_posted` and `reaction` carry re-mapped post ids, `reaction` carries `author_id` instead of `author`, and `quote_added` has a `quote_id`, also for a quote edited after it was added.
14. Lookup errors: with the FakeCatalog throttling an ISBN twice and then answering, the row is matched; with it unavailable, the run aborts (exit 1, nothing written), and with `--allow-unmatched` the row is kept as bare facts and counted as `isbn_error`.
15. `rematch-legacy`: after an `--offline` import, programming the FakeCatalog with the ISBN and running `rematch-legacy` upgrades the bare-facts edition and its work (source becomes the source's, legacy-ranked title replaced), and the shelf entry still points at the same work and edition.
