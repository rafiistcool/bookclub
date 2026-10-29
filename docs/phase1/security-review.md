# Phase 1 security review (2026-10-28)

A whole-phase security review of branch `phase-1`: the Rust server in `server/` and the Vue app in `frontend/`, at `18501e0`. Reviewers looked for defects through the authn, authz, visibility, input, resource and transport lenses. Every finding below was confirmed by at least two independent verifiers, who tried to refute it against the code. The integrator then fixed each one at its root, with a regression test that fails without the fix. The behaviour tests of the feed fix were also checked by mutation: with the fix reverted, all six fail.

Severity follows the review rubric: **medium** covers a denial of service by one user and a narrow leak of private data; **low** covers defence in depth and fixes that depend on deployment or on a hostile same-site page.

Abbreviations: C = `conventions.md`, E = `endpoint-map.md`.

## Outcome

| # | Severity | Finding | Outcome |
|---|---|---|---|
| 1 | medium | A right current password cleared its own limit, so one account could run argon2 without limit and starve every login | Fixed in `3700d01` |
| 2 | medium | The activity feeds showed an old only-me rating, take or quote text once the current entry or quote was audience | Fixed in `4ef65a5`, refined in `d12fe29` |
| 3 | medium | An anonymous attacker could block a signed-in user's password change, email change and account deletion | Fixed in `3700d01` (the login half is a product decision, see below) |
| 4 | medium | Unlimited avatar uploads let one user take over the picture pipeline | Fixed in `c86e689` |
| 5 | medium | One user could use up the shared Open Library budget (3 requests a second) | Fixed in `c86e689` |
| 6 | medium | The data export held the whole account in memory, with no limit on concurrent exports and no deadline | Fixed in `c86e689` |
| 7 | low | A password change kept the current session's secret, so a copied cookie survived it unseen | Fixed in `3700d01` |
| 8 | low | The verify page posted the token on load, so a link scanner or a curious click verified an address for someone else's account | Fixed in `895ef49` |
| 9 | low | Demoting an admin and then removing them left every invite they had listed working | Fixed in `fd36167` |
| 10 | low | No anti-framing header, so a same-site page could clickjack one-tap actions | Fixed in `e92d691` |
| 11 | low | API answers with personal data carried no `Cache-Control` | Fixed in `e92d691` |
| 12 | low | Diary drafts stayed in localStorage after logout and account deletion | Fixed in `e88f02e` |
| 13 | low | `SMTP_URL` accepted cleartext `smtp://` and opportunistic STARTTLS in production | Fixed in `895ef49` |
| 14 | low | Anyone who knew an address could fill its daily mail limit and silence its reset mails and security notices | Fixed in `895ef49` |

`7ba61db` regenerates `server/.sqlx/` and `frontend/src/api/generated.ts` for all of the above.

## Findings and fixes

### 1. [medium] Unlimited argon2 work through the current-password check

`verify_current_password` ran `login_failures.check_and_hit("u:<id>")`, verified, and cleared the key when the password was right. One session could send `PATCH /api/auth/password` with `{P, P}` (400 `Pick a different password`) without end, many requests at a time. Every hash and verify on the instance shares four permits, and requests could wait up to the 600 s request deadline. Every login, sign-up, token creation and reset confirm then queued behind the attacker.

**Fix:**
- Every current-password check counts, right or wrong, before the verify, in two buckets that nothing clears: `password_check_session` (`s:<session_id>`, 10 per 10 minutes) and `password_check_user` (`u:<id>`, 30 per hour).
- `PUT /api/auth/me/email` checks `email_send_user` before the password without counting, and counts once the password is right, so a refused change costs no argon2 work.
- As a backstop, a password operation waits at most 10 s for a permit and then fails with `password::Busy`. The server answers 503 `The server is busy. Try again in a moment.`, a new string listed in E §11.3 and mapped in `apiErrors.ts`.

C §5.4, §5.5, §10.1; E §2.

**Tests:**
- `auth_account.rs`: `current_password_checks_count_per_session_and_a_right_one_clears_nothing`, `current_password_checks_are_limited_per_account`.
- `auth_email.rs`: `the_change_limit_is_checked_before_the_password` (the refusal comes while the password semaphore is held).
- `auth::password` unit test: `a_full_semaphore_answers_busy_after_the_wait`.

### 2. [medium] Old private ratings and quote text in the activity feeds

`status_event` wrote `rating` and `take` into every `shelf_finished` payload, and the feeds redacted only by the actor's current entry. An only-me rating came back in three cases:
- the work was removed and added again with the default audience visibility;
- a take written while only-me was rewritten and the entry shared;
- a work merge moved an only-me entry's event onto an audience entry.

