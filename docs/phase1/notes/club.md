# CLUB (#45): club features notes

Implementation notes for `server/src/picks/**`, `server/src/votes/**`, `server/src/overlap.rs`,
`server/src/activity/feed.rs` and `server/tests/club_*.rs`. The design is endpoint-map.md §9 and
docs/phase1/spec/club.md; this file records where the code departs from them or adds detail, and
what other packages need to know.

## Status

- **Part 1 (done):** `server/src/picks/**`: the current pick (`GET`, `PUT`, `PATCH`, `DELETE
  /api/clubs/{slug}/pick`), the history, the meeting download, milestones; the pick and
  milestone rows of the role table; the pick readers in `club_visibility.rs`.
- **Part 2 (to do):** `votes/**` (vote, deadline, nominations, cast, apply, auto-close,
  suggestions), `overlap.rs`, `activity/feed.rs` (club and home feeds), their role rows (add
  their tags to `TAGS` in `tests/club_roles.rs`), the feed cases of `club_visibility.rs`, and
  the parity files `test_vote.py`, `test_vote_deadline.py`, `test_overlap.py`,
  `test_activity.py` (`test_push.py` is reasoned, Phase 4).

Module map (`server/src/picks/`): `mod.rs` (routes, the strings shared with POSTS, the write
lock, `club_zone`), `current.rs` (handlers and `set_pick`), `out.rs` (`ClubPickOut`, its rows
and loader), `ics.rs` (handler and the RFC 5545 builder), `milestones.rs`.

## API surface (tag `picks`)

| Route | Operation id | Schemas |
|---|---|---|
| `GET /api/clubs/{slug}/pick` | `picks_get` | `ClubPickCurrentOut {pick: ClubPickOut\|null, timezone}` |
| `PUT /api/clubs/{slug}/pick` | `picks_set` | `ClubPickSetIn {work_id, note?, meeting_at?}` → `ClubPickOut` |
| `PATCH /api/clubs/{slug}/pick` | `picks_update` | `ClubPickPatchIn {note?, meeting_at?}` → `ClubPickOut` |
| `DELETE /api/clubs/{slug}/pick` | `picks_clear` | → `ClubPickCurrentOut` (`pick: null`) |
| `GET /api/clubs/{slug}/pick/history` | `picks_history` | `ClubPickHistoryOut {items, timezone}` |
| `GET /api/clubs/{slug}/pick/meeting.ics` | `picks_meeting_ics` | `text/calendar` |
| `GET /api/clubs/{slug}/picks/{pick_id}/milestones` | `picks_list_milestones` | `MilestoneListOut {pick_id, items, timezone}` |
| `POST /api/clubs/{slug}/picks/{pick_id}/milestones` | `picks_create_milestone` | `MilestoneIn` → 201 `MilestoneOut` |
| `PATCH /api/clubs/{slug}/picks/{pick_id}/milestones/{milestone_id}` | `picks_update_milestone` | `MilestonePatchIn` → `MilestoneOut` |
| `DELETE /api/clubs/{slug}/picks/{pick_id}/milestones/{milestone_id}` | `picks_delete_milestone` | → 204 |

Other schemas: `ClubPickReader`, `MilestoneUnit` (`"percent"|"chapter"`). Members read; admins
(and the owner) write.

## Deviations and added detail (part 1)

### Locks and concurrency

- **Every pick and milestone write takes the club row `FOR NO KEY UPDATE`** (`picks::lock_as_admin`)
  and re-reads the caller's role under it: a caller removed meanwhile gets 404 `No such club`,
  one demoted 403 `Only club admins can do that`. `FOR NO KEY UPDATE` conflicts with itself and
  with the membership writes' `FOR UPDATE`, but not with the `FOR KEY SHARE` of foreign-key
  checks, so club posts and events are not blocked. conventions §13 lists no lock for picks; the
  spec found the race (two `PUT pick` calls leave two current picks), and the partial unique
  index alone would turn it into a 500.
- **`picks::current::set_pick(conn, club_id, PickWrite)`** takes that lock itself, so callers may
  hold it already. Part 2's apply and auto-close call it. **Lock order: the club row first, then
  the vote row** (`next_up_votes … FOR UPDATE`): apply and auto-close must call
  `picks::lock_club_writes` before they lock the vote. A club delete holds the club row `FOR
  UPDATE` and its cascade then locks the club's vote rows, so an apply that held the vote row
  and then waited for the club row would deadlock with it (as would any club-then-vote path
  part 2 adds, such as a first nomination that locks the club to create the vote).
- `ended_at` of the replaced pick and `started_at` of the new one are `clock_timestamp()` under
  the lock (not `now()`, the transaction start), so a pick never ends before it started, whichever
  of two racing transactions began first. `DELETE` uses `clock_timestamp()` too.

### Picks

