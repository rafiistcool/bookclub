# Port spec: club (pick, milestones, pick posts and reactions, meeting and ICS, next-up vote, overlap, activity, timezone)

Sources read in full:
- `backend/app/routers/{pick,vote,overlap,activity,milestones}.py`
- `backend/app/{pick_ops,activity,ics,timezone,serialize,shelf_ops,works,push,i18n,branding,config,deps,db}.py`
- `ClubPick`, `PickMilestone`, `ClubPickPost`, `PostReaction`, `Event`, `NextUpVote`, `NextUpNomination`, `NextUpBallot`, `ShelfEntry`, `Book` and `BookPost` in `models.py`, and the matching schemas in `schemas.py`
- the activity call sites in `routers/{shelf,quotes,auth,diary,books,stats}.py`
- tests: `backend/tests/{conftest,test_pick,test_pick_posts,test_vote,test_vote_deadline,test_overlap,test_activity,test_notes_and_meeting,test_migrations}.py`, plus the milestone test in `test_stats_milestones_quotes.py`
- frontend: `frontend/src/api/client.ts`, `types.ts`, `i18n/apiErrors.ts`, `shelf.ts`
- Forgejo issues #40 and #45

Behaviour marked **(probed)** was confirmed by running the Python app (FastAPI 0.162, Starlette 1.10, pydantic 2.14, SQLAlchemy 2.1, Python 3.12) against a scratch SQLite database. The probe scripts are in `scratchpad/probe/club/`.

