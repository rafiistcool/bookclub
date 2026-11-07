# The personal cover choice

Added on 2026-11-07 (branch `design/cover-choice`). A reader chooses the cover a book on their shelf shows. The integrator moves the contract changes below into `conventions.md`, `endpoint-map.md`, `CLAUDE.md` and the READMEs (done for the server side in this branch), and keeps this note as the record.

## The decisions (Rafael, 2026-11-07)

- **Personal.** The choice belongs to the reader's shelf entry. It shows on their shelf, also to the friends and club mates who see that shelf, on Home and on the book page for the reader. Every other reader keeps the standard cover. Instance admins keep "set for everyone" (`PUT /api/admin/works/{work_id}/cover`).
- **Sources.** Only those the catalog already uses, never Google Books (`docs/plan.md`, "Catalog"):
  - the covers of the work and its editions in the catalog;
  - Open Library edition covers, also of editions that are not in the catalog;
  - with `DNB_PORTAL_COVERS` (private instance only), the publisher's cover from the DNB portal;
  - a photo of the reader's own copy, through the cover scanner.
- **Option A** of the design brief: a nullable `shelf_entries.cover_media_id`. "Choose cover" exists only for a book on the shelf.

## Schema (migration 3902, the integrator's block)

- `shelf_entries.cover_media_id bigint NULL REFERENCES media ON DELETE SET NULL`, with a partial index. NULL means the standard cover (the edition's, else the work's).
- `media.personal boolean NOT NULL DEFAULT false`: a photo of the reader's copy uploaded as their own cover. A CHECK keeps it to uploaded covers. `ingest::set_cover` refuses such a row, so it never becomes a catalog cover, not even through an admin's write.
- `media.unused_since timestamptz NULL`: personal photos only; the sweeper's mark (below).
- Migration 3903 (after the review): `editions.portal_checked_at timestamptz NULL`, when the candidate list last asked the DNB portal for the edition and it had no usable cover (below).

## Endpoints

- **`GET /api/books/works/{work_id}/cover-candidates` → `CoverCandidatesOut {items, partial}`** (`catalog::covers::choice`, operation `catalog_cover_candidates`). `CoverCandidateOut {key, source, cover_url, cover_thumb_url, cover_public_id, edition_label, chosen}`, `source` one of `catalog`, `openlibrary`, `dnb_portal`, `photo`; `CoverEditionLabel {title, publisher, year, language, isbn13}`, each nullable. The order of the list:
  1. The reader's current choice, when it is still usable. A personal photo is listed only here.
  2. With `DNB_PORTAL_COVERS`, and only for a book on the caller's shelf: up to 3 of the work's German-group editions without a cover ask the portal (`CatalogLookup::publisher_cover`). Not for a book a reader added, or one a takedown concerns (`cover_search.takedown_at` on the work or on a work merged into it: `search::under_takedown`, as in the cover search). Not again for an edition the portal had no usable cover for in the last `PORTAL_RECHECK_DAYS` (7) days (`editions.portal_checked_at`, written on a 404, a non-image answer or a picture `store_found_cover` refuses; no answer records nothing and leaves the list `partial`). A picture is stored (`MediaService::store_found_cover`); then, in one transaction, the media row is registered, the work is locked `FOR UPDATE`, the edition must still be the work's and `under_takedown` is read again, and the picture is set as the edition's cover with provenance `dnb`, only where there is none (`set_cover(…, only_if_empty = true)`). Otherwise the transaction rolls back and the stored files go (`discard_cover`). It then lists as the catalog's cover with `source: "dnb_portal"`. `set_cover` of an edition also fills an empty work cover, as everywhere.
  3. The work's cover, then its editions' covers by edition id, each picture once. A work cover that is also an edition's gets the edition's label.
  4. Open Library. For a work with an Open Library id: `works/{id}/editions.json?limit=50` (the new `CatalogService::ol_work_editions`, labels from the edition record) and the work record's `covers` (`ol_work`). For a work without one: `CatalogLookup::cover_by_isbn` for up to 3 of its ISBNs (`/isbn/{isbn}.json`). Cover ids already listed are skipped; a listed picture without a label takes the Open Library edition's.
  - `chosen` marks the picture the reader's entry shows now: their choice, else the standard cover (the entry's edition's, else the work's) when the list has it. No entry: none is chosen.
  - At most 24 candidates, plus the current choice. The Open Library ones get their lazy media rows through `ingest::ol_cover_medias`, which is public now. A frozen interface changed: only its visibility, not its behaviour. The bytes are fetched on the first `/media/{id}/{l|s}` request; a picture Open Library has not answers 404, and the client hides the candidate.
  - Open Library's answers go through the 18-hour response cache and the `ol_api` throttle. The account's catalog turn (`upstream_turn`: 60 a minute, two at a time) is taken only when an answer is not cached (`ol_work_editions_cached`, `ol_work_cached`), or for the ISBN and portal calls.
  - A source without an answer, or a turn that was refused, gives `partial: true`, and the request still answers 200 with what it has. 404 `No such book.`; an unknown `OL…W` is not imported here.
  - No prefetch: the chooser loads only the thumbnails it shows, lazily, so 24 candidates do not mean 48 fetches.
