# Endpoint map: legacy Python API → Rust server

Every legacy endpoint is mapped to its new method and path, or marked as dropped. Every new endpoint that Phase 1 needs is listed too. Each endpoint carries its issue number and the **work package that owns it** (see `work-plan.md`):

| Package | Issues |
|---|---|
| `W1` | foundation (#36 #37 #39 #40 #46 core) |
| `CAT` | #41 |
| `HAR` | #42 |
| `MED` | #43 |
| `SHELF` | #44a |
| `POSTS` | #44b |
| `CLUB` | #45 |
| `IMP` | #38 |
| `OAPI` | #47 |

Phase 2 Wave 1 (`docs/phase2/work-plan.md`, notes in `docs/phase2/notes/`) added routes for two more packages, and changed behaviour without new routes for three:

| Package | Issues | Here |
|---|---|---|
| `ADM` | #54 | admin edits, the admin summary, queue paging and history, takedown notices (§4, §6) |
| `LEG` | #53 | `GET /api/legal/{doc}`, `POST /api/auth/terms`, terms on sign-up, `/api/config` fields (§1, §2) |
| `DEP` | #51 | the CLI commands `healthcheck` and `send-test-mail` (§10) |
| `GATE` | #55 | the SPA shell's CSP (§1) |
| `CUT` | #52, #38 | the importer's WAL-safe read (§10) |

Rules for every endpoint are in `conventions.md`:
- JSON error `{"detail": …}`.
- 400 for validation, with one detail string joined by `"; "`.
- 401 `Not signed in`.
- 404 for anything missing **or not visible**.
- Timestamps are RFC 3339 UTC with `Z`.
- Every response field is always present; nullable fields are `null`.

**Paths:**
- Personal endpoints keep their legacy paths.
- Club-scoped endpoints move under `/api/clubs/{slug}/…`.
- The catalog stays under `/api/books/…`.

**Path parameter names are fixed per position**, because axum/matchit rejects two routes that use different names at the same segment:

| Parameter | Used by |
|---|---|
| `{slug}` | club |
| `{work_id}` | work: a numeric id, `OL…W`, or a legacy key such as `BCdeadbeef01`, `ISBN9780316769488`, `GBabc` |
| `{edition_id}` | edition |
| `{entry_id}` | shelf entry |
| `{post_id}` | post |
| `{quote_id}` | quote |
| `{pick_id}` | pick |
| `{milestone_id}` | milestone |
| `{invite_id}` | invite |
| `{code}` | invite code |
| `{username}` | user |
| `{session_id}` | session |
| `{job_id}` | import job |
| `{public_id}` | media |
| `{variant}` | media variant |
| `{subject}` | subject |
| `{isbn}` | ISBN |
| `{request_id}` | edit request |
| `{report_id}` | report |
| `{doc}` | legal document: `imprint`, `privacy` or `terms` (LEG) |

**Each path string is owned by exactly one package.** Packages never register the same path twice: axum would panic on overlapping method routes after `merge`.

---

## 0. Shared DTOs (frozen in Wave 1; `W1` defines them, everyone uses them)

Their exact Rust definitions are in `crate::dto` (conventions.md §20); no package defines its own copy.

```text
BookOut {                       // replaces the legacy BookOut; one per work, edition-aware
  id: i64,                      // work id (legacy: books.id)
  ol_work_key: string|null,     // "/works/OL…W" for Open Library works, else null
  title: string, authors: string, year: i32|null,
  cover_url: string|null,       // same-origin "/media/{public_id}/l" (never openlibrary.org)
  cover_thumb_url: string|null, // "/media/{public_id}/s"
  cover_public_id: string|null, // the media public_id behind the cover URLs: the key for "Report cover"
  pages: i32|null, spine_color: string|null,   // with an edition context, the edition's title (if it has its own),
                                               // pages, cover and spine colour win
  source: "openlibrary"|"dnb"|"user"|"legacy",
  custom: bool                  // source == "user"; kept for the "Added by a reader" label
}
  Dropped: the legacy numeric cover_id (it made clients call covers.openlibrary.org).
  Without an edition context, year, pages and cover come from the work, whose empty values ingest fills
  from its editions (conventions.md §7.3), so DNB works and works whose only cover is an edition photo
  still show them.

EditionOut { id, work_id, isbn13|null, isbn10|null (derived from isbn13; null for 979), title|null, subtitle|null,
             publisher, language|null, format|null, pages|null, publication_year|null,
             planned_publication_date: "YYYY-MM-DD"|null, contributors: [{name, role: "translator"|"illustrator"|"editor"}],
             cover_url|null, cover_thumb_url|null, cover_public_id|null, source }

SearchHit { id, ol_work_key|null, title, authors, year, cover_url, cover_thumb_url, cover_public_id, isbn|null,
            source, custom, on_shelf, shelf_id, club_picks: ClubRef[],
            planned_publication_date: "YYYY-MM-DD"|null }   // set on new-releases hits (HAR), null elsewhere
SearchPage { items: SearchHit[], page, has_more }             // CAT (search, trending, subjects) and HAR (new-releases)

UserOut (self only) { id, username, email|null, email_verified: bool, pending_email|null, needs_email_prompt: bool,
                      needs_terms: bool,   // LEG (#53): the operator's terms are configured and not accepted in their current version
                      theme, color_mode, locale, timezone, avatar_url|null, is_admin: bool,
                      clubs: ClubSummary[] }
UserSummary (anyone else, and the owner block of a shelf) { id, username, avatar_url|null }
ClubRef { slug, name }
ClubSummary { slug, name, role: ClubRole, timezone, member_count }
ClubRole    = "member"|"admin"|"owner"   // also ClubOut.role and MemberOut.role
ShelfStatus = "want_to_read"|"currently_reading"|"finished"|"did_not_finish"
Visibility  = "audience"|"only_me"
ReactionOut { emoji, count, mine, users: string[] }   // users = only names the viewer may see; count = all
```

**`avatar_url`** is `/media/{public_id}/a`. It is public, unguessable and immutable, so it is cached for a year. The legacy `/api/members/{u}/avatar?v=` URL is gone; the frontend treats `avatar_url` as opaque.

---

## 1. Platform

| Legacy | New | Issue / owner | Changes |
|---|---|---|---|
| `GET /api/health` | `GET /api/health` | #36 / done | Already `{"status":"ok"}` / 503 `{"status":"unavailable"}` (reasoned change from `{"ok":true}`). |
| `GET /api/config` | `GET /api/config` | #39 / W1 | `{name, theme, theme_dark, public_url, signup: "open"\|"invite"}`. `name` = `SERVICE_NAME`; `theme` and `theme_dark` are the legacy defaults `#b44a2a` and `#8e361c` (no setting). `timezone` is **removed**: clubs carry their own, and users have `UserOut.timezone`. Public, no session, answers from memory. Schema `ServiceConfig`, operation `auth_get_config` (tag `auth`). **LEG (#53) adds** `legal: LegalDoc[]` (the documents `GET /api/legal/{doc}` serves; `[]` without texts), `terms_version: string\|null` (the current terms version a sign-up must send; null without terms) and `contact_email: string\|null` (`CONTACT_EMAIL`, the public contact and takedown address; null when unset in DEV_MODE). |
| – | `GET /api/legal/{doc}` | #53 / LEG | **Public.** `?locale=de\|en` (optional, default `en`; case and spaces ignored) → `LegalDocumentOut {doc: LegalDoc, locale: "de"\|"en", fallback: bool, updated: "YYYY-MM-DD"\|null, html: string}` with `LegalDoc = "imprint"\|"privacy"\|"terms"`. `html` is the operator's fragment from `LEGAL_DIR` (or the DEV_MODE placeholder); `fallback` is true when the other language was served because the asked one is missing; `updated` comes from `manifest.json`. 404 `Not found` for an unknown `doc` (exact lowercase names only) or a document the operator has not provided in either language; 400 `locale must be one of: en, de`. `Cache-Control: no-store` like every API answer. Operation `legal_get_document`, tag `legal` (conventions.md §5.9). |
| `GET /manifest.webmanifest` | **dropped** | #48 | `frontend/public/manifest.webmanifest` is served statically, and branding is fixed at build time. |
| `GET /api/docs`, `GET /api/openapi.json` (debug only) | `GET /api/openapi.json` (**public**, always) | #47 / W1 wires it, OAPI checks it | It is the API contract; it contains no secrets. No docs UI is served. |
| SPA catch-all `GET /{path}` | same, when `STATIC_DIR` is set | #48 / W1 | `index.html` and every client route get `Cache-Control: no-store`. `/assets/*` gets `public, max-age=31536000, immutable`, and a miss is 404 JSON, never the shell. `/api/*` and `/media/*` never fall through to the shell. GET and HEAD are answered. **GATE (#55):** every answer of the SPA fallback carries `Content-Security-Policy: <spa::SHELL_CSP>` (scripts, styles, pictures, fonts, fetches and the manifest from the own origin only; `data:` and `blob:` pictures, a `blob:` manifest, inline styles; no plugins, no framing; conventions.md §3 "Fallbacks"). Every other answer keeps `frame-ancestors 'none'`. The SPA's public legal pages are `/imprint`, `/privacy` and `/terms` (LEG), with the redirects `/impressum`, `/datenschutz` and `/nutzungsbedingungen`; the fallback serves them like any client route. |
| `GET /api/backup` | **dropped** → `GET /api/auth/me/export` | #39 / W1 | Personal DSGVO export instead of the whole database. Instance backups become server-side `pg_dump` (#51). |
| `GET /api/push/vapid`, `GET/POST/DELETE /api/push/subscriptions`, `POST /api/push/test` | **dropped** | Phase 4 (#60) | Push subscriptions are not imported. |
| (unknown `/api/*`) | 404 `{"detail":"Not found"}` | W1 | |
| (wrong method) | 405 `{"detail":"Method Not Allowed"}` | W1 | |

## 2. Accounts and auth (#39, owner W1 unless noted)

| Legacy | New | Auth | Request → Response | Errors and changes |
|---|---|---|---|---|
| `POST /api/auth/register` | `POST /api/auth/register` | none | `{username, email, password, invite_code?: string, timezone?: string, locale?: "en"\|"de", terms_version?: string\|null}` → **201** `UserOut`, sets the cookie, sends the verification mail | `Username must be 2–32 characters: lowercase letters, numbers, underscore` (EN DASH), `Password must be at least 8 characters`, `Enter a valid email address`, `That is not a valid time zone` (400; an old zone name such as `Asia/Calcutta` is accepted and stored as the current one, conventions.md §12); 409 `That username is already taken` (checked first). **No 409 for the email:** the answer never says whether an address is in use. If it belongs to a verified account, no verification link is sent; that address gets the "someone tried to use your address" notice instead (conventions.md §5.5). 400 `That invite code is not valid`; 400 `Sign-up needs an invite code right now` (SIGNUP_MODE=invite with no code); 409 `This club is full`; 429 `Too many attempts. Wait a few minutes.` (5 sign-ups per IP per hour). A club invite also creates the membership and a `member_joined` event; a sign-up invite (club_id NULL) only unlocks sign-up. Calling it while signed in switches the session, as in the legacy app. **LEG (#53):** when the operator's terms are configured, `terms_version` must be their current version (`/api/config.terms_version`), else 400 `The terms of use have changed. Reload the page and try again.` (checked after the 409 for the username and before the invite checks; it counts against the sign-up limit like every validated attempt); the account stores the version with `terms_accepted_at`. Without terms a given version is ignored. |
| `POST /api/auth/login` | `POST /api/auth/login` | none | `{username, password}`; `username` may also be the email (it contains `@`) → **204**, sets the cookie | 401 `Wrong username or password`; 429 `Too many attempts. Wait a few minutes.` (10 failures per identifier per 600 s, sliding, cleared on success: a username, a verified address and an identifier that names no account each have a bucket of their own, so a 429 never tells a used address from a free one, conventions.md §5.4; plus 30 attempts per IP per 600 s). Unknown accounts run a dummy argon2 verify (constant time). Legacy hashes with old parameters are re-hashed on success. |
| `POST /api/auth/logout` | `POST /api/auth/logout` | none | → **204**; deletes the server session and clears the cookie (only if a session existed) | A request with a bearer token deletes that token's session instead (no cookie). |
| `GET /api/auth/me` | `GET /api/auth/me` | user | → `UserOut` | New fields: `email`, `email_verified`, `pending_email`, `needs_email_prompt`, `needs_terms` (LEG #53), `timezone`, `is_admin`, `clubs`. `clubs` saves the club switcher a request. Built by `auth::user_out` (frozen; MED's avatar endpoints reuse it). |
| `PATCH /api/auth/me/preferences` | same | user | `{theme?, color_mode?, locale?, timezone?}` (null or absent = unchanged; a missing body is 400 `Field required`) → `UserOut` | Legacy allow-list messages are kept verbatim, plus `That is not a valid time zone`. An old zone name (`Asia/Calcutta`) is stored as the current one (`Asia/Kolkata`, conventions.md §12). |
| `PATCH /api/auth/password` | same | user | `{current_password, new_password}` → **204**; a cookie session gets a new cookie | 429 `Too many attempts. Wait a few minutes.` (the current-password limits: 10 checks per session in ten minutes and 30 per account an hour, right or wrong, never cleared; not the login's bucket, conventions.md §5.4); 400 `Current password is wrong`; 400 `Pick a different password`; 400 `New password must be at least 8 characters`; 503 `The server is busy. Try again in a moment.` (no password permit within 10 s). **New: deletes every other session and bearer token of the user, and marks every open reset link used, and on a verified account the open verification link of a pending address change (conventions.md §5.5). A cookie session is replaced by a new one with the same label (`Set-Cookie`), so a copy of the old cookie stops working; a bearer token keeps its secret.** |
| `GET /api/auth/notifications` | same | user | → `{notify_meeting, notify_pick, notify_note}` | Unchanged. Kept for Phase 4. |
| `PATCH /api/auth/notifications` | same | user | `{notify_meeting?, notify_pick?, notify_note?}` → same | Strict JSON booleans (reasoned change from pydantic's lax bools). |
| `PUT /api/auth/me/avatar` | same | user | multipart `file` → `UserOut` | **Owner MED (#43).** 429 `Slow down a little. Try again later.` (10 per account an hour, or another picture upload of the account still running; both checked before the body is read; security review 2026-10-28 #4). Same strings and limits: 413/400 `That picture is too large (50 MB max)`, `Use a JPEG, PNG, WebP, or GIF`, `That file is not a picture`, `That picture has too many pixels`. The output is a 256×256 **WebP**, EXIF stripped, in the media store. The previous avatar's media row is deleted in the same transaction and its bytes after commit. |
| `DELETE /api/auth/me/avatar` | same | user | → `UserOut` | **Owner MED.** Deletes the media row and its bytes. |
| `GET /api/members/{username}/avatar` | **dropped** | – | – | `avatar_url` now points at `/media/{public_id}/a`. The 404 string `No profile picture` goes away. |
| – | `POST /api/auth/verify-email` | none | `{token}` → **204** | 400 `That link has expired or was already used` (also for a token that is not the user's newest verification link, or whose address is no longer the pending one); 409 `That email is already in use` (another account verified the same address first; only a link holder can see this). Sets `users.email` to the token's address and `email_verified_at`, and marks every other open verification and reset link of the user used. Does not log anyone in; the link (`/verify-email#token=…`) works on any device. |
| – | `POST /api/auth/verify-email/preview` | none | `{token}` → `{username, email}` (`VerifyEmailPreview`) | 400 `That link has expired or was already used` for every token verify would refuse. Read-only: the verify page names the account and the address and verifies only on the reader's tap, so a link scanner or a curious click verifies nothing (security review 2026-10-28 #8). |
| – | `POST /api/auth/verify-email/resend` | user | → **204** | 400 `Add an email address first`; 429 `Slow down a little. Try again later.` (3 per hour per account, shared with `PUT /me/email`). The new link replaces the previous one. It renews the pending address: the account's own unverified address, or the address of an open email change. A verified account with no change waiting gets 204 and nothing is sent or counted. No link goes to an address a verified account holds (conventions.md §5.5). The answer comes before the mail, which goes out in the background, so its timing does not tell whether the address is held either. |
| – | `PUT /api/auth/me/email` | user | `{email, current_password}` → **204** | 400 `Current password is wrong` (counts against the current-password limits, 429 `Too many attempts. Wait a few minutes.`), `Enter a valid email address`; 429 `Slow down a little. Try again later.` (3 per hour per account, shared with resend, checked before the password and counted after it; plus a silent per-address mail limit, conventions.md §10.1). **No 409**: the answer is the same whether or not the address is in use (conventions.md §5.5). With no current email, or an unverified one, the address is replaced at once and a link is sent. A verified address stays until the new one is verified (`UserOut.pending_email` shows the pending one). Marks every earlier open verification link and every open reset link used. The old verified address gets an "email changed" notice, and `pending_email` shows the new address, in both cases. The account's own verified address cancels a pending change and sends nothing. Imported users add their email here. |
| – | `POST /api/auth/me/email-prompt/dismiss` | user | → **204** | Sets `email_prompt_dismissed_at`. |
| – | `POST /api/auth/terms` | user | `{terms_version}` (`TermsIn`) → `UserOut` (`needs_terms` false) | **Owner LEG (#53).** Accepts the operator's current terms (the SPA's `TermsPrompt`). 400 `The terms of use have changed. Reload the page and try again.` (not the current version, or no terms configured); 400 `Field required`. Accepting the version already accepted keeps the first time. Operation `auth_accept_terms`. |
| – | `POST /api/auth/password-reset` | none | `{email}` → **204, always** (no account enumeration) | 429 `Too many attempts. Wait a few minutes.` (5 per IP per hour, 3 per account per hour; the account limit is silent). Silently, nothing is sent while the account has an open reset link, and a link whose mail was not sent is voided (conventions.md §5.5). A link (`/reset-password#token=…`, valid 1 h) goes only to a **verified** address. The answer comes before the lookup and the mail, which run in the background, so its timing tells nothing either. |
| – | `POST /api/auth/password-reset/confirm` | none | `{token, new_password}` → **204** | 400 `That link has expired or was already used` (also when the token's address is no longer the account's verified address), `New password must be at least 8 characters`. Sets the password, deletes **all** sessions and tokens, and marks every other open reset link and the verification link of a pending address change used; the user then logs in. Also clears the account's login-failure limits (by name and by address). |
| – | `POST /api/auth/tokens` | none | `{username, password, label}` → **201** `{token: "bc_…", session: SessionOut}` | Bearer token for the apps (Phase 4 uses it; tested in Phase 1). Same rate limits and errors as login. `label` is required and cut to 200 characters; `session.current` is false (this request did not use the token). |
| – | `GET /api/auth/sessions` | user | → `{items: SessionOut[]}` with `SessionOut {id, kind: "cookie"\|"bearer", label, created_at, last_used_at, current: bool}` | Live sessions only, most recently used first. `label` is the User-Agent for cookie sessions. Schemas: `SessionOut`, `SessionList`, `SessionKind`, `TokenIn`, `TokenOut`. |
| – | `DELETE /api/auth/sessions/{session_id}` | user | → **204** | 404 `No such session` for another user's session. Deleting the current session logs out. |
| – | `DELETE /api/auth/me` | user | `{password}` → **204**, clears the cookie | 400 `Current password is wrong` (counts against the current-password limits; 429 `Too many attempts. Wait a few minutes.`); 409 `Transfer or delete your clubs first` (the user owns a club with other members). Solo clubs the user owns are deleted with the account. What happens to the data is in conventions.md §5.6. |
| – | `GET /api/auth/me/export` | user | → `application/zip` attachment `bookclub-export-<username>-<YYYYMMDD>.zip` (today in the user's time zone; `Cache-Control: no-store`) with `data.json` and `media/…` | Every personal table (conventions.md §5.7). `profile.pending_email` is null unless the account's own address is verified, like `UserOut`. Rows of a club the user no longer belongs to keep only the user's own facts (no other members, no pick note or meeting, no invite code or counters). The ZIP is streamed, one picture at a time, so the response has no `Content-Length`. Rate limit: 5 per hour, 429 `Slow down a little. Try again later.`; the same 429 while another export of the account runs or two run on the instance. A client that takes no chunk for 60 s, or a download still running after an hour, ends the export (the archive then lacks its central directory; security review 2026-10-28 #6). |
| `GET /api/auth/me/favorites` | same | user | → `{items: [{position, book: BookOut}]}` | **Owner SHELF (#44).** |
| `PUT /api/auth/me/favorites` | same | user | `{work_ids: i64[]}` (legacy `book_ids`) → same | **Owner SHELF.** `At most 3 favourites`, `Each book can only be a favourite once`, and **`That book is not in the catalog`** (renamed from `That book is not in the club yet`; `apiErrors.ts` is updated in #48). |
| `POST /api/auth/me/favorites/{work_id}` | same (numeric work id) | user | → same | **Owner SHELF.** 409 `You already have 3 favourites`; 400 `That book is not in the catalog`. |
| `DELETE /api/auth/me/favorites/{work_id}` | same | user | → same | **Owner SHELF.** |

## 3. Clubs, membership, roles, invites (#40, owner W1)

Membership is checked on every `/api/clubs/{slug}/…` route **before** anything else:
- A non-member, or a slug that does not exist, gets **404 `No such club`**.
- Role checks come next: 403 `Only club admins can do that` or `Only the club owner can do that`.

Operation ids (tag `clubs`): `clubs_list`, `clubs_create`, `clubs_get`, `clubs_update`, `clubs_delete`, `clubs_list_members`, `clubs_change_role`, `clubs_remove_member`, `clubs_leave`, `clubs_transfer`, `clubs_list_invites`, `clubs_create_invite`, `clubs_revoke_invite`, `clubs_preview_invite`, `clubs_accept_invite`; the sign-up invites are `admin_list_invites` and `admin_create_invite` (tag `admin`).

| Legacy | New | Role | Request → Response | Errors and changes |
|---|---|---|---|---|
| – | `GET /api/clubs` | user | → `{items: ClubOut[]}`, newest join first | `ClubOut {slug, name, timezone, role, member_count, created_at, current_pick: ClubPickSummary\|null}` with `ClubPickSummary {id, book: BookOut, meeting_at\|null}` (the book without an edition context). The home page renders its per-club cards from this. Schema `ClubList`. |
| – | `POST /api/clubs` | verified email | `{name, timezone?}` → **201** `ClubOut` | 403 `Verify your email address first`; 400 `Give the club a name`, `Keep the club name to 80 characters`, `That is not a valid time zone` (all messages, joined by `; `); then 409 `You can create at most 10 clubs` (clubs the user created that still exist, counted under the user-row lock). The name is trimmed and inner whitespace runs collapse to one space. `timezone` defaults to the creator's; an old zone name is stored as the current one (conventions.md §12). The slug is generated and immutable: the lowercased name with `ä ö ü ß` as `ae oe ue ss`, other accents dropped and every other run of characters one `-`, cut to 50 characters (`club` when nothing is left), plus `-` and 4 random `[a-z0-9]`. Creating records no event. Schemas `ClubIn`, `ClubOut`. |
| – | `GET /api/clubs/{slug}` | member | → `ClubOut` | |
| – | `PATCH /api/clubs/{slug}` | admin | `{name?, timezone?}` → `ClubOut` | `null` or absent leaves a value; `{}` is a no-op. Same 400 strings as create. The slug never changes. Schema `ClubPatchIn`. |
| – | `DELETE /api/clubs/{slug}` | owner | → **204** | Cascades every club row (memberships, removals, invites, picks, posts, votes, events). Runs under the club-row lock. |
| `GET /api/members` | `GET /api/clubs/{slug}/members` | member | → `{items: MemberOut[]}` | `MemberOut {username, role, joined_at, you: bool, avatar_url, currently_reading_count, currently_reading_preview: BookOut[] (≤ 3, ordered by position, id)}`. **Includes the caller** (`you: true`). Ordered owner, admins, members, then username (byte order). One query with `row_number()`, not N+1, reading the shelves through `bc_visible_shelf` restricted to the club's members; a preview book has the shelf entry's edition as its context. Schema `MemberList`. |
| – | `PATCH /api/clubs/{slug}/members/{username}` | owner | `{role: "admin"\|"member"}` → `MemberOut` | 404 `No member with that name` (unknown name or not a member; the name is stripped and lowercased); 400 `Use transfer to change the owner` (for `"owner"`, and for the owner as the target); any other role is 400 `Invalid value`. Schema `RoleIn`. A demotion revokes nothing: the person's invites stay the club's, and the other admins see and can revoke them. Demoting an admin records the time (`club_memberships.admin_until`), which a later removal uses. |
| – | `DELETE /api/clubs/{slug}/members/{username}` | admin | → **204** | Admins can remove members; the owner can also remove admins. Nobody can remove the owner (403 `Only the club owner can do that`, also for an admin removing an admin), and nobody can remove themselves: 400 `Use leave to leave the club`. 404 `No member with that name`. Runs under the club-row lock with both roles re-read and the guard in the `DELETE` itself (conventions.md §5.3). Records a `club_removals` row: invites created before the removal no longer work for that person; a new invite does. **Revokes the club's active invites the person knows**, so a second account cannot use them either: the ones they created or redeemed, every one when the target is an admin (admins list them all), and, for a member who was demoted from admin, every one created up to that demotion (security review 2026-10-28 #9). Admins share a fresh link afterwards. Links forwarded to a member by others are not known to the server; admins revoke them by hand. |
| – | `POST /api/clubs/{slug}/leave` | member | → **204** | 409 `Transfer ownership before leaving` (the owner, including a sole-member owner, who should delete the club instead). Same lock and in-statement guard. |
| – | `POST /api/clubs/{slug}/transfer` | owner | `{username}` → `ClubOut` | 404 `No member with that name` (the demotion is rolled back). The old owner becomes admin; both updates run in one transaction under the club-row lock, demote first. The answer is the club as the caller now sees it (`role: "admin"`); a transfer to oneself changes nothing. Schema `TransferIn`. |
| `GET /api/invites` | `GET /api/clubs/{slug}/invites` | admin | → `{items: InviteOut[]}`, newest first (`created_at DESC, id DESC`) | Admins see **all** of the club's invites. `InviteOut {id, code, url, created_by\|null, created_at, expires_at\|null, max_uses\|null, use_count, used_by: string[], revoked: bool, active: bool}`. `url` = `{PUBLIC_URL}/join/{code}`. `created_by` is the creator's username (null when the account is gone or the CLI minted it); `used_by` has one username per use, oldest first (a member who left and came back appears twice; deleted accounts are left out); `active` = not revoked, not expired, not used up. Schemas `InviteList`, `InviteOut`. |
| `POST /api/invites` | `POST /api/clubs/{slug}/invites` | admin | `{kind: "single_use"\|"link", expires_in_days?: 1..30}` → **201** `InviteOut` | Defaults: `single_use` → max_uses 1, expires in 30 days; `link` → no use limit, expires in 7 days. Codes are 10 Crockford base32 characters, as before. `kind` is required. Runs under the club-row lock with the caller's role re-read (conventions.md §5.3): a caller removed meanwhile gets 404 `No such club`, one demoted 403 `Only club admins can do that`. Schemas `InviteIn`, `InviteKind`. |
| – | `DELETE /api/clubs/{slug}/invites/{invite_id}` | admin | → **204** | Sets `revoked_at` (revoking twice is harmless). 404 `No such invite` (also another club's invite). Runs under the club-row lock with the caller's role re-read, like the create. |
| – | `GET /api/invites/{code}` | none | → `{club: ClubRef\|null, kind: "club"\|"signup"}` | Join-page preview. 404 `That invite code is not valid` (unknown, revoked, expired or used up: one answer for all). Rate limit: 60 per IP per hour, 429 `Slow down a little. Try again later.` The code is normalised like at redemption. Uses nothing. Schemas `InvitePreview`, `InviteTarget`. |
| – | `POST /api/invites/{code}/accept` | user | → **200** `ClubOut` | 400 `That invite code is not valid`; 409 `This club is full`; 409 `You are already in this club` (the invite is **not** consumed); 429 `Slow down a little. Try again later.` (20 attempts per account per hour, valid code or not, checked before the transaction: the preview's per-IP limit is no guard against guessing otherwise). The atomic consume (`… AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now()) AND (max_uses IS NULL OR use_count < max_uses) RETURNING`), the 20-member check under a lock on the club row, the membership insert, a `club_invite_uses` row (a log, not unique: someone who left may use the same link again) and the `member_joined` event all happen in one transaction. A sign-up-only invite gives 400 `That invite code is not valid`, and so does an invite created before the caller was removed from that club (`club_removals`); a successful join deletes the removal row. |
| – | `POST /api/admin/invites` | instance admin | `{max_uses?: 1..100, expires_in_days?: 1..90}` → **201** `InviteOut` (club_id NULL) | Closed-beta sign-up invites for people who want no club. An absent field means no limit of that kind; with neither (`{}`), the invite is single-use and expires in 30 days. Schema `SignupInviteIn`. The CLI equivalent is `bookclub-server beta-invite [--uses N] [--days D]` (same defaults; it prints the code and the join link). Every `/api/admin/*` route answers 404 `Not found` to non-admins: one route layer guards them all (conventions.md §2), and a generic test walks them (§17.7). |
| – | `GET /api/admin/invites` | instance admin | → `{items: InviteOut[]}` | Every sign-up invite (club_id NULL), newest first. |

## 4. Catalog (#41, owner CAT unless noted)

Every `/api/books/*` route requires a signed-in user (parity: every legacy books route had `get_current_user`). Search, trending, subjects, the import of an unknown `OL…W` and a stub's first view may call Open Library, whose 3 requests a second every user shares: they count against `catalog_user` (60 per account a minute, 429 `Slow down a little. Try again later.`; a first view over it serves the stub without enriching it) and run two at a time per account, further ones waiting for a turn (`catalog::upstream_turn`, conventions.md §10.1; security review 2026-10-28 #5). `page` of search and subjects is 1 to 100 (400 `Input should be less than or equal to 100`). Writes to catalog rows go through `ingest` with field-level provenance (conventions.md §7.2): a source never overwrites what a reader entered or an admin approved.

| Legacy | New | Request → Response | Changes |
|---|---|---|---|
| `GET /api/books/search` | same | `q, subject, sort, page, limit` (same rules and strings) → `SearchPage` (§0) | **SearchHit** (§0). Dropped: `cover_id`, `cover_edition_key`, `club_pick` (bool). `club_picks` lists the viewer's clubs whose **current** pick is this work (clubs from `bc_member_clubs`). Merged-away works never appear. After persisting hits, CAT queues their Open Library covers with `MediaService::prefetch`. **Every remote hit is persisted** as a stub work (one batched insert-only upsert, never overwriting), so every hit has an `id`. Local hits (Postgres FTS plus trigram, page 1 only) come first. Remote hits are merged without duplicates (by work id, ISBN or OL id) and **capped at `limit`** (the legacy `limit + 1` quirk is fixed). An ISBN-shaped query goes through the ISBN lookup chain. A failing fallback query no longer fails the search when another one succeeds. Status codes are unchanged (400 short query / 429 / 502), because Discover branches on them. |
| `GET /api/books/trending` | same | `limit` → `SearchPage` | Live from Open Library (cached for 18 h), hits persisted, as above. |
| `GET /api/books/subjects/{subject}` | same | `page, limit` → `SearchPage` | Same. Subject browse keeps OL `lang=en` (parity); German users also get the DNB new-releases row (§5). |
| `POST /api/books/custom` | same | `{title, authors?, year?, description?, isbn?, publisher?, pages?, language?}` → **201** `BookDetailOut` | Creates a **user-added work** (`source='user'`, `created_by`, every entered field with provenance `user`), plus an edition when ISBN or edition facts are given. It is searchable by everyone right away; abuse is handled through reports. **With an ISBN, the remote lookup runs first** (`CatalogLookup::resolve_isbn_remote`, after the local check): a local or remote hit is 409 `That ISBN is already in the catalog` (body includes `work_id` and `edition_id`), so a typo can never claim a real ISBN that DNB or Open Library knows; when the lookup cannot answer, 429 `The library is busy. Wait a few seconds and try again.` or 502 `Could not look that ISBN up right now. Try again.`. Rate limit: 30 per day. |
| `GET /api/books/works/{work_id}` | same | → `BookDetailOut` | `{work_id}` may be a numeric id, `OL…W` (imported on first view, as before) or a legacy key (`BC…`, `ISBN…`, `GB…`, resolved through `legacy_work_keys`). The frontend replaces the URL with `/book/{id}`. A merged work answers with its target's id. **BookDetailOut** `{id, ol_work_key, title, subtitle, authors, year, cover_url, cover_thumb_url, cover_public_id, description, subjects[], pages, ol_rating, ol_rating_count, source, custom, editions_count, on_shelf, shelf_id, edition_id, rating, rating_visibility, take, dnf_reason, progress, readers: BookReader[], club_rating, rating_count, quote_count, favorite_position, club_picks: ClubRef[], can_refresh}`. `readers`, `club_rating`, `rating_count` and `quote_count` cover only the viewer's **audience** (plus the viewer for the rating mean and for their own quotes); they read through `bc_visible_shelf` and `bc_visible_quotes`, so other users' `only_me` ratings **and `only_me` quotes** are excluded. **A stub OL work is enriched once on its first detail view** (one outbound fetch; on failure the stub is served with 200). **A DNB work gets one Open Library cover check by ISBN on its first detail view** (conventions.md §7.4). Known works never go outbound again (local-first). |
| `POST /api/books/works/{work_id}/refresh` | same | → `BookDetailOut` | Any signed-in user, OL works only, 10 per hour per user. `Mode::Refresh` under the provenance rules: it never overwrites `user` or `admin` fields (including a user's or an admin-approved cover), and fields absent upstream keep their value. 400 **`This book was added by a reader, not Open Library.`** (replaces the "club" string; the same string for `dnb`/`legacy` sources). Legacy bare-facts works are upgraded by `rematch-legacy`, not by refresh. |
| `GET /api/books/work/{work_id}` | **dropped** | – | Merged into `BookDetailOut`, which now carries `pages`, `ol_rating`, `ol_rating_count` and `quote_count`. #49 records the reason. |
| `POST /api/books/work/{work_id}/refresh` | **dropped** | – | Same. |
| `GET /api/books/isbn/{isbn}` | same | → `IsbnHitOut {isbn (normalised ISBN-13), work_id, edition_id, ol_work_key, title, authors, year, pages, cover_url, cover_thumb_url, cover_public_id, on_shelf, shelf_id}` | The chain is local editions, then by registration group: `978-3` → DNB SRU → Open Library; every other group → Open Library only (conventions.md §7.4). The hit is **persisted** (edition plus work). ISBN-10 input is converted to ISBN-13, and checksums are validated: 400 `That is not a valid ISBN-10 or ISBN-13`; 404 `No book found for that ISBN`; 502 `Could not look that ISBN up right now. Try again.`; 429 when every source is throttled. |
| – | `GET /api/books/works/{work_id}/editions` | → `{items: EditionOut[]}` | |
| – | `POST /api/books/works/{work_id}/editions` | `{isbn?, title?, subtitle?, publisher?, language?, format?, pages?, publication_year?}` → **201** `EditionOut` | Adds a missing edition (for example after a failed scan), fields with provenance `user`. With an ISBN, the local and remote lookup runs first exactly as for `POST /custom`: a hit is 409 `That ISBN is already in the catalog` (with `work_id`, `edition_id`); 429/502 when the lookup cannot answer. Rate limit: shared with custom books. |
| – | `GET /api/books/editions/{edition_id}` | → `EditionOut` | |
| – | `POST /api/books/works/{work_id}/edit-requests` | `{fields: {title?, subtitle?, authors?, year?, description?, subjects?, pages?}, note?}` → **201** `{id, status}` | Edits to existing records go to the admin queue. 50 per day. |
| – | `POST /api/books/editions/{edition_id}/edit-requests` | `{fields: {…edition fields}, note?}` → **201** `{id, status}` | Same limit. |
| – | `POST /api/books/works/{work_id}/merge-requests` | `{into_work_id, note?}` → **201** `{id, status}` | "These two are the same book" (a DNB and an OL record of one work, a legacy bare-facts work and its real record). Creates a `work_merge` request. 400 `A book cannot be merged into itself`; 404 `No such book.` for either id. Shares the edit-request limit. |
| – | `GET /api/admin/edit-requests` | **instance admin**, `?status=open\|resolved&page=n` → `EditRequestList {items: EditRequestOut[], page, has_more}` | The admin queue (#54, ADM). `status` defaults to `open` (oldest first, `created_at, id`); `resolved` lists `approved`/`rejected` requests newest decision first (`decided_at DESC, id DESC`). 50 a page (`admin::queue::PAGE_SIZE`); `page` is 1 to 1000, default 1; a page past the end is `{items: [], page, has_more: false}`. 400 `Input should be 'open' or 'resolved'`, `Input should be greater than or equal to 1`, `Input should be less than or equal to 1000`, `Input should be a valid integer, unable to parse string as an integer`. **`EditRequestOut.current: object`** (always present): for `work_edit` and `edition_edit`, the record's value now for each key of `payload`, under the same key (empty text is `null`, subjects an array, `planned_publication_date` `YYYY-MM-DD`); a `work_edit` reads the work its `work_id` was merged into. `{}` for `cover_replace`, `work_merge`, a record that is gone, and keys the server does not know. |
| – | `POST /api/admin/edit-requests/{request_id}/approve` | **instance admin** → `EditRequestOut` | Applies the fields through `ingest::apply_admin_fields` (provenance `admin`, so no source overwrites them). `cover_replace` goes through `ingest::set_cover(…, Admin)`. `work_merge` runs `catalog::merge_works` (conventions.md §7.6: shelf, favourite, nomination and layout collisions resolved, every reference re-pointed, `merged_into_id` set). |
| – | `POST /api/admin/edit-requests/{request_id}/reject` | **instance admin**, `{note?}` → `EditRequestOut` | |
| – | `POST /api/admin/works/{work_id}/merge` | **instance admin**, `{into_work_id}` → `BookDetailOut` (of the target) | Direct merge without a request, same `merge_works`. |
| – | `PATCH /api/admin/works/{work_id}` | **instance admin**, `AdminEditIn {fields}` → `BookDetailOut` (the work as it is now, seen by the admin) | **Owner ADM (#54).** The direct admin edit ("Edit as admin"): `fields` has the keys, value rules and null-clears of `EditRequestIn.fields` and goes through `ingest::apply_admin_fields` in one transaction, so every written field gets provenance `admin` (a cleared field stays empty, conventions.md §7.2). `{work_id}` takes a work id, `OL…W` or a legacy key and follows merges. 400 `Field required` (no or empty `fields`), `Unknown field: <key>`, `Invalid value for <key>` (byte for byte the approval's); 404 `No such book.`. Files no request and leaves no record. Operation `catalog_admin_edit_work`, tag `admin`. |
| – | `PATCH /api/admin/editions/{edition_id}` | **instance admin**, `AdminEditIn {fields}` → `EditionOut` | **Owner ADM.** The same for an edition's fields. Operation `catalog_admin_edit_edition`. |

## 5. DNB harvest (#42, owner HAR)

| New | Response | Notes |
|---|---|---|
| `GET /api/books/new-releases?language=ger&page=1&limit=24` | `SearchPage` (`has_more` set; no remote calls) | Upcoming and recent titles, local only: editions with `planned_publication_date` between today − 30 days and today + 120 days, `withdrawn_at` NULL (a deleted DNB record withdraws all its ISBNs, conventions.md §7.5) and the given language, one hit per work (its earliest such edition), ordered by that date, then work id. `SearchHit.planned_publication_date` is that date. It feeds a Discover row for German users. |
| (background job) | – | Every `DNB_HARVEST_INTERVAL_MINUTES` (≥ 30, default 240) when `DNB_HARVEST=true`, for each set in `DNB_HARVEST_SETS` (`dnb-all:reiheN` and `dnb:reiheA`). |

## 6. Media (#43, owner MED)

| New | Auth | Request → Response | Notes |
|---|---|---|---|
| `GET /media/{public_id}/{variant}` | none | → image bytes, `Cache-Control: public, max-age=31536000, immutable`, `X-Content-Type-Options: nosniff`, `Content-Type` from the row | Variants: covers `s` (≤ 240 px wide WebP), `l` (≤ 800 px WebP); avatars `a`. `removed_at` is checked first: 410 `This picture was removed` after a takedown, and a removed row is never fetched again. An Open Library cover row with no bytes yet is fetched **by cover id** from `covers.openlibrary.org` (throttled, waiting up to 10 s; concurrent requests are coalesced), converted, stored, colour-sampled and then served; most covers are already stored by the prefetch queue. 404 `Not found` for an unknown id, variant or a failed OL fetch; 429 while Open Library is throttled. **Every non-200 answer has `Cache-Control: no-store`**, so a cover that failed once is retried on the next view. |
| `POST /api/books/works/{work_id}/cover` | user | multipart `file` (and an optional `edition_id` form field) → **200** `{book: BookOut}` when applied directly | A photo of the user's copy. EXIF/GPS is stripped by re-encoding, the image is cropped/resized and stored as WebP. **The first cover of an edition (or of a work without editions) becomes the cover right away**, atomically (`… WHERE cover_media_id IS NULL RETURNING`), with provenance `user`, so no Open Library refresh replaces it. **If a cover exists (or a concurrent upload won), the upload becomes a `cover_replace` edit request**: 202 `{request_id}`. A picture whose original bytes match a removed one is refused: 400 `This picture was removed`. Same picture error strings as the avatar. 30 per day per user. |
| `POST /api/media/{public_id}/reports` | user | `{reason: "copyright"\|"wrong_book"\|"offensive"\|"other", details?}` → **201** `ReportCreated {id, status: "open"\|"removed"}` | Notice-and-takedown. `public_id` is `cover_public_id` from `BookOut`, `EditionOut`, `SearchHit` or `BookDetailOut`. 20 **new** reports per day (`report_user`); a repeat of an open report is answered with it and not counted there. Every request, repeats and 404s included, counts against 30 per minute per account (`report_request_user`, checked first; 429 `Slow down a little. Try again later.`). A repeat is answered from a plain read without a lock (security review, Phase 2 Wave 1 #1). 404 for an unknown media id. **Idempotent per reader and picture (ADM #54):** a reader with an open report of the picture gets that report back (same `id`, `status: "open"`, nothing written, even at the limit); a report of a picture already taken down is recorded closed on arrival (`status = 'removed'`, `resolved_at = created_at`, `resolved_by` NULL), never enters the queue, and answers `status: "removed"`; a reader whose earlier report was dismissed files a new one. |
| `GET /api/admin/reports` | instance admin | `?status=open\|resolved&page=n` → `ReportList {items: ReportOut[], page, has_more}` | **ADM (#54):** `open` (the default) oldest first (`created_at, id`); `resolved` lists `removed`/`dismissed` reports newest first (`resolved_at DESC, id DESC`). Paging and 400s as for `GET /api/admin/edit-requests` (§4). **`ReportOut.closed_on_arrival: bool`**: the report was closed when it arrived because the picture was down already (`status = 'removed'`, `resolved_by` NULL and `resolved_at = created_at`); no admin decided it. |
| `GET /api/admin/summary` | instance admin | → `AdminSummary {target_hours, reports: AdminQueueSummary, edit_requests: AdminQueueSummary, report_stats: AdminReportStats}` | **Owner ADM (#54)**, operation `admin_summary`. `target_hours` is 72 (`admin::queue::TARGET_HOURS`). `AdminQueueSummary {open, overdue, oldest: AdminOldestItem {id, created_at}\|null}`; `overdue` counts open items older than `target_hours`. `AdminReportStats {days: 90, resolved, median_hours, max_hours, over_target}` over reports resolved in the last 90 days, not closed on arrival; hours are `resolved_at - created_at` rounded to 0.1, median and max null when nothing was resolved. |
| `POST /api/admin/reports/{report_id}/resolve` | instance admin | `{action: "remove"\|"dismiss"}` → `ReportOut` | `remove` deletes the bytes, sets `media.removed_at` and clears every `cover_media_id` that pointed at it (`ingest::clear_media`). The removal sticks: `ingest::ol_cover_media` never returns a removed row, so no refresh, enrichment or harvest attaches it again, and no fetch or backfill downloads it again. `remove` resolves every open report of the picture; **`dismiss` closes every open report of the picture too** (`status = 'dismissed'`, ADM #54); a report whose media row is gone resolves only itself. |
| `DELETE /api/admin/media/{public_id}` | instance admin | optional body `TakeDownIn {details?, received_at?, reason?}` or `null` → **204** | Direct takedown, same effect as `remove`. **Every takedown** (this and `remove`) also rejects the picture's open `cover_replace` requests with the decision note `The picture was taken down` (ADM #54). **Without a body** (or an empty one, or `null`) nothing else happens. **With a body**, it also records the emailed notice as one `reports` row: `target_kind 'cover'`, the picture and the book that showed it, `reason` (default `other`), `details` (cut to 2000 characters), `created_by` NULL, `created_at = received_at` (default now; up to 5 minutes ahead is clamped to now), `status 'removed'`, `resolved_by` the admin; it shows in the history and the statistics, never in the open queue. A picture already down still gets the record. 400 `Input should be 'copyright', 'wrong_book', 'offensive' or 'other'`, `Input should be in the past` (`received_at` later than 5 minutes ahead), `Invalid value` (not RFC 3339), the usual body 400s; 400 `Only notices about covers can be recorded` for a body about an avatar (nothing taken down; ADM2 lifts this). 404 `Not found`. |
| `POST /api/shelf/spine-colors` (legacy, same path) | user | → `{colors: {"<work id>": "#rrggbb"}, remaining}` | **Owner MED.** Same contract. Colours are computed at cover ingest; this endpoint is a backfill: up to 12 of the caller's shelf works whose cover has no `dominant_color`, fetching OL bytes when needed. Keys are `BookOut.id` (work id). |

## 7. Personal shelf (#44a, owner SHELF)

| Legacy | New | Request → Response | Changes |
|---|---|---|---|
| `GET /api/shelf[?username=]` | same | → `ShelfListOut {timezone, user: UserSummary, items: ShelfItemOut[], favorites: FavoriteOut[]}` | `timezone` = the **owner's** zone. `user` is a `UserSummary` (no theme or locale leak). Visibility: self or audience; strangers and unknown names get the same 404 `No member with that name`. For non-owners, `rating`/`take` are null/`""` when `rating_visibility='only_me'`, and `rating_visibility` itself is null. Order unchanged: `status` text, `position`, `id`. **ShelfItemOut** `{id, status, position, shelf_position, updated_at, started_at, finished_at, book: BookOut, edition: EditionOut\|null, rating, rating_visibility\|null, take, dnf_reason, progress}`. |
| `POST /api/shelf` | same | `{work_id, edition_id?, status?, rating?, rating_visibility?, take?, dnf_reason?, progress?}` → **201** `ShelfItemOut` | **No client book metadata** (`ol_work_key`, `title`, `authors`, `cover_id`, `cover_url`, `year` are gone; remote hits already have ids). 404 `No such book.` for an unknown work; 400 `That edition belongs to another book`. 409 `{"detail":"Already on your shelf","item":ShelfItemOut}` also covers the concurrent-insert race (unique violation → 409). The events are unchanged. |
| `PATCH /api/shelf/{entry_id}` | same | legacy fields plus `edition_id?` (tri-state) and `rating_visibility?` → `ShelfItemOut` | Identical semantics and check order ("Nothing to update" before the 404), strings unchanged. A PATCH with only `rating_visibility` leaves the column and `updated_at` alone, like a `shelf_position`-only one. `finished_on` uses the **user's** timezone (noon local → UTC; "today" in the user's zone). A per-user `FOR UPDATE` lock on the users row serialises reorders. |
| `DELETE /api/shelf/{entry_id}` | same | → **204** | Unchanged: `shelf_removed` event, layout pruned in the same transaction, column renumbered. |
| `POST /api/shelf/import` | same | multipart `file` → **202** `ImportJobOut` | **Now a background job.** `ImportJobOut {id, status: "queued"\|"running"\|"done"\|"failed", total, processed, imported, skipped, skips: [{title, code, reason}], error\|null}`. Parse errors are still a synchronous 400 with the legacy strings. Codes: `stopped_after_400`, `no_title`, `unknown_shelf`, `no_match`, `catalog_unavailable`, `already_on_shelf`; `reason` keeps the legacy English. Lookups go through `CatalogLookup` (local first). New: the CSV ISBN sets `edition_id` when it resolves; `My Rating` (1–5, `read` rows) and `Date Read` (→ `finished_at`, noon in the user's zone) are imported. One active job per user: 409 `An import is already running`. |
| – | `GET /api/shelf/import/{job_id}` | → `ImportJobOut` | 404 `No such import` (also for another user's job). The frontend polls every 2 s. |
| `GET /api/shelf/physical-layout[?username=]` | same | → `PhysicalShelfLayout` (shape unchanged) | Same visibility and 404 as the shelf. Read-time pruning is unchanged; a stored layout that fails the schema falls back to the default and never causes a 500. |
| `PUT /api/shelf/physical-layout[?username=]` | same | `PhysicalShelfLayout` → same | `?username=` naming anyone but the caller → **404 `No member with that name`** (#46 AC). Validation strings unchanged. |
| `GET /api/search-history`, `POST`, `DELETE`, `DELETE /{entry_id}` | same | unchanged | Casefold via the `caseless` crate; `Search history entry not found` kept (mapped). |
| `GET /api/quotes` | same | `work?: i64` (legacy `work=/works/…`), `username?`, `limit` → `{items: QuoteOut[]}` | **QuoteOut** `{id, book: BookOut, edition_id, author, mine, body, page, visibility, created_at, created_label}`. Only visible quotes are returned (own, plus audience and `audience` visibility). An unknown or stranger `username` → `{"items":[]}`. `created_label` uses the viewer's timezone. |
| `POST /api/quotes` | same | `{work_id, edition_id?, body, page?, visibility?}` → **201** `QuoteOut` | No client book metadata. 404 `No such book.`. `edition_id` defaults to the user's shelf edition for the work. The `quote_added` event now carries `quote_id`. |
| `PATCH /api/quotes/{quote_id}` | same | `{body?, page? (tri-state), visibility?}` → `QuoteOut` | Unchanged strings. 404 for a quote the viewer cannot see. `updated_at` (only in the export) moves when `body` or `page` is sent, not for `visibility` alone. |
| `DELETE /api/quotes/{quote_id}` | same | → **204** | Unchanged. |
| `GET /api/stats` (club-wide) | `GET /api/stats?username=&year=` (**personal**) | → `PersonalStatsOut {username, year, years[], finished, dnf, pages, average_rating, five_stars, top_book, longest_book, quotes, by_month[]}` | The legacy `MemberYear` fields for one user. Visibility as for the shelf; for others, `only_me` ratings and quotes are excluded. `pages` = the edition's pages, else the work's. Year boundaries stay UTC (parity). Club stats are in §9. |

## 8. Diary entries and pick discussions (#44b, owner POSTS)

Posts are one table with two scopes (conventions.md §6.3).

**PostOut** (legacy `DiaryEntryOut` plus scope fields):

```text
{ id, author: string|null, author_avatar_url, mine, body, deleted, visibility,
  spoiler_upto, progress_at, status_at, author_rating, parent_id,
  club: ClubRef|null, pick_id|null, milestone_id|null,
  created_at, created_label, edited, reactions: ReactionOut[], replies: PostOut[] }
```

- `author` is null only for tombstones left by a deleted account.
- `author_rating` respects `only_me`.

| Legacy | New | Request → Response | Changes |
|---|---|---|---|
| `GET /api/books/works/{work_id}/posts` | same (`{work_id}` as in §4) | → `DiaryOut {work_id, my_progress, my_status, items: PostOut[], timezone}` | Items are the viewer-visible **diary entries** on the work plus the **club posts** on the work from the viewer's clubs (each with `club`). Order is `created_at, id` for roots and replies. An unknown work → 404 `No such book.`. `timezone` = the viewer's zone; a club post's `created_label` is in its club's zone, a diary entry's in the viewer's (conventions.md §12). |
| `POST /api/books/works/{work_id}/posts` | same | `{body, spoiler_upto?, parent_id?, visibility?}` → **201** `PostOut` | Creates a **diary entry**. A reply inherits its root's scope (enforced by `posts_parent_fk`): replying to a club post creates a club reply and needs membership of **that** club (`role_in`). `book` (client metadata) is **dropped**, and so is `Nobody has this book yet; send its title to start the diary`. Parent checks and strings are unchanged; a parent the viewer cannot see → 400 `That entry is not on this book`. No event (parity). |
| `PATCH /api/posts/{post_id}` | same | `{body?, spoiler_upto? (tri-state), visibility? (diary roots only)}` → `PostOut` | 404 `No entry with that id` (missing, tombstone **or not visible**, through `visibility::post_access`); 403 `You can only edit your own entries`; 400 `Nothing to update`. Sending `body` or `spoiler_upto` sets `updated_at` (`edited`), even with unchanged values (R8); a PATCH that only changes `visibility` leaves it as it is. |
| `DELETE /api/posts/{post_id}` | same | → **204** | 404 as for PATCH. 403 `You can only delete your own entries`, **except** admins and the owner **of the post's own club** on its club posts (moderation; the role comes from `PostAccess.club_role`, never from another club). The tombstone rule is unchanged. |
| `POST /api/posts/{post_id}/reactions` | same | `{emoji}` → `PostOut` | **404 `No entry with that id` for a missing, tombstoned or invisible post** (another club's post, a stranger's diary entry, someone else's `only_me` entry), through the same `visibility::post_access` as PATCH/DELETE: the response carries the body. The toggle is atomic. A reaction on a club post records the `reaction` event (`{emoji, post_id, author_id}`; the feed shows it as `author`); a reaction on a diary entry records none (parity). `users` is filtered to the viewer's audience, club members and self. |
| `GET /api/diary` | same | `before?, limit (1..50, default 10)`, **new `club?: slug`**, **new `mine?: bool`** → `DiaryFeedOut {items: [{entry: PostOut (no replies), book: BookOut, parent_author\|null, my_progress, my_status, club: ClubRef\|null}], has_more, timezone}` | Reads through `bc_visible_posts`. Without `club`: the **combined home feed** (visible diary entries plus club posts of all the viewer's clubs). With `club`: resolved by `clubs::extract::member_by_slug`; that club's posts plus diary entries by its current members that the viewer may see; a non-member gets 404 `No such club`. With `mine=true` (a lax boolean, default false): only the caller's own diary writing, their entries and their replies to diary entries, no club posts (the SPA's `/diary`); it combines with `club`. Keyset on `id DESC`; tombstones excluded. `parent_author` is null when the parent's author is not visible. `timezone` = the viewer's zone; a club post's `created_label` is in its club's zone (conventions.md §12). |
| `GET /api/pick/posts` | `GET /api/clubs/{slug}/pick/posts` | → `PickThreadOut {pick_id, can_post, my_progress, my_status, items: PostOut[], timezone}` | The current pick's discussion. 404 `No club pick yet`. Shape = the diary shape (the legacy `PickPostOut` folded into `PostOut`), so the frontend reuses `BookDiary`/`DiaryEntry`. Replies are allowed (the legacy pick posts had none). `timezone` = the club's zone. |
| `POST /api/pick/posts` | `POST /api/clubs/{slug}/pick/posts` | `{body, spoiler_upto?, milestone_id?, parent_id?}` → **201** `PostOut` | Any member. Records `note_posted` `{post_id}`. 400 `That milestone is not on this pick`. |
| `GET /api/pick/{pick_id}/posts` | `GET /api/clubs/{slug}/picks/{pick_id}/posts` | → `PickThreadOut` | 404 `No club pick with that id` (also for another club's pick). |
| `POST /api/pick/{pick_id}/posts` | `POST /api/clubs/{slug}/picks/{pick_id}/posts` | as above | 400 `That pick is closed`. |
| `PATCH /api/pick/posts/{post_id}` | `PATCH /api/posts/{post_id}` | – | Unified with diary entries. The strings become the diary ones (`You can only edit your own entries`); the legacy note strings had no UI. |
| `DELETE /api/pick/posts/{post_id}` | `DELETE /api/posts/{post_id}` | – | Admins may delete others' club posts. |
| `POST /api/pick/posts/{post_id}/reactions` | `POST /api/posts/{post_id}/reactions` | – | |

## 9. Club features (#45, owner CLUB)

The `timezone` in every club-scoped response is the club's, and its labels (`meeting_label` and the like) use the legacy format in the club zone. The combined home feed (`GET /api/activity`) is not club-scoped: its `timezone` is the viewer's, and it labels club items in their club's zone and personal items in the viewer's (conventions.md §12).

| Legacy | New | Role | Request → Response | Changes |
|---|---|---|---|---|
| `GET /api/pick` | `GET /api/clubs/{slug}/pick` | member | → `{pick: ClubPickOut\|null, timezone}` | **ClubPickOut** `{id, book: BookOut, set_by\|null, note, started_at, ended_at, meeting_at, meeting_local, meeting_label, on_shelf, shelf_id, readers: ClubPickReader[], reading[], finished[]}`. Readers are **club members** only (viewer first, then username); `rating`/`take` are redacted for `only_me`. **No side effects** (the meeting-reminder push is dropped until Phase 4). |
| `GET /api/pick/history` | `GET /api/clubs/{slug}/pick/history` | member | → `{items, timezone}` | Unchanged order (`ended_at DESC, id DESC`). |
| `PUT /api/pick` | `PUT /api/clubs/{slug}/pick` | **admin** | `{work_id, note?, meeting_at?}` → `ClubPickOut` | No client metadata. Same work as the current pick → update **only the keys present** (fixes the legacy wipe of `note` and the change of `set_by`). A different work → end the current pick and record `pick_set`. 404 `No such book.`; 400 `note must be 280 characters or fewer`, `meeting_at must be a date and time`. |
| – | `PATCH /api/clubs/{slug}/pick` | admin | `{note?, meeting_at? (tri-state)}` → `ClubPickOut` | Edits the meeting and note of the current pick without touching `set_by`. 404 `No club pick yet`. The frontend's meeting edit uses this. |
| `DELETE /api/pick` | `DELETE /api/clubs/{slug}/pick` | admin | → `{pick: null, timezone}` | `pick_cleared`. |
| `GET /api/pick/meeting.ics` | `GET /api/clubs/{slug}/pick/meeting.ics` | member | → `text/calendar` | Golden-output rules unchanged. Changes: `PRODID` and `SUMMARY` use the **club name**; `DESCRIPTION` links `{PUBLIC_URL}/book/{work_id}` instead of openlibrary.org; `URL:` is `{PUBLIC_URL}/c/{slug}`. 404 `No meeting scheduled`. |
| `GET /api/pick/{pick_id}/milestones` | `GET /api/clubs/{slug}/picks/{pick_id}/milestones` | member | → `{pick_id, items: MilestoneOut[], timezone}` | **MilestoneOut** `{id, pick_id, title, unit: "percent"\|"chapter"\|null, range_from, range_to, due_at, due_local, due_label, position, note_count, passed}`. `chapter_from`/`chapter_to` are renamed to `range_from`/`range_to` with `unit`. `note_count` counts club posts with the milestone. |
| `POST /api/pick/{pick_id}/milestones` | `POST …/picks/{pick_id}/milestones` | **admin** | `{title, unit?, range_from?, range_to?, due_at?}` → **201** | Legacy strings, plus `Percent milestones go up to 100` and `A range needs a unit`. `That pick is closed`, `Chapter range is backwards` (now also for percent). `milestone_added` is recorded. |
| `PATCH …/milestones/{milestone_id}` | same, under the club | admin | partial (only present keys change) → `MilestoneOut` | **Fixed:** partial update instead of a full replace, and the backwards check also runs on edit. The closed-pick check stays create-only (parity). |
| `DELETE …/milestones/{milestone_id}` | same | admin | → **204** | Detaches posts (via the FK), renumbers positions densely. |
| `GET /api/vote` | `GET /api/clubs/{slug}/vote` | member | → `VoteOut` (legacy shape) | `member_count` and `not_voted` cover **club members**. Lazy auto-close is unchanged; it locks the vote row with `FOR UPDATE`. If the winning nominator left the club, they are still credited; if their account was deleted, `set_by` and the event actor are NULL. |
| `PATCH /api/vote` | `PATCH /api/clubs/{slug}/vote` | **admin** | `{closes_at}` → `VoteOut` | Strings unchanged. |
| `POST /api/vote/nominations` | `POST /api/clubs/{slug}/vote/nominations` | member | `{work_id}` → **201** `VoteOut` | No client metadata. The limit of 6, the current-pick check and the duplicate check all run under a lock on the vote row. |
| `POST /api/vote/cast` | `POST /api/clubs/{slug}/vote/cast` | member | `{nomination_id}` → `VoteOut` | `voted` is recorded only on the first ballot (`INSERT … ON CONFLICT … RETURNING (xmax = 0)`). |
| `POST /api/vote/apply` | `POST /api/clubs/{slug}/vote/apply` | **admin** | `{nomination_id, meeting_at?}` → `{pick, vote}` | |
| `GET /api/vote/suggestions` | `GET /api/clubs/{slug}/vote/suggestions` | member | → unchanged shape, plus `reason_codes` | Each item's `reason_codes` holds its `reasons` (English, unchanged) as codes with values, in the same order, so clients word them themselves: `{code: "wanted", usernames}` (sorted; one name or two and more), `{code: "like_past_picks", subjects}` (at most 3), `{code: "ol_rating", rating}` (unrounded). Schema `VoteReasonOut`. Candidates come from the want-to-read entries of **club members**. "Like past picks" uses `works.subjects`; "Open Library rating" uses `works.ol_rating` (both are kept in the catalog). |
| `GET /api/overlap` | `GET /api/clubs/{slug}/overlap` | member | `include_reading` (lax bool) → unchanged shape | Computed over club members only. |
| `GET /api/activity` | `GET /api/activity` (**combined home feed**) and `GET /api/clubs/{slug}/activity` | user / member | `before?, limit (1..100, default 30), username?` → `ActivityOut {items: ActivityItem[], has_more, timezone}` | **ActivityItem** `{id, kind, actor\|null, mine, book: BookOut\|null, pick_id, club: ClubRef\|null, payload, created_at, created_label}`. `reaction` payloads are stored with `author_id` and served with the legacy key `author` (the username, or null when the account is gone). Club feed = the club's events plus personal events of **current members** created after their `joined_at`, filtered and redacted by the policy (only_me rating/take stripped from `shelf_finished`, and a rating or take that was only-me or whose entry went is sealed to its actor in the stored payload, so it never shows to others again; `quote_added` hidden from others when the quote is gone or `only_me`, and served with the quote's current `excerpt` and `page`; conventions.md §6.4). Home feed = the union over all the viewer's clubs plus the viewer's own personal events, deduplicated by id. `username` filter: an unknown or non-visible name → empty list. Labels and `timezone`: the club feed uses the club's zone; the home feed labels club items in their club's zone and personal items in the viewer's zone, and its `timezone` is the viewer's (conventions.md §12). |
| – | `GET /api/clubs/{slug}/stats?year=` | member | → legacy `StatsOut` | **Owner SHELF** (it shares `MemberYear` with the personal `GET /api/stats`, so one package owns both). `members` = club members, `picks` = club picks; reader counts use club members' shelf entries for the pick's work, read through `bc_visible_shelf`; `only_me` ratings and quotes are excluded. |

## 10. Legacy importer and CLI (#38, owner IMP; CLI wiring by W1)

These are not HTTP endpoints; they are listed for completeness. The CLI is `bookclub-server <subcommand>`:

- `serve` (default)
- `import-legacy …` (IMP)
- `rematch-legacy [--dry-run]` (IMP: re-resolves `source='legacy'` editions by ISBN through `CatalogLookup::resolve_isbn_remote` and upgrades them under the provenance rules; legacy-mapping.md §3.3)
- `make-admin <username>` (W1)
- `beta-invite [--uses N] [--days D]` (W1)
- `openapi` (W1: prints the OpenAPI JSON for #47; needs no database)
- `healthcheck` (DEP #51: GETs `/api/health` on the port `BIND_ADDR` names, 3 s, exit 0 only on 200; the image's `HEALTHCHECK`; no other setting, no database)
- `send-test-mail <address>` (DEP #51: one test mail through the configured transport; exit 1 when the SMTP server refuses; no database)

`import-legacy` reads a private copy of the legacy file and its `-wal` in `$TMPDIR`, never the file itself (CUT #52, conventions.md §16). The log goes to stderr; stdout holds only a command's result (conventions.md §19).

## 11. Detail strings

### 11.1 Kept verbatim and already mapped in `apiErrors.ts`

- `Not signed in`
- `That username is already taken`
- `That invite code is not valid`
- `Too many attempts. Wait a few minutes.`
- `Wrong username or password`
- `Current password is wrong`
- `Pick a different password`
- `That picture is too large (50 MB max)`
- `That picture has too many pixels`
- `Use a JPEG, PNG, WebP, or GIF`
- `That file is not a picture`
- `Each book can only be a favourite once`
- `At most 3 favourites`
- `You already have 3 favourites`
- `Reading date requires a finished book`
- `Reading date cannot be in the future`
- `Search history entry not found`

### 11.2 Removed from `apiErrors.ts` in #48

- `No profile picture`: the endpoint is gone.
- `That book is not in the club yet`: becomes `That book is not in the catalog`.

### 11.3 New strings #48 must map (en and de)

**Auth:**
- `That email is already in use`
- `Enter a valid email address`
- `That link has expired or was already used`
- `Add an email address first`
- `Sign-up needs an invite code right now`
- `Verify your email address first`
- `New password must be at least 8 characters`
- `That is not a valid time zone`
- `Transfer or delete your clubs first`

**Clubs:**
- `No such club`
- `Only club admins can do that`
- `Only the club owner can do that`
- `This club is full`
- `You are already in this club`
- `You can create at most 10 clubs`
- `Transfer ownership before leaving`
- `Use transfer to change the owner`
- `Use leave to leave the club`
- `Give the club a name`
- `Keep the club name to 80 characters`
- `No such invite`
- `No member with that name`

**Catalog and personal:**
- `That book is not in the catalog`
- `That edition belongs to another book`
- `That ISBN is already in the catalog`
- `This book was added by a reader, not Open Library.`
- `An import is already running`
- `No such import`
- `No such session`
- `This picture was removed`
- `A book cannot be merged into itself`

**Platform:**
- `Slow down a little. Try again later.` (429 from non-auth rate limits)
- `Cross-site request refused` (403 from the origin check; only hit by misconfigured clients)
- `Request body is too large` (413 for a JSON body over the route's limit, 1 MiB by default)
- `The request took too long` (408 from the 10-minute request deadline, conventions.md §3)
- `The server is busy. Try again in a moment.` (503 when no password permit frees up within 10 s: login, sign-up, token creation, reset confirm and the current-password checks; conventions.md §5.4; security review 2026-10-28 #1)

**Phase 2 Wave 1:**
- `The terms of use have changed. Reload the page and try again.` (LEG #53, `msg::TERMS_CHANGED`; 400 from sign-up and `POST /api/auth/terms`). Mapped to `errors.termsChanged`, whose copy says the app fetched the new version instead of asking for a reload.
- `Input should be in the past` (ADM #54, `media::reports::RECEIVED_IN_FUTURE`) and `Only notices about covers can be recorded` (`media::reports::NOTICE_COVERS_ONLY`): 400s of the takedown notice. The SPA can reach both (a device clock running fast; an avatar's URL), and `AdminTakeDown.vue` answers them in its own words by exact `detail` (`admin.ts::TAKEDOWN_DETAILS`). Not yet in `apiErrors.ts`.
- `Input should be 'open' or 'resolved'` (ADM, `admin::queue::STATUS_CHOICES`): the queue lists' `status`; the SPA never sends another value.
- `Only notices about covers can be recorded` disappears with ADM2's avatar reports; `The picture was taken down` is a stored decision note (`media::reports::TAKEN_DOWN_NOTE`), not a detail string.

**Optional (Discover branches by status; mapping them only translates the text):** the catalog's legacy strings `No such book.`, `Need at least 3 characters. Add the author, or paste an ISBN.`, `Search is too long`, `Could not search the library right now. Try again.`, `Could not load the library right now. Try again.`, `The library is busy. Wait a few seconds and try again.`

The Rust test `server/tests/api_error_copy.rs` asserts that every mapped string exists in `server/src`. Until #48 rewrites the mapping, it carries an explicit allow-list of the two legacy-only strings in §11.2; #48 empties the list and deletes `backend/tests/test_api_error_copy.py`.

## 12. Legacy → new path index (quick lookup)

| Legacy path | New path |
|---|---|
| `/api/auth/*` | `/api/auth/*` (same), plus the new auth routes |
| `/api/invites` | `/api/clubs/{slug}/invites`; redemption at `/api/invites/{code}/accept` |
| `/api/members` | `/api/clubs/{slug}/members` |
| `/api/members/{u}/avatar` | dropped (`/media/{public_id}/a`) |
| `/api/books/search`, `/trending`, `/subjects/{s}`, `/custom`, `/works/{id}`, `/works/{id}/refresh`, `/isbn/{isbn}` | same |
| `/api/books/work/{id}`, `/api/books/work/{id}/refresh` | dropped (merged into `/works/{id}`) |
| `/api/books/works/{id}/posts`, `/api/posts/*`, `/api/diary` | same |
| `/api/shelf*`, `/api/search-history*`, `/api/quotes*` | same |
| `/api/stats` | `/api/stats` (personal) and `/api/clubs/{slug}/stats` |
| `/api/pick*` | `/api/clubs/{slug}/pick*`, and `/api/clubs/{slug}/picks/{pick_id}/*` |
| `/api/pick/posts/{id}*` | `/api/posts/{id}*` |
| `/api/vote*` | `/api/clubs/{slug}/vote*` |
| `/api/overlap` | `/api/clubs/{slug}/overlap` |
| `/api/activity` | `/api/activity` (home) and `/api/clubs/{slug}/activity` |
| `/api/backup` | `/api/auth/me/export` |
| `/api/push/*` | dropped (Phase 4) |
| `/manifest.webmanifest` | static file |