- **`PUT` body:** `work_id` is an integer (`works.id`, i.e. `BookOut.id`). A string such as
  `"OL1W"` is 400 (serde's integer message). It is resolved with `require_work`, so a merged-away
  id becomes its survivor. Order: body validation (note), then `meeting_at` (both 400), then 404
  `No such book.`, then the lock and the role re-check.
- **`note` is tri-state** (conventions §4.4): absent leaves it (same work) or `""` (new pick);
  `null` clears it to `""` (the legacy app answered `null` with `Input should be a valid string`);
  a string is trimmed (Python `strip`) and limited to 280 code points.
- **`meeting_at`:** absent leaves it (same work) or none (new pick); `null`, `""` or whitespace
  clear it.
- **Same work:** only the present keys change; `set_by` and `started_at` stay; no event
  (endpoint-map §9 fix of the legacy note wipe and setter change).
- **`PATCH …/pick`** with `{}` is a no-op 200. An ended pick is not current: 404 `No club pick yet`.
- **`DELETE …/pick`** without a current pick answers 200 `{"pick": null}` and writes nothing
  (parity); it is admin-only like the other writes.
- **`ClubPickOut.book`** has no edition context (members read different editions), like
  `ClubPickSummary`.
- **`set_by`** is null when the setter's account is gone (legacy fallback `"unknown"`).
- **Readers** are the club's members with the work on their shelf (any status), read through
  `bc_visible_shelf(viewer)` joined with the club's memberships: "only me" rating and take are
  `null`/`""` for everyone but the owner; `dnf_reason` and `progress` stay. Order: the viewer
  first, then username in byte order; `reading`/`finished` keep that order. Empty on ended picks;
  `on_shelf`/`shelf_id` (the caller's own entry) are filled on ended picks too.
- **`timezone`** (every response) is the club's stored zone, or `UTC` when the stored value is
  not a zone (a corrupt row; club writes validate it). Labels use the same zone.

### Events

- `pick_set` (new pick only): actor = the setter, work, pick, payload `{"meeting_at"}` when a
  meeting is set. `pick_cleared`: payload `{}`. `milestone_added`: payload `{"title", "due_at"?}`.
- **Payload timestamps are RFC 3339 UTC with `Z`** (`2026-08-22T19:00:00Z`), like every
  timestamp of the new API. The legacy app wrote Python `isoformat()` (`…+00:00`); imported
  legacy payloads keep that form. Both parse with `Date`, so the feed renders either.

### Meeting download (`.ics`)

- The builder is the legacy one byte for byte (escape order, lone CR kept, 75/74-octet folding
  that never splits a UTF-8 sequence, CRLF, 90 minutes, `UID:pick-{id}@bookclub`). Its unit
  tests reproduce the Python golden output, the UTF-8 fold vector and an escape/fold vector, all
  generated with `backend/app/ics.py`.
- Inputs changed per endpoint-map §9: `PRODID` and `SUMMARY` use the **club name** (as stored;
  club names are already trimmed, collapsed and at most 80 characters, so the legacy
  `sanitize_name` is not applied); the description links `{PUBLIC_URL}/book/{work_id}` instead of
  openlibrary.org; **the description's last line and `URL:` are both the club page
  `{PUBLIC_URL}/c/{slug}`** (in the legacy app both were the instance URL). `PUBLIC_URL` is
  always configured, so `URL:` is always present.
- `Content-Type: text/calendar; charset=utf-8`, `Content-Disposition: attachment;
  filename="bookclub-{pick_id}.ics"` (unchanged).

### Milestones

- **Model:** `unit` (`percent`, `chapter`, or null for a title-only checkpoint), `range_from`
  (null = "up to `range_to`"), `range_to`; the title stays required (work-plan appendix "club" 7).
- **Create, order of checks:** validation (title: `Give the milestone a title`, `Keep the title to
  120 characters`; each range end `Input should be greater than or equal to 0`; then the new `A
  range needs a unit` and `Percent milestones go up to 100`; all joined by `; `), then 404 `No
  club pick with that id`, 400 `That pick is closed`, 400 `Chapter range is backwards` (also
  for percent), 400 `meeting_at must be a date and time` (`due_at`, the legacy message). An
  unknown `unit` value is 400 `Invalid value`.
- **PATCH** is partial (every field `Patch<T>`); `{}` is a no-op 200. `title: null` is 400 `Give
  the milestone a title`; `null` clears `unit`, a range end or `due_at`, and `""` clears `due_at`.
  Order: validation, 404 pick, 404 `No milestone with that id`, then the range rules on the
  **merged** values (`A range needs a unit`, `Percent milestones go up to 100`, then `Chapter
  range is backwards`), then `due_at`. No closed-pick check on PATCH or DELETE (parity).
- `position` on create is the number of the pick's milestones, under the club lock; DELETE
  renumbers densely in `(position, id)` order; posts are detached by `posts_milestone_fk`.
- **`note_count`** counts the club's live posts with the milestone (`deleted_at IS NULL`); a
  tombstone (a deleted post kept for its replies) does not count.
- `passed` = `due_at <= now()` at read time; a past due time on create is allowed.

## Strings

All are module constants in `picks` (`mod.rs`, `current.rs`, `milestones.rs`).

- **New, for #48 to map (en and de):** `Percent milestones go up to 100`, `A range needs a unit`.
- **Legacy strings of this area that the SPA showed raw** (never in `apiErrors.ts`), unchanged:
  `No club pick yet`, `No club pick with that id`, `That pick is closed`, `No meeting scheduled`,
  `note must be 280 characters or fewer`, `meeting_at must be a date and time`, `No milestone with
  that id`, `Give the milestone a title`, `Keep the title to 120 characters`, `Chapter range is
  backwards`. #48 may map them when it builds the club pages.
- Also returned here: `No such club`, `Only club admins can do that`, `No such book.` (W1
  constants).

## For other packages

- **POSTS:** the pick thread strings are the same text as `picks::{NO_PICK_YET,
  NO_PICK_WITH_ID, PICK_CLOSED}` (POSTS keeps its own constants; they must stay byte-identical).
  The thread's `timezone` should follow `picks::club_zone` (stored zone, `UTC` for a corrupt one).
  A club post's `milestone_id` must name a milestone of the same pick (`posts_milestone_fk`);
  `note_count` ignores tombstones.
- **FE (#48):** the meeting edit uses `PATCH …/pick`; `PUT …/pick` with the same work keeps the
  note and meeting unless they are sent. Payload `meeting_at`/`due_at` strings may end in `Z` or
  `+00:00`.
- **PAR (#49):** see "Parity" below.

## Parity (part 1)

Ported, with `/// parity:` lines:
- `test_pick.py` (8): all in `tests/club_picks.rs`. Reasoned changes inside the ports:
  - `::test_pick_requires_auth`: also covers PATCH, history and the `.ics`.
  - `::test_empty_pick_then_set_and_read`: `/api/config` has no `timezone` any more (W1); the
    pick's `timezone` is checked instead.
  - `::test_reading_vs_finished_and_clear`: clearing is admin-only now; a member's DELETE is 403.
    Shelf entries are inserted with SQL (SHELF's routes).
  - `::test_meeting_uses_club_timezone`: the zone is the club's (`POST /api/clubs` with
    `timezone`), not `BOOKCLUB_TZ`.
  - `::test_invalid_timezone_falls_back_to_utc`: replaced by "400 `That is not a valid time
    zone` on club update" plus "a corrupt stored zone reads as UTC"
    (`an_invalid_zone_is_refused_and_a_corrupt_one_reads_as_utc`).
  - `::test_invalid_pick_payload`: the blank-title and `/works/` key checks are gone with the
    client metadata (spec §5.4); an unknown `work_id` is 404 `No such book.`, a missing one 400
    `Field required`.
- `test_notes_and_meeting.py::test_meeting_ics_download` → `club_picks.rs`
  (`the_meeting_downloads_as_an_ics_file`): the club name and the new links replace
  `Bookclub` and the openlibrary.org link.
- `test_notes_and_meeting.py::test_ics_folds_long_lines_without_breaking_utf8` → unit test in
  `src/picks/ics.rs`.
- `test_stats_milestones_quotes.py::test_milestones_crud_and_threads` (milestone parts) →
  `tests/club_milestones.rs::milestones_crud_and_threads`. The post parts use rows inserted with
  SQL (the post routes are POSTS'; "an unknown milestone on a post is 400" is POSTS' test). The
  PATCH clears `due_at` with an explicit `null` (partial update). "The feed contains
  `milestone_added`" is checked on the `events` table until part 2's feed exists. The club-stats
  assertions of that file are SHELF's.

Reasoned away (no Rust port of the behaviour): none in part 1.

## Tests (part 1)

- `tests/club_picks.rs` (19): the 8 `test_pick.py` ports, the `.ics` download (and UTC times),
  same-work partial updates, PATCH, events, history order, merged works, two clubs, two
  concurrent PUTs in a fixed order (held club lock), a burst of 8 concurrent PUTs, and an admin
  demoted while waiting for the lock.
- `tests/club_milestones.rs` (10): the milestone port, units and ranges, the order of checks,
  partial PATCH, ended picks, dense renumbering, `note_count`, club-zone labels, ids from another
  club or pick, member/admin access.
- `tests/club_roles.rs` (1): an allowed and a denied case for every `picks` operation, with a
  completeness check over the OpenAPI document.
- `tests/club_visibility.rs` (2): pick readers across the viewer types (only-me redaction,
  `dnf_reason`/`progress` visible, members of this club only) and the reader order.
- Unit tests (7): the three `.ics` golden vectors, the UTF-8 fold port, fold edges, the
  milestone range rules, `club_zone`.
- Mutation-checked: without the club lock the burst test gets a 500; without the role re-check
  the demoted-admin test fails; reading readers from `shelf_entries` instead of
  `bc_visible_shelf` fails the redaction test.