- **`PUT /api/shelf/{entry_id}/cover` `{key}` → `ShelfItemOut`** (`shelf::cover::choose_cover`). The keys:
  - `ol:<cover id>`: an Open Library cover whose media row exists.
  - `media:<public id>`: one of the entry's work's or editions' covers, any Open Library row, or the caller's own personal photo.
  - The picture must be usable: a cover, not taken down, not gone (`ol_gone_at`), not with the bytes of a picture a takedown judged.
  - Otherwise 400 `This cover cannot be chosen` (`shelf::cover::COVER_NOT_AVAILABLE`, mapped to `errors.coverNotChoosable`). Another user's entry is 404 `Book not on your shelf`. `updated_at` stays.
  - Lock order: the media row `FOR SHARE`, with the check in the same statement, then the entry. No users row lock: no positions change.
- **`DELETE /api/shelf/{entry_id}/cover` → `ShelfItemOut`** (`shelf::cover::reset_cover`): back to the standard cover.
- **`PUT /api/shelf/{entry_id}/cover/photo`**, multipart `file` → `ShelfItemOut` (`media::cover::upload_personal_cover`, in the media routes with the upload body limit).
  - The reader upload's steps: one upload at a time, 30 a day, the pipeline, the removed-hash check. `media::cover::receive`'s tail became `process_and_store`, which both use.
  - The row is registered with `personal = true` and becomes the entry's cover in one transaction. No `cover_replace` request.
- **`ShelfItemOut.personal_cover`** and **`BookDetailOut.personal_cover`** (both `bool`). The book cover fields already carry the choice.

## Where the choice shows

`BookRef` and `book_outs` stay frozen; the choice belongs to an entry. `shelf::cover::personal_covers(conn, entry_ids)` loads the usable choices, and `apply_personal_cover` puts one into a `BookOut` (URLs, public id, spine colour). The callers:

- `shelf::entries::item_outs`: the shelf, for the owner and for everyone who may see it through `bc_visible_shelf`. Home's shelf rows come from here too.
- `friends::out::friends_out` and `clubs::out::member_outs`: the currently-reading previews. They now select the entry id from `bc_visible_shelf`.
- `shelf::stats`: the top and longest book of the year in review (`YearTotals.top_entry`, `longest_entry`).
- `catalog::detail::book_detail`: the viewer's own entry. It also fixes a gap: the book page now shows the cover of the edition on the viewer's entry (it showed the work's), while the title and pages stay the work's.
- `media::spine`: the colour backfill samples the chosen picture.
- `shelf::favorites::favorite_outs` (after the review): a favourite on the owner's shelf shows as that entry, the choice, else the entry's edition's cover, else the work's, in the shelf's favourites row (`ShelfListOut.favorites`) and `/api/auth/me/favorites`. Its callers check that the viewer may see the owner, and so their shelf. A favourite that is not on the shelf shows the work's cover.

Not personal: search hits, club picks (Home's hero), the activity feed, report lists.

## Lifecycle

- **Takedown.** `ingest::clear_references`, under `clear_media` and `clear_gone_media`, also clears `shelf_entries.cover_media_id`, after works and editions, not counted. The entry shows its standard cover again.
- **Reports.** A shelf visitor can report a chosen cover like any cover (`POST /api/media/{public_id}/reports`). `media::reports::shown_on` falls back to the work of an entry that chose the picture, so the admin sees the book. The report never names the entry or the reader. A personal photo's takedown reaches its uploader's statement of reasons through `media.uploaded_by`, as for any upload.
- **Merge.** `merge_in` locks the works' covers and the personal covers of the two works' entries `FOR SHARE` before the works, in one statement in id order (`lock_pictures`). `merge_shelves` gives the kept entry its own choice, else the dropped entry's, when that one was locked (not taken down).
  - A choice made after that lock (the choice takes no users lock) is seen under the entries' `FOR UPDATE`: the merge starts over and locks it then (`merge_shelves` returns false before the last try). Before the review, a choice on the dropped entry could get lost.
  - A takedown locks the reported picture before its copies (`cover_copies`), not in id order, so a merge holding a copy (a work's cover) and waiting for the reported picture (an entry's choice) deadlocked with it (40P01, a 500 for one of the two admins). The merge now waits for the pictures' locks only `MEDIA_LOCK_TIMEOUT` (200 ms `lock_timeout`, below Postgres' 1 s `deadlock_timeout`), then rolls its savepoint back and starts over after a pause of 50 ms times the try; a deadlock is retried too. `RETRIES` is 4 (it was 2, for the loser's cover only); the last try waits as long as it takes.
- **Removal and Undo.** Removing a book deletes the entry and its choice. The client's Undo adds the book again and sends the same key. A personal photo stays findable for 7 days.
- **The sweeper** (`auth::sweep_personal_covers`, in the hourly cleanup job after `delete_rejected_cover_photos`):
  - Each run marks the personal photos no entry chooses with `unused_since = now()` and unmarks those chosen again.
  - It deletes those unused for `PERSONAL_COVER_GRACE_DAYS` (7), with their files: lock `FOR UPDATE`, then a second look.
  - A photo an open report names stays for the admin; deleting it later clears a decided report's `media_id`.
  - A taken-down photo keeps its row and hashes.
  - The grace counts from the first run that saw the photo unused, so it is 7 days plus up to an hour.
- **Account deletion** (`auth::deletion::delete_user`) deletes the user's personal photos that are not taken down and that no open report names, and their files after the commit. Shelf entries cascade.
- **Rejected cover requests.** `delete_rejected_cover_photos` keeps a photo a shelf entry chooses. This is the `MEDIA_REFERENCES` rule, `server/tests/jobs.rs`.
- **Export.** `shelf_entries` rows drop the internal `cover_media_id` and carry `cover_public_id`. `uploaded_media` entries carry `personal`.
- **Public launch.** The step that removes every `dnb_portal` cover (`publisher-covers.md`) deletes the rows, and `ON DELETE SET NULL` clears the choices that showed one.

## Rules this triggered

- **A new foreign key to `media`.** `MEDIA_REFERENCES` in `server/tests/jobs.rs` has `shelf_entries.cover_media_id`, and `delete_rejected_cover_photos` checks it, with a case in `rejected_cover_photos_go_with_their_request`.
- **No new foreign key to `users`, and no new column of `users`.** So no `empty_sign_ups`, `DATA`, `TRACES`, `USERS_COLUMNS`, `parity_visibility` or `auth_export` `SOURCES` change. The export picks the new column up through `to_jsonb`, shaped as above.
- **A new detail string,** mapped in `frontend/src/i18n/apiErrors.ts` (`api_error_copy`).
- **OpenAPI.** The new routes and DTOs are in the document, and `endpoint-map.md` §0 lists the three candidate DTOs (`openapi_contract`). `frontend/src/api/generated.ts` must be regenerated (`npm run gen:api`); until then `openapi_contract::generated_types_require_response_fields_only` fails on `CoverCandidatesOut`.
- **Legal facts: open until the public launch.** CLAUDE.md allows no legal work before the launch, and this change adds no table that references `users`, so `docs/legal/` is unchanged. At the launch:
  - `privacy-facts.md`: the cover choice in the Reading data row (`shelf_entries.cover_media_id`); in the Pictures row, a reader's own photo (`media.personal`), seen by the reader and their audience; the cover choice as an Open Library trigger (`ol_work_editions`, up to 3 ISBN covers, 18 h cache); a retention row for personal photos (7 days after the last choice, and with the account).
  - `vvt.md`: the shelf record, and the contributions record's `media` scope.
  - `processors.md`: the work id or ISBNs Open Library receives when a reader opens the choice.
  - The DNB portal stays out of the legal documents, as in `publisher-covers.md`: private instance only.

## Tests

- **`server/tests/shelf_cover_choice.rs`** (wiremock for `editions.json` and the work record, `FakeCatalog` for the portal and the ISBN cover):
  - the list's order, labels, deduplication and cache;
  - a choice on the shelf as the owner, the club mate and the friend see it, in the friends' and the club's previews and on the owner's book page, while another reader's book page and the strangers (404) see nothing of it;
  - the book page's edition cover and the stats' top book;
  - key validation and other users' entries;
  - a photo stays personal: no request, `set_cover` refuses it, another reader cannot choose it, and a report names the book;
  - a takedown brings back the standard cover;
  - Undo with a personal photo;
  - the portal with and without the setting, and the ISBN cover;
  - an Open Library outage gives `partial`;
  - merge, export and account deletion.
  - after the review: the favourites row shows the choice (and the entry's edition cover); the portal only for a book on the shelf, once per edition without a publisher cover for 7 days, again after no answer (`the_portal_is_not_asked_again_for_an_edition_without_a_publisher_cover`); no publisher cover on a book whose cover a takedown cleared during the portal request (`FakeCatalog::during_publisher_cover`) or on a work merged into it (`the_portal_leaves_a_book_a_takedown_concerns`); a choice made while a merge locks survives it (`a_merge_keeps_a_choice_made_while_it_locks`); a merge gives way to a takedown that holds the chosen picture and wants the work's cover (`a_merge_gives_way_to_a_takedown_that_holds_a_picture`, a 40P01 before).
- **`server/tests/jobs.rs`:** `MEDIA_REFERENCES`, the chosen photo of a rejected request, and `personal_cover_photos_go_seven_days_after_their_last_choice`.
- **`server/tests/catalog_detail.rs`:** the `BookDetailOut` field list.
- **Unit:** `shelf::cover::tests::keys_parse_in_their_two_forms_only`.

## Risks left

- **Open Library load.** A first opening of the chooser for a popular work costs two API requests (cached afterwards) and up to 24 thumbnail fetches as the reader scrolls, through the `ol_covers` throttle.
- **Storage.** Lazy Open Library rows of candidates nobody chose stay, as the search's do, but their bytes exist only once someone loaded the thumbnail (`s`) or the large picture. A sweeper that drops the stored bytes of unreferenced Open Library rows after 30 days is possible later.
- **Choosing any Open Library row.** `ol:` and `media:` keys of an Open Library row are accepted for any book. The picture is public catalog content anyway, and the choice is the reader's own.
- **Takedown "wrong book".** A takedown judged `wrong_book` takes down only the copies the same book shows as a catalog cover (`cover_copies`); a personal choice of a copy is not among them.

## Frontend after the review

- `ShelfPage.vue` `replaceItem` copies a changed cover (`book.cover_*`, `book.spine_color`) onto the matching favourite, after a choice and after its Undo. The server sends the choice in `ShelfListOut.favorites`.
- While `personal_cover` is true the "…" sheet hides "Suggest a photo" and "Report cover" (both would act on the reader's own choice); a `has_catalog_cover` field would let them stay.