Which endpoints the current SPA still calls (`api/client.ts`):
- **Used:** `GET/PUT/DELETE /api/pick`, `GET /api/pick/history`, `GET /api/overlap`, `GET /api/vote`, `POST /api/vote/nominations`, `POST /api/vote/cast`, `POST /api/vote/apply`.
- **No UI since the editorial redesign (PR #3), but implemented and tested:**
  - pick posts, edits, deletes and reactions
  - `meeting.ics`
  - milestones
  - the vote deadline (`PATCH /api/vote`) and the turnout fields
  - `/api/vote/suggestions`
  - `/api/activity`

  #48 plans club pages for "pick, votes, events, discussions, overlap, members" and a home feed, so these come back into use.

---

## 1. Endpoints

### 1.0 Conventions for every endpoint in this area

**Auth.** Every endpoint needs a signed-in session (`deps.get_current_user`). Without one, it returns **401** `"Not signed in"`. The full cookie semantics are in `spec/auth.md`. There is no role or ownership check anywhere in this area: every member can do everything, except edit and delete other people's pick posts. Check order (probed):
1. JSON parse error (400 `"JSON decode error"`)
2. auth (401)
3. body, query and path validation (400)
4. handler logic

So a signed-out request with an invalid body gets 401.

**Error body.** Errors are `{"detail": "<English string>"}`.
- Validation errors are **400**, never 422.
- Messages come in field-declaration order. A leading `"Value error, "` is stripped and the messages are joined with `"; "`.
- Generic pydantic messages seen in this area (probed):
  - `"Field required"`
  - `"Input should be a valid string"`: a number or `null` for a non-optional string such as `note`
  - `"Input should be a valid integer, unable to parse string as an integer"`
  - `"Input should be greater than or equal to 0"`
  - `"Input should be greater than or equal to 1"`
  - `"Input should be less than or equal to 100"`
  - `"Input should be a valid boolean, unable to interpret input"`
- Pydantic is in **lax mode**: `"1"` and `1.0` are accepted where an int is expected (probed: `{"nomination_id": "1"}` passes validation).
- Unknown JSON keys are ignored.

**No string in this area is mapped in `frontend/src/i18n/apiErrors.ts`** except `"Not signed in"`. The SPA shows the raw English `detail` for every other error here, including in the German UI.

**Timestamps in responses (probed).** The serialized format depends on where the value comes from:
- **Naive ISO 8601, no offset**, e.g. `"2026-10-27T10:59:36.097337"`: everything read back from SQLite. Microseconds are printed only when non-zero (`"2026-08-22T19:00:00"`). This covers `started_at`, `ended_at`, `meeting_at`, post `created_at`, and activity `created_at`.
- **With `Z`**, e.g. `"2030-01-01T18:00:00Z"`: `VoteOut.closes_at` and `MilestoneOut.due_at`. The code re-attaches UTC to these before serializing.

Every stored value is UTC. The SPA appends `Z` to any string without an offset (`shelf.ts: clubDate`), so emitting RFC 3339 with `Z` everywhere is safe.

**`timezone` fields.** Several responses carry `"timezone": "<IANA name>"`. That is `resolved_timezone(BOOKCLUB_TZ)` (2.1), the single club timezone of the instance. `GET /api/config` also returns it (owned by `spec/auth.md`).

**BookOut** (embedded as `book` everywhere in this area; `serialize.book_out`):
```json
{"id": int, "ol_work_key": str, "title": str, "authors": str, "cover_id": int|null,
 "year": int|null, "cover_url": str|null, "pages": int|null, "spine_color": str|null}
```
- `cover_url` comes from `serialize.cover_url`, in this order:
  1. a stored Google `cover_image_url`
  2. else `https://covers.openlibrary.org/b/id/{cover_id}-L.jpg?default=false` when `cover_id > 0`
  3. else, only when no Google key is configured and the key is `ISBN…`, `https://covers.openlibrary.org/b/isbn/{isbn}-L.jpg?default=false`
  4. else null
- `spine_color` is `book.spine_color or None`, so `""` becomes null.

The catalog spec owns BookOut; see 5.4 for what changes.

**Deleted-user fallbacks.** Python never deletes users, but the code falls back to the literal `"unknown"` for:
- a missing pick setter (`set_by`)
- a missing post author
- a missing reaction user
- a missing nominator (`nominated_by`)
- a missing event actor (`actor`)

Ballots whose user is missing are skipped (2.8).

---

### 1.1 `GET /api/pick`: the current club pick

- **Query/body:** none.
- **Logic:**
  1. `current_pick` is the `club_picks` row with `ended_at IS NULL` and the highest `id` (2.3).
  2. If there is none, return `{"pick": null, "timezone": tz}`.
  3. Otherwise, **side effect first:** `push.maybe_schedule_meeting_reminder`. If `meeting_at` is set, `reminder_sent_at` is NULL, the book exists and `0 < meeting_at − now ≤ 24 h`:
     - set `reminder_sent_at = now` and **commit**
     - queue a Web Push of kind `meeting` to every user with `notify_meeting=true`, **including the caller**, with tag `meeting-{pick_id}`, url `/`, and copy from `i18n.PUSH_COPY["meeting"]`, e.g. en `"{club}: meeting tomorrow"` / `"{title} — see you at the meeting."`
     - this read therefore writes
  4. Return `{"pick": ClubPickOut, "timezone": tz}`.
- **Does not** auto-close an expired vote (only the vote endpoints do; see 2.8).
- **Response 200 `ClubPickCurrentOut`:** `{"pick": ClubPickOut|null, "timezone": str}`.
- **ClubPickOut** (field order as listed; computation in 2.5):
```json
{"id": int, "book": BookOut, "set_by": str, "note": str,
 "started_at": datetime, "ended_at": datetime|null, "meeting_at": datetime|null,
 "meeting_local": str|null, "meeting_label": str|null,
 "on_shelf": "want_to_read"|"currently_reading"|"finished"|"did_not_finish"|null,
 "shelf_id": int|null,
 "readers": [ClubPickReader], "reading": [str], "finished": [str]}
```
- **ClubPickReader:** `{"username": str, "status": ShelfStatus, "rating": int|null, "take": str, "dnf_reason": str, "progress": int|null, "avatar_url": str|null}`.
- **Errors:** 500 `"Club pick is missing its book"` (unreachable, because of the FK).

### 1.2 `GET /api/pick/history`: ended picks

- **Logic:**
  - Select all picks with `ended_at IS NOT NULL`, ordered by `ended_at DESC, id DESC`.
  - Each one is serialized with `pick_out` for the caller. Because they are ended, `readers`, `reading` and `finished` are `[]`, but **`on_shelf` and `shelf_id` are still the caller's entry** (probed).
  - There is no pagination and no limit.
- **Response 200 `ClubPickHistoryOut`:** `{"items": [ClubPickOut], "timezone": str}`.
- The current pick is never included.

### 1.3 `PUT /api/pick`: set (or update) the current pick

- **Body `ClubPickSetIn`** (declaration order, which is also the order of error messages):

| field | type | default | rule / error |
|---|---|---|---|
| `ol_work_key` | str, required | none | Stripped. Must start with `"/works/"`, else `"ol_work_key must be an Open Library work key"`. Nothing else about it is validated: `"/works/a/b/XYZ"` is accepted and stored as `"/works/XYZ"` (probed). |
| `title` | str, required | none | Stripped. If empty: `"title is required"`. No maximum length. |
| `authors` | str | `""` | Not stripped, not limited. |
| `cover_id` | int\|null | null | Values ≤ 0 are treated as null by `upsert_book`. |
| `cover_url` | str\|null | null | `optional_cover_url`: stripped, then kept only if it is an allowed **Google** cover host; Open Library and unknown hosts become null. There is no error. |
| `year` | int\|null | null | No range check. |
| `note` | str (not nullable) | `""` | Stripped, then `len > 280` code points gives `"note must be 280 characters or fewer"`. `null` gives `"Input should be a valid string"`. |
| `meeting_at` | str\|null | null | Parsed in the handler (2.1). A parse failure gives 400 `"meeting_at must be a date and time"`. Whether the key is **present** matters (see below). |

  - Example (probed): `{"ol_work_key":"OL1W","title":"  ","note":"x"*281}` returns `"ol_work_key must be an Open Library work key; title is required; note must be 280 characters or fewer"`.
  - An empty body returns `"Field required; Field required"`.
  - The `meeting_at` parse error is raised in the handler, so it only shows when the body validators all passed.
- **Logic (`pick_ops.apply_club_pick`):**
  1. Parse `meeting_at` in the club timezone into UTC, or None. On failure: 400.
  2. **`upsert_book`, the catalog side effect (2.4):** find or create the `books` row by the canonical key. **An existing book is mutated:** `title` is always overwritten, and `authors`, `cover`, `year` are overwritten when provided. Any member can rename a catalog book this way.
  3. Load the current pick.
  4. **If the current pick has the same `book_id`, update it in place:**
     - `note = payload.note`, which resets to `""` when the key is omitted (probed)
     - `set_by_id = caller`
     - `meeting_at` changes **only if the key `meeting_at` was present in the JSON**: a value sets it, and `null`, `""` or whitespace clear it (probed)
     - `started_at` and `reminder_sent_at` are untouched, so a moved meeting is **not** re-reminded
     - **no activity event, no push**
     - commit
  5. **Otherwise:**
     - If a current pick exists, set its `ended_at = now`. This records no `pick_cleared`.
     - Insert a new `club_picks` row (book, `set_by` = caller, note, `meeting_at`, `started_at` = now).
     - Flush.
     - Record activity `pick_set` with book and pick. Its payload is `{"meeting_at": meeting.isoformat()}` when set, e.g. `"2026-08-22T19:00:00+00:00"`, otherwise `{}`.
     - Commit.
  6. The router compares the pick id before and after. If it changed, it queues a push of kind `pick`:
     - to every user except the caller with `notify_pick=true`
     - tag `pick-{id}`, url `/`
     - en copy `"{club}: new club pick"` / `"{actor} chose {title}."`
- **Response 200 `ClubPickOut`** (not wrapped), computed for the caller.
- **Errors:** 400 validation and meeting parse; 500 `"Could not save the club pick"` (unreachable).

### 1.4 `DELETE /api/pick`: end the current pick without a successor

- **Logic:**
  - If there is no current pick, return 200 `{"pick": null, "timezone": tz}` and write nothing.
  - Otherwise set `ended_at = now` and record `pick_cleared` with book and pick, payload `{}`. Commit.
- **Response 200 `ClubPickCurrentOut`** with `pick: null` (not 204).

### 1.5 `GET /api/pick/posts`: thread of the current pick

- **Errors:** 404 `"No club pick yet"` when there is no current pick.
- **Response 200 `PickPostListOut`:**
```json
{"pick_id": int, "can_post": bool, "my_progress": int|null,
 "my_status": ShelfStatus|null, "items": [PickPostOut], "timezone": str}
```
  - `can_post` is `pick.ended_at IS NULL`.
  - `my_status` and `my_progress` come from the caller's shelf entry for `pick.book_id`:
    - `my_status` is the entry's status, or null when there is no entry
    - `my_progress` is **100** if finished, `entry.progress` (may be null) if currently reading, and **null** otherwise (want_to_read, did_not_finish, no entry)
  - `items` are the pick's posts ordered by `created_at ASC, id ASC`, oldest first.
- **PickPostOut:**
```json
{"id": int, "author": str, "mine": bool, "body": str, "spoiler_upto": int|null,
 "milestone_id": int|null, "created_at": datetime, "created_label": str,
 "edited": bool, "reactions": [ReactionOut]}
```
  - `mine`: `author_id == caller`
  - `created_label`: `meeting_label(created_at, tz)`; never null in practice (`or ""`)
  - `edited`: `updated_at IS NOT NULL`
- **ReactionOut:** `{"emoji": str, "count": int, "mine": bool, "users": [str]}`. Aggregation and order are in 2.6.

### 1.6 `POST /api/pick/posts`: post on the current pick

- **Body `PickPostIn`:**
  - `body`: str, required. Stripped. Empty gives `"Write a short take, quote, or meeting note"`. More than 1000 **code points** after the strip gives `"Keep it to 1000 characters"` (`"ä"*1000` with surrounding spaces passes, probed).
  - `spoiler_upto`: int or null, `0..100`. Errors are `"Input should be greater than or equal to 0"` and `"Input should be less than or equal to 100"`.
  - `milestone_id`: int or null.
- **Check order:**
  1. body validation (400; this runs **before** the 404, probed)
  2. no current pick: 404 `"No club pick yet"`
  3. `_create_post`, which applies the rules in 1.8
- **Response 201 `PickPostOut`.**

### 1.7 `GET /api/pick/{pick_id}/posts`: thread of any pick (current or ended)

- **Path:** `pick_id` int. A non-integer gives 400 `"Input should be a valid integer, unable to parse string as an integer"`. An integer beyond int64 makes SQLite raise OverflowError, a **500** (probed).
- **Errors:** 404 `"No club pick with that id"`.
- **Response 200 `PickPostListOut`** (same as 1.5). An ended pick has `can_post: false`.

### 1.8 `POST /api/pick/{pick_id}/posts`

- **Body:** `PickPostIn` (1.6).
- **Checks, in order:**
  1. validation (400)
  2. 404 `"No club pick with that id"`
  3. pick ended: 400 `"That pick is closed"`. This runs before the milestone check, so a closed pick with a bad milestone still says "closed" (probed).
  4. `milestone_id` given but no such milestone, or a milestone on another pick: 400 `"That milestone is not on this pick"`
- **Writes:**
  - insert `club_pick_posts` (pick_id, author = caller, body, spoiler_upto, milestone_id, `created_at` now, `updated_at` NULL)
  - flush
  - record `note_posted` with book and pick, payload `{"post_id": <id>}`
  - commit
- **Push:** kind `note` to every user except the author with `notify_note=true`:
  - tag `note-{pick_id}`, url `/`
  - en copy `"{actor} on {title}"` / `"{excerpt}"`
  - the excerpt is the body if it is ≤ 90 code points, else the first 87 plus `"…"` (U+2026)
- **Response 201 `PickPostOut`.** 500 `"Could not save that note"` is unreachable.

### 1.9 `PATCH /api/pick/posts/{post_id}`: edit own post

- **Body `PickPostPatchIn`:**
  - `body`: str or null. When not null, the same rules as 1.6.
  - `spoiler_upto`: int or null, `0..100`. **Presence** of the key matters.
  - `milestone_id` is not editable. The key is ignored.
- **Checks, in order:**
  1. validation (400; runs before the 404/403, probed)
  2. no post: 404 `"No note with that id"`
  3. not the author: 403 `"You can only edit your own notes"`. An empty body from a non-author also gets 403.
  4. `body` is null **and** the key `spoiler_upto` is absent: 400 `"Nothing to update"`. Both `{}` and `{"body": null}` give this.
- **Writes:**
  - `body` when not null
  - `spoiler_upto` when the key is present (`null` clears it)
  - `updated_at = now`, **always**, even with identical values
  - commit
  - no activity event
- Works on posts of **ended** picks (probed); there is no closed check.
- **Response 200 `PickPostOut`**, with `edited: true`.

### 1.10 `DELETE /api/pick/posts/{post_id}`

- **Checks:** 404 `"No note with that id"`; not the author: 403 `"You can only delete your own notes"`.
- **Writes:** delete all `post_reactions` of the post, then the post. Commit.
  - No activity event.
  - Earlier `note_posted` and `reaction` events stay and keep a dangling `post_id`.
- Works on ended picks (probed).
- **Response 204**, empty body. A second delete gives 404.

### 1.11 `POST /api/pick/posts/{post_id}/reactions`: toggle a reaction

- **Body `ReactionIn`:** `emoji`: str, stripped, must be one of exactly these six strings, else `"Pick one of the club reactions"`:
  - `"❤️"` (U+2764 U+FE0F; a bare U+2764 is **rejected**, probed)
  - `"👍"` (U+1F44D)
  - `"😂"` (U+1F602)
  - `"😮"` (U+1F62E)
  - `"🔥"` (U+1F525)
  - `"📚"` (U+1F4DA)

  Validation runs before the 404, so a bad emoji on a missing post gives 400.
- **Checks:** 404 `"No note with that id"`.
- **Logic:**
  - **If a `(post_id, caller, emoji)` row exists:** delete it. No event.
  - **Otherwise:** insert it and record `reaction` with `book = pick.book` and `pick = the post's pick`. The payload is `{"emoji": e, "post_id": id, "author": <post author username>}`; `author` is dropped if null.
  - Commit.
- There is no restriction on reacting to your own post, or to posts of ended picks.
- Toggling on again later records **another** `reaction` event.
- **Response 200 `PickPostOut`** (status 200, not 201).

### 1.12 `GET /api/pick/meeting.ics`: download the current meeting

- **Errors:** 404 `"No meeting scheduled"` when there is no current pick, or the pick has no `meeting_at`, or no book.
- **Response 200:**
  - body: text from `ics.build_meeting_ics` (2.12)
  - header `Content-Type: text/calendar; charset=utf-8`
  - header `Content-Disposition: attachment; filename="bookclub-{pick_id}.ics"`
  - no cache headers
- **Inputs:**
  - club name: `sanitize_name(BOOKCLUB_NAME)`. That strips, drops non-printable characters and `<` `>`, collapses whitespace, cuts to 48 characters, and defaults to `"Bookclub"`.
  - `book.title`, `book.authors`, `pick.note`, `pick.meeting_at`, `book.ol_work_key`
  - public URL: `BOOKCLUB_PUBLIC_URL`, already reduced to `scheme://host[:port]` by the settings validator
- Only the **current** pick has an ICS. There is no feed or subscription URL.

### 1.13 `GET /api/pick/{pick_id}/milestones`

- **Errors:** 404 `"No club pick with that id"`. Any pick, current or ended.
- **Response 200 `MilestoneListOut`:** `{"pick_id": int, "items": [MilestoneOut], "timezone": str}`, with items ordered by `position ASC, id ASC`.
- **MilestoneOut:**
```json
{"id": int, "pick_id": int, "title": str, "chapter_from": int|null, "chapter_to": int|null,
 "due_at": datetime|null, "due_local": str|null, "due_label": str|null,
 "position": int, "note_count": int, "passed": bool}
```
  - `due_at` is serialized **with `Z`**, e.g. `"2030-01-05T20:00:00Z"`.
  - `note_count` is the number of `club_pick_posts` with this `milestone_id`.
  - `passed` is `due_at IS NOT NULL AND due_at <= now`.
  - `due_local` and `due_label` follow 2.1.

### 1.14 `POST /api/pick/{pick_id}/milestones`

- **Body `MilestoneIn`:**
  - `title`: str, required. Stripped. Empty gives `"Give the milestone a title"`. More than 120 code points gives `"Keep the title to 120 characters"`.
  - `chapter_from`: int or null, ≥ 0.
  - `chapter_to`: int or null, ≥ 0.
  - `due_at`: str or null.
- **Checks, in order:**
  1. validation (400)
  2. 404 `"No club pick with that id"`
  3. pick ended: 400 `"That pick is closed"`
  4. both chapters set and `chapter_to < chapter_from`: 400 `"Chapter range is backwards"`
  5. `due_at` parse failure: 400 **`"meeting_at must be a date and time"`** (the message really says `meeting_at`)
- A due date in the past is **allowed** and gives `passed: true` (probed).
- **Writes:**
  - insert with `position = number of existing milestones on the pick` and `created_by_id = caller`
  - flush
  - record `milestone_added` with book and pick, payload `{"title": <stripped title>, "due_at": <iso with +00:00>}`; `due_at` is dropped if null
  - commit
- **Response 201 `MilestoneOut`.**

### 1.15 `PATCH /api/pick/{pick_id}/milestones/{milestone_id}`

- **Body:** `MilestoneIn` again. It is a **full replacement**: omitted chapters and `due_at` become null, and `title` is required.
- **Checks:**
  1. validation
  2. 404 `"No club pick with that id"`
  3. the milestone does not exist or `milestone.pick_id != pick_id`: 404 `"No milestone with that id"`
  4. `due_at` parse failure: 400 `"meeting_at must be a date and time"`
- **No backwards-range check and no closed-pick check on PATCH** (probed: `chapter_from: 9, chapter_to: 1` is accepted, and editing on an ended pick works).
- `position` is unchanged. No activity event.
- **Response 200 `MilestoneOut`.**

### 1.16 `DELETE /api/pick/{pick_id}/milestones/{milestone_id}`

- **Checks:** 404 `"No club pick with that id"`, then 404 `"No milestone with that id"`.
- **Writes:**
  - set `milestone_id = NULL` on every post that referenced the milestone, so the posts survive
  - delete the milestone
  - renumber the pick's remaining milestones `position = 0..n-1` in `(position, id)` order
  - commit
- Allowed on ended picks.
- **Response 204.**

### 1.17 `GET /api/vote`: the open next-up vote

- **Logic:** first run `maybe_auto_close`, which may apply the winner and commit (2.8), then serialize the open vote.
- **Response 200 `VoteOut`** (field order as listed):
```json
{"vote_id": int|null, "nominations": [VoteNominationOut], "my_vote_id": int|null,
 "nomination_limit": 6, "can_nominate": bool, "timezone": str,
 "closes_at": datetime|null, "closes_local": str|null, "closes_label": str|null,
 "not_voted": [str], "voted_count": int, "member_count": int, "leader_id": int|null}
```
- **No open vote:**
  - `vote_id: null`, `nominations: []`, `my_vote_id: null`, `can_nominate: true`
  - `closes_*: null`, `leader_id: null`, `voted_count: 0`
  - `not_voted: []` (**empty, not every member**)
  - `member_count` = the number of **all users** (probed)
- **Open vote:**
  - `nominations` ordered by `(-votes, id)`
  - `my_vote_id` is the nomination the caller's ballot points to
  - `can_nominate` is `len(nominations) < 6`
  - `closes_at` is serialized **with `Z`**
  - `closes_local` and `closes_label` follow 2.1
  - `not_voted` is the usernames of **all users** (ordered by username) who have no ballot in this vote
  - `voted_count` is the number of distinct users with a ballot
  - `member_count` is the number of all users
  - `leader_id` comes from 2.8
- **VoteNominationOut:** `{"id": int, "book": BookOut, "nominated_by": str, "votes": int, "voters": [str], "mine": bool}`
  - `voters` is sorted alphabetically
  - `votes` is `len(voters)`
  - `mine` is whether the caller's ballot is on this nomination

### 1.18 `PATCH /api/vote`: set or clear the deadline

- **Body `VoteDeadlineIn`:** `{"closes_at": str|null}`, default null. `{}` therefore **clears** the deadline (probed).
- **Checks, in order:**
  1. no open vote: 404 `"Nominate a book first"`. This runs before parsing, so a garbage date with no vote gives 404 (probed).
  2. parse failure: 400 `"meeting_at must be a date and time"`
  3. parsed value `<= now`: 400 `"The deadline has to be in the future"`
- **Writes:** `closes_at` = the parsed value, or NULL for `null`, `""` or whitespace. Commit. No activity event.
- Does **not** run `maybe_auto_close`.
- **Response 200 `VoteOut`.**

### 1.19 `POST /api/vote/nominations`

- **Body `VoteNominateIn`:**
  - `ol_work_key`, required, same rule and message as 1.3
  - `title`, required, same rule and message as 1.3
  - `authors` (`""`), `cover_id`, `cover_url`, `year`
  - no `note` or `meeting_at`; extra keys are ignored (probed)
- **Logic, in order:**
  1. `maybe_auto_close` (may commit an auto-close)
  2. If there is no open vote, create a `next_up_votes` row and flush. It is created lazily by the first nomination.
  3. Nomination count ≥ 6: 400 `"This vote already has enough books"`
  4. `upsert_book` (the same catalog mutation as 1.3)
  5. The book is the **current** pick: 400 `"That book is already the club pick"`. Past picks may be nominated again.
  6. Already nominated in this vote: 400 `"That book is already nominated"`
  7. Insert the nomination (`nominated_by` = caller) and record `nominated` with the book, payload `{}` and **no pick**. Commit.

  Any 400 after step 2 rolls back the new vote row too.
- **Response 201 `VoteOut`.**

### 1.20 `POST /api/vote/cast`

- **Body `VoteCastIn`:** `{"nomination_id": int}`, required (`"Field required"`).
- **Logic:**
  1. `maybe_auto_close`
  2. No open vote: 404 `"No open next-up vote"`
  3. The nomination is not in the open vote: 404 `"No nomination with that id"`
  4. If the caller has no ballot in this vote, insert one and record `voted` with `book = nomination.book`, payload `{}`, no pick.
  5. Otherwise set `ballot.nomination_id`. **No event**, whether it switched or re-cast the same choice.
  6. Commit.
- **Response 200 `VoteOut`.**

### 1.21 `POST /api/vote/apply`: make a nomination the club pick

- **Body `VoteApplyIn`:**
  - `nomination_id`: int, required
  - `meeting_at`: str or null. A number gives `"Input should be a valid string"`.
- **Logic:**
  1. **Does not** run `maybe_auto_close`.
  2. No open vote: 404 `"No open next-up vote"`. A missing or foreign nomination: 404 `"No nomination with that id"`.
  3. `apply_club_pick` with `ClubPickSetIn(ol_work_key, title, authors, cover_id, year)` from the nomination's book. **`meeting_at` is passed explicitly**, so it counts as "present"; `note=""`. A bad `meeting_at` gives 400 `"meeting_at must be a date and time"`.
     - **Normal case:** this ends the current pick, creates a new one with `set_by` = caller, and records `pick_set`.
     - **Edge case** (probed): the nominated book is already the current pick, because someone set it via `PUT /api/pick` after the nomination. The same-book branch runs: `note` becomes `""`, `meeting_at` is overwritten (cleared if not given), `set_by` becomes the caller, and **no** `pick_set` event is recorded.
  4. `vote.ended_at = now` and `vote.winner_nomination_id = nomination.id`.
  5. Record `vote_closed` with book and pick, payload `{"votes": len(nomination.ballots)}`. **Zero votes is allowed.**
  6. Commit.
  7. Push of kind `pick` via `notify_new_pick`. This is **unconditional**, even in the same-book case.
- No role check: any member can apply any nomination, regardless of votes.
- **Response 200 `VoteApplyOut`:** `{"pick": ClubPickOut, "vote": VoteOut}`. `vote` is always the "no open vote" shape (1.17), because a new vote starts only with the next nomination.

### 1.22 `GET /api/vote/suggestions`

- **Logic:** 2.9. No `maybe_auto_close`.
- **Response 200 `VoteSuggestionsOut`:** `{"items": [{"book": BookOut, "score": int, "reasons": [str]}]}`, at most 6 items.

### 1.23 `GET /api/overlap`: books two or more members want to read

- **Query:** `include_reading`: bool, default false.
  - Accepted spellings (case-insensitive, probed): `true`, `1`, `yes`, `on`, `t`, `y`, and the false counterparts.
  - Anything else, including an empty value, gives 400 `"Input should be a valid boolean, unable to interpret input"`.
- **Logic:** 2.10.
- **Response 200 `OverlapOut`:**
```json
{"items": [{"book": BookOut, "count": int, "members": [{"username": str, "status": ShelfStatus}]}],
 "include_reading": bool}
```

### 1.24 `GET /api/activity`: club activity feed

- **Query parameters:**
  - `before`: int ≥ 1, optional. 0 gives `"Input should be greater than or equal to 1"`, and a non-int gives the integer parse message.
  - `limit`: int, 1..100, default 30. `1.5` gives the integer parse message.
  - `username`: str, optional. It is lowercased but **not stripped**, and an empty string means no filter.
- **Logic:**
  - Select events ordered by `id DESC`, `LIMIT limit+1`.
  - With `before`, keep only `id < before` (keyset pagination by id).
  - With `username`: if no user has that username, return `{"items": [], "has_more": false, "timezone": tz}`; otherwise keep only that actor's events.
  - `has_more` is whether more than `limit` rows came back. Return the first `limit`.
- **Response 200 `ActivityOut`:** `{"items": [ActivityItem], "has_more": bool, "timezone": str}`.
- **ActivityItem:**
```json
{"id": int, "kind": str, "actor": str, "mine": bool, "book": BookOut|null,
 "pick_id": int|null, "payload": object, "created_at": datetime, "created_label": str}
```
  - `payload` is the stored JSON object; it becomes `{}` when the stored text fails to decode or is not an object
  - `mine` is `actor_id == caller`
  - `created_label` is `meeting_label(created_at, tz)`, or `""`
- The kinds and their payloads are in 2.11.
- The feed covers the **whole instance**: personal shelf events and club events are mixed.

### 1.25 Related endpoints owned by other specs

- `GET /api/config` returns `timezone` (`spec/auth.md`).
- `club_pick` booleans on search hits and book details: `books.py: _current_pick_key` and `current_pick` (catalog spec).
- `book_posts.pick_id` stamping in the diary: `diary.py` sets `pick_id` when the current pick's book is the diary book (personal/diary spec).
- `GET /api/stats` lists all picks, current and ended, ordered by `id DESC` with per-pick reader stats (stats spec). #45 says that "club stats" become per club.

---

## 2. Domain rules

### 2.1 Timezone, parsing and labels (`timezone.py`)

- **`resolved_timezone(name)`:**
  - `(name or "UTC").strip() or "UTC"`
  - if `ZoneInfo(raw)` raises `ZoneInfoNotFoundError`, return `"UTC"`
  - case-sensitive: `"utc"` gives `"UTC"` because the lookup fails, and `" Europe/Berlin "` gives `"Europe/Berlin"` (probed)
  - quirks: `"localtime"` is accepted (a file in `/usr/share/zoneinfo`), and `"EST5EDT"` falls back to UTC on this tzdata
  - a path like `"../etc/passwd"` raises an uncaught `ValueError`, a 500 on every request
  - in Python this is env-only, so no user input reaches it
- **`parse_meeting(value, tz)`** is used for `meeting_at`, `closes_at` and `due_at`:
  - `None` → None. A stripped empty string → None.
  - A trailing `Z` is replaced with `+00:00`.
  - Then Python 3.12 `datetime.fromisoformat`. A failure gives `ValueError("meeting_at must be a date and time")`, which becomes **400 with that exact text** for all three fields.
  - A naive value is localised to the club zone with `replace(tzinfo=…)`, so `fold=0`. Every value is converted to UTC.
  - Accepted (probed):
    - `2026-08-22` (midnight local)
    - `2026-08-22T19`, `2026-08-22T19:00`, `2026-08-22 19:00`, `2026-08-22t19:00`
    - seconds, and fractions with `.` or `,`
    - `±HH:MM`, `±HHMM`, `Z`
    - ISO week dates `2026-W34-6`
    - basic format `20260822T1900`
  - Rejected: `2026-234` (ordinal), `24:00`, `2026-8-2`, `08/22/2026`, `next Tuesday`.
  - The SPA sends `<input type="datetime-local">` values (`YYYY-MM-DDTHH:MM`) or null.
- **DST behaviour, `fold=0` (probed in Europe/Berlin):**
  - Non-existent local time `2026-03-29T02:30` (spring-forward gap): the pre-transition offset (+01:00) is used, giving `2026-03-29T01:30Z`, displayed back as `03:30 CEST`.
  - Ambiguous `2026-10-25T02:30` (fall-back): the **first** occurrence (+02:00 CEST) is used, giving `2026-10-25T00:30Z`.
  - This is exactly jiff's `Disambiguation::Compatible` (RFC 5545/Temporal "compatible"). chrono-tz needs manual handling of `LocalResult::None`.
- **`meeting_local(dt, tz)`:**
  - None → None
  - a naive `dt` is treated as UTC
  - convert to the zone and format `%Y-%m-%dT%H:%M`, e.g. `"2026-08-22T19:00"`
  - seconds are dropped
- **`meeting_label(dt, tz)`:**
  - same conversion, then `%a %d %b %Y, %H:%M %Z` in the C locale (English names, zero-padded day)
  - examples (probed): `"Sat 22 Aug 2026, 19:00 UTC"`, `"Wed 01 Jul 2026, 20:00 CEST"`, `"Sat 22 Aug 2026, 19:00 -03"` for America/Sao_Paulo, `"Sun 23 Aug 2026, 03:30 IST"` for Asia/Kolkata
  - `%Z` is the tzdb abbreviation, numeric where tzdb has no letters
  - the label is **always English**, regardless of the user locale
  - used for `meeting_label`, `closes_label`, `due_label`, post `created_label` and activity `created_label`

### 2.2 Time source and storage

- `utcnow()` is `datetime.now(timezone.utc)`.
- SQLite stores `YYYY-MM-DD HH:MM:SS.ffffff` strings **without an offset**, always UTC, and reads them back naive.
- Comparisons in code re-attach UTC (`_aware`). Examples: `maybe_auto_close` compares `closes_at > now`, and milestones compare `due_at <= now`.

### 2.3 Pick state machine

- A pick is **current** while `ended_at IS NULL`. `current_pick` takes the highest id among those. There is no DB constraint, so in theory several could be open, and only the newest counts.
- Transitions:
  - **none → current:** `PUT` with any book, `vote/apply`, or auto-close
  - **current → same current (update):** `PUT` with the same book (`book_id` equality after `upsert_book`). This updates note, setter and (if present) meeting. No history entry.
  - **current → ended plus a new current:** `PUT`, apply or auto-close with a different book
  - **current → ended (none current):** `DELETE /api/pick`
- An ended pick never reopens. History is every ended pick.
- **What is frozen on an ended pick:**
  - new posts and new milestones are rejected (400 `"That pick is closed"`)
  - edits and deletes of posts, reactions, and milestone edits and deletes are still allowed
  - `pick_out` returns empty `readers`, `reading` and `finished` for ended picks
- **Meeting:** exactly one optional `meeting_at` per pick. There is no meetings table, location, or RSVP. `reminder_sent_at` marks that the 24 h reminder was sent; it is never reset.

### 2.4 `upsert_book` (shared by pick, nomination, shelf and quotes)

- The key is canonicalised: `work_key(work_id_from_key(key))`, i.e. `"/works/" + last path segment`. An `ISBN…` id gets an uppercase `X`.
- **Missing book:** insert with title, authors, `cover_id` (only if > 0), `cover_image_url` (only an allowed Google URL), year. The other columns take their defaults: description `""`, subjects `"[]"`, pages null, spine_color null.
- **Existing book:**
  - `title := payload.title`, always
  - `authors := payload.authors` only if non-empty
  - `cover_id` and `cover_image_url` are replaced when the new value is not None; if either changed, `spine_color := NULL`, so it is re-sampled later
  - `year := payload.year` if not None
- The work-id regexes in `works.py` (`OL\d+W`, `BC[a-f0-9]{10}`, `ISBN…`, `GB…`) are **not** enforced on these endpoints.

### 2.5 `pick_out` (ClubPickOut computation)

- `set_by` is the setter's username, or `"unknown"`.
- **Readers:**
  - every `shelf` row of **any user** whose `book_id == pick.book_id`, in **any** status (including want_to_read and did_not_finish)
  - loaded ordered by `status, id`, then **re-sorted** by `(username != viewer.username, username)`: the viewer first, then alphabetical (probed: `grace, ada, bob, zed` for viewer grace)
  - each reader carries `status`, `rating`, `take`, `dnf_reason`, `progress` and `avatar_url` (`/api/members/{username}/avatar?v={epoch}` or null)
- `reading` is the usernames of readers with status currently_reading, and `finished` those with status finished. Both lists **keep the reader order** (viewer first).
- `on_shelf` and `shelf_id` are the viewer's own entry for the book; they are also filled for ended picks.
- `readers`, `reading` and `finished` are `[]` when the pick is ended.
- `meeting_local` and `meeting_label` follow 2.1.

### 2.6 Pick posts and reactions

- **Thread order:** `created_at ASC, id ASC`.
- **Reactions:**
  - group by emoji, keeping each user's username (`"unknown"` if the user is missing)
  - `count` is the group size
  - `mine` is whether the viewer's username is in the group
  - `users` is sorted
  - the groups are sorted by `(-count, emoji)`, comparing emoji strings by code point
  - so on equal counts the order is ❤️ (U+2764) < 👍 < 📚 < 🔥 < 😂 < 😮 (probed). A byte-wise UTF-8 comparison gives the same order.
- `spoiler_upto` is a **percentage of the book** the note is safe up to. It is compared client-side with `my_progress`, also a percentage: `shelf.progress` is validated 0..100 in the shelf schemas, and finished counts as 100.
- `milestone_id` ties a post to a milestone of **the same pick**.
- Edits do not touch `created_at`.

### 2.7 Milestones

- They are a reading schedule per pick: a title, an optional chapter range, and an optional due time.
- `position` is assigned as the current count when created. A delete renumbers the rest densely, and there is no reorder endpoint.
- Units: `chapter_from` and `chapter_to` are **chapter numbers** (≥ 0, inclusive range, either may be null). There are no page or percent milestones in Python.
- The backwards check runs only on create.
- `passed` is computed at read time.
- `note_count` counts `club_pick_posts` rows with this milestone. It does **not** count diary (`book_posts`) rows.

### 2.8 Next-up vote state machine

- **Open vote:** the `next_up_votes` row with `ended_at IS NULL` and the highest id. At most one is meaningful.
  - It is created **only** by the first `POST /api/vote/nominations` when none is open.
  - It ends by `apply` (manual) or auto-close. An ended vote is never shown again (there is no history endpoint).
- **Nominations:**
  - at most 6 per vote (`NOMINATION_LIMIT`)
  - unique `(vote_id, book_id)`
  - the current pick's book cannot be nominated
  - order of creation is `(created_at, id)`
- **Ballots:** at most one per `(vote_id, user_id)`. Re-casting moves the ballot. A `voted` event is only recorded on the first ballot. There is no "retract ballot" endpoint.
- **Leader (`_leader`):**
  - iterate nominations in `(created_at, id)` order
  - keep the first with the strictly highest `len(ballots)`, so **ties go to the earliest nomination**
  - return None when every count is 0
  - `len(ballots)` here includes ballots of deleted users, while `votes` in the output excludes them
- **Deadline:** `closes_at` is optional and must be in the future when set. It can be cleared.
- **Auto-close (`maybe_auto_close`)** is lazy: there is no scheduler. It runs at the start of `GET /api/vote`, `POST /api/vote/nominations` and `POST /api/vote/cast` only, **not** on `GET /api/pick`, `PATCH /api/vote`, `apply` or `suggestions`.
  - No open vote, or `closes_at` is null or still in the future: do nothing.
  - **No leader** (no ballots), or the leader's nominator is missing: set `closes_at = NULL`, **commit**, and leave the vote open so the club decides by hand.
  - Otherwise:
    1. `apply_club_pick` with the winner's book, **no `meeting_at`** (so the new pick has no meeting), `note=""`, and `actor` = **the winner's nominator**. That makes the nominator `set_by` and the actor of `pick_set`.
    2. `vote.ended_at = now`, `winner_nomination_id = winner.id`.
    3. Record `vote_closed` as the nominator with book and pick, payload `{"votes": len(winner.ballots), "auto": true}`.
    4. Commit.
    5. Push of kind `pick` (excluding the nominator).

    Event order is `pick_set` then `vote_closed`, so the newest-first feed shows `vote_closed` first (asserted by a test).
- **Manual apply:** 1.21. Its `vote_closed` payload has **no** `auto` key.

### 2.9 Suggestions (`GET /api/vote/suggestions`)

1. **Excluded books:**
   - the current pick's book
   - every nomination in the open vote
   - every book of an **ended** pick (`past_ids`)
2. **`liked_subjects`:** for each book in `past_ids`, every subject from `subjects_from_json(book.subjects)`, lowercased, goes into a set. The code keeps counts, but only membership is used. The current pick's subjects do not count.
3. **Candidates:** every shelf entry with status `want_to_read` from **all users**, whose book is not excluded, grouped by book.
4. **Per candidate:**
   - `names` = the sorted usernames of the wanters
   - **2 or more names:** `score += 3 * n`, reason `"On {n} shelves: {names joined by ', '}"`
   - **exactly 1 name:** `score += 1`, reason `"{name} wants to read it"`
   - `matched` = the book's subjects (original case, the book's order, duplicates kept) whose lowercase form is in `liked_subjects`. If any: `score += len(matched)` (**all** matches count), reason `"Like past picks: " + ", ".join(matched[:3])`
   - if `book.ol_rating` is set and ≥ 4.0: `score += 1`, reason `f"Open Library rating {ol_rating:.1f}"`. Python rounds half-to-even on the binary value: 4.25 gives `"4.2"` (probed).
   - reasons are always in that order
