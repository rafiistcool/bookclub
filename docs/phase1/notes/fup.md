# FUP (after Wave 3, #36 #48 #49): small follow-ups

Owner: FUP. FUP takes five items the Wave 3 packages found and left for the integrator (`notes/fe.md` "For the integrator" and "Server issues found by the smoke test", `notes/int.md`). These notes record what changed, the decisions behind it, and what is left. Other packages' notes are unchanged, apart from the "fixed" marks in `notes/fe.md`, two pointers in `notes/har.md`, and "Changed by FUP" pointers in `notes/posts.md` and `notes/shelf.md`. Where those notes still describe the old behaviour, this file is the record.

## What was built

### A. Admin queue: every request carries its book

- **Bug:** `catalog::admin::edit_request_outs` took each `BookOut` out of its map with `books.remove(&BookRef { work_id, edition_id })`. Of two open requests on the same work and edition, only the first got `book`; the second answered `book: null`.
- **Fix:** it now clones the book (`books.get(..).cloned()`).
- **Test:** `tests/catalog_requests.rs::requests_on_the_same_edition_both_carry_the_book` files a correction of an edition and a photo for it, then checks that both listed requests carry the work. It failed before the fix (`book: null` on the second request).
- **Frontend:** `components/admin/AdminRequests.vue` no longer reads `GET /api/books/works/{id}` for a request without `book`. That read existed only for this bug. A request without `book` now shows "This book is no longer in the catalog."
  - The merge target is still read: the queue names it only by id.
  - `pages/AdminPage.test.ts`: the case "reads the books the queue names without carrying them" became "reads only the merge target". Two requests on one edition both show the book, a request without `book` says the book is gone, and the only read is for the merge target.
- The other `books.remove` call sites were checked and left as they are. `catalog::hits::annotated_hits` leaves out a repeated `BookRef` on purpose (its doc says so). `shelf::entries` and `shelf::favorites` read one row per user and work, so their keys never repeat.

### B. Old zone names are accepted and stored as the current name

- **Bug:** `tz::parse_zone` used `TimeZone::get`, and jiff's `TimeZone::get` reads the host's tzdb first. Without `tzdata-legacy`, Debian 13 lacks 113 of tzdb's 162 links, `Asia/Calcutta`, `Europe/Kiev` and `America/Buenos_Aires` among them. That covers both the agents VM and `debian:trixie-slim`, the server image's base. The image built here earlier (`bookclub-server:scaffold`) has tzdata 2026c, older than the bundled 2026e. So those names got 400 `That is not a valid time zone`. On a host without tzdata the bundled tzdb would have accepted them and stored them as sent.
- **Fix (`server/src/tz.rs`):**
  - `parse_zone` keeps its frozen signature and validates against `TimeZoneDatabase::bundled()`. A name passes if the bundled tzdb knows it in that exact spelling (case-sensitive, as before).
  - For an old name (`LINKS`, decision 2), `parse_zone` returns the zone it links to. jiff's `iana_name()` returns a link's own name unresolved (checked: `Asia/Calcutta` → `Some("Asia/Calcutta")`), so a table is needed.
  - New `tz::canonical_zone_name(name) -> Result<String, InvalidZone>` returns `parse_zone(name)?.iana_name()`. Every writer stores its result:
    - `RegisterIn` (sign-up)
    - `PreferencesIn` (`PATCH /api/auth/me/preferences`)
    - `clubs::manage::validate_zone` (club create and `PATCH /api/clubs/{slug}`)
    - the importer's `check_args`, which now writes the current name back into `args.timezone` (`run` takes `mut args`)
  - `zone_or_utc` falls back to the bundled tzdb when the host's lacks a name. A stored name that `parse_zone` accepted, for example a zone newer than the host's tzdata, then never reads as UTC. The lookup is `zone_or_utc_with(name, host)`, which takes the host lookup as a closure so the fallback can be tested on any host.
  - The DTO doc comments of the three zone fields (OpenAPI, so `generated.ts`) and the importer's `--timezone` help say that an old name is stored as the current one.
