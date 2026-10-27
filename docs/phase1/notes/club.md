# CLUB (#45): club features notes

Implementation notes for `server/src/picks/**`, `server/src/votes/**`, `server/src/overlap.rs`,
`server/src/activity/feed.rs` and `server/tests/club_*.rs`. The design is endpoint-map.md §9 and
docs/phase1/spec/club.md; this file records where the code departs from them or adds detail, and
what other packages need to know.

## Status

- **Part 1 (done):** `server/src/picks/**`: the current pick (`GET`, `PUT`, `PATCH`, `DELETE
  /api/clubs/{slug}/pick`), the history, the meeting download, milestones; the pick and
  milestone rows of the role table; the pick readers in `club_visibility.rs`.
- **Part 2 (done):** `server/src/votes/**` (vote, deadline, nominations, cast, apply, lazy
  auto-close, suggestions), `server/src/overlap.rs`, `server/src/activity/feed.rs` (club and
  home feeds); their role rows; the feed cases of `club_visibility.rs`; the concurrency tests;
  the ports of `test_vote.py`, `test_vote_deadline.py`, `test_overlap.py` and
  `test_activity.py`; `test_push.py` reasoned (Phase 4).

Module map (`server/src/picks/`): `mod.rs` (routes, the strings shared with POSTS, the write
lock, `club_zone`), `current.rs` (handlers and `set_pick`), `out.rs` (`ClubPickOut`, its rows
and loader), `ics.rs` (handler and the RFC 5545 builder), `milestones.rs`.

Module map (`server/src/votes/`): `mod.rs` (routes, strings, `lock_open_vote`,
`lock_as_member`, `tallies` and `leader`), `ballot.rs` (GET and PATCH vote, nominate, cast),
`close.rs` (apply, `maybe_auto_close`, `run_auto_close`), `out.rs` (`VoteOut` and its loader),
`suggestions.rs`. `server/src/overlap.rs` and `server/src/activity/feed.rs` are one file each.

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

## API surface (tags `votes`, `overlap`, `activity`)

| Route | Operation id | Schemas |
|---|---|---|
| `GET /api/clubs/{slug}/vote` | `votes_get` | `VoteOut` (`VoteNominationOut`) |
| `PATCH /api/clubs/{slug}/vote` | `votes_set_deadline` | `VoteDeadlineIn {closes_at?}` → `VoteOut` |
| `POST /api/clubs/{slug}/vote/nominations` | `votes_nominate` | `VoteNominateIn {work_id}` → 201 `VoteOut` |
| `POST /api/clubs/{slug}/vote/cast` | `votes_cast` | `VoteCastIn {nomination_id}` → `VoteOut` |
| `POST /api/clubs/{slug}/vote/apply` | `votes_apply` | `VoteApplyIn {nomination_id, meeting_at?}` → `VoteApplyOut {pick, vote}` |
| `GET /api/clubs/{slug}/vote/suggestions` | `votes_suggestions` | `VoteSuggestionsOut {items: VoteSuggestionOut[]}` |
| `GET /api/clubs/{slug}/overlap?include_reading=` | `overlap_get` | `OverlapOut {items: OverlapBookOut[], include_reading}`, `OverlapMember` |
| `GET /api/activity?before=&limit=&username=` | `activity_home_feed` | `ActivityOut {items: ActivityItem[], has_more, timezone}` |
| `GET /api/clubs/{slug}/activity?before=&limit=&username=` | `activity_club_feed` | `ActivityOut` |

Members (unverified ones too) read, nominate and cast; admins (and the owner) set the deadline
and apply a nomination. The home feed needs only a session.

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

## Deviations and added detail (part 2)

### Votes

- **Nominate body:** `{work_id}`, an integer (`BookOut.id`), resolved with `require_work` (a
  merged-away id becomes its survivor; unknown: 404 `No such book.`). No client metadata and no
  catalog upsert (spec §5.4); the `/works/` key and title checks are gone.