5. Sort by `(-score, title.lower())` and take the first 6.

Probed example:
```json
{"score": 6, "reasons": ["ada wants to read it", "Like past picks: women, ROME, history", "Open Library rating 4.2"]}
```
`subjects_from_json` returns `[]` on invalid JSON or a non-list, and stringifies the items.

### 2.10 Overlap (`GET /api/overlap`)

- Statuses are `{want_to_read}`, plus `currently_reading` when `include_reading`.
- Take every shelf entry of **all users** in those statuses and group them by book.
- Keep books with **2 or more distinct users**. The viewer does not need to be one of them.
- `members` is `[(username, status)]` sorted by username.
- `count` is `len(members)`.
- Sort by `(-count, title.lower())`. There is no limit and no pagination.
- Finished and did-not-finish entries never count.

### 2.11 Activity log (`activity.py`)

- **`record(session, actor, kind, book=None, pick=None, **payload)`:**
  - raises `ValueError` for an unknown kind
  - **drops payload keys whose value is None**
  - stores `json.dumps(clean, ensure_ascii=False)` with keys in call order
  - `book_id` and `pick_id` are nullable FKs
  - `created_at` is now
  - it shares the caller's transaction, so a rollback drops the event too
- **The log is append-only:** nothing ever updates or deletes an event. Deleting a post, a shelf entry or a reaction leaves the events in place.
- **Kinds (`KINDS`) and their emitters:**

