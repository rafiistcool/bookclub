# FRIENDS (#57) implementation notes

Owner: FRIENDS. These notes record what the friends package built, where it decided something the plan (`docs/phase3/work-plan.md` §4, §6 answers 16–29) left open or where the code made it differ, and the texts the integrator moves into `docs/phase1/conventions.md`, `docs/phase1/endpoint-map.md`, `CLAUDE.md`, `README.md` and `docs/plan.md`. The design documents themselves are unchanged.

Status: **part 1 (data and visibility) done**: e5d86e3 (the schema and the account-reference rules), c06b067 (the visibility layer and its tests). **Part 2 (the API) done**: 13ceec5 (the friend and block routes, the rate limit, the matrix's `Friends` kind), a888202 (friend invites into clubs); below, "Part 2: the API". **Part 3 (the SPA) done**: `/friends`, the store and its badge, the invite sheet, the Home row, personal invites on the members page; below, "Part 3: the SPA". Before its final checks the branch still merges `phase-3` once COVER is in it (`work-plan.md` §2.1 rule 1).

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

## Part 2: the API

### What part 2 built

- **`server/src/friends/`**, merged in `lib.rs` (`pub mod friends;`, `.merge(friends::routes())` after `legal::routes()`), tag `friends`, the operation ids of §4.5:
  - `mod.rs`: the routes, `GET /api/friends` (`friends_list`), `DELETE /api/friends/club-invites/{invite_id}` (`friends_decline_club_invite`), and the shared helpers: `PathName` (the `{username}` extractor), `account_id`, `lock_pair` (the lock order below), `revoke_pair_invites`, and `locked_friend` for the friend invites of `clubs::invites`.
  - `requests.rs`: `POST /api/friends/requests` (`friends_send_request`, body `FriendRequestIn {username}`), `POST /api/friends/requests/{username}/accept` (`friends_accept_request`), `DELETE /api/friends/requests/{username}` (`friends_delete_request`: decline and cancel), `DELETE /api/friends/list/{username}` (`friends_remove`).
  - `blocks.rs`: `PUT` and `DELETE /api/friends/blocks/{username}` (`friends_block`, `friends_unblock`).
  - `out.rs`: `FriendsOut`, `FriendOut`, `FriendReadingOut`, `FriendRequestOut`, `BlockedOut`, `FriendClubInviteOut` (§4.5), and `friends_out(conn, viewer)`, which every route answers with. Five queries, no N+1: the friends (`bc_friends`, by username), the currently-reading previews (the windowed query of §4.5 through `bc_visible_shelf`, then `book_outs`), both directions of the pending requests in one query (hidden ones left out of `incoming`; a picture only where `bc_can_see_user`), the blocks, and the open bound invites to clubs the caller is not in.
- **Every transition of §4.2 and §4.3** with the answers of the tables there and of §4.9's table for an unknown name. Unblock always answers 200 with `FriendsOut`.
- **Rate limit** (`server/src/ratelimit.rs`): `friend_request_user: SlidingWindow::new(20, DAY)`, keyed `u:<id>`, 429 `Slow down a little. Try again later.` with `Retry-After`; in `thresholds_match_the_table`. `send` counts every request that reaches the handler (a body that fails validation never does), before any lookup. `block` counts an unknown name and every block of an account the caller has no `friendships` row with (checked under the pair's locks); the caller's own name (400) is not counted.
- **Error strings** (`server/src/error.rs`, `msg`, add-only): `NO_ACCOUNT` (moved from `admin::users`, which now uses `msg::NO_ACCOUNT`), `SELF_ACCOUNT`, `YOU_BLOCKED`, `NO_FRIEND_REQUEST`, `NO_FRIEND`, `FRIEND_IN_CLUB`, `ONLY_FRIEND_INVITE_NAMES` (`Only a friend invite names a person`). `No such invite` stays `clubs::invites::NO_SUCH_INVITE`.
- **Friend invites into clubs** (§4.6):
  - `clubs/invites.rs`: `InviteKind::Friend`; `InviteIn.username` (normalised; required for `friend`, 400 `Field required`; refused for the other kinds, 400 `Only a friend invite names a person`); `create_invite` defaults `friend` to one use and 30 days (`expires_in_days` 1–30), and under the club lock runs `friends::locked_friend` (404 `No friend with that name`), then the membership check (409 `That friend is already in this club`), then answers with the caller's open friend invite for that friend when there is one, else inserts through the new `insert_bound_invite` (`insert_invite` is its wrapper without an invitee). `consume` adds `AND (invitee_id IS NULL OR invitee_id = $2)` with the redeeming account's id. The module docs and both utoipa descriptions say so.
  - `clubs/out.rs`: `InviteOut` gains `personal` (a bound invite) and `invitee` (the friend's username, for the invite's creator only). `club_invite_outs`/`club_invite_out` take the viewer; `invite_outs`/`invite_out` keep their signatures for `admin/invites.rs` (sign-up invites, never personal) and pass no viewer.
  - `clubs/members.rs`: `revoke_known_invites` adds `OR invitee_id = $2`; the descriptions of `remove_member` and `Known::Own` say so.
- **Tests:**
  - `server/tests/friends_requests.rs` (9): every row of §4.2 (send, repeat, accept, mutual send, decline, cancel, remove and every 404), the race of two tasks sending to each other (five rounds, one accepted row each), normalisation (`" Ada "`, `%20BOB%20`), a NUL name in the path (404) and in the body (400, deviation 1), a deleted account, the 21st request in a day (unknown names, repeats and a block counted; accept, decline, remove and unblock not), and a suspended friend staying a friend (answer 27).
  - `server/tests/friends_blocks.rs` (7): a block in each pair state; a request after the block, compared field by field with one nobody answers; 409 `You blocked this account`; unblock dropping the hidden request and its identical answers (an unblocked name, an unknown one, NUL, the caller); 25 requesters blocked in a day for free, then the budget counted from zero; the pair's friend invites revoked both ways; the clubs untouched, and a later join through a third admin's friend invite making the blocked account `Audience` again (answer 21); blocking back.
  - `server/tests/friends_invites.rs` (12): the acceptance's end to end; 403 for a member; the same 404 for a stranger, an unknown name, the caller, both sides of a pending request and a block either way, also for the blocked account's own attempt; the body rules; 409 for a member; a second invite answering the first (and a co-admin's being their own, deviation 2); another account's accept and a sign-up refused with the code; the 20-member cap; decline, unfriend (either side), block and a removal from the club revoking; 21 days valid, 31 days not; `invitee` for the creator only; and an invite waiting for an unfriend that holds the friendship's row (the `FOR SHARE`).
  - `server/tests/friends_visibility.rs` (+3, the `GET /api/friends` cases of §4.11): requesters' pictures only where `bc_can_see_user`; nobody else's lists (also with `?username=`, which the route ignores); three current books in the friend's shelf order with the count and progress, no "only me" take.
  - `server/tests/clubs_invites.rs`: `personal` and `invitee` in the one exact `InviteOut`.
  - The matrix (`server/tests/parity_visibility.rs`): `Kind::Friends` (label "Friends") with its five cases: the owner's four lists; each other viewer's lists hold only their own rows (the friend sees the owner, the two sides of a pending request their request, the blocked account nothing), and the `{username}` routes about the owner change nothing for a viewer without a row; an unknown name answers like a name without a row on accept, decline, remove and unblock, and the block alone gives 404 `No account with that name`. `USER_COLUMNS`: `friendships.requester_id`, `friendships.addressee_id` and `user_blocks.user_id` are `Kinds(&[Kind::Friends])`. `USERNAME_ROUTES` gains the five routes of §4.8 (16 rows). The checklist's #46 table has the row "Friends" and says 16 routes.
  - `server/tests/common/visibility.rs` and the helpers of `friends_visibility.rs` now make the friendships, the unfriend and the block through the routes (part 1's deviation 4 is gone).
- `server/.sqlx/` regenerated on a freshly reset database; `frontend/src/api/generated.ts` regenerated; `frontend/src/test/fixtures.ts` `invite()` gains `personal: false` and `invitee: null` (add-only).
- **No doc or mapping line was needed** for `openapi_contract.rs` (no §0 DTO changed; the new schemas are module-local) or `api_error_copy.rs` (it checks mapped strings against the server, and part 2 maps nothing; `No account with that name` stays a literal, now in `msg`).

### Decisions

- **The pair's turn.** Every write about a pair (send, accept, decline or cancel, remove, block, unblock) first takes `pg_advisory_xact_lock(0x6263_0300, hashtext('<lower id>:<higher id>'))`, beside ingest's `0x6263_0100` and the reports' `0x6263_0200`. Without it, a request and a block by the same account could interleave (the block's `DELETE` does not see the request's uncommitted row, so a pending request would survive next to its sender's own block), and an unblock could miss a request sent at the same moment and surface it. The plan's `INSERT … ON CONFLICT DO NOTHING`, then `FOR UPDATE`, stays as written.
- **Both accounts' rows `FOR KEY SHARE`** come next, in id order. An account deletion locks its row `FOR UPDATE` before its cascade reaches `friendships`, `user_blocks` and `club_invites`, in an order of its own; a pair write that held the pair's row and then waited for an invite row the cascade held would deadlock with it. With the account rows first, the deletion either finished before (the write answers as for an unknown name) or waits for the write. This replaces the plan's mapping of a foreign-key violation to 404 `No account with that name`: the answer is the same, and the violation can no longer happen.
- **Lock order**, then: the pair's turn → both accounts' rows → the `user_blocks` row → the pair's `friendships` row → the pair's invite rows (the plan's order for a block, after the first two). No pair write locks a club.
- **A friend invite** takes the club (with the caller's role re-read) → the friend's account row `FOR KEY SHARE` → the pair's accepted `friendships` row `FOR SHARE` → the membership check → the insert. The account row comes before the friendship for the deletion's sake, as above; `consume` takes the inviter's row the same way. The `FOR SHARE` conflicts with an unfriend's or a block's `DELETE`, so an invite cannot slip past either (`friends_invites.rs::an_invite_waits_for_an_unfriend_in_flight`). A block always deletes the friendship, so an accepted row is also proof that neither blocked the other.
- **`accepted_at = clock_timestamp()`**, not `now()`: the other side's request can have been created after the accepting transaction began (it waited for the pair's turn), and `friendships_accepted_after` would refuse `now()`.
- **`PathName`**, the `{username}` extractor of the friend routes: normalised with `normalize_username`; a NUL character or a path that is not UTF-8 is `None`, which every route answers as an unknown name (the club extractors' rule for `{slug}`, conventions §4.5), never with `ApiPath`'s 400.
- **Revocations** by an unfriend or a block (`revoke_pair_invites`) and a decline touch only open invites (not revoked, expired or used up), like `revoke_known_invites`: a used invite keeps its history in the club's invite list. A decline needs no club lock (one row); declining twice is 200; an invite made for someone else, or an ordinary one, is 404 `No such invite`.
- **`FriendClubInviteOut.expires_at` is never null:** the query takes `expires_at > now()`, and `kind: "friend"` always sets 1 to 30 days.
- **Order of ties:** `incoming` and `outgoing` by `created_at DESC`, then username; `blocked` the same; `club_invites` by `created_at DESC, id DESC`, like the club's invite list.
- **The friend route answers are read inside the write's transaction**, before the commit, so a write's answer always shows its own result.

### Deviations from the plan

1. **A NUL character in the request body is 400 `Invalid value`**, not 404 `No account with that name` (§4.2's table, §4.11). `ApiJson` refuses U+0000 anywhere in a body before the handler runs (conventions §4.3: the only JSON extractor). No account can have such a name, so the 400 discloses nothing. In the path, a NUL name is unknown, as planned (`PathName`).
2. **A second friend invite answers with the caller's own open invite only** (§4.6: "an active bound invite for the same friend and club"). Answering with a co-admin's open invite would tell the caller whom that admin is friends with, which answer 24 keeps from them. A co-admin who invites the same friend gets an invite of their own, so the friend may see two invitations into one club, one from each admin.
3. **Two locks the plan does not give** (the pair's turn and the account rows), see "Decisions". The plan's foreign-key mapping is replaced by the account rows' lock read, with the same 404.
4. **`accepted_at` is `clock_timestamp()`** (§4.2 writes `now()`), see "Decisions".
5. **An unfriend and a block revoke only open invites**; §4.3's SQL revokes every invite that is not yet revoked, used ones included.
6. **`invite_outs` and `insert_invite` keep their signatures**: `admin/invites.rs` (not a FRIENDS file) calls them. `club_invite_outs`, `club_invite_out` and `insert_bound_invite` are the new forms with a viewer or an invitee.
7. **Part 1's prose counts:** the checklist's #46 now says 16 routes, as the plan does.

### Texts for the integrator (part 2)

**`endpoint-map.md`, a new section "Friends (#57, owner FRIENDS)"** after §10 (or as §13 with the index):

| Path | Who | Request → response | Notes |
|---|---|---|---|
| `GET /api/friends` | signed in | → `FriendsOut` | The caller's own lists only; nothing about anyone else's friends, requests or blocks is ever served, and there are no counts. |
| `POST /api/friends/requests` | signed in | `FriendRequestIn {username}` → `FriendsOut` | none → pending (hidden from the addressee if they blocked the caller); pending either way or friends: a request back accepts, a repeat changes nothing. 400 `Field required`, `Invalid value` (NUL), `That is your own account`; 404 `No account with that name`; 409 `You blocked this account`; 429 `Slow down a little. Try again later.` |
| `POST /api/friends/requests/{username}/accept` | signed in | → `FriendsOut` | A pending request from `username` that the caller can see. 404 `No such friend request` otherwise. |
| `DELETE /api/friends/requests/{username}` | signed in | → `FriendsOut` | Declines a visible request from `username` or cancels the caller's own (hidden or not). No trace. 404 `No such friend request`. |
| `DELETE /api/friends/list/{username}` | signed in | → `FriendsOut` | Ends a friendship; the pair's open friend invites are revoked. 404 `No friend with that name`. Not `/api/friends/{username}`: names such as `requests` would collide. |
| `PUT /api/friends/blocks/{username}` | signed in | → `FriendsOut` | Silent. Deletes the pair's row in any state, revokes the pair's open friend invites; later requests from them are stored and hidden. 400 `That is your own account`; 404 `No account with that name`; 429. |
| `DELETE /api/friends/blocks/{username}` | signed in | → `FriendsOut` | Deletes the block and the requests they sent meanwhile. Always 200, for an unknown name too. |
| `DELETE /api/friends/club-invites/{invite_id}` | the invitee | → `FriendsOut` | Declines a friend invite into a club (revokes it if still open). 404 `No such invite` (also one made for someone else). |

- `{username}` is normalised like every username lookup; a NUL character or a path that is not UTF-8 counts as an unknown name (never 400).
- DTOs (none in §0; names unique crate-wide): `FriendsOut {friends: FriendOut[], incoming: FriendRequestOut[], outgoing: FriendRequestOut[], blocked: BlockedOut[], club_invites: FriendClubInviteOut[]}`; `FriendOut {user: UserSummary, since, currently_reading_count, currently_reading: FriendReadingOut[]}` (at most 3, shelf order, through `bc_visible_shelf`); `FriendReadingOut {book: BookOut, progress|null}`; `FriendRequestOut {user: UserSummary, created_at}` (`avatar_url` null unless `bc_can_see_user`); `BlockedOut {username, created_at}`; `FriendClubInviteOut {id, club: ClubRef, invited_by|null, code, created_at, expires_at}`; `FriendRequestIn {username}`.
- Order: `friends` by username; `incoming`, `outgoing` and `blocked` newest first (then username); `club_invites` newest first, only open invites to clubs the caller is not in.
- Rate limit: §10.1 `friend_request_user`.

**`endpoint-map.md` §3:**
- `GET /api/clubs/{slug}/invites`: `InviteOut` gains `personal: bool` (a friend invite) and `invitee|null` (the friend's username, only for the invite's creator; other admins see a personal invite without the name, answer 24).
- `POST /api/clubs/{slug}/invites`: `{kind: "single_use"|"link"|"friend", expires_in_days?: 1..30, username?}`. `friend`: one use, 30 days by default, bound to `username`, who must be the caller's friend (404 `No friend with that name`, also for an unknown name, the caller, a pending request and a block either way) and not a member (409 `That friend is already in this club`). `username` is required for `friend` (400 `Field required`) and refused for the others (400 `Only a friend invite names a person`). While the caller's friend invite for that friend is still open, a second call answers 201 with it.
- `POST /api/invites/{code}/accept` and sign-up: a friend invite is redeemable only by its invitee; anyone else, and every sign-up, gets 400 `That invite code is not valid`. The preview (`GET /api/invites/{code}`) is unchanged.
- `DELETE /api/clubs/{slug}/members/{username}`: the removal also revokes the open friend invites made for the removed member.

**`endpoint-map.md` §11.3, a "Phase 3 (FRIENDS, #57)" table** (the SPA part maps them in `apiErrors.ts`, en and de):

| String | Where | Constant |
|---|---|---|
| `No account with that name` | 404, friend request, block (and, as before, suspend and unsuspend) | `msg::NO_ACCOUNT` (moved from `admin::users::NO_ACCOUNT`; already mapped to `errors.noAccount`) |
| `That is your own account` | 400, friend request and block | `msg::SELF_ACCOUNT` |
| `You blocked this account` | 409, friend request | `msg::YOU_BLOCKED` |
| `No such friend request` | 404, accept, decline, cancel | `msg::NO_FRIEND_REQUEST` |
| `No friend with that name` | 404, remove, friend invite | `msg::NO_FRIEND` |
| `That friend is already in this club` | 409, friend invite | `msg::FRIEND_IN_CLUB` |
| `Only a friend invite names a person` | 400, `POST /api/clubs/{slug}/invites` | `msg::ONLY_FRIEND_INVITE_NAMES` |

The ADM2 row for `No account with that name` changes its constant to `msg::NO_ACCOUNT`.

**`conventions.md` §5.3 (clubs and invites):**
- `consume`'s atomic `UPDATE` also requires `invitee_id IS NULL OR invitee_id = <the redeeming account>`; sign-up passes the new account's id, so a bound code never signs anyone up.
- "**Friend invites** (FRIENDS): `kind: "friend"` inserts a single-use invite with `invitee_id` (`clubs::invites::insert_bound_invite`). Lock order: the club (`lock_as_admin`), the friend's `users` row `FOR KEY SHARE`, the pair's accepted `friendships` row `FOR SHARE` (`friends::locked_friend`), then the insert. The share lock conflicts with an unfriend's or a block's `DELETE`, which then revoke the new invite or make it wait for them."
- The removal paragraph: `revoke_known_invites` also revokes the open friend invites made for the removed person.

**`conventions.md`, a new subsection "Friends" (in §5, after §5.3, or §6.6):**
- One row per pair; a block is its own row and deletes the pair's row; hidden requests (from an account the addressee blocked) stay, show as sent, and are left out of the addressee's lists, badge and actions.
- "**Lock order of a pair write** (send, accept, decline or cancel, remove, block, unblock; `friends::lock_pair`): the pair's turn, `pg_advisory_xact_lock(0x6263_0300, hashtext('<lower id>:<higher id>'))` (beside ingest's `0x6263_0100` and the reports' `0x6263_0200`) → both accounts' `users` rows `FOR KEY SHARE`, in id order (a deletion locks its row `FOR UPDATE` before its cascade, so it either finished or waits; a gone account answers as an unknown name) → the `user_blocks` row → the pair's `friendships` row → the pair's invite rows. No pair write locks a club; a friend invite takes the club first and then the friend's row and the friendship (§5.3)."
- `accepted_at` is `clock_timestamp()` (the other side's request may be newer than the transaction).
- Send: `INSERT … ON CONFLICT ((least(…)), (greatest(…))) DO NOTHING`, then the pair's row `FOR UPDATE`: a request back accepts.
- Unfriend and block revoke the pair's open friend invites (`friends::revoke_pair_invites`: bound to one, created by the other, not revoked, expired or used up).
- `{username}` of the friend routes: `friends::PathName`, normalised; NUL or non-UTF-8 is an unknown name (the NUL paragraph of §4.5 gains "and the friend routes answer such a `{username}` as an unknown name").

**`conventions.md` §10.1:** the `RateLimits` struct line gains `pub friend_request_user`, and the table a row: "`friend_request_user`: every `POST /api/friends/requests` per account (`u:<id>`), unknown names and repeats included, and every `PUT /api/friends/blocks/{username}` unless the caller already has a `friendships` row with that account (a block of a requester is free) | 20 per day (429 `Slow down a little. Try again later.`)".

**`conventions.md` §6.5:** `Viewers::new` makes the friendships, the unfriend and the block through the friend routes. The matrix's kinds gain `Friends` (`GET /api/friends` and the five `{username}` routes): 34 columns, 16 routes, 11 schemas.

**`docs/plan.md:31`:** "They see each other's reading and can invite each other into clubs they administer."

**`CLAUDE.md`:** nothing new for part 2 beyond part 1's TRACES wording (the SPA part adds its `/friends` line).

**For #53 (facts only):** a friend request names the account by its exact username, and the server says whether such an account exists (as sign-up does), at most 20 times a day per account; a block is never told to the blocked account, whose requests are kept but not shown; a friend invite into a club names the friend only to the admin who made it.

## Part 3: the SPA

### What part 3 built

- **`frontend/src/api/client.ts`** (add-only): `FriendsOut` in the import list and eight calls after `clubStats`: `friends`, `sendFriendRequest`, `acceptFriendRequest`, `deleteFriendRequest` (decline and withdraw), `removeFriend`, `blockAccount`, `unblockAccount`, `declineClubInvite`. Friend invites go through the existing `createInvite(slug, {kind: "friend", username})`. `types.ts` is unchanged: the new code imports `FriendsOut` and the rest from `api/generated` directly (its header allows that).
- **`frontend/src/stores/friends.ts`** (Pinia `friends`):
  - state `data: FriendsOut | null`, `owner` (the account id the lists belong to), `failed`, `loadedAt`, `answers`;
  - getters `lists` (the lists, or null when they belong to another account) and `badge` (`incoming.length + club_invites.length`);
  - actions `load()`, `loadIfStale()`, and one per route (`send`, `accept`, `deleteRequest`, `remove`, `block`, `unblock`, `declineClubInvite`), each replacing the lists with the answer (`take`);
  - `reloadWhenVisible()`: a `visibilitychange` listener that calls `loadIfStale()` (at most once a minute, `RELOAD_AFTER_MS`), returning its remover.
  - A failed `load()` keeps the lists and sets `failed`; only the page reads it, and only when it has no lists. The badge never shows an error.
- **`frontend/src/layouts/AppShell.vue`**: `load()` on mount, `reloadWhenVisible()` while mounted, and a load when the set of the viewer's clubs changes (see "Decisions"). The sidebar's Friends item carries a count pill with a visually hidden "N requests or invitations waiting" (the pattern of `AdminLink.vue`).
- **`frontend/src/nav.ts`**: `NavIconName` gains `"friends"`; the item `{to: "/friends", labelKey: "nav.friends", icon: "friends", bottomBar: false}` after Club. **`NavIcon.vue`**: a person and a heart.
- **`frontend/src/router.ts`**: `{path: "friends", component: FriendsPage}` (lazy, before `settings`) replaces the redirect to `/club`; `/friends/:username` → `/u/:username` stays.
- **`frontend/src/pages/FriendsPage.vue`** at `/friends`, top to bottom as §4.10 lists it:
  1. "Add a friend": the username field, "Type their exact username.", "Send request". Errors stay under the field (`No account with that name`, `You blocked this account`, `That is your own account`, 429). The toast says what became of it: "Request sent to <name>", "You and <name> are friends now" (their request was waiting), or "<name> is already your friend".
  2. "Requests to you": avatar (null unless visible, as the server sends it), name, "asked <day>", Accept, Decline, Block (confirmed in a sheet).
  3. "Invitations to clubs": "<name> invited you to <club>" ("You were invited to <club>" when the inviter is gone), "valid until <day>", Join (a link to `/join/<code>`) and Decline.
  4. "Your friends": avatar, name (link to `/u/<name>`), "Friends since <day>", up to three current books (cover, title, progress) and "+N more"; an Options sheet per friend with "Invite to a club…" (only when the viewer owns or runs a club), "Remove from friends" and "Block", each of the last two confirmed in the sheet.
  5. "Requests you sent": name, "sent <day>", Withdraw.
  6. "Blocked": name, "blocked <day>", Unblock, and the line of §4.10 ("Blocking ends the friendship and hides their requests. It does not hide anything in clubs you share now or join later. …"). The block confirmation says the same in short: "<name> is not told. … It does not hide anything in clubs you share: there you still see each other." (answer 21).
  - An empty line for every list; a skeleton until the first answer; "Could not load your friends" with "Try again" when the first load fails (the add form still works).
- **`frontend/src/components/InviteFriendSheet.vue`**: prop `username`; lists the clubs of `UserOut.clubs` where the viewer is owner or admin, one Invite button each (`kind: "friend"`), "Invitation sent" in the club's row, the translated 404 `No friend with that name` or 409 `That friend is already in this club` in the club's row; "You do not run a club. …" without such a club.
- **`frontend/src/pages/HomePage.vue`**: a compact Friends row after the clubs (or the solo prompt), a link to `/friends`: the count pill when requests or invitations wait, "Add friends" when the viewer has no friends, else just the arrow. Home asks nothing itself; the shell loads the lists.
- **`frontend/src/pages/club/ClubMembersPage.vue`**: a bound invite's line starts with "Personal invite for <name>" for its creator and "Personal invite" for other admins, in place of "Single use"; and a layout fix (below).
- **i18n** (`en.json`, `de.json`): `nav.friends`; `friends.*` (title, lede, `add.*`, `incoming.*`, `invites.*`, `list.*`, `outgoing.*`, `blocked.*`, `block.*`, `remove.*`, `inviteSheet.*`, `waiting`, `homeAdd`, `loadFailed`, `actionFailed`); `invites.personal`, `invites.personalFor`; `errors.selfAccount`, `errors.youBlocked`, `errors.noFriendRequest`, `errors.noFriend`, `errors.friendInClub`, `errors.onlyFriendInviteNames`. `errors.noMember` was changed in part 1.
- **`frontend/src/i18n/apiErrors.ts`** (add-only, a "Friends" group before "Catalog"): `That is your own account`, `You blocked this account`, `No such friend request`, `No friend with that name`, `That friend is already in this club`, `Only a friend invite names a person`. `No account with that name` was mapped already (`errors.noAccount`, ADM2). `server/tests/api_error_copy.rs` finds each in `error.rs`.
- **Tests:**
  - new: `stores/friends.test.ts` (6: the badge, replacing the lists after every action, a failed load, one load at a time and no older lists over a write's answer, another account on the tab, the visibility reload at most once a minute and its remover), `pages/FriendsPage.test.ts` (8), `components/InviteFriendSheet.test.ts` (3), `layouts/AppShell.test.ts` (3: the sidebar item after Club with its count and outside the tab bar, no count without anything waiting or after a failed load, the reloads on visibility and on a change of clubs, none after a sign-out or an unmount);
  - changed: `nav.test.ts` (the item, the five tabs), `router.test.ts` (`/friends` is the page, with and without clubs; `/friends/grace` still leads to `/u/grace`), `pages/HomePage.test.ts` (the row's three states), `pages/club/ClubMembersPage.test.ts` (personal invites for the creator and another admin), `pages/LegalPage.test.ts` (a `friends` mock answering empty lists), `acceptance.test.ts` (a `GET /api/friends` handler in both fake servers, and a third case, below);
  - fixtures: a new `frontend/src/test/friends.ts` (`friendsOut`, `friendOut`, `reading`, `friendRequest`, `blockedOut`, `friendClubInvite`).
  - The suite: 75 files, 500 tests (71 and 475 before part 3).

### Decisions

- **The lists know their account.** `session.ts` (not a FRIENDS file) clears the clubs on a sign-in or sign-out, but knows nothing of this store. So the store records the account that asked, and `lists` (and with it the badge and the page) is null for any other account: after a sign-out and another sign-in on the same tab, the previous account's requests never flash up while the new load runs.
- **One load at a time, and a write's answer wins.** The shell and the page both load on mount; the second call joins the first. Every answer the store takes raises `answers`; a load that began before a write's answer was taken drops its own (older) answer.
- **A change of clubs reloads the lists** (a watcher in `AppShell.vue` on the sorted slugs of `UserOut.clubs`, not on a sign-out). `club_invites` only lists clubs the caller is not in, so a join (also with a code on `/clubs`), a leave or a new club changes what waits. Joining through `/join/<code>` leaves the shell (the join page is outside it), so the shell's mount loads again anyway; the watcher covers the changes inside the shell. Not in §4.10, which names mount and visibility only.
- **Requesters are not links.** A requester is usually not audience (no shared club), so `/u/<name>` would answer 404; friends link to `/u/<name>`.
- **The Home row** has three states (§4.10: "the badge count when there are requests or invitations, and 'Add friends' with none"): the count when something waits, "Add friends" when the viewer has no friends, else only the arrow. With friends and nothing waiting, "Add friends" read as if the list were empty.
- **Confirmations** use one sheet that switches between the friend's options and the confirmation of a removal or a block (keyed by mode, so focus starts on the first button of each). A failed removal or block keeps the sheet open, with the reason in a toast.
- **Dates** are days in the viewer's own zone (`UserOut.timezone`), like the rest of the personal pages.
- **The block confirmation** is shown for a block from a request too (§4.10 asks for it there).

### Deviations from the plan

1. **"+N more" links to `/u/<name>`**, not `/u/<name>/shelf` (§4.10). In the code, `/u/<name>/shelf` is the physical shelf (`MemberPhysicalShelfPage.vue`, the spines on their boards); `/u/<name>` is the shelf list (`MemberShelfPage.vue`) with its status filter, where the other books in progress are listed. The intent (see the rest of what they are reading) is kept.
2. **Fixture helpers in a new `frontend/src/test/friends.ts`**, not in `fixtures.ts` (§2.1 rule 2 allows add-only lines there). That keeps `fixtures.ts` at part 2's two lines, away from SCAN's `isbnHit()`.
3. **A new `frontend/src/layouts/AppShell.test.ts`.** The shell had no test of its own; §2.2 asks for the shell's changes "with their tests". `LegalPage.test.ts` and `acceptance.test.ts` mount the shell too and got their mock and handlers.
4. **`acceptance.test.ts` gains a third case**, the SPA half of the acceptance in the fake server: Home's row counts a request and an invitation, `/friends` accepts the request, the invitation's Join link is `/join/<code>`, the join page joins, and back in the shell the count is gone; `server.unknown` is empty. In the second case (two clubs) `server.unknown` was not checked before and is not empty (`GET …/vote` and `…/vote/suggestions` of one club have no handler); it now checks that no `/api/friends` request is unknown, rather than growing that case's fake server.
5. **A layout fix on the members page outside the "for <name>" hunk.** At 360 px a member card with three current covers and the Manage button needs about 358 px in a 328 px column, so the page scrolled sideways (found in the browser run; not caused by friends, any co-member reading three books does it). `.member-link` now wraps and `.member-meta` has a basis of 7rem, so on a phone the covers go below the name.

### The browser run (8094, synthetic accounts)

The server from this branch with the built SPA (`STATIC_DIR=../frontend/dist`, `DEV_MODE=true`, Postgres on 5462), three accounts made with `beta-invite` codes and verified through the logged mails, two browser sessions (`playwright-cli`), mostly at 360 × 780:
- the second account sent " Ada " from `/friends` ("Request sent to ada"); the third asked through the API;
- the first account's Home row showed "2"; on `/friends` it accepted the second ("You and grace are friends now") and blocked the third through the sheet (the copy of answer 21 shown); the third asked again, saw its request as sent, and the first account's lists after a reload showed it only under Blocked;
- the friend card showed three current books with progress and "+2 more";
- Options → "Invite to a club…" → Invite showed "Invitation sent"; the friend's `/friends` showed "ada invited you to <club>", Join led to `/join/<code>`, the join page joined, the friend appeared on the members page, the creator's members page showed the used invite as "Personal invite for grace", and the friend's Home row no longer counted it;
- at 1280 px the sidebar's Friends item showed its count;
- `document.documentElement.scrollWidth` was 360 on Home, `/friends` (English and German, with a 21-character username) and the members page (after deviation 5); both consoles had no error besides the signed-out `GET /api/auth/me` 401 before sign-in.

### Texts for the integrator (part 3)

**`CLAUDE.md`, "Frontend (`frontend/src/`)"**, after the "Pages map 1:1 …" bullet:
- "Friends (#57): `/friends` (`pages/FriendsPage.vue`) holds the friends, requests both ways, invitations into clubs and blocks; `stores/friends.ts` keeps `GET /api/friends` (every friend route answers with the whole lists). The shell loads it on mount, when the tab comes back after a minute and when the viewer's clubs change, and shows what waits (requests and invitations) on the sidebar's Friends item and on Home's Friends row. Friends is a sidebar-only nav item (`bottomBar: false`): the phone's tab bar keeps five tabs. `components/InviteFriendSheet.vue` makes a personal invite (`kind: "friend"`) into a club the viewer runs."
- In the same bullet, "old paths redirect there" stays true: `/friends/:username` still redirects to `/u/:username`; only `/friends` itself is a page now.

**`endpoint-map.md` §11.3 (or the "Phase 3 (FRIENDS, #57)" table of part 2):** the SPA maps every new string, en / de:

| String | Key | en | de |
|---|---|---|---|
| `That is your own account` | `errors.selfAccount` | That is your own account | Das ist dein eigenes Konto |
| `You blocked this account` | `errors.youBlocked` | You blocked this account. Unblock it first. | Du hast dieses Konto blockiert. Gib es zuerst frei. |
| `No such friend request` | `errors.noFriendRequest` | That request no longer exists | Diese Anfrage gibt es nicht mehr |
| `No friend with that name` | `errors.noFriend` | None of your friends has that name | Keiner deiner Freunde hat diesen Namen |
| `That friend is already in this club` | `errors.friendInClub` | That friend is already in this club | Diese Person ist schon in diesem Club |
| `Only a friend invite names a person` | `errors.onlyFriendInviteNames` | Only a personal invite names a person | Nur eine persönliche Einladung nennt eine Person |
| `No account with that name` | `errors.noAccount` (ADM2) | unchanged | unchanged |

**`README.md`, the feature list**, a bullet after "Diary": "**Friends** — Ask someone by their exact username; once they accept, you see each other's shelves, ratings, quotes and notes, as club mates do, and what they are reading. Club admins invite a friend into a club with a personal invite only that friend can use. Blocking is silent and ends the friendship; it does not reach into clubs you share."

**`conventions.md`:** nothing for the SPA beyond parts 1 and 2.

**`docs/plan.md:31`:** part 2's sentence.

### Later ideas

- From the plan (answer 26): friend actions on a member's page (`/u/<name>`: "Ask to be friends", "Invite to a club…"), inviting a friend from a club's members page, a "Friends are reading" strip on Home.
- The count on the phone's top bar (the avatar) as well as on Home's row, if requests are missed on phones.
- The members page's invite blurb could mention personal invites ("To invite one friend, use their options on your Friends page").
- Seen in the browser run, not FRIENDS': the members page calls a used invite "ended <expiry>", although it ended at its use, before the expiry.
- Push for requests and invitations waits for #60 (answer 25).
