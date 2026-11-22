# TEXTS (#55, Q11 option C) implementation notes

Owner: TEXTS, on `launch/texts`. Q11 (`docs/phase2/work-plan.md` §4) option C: B plus posts, replies, quotes and rating takes, with text snapshots shown to the instance admin. Rafael confirmed C before the public launch. B (ADM2, `docs/phase2/notes/adm2.md`) gave the queue, the decision mails and the export rules this builds on.

Status: **built.** Server, SPA and docs; every check in `CLAUDE.md` passes except the ones that need Docker (no Docker on the host; see "Checks").

## Migration

**`server/migrations/4000_reports_texts.sql`** (block 4000–4049):

- `reports.target_kind` also `post`, `quote`, `take` (`reports_target_kind_check` keeps its name).
- New target columns, each `ON DELETE SET NULL` with a partial index: `post_id → posts`, `quote_id → quotes`, `shelf_entry_id → shelf_entries`, each only on its kind (`reports_post_targets`, `reports_quote_targets`, `reports_take_targets`).
- `target_user_id` now also names the author of a text: `reports_target_user_avatars` is replaced by `reports_target_user_kinds` (`avatar`, `post`, `quote`, `take`).
- `text_snapshot text NULL`, at most 1000 characters (the longest text kind: posts and quotes; a take has 140), only on text kinds (`reports_text_snapshot_texts`).
- `work_id` keeps the book (a merge re-points it, as for every report).
- No new foreign key to `users`: `auth_export.rs` still counts 29, `jobs.rs` needs no row. `parity_schema.rs` and `legal_docs.rs` (no new table) are unaffected.

## Contract changes

### New routes (`server/src/text_reports.rs`, merged in `lib.rs`)

| Route | Operation, tag | Answers |
|---|---|---|
| `POST /api/posts/{post_id}/reports` | `posts_report`, `posts` | 201 `ReportCreated`; 400 reasons / `A text cannot be reported as the wrong book` / `You cannot report your own text`; 401; 404 `No entry with that id`; 429 |
| `POST /api/quotes/{quote_id}/reports` | `shelf_report_quote`, `shelf` | the same, 404 `No quote with that id` |
| `POST /api/books/works/{work_id}/takes/{username}/reports` | `shelf_report_take`, `shelf` | the same, 404 `No such book.` (unknown work) or `Not found` (no reader, no entry, no take, hidden take) |

Body `ReportIn` as for every report.

