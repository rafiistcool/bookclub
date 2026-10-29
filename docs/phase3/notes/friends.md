# FRIENDS (#57) implementation notes

Owner: FRIENDS. These notes record what the friends package built, where it decided something the plan (`docs/phase3/work-plan.md` §4, §6 answers 16–29) left open or where the code made it differ, and the texts the integrator moves into `docs/phase1/conventions.md`, `docs/phase1/endpoint-map.md`, `CLAUDE.md`, `README.md` and `docs/plan.md`. The design documents themselves are unchanged.

Status: **part 1 (data and visibility) done**: e5d86e3 (the schema and the account-reference rules), c06b067 (the visibility layer and its tests). **Part 2 (the API) done**: 13ceec5 (the friend and block routes, the rate limit, the matrix's `Friends` kind), a888202 (friend invites into clubs); below, "Part 2: the API". **Part 3 (the SPA) done**: `/friends`, the store and its badge, the invite sheet, the Home row, personal invites on the members page; below, "Part 3: the SPA". **The review round** fixed what the FRIENDS review found; below, "Review round". **The checks round** fixed what the E2E run and the security review found; below, "Checks". Before its final checks the branch still merges `phase-3` once COVER is in it (`work-plan.md` §2.1 rule 1).

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
5. **The purge locks the clubs of the invites bound to an account before the account** (not in the plan), see the next section (rewritten in the review round: the first argument, that it could not deadlock, was wrong).

### Lock order of the purge

An empty sign-up may now hold friend invites bound to it (`club_invites.invitee_id` is a trace, §4.7; an unfriend revokes such an invite but leaves its row). `delete_user`'s `lock_clubs` locks their clubs. Before the review round the purge locked the account first, so these clubs came after the account, the reverse of every membership write (conventions §5.3, §5.6), and it deadlocked with an admin of such a club asking for a friend invite for that name: `create_invite` holds the club and `friends::locked_friend` then takes the account's row `FOR KEY SHARE` before it looks at the friendship (the argument that "a new invite needs an accepted friendship" missed that order).

Now `purge_empty_sign_ups` takes, per account:
1. the clubs of the invites bound to it, `FOR UPDATE` in id order;
2. the account's row, then its sessions, both `FOR UPDATE`;
3. `is_empty_sign_up` again, in a new statement;
4. a check, in a new statement, that no invite bound to the account sits in a club step 1 did not lock. Such an invite can only be added while the account's row is free and needs an accepted friendship, which is data; should one appear between steps 1 and 2 and its friendship end again, the account is left to the next run;
5. `delete_user`, whose `lock_clubs` re-takes the clubs of step 1 and finds no other (an empty account has no other row in a club).

So the purge never locks a club after the account. Tests: `jobs.rs::the_purge_locks_a_bound_invites_club_before_the_account` (an admin holding the club reads the account while the purge waits: no deadlock, the account goes) and `a_bound_invite_made_while_the_purge_waits_defers_the_account` (step 4).

### Texts for the integrator

**`conventions.md` §5.6 (account deletion):**
- In the lock order paragraph, the list of rows becomes: "memberships (also those the user invited, `invited_by`), removals, clubs the user created (`clubs.created_by`), invites (created by the user or bound to them, `invitee_id`) and invite uses, picks, milestones, posts, reactions, nominations, ballots and events". Add: "Friendships and blocks reference no club and cascade without a lock."
- Step 4's cascade list gains "friendships and friend requests (both directions), blocks (both directions), and the friend invites bound to the user (`club_invites.invitee_id`; ON DELETE CASCADE, never SET NULL, which would turn a bound invite into an open code)".
- "A second caller" paragraph: replace "Its lock order is the user row, then the user's sessions (both `FOR UPDATE`), as in a password change and a reset; it takes no club row first, because an empty account has no row in any club and none can arrive while its row is locked, so `lock_clubs` locks nothing (`delete_user`'s doc comment says so)." with: "Its lock order is the clubs of the friend invites bound to the account (`FOR UPDATE`, in id order: an empty account has no other row in a club, but another account's friend invite for it is a trace, §5.7), then the user row, then the user's sessions (both `FOR UPDATE`), as in a password change and a reset. After its second look it checks that no invite bound to the account sits in a club it did not lock first (one can only be added while the account's row is free, and needs a friendship, which is data); otherwise the account waits for the next run. So `lock_clubs` takes no club after the account: an admin who holds such a club and then reads the account (a new friend invite for it) is waited for, not deadlocked with (`jobs.rs::the_purge_locks_a_bound_invites_club_before_the_account`, `a_bound_invite_made_while_the_purge_waits_defers_the_account`)."

**`conventions.md` §5.7 (export):**
- Table rows after `invite_uses`: `club_invites_received` (club name, who invited, created, expires, used, revoked; never the code) ← `club_invites.invitee_id`; `friends` (`{friends: [{username, accepted_at}], requests_sent: [{username, created_at}], requests_received: [{username, created_at}]}`) ← `friendships.requester_id`, `friendships.addressee_id`; `blocks` (`[{username, created_at}]`) ← `user_blocks.user_id`.
- `invites_created`: "the whole row minus `created_by` and `invitee_id`, with `invitee` (the username of a friend invite's invitee)".
- The shapes sentence: "`friends` is an object of three lists, like `reports`".
- "Both directions of the friendships are listed, requests hidden by the user's own block included: each row is the user's own act or one addressed to them."
- Exempt: "`user_blocks.blocked_user_id` (who blocked the account; telling it would undo the block, Art. 15(4), like the reporter of a report)."
- The pinned count: "34 since FRIENDS (`friendships.requester_id`, `friendships.addressee_id`, `user_blocks.user_id`, `user_blocks.blocked_user_id`, `club_invites.invitee_id`)".
- The purge paragraph: "or a row in `TRACES` with the reason when it is a trace of signing up or in, or of another account's act the account never answered (today `sessions.user_id`, `email_tokens.user_id`, `club_invite_uses.user_id` of a sign-up invite, `friendships.addressee_id` of a pending request, `user_blocks.blocked_user_id`, `club_invites.invitee_id`). `friendships.addressee_id` is in both lists: an accepted request is data."
- The `reports` paragraph (`about_me`): replace "only club mates can report a profile picture, so in a club of 2 to 5 any of them can name the reporter" with "only club mates and friends can report a profile picture, so in a club of 2 to 5, or between friends, any of them can name the reporter" (added in the review round; the comment in `auth::export` and the doc comment of `about_me_holds_only_removals_never_who_reported_or_when` say the same).
- §5.7's "Clubs the user no longer belongs to" and §5.5 are unchanged.

**`conventions.md` §6.1:** the `Audience` row: "they share at least one club or are friends (an accepted `friendships` row; a pending request grants nothing)". Add after "Club data is visible to the club's members only": "A friend is audience for personal data only: club data (readers, overlap, suggestions, members' previews, club stats, the club feed and club posts) stays the club members'."

**`conventions.md` §6.2:** a row for `bc_friends(viewer)` → `(user_id, since)`: "the viewer's friends and when each friendship began (`accepted_at`) | friend lists, the home feed's friend branch". `bc_visible_owners`: "the viewer and everyone in their audience (co-members and friends, each once)". `bc_can_see_user`: unchanged use; its body has the friend branch in the explicit pair form (never `least`/`greatest`, which ignore NULLs). Add: "`visibility_policy.rs::policy_functions_still_inline` checks with EXPLAIN that the set-returning functions are still inlined."

**`conventions.md` §6.3** (`author_avatar_url` on posts, added in the review round): "(`POST /api/media/{public_id}/reports` takes the profile picture of someone the reader shares a club with or is friends with; …)".

**`conventions.md` §6.4:** "**Home feed** = the union of the viewer's club feeds, plus the viewer's own personal events, plus the personal events of the viewer's friends created at or after the friendship began (`bc_friends(viewer).since`), deduplicated by id. The friend branch sits inside the personal-event group, so a friend's club events reach only that club's members. This is the one place outside the policy functions that reads friendships (migration 3100's header)." The `?username=` filter follows: a friend's name resolves (`bc_can_see_user`).

**`conventions.md` §6.5:** "viewer types: self, co-member, friend (in no club), member of another club, stranger (in no club, a friend of the friend), former co-member, both sides of a pending friend request, former friend, blocked friend". The matrix has five cases (the test keeps the name `every_personal_data_kind_has_its_four_viewer_cases`, which the checklist names). Record the amendment of the W1-frozen `tests/common/visibility.rs`: `Viewers` gains `friend`, `asked`, `requested`, `exfriend` and `blocked`; `audience()` is `[co_member, friend]`; `strangers()` adds the last four.

**`conventions.md` §10.1:** part 2 (the request limit).

**`CLAUDE.md`, "References to accounts and pictures":** "…or a `TRACES` row with the reason (a trace of signing in or another account's act; `every_reference_to_an_account_is_data_or_a_trace_of_signing_in`)". The "A new foreign key to `users`, …" sentence stays.

**`endpoint-map.md`:** part 2 (the Friends section, `kind: "friend"`, `InviteOut.invitee` and `personal`, the new strings). From part 1 the three changed descriptions (`activity_home_feed`, `auth_delete_me`, `media_report`), the `errors.noMember` copy, and two texts (added in the review round): `POST /api/media/{public_id}/reports`, "taken when the reporter can see the owner (`bc_can_see_user`: they share a club or are friends, or it is their own)"; the posts notes, "`author_avatar_url` only when the viewer can see the author (`bc_can_see_user`: they share a club or are friends, or it is the viewer)".

**`conventions.md` §16** (the purge, added in the review round): "and no data (no avatar, and a `NOT EXISTS` for every foreign key to `users` except the traces of signing up and in and of other accounts' acts it never answered, §5.7). Each account goes in a transaction of its own: the clubs of the friend invites bound to it (`FOR UPDATE`, in id order), the user row, then its sessions, both `FOR UPDATE`, then `is_empty_sign_up` again in a new statement (…), then a check that no invite bound to it sits in a club not locked first (else it waits for the next run), then `auth::deletion::delete_user` (§5.6) …".

**For #53 (the privacy text), facts only:** friendships and requests are stored per pair with the time of the request and of the acceptance; a friend sees what a club mate sees of personal data (shelf, ratings except "only me", quotes, diary, stats, the home feed from the acceptance on), never club data; a block is stored, never told to the blocked account, and withheld from its export; all of it goes with either account; the export lists friendships, requests both ways, the account's own blocks and the friend invites it received. Added in the review round: friends may see and report each other's profile pictures, as club mates may (today `takedown.md:21`, `:40` and `:72` say "they share a club" and "only club mates can report a profile picture"); an empty sign-up may still hold friend requests it received and did not answer, other accounts' blocks of it and friend invites bound to it, which are deleted with it (today `privacy-facts.md:122` says it has "no row that references the account except its sessions, its mail links and the use of a sign-up invite").

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

- **The pair's turn.** Every write about a pair (send, accept, decline or cancel, remove, block, unblock) first takes `pg_advisory_xact_lock(0x6263_0300, hashtext('<lower id>:<higher id>'))`, beside ingest's `0x6263_0100` and the reports' `0x6263_0200`. Without it, a request and a block by the same account could interleave (the block's `DELETE` does not see the request's uncommitted row, so a pending request would survive next to its sender's own block), and an unblock could miss a request sent at the same moment and surface it. The plan's `INSERT … ON CONFLICT DO NOTHING`, then `FOR UPDATE`, stays as written. Tests (review round): `friends_blocks.rs::a_request_waits_for_its_senders_block_in_flight`, `an_unblock_waits_for_a_hidden_request_in_flight`.
- **Both accounts' rows `FOR KEY SHARE`** come next, in id order. An account deletion locks its row `FOR UPDATE` before its cascade reaches `friendships`, `user_blocks` and `club_invites`, in an order of its own; a pair write that held the pair's row and then waited for an invite row the cascade held would deadlock with it. With the account rows first, the deletion either finished before (the write answers as for an unknown name) or waits for the write. This replaces the plan's mapping of a foreign-key violation to 404 `No account with that name`: the answer is the same, and the violation can no longer happen. Test (review round): `friends_requests.rs::a_request_or_a_block_waits_for_a_deletion_in_flight`.
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
| `POST /api/friends/requests` | signed in | `FriendRequestIn {username}` → `FriendsOut` | none → pending (hidden from the addressee if they blocked the caller); pending either way or friends: a request back accepts, a repeat changes nothing. 400 `Field required`, `Invalid value` (NUL), `That is your own account`; 404 `No account with that name`; 409 `You blocked this account`; 429 `Slow down a little. Try again later.` (20 per account and 60 per address a day). |
| `POST /api/friends/requests/{username}/accept` | signed in | → `FriendsOut` | A pending request from `username` that the caller can see. 404 `No such friend request` otherwise. |
| `DELETE /api/friends/requests/{username}` | signed in | → `FriendsOut` | Declines a visible request from `username` or cancels the caller's own (hidden or not). No trace. 404 `No such friend request`. |
| `DELETE /api/friends/list/{username}` | signed in | → `FriendsOut` | Ends a friendship; the pair's open friend invites are revoked. 404 `No friend with that name`. Not `/api/friends/{username}`: names such as `requests` would collide. |
| `PUT /api/friends/blocks/{username}` | signed in | → `FriendsOut` | Silent. Deletes the pair's row in any state, revokes the pair's open friend invites; later requests from them are stored and hidden. 400 `That is your own account`; 404 `No account with that name`; 429 (counted with the requests unless a request or friendship exists). |
| `DELETE /api/friends/blocks/{username}` | signed in | → `FriendsOut` | Deletes the block and the requests they sent meanwhile. Always 200, for an unknown name too. |
| `DELETE /api/friends/club-invites/{invite_id}` | the invitee | → `FriendsOut` | Declines a friend invite into a club (revokes it if still open). 404 `No such invite` (also one made for someone else). |