| kind | emitted by | book | pick | payload (None keys omitted) |
|---|---|---|---|---|
| `member_joined` | `POST /api/auth/register` | null | null | `{}` |
| `shelf_added` | `POST /api/shelf` with status want_to_read or currently_reading | yes | null | `{"status": <status>}` |
| `shelf_moved` | `PATCH /api/shelf/{id}` status change to want_to_read or currently_reading | yes | null | `{"status": <new status>}` |
| `shelf_finished` | shelf add or patch into finished | yes | null | `{"rating": int?, "take": str?}`; an empty take is omitted |
| `shelf_dnf` | shelf add or patch into did_not_finish | yes | null | `{"reason": str?}`; an empty reason is omitted |
| `shelf_removed` | `DELETE /api/shelf/{id}` | yes | null | `{"status": <status at removal>}` |
| `progress` | `PATCH /api/shelf/{id}` with an unchanged currently_reading status, the `progress` key present, a non-null value that differs from before | yes | null | `{"progress": int}` |
| `pick_set` | new pick via PUT, apply or auto-close | yes | yes | `{"meeting_at": "<iso +00:00>"?}` |
| `pick_cleared` | `DELETE /api/pick` | yes | yes | `{}` |
| `note_posted` | create pick post | yes | yes | `{"post_id": int}` |
| `reaction` | reaction toggled **on** | yes | yes | `{"emoji": str, "post_id": int, "author": str?}` |
| `nominated` | nomination | yes | null | `{}` |
| `voted` | first ballot | yes | null | `{}` |
| `vote_closed` | apply / auto-close | yes | yes | `{"votes": int}` / `{"votes": int, "auto": true}` |
| `milestone_added` | create milestone | yes | yes | `{"title": str, "due_at": "<iso +00:00>"?}` |
| `quote_added` | `POST /api/quotes` | yes | null | `{"page": int?, "excerpt": body[:80]}` (code points) |

- The Goodreads import, quote edits and deletes, diary posts, milestone edits and deletes, and post edits and deletes record **nothing**.
- Feed rendering is client-side from `kind` plus `payload`.

### 2.12 ICS rendering (`ics.py`)

**Lines**, in order:
1. `BEGIN:VCALENDAR`
2. `VERSION:2.0`
3. `PRODID:-//{esc(club_name)}//Bookclub//EN`
4. `CALSCALE:GREGORIAN`
5. `METHOD:PUBLISH`
6. `BEGIN:VEVENT`
7. `UID:pick-{pick_id}@bookclub`
8. `DTSTAMP:{now}`
9. `DTSTART:{meeting_at}`
10. `DTEND:{meeting_at + 90 min}`
11. `SUMMARY:{esc(club_name + ": " + title)}`
12. `DESCRIPTION:{esc(description)}`
13. `URL:{public_url}`, only if a public URL is set
14. `END:VEVENT`
15. `END:VCALENDAR`

**Times:** UTC `%Y%m%dT%H%M%SZ`. A naive datetime is treated as UTC. The meeting lasts a fixed 90 minutes.

**Description:**
- the parts are joined with `"\n"`, then escaped
- part 1 is `title`, plus `" — " + authors` when authors is non-empty (EM DASH U+2014 with spaces)
- then `note`, if non-empty
- then `"https://openlibrary.org" + ol_work_key`, **even for `BC…`, `ISBN…` and `GB…` keys**, which gives dead links (probed)
- then `public_url`, if set

