# LEG2 (#53) implementation notes

Owner: LEG2. These notes record what the retention package built, the decisions it took where the plan left room, and what the integrator moves into `conventions.md`, `CLAUDE.md`, `README.md` and the other packages' files. The design documents are unchanged.

Scope: work plan §2.3 "LEG2", Q14 = A ("Answers (2026-10-29)"), and §0 items 20 and 21.

## Status

Done:

1. **Retention in `auth::cleanup`** (`server/src/auth/mod.rs`): decided reports and catalog edit requests are deleted 3 years after their decision, and empty sign-ups 30 days after their last sign-in, through `auth::deletion::delete_user`.
2. **The photo of a rejected cover request** goes with its request: `auth::delete_rejected_cover_photos(conn, store)`. It needs the media store, so **the integrator wires it into `jobs::auth_cleanup_loop`** (below, "For the integrator", item 1). Until then `cleanup` keeps such requests rather than orphan their photos.
3. **Sign-ins that end in a logout count:** migration `server/migrations/2201_users_last_seen.sql` adds `users.last_seen_at`, written by the trigger `sessions_keep_last_use` when a session is deleted before it expires.
4. **The legal documents** (`docs/legal/privacy-facts.md`, `vvt.md`, `processors.md`; not `takedown.md`, which ADM2 owns in this wave): the decided values of Q4, Q9 (with the Storage Box snapshots of the security review's option A, as DEP2 set them up), Q10 A, Q12 A, Q14 A and Q15, and amendment 20; the heartbeat finding with the option chosen for now. "To confirm" stays on what is still open, and "to sign" marks the DPAs.

New: migration 2201 (a column, a function, a trigger), and the public functions `auth::delete_rejected_cover_photos` and `auth::is_empty_sign_up`. No route, DTO, `error::msg` string, environment variable, CLI command or crate. `generated.ts` is unchanged; `server/.sqlx/` gains the new query files.

Left for later (values that exist only once the service goes live; Rafael decides them then, and each is marked "to confirm" or "to sign" where it appears, collected in `privacy-facts.md` §0): the name and domain (Q1); Brevo's account, DNS records and DPA; the Storage Box, its plan (which sets how many snapshots are kept), location and DPA; which uptime check and its location; the healthchecks.io account and location; the legal source's confirmation of every "to confirm with the lawyer".

Left for the integrator, because the files belong to other packages:

- wiring `delete_rejected_cover_photos` into `server/src/jobs.rs` (ADM2's);
- `users.suspended_at` in the purge, at ADM2's merge (`jobs.rs::every_column_of_users_is_considered_by_the_purge` fails until it is done);
- the `takedown.md` retention line, at ADM2's merge;
- the heartbeat change in DEP2's scripts, at DEP2's merge.

The exact changes are under "For the integrator", "When merging ADM2" and "When merging DEP2".

## Retention (Q14 A)

`auth::cleanup(conn) -> sqlx::Result<()>` keeps its signature. Each hourly run deletes, in this order:

1. expired sessions, and used or expired email tokens (as before);
2. `reports` with `status <> 'open' AND resolved_at < now() - interval '3 years'`;
3. `catalog_edit_requests` with `status <> 'open' AND decided_at < now() - interval '3 years'`, except a rejected `cover_replace` whose photo is still stored (`media.removed_at IS NULL`): those are left to `delete_rejected_cover_photos`;
4. up to `auth::PURGE_BATCH` (500) empty sign-ups (`auth::purge_empty_sign_ups`).

The log line is `deleted expired sessions and email tokens, old decisions and empty sign-ups` with the fields `sessions`, `tokens`, `reports`, `edit_requests`, `sign_ups`: counts only.

Open reports and requests stay however old they are. The period runs from the decision, not from the filing. Only `catalog_edit_requests` and `reports` rows are deleted here; nothing references them, so the deletes cascade nowhere. The takedown itself (`media.removed_at`, `media_removed_hashes`) is not touched: only the record of the notice goes. The deletes touch only decided rows, which no writer updates, so they wait for no takedown, dismissal or approval.

### Photos of rejected cover requests

When a book already has a cover, an uploaded photo becomes a `media` row (kind `cover`, `uploaded_by` the requester, stored files, served at `/media/{public_id}`) and a `cover_replace` request. Rejecting the request only sets its status, and no code path deletes cover media. Deleting the request after 3 years would therefore leave the photo, with its uploader, referenced by nothing and still served by URL.

`auth::delete_rejected_cover_photos(conn, store) -> sqlx::Result<u64>` takes the photos of rejected `cover_replace` requests decided more than 3 years ago and not taken down, lowest media id first, at most `PURGE_BATCH`, and for each one, in a transaction of its own:

1. `SELECT … FROM media WHERE id = $1 AND removed_at IS NULL FOR UPDATE`: media rows first, as a takedown and an approval lock them (conventions §7.2);
2. a second look in a new statement: the photo stays (with its request) while a work or an edition has it as its cover, an account as its picture, a report names it, a removed hash names it, or a request that is not due references it. A write that would add such a reference (`set_cover` locks the photo `FOR SHARE`, a report's foreign key `FOR KEY SHARE`) has either committed and is seen, or waits and finds the photo gone;
3. `DELETE FROM media`: the due request goes with it (`catalog_edit_requests.media_id` is `ON DELETE CASCADE`);
4. commit, then delete the files under `storage_prefix(kind, public_id)` and `storage_key` (`media::delete_prefixes`, which logs a failure and goes on, as every other picture deletion does).

A taken-down photo has no files left; its row stays like every taken-down picture's (`privacy-facts.md` §7, "Removed pictures"), and `cleanup` deletes its request. A photo kept because a report names it goes on a later run, once the report itself is deleted 3 years after its decision. An approved request's photo is the cover, so `cleanup` deletes only the request. A cover replaced later (by an approval or an admin edit) stays as an unreferenced media row, as it did before; that is catalog-wide and not part of Q14.

### Empty sign-ups

`auth::empty_sign_ups` (private) is one `query_scalar!` that both picks the candidates and checks one account again (`auth::is_empty_sign_up`). An empty sign-up:

- **has an address it never verified:** `email IS NOT NULL AND email_verified_at IS NULL`. Sign-up always stores the address it was given (`RegisterIn.email` is required), so an account without any address is an imported one that never added one, and it is never purged. An imported account that typed an address without verifying it is kept while it holds data (Q14's note); it always does at the import, as a member of the imported club.
- **is not an instance admin.**
- **shows no sign-in for 30 days:** `greatest(created_at, password_changed_at, terms_accepted_at, last_seen_at)` is older than 30 days, no session of the account has `last_used_at` within them, and no email token was created within them. Each of these needs a signed-in account or is the sign-up itself.
- **holds no data:** `avatar_media_id IS NULL`, and one `NOT EXISTS` per foreign-key column to `users` except the traces of signing up and signing in: `sessions.user_id`, `email_tokens.user_id`, and `club_invite_uses.user_id` when the invite is a sign-up invite (`club_invites.club_id IS NULL`). The use of a club invite is data. Without the sign-up-invite exception no beta sign-up could ever be empty: every one uses a sign-up invite.

`purge_empty_sign_ups(conn) -> sqlx::Result<u64>` takes the candidates lowest id first, then for each one, in a transaction of its own:

1. `SELECT password_hash FROM users WHERE id = $1 FOR UPDATE`;
2. `SELECT id FROM sessions WHERE user_id = $1 FOR UPDATE`. Using an existing session (`auth::extract`'s update of `last_used_at`) does not change `user_id`, so it takes no lock on the users row; this lock makes it wait or be seen;
3. `is_empty_sign_up` again, in a new statement. A write that gives the account data or a new session references it and holds the users row `FOR KEY SHARE` until it commits. So once both locks are granted, the write has either committed and the second look sees it, or it waits and fails (or, for a session's update, changes nothing) when the account is gone. The single-snapshot version (a plain `DELETE … WHERE NOT EXISTS …`) would delete the account and cascade the new row away; `data_written_while_the_purge_waits_keeps_the_account` and `a_session_used_while_the_purge_waits_keeps_the_account` show it (below);
4. `deletion::delete_user(tx, id, hash)` with the hash read under the lock: the same path as `DELETE /api/auth/me`, so sessions, tokens and everything `delete_user` handles go with it, and the sign-up invite keeps its use with `user_id` NULL;
5. commit. If `delete_user` returned storage prefixes (impossible while the rule requires no picture; the job has no media store to delete files with), the account is kept and an error logged.

- **Lock order:** users row, then sessions, as in a password change, a reset and an account deletion. The trigger of migration 2201 never waits for a users row (below), so a logout cannot close a cycle. Conventions §5.6 says clubs first, then the user: the purge takes the user row first, but an empty account has no row in any club and none can arrive while the row is locked, so `lock_clubs` locks nothing. `delete_user`'s doc comment says so.
- **Errors:** a failing `delete_user` (a database error, logged by `AppError::internal`) is logged as `deleting an empty sign-up failed; kept`, rolled back, and the purge goes on with the next account. The log names no account and no storage key (`privacy-facts.md` §4 says the app log has no user id); the operator finds the accounts left with the query of `empty_sign_ups`. A failure of the connection itself (begin, lock, rollback, commit) ends the run, and `run_locked` logs the job as failed.
- **Batch:** 500 accounts a run; a spam wave is purged over several hours. Each re-check scans tables without an index on the user column (`works.created_by`, `editions.created_by`, `events.actor_id` for club events, …) like `DELETE FROM users`' own foreign-key checks already do. If that ever gets slow, partial indexes `WHERE <column> IS NOT NULL` in a 22xx migration help both.

### A sign-in that ends in a logout (migration 2201)

A logout (`session::delete_by_secret`), the revocation of a session (`tokens.rs`), a password change (every other session), a reset and a suspension delete the session row, and login writes nothing to `users`. Without a record, "no sign-in for 30 days" counted from the last remaining trace: sign-up on day 0, sign-in on day 40 and logout, no data, no verified address: deleted on day 40, not day 70.

`server/migrations/2201_users_last_seen.sql`:

- `users.last_seen_at timestamptz NULL`: the last use of the newest session that ended before it expired.
- `sessions_keep_last_use()`, an `AFTER DELETE ON sessions FOR EACH ROW WHEN (OLD.expires_at > now())` trigger: `UPDATE users SET last_seen_at = OLD.last_used_at` where it is newer, on the row selected `FOR NO KEY UPDATE SKIP LOCKED`.
- **Expired sessions write nothing:** a cookie session expires 30 days after its last use and an app token 90 days, so an expired one is older than the rule; the hourly cleanup of expired rows stays a plain delete.
- **Why `SKIP LOCKED`:** a logout holds its session row and would then wait for the users row, while a password change, a reset, a suspension, an account deletion and the purge hold the users row and then delete or lock the sessions. Waiting could deadlock a logout on one device with a password change on another. Whoever holds the row writes a newer trace itself (`password_changed_at`, a new session), deletes the account, or is the purge. A transaction that holds the row itself (a password change deleting the other sessions, a suspension) is not skipped. The deletion of the account itself (the cascade) finds no row.
- **What is left:** a logout whose trigger runs in the microseconds while the purge holds that very account's row is skipped; the purge then waits for the logout's session row, and its second look sees neither. That needs an account untouched for 30 days to sign in and log out exactly while the purge is at it.
- The export carries the column with the rest of the users row (`to_jsonb(u) - 'password_hash'`); `auth_export.rs`'s `the_export_tells_no_sign_up_from_another` ignores `*_at` fields, so it passes unchanged.
- Alternatives left out: writing the time in `session::create` and at logout (frozen `session.rs`, ADM2's `login.rs`), or a table of its own (an FK check on every logout, and its own lock order).

### Decisions and their alternatives

- **An app token unused for 30 days does not keep an empty account**, although it stays valid for 90 days: the rule is "no sign-in for 30 days", and its `last_used_at` is the sign-in evidence.
- **Instance admins are never purged**, whatever their data.
- **Imported accounts:** "never added an address" is the marker, because the importer leaves no other (the `users` row has no origin column; `server/src/importer/users.rs` is not LEG2's). If Rafael wants every imported account kept regardless, the importer needs to record it (a column in a 22xx migration plus one line in `import_users`), and `empty_sign_ups` one condition.

## Tests (`server/tests/jobs.rs`, 16 tests, 11 new in this package)

- `cleanup_deletes_decisions_three_years_after_they_were_made`: removed and dismissed reports and approved and rejected requests filed 3 years 2 days and decided 3 years 1 day ago are deleted; the same decided 2 years 11 months ago, and **filed 5 years ago but decided 2 years ago**, stay (pins the period to the decision, not the filing); open ones filed 5 years ago stay; the taken-down picture's row stays.
- `cleanup_deletes_sign_ups_that_stayed_empty_for_30_days`: a sign-up from 31 days ago with an app token last used 31 days ago and a sign-up invite use is deleted, with its session, and the invite use stays with `user_id` NULL; a sign-up whose last sign-in, 31 days ago, ended in a logout is deleted too. Kept: signed up 29 days ago; a session used 29 days ago; **a sign-in 10 days ago that ended with a logout**; the password changed 10 days ago; the terms accepted 10 days ago; a verification link sent yesterday; a verified address; an imported account without an address (700 days old); an imported account with a typed address and a membership; an instance admin; a profile picture; a shelf entry. A second run deletes nothing more.
- `every_reference_to_an_account_is_data_or_a_trace_of_signing_in`: reads every foreign-key column to `users` from `pg_constraint` (the query of `auth_export.rs`) and requires each to be in `DATA` (26 columns, each with SQL that gives an account a row there) or `TRACES` (3, each with the reason and SQL). Then it gives one 31-day-old sign-up a row behind each column. **Before the run** it asks `auth::is_empty_sign_up` for each: false for every `DATA` account, true for the control and every `TRACES` account, so a kept account is kept by the rule, never by a deletion that failed and was logged. After the run every `DATA` account must still be there and every `TRACES` account and the control gone. A new foreign key to `users` fails here until it is added to `DATA` (and then fails until `empty_sign_ups` has its `NOT EXISTS`) or to `TRACES`.
- `every_column_of_users_is_considered_by_the_purge`: every column of `users` (from `information_schema`) must be in `USERS_COLUMNS` with how the purge reads it. A new column (ADM2's `suspended_at`) fails until it is named, after `empty_sign_ups` has its condition.
- `data_written_while_the_purge_waits_keeps_the_account`: an uncommitted shelf entry holds the account's row; the purge picks the account, waits on the lock (seen in `pg_stat_activity`), and after the commit keeps it (0 deleted, the entry is there).
- `a_session_used_while_the_purge_waits_keeps_the_account`: an uncommitted update of a 31-day-old session's `last_used_at` (what `auth::extract` does); the purge waits on the session lock and keeps the account.
- `a_session_that_ends_early_leaves_its_last_use_on_the_account`: a logout records the session's last use; an older one does not overwrite it; an expired session records nothing; while another transaction holds the users row the logout neither waits (it runs under `lock_timeout = '2s'`) nor records; inside a transaction that holds the row (a password change) it records; deleting an account with a live session works.
- `the_purge_deletes_a_batch_per_run`: 501 empty sign-ups: 500, then 1.
- `a_failing_deletion_keeps_that_account_and_the_purge_goes_on`: a trigger that refuses deleting the first account: it stays, the second goes.
- `rejected_cover_photos_go_with_their_request`: pins the foreign keys to `media` (`MEDIA_REFERENCES`); a photo rejected 3 years and a day ago loses its row, its request and its files; kept: one rejected inside the period, one whose request is open after 5 years, one a report names (with its request); an approved request past the period goes while its photo stays as the cover; a taken-down photo's request goes and its row stays. `cleanup` alone leaves the due request in place. A second run deletes nothing.
- `a_report_written_while_the_photo_is_deleted_keeps_it`: an uncommitted report on the photo; the deletion waits on the photo's lock and keeps it.

Mutation checks, done by hand and reverted:

- `NOT EXISTS` for `events.actor_id` removed from `empty_sign_ups`: `every_reference…` fails.
- The second look under the lock disabled: `data_written_while_the_purge_waits_keeps_the_account` fails (1 deleted instead of 0).
- `u.last_seen_at` removed from the `greatest(…)`: `cleanup_deletes_sign_ups…` fails with `an account with a sign-in 10 days ago that ended with a logout was deleted`.
- The sessions lock removed from the purge: `a_session_used_while_the_purge_waits_keeps_the_account` fails (1 deleted instead of 0).
- Both deletes keyed on `created_at` instead of the decision: `cleanup_deletes_decisions…` fails (the filed-early, decided-late rows go).
- The cleanup's exception for rejected cover requests removed: `rejected_cover_photos_go_with_their_request` fails.
- A temporary migration `2299_tmp_probe.sql`:
  - `reports.target_user_id REFERENCES users` with the default `NO ACTION`, listed in `DATA` but without a `NOT EXISTS` (the reviewer's case: the deletion fails and is logged, so the account survived): `every_reference…` now fails with `accounts with data count as empty sign-ups; add a NOT EXISTS to auth::empty_sign_ups for ["reports.target_user_id"]`;
  - the trigger function without `SKIP LOCKED`: `a_session_that_ends_early…` fails with `canceling statement due to lock timeout`;
  - `users.suspended_at`: `every_column_of_users…` fails with `… does not consider these columns of users: ["suspended_at"]`;
  - a new foreign key to `media`: `rejected_cover_photos_go_with_their_request` fails.

## The legal documents

`server/tests/legal_docs.rs` passes unchanged (citations, names, hosts, tables). Changes:

- **`privacy-facts.md`:**
  - §0 is "Decisions": the options decided on 2026-10-29 with what is still open per row; the Storage Box snapshots as DEP2 set them up (one a day, as many slots as the plan has; BX11: 10); the heartbeat option chosen for now (below); a short list of what is still open (Q1, the legal source's review, the app log's retention, the minimum age), and one list of everything to fill in when the service goes live.
  - §2: `users.last_seen_at` in the account row.
  - §4: the app log is kept by size (5 × 10 MB per container, `deploy/hosted/compose.yaml` `x-logging`), with no time limit (to confirm with the lawyer), and the hourly cleanup logs counts only. The proxy log as DEP built it: a file of its own, with the query, invite codes and Referer redacted and cookies written as `REDACTED`, at most 7 days through logrotate.
  - §5: the lead sentence now says only what is true of every host (from the server's or the VM's own IP address, never with a visitor's IP address or cookies) and points to the table for what each receives: the catalogs no account id or username, the mail provider the recipient's address and username, the backup target every table as ciphertext. Brevo, with ADM's admin mails (`admin_reports`, `admin_requests_digest`) and ADM2's decision mails; the Storage Box with restic and its snapshots; the uptime check; healthchecks.io with what the heartbeats carry, the update's included.
  - §7: the empty-sign-up rule in full, a logout's sign-in included; reports and requests 3 years after the decision, the photo of a rejected cover request with its request; the backups with the Storage Box snapshots on top of restic's 12 months; the old app's data as decided (Q14).
  - §8: suspended accounts (amendment 20): no sign-in, so no in-app export or deletion; the owner writes to `CONTACT_EMAIL` and the operator answers by hand, with ADM2's steps in `takedown.md`. To confirm with the lawyer: how the owner proves the account is theirs, and whether evidence for the suspension may be kept back from a deletion. Also a line on empty accounts.
  - §10: no admin screen shows addresses, because the app sends the Q12 A mails itself.
  - Facts from Wave 1 merges that LEG's notes asked for: the theme script is `frontend/public/theme-init.js` (§3), and the CSP is in place (§6, `server/src/spa.rs` (`SHELL_CSP`)).
- **`vvt.md`:** version 2026-10-29; the measures (backups and Storage Box snapshots, monitoring, a "Storage limitation" line with the rejected cover photos, requests by mail for accounts that cannot sign in); activities 1 (`last_seen_at`), 2 (Brevo), 8 (the photo of a rejected cover request) and 9 (3 years after the decision; Q12 A mails, Q15 suspension), 10 (log facts), 11 (suspended accounts), 12 (Hetzner, restic, snapshots), 13 (both monitors, with the heartbeat log and the chosen option), 14 (Q14 legacy retention).
- **`processors.md`:** Brevo and the Storage Box as processors with "to sign" DPAs and the snapshot setup; monitoring split into the uptime check (not a processor of users' data) and healthchecks.io (not a processor of users' data with the chosen option; to confirm with the lawyer).

### The heartbeats: the option chosen for now

`deploy/hosted/backup-lib.sh` (`supervise`) posts the last 50 lines of the backup and restore-test logs to healthchecks.io, and DEP2's `deploy/hosted/update.sh` the last 50 lines of an update's log, which holds `backup.sh`'s output. Those lines can name `avatars/<xx>/<public id>/a` (a picture deleted during the backup, or one a restore test missed), and a public id is the secret part of a picture's URL.

The two options were signing a DPA with healthchecks.io or dropping the log from the ping. Rafael asked to take the recommended option for now and decide what matters at go-live, so I chose a third that keeps the log's value: **blank every public id out of the tail before it is sent.** healthchecks.io then receives no personal data, is not a processor of users' data, and needs no DPA for it (to confirm with the lawyer); the alert still says why a run failed. The change is in DEP2's files (below); `processors.md`, `privacy-facts.md` §0 and §5 and `vvt.md` activity 13 say it is still to do.

## For the integrator

1. **Required: run `delete_rejected_cover_photos` in the hourly job.** In `server/src/jobs.rs` (ADM2's), `auth_cleanup_loop`:

   ```rust
   run_locked(&state.db, lock_ids::AUTH_CLEANUP, "auth cleanup", |conn| {
       let media = state.media.clone();
       Box::pin(async move {
           auth::cleanup(conn).await?;
           let photos = auth::delete_rejected_cover_photos(conn, media.as_ref()).await?;
           tracing::info!(photos, "deleted the photos of rejected cover requests");
           Ok(())
       })
   })
   .await;
   ```

   Without it, rejected cover requests with a stored photo are never deleted (`cleanup` leaves them, so their photos are not orphaned), and `privacy-facts.md` §7 and `vvt.md` activity 8 promise more than the code does. A test that `jobs::spawn_all` deletes a due photo (as `background_jobs_clean_up_at_start_and_stop_on_shutdown` does for a session) belongs in `server/tests/jobs.rs` with it; LEG2 could not add one, because it fails until the line is there.
2. **conventions §16:** `auth::cleanup` (hourly) also deletes reports and catalog edit requests 3 years after their decision, and up to `auth::PURGE_BATCH` empty sign-ups (`auth::purge_empty_sign_ups`, the rule above); the same job runs `auth::delete_rejected_cover_photos` (item 1).
3. **conventions §5.6:** `delete_user` has a second caller, the purge, which locks the user row and then its sessions first (no club rows; see "Lock order").
4. **conventions, schema notes:** `users.last_seen_at` and the trigger `sessions_keep_last_use` (migration 2201): a session deleted before it expires leaves its last use on the account, without waiting for a locked users row. Code that deletes sessions needs nothing new.
5. **conventions §5.7, work plan §2.1 rule 4, CLAUDE.md "Cross-cutting rules enforced by tests":**
   - a new foreign key to `users` also needs a `NOT EXISTS` in `auth::empty_sign_ups` and a `DATA` row in `tests/jobs.rs` (or a `TRACES` row with the reason): `jobs.rs::every_reference_to_an_account_is_data_or_a_trace_of_signing_in` fails otherwise;
   - a new column of `users` needs a row in `USERS_COLUMNS` (`every_column_of_users_is_considered_by_the_purge`), and a condition in `empty_sign_ups` when it records a sign-in or keeps the account;
   - a new foreign key to `media` needs a check in `delete_rejected_cover_photos` and a row in `MEDIA_REFERENCES` (`rejected_cover_photos_go_with_their_request`).

   The test list in CLAUDE.md ("Some test files read docs …") is unchanged: `jobs.rs` reads only the schema.
6. **README / deploy/hosted/README.md (optional):** the server deletes empty sign-ups after 30 days, and old decisions (with the photos of rejected cover requests) after 3 years; nothing to configure.
7. **`docs/legal/`** cites `server/src/auth/mod.rs` (`cleanup`, `purge_empty_sign_ups`, `delete_rejected_cover_photos`), `server/src/auth/deletion.rs` (`delete_user`), `server/migrations/2201_users_last_seen.sql`, `server/src/jobs.rs` (`auth_cleanup_loop`), `server/src/email/templates.rs` (`admin_reports`, `admin_requests_digest`) and `server/src/spa.rs` (`SHELL_CSP`) by name; `legal_docs.rs` keeps them honest.

### When merging ADM2

- **`users.suspended_at`** (`2102_users_suspended_at.sql`): `every_column_of_users_is_considered_by_the_purge` fails until it is handled. A suspended account is never purged: the suspension is a moderation record, purging would free its username, and a suspended account cannot sign in, so an empty one would otherwise go 30 days after its last sign-in. Three changes:
  - `server/src/auth/mod.rs`, `empty_sign_ups`, after `AND NOT u.is_admin`: `AND u.suspended_at IS NULL`, and a bullet "**is not suspended.**" in its doc comment;
  - `server/tests/jobs.rs`, the `kept` list of `cleanup_deletes_sign_ups_that_stayed_empty_for_30_days`: `("a suspension", sign_up(db, "suspended", 60).await, "UPDATE users SET suspended_at = now() - interval '40 days' WHERE id = {u}")`;
  - `USERS_COLUMNS`: `("suspended_at", "a suspended account is never purged")`, and the array length.

  Then `cargo sqlx prepare -- --all-targets`. `privacy-facts.md` §7 ("Empty sign-ups") gets "suspended accounts" in its list of accounts never deleted this way.
- **`reports.target_user_id`** (`2101_reports_targets.sql`, `ON DELETE SET NULL`): `every_reference…` fails until it is handled. An avatar report about the account is a moderation record, so it is data: `AND NOT EXISTS (SELECT 1 FROM reports x WHERE x.target_user_id = u.id)` in `empty_sign_ups`, and in `DATA` `("reports.target_user_id", "INSERT INTO reports (target_kind, reason, target_user_id) VALUES ('avatar', 'offensive', {u})")` with the array length. Any other new foreign key to `users` the same way, or into `TRACES` with the reason. `auth_export.rs` fails the same way for the export, and `legal_docs.rs` for `vvt.md` if it is a new table.
- **`reports`:** the cleanup keys on `status <> 'open'` and `resolved_at`, so the new target kinds, `via` and `closed_on_arrival` need nothing as long as every decision (and every report closed on arrival) sets `resolved_at`. The test fixtures insert cover reports with `media_id` and `work_id`, which `2101`'s checks accept.
- **`docs/legal/takedown.md`, §6 "Records", the "Retention" bullet** (ADM2's working copy, line 70) still reads "3 years after the decision (to confirm, Q14; LEG2 deletes resolved reports and decided requests after the period)". Replace it with: "**Retention.** Reports and decided catalog requests are deleted 3 years after the decision by the hourly cleanup (`server/src/auth/mod.rs` (`cleanup`)), the photo of a rejected cover request with its request (`delete_rejected_cover_photos`); open reports and requests are kept until decided (Q14, `privacy-facts.md` §7). The mails in the contact mailbox: 3 years after the decision (to confirm)."
- **`takedown.md`:** `privacy-facts.md` §8 and `vvt.md` activity 11 point to ADM2's manual export and deletion steps for suspended accounts there.
- **ADM2's suspension deletes the account's sessions** in the transaction that sets `suspended_at`, so the trigger of 2201 records their last use (the transaction holds the row itself). Nothing to change.
- **`legal_docs.rs` `PLANNED`:** unchanged here; the integrator removes ADM2's entries as before.

### When merging DEP2

- **The heartbeat change (the option chosen above).** In `deploy/hosted/backup-lib.sh`, `supervise`, replace `tail -n 50 "$BACKUP_DIR/$name.log" >"$BACKUP_DIR/$name.log.tail"` with a filter that blanks every public id, for example `tail -n 50 "$BACKUP_DIR/$name.log" | sed -E 's/[a-z0-9]{26,}/<id>/g' >"$BACKUP_DIR/$name.log.tail"`, and the same for `tail -n 50 "$report" >"$report.tail"` in `deploy/hosted/update.sh`. A public id is 26 characters of `[a-z0-9]` (`media.public_id`'s check); the pattern also blanks restic's 64-character ids, which the operator still finds on the VM. Then shellcheck. Once done, drop "still to do" from `processors.md` (healthchecks.io), `privacy-facts.md` §0 and §5, and `vvt.md` activity 13.
- `privacy-facts.md`, `vvt.md` activity 12 and `processors.md` cite DEP2's setup of the Storage Box snapshots (`deploy/hosted/README.md`, "Storage Box snapshots": one a day, as many slots as the plan has) and `deploy/hosted/update.sh`; both exist only once DEP2 is merged. If DEP2 changes them before the merge, these lines follow.

## Verified by hand

- Server against the package database (`DEV_MODE=true BIND_ADDR=0.0.0.0:8084 PUBLIC_URL=http://agents.lan:8084 cargo run`), with two fictional sign-ups inserted with `psql`: `handcheck` (31 days old, unverified, nothing else) and `fresh` (29 days). The job's first tick at startup logged `deleted expired sessions and email tokens, old decisions and empty sign-ups sessions=0 tokens=0 reports=0 edit_requests=0 sign_ups=1` and `job: finished job="auth cleanup"`; `SELECT id, username FROM users` then showed only `fresh`; `/api/health` answered 200. Server stopped, database reset (`cargo sqlx database reset -y`).
- The reviewed case, through the real routes (same server; a `beta-invite` code; `curl` with `Origin: http://agents.lan:8084`): `POST /api/auth/register` (201) left `last_seen_at` NULL with one session; `POST /api/auth/logout` (204) left no session and `last_seen_at` set to the logout's moment. Aged with `psql` to created and password changed 40 days ago, last seen 1 day ago, and restarted: the startup run logged `sign_ups=0`, and the account stayed. With last seen 31 days ago it stayed again, because its verification link was minutes old; with that link also 31 days old, the next run logged `sign_ups=1`, the account was gone and the invite use stayed with `user_id` NULL. The log lines carry counts only. Server stopped, database reset.
- The wiring of item 1 under "For the integrator", applied to `server/src/jobs.rs` with a temporary test in `tests/jobs.rs` (`jobs::spawn_all` with a photo rejected 3 years and a day ago): it compiled, and the job's startup run deleted the photo's row and files. Both reverted; `jobs.rs` is unchanged on this branch.

## Checks

From `server/`: `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`, `cargo sqlx prepare -- --all-targets` against a freshly reset database, `cargo sqlx prepare --check -- --all-targets`. From `frontend/` (nothing changed there; run to confirm): `npm run check:api`, `npm run typecheck`, `npm test`, `npm run build`.
