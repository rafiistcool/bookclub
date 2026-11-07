# Reading and trust (stage 7a)

Added on 2026-11-07 (branch `design/reading-trust`), from the second critique of the redesign (`.impeccable/critique/2026-11-07T02-17-34Z__frontend-src.md`, P2 items 2 to 4 and the small observations about votes, meetings and milestones). The integrator moves the contract changes below into `conventions.md`, `endpoint-map.md`, `CLAUDE.md` and the READMEs, and keeps this note as the record.

## Progress in pages

The server stored only a percent, and the progress sheet derived pages from it: one page more often changed nothing (207 → 208 of 334 stays 62 %, "Save" stayed grey) and a typed page 218 came back as 217.

### Schema (migration 3904, the integrator's block)

- `shelf_entries.current_page integer NULL CHECK (current_page >= 0)`, and `shelf_entries_page_reading CHECK (current_page IS NULL OR status = 'currently_reading')`, like `progress`.
- `bc_visible_shelf` is unchanged (its OUT columns are frozen, `3100_friends.sql`). The readers of the page join `shelf_entries` on the id the function returned, as the cover choice does with `cover_media_id`; the visibility rule stays the function's. The page is shelf state like `progress`: whoever sees the progress sees the page.
- The export carries it on its own (`to_jsonb` of the row). Account deletion cascades with the entry. No new reference to `users` or `media`.

### `PATCH /api/shelf/{entry_id}`