- `{username}` is normalised like every username lookup; a NUL character or a path that is not UTF-8 counts as an unknown name (never 400).
- Accept, decline or cancel, remove and unblock are not counted. For an unknown name, or the caller's own, they take the turn of the caller and `friends::NO_ACCOUNT` (-1, an id no account has) and run the same statements, so neither the answer nor its timing tells an unknown name from an existing one (checks round).
- DTOs (none in §0; names unique crate-wide): `FriendsOut {friends: FriendOut[], incoming: FriendRequestOut[], outgoing: FriendRequestOut[], blocked: BlockedOut[], club_invites: FriendClubInviteOut[]}`; `FriendOut {user: UserSummary, since, currently_reading_count, currently_reading: FriendReadingOut[]}` (at most 3, shelf order, through `bc_visible_shelf`); `FriendReadingOut {book: BookOut, progress|null}`; `FriendRequestOut {user: UserSummary, created_at}` (`avatar_url` null unless `bc_can_see_user`); `BlockedOut {username, created_at}`; `FriendClubInviteOut {id, club: ClubRef, invited_by|null, code, created_at, expires_at}`; `FriendRequestIn {username}`.
- Order: `friends` by username; `incoming`, `outgoing` and `blocked` newest first (then username); `club_invites` newest first, only open invites to clubs the caller is not in.
- Rate limits: §10.1 `friend_request_user` and `friend_request_ip` (requests and blocks), `friend_invite_user` (friend invites).