- **Tests:**
  - Unit tests in `tz`:
    - `old_names_resolve_to_the_current_name`: links resolve; current names, `zone.tab` names and `UTC` stay as sent; wrong-case, made-up names and `Etc/Unknown` stay refused.
    - `stored_names_fall_back_to_the_bundled_tzdb`: through `zone_or_utc_with` with a host that knows nothing, so it fails without the fallback whatever tzdata the test machine has (seen failing). A host that knows the name wins.
    - `the_link_table_matches_the_bundled_tzdb`: sorted; every old name and target is known by its exact spelling; no chains; the same UTC offsets on 1 January and 1 July of every year 1970–2039; 150 entries.
    - `the_bundled_tzdb_is_the_release_the_link_table_was_built_from`: the bundled tzdb's names, sorted, have count 598 and FNV-1a hash `b3abd19f04a249e2`. Any jiff-tzdb update that adds, drops or renames a name fails it until `LINKS` is compared with the new `tzdata.zi` (decision 2).
  - New integration tests, each of which failed before the callers changed (the link was accepted and stored as sent):
    - `tests/auth_register.rs::an_old_zone_name_is_stored_as_the_current_one`: `Asia/Calcutta` → `Asia/Kolkata` in the answer and the row; `asia/calcutta` and `Asia/Atlantis` are 400 and create no user.
    - `tests/auth_account.rs::an_old_zone_name_is_stored_as_the_current_one`: the same through the preferences PATCH; the bad names change nothing.
    - `tests/clubs_manage.rs::an_old_zone_name_is_stored_as_the_current_one`: create with `Asia/Calcutta`, then PATCH with `America/Buenos_Aires` → `America/Argentina/Buenos_Aires`; wrong-case and made-up names are 400 and change nothing.
    - `tests/importer_run.rs::an_old_zone_name_is_imported_as_the_current_one`: `--timezone europe/kiev` is refused before anything is written; `Europe/Kiev` imports the club and every user as `Europe/Kyiv`.
- **SPA (`frontend/src/timezone.ts`):** the sign-up retry without the zone is unchanged. The `currentZoneName` table gained five names that ICU browsers report and `LINKS` resolves: `America/Coral_Harbour` → `America/Atikokan`, and `CST6CDT`, `EST5EDT`, `MST7MDT`, `PST8PDT` → `America/Chicago`, `America/New_York`, `America/Denver`, `America/Los_Angeles` (decision 9). Its 18 earlier names map to the same current name as `LINKS`. The header comment no longer says that the server refuses old names.
  - `timezone.test.ts` reads `LINKS` from `server/src/tz.rs` and checks that every name the running Node's ICU reports (its `supportedValuesOf("timeZone")` and its name for every `LINKS` entry) maps to the name the server stores. It failed on the five names before they were added.

### C. A visibility-only PATCH is no edit

- **Bug:** `posts::edit::update_post` set `updated_at = now()` on every PATCH. Turning a diary entry "Only me" and back marked it "edited" for every reader (`notes/fe.md` "Server issues found by the smoke test").
- **Fix:** `updated_at = CASE WHEN $7 THEN now() ELSE updated_at END`, where `$7` is "`body` or `spoiler_upto` was sent". Content edits move it exactly as before, including a PATCH that sends unchanged text (R8). A PATCH with only `visibility` leaves it alone, and one with `body` and `visibility` moves it.
- **Quotes** have the same PATCH shape (`{body?, page?, visibility?}`). `QuoteOut` has no `edited` flag, but `quotes.updated_at` is in the account export. `shelf::quotes::update_quote` now follows the same rule: `body` or `page` moves it, `visibility` alone does not.
- **Shelf entries** (`PATCH /api/shelf/{id}` with only `rating_visibility`) no longer move `updated_at` either (decision 8, added after review).
- **Docs:**
  - The module comment and the operation description of `posts/edit.rs` (OpenAPI, so `generated.ts`).
  - endpoint-map §7 (`PATCH /api/quotes/{quote_id}`, `PATCH /api/shelf/{entry_id}`) and §8 (`PATCH /api/posts/{post_id}`).
  - After review: spec/diary.md §1.3 step 4 and R8 (each with a "Phase 1" line: `visibility` alone no longer moves `updated_at`; R8 holds for `body` and `spoiler_upto`), `notes/posts.md` (the PATCH line and deviation 1) and `notes/shelf.md` decision 10, each with a "Changed by FUP" pointer.