- **Order of checks:**
  - nominate: body (400) → the auto-close (its own transaction, committed) → 404 `No such book.`
    → the club lock and the membership re-check (404 `No such club`) → the open vote, or a new
    one → 400 `This vote already has enough books` → 400 `That book is already the club pick`
    → 400 `That book is already nominated`. The legacy app checked the count before it upserted
    the book; here the book is resolved first. A refused first nomination rolls back the vote
    it opened (parity).
  - cast: body → the auto-close → lock and membership → 404 `No open next-up vote` → 404 `No
    nomination with that id` → ballot upsert; `voted` only when the row was inserted (`xmax =
    0`).
  - PATCH (admins): lock and role (403 `Only club admins can do that`) → 404 `Nominate a book
    first` (before the date is parsed, parity) → 400 `meeting_at must be a date and time` → 400
    `The deadline has to be in the future`. `closes_at` absent, `null`, `""` or whitespace
    clears it (parity: `{}` clears), so it is a plain `Option<String>`, not `Patch`. A number is
    400 `Input should be a valid string`. No event, no auto-close.
  - apply (admins): body → lock and role → 404 `No open next-up vote` → 404 `No nomination with
    that id` → 400 `meeting_at must be a date and time` (parity: the legacy app parsed the
    meeting inside `apply_club_pick`, after the lookups). No auto-close. Any nomination, whatever
    its votes; zero votes are fine.
- **The electorate is the club's current members.** endpoint-map §9 says so for `member_count`
  and `not_voted`; here only current members' ballots count as well: `votes`, `voters`,
  `my_vote_id`, `voted_count`, the leader, the auto-close winner and `vote_closed.votes`. A
  member who leaves keeps the ballot row, which counts again if they rejoin. So `voted_count +
  len(not_voted) == member_count` always. Ballots of deleted accounts are deleted with them, so
  the leader and the counts can no longer disagree (spec §6 risk 9).
- **`nominated_by` is null** when the nominator's account is gone (legacy `"unknown"`), like
  `set_by`. A nominator who left the club keeps the credit (and their username shows).
- **Auto-close actor** (work-plan appendix "club" 9): the winning nominator, also after they
  left the club. For a deleted account, the pick's `set_by` and the actor of `pick_set` and
  `vote_closed` are NULL (served as `actor: null`). The legacy app cleared the deadline instead
  when the nominator was missing.
- **Applying (or auto-closing into) the book that is already the current pick** (set by `PUT
  pick` after the nomination) goes through `picks::current::set_pick`, so only what is given
  changes: apply sets the meeting (cleared when `meeting_at` is absent or null, parity: apply
  always passes it), the auto-close changes nothing of the pick. `set_by`, the note and
  `started_at` stay and no `pick_set` is recorded (parity for the event). The legacy app wiped
  the note and made the applier (or nominator) the setter: the same fix endpoint-map §9 makes
  for `PUT pick`. The vote still ends and `vote_closed` is recorded.
- **A new pick from a vote** has note `""`, the given meeting (none on auto-close), `pick_set`
  with `{"meeting_at"}` when set, then `vote_closed` `{"votes": n}` (plus `"auto": true` on
  auto-close). `pick_set` comes first, so the feed shows `vote_closed` on top (parity).
