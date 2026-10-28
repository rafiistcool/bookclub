# ADM (#54) implementation notes

Owner: ADM. These notes record what the admin-queue package built, where it decided something the plan left open, and what the integrator moves into `docs/phase1/endpoint-map.md`, `docs/phase1/conventions.md`, `CLAUDE.md` and `README.md`. The design documents themselves are unchanged.

Status: both parts are done. Part 1 built work-plan ADM scope items 1 to 4 (the sections up to "Open points (part 1)"); part 2 built items 5 to 7 (from "Part 2" on): the takedown record and the "take down by URL" form, the summary, paging and history, and the admin notification job.

## Contract changes (part 1)

### New routes

| Route | Auth | Request → Response | Errors |
|---|---|---|---|
| `PATCH /api/admin/works/{work_id}` | instance admin | `AdminEditIn {fields}` → `BookDetailOut` (the work as it is now, seen by the admin) | 400 `Field required` (no or empty `fields`), `Unknown field: <key>`, `Invalid value for <key>`; 404 `No such book.`; 404 `Not found` for everyone else (the guard) |
| `PATCH /api/admin/editions/{edition_id}` | instance admin | `AdminEditIn {fields}` → `EditionOut` | the same |

- Operation ids `catalog_admin_edit_work` and `catalog_admin_edit_edition`, tag `admin`, in `catalog::admin_routes()` (`server/src/catalog/admin.rs`). No `lib.rs` change was needed: `admin_router` already merges `catalog::admin_routes()`.
- `{work_id}` takes a work id, `OL…W` or a legacy key, and follows merges (`require_local_work`), like the direct merge. A merged-away id edits the work it was merged into.
- `fields` has the keys, value rules and null-clears of `EditRequestIn.fields`, and goes through `ingest::apply_admin_fields` in one transaction: every written field gets provenance `admin`. The 400s are the approval's, byte for byte (the test sends each invalid payload both ways).
- A direct edit files no request and leaves no record. Part 2 may add an audit row if Rafael wants one.

### Changed DTOs (none of them is a §0 DTO)

- `EditRequestOut.current: object` (new, always present). For `work_edit` and `edition_edit`: the record's value now for each key of `payload`, under the same key. Empty text is `null` (as a proposal clears it), subjects are an array, `planned_publication_date` is `YYYY-MM-DD`. A `work_edit` reads the work its `work_id` was merged into (the work an approval would change). `{}` for `cover_replace`, `work_merge`, a record that is gone, and keys the server does not know.
- `ReportCreated.status: string` (new): `open` (an admin will look at it) or `removed` (the picture was already taken down; nothing waits in the queue). `id` is still always set.
- New schema `AdminEditIn {fields: object}`.

### Changed behaviour

1. **Reports are idempotent per reader and picture** (`POST /api/media/{public_id}/reports`). Still 201 `ReportCreated` in every success case:
   - A reader who already has an **open** report of the picture gets that report back (same `id`, `status: "open"`). Nothing is inserted or changed, and the 20-a-day limit is not hit again (a repeat answers even at the limit).
   - A report of a picture that was **already taken down** is recorded as a closed row: `status = 'removed'`, `resolved_at = created_at`, `resolved_by = NULL` (no admin decided it). It never enters the open queue, and the answer says `status: "removed"`. A repeat by the same reader returns their `removed` report, which is also the report that took the picture down, if they filed it.
   - A reader whose earlier report was **dismissed** files a new open report (counted).
   - The limit (`report_user`, 20 a day) now counts new rows only. endpoint-map §6 says "20 per day": make it "20 new reports per day; a repeat of an open report is answered with it and not counted".
2. **"dismiss" closes every open report of the picture** (`POST /api/admin/reports/{report_id}/resolve` with `dismiss`): `status = 'dismissed'`, `resolved_by` the admin. "remove" already resolved every open report of the picture through the takedown. A report whose `media_id` is NULL (media row deleted) resolves only itself.
3. **A takedown rejects the picture's open `cover_replace` requests** (`media::reports::take_down_locked`, so both the direct takedown and "remove"): `status = 'rejected'`, `decided_by` the admin, `decided_at = now()`, `decision_note = 'The picture was taken down'` (`media::reports::TAKEN_DOWN_NOTE`; a stored note, not an API error string, so nothing to map in `apiErrors.ts`). Before, such a request stayed open and approving it failed with 400 `This picture was removed`.

