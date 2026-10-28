# ADM (#54) implementation notes

Owner: ADM. These notes record what the admin-queue package built, where it decided something the plan left open, and what the integrator moves into `docs/phase1/endpoint-map.md`, `docs/phase1/conventions.md`, `CLAUDE.md` and `README.md`. The design documents themselves are unchanged.

Status: part 1 of 2 is done (work-plan ADM scope items 1 to 4). Items 5 to 7 (the takedown record and the "take down by URL" form, the summary and history, the admin notification) follow in part 2; their notes will be added here.

## Contract changes

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

## Open points and notes for part 2

- **A field an admin clears does not stay cleared** (blocked on the frozen `ingest.rs`; see `blocked_on_frozen` in the part 1 report). `apply_admin_fields` drops the provenance of a cleared field (`admin_mark(…, present = false)` → `prov.remove`), and `decide` fills every empty field. Checked by hand with a throwaway test: a description an admin cleared through `PATCH /api/admin/works/{id}` came back from Open Library on the next refresh (`field_sources.description = "openlibrary"`). Q18 ("remove a description on notice with the direct admin edit") depends on the fix. The edits that set a value do stick (tested).
- **Summary and history (part 2):** reports closed on arrival (a removed picture) have `resolved_by = NULL` and `resolved_at = created_at`. The 90-day resolution stats should count only rows an admin resolved (`resolved_by IS NOT NULL`), or they pull the median towards zero; the history view can label them "closed: already taken down".
- **"Edit as admin" from the queue** opens the record as it is now, not pre-filled with the proposal, and the request stays open. The alternative ("approve with changes": pre-fill the proposal, then approve and patch) changes only `AdminRequests.vue`.
- The admin form edits the fields the reader's form has. `planned_publication_date` is accepted by the PATCH but has no input yet (`editRequests.ts::EDITION_FIELDS`, which ADM does not own, lacks it).
