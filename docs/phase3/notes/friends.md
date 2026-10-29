# FRIENDS (#57) implementation notes

Owner: FRIENDS. These notes record what the friends package built, where it decided something the plan (`docs/phase3/work-plan.md` §4, §6 answers 16–29) left open or where the code made it differ, and the texts the integrator moves into `docs/phase1/conventions.md`, `docs/phase1/endpoint-map.md`, `CLAUDE.md`, `README.md` and `docs/plan.md`. The design documents themselves are unchanged.

Status: **part 1 (data and visibility) done**: e5d86e3 (the schema and the account-reference rules), c06b067 (the visibility layer and its tests). Part 2 (the friend and block routes, friend invites into clubs, the rate limit, the SPA) follows on the same branch, `phase3/friends`.

## Part 1: data and visibility

### What part 1 built

- **`server/migrations/3100_friends.sql`**, as §4.1 and §4.4 write it:
  - `friendships` (one row per pair: the primary key `(requester_id, addressee_id)`, the unique index `friendships_pair_key` on `(least, greatest)`, `friendships_not_self`, `friendships_accepted_after`; `accepted_at` NULL = pending), with `friendships_addressee_idx`.
  - `user_blocks` (`user_id` blocked `blocked_user_id`, `user_blocks_not_self`), with `user_blocks_blocked_idx`.
  - `club_invites.invitee_id` (`ON DELETE CASCADE`), the check `club_invites_bound` (bound only with a club and `max_uses = 1`) and the partial index `club_invites_invitee_idx`.
  - `COMMENT ON` both tables, `friendships.accepted_at` and `club_invites.invitee_id`; the header comment names the home feed as the one place outside the policy functions.
  - `bc_friends(viewer) → (user_id, since)`; `bc_can_see_user` and `bc_visible_owners` replaced with `CREATE OR REPLACE`, same names, parameters and OUT columns.
