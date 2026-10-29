# CUT2 (#38): rehearsal of the legacy import on a copy of the production database

Notes for the integrator. CUT2 changed no repo file; this file is its record. The production copy, the reports (`dry.txt`, `real.txt`), the compare output and the media never left `<rehearsal>` (mode 700) on the agents VM. They never entered a repo, a commit or a Forgejo comment. #38 gets only the counts below.

Placeholders: `<copy>` is the `app.backup` copy, `<rehearsal>` is a mode-700 directory next to it, `<worktree>` is `../bookclub-cut2`, `<owner>` is the username of the legacy user with id 1.

## Setup

- **Commit:** 8ecc528 (`phase-2` with Wave 1; Wave 2 does not change the importer), in a detached worktree. Release build: `SQLX_OFFLINE=true cargo build --release --locked` took 5 min 15 s with `jobs = 3` (rustc 1.99.1). SPA: `npm ci && npm run build`.
- **Target:** `bookclub-pg-rehearsal` (postgres:17, 17.11) on 127.0.0.1:5456, empty. `import-legacy` applied the migrations itself.
- **Source:** an `app.backup` copy (SQLite, WAL mode in the header, no `-wal` next to it, 2 users). Its sha256 was recorded before the runs and checked after them: unchanged, and no `-wal` or `-shm` appeared next to it. The importer read its private copy (CUT's WAL fix) and removed it afterwards.

## Assumptions (production facts deferred, work plan §4 "Answers (2026-10-29)")

- `--owner <owner>`: the username of the legacy user with id 1.
- `--club-name "Bookclub"`: the Python app's default `BOOKCLUB_NAME`.
- `--timezone Europe/Berlin`: both users have locale `de`. The Python default `BOOKCLUB_TZ` is `UTC`, so the production `.env` decides this value at the cutover.
- `--club-slug bookclub-8rqp`: `bookclub-` plus 4 random lowercase letters and digits, generated once (D5) and used for both runs. The cutover uses it too, unless the real `BOOKCLUB_NAME` is not "Bookclub". In that case Rafael decides before the announcement whether to keep it or generate a new one: the slug is the club's URL and never changes after the import.
- `--allow-unmatched` (D6).

## Commands

The environment for both runs and the server:

```bash
export DATABASE_URL=postgres://bookclub:bookclub@127.0.0.1:5456/bookclub
export MEDIA_DIR=<rehearsal>/media DEV_MODE=true
export TMPDIR=<rehearsal>/tmp   # the importer's private copy of the source stays in <rehearsal>
```

```bash
cd <rehearsal>
<worktree>/server/target/release/bookclub-server import-legacy <copy> --owner <owner> \
    --club-name "Bookclub" --timezone Europe/Berlin --club-slug bookclub-8rqp \
    --allow-unmatched --dry-run > dry.txt
# the same command without --dry-run > real.txt
diff <(tail -n +2 dry.txt) real.txt
```

The VM has neither `sqlite3` nor `psql`. `compare.sh` therefore ran unchanged in a throwaway container (sqlite3 3.46.1, psql 17.11); only the way it is called is adapted:

```bash
docker run --rm --network host \
    -v <worktree>/deploy/cutover:/cutover:ro -v <copy>:/legacy/bookclub-backup.db:ro \
    -v <rehearsal>:/rehearsal \
    -e DATABASE_URL=postgres://bookclub:bookclub@127.0.0.1:5456/bookclub \
    debian:trixie-slim bash -c '
        apt-get update -qq && apt-get install -y -qq --no-install-recommends sqlite3 postgresql-client &&
        setpriv --reuid=1000 --regid=1000 --clear-groups env HOME=/tmp TMPDIR=/rehearsal/tmp \
            bash /cutover/compare.sh --report /rehearsal/real.txt /legacy/bookclub-backup.db \
            > /rehearsal/compare.txt'
```

The server for the check:

```bash
cd <worktree>/server && DATABASE_URL=postgres://bookclub:bookclub@127.0.0.1:5456/bookclub \
    MEDIA_DIR=<rehearsal>/media DEV_MODE=true STATIC_DIR=../frontend/dist \
    BIND_ADDR=0.0.0.0:8080 PUBLIC_URL=http://agents.lan:8080 ./target/release/bookclub-server
```

## Results

| | dry run | real run |
|---|---|---|
| exit | 0 | 0 |
| wall time | 20.4 s | 20.3 s |
| lookup wait | 0.0 s | 0.0 s |

- **diff:** empty. `real.txt` is byte for byte `dry.txt` without its `DRY RUN` header.
- **Lookups:** each run made about 37 answered DNB and Open Library requests plus 17 Open Library 404s. Nothing was throttled.

| entity | source → target |
|---|---|
| users | 2 → 2 |
| avatars | 2 → 2 |
| club | - → 1 (`bookclub-8rqp`, 2 members) |
| invites | 3 → 3 (2 used, 1 unused imported as revoked) |
| books | 51 → 50 works (openlibrary 13, isbn matched 21, isbn unmatched 15, isbn error 0, isbn invalid 0, google volume 2, user 0, other 0; 1 merged) |
| editions | - → 36 |
| covers | 13 → 13 |
| shelf | 50 → 50 |
| physical_shelf_layouts | 2 → 2 |
| user_favorites | 6 → 6 |
| club_picks | 3 → 3 |
| pick_milestones | 0 → 0 |
| book_posts | 14 → 14 (14 diary entries, 0 club posts; 1 stamped pick id dropped) |
| club_pick_posts, book_post_reactions, post_reactions, quotes | 0 → 0 |
| next_up_votes, next_up_nominations, next_up_ballots | 0 → 0 |
| events | 146 → 146 (5 club, 141 personal) |
| search_history | 23 → 23 |
| push_subscriptions | 0 → DROPPED (Phase 4) |

- **Report sections:**
  - `normalised`: `book_posts.pick_id 1, books.pages 5, events.payload.rating_take 0`.
  - `merges`: 1 item.
  - `differences`: 19 items: 4 year changes and 16 page changes, 4 of which fill pages that were empty.
  - `isbn keys`: 36 (21 matched, 15 unmatched).
  - `notes`: none.
- **compare.sh:** exit 0.
  - Every count matches the report.
  - `club_memberships` 2 = 2, `club_invite_uses` 2 = 2.
  - `accounts: all 2 match (id|username|password_hash)`.
  - No per-user difference.
  - A second run after the server check gave exactly the same output.
- **Server:** `GET /api/health` returned 200 `{"status":"ok"}`, and `GET /` returned 200 with the SPA shell. The log had no WARN or ERROR. Stopped with SIGTERM. Nobody signed in (0 sessions).

## Problems found, and what they change in the runbook

Nothing blocks the cutover, and no command in `docs/phase2/cutover.md` needs to change.

1. **compare.sh needs `sqlite3` and `psql`.** The agents VM has neither, so the script ran in the container above. The runbook already installs `sqlite3` on the app VM and runs `psql` through `docker compose exec`. The container call is a fallback if the app VM should stay without `sqlite3`.
2. **A zero count in `normalised`.** The report prints `events.payload.rating_take 0`: `ReportBuilder::normalised_by` stores a key even when n is 0 (`importer/events.rs:217`). This is cosmetic, but the go/no-go reader should expect it. A later package can skip n == 0; the cutover does not need that fix.
3. **Open Library 404s in the log.** Each ISBN Open Library does not know logs a WARN `outbound request failed ... status=404` (17 per run). This is expected and only noise in the log.
4. **Most matched ISBN rows are in `differences`.** 19 of the 21 matched ISBN rows show the catalog's year or pages instead of what the legacy app showed. This is expected. The cutover's dry run should show about the same; at the go/no-go only new kinds of lines count as unexpected.
5. **Unmatched ISBNs.** 15 ISBN rows are bare facts: neither DNB nor Open Library knows them. `rematch-legacy` may find some later (runbook 3.5); the rehearsal did not run it.
6. **The time zone is an assumption.** The Python default `BOOKCLUB_TZ` is `UTC`, and the rehearsal assumed `Europe/Berlin`. The real value comes from the production `.env` (§5 item 2).
7. **Where the copies of the source go.** The importer and compare.sh both copy the source into `$TMPDIR`. Here, `TMPDIR` pointed into `<rehearsal>` so personal data stayed there. On the app VM the importer's copy lands in the container's `/tmp`, which the section 1 checklist already covers, and compare.sh's copy lands in root's `/tmp` (mode 700, removed on exit).

**Downtime window** (the rule in cutover.md section 1): 2 × about 20 s for the runs, plus 0 s lookup wait, plus 1 h of checks, comes to about 1 h. Add each `docker compose run` start-up, the snapshot and its transfer, and the Caddy steps. Announcing about 1.5 h leaves room for all of that.

**Go/no-go baseline:** the counts above. At the cutover every count is at least these, and nothing appears in `notes`.

## Still to do

1. **Rafael's browser check** at `http://agents.lan:8080` (the orchestrator starts the server with the command above): sign in with the old password, then check the shelves, layouts, picks, diary, activity feed and an old `/works/OL…W` link. The sign-in re-hashes his password hash, and the email prompt may store an address. After that, compare.sh no longer reports matching accounts, which is expected.
2. **#38:** post only the counts, the slug, the run time and the lookup wait, then close it once Rafael confirms.
3. **Delete everything right after the check** (Q14 A: the dev-VM copy is deleted right after the rehearsal):
   - stop the server;
   - `docker rm -fv bookclub-pg-rehearsal`;
   - `rm -rf ~/bookclub-prod-kopie`: the copy, the reports, the compare output, the media and the private owner and slug files (the slug is in #38 by then);
   - `git worktree remove --force ../bookclub-cut2`.
4. **Before the cutover:** Rafael's production facts (§5 item 2): `BOOKCLUB_NAME`, `BOOKCLUB_TZ` and the owner's username. If the name is not "Bookclub", decide on the slug (see the assumptions).