- **Tests:**
  - `tests/posts_diary.rs::a_visibility_change_alone_is_no_edit`:
    - `only_me` and back leave `edited` false and `updated_at` NULL, also as another member sees the entry.
    - A spoiler change sets `edited`.
    - A visibility-only PATCH after it leaves `updated_at` at the same instant.
    - `body` with `visibility` moves it on.
  - `tests/posts_diary.rs::edits_mark_the_entry_and_change_its_visibility` now also checks that `"audience"` on a reply answers `edited: false`.
  - `tests/shelf_quotes.rs::a_visibility_change_alone_is_no_edit`: the same for `quotes.updated_at`, with a page change as the edit.
  - `tests/shelf_entries.rs::a_rating_visibility_change_alone_is_no_change` (decision 8).
  - The new tests failed before the fix.

### D. The importer slugs its club with club creation's helpers

`importer/club.rs` had its own copies of `clubs::manage::slug_base` and `slug_suffix`. They were identical apart from visibility and comments: same transliteration, same 50-character cut, same `club` fallback and same alphabet. The importer now imports both from `clubs::manage`, where `slug_suffix` became `pub(crate)`, and the copies are deleted with their `MAX_SLUG_BASE` and `SLUG_ALPHABET`. `SLUG_ATTEMPTS` stays separate: 8 for club creation and 20 for the importer, each its own loop. `importer::club::tests::slugs_follow_club_creation` and every importer test stay green.

### E. Docs made stale by Wave 3

- **`CLAUDE.md`:**
  - The checks block gained `npm run check:api`, which CI runs in the server job. A line says that `generated.ts` comes from `npm run gen:api`.
  - The Vite paragraph names the proxy to the Rust server on 8080 (`/api` and `/media`) and the agents-VM recipe from `notes/fe.md`. The first account comes from `beta-invite`.
  - "Request flow" now says that `npm run build` writes `frontend/dist/`, which the Rust server serves as `STATIC_DIR` with `no-store` on `index.html`. The SPA speaks only the Rust API, so running the Python app with it is not supported. Production runs the Python app with its own built SPA until #52, and no Python release is cut from `phase-1`.
  - "Backend features without a frontend": the password form is back (Settings, `PATCH /api/auth/password`). Push is still absent: the Python push endpoints have no frontend, and the Rust server drops `/api/push/*` until Phase 4. The notification switches exist for that.
  - The API errors rule names only `server/tests/api_error_copy.rs`.
- **`README.md`:**
  - The repository layout lists `server/` as not released yet.
  - The build-path sentence says what a release does (`backend/app/static/`) and what this branch does (`frontend/dist/` for the Rust server).
  - A short "In transition" note opens "Develop".
  - The one-process run says it works from a release tag.
  - After review: "Building from source" (the Docker overlay) says it works from a release tag, as the root `Dockerfile` fails on this branch on purpose, and the Troubleshooting entry "Vite loads but login/search fails" says that on this branch Vite proxies `/api` and `/media` to the Rust server on `:8080`. Both sit outside the "In transition" note.
  - Nothing unreleased is described as released.
- **`notes/har.md`:** one "Fixed by INT" line under the "Open points" item and one under decision 15's deleted-record bullet, each pointing to `notes/int.md` decisions 1–3.
- **`notes/fe.md`:** the fixed items are marked "Fixed by FUP":
  - the queue's `book: null` (twice)
  - the visibility-only edit (three places: part 3, decision 27, the server issue)
  - the zone names (decision 28 and "For the integrator")
  - `CLAUDE.md` and `README.md`
- **`parity-checklist.md`:** a "Known intentional changes" row for old zone names, which Python stored as sent.
- The root `Dockerfile` and the release workflow are untouched (`notes/fe.md`: the Python image must not be cut from `phase-1`).