- **The auto-close** runs at the start of `GET vote`, nominate and cast (not `GET pick`,
  `PATCH vote`, apply or suggestions; parity), in a transaction of its own that commits before
  the request's work: a nomination refused with 400 afterwards does not undo it (parity). It
  reads the due vote without a lock, then takes the club row, then the vote row `FOR UPDATE`
  with the deadline re-checked in that locked `SELECT`, then the nominator's user row `FOR KEY
  SHARE`, then writes. A request that waited sees the vote ended and does nothing.
- **Locks:** every vote write takes the club row first (`picks::lock_club_writes`, `FOR NO KEY
  UPDATE`, the same lock as pick writes, so vote and pick writes of one club are serialised)
  and re-reads the caller's membership (nominate, cast: 404 `No such club` when removed
  meanwhile) or role (PATCH, apply: `picks::lock_as_admin`), then the open vote row `FOR
  UPDATE`. conventions §13 lists only the vote row. The club row is needed because a first
  nomination has no vote row to lock (two of them would hit `next_up_votes_one_open`, a 500),
  and for the club-then-vote lock order (part 1). Account deletion locks the clubs it writes in
  (a nomination or ballot counts) before the user row, so it orders with these writes too.
- `closes_at` is RFC 3339 with `Z`; `closes_local` and `closes_label` are in the club's zone.
  Nominations are ordered by votes (descending), then id; voters by username (byte order).
- **Suggestions:** candidates are the want-to-read entries of current club members, read
  through `bc_visible_shelf(caller)` (legacy: all users). Excluded: every pick of the club,
  current or ended, and the open vote's nominations. Liked subjects come from `works.subjects`
  (`text[]`, so no JSON parsing) of ended picks, lowercased with `to_lowercase`. The rating is
  `works.ol_rating`, formatted `{:.1}`, which rounds the exact binary value half to even like
  Python (`4.25` → `4.2`, unit test). After `(-score, lowercased title)`, ties go by work id. The
  book is the work (no edition context).

### Overlap

- Over the current club members' entries, read through `bc_visible_shelf(caller)`. Members by
  username (byte order); after `(-count, lowercased title)`, ties go by work id.
  `include_reading` is a lax boolean; an empty value is 400 (parity).

### Activity feeds

- **Routes:** `GET /api/activity` (home) and `GET /api/clubs/{slug}/activity` (club), as
  conventions §6.4. The home feed is one query over `events` (no duplicates by construction):
  club events of `bc_member_clubs(caller)`, the caller's own personal events, and personal events
  of the members of those clubs from their `joined_at` in that club.
- **Redaction runs in SQL,** so `limit` and `has_more` count visible items only:
  - `quote_added` is shown to others only when `payload.quote_id` names a quote of the actor
    that `bc_visible_quotes(caller)` returns. A missing or non-integer `quote_id` counts as
    gone (an imported legacy event without a match is the actor's alone).
  - `shelf_finished` loses `rating` and `take` unless the actor's current entry for the event's
    work exists and `bc_can_see_item(caller, actor, rating_visibility)`. Other keys stay. The
    actor always sees their own payload.
- **`reaction`:** `author_id` is removed and `author` is always present: the username, or null
  when the account is gone or the payload has no integer `author_id` (an imported reaction whose
  legacy author was unknown). The legacy payload left `author` out when it was None.
- `actor` is null for a system event (an auto-close credited to a deleted account); a deleted
  account's own events are deleted with it. `club` is the `ClubRef` of a club event, null for a
  personal one. `mine` is "the caller is the actor".
- **Labels and `timezone`:** the club feed labels everything in the club's zone, and its
  `timezone` is the club's. The home feed labels club items in their club's zone and personal
  items in the caller's zone (work-plan appendix "club" 4), and its `timezone` is the caller's.
  conventions §12's table says "home-feed items: the viewer's zone"; the appendix answers that
  open question per item, so the code follows it.
- **`username` filter:** lowercased (Unicode), not stripped; empty means no filter. Club feed:
  the name must be a current member of the club; home feed: someone the caller can see
  (`bc_can_see_user`: themselves or a co-member). Anything else is an empty page (`items: [],
  has_more: false`).
- **Query validation:** `before` ≥ 1, `limit` 1..100 (default 30), pydantic wording, all
  messages joined by `"; "`; a non-integer is `Input should be a valid integer, unable to
  parse string as an integer`.
- `member_joined` is recorded when someone joins a club by invite (W1); creating a club records
  nothing, so an owner has no `member_joined` in their own club.
- One `OR` query per feed over `events`, newest first by the primary key. Fine at Phase 1 sizes;
  a `UNION ALL` per index (`events_club_feed_idx`, `events_personal_feed_idx`) is the obvious
  step if feeds grow.

## Strings

All are module constants in `picks` (`mod.rs`, `current.rs`, `milestones.rs`) and `votes`
(`mod.rs`).

- **New, for #48 to map (en and de):** `Percent milestones go up to 100`, `A range needs a unit`.
  Part 2 adds no new string.
- **Legacy strings of this area that the SPA showed raw** (never in `apiErrors.ts`), unchanged:
  `No club pick yet`, `No club pick with that id`, `That pick is closed`, `No meeting scheduled`,
  `note must be 280 characters or fewer`, `meeting_at must be a date and time`, `No milestone with
  that id`, `Give the milestone a title`, `Keep the title to 120 characters`, `Chapter range is
  backwards`, `Nominate a book first`, `The deadline has to be in the future`, `This vote
  already has enough books`, `That book is already the club pick`, `That book is already
  nominated`, `No open next-up vote`, `No nomination with that id`. #48 may map them when it
  builds the club pages.
- Also returned here: `No such club`, `Only club admins can do that`, `No such book.`, `Not
  signed in` (W1 constants) and the pydantic-style 400s of `extract.rs`.

## For other packages

- **POSTS:** the pick thread strings are the same text as `picks::{NO_PICK_YET,
  NO_PICK_WITH_ID, PICK_CLOSED}` (POSTS keeps its own constants; they must stay byte-identical).
  The thread's `timezone` should follow `picks::club_zone` (stored zone, `UTC` for a corrupt one).
  A club post's `milestone_id` must name a milestone of the same pick (`posts_milestone_fk`);
  `note_count` ignores tombstones. The feed expects `note_posted {"post_id"}` and `reaction
  {"emoji", "post_id", "author_id"}` with `post_id` = `posts.id` and `author_id` = the post
  author's user id (conventions §15); it serves `author_id` as `author`.
- **SHELF:** the feed's `shelf_finished` redaction reads the actor's **current** entry for the
  event's `work_id`; `quote_added` must carry `quote_id` (the feed hides an event without one
  from everyone but the actor).
- **IMP:** imported `quote_added` events without a `quote_id`, and `reaction` events without an
  `author_id`, stay in the table: the first are the actor's alone, the second show `author: null`.
- **FE (#48):** the meeting edit uses `PATCH …/pick`; `PUT …/pick` with the same work keeps the
  note and meeting unless they are sent. Payload `meeting_at`/`due_at` strings may end in `Z` or
  `+00:00`. Nominate sends `{work_id}`; the deadline and apply are admin-only (403 `Only club
  admins can do that`). `VoteNominationOut.nominated_by`, `ActivityItem.actor` and the
  `reaction` payload's `author` can be null. Feed items carry `club` (null for personal events);
  the home feed's `timezone` is the user's, and each `created_label` is ready to show.
- **Docs (integrator):** conventions §13's lock list could name the club row for vote writes
  (nominate, cast, deadline, apply, auto-close), taken before the vote row.
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
  `milestone_added`" reads the club feed. The club-stats assertions of that file are SHELF's.

Reasoned away (no Rust port of the behaviour): none in part 1.

## Parity (part 2)

Ported, with `/// parity:` lines. Common changes: nominate takes `{work_id}` (catalog works
made by the harness, not client metadata); members are added with `TestApp::join` or an invite
instead of `register`; the deadline and apply are made by the owner (admin-only now).
- `test_vote.py` (7) → `tests/club_votes.rs`:
  - `::test_vote_requires_auth` → `vote_routes_require_a_session` (also PATCH and suggestions).
  - `::test_empty_vote_then_nominate_and_count` → `an_empty_vote_opens_with_the_first_nomination`
    (`member_count` is the club's members).
  - `::test_members_can_vote_and_change_vote` → `members_vote_and_change_their_vote` (plus the
    `voted` events).
  - `::test_cannot_nominate_current_pick_or_duplicate` →
    `the_current_pick_and_duplicates_cannot_be_nominated`.
  - `::test_nomination_limit` → `a_vote_takes_six_nominations`.
  - `::test_apply_winner_uses_club_pick_flow` → `applying_the_winner_makes_it_the_pick`: the old
    pick's post is a row inserted with SQL and checked with SQL (the thread routes are POSTS').
  - `::test_apply_requires_an_open_nomination` → `apply_needs_an_open_nomination` (the strings
    are checked: `No open next-up vote`, then `No nomination with that id`).