`quote_added` also served the excerpt frozen at creation, which could be text the owner had kept only-me.

**Fix:** a stored `shelf_finished` payload's `rating` and `take` are sealed to the actor (`"private": true`) unless its actor's entry for the work exists and is `audience`. Others never see a sealed rating again, whatever the entry becomes; the actor keeps seeing their own, as before.
- `status_event` seals them for an only-me entry.
- `activity::seal_finished_ratings` seals them whenever the actor has no audience entry for the work. It runs on removal, on a save as only-me, before and after a work merge, and once after the legacy import (counted as `events.payload.rating_take`).
- The feeds serve a `quote_added` item's `excerpt` and `page` from the quote as it is now.
- The read-time redaction stays as a backstop, and the seal itself is never served.

The first version (`4ef65a5`) removed the rating and take from the stored payload. That also took them from the actor's own feed, which `parity_visibility.rs` pins ("myself" sees their own only-me rating). `d12fe29` seals them instead.

C §6.4; E §9; legacy-mapping §12.

**Tests:**
- `club_visibility.rs`: `a_private_rating_stays_hidden_when_the_work_comes_back`, `a_removed_entry_seals_the_ratings_of_its_events`, `a_take_written_while_only_me_stays_hidden_after_the_entry_opens_up`, `making_a_rating_only_me_seals_it_in_the_events`, `a_merge_does_not_show_the_rating_of_an_only_me_entry`, `a_shared_quote_shows_its_current_text`.
- `activity::feed` unit test `the_seal_is_never_served`.
- `shelf_entries.rs::shelf_writes_record_their_events` now expects the removed entry's event sealed.

### 3. [medium] Failed logins by others blocked a signed-in owner

The current-password check used the same `login_failures` key `u:<id>` that anonymous logins by username fill. Ten wrong logins every 10 minutes locked the owner out of changing their password or address and out of deleting their account (DSGVO Art. 17).

