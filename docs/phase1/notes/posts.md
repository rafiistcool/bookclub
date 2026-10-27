# POSTS (#44b, pick-post part of #45) implementation notes

Owner: POSTS. These notes record what the code does where the design left room. They also list the deviations, the new detail strings, the Python tests that are reasoned rather than ported, and what FE, PAR and the other packages need to know. The design documents themselves are unchanged.

## Modules (`server/src/posts/`)

| File | Contents |
|---|---|
| `mod.rs` | `routes()`, the detail strings, the reaction set, the body rule (`checked_body`) and the `spoiler_upto` range |
| `out.rs` | `PostOut`, `position` (legacy `_position`), `positions`/`my_position` (the viewer's own shelf), `post_outs`/`post_out` (rows through `bc_visible_posts`, `author_rating` through `bc_visible_shelf`, reaction names through `bc_visible_owners`), the reaction aggregation |
| `write.rs` | `NewPost`, `insert_post` (position snapshot, closed-pick guard in the INSERT, `note_posted`), `reply_parent` (parent checks through `visibility::post_access`, then `FOR KEY SHARE`) |
| `diary.rs` | `GET/POST /api/books/works/{work_id}/posts`, `GET /api/diary`; `DiaryOut`, `DiaryEntryIn`, `FeedQuery`, `DiaryFeedItemOut`, `DiaryFeedOut` |
| `picks.rs` | `GET/POST /api/clubs/{slug}/pick/posts`, `GET/POST /api/clubs/{slug}/picks/{pick_id}/posts`; `PickThreadOut`, `PickPostIn` |
| `edit.rs` | `PATCH/DELETE /api/posts/{post_id}`, `POST /api/posts/{post_id}/reactions`; `PostPatchIn`, `ReactionIn` |

Every operation has tag `posts`. Operation ids: `posts_work_thread`, `posts_write_entry`, `posts_feed`, `posts_update`, `posts_delete`, `posts_react`, `posts_current_pick_thread`, `posts_write_current_pick`, `posts_pick_thread`, `posts_write_pick`.

Schema names (other packages must not reuse them): `PostOut`, `DiaryOut`, `DiaryEntryIn`, `DiaryFeedItemOut`, `DiaryFeedOut`, `PickThreadOut`, `PickPostIn`, `PostPatchIn`, `ReactionIn`. `PostOut.replies` is recursive (`#[schema(no_recursion)]`; the document has `items: {$ref: PostOut}`).

No migration. No change outside `server/src/posts/**`, `server/tests/posts_*.rs`, `server/.sqlx/` and this file.

## How reads work

- **One loader.** Every endpoint first selects post ids (`JOIN bc_visible_posts($viewer)`), then `post_outs` loads the rows again through `bc_visible_posts`, so a caller can never hand it an id the viewer may not see. Threads keep tombstones as anchors; the feed adds `deleted_at IS NULL`.
- **Work thread** (`GET /api/books/works/{work_id}/posts`): roots `WHERE work_id = $w AND parent_id IS NULL`, then their visible replies, both ordered `created_at, id`. That is the viewer's visible diary entries plus the club posts of the viewer's clubs (each with `club`).
- **Pick threads:** the club comes from `ClubMember`; the pick is `WHERE club_id = $club AND (ended_at IS NULL | id = $pick)`; roots `WHERE club_id = $club AND pick_id = $pick AND parent_id IS NULL`, with replies. `can_post` = the pick has not ended. `timezone` = the club's zone.
- **Feed** (`GET /api/diary`): keyset on `id DESC` with `limit + 1`. `?club=<slug>` goes through `clubs::extract::member_by_slug` (404 `No such club`) and keeps that club's posts plus diary posts (roots and replies) whose author is a **current** member of the club. An empty `club=` is no filter. Each item carries `book` (`BookOut` without an edition context), the viewer's `my_progress`/`my_status` on that work, `club` (the same `ClubRef` as `entry.club`) and `parent_author`.
- **`author_rating`:** `bc_visible_shelf(viewer)` row of the author for the post's work with status `finished`, so it is live, null unless finished, and null for someone else's `only_me` rating (also null for a tombstone of a deleted account).
- **Reactions:** `count` counts every row, `mine` is exact, `users` lists the names in `bc_visible_owners(viewer)` (the viewer and everyone sharing a club with them, which includes the members of a club post's club), sorted by byte order. Groups are sorted by count descending, then emoji by code point (spec R12). Tombstones show `[]`.
- **`parent_author`:** the parent's author's username when that user is in `bc_visible_owners(viewer)`, else null. A diary reply the viewer can see always has a visible parent author, except under the tombstone of a deleted account (null). For a club reply whose parent's author left the club and shares no other club with the viewer, it is null too, although the club's own thread still names that author (club posts are club data).
- **`created_label`:** a club post's label is in **its club's** zone wherever it appears (pick thread, work thread, feed, PATCH and reaction answers); a diary entry's label is in the **viewer's** zone. `DiaryOut.timezone`/`DiaryFeedOut.timezone` are the viewer's zone, `PickThreadOut.timezone` the club's.

## How writes work

- **Order of checks, diary POST:** auth → body validation (400, all messages in field order) → `{work_id}` (`WorkRef::parse` + `resolve_local`: numeric id, `OL…W`, `BC…`/`ISBN…`/`GB…` legacy keys; a merged work resolves to its target; anything else 404 `No such book.`) → for a reply: the parent (below) → insert.
- **Order of checks, pick POST:** auth and membership (404 `No such club`) → path (400 non-numeric pick id) → body (400) → the pick (404 `No club pick yet` / `No club pick with that id`, another club's pick included) → `That pick is closed` → `That milestone is not on this pick` (`milestones WHERE club_id AND pick_id AND id`) → the parent → insert.
- **Parent of a reply** (`write::reply_parent`): `visibility::post_access(…, allow_tombstone = true)` (a 404 becomes 400 `That entry is not on this book`; a club parent also needs `club_role`), then `SELECT … FOR KEY SHARE` on the parent, then, in the legacy order: not on this thread (another work; on the pick routes also another club or pick) → 400 `That entry is not on this book`; a reply → 400 `Reply to the entry itself, not to a reply`; a tombstone → 400 `That entry was deleted`. The reply copies the parent's `club_id` and `pick_id` (`posts_parent_fk`).
- **Insert** (`write::insert_post`): snapshot of the author's own shelf position (`position`), then one `INSERT … SELECT … WHERE club_id IS NULL OR EXISTS (open pick)`; zero rows is 400 `That pick is closed` (a pick that ends while the request runs gets no new post). The milestone goes in through a sub-select on `milestones`, so one deleted meanwhile becomes NULL instead of a foreign-key 500. Every club post records `note_posted` `{post_id}` (club, actor, work, pick).
- **PATCH:** `post_access(…, false)` (404) → author only (403 `You can only edit your own entries`, also for `{}`) → `Nothing to update` (body null, `spoiler_upto` absent, `visibility` null) → `only_me` on a reply or club post (400 `Replies and club posts cannot be private`) → `UPDATE … WHERE id AND author_id AND deleted_at IS NULL` with `updated_at = now()` always (spec R8); zero rows is 404. Answers `PostOut` with replies.
- **DELETE:** `post_access(…, false)` → the author, or `club_role >= Admin` **of that post's club** on a club post (moderation), else 403 `You can only delete your own entries` → `SELECT … FOR UPDATE` on the post (404 if it went meanwhile) → delete its reactions → a post with replies becomes a tombstone (`body ''`, `spoiler_upto NULL`, `deleted_at now()`; `progress_at`, `status_at`, `updated_at`, `pick_id`, `milestone_id` and `created_at` stay), any other is deleted. No event.
- **Reactions:** body validation (the stripped emoji must be one of the six, 400 `Pick one of the club reactions`) → `post_access(…, false)` → `SELECT … FOR KEY SHARE` (404 if deleted meanwhile) → `DELETE … RETURNING`; if nothing was deleted, `INSERT … ON CONFLICT DO NOTHING RETURNING`. Only a stored insert on a club post records `reaction` `{emoji, post_id, author_id}` (club, actor, work, pick). Diary reactions record nothing (parity). Answers `PostOut` with replies.
- **Lock order:** a reply and a reaction hold the post `FOR KEY SHARE`, a delete takes it `FOR UPDATE`. A delete therefore waits for a reply in flight and leaves a tombstone, and a reply that waited for a delete gets `That entry was deleted` or `That entry is not on this book`, never a 500 (`tests/posts_races.rs`, deterministic with a held lock; removing the reply's lock makes `a_reply_waits_for_a_delete_in_flight` fail). Account deletion (W1) locks the user's posts `FOR UPDATE` after the user row, so the same guarantees hold against it.

## Deviations and decisions (beyond endpoint-map §8)

1. **New 400 string `Replies and club posts cannot be private`.** `POST /api/books/works/{work_id}/posts` with `parent_id` and `"visibility": "only_me"` is a validation error; `PATCH /api/posts/{post_id}` with `"visibility": "only_me"` on a reply or a club post is refused after the 403/`Nothing to update` checks. `"audience"` is accepted there (a no-op that still bumps `edited`). The schema CHECK `posts_visibility_scope` would otherwise answer 500.
2. **`note_posted` for every new club post**, replies included, whichever route created them (the pick routes, or the diary route replying to a club post). Endpoint-map's "No event (parity)" on the diary route holds for diary entries, which record nothing.
3. **Club replies.** Replies are allowed on the pick routes (the parent must be a root of the same pick in the same club) and through the diary route (the parent's scope wins; membership of the parent's club is required through `post_access.club_role`). A new club post, reply or not, on a pick that has ended is 400 `That pick is closed`; edits, deletes and reactions on such posts stay allowed (parity: no closed check).
4. **`created_label` zones** as described above (club zone for club posts everywhere, viewer zone for diary entries).
5. **`parent_author`** follows `bc_visible_owners`, as described above.
6. **`DiaryOut`** is `{work_id, my_progress, my_status, items, timezone}`: the legacy `ol_work_key` and `book_id` are gone (an unknown work is a 404 now, so `book_id` would never be null).
7. **The feed's `?club=`** has no `joined_at` cut-off (unlike the activity feed): a new member's older diary entries show in the club's feed. A member's diary replies count as diary entries.
8. **`spoiler_upto`** is parsed as a 64-bit integer, so `101`, `-1` and huge values all get pydantic's range message instead of `Invalid value`; the OpenAPI type is `int64`.
9. **Moderation** reads the role from `post_access.club_role` inside the request's transaction; it is not re-checked under a club lock (a post delete is not a membership write, conventions §5.3).
10. **Path ids:** `{post_id}` and `{pick_id}` are `ApiPath<i64>` (non-numeric 400, overflow 404 `Not found`); `{work_id}` is a string resolved as above.

## Detail strings

New (FE may map it; the UI never sends `only_me` for a reply, so it is optional): `Replies and club posts cannot be private`.

Kept verbatim from the legacy app, as `const`s in `posts/mod.rs` (none is mapped in `apiErrors.ts` today): `Write a short take, quote, or meeting note`, `Keep it to 1000 characters`, `That entry is not on this book`, `Reply to the entry itself, not to a reply`, `That entry was deleted`, `You can only edit your own entries`, `You can only delete your own entries`, `Nothing to update`, `Pick one of the club reactions`, `No club pick yet`, `No club pick with that id`, `That pick is closed`, `That milestone is not on this pick`. From `error::msg`: `NO_SUCH_BOOK` (`No such book.`), `NO_ENTRY` (`No entry with that id`), `NO_SUCH_CLUB`.

Gone with the unified `/api/posts/{post_id}` routes (endpoint-map §8): `No note with that id`, `You can only edit your own notes`, `You can only delete your own notes`, `Could not save that note`; with the client book metadata: `Nobody has this book yet; send its title to start the diary`, `A title is required`.

## Python tests

Ported, with a `/// parity:` line: all of `test_pick_posts.py` (5, `tests/posts_picks.rs`), the six listed `test_notes_and_meeting.py` tests (`tests/posts_picks.rs`) and 13 of the 15 `test_diary.py` tests (`tests/posts_diary.rs`, and `test_entry_on_the_current_pick_records_the_pick` in `tests/posts_picks.rs`). These carry a reasoned change, explained on the test:

- `test_diary.py::test_diary_on_an_unknown_book_is_empty_not_missing` -> a work the catalog has but nobody wrote about is an empty diary; an id the catalog does not know (`OL99W`, an unknown number or legacy key) is 404 `No such book.` (work-plan appendix, diary 13).
- `test_diary.py::test_first_entry_creates_the_book_when_given_its_details` -> no client book metadata: an entry needs a catalog work (404 otherwise) and a `book` key is ignored; the rest (stripped body, author, snapshot fields, label, shelf untouched) is unchanged.
- `test_diary.py::test_entry_on_the_current_pick_records_the_pick` -> "a club post records its pick": the pick routes store `club_id` and `pick_id`; a diary entry on the pick's work stores neither.
- `test_diary.py::test_custom_book_has_a_diary_without_open_library` -> the custom book is a user-added work, reached by id and by its legacy `BC…` key; no request goes upstream.
- `test_diary.py::test_viewer_position_drives_the_spoiler_shield_fields` and the other two-user tests -> the second user joins the first user's club (the legacy "everyone is in the club").
- `test_pick_posts.py::test_post_time_uses_club_timezone` -> `BOOKCLUB_TZ` is the club's zone, set with `PATCH /api/clubs/{slug}`.
- `test_pick_posts.py::*` and `test_notes_and_meeting.py::*` -> picks and milestones are inserted with SQL (their routes are CLUB's); the legacy `/api/pick/posts/{post_id}` edit, delete and reaction routes are `/api/posts/{post_id}…`.

Not ported here (one line each for parity-notes.md):

- `test_diary.py::test_legacy_pick_notes_are_imported_once_with_reactions` -> moved to IMP: the boot-time copy of `club_pick_posts` is the importer's job now (work-plan IMP parity list).
- `test_diary.py::test_club_rating_averages_finished_readers` -> moved to CAT: it tests `GET /api/books/works/{work_id}` (`BookDetailOut.club_rating`/`rating_count`, audience-scoped), which CAT owns; no POSTS route is involved.

## Tests

- `tests/posts_diary.rs` (16): the `test_diary.py` ports, plus visibility edits, labels in the viewer's zone, merged works and the `?club=` feed.
- `tests/posts_picks.rs` (17): the `test_pick_posts.py` and `test_notes_and_meeting.py` ports, "a club post records its pick", `note_posted` for posts and replies (both routes), milestones, picks of other clubs, moderation, posts on ended picks.
- `tests/posts_visibility.rs` (9): diary roots by visibility across the `Viewers`, replies by people outside the viewer's audience, reaction names, `parent_author`, club posts across clubs, reacting to an invisible post (404 without the body), club B's admin and owner unable to delete club A's post, `author_rating` with `only_me`, the deleted account's tombstone and its replies.
- `tests/posts_races.rs` (6): a delete waiting for a reply in flight (tombstone), a reply and a reaction waiting for a delete in flight (400/404), parallel reactions, a double toggle, replies racing a delete.
- Unit tests: `posts::tests` (2), `posts::out::tests` (3).

The W1 walkers now cover the POSTS routes too: `auth_required.rs` (401), `clubs_non_member.rs` (404 `No such club` on the pick routes; 404 `No entry with that id` on `/api/posts/{post_id}…` for club A's post and reply; 400 on a diary reply to it; `?club=` 404; the positive controls in the work thread and `/api/diary`).

## For FE (#48)

- `PostOut` replaces `DiaryEntryOut` and `PickPostOut`; edits, deletes and reactions of pick posts go to `/api/posts/{post_id}…`. `author` is null only on the tombstone of a deleted account; `author_avatar_url` is the `/media/…/a` URL.
- `DiaryOut` has `work_id` instead of `ol_work_key`/`book_id`.
- `reactions[].users` may hold fewer names than `count` (people outside the viewer's audience are counted, not named).
- `parent_author` may be null on a reply (deleted or not visible author).
- A club post's `created_label` is in its club's zone even in the work thread and the feed.
- `visibility` is only meaningful on diary roots; the composer should offer it only there.
- Club threads allow replies now (`parent_id` on `POST /api/clubs/{slug}/pick/posts`).

## For other packages

- **CLUB:** `note_posted` is `{post_id}` and `reaction` is `{emoji, post_id, author_id}` with `club_id`, `work_id` and `pick_id` set; the feed resolves `author_id`. A milestone's `note_count` is `count(*) FROM posts WHERE milestone_id = $id` (club posts; replies may carry a milestone too). Deleting a milestone needs no detach step: `posts_milestone_fk` sets `milestone_id` to NULL. Ending or changing a pick touches no post; its thread stays readable by id.
- **IMP:** posts written by the importer are read the same way (`bc_visible_posts`); a club post needs `club_id` and `pick_id` set together, and a reply copies its parent's scope.