**`esc`** replaces, in order:
1. `\` → `\\`
2. `;` → `\;`
3. `,` → `\,`
4. CRLF → `\n`
5. LF → `\n`

A lone CR is not escaped, and colons are not escaped.

**Folding:**
- each line longer than 75 **octets** is cut into a first chunk of ≤ 75 bytes and continuation chunks of ≤ 74 bytes, each prefixed by one space
- the code backs off so a UTF-8 sequence is never split
- lines are joined with CRLF, and the body ends with CRLF

**Golden output** (probed; `now` fixed at 2026-09-01T12:00Z):
- input: pick 7, `"Bookclub"`, `"Circe"`, `"Madeline Miller"`, note `"Bring snacks; wine, too"`, meeting 2026-09-10T19:30Z, `/works/OL1W`, public URL `https://books.example.com`

```
BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Bookclub//Bookclub//EN\r\nCALSCALE:GREGORIAN\r\nMETHOD:PUBLISH\r\nBEGIN:VEVENT\r\nUID:pick-7@bookclub\r\nDTSTAMP:20260901T120000Z\r\nDTSTART:20260910T193000Z\r\nDTEND:20260910T210000Z\r\nSUMMARY:Bookclub: Circe\r\nDESCRIPTION:Circe — Madeline Miller\nBring snacks\; wine\, too\nhttps://o\r\n penlibrary.org/works/OL1W\nhttps://books.example.com\r\nURL:https://books.example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n
```

The `\n` inside DESCRIPTION is the two characters backslash and `n`.

**UTF-8 fold vector:**
- input: `"Bücherclub"`, title `"Ä"*40`, no authors or note, 2026-01-01T18:00Z, `/works/OL7W`, `now` 2026-01-01T00:00Z
- SUMMARY is `SUMMARY:Bücherclub: ` followed by 27 `Ä`, then `\r\n ` and 13 `Ä`
- DESCRIPTION is `DESCRIPTION:` followed by 31 `Ä`, then `\r\n ` and 9 `Ä\nhttps://openlibrary.org/works/OL7W`

### 2.13 Push hooks (Web Push; deferred to Phase 4 under the plan)

| trigger | kind | recipients | tag | pref |
|---|---|---|---|---|
| new pick (PUT, apply, auto-close) | `pick` | all users except the actor | `pick-{pick_id}` | `notify_pick` |
| new pick post | `note` | all users except the author | `note-{pick_id}` | `notify_note` |
| `GET /api/pick` within 24 h of the meeting, once | `meeting` | all users, including the viewer | `meeting-{pick_id}` | `notify_meeting` |

- Copy is per recipient locale (en/de) from `backend/app/i18n.py`. `{club}` is the sanitized `BOOKCLUB_NAME`.
- Delivery runs as background tasks after the response. 404/410 responses prune the subscription.

### 2.14 Transactions and concurrency (Python)

- There is one SQLAlchemy session per request, on SQLite (one writer), with no explicit locks.
- `maybe_auto_close` and the meeting reminder **commit mid-request**, before the main work of the request.
- **Races Python does not guard against:**
  - two concurrent `PUT /api/pick` can leave two open picks
  - two first nominations can create two open votes
  - nomination count then insert can exceed 6
  - concurrent auto-closes can apply twice

  The unique constraints (`uq_next_up_vote_book`, `uq_next_up_vote_user`, `uq_reaction_post_user_emoji`) turn the remaining duplicate inserts into IntegrityError, a 500.

---

## 3. Data

### 3.1 Tables this area owns (SQLite DDL, fresh `create_all`)

**Common to all of them:**
- No `AUTOINCREMENT`, so SQLite may **reuse the highest deleted rowid**. A deleted newest post's id can be handed to the next post, and old `events.payload.post_id` values then point at the wrong post.
- Every `DATETIME` is naive UTC text, `YYYY-MM-DD HH:MM:SS[.ffffff]`.

**`club_picks`**
- `id` INTEGER PK
- `book_id` INTEGER NOT NULL FK `books.id` (indexed)
- `set_by_id` INTEGER NOT NULL FK `users.id`: last setter (the latest same-book PUT or apply overwrites it; for auto-close it is the nominator)
- `note` VARCHAR(280) NOT NULL: `""` means no note
- `started_at` DATETIME NOT NULL
- `ended_at` DATETIME NULL (indexed): NULL means current
- `meeting_at` DATETIME NULL: the meeting start in UTC; NULL means no meeting
- `reminder_sent_at` DATETIME NULL: NULL means the 24 h reminder has not been sent
- Migrations: `meeting_at` and `reminder_sent_at` were added via `COLUMN_MIGRATIONS` (DDL `DATETIME`). The legacy schema had only `id, book_id, set_by_id, note, started_at, ended_at`.

**`pick_milestones`**
- `id` INTEGER PK
- `pick_id` INTEGER NOT NULL FK `club_picks.id` (indexed)
- `title` VARCHAR(120) NOT NULL
- `chapter_from` INTEGER NULL and `chapter_to` INTEGER NULL: chapter numbers, ≥ 0 at the API, inclusive; NULL means open or unspecified
- `due_at` DATETIME NULL: UTC
- `position` INTEGER NOT NULL: dense 0..n-1 per pick
- `created_by_id` INTEGER NOT NULL FK users
- `created_at` DATETIME NOT NULL
- The whole table was created by `create_all` in a later release; no column migrations.

**`club_pick_posts`**
- `id` INTEGER PK
- `pick_id` INTEGER NOT NULL FK (indexed)
- `author_id` INTEGER NOT NULL FK users
- `body` VARCHAR(1000) NOT NULL, stripped
- `spoiler_upto` INTEGER NULL: percent 0..100; NULL means no spoiler flag
- `milestone_id` INTEGER NULL FK `pick_milestones.id`: NULL means none, or a detached post
- `created_at` DATETIME NOT NULL
- `updated_at` DATETIME NULL: NULL means never edited
- Migrations: `spoiler_upto` (`INTEGER`), `milestone_id` (`INTEGER REFERENCES pick_milestones(id)`) and `updated_at` (`DATETIME`). Legacy rows have NULLs there.

**`post_reactions`**
- `id` PK
- `post_id` NOT NULL FK `club_pick_posts.id` (indexed)
- `user_id` NOT NULL FK
- `emoji` VARCHAR(16) NOT NULL: one of the six strings in 1.11
- `created_at` NOT NULL
- `UNIQUE (post_id, user_id, emoji)` (`uq_reaction_post_user_emoji`)

**`next_up_votes`**
- `id` PK
- `started_at` NOT NULL
- `ended_at` NULL (indexed): NULL means open
- `closes_at` NULL: UTC deadline; NULL means no deadline (added via `COLUMN_MIGRATIONS`)
- `winner_nomination_id` INTEGER NULL with **no FK**: set when ended through apply or auto-close
- Legacy ended votes may have NULL `winner_nomination_id`.

**`next_up_nominations`**
- `id` PK
- `vote_id` NOT NULL FK (indexed)
- `book_id` NOT NULL FK `books.id`
- `nominated_by_id` NOT NULL FK users
- `created_at` NOT NULL
- `UNIQUE (vote_id, book_id)` (`uq_next_up_vote_book`)

**`next_up_ballots`**
- `id` PK
- `vote_id` NOT NULL FK (indexed)
- `nomination_id` NOT NULL FK (indexed)
- `user_id` NOT NULL FK
- `created_at` NOT NULL: the **first** cast; it is not updated when a ballot moves
- `UNIQUE (vote_id, user_id)` (`uq_next_up_vote_user`)

**`events`**
- `id` PK
- `actor_id` NOT NULL FK users (indexed)
- `kind` VARCHAR(32) NOT NULL (indexed): one of the 16 kinds in 2.11
- `book_id` NULL FK books
- `pick_id` NULL FK `club_picks`
- `payload` VARCHAR NOT NULL: a JSON object as text, non-ASCII stored raw, default `'{}'`
- `created_at` NOT NULL (indexed)

### 3.2 Tables this area reads or mutates but does not own

- **`users`:**
  - `username`, for display and the `username` filter
  - all rows, for `member_count` and `not_voted`
  - `avatar_updated_at`, for the reader avatar URL
  - `notify_*` and `locale`, for push
- **`shelf`** (personal): `user_id`, `book_id`, `status` (`want_to_read|currently_reading|finished|did_not_finish`), `rating` (1–5), `take` (≤ 140), `dnf_reason` (≤ 200), `progress` (percent 0–100, only while currently_reading). Read for readers, overlap, suggestions, `my_progress` and `my_status`.
- **`books`** (catalog):
  - created or **mutated** by `upsert_book` from PUT pick and nominate
  - read: `title`, `authors`, `cover_id`, `cover_image_url`, `year`, `pages`, `spine_color`, `ol_work_key`
  - `subjects` (a JSON list of Open Library subject strings) and `ol_rating` (float) are used only by suggestions