**Fix (with #1):** the current-password check has buckets of its own, keyed by the session (and a per-account ceiling that only the account's own sessions can fill). Failed logins never touch them, and wrong current passwords no longer block the login. C §5.4, §10.1.

**Tests:**
- `auth_account.rs::failed_logins_and_current_password_checks_do_not_block_each_other`
- `auth_delete.rs::failed_logins_by_others_do_not_block_the_deletion`
- `auth_email.rs::a_wrong_password_changes_nothing_and_counts_as_a_check`

**Not changed (product decision):** the login lockout itself. The finding notes that the attacker also locks the owner out of logging in on a new device. That is the per-account login limit of C §10.1, a documented trade-off carried over from the legacy app (auth spec §2.3, and Wave 1 security #2). Counting failures per account and client address, under a higher ceiling per account, would keep one remote address from locking the owner out. It would also give a distributed guesser more tries per account. That trade-off is Rafael's to decide. A password reset already clears both login keys.

**Decision record (Q17 in `docs/phase2/work-plan.md` §4, #55):** open, waiting for Rafael.
- **A, recommended for launch:** keep counting per account, as today: 10 failures per 10 minutes, with the username and the verified address in separate buckets (`u:<id>`, `u:<id>:email`). Strangers cannot see usernames, and a password reset clears both buckets.
- **B:** count per account and client address (`ip_key`, so an IPv6 /64), under a ceiling per account of about 100 failures an hour. One remote address could then no longer lock the owner out, but a guesser with many addresses would get up to the ceiling instead of 10 tries per 10 minutes.
- Under A the code stays as it is. Under B, ADM2 changes the login limits (`server/src/auth/login.rs`, `server/src/ratelimit.rs`, C §10.1) and their tests in `server/tests/auth_login.rs`.
- When Rafael answers, record the option and the date here.

### 4. [medium] Avatar uploads monopolised the picture pipeline

`PUT /api/auth/me/avatar` had no rate limit, and every picture shares one pipeline of two permits: uploads, Open Library cover fetches, prefetch and spine colours. Small 25 Mpx PNGs cost about a second of CPU each. A body was buffered (up to 50 MiB) before the semaphore. The permit was also borrowed in the async function, so a client that disconnected freed it while its decode kept running.

**Fix:**
- `avatar_user` allows 10 uploads per account an hour.
- Each account may run one picture upload at a time (`upload_gate`, a new `ratelimit::KeyedGate`). The turn is taken before the body is read; a second upload gets 429 `Slow down a little. Try again later.`
- Cover uploads check `upload_user` before the body, without counting.
- The pipeline moves an owned permit into its blocking task (`media::service::with_picture_permit`).

C §10.1; E §2; `notes/med.md` deviation 5 is superseded.

**Tests:**
- `media_avatars.rs`: `avatar_uploads_are_limited_per_account`, `a_second_upload_while_one_runs_is_refused`
- `media::service` unit test: `a_cancelled_caller_leaves_the_permit_with_the_running_job`
- `ratelimit` unit tests of the gate

**Not taken:** a permit of its own for the Open Library cover fetches. With one upload per account at a time and 10 avatars an hour, one account can no longer fill the queue.

### 5. [medium] One user used up the Open Library budget

Search, subjects, trending and the import of unknown works share one `ol_api` throttle of 3 requests a second. Request paths wait at most 2 s for a slot. None of these routes had a per-user limit. Empty answers are not cached, and `page` was unbounded, so one script could keep the throttle full and give every other user 429 `The library is busy`.

**Fix:** `catalog::upstream_turn`, used by search, trending, subjects, the import of an unknown `OL…W`, and a stub's first view:
- `catalog_user` allows 60 such requests per account a minute (429 `Slow down a little. Try again later.`). A first view over the limit serves the stub without enriching it.
- `catalog_gate` lets two of them run per account at a time; further ones wait for a turn. One account then holds only a few throttle slots, and every other request still gets one within its wait.
- `page` of search and subjects stops at 100.

C §10.1; E §4.

**Tests:**
- `catalog_search.rs`: `catalog_requests_are_limited_per_account`, `an_account_has_two_catalog_requests_at_a_time`, `pages_stop_at_100`
- `catalog_detail.rs::imports_and_first_views_count_against_the_catalog_limit`

**Not taken:** caching empty answers for a short time. The parity tests `empty_search_pages_are_not_cached` and `empty_responses_do_not_freeze_retries` (marked KEEP) pin that a lucky-empty answer is never cached. The per-account limits already bound what one account can send upstream.

### 6. [medium] The export held the whole account in memory

Each export built every section as a `serde_json::Value`, then a pretty copy, then the deflated copy. Up to 5 exports per user ran at once, with no global limit. A client that stopped reading kept the producer, and all of that memory, alive until the connection died: the streamed body does not count against the request deadline, and hyper has no write timeout.

**Fix:**
- Two exports stream at once on the instance (`RateLimits::exports`) and one per account (`export_gate`). Both are taken before the rows are read and held until the stream ends; a refused export gets 429 and does not count against the 5 per hour.
- `data.json` is serialised once, which drops the `Value`, then deflated and sent in 256 KiB slices.
- A chunk the client does not take within 60 s ends the export, and so does a download still running after an hour.

C §5.7, §10.1; E §2.

**Tests:**
- `auth_export.rs::exports_run_two_at_a_time_and_one_per_account`
- `auth::export` unit tests: `a_client_that_stops_reading_ends_the_export`, and the ZIP test, now written in slices

**Not taken:** a cap on the number of posts and quotes. The export's memory is now bounded per export and in number, which was the defect. Abuse through mass posting is a moderation matter.

### 7. [low] A password change kept the current session's secret

A copy of the session cookie, from a shared browser profile or an extension, is the same `sessions` row. The session list could not show it, and a password change deleted only the other rows.

**Fix:** the change deletes the current cookie session as well, creates a new one with the same label, and answers with its `Set-Cookie`. A bearer token keeps its secret, because the app holds it. C §5.1, §5.4; E §2.

**Tests:** `auth_account.rs`: `a_password_change_renews_the_cookie_and_signs_out_copies`, `a_password_change_by_bearer_token_keeps_the_token`.

### 8. [low] The verify page verified on load

**Fix:**
- A new read-only `POST /api/auth/verify-email/preview` answers `{username, email}` for any token that verify would accept.
- The page shows "{email} becomes the email address of the account {username}" and verifies only on the reader's tap, as `/join` already did.
- New UI strings are in both `en.json` and `de.json`.

C §5.5, §17.7; E §2.

**Tests:**
- `auth_email.rs::a_preview_names_the_account_and_changes_nothing`
- `AuthPages.test.ts`: "names the account and confirms the address only on a tap"
- the public allow-list in `auth_required.rs`

### 9. [low] Demote-then-remove left the invites the admin had seen

This was the residual of Wave 1 security #5. The demotion half, "a demotion revokes nothing", stays the product default, and the fix does not change it.

**Fix:**
- A demotion from admin now records `club_memberships.admin_until` (migration `1900_club_memberships_admin_until.sql`, in the integrator's block). It uses `clock_timestamp()` under the club lock, so it orders with the invites' dates.
- A removal revokes every active invite created up to that time, as it already revoked all of them for a current admin.
- What remains is documented: links forwarded to a member, and the codes of an admin who left the club and came back (the membership row goes on leave).

C §5.3, §13; E §3.

**Test:** `clubs_invites.rs::a_removal_revokes_the_invites_the_removed_person_knows`, which now has a demote-then-remove case where a second account is refused the link the demoted admin had seen.

### 10. [low] No anti-framing header

**Fix:** every response carries `Content-Security-Policy: frame-ancestors 'none'`, `X-Frame-Options: DENY` and `X-Content-Type-Options: nosniff`. They are set in the outer layer next to `Referrer-Policy`, so the origin check's 403, a CORS preflight and the panic 500 carry them too. C §3.

**Test:** `platform.rs::every_response_has_referrer_policy_and_a_request_id` asserts the headers on every kind of answer.

**Not taken:**
- A baseline CSP for the SPA shell (script sources and the like). It needs a hash of the inline theme script and care with the Vite build, and framing was the defect. Phase 2 adds it (#55): every answer of the SPA fallback carries `spa::SHELL_CSP`, the theme script moved to `frontend/public/theme-init.js`, and this layer became `if_not_present`, so it no longer replaces the shell's policy. API answers keep `frame-ancestors 'none'`. Tests: `platform.rs::the_spa_carries_the_shell_policy`, `frontend/src/thirdParty.test.ts`.
- HSTS. It belongs to the reverse proxy that terminates TLS (#51), as C §3 records.

### 11. [low] API answers carried no `Cache-Control`

**Fix:** a `route_layer` on the API and admin routes adds `Cache-Control: no-store` wherever the handler set none. `/media` (immutable) and the export (`no-store`) keep their own. C §3.

**Test:** `platform.rs::api_answers_are_not_stored`, which covers `/api/auth/me`, the shelf, a club's invites, the health check and a 401.

### 12. [low] Diary drafts outlived the session

**Fix:** three actions now forget every `bookclub.draft.*` key and `bookclub.club`:
- signing out on this device;
- ending the current session in Settings;
- deleting the account.

They use `session.reset({ leavingDevice: true })` and `drafts.forgetDrafts`. A session that merely expired (the 401 handler) keeps them for the reader's next sign-in, as one verifier advised. `notes/fe.md` item 5; `spec/frontend.md` §3.3.

**Tests:**
- `drafts.test.ts`: "forgets every note on this device, and nothing else", and blocked storage
- `session.test.ts`: "forgets unsent notes and the last club when the reader leaves this device, not when the session expires"

### 13. [low] Cleartext SMTP accepted in production

**Fix:** without `DEV_MODE`, `Config::from_lookup` accepts only `smtps://` or `smtp://…?tls=required`. Plain `smtp://` and `?tls=opportunistic` are refused with a startup error naming `SMTP_URL`, unless the host is `localhost` or a loopback address (a relay on the same host, as both verifiers advised). `DEV_MODE` still talks to a local test server however it likes. C §11.

**Test:** `config::tests::production_smtp_needs_tls_unless_the_relay_is_local`.

### 14. [low] Reset spam silenced an address's mail

**Fix:**
- Links (verification and reset) count in `email_target`. The "email changed" and "address in use" notices each have a per-address bucket of their own (`notice_target`, keyed `changed:<address>` and `in_use:<address>`, 6 a day each). `mail::deliver` takes a `MailKind`.
- A reset request sends nothing while the account has an open reset link (at most one an hour), or when the address's link bucket would drop the mail.
- A link whose mail was not sent is voided (`void_token`), so it never holds back the next request.

C §5.5, §10.1; E §2.

**Tests:**
- `auth_email.rs::link_floods_do_not_silence_the_security_notices`
- `auth_reset.rs`: `one_reset_link_is_open_at_a_time`, `a_reset_the_mail_limit_drops_leaves_no_open_link`, and the reworked `requests_are_limited_per_ip_and_silently_per_account` and `a_reset_link_dies_when_another_is_used`

## Phase 2 Wave 1

A security review of branch `phase-2` after Wave 1 (DEP #51, GATE #55, ADM #54, LEG #53, CUT #52), at `325a7a2`, through the endpoint and operations lenses. Three verifiers tried to refute each finding against the code, and two of them reproduced finding 1 on Postgres 17. A finding counts as confirmed when at least two of the three did not refute it. Severity follows the rubric above.

| # | Severity | Finding | Outcome |
|---|---|---|---|
| 1 | medium | Repeat reports skipped the rate limit but still locked the picture row, so one account could block a takedown | Fixed in `f3556d6` |
| 2 | low | The release job held the registry write token and the host's Docker daemon while it ran actions referenced by tags that can be moved | Fixed in `9754dec` (the actions are pinned); the token is Rafael's decision, see below |
| 3 | low | Root on the app VM can delete every backup | Option A chosen (2026-10-29), set up with the VM; see below |

### 1. [medium] Repeat reports held off a takedown

`report` locked the picture `FOR SHARE`, then waited on the reader's advisory lock, and a repeat returned before `report_user` counted anything. One account could keep the picture share-locked without a break by keeping enough repeats in flight to fill the 10-connection pool. The verifiers corrected the finding here: 3 open requests are not enough. A new `FOR SHARE` locker joins the holders ahead of a waiting `FOR UPDATE`, so an admin's takedown (`FOR UPDATE`) or dismissal (`FOR NO KEY UPDATE`) waited until the 600 s request deadline, or failed on the pool's acquire timeout. Meanwhile the picture stayed visible and every other request slowed down.

**Fix:**
- A repeat is answered from one plain read before the transaction, with no lock. A repeat that races a takedown may still answer `open`, and the takedown then resolves that report.
- A new report takes the reader's advisory lock (`0x6263_0200`) before it locks the picture. Of one reader's requests for one picture, only the one whose turn it is holds the picture.
- Every report request, repeats and 404s included, counts against `report_request_user` first (30 a minute per account). `report_user` (20 new reports a day) is unchanged.
- No `lock_timeout` was added: with nothing left that can starve the takedown, it would only turn a report that waits on a slow takedown into a 500.

C §7.2, §10.1; E §6. The commit also regenerates `server/.sqlx/` and `generated.ts`.

**Tests** (`media_reports.rs`), each failing without the fix:
- `repeat_reports_take_no_lock`: repeats answer while another request of the reader holds its turn, and while a takedown holds the picture.
- `a_report_waiting_for_its_turn_does_not_hold_off_a_takedown`: also fails when only the old lock order is restored.
- `every_report_request_counts_against_a_minute_limit`.

### 2. [low] Release actions resolved by tags that can be moved

`publish` passes `REGISTRY_TOKEN` (the owner's token, `write:package`) to `docker/login-action`, and runs on the `docker` runner, whose jobs get the host's Docker daemon. All its actions were referenced by tags, and `dtolnay/rust-toolchain` by a branch. If an upstream tag were moved, the next release would run that code with the token and with root on the host, and could push to `bookclub-server:stable`, which `deploy/hosted/compose.yaml` pulls.

The token path is older than this branch. `phase-2` added the daemon (`runs-on: docker`) and `setup-buildx-action`. One verifier rejected the finding as pre-existing and not exposed yet: no `REGISTRY_TOKEN` and no `docker` runner exist yet. All three agreed that the `rust-toolchain` part is weaker than stated, because the `server` job gets neither the token nor the daemon.

**Fix:** every `uses:` in `.forgejo/workflows/` names a full commit, with its release in a comment. That covers `release.yml`, and `ci.yml` too, because its `image` job runs on the same runner's daemon. `rust-toolchain` names `toolchain: stable` explicitly. `release.yml` explains how to move a pin, and `deploy/hosted/README.md` ("Releases") says pins move only on purpose.

**Test:** `deploy_env.rs::workflows_pin_every_action_to_a_commit` fails on a `uses:` without a commit and a version comment.

**Left for Rafael:**
- **A dedicated token.** Use one that can write only this image instead of the owner's `write:package` token. Forgejo scopes package tokens per owner, not per package, so this means a bot account in an organisation that owns the image, which moves the image path. Until then, `REGISTRY_TOKEN` can write all of the owner's packages.
- **Moving tags that are not `uses:`.** These still run by tag: the job image `catthehacker/ubuntu:act-24.04`, the BuildKit image `setup-buildx-action` starts (`moby/buildkit:buildx-stable-1`), and production's pull of `bookclub-server:stable`, which no digest or signature checks.
- **The GitHub copies.** `.github/workflows/` is disabled on GitHub and stays unpinned.

### 3. [low] The VM can delete its own off-site backups

The VM's Storage Box key (`/root/.ssh/id_ed25519_backup`, mounted into the restic container through `RESTIC_SSH_DIR`) has full SFTP rights on the repository. `backup.sh` relies on that, because it runs `restic forget --prune` every night. With root on the VM, an attacker can wipe the repository, then drop the database volume, and the heartbeats only notice afterwards.

The verifiers corrected two details:
- The finding's `forget --keep-last 0` is refused, but `forget <ids> --prune`, `--unsafe-allow-remove-all` with a filter, or `rm -r` over SFTP work.
- Mounting only the backup key instead of all of `/root/.ssh` would not help, because root reads the key on the host.

**Not fixed.** The fix changes the backup design that work-plan Q9 settled, and costs money or a second machine. The options:
- **(A) Storage Box snapshots.** Automatic snapshots set up in Hetzner's console, which the SFTP user cannot delete. No code changes, but the snapshots use the box's quota.
- **(B) rest-server `--append-only`.** Runs on a host the VM cannot log in to. `backup.sh` stops pruning, and that host prunes instead.
- **(C) An S3 bucket with object lock.** Changes `RESTIC_REPOSITORY` and the provider.

(A) is the smallest step. **Rafael chose (A) on 2026-10-29**; `deploy/hosted/README.md` describes the setup ("Backups", "Storage Box snapshots") and the restore after a compromise ("Restore", "Restoring from a Storage Box snapshot"). It takes effect when Rafael enables the snapshot plan (work plan §5, item 12). Until the snapshots run, the off-site copy protects against losing the VM, not against someone who controls it.

### Rejected

- [medium] The fork gate on CI's `image` job lets AGit pull requests through, and that job gets the host Docker socket: refuted by all three verifiers.
- [low] The app port is published on all interfaces, and nothing enforces that only the proxy reaches it: refuted by all three verifiers.

## Phase 2 Wave 2

A security review of Wave 2, through the reports and operations lenses: `8ecc528..phase-2` (DEP2 and LEG2 merged, CUT2's record), and ADM2 on its branch `phase2/adm2` at `00fab8a`, which is not merged yet, because the reports on profile pictures exist only there. Three verifiers tried to refute each finding against the code; for finding 7, each ran restic 0.17.3. A finding counts as confirmed when at least two of the three did not refute it. Severity follows the rubric above. Rafael's answer for Wave 2 (2026-10-29) applies: take the recommended option, and leave what only matters at go-live until then. The two ADM2 fixes are commits on `phase2/adm2` and land with its merge; their notes (`docs/phase2/notes/adm2.md`, "Security review fixes") carry the contract changes for it.

| # | Severity | Finding | Outcome |
|---|---|---|---|
| 1 | medium | The export's `reports.about_me` told the owner of open and dismissed reports of their profile picture, with the time; only club mates can file those, so in a club of 2 to 5 it named the reporter | Fixed in `b074148` (`phase2/adm2`) |
| 2 | medium | The update timer deploys whatever the registry tag names within 3 minutes, unchecked, and a pinned version is no freeze | Documented in `ffcb314`, with a digest pin that freezes; signing and a narrower token are Rafael's, see below |
| 3 | medium | The restore after a compromise brought back the attacker's access that lives in the database: plain-text invite codes, a planted session, an admin flag | Fixed in `b202a5f` |
| 4 | low | Replacing or deleting a reported profile picture defeated the report: "remove" was refused, no hash was blocked, and the same file could come back | Fixed in `44a281f` (`phase2/adm2`) |
| 5 | low | The restore after a compromise kept the heartbeat URLs the attacker knows, so they could keep the new VM's checks green | Fixed in `5c41954` |
| 6 | low | The README said the Storage Box snapshots close the gap of Wave 1 finding 3, but everything that would notice a damaged repository runs on the VM | The limit is documented in `e56fd6f`; the check from outside the VM is Rafael's, see below |
| 7 | low | A later backup the same day pruned the backup `update.sh` takes before an update, which a rollback across a migration needs | Fixed in `877be06` |

### 1. [medium] The export named the reporter of a profile picture

`reports.about_me` (`auth/export.rs`, ADM2) listed every report of the user's content, whatever its status, with `status`, `via` and the exact `created_at`. A profile picture can only be reported by someone who shares a club with its owner (`bc_can_see_user`), and a dismissal mails only the reporters. So an owner could export their data (5 times an hour) and learn that a club mate reported their picture, when, and while the report was still open; in a club of two that is one person. ADM2's own rule, that the reporter is never named to the other side (Q12), failed through the export. The verifiers noted that cover, book and edition reports identify much less (any signed-in reader can file them), and that the section was a deliberate Art. 15 choice.

**Fix:** `about_me` holds only removals (`status = 'removed'`, not closed on arrival), one entry per decision (its target and `resolved_at`), as `{target_kind, media_id, work_id, edition_id, reason, status, resolved_at}`: the reason of the decision's oldest report and the decision's date, without `id`, `via` or `created_at`. The statement of reasons already told the owner of every removal. Open and dismissed reports, how many reports a decision closed, and when and how they came in are withheld to protect the reporters (Art. 15 (4) GDPR, § 29 (1) BDSG); `docs/legal/takedown.md` ("Export") says so. Dropping only `created_at` would not have helped in a club of two.

**Tests** (`auth_export.rs`, both failing without the fix):
- `about_me_holds_only_removals_never_who_reported_or_when`: the owner exports after an open report, after its dismissal, after two reports removed in one decision (one entry with exactly the fields above and the decision's time), and after a report closed on arrival (no new entry).
- `every_section_holds_the_users_rows`: removals of four kinds side by side with an open and a dismissed report, which are absent; a second report of the same decision adds no entry.

### 2. [medium] Whoever can push the image deploys it

`update.sh` (DEP2, every 3 minutes) pulls the tag `BOOKCLUB_TAG` names, pinned or `stable`, and starts whatever image it names now, with `.env` (the database password, the SMTP credentials) and a route out. Nothing checks a digest or a signature, and the registry takes a second push under an exact version too. So the user `rafiistcool`, any token with `write:package` on that user (`REGISTRY_TOKEN`, which the publish job hands its job image and BuildKit, both pulled by tag), and whoever controls Forgejo deploy to production with nobody in the loop. The README and `env.example` presented a pinned version as a freeze. Two verifiers confirmed the mechanism and rated it low; the third rejected it as the chosen pull deploy (Q8 option A) whose trust Wave 1 finding 2 already lists under "Left for Rafael". What the timer adds is the unattended deploy and a pin that no longer protects.

**Fix (`ffcb314`, documentation):** `deploy/hosted/README.md`, "Updates and rollback", says who can deploy and that a pinned version is no freeze. "Rollback" shows how to freeze the app: pin the running image's digest (`BOOKCLUB_TAG=0.11.0@sha256:…`), or stop the timer. The digest form was checked with Docker 29 and a throwaway local registry: `compose config --images`, `compose pull`, `image inspect` and `compose up` take it, so `update.sh` needs no change, and a `0.11.0` pushed again with other content left the pinned app on its old image. `env.example` says the same. No test: nothing in the code changed.

**Left for Rafael** (with the open items of Wave 1 finding 2):
- **Signed releases.** An unattended deploy that needs more than the token: sign each released digest with a key that is not on the runner (for example `cosign sign --key` from the admin machine after the release run), and have `update.sh` verify the pulled digest before `backup.sh` and `up`, an unverified image counting as a failed update. It adds a manual step to every release and a key to keep.
- **A token for this image only, and the job and BuildKit images by digest**, as Wave 1 recorded: a bot account in an organisation that owns the image (which moves the image path), and digest pins that need updating by hand.

### 3. [medium] The restore after a compromise kept the database's access paths

"Restoring from a Storage Box snapshot" (DEP2) treats the VM as compromised and rotates every secret outside the database, then restored the database unchanged and started the app on it. Whoever had root could read the database and, while they were in, write it. Invite codes are stored as they are, and a link or sign-up invite admits anyone until it expires. A session is the SHA-256 of its token, with no server secret, so a `sessions` row planted for a token the attacker holds signs in on any server with this database, as does an admin flag, a membership or a changed address. The snapshot is from before the damage, not from before the intrusion.

**Fix:**
- `deploy/hosted/revoke-access.sql` deletes every session, revokes every open club and sign-up invite, and marks every unused email link used, in one transaction.
- The runbook now stops after step 4 of "Restoring for real", runs the file, lists the instance admins and what changed since the intrusion may have begun (new accounts, verified addresses, password changes, memberships) with what to do about each, backs up again so that the newest snapshot holds the cleaned database, and only then starts the app.
- A last step names the duties of a personal data breach: the authority within 72 hours unless it is unlikely to put anyone at risk (Art. 33 GDPR), the users when the risk is high (Art. 34), with a request to choose new passwords, and the record of Art. 33 (5). Whether to report is Rafael's call when it happens.
- Not done: forcing new passwords. Accounts without a verified address could not reset theirs, so the runbook asks for new passwords instead.

**Test:** `auth_sessions.rs::the_revocation_after_a_compromise_ends_every_reusable_access` runs the file against the current schema: a cookie session, an app token, a club link, a sign-up invite and a reset link work before it and fail after, and the password still signs in. Dropping any one of its three statements fails the test.

### 4. [low] A reported profile picture could be replaced to dodge the decision

`media/avatar.rs` (`delete_avatars`) deleted every avatar row of the owner on a replace or delete, the reported one too. Its reports lost their picture (`ON DELETE SET NULL`), "remove" answered 409 `The reported picture is no longer online`, no hash went into `media_removed_hashes`, and the same file could be uploaded again once the report was closed. The admin could not even see what had been reported. ADM2's notes listed this as an open integrator item. It applies only once ADM2 is merged: on `phase-2`, only covers can be reported.

**Fix (`phase2/adm2`):**
- A profile picture with an open report is **held** when its owner replaces or deletes it: it leaves the profile at once and keeps its row and files under a new public id, which only the admin queue shows. Its old URL answers 404 like a deleted picture's, and the owner's export leaves it out, so neither tells the owner of the report (finding 1). The avatar rows are locked before the open reports are read, so a report being filed is seen.
- "Remove" takes a held picture down like any other and blocks its hashes. A takedown of a profile picture also takes down the owner's other live profile pictures with the same bytes as received: the same file uploaded again while the first was held. The owner gets one statement of reasons.
- "Dismiss" deletes a held picture with its files, and its reporters learn that it is no longer online.
- `gone` (409 on "remove") remains for a profile picture without its row: an account deleted since.
- Limit: the hash block compares exact bytes, so a re-encoded copy still needs a new report.

**Tests** (`media_reports.rs`, `auth_export.rs`):
- `a_reported_avatar_its_owner_replaces_is_held_for_the_decision`, `an_avatar_uploaded_again_while_reported_comes_down_with_it`, `a_dismissed_report_of_a_replaced_avatar_deletes_it` and `auth_export.rs::a_held_avatar_is_left_out_of_the_export_until_it_is_decided` fail without the fix. Disabling the takedown of the copies, or the deletion of a dismissed held picture, fails one test each.
- `reports_of_the_avatar_of_a_deleted_account_are_closed_as_gone` replaces `reports_of_an_avatar_replaced_meanwhile_are_closed_as_gone` for the gone path that remains.

### 5. [low] The old heartbeat URLs survived a compromise

The snapshot restore set `backup.env` up again "as in Backups, Setting up, step 2", which only fills in the heartbeat URLs, so the old ones came back. They were in the compromised VM's `backup.env`, and a ping URL is its check's only credential. The verifiers corrected the impact: a run that fails still alerts, because each failure ping is a flip to down that healthchecks.io reports whatever comes after. What forged `/0` pings hide is a run that never happens: a timer that is off, a VM that is down, a hung run.

**Fix:** the section creates the three checks anew, puts only their URLs into the new `backup.env`, and deletes the old checks. It also copies `update.sh` with the backup scripts and, at the end, installs and enables the three timers, which a new VM never had: the step it pointed to started only the update timer, whose unit did not exist there. Documentation only.

### 6. [low] The Storage Box snapshots narrow the gap, they do not close it

"Storage Box snapshots" said the box's own snapshots close the gap of Wave 1 finding 3. They rotate (BX11: 10 daily slots), and everything that would notice a damaged repository runs on the VM: the backup, the Sunday `restic check` and restore test, and their heartbeats, whose URLs are in `backup.env`. Root on the VM can stub the scripts and keep pinging, or give the repository a password of their own so that the backups and checks really pass (one verifier's variant), wait until every slot holds the damaged repository, and then drop the database.

**Fix (`e56fd6f`, documentation):** the README says the snapshots narrow the gap, says who notices and who does not, and describes the check that closes it ("A check from outside the VM"): a read-only sub-account of the box, used once a day from a machine outside the VM (`restic --no-lock snapshots --latest 1` and `cat config`), with a heartbeat of its own that alerts on a stale newest snapshot, a changed repository id, or a password that no longer opens it. Its credentials never reach the VM.

**Left for Rafael:** the machine that runs the check, and the sub-account. The work plan's section 5, item 13, has the step.

### 7. [low] A later backup pruned the pre-update backup

`update.sh` (DEP2) backs up before every update, and the README's rollback across a migration restores that snapshot. `backup.sh` ran `forget --keep-daily 7 --keep-weekly 4 --keep-monthly 12`, and `--keep-daily` keeps only the newest snapshot of a day. A hotfix release the same day, the nightly backup after an update past midnight, a backup by hand, or the rollback's own backup pruned it, leaving the previous day's.

**Fix:** `backup.sh` adds `--keep-within 14d`. With restic 0.17.3 on a local repository, an update at 10:00, a hotfix at 15:00, and an update at 00:30 before the 03:40 nightly backup kept all six snapshots, where the old policy kept three. The README, the `backup.sh` header and the retention lines of `docs/legal/` (privacy facts, VVT, processors) say so. The longest retention stays 12 months.

**Test:** `deploy_env.rs::backups_keep_every_snapshot_of_the_last_two_weeks` runs `backup.sh` against a stand-in for `docker` and fails without the window.

### Rejected

None: every finding of this round was confirmed.
