# Phase 1 security review (2026-10-28)

A whole-phase security review of branch `phase-1`: the Rust server in `server/` and the Vue app in `frontend/`, at `18501e0`. Reviewers looked for defects through the authn, authz, visibility, input, resource and transport lenses. Every finding below was confirmed by at least two independent verifiers, who tried to refute it against the code. The integrator then fixed each one at its root, with a regression test that fails without the fix. The behaviour tests of the feed fix were also checked by mutation: with the fix reverted, all six fail.

Severity follows the review rubric: **medium** covers a denial of service by one user and a narrow leak of private data; **low** covers defence in depth and fixes that depend on deployment or on a hostile same-site page.

Abbreviations: C = `conventions.md`, E = `endpoint-map.md`.

## Outcome

| # | Severity | Finding | Outcome |
|---|---|---|---|
| 1 | medium | A right current password cleared its own limit, so one account could run argon2 without limit and starve every login | Fixed in `3700d01` |
| 2 | medium | The activity feeds showed an old only-me rating, take or quote text once the current entry or quote was audience | Fixed in `4ef65a5`, refined in `SEALCOMMIT` |
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

The first version (`4ef65a5`) removed the rating and take from the stored payload. That also took them from the actor's own feed, which `parity_visibility.rs` pins ("myself" sees their own only-me rating). `SEALCOMMIT` seals them instead.

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
- A baseline CSP for the SPA shell (script sources and the like). It needs a hash of the inline theme script and care with the Vite build, and framing was the defect.
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