- New field `current_page` (`Patch<i32>`, 0 to 100000, the catalog's largest page count; 400 otherwise). Alone it is something to update (`Nothing to update` counts it) and it places the item like `progress` (`updated_at` moves).
- The page count is the entry's edition's, else the work's (`shelf::entries::page_count`, the same count `ShelfItemOut.book.pages` shows).
- With a page count: the page is clamped to it, and `progress` becomes `shelf::ordering::page_percent(page, pages)`: the nearest whole percent, but 100 only on the last page and at least 1 from page 1 on. A `progress` sent with the page loses.
- Without a page count: the page is dropped (stored NULL) and `progress` is what was sent, or stays.
- `progress` sent without `current_page` clears the page: the percent is the reader's position now.
- `current_page: null` clears the page and keeps the percent.
- Another `edition_id` clears a stored page (a page of one edition is no page of another) and keeps the percent, unless `current_page` comes with it.
- Leaving `currently_reading` clears the page with the percent (`apply_progress`). A page sent with another status is dropped, like `progress`.
- Events: a `progress` event `{progress}` is recorded when the percent changed, from a page or a percent. One page more that keeps the percent records nothing (the feeds already say 62 %). The payload is unchanged.
- `POST /api/shelf`: `ShelfAddIn.current_page` (`Option<i32>`, 0 to 100000) works as in the PATCH: with a page count it is clamped and sets `progress`, which it wins over; without one, or for any status but `currently_reading`, it is dropped. The Undo of a removal (the shelf, the book page) sends it, so the reader's page comes back, not a page rounded from the percent.
- Catalog merges (`catalog::merge`, `paged_entries` and `drop_stale_pages`): an entry of the two works whose page counts against another page count after the merge (another work, an edition taken over from the dropped entry, the winner's count after `merge_fields`) drops the page and keeps the percent, as a PATCH to another edition does. A page whose count stays (its edition moved along) is kept. A percent copied from the dropped entry comes without a page (`merge_shelf_fields` is unchanged).

### Where the page is returned

- `ShelfItemOut.current_page` (the shelf, Home's rows and hero, every PATCH answer).
- `BookDetailOut.current_page` and `BookDetailOut.progress_pages`: the page count the viewer's progress counts in, the edition on their entry's, else the work's. `pages` stays the work's, as the book page shows it. Null when the work is not on the viewer's shelf or has no page count.
- `ClubPickReader.current_page` and `ClubPickReader.progress_pages` (the club home's and Home's own reader row; every reader's, like `progress`).
- Not in `BookReader`, `FriendReadingOut`, the activity feed or the posts: they keep the percent.

### The frontend

- `frontend/src/progress.ts`: `ReadingPosition {progress, page}`, `positionOf`, `positionPatch` (the page when there is one, else the percent), `positionSaved` (the toast, "Fortschritt: S. 208 · 62 %"), `pagePercent` (the server's rounding), `pageOfPercent`.
- `ProgressEditor` takes `page` (the stored `current_page`) beside `value` and `pages`, emits `save` with a `ReadingPosition`, and compares pages when it counts in pages: any page moved is a change. An entry saved in percent before shows the page its percent stands for, as before.
- `BottomSheet` has a new prop `dirty`: while it is true a drag gives a little and springs back, as after typing. `ProgressEditor` passes "the draft differs from what is stored", so a swipe after "+10" no longer discards the draft; back at the stored value a swipe closes again. × stays the way out.

## Spoilers protected by the app

Owner's decision: collapse automatically. Nothing on the server changed: `PostOut.progress_at` (the writer's percent when writing, 100 when finished) and `my_progress`/`my_status` in `DiaryOut`, `PickThreadOut` and `DiaryFeedItemOut` already carry what the rule needs.

- `frontend/src/spoilers.ts` (`spoilerGuard`, `isGuarded`, `readerPercent`), with its colocated test:
  - the reader's percent: finished 100, reading their progress, any other status or no entry 0;
  - always open: one's own posts, deleted ones, and everything for a reader who finished (or gave up, as before);
  - the writer's flag (`spoiler_upto`) wins both ways: below it the post is `marked`; past it the post is open, even when its writer was further on;
  - without a flag, a post written further on than the reader is (`progress_at > reader`) is `ahead`.
- `DiaryEntry`: `ahead` folds the whole post into one quiet line, "jonas · bei 35 % · du bist bei 0 % · Antippen zum Lesen" (the writer's place is `positionMarker`, so a finished writer reads "Gelesen ★★★★☆"). `marked` keeps the blur, with the label "Spoiler bis 70 % · du bist bei 35 %" in the text colour instead of the red "Sicher bis 70 %". A tap shows the post for this view and moves the focus to it. Replies under a folded post are judged on their own.
- `DiaryFeed` (Home, the club's discussion page, `/diary`): the guard's line stands in for the excerpt ("Bei 35 % · du bist bei 0 %"), as a button that shows the excerpt in place. While an `ahead` line shows, the foot row leaves out the position marker the line already starts with.
- The guards follow the reader's position: the server's `my_progress`/`my_status` arrive with each load, so every write that moves the position or the status reloads the diary on that page. Home reloads "Zuletzt geschrieben" when an entry it adopts (hero, reading rows, their Undo, a finish) changed its status or percent, not for stars alone; the book page after a progress save, a finish, a status segment, a removal and its Undo. `DiaryFeed.reload()` asked for while a page loads reads once more after it.
- The open reaction picker takes a line of its own under the action row (`.reaction-picker-line`, CSS `order`, after "+" in the DOM), and its buttons wrap in a column narrower than 256 px.
- The flag's words: `diary.safeUpto` is "Spoiler bis {n} %" / "Spoiler up to {n}%", `diary.wholeBook` "Spoiler: ganzes Buch"; the header no longer lower-cases the label (German nouns).

## Quieter posts

- One line under each post: the reactions on the left, "Antworten" and a "…" icon on the right, with or without reactions.
- "…" opens `PostActionsSheet` (a `BottomSheet` with light rows like the book page's): "Bearbeiten" and "Löschen" for one's own post, "Als Admin löschen" for a club admin on someone else's club post. Moderation lives only there. Deleting asks once more in the row ("Wirklich löschen?"). Focus: the sheet returns it to "…"; "Bearbeiten" moves it into the text field; a deleted post hands it to its list, a tombstone keeps it.
- "Melden" is not there: posts have no report target on the server (`ReportSheet` reports covers, profile pictures, books and editions). Left open for the reports owner.

## Small items

- Vote: `vote.waitingFor` is plural-aware, "es fehlt noch: tim" / "es fehlen noch: max, tim".
- `frontend/src/i18n/dates.ts`: `formatEventTime` for the club's meeting (club home, Home's hero title), milestone dates and the vote's end: no year in the current year, no zone unless the club's clock differs from the viewer's (this device's zone) at that moment. `formatDay` takes an optional `now` that leaves out the current year (the past picks' meeting days). `formatClubDate` (exact times in titles, the admin pages) is unchanged.
- Milestones: "Entfernen" shows only after "Bearbeiten" in the section head (then "Fertig"), regular weight and muted, and asks "Wirklich entfernen?" before it removes; the button is described by the stop's title. The toggle has no `aria-pressed`: its label ("Bearbeiten"/"Fertig") says the state. After a removal focus moves to the next stop's "Entfernen" (else the one before), and with the last stop gone to "Hinzufügen".

## Files outside the stage's list

- `frontend/src/components/BottomSheet.vue` (the `dirty` prop) and its test: the drag protection the task asked for lives there.
- `server/tests/catalog_detail.rs`: the key list of `BookDetailOut`.