- **`book_posts`** (diary): `pick_id` (the current pick when the entry was written on that pick's book) and `legacy_post_id` (unique; set for rows copied from `club_pick_posts`).
- **`book_post_reactions`**: copies of `post_reactions` made by the data migration below.
- **`push_subscriptions`**: push only.

### 3.3 Data migrations touching this area (`db.py`, run on every boot)

- `COLUMN_MIGRATIONS` adds:
  - `club_picks.meeting_at` and `club_picks.reminder_sent_at`
  - `club_pick_posts.spoiler_upto`, `club_pick_posts.milestone_id` and `club_pick_posts.updated_at`
  - `next_up_votes.closes_at`
- **`import_legacy_pick_posts`** (idempotent, every boot):
  1. For every `club_pick_posts` row (joined to its pick, ordered `created_at, id`) whose id is not yet a `book_posts.legacy_post_id`, insert a **top-level** `book_posts` row:
     - `book_id = pick.book_id`, `author_id`, `body`, `spoiler_upto`
     - `pick_id = pick.id`, `legacy_post_id = post.id`
     - `created_at` and `updated_at` copied
     - `progress_at`, `status_at`, `parent_id` and `deleted_at` NULL
  2. Copy each `post_reactions` row of that post into `book_post_reactions`, keeping `created_at`.
- **Consequences for the import (#38):**
  - every club pick post exists **twice** in a production DB: once in `club_pick_posts`, once in `book_posts`
  - the diary copy is what the SPA has shown and let people edit, reply to and react on since the diary shipped, so the two copies may have **diverged**: a diary edit is not reflected back, and reactions added later live only on the diary copy
  - milestones are **not** carried into `book_posts`
  - a pick post created after that migration (only possible through the API, since there is no UI) gets copied on the next restart

---

## 4. Tests (parity checklist; 43 tests in the listed files)

### backend/tests/test_pick.py (8)
- `test_pick.py::test_pick_requires_auth`: GET, PUT and DELETE `/api/pick` signed out all return 401.
- `test_pick.py::test_empty_pick_then_set_and_read`: an empty club returns `pick: null` and `timezone "UTC"`, and `/api/config` returns timezone UTC. PUT with a note returns 200 with `book.title`, `set_by=ada`, the note, `ended_at` null, `meeting_at` null, `on_shelf` null, and empty `readers`, `reading` and `finished`. GET then returns the pick with `ol_work_key "/works/OL1W"`.
- `test_pick.py::test_same_book_updates_meeting_without_history`: a second PUT of the same book with a new `meeting_at` returns 200 with `meeting_local "2026-08-29T18:30"` and a non-empty `meeting_label`. History stays `[]` with `timezone "UTC"`.
- `test_pick.py::test_replacing_pick_archives_previous_with_meeting`: a PUT of a different book makes it current with a null meeting. History has one item (Circe) with `ended_at` set, `meeting_local "2026-08-22T19:00"`, and empty `readers`, `reading` and `finished`.
- `test_pick.py::test_reading_vs_finished_and_clear`: ada has the book as currently reading, so `on_shelf currently_reading` and `reading ["ada"]`. grace adds it finished, so for grace `on_shelf finished`, `reading ["ada"]`, `finished ["grace"]`. DELETE returns 200 `pick: null`, GET returns null, and history has one item.
- `test_pick.py::test_meeting_uses_club_timezone`: with `BOOKCLUB_TZ=America/New_York`, config returns that timezone. A PUT with `meeting_at "2026-08-22T19:00"` gives `meeting_local "2026-08-22T19:00"`, `meeting_at` starting with `"2026-08-22T23:00:00"`, and a non-empty label. GET returns timezone New York.
- `test_pick.py::test_invalid_timezone_falls_back_to_utc`: `BOOKCLUB_TZ=Not/AZone` makes config report UTC, and the meeting is stored as 19:00 UTC with local 19:00.
- `test_pick.py::test_invalid_pick_payload`: a blank title, a key without `/works/`, a 281-character note and `meeting_at "next Tuesday"` each return 400.

### backend/tests/test_pick_posts.py (5)
- `test_pick_posts.py::test_posts_require_auth_and_a_pick`: GET and POST `/api/pick/posts` signed out return 401. Signed in with no pick, both return 404.
- `test_pick_posts.py::test_members_can_post_and_read_oldest_first`: a POST returns 201 with author ada, body stripped to `"Opening note"`, and a non-empty `created_label`. A second member posts. The thread has the right `pick_id`, `can_post` true, and bodies and authors oldest first. `/api/pick/{id}/posts` returns the same order.
- `test_pick_posts.py::test_old_thread_stays_when_pick_changes`: after the pick changes, the current thread has the new `pick_id`, no items and `can_post` true. The old thread by id has `can_post` false and keeps its post. A POST to the old pick returns 400.
- `test_pick_posts.py::test_post_time_uses_club_timezone`: with New York, the post's `created_label` contains `EDT` or `EST`, and the thread timezone is America/New_York.
- `test_pick_posts.py::test_invalid_post_body`: a whitespace-only body returns 400, and 1001 characters returns 400.

### backend/tests/test_vote.py (7)
- `test_vote.py::test_vote_requires_auth`: GET `/api/vote`, and POST to nominations, cast and apply, all return 401 signed out.
- `test_vote.py::test_empty_vote_then_nominate_and_count`:
  - empty vote: `vote_id` null, nominations `[]`, `my_vote_id` null, `can_nominate` true, `nomination_limit` 6
  - nominating returns 201 with a `vote_id`, `can_nominate` true, and one nomination (title Circe, `nominated_by` ada, votes 0, mine false)
  - the pick is still null
- `test_vote.py::test_members_can_vote_and_change_vote`:
  - ada casts for Circe: `my_vote_id` is Circe's id, Circe has votes 1, voters `["ada"]`, mine true, and Achilles has 0
  - grace casts for Circe: votes 2, voters `["ada","grace"]`
  - grace switches to Achilles: Circe 1, Achilles 1, `my_vote_id` Achilles
  - the pick stays null
- `test_vote.py::test_cannot_nominate_current_pick_or_duplicate`: nominating the current pick returns 400, and nominating an already nominated book returns 400.
- `test_vote.py::test_nomination_limit`: six nominations return 201 each. The seventh returns 400 with detail `"This vote already has enough books"`.
- `test_vote.py::test_apply_winner_uses_club_pick_flow`:
  - the old pick has a post; Achilles is nominated, gets a vote, and the pick is still Circe
  - apply with `meeting_at "2026-09-12T19:00"` returns 200: a new pick id, title Achilles, `set_by` ada, `meeting_local "2026-09-12T19:00"`, and `vote.vote_id` null with nominations `[]`
  - the current pick matches, `history[0]` is the old pick, and the old thread keeps its post with `can_post` false
  - a fresh GET vote returns `vote_id` null, and a new nomination returns 201 with Wolf Den first
- `test_vote.py::test_apply_requires_an_open_nomination`: apply with nomination 99 and no vote returns 404. After nominating and applying, a second apply of the same nomination returns 404.

### backend/tests/test_vote_deadline.py (5)
- `test_vote_deadline.py::test_deadline_and_turnout_reporting`:
  - PATCH with no vote returns 404
  - after one nomination: `closes_at` null, `member_count` 1, `not_voted ["ada"]`, `voted_count` 0, `leader_id` null
  - a past deadline returns 400, and `"not a date"` returns 400
  - a future deadline returns 200 with `closes_local "2030-01-01T18:00"` and a non-empty label
  - after grace votes: `member_count` 2, `voted_count` 1, `not_voted ["ada"]`, `leader_id` = that nomination
  - PATCH with `{"closes_at": null}` clears it
- `test_vote_deadline.py::test_deadline_auto_applies_the_leader`:
  - grace nominates Achilles and votes for it, a deadline is set, then expired directly in the DB
  - GET vote returns `vote_id` null and nominations `[]`
  - the pick is Achilles with `set_by` **grace** (the nominator)
  - the first two activity kinds are `["vote_closed","pick_set"]`, `items[0].payload == {"votes": 1, "auto": true}`, and the actor is grace
- `test_vote_deadline.py::test_deadline_with_no_ballots_reopens_without_deadline`: an expired deadline with no ballots leaves the vote open (`vote_id` not null) with `closes_at` null, the nomination still there, and the pick still null.
- `test_vote_deadline.py::test_tie_goes_to_earliest_nomination`: one vote each for Circe (nominated first) and Achilles gives `leader_id` Circe. After the deadline expires and GET vote runs, the pick is Circe.
- `test_vote_deadline.py::test_suggestions_rank_shared_tbr_and_subject_affinity`:
  - setup: suggestions are initially `{"items": []}`. Dune becomes a past pick with subjects `["Science fiction","Politics"]`. ada wants Circe, Piranesi and Dune; grace wants Circe and Achilles; Achilles gets subjects `["Greek mythology","Science fiction"]` and `ol_rating` 4.4.
  - Circe is first with score 6 and reasons `["On 2 shelves: ada, grace"]`, and Dune is excluded.
  - Achilles has score 3, with reasons containing `"Like past picks: Science fiction"` and `"Open Library rating 4.4"`.
  - After Circe is nominated, it disappears from the suggestions.

### backend/tests/test_overlap.py (4)
- `test_overlap.py::test_overlap_requires_auth`: GET `/api/overlap` signed out returns 401.
- `test_overlap.py::test_two_want_same_book_shows_one_does_not`: ada wants Circe and Achilles, grace wants Circe. The result has `include_reading` false, titles `["Circe"]`, count 2, members `["ada","grace"]`, all want_to_read.
- `test_overlap.py::test_finished_and_solo_want_are_excluded`: ada has Circe finished and grace wants it, so items is `[]`.
- `test_overlap.py::test_include_reading_counts_want_plus_reading`: ada is reading Circe and grace wants it. The default returns `[]`. `include_reading=True` returns that flag true, Circe with count 2, and statuses `{ada: currently_reading, grace: want_to_read}`.

### backend/tests/test_activity.py (3)
- `test_activity.py::test_feed_requires_auth`: GET `/api/activity` signed out returns 401.
- `test_activity.py::test_feed_records_the_club_story_newest_first`:
  - after registering, the feed is `[("ada","member_joined")]`
  - the scripted flow (shelf add, move, progress 50 twice, finish with rating 4 and take "Great", pick, post, reaction, nominate, cast twice, shelf delete) yields exactly, newest first: `shelf_removed, voted, nominated, reaction, note_posted, pick_set, shelf_finished, progress, shelf_moved, shelf_added, member_joined`
  - so the unchanged progress and the re-cast produce no events
  - `shelf_finished` has book Circe, payload `{"rating":4,"take":"Great"}`, mine true and a non-empty `created_label`
  - `progress` has payload `{"progress":50}`
  - `reaction` has payload emoji `❤️` and author `"ada"`
  - `pick_set` has a non-null `pick_id` and book Achilles
- `test_activity.py::test_feed_paginates_and_filters_by_member`:
  - 5 shelf adds plus `member_joined`
  - `limit=3` returns 3 items with `has_more` true; `before=last id` returns the next 3 with `has_more` false; the pages are disjoint
  - after grace registers: `username=grace` returns `[("grace","member_joined")]`, `username=ada` returns only ada's events, `username=nobody` returns `[]`, and grace's own item has `mine` true

### backend/tests/test_notes_and_meeting.py (11)
- `test_notes_and_meeting.py::test_post_carries_spoiler_flag_and_viewer_progress`:
  - a post with `spoiler_upto` 40 returns it, with mine true, edited false and reactions `[]`
  - the thread has `my_progress` and `my_status` null
  - after a currently-reading shelf entry with progress 25: `my_progress` 25 and `my_status` currently_reading
  - after moving it to finished: `my_progress` 100 and `my_status` finished
- `test_notes_and_meeting.py::test_spoiler_flag_must_be_a_percentage`: `spoiler_upto` 101 returns 400, and -1 returns 400.
- `test_notes_and_meeting.py::test_author_can_edit_and_delete_own_note`:
  - PATCH body returns 200 with the new body and edited true
  - PATCH `spoiler_upto` 60 sets it, and PATCH `spoiler_upto` null clears it
  - PATCH `{}` returns 400
  - DELETE returns 204, the thread is empty, and a second DELETE returns 404
- `test_notes_and_meeting.py::test_other_members_cannot_edit_or_delete`: grace's PATCH and DELETE on ada's post return 403, and the thread shows `mine` false for grace.
- `test_notes_and_meeting.py::test_reactions_toggle_and_aggregate`:
  - ada ❤️ gives `[{"emoji":"❤️","count":1,"mine":true,"users":["ada"]}]`
  - grace ❤️ gives count 2, mine true, users `["ada","grace"]`
  - grace 🔥 gives the order `["❤️","🔥"]`
  - grace ❤️ again toggles it off: `{❤️, count 1, mine false, users ["ada"]}`
  - 🍕 returns 400, and post 9999 returns 404
- `test_notes_and_meeting.py::test_deleting_a_note_removes_its_reactions`: a post with a 👍 reaction can be deleted (204), with no FK failure.
- `test_notes_and_meeting.py::test_change_password` (**auth area**, #39): a wrong current password returns 400, a short new password returns 400, the same password returns 400, and a valid change returns 204. Then the old password login returns 401 and the new one returns 204.
- `test_notes_and_meeting.py::test_change_password_requires_auth` (**auth area**): PATCH `/api/auth/password` signed out returns 401.
- `test_notes_and_meeting.py::test_notification_prefs_default_on_and_patch` (**auth/settings area**): GET returns all three prefs true. PATCH `notify_note: false` returns the updated triple, and it persists.
- `test_notes_and_meeting.py::test_meeting_ics_download`:
  - with no pick, and with a pick without a meeting, it returns 404
  - with meeting `2026-09-10T19:30` UTC and note `"Bring snacks; wine, too"`: 200, Content-Type starting with `text/calendar`, Content-Disposition containing `filename="bookclub-`
  - the body starts with `BEGIN:VCALENDAR\r\n` and ends with `END:VCALENDAR\r\n`, and every line is ≤ 75 UTF-8 bytes
  - after unfolding it contains `BEGIN:VEVENT`, `DTSTART:20260910T193000Z`, `DTEND:20260910T210000Z`, `SUMMARY:Bookclub: Circe`, `Bring snacks\; wine\, too` and `https://openlibrary.org/works/OL1W`
- `test_notes_and_meeting.py::test_ics_folds_long_lines_without_breaking_utf8`: a unit test of `build_meeting_ics` with club `"Bücherclub"` and title `"Ä"*120`. Every line is ≤ 75 bytes, and the unfolded text contains `"SUMMARY:Bücherclub: " + "Ä"*120`.

### Cross-reference: `backend/tests/test_stats_milestones_quotes.py::test_milestones_crud_and_threads`

The stats, milestones and quotes file is listed in another area's spec. This test exercises this area:
- the empty list is exactly `{"pick_id", "items": [], "timezone": "UTC"}`
- create returns 201 with position 0, `due_local "2030-01-05T20:00"`, passed false and `note_count` 0
- a second create gets position 1 and `due_at` null
- an empty title returns 400, and a backwards range returns 400
- a post with `milestone_id` returns 201 and echoes it; an unknown milestone returns 400; `note_count` becomes 1
- PATCH (a full replacement) returns 200 with the new title and `due_at` null
- DELETE returns 204, and the remaining milestone is renumbered to position 0
- the post survives with `milestone_id` null
- `/api/pick/999/milestones` returns 404, and deleting milestone 9999 returns 404
- after the pick changes, POST returns 400
- the feed contains `milestone_added`

---

## 5. Multi-club mapping

Labels:
- **[plan]**: decided in `plan.md` or the issues (#37, #38, #40, #41, #43, #45, #46, #48, #49)
- **[proposal]**: my recommendation
- **[open]**: listed in section 7

### 5.1 Routes

- Every endpoint in section 1 becomes club-scoped. Following `spec/auth.md`: `/api/clubs/{slug}/pick`, `/api/clubs/{slug}/pick/history`, `/api/clubs/{slug}/pick/posts`, `/api/clubs/{slug}/pick/{pick_id}/posts`, `/api/clubs/{slug}/pick/posts/{post_id}`, …, `/api/clubs/{slug}/vote…`, `/api/clubs/{slug}/overlap`, `/api/clubs/{slug}/activity` [proposal].
- Every child id in a path (`pick_id`, `post_id`, `milestone_id`, `nomination_id`) must be resolved **within that club**. An id from another club gets the same 404 as a missing one, so nothing leaks [plan #40, #45].
- Non-members get **404, never 403**, on every one of these routes [plan #40]. The 404 detail string for an unknown or foreign club is shared across all club routes [open].
- A **home feed** across all of the user's clubs [plan #48] needs an unscoped endpoint, e.g. `GET /api/feed` [proposal]. Its items carry `club: {slug, name}`.

### 5.2 Club-scoped vs personal data

- **Club-scoped (carry `club_id` FK to `clubs`)** [plan #37]:
  - `club_picks`, `pick_milestones`, `club_pick_posts`, `post_reactions`
  - `next_up_votes`, `next_up_nominations`, `next_up_ballots`
  - club `events` (activity)
  - #37's AC says club-scoped tables carry `club_id`. I recommend putting it on the child tables too, with composite FKs such as `(club_id, pick_id) → club_picks(club_id, id)`, so a cross-club reference is impossible at the DB level [proposal].
- **Invariants as constraints** [proposal]:
  - `UNIQUE (club_id) WHERE ended_at IS NULL` on `club_picks` (one current pick)
  - the same on `next_up_votes` (one open vote)
  - keep `UNIQUE (vote_id, work_id)`, `UNIQUE (vote_id, user_id)` and `UNIQUE (post_id, user_id, emoji)`
- **Personal, read by this area through the visibility policy** [plan #46]:
  - `shelf` entries (statuses, progress, ratings, takes, DNF reasons)
  - `quotes`, for `quote_added` events
- **Audience:** members of any shared club, so co-members of *this* club are always in the audience. Ratings can still be "only me", so these must be redacted when they are not the viewer's:
  - the reader `rating` (and probably `take`, [open]) in `pick_out`
  - `shelf_finished` payloads in the feed
  - `quote_added` events for only-me quotes
- **Club membership replaces "all users"** in every computation that used `select(User)` or all shelf rows:
  - `readers`, `reading` and `finished`
  - `member_count` and `not_voted`
  - overlap candidates
  - suggestion candidates (want-to-read of current club members only)
  - push recipients (Phase 4)
- **`member_joined`** becomes a club event emitted on **joining a club**, not on account creation [proposal; agrees with `spec/auth.md`].

### 5.3 Roles and permissions

#40: "Admin: invites, picks, votes, events, and removing members. Member: everything else". Owner is a superset of admin.

| action | Python | proposed role |
|---|---|---|
| read the pick, history, threads, milestones, vote, suggestions, overlap, activity, ICS | any user | any member |
| `PUT`/`DELETE` pick | any user | admin+ [plan: "picks"] |
| create, edit and delete milestones | any user | admin+ [proposal; part of "picks"] |
| create pick post; react | any user | any member |
| edit own post | author | author (still a member) |
| delete own post | author | author; admins may also delete others' posts for moderation [open] |
| nominate, cast | any user | any member [proposal; "votes" for admins read as managing the vote] [open] |
| set or clear the deadline, apply the winner | any user | admin+ [plan: "votes"] |
| auto-close | system | system (unchanged) |

- A role denial is the first 403 in this area other than the post-ownership ones. It needs a new English `detail` string, plus an `apiErrors.ts` mapping and en/de keys [plan #48 "apiErrors.ts mapped to the new server's strings"].
- **Auto-close actor:** Python makes the winner's nominator the setter, and skips the close if the nominator is missing. Under clubs the nominator may have **left the club**; decide whether to still credit them, fall back to a system actor, or skip [open].

### 5.4 Works vs editions; Google removal; catalog writes

- **Picks and nominations point to works** [plan]. So `club_picks.book_id` and `next_up_nominations.book_id` become `work_id`. Readers match shelf entries by `work_id`, whatever edition each member owns. The point is that members can read different editions or translations [plan #45].
- **Request bodies** for `PUT pick` and `POST nominations` should take a **work id from our catalog**, plus `note` and `meeting_at` [proposal].
  - **Drop the client-supplied** `title`, `authors`, `cover_id`, `cover_url` and `year`. Python's `upsert_book` let any member overwrite a catalog record by setting a pick; under the plan, edits to existing records go to the admin queue [plan #41].
  - An unknown work id needs a new 404 or 400 string.
  - The `"ol_work_key must be an Open Library work key"` and `"title is required"` validations go away. These become reasoned changes in #49.
- **The book DTO changes:**
  - `ol_work_key` and `cover_id` become our work id and a `cover_url` served from our own domain [plan #43]. No `covers.openlibrary.org` URLs reach clients.
  - `pages` is edition-specific, so it is null or dropped at work level [proposal].
  - `spine_color` comes from a cover or edition [open].
- **Google Books is removed** [plan]:
  - the `cover_url` input on pick and nominate disappears
  - `ISBN…` and `GB…` books are re-matched by the importer [plan #38]
- **Suggestions** use `books.subjects` (Open Library subjects) and `books.ol_rating`. Whether the new catalog keeps OL subjects and ratings per work, and so whether the "Like past picks" and "Open Library rating" reasons survive, is [open]. DNB records carry GND and DDC subjects, but their licensing for this use is unverified.
- **ICS DESCRIPTION:** link to our own work page (`{public_url}/books/…`) instead of `https://openlibrary.org{key}`, which is wrong for any non-OL work [proposal]. Use `URL:` for the club page `{public_url}/c/{slug}` [proposal].

### 5.5 Milestones as percent or chapter [plan #37, #45]

- The new model is `kind ∈ {percent, chapter}` with a value or range. Spoiler flags on posts are already percent.
- **Legacy mapping by the importer** [plan #45]: Python milestones are chapter ranges (`chapter_from`, `chapter_to`, either nullable) with a title and an optional `due_at`.
  - `kind=chapter` with `from` and `to` copied
  - rows with both chapters null need a "title-only" representation (a nullable kind) [open]
  - `position`, `due_at` and `created_by` are copied
- **API shape** [open]: a range or a single "up to" value; whether `title` stays required; whether percent milestones are validated 0..100 with `to ≥ from`. The Python rule "backwards check on create only" should become a check on both create and edit [proposal].

### 5.6 Timezone

- `BOOKCLUB_TZ` was instance-wide. It becomes **per-club** (a `clubs.timezone` IANA column) or per-user [open; also `spec/auth.md` Q11]. Recommendation: per club, because meetings and deadlines are club events.
  - Validate on write against the tz database, and reject invalid names with 400. Python silently fell back to UTC, but that was config, not user input.
  - Keep the fallback to UTC for a corrupt stored value.
- The importer sets the imported club's timezone from the old instance's `BOOKCLUB_TZ` (CLI flag) [proposal].
- Every `timezone` response field becomes the club's timezone.
- **The home feed** spans clubs. Item labels use each item's club timezone, or the viewer's own [open]. Personal events in the home feed have no club [open].
- `test_meeting_uses_club_timezone` and `test_invalid_timezone_falls_back_to_utc` become club-setting tests. The env-based fallback test becomes "invalid timezone rejected on club update", which is a reasoned change.

### 5.7 Activity feed

- **Schema:** `events.club_id` is **nullable** [proposal].
  - **Club kinds** carry it: `member_joined` (per club), `pick_set`, `pick_cleared`, `note_posted`, `reaction`, `nominated`, `voted`, `vote_closed`, `milestone_added`.
  - **Personal kinds** have `club_id NULL`: `shelf_added`, `shelf_moved`, `shelf_finished`, `shelf_dnf`, `shelf_removed`, `progress`, `quote_added`.
- **Club feed** = the club's events plus the personal events of the club's **current members**, after visibility redaction (5.2) [proposal].
  - The alternative is to fan out one row per club at write time. It is simpler to read but freezes membership and visibility at write time [open].
- **Payload snapshots leak.** The payload stores `rating`, `take` and the quote `excerpt` at write time.
  - Under per-item "only me", reads must redact using the **current** visibility of the underlying row.
  - Python's `quote_added` payload has no `quote_id`, so add `quote_id` (and a `shelf_id` or work id) to new payloads so the row can be looked up [proposal].
  - Legacy events without those ids need a rule: hide them, or match on (actor, book) [open].
- **Pagination:** keep keyset pagination on `id` (`before`, `limit` 1..100, default 30, `has_more`). Postgres ids are global, so the same scheme works for the home feed.
  - The `username` filter becomes "member of this club". An unknown or non-member name returns an empty list, as in Python, so nothing leaks.
- **Activity for deleted accounts:** depends on the #39 decision (delete or anonymise). `"unknown"` is Python's fallback label.

### 5.8 Push and reminders

- **Push is dropped in Phase 1** and returns in Phase 4 [plan #38, #49].
- Drop the write side effect on `GET /api/pick` [proposal]. A background job (the server needs one anyway for the DNB harvester [plan #42]) can send meeting reminders later.
- Keep `club_picks.reminder_sent_at` so Phase 4 does not re-send reminders for imported picks [proposal].
- `notify_*` prefs stay personal; whether they become per club is [open] (`spec/auth.md` Q15).

### 5.9 Pick posts vs the diary; "events"

- #48 lists **"discussions"** among the club pages, which matches pick posts and reactions as club-scoped threads [plan #45 "pick posts with reactions"].
- **The diary stays personal** [plan: "Personal data … diary"].
  - `book_posts.pick_id` was a single-instance link to "the pick running when written". Under clubs it would need `(club, pick)` context, or it should be dropped [open].
- **Importer** [plan #38 "pick posts + reactions"]: production data holds each legacy pick post twice (3.3). Which copy wins needs deciding [open]:
  - import `club_pick_posts` and `post_reactions` as club discussion threads and **skip** the `book_posts` rows that have `legacy_post_id`, or
  - keep the diary copies, which may have newer edits, reactions or **replies** (diary replies can hang off a legacy-imported entry), and not import `club_pick_posts`
- **Meaning of "events"** in #37, #38 and #40 ("Admin: … events") and #48 ("events" page) [open]:
  - Python has no meetings table. A meeting is `club_picks.meeting_at`, one per pick, and the `events` table is the activity log.
  - #45's AC ("separate picks, votes and event lists") reads as the activity log.
  - "Admin: events" and an "events" page read as meetings.
  - For parity, keep `meeting_at` on the pick and map it 1:1 [proposal]. A separate meetings table can come later.

### 5.10 Importer notes for this area [plan #38]

- Everything becomes one club. Copy:
  - picks (`ended_at`, `meeting_at`, `reminder_sent_at`)
  - milestones (5.5)
  - posts and reactions (5.9)
  - votes, nominations and ballots, **including an open vote and its deadline**
  - events
- **Book references:**
  - `BC…` books become club-added works
  - Google `ISBN…` and `GB…` are re-matched or kept as bare-facts works [plan]
  - two legacy books that map to the **same work** can collide on `UNIQUE (vote_id, work_id)` for nominations, or merge two picks of "different" books [open]
- **Constraint conflicts:** several open picks or open votes (possible in SQLite) violate the new partial unique indexes. End all but the highest id at import time [proposal].
- **Event payloads:** `post_id` in payloads refers to old ids, and they may already be wrong because of rowid reuse (3.1). Remap them or drop the key [proposal].
- **Ordering:** event ids must keep their relative order because the feed is ordered by id. Insert events in ascending old id [proposal].
- **Timestamps:** every naive SQLite timestamp is UTC. Parse both `YYYY-MM-DD HH:MM:SS` and `…SS.ffffff`.
- Counts per entity go into the dry run [plan].

### 5.11 What no longer makes sense (reasons for the #49 checklist)

- **Instance-wide scope:** "all users" as the electorate, the readers and the overlap population.
- **Configuration that becomes club data:** the `BOOKCLUB_TZ` env and its fallback test, and `BOOKCLUB_NAME` in ICS and push copy (the club name now).
- **Catalog changes:** client-supplied book metadata and `upsert_book` mutating the catalog from pick and nominate, the `"/works/"` key validation, Google cover URLs on input, and the OL link in ICS for non-OL works.
- **Push:** the push side effects (Phase 4).
- **Tests that move:** `test_change_password`, `test_change_password_requires_auth` and `test_notification_prefs_default_on_and_patch` live in this file but belong to #39 and settings. Map them there.

### 5.12 New tests required by the plan

- [plan #45] **User in two clubs:** they see two separate picks, votes, milestones, threads and activity lists. Ids from club A used under club B's path return 404.
- [plan #40] **Non-member access:** a non-member gets 404 on every route in 5.1, including the ICS.
- [plan #40] **Roles:** every row of the role table in 5.3 has an allowed and a denied case.
- [plan #46] **Visibility redaction:** an only-me rating is hidden from co-members in `readers` and in `shelf_finished` payloads, and an only-me quote's `quote_added` event is hidden.
- [proposal] **Concurrency:** two concurrent first nominations create one vote; concurrent auto-close applies once; two concurrent `PUT pick` leave exactly one current pick.

---

## 6. Port risks

**Request semantics**

1. **Presence vs null.** Python uses `model_fields_set` in four places:
   - `PUT /api/pick` `meeting_at` (same-book branch): an omitted key keeps the meeting; `null` or `""` clears it
   - `PATCH` post `spoiler_upto`
   - `PATCH /api/vote` `closes_at`: `{}` clears it, because the default is null
   - `apply`'s internal `ClubPickSetIn`, which always counts as present

   Serde's `Option<T>` cannot tell the cases apart. Use `Option<Option<T>>` with `#[serde(default)]`, or a tri-state type.
2. **Same-book `PUT` resets `note` to `""` when omitted, and changes `set_by`.** It records no event and sends no push. `apply` and auto-close into the same book also clear the note and (for apply) the meeting.
3. **`"meeting_at must be a date and time"` is the error for `closes_at` and `due_at` too.** Copy it verbatim. Check order also matters:
   - the vote deadline: 404 before the parse
   - milestones: closed, then backwards, then parse
   - posts: closed before the milestone check
   - validation runs before the 404s

**Time handling**

4. **DST disambiguation.** Python's `fold=0` corresponds to jiff `Compatible`. A gap moves forward by the gap length; an ambiguous time takes the earlier instant. chrono-tz's `single()` returns None in both cases, and naively unwrapping it panics or rejects valid input. Use the test vectors in 2.1.
5. **The ISO parser's breadth.** Python accepts date-only (midnight local), hour-only, a space or lowercase `t` separator, a comma fraction, `±HHMM`, week dates and basic format.
   - Accept at least: date-only, `T`/`t`/space, optional seconds and fraction, `Z` or offsets with and without a colon.
   - Record dropping the exotic forms as a reasoned change.
6. **Label format.** Use `%a %d %b %Y, %H:%M %Z`: English names, a zero-padded day, a comma after the year, and tzdb abbreviations (`-03` style for zones without letters). `meeting_local` drops seconds. Do not localize these strings.
7. **Timestamp serialization differs by field in Python** (naive vs `Z`). Emit RFC 3339 `Z` everywhere; tests that use `startswith("2026-08-22T23:00:00")` keep passing. Microsecond precision: Postgres keeps microseconds, and Python prints them only when non-zero.

**Ordering and aggregation**

8. **Ordering details:**
   - readers: viewer first, then username, and `reading`/`finished` inherit that order
   - nominations: `(-votes, id)`, not creation time
   - leader ties: earliest `(created_at, id)` with strictly greater counts, and None when every count is 0
   - reactions: `(-count, emoji code point)`
   - posts: `(created_at, id)`
   - history: `(ended_at DESC, id DESC)`
   - overlap and suggestions: `(-score/-count, lower(title))`, where Python `str.lower()` is Unicode-aware, so use Rust `to_lowercase()`
   - milestones: `(position, id)`
   - Postgres `ORDER BY` collation differs from Python for text, so sort in Rust or use `COLLATE "C"`.
9. **Counts excluding missing users.** `votes`, `voters` and `voted_count` exclude ballots of missing users, but `_leader` counts them. This only matters after account deletion, so decide one rule.
10. **`not_voted` is `[]` when there is no open vote, not every member.** `member_count` is still filled in.

**State and side effects**

11. **Lazy auto-close runs only on GET vote, nominate and cast.** It commits **before** the rest of the request; a nominate can then fail with 400 after the close is already committed. Do not add it to `GET pick` without updating the tests' expectations. Under Postgres, lock the vote row (`SELECT … FOR UPDATE`) and re-check `ended_at IS NULL` and `closes_at <= now` inside the transaction.
12. **The "no ballots" auto-close path writes:** it clears `closes_at` and leaves the vote open. A port that only "skips" leaves an expired deadline that keeps re-triggering.
13. **Auto-close credits the winner's nominator** as `set_by` and as the actor of `pick_set` and `vote_closed`, with payload `{"votes": n, "auto": true}`. Manual apply has no `auto` key, and `auto: false` is never written.
14. **Event payloads drop None-valued keys,** so `{}` and `{"rating": 4}` are both valid `shelf_finished` payloads. They keep `false`, `0` and `""` unless the call site converts them: `take or None` and `reason or None` do convert empty strings.
15. **Events are never deleted.** A deleted post or reaction leaves its events behind, with dangling `post_id`s. Toggling a reaction on, off and on records two `reaction` events.
16. **`voted` is recorded only on the first ballot.** A switch records nothing. Use `INSERT … ON CONFLICT (vote_id, user_id) DO UPDATE … RETURNING (xmax = 0) AS inserted`, or an explicit select, to know whether to emit.
17. **Reaction toggle under concurrency.** Delete-or-insert must be atomic per `(post, user, emoji)`; Python's race gives a 500 on the unique constraint. Emit the event only when a row was inserted.
18. **The nomination limit of 6 is a count-then-insert.** Serialize per vote (lock the vote row) or the limit can be exceeded. Also check the current pick and the duplicate under the same lock.
19. **`can_post` and closed checks apply only to creating posts and milestones.** Edits, deletes, reactions, milestone edits and milestone deletes on ended picks must keep working (probed).
20. **Milestone `PATCH` is a full replacement** with no backwards check. Milestone delete detaches posts and renumbers positions densely. `position` on create is the current count, not max+1.
21. **`my_progress` mapping:** finished gives 100, currently reading gives `progress` (possibly null), and every other status gives null while `my_status` is still set.

**Validation details**

22. **Length rules count Unicode code points after `strip()`:** note 280, post 1000, milestone title 120. Python's `strip()` removes Unicode whitespace; Rust `str::trim()` is close (both use White_Space). Use `chars().count()`, not `len()`.
23. **The emoji whitelist is exact strings,** and ❤️ includes U+FE0F. Do not normalize or strip variation selectors; the value is only `strip()`-ed.
24. **Lax integer coercion.** `{"nomination_id": "1"}` is accepted by Python; serde rejects it with a different 400. That is fine as a reasoned change, but keep the status code 400 (not 422) via a custom JSON rejection handler.
25. **Huge path ids** give a Python 500. In Rust, `Path<i64>` overflow is a 400; treat it as 404 for non-leakage [proposal].

**Rendering**

26. **ICS bytes:**
    - CRLF line endings and a trailing CRLF
    - fold at 75 bytes for the first chunk, 74 for continuations, never splitting a UTF-8 sequence
    - escape order with backslash first, CR and LF handling, no escaping of colons
    - the EM DASH separator in DESCRIPTION
    - the fixed 90-minute duration
    - `UID:pick-{id}@bookclub`
    - `PRODID` uses the escaped club name
    - DESCRIPTION's public-URL line and `URL:` appear only when set

    Use the golden strings in 2.12 as unit tests.
27. **`upsert_book` mutation on pick and nominate.** If the port keeps any client-supplied metadata path, it must not overwrite catalog titles (#41 admin queue). If it drops it, record the change.

**Data**

28. **Partial unique indexes vs legacy data:** the importer must resolve multiple open picks or votes before inserting (5.10).
29. **Legacy rowid reuse** makes old `events.payload.post_id` unreliable; do not resolve them blindly.
30. **Suggestion score details:**
    - `3 * n` for n ≥ 2, otherwise 1
    - **all** matched subjects add to the score, but only 3 are listed
    - matched subjects keep their original case and order, duplicates included
    - the rating needs ≥ 4.0 and is formatted `{:.1}`
    - Python rounds ties half-to-even on the binary value (4.25 gives "4.2"), so check Rust's `format!("{:.1}")` against the same vector
    - ended picks only for `liked_subjects`, and want-to-read entries only for candidates

---

## 7. Open questions (not settled by plan.md or the issues)

1. **Which vote actions are admin-only?** #40 says admins handle "votes". Does that cover nominating and casting, or only setting the deadline and applying the winner?
2. **May admins delete or edit other members' pick posts** (moderation), and what 403 string do role denials use?
3. **What does "events" mean in #37, #38, #40 and #48?** The activity log, a new meetings feature, or both? Is a meeting still one optional `meeting_at` per pick?
4. **Is the club timezone a per-club setting, a per-user setting, or both?** How are cross-club home-feed labels computed?
5. **Activity storage:** a nullable `club_id`, with personal events joined in by current membership at read time, or fan-out at write time? How are legacy `quote_added` and `shelf_finished` payloads redacted when the rating or quote is "only me"?
6. **Does a new member's older personal activity show up** in the club feed?
7. **What is the milestone model under "percent or chapter"?** A range or a single "up to" value? How are legacy title-only milestones (no chapters) represented? Is the title still required?
8. **Pick posts vs diary:** which copy of each legacy pick post does the importer keep (`club_pick_posts` or the diverged `book_posts` copy with `legacy_post_id`), and what happens to diary replies attached to those copies? Does `book_posts.pick_id` survive in a multi-club world?
9. **Auto-close setter:** when the winning nominator has left the club, does the auto-close still credit them, use a system actor, or leave the vote open?
10. **Suggestions in the new catalog:** do works keep Open Library subjects and ratings? If not, drop the "Like past picks" and "Open Library rating" reasons, or replace them (e.g. with DNB subjects)?
11. **Reader redaction:** when a co-member's rating is "only me", is `take` hidden too? What about `dnf_reason` and `progress`?
12. **Book DTO at work level:** what replaces `ol_work_key`, `cover_id`, `pages` and `spine_color` for picks, nominations, overlap and suggestions?
13. **Importer collisions:** if two legacy books merge into one work (e.g. a Google `ISBN…` row re-matched to an existing `OL…W`), how are duplicate nominations in one vote, or picks that become "the same book", handled?
14. **Endpoints with no UI today** (posts, reactions, ICS, milestones, deadline, suggestions, activity): port all of them in Phase 1 for parity, or only what #48's pages use, with the rest reasoned in #49?
15. **Meeting reminders and vote auto-close:** stay lazy-on-read for parity, or move to the background job runner with the lazy path kept as a fallback?