## Decisions and deviations

1. **`canonical_zone_name` is a new function in the frozen `tz` interface** (conventions §12 updated). `parse_zone` keeps its signature, and its zone's `iana_name()` is the current name, as asked. The helper only keeps four callers from repeating `iana_name().map(..).ok_or(..)`.
2. **Which names count as old (`tz::LINKS`).** The bundled tzdb (jiff-tzdb 0.1.9, tzdb 2026e, 598 names) does not mark links: a link and its target share the same data, and so do zones that tzdb merged (`Europe/Oslo` has `Europe/Berlin`'s data). The table is therefore fixed in the source and generated once:
   - Source: the 162 `Link` lines of 2026e's `tzdata.zi` as built with `backzone` for `zone.tab` (`PACKRATLIST=zone.tab`, the file Debian 13 ships). Its zones plus links are exactly the bundled names.
   - Removed: the 11 links that `zone.tab` lists, each a country's own name that browsers report (`Europe/Bratislava`, `Europe/Podgorica`, `Arctic/Longyearbyen`, `America/Kralendijk`, …). Storing `Europe/Prague` for a Slovak reader would make Settings offer "use this device's zone" forever.
   - Removed: `UTC`, the column default and the name every client sends for UTC (a link to `Etc/UTC` in 2026e).
   - That leaves 150 entries.
   - The `backzone` build keeps every `zone.tab` name a zone. So `Pacific/Truk` → `Pacific/Chuuk` (as the SPA maps it), not `Pacific/Port_Moresby`, and `Europe/Oslo`, `Atlantic/Reykjavik` and `America/Curacao` stay as they are.
   - Other UTC aliases (`Etc/Zulu`, `Universal`, `UCT`, …) resolve to `Etc/UTC` and the GMT aliases to `Etc/GMT`, as tzdb links them. Only the plain `UTC` is kept.
   - **After a jiff-tzdb update**, compare `LINKS` with that release's `tzdata.zi`. `the_link_table_matches_the_bundled_tzdb` fails if an entry stops existing or stops matching its target. `the_bundled_tzdb_is_the_release_the_link_table_was_built_from` fails on any change to the bundled names, a new link included, because jiff cannot tell a link from a zone; update its count and hash once `LINKS` is checked.
3. **Validation reads the bundled tzdb, labels the host's first.** Writes must not depend on the host's tzdata: the old behaviour differed between Debian, a host without tzdata, and one with `tzdata-legacy`. Reads keep `TimeZone::get`, so a newer host tzdata still corrects rules, and only fall back to the bundled tzdb for a name the host lacks. conventions §12 said the slim image "needs no tzdata". That stays true, but its claim that the server ran "on top of" the bundled tzdb was not: jiff prefers the host's. §12 now says so.
4. **Python parity for zones.** Python's `ZoneInfo` accepted links and the app stored the name as given. The server stores the current name instead, so every stored name is one that any tzdata has (a reasoned change, now in `parity-checklist.md`). Python on Linux was case-sensitive, and so is the server.
5. **Quotes follow the posts rule (item C)**, although `QuoteOut` shows no edit. Python's quotes had no `updated_at` (spec/diary.md §1.9), so no parity rule binds it. Keeping one rule for both PATCH shapes is simpler to explain, and the export no longer dates a visibility switch as an edit.
6. **`slug_suffix` is shared too (item D).** The item named `slug_base`, but the importer's `slug_suffix` was the same kind of identical copy, in the same two files.
7. **`CLAUDE.md` beyond the named lines.** The checks block lacked `npm run check:api`, which CI has run since OAPI, so "CI runs all of them" was stale too. It got that one command and one sentence about `gen:api`.
8. **A shelf entry's `rating_visibility` alone is no change either** (after review, in `shelf/entries.rs`, SHELF's file). `PATCH /api/shelf/{id}` with only `rating_visibility` used to run `place_item` like a rating-only PATCH (`notes/shelf.md` decision 2), which renumbers the column and moves `updated_at`. That `updated_at` is no edited flag, but it is the year an abandoned book counts in (`MemberYear.dnf`: "counted in the year of their last change"), so hiding the rating of a book dropped in 2025 moved it into this year's stats. `ShelfPatchIn::places_item` now skips `place_item` for that PATCH, as spec/shelf.md §1.7 step 8 already does for a `shelf_position`-only one. Every other field sent with it (rating, take, `dnf_reason`, progress, `finished_on`, `edition_id`, status, position) still moves `updated_at`, so the legacy rule stands for every field Python had. Python had no `rating_visibility`, so no parity rule binds it. spec/shelf.md (step 8 and port risk 2), spec/diary.md (the stats note), endpoint-map §7, the field's OpenAPI description and `notes/shelf.md` decision 2 say so.
9. **ICU's old names are mapped in the SPA, not kept out of `LINKS`.** ICU still reports `America/Coral_Harbour` for Atikokan and `CST6CDT`, `EST5EDT`, `MST7MDT`, `PST8PDT` as they are (Node 26, ICU 78.3, tz 2026e). The server now stores the current name, so a device on one of them saw "use this device's zone" in Settings for good. For `America/Coral_Harbour` that was a regression of item B, since Debian's tzdata has the name and the server stored it as sent before. The fix is in the SPA's `currentZoneName` table, like the 18 names that were already there. Keeping these five names out of `LINKS`, as the 11 `zone.tab` names are, would have stored names that are not current. The `zone.tab` names stay because they are a country's own name, not an old spelling. **Rule:** every name in `LINKS` that ICU browsers report must be in the SPA's table. `timezone.test.ts` checks this against the ICU of the Node that runs it, so a newer ICU in CI can add a name to the list.

