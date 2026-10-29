# ADM2 (#54, #55) implementation notes

Owner: ADM2. These notes record what the second admin package built, where it decided something the plan left open, and what the integrator moves into `docs/phase1/endpoint-map.md`, `docs/phase1/conventions.md`, `CLAUDE.md`, `README.md` and LEG2's legal documents. The design documents themselves are unchanged.

Built with Rafael's answers of 2026-10-29 (work plan §4): Q11 = B (reports on covers, profile pictures and the works and editions readers added), Q12 = A (the app mails the decision and the statement of reasons), Q15 = the suspend flag, Q17 = A (no per-address login lockout, nothing to build).

Status: **part 1 (the server) is done**; part 2 (the SPA: `ReportSheet.vue`, the "Report" actions, `AdminReports` for every kind, `AdminUsers.vue`) follows. See "Left for part 2".

## Migrations

- **`server/migrations/2101_reports_targets.sql`**
  - `reports.target_kind` allows `cover`, `avatar`, `work` and `edition` (the constraint keeps its name `reports_target_kind_check`).
  - Target columns, by kind: `cover` uses `media_id` (and `work_id`/`edition_id`, the book that showed it); `avatar` uses `media_id` and the new **`target_user_id`** (whose profile picture it was when reported; `REFERENCES users ON DELETE SET NULL`); `work` uses `work_id`; `edition` uses `edition_id` and `work_id` (its work). Two checks keep them apart: `media_id` only on picture reports, `target_user_id` only on avatar reports.
  - **`via text NOT NULL DEFAULT 'app'`** (`app` | `notice`): a reader's report, or an emailed notice an admin recorded with a direct takedown. Before, a notice and a report whose reporter deleted the account both had only `created_by IS NULL` (adm.md "Open points (part 2)").
  - **`closed_on_arrival boolean NOT NULL DEFAULT false`** (checked: only with `status = 'removed'`). ADM's suggestion (adm.md "Closed on arrival"): it replaces the three-column timestamp test in `ReportOut.closed_on_arrival` and in the summary's statistics, and removes the one ambiguous case ADM recorded. Backfill for older rows: the old timestamp rule, and `via = 'notice'` for `created_by IS NULL AND status = 'removed'` (no production data predates the migration: the Python app runs production, and the importer does not import reports).
  - Indexes: `reports_target_user_id_idx` (the user deletion's `SET NULL`), `reports_open_work_idx`, `reports_open_edition_idx` (a decision closes every open report of a work or edition).
- **`server/migrations/2102_users_suspended_at.sql`:** `users.suspended_at timestamptz NULL` and a partial index for the list. No foreign key. The export's profile is `to_jsonb(u) - 'password_hash'`, so it carries the column (amendment 17).
- **Export check (amendment 19):** `target_user_id` is the 29th foreign-key column to `users`; `server/tests/auth_export.rs` pins 29 and lists it under `reports`. `parity_schema.rs` is unaffected (`reports` stays a global table, no new table). `legal_docs.rs` is unaffected (`reports` is already in `vvt.md`).

## Contract changes

### New routes

| Route | Auth | Request → Response | Errors |
|---|---|---|---|
| `POST /api/books/works/{work_id}/reports` | signed in | `ReportIn {reason, details?}` → **201** `ReportCreated {id, status: "open"}` | 400 `Only books readers added can be reported` (new), `Input should be 'copyright', 'wrong_book', 'offensive' or 'other'`, the body 400s; 401; 404 `No such book.`; 429 `Slow down a little. Try again later.` |
| `POST /api/books/editions/{edition_id}/reports` | signed in | the same | the same |
| `POST /api/admin/users/{username}/suspend` | instance admin | → `AdminUserOut` | 400 `You cannot suspend your own account` (new); 404 `No account with that name` (new); 404 `Not found` for everyone else (the guard) |
| `POST /api/admin/users/{username}/unsuspend` | instance admin | → `AdminUserOut` | 404 `No account with that name`; 404 `Not found` (the guard) |
| `GET /api/admin/users/suspended` | instance admin | → `AdminUserList {items: AdminUserOut[]}`, the latest suspension first | 404 `Not found` (the guard) |

- Operation ids: `catalog_report_work`, `catalog_report_edition` (tag `catalog`, in `catalog::routes()`), `admin_suspend_user`, `admin_unsuspend_user`, `admin_list_suspended_users` (tag `admin`, `admin::users::admin_routes()`, merged by `admin::admin_routes()`; no `lib.rs` change).
- **Work and edition reports** (`server/src/catalog/reports.rs`): only `source = 'user'` records (BookOut `custom`); Open Library, DNB and legacy records are 400 `Only books readers added can be reported` (they are corrected through edit requests and the direct admin edit, Q18). `{work_id}` takes a work id, `OL…W` or a legacy key and follows merges (`require_local_work`), so the report names the work a reader sees now. An edition report stores the edition and its work. Idempotent per reader and target like a cover report (a repeat answers with the reader's open report, same id; a dismissed report does not count). Shared limits with picture reports: `report_request_user` (30 a minute, every request) and `report_user` (20 new reports a day across all kinds). The reader's turn is the advisory lock of picture reports (`0x6263_0200`) with the keys `<user>:work:<id>` and `<user>:edition:<id>`.
- **`/suspend`** normalises the name like every username lookup (`" ROOT "` is `root`), locks the user row `FOR NO KEY UPDATE` (the login's lock), sets `suspended_at` (a second suspension keeps the first time) and deletes every session and bearer token of the account, in one transaction. An admin cannot suspend themselves; another admin can be suspended (then the admin routes refuse them too). **`/unsuspend`** clears the column; an account that is not suspended stays as it is. Both log the admin and user ids at INFO.

### Changed routes

- **`POST /api/media/{public_id}/reports`** also takes a profile picture (the public id in an `avatar_url`), when the reporter can see its owner (`bc_can_see_user`: they share a club, or it is their own); otherwise 404 `Not found`, as for an unknown picture. The row is `target_kind = 'avatar'` with `target_user_id` the owner (`media.uploaded_by`). Everything else (idempotence, closed on arrival, limits, locks) as for covers.
- **`DELETE /api/admin/media/{public_id}`** with a body records a notice about a profile picture too: `target_kind = media.kind`, `target_user_id` the owner for an avatar, `via = 'notice'`, `closed_on_arrival` as before. ADM's refusal `Only notices about covers can be recorded` (`NOTICE_COVERS_ONLY`) is gone, and so is its `apiErrors.ts` line (`api_error_copy.rs` requires every mapped string to exist in the server).
- **`POST /api/admin/reports/{report_id}/resolve`** decides every kind. "remove" and "dismiss" resolve every open report of the same picture (`media_id`), work (`target_kind = 'work'`, `work_id`) or edition (`target_kind = 'edition'`, `edition_id`).
- **Decision notices** on resolve and on the direct takedown (below).
- **`POST /api/auth/login`** and **`POST /api/auth/tokens`**: 403 `This account is suspended` (new) for a suspended account, only once the password matched (`auth::login::confirm`, which both share); a wrong password stays 401 `Wrong username or password`, so the answer tells a guesser nothing. The login rate limits are unchanged (Q17 = A).
- **Every authenticated route:** a suspended account has no live session. `auth::extract` resolves cookie and bearer sessions only for `users.suspended_at IS NULL`, so a session that slipped past the suspension (a login that raced it) is 401 `Not signed in`, and a cookie gets the clearing `Set-Cookie`, like a dead one. `MaybeUser` treats it as signed out.

### What "remove" means per kind

| Kind | "remove" | The other side (statement of reasons) |
|---|---|---|
| `cover` | ADM's takedown, unchanged: files deleted, hashes blocked, every book loses the cover, `/media` 410, open `cover_replace` requests rejected | `media.uploaded_by` (none for Open Library covers) |
| `avatar` | the same takedown. The avatar then shows nowhere: every query that builds an `avatar_url` joins `media … AND removed_at IS NULL` (`dto::user_summaries`, `visibility::visible_user`, `auth::user_out`, `clubs/out.rs`, `posts/out.rs`, `picks/out.rs`), so `users.avatar_media_id` keeps pointing at the removed row (clearing it would lock the user row after the media row, against the avatar upload's order) | the owner, `media.uploaded_by` |
| `work` (`source = 'user'`) | what the reader typed is withdrawn and the record stays, so shelves, posts, picks and votes keep working: title `[removed]` (`catalog::reports::REMOVED_TITLE`), subtitle, authors (and the author links), description and subjects cleared. Through `ingest::apply_admin_fields`, so every field gets provenance `admin` and no source fills it again. Year, pages and the cover stay (a cover is reported on its own) | `works.created_by` (none for imported `BC…` books) |
| `edition` (`source = 'user'`) | its own title, subtitle and publisher cleared (it shows its work's title); ISBN, language, format, pages and year stay. Same provenance rule | `editions.created_by` |

"Remove" of a record withdrawn before changes nothing and sends no statement; the reporters still learn the decision. Lock order of a catalog decision: the work row, then the edition row, then `reports` (ingest's and the direct admin edit's order). The record looked up for an edition is its work as it is now (a merge moves editions).

### Decision notices (Q12 option A)

`server/src/admin/notices.rs` (`decided`, `Notices::send`) and two templates in `server/src/email/templates.rs` (`report_decision`, `statement_of_reasons`, with the `Reported` enum):

- **Built in the decision's transaction, sent after the commit.** The reporters are the `created_by` of every report the admin resolved in this transaction (`resolved_by = admin AND resolved_at = now()`, so a decision that resolves several reports of one target mails each reporter once). A rollback (409, an error) builds nothing that is sent. `Notices::send` sends them one by one (15 s timeout each, logged on failure) in a spawned task that the handler awaits: the admin's answer comes once the mails are handed to the transport, and closing the page does not cancel them. (A detached task without the await could not be observed by the tests: `TestApp::settle` counts `AppState` references, and the test clients hold many.)
- **To the reporter** (`report_decision`): removed or not removed, naming what they reported ("the cover picture of "Circe"", "the profile picture", "the book "X"", "the edition of "X""), never the uploader. Not removed adds where to object (`CONTACT_EMAIL`, else "the admins of {service}").
- **To the other side**, on "remove" only and only when something changed (`statement_of_reasons`, DSA Art. 17): what was removed and what changes, the ground (the decided report's or the notice's reason; for a direct takedown without either, "an admin found that it breaks the law or the terms of use"; a direct takedown without a notice uses the oldest report it resolved), that a person decided, how to object, and `{PUBLIC_URL}/terms`. It never names the reporter or quotes `details`.
- A reporter who is also the owner gets the statement only. Only verified addresses (`email_verified_at IS NOT NULL`) are mailed, in `users.locale`; a suspended account is still told. A notice recorded from an email (`via = 'notice'`) has no reporter account: the admin answers that sender by mail. A work or edition that no longer exists gets no notices.
- Mails go through `state.email` directly, not `auth::mail::deliver` (whose per-address limits are for links and security notices), like the admin notification job.

### Legal footer (#53)

Every template in `templates.rs` ends with a signature separator and one line: `Imprint: {PUBLIC_URL}/imprint · Privacy: {PUBLIC_URL}/privacy` (German: `Impressum: … · Datenschutz: …`), added in the one `mail()` helper every template goes through. `every_template_ends_with_the_legal_footer` checks every `pub fn … -> Email` of the file (a source scan), so a new template fails until it is covered. `email::test_mail` (DEP's `send-test-mail`, in `email/mod.rs`) is not a template and has no footer.

### Changed DTOs (none is a §0 DTO)

- `ReportOut` gains `target_kind` (`cover` | `avatar` | `work` | `edition`), `via` (`app` | `notice`), `avatar_url` (a reported profile picture, `/media/{public_id}/a`, null once removed; `cover_url`/`cover_thumb_url` are now null for avatars) and `target_user: UserSummary | null` (whose profile picture). `book` is the reported book for `work` and `edition` (with the edition). `closed_on_arrival` now reads the column.
- New schemas `AdminUserOut {id, username, avatar_url, is_admin, created_at, suspended_at}` (no address) and `AdminUserList {items}`.
- `ReportIn` and `ReportResolveIn` doc comments name the new kinds; the shapes are unchanged.

### Detail strings

New, all mapped in `frontend/src/i18n/apiErrors.ts` with `errors.*` keys in both catalogs:

| String | Where | Key |
|---|---|---|
| `This account is suspended` | `error::msg::ACCOUNT_SUSPENDED` (login, tokens) | `errors.accountSuspended` |
| `Only books readers added can be reported` | `catalog::reports::ONLY_READER_BOOKS` | `errors.onlyReaderBooks` |
| `No account with that name` | `admin::users::NO_ACCOUNT` | `errors.noAccount` |
| `You cannot suspend your own account` | `admin::users::CANNOT_SUSPEND_SELF` | `errors.cannotSuspendSelf` |

Removed: `Only notices about covers can be recorded` (`media::reports::NOTICE_COVERS_ONLY`) and its `apiErrors.ts` line. The catalog keys `errors.noticeCoversOnly` stay until part 2 removes the form's fallback (`admin.ts` `TAKEDOWN_DETAILS`, `AdminTakeDown.vue`).

### Other changes

- `admin::notify` (the admin notification job) mails only admins who are not suspended.
- `GET /api/admin/summary`: the statistics leave out `closed_on_arrival` rows by the column (same results; the ambiguous case is gone).
- Export (`auth/export.rs`, the reports section): `reports.filed` no longer carries `target_user_id` (another account); new **`reports.about_me`**: the reports of the user's profile picture (`target_user_id`), of pictures they uploaded (`media.uploaded_by`) and of works and editions they added (`created_by`), as `{id, target_kind, media_id, work_id, edition_id, reason, status, via, created_at, resolved_at}`, without `created_by` or `details` (the reporter is never named to the other side). `profile.suspended_at` comes with the `users` row.
- No new environment variable, CLI command or crate.

## Tests

- `server/tests/catalog_reports.rs` (new, 4): a report on a user-added work in the queue (fields, repeat, summary count); "remove" withdraws a work (the API's detail, provenance `admin`, a shelf entry stays, both open reports resolved, mails to both reporters and the creator without names, a second removal sends no second statement); edition reports (dismiss changes nothing and mails only the reporter; remove clears title, subtitle and publisher, the facts stay, a German statement); refusals (Open Library work by id and by `OL…W`, DNB edition, unknown ids, bad reason, anonymous), a report following a merge, and the shared daily limit (19 + 1 work reports, then an edition report is 429).
- `server/tests/admin_users.rs` (new, 5): acceptance (cookie and bearer token 401 after a suspension, sessions gone, login by name and by address and token creation 403, a wrong password still 401, the club membership stays, unsuspend restores login and tokens); a session that slipped through is 401 with a clearing cookie and a suspended admin cannot use admin routes; the checks (self by name and normalised, unknown names, idempotent suspend keeps the first time, unsuspend of an unsuspended account, suspending another admin); the list (newest first, no address); a suspended admin gets no admin notification.
- `server/tests/media_reports.rs` (+5, 2 changed): `an_avatar_report_then_remove_hides_it_everywhere` (acceptance: a stranger 404, a club member reports, the queue item, "remove": `avatar_url` null in `/api/auth/me`, the members list and the report, `/media/…/a` 410, re-upload refused, the reporter's and the owner's mails without names or details); `decisions_mail_the_reporters_and_the_uploader` (acceptance: three reporters in English, German and unverified, one decision, three mails; "dismiss" mails only the reporter); `a_direct_takedown_mails_too` (reporters and uploader; a second takedown mails nobody; the admin's own finding; an Open Library cover); `a_rolled_back_decision_mails_nobody` (acceptance: another admin's decision commits while this one waits for the report row; 409, the takedown rolled back, the picture still served, no mail); `a_notice_about_an_avatar_is_recorded` (replaces ADM's refusal test). `reports_are_checked_and_limited` now uses a stranger's avatar for the 404.
- `server/tests/admin_guard.rs`: 15 admin operations (the three user routes), `users` and `sessions` watched, real targets per operation (`mallory` to suspend, the suspended `sleeper` to unsuspend; `Seeded.op_ids` holds per-operation path values), and the positive control suspends `mallory` (changes `users` and `sessions`). `auth_required.rs` walks the two report routes unchanged (401 for anonymous callers).
- `server/tests/auth_export.rs`: 29 foreign keys, `reports.target_user_id` in `SOURCES`; the section test's reports are avatar reports of each other's picture, so `reports.about_me` holds one row per user, and the test checks it hides `created_by`, `details`, `resolved_by` and `target_user_id`, that `filed` has no `target_user_id`, and `profile.suspended_at`.
- `email::templates` unit tests (+3): the footer on every template in both languages (with the source scan), the reporter's decision in both languages, the statement of reasons per kind, per reason and without a reason.

## Verified by hand (part 1)

The worktree's server (`DEV_MODE=true BIND_ADDR=0.0.0.0:8083 PUBLIC_URL=http://agents.lan:8083 CONTACT_EMAIL=abuse@bookclub.test`, log mail transport, `MEDIA_DIR` in the scratchpad) against the package database, with fictional users registered through the API with a `beta-invite` code (`root` made admin with `make-admin`, `ada` in German, `bob` in a club with `ada`, `carl` in none), driven by a stdlib Python script over HTTP with bearer tokens:

- `carl` reporting `ada`'s new avatar: 404 `Not found`; `bob`: 201. The queue item had `target_kind` `avatar`, `via` `app`, `avatar_url` `/media/…/a`, `cover_url` null, `target_user` `ada`. "Remove": `ada`'s `avatar_url` null, `/media/…/a` 410 `This picture was removed`. The log showed "Your report on Bookclub: removed" to `bob` and "Dein Profilbild bei Bookclub wurde entfernt" to `ada` (the ground "anstößig oder beleidigend", the contact address, `/terms`, the German footer; no name of `bob`, no details).
- `carl`'s custom book reported by `ada`, removed: the answer's book title `[removed]`, authors empty; "Deine Meldung bei Bookclub: entfernt" to `ada`, "A book you added on Bookclub was removed" to `carl` with "Manual Zine" and the copyright ground. `POST /api/books/works/OL1W/reports` (not in the catalog) 404 `No such book.`
- Suspension: `root` suspending `root` 400; `ada` suspended (her token 401, login 403 `This account is suspended`, a wrong password 401 `Wrong username or password`), listed by `GET /api/admin/users/suspended`.
- **`takedown.md` §7, the manual export, step by step:** unsuspend through the API, the SQL of step 2.2 in `psql` (a 46-character `bc_…` token), `curl` of `/api/auth/me/export` gave a ZIP whose `data.json` was `ada`'s (with `reports.about_me` holding the avatar report), suspend again: no session of `ada` left, the token 401.
- Every mail in the log, the sign-up verification mails too, ended with the footer.
- Afterwards: the server stopped, the database reset (`cargo sqlx database reset -y`), the media directory deleted.

Mutation check (reverted): with the `InstanceAdmin` route layer removed from `lib.rs` and `suspend` taking `CurrentUser`, `admin_guard.rs` fails (`POST /api/admin/users/{username}/suspend as owner`: 200, `mallory` suspended).

## For the integrator

- **endpoint-map §4:** the two report routes (table above). **§6:** profile pictures on `POST /api/media/{public_id}/reports` (with the visibility rule), notices about avatars on the takedown (and the removed 400), resolve for every kind with what "remove" does, the decision notices, `ReportOut`'s new fields. **§2:** 403 `This account is suspended` on login and tokens; a suspended account's sessions are refused. **New admin rows:** the three user routes. **§11:** the four new strings, and `Only notices about covers can be recorded` removed.
- **`server/src/auth/tokens.rs` (not ADM2's):** its `#[utoipa::path]` should list `(status = 403, body = ErrorBody, description = "`This account is suspended` (the password matched)")`; the behaviour is in place through `confirm`.
- **conventions §5.3 (who is calling):** suspended accounts have no live session. **§5.7:** `reports.target_user_id` and `reports.about_me`; 29 foreign keys. **§7.2:** the lock order of a catalog decision, and the advisory keys `<user>:work:<id>`, `<user>:edition:<id>` in `0x6263_0200`. **§9:** the two decision templates and the legal footer on every template. **§16/§10:** nothing new.
- **LEG2 / `docs/legal/` (not ADM2's, except `takedown.md`):**
  - `privacy-facts.md` §5 (mail provider row): add the decision notices (`report_decision`, `statement_of_reasons`) and say every mail carries the imprint and privacy links; §4 or §7: the suspension (`users.suspended_at`, sessions deleted) and, per amendment 20, that a suspended account loses the in-app export and deletion and gets them through `CONTACT_EMAIL` (`takedown.md` §7 has the steps); the reports export section (`about_me`).
  - `vvt.md`: the reports activity gains profile pictures and user-added books, `target_user_id`, `via`; the suspension as part of abuse handling.
  - `server/tests/legal_docs.rs`: the four ADM2 entries can leave `PLANNED` (the routes exist now). `EDITED_ELSEWHERE` already lists ADM2's files.
- **README / docs for admins:** reports on profile pictures and user-added books; what "remove" does; reporters and uploaders are mailed; suspending accounts (and §7 of `takedown.md` for the manual export and deletion).
- **Frontend (shared catalogs):** `errors.noticeCoversOnly` is unused once part 2 lands.

## Deviations

- **`apiErrors.ts` lost one line** (`Only notices about covers can be recorded`), though it is add-only: the plan has ADM2 remove that check, and `api_error_copy.rs` fails while a mapped string is gone from the server. The catalogs only gained keys.
- **Two extra `reports` columns** (`via`, `closed_on_arrival`) beside the plan's "target columns": ADM's open points asked for them, and `2101` touches `reports` anyway.
- **A read endpoint** `GET /api/admin/users/suspended` (the task allowed one), so the admin page can list and lift suspensions.
- **`reports.about_me`** in the export goes beyond the one new foreign key: reports of pictures and books the user contributed have no key to the user, but are about their content (Art. 15). Without reporter or details.
- **The decision notices are awaited** by the admin's request (in a spawned task), not detached; see "Decision notices".
- **Removed avatars keep `users.avatar_media_id`** (see the table): every reader already skips a removed picture, and clearing it would invert the avatar upload's lock order.
- **`server/tests/parity_visibility.rs` (not ADM2's) gained three add-only rows**, so the suite is green: its `every_personal_column_route_and_rating_field_is_in_the_matrix` requires every foreign key to `users` and every operation with a `username` parameter to be classified. The rows: `reports.target_user_id` as `Not("moderation: whose profile picture was reported, for instance admins only (media_reports.rs); exported to that account without the reporter (auth_export.rs)")`, and `POST /api/admin/users/{username}/suspend` and `/unsuspend` as `Not("instance administration: instance admins only (admin_guard.rs, admin_users.rs)")`. The same kind of rule as amendment 19 (`auth_export.rs`); if the gate reverts the file, the integrator applies these three rows.
- **`frontend/src/pages/AdminPage.test.ts`** (ADM's test, so ADM2's): its `ReportOut` fixture gained the four new required fields, or `npm run typecheck` fails on the regenerated types.

## Left for part 2

- `ReportCoverSheet.vue` → `ReportSheet.vue` for every kind, with "No account? Write to {contact_email}"; the "Report" action on `MemberShelfPage.vue` (the member's profile picture) and on user-added books and editions; `AdminReports` rendering every kind (`target_kind`, `avatar_url`, `target_user`, `via`), the "remove" wording per kind; `AdminUsers.vue` (suspend by name, the list with "unsuspend"); the takedown form's avatar fallback removed (`admin.ts` `TAKEDOWN_DETAILS`, `AdminTakeDown.vue`, the `errors.noticeCoversOnly` keys); `takedown.md` updated for the renamed sheet and the new screens; a browser check.
- `client.ts` calls for the new routes.

## Open points

- **`[removed]`** is a stored English title, shown to readers of both languages. The SPA could render a localized label for a custom book with exactly that title (part 2), or Rafael picks another placeholder (one constant).
- **Suspension mails:** the app sends none; DSA Art. 17 covers suspensions too, so the operator writes the statement by hand (`takedown.md` §5). A template like `statement_of_reasons` with a reason field on `/suspend` would be small if the legal source wants it.
- **Notices about books by email** have no record path of their own: the admin files a report in the app with the notice in its details (`takedown.md` §2). A body on a future "take down a book" admin route could mirror the picture takedown.
- **Who suspended** is not stored (only when); the mailbox record holds it. A `suspended_by` column would be a new foreign key to `users` (export count).
