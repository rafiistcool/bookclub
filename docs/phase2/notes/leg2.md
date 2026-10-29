# LEG2 (#53) implementation notes

Owner: LEG2. These notes record what the retention package built, the decisions it took where the plan left room, and what the integrator moves into `conventions.md`, `CLAUDE.md`, `README.md` and the other packages' files. The design documents are unchanged.

Scope: work plan §2.3 "LEG2", Q14 = A ("Answers (2026-10-29)"), and §0 items 20 and 21.

## Status

Done:

1. **Retention in `auth::cleanup`** (`server/src/auth/mod.rs`): decided reports and catalog edit requests are deleted 3 years after their decision, and empty sign-ups 30 days after their last sign-in, through `auth::deletion::delete_user`.
2. **The legal documents** (`docs/legal/privacy-facts.md`, `vvt.md`, `processors.md`; not `takedown.md`, which ADM2 owns in this wave): the decided values of Q4, Q9 (with the Storage Box snapshots of the security review's option A), Q10 A, Q12 A, Q14 A and Q15, and amendment 20. "To confirm" stays on what is still open, and "to sign" marks the DPAs.

No route, DTO, `error::msg` string, environment variable, CLI command, crate or migration. `generated.ts` is unchanged; `server/.sqlx/` gains three query files.

## Retention (Q14 A)

`auth::cleanup(conn) -> sqlx::Result<()>` keeps its signature, so `server/src/jobs.rs` (ADM2's) is unchanged. Each hourly run now deletes, in this order:

1. expired sessions, and used or expired email tokens (as before);
2. `reports` with `status <> 'open' AND resolved_at < now() - interval '3 years'`;
3. `catalog_edit_requests` with `status <> 'open' AND decided_at < now() - interval '3 years'`;
4. up to `auth::PURGE_BATCH` (500) empty sign-ups (`auth::purge_empty_sign_ups`).

The log line is now `deleted expired sessions and email tokens, old decisions and empty sign-ups` with the fields `sessions`, `tokens`, `reports`, `edit_requests`, `sign_ups`.

Open reports and requests stay however old they are. Nothing references `reports` or `catalog_edit_requests`, so the deletes cascade nowhere. The takedown itself (`media.removed_at`, `media_removed_hashes`) is not touched: only the record of the notice goes. The deletes touch only decided rows, which no writer updates, so they wait for no takedown, dismissal or approval.

### Empty sign-ups

`auth::empty_sign_ups` (private) is one `query_scalar!` that both picks the candidates and checks one account again. An empty sign-up:

- **has an address it never verified:** `email IS NOT NULL AND email_verified_at IS NULL`. Sign-up always stores the address it was given (`RegisterIn.email` is required), so an account without any address is an imported one that never added one, and it is never purged. An imported account that typed an address without verifying it is kept while it holds data (Q14's note); it always does at the import, as a member of the imported club.
- **is not an instance admin.**
- **shows no sign-in for 30 days:** `greatest(created_at, password_changed_at, terms_accepted_at)` is older than 30 days, no session of the account has `last_used_at` within them, and no email token was created within them. Each of these needs a signed-in account or is the sign-up itself.
- **holds no data:** `avatar_media_id IS NULL`, and one `NOT EXISTS` per foreign-key column to `users` except the traces of signing up and signing in: `sessions.user_id`, `email_tokens.user_id`, and `club_invite_uses.user_id` when the invite is a sign-up invite (`club_invites.club_id IS NULL`). The use of a club invite is data. Without the sign-up-invite exception no beta sign-up could ever be empty: every one uses a sign-up invite.

`purge_empty_sign_ups(conn) -> sqlx::Result<u64>` takes the candidates lowest id first, then for each one, in a transaction of its own:

1. `SELECT password_hash FROM users WHERE id = $1 FOR UPDATE`;
2. `empty_sign_ups(Some(id))` again, in a new statement. A write that would give the account data or a sign-in references it and holds the row `FOR KEY SHARE` until it commits. So once the lock is granted, the write has either committed and the second look sees it, or it waits and fails when the account is gone. The single-snapshot version (a plain `DELETE … WHERE NOT EXISTS …`) would delete the account and cascade the new row away; `data_written_while_the_purge_waits_keeps_the_account` shows it (below).
3. `deletion::delete_user(tx, id, hash)` with the hash read under the lock: the same path as `DELETE /api/auth/me`, so sessions, tokens and everything `delete_user` handles go with it, and the sign-up invite keeps its use with `user_id` NULL.
4. Commit. If `delete_user` returned storage prefixes (impossible while the rule requires no picture; the job has no media store to delete files with), the account is kept and an error logged.

- **Lock order** (conventions §5.6 says clubs first, then the user): the purge takes the user row first. An empty account has no row in any club and none can arrive while the row is locked, so `lock_clubs` locks nothing and no order is broken. `delete_user`'s doc comment now says so.
- **Errors:** a failing `delete_user` (a database error, logged by `AppError::internal`) is logged as `deleting an empty sign-up failed; kept`, rolled back, and the purge goes on with the next account. A failure of the connection itself (begin, rollback, commit) ends the run, and `run_locked` logs the job as failed.
- **Batch:** 500 accounts a run; a spam wave is purged over several hours. Each re-check scans tables without an index on the user column (`works.created_by`, `editions.created_by`, `events.actor_id` for club events, …) like `DELETE FROM users`' own foreign-key checks already do. If that ever gets slow, partial indexes `WHERE <column> IS NOT NULL` in a 22xx migration help both.

### Decisions and their alternatives

- **"No sign-in for 30 days" has a gap:** a sign-in whose session ended by logging out leaves no row behind, so the 30 days then count from the last trace (sign-up, password change, terms acceptance, a session still there, a mail link). Example: sign-up on day 0, sign-in on day 10, logout on day 10, no data, no verified address: deleted on day 30, not day 40. The exact rule needs a `users.last_sign_in_at` written at every sign-in, by `session::create` (`server/src/auth/session.rs`, frozen) or a trigger on `sessions`; I left it out (a frozen file, or hidden logic in a trigger). `privacy-facts.md` §7 states the gap.
- **An app token unused for 30 days does not keep an empty account**, although it stays valid for 90 days: the rule is "no sign-in for 30 days", and its `last_used_at` is the sign-in evidence.
- **Instance admins are never purged**, whatever their data.
- **Imported accounts:** "never added an address" is the marker, because the importer leaves no other (the `users` row has no origin column; `server/src/importer/users.rs` is not LEG2's). If Rafael wants every imported account kept regardless, the importer needs to record it (a column in a 22xx migration plus one line in `import_users`), and `empty_sign_ups` one condition.

## Tests (`server/tests/jobs.rs`, 6 new, 11 in the file)

- `cleanup_deletes_decisions_three_years_after_they_were_made`: removed and dismissed reports and approved and rejected requests decided 3 years and a day ago are deleted; the same decided 2 years 11 months ago, and open ones filed 5 years ago, stay; the taken-down picture's row stays.
- `cleanup_deletes_sign_ups_that_stayed_empty_for_30_days`: a sign-up from 31 days ago with an app token last used 31 days ago and a sign-up invite use is deleted, with its session, and the invite use stays with `user_id` NULL. Kept: signed up 29 days ago; a session used 29 days ago; the password changed 10 days ago; the terms accepted 10 days ago; a verification link sent yesterday; a verified address; an imported account without an address (700 days old); an imported account with a typed address and a membership; an instance admin; a profile picture; a shelf entry. A second run deletes nothing more.
- `every_reference_to_an_account_is_data_or_a_trace_of_signing_in`: reads every foreign-key column to `users` from `pg_constraint` (the query of `auth_export.rs`) and requires each to be in `DATA` (26 columns, each with SQL that gives an account a row there) or `TRACES` (3, each with the reason and SQL). Then it gives one 31-day-old sign-up a row behind each column: every `DATA` account must stay, every `TRACES` account and a control account must go. **A new foreign key to `users` fails here** until it is added to `DATA` (and then fails until `empty_sign_ups` has its `NOT EXISTS`) or to `TRACES`.
- `data_written_while_the_purge_waits_keeps_the_account`: an uncommitted shelf entry holds the account's row; the purge picks the account, waits on the lock (seen in `pg_stat_activity`), and after the commit keeps it (0 deleted, the entry is there).
- `the_purge_deletes_a_batch_per_run`: 501 empty sign-ups: 500, then 1.
- `a_failing_deletion_keeps_that_account_and_the_purge_goes_on`: a trigger that refuses deleting the first account: it stays, the second goes.

Mutation checks, done by hand and reverted:
- `NOT EXISTS` for `events.actor_id` removed from `empty_sign_ups`: `every_reference…` fails with `accounts with data were purged; add a NOT EXISTS to auth::empty_sign_ups for ["events.actor_id"]`.
- The second look under the lock disabled: `data_written_while_the_purge_waits_keeps_the_account` fails (1 deleted instead of 0).
- A temporary migration `2299_tmp_probe.sql` adding `reports.target_user_id REFERENCES users`: `every_reference…` fails with `the purge of empty sign-ups does not consider these foreign keys to users: ["reports.target_user_id"]`.

## The legal documents

`server/tests/legal_docs.rs` passes unchanged (citations, names, hosts, tables). Changes:

- **`privacy-facts.md`:**
  - §0 is now "Decisions": the options decided on 2026-10-29 with what is still open per row, and a short list of what is still open: Q1, the legal source's review, the app log's retention, the minimum age.
  - §4: the app log is kept by size (5 × 10 MB per container, `deploy/hosted/compose.yaml` `x-logging`), with no time limit (to confirm with the lawyer). The proxy log as DEP built it: a file of its own, with the query, invite codes and Referer redacted and cookies written as `REDACTED`, at most 7 days through logrotate.
  - §5: Brevo, with ADM's admin mails (`admin_reports`, `admin_requests_digest`) and ADM2's decision mails; the Storage Box with restic and its snapshots; the uptime check; healthchecks.io with what the heartbeats carry (below).
  - §7: the empty-sign-up rule in full, with the logout gap; reports and requests 3 years after the decision; the backups with the Storage Box snapshots on top of restic's 12 months; the old app's data as decided (Q14).
  - §8: suspended accounts (amendment 20): no sign-in, so no in-app export or deletion; the owner writes to `CONTACT_EMAIL` and the operator answers by hand, with ADM2's steps in `takedown.md`. To confirm with the lawyer: how the owner proves the account is theirs, and whether evidence for the suspension may be kept back from a deletion. Also a line on empty accounts.
  - §10: no admin screen shows addresses, because the app sends the Q12 A mails itself.
  - Facts from Wave 1 merges that LEG's notes asked for: the theme script is `frontend/public/theme-init.js` (§3), and the CSP is in place (§6, `server/src/spa.rs` (`SHELL_CSP`)).
- **`vvt.md`:** version 2026-10-29; the measures (backups and Storage Box snapshots, monitoring, a new "Storage limitation" line, requests by mail for accounts that cannot sign in); activities 1, 2 (Brevo), 8 and 9 (3 years after the decision; Q12 A mails, Q15 suspension), 10 (log facts), 11 (suspended accounts), 12 (Hetzner, restic, snapshots), 13 (both monitors, with the heartbeat log tail), 14 (Q14 legacy retention).
- **`processors.md`:** Brevo and the Storage Box as processors with "to sign" DPAs; monitoring split into the uptime check (not a processor of users' data) and healthchecks.io (verdict to confirm, below).

**Finding for DEP2 and Rafael: the heartbeats can carry a picture's storage key.** `deploy/hosted/backup-lib.sh` (`supervise`, `heartbeat`) posts the last 50 lines of the backup and restore-test logs to healthchecks.io. Those lines can name `avatars/<xx>/<public id>/a` (a picture deleted during the backup, or one a restore test missed), and a public id is the secret part of a picture's URL. `processors.md` leaves the verdict to confirm: either sign healthchecks.io's DPA (if it offers one), or send the end ping without the log (a change in DEP2's `backup-lib.sh`), which keeps healthchecks.io out of the processors. What DEP2's update timer sends is also to confirm.

## For the integrator

- **conventions §16:** `auth::cleanup` (hourly) also deletes reports and catalog edit requests 3 years after their decision, and up to `auth::PURGE_BATCH` empty sign-ups (`auth::purge_empty_sign_ups`, the rule above).
- **conventions §5.6:** `delete_user` has a second caller, the purge, which locks the user row first (no club rows; see "Lock order").
- **conventions §5.7, work plan §2.1 rule 4, CLAUDE.md "Cross-cutting rules enforced by tests":** a new foreign key to `users` now also needs a `NOT EXISTS` in `auth::empty_sign_ups` and a `DATA` row in `tests/jobs.rs` (or a `TRACES` row with the reason); `jobs.rs::every_reference_to_an_account_is_data_or_a_trace_of_signing_in` fails otherwise. The test list in CLAUDE.md ("Some test files read docs …") is unchanged: `jobs.rs` reads only the schema.
- **README / deploy/hosted/README.md (optional):** the server deletes empty sign-ups after 30 days and old decisions after 3 years; nothing to configure.
- **`docs/legal/`** cites `server/src/auth/mod.rs` (`cleanup`, `purge_empty_sign_ups`), `server/src/auth/deletion.rs` (`delete_user`), `server/src/jobs.rs` (`auth_cleanup_loop`), `server/src/email/templates.rs` (`admin_reports`, `admin_requests_digest`) and `server/src/spa.rs` (`SHELL_CSP`) by name; `legal_docs.rs` keeps them honest.

### When merging ADM2

- **New foreign keys to `users`** (for example `users.suspended_by`, or a report target column such as `reports.target_user_id`): `jobs.rs` fails until each is added to `empty_sign_ups` and `DATA` (a suspension or a report by the account is data), or to `TRACES`. `auth_export.rs` fails the same way for the export, and `legal_docs.rs` for `vvt.md` if it is a new table.
- **`users.suspended_at`** is not a foreign key, so no test catches it. Recommended: a suspended account is never purged (the suspension is a moderation record, and purging would free its username): add `AND u.suspended_at IS NULL` to `empty_sign_ups` and a kept case to `cleanup_deletes_sign_ups_that_stayed_empty_for_30_days`. A suspended account cannot sign in, so without that line an empty one would be deleted 30 days after its last sign-in.
- **`reports`:** the cleanup keys on `status <> 'open'` and `resolved_at`, so new target kinds and statuses need nothing as long as every decision sets `resolved_at`. The test fixtures insert cover reports with `media_id` and `work_id`, which a stricter `2101` check should still accept.
- **`takedown.md`:** `privacy-facts.md` §8 and `vvt.md` activity 11 point to ADM2's manual export and deletion steps for suspended accounts there.
- **`legal_docs.rs` `PLANNED`:** unchanged here; the integrator removes ADM2's entries as before.

### When merging DEP2

- The Storage Box snapshot count (`privacy-facts.md` §0, §7; `vvt.md` activity 12; `processors.md`) and what the update timer's heartbeat carries are "to confirm" until DEP2's README states them.
- The heartbeat log tail (finding above).

## Verified by hand

- Server against the package database (`DEV_MODE=true BIND_ADDR=0.0.0.0:8084 PUBLIC_URL=http://agents.lan:8084 cargo run`), with two fictional sign-ups inserted with `psql`: `handcheck` (31 days old, unverified, nothing else) and `fresh` (29 days). The job's first tick at startup logged `deleted expired sessions and email tokens, old decisions and empty sign-ups sessions=0 tokens=0 reports=0 edit_requests=0 sign_ups=1` and `job: finished job="auth cleanup"`; `SELECT id, username FROM users` then showed only `fresh`; `/api/health` answered 200. Server stopped, database reset (`cargo sqlx database reset -y`).

## Checks

From `server/`: `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked` (88 test binaries, 1337 tests, 0 failed), `cargo sqlx prepare -- --all-targets` against a freshly reset database (three new query files, nothing else changed), `cargo sqlx prepare --check -- --all-targets`. From `frontend/` (nothing changed there; run to confirm): `npm run check:api`, `npm run typecheck`, `npm test` (72 files, 456 tests), `npm run build`.
