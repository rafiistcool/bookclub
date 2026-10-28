# FUP (after Wave 3, #36 #48 #49): small follow-ups

Owner: FUP. FUP takes five items the Wave 3 packages found and left for the integrator (`notes/fe.md` "For the integrator" and "Server issues found by the smoke test", `notes/int.md`). These notes record what changed, the decisions behind it, and what is left. Other packages' notes are unchanged, apart from the "fixed" marks in `notes/fe.md` and two pointers in `notes/har.md`. `notes/posts.md`, `notes/shelf.md` and the specs are not FUP's files: where they still describe the old behaviour, this file is the record, and "For the integrator" holds the lines they need.

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
- **Shelf entries** (`PATCH /api/shelf/{id}` with only `rating_visibility`) still move `updated_at`. The fix is in `shelf/entries.rs`, outside FUP's files, so it is handed to the integrator as a patch (decision 8).
- **Docs:**
  - The module comment and the operation description of `posts/edit.rs` (OpenAPI, so `generated.ts`).
  - endpoint-map §7 (`PATCH /api/quotes/{quote_id}`) and §8 (`PATCH /api/posts/{post_id}`).
  - Not changed, as they are outside FUP's files: spec/diary.md §1.3 step 4 and R8 still say that every PATCH moves `updated_at`, and so do `notes/posts.md` (the PATCH line and deviation 1) and `notes/shelf.md` decision 10. "For the integrator" has the lines they need.
- **Tests:**
  - `tests/posts_diary.rs::a_visibility_change_alone_is_no_edit`:
    - `only_me` and back leave `edited` false and `updated_at` NULL, also as another member sees the entry.
    - A spoiler change sets `edited`.
    - A visibility-only PATCH after it leaves `updated_at` at the same instant.
    - `body` with `visibility` moves it on.
  - `tests/posts_diary.rs::edits_mark_the_entry_and_change_its_visibility` now also checks that `"audience"` on a reply answers `edited: false`.
  - `tests/shelf_quotes.rs::a_visibility_change_alone_is_no_edit`: the same for `quotes.updated_at`, with a page change as the edit.
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
8. **A shelf entry's `rating_visibility` alone should be no change either. Proposed, not applied: it needs `shelf/entries.rs`, SHELF's file.** `PATCH /api/shelf/{id}` with only `rating_visibility` runs `place_item` like a rating-only PATCH (`notes/shelf.md` decision 2), which renumbers the column and moves `updated_at`. That `updated_at` is no edited flag, but it is the year an abandoned book counts in (`MemberYear.dnf`: "counted in the year of their last change"), so hiding the rating of a book dropped in 2025 moves it into this year's stats. The review asked FUP to fix this too. The fix (`ShelfPatchIn::places_item`, which skips `place_item` for that PATCH as spec/shelf.md §1.7 step 8 already does for a `shelf_position`-only one) and its test were written and passed every check, but the ownership gate refused `shelf/entries.rs`, so both were reverted to `phase1-green`. The test went with the fix: on its own it fails (`updated_at` moves to now; seen failing against the reverted `entries.rs`). Both are kept verbatim under "For the integrator". With the patch, every other field sent with `rating_visibility` (rating, take, `dnf_reason`, progress, `finished_on`, `edition_id`, status, position) still moves `updated_at`, so the legacy rule stands for every field Python had. Python had no `rating_visibility`, so no parity rule binds it.
9. **ICU's old names are mapped in the SPA, not kept out of `LINKS`.** ICU still reports `America/Coral_Harbour` for Atikokan and `CST6CDT`, `EST5EDT`, `MST7MDT`, `PST8PDT` as they are (Node 26, ICU 78.3, tz 2026e). The server now stores the current name, so a device on one of them saw "use this device's zone" in Settings for good. For `America/Coral_Harbour` that was a regression of item B, since Debian's tzdata has the name and the server stored it as sent before. The fix is in the SPA's `currentZoneName` table, like the 18 names that were already there. Keeping these five names out of `LINKS`, as the 11 `zone.tab` names are, would have stored names that are not current. The `zone.tab` names stay because they are a country's own name, not an old spelling. **Rule:** every name in `LINKS` that ICU browsers report must be in the SPA's table. `timezone.test.ts` checks this against the ICU of the Node that runs it, so a newer ICU in CI can add a name to the list.