### Lock order (conventions §7.2 "Covers and takedowns", for the integrator)

Everything that touches a picture's reports or requests now locks the media row first, so takedowns, reports, dismissals and cover approvals take turns instead of deadlocking:

- **Takedown:** media `FOR UPDATE` → works and editions (`clear_media`) → reports → `catalog_edit_requests` (new).
- **Cover approval** (`approve_edit_request`): the request's picture `FOR SHARE` (new: `lock_request_media`, before the request row) → the request row `FOR UPDATE` → catalog rows. Without the new first lock, an approval holding the request row and a takedown holding the picture deadlocked (SQLSTATE 40P01, a 500). `catalog_requests.rs::a_takedown_rejects_a_cover_request_an_admin_is_approving` shows it: with the lock removed the approval answers 500, with it 409 `This request was already decided`.
- **Report:** media `FOR SHARE` → the advisory lock `pg_advisory_xact_lock(0x6263_0200, hashtext('<user id>:<media id>'))` (a new two-key lock space for "one reader's reports of one picture", beside ingest's `0x6263_0100` and `jobs::lock_ids`) → the reports row. A report filed while a takedown runs either sees the removal or is resolved by it.
- **Dismiss:** media `FOR NO KEY UPDATE` → reports. Two dismissals, or a dismissal and a takedown, of one picture take turns; a report filed meanwhile is dismissed too.

### Detail strings, env variables, CLI, migrations

- No new API error string, so `error.rs` and `apiErrors.ts` are unchanged. The new routes answer existing strings only (`Field required`, `Unknown field: <key>`, `Invalid value for <key>`, `No such book.`, `Not found`).
- No new environment variable, CLI command, crate or migration.

## Frontend

- `api/client.ts` (add-only): `edition(editionId)` (`GET /api/books/editions/{edition_id}`, which existed without a client call), `adminEditWork(workId, fields)`, `adminEditEdition(editionId, fields)`.
- `admin.ts`: `proposedFields` returns `before` (the current value, labelled, or null when not known), `wasEmpty` and `same` (the proposal is what the record holds) beside `value` and `cleared`.
- `components/admin/AdminRequests.vue`: each proposed field shows "Now" (struck through, or "(empty)") above "Proposed"; a long description is clamped to four lines. Every card that names a book has **Edit as admin**: it reads the request's edition (when `edition_id` is set) or else its book fresh, opens the admin form, and after a save reads the queue again (the `current` values change). The request stays open for its own decision.
- `components/EditRequestSheet.vue`: an `admin` prop turns the reader's correction form into the admin's: no note, "Save for everyone", `PATCH` instead of a request, and a `saved` event with the answer. One form, one set of field rules (`editRequests.ts`, unchanged).
- `pages/BookDetailPage.vue`: for instance admins (`UserOut.is_admin`) a line "Edit as admin" (the book) and, when the reader has chosen an edition, "Edit your edition as admin". A book edit adopts the answer; an edition edit re-reads the book and remounts the edition picker (`:key`), so the page shows the new values.
- `components/admin/AdminReports.vue`: a decision drops every listed report of the same picture (the server closed them all).
- `components/ReportCoverSheet.vue`: `status: "removed"` toasts "This picture was already taken down. Thanks anyway."
- New strings (both languages): `admin.empty`, `admin.now`, `admin.proposed`, `admin.noChange`, `admin.editAsAdmin`, `admin.editEditionAsAdmin`, `admin.edit.{title,editionTitle,blurb,save,saved,failed,loadFailed}`, `cover.alreadyRemoved`.

## Tests

- `server/tests/admin_guard.rs` (rewritten): seeds an open edit request and an open report (both filed through the API by a reader), a live picture (the work's cover), a work with an edition and a second work, and walks every admin operation with those **real ids and a valid body each** (approve, reject with a note, resolve with `remove`, merge into the second work, the two PATCHes with a change, an invite with `max_uses`). It asserts the exact list of 11 admin operations (a new one fails the test until it is added with its ids and body), 404 `Not found` for a club owner and an unverified user, and that every row of `works`, `editions`, `media`, `media_removed_hashes`, `catalog_edit_requests`, `reports`, `club_invites` and `legacy_work_keys` is unchanged. A positive control sends three of the same requests as an admin and sees exactly the expected tables change.
  - **Mutation check (done by hand, reverted):** with the route layer in `lib.rs` removed and `approve_edit_request` taking `CurrentUser` instead of `InstanceAdmin`, the new test fails (`POST /api/admin/edit-requests/{request_id}/approve as owner`: 200, the request approved). The Phase 1 version of the test, run against the same mutation, still passed.
- `server/tests/media_reports.rs`: `a_repeat_report_answers_with_the_open_report`, `reports_of_a_removed_picture_never_enter_the_queue`, `dismissing_closes_every_open_report_of_the_picture`, `a_takedown_rejects_the_cover_requests_for_the_picture`. `reports_are_checked_and_limited` now reaches the limit with 20 different pictures (20 reports of one picture are one report now), and the end of `a_takedown_resolves_every_open_report_of_the_picture` (which reported the removed picture into the open queue) moved into the new removed-picture test.
- `server/tests/catalog_requests.rs`: `edit_requests_carry_the_current_values` (work and edition values, null for empty text, `{}` for cover and merge requests, `current == payload` after approval), `current_values_follow_a_merge`, `a_takedown_rejects_a_cover_request_an_admin_is_approving` (the lock order; see above).
- `server/tests/catalog_admin_edit.rs` (new): provenance `admin` and a refresh that keeps the admin's values while it updates the subjects; an edition and work edit that outlast a DNB re-harvest of the same record (which still updates the publication year); every invalid payload gets the same 400 from the PATCH and from approving a request with that payload, and the rows are unchanged; empty `fields` and a missing body are `Field required`; unknown ids are 404 `No such book.`; anonymous 401, non-admin 404; an edit of a merged-away id lands on the surviving work.
- Frontend: `admin.test.ts` (before and after values, `(empty)`, "already so", German), `pages/AdminPage.test.ts` (before and after in the cards, Edit as admin on a book and on an edition, a 400 shown in the sheet, the queue read again after a save, one decision closing every report of a picture), `components/ReportCoverSheet.test.ts` (the removed-picture toast), `pages/BookDetailPage.test.ts` (only admins see the links; book and edition edits).

## Verified by hand

The built SPA served by the server (`STATIC_DIR=../frontend/dist BIND_ADDR=0.0.0.0:8083 PUBLIC_URL=http://agents.lan:8083 DEV_MODE=true`) against the package database, with fictional users (`root` made admin with `make-admin`, `ada`, `grace`), driven with `playwright-cli` at 1280 px and 390 px, light and dark:

- `curl`: a repeat report answered with the same id; a non-admin `PATCH /api/admin/works/1` got 404 `Not found`; `{"pages": 0}` got 400 `Invalid value for pages`.
- `/admin`: a book correction and an edition correction show "Now" and "Proposed" per field, "(empty)" for an empty field, no horizontal scroll at 390 px (field names go above their values there). "Edit as admin" on the edition request opened the edition form; saving showed the toast, the queue re-read showed the new values with "already so", and the database had `field_sources.publisher = "admin"` and `pages = "admin"`.
- Reports tab: two reports of one cover; "Keep it" on one closed both (both rows `dismissed` by `root`).
- `/book/1` as admin: "Edit as admin" changed title and year in place (`field_sources` `admin` for both), "Edit your edition as admin" changed the year and the edition picker showed it at once. No console errors.
- Found and fixed on the way: the "(empty)" value first took the app's global `.empty` empty-state style (a dashed box); the class is `was-empty` now.

## For the integrator

- **endpoint-map §4:** add the two PATCH routes (table above). Add `current` to the `EditRequestOut` description of `GET /api/admin/edit-requests`.
- **endpoint-map §6:** the report route (idempotent; `ReportCreated {id, status}`; 20 new reports per day), resolve (`dismiss` closes every open report of the picture), and the takedown (rejects the picture's open `cover_replace` requests with the note `The picture was taken down`).
- **conventions §7.2 "Covers and takedowns":** the lock order above, including the new advisory lock space `0x6263_0200`. Conventions §7 "writers": the direct admin edit is a second caller of `apply_admin_fields` (today: "admin edits (`apply_admin_fields`, CAT's approve)").
- **README / docs for admins:** "Edit as admin" on a book page and in the queue changes a book's facts at once.

## Open points (part 1)

- **A field an admin clears does not stay cleared** (blocked on the frozen `ingest.rs`; see `blocked_on_frozen` in the part 1 report). `apply_admin_fields` drops the provenance of a cleared field (`admin_mark(…, present = false)` → `prov.remove`), and `decide` fills every empty field. Checked by hand with a throwaway test: a description an admin cleared through `PATCH /api/admin/works/{id}` came back from Open Library on the next refresh (`field_sources.description = "openlibrary"`). Q18 ("remove a description on notice with the direct admin edit") depends on the fix. The edits that set a value do stick (tested).
- **Summary and history:** done in part 2. Reports closed on arrival (a removed picture) have `resolved_by = NULL` and `resolved_at = created_at`; the 90-day statistics count only rows with `resolved_by IS NOT NULL`, and the history labels the others "Closed on arrival: the picture was already taken down".
- **"Edit as admin" from the queue** opens the record as it is now, not pre-filled with the proposal, and the request stays open. The alternative ("approve with changes": pre-fill the proposal, then approve and patch) changes only `AdminRequests.vue`.
- The admin form edits the fields the reader's form has. `planned_publication_date` is accepted by the PATCH but has no input yet (`editRequests.ts::EDITION_FIELDS`, which ADM does not own, lacks it).

# Part 2: takedown record, summary and history, admin notification

## Contract changes (part 2)

### New and changed routes

| Route | Auth | Request → Response | Errors |
|---|---|---|---|
| `GET /api/admin/summary` (new) | instance admin | → `AdminSummary` | 401 `Not signed in`; 404 `Not found` for everyone else (the guard) |
| `GET /api/admin/reports?status=open\|resolved&page=n` (changed) | instance admin | → `ReportList {items, page, has_more}` | 400 `Input should be 'open' or 'resolved'`, `Input should be greater than or equal to 1`, `Input should be less than or equal to 1000`, `Input should be a valid integer, unable to parse string as an integer` |
| `GET /api/admin/edit-requests?status=open\|resolved&page=n` (changed) | instance admin | → `EditRequestList {items, page, has_more}` | the same |
| `DELETE /api/admin/media/{public_id}` (changed) | instance admin | optional body `TakeDownIn {details?, received_at?, reason?}` → **204** | 400 `Input should be 'copyright', 'wrong_book', 'offensive' or 'other'`, `Input should be in the past` (new), `Invalid value` (a `received_at` that is not RFC 3339), the usual body 400s; 404 `Not found` |

- **Summary** (`server/src/admin/summary.rs`, operation id `admin_summary`, tag `admin`, merged through the new `admin::admin_routes()`):
  - `target_hours`: 72 (`admin::queue::TARGET_HOURS`, D4; Q12 may change it, one constant).
  - `reports` and `edit_requests`, each `AdminQueueSummary {open, overdue, oldest: AdminOldestItem {id, created_at} | null}`. `overdue` counts open items older than `target_hours`. Both queues use the same threshold.
  - `report_stats: AdminReportStats {days: 90, resolved, median_hours, max_hours, over_target}` over reports with `status <> 'open'`, `resolved_by IS NOT NULL` and `resolved_at` in the last 90 days. Hours are `resolved_at - created_at`, rounded to 0.1; median and max are null when nothing was resolved. `over_target` counts those that took longer than `target_hours`. Reports closed on arrival (`resolved_by` null) are left out, as part 1 recommended.
- **Paging** (`server/src/admin/queue.rs::QueueQuery`, shared by both lists): `status` defaults to `open` (oldest first, `created_at, id`), `resolved` lists `removed`/`dismissed` reports by `resolved_at DESC, id DESC` and `approved`/`rejected` requests by `decided_at DESC, id DESC`. 50 a page (`PAGE_SIZE`); `page` is 1 to 1000, default 1; a page past the end is `{items: [], has_more: false}`. Without parameters both answer the first page of open items, as before plus the two new fields.
- **Takedown record** (`media::reports::take_down`, D2): without a body, or with an empty one, nothing changes (204, no row). With a JSON body, the takedown also inserts one `reports` row: `target_kind 'cover'`, the picture and the book that showed it (read before the takedown clears the covers), `reason` (default `other`), `details` (cut to 2000 characters, as a reader's), `created_by NULL` (the sender has no account here), `created_at = received_at` (default now; up to 5 minutes ahead of the server clock is accepted and clamped to now, later is 400 `Input should be in the past`), `status 'removed'`, `resolved_by` the admin, `resolved_at now()`. It is in the history and in the statistics, never in the open queue.
  - A picture that was down already still gets the record. If the notice arrived before the picture came down, it is resolved at `media.removed_at` by the admin who records it; if it arrived after, it is closed on arrival (`resolved_by NULL`, `resolved_at = created_at`), like a reader's report of a removed picture.
  - Avatars are taken down without a record: `reports.target_kind` allows only `cover` until ADM2's `2101_reports_targets.sql`. ADM2 then drops the `picture.kind == "cover"` filter in `take_down` and writes `target_kind = media.kind`.
  - The body is read by a small `OptionalJson<T>` extractor in `media/reports.rs`: an empty body is `None`, anything else goes through `ApiJson` with its 400s. It could move to `extract.rs` (frozen for ADM) if another route needs it.
  - `TakeDownIn.reason` is an addition to the plan's `{details, received_at}`: emailed notices are mostly about copyright, and the history and statistics read better with the reason.

### Changed DTOs (none is a §0 DTO)

- `ReportList` and `EditRequestList` gain `page: int64` and `has_more: bool`.
- New schemas: `AdminSummary`, `AdminQueueSummary`, `AdminOldestItem`, `AdminReportStats`, `TakeDownIn`.

### Detail strings

- New: `Input should be in the past` (`media::reports::RECEIVED_IN_FUTURE`, pydantic's `PastDatetime` message) and `Input should be 'open' or 'resolved'` (`admin::queue::STATUS_CHOICES`). Neither is mapped in `apiErrors.ts`: the SPA never sends an unknown status, and the takedown form refuses a future arrival time itself with `admin.takeDownForm.future`. Mapping them would need `errors.*` keys, which are outside ADM's part of the catalogs.

### Migration, job, templates

- **`server/migrations/2100_admin_notify_state.sql`:** `admin_notify_state (queue text PK CHECK IN ('reports','edit_requests'), last_item_id bigint NOT NULL DEFAULT 0, sent_at timestamptz NULL)`, both rows inserted by the migration. No foreign key to `users`, so `auth_export.rs` is unaffected.
- **The job** (`server/src/admin/notify.rs`, `jobs::admin_notify_loop`, lock `jobs::lock_ids::ADMIN_NOTIFY = 0x6263_0003`): every 5 minutes (`RUN_EVERY`; the first tick runs at startup), under `run_locked`. Per queue, when the interval since `sent_at` has passed (1 hour for reports, 24 hours for catalog requests) and open items with `id > last_item_id` exist, it mails every user with `is_admin AND email_verified_at IS NOT NULL`, in `users.locale`, then sets `last_item_id` to the newest of them and `sent_at = now()`. The state moves on only when at least one mail went out, so an SMTP outage, or an instance without a verified admin, loses nothing. Items resolved before a run are no news. Mails go through `state.email` directly (15 s timeout each), not through `auth::mail::deliver`, whose per-address limits are for links and security notices.
  - So the first report after a quiet hour reaches the admins within 5 minutes; further reports wait for the hour. The digest is "at most once a day when something new arrived", not a fixed time of day (that would need a time zone per admin).
  - On the first start after the migration, `last_item_id = 0`, so the admins get one mail per queue about what is open already.
  - No new environment variable (D1). The job runs whenever the server runs; with `EMAIL_TRANSPORT=log` the mails go to the log.
- **Templates** (`email::templates::{admin_reports, admin_requests_digest}`, `QueueNews {new, open, oldest_hours}`), en and de by `users.locale`: counts and the oldest item's age only, never a reporter, a username other than the recipient's, or what a report says. Reports link to `{PUBLIC_URL}/admin?tab=reports`, the digest to `{PUBLIC_URL}/admin?tab=requests`. Ages read "less than an hour", "71 hours", then "3 days" (hours below the 72-hour target, as on the admin page). ADM2's legal footer (`templates.rs:39,73,110,154` in the plan) also goes into these two.

### Shared files touched

- `server/src/lib.rs`: `admin_router` merges `admin::admin_routes()` instead of `admin::invites::admin_routes()` (the one ADM line), and the comment beside the route layer names it.
- `frontend/src/api/client.ts` (add-only): `adminSummary()`, `adminReportPage(status, page)`, `adminEditRequestPage(status, page)`, `adminTakeDown(publicId, notice?)`, and the imports `AdminSummary`, `TakeDownIn`. `adminReports()` and `adminEditRequests()` are now unused; the add-only rule kept them, and the integrator may delete them.
- `frontend/src/pages/SettingsPage.vue`: the admin link (`:244`) is `<AdminLink />`, plus its import line (`:7`, beside `AccountSettings`). LEG's section at the end is untouched.
- `frontend/src/i18n/{en,de}.json`: new `admin.*` keys only.
- **Outside the plan's list** (needed because the plan's migration adds a table): `server/tests/parity_schema.rs` (`admin_notify_state` in `GLOBAL`, 15 → 16) and the "Neither" row of the #37 table in `docs/phase1/parity-checklist.md`. Without them `club_scoped_tables_carry_club_id_and_personal_tables_do_not` and `the_checklist_classifies_every_table_like_the_schema_test` fail by design. One line each, add-only.

## Frontend (part 2)

- `pages/AdminPage.vue`: reads `GET /api/admin/summary` on mount and after every decision; both tab badges show the open count from it (not the length of the loaded list), in the danger colour with a "N overdue" title when something is overdue. Passes `target_hours` and `report_stats` down. `?tab=` works as before (the mail links use it).
- `components/admin/useQueue.ts` (new): one paged queue with an "Open" and a "Decided" view, "Show more", and dropping of decided items. The next page is `floor(shown / 50) + 1` with the shown ids dropped (`admin.ts::nextQueuePage`), so a decision in the open view cannot make "Show more" skip an item.
- `components/admin/AdminReports.vue` and `AdminRequests.vue`: an "Open | Decided" switch. Open items show "waiting 5 hours" and, from 72 hours, an "Overdue" badge and a red border. "Decided" shows the status, "Taken down by root · date · after 50 hours" (requests: "Approved by …" / "Rejected by …" and the decision note), "Closed on arrival: …" for a report without `resolved_by`, and no buttons; a decided request shows its proposal without "Now" values. The reports history starts with the 90-day figures (decided, median hours, longest, took longer than 72 hours).
- `components/admin/AdminTakeDown.vue` (new): "Take down a picture by URL", a `<details>` above the open reports. It takes a picture address (`…/media/{public_id}/…`), a bare public id, or a book page (`…/book/{id}` or a legacy key, which means the book's cover through `GET /api/books/works/{id}`), when the notice arrived (`datetime-local`, default now, refused when in the future), the reason (default copyright) and what the notice says. The first tap shows the picture and the takedown warning, the second sends `DELETE` with the notice; then the queue and the badges are read again.
- `components/admin/AdminLink.vue` (new), used in Settings: the admin link with the number of open reports and requests (visually a pill, for screen readers "3 items waiting"), marked when one is overdue; the plain link when the summary fails.
- `admin.ts`: `pictureRef`, `hoursBetween`, `formatAge`, `isOverdue`, `nextQueuePage`, `QUEUE_PAGE_SIZE`, `toLocalInput`, `fromLocalInput`.
- New strings (both languages): `admin.{views,openView,history,noHistory,loadMore,waiting,overdue,overdueCount,waitingCount,decidedAfter,takenDownBy,keptBy,approvedBy,rejectedBy,closedOnArrival}`, `admin.age.{underAnHour,hours,days}`, `admin.status.{open,removed,dismissed,approved,rejected}`, `admin.stats.{title,resolved,median,max,overTarget}`, `admin.takeDownForm.{toggle,blurb,url,urlPlaceholder,receivedAt,reason,details,check,notAPicture,noCover,future,done,failed}`. German ages are in the dative ("wartet seit 3 Tagen", "nach 5 Stunden").

## Tests (part 2)

- `server/tests/admin_summary.rs` (new, 6 tests): an empty instance; a report resolved after 50 h gives median 50 and `over_target` 0, a second after 80 h gives median 65, max 80, `over_target` 1, a report closed on arrival and one resolved 91 days ago are not counted, and a direct takedown of a notice received 10 h ago is; open, overdue and oldest per queue; `?status=resolved` newest first with 50 a page, `has_more`, page 2 and a page past the end, `?status=open` and no parameters; the same for catalog requests; the 400s of the query.
- `server/tests/admin_notify.rs` (new, 5 tests): one mail per verified admin in their locale (`root` English, `wurzel` German), none to an unverified admin or a reader, no reporter named; a second run with nothing new sends nothing; a new report within the hour sends nothing, after the hour it sends "1 new report" with 3 open; a report resolved before the run is no news; catalog requests as a digest (not at 23 h, again at 24 h, to `?tab=requests`); a refusing transport and an instance without a verified admin leave the state as it was, and the next run sends; the job runs under `ADMIN_NOTIFY` and skips while another session holds it.
- `server/tests/media_reports.rs`: `a_takedown_with_a_notice_records_it` (the row as above, in the history and not in the open queue; `{}` records reason `other` and now; no body records nothing), `a_notice_about_a_removed_picture_is_recorded_too` (before and after the removal), `a_takedown_notice_is_checked` (unknown reason, future time, malformed time, wrong types, a non-object body: 400 and the picture stays; long details are cut to 2000). Two empty-queue assertions now include `page` and `has_more`.
- `server/tests/admin_guard.rs`: `ADMIN_OPERATIONS` has 12 operations (`GET /api/admin/summary`), and the DELETE takedown is walked with a notice body, so an unguarded takedown would show in the `reports` and `media` snapshots.
- `email::templates` unit tests: both admin mails link to their tab in both languages; counts, singular and plural in English and German; ages.
- Frontend: `admin.test.ts` (URLs, ages in both languages, overdue at exactly 72 h, the next page, `datetime-local`), `pages/AdminPage.test.ts` (both badges on first load with the overdue mark, ages and the overdue flag in both queues and in German, the reports history with the figures, labels and "Show more" calling page 2, the requests history without buttons, the takedown form by picture address and by book page with the future-time refusal and the "no picture" case, the badges re-read after a decision), `components/admin/AdminLink.test.ts`.

## Verified by hand (part 2)

The built SPA served by the server (`STATIC_DIR=../frontend/dist BIND_ADDR=0.0.0.0:8083 PUBLIC_URL=http://agents.lan:8083 DEV_MODE=true`, log mail transport) against the package database, with the fictional users of part 1 (`root` and `grace` verified instance admins, `grace` in German, `ada` a reader), driven with `playwright-cli` at 1280 px and 390 px, light and dark:

- **The job:** at startup it mailed the 3 open catalog requests as a digest, in English to `root` and in German to `grace`, with `http://agents.lan:8083/admin?tab=requests`. A report `ada` filed afterwards (moved 80 hours back in the database) was mailed on the next tick, five minutes later ("1 new report on Bookclub", "the oldest has waited 3 days", `?tab=reports`; German "1 neue Meldung bei Bookclub").
- **curl:** the summary as `root` (1 open report, 3 open requests, the oldest of each, 2 resolved reports in the statistics); 404 `Not found` for `ada`; `?status=resolved` newest first; `?status=x` is 400 `Input should be 'open' or 'resolved'`.
- **Settings:** "Open the admin page" with a pill "4" in the danger colour (one report overdue), read by screen readers as "4 items waiting".
- **`/admin`:** both badges on load ("Requests 3", "Reports 1" in red). Requests show "waiting 50 hours" and "waiting less than an hour"; the report shows "waiting 3 days", "Overdue" and a red border.
- **Take down by URL:** pasted `http://agents.lan:8083/media/<public id>/l` of the picture a `cover_replace` request proposed, a time the day before (browser in CET), copyright and a note. "Show the picture", then "Take down for good": the database had one `removed` report with `created_by` null, `created_at` 08:30 UTC (09:30 CET as typed), `resolved_by` root, the picture and its book; the picture was removed and its cover request rejected with "The picture was taken down"; the badge went from "Requests 3" to "Requests 2". A book page address (`…/book/1`) showed that book's cover in the preview (cancelled there).
- **History:** "Decided" in reports showed the figures (3 decided, median 29.9, longest 50, 0 over 72 hours), the notice ("someone", "Taken down by root · … · after 29 hours", its details) and the two kept reports ("after 6 hours", "after 50 hours"). "Decided" in requests showed the rejected cover request with its decision note and no buttons.
- **390 px, dark:** no horizontal scroll (`scrollWidth` 390), the form's fields stack, the preview and buttons fit.
- The console showed only 404s for `/media/…` bytes that this package database's media directory does not hold (rows left from part 1); nothing from the new code.

## For the integrator (part 2)

- **endpoint-map §6:** `GET /api/admin/summary`; `?status=&page=` on `GET /api/admin/reports` (the row says "open, oldest first" today) and its `ReportList {items, page, has_more}`; the optional `TakeDownIn` body on `DELETE /api/admin/media/{public_id}` and the record it writes. **§4:** the same paging on `GET /api/admin/edit-requests`. **§11:** the two new detail strings above (unmapped).
- **conventions §16:** the admin notification job in the list of jobs (`admin::notify::run`, every 5 minutes, `ADMIN_NOTIFY`), and `ADMIN_NOTIFY = 0x6263_0003` beside `AUTH_CLEANUP` and `DNB_HARVEST`. **§9:** the two admin templates.
- **conventions §5.7 / schema notes:** `admin_notify_state` holds no personal data and no user reference.
- **README / docs for admins:** instance admins with a verified address get a mail about new reports (at most hourly) and a daily digest of catalog requests; "Take down a picture by URL" records an emailed notice; the admin page shows ages, overdue items and the history. `docs/legal/takedown.md` (ADM2/LEG) can say that notices by email are recorded with their arrival time and decided within the target.
- **ADM2:** drop the cover-only filter of the notice record once `target_kind` knows avatars; add the legal footer to the two new templates.

## Open points (part 2)

- The overdue threshold applies to catalog requests too (one constant, D4). If only reports should be flagged, `AdminRequests.vue` drops its two `overdue` uses and the summary keeps counting.
- The direct takedown records `created_by NULL`, so the history shows "someone" for an emailed notice, as for a reporter whose account was deleted. A column telling the two apart (for example `reports.via 'app' | 'notice'`) would need a migration in 21xx.
- The digest goes out at most daily but at no fixed hour. A fixed morning hour per admin would need their time zone in the job.
