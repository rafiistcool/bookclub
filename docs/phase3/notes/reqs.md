# REQS (#55) implementation notes

Owner: REQS. The friend-request part of #55's added scope (`docs/phase3/work-plan.md` §9, "#55, the rest": "a cap on pending incoming friend requests per account (or their expiry) and a way for admins to see request spam"), built before open sign-up. These notes record what the package built, where it decided something the owner's brief left open, and the contract changes. REQS was its own integrator: `docs/phase1/conventions.md` (§5.10, §10.1, §16), `docs/phase1/endpoint-map.md` (the package table, §2, §11.3, §13), `docs/phase3/work-plan.md` (§4.2, answer 19, §9), `README.md` and `CLAUDE.md` already carry the texts below.

Rafael's brief (2026-11-21): pending requests expire after 30 days; at most 50 pending requests out per account at once (hidden ones count); an admin view of request spam in the "Accounts" tab, counts only.

Status: **done**, on `launch/reqs`.

## Migration

**`server/migrations/4050_friendships_expiry.sql`** (REQS' block 4050–4099):

- `friendships_pending_idx ON friendships (created_at) WHERE accepted_at IS NULL`: the expiry's search, without the accepted rows.
- `user_blocks_created_idx ON user_blocks (created_at)`: the admin list counts the blocks of the last 30 days.
- A new `COMMENT ON TABLE friendships`: the old one said "a request never expires".

No new column, table or foreign key: the export, the purge, `parity_schema.rs`, `parity_visibility.rs`'s columns and the VVT are unaffected.

## Contract changes

### New route

| Route | Auth | Request → Response | Errors |
|---|---|---|---|
| `GET /api/admin/users/friend-requests` | instance admin | → `AdminRequestSignalList {items: AdminRequestSignalOut[]}`; `AdminRequestSignalOut {user: AdminUserOut, pending_requests, blocks_received}` | 404 `Not found` (the guard) |

- Operation id `admin_list_friend_request_signals`, tag `admin`, in `admin::users::admin_routes()` next to the suspension routes (no `lib.rs` change). The path sits under `/api/admin/users/` like `suspended`; it does not collide with `{username}/suspend` (another segment count).
- At most 20 accounts (`REQUEST_SIGNALS_SHOWN`) with at least one pending request out (hidden ones included) or one block received in the last 30 days (`BLOCKS_COUNTED_DAYS`, by `user_blocks.created_at`; an unblock deletes the row, so it no longer counts).
- **Order:** `pending_requests + blocks_received` descending, then `blocks_received`, then username. The sum, because a block deletes the request it answers (§4.3): a spammer's request stays in the sum whether it still waits or was blocked, so blocking does not make an account look better. A block is the stronger signal, so it breaks ties.
- **Counts only:** never whom the account asked or who blocked it. `user` is the existing `AdminUserOut` (id, username, avatar, `is_admin`, `created_at`, `suspended_at`), not a bare `UserSummary`: the admin page shows suspended accounts as suspended, and `created_at` tells a fresh account. A suspended account stays in the list (its requests still wait, its blocks still count) until both age out.
- `admin_guard.rs`'s `ADMIN_OPERATIONS` gains it (22 → 23); it writes nothing, so the watched tables stay.

### Changed routes and DTOs

- **`POST /api/friends/requests`:** 409 `You already have 50 open friend requests. Withdraw some or wait for answers.` (`msg::TOO_MANY_OPEN_REQUESTS`, mapped to `errors.tooManyOpenRequests`, en and de) when the request would add a 51st pending row out of the caller, hidden ones counted. A request back (it accepts), a repeat and a request to a friend add no row and are never refused. The OpenAPI description says so.
- **`FriendRequestOut` gains `expires_at`** (`created_at` + 30 days), for incoming and outgoing alike. `generated.ts` regenerated.
- **Expiry:** a pending request, hidden or not, is deleted by the hourly cleanup within the hour after `expires_at`. From then on accept and decline or cancel answer 404 `No such friend request`, and either side may ask again: an expired request leaves no trace, like a decline.

### Decisions on open points

- **409, not 429**, for the cap: it is a state of the account (requests waiting), not a rate; it has no `Retry-After`, and withdrawing a request lifts it at once. The 409 is still counted against the 20-a-day limit (the counting happens first, as for every request), so the cap tells no more about names than a 200 would.
- **Outgoing, not incoming, is capped** (the brief; work-plan §9 had said "incoming"). The senders' caps, the daily limits (20 per account, 60 per address) and the expiry bound what one account receives; a cap on incoming would let spammers lock a victim out of real requests.
- **Hidden requests count** against their sender's cap and in the admin list: the sender cannot tell them apart, and they are the clearest spam signal.
- **The cap counts all pending rows**, also those past `expires_at` that the cleanup has not reached yet; at most an hour later they are gone.
- **The expiry is part of `auth::cleanup`**, before the purge of empty sign-ups, not a job of its own: it runs hourly under `AUTH_CLEANUP` like every other retention period, and a sign-up whose only data was an unanswered request is then empty in the same run. The log line gains `friend_requests` (`deleted expired sessions and email tokens, old decisions, expired friend requests and empty sign-ups`).
- **No batch limit** on the expiry: it reads 500 due rows at a time in key order (keyset) until none is left, each deleted in a transaction of its own. A spam wave is at most 50 rows per account, and each transaction is one advisory lock and one `DELETE`.
- **The SPA** shows "expires in N days" (N rounded up; "expires within a day" in the last day and after) next to "sent <day>" on the requests one sent, nowhere else. `daysLeft` lives in `stores/friends.ts`. The incoming list shows no expiry: the addressee need not act.
- **The admin view** is a second view of the "Accounts" tab (a segmented switch "Suspended" / "Friend requests", the pattern of the Covers tab), loaded when first opened. The suspend form stays above both. "Suspend…" on a row puts the name into the form at its second tap and focuses the confirm button, so a suspension from the list confirms exactly like a typed one; the row then shows "Suspended". The admin's own row has the button disabled (the server refuses it anyway).

## Lock order

The pair writes keep FRIENDS' order (conventions.md §5.10); REQS adds one lock before it and one more writer:

1. **The requester's turn** (send only): `pg_advisory_xact_lock(0x6263_0301, hashtext('<requester id>'))`, `friends::REQUESTER_LOCK_SPACE`, `friends::lock_requester`. Two requests of one account to different people take different pair turns, so without it both could count 49 and both insert. Nothing else takes it, and a request takes it before its pair's turn: no transaction holds a pair's turn while it waits for a requester's turn, so the order cannot reverse.
2. The pair's turn, both accounts' rows `FOR KEY SHARE`, `user_blocks`, the pair's `friendships` row, the pair's invites (unchanged).

Only one write adds a pending row out of an account: that account's own send. Accepts, declines, cancels, blocks, unblocks, the expiry and deletions only remove pending rows or turn them into friendships, so counting under the requester's turn is enough.

**Send** now reads the pair's row `FOR UPDATE` first and acts on it (none → the cap, then a plain `INSERT`; pending from the other → accept; else nothing). Under the pair's turn nothing else inserts or deletes that row, and the accounts' `FOR KEY SHARE` holds off a deletion. FRIENDS' `INSERT … ON CONFLICT DO NOTHING` first would have counted the cap after inserting.

**The expiry** (`friends::requests::expire_requests`) takes the pair's turn (`friends::lock_pair_turn`, which `lock_pair` now uses too) and then deletes the row only while it is still pending and due. It takes no account row: deleting a request references nothing, and an account deletion that reaches the row first (its cascade) makes the expiry wait, never the reverse. So an accept or a request back at the same moment either commits first (the row is a friendship, which the expiry leaves) or finds the request gone (404 `No such friend request`; a send then makes a new request). Without the turn the row locks alone would give the same outcomes for accept; the turn keeps the rule that every write about a pair takes it, so the pair's state cannot change between a pair write's reads.

## Tests

Server:

- `friends_requests.rs`:
  - `pending_requests_expire_after_30_days`: a 31-day-old request and a hidden one go, a 29-day-old one and a 400-day-old friendship stay; `expires_at - created_at` is 30 days; after the expiry the accept is 404 and a new request works; `auth::cleanup` runs the expiry.
  - `the_expiry_deletes_every_due_request`: 1201 due rows, more than two reads.
  - `the_expiry_waits_for_an_accept_in_flight`: the pair's turn held by hand with the accept's `UPDATE`; the expiry waits, then deletes nothing.
  - `an_accept_racing_the_expiry_wins_or_finds_no_request`: 8 rounds of a real accept against the expiry; either 200 and friends, or 404 `No such friend request` and no row.
  - `fifty_pending_requests_out_at_most`: 50 pending (one hidden) → the 51st is 409; a repeat (hidden or not), a request to a friend and a request back (accepts) are 200; accepting through its route works; a withdrawal and an expiry each free a place.
  - `concurrent_requests_cannot_pass_the_cap`: 5 rounds of two requests at once with one place left: one 200, one 409, 50 rows.
  - `a_request_waits_for_the_requesters_turn`: the requester's turn held by hand holds the request.
  - Without `lock_requester`, `concurrent_requests_cannot_pass_the_cap` and `a_request_waits_for_the_requesters_turn` fail; without the expiry's turn, `the_expiry_waits_for_an_accept_in_flight` fails (checked by removing each call).
- `jobs.rs::cleanup_expires_friend_requests_before_the_purge`: a sign-up whose only data was a 31-day-old request goes in the same run; one with a 29-day-old request stays.
- `admin_users.rs::the_admin_page_lists_accounts_by_their_friend_requests`: the counts (hidden requests included, blocks older than 30 days left out, a blocked request counted once), the keys of the DTO, the order (sum, then blocks, then name), a suspended account marked, at most 20, accounts without a signal left out.
- `admin_guard.rs`: the new route in `ADMIN_OPERATIONS`; non-admins get 404.
- Updated for `expires_at`: `friends_blocks.rs::a_request_after_the_block_is_hidden_from_the_blocker` (nulls it with `created_at` when it compares a hidden request with an ignored one) and `friends_visibility.rs::a_friend_shows_three_current_books_in_shelf_order` (the keys of a `FriendRequestOut`).

SPA:

- `stores/friends.test.ts`, `daysLeft`: rounding up, 0 after the time.
- `pages/FriendsPage.test.ts`: "says when each sent request expires" (12 days, within a day, past); the cap's 409 shows by the field, translated.
- `components/admin/AdminUsers.test.ts` (new): the views, the list loaded only when opened, the counts and the suspended badge, the empty and the error state with retry, "Suspend…" filling the form at its second tap with focus on the confirm button, and the row marked after the suspension.

## For #53 (the legal texts, not written here)

The privacy text and `docs/legal/privacy-facts.md` §7 (retention, Q14) should say that an unanswered friend request is deleted 30 days after it was sent, and that instance admins see the number of pending requests and of blocks received in the last 30 days per account (not who), to fight spam. REQS added no legal text (the rule until the public launch).