## For the integrator

- **No migration and no new crate.** `.sqlx/`: two query files were replaced, for the `UPDATE posts` in `posts/edit.rs` and the `UPDATE quotes` in `shelf/quotes.rs`.
- **`generated.ts` changed** (committed, from `npm run gen:api`): the descriptions of `RegisterIn.timezone`, `PreferencesIn.timezone`, `ClubIn.timezone` and the `posts_update` operation. No type changed.
- **`tz::LINKS`** needs a look whenever `jiff-tzdb` is bumped (decision 2). `cargo update` that brings a new jiff-tzdb fails `the_bundled_tzdb_is_the_release_the_link_table_was_built_from` until then.
- **Frontend test reads server source:** `timezone.test.ts` reads `server/src/tz.rs` through `node:fs` (Vite serves nothing outside `frontend/`), as `server/tests/api_error_copy.rs` reads `frontend/src/i18n/apiErrors.ts`. It needs the whole repository checked out, which CI has.
- **Root `Dockerfile`:** still fails at `COPY --from=frontend /src/backend/app/static`, on purpose (`notes/fe.md`). FUP did not touch it.

### Changes outside FUP's files (applied by the integrator)

**Applied** on `phase-1` after the merge, both patches unchanged, plus the endpoint-map §7 sentence. `a_rating_visibility_change_alone_is_no_change` was checked to fail without the `entries.rs` change and to pass with it. The text below is kept as the record.

The review asked FUP to finish these. They were written and passed every check, but five of the files are not on FUP's list (`work-plan.md` file ownership), so the gate refused them. They were reverted to `phase1-green`, and so was `server/tests/shelf_entries.rs`, whose only change was the test of the reverted code. Each is needed for one reason:

- **Decision 8 (code and test):** `server/src/shelf/entries.rs` (SHELF's) holds the shelf PATCH, and nowhere else can skip `place_item`. Its test, `tests/shelf_entries.rs::a_rating_visibility_change_alone_is_no_change`, went with it: without the fix it fails (`updated_at` answers now instead of `2025-03-01T12:00:00Z`), and an ignored test is not allowed. The patch below adds both. Apply it from the repository root with `git apply`, then run `cargo test --test shelf_entries`. It changes no `query!` macro (no `.sqlx/` change) and no type: the field's new OpenAPI description sits inside a `oneOf`, which openapi-typescript does not print, so `npm run check:api` stays green without a new `generated.ts`. With it, also add this sentence to the endpoint-map §7 row `PATCH /api/shelf/{entry_id}`, after "strings unchanged.": "A PATCH with only `rating_visibility` leaves the column and `updated_at` alone, like a `shelf_position`-only one." It was taken out with the code, since without the code it is not true.
- **Item C and decision 8 (docs):** `docs/phase1/spec/diary.md` and `docs/phase1/spec/shelf.md` still say that every PATCH moves `updated_at`, and `notes/posts.md` and `notes/shelf.md` describe the old behaviour. Item C is in the code (`posts/edit.rs`, `shelf/quotes.rs`), so the spec and the two notes are now stale for posts and quotes. The second patch adds a "Phase 1" line to each spec rule and a "Changed by FUP" pointer to each note line. Its decision-8 lines (spec/diary.md's stats note, spec/shelf.md step 8 and port risk 2, `notes/shelf.md` decision 2) belong only with the first patch; drop them if that one is not applied.

Decision 8, `server/src/shelf/entries.rs` and `server/tests/shelf_entries.rs`:

```diff
diff --git a/server/src/shelf/entries.rs b/server/src/shelf/entries.rs
index 726c35a..3eeee91 100644
--- a/server/src/shelf/entries.rs
+++ b/server/src/shelf/entries.rs
@@ -163,6 +163,7 @@ pub struct ShelfPatchIn {
     #[serde(default)]
     #[schema(value_type = Option<i64>)]
     pub edition_id: Patch<i64>,
+    /// Who sees the rating and the take. Alone it changes nothing else, not even `updated_at`.
     #[serde(default)]
     pub rating_visibility: Option<Visibility>,
 }
@@ -204,6 +205,24 @@ impl ShelfPatchIn {
             && self.edition_id.is_missing()
             && self.rating_visibility.is_none()
     }
+
+    /// Whether `place_item` runs, which renumbers the column and moves `updated_at`: on every
+    /// PATCH (spec/shelf.md §1.7 step 8) but one whose only placement is `shelf_position`, and
+    /// one that sets nothing but `rating_visibility` (new in Phase 1): hiding or showing a
+    /// rating changes nothing about the reading, so an abandoned book keeps the year of its
+    /// last change in the stats.
+    fn places_item(&self) -> bool {
+        if self.status.is_some() || self.position.is_some() {
+            return true;
+        }
+        self.shelf_position.is_none()
+            && !(self.rating.is_missing()
+                && self.take.is_missing()
+                && self.dnf_reason.is_missing()
+                && self.progress.is_missing()
+                && self.finished_on.is_missing()
+                && self.edition_id.is_missing())
+    }
 }
 
 /// The trimmed text, recording `too_long` when it has more than `max` characters.
@@ -453,6 +472,7 @@ async fn patch_entry(
     let previous_progress = entry.fields.progress;
     let status = body.status.unwrap_or(entry.status);
     let position = body.position.unwrap_or(entry.position.into());
+    let places_item = body.places_item();
 
     if let Some(finished_on) = body.finished_on.as_option() {
         if status != ShelfStatus::Finished {
@@ -497,7 +517,7 @@ async fn patch_entry(
     if let Some(shelf_position) = body.shelf_position {
         place_on_shelf(conn, &mut entry, shelf_position).await?;
     }
-    if body.shelf_position.is_none() || body.status.is_some() || body.position.is_some() {
+    if places_item {
         place_item(conn, &mut entry, status, position).await?;
     }
     if let Some(finished_on) = body.finished_on.into_option() {
diff --git a/server/tests/shelf_entries.rs b/server/tests/shelf_entries.rs
index f0ff865..7b73eca 100644
--- a/server/tests/shelf_entries.rs
+++ b/server/tests/shelf_entries.rs
@@ -701,6 +701,64 @@ async fn patch_changes_the_edition_and_the_rating_visibility(db: PgPool) {
         .assert_status(400);
 }
 
+/// The client's abandoned books in `year` of the stats.
+async fn dnf_in(client: &TestClient, year: i32) -> i64 {
+    let res = client.get(&format!("/api/stats?year={year}")).await;
+    res.assert_status(200);
+    res.json()["dnf"].as_i64().unwrap()
+}
+
+/// Hiding or showing a rating is no change to the entry: a PATCH with only `rating_visibility`
+/// leaves `updated_at` and the column alone, so an abandoned book stays in the year of its
+/// last change in the stats. Any other field sent with it still moves `updated_at`.
+#[sqlx::test]
+async fn a_rating_visibility_change_alone_is_no_change(db: PgPool) {
+    let app = TestApp::new(db).await;
+    let ada = app.user("ada").await;
+    let dropped = add(
+        &ada,
+        &circe(&app).await,
+        json!({"status": "did_not_finish", "rating": 2, "dnf_reason": "Too slow"}),
+    )
+    .await;
+    add(
+        &ada,
+        &app.ol_work("OL2W", "Song", "").await,
+        json!({"status": "did_not_finish"}),
+    )
+    .await;
+    // Last changed in 2025; it sits below the newer book in its column.
+    sqlx::query("UPDATE shelf_entries SET updated_at = '2025-03-01T12:00:00Z' WHERE id = $1")
+        .bind(dropped["id"].as_i64().unwrap())
+        .execute(&app.db)
+        .await
+        .unwrap();
+    assert_eq!(dnf_in(&ada, 2025).await, 1);
+
+    for visibility in ["only_me", "audience"] {
+        let item = patch(
+            &ada,
+            &dropped["id"],
+            json!({"rating_visibility": visibility}),
+        )
+        .await;
+        assert_eq!(item["rating_visibility"], visibility);
+        assert_eq!(item["updated_at"], "2025-03-01T12:00:00Z");
+        assert_eq!(item["position"], 1);
+        assert_eq!(dnf_in(&ada, 2025).await, 1);
+    }
+
+    let edited = patch(
+        &ada,
+        &dropped["id"],
+        json!({"rating_visibility": "only_me", "dnf_reason": "Too slow, really"}),
+    )
+    .await;
+    assert_eq!(edited["rating_visibility"], "only_me");
+    assert_ne!(edited["updated_at"], "2025-03-01T12:00:00Z");
+    assert_eq!(dnf_in(&ada, 2025).await, 0);
+}
+
 /// The list is ordered by the status text (alphabetically), then position and id.
 #[sqlx::test]
 async fn the_list_is_ordered_by_status_text(db: PgPool) {
```

Item C and decision 8, the specs and the two notes:

```diff
diff --git a/docs/phase1/notes/posts.md b/docs/phase1/notes/posts.md
index 0015922..e401f8f 100644
--- a/docs/phase1/notes/posts.md
+++ b/docs/phase1/notes/posts.md
@@ -37,6 +37,7 @@ No migration. No change outside `server/src/posts/**`, `server/tests/posts_*.rs`
 - **Parent of a reply** (`write::reply_parent`): `visibility::post_access(…, allow_tombstone = true)` (a 404 becomes 400 `That entry is not on this book`), the parent's club row `FOR KEY SHARE`, the thread's work (`lock_work`), `post_access` again under those locks (a club parent also needs `club_role`), then `SELECT … FOR KEY SHARE` on the parent, then, in the legacy order: not on this thread (another work than the locked one; on the pick routes also another club or pick) → 400 `That entry is not on this book`; a reply → 400 `Reply to the entry itself, not to a reply`; a tombstone → 400 `That entry was deleted`. The reply copies the parent's `club_id` and `pick_id` (`posts_parent_fk`).
 - **Insert** (`write::insert_post`): snapshot of the author's own shelf position (`position`), then one `INSERT … SELECT … WHERE club_id IS NULL OR EXISTS (open pick)`; zero rows is 400 `That pick is closed` (a pick that ends while the request runs gets no new post). The milestone is checked under `FOR KEY SHARE` before the insert: a delete in flight is waited for and then answers `That milestone is not on this pick` (an unlocked check passed it and the insert's foreign key then failed with a 500), and a delete that starts later waits for the post and then detaches it (`ON DELETE SET NULL`). Every club post records `note_posted` `{post_id}` (club, actor, work, pick).
 - **PATCH:** `post_access(…, false)` (404) → author only (403 `You can only edit your own entries`, also for `{}`) → `Nothing to update` (body null, `spoiler_upto` absent, `visibility` null) → `only_me` on a reply or club post (400 `Replies and club posts cannot be private`) → `UPDATE … WHERE id AND author_id AND deleted_at IS NULL` with `updated_at = now()` always (spec R8); zero rows is 404. Answers `PostOut` with replies.
+  - **Changed by FUP:** `updated_at` moves only when `body` or `spoiler_upto` is sent; `visibility` alone leaves it (`notes/fup.md` item C).
 - **DELETE:** `post_access(…, false)` → the author, or `club_role >= Admin` **of that post's club** on a club post (moderation), else 403 `You can only delete your own entries` → `SELECT … FOR UPDATE` on the post (404 if it went meanwhile) → delete its reactions → a post with replies becomes a tombstone (`body ''`, `spoiler_upto NULL`, `deleted_at now()`; `progress_at`, `status_at`, `updated_at`, `pick_id`, `milestone_id` and `created_at` stay), any other is deleted. No event.
 - **Reactions:** body validation (the stripped emoji must be one of the six, 400 `Pick one of the club reactions`) → `write::access_in_lock_order` (`post_access(…, false)`, the club row, the post's work, `post_access` again) → `SELECT … FOR KEY SHARE` (404 if deleted meanwhile) → `DELETE … RETURNING`; if nothing was deleted, `INSERT … ON CONFLICT DO NOTHING RETURNING`. Only a stored insert on a club post records `reaction` `{emoji, post_id, author_id}` (club, actor, work, pick). Diary reactions record nothing (parity). Answers `PostOut` with replies.
 - **Lock order: club row, then work, then post.** A club delete, a removal and an account deletion lock the club rows `FOR UPDATE` before they reach posts (conventions §5.3, §5.6). So every club write here takes the club row `FOR KEY SHARE` first: a new pick post through `write::lock_club_member` (which then checks the membership again in a statement of its own: 404 `No such club` when the club was deleted or the caller removed while the request waited), a reply or a reaction on a club post through `write::reply_parent` / `write::access_in_lock_order` (`post_access`, the club lock, the work lock, `post_access` again). Only then is the post locked. Locking the post first and needing the club row later for the foreign keys of the insert and the event deadlocked with a club delete (`tests/posts_races.rs::a_reaction_on_a_club_post_waits_for_a_club_delete` and `::a_reply_to_a_club_post_waits_for_a_club_delete` fail on the old order). A member removed while a reply or reaction waited is refused by the second `post_access` (`::a_reply_to_a_club_post_waits_for_a_removal`: 400; `::a_reaction_on_a_club_post_waits_for_a_removal`: 404, nothing stored. For the reaction the answer's own `post_out`, read in the same transaction, would refuse it too and roll the toggle back).
@@ -46,7 +47,7 @@ No migration. No change outside `server/src/posts/**`, `server/tests/posts_*.rs`
 
 ## Deviations and decisions (beyond endpoint-map §8)
 
-1. **New 400 string `Replies and club posts cannot be private`.** `POST /api/books/works/{work_id}/posts` with `parent_id` and `"visibility": "only_me"` is a validation error; `PATCH /api/posts/{post_id}` with `"visibility": "only_me"` on a reply or a club post is refused after the 403/`Nothing to update` checks. `"audience"` is accepted there (a no-op that still bumps `edited`). The schema CHECK `posts_visibility_scope` would otherwise answer 500.
+1. **New 400 string `Replies and club posts cannot be private`.** `POST /api/books/works/{work_id}/posts` with `parent_id` and `"visibility": "only_me"` is a validation error; `PATCH /api/posts/{post_id}` with `"visibility": "only_me"` on a reply or a club post is refused after the 403/`Nothing to update` checks. `"audience"` is accepted there (a no-op that still bumps `edited`; **since FUP it no longer does**, `notes/fup.md` item C). The schema CHECK `posts_visibility_scope` would otherwise answer 500.
 2. **`note_posted` for every new club post**, replies included, whichever route created them (the pick routes, or the diary route replying to a club post). Endpoint-map's "No event (parity)" on the diary route holds for diary entries, which record nothing.
 3. **Club replies.** Replies are allowed on the pick routes (the parent must be a root of the same pick in the same club) and through the diary route (the parent's scope wins; membership of the parent's club is required through `post_access.club_role`). A new club post, reply or not, on a pick that has ended is 400 `That pick is closed`; edits, deletes and reactions on such posts stay allowed (parity: no closed check).
 4. **`created_label` zones: conflicts with conventions §12.** A club post's label is in its club's zone everywhere (pick thread, work thread, `/api/diary`, PATCH and reaction answers); a diary entry's label is in the viewer's zone. This follows work-plan appendix diary 11 ("the club's zone for club labels, the viewer's zone for personal ones") and club 4 ("Home-feed labels use the item's club zone for club items"), and keeps a club post's label the same wherever it appears. The conventions §12 "Which zone applies" table says instead that `created_label` on diary and home-feed items uses the viewer's zone, and keeps club zones for "club feed and thread labels". **For the integrator:** amend the §12 row to "`created_label` on diary and home-feed items: the viewer's zone for personal items, the item's club zone for club items", or tell POSTS to switch the work thread and `/api/diary` to the viewer's zone (one line in `out::post_outs` plus `tests/posts_picks.rs::post_times_use_the_club_timezone`). As built, a club post's label in the work thread and the feed can be in another zone than the response's `timezone` (the viewer's). `tests/posts_picks.rs::post_times_use_the_club_timezone` asserts the club zone in the pick thread, the work thread and the feed.
diff --git a/docs/phase1/notes/shelf.md b/docs/phase1/notes/shelf.md
index 5d5792f..d035a5d 100644
--- a/docs/phase1/notes/shelf.md
+++ b/docs/phase1/notes/shelf.md
@@ -36,6 +36,7 @@ Schema names: `ShelfItemOut`, `ShelfListOut`, `ShelfAddIn`, `ShelfPatchIn`, `Fav
 
 1. **POST check order:** 404 `No such book.` and 400 for a foreign edition come before the 409. The legacy app had no such checks.
 2. **`Nothing to update` counts the new fields:** a present `edition_id` (tri-state, so `null` counts too) and a non-null `rating_visibility` count as something to update. A PATCH with only `rating_visibility` behaves like a rating-only PATCH: the column is re-sequenced and `updated_at` moves. A status change keeps `rating_visibility`.
+   - **Changed by FUP:** a PATCH with only `rating_visibility` now leaves the column and `updated_at` alone, like a `shelf_position`-only one, so an abandoned book keeps its year in the stats (`notes/fup.md` decision 8).
 3. **`finished_on` range:** a date outside 0001-01-02..9999-12-30 (year 0 or negative years, which `time::Date` accepts) is 400 `Invalid value` (`tz::personal_date_in_range`). An invalid date string is 400 `Invalid value`.
 4. **Layout PUT with `?username=`:** validation (400) comes before the 404 for another user's name (conventions §4.2: body validation, then handler logic). `?username=` naming the caller in any case, or an empty value, saves the caller's layout. Any other name is 404 `No member with that name`, the same body as for an unknown name.
 5. **Layout validation messages.** The custom strings are verbatim. The generic ones follow pydantic's wording where one exists: `List should have at least 1 item after validation, not 0`, `List should have at most 12 items after validation, not 13`, `Dictionary should have at most N items after validation, not M`, `Input should be greater than 0`, `String should match pattern '^#[0-9a-fA-F]{6}$'`, `String should match pattern '^decor:[a-zA-Z0-9_-]{1,64}$'`, `Input should be greater than or equal to 26` / `less than or equal to 64`. Type errors (`true`, `1.5`, `"1"` as an entry, a list for `decorations`, an unknown enum value) are serde errors, so the answer is `Invalid value` or `Input should be a valid list`. All of them are 400 and none is mapped in the frontend.
@@ -134,6 +135,7 @@ Operation ids (tag `shelf`): `shelf_start_import`, `shelf_get_import`, `shelf_li
 **Quotes:**
 9. **Requests.** `POST` takes `{work_id, edition_id?, body, page?, visibility?}`; the legacy book metadata (`ol_work_key`, `title`, …) is gone. An unknown work is 404 `No such book.` (it replaces `title is required for a new book`), and a merged work resolves to its target. `edition_id` must be an edition of the work (400 `That edition belongs to another book`, a missing edition too). Without it, the quote takes the caller's shelf edition of the work at creation time, else null.
 10. **`PATCH`** takes `{body?, page?, visibility?}`. `body: null` and `visibility: null` count as absent, `page: null` clears the page. Check order (diary R6): validation (400) → not visible (404 `No quote with that id`) → someone else's (403) → `Nothing to update` (400). `DELETE` checks the same way. A quote the caller cannot see (a stranger's, or someone else's `only_me` one) is 404, never 403. `updated_at` is set on every PATCH; it is not in `QuoteOut`.
+    - **Changed by FUP:** only a PATCH with `body` or `page` sets it; `visibility` alone does not (`notes/fup.md` item C).
 11. **`GET /api/quotes`.** `work` is a numeric work id (a merged id finds its target; an unknown id gives `{"items": []}`; `OL1W` is 400 `Input should be a valid integer, unable to parse string as an integer`). An empty `work=` is no filter, like an absent one (legacy `if work:`, diary R24), so `?work=&username=ada` is `?username=ada`. A value of only spaces is not empty and is 400. `username` goes through `visibility::visible_user`: an unknown name and a stranger both give `{"items": []}`. That function also trims the name (Python only lowercased it). `limit` is 1..500 (default 100) with pydantic's messages. The order is `id DESC`.
 12. **`QuoteOut`** = `{id, book, edition_id, author, mine, body, page, visibility, created_at, created_label}`. `book` uses the quote's edition as context. `created_label` is `tz::label` in the viewer's zone.
 13. **`quote_added`** is personal (`club_id` NULL, `work_id` set), with payload `{page?, excerpt, quote_id}`. `excerpt` is the first 80 characters of the trimmed body. Deleting a quote keeps the event.
diff --git a/docs/phase1/spec/diary.md b/docs/phase1/spec/diary.md
index 6d72647..3c9a877 100644
--- a/docs/phase1/spec/diary.md
+++ b/docs/phase1/spec/diary.md
@@ -229,6 +229,7 @@ Anything else → 400 `"Pick one of the club reactions"`. A bare U+2764 without
   2. If `author_id != me` → 403 `"You can only edit your own entries"`. This runs before the empty-body check, so a non-author's `{}` gets 403.
   3. If `body` is null and `spoiler_upto` is absent → 400 `"Nothing to update"`. This includes `{"body": null}` (probed).
   4. Apply the fields. Set `updated_at = now` **unconditionally**, even when the values did not change. Commit.
+     - **Phase 1:** this holds for `body` and `spoiler_upto`. The new `visibility` (§5.1) alone does not move `updated_at`, so switching an entry to "Only me" and back does not mark it edited (`docs/phase1/notes/fup.md` item C).
 - **Response 200** `DiaryEntryOut`. For a top-level entry it includes the full `replies` list (`created_at, id` ascending); for a reply, `replies: []`.
 - **Not changed:** progress_at, status_at, pick_id. There is no activity event.
 
@@ -383,7 +384,7 @@ The response is **204**. The `quote_added` event stays in `events`.
 - **`best_pick`:** among picks with a non-null `average_rating`, the maximum by `(average_rating, len(ratings))`. On a full tie the first in `id DESC` order wins, i.e. the newest pick.
 - **`year_of(entry)`:** the UTC year of `finished_at`, else of `updated_at` (NOT NULL, so the `now()` fallback is dead code).
   - DNF entries always have `finished_at = NULL`, so they count by `updated_at`.
-  - `updated_at` is bumped by almost every shelf PATCH, through `place_item`; only a pure `shelf_position` change skips it.
+  - `updated_at` is bumped by almost every shelf PATCH, through `place_item`; only a pure `shelf_position` change skips it (Phase 1: and a pure `rating_visibility` change, `docs/phase1/notes/fup.md` decision 8).
   - So editing an old DNF entry's reason moves it into the current year.
 - **Rounding:** Python's `round(x, 2)` rounds half to even on the binary value (probed: 33/8 → `4.12`, 37/8 → `4.62`, 35/8 → `4.38`). JSON prints `5.0` for an integral float.
 - **Side effects:** none.
@@ -1000,6 +1001,7 @@ A non-author sending `{}` gets **403**, not 400.
 - In serde use `Option<Option<i32>>` with `#[serde(default, deserialize_with = …)]` (or `serde_with::rust::double_option`). Plain `Option` loses the distinction, and test 7 (`{"spoiler_upto": null}` clears) and the quotes test (`{"page": null}` clears) fail.
 
 **R8. PATCH entry bumps `updated_at` unconditionally**, so `edited` becomes true even when nothing changed. Reactions never touch `updated_at`.
+- **Phase 1:** R8 holds for every PATCH that sends `body` or `spoiler_upto`. A PATCH with only `visibility` (new in Phase 1) leaves `updated_at` as it is (`docs/phase1/notes/fup.md` item C).
 
 **R9. Response shape differs by endpoint.**
 - PATCH and reaction on a **top-level** entry return its full `replies`; on a reply, `[]`.
diff --git a/docs/phase1/spec/shelf.md b/docs/phase1/spec/shelf.md
index 28875df..3e4714d 100644
--- a/docs/phase1/spec/shelf.md
+++ b/docs/phase1/spec/shelf.md
@@ -368,6 +368,7 @@ Updates one of the caller's shelf entries.
 7. If `shelf_position` is not null: `place_on_shelf(entry, shelf_position)` (§2.5).
 8. If `shelf_position` is null, **or** status is not null, **or** position is not null: `place_item(entry, status, position, stamp_dates = status != previous_status)`.
    - This means **every PATCH except a `shelf_position`-only one** (`shelf_position` possibly combined with rating/take/dnf_reason/progress/finished_on, but without status or position) re-sequences the column and **bumps `updated_at`**. A rating-only PATCH bumps `updated_at` **(probed)**. A `shelf_position`-only PATCH leaves `updated_at` and all reading metadata untouched **(test)**.
+   - **Phase 1:** a PATCH with only the new `rating_visibility` skips `place_item` too, so it leaves the column and `updated_at` as they are (`docs/phase1/notes/fup.md` decision 8).
    - Dates are stamped only when the status actually changed. A same-status PATCH never re-stamps, so a cleared `finished_at` stays null **(test)**.
 9. If `finished_on` is present:
    - `finished_at` = `datetime.combine(finished_on, 12:00, tz=zone)` converted to UTC, or null. Noon local time in the instance zone; for example Berlin 2025-08-01 gives `2025-08-01T10:00:00Z`, and Kiritimati (UTC+14) 2025-07-01 gives `2025-06-30T22:00:00Z`.
@@ -1062,7 +1063,7 @@ Matched books are copied into the catalog. A Goodreads ISBN could set `edition_i
    - `rating`, `take`, `dnf_reason`, `progress` and `finished_on` need tri-state deserialisation (`Option<Option<T>>` with `#[serde(default, deserialize_with=…)]`): absent means keep, null means clear.
    - `status`, `position` and `shelf_position` treat null as absent.
    - The "Nothing to update" check uses presence, not value, for the five tri-state fields, and runs **before** the 404 ownership check.
-2. **When `place_item` runs.** It runs on every PATCH unless `shelf_position` is set and both `status` and `position` are null or absent. It bumps `updated_at` and renumbers the column. A `shelf_position`-only PATCH must leave `updated_at` byte-identical (the test deep-compares items).
+2. **When `place_item` runs.** It runs on every PATCH unless `shelf_position` is set and both `status` and `position` are null or absent (Phase 1: or the PATCH sets nothing but `rating_visibility`). It bumps `updated_at` and renumbers the column. A `shelf_position`-only PATCH must leave `updated_at` byte-identical (the test deep-compares items).
 3. **Date stamping.**
    - PATCH stamps only on an actual status change. POST and import always stamp with `changed=False`.
    - `finished_on` is applied **after** stamping and then clamps `started_at`.
```

## For PAR (#49)

- One new "Known intentional changes" row (old zone names). No `/// parity:` line changed. The new tests port no Python test: the Python app had neither per-user zones nor post visibility.

## Tests

- Server: 11 new tests (4 unit tests in `tz`, 7 integration tests: `catalog_requests.rs`, `auth_register.rs`, `auth_account.rs`, `clubs_manage.rs`, `importer_run.rs`, `posts_diary.rs`, `shelf_quotes.rs`), and one extended (`posts_diary.rs::edits_mark_the_entry_and_change_its_visibility`). The decision-8 test in `shelf_entries.rs` is in the patch above, not in the tree.
- Frontend: one test rewritten (`pages/AdminPage.test.ts`) and one added (`timezone.test.ts`, decision 9).