- **Visibility** (`server/src/visibility.rs`): only the doc comments of `Relation::Audience` and `relation` change ("they share at least one club or are friends"). No new variant.
- **Home feed** (`server/src/activity/feed.rs`): the WHERE clause of §4.4, the friend branch inside the `e.club_id IS NULL` group. The module and handler doc comments say so (the handler's is the OpenAPI description of `activity_home_feed`).
- **Doc-comment hunks** "they share a club" → "they share a club or are friends" in `server/src/media/reports.rs` (module, `report`'s description, the comment at the avatar check) and `server/src/posts/out.rs` (`load_rows`).
- **Account references** (§4.7):
  - Export (`server/src/auth/export.rs`): new keys `friends`, `blocks`, `club_invites_received`; `invites_created` drops `invitee_id` and adds `invitee` (username). A new `friend_sections` holds the first two; `club_invites_received` sits in `club_sections` after `invite_uses`.
  - Purge (`auth::empty_sign_ups`): three `NOT EXISTS` (requests sent, accepted friendships as addressee, blocks made). Doc comments of `empty_sign_ups` and `purge_empty_sign_ups` updated (see "Lock order of the purge").
  - Deletion (`auth::deletion::lock_clubs`): `UNION ALL SELECT club_id FROM club_invites WHERE invitee_id = $1`; the doc comments of `delete_account` (its OpenAPI description), `delete_user` and `lock_clubs` say what goes with the account and why friendships and blocks need no lock.
  - `server/tests/auth_export.rs` (`SOURCES`, `EXEMPT`, 29 → 34, every section's fixture rows), `server/tests/jobs.rs` (`DATA` 27 → 30, `TRACES` 3 → 6), `server/tests/auth_delete.rs` (the case "friend invite bound to them"), `server/tests/parity_schema.rs` (`user_blocks` in `PERSONAL`, `friendships` in `GLOBAL`), the #37 table of `docs/phase1/parity-checklist.md`.
  - `docs/legal/vvt.md`: the rows `friendships | 16`, `user_blocks | 16`, `club_invites | 6, 15, 16`, and the stub heading "16. Friends (written with #53)" after activity 15.
  - `deploy/hosted/revoke-access.sql` (one header line, now two) and step 7 of the README's restore section (the friendships query, and "and friendships" in the step's sentence).
- **The matrix** (§4.8): `Case::Friend` in `server/tests/parity_visibility.rs` (`CASES` 4 → 5, every kind handles it), the five new columns in `USER_COLUMNS`, `server/tests/common/visibility.rs` (amended, below), `server/tests/visibility_policy.rs`, `server/tests/club_visibility.rs`, `server/tests/posts_visibility.rs` (a count the second audience viewer doubles), and the #46 table of the checklist (a "Friend" column, the prose counts).
- **New tests:**
  - `server/tests/friends_visibility.rs`: a friend's club events never reach a viewer outside the club (with or without `?username=`); personal events from the friendship on, nothing for either side of a pending request, nothing after an unfriend; an "only me" rating never reaches a friend (shelf, book page, feed payload); a friend sees the owner's profile picture on posts and may report it, a requester gets 404 `Not found`.
  - `visibility_policy.rs::policy_functions_still_inline` (§4.4's `EXPLAIN` test, also over `bc_visible_quotes` and `bc_visible_posts`): no `Function Scan`. `bc_friends` inlines inside `bc_visible_owners`, so the fallback of §4.4 (writing its branches into `bc_visible_owners`) was not needed.
  - `visibility_policy.rs::a_friend_who_is_also_a_co_member_counts_once`: one row in `bc_visible_owners`, one reader, `rating_count` 1, one reaction name.
- **Copy:** `errors.noMember` is "Nobody by that name in your clubs or among your friends" / "Niemand mit diesem Namen in deinen Clubs oder unter deinen Freunden" (§4.4). The server string `No member with that name` is unchanged.
- `server/.sqlx/` regenerated on a freshly reset database; `frontend/src/api/generated.ts` regenerated (three descriptions changed, no type).

### Decisions

- **The friend viewers are written with SQL in `common/visibility.rs`** for now (the task for part 1: the routes come in part 2). The rows are what the routes leave: an accepted row for `owner`–`friend` and `friend`–`stranger`, a pending row each for `asked` → `owner` and `owner` → `requested`, no row for `exfriend` (inserted accepted, then deleted, as an unfriend does) and a `user_blocks` row instead of the friendship for `blocked`. Part 2 may switch them to the routes (§4.8 says so); the fixture's meaning stays.
- **`audience()` is `[co_member, friend]`, `strangers()` has seven viewers.** Loops over them in the package tests (`shelf_`, `posts_`, `catalog_`, `club_visibility.rs`) cover the new viewers without changes, except the two below.
- **`club_visibility.rs::finished_ratings_in_the_feeds_follow_the_entry`**: the plan switches the audience loop to `&v.co_member`. It loops two paths, so I kept `audience()` for the home feed (the friend sees the redacted event there) and use `[&v.co_member]` for the club feed, plus an explicit 404 `No such club` for the friend on it. That keeps the plan's intent and the friend's home-feed coverage.
- **The matrix's Friend case per kind** follows the §4.8 table. Two checks need a temporary change, undone in the same check, because one world serves every kind: the Diary case has the owner write a club post in `shared` (the friend cannot see or react to it) and deletes it again; the Activity case moves `accepted_at` a day later (every event of the owner then predates the friendship and is hidden) and back.
- **Export shapes.** `friends` is `{friends: [{username, accepted_at}], requests_sent: [{username, created_at}], requests_received: [{username, created_at}]}`, `blocks` `[{username, created_at}]`, `club_invites_received` `[{club_name, invited_by, created_at, expires_at, used, revoked}]` with `used` = `use_count > 0` and `revoked` = `revoked_at IS NOT NULL` (booleans; the code is never in it). Lists are oldest first, like the other sections. `invites_created` gains `invitee` only in the whole-row form (an admin of the club); the reduced form for a former admin stays as it was.
- **The three friend columns in `USER_COLUMNS` are `Not(…)` until part 2.** `Kind::Friends` covers `GET /api/friends` and the username routes (§4.8), which do not exist yet; a kind needs its routes for its five cases, and a kind no table names fails clippy. So `friendships.requester_id`, `friendships.addressee_id` and `user_blocks.user_id` give the reason "friend data: no route serves it before FRIENDS part 2, which makes it Kind::Friends". Part 2 turns them into `Kinds(&[Kind::Friends])`, adds the kind, its checklist row "Friends" and the five `USERNAME_ROUTES` rows. `user_blocks.blocked_user_id` and `club_invites.invitee_id` have their final `Not(…)` reasons (they name `friends_blocks.rs` and `friends_invites.rs`, which part 2 writes).

### Deviations from the plan

1. **The export's friendships carry `accepted_at`, not `since`** (§4.7 writes `{friends: [{username, since}], …}`). The export rewrites timestamps to RFC 3339 UTC with `Z` by key, for every key ending in `_at` (`utc_timestamps`, conventions §5.7); `since` would stay in Postgres's `+00:00` form and break that rule (`assert_utc_timestamps` only looks at `_at` keys, so a test would not even catch it). `accepted_at` is also the column's name. The API's `FriendOut.since` (part 2) is unaffected.
2. **`parity_schema.rs` `GLOBAL` goes 15 → 16, not 17 → 18**: COVER's two tables are not in `phase-3` yet. After the merge it is 18, as the plan says.
3. **The checklist's #46 prose says 34 columns, 11 routes and 11 schemas** (the plan: 16 routes). The five username routes come with part 2, which makes it 16. (The prose said 28 columns and 9 routes before: both were stale since ADM2.)
4. **`common/visibility.rs` writes the friendships with SQL** (§4.8: "through the routes"), see "Decisions".
5. **The purge's lock order needed a new argument** (not in the plan), see the next section.

### Lock order of the purge

An empty sign-up may now hold a friend invite bound to it (`club_invites.invitee_id` is a trace, §4.7). `purge_empty_sign_ups` locks the account first and then calls `delete_user`, whose `lock_clubs` now locks that invite's club: a club after the account, the reverse of every membership write (conventions §5.3, §5.6). It cannot deadlock, and the doc comments of `purge_empty_sign_ups` and `delete_user` say why: no club write waits for the account's row while it holds that club. The only ones that would are the account's own redemption of the invite (it needs a session: the purge holds the account's sessions locked and re-checks after the lock, and a new sign-in waits for the account's row) and a new invite bound to it (`kind: "friend"` needs an accepted friendship, which is data, and an empty account can only get one by accepting a request itself). A club delete or a revocation touches the invite row but never the account's row, so the purge only waits for it.

### Texts for the integrator

**`conventions.md` §5.6 (account deletion):**
- In the lock order paragraph, the list of rows becomes: "memberships (also those the user invited, `invited_by`), removals, clubs the user created (`clubs.created_by`), invites (created by the user or bound to them, `invitee_id`) and invite uses, picks, milestones, posts, reactions, nominations, ballots and events". Add: "Friendships and blocks reference no club and cascade without a lock."
- Step 4's cascade list gains "friendships and friend requests (both directions), blocks (both directions), and the friend invites bound to the user (`club_invites.invitee_id`; ON DELETE CASCADE, never SET NULL, which would turn a bound invite into an open code)".
- "A second caller" paragraph: replace "it takes no club row first, because an empty account has no row in any club and none can arrive while its row is locked, so `lock_clubs` locks nothing" with: "it takes no club row first. An empty account has no row in any club but, at most, a friend invite bound to it, whose club `lock_clubs` then locks after the account. No club write waits for the account's row while it holds that club: the account's own redemption needs a session (locked by the purge, or, signing in, waiting for the account's row), and a new invite bound to it needs an accepted friendship, which is data that only the account itself can create."

**`conventions.md` §5.7 (export):**
- Table rows after `invite_uses`: `club_invites_received` (club name, who invited, created, expires, used, revoked; never the code) ← `club_invites.invitee_id`; `friends` (`{friends: [{username, accepted_at}], requests_sent: [{username, created_at}], requests_received: [{username, created_at}]}`) ← `friendships.requester_id`, `friendships.addressee_id`; `blocks` (`[{username, created_at}]`) ← `user_blocks.user_id`.
- `invites_created`: "the whole row minus `created_by` and `invitee_id`, with `invitee` (the username of a friend invite's invitee)".
- The shapes sentence: "`friends` is an object of three lists, like `reports`".
- "Both directions of the friendships are listed, requests hidden by the user's own block included: each row is the user's own act or one addressed to them."
- Exempt: "`user_blocks.blocked_user_id` (who blocked the account; telling it would undo the block, Art. 15(4), like the reporter of a report)."
- The pinned count: "34 since FRIENDS (`friendships.requester_id`, `friendships.addressee_id`, `user_blocks.user_id`, `user_blocks.blocked_user_id`, `club_invites.invitee_id`)".
- The purge paragraph: "or a row in `TRACES` with the reason when it is a trace of signing up or in, or of another account's act the account never answered (today `sessions.user_id`, `email_tokens.user_id`, `club_invite_uses.user_id` of a sign-up invite, `friendships.addressee_id` of a pending request, `user_blocks.blocked_user_id`, `club_invites.invitee_id`). `friendships.addressee_id` is in both lists: an accepted request is data."
- §5.7's "Clubs the user no longer belongs to" and §5.5 are unchanged.

**`conventions.md` §6.1:** the `Audience` row: "they share at least one club or are friends (an accepted `friendships` row; a pending request grants nothing)". Add after "Club data is visible to the club's members only": "A friend is audience for personal data only: club data (readers, overlap, suggestions, members' previews, club stats, the club feed and club posts) stays the club members'."

**`conventions.md` §6.2:** a row for `bc_friends(viewer)` → `(user_id, since)`: "the viewer's friends and when each friendship began (`accepted_at`) | friend lists, the home feed's friend branch". `bc_visible_owners`: "the viewer and everyone in their audience (co-members and friends, each once)". `bc_can_see_user`: unchanged use; its body has the friend branch in the explicit pair form (never `least`/`greatest`, which ignore NULLs). Add: "`visibility_policy.rs::policy_functions_still_inline` checks with EXPLAIN that the set-returning functions are still inlined."

**`conventions.md` §6.4:** "**Home feed** = the union of the viewer's club feeds, plus the viewer's own personal events, plus the personal events of the viewer's friends created at or after the friendship began (`bc_friends(viewer).since`), deduplicated by id. The friend branch sits inside the personal-event group, so a friend's club events reach only that club's members. This is the one place outside the policy functions that reads friendships (migration 3100's header)." The `?username=` filter follows: a friend's name resolves (`bc_can_see_user`).

**`conventions.md` §6.5:** "viewer types: self, co-member, friend (in no club), member of another club, stranger (in no club, a friend of the friend), former co-member, both sides of a pending friend request, former friend, blocked friend". The matrix has five cases (the test keeps the name `every_personal_data_kind_has_its_four_viewer_cases`, which the checklist names). Record the amendment of the W1-frozen `tests/common/visibility.rs`: `Viewers` gains `friend`, `asked`, `requested`, `exfriend` and `blocked`; `audience()` is `[co_member, friend]`; `strangers()` adds the last four.

**`conventions.md` §10.1:** part 2 (the request limit).

**`CLAUDE.md`, "References to accounts and pictures":** "…or a `TRACES` row with the reason (a trace of signing in or another account's act; `every_reference_to_an_account_is_data_or_a_trace_of_signing_in`)". The "A new foreign key to `users`, …" sentence stays.

**`endpoint-map.md`:** part 2 (the Friends section, `kind: "friend"`, `InviteOut.invitee` and `personal`, the new strings). From part 1 only the three changed descriptions (`activity_home_feed`, `auth_delete_me`, `media_report`) and the `errors.noMember` copy.

**For #53 (the privacy text), facts only:** friendships and requests are stored per pair with the time of the request and of the acceptance; a friend sees what a club mate sees of personal data (shelf, ratings except "only me", quotes, diary, stats, the home feed from the acceptance on), never club data; a block is stored, never told to the blocked account, and withheld from its export; all of it goes with either account; the export lists friendships, requests both ways, the account's own blocks and the friend invites it received.