- `test_vote_deadline.py` (5):
  - `::test_deadline_and_turnout_reporting` → `club_votes.rs::the_deadline_and_turnout_are_reported`.
  - `::test_deadline_auto_applies_the_leader` → `club_votes.rs::the_deadline_applies_the_leader`
    (reads the club feed).
  - `::test_deadline_with_no_ballots_reopens_without_deadline` →
    `club_votes.rs::a_deadline_without_ballots_is_cleared`.
  - `::test_tie_goes_to_earliest_nomination` → `club_votes.rs::a_tie_goes_to_the_earliest_nomination`.
  - `::test_suggestions_rank_shared_tbr_and_subject_affinity` →
    `club_suggestions.rs::suggestions_rank_shared_wants_and_subject_affinity`: shelf entries,
    subjects and the rating are written with SQL (SHELF's routes; subjects are `text[]`).
- `test_overlap.py` (4) → `tests/club_overlap.rs`: `overlap_requires_a_session`,
  `a_book_two_members_want_is_shown`, `finished_books_and_solo_wants_are_left_out`,
  `include_reading_counts_reading_too` (shelf entries with SQL).
- `test_activity.py` (3) → `tests/club_activity.rs`:
  - `::test_feed_requires_auth` → `feeds_require_a_session` (both feeds).
  - `::test_feed_records_the_club_story_newest_first` →
    `the_feed_tells_the_club_story_newest_first`: `member_joined` comes from an invite join, not
    registration; the shelf, progress, post and reaction events are inserted with SQL (their
    emitters are SHELF's and POSTS'; "unchanged progress records nothing" is SHELF's rule to
    test); pick, nomination and ballots go through the routes (the re-cast records nothing).
    Checked in the club feed and the home feed.
  - `::test_feed_paginates_and_filters_by_member` → `the_feed_pages_and_filters_by_member`
    (both feeds; the name filter is lowercased).

Reasoned away (Web Push is dropped until Phase 4: endpoint-map §12 `/api/push/*`, work-plan
appendix "club" 15; the activity events stay):
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

## Tests (part 2)

- `tests/club_votes.rs` (24): the 11 vote and deadline ports, the club zone of the deadline,
  nomination order, the order of checks, merged works, applying the current pick, the auto-close
  credit (nominator left, account deleted), current-member ballots, where the auto-close runs
  and that it commits first, two clubs, and the concurrency cases: two first nominations meeting
  at a held club lock open one vote; a burst of 9 nominations gives six 201s, three 400s and one
  vote; two `GET vote` calls after the deadline meeting at a held lock close the vote once; a
  burst of reads, ballots and nominations after the deadline closes it once.
- `tests/club_suggestions.rs` (2): the suggestions port; members only, the limit of 6, title
  order, want-to-read only.
- `tests/club_overlap.rs` (6): the 4 overlap ports, the lax boolean, members only and order.
- `tests/club_activity.rs` (7): the 3 activity ports, query validation, two clubs (separate club
  feeds, a combined home feed, the name filters), labels and zones, odd stored payloads.
- `tests/club_visibility.rs` (+3, 5 in all): `shelf_finished` redaction in both feeds across the
  viewer types (only me, audience, entry removed); `quote_added` visibility (audience, only me,
  deleted, someone else's quote, no `quote_id`); personal events from `joined_at` on, only while
  a member, and per shared club in the home feed.
- `tests/club_roles.rs` (1): 19 rows now, one per operation of the `picks`, `votes`, `overlap`
  and `activity` tags (the home feed's denied caller is anonymous: 401), with the completeness
  check over the OpenAPI document.
- Unit tests (+6): the leader rule, the suggestion scores and reasons (including the probed
  example and `{:.1}` rounding), and the feed payload rules.
- **#45 acceptance:** two clubs: picks (`club_picks.rs`), votes (`club_votes.rs`), milestones
  (`club_milestones.rs`), activity (`club_activity.rs`); threads are POSTS' routes. Ids of club A
  under club B's path give 404: picks and milestones (`club_milestones.rs`), nominations
  (`club_votes.rs`). Every role row: `club_roles.rs`. Concurrency: one vote from two first
  nominations, one auto-close, one current pick from two `PUT pick` (`club_picks.rs`).
- Mutation-checked: without the club lock in nominate, both nomination concurrency tests fail;
  without the club and vote row locks in the auto-close, both auto-close concurrency tests fail.