**`endpoint-map.md` §3:**
- `GET /api/clubs/{slug}/invites`: `InviteOut` gains `personal: bool` (a friend invite) and `invitee|null` (the friend's username, only for the invite's creator; other admins see a personal invite without the name, answer 24). `used_by` of a personal invite is filled for its creator only too: other admins get `[]`, with `use_count` as it is (review round).
- `POST /api/clubs/{slug}/invites`: `{kind: "single_use"|"link"|"friend", expires_in_days?: 1..30, username?}`. `friend`: one use, 30 days by default, bound to `username`, who must be the caller's friend (404 `No friend with that name`, also for an unknown name, the caller, a pending request and a block either way) and not a member (409 `That friend is already in this club`). `username` is required for `friend` (400 `Field required`) and refused for the others (400 `Only a friend invite names a person`). While the caller's friend invite for that friend is still open, a second call answers 201 with it. Every `kind: "friend"` call counts against `friend_invite_user` (20 per account a day, unknown names and repeats included; 429 `Slow down a little. Try again later.`), so a friend's decline does not stop a new invite but the repeats are bounded, as for requests (checks round).
- `POST /api/invites/{code}/accept` and sign-up: a friend invite is redeemable only by its invitee; anyone else, and every sign-up, gets 400 `That invite code is not valid`. The preview (`GET /api/invites/{code}`) gains `personal: bool` (checks round): true for a friend invite, the same for every viewer, never whom it is for. Its row (`endpoint-map.md:202`) becomes `→ {club: ClubRef|null, kind: "club"|"signup", personal: bool}`.
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
- "**Lock order of a pair write** (send, accept, decline or cancel, remove, block, unblock; `friends::lock_pair`): the pair's turn, `pg_advisory_xact_lock(0x6263_0300, hashtext('<lower id>:<higher id>'))` (beside ingest's `0x6263_0100` and the reports' `0x6263_0200`) → both accounts' `users` rows `FOR KEY SHARE`, in id order (a deletion locks its row `FOR UPDATE` before its cascade, so it either finished or waits; a gone account answers as an unknown name) → the `user_blocks` row → the pair's `friendships` row → the pair's invite rows. No pair write locks a club; a friend invite takes the club first and then the friend's row and the friendship (§5.3). Tests: `friends_blocks.rs::a_request_waits_for_its_senders_block_in_flight`, `an_unblock_waits_for_a_hidden_request_in_flight`, `friends_requests.rs::a_request_or_a_block_waits_for_a_deletion_in_flight`."
- `accepted_at` is `clock_timestamp()` (the other side's request may be newer than the transaction).
- Send: `INSERT … ON CONFLICT ((least(…)), (greatest(…))) DO NOTHING`, then the pair's row `FOR UPDATE`: a request back accepts.
- Unfriend and block revoke the pair's open friend invites (`friends::revoke_pair_invites`: bound to one, created by the other, not revoked, expired or used up).
- `{username}` of the friend routes: `friends::PathName`, normalised; NUL or non-UTF-8 is an unknown name (the NUL paragraph of §4.5 gains "and the friend routes answer such a `{username}` as an unknown name").

**`conventions.md` §10.1:** the `RateLimits` struct line gains `pub friend_request_user`, `pub friend_request_ip` and `pub friend_invite_user`, and the table three rows (the last two from the checks round):
- "`friend_request_user`: every `POST /api/friends/requests` per account (`u:<id>`), unknown names and repeats included, and every `PUT /api/friends/blocks/{username}` unless the caller already has a `friendships` row with that account (a block of a requester is free) | 20 per day (429 `Slow down a little. Try again later.`)";
- "`friend_request_ip`: the same requests and blocks per client address (`ip_key`), counted after `friend_request_user`, so accounts made from one address share one budget | 60 per day (429)";
- "`friend_invite_user`: every `POST /api/clubs/{slug}/invites` with `kind: "friend"` per account (`u:<id>`), unknown names and repeats of an open invite included | 20 per day (429)".
- After the table: "Accept, decline or cancel, remove and unblock are not counted; they answer an unknown name only after the same work as an existing one (`friends::NO_ACCOUNT`), so their timing does not tell names apart either."


**`conventions.md` §6.5:** `Viewers::new` makes the friendships, the unfriend and the block through the friend routes. The matrix's kinds gain `Friends` (`GET /api/friends` and the five `{username}` routes): 34 columns, 16 routes, 11 schemas.

**`docs/plan.md:31`:** "They see each other's reading and can invite each other into clubs they administer."

**`CLAUDE.md`:** nothing new for part 2 beyond part 1's TRACES wording (the SPA part adds its `/friends` line).

**For #53 (facts only):** a friend request names the account by its exact username, and the server says whether such an account exists (as sign-up does), at most 20 times a day per account and 60 per client address; a block is never told to the blocked account, whose requests are kept but not shown; a friend invite into a club names the friend only to the admin who made it, also once it was used.

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
- "Friends (#57): `/friends` (`pages/FriendsPage.vue`) holds the friends, requests both ways, invitations into clubs and blocks; `stores/friends.ts` keeps `GET /api/friends` (every friend route answers with the whole lists; a 404 or 409 loads them again, as the other side changed the pair first). The shell loads it on mount, when the tab comes back after a minute and when the viewer's clubs change, and shows what waits (requests and invitations) on the sidebar's Friends item and on Home's Friends row. Friends is a sidebar-only nav item (`bottomBar: false`): the phone's tab bar keeps five tabs. `components/InviteFriendSheet.vue` makes a personal invite (`kind: "friend"`) into a club the viewer runs."
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

## Review round

The FRIENDS review found nine things (seven minor, two nits). Each was checked against the code first; all nine held, and two of them (the FriendsPage findings) were the same defect. Each fix that changes behaviour has a test that fails without it, run both ways.

### Fixed

1. **A used personal invite named its friend to co-admins through `used_by`** (answer 24; 281af5d). `clubs::out::club_invite_outs` now fills `used_by` of a personal invite for its creator only, like `invitee`; `use_count` stays, so a used one still reads as used. `friends_invites.rs::only_the_creator_sees_whom_an_invite_is_for` now lets the friend join and checks the co-admin's list again (and that an ordinary invite still lists who used it, for every admin). Left: a co-admin can still work out whom a used personal invite was for, from one look at the invite list and the members, not only by watching the list (corrected in "Checks", security finding 4; not fixed). The `InviteOut.used_by` description changed (`generated.ts`).
2. **A friend invite could deadlock with the purge of empty sign-ups** (ed44253). The purge locked the account and then, in `delete_user`, the clubs of the invites bound to it; an admin holding such a club and asking for a friend invite for that name takes the account's row before the friendship check. The purge now locks those clubs first and defers an account whose bound invite appeared in another club meanwhile; see "Lock order of the purge" (rewritten) and the corrected §5.6 and §16 texts. Tests: `jobs.rs::the_purge_locks_a_bound_invites_club_before_the_account` (before the fix Postgres aborted the purge as the deadlock's victim, and the account stayed), `a_bound_invite_made_while_the_purge_waits_defers_the_account`.
3. **No test failed without `lock_pair`'s pair turn or its account-row locks** (b2d1efc). Three tests now hold a lock by hand, start the write and release it: `friends_blocks.rs::a_request_waits_for_its_senders_block_in_flight` (without the turn: 200 and a pending request next to the sender's own block), `an_unblock_waits_for_a_hidden_request_in_flight` (without the turn: the hidden request surfaced), `friends_requests.rs::a_request_or_a_block_waits_for_a_deletion_in_flight` (without the account rows' `FOR KEY SHARE`: 500 from the foreign key). `friends::PAIR_LOCK_SPACE` and the new `friends::pair_key` are public, so the tests take the server's own turn.
4. **The Friends page kept rows that no longer existed after a 404** (d4e6843; two findings, the second one also about 409s, `confirm()` and the invite sheet). The store's actions go through a new `write()`: a 404 or 409 (`isStale`) loads the lists again (`refresh()`, after a load already running, whose answer may predate the change), and the error still goes to the caller. The Remove and Block confirmation closes on such a refusal, as the friend leaves the list. `InviteFriendSheet` refreshes the lists on a 404 (no longer a friend, or the club is gone), not on its 409 (`That friend is already in this club` changes no list). Tests: `stores/friends.test.ts` (a 404 and a 409 reload, a 429 and a network error do not; a running load is followed by a fresh one), `pages/FriendsPage.test.ts` (an Accept, a Remove and a Decline that each answer 404, with the next lists), `components/InviteFriendSheet.test.ts` (a 404 reloads, a 409 does not).
5. **The §5.7 text "club mates and friends" was missing** (9029a86, and these notes), and two FRIENDS comments still said only club mates can report a profile picture. The notes now give the §5.7 replacement, and the §6.3 and endpoint-map texts (part 1's texts); `auth/export.rs` and `auth_export.rs` say "club mates and friends".
6. **The #53 facts left out two changes that make today's legal texts false:** friends may see and report each other's profile pictures, and an empty sign-up may hold unanswered requests, blocks by others and friend invites bound to it. Both are in part 1's "For #53" list now, with the lines of `takedown.md` and `privacy-facts.md` they contradict. The legal texts stay untouched (§2.1 rule 11).
7. **Copy** (fa7b87b): de `friends.add.heading` is "Jemanden hinzufügen" (it was the plural "Freunde hinzufügen", Home's row; the singular "Jemanden" avoids a masculine "Freund" for one person, as "Diese Person" does in `errors.friendInClub`), and `friends.inviteSheet.blurb` says the friend joins "from there" / "von dort" instead of "with one tap" (joining takes Join and then the join page's button).
8. **A doc-comment line of `empty_sign_ups` ran to 123 columns;** rewrapped at 100 (9029a86).

### Not fixed

None.

### Counts after the round

`friends_requests.rs` 10 tests, `friends_blocks.rs` 9, `friends_invites.rs` 12, `jobs.rs` 20; the frontend suite 75 files and 503 tests (500 before).

## Checks

After the review round, an E2E run in the browser and a security review (three lenses; each candidate checked by three verifiers) looked at the branch. Each finding was checked against the code first. Each fix that changes behaviour has a test that fails without it, run both ways (the old handler or page in place, the new test red).

### The E2E run

Steps, in short, as far as its findings report them (synthetic accounts): Home's activity for a reader with a club and a friend (wren saw "juniper finished The Hobbit"); an admin inviting a friend, which leaves an active personal invite on the club's members page; `/join/<code>` of that personal invite for another signed-in account (orrin, whose submit was refused with `That invite code is not valid`) and signed out; the members page with an active ordinary and a personal invite at 360 × 780 in German.

| Finding | Verdict | Fix |
|---|---|---|
| Members page scrolls sideways at 360 px in German with an active invite (low; older than FRIENDS) | Holds: `.invite-actions` was `flex: 0 0 auto`, and "Kopieren" plus "Zurückziehen" (235 px) did not fit beside the code in the 328 px column | 9736f70: the code column takes what the actions leave but never less than the code (`flex: 1 1 0; min-width: min-content`), the actions wrap onto two lines and align right, a long name in the line under the code breaks. No unit test (jsdom has no layout); checked in the browser, below |
| Friends' events never reach Home for a reader with friends and no club (minor) | Holds: `HomePage.vue` rendered `ActivityFeed` only `v-if="hasClubs"` | 19df28c: the feed shows with a club or a friend (`friends.lists.friends`); a solo reader without friends still makes no activity call and keeps the favourites. Test: `HomePage.test.ts` "shows friends' events to a viewer with friends and no club" |
| The join preview of a personal invite offers Join, or sign-up, to anyone holding the code (nit) | Holds: the preview answered `{club, kind}` for any viewer; only the submit failed | ce5d630: `InvitePreview.personal`; `JoinPage.vue` offers a visitor only "Sign in" (a personal code never makes an account, and it is not kept for sign-up), and tells a signed-in reader whose `club_invites` lack the code that the invite is someone else's, without a Join button. Tests: `JoinPage.test.ts` (four cases), `clubs_invites.rs`/`clubs_exit.rs` (`personal: false`), `friends_invites.rs` (`personal: true`), `acceptance.test.ts` (the third case's preview is personal) |

**The browser check after the fixes** (8094, `DEV_MODE`, a freshly reset database, accounts ada, grace and orrin made with a `beta-invite` code, `playwright-cli`, 360 × 780):
- ada runs "Lantern Readers", is friends with grace, finished a custom book after the friendship, and invited grace; ada's members page in German: `scrollWidth` 360, the invite row 16–344 px, "Kopieren" above "Zurückziehen"; with a 32-character friend's invite as well, still 360. In English at 360 px and in both languages at 1280 px the two actions stay on one line.
- grace (no club) on Home: the sections "Read together", "Currently reading", "Your notes", "Activity", with "ada finished The Hobbit".
- `/join/<grace's code>`: grace sees "Join Lantern Readers" and joins (to `/c/lantern-readers-2sbx`); orrin sees "This is a personal invite for someone else…" and only "Home"; a visitor (the code typed in lower case with a dash) sees "This is a personal invite. Sign in…" and only "Sign in", and nothing is kept in `sessionStorage`.
- No console error besides the visitor's `GET /api/auth/me` 401.

### The security review

| # | Candidate | Verdict | Fix |
|---|---|---|---|
| 1 | The uncounted friend routes (accept, decline or cancel, remove, unblock, and friend invites) told an existing name from an unknown one by their timing: an unknown name answered after one query, an existing one after the pair's turn, the account row locks and the write; nothing limited the samples (low) | Confirmed (3 of 3). The verifiers note that `visibility::visible_user` already had a smaller gap inside one statement | cffd692: the four pair routes take the turn of the caller and `friends::NO_ACCOUNT` (-1) and run the same statements with it. Test: `friends_requests.rs::an_unknown_name_answers_after_the_same_work` (a held `NO_ACCOUNT` turn holds an unknown name's and the caller's own name's answer on all four routes, as a pair's turn holds an existing name's; the old handlers answered at once). Friend invites: 4552dc3 (#3) |
| 2 | The per-account request limit multiplied with open sign-up: 5 accounts an hour per address, each with 20 counted probes a day, unverified accounts included (low; needs `SIGNUP_MODE=open`) | Confirmed (3 of 3) | cffd692: `friend_request_ip`, 60 per client address a day, counted after `friend_request_user`. `VerifiedUser` was not chosen: mail stays off after the private move (answer 31), so no account could verify and nobody could send a request. Test: `friends_requests.rs::sixty_requests_a_day_from_one_address` (three accounts use 60, a fourth from that address gets 429 on a request and a counted block, a free block still works, another address still gets 404) |
| 3 | A friend's club invite could be made again at once after each decline, without a limit: invite creation was exempt (§4.9) (low) | Confirmed (3 of 3) | 4552dc3: `friend_invite_user`, 20 `kind: "friend"` calls per account a day, unknown names and repeats included. No stored decline: answer 19 rejected one for requests, and both are now bounded alike. It also bounds what this route's timing could tell about names (#1). Test: `friends_invites.rs::twenty_friend_invites_a_day` (ten invites declined and made again, repeats and unknown names count, the 21st is 429 with `Retry-After`, a single-use invite is still free) |
| 4 | A co-admin can work out whom a used personal invite was for by elimination: every member after the owner came in through an invite, ordinary invites list `used_by` to every admin, so a member in none of them came through a personal one, and with two admins it was the other admin's (`created_by`); `created_at` against `joined_at` narrows several. A second, contrived oracle: removing the friend flips the other admin's still-open personal invite for them to revoked (`revoke_known_invites`, `OR invitee_id = $2`) (low) | Confirmed (3 of 3). The review round's note ("a co-admin who watches the list") understated it: one look is enough | **Not fixed; needs Rafael.** Leaving out `use_count` and `active`, or other admins' used personal invites, does not close it: with two admins the elimination works from the members alone. Closing it means hiding `used_by` of ordinary invites from co-admins too, which changes what admins have seen since Phase 1. The review round's "Left:" line now says so (this commit). What is told: that two people who are now co-members were friends when the invite was made; no shelf, rating or post |
| 5 | After a compromise, a planted pending request survived `revoke-access.sql` and step 7's review (`accepted_at >= :since` never lists a pending row), and became friend access with one click by the account the attacker keeps (medium; the verifiers rate it low, since rows written with old dates pass step 7 anyway, which the README already says) | Confirmed (3 of 3) | 2725b5f: `revoke-access.sql` deletes every pending request, as it revokes every open invite; step 6 says so; step 7 lists friendships with `created_at` and `accepted_at` where either is since the date; the remedy paragraph gives the `DELETE` for one friendship. Test: `auth_sessions.rs::the_revocation_after_a_compromise_ends_every_reusable_access` (a planted request is gone, the account it was for gets 404 on accept after signing in again, an accepted friendship stays) |

### Deviations from the plan

1. **Two new limits** (§4.9 says friend invites are not counted and gives one limit): `friend_request_ip` (60 per address a day) and `friend_invite_user` (20 per account a day). §4.9's intent, that only the two counted routes tell names apart and that probing is bounded per address as sign-up's is, is kept. Accept, decline or cancel, remove and unblock stay uncounted; they do the same work for an unknown name instead (`NO_ACCOUNT`).
2. **`revoke-access.sql` gets a statement**, and the README more than §4.7's one query: step 6's sentence, step 7's query (`created_at` too), and the remedy's `DELETE`. `server/tests/auth_sessions.rs` (not a FRIENDS file) gets the friendship part of the revocation test.
3. **`InvitePreview.personal`** (`clubs/redeem.rs`) and the join page (`JoinPage.vue` and its test) are not FRIENDS files, and the plan says the preview is unchanged (§4.6). One assertion each in `clubs_exit.rs` and `clubs_invites.rs` gains `"personal": false`. The copy is under `friends.personalInvite.*` (the keys FRIENDS may add, §2.1 rule 2).
4. **Home's activity for a reader without a club** (§4.10 lists only the Friends row for Home): the feed shows with a friend, as §4.4 means it to.
5. **The members page's invite row** outside the "for <name>" hunk, like part 3's deviation 5.

### Texts for the integrator (checks)

- `conventions.md` §10.1, `endpoint-map.md` (the Friends section, §3's `POST …/invites` and the preview) and #53: part 2's texts above, as corrected in this round.
- `deploy/hosted/README.md` and `revoke-access.sql` are changed in place (no text to move).
- **`CLAUDE.md`, "Frontend"**, the Friends bullet of part 3 gains: "The join page knows a personal invite (`InvitePreview.personal`): signed out it offers only signing in, and signed in it joins only when the code is among the reader's `club_invites`."
- **For Rafael (finding 4):** keep it (a co-admin can learn that a co-member came in through another admin's personal invite, so that the two were friends), or hide every invite's `used_by` from admins other than its creator, which also hides who came in through a co-admin's ordinary invite.

### Counts after the checks

`friends_requests.rs` 12 tests, `friends_invites.rs` 13, `friends_blocks.rs` 9, `auth_sessions.rs` 20; the frontend suite 75 files and 508 tests.