## For the integrator

- **No migration and no new crate.** `.sqlx/`: two query files were replaced, for the `UPDATE posts` in `posts/edit.rs` and the `UPDATE quotes` in `shelf/quotes.rs`.
- **`generated.ts` changed** (committed, from `npm run gen:api`): the descriptions of `RegisterIn.timezone`, `PreferencesIn.timezone`, `ClubIn.timezone` and the `posts_update` operation. No type changed. (The new description of `ShelfPatchIn.rating_visibility`, decision 8, sits inside a `oneOf`, which openapi-typescript does not print, so it left `generated.ts` as it was.)
- **Files outside FUP's list, changed after review** (the review asked to finish these): `server/src/shelf/entries.rs` and `server/tests/shelf_entries.rs` (decision 8); `docs/phase1/spec/diary.md` and `docs/phase1/spec/shelf.md` ("Phase 1" lines only); `notes/posts.md` and `notes/shelf.md` ("Changed by FUP" pointers only).
- **`tz::LINKS`** needs a look whenever `jiff-tzdb` is bumped (decision 2). `cargo update` that brings a new jiff-tzdb fails `the_bundled_tzdb_is_the_release_the_link_table_was_built_from` until then.
- **Frontend test reads server source:** `timezone.test.ts` reads `server/src/tz.rs` through `node:fs` (Vite serves nothing outside `frontend/`), as `server/tests/api_error_copy.rs` reads `frontend/src/i18n/apiErrors.ts`. It needs the whole repository checked out, which CI has.
- **Root `Dockerfile`:** still fails at `COPY --from=frontend /src/backend/app/static`, on purpose (`notes/fe.md`). FUP did not touch it.

## For PAR (#49)

- One new "Known intentional changes" row (old zone names). No `/// parity:` line changed. The new tests port no Python test: the Python app had neither per-user zones nor post visibility.

## Tests

- Server: 12 new tests (4 unit tests in `tz`, 8 integration tests: `catalog_requests.rs`, `auth_register.rs`, `auth_account.rs`, `clubs_manage.rs`, `importer_run.rs`, `posts_diary.rs`, `shelf_quotes.rs`, `shelf_entries.rs`), and one extended (`posts_diary.rs::edits_mark_the_entry_and_change_its_visibility`).
- Frontend: one test rewritten (`pages/AdminPage.test.ts`) and one added (`timezone.test.ts`, decision 9).