**Order of checks:** auth → body (the reason list 400) → `report_request_user` (30 a minute, every request) → `wrong_book` 400 → (take: the work, then the reader's entry) → the plain read: 404 when hidden or missing, `removed` when an admin removed it (below), 400 for one's own → a repeat answers from the plain read → the reader's turn → the author's row → the locked read again → a repeat → `report_user` (20 a day, shared) → insert.

**Visibility.** The reporter must see the text through the policy, else 404 like an unknown id: a post through `bc_can_see_post` (club posts to members; diary entries and replies to the author's audience unless `only_me`), a quote through `bc_can_see_item` with its `visibility`, a take through `bc_can_see_item` with its entry's `rating_visibility` (what `bc_visible_shelf` applies). A tombstone and an empty take are 404.

### Changed routes and DTOs

- **`ReportOut`** gains `text_snapshot: string|null`, `current_text: string|null` (null once removed or deleted) and `club: ClubRef|null` (a club post's club, from the post as it is now). `target_user` is the author for text kinds, `book` the book the text is on. `gone` for a text kind: its text is missing (deleted, a tombstone, an emptied take) and the report was not `removed`.
- **`POST /api/admin/reports/{report_id}/resolve`** decides the text kinds (`text_reports::decide`): new 409 `The reported text is no longer online`.
- **`ReportIn`, `ReportResolveIn`** doc comments name the new kinds; shapes unchanged.
- **Export:** `reports.filed` drops `text_snapshot`; `reports.about_me` includes text reports through `target_user_id` and adds `text_snapshot` to a text kind's entry (no `post_id`/`quote_id`: a removed quote is deleted, a removed post deleted or a tombstone, so they would be null or meaningless; `work_id` names the book).
- **Account deletion** (`auth::deletion::delete_user`) clears `text_snapshot` of every report whose `target_user_id` is the account, before the user row goes; the reports stay. The `DELETE /api/auth/me` doc comment says so (`generated.ts`).

### Detail strings (all mapped in `apiErrors.ts`, keys in both catalogs)

| String | Constant | Key |
|---|---|---|
| `You cannot report your own text` (400) | `text_reports::OWN_TEXT` | `errors.ownText` |
| `A text cannot be reported as the wrong book` (400) | `text_reports::TEXT_WRONG_BOOK` | `errors.textWrongBook` |
| `The reported text is no longer online` (409, admin) | `text_reports::TEXT_GONE` | `errors.textGone` |

### Mail templates

`Reported` gains `Post(Text)`, `Quote(Text)`, `Take(Text)` with `Text { book: Option<&str>, excerpt: Option<&str> }`. The reporter's `report_decision` names the kind and the book ("the post on "Circe" you reported", „den Beitrag zu „Circe“, den du gemeldet hast,“; German pronouns er/es), says "Whoever wrote it is told…" on a removal, and "because it was deleted" when gone; it never quotes the text. The author's `statement_of_reasons` says what changes per kind and quotes the removed text (the first 200 characters, `text_reports::EXCERPT_CHARS`): "The removed text read: "…"" / „Der entfernte Text lautete: „…““. Both through `mail()`; `every_template_ends_with_the_legal_footer` covers the new variants (the `REPORTED` list grew to 9).

## Decisions

- **One kind for posts.** Club posts, replies and diary entries are `post`; `club` in `ReportOut` and the admin page tell a club post apart.
- **Take addressing: `/api/books/works/{work_id}/takes/{username}/reports`.** A take shows in `ShelfItemOut` (with the entry id), in `ClubPickReader` (username only) and in the `shelf_finished` payload of the activity feeds (actor and work). The pair (work, username) is the only address every place has; an entry id would have meant new fields on two DTOs. `{work_id}` follows merges like the book page, the name is normalised. The report stores `shelf_entry_id`. The route has a `username` parameter, so `parity_visibility.rs` lists it (as `Not`, moderation: it serves no personal data, only 201 or 404 under the Shelf rule) and the checklist counts 17 such routes.
- **404 strings:** each kind's existing "no such id" (`No entry with that id`, `No quote with that id`); a take has none, so `Not found` (`msg::NOT_FOUND`).
- **Snapshot bound:** 1000 characters by a CHECK; the snapshot is the text read under the lock when the report is filed. A repeat (same reader, open report) keeps the first snapshot, even after an edit.
- **Edited since:** the queue shows the snapshot ("As reported") and, when different, the current text ("Now, after an edit"); "remove" withdraws the current text.
- **Gone:** a post deleted or tombstoned by its author, a deleted quote, a take cleared (or its entry removed, or the account deleted) since the report. "remove" 409, "dismiss" closes it: with every open report of the same text while the row exists (a tombstone, an entry with an empty take), alone once the row is gone (`post_id`/`quote_id`/`shelf_entry_id` are NULL then), like a deleted account's avatar. Reporters get the "no longer online … because it was deleted" outcome, nobody a statement.
- **Closed on arrival:** a text an admin removed can still be read only as a tombstoned post (it had replies) or an empty take; a report of it (a page loaded before the decision) is recorded `removed`, closed on arrival, with no snapshot, and the reporter's toast says it was already removed. A removed quote, or a removed post without replies, is deleted: a later report is 404 like an unknown id. A take written again after the removal is reportable again.
- **"Remove" per kind:**
  - post: `posts::edit::withdraw`, extracted from `delete_post` so both share it: the post locked, its reactions deleted, a tombstone when it has replies, else deleted. Its `note_posted` and `reaction` events reference the post by id, as after an author's delete.
  - quote: deleted; its `quote_added` events lose the stored `excerpt` (others' feeds already hide a quote that is gone; the author's own feed showed the stored excerpt).
  - take: `shelf_entries.take = ''`, rating, visibility and `updated_at` untouched; the author's `shelf_finished` events of that work lose a `take` equal to the current take or to any snapshot of a report of that entry. Older takes of the entry that nobody reported stay in older events.
- **Every open report of the text is resolved before the text goes**, inside `decide`: a deleted post or quote clears the reports' foreign keys, and the generic "every open report of the same target" update in `resolve_report` would then find only the pressed one.
- **The statement quotes the removed text** to its author (DSA Art. 17 asks the statement to identify the content; an author may have several posts on one book). The reporter's mail and the export of the reporter never carry the text.
- **Export of the author:** the snapshot of the decision's oldest report (the row `about_me` already picks for the reason), the author's own words.

## Lock order (conventions §7.2, new bullet)

Report: reader's turn (`REPORT_LOCK_SPACE`, `<user>:post:<id>`, `<user>:quote:<id>`, `<user>:take:<entry id>`) → author's `users` row `FOR KEY SHARE` (`media::reports::lock_owner`, now `pub(crate)`) → the text's row `FOR SHARE` → insert. Decision: author's row `FOR KEY SHARE` → the text's row `FOR UPDATE` → the `reports` rows of the text → the withdrawal (and the author's `events` for a quote or take). The author's row first matches account deletion, work merges (users before posts, quotes, entries and events) and shelf writes (users `FOR UPDATE` before the entry); taken only through the report's foreign key after the text, a report deadlocked with the author's account deletion (`a_report_racing_the_authors_account_deletion_does_not_deadlock` gets a 500 with that lock removed; checked by hand and reverted).

Accepted race, as for the other reports: a report's `work_id` written during a merge that does not lock the author's row may keep the merged-away work; the queue then shows that work.

## Retention

No new period. Resolved reports go 3 years after the decision (`auth::cleanup`, unchanged: it deletes the rows, snapshots included). Open ones stay until decided. The author's account deletion clears the snapshots earlier.

## SPA

- `components/ReportSheet.vue`: `ReportTarget` gains `{kind: "post", postId, author}`, `{kind: "quote", quoteId, author}`, `{kind: "take", workId, author}` and `isTextTarget`; titles `report.title.{post,quote,take}`, the line under the title `report.by.*`; reasons without `wrong_book` (as for every non-cover kind). `api.reportPost`, `api.reportQuote`, `api.reportTake` in `api/client.ts`.
- Posts: `PostActionsSheet.vue` gets `canReport` and a "Report" row (`FlagIcon`); `DiaryEntry.vue` shows "…" on every post of someone else (not a tombstone, not a deleted account's) and opens the sheet. That covers the book page's diary, the club discussions and the diary feed.
- Quotes: `QuoteList.vue`, a quiet "Report" under someone else's quote (book page, a reader's quotes).
- Takes: `pages/club/ClubHomePage.vue` ("Report" beside another member's take under the pick), `pages/MemberPhysicalShelfPage.vue` ("Report this take" in the book's sheet; the sheet closes for the report), `components/ActivityFeed.vue` ("Report this take" under another reader's finish with a take, home and club feeds). The book page shows no other reader's take (`BookReader` has none), and the list view of a member's shelf shows no takes.
- `components/admin/AdminReports.vue`: badges Post / Quote / Take, "Post by {name}" with "in {club}", the book, "As reported" with the snapshot (or "The text is gone: its author deleted the account."), "Now, after an edit" when the current text differs; "Remove the text" with a blurb per kind; a gone text offers only "Close the report"; after a text decision the queue is read again.
- Catalogs: `report.action.{text,take}`, `report.title.{post,quote,take}`, `report.by.{post,quote,take}`, `report.takeOf`, `errors.{ownText,textWrongBook,textGone}`, `admin.target.{post,quote,take}`, `admin.removeBlurb.{post,quote,take}`, `admin.gone.text`, `admin.textBy.{post,quote,take}`, `admin.inClub`, `admin.asReported`, `admin.textNow`, `admin.snapshotGone` (en and de, added only).

## Tests

Server, `server/tests/text_reports.rs` (10):

- `a_post_report_shows_the_snapshot_and_the_current_text`: filing, a repeat (same id), the queue item's fields, an edit since (snapshot vs current), a club post's `club`, the summary count.
- `text_reports_follow_the_visibility_policy`: a stranger 404 for a diary entry, a club post, a quote and a take; a club mate 404 for an `only_me` entry and quote; an only-me take 404; unknown ids, names, readers without a take, an empty take and a tombstone 404; a non-member 404 for a club post while the same user can report a diary entry of a club mate; unknown work `No such book.`; own text 400 per kind (the name normalised); `wrong_book` 400 (before the lookup); a bad reason 400.
- `remove_withdraws_each_kind_of_text`: a club post with a reply becomes a tombstone (the reply stays), both reporters' reports resolved, mails to both reporters (no author, no text) and a statement to the author quoting the text (no reporter, no details); a post without replies deleted; a quote deleted, gone from `GET /api/quotes` for both; a take cleared, the rating kept, gone from the member's shelf, the club pick's readers and both feeds; the history.
- `dismiss_keeps_the_text`: the text stays, only the reporter is mailed, a second decision 409, a new report after a dismissal is new.
- `a_text_its_author_deleted_is_gone`: a deleted post and a cleared take are `gone`, `remove` 409 `The reported text is no longer online` with no mail, `dismiss` mails "no longer online … deleted", nothing to the author.
- `a_removed_text_closes_later_reports_on_arrival`: a tombstoned post and a cleared take answer `removed`, repeats name it, nothing enters the queue, no mail, no snapshot; a new take is reportable again.
- `the_export_shows_the_author_the_removal_and_the_reporter_no_text`: `about_me` with `text_snapshot`, without reporter fields; `filed` without `text_snapshot` and `target_user_id`, and the reporter's export nowhere holds the text.
- `deleting_the_authors_account_clears_the_snapshots`: after the deletion the reports stay, only another author's snapshot is left, the queue shows them gone without author.
- `text_reports_share_the_limits_and_take_turns`: two reports at once by one reader are one report; 20 a day, the 21st 429, a repeat still answers.
- `a_report_racing_the_authors_account_deletion_does_not_deadlock` (lock order).

Also changed: `email::templates` unit tests (the `REPORTED` list, `texts_are_named_to_the_reporter_and_quoted_to_their_author`); `parity_visibility.rs` (one `USERNAME_ROUTES` row); `clubs_non_member.rs` (its walk over `/api/posts/{post_id}…` sends a `reason`, so the new route answers 404 like the others instead of the body's 400).

SPA: `components/TextReports.test.ts` (new, 5: the sheet per kind and its routes, German, closed on arrival; `isTextTarget`; the post's "…" sheet offers "Report" for someone else's post only, not for one's own or a tombstone; `QuoteList` only on someone else's quote; the feed only on another reader's take); `pages/AdminPage.test.ts` (+1: text kinds, snapshot vs current, club, gone, remove, German); `pages/club/ClubHomePage.test.ts` (+1); `DiaryEntry.test.ts`, `BookDiary.test.ts`, `QuoteList.test.ts`: someone else's post or quote now has a "Report" action (their old "no actions" assertions changed).

## Checks

All of `CLAUDE.md`'s checks except the Docker ones (no Docker on this host): smoke test, shellcheck, actionlint, the compose and Caddy validation were not run; TEXTS changed nothing under `deploy/` or `.forgejo/`.

## For the integrator

- **Files outside this package's area:** `server/src/posts/edit.rs` (`withdraw` extracted from `delete_post`, behaviour unchanged), `server/src/auth/deletion.rs`, `server/src/auth/export.rs`, `server/src/media/reports.rs`, `server/src/email/templates.rs`, `server/src/lib.rs`, `server/tests/parity_visibility.rs`, `server/tests/clubs_non_member.rs`, the SPA components above.
- **`docs/legal/vvt.md`** (not touched: no legal text beyond the factual docs): the reports activity can name posts, quotes and takes, `text_snapshot`, and its deletion with the author's account.
- **`docs/phase3/work-plan.md` §9:** the #55 line is the friend-request package's line too; merge both edits.
- **README / CLAUDE.md:** the report sheet now also covers posts, quotes and takes.

## Open points

- A take's older versions in older `shelf_finished` events (a re-read finished again with another take) stay when the reported take is removed, unless they equal a reported snapshot.
- A post deleted by its author while several reports are open leaves each report to be closed on its own (no key ties them any more).
