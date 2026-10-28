# Port spec: frontend (how the Vue app consumes the API, and what #48 changes)

Area key: `frontend`. Scope: everything under `frontend/` (Vue 3 + Pinia + vue-router + vue-i18n, Vite 7, Vitest 4). The Python backend is consulted only where the UI depends on a behaviour, status code or string. Backend endpoint internals are specified in the sibling specs (`spec/auth.md`, `spec/shelf.md`, `spec/catalog.md`, `spec/google.md` and the club/diary specs); this document is the consumer side of that contract.

Labels used below: **[plan]** is decided in `plan.md`, `phase1-issues.md` or Forgejo #40/#45/#46/#48 (read for detail). **[proposal]** is my recommendation for the implementer. **[open]** is not settled; it is repeated in section 7.

Files read in full: `api/client.ts`, `types.ts`, `i18n/apiErrors.ts`, `i18n/index.ts`, `i18n/dates.ts`, `router.ts`, `nav.ts`, `main.ts`, `App.vue`, `brand.ts`, `assetRecovery.ts`, `autosize.ts`, `constants.ts`, `shelf.ts`, `diary.ts`, `drafts.ts`, `physicalShelf.ts`, `pages/discoverCache.ts`, `composables/useMediaQuery.ts`, every `stores/*.ts`, every `pages/*.vue`, every `components/*.vue`, `layouts/AppShell.vue`, `index.html`, `vite.config.ts`, `package.json`, every `src/**/*.test.ts`, `test/setup.ts`.

---

## 1. API call sites, grouped by endpoint

### 1.0 The client contract (`src/api/client.ts`)

Every HTTP call except two goes through `request<T>(path, init)`:

- **URL:** always a relative `/api/...` path. There is no base URL, no origin config and no version prefix. Vite dev proxies `/api` to `http://127.0.0.1:8000` (`vite.config.ts`).
- **Credentials:** `credentials: "include"` on every call. Auth is the `bookclub_session` cookie only. No `Authorization` header, no CSRF header, no custom headers.
- **Content-Type:** set to `application/json` only when `init.body` is truthy, not a `FormData`, and no Content-Type was given. FormData uploads (avatar, Goodreads CSV) send no Content-Type, so the browser adds the multipart boundary (tested).
- **204:** `response.status === 204` returns `undefined` without reading the body.
- **Everything else:** `response.text()`, then `JSON.parse(text)` if non-empty, else `null`. A non-JSON body (HTML proxy error page, axum plain-text extractor rejection) makes `JSON.parse` throw a `SyntaxError`, which is **not** an `ApiError`. Pages then show their generic fallback string.
- **Error (`!response.ok`):** `detail = data?.detail`. The message is `translateApiError(detail)` when `detail` is a string, otherwise `t("errors.generic")` ("Something went wrong. Try again."). The thrown error is `new ApiError(message, response.status, detail ?? data, data?.item)`.
  - `ApiError.status` drives branching (table in 2.2).
  - `ApiError.item` carries the conflicting `ShelfItem` from the 409 on `POST /api/shelf`.
  - `ApiError.detail` keeps the raw English detail. No page reads it.
- **Abort:** `init.signal` is passed through for search, trending, subject and book detail. Callers detect `AbortError` (DOMException or Error with `name === "AbortError"`) and ignore it.
- **No global 401 handling.** A session that expires mid-use surfaces as `"Not signed in"` / `"Nicht angemeldet"` in whatever error slot the page has. There is no redirect to `/login` until the next full load, because `session.ready` stays true.
- **No retries, no timeouts, no request dedupe** in the client. Pages implement their own stale-response guards (Discover, BookDetail).

**Two calls bypass `request()`:**

1. `SettingsPage.vue` renders `<a href="/api/backup">` (plain navigation; the browser downloads the SQLite file).
2. `<img src>` URLs taken from response fields: `avatar_url` (`/api/members/{username}/avatar?v={unix}`) and cover URLs (section 1.13). These rely on the cookie being sent with same-origin image requests.

**`translateApiError(detail)`** (`i18n/apiErrors.ts`): exact, case-sensitive string lookup in `API_ERROR_KEYS`, returning `t(key)` on a hit and the raw English detail on a miss. No prefix or pattern matching, so dynamic strings such as `"The library is busy. Try again in 20 seconds."` always pass through untranslated.

### 1.1 The complete `API_ERROR_KEYS` map (19 entries, verbatim)

| # | Backend `detail` (exact) | i18n key | en.json text | de.json text | Raised by (Python) | Status |
|---|---|---|---|---|---|---|
| 1 | `Reading date requires a finished book` | `errors.readingDateFinished` | Reading dates can only be set for finished books. | Ein Lesedatum ist nur für gelesene Bücher möglich. | `routers/shelf.py` PATCH `/api/shelf/{id}` | 400 |
| 2 | `Reading date cannot be in the future` | `errors.readingDateFuture` | The reading date cannot be in the future. | Das Lesedatum darf nicht in der Zukunft liegen. | same | 400 |
| 3 | `Search history entry not found` | `errors.historyNotFound` | That search is no longer in your history. | Diese Suche ist nicht mehr in deinem Verlauf. | `routers/search_history.py` DELETE `/{id}` | 404 |
| 4 | `Not signed in` | `errors.notSignedIn` | Not signed in | Nicht angemeldet | `deps.get_current_user` (every authed route) | 401 |
| 5 | `That username is already taken` | `errors.usernameTaken` | That username is already taken | Dieser Benutzername ist schon vergeben | POST `/api/auth/register` | 409 |
| 6 | `That invite code is not valid` | `errors.inviteInvalid` | That invite code is not valid | Dieser Einladungscode gilt nicht | POST `/api/auth/register` | 400 |
| 7 | `Too many attempts. Wait a few minutes.` | `errors.tooManyAttempts` | Too many attempts. Wait a few minutes. | Zu viele Versuche. Warte ein paar Minuten. | POST `/api/auth/login`, PATCH `/api/auth/password` | 429 |
| 8 | `Wrong username or password` | `errors.wrongCredentials` | Wrong username or password | Falscher Benutzername oder Passwort | POST `/api/auth/login` | 401 |
| 9 | `Current password is wrong` | `errors.currentPasswordWrong` | Current password is wrong | Aktuelles Passwort ist falsch | PATCH `/api/auth/password` (no UI today) | 400 |
| 10 | `Pick a different password` | `errors.differentPassword` | Pick a different password | Wähle ein anderes Passwort | PATCH `/api/auth/password` (no UI today) | 400 |
| 11 | `That picture is too large (50 MB max)` | `errors.pictureTooLarge` | That picture is too large (50 MB max) | Das Bild ist zu groß (höchstens 50 MB) | PUT `/api/auth/me/avatar` (`avatars.TOO_LARGE`) | 413 (Content-Length precheck) or 400 (after read) |
| 12 | `That picture has too many pixels` | `errors.pictureTooManyPixels` | That picture has too many pixels | Das Bild hat zu viele Pixel | PUT avatar | 400 |
| 13 | `Use a JPEG, PNG, WebP, or GIF` | `errors.usePicture` | Use a JPEG, PNG, WebP, or GIF | Nimm ein JPEG, PNG, WebP oder GIF | PUT avatar (no `file` field, or unsupported type) | 400 |
| 14 | `That file is not a picture` | `errors.notAPicture` | That file is not a picture | Das ist keine Bilddatei | PUT avatar | 400 |
| 15 | `No profile picture` | `errors.noPicture` | No profile picture | Kein Profilbild | GET `/api/members/{u}/avatar`. Only ever fetched by `<img>`, so this mapping is effectively dead. | 404 |
| 16 | `That book is not in the club yet` | `errors.bookNotInClub` | That book is not in the club yet | Dieses Buch ist noch nicht im Club | favourites PUT/POST (`favorites.BOOK_NOT_IN_CLUB`) | 400 |
| 17 | `Each book can only be a favourite once` | `errors.duplicateFavourite` | Each book can only be a favourite once | Jedes Buch darf nur einmal ein Favorit sein | `schemas.FavoritesIn` validator, so it arrives via the 400 validation handler | 400 |
| 18 | `At most 3 favourites` | `errors.tooManyFavourites` | At most 3 favourites | Höchstens 3 Favoriten | `FavoritesIn` validator and `favorites.TOO_MANY_FAVOURITES` | 400 |
| 19 | `You already have 3 favourites` | `errors.favouritesFull` | You already have 3 favourites | Du hast schon 3 Favoriten | POST `/api/auth/me/favorites/{id}` (`FAVOURITES_FULL`) | 409 |

Notes:

- `errors.generic` ("Something went wrong. Try again." / "Etwas ist schiefgelaufen. Versuch es nochmal.") is the 20th key in `errors.*`. It is used for non-string `detail`, not mapped.
- The Python validation handler joins multiple pydantic messages with `"; "`. A joined string never matches a map entry, so it shows raw.
- `backend/tests/test_api_error_copy.py` asserts that every quoted key in `apiErrors.ts` (regex `"([^"]+)":\s*"errors\.`) appears somewhere in `backend/app/**/*.py`, with at least 10 entries. That test needs a server-side twin that greps `server/src` (section 6, R-19).

### 1.2 Auth and session

#### `GET /api/config` (public, no auth)
- **Client:** `api.config()`. **Caller:** `stores/club.ts` `useClub().load()`, awaited at top level in `main.ts` **before the app mounts**.
- **Sent:** nothing.
- **Read:** `name`, `theme` (accent hex), `theme_dark`, `public_url`, `timezone`. The response is `$patch`ed onto the store wholesale, so extra fields land in the store too.
  - `name` sets the wordmark in `AppShell`, `LoginPage` and `RegisterPage`, plus `document.title`, the `apple-mobile-web-app-title` meta and the runtime manifest blob (`brand.applyBrand`).
  - `theme` / `theme_dark` set the inline `--accent` / `--accent-dark` only when the Paper theme is active (`brand.applyClubAccent`).
  - `timezone` is used by `DiaryEntry.vue` and `ClubPage.vue` to format `created_at`.
  - `public_url` is not read anywhere.
- **Errors:** any failure falls back to `{name:"Bookclub", theme:"#b44a2a", theme_dark:"#8e361c", public_url:"", timezone:"UTC"}`. `loaded=true` either way. A request that hangs blocks the mount (no timeout).

#### `GET /api/auth/me`
- **Client:** `api.me()`. **Callers:**
  - `stores/session.ts` `hydrate()`, from the router `beforeEach` on the first navigation.
  - `stores/session.ts` `login()`, right after `POST /login`.
- **Read:** the whole `User`: `id`, `username`, `theme`, `color_mode`, `locale`, `avatar_url`.
  - Stored in `session.user`.
  - `theme` / `color_mode` go to `useTheme().adopt()` (ignored unless in the allowlists `paper|slate|forest|ink` and `light|dark|system`).
  - `locale` goes to `useLocale().adopt()` (ignored unless `en|de`).
- **Errors:**
  - In `hydrate()`, an `ApiError` with status 401 means logged out (`user=null`). Any other error is rethrown. The router guard catches it and also sets `user=null` and `ready=true`.
  - `ready` becomes true in `finally`, so the server is asked once per page load.

#### `POST /api/auth/register` (public)
- **Client:** `api.register({username, password, invite_code})`. **Caller:** `stores/session.ts` `register()`, called from `RegisterPage.vue`.
- **Sent:** `username`, `password` and `invite_code`, all as typed with no trimming or case change. The form has `minlength=8` on password, `required` on all three, `autocapitalize="characters"` on the invite field and `spellcheck=false` on username.
- **Read:** the whole `User` response becomes `session.user`. The server sets the session cookie, so registering logs the user in.
- **After success:**
  - `theme.persist({theme, color_mode})` and `locale.persist()` push the browser's current choices to the new account (fire-and-forget, errors swallowed).
  - Then `router.replace("/")`.
- **Errors shown** (`ApiError.message`, else `auth.couldNotCreate`):
  - mapped #5 and #6
  - unmapped pydantic text: `Username must be 2–32 characters: lowercase letters, numbers, underscore` and `Password must be at least 8 characters` (each can be `"; "`-joined with the other)

#### `POST /api/auth/login` (public)
- **Client:** `api.login({username, password})`. **Caller:** `session.login()` from `LoginPage.vue`.
- **Sent:** `username` and `password` as typed. The server lowercases and trims the username.
- **Read:** nothing; the response is a 204.
- **Then:**
  - `GET /api/auth/me`, then `theme.adopt(user)`.
  - If `locale.explicit` (the user clicked EN/DE on an auth screen this session), `locale.persist()` saves that choice to the account. Otherwise `locale.adopt(user)`.
  - Then `router.replace(safeNextPath(route.query.next))`.
- **`safeNextPath`:** non-strings, strings not starting with `/`, strings starting with `//`, and strings containing `\` all become `/`.
- **Errors:** mapped #7 and #8; fallback `auth.couldNotSignIn`.

#### `POST /api/auth/logout`
- **Client:** `api.logout()`. **Caller:** `session.logout()` from `SettingsPage.vue`.
- **Read:** 204.
- **`finally`:** `user=null` and `locale.clearExplicit()` (even if the request failed), then `router.push("/login")`.

#### `PATCH /api/auth/me/preferences`
- **Client:** `api.savePreferences(body)`. **Callers:**
  - `stores/theme.ts` `persist()` with `{theme}` (from `setTheme`), `{color_mode}` (from `setMode`), or both (after register).
  - `stores/locale.ts` `persist()` with `{locale}` (from `setLocale`, from login with an explicit locale, and from register).
- **Read:** the response `User` is **ignored** by both stores. On success, `locale.persist` sets `explicit=false`.
- **Errors:** swallowed silently (localStorage keeps the choice). The server's allowlist messages are never shown: `theme must be one of: paper, slate, forest, ink`, `color_mode must be one of: …` and `locale must be one of: en, de`.

#### `PUT /api/auth/me/avatar` (multipart)
- **Client:** `api.uploadAvatar(file)`, which sends FormData with field `file`. **Caller:** `SettingsPage.vue` `onAvatarFile`.
- **Client precheck:** `file.size > 50*1024*1024` shows the toast `t("errors.pictureTooLarge")` without a request. The input's `accept` is `image/jpeg,image/png,image/webp,image/gif`.
- **Read:** the response `User` replaces `session.user` wholesale, which updates `avatar_url` everywhere.
- **Errors:** mapped #11–#14 as toasts; fallback `settings.photoFailed`.

#### `DELETE /api/auth/me/avatar`
- **Caller:** `SettingsPage.vue` `removeAvatar`. The response `User` becomes `session.user`. Errors are toasted, fallback `settings.photoFailed`.

#### `GET /api/members/{username}/avatar`
- Never called via `request()`. `Avatar.vue` renders `<img src=avatar_url loading=lazy>` and falls back to the initial on `error`; a `src` change resets the fallback.
- **`avatar_url` sources:** `User.avatar_url`, `Member.avatar_url`, `BookReader.avatar_url`, `ClubPickReader.avatar_url`, `DiaryEntry.author_avatar_url`.

### 1.3 Favourites (mounted under `/api/auth/me/favorites`)

| Endpoint | Client fn | Caller | Sent | Read | Error handling |
|---|---|---|---|---|---|
| `GET /api/auth/me/favorites` | `myFavorites()` | `SettingsPage.vue` on mount | – | `items[]` (`position`, `book.{id,ol_work_key,title,cover_id,cover_url}`) | toast message, fallback `favorites.saveFailed` |
| `PUT /api/auth/me/favorites` | `replaceFavorites(bookIds)` | `SettingsPage.vue` (from `FavoritesEditor` `replace`) | `{book_ids: number[]}`, ordered and compacted (move ↑/↓ swaps, clear removes one, clear-all sends `[]`) | `items[]` replaces the local list | toast `bookIds.length ? favorites.saved : favorites.cleared`; errors are mapped #16–#18, fallback `favorites.saveFailed` |
| `POST /api/auth/me/favorites/{book_id}` | `addFavorite(id)` | `BookDetailPage.vue` `toggleFavorite` | `book_id` = `BookDetail.id` (numeric DB book id) | finds `items[].book.id === detail.id` and takes its `position` as `favorite_position` | **any 409** shows `t("favorites.full")` ("You already have 3 favourites. Remove one first, or reorder in Settings.") regardless of detail; other errors use the message, fallback `favorites.saveFailed` |
| `DELETE /api/auth/me/favorites/{book_id}` | `removeFavorite(id)` | `BookDetailPage.vue` | as above | response ignored; sets `favorite_position=null` | as above |

Server facts the UI depends on:
- POST is idempotent: a book that is already a favourite returns the current list.
- Positions are 1..3, compacted after remove.
- A favourite does not need to be on the user's shelf (`favorites.settingsBlurb` says so).

### 1.4 Search history (`/api/search-history`, used only by `DiscoverPage.vue`)

| Endpoint | Client fn | When | Sent | Read |
|---|---|---|---|---|
| `GET /api/search-history` | `searchHistory()` | on mount, and on each first `focus`/`click` of the search input (`openHistory`) | – | `SearchHistoryEntry[]`: `id`, `query`, `last_used_at` (last is not read). Server order: `last_used_at` desc, `id` desc; at most 20 rows. |
| `POST /api/search-history` | `rememberSearch(query)` | on submit when the trimmed query is non-empty and not "short" (see 2.6), including re-running a history entry | `{query}` (already trimmed) | the full updated list (`SearchHistoryEntry[]`) replaces `history` |
| `DELETE /api/search-history` | `deleteSearchHistory()` | "Clear history" | – | 204; clears the local list |
| `DELETE /api/search-history/{id}` | `deleteSearchHistory(id)` | "×" next to an entry | – | 204; removes the entry locally |

- **Write ordering:** POST and DELETE calls run one after another through a promise chain (`historyWrites`). A version counter (`historyVersion`) drops results from superseded calls.
- **Busy state:** `historyBusy` blocks new deletes while a write is in flight. It is released only by the latest version.
- **Error handling:**
  - GET and POST failures are swallowed; search must keep working.
  - A DELETE failure shows the toast `discover.historyDeleteFailed`. The mapped #3 string is never shown because the toast does not use `err.message`.
- **Server rules the UI relies on:**
  - The server collapses whitespace.
  - It dedupes case-insensitively (casefold) and updates the stored spelling to the latest one.
  - It keeps at most 20 entries.
  - It returns a 400 (`Search must contain between 1 and 300 characters`) the UI never shows.

### 1.5 Catalog: search, browse, detail, custom books (`/api/books`)

#### `GET /api/books/search`
- **Client:** `api.search(params, {signal})`, which builds a query string, **dropping falsy values**: `q`, `subject`, `sort`, `page`, `limit` (tested).
- **Caller:** `DiscoverPage.vue` `requestSearch(nextPage, sort, signal)`.
- **Sent:**
  - `q` = the submitted query (trimmed), or omitted.
  - `subject` = the subject chip value (`fiction|fantasy|mystery|romance|science_fiction|history|biography|horror|young_adult`), or omitted.
  - `sort` is always sent: the user's pick (`readinglog|new|title`), otherwise `relevance` when there is a query and `readinglog` when there is only a subject.
  - `page` starts at 1.
  - `limit` is never sent, so the server default is 24.
- **Read:**
  - `items[]` (SearchHit): `ol_work_key`, `title`, `authors`, `cover_id`, `cover_edition_key`, `isbn`, `cover_url`, `on_shelf`, `club_pick`.
  - `page`, `has_more`.
  - `shelf_id`, `year` and `custom` are **not** read by Discover.
- **Status branching** (`classifyError`):
  - 400 is shown as "short" (title `discover.shortTitle`).
  - 429 is "rate" (`discover.rateTitle`).
  - 502 is "unavailable" (`discover.unavailableTitle`).
  - Anything else is "generic". A non-`ApiError` gets the message `discover.searchFailed`.
- **Popular fallback:** if a reset load of page 1 with sort `readinglog` fails as "unavailable" (502), the page retries once with `relevance` and shows `discover.popularUnavailable`.
- **Unmapped server strings that surface here:**
  - `Need at least 3 characters. Add the author, or paste an ISBN.`
  - `Search is too long`
  - `Could not search the library right now. Try again.`
  - `Could not load the library right now. Try again.`
  - `The library is busy. Wait a few seconds and try again.`
  - `The library is busy. Try again in {N} seconds.`
  - `That is not a valid subject.`

#### `GET /api/books/trending?limit=N`
- **Client:** `api.trending(limit=12, init)`. **Caller:** `DiscoverPage.vue` (browse surface and retry), always with `limit=14`.
- **Read:** `items[]`, with the same SearchHit fields as search.
- **Errors:** message shown inline with "Try again"; fallback `discover.reachFailed`.

#### `GET /api/books/subjects/{subject}?page=P&limit=L`
- **Client:** `api.subject(subject, page=1, limit=12, init)`. The subject is `encodeURIComponent`ed (tested: `science fiction` becomes `science%20fiction`).
- **Caller:** `DiscoverPage.vue`. It always sends `page=1&limit=14` for the 5 browse rows (`fiction`, `science_fiction`, `mystery`, `history`, `fantasy`), 2 at a time (`SHELF_CONCURRENCY=2`).
- **Read:** `items[]`. Errors become a row error with "Try again" and "Search it instead"; fallback `discover.shelfLoadFailed`.

#### `GET /api/books/works/{work_id}`
- **Client:** `api.book(workId, init)`. The path uses the **bare work id** (`OL1168007W`, `BCdeadbeef01`, `ISBN978…`, `GB…`), never `/works/…` (tested).
- **Callers:**
  - `BookDetailPage.vue` `load()`, with an abort signal that is aborted on route change and unmount.
  - `refreshClubRating()` after a rating change, without a signal.
- **Sent:** `route.params.workId` verbatim; the server canonicalises an `ISBN…x` id to upper case.
- **Read (BookDetail):**
  - Identity and metadata: `id` (for favourites), `ol_work_key`, `title`, `authors`, `cover_id`, `cover_url`, `year`, `description` (clamped above 420 chars with "Read more"), `subjects[]` (rendered as chips linking `/discover?q=<subject>`).
  - The viewer's shelf: `on_shelf`, `shelf_id`, `rating`, `take`, `dnf_reason`, `progress`.
  - Club context: `club_pick` (kicker; disables "Set as club pick"), `readers[]` (`username`, `status`, `rating`, `progress`, `avatar_url`), `club_rating` (shown with `toFixed(1)`), `rating_count`.
  - `custom` and `favorite_position`.
  - `refreshClubRating` copies only `club_rating`, `rating_count` and `readers`.
- **Errors:** an error state with "Try again" and "Back to Discover"; fallback `book.loadFailed`.
- **Unmapped strings:**
  - `No such book.`
  - `Could not load that book right now. Try again.`
  - `The library is busy. Wait a few seconds and try again.` (Google mode)
- After the shown "still loading" seconds (`loadWaitSec >= 2`), the page shows `book.stillLoading`.
- **Side request:** after every successful load the page also calls `GET /api/pick`, only to read `timezone` for the "Set as club pick" meeting sheet.

#### `POST /api/books/works/{work_id}/refresh`
- **Client:** `api.refreshBook(workId)`. **Caller:** `BookDetailPage.vue` `refreshDetails`. It appears as the "Refresh details" link (non-custom books) and the "Refresh from Open Library" link (shown when the book has no cover, is not custom, and the id is `OL…W`).
- **Read:** the full BookDetail replaces the page state.
- **Errors:** toast; fallback `book.refreshFailed`.
- **Unmapped strings:**
  - `This book was added by the club, not Open Library.`
  - `This book was added from Google Books. Set GOOGLE_BOOKS_API_KEY to refresh.`
  - `No such book.`
  - `Could not load that book right now. Try again.`

#### `POST /api/books/custom`
- **Client:** `api.createCustomBook(body)`. **Caller:** `components/AddBookSheet.vue`, opened from Discover's empty and error states with the submitted query as the initial title.
- **Sent:**
  - `title`: trimmed and required. The client blocks an empty title with `addBook.titleRequired`; `maxlength=500`.
  - `authors`: trimmed, `maxlength=300`.
  - `year`: a number, or `null` when empty or not finite (`<input type=number min=1 max=3000>`). Tested: `"2024"` is sent as `2024`.
  - `description`: trimmed, `maxlength=4000`.
- **Read:** only `ol_work_key`. The sheet closes and `router.push("/book/" + workId(ol_work_key))` runs (tested: `/book/BCdeadbeef01`).
- **Errors:** inline; fallback `addBook.failed`. Unmapped: `A title is required`, and the year range message from pydantic.

### 1.6 Shelf (`/api/shelf`)

#### `GET /api/shelf` (own shelf)
- **Client:** `api.myShelf()`. **Callers:**
  - `ShelfPage.vue` `load()`, and again after an undo re-add.
  - `PhysicalShelfPage.vue` `load()`, in parallel with the layout.
- **Read:**
  - `items[]` (ShelfItem): `id`, `status`, `position`, `shelf_position`, `finished_at`, `rating`, `take`, `dnf_reason`, `progress`.
  - `book.{id, ol_work_key, title, authors, cover_id, cover_url, year, pages, spine_color}`.
  - `favorites[]` and `timezone`.
  - `user` is **not** read on the own shelf; `ShelfPage` uses `session.user` for the header.
  - `started_at` and `updated_at` are never read.
- **Errors:** inline; fallback `shelf.loadFailed` or `physicalShelf.loadFailed`.

#### `GET /api/shelf?username={u}` (another member's shelf)
- **Client:** `api.friendShelf(username)`, with the username `encodeURIComponent`ed. **Callers:** `MemberShelfPage.vue`, `MemberPhysicalShelfPage.vue`.
- **Read:** `user.username` (replaces the route param, so the server's canonical lower-case wins), `user.avatar_url`, `items[]`, `favorites[]`, `timezone`.
- **Errors:** inline; fallback `memberShelf.loadFailed`. Unmapped: `No member with that name`.

#### `POST /api/shelf`
- **Client:** `api.addToShelf(body)`. **Sent:** `ol_work_key` (`/works/…`), `title`, `authors`, `cover_id`, `cover_url`, `year` and `status`, plus the optional FinishNote fields `rating`, `take`, `dnf_reason`, `progress`.
- **Callers:**
  - `HomePage.vue` `startReading`: the current pick's book with `status: currently_reading`.
  - `BookDetailPage.vue` `setStatus(status)` when not on the shelf, and the undo after remove with the previous `status`, `rating`, `take`, `dnf_reason` and `progress`.
  - `ShelfPage.vue` undo after remove: the previous `status`, `rating`, `take`, `dnf_reason` and `progress`. It does **not** send `finished_at`/`started_at`, so an undo loses the reading dates.
- **Read on 201:**
  - Home reads `status` and `id`.
  - BookDetail reads `id`; its undo also reads `status`, `rating`, `take`, `dnf_reason` and `progress`.
  - The ShelfPage undo ignores the response and refetches the shelf.
- **409 contract (load-bearing):** the body is `{"detail":"Already on your shelf","item":<ShelfItemOut>}` and the client exposes it as `ApiError.item` (tested).
  - Home sets `on_shelf`/`shelf_id` from `item` and toasts `home.alreadyOnShelf` with the localised status.
  - BookDetail adopts `item.id`/`item.status` and **re-runs `setStatus`**, which becomes a PATCH.
- **Other unmapped strings:**
  - `ol_work_key must be a /works/ key`
  - `title is required`
  - `Keep the take to 140 characters`
  - `Keep the reason to 200 characters`
  - pydantic range messages for rating 1–5 and progress 0–100
  - `Could not save that book`

#### `PATCH /api/shelf/{id}`
- **Client:** `api.patchShelf(id, body)`. Body type: `{status?, position?, shelf_position?, finished_on?: string|null, rating?, take?, dnf_reason?, progress?}`.

| Caller | Body sent | Read from response |
|---|---|---|
| `HomePage.saveProgress` | `{progress}` (slider 0–100, step 5) | `progress` into my reader row; then reloads the diary (spoiler shields depend on progress) |
| `HomePage.beginReading` | `{status:"currently_reading", position:0}` | `status`; then reloads the page |
| `BookDetailPage.setStatus` | `{status, position:0}` | **nothing**; only sets `on_shelf=status` locally, so the rating/take/progress the server just cleared stay stale in the page |
| `BookDetailPage.patch` (progress slider) | `{progress}` | `rating`, `take`, `dnf_reason`, `progress` |
| `BookDetailPage.rate` | `{rating: n}`, or `{rating: null}` when the same star is tapped again | same; then `GET /api/books/works/{id}` refreshes `club_rating`/`readers` |
| `BookDetailPage` "Save take" | `{take}` (`maxlength=140`) | same |
| `BookDetailPage` "Save reason" | `{dnf_reason}` (`maxlength=200`) | same |
| `ShelfPage.onReordered` (drag, or prev/next in the sheet) | `{shelf_position: i}`, plus `finished_on: "YYYY-MM-01"` when a finished book was dropped in a different month (`null` if the target month is ""). Tested exact bodies: `{shelf_position:0}` and `{shelf_position:0, finished_on:"2026-08-01"}`. | the whole item replaces the row by `id` |
| `ShelfPage.changeStatus` | `{status, position:0}` (tested) | the whole item replaces the row; the client mirrors the server's per-status `position` shifts (2.7) |
| `ShelfPage.saveDate` | `{finished_on: "YYYY-MM-DD"}`, built from month precision `"<YYYY-MM>-01"` or day precision; `{finished_on:null}` for "Clear date" (tested) | the whole item replaces the row |

- **Errors:** toasts; fallback `book.saveFailed` or `shelf.moveFailed`.
- **Mapped:** #1 and #2.
- **Unmapped:**
  - `Nothing to update`
  - `Book not on your shelf`
  - `Keep the take to 140 characters`
  - `Keep the reason to 200 characters`
  - pydantic messages (for example `finished_on` not a date)

#### `DELETE /api/shelf/{id}`
- **Client:** `api.removeFromShelf(id)`. **Callers:** `BookDetailPage.remove` (no confirmation; an undo toast with a 7 s action) and `ShelfPage.confirmRemove` (after a confirmation sheet; undo toast).
- **Read:** 204.
- **Errors:** fallback `shelf.removeFailed`. Unmapped `Book not on your shelf`.
- **Server side effect the UI relies on:** the id is removed from the saved physical layout in the same transaction.

#### `POST /api/shelf/import` (multipart)
- **Client:** `api.importGoodreads(file)`, FormData field `file`, input `accept=".csv,text/csv"`. **Caller:** `SettingsPage.vue`.
- **Read:**
  - `imported` and `skipped` go into the toast: `settings.importedSkipped` / `importedBooks` / `skippedOnly` / `nothingToImport`.
  - `skips[]` (`title`, `reason`) is listed as `"{title || 'Untitled'} — {reason}"`. **`reason` is raw English from the server** (`"No match"`, `"Already on your shelf"`, …). The literal `"Untitled"` is hard-coded English in the template.
- **Errors:** toast; fallback `settings.importFailed`.
- **Unmapped:**
  - `That file is too large. Keep the export under 1.5 MB.`
  - `The file must be a UTF-8 Goodreads CSV export`
  - `That does not look like a Goodreads export`
  - `The library is busy. Wait a few seconds and try again.` (429)

#### `GET /api/shelf/physical-layout[?username={u}]`
- **Client:** `api.physicalShelfLayout(username?)`. **Callers:**
  - `PhysicalShelfPage.vue` (own layout, no param).
  - `MemberPhysicalShelfPage.vue` (with the username, tested).
- **Read:** the full `PhysicalShelfLayout`: `rows: (number|string)[][]`, `material`, `lights`, `spines: {[shelfEntryId]: {color?, width?, display?}}`, `decorations: {["decor:…"]: {kind, color}}`. It is always passed through `reconcileLayout(saved, items)` before rendering (2.8).
- **Errors:** inline with "Retry"; the editor is **locked** (no edits possible, so a default can never overwrite the saved layout; tested). Unmapped: `No member with that name`.

#### `PUT /api/shelf/physical-layout`
- **Client:** `api.savePhysicalShelfLayout(layout)`. **Caller:** `PhysicalShelfPage.persist()`, the only writer.
- **Sent:** a deep JSON copy of the full layout. `rows` hold numeric shelf entry ids and `"decor:<uuid>"` strings; `spines` contains only books with a custom `color`/`width`/`display`; `decorations` contains only placed decorations.
- **Read:** the echoed layout is reconciled again and becomes the state.
- **Errors:** roll back to the previous layout, notice `physicalShelf.saveFailed`, toast the message.
- **Unmapped:**
  - `Physical shelf books must be on your shelf`
  - `At most 2100 items fit on the physical shelf`
  - `At most 2000 books fit on the physical shelf`
  - `Each item can only appear once on the physical shelf`
  - `Spine appearances must refer to shelf entry IDs`
  - `Each decoration must appear exactly once on the physical shelf`
  - pydantic pattern and range messages

#### `POST /api/shelf/spine-colors`
- **Client:** `api.spineColors()`. **Caller:** `PhysicalShelfPage.sampleCoverColors()` after load, for up to 5 rounds.
- **Loop:**
  - Each round runs only while some item has `book.spine_color == null && book.cover_url`.
  - It stops when `remaining` is 0 or on any error (silently).
- **Read:** `colors: {[bookId as string]: "#rrggbb"}`, written onto every item with that `book.id`, and `remaining: number`.
- Because the server serialises a stored `""` (no usable cover) as `null`, the client cannot tell "not sampled yet" from "no usable cover". The loop therefore fires at least one request on every visit while any such book exists (the server answers `remaining: 0`).

### 1.7 Members and invites

#### `GET /api/members`
- **Caller:** `ClubPage.vue` `loadMembers()`.
- **Read:** `Member[]`: `username`, `avatar_url`, `currently_reading_count`, `currently_reading_preview[]` (`title`, `cover_id`, `cover_url`, `ol_work_key`).
- **The server excludes the viewer.** The UI renders "you" first from `session.user`, labels the count `members.length + 1`, and shows the "You're the only one here" empty state (with a link to `/settings` to mint an invite) when the list is empty.
- **Errors:** inline; fallback `club.loadFailed`.

#### `GET /api/invites`
- **Caller:** `SettingsPage.vue` on mount and after minting.
- **Read:** `Invite[]`: `code`, `used`, `used_by`. `created_at` is not read. The server returns only invites **created by the viewer**, newest first.
- **The UI:** shows the "N unused" count, plus "Copy" for unused codes and "Used by {name}" for used ones.
- **Errors:** inline; fallback `settings.invitesLoadFailed`.

#### `POST /api/invites`
- **Caller:** `SettingsPage.mint()`.
- **Read:** `code`. The list is reloaded and the code is copied with `navigator.clipboard.writeText`. The toast is `settings.inviteCopied`; if the clipboard fails, the toast shows the raw code.
- **Errors:** fallback `settings.inviteFailed`.

### 1.8 Club pick (`/api/pick`)

#### `GET /api/pick`
- **Callers:**
  - `HomePage.vue` `load()`, in `Promise.all` with history.
  - `ShelfPage.vue` `load()`, to highlight the pick's book with the "Club" badge (`book.ol_work_key`).
  - `BookDetailPage.vue`, only for `timezone`.
- **Read (`ClubPickCurrent`):** `pick` (nullable) and `timezone`.
- **Read from `pick`:**
  - `book.{ol_work_key, title, authors, cover_id, cover_url, year}`.
  - `set_by`, `note`, `meeting_at`, `meeting_local` (prefills the meeting sheet), `meeting_label` (fallback when `formatClubDate` fails).
  - `on_shelf`, `shelf_id` (the viewer's own entry).
  - `readers[]`: `username`, `status`, `rating`, `take`, `progress`, `avatar_url`. The viewer's own row is found by `username === session.user.username` and supplies the progress slider value.
  - **Not read:** `id`, `started_at`, `ended_at`, `reading[]`, `finished[]`, `readers[].dnf_reason`.
- **Errors:**
  - Home shows the message (fallback `home.loadFailed`).
  - Shelf and BookDetail swallow errors (pick treated as null, timezone left as "UTC").
- **Server side effect:** GET may schedule a meeting-reminder push in the background (the push router stays in the backend).

#### `GET /api/pick/history`
- **Caller:** `HomePage.vue`.
- **Read:** `items[]` (ClubPick: `id`, `book.*`, `meeting_at`, `meeting_label`, `set_by`). `timezone` is not read; Home uses the one from `GET /api/pick`.
- **Order:** the server sorts by `ended_at` desc, then `id` desc; the UI keeps that order. Each tile's subtitle is the meeting date, else `meeting_label`, else `set_by`.

#### `PUT /api/pick`
- **Client:** `api.setClubPick(body)`. **Sent:** `ol_work_key`, `title`, `authors`, `cover_id`, `cover_url`, `year`, `meeting_at`. `meeting_at` is the `<input type=datetime-local>` value `"YYYY-MM-DDTHH:MM"` (no offset, meant in the club timezone) or `null`. **`note` is never sent** by any caller.
- **Callers:**
  - `HomePage.saveMeeting`: the *current* pick's book again, which only edits the meeting.
  - `BookDetailPage.confirmClubPick`: sets the club pick to this book.
  - `ShelfPage.confirmClubPick`: same, from a shelf item.
- **Read:**
  - Home ignores the response and reloads.
  - BookDetail sets `club_pick=true`.
  - Shelf stores the response as `clubPick` (for the badge).
- **Errors:** toasts; fallbacks `home.meetingFailed`, `book.setPickFailed`, `shelf.setPickFailed`.
- **Unmapped:**
  - `meeting_at must be a date and time`
  - `ol_work_key must be an Open Library work key`
  - `title is required`
  - `note must be 280 characters or fewer`
  - `Could not save the club pick`
- **Hidden server behaviour:** re-PUTting the current book overwrites `note` with `""` (the default) and `set_by` with the caller. Editing the meeting from Home therefore **wipes the pick note and changes "Chosen by"**. See risk R-11.

#### `DELETE /api/pick`
- `api.clearClubPick()` exists in the client but has **no caller**.

### 1.9 Overlap and next-up vote

#### `GET /api/overlap[?include_reading=true]`
- **Client:** `api.overlap(includeReading)`. The parameter is present only when true. **Caller:** `ClubPage.vue`, on mount and on each toggle of the "Include Reading" chip.
- **Read:** `items[]`: `book.{ol_work_key, title, authors, cover_id, cover_url, year}` and `members[]` (`username`, `status`). `count` and the response's `include_reading` are not read.
- **Order:** the server sorts by count desc, then lower-case title.
- **Errors:** list cleared; inline message, fallback `club.overlapFailed`.

#### `GET /api/vote`
- **Callers:**
  - `components/NextUpVote.vue` `load()`, on mount and after nominating from overlap.
  - `HomePage.vue` after the pick loads (errors leave `vote=null`).
- **Read:**
  - `nominations[]`: `id`, `book.{ol_work_key, title, authors, cover_id, cover_url}`, `nominated_by`, `votes`, `voters[]`, `mine` (I voted for this one).
  - `nomination_limit`, and `timezone` (for the confirm-winner sheet).
- **Not read:** `vote_id`, `my_vote_id`, `can_nominate`, `closes_at`, `closes_local`, `closes_label`, `not_voted`, `voted_count`, `member_count`, `leader_id`.
- **Errors:** inline; fallback `vote.loadFailed`.
- **Server side effect:** GET can auto-close a vote whose deadline passed. That replaces the pick, though the UI has no deadline control.

#### `POST /api/vote/nominations`
- **Client:** `api.nominate(VoteBook)`. **Sent:** `ol_work_key`, `title`, `authors`, `cover_id`, `cover_url`, `year`.
- **Callers:**
  - `ClubPage.vue` (overlap row "Nominate", then reloads the vote section).
  - `BookDetailPage.vue` ("Nominate"; no state refresh).
  - `ShelfPage.vue` (status sheet, "More actions", "Nominate").
- **Read:** the response (a full `NextUpVote`) is ignored by all three.
- **Errors:** toasts; fallbacks `shelf.nominateFailed` / `book.nominateFailed`.
- **Unmapped:**
  - `This vote already has enough books`
  - `That book is already the club pick`
  - `That book is already nominated`
  - pydantic `ol_work_key`/`title` messages

#### `POST /api/vote/cast`
- **Client:** `api.castVote(nominationId)`, body `{nomination_id}` (tested exact JSON, method, credentials and content type). **Caller:** `NextUpVote.cast`.
- **Read:** the full `NextUpVote` replaces the state.
- Not a toggle: casting on your current choice is a no-op, and casting on another moves the ballot.
- **Errors:** toast; fallback `vote.saveFailed`. Unmapped: `No open next-up vote`, `No nomination with that id`.

#### `POST /api/vote/apply`
- **Client:** `api.applyWinner(nominationId, meetingAt?)`, body `{nomination_id, meeting_at: meetingAt ?? null}`. **Caller:** `NextUpVote.confirm`, after the MeetingSheet.
- **Read:** `vote` (the new, empty open-vote state) and `pick` (not read).
- Toasts `vote.confirmed` with the title and emits `applied`. **Nobody listens** to `applied` in `ClubPage`, so Home is stale until the user navigates.
- **Errors:** toast; fallback `vote.confirmFailed`. Unmapped: `No open next-up vote`, `No nomination with that id`, `meeting_at must be a date and time`.

### 1.10 Book diary (per-book entries with replies; backend table `book_posts`)

All of these are used only by `components/BookDiary.vue`, which is mounted on `BookDetailPage` (full) and `HomePage` (current pick, `preview=3`, heading `home.diary`). Display lives in `components/DiaryEntry.vue` and `SpoilerControl.vue`.

#### `GET /api/books/works/{work_id}/posts`
- **Client:** `api.diary(workId)`.
- **Read:** `book_id` (null when the book is not in the DB yet), `my_progress`, `my_status`, and `items[]` (top-level entries, `created_at` ascending, each with `replies[]` ascending).
- **Entry fields read:** `id`, `author`, `author_avatar_url`, `mine`, `body`, `deleted`, `spoiler_upto`, `progress_at`, `status_at`, `author_rating`, `parent_id`, `created_at`, `created_label`, `edited`, `reactions[]` (`emoji`, `count`, `mine`, `users[]`), `replies[]`.
- `ol_work_key` and `timezone` are not read; formatting uses `useClub().timezone`.
- **Errors:** inline; fallback `diary.loadFailed`. Unmapped: `No such book.`

#### `POST /api/books/works/{work_id}/posts`
- **Client:** `api.addDiaryEntry(workId, body)`.
- **New entry:** `{body (trimmed, ≤1000), spoiler_upto: number|null, book?}`. `book` is sent only when `diary.book_id` is null: `{title, authors, cover_id, cover_url, year}` from the page.
- **Reply:** `{body, spoiler_upto, parent_id}` (tested JSON for the reply shape).
- **Read:** the created `DiaryEntry` is appended locally (to `items`, or to `parent.replies`). After the first-ever entry on a book (`book_id` null), the diary reloads.
- **Errors:** toasts; fallbacks `diary.postFailed` / `diary.replyFailed`.
- **Unmapped:**
  - `Write a short take, quote, or meeting note`
  - `Keep it to 1000 characters`
  - `Nobody has this book yet; send its title to start the diary`
  - `That entry is not on this book`
  - `Reply to the entry itself, not to a reply`
  - `That entry was deleted`
  - `Could not save that entry`
  - pydantic range message for `spoiler_upto` 0–100

#### `PATCH /api/posts/{id}`
- **Client:** `api.editDiaryEntry(id, {body, spoiler_upto})`. The client always sends both fields; `spoiler_upto: null` clears the flag.
- **Read:** the updated entry. For a top-level entry the client keeps its **local** `replies` and ignores the server's.
- **Errors:** toast; fallback `diary.saveFailed`. Unmapped: `No entry with that id`, `You can only edit your own entries` (403), `Nothing to update`, the body messages.

#### `DELETE /api/posts/{id}`
- 204. Locally, a top-level entry that has replies becomes a tombstone (`body:""`, `deleted:true`, `spoiler_upto:null`, `reactions:[]`); otherwise the entry is removed.
- Delete needs two taps within 4 s (`DiaryEntry.askDelete`).
- **Errors:** fallback `diary.deleteFailed`. Unmapped: `No entry with that id`, `You can only delete your own entries` (403).

#### `POST /api/posts/{id}/reactions`
- **Client:** `api.reactToEntry(id, emoji)`, body `{emoji}`. This toggles the reaction.
- The emoji is one of `❤️ 👍 😂 😮 🔥 📚` (`diary.REACTIONS`, identical to the server's `REACTIONS`; `❤️` is U+2764 U+FE0F).
- The picker hides emojis the viewer already used. Tapping an existing chip toggles it.
- **Read:** the updated entry replaces it in place.
- **Errors:** fallback `diary.reactFailed`. Unmapped: `Pick one of the club reactions`, `No entry with that id`.

#### `GET /api/diary?limit=L[&before=ID]` (club-wide "Recently written" feed)
- **Client:** `api.diaryFeed(before?, limit=8)`. `before` is added only when truthy (tested: `/api/diary?limit=8` and `/api/diary?limit=5&before=42`).
- **Caller:** `ClubPage.vue` `loadFeed(more)`. "Show more" passes `before` = the last item's `entry.id`, and the page appends.
- **Read:**
  - `items[]`: `entry.{id, author, mine, body, deleted, spoiler_upto, progress_at, status_at, author_rating, created_at, created_label, reactions[].{emoji,count}}`, `book.{ol_work_key, title, cover_id, cover_url}`, `parent_author`, `my_progress`, `my_status`.
  - `has_more`. `timezone` is not read.
- **Order:** the server orders by entry id descending and excludes deleted entries.
- **Errors:** inline; fallback `club.diaryLoadFailed`.

### 1.11 Backup

- `GET /api/backup` is reached through a plain `<a href>` in Settings ("Download database"). There is no JS involvement; errors show as a raw JSON page.

### 1.12 Backend endpoints with no frontend caller (they exist and are tested in Python)

These are the inventory for #48's "back in the UI" work and for the parity checklist:

- `PATCH /api/auth/password`: the change-password form was removed in PR #3; #48 brings it back **[plan]**.
- `GET`/`PATCH /api/auth/notifications`: notification preferences; #48 lists them under account settings **[plan]**.
- `/api/push/*`: `vapid`, `subscriptions` GET/POST/DELETE, `test`. Push moves to native/Web Push in Phase 4 **[plan]**.
- `/api/quotes` GET, POST, PATCH `/{id}`, DELETE `/{id}`: #48 adds a personal `/quotes` page **[plan]**.
- `/api/activity` (GET): the club event feed; #45 "events … per club".
- `/api/stats` (GET): per-person and club stats.
- `/api/pick/{pick_id}/milestones` (GET, POST, PATCH, DELETE): reading-schedule milestones.
- `/api/pick/posts`, `/api/pick/posts/{id}`, `/api/pick/posts/{id}/reactions`, `/api/pick/{pick_id}/posts`: legacy pick notes. `db.import_legacy_pick_posts` copies them into `book_posts` on boot.
- `/api/pick/meeting.ics`: an iCalendar file for the current meeting.
- `PATCH /api/vote` (vote deadline `closes_at`) and `GET /api/vote/suggestions`.
- `GET /api/books/work/{id}`, `POST /api/books/work/{id}/refresh`, `GET /api/books/isbn/{isbn}`: an alternative detail shape and ISBN lookup.
- `DELETE /api/pick` (client function exists, no caller).
- `/api/health` and `/manifest.webmanifest`. The SPA links the manifest from `index.html`, and `brand.applyBrand` then replaces it with a runtime blob URL.

### 1.13 Outbound requests the browser makes to third parties (relevant to #43 and the DSGVO)

- **`BookCover.vue`** builds a source chain and steps to the next source on `error`, or on `load` with `naturalWidth <= 1` or `naturalHeight <= 1`:
  1. `remoteCoverUrl(imageUrl)`: only `https` URLs on `books.google.com`, `*.books.google.com`, `googleusercontent.com` or `*.googleusercontent.com`; `http://` is upgraded. Open Library CDN URLs passed as `imageUrl` are deliberately **ignored**, so the size tier stays correct.
  2. `https://covers.openlibrary.org/b/id/{cover_id}-{S|M|L}.jpg?default=false`. Sizes `xs`/`sm` use S, `md`/`lg`/`tile` use M, `fluid` uses L. Only when `cover_id > 0`.
  3. `https://covers.openlibrary.org/b/olid/{cover_edition_key}-{tier}.jpg?default=false`. Skipped for Google catalog keys.
  4. `https://covers.openlibrary.org/b/isbn/{isbn or ISBN from work key}-{tier}.jpg?default=false`. Skipped for Google catalog keys (`ISBN…`/`GB…`).
  5. If every source fails, a cloth "binding" with a monogram (colour from `bindingColor(workKey || title)`).
- **External links** (not fetched): `https://openlibrary.org/works/{OL…W}`, `https://books.google.com/books?id={vol}` and `https://books.google.com/books?vid=ISBN{isbn}` (`constants.catalogUrl`).
- **Fonts** are bundled (`@fontsource/*`). There are no other third-party requests.

---

## 2. Domain rules (client-side logic a reimplementation or the new UI must keep)

### 2.1 Boot and session

1. `main.ts` runs four steps in order:
   1. `installAssetRecovery()`.
   2. Pinia and i18n.
   3. `await useClub().load()` (`GET /api/config`).
   4. `useTheme().init()`, then `useLocale().init()`, then `app.use(router).mount("#app")`.
2. **Theme flash prevention (`index.html` inline script):**
   - It reads `localStorage["bookclub.theme"]` (allowlist `paper|slate|forest|ink`, default `paper`) and `bookclub.mode` (`light|dark`; anything else resolves `system` via `matchMedia`).
   - It reads `bookclub.locale` (`en|de`, else `de` if `navigator.language` starts with `de`, else `en`).
   - It sets `data-theme`, `data-mode` and `lang` on `<html>` before the first paint.
3. **Router guard (`router.beforeEach`):**
   - If the session is not `ready`, it runs `hydrate()` (on error: `ready=true`, `user=null`).
   - A route with `meta.auth` and no user goes to `{path:"/login", query:{next: to.fullPath}}`.
   - A route with `meta.guest` and a user goes to `/`.
4. **Login locale rule (tested):**
   - If the user touched EN/DE on an auth screen (`locale.choose`, which counts even when that locale is already showing), the choice is persisted to the account after login and wins over the server value.
   - Otherwise the server locale is adopted.
   - Logout clears the explicit flag.
5. **Theme:** `adopt(user)` applies server prefs on hydrate and login. `setTheme`/`setMode` apply locally, mirror to localStorage, and PATCH (best effort).
   - `applyClubAccent` sets an inline `--accent` only for the Paper theme; other themes clear it.
   - `applyThemeColor` copies the computed `--bg` into `<meta name="theme-color">`.
6. **Asset recovery:** on `vite:preloadError`, reload at most once per 60 s, guarded in `sessionStorage["bookclub.asset-reload-at"]`. If storage throws, it never reloads (tested).

### 2.2 Status-code branching (anything a server must reproduce)

| Where | Condition | Behaviour |
|---|---|---|
| `session.hydrate` | 401 on `/api/auth/me` | treated as logged out; any other status is rethrown and the guard treats it as logged out too |
| Home `startReading`, BookDetail `setStatus` | 409 with `item` on `POST /api/shelf` | adopt `item.id`/`item.status`; BookDetail retries as a PATCH |
| BookDetail `toggleFavorite` | 409 on `POST favorites/{id}` | fixed copy `favorites.full` |
| Discover `classifyError` | 400 / 429 / 502 / other | short / rate / unavailable / generic panels; 502 on page 1 with sort `readinglog` retries with `relevance` |
| client `request` | 204 | `undefined` |
| client `request` | non-2xx with string `detail` | translated or raw message |
| client `request` | non-2xx with structured `detail` | `errors.generic` (tested with `{detail:{code:1}}` and 500) |

### 2.3 Time and time zones

- **`formatClubDate(iso, tz, locale)`** (`i18n/dates.ts`):
  - Format: `Intl.DateTimeFormat` with BCP-47 `en-GB` or `de-DE`, `{weekday:short, day:numeric, month:short, year:numeric, hour:2-digit, minute:2-digit, timeZoneName:short}`, in time zone `tz || "UTC"`.
  - Returns `""` for empty or invalid input; on an Intl error it returns `toISOString()`.
  - Callers fall back to the server's `created_label`/`meeting_label` (English, `"%a %d %b %Y, %H:%M %Z"`) when it returns `""`.
  - **It uses `new Date(iso)`, so an offsetless ISO string is read as device-local time.** This is a latent bug with SQLite's offsetless timestamps.
- **`clubDate(iso, tz)`** (`shelf.ts`): appends `Z` when the string has no `Z`/`±hh:mm` suffix (an offsetless value is UTC), then formats `YYYY-MM-DD` in `tz` through `en-CA` parts. Used for:
  - the finished month (`readingMonth`)
  - "today" for the reading-date `max`
  - the month picker's current month
- **`relativeDay(iso)`** (`constants.ts`): whole-day difference in **device local** time.
  - `0` gives today, `1` tomorrow, `-1` yesterday.
  - Up to 13 days gives "in N days" / "N days ago"; from 14 days on, `Math.round(days/7)` weeks.
- **Meeting input:** `<input type=datetime-local>` gives `"YYYY-MM-DDTHH:MM"`, trimmed; empty means `null`.
  - The server interprets an offsetless value in the **club** timezone and stores UTC.
  - The sheet prefills from `meeting_local` (server-rendered `"%Y-%m-%dT%H:%M"` in club tz) and shows the hint `meeting.optionalTz` with the club tz.
- **Reading dates:**
  - "Month" precision sends `<YYYY-MM>-01`; "day" precision sends the date.
  - The client blocks values after "today" in the club timezone (from `shelf.timezone`), and the server re-checks with the same timezone.
  - The server stores `finished_on` at 12:00 local, converted to UTC, and clamps `started_at` to it when needed.
  - The optimistic month move sets `finished_at` to `"<YYYY-MM>-01T12:00:00Z"` locally until the server answers.
- **Month labels:** `readingMonthLabel("YYYY-MM", locale)` formats the long month and year in UTC at noon on the 1st. Tested: "Januar 2026" and "January 2026".

### 2.4 Sorting done client-side

- **Shelf `orderedShelf`:**
  - Items without `shelf_position` come first (newly added books), ordered by the status index in `STATUSES` (want, reading, finished, dnf), then `position`, then `id`.
  - Then items with a `shelf_position` in ascending order, then `position`, then `id`.
  - This mirrors the server's `place_on_shelf` ordering exactly.
- **Finished filter (`visibleShelf`):**
  - Stable sort by finished month, newest first.
  - An empty month (no date) sorts last because of `localeCompare`.
  - Manual order breaks ties within a month.
- **`ShelfBoard`:**
  - Active books (not finished) are rendered as one group in `orderedShelf` order.
  - Finished books are grouped per month, newest first.
  - The owner (not read-only) also gets extra month targets: user-added months, plus the latest month (or the current club month on an empty shelf) and the month before it, as empty drop targets.
- **Home readers:** rank `currently_reading:0, finished:1, want_to_read:2, did_not_finish:3`, then `username.localeCompare`. "N of M finished" counts readers with status finished.
- **Home "Next up" leader:** `[...nominations].sort((a,b)=>b.votes-a.votes)[0]`. That is stable over the server order `(-votes, id)`, so ties go to the lower nomination id.
- **`NextUpVote` list:** `votes` desc, then `book.title.localeCompare`. Ties can therefore put a **different** "leader" on Home and on the Club page. The `leading` style applies only to index 0 when it has `votes > 0`.
- **`FavoritesEditor`:** `position` asc, then `book.id`. Three slots are always shown; empty slots read "Empty".
- **Server order kept as is:** members (username asc, viewer excluded), overlap, pick history, diary entries (`created_at` asc), replies (asc), the feed (id desc), search history (`last_used_at` desc).

### 2.5 Pagination and dedupe

- **Discover:**
  - Infinite scroll uses an `IntersectionObserver` (`rootMargin: 240px`) plus a manual check after each page (the sentinel within `innerHeight + 240`).
  - `hasMore = result.has_more && items.length > 0`.
  - Pages after the first are **deduped by `ol_work_key`** against everything already shown.
  - A failed later page keeps the current hits, sets `moreError` and toasts `discover.loadMoreFailed`.
- **Diary feed:** keyset pagination on `before=<last entry id>`. The client limit is 8, the server default is 10 and the maximum is 50.
- **Search history UI:** shows 5 entries, "Show all" expands to everything.
  - Filter: `entry.query.toLocaleLowerCase().includes(input.trim().toLocaleLowerCase())`.
  - The dropdown is hidden when nothing matches.

### 2.6 Discover state machine (`DiscoverPage.vue`)

- **Two surfaces:** *browse* when there is no submitted query and no subject; otherwise *results*.
- **Route sync:** URL `?q=&subject=` is two-way via `router.replace({path:"/discover", query})`. Reading the route on mount decides whether a watcher fetch will happen; that avoids a double fetch (tested).
- **Short query:** a query of 1–2 characters that is not an ISBN (`/^(?:\d{9}[\dXx]|\d{13})$/` after stripping spaces and dashes) and has no subject is "short".
  - The page shows a local error and **does not call search** (tested with "it").
  - It does not save the term to history.
- **Stale responses:** a `requestSeq` counter plus an `AbortController` per load ignores stale responses (tested).
  - The current tiles stay visible during a refetch (tested for a newer search, a sort change and route catch-up).
  - "Clear" empties immediately (tested).
- **Sort chips:** Popular (`readinglog`), New (`new`), Title (`title`).
  - Picking the implied default (relevance with a query, readinglog without) clears the pick.
  - "Not on shelf" is a client-side filter on `!hit.on_shelf`.
- **Browse caching:**
  - `discoverCache.ts` holds a module-level snapshot of trending and rows that survives remounts within the SPA session, with no TTL.
  - On remount the page hydrates from the snapshot and refetches *quietly*: errors do not replace existing tiles (tested).
  - Rows load at concurrency 2 (tested: `fiction` and `science_fiction` start first, `mystery` after one resolves).
- **"Still loading/searching" notes** appear after 2 s (`STILL_WAITING_AFTER_SEC`), driven by a 500 ms interval (tested with fake timers: "2s so far").
- **Copy:** the search placeholder is "Title, author, or ISBN" (tested). The empty and miss states offer "Clear {subject}", "Back to browsing" and "Add your own book" (tested).

### 2.7 Optimistic updates and rollbacks

- **Shelf reorder (`ShelfPage.onReordered`):**
  - Ignored while `saving`, so overlapping moves are blocked (tested).
  - A month change on a non-finished book is ignored.
  - Optimistically rewrites every item's `shelf_position` to its new index and patches one item; on failure it restores the snapshot (tested).
  - On success it replaces only the patched item with the server copy.
  - The toast is `shelf.dateSaved` (month changed) or `shelf.orderSaved`.
- **Prev/next in the sheet:** works within the visible rows of the same section (same finished month, or all non-finished) and maps the move to a full-shelf index with `shelfDropPosition` (2.8).
- **Status change (`ShelfPage.changeStatus`):** after the server reply it mirrors the per-status `position` maintenance:
  - books in the target status get `position+1`;
  - books in the old status after the moved one get `position-1`.
- **Physical shelf `persist(next, previous)`:**
  - Refuses while `saving` or after a load error.
  - Sets the layout optimistically and PUTs it; on success it reconciles the echo; on failure it restores `previous` (tested for material, row removal, drag, decoration add).
  - Single-step undo (`lastUndo`) re-persists the previous layout without recording a new undo.
- **Favourites:** no optimistic state; the server list replaces the local one.
- **Diary:** server responses are merged in place (`replaceEntry`); deletions are applied locally.

### 2.8 Physical shelf algorithms (`physicalShelf.ts`; pure, tested)

**Constants:**

| Name | Value |
|---|---|
| `DEFAULT_ROWS` | 3 |
| `MAX_ROWS` | 12 |
| `ROW_CAPACITY` | 560 px |
| Decoration widths | plant 76, vase 58, candle 50, frame 74, bookend 66, globe 72 |
| `MAX_FACE_HEIGHT` | 172 |
| `MAX_STACK_HEIGHT` | 150 |
| `MAX_STACK_BOOKS` | 5 |
| Ink colours | light `#fff8ed`, dark `#2f2b26` |
| Palette | 12 cloth colours |

- **`stableHash`:** FNV-1a 32-bit over UTF-16 code units, using `Math.imul(hash ^ code, 16777619)` from offset 2166136261, then `>>> 0`.
- **`defaultSpine(item)`:**
  - The hash key is `book.ol_work_key`, or `"title:authors"` without one. Styling is per work and independent of the shelf id.
  - Colour: `book.spine_color`, else `PALETTE[hash % 12]`.
  - Width from pages: `clamp(round(24 + pages/18), 26, 64)` when pages is a finite number above 0; otherwise `32 + (hash>>>8) % 17`.
  - Height: `150 + (hash>>>16) % 41`. Variant: `(hash>>>24) % 4`.
  - Ink: the higher WCAG contrast of dark and light against the colour.
- **`reconcileLayout(layout, items)`:**
  - Board count: `min(12, saved.length || 3)`.
  - Rows beyond 12 fold into the last row.
  - Duplicates are dropped (first wins), and so are ids that are not current shelf items.
  - Decoration ids must start with `"decor:"`, be longer than the prefix, and have a record in `decorations`; only placed records are kept.
  - Spines are kept only for current item ids.
  - Unplaced items are appended in `orderedShelf` order after the last non-empty row. When the next item would overflow 560 px, packing moves on to a new row, as long as fewer than 12 rows exist.
  - Footprints: spine `width` (custom or default); `flat` uses the book height; `cover` uses `round(faceHeight*2/3)`.
  - `lights` is coerced to a boolean.
- **`moveBook(layout, id, row, index)`:**
  - The index refers to the destination after removing the object; it is clamped and truncated.
  - Every duplicate of the id is removed first.
  - Invalid input returns the **same** layout object.
- **`removeShelfRow`:** the board's objects are appended to the previous board (board 0's go to the new board 0). It cannot remove the last board; invalid indices return the same object.
- **`stackSlots`:** consecutive `flat` books pile up. A pile has at most 5 books and at most 150 px of total thickness; a non-flat object ends the pile.
- **Page rules:**
  - At most 100 decorations and at most 12 rows.
  - The spine width slider runs 26–64. A colour must match `^#[0-9a-f]{6}$` (case-insensitive) before save.
  - A style equal to the defaults is deleted from `spines`, so a plain spine stores nothing (tested).
  - New decoration ids are `"decor:" + crypto.randomUUID()`, or 32 hex characters from `getRandomValues` when `randomUUID` is unavailable (insecure context).

### 2.9 Spoiler shielding (client-only; the server always sends the body)

- **`isShielded(entry, myProgress, myStatus)`** is false when the entry is deleted, is mine, or has `spoiler_upto == null`. It is also false when my status is finished or did_not_finish. Otherwise it is `(myProgress ?? 0) < spoiler_upto`.
- **Where the body still reaches the DOM:**
  - In `DiaryEntry.vue`, a shielded body **is still rendered in the DOM**, blurred by CSS, with a reveal button: "Safe up to N%" and "You're at N%".
  - In `ClubPage`'s feed, a shielded preview is replaced by `diary.spoilerPreview` ("Spoiler-flagged note"), so the secret text is **not in the DOM** (tested). Expand is disabled for shielded rows.
- **`defaultSpoilerUpto`:** 100 if I finished; my progress if I am reading with a progress value; otherwise null. The composer resets to it after each post unless the user touched the control.
- **`positionMarker`:**
  - Finished: "Finished ★★★★☆" (or "Finished" without a rating).
  - DNF: "DNF".
  - Reading with a progress value: "at N%".
  - Want to read: "before starting".
  - Otherwise `""`.
- **`excerpt`:** collapses whitespace and trims to at most 140 characters including the trailing `…`.
- **`countEntries`:** non-deleted top-level entries plus all replies.
- **Server inputs:** `my_progress` is 100 when finished, `progress` when reading, null otherwise. `author_rating` is set only when the author's shelf entry is finished.

### 2.10 Drafts (`drafts.ts`, tested)

- Keys:
  - `bookclub.draft.<username|anon>.<workId>` for the composer.
  - `…<workId>.reply.<entryId>` for replies.
- A whitespace-only draft is removed. A storage error is swallowed.
- A draft is cleared right after a successful post, before the field clears.

### 2.11 Toasts (`stores/toast.ts`)

- Plain toasts last 3200 ms, toasts with an action 7000 ms; at most 3 are visible and the oldest is dropped.
- `run(id)` dismisses the toast, then awaits its action. Used for Undo and "Write a few words" (focuses the diary composer after rating).

### 2.12 Client-side limits mirrored from the server

| Field | Client limit |
|---|---|
| take | `maxlength` 140 |
| dnf_reason | `maxlength` 200 |
| diary body and reply | `maxlength` 1000 |
| custom book title / authors / notes | 500 / 300 / 4000 |
| custom book year | 1–3000 |
| password on register | `minlength` 8 |
| avatar | 50 MiB precheck |
| progress | 0–100, step 5 |
| spoiler slider | 0–100, step 5 |
| favourites | 3 |
| physical shelf rows | 12 |
| physical shelf decorations | 100 |
| spine width | 26–64 |

### 2.13 Work-key helpers (`constants.ts`)

- `workId("/works/X") = "X"`; `bookPath(key) = "/book/" + workId(key)`.
- **Recognisers:**
  - `isClubWorkId`: `^BC[a-f0-9]{10}$`
  - `isOpenLibraryWorkId`: `^OL\d+W$`
  - `isIsbnWorkId`: `^ISBN(?:\d{9}[\dXx]|\d{13})$`
  - `isGoogleWorkId`: `^GB[A-Za-z0-9_-]{1,40}$`
- `isbnFromWorkKey` accepts `ISBN`/`isbn` prefixes and upper-cases a trailing `x`.
- **Uses:**
  - Choosing the catalog link: Open Library or Google Books.
  - The custom-book label `book.addedByClub`.
  - Whether to show the "refresh from OL" prompt.
  - Whether `BookCover` may use OL edition/ISBN fallbacks.

---

## 3. Data

### 3.1 Server tables behind each call site (indirect; the frontend never touches the DB)

| Call site group | Tables read | Tables written |
|---|---|---|
| auth/me, register, login, preferences, avatar | `users`, `invites` | `users` (theme, color_mode, locale, avatar, avatar_mime, avatar_updated_at), `invites` (used_by_id, used_at), `events` (`member_joined`) |
| favourites | `user_favorites`, `books` | `user_favorites` |
| search history | `search_history` | `search_history` (upsert on `(user_id, normalized_query)`, prune to 20) |
| search, trending, subjects, detail, refresh, custom | `books`, `shelf`, `club_picks`, `user_favorites` | `books` (details_fetched_at, description, subjects, pages, ol_rating, cover; custom `BC…` rows), plus in-process caches |
| shelf GET/POST/PATCH/DELETE, import | `shelf`, `books`, `users`, `user_favorites` | `shelf`, `books` (upsert from **client-supplied** title/authors/cover/year), `events` (`shelf_*`, `progress`), `physical_shelf_layouts` (on delete) |
| physical layout, spine colours | `physical_shelf_layouts`, `shelf`, `books` | `physical_shelf_layouts` (JSON blob), `books.spine_color` |
| members, invites | `users`, `shelf`, `books`, `invites` | `invites` |
| pick, history | `club_picks`, `books`, `shelf`, `users` | `club_picks`, `books`, `events` (`pick_set`) |
| overlap | `shelf`, `books`, `users` | – |
| vote | `next_up_votes`, `next_up_nominations`, `next_up_ballots`, `books`, `users` | the same three vote tables, `club_picks` (apply, auto-close), `events` (`nominated`, `voted`, `vote_closed`) |
| diary, feed, reactions | `book_posts`, `book_post_reactions`, `shelf`, `books`, `club_picks`, `users` | `book_posts` (tombstone via `deleted_at`), `book_post_reactions`, `books` (first post) |
| backup | the whole SQLite file | – |

### 3.2 Semantics and quirks the UI depends on

- **Statuses:** `want_to_read | currently_reading | finished | did_not_finish`. That order is `STATUSES` and is used as the initial shelf order.
- **Shelf fields:**
  - `rating` (1–5) and `take` exist only while the book is finished; `dnf_reason` only while DNF; `progress` (0–100 %) only while reading. The server clears them on every status change.
  - `position` is per-status order, maintained by the server. `shelf_position` is the optional manual whole-shelf order; null means "not arranged yet".
  - `started_at`/`finished_at` are stamped by status transitions; moving back to want_to_read clears both.
- **Timestamps:** SQLite returns naive datetimes, so pydantic emits ISO strings **without an offset** (for example `"2025-12-31T23:30:00"`). `clubDate` handles them as UTC; `formatClubDate` does not (2.3).
- **`spine_color`:** the DB has three states: NULL (not sampled), `""` (no usable cover) and `"#rrggbb"`. The API collapses `""` to `null`.
- **`cover_id`:** values `<= 0` mean "none" (Open Library uses -1). `cover_url` from the API is the Google image URL, or an OL CDN `-L` URL built from `cover_id`, or (non-Google mode) an OL ISBN URL. `BookCover` ignores OL CDN `cover_url` and rebuilds a size-appropriate one from `cover_id`.
- **`avatar_url`:** `"/api/members/{username}/avatar?v={unix seconds of avatar_updated_at}"`, or null. The `v` busts the 1-day private cache.
- **`ol_work_key`:** `/works/<id>`, where the id is `OL…W` (Open Library), `BC` + 10 hex (club-added), `ISBN…` (Google with ISBN) or `GB…` (Google volume).
- **`BookDetail.club_rating`:** the mean of `rating` over **every user's** finished entries (instance-wide), rounded to 1 decimal. `rating_count` is the number of those ratings.
- **Viewer exclusions:** `readers` excludes the viewer; `members` excludes the viewer.
- **`ClubPick.readers`:** **all** users with the book on their shelf, including the viewer, ordered by `status, id`. The UI re-sorts.
- **`favorite_position`:** 1..3 or null.
- **Diary entries:**
  - Replies are one level deep; `parent_id` always points at a top-level entry.
  - Deleting a top-level entry that has replies leaves a tombstone (`deleted:true`, empty body, no reactions, `edited:false`).
  - `edited` means `updated_at` is set.
  - `progress_at`/`status_at` snapshot the author's position at write time.
- **Legacy data migration that changes what the UI shows:** `db.import_legacy_pick_posts` (idempotent, every boot) copies each `club_pick_posts` row into `book_posts` as a top-level entry on the pick's book, with its reactions, marked by `legacy_post_id`. Legacy pick notes therefore appear in today's book diary and feed. The importer (#38) must not import them twice: once as pick posts and once as diary entries.
- **Physical layout JSON:** keys are **shelf entry ids** (`shelf.id`, integers), not book ids. This must survive the importer's id remapping (risk R-6).

### 3.3 Browser-side storage (owned by the frontend)

| Key | Storage | Value | Writer / reader |
|---|---|---|---|
| `bookclub.theme` | localStorage | `paper\|slate\|forest\|ink` | theme store, `index.html` boot script |
| `bookclub.mode` | localStorage | `light\|dark\|system` | same |
| `bookclub.locale` | localStorage | `en\|de` | `i18n.applyLocale`, `detectLocale`, boot script |
| `bookclub.draft.<user>.<workId>[.reply.<id>]` | localStorage | draft text; every key is forgotten on sign-out here, ending the current session and account deletion, not on an expired session (security review 2026-10-28 #12) | `drafts.ts` via `BookDiary` |
| `bookclub.asset-reload-at` | sessionStorage | ms timestamp | `assetRecovery.ts` |
| Discover browse snapshot | module memory | trending and rows | `discoverCache.ts` |

All storage access is wrapped in try/catch.

### 3.4 i18n catalogs

- `en.json` and `de.json` hold **516 leaf keys** each, in 24 sections:

  | Section | Keys | Section | Keys | Section | Keys |
  |---|---|---|---|---|---|
  | nav | 8 | home | 36 | diary | 36 |
  | cover | 1 | shelf | 45 | vote | 18 |
  | common | 22 | favorites | 19 | meeting | 4 |
  | status | 4 | memberShelf | 7 | addBook | 11 |
  | statusShort | 4 | club | 24 | card | 4 |
  | relative | 7 | discover | 58 | errors | 20 |
  | auth | 14 | book | 50 | physicalShelf | 69 |
  | locale | 2 | settings | 42 | | |
  | theme | 11 | | | | |

- Plurals use vue-i18n `a | b` with `{n}`; `tp(key, n)` passes `n`.
- `constants.STATUS_LABEL`/`STATUS_SHORT` are unused English duplicates of `status.*`/`statusShort.*`.

---

## 4. Tests (every test function in `frontend/src/**/*.test.ts`; 152 functions, 158 cases with `it.each` expanded)

### src/api/client.test.ts (10)
- `client.test.ts::sends JSON with credentials`: `castVote(7)` POSTs `/api/vote/cast` with `credentials: include`, body `{"nomination_id":7}` and Content-Type `application/json`.
- `client.test.ts::builds query strings, dropping empty values`:
  - search `{q:"dune", subject:"", sort:"new", page:2}` gives `/api/books/search?q=dune&sort=new&page=2`, and `search()` gives `/api/books/search`;
  - `trending(8)` gives `?limit=8`;
  - `subject("science fiction",2,6)` gives `/api/books/subjects/science%20fiction?page=2&limit=6`;
  - the abort signal is passed through.
- `client.test.ts::addresses book detail by bare work id`: `book("OL1168007W")` gives `/api/books/works/OL1168007W`; refresh is a POST to `/refresh`; `createCustomBook` POSTs `/api/books/custom`.
- `client.test.ts::addresses the diary by work id and entries by id`:
  - GET/POST `/api/books/works/OL1W/posts`, with the body JSON `{body, spoiler_upto, parent_id}` in that key order;
  - PATCH and DELETE `/api/posts/7`; POST `/api/posts/7/reactions`;
  - feed `/api/diary?limit=8` and `/api/diary?limit=5&before=42`.
- `client.test.ts::replaces and toggles favourites`: PUT `/api/auth/me/favorites` with `{"book_ids":[3,1]}`; GET the same path; POST and DELETE `/api/auth/me/favorites/9`.
- `client.test.ts::patches theme preferences`: `savePreferences({theme:"ink"})` sends a PATCH to `/api/auth/me/preferences` with exactly `{"theme":"ink"}`.
- `client.test.ts::returns undefined on 204 and parses JSON otherwise`: logout with 204 gives `undefined`; config with 200 gives the parsed object.
- `client.test.ts::throws ApiError with detail and the conflicting shelf item`: a 409 `{detail:"Already on your shelf", item}` rejects with `name ApiError`, `status 409`, `message "Already on your shelf"` and `item`.
- `client.test.ts::falls back to a generic message when detail is structured`: a 500 `{detail:{code:1}}` gives the `ApiError` message "Something went wrong. Try again."
- `client.test.ts::does not set a JSON content type for form uploads`: `importGoodreads` sends FormData with no Content-Type header.

### src/assetRecovery.test.ts (3)
- `assetRecovery.test.ts::reloads once when an old page chunk is missing, without a reload loop`: the first `vite:preloadError` reloads and calls `preventDefault`; a second boot within 60 s does not reload.
- `assetRecovery.test.ts::does not risk a reload loop when browser storage is blocked`: if `sessionStorage.setItem` throws, there is no reload.
- `assetRecovery.test.ts::does not reload for ordinary API or application errors`: a plain `error` event does not reload.

### src/components/AddBookSheet.test.ts (1)
- `AddBookSheet.test.ts::submits a numeric year from the number input without crashing`: sends `{title:"Kitchen Zine 2", authors:"Ada", year:2024, description:""}` and pushes `/book/BCdeadbeef01`.

### src/components/Avatar.test.ts (4)
- `Avatar.test.ts::shows the username initial when there is no picture`: text "A", no img.
- `Avatar.test.ts::renders the picture when a src is set`: img `src=/api/members/ada/avatar?v=1`, `loading=lazy`, no initial.
- `Avatar.test.ts::falls back to the initial when the picture fails to load`: after an img error, the img is gone and "A" shows.
- `Avatar.test.ts::shows the picture again after src changes`: a new src resets the failure.

### src/components/BookCover.test.ts (8)
- `BookCover.test.ts::requests a medium tile with default=false, not a large GIF-friendly URL`: a tile is `…/b/id/8739376-M.jpg?default=false`, `lazy`, alt "Cover of Circe".
- `BookCover.test.ts::does not render an image for null or -1 cover ids`: no img; the monogram "C" shows.
- `BookCover.test.ts::does not invent an Open Library ISBN CDN URL for Google catalog keys`: `workKey /works/ISBN…` gives no img.
- `BookCover.test.ts::ignores an Open Library CDN imageUrl so xs tiles stay on the S tier`: an OL `-L` imageUrl with `cover_id` 123 at xs gives `/b/id/123-S.jpg`.
- `BookCover.test.ts::renders an https cover URL and skips Open Library sources`: an `http://books.google.com/...` imageUrl is upgraded to https and used.
- `BookCover.test.ts::uses an ISBN prop for Open Library works when cover_id is missing`: gives `/b/isbn/9780316769488-M.jpg`.
- `BookCover.test.ts::falls back to an edition key when cover_id is missing`: gives `/b/olid/OL1M-M.jpg`.
- `BookCover.test.ts::loads the first visible covers eagerly`: `eager` gives `loading=eager`.

### src/components/FavoritePortrait.test.ts (3)
- `FavoritePortrait.test.ts::hides when empty and no hint is given`: no `.portrait`, empty text.
- `FavoritePortrait.test.ts::shows a subtle empty hint on your own shelf`: the hint text shows, with no list.
- `FavoritePortrait.test.ts::renders ordered covers with rank labels`:
  - a `ul.portrait` with `aria-label` "Favourite books" and 2 links to `/book/OL1W`…;
  - link aria-labels "1. Circe" and "2. Galatea", with no `role` attribute;
  - ranks "1" and "2", and 2 images.

### src/components/FavoritesEditor.test.ts (3)
- `FavoritesEditor.test.ts::shows three slots and emits a compacted replace`: "Position 1…3" and "Empty"; "Move down" emits `[2,1]`; "Clear this favourite" emits `[2]`.
- `FavoritesEditor.test.ts::sorts by position before indexing into ids`: unsorted input renders Circe then Galatea; move down emits `[2,1]`.
- `FavoritesEditor.test.ts::clears the set`: "Clear all" emits `[]`.

### src/components/ShelfBoard.test.ts (5)
- `ShelfBoard.test.ts::shows active books together and completed months newest first in the all view`:
  - month headings "September 2026 1" and "August 2026 1";
  - statuses Want / Reading / Done / Done;
  - the club pick badge is shown for `clubPickKey`.
- `ShelfBoard.test.ts::keeps direct touch-delayed dragging and separates unfinished books from month targets`: no handle, `delay` 250, `delayOnTouchOnly`; the active group differs from the month groups; the month groups share one group.
- `ShelfBoard.test.ts::emits the target month even when a cross-month drop has identical old/new indices`: emits `reordered` with `[september, 2, "2026-08"]`.
- `ShelfBoard.test.ts::provides an empty adjacent month, and allows adding an older month across years`: shows "September 2026 1" and "August 2026 0"; a drop into the empty month emits `[september, 0, "2026-08"]`; the month picker adds "December 2025 0".
- `ShelfBoard.test.ts::uses the club timezone and keeps member shelves read-only without empty drop targets`: `2025-12-31T23:30:00` in Europe/Berlin falls in "January 2026"; read-only means draggable disabled, no buttons and no empty months.

### src/constants.test.ts (6)
- `constants.test.ts::maps our sizes onto Open Library's image tiers`: xs is S, md and tile are M, the default is L; null, 0 and -1 give null.
- `constants.test.ts::builds edition and ISBN fallbacks`: the olid S URL; the ISBN URL with dashes stripped; `extractIsbn` valid/invalid.
- `constants.test.ts::derives the detail route and Open Library link from a work key`:
  - `workId`, `bookPath`, `openLibraryUrl`;
  - the BC, OL, ISBN and GB recognisers, `isGoogleCatalogWorkKey`;
  - `remoteCoverUrl` allowlist (http upgraded; javascript:, OL CDN and attacker hosts give null);
  - `isbnFromWorkKey` upper-cases x; `googleBooksUrl`; `catalogUrl` for ISBN.
- `constants.test.ts::builds monograms and star labels`: monogram "D" and "?"; `starLabel(3)` is `★★★☆☆`; null gives "".
- `constants.test.ts::returns an empty string for missing or invalid dates`: `relativeDay(null)` and an invalid date give "".
- `constants.test.ts::describes distances in days, then weeks`: today, tomorrow, yesterday, "in 9 days", "in 3 weeks", "12 days ago", "4 weeks ago".

### src/diary.test.ts (9)
- `diary.test.ts::describes where the author was`: "at 45%", "Finished ★★★★☆", "Finished", "DNF", "before starting", and `""` for none or for reading without progress.
- `diary.test.ts::hides flagged entries from readers who are behind`: flag 60 shields at 25 and at null/null; it does not shield at 60 or 80.
- `diary.test.ts::never shields the writer, finished or DNF readers, unflagged or deleted entries`: covers all five exemptions.
- `diary.test.ts::proposes the writer's own position`: (40, reading) gives 40; finished gives 100; want gives null; none gives null.
- `diary.test.ts::labels the flag`: "No spoiler flag", "Whole book", "Safe up to 35%".
- `diary.test.ts::hides the excerpt when the viewer is behind or has not shelved the book`: shows "Spoiler-flagged note" for behind, unshelved and want-to-read viewers.
- `diary.test.ts::shows the excerpt to the writer, finished or DNF readers, and readers past the flag`: the secret text shows in all five cases.
- `diary.test.ts::flattens whitespace and trims with an ellipsis`: `"a  b\n\n c"` gives `"a b c"`; 200 × "x" with max 20 gives 20 characters ending in "…".
- `diary.test.ts::counts entries and replies, skipping tombstones`: gives 3.

### src/drafts.test.ts (3)
- `drafts.test.ts::keeps drafts apart per member, book and reply`: `bookclub.draft.ada.OL1W`, `….reply.7`, and `anon` for null.
- `drafts.test.ts::restores a saved draft and forgets an emptied one`: round-trips; whitespace removes the key (storage length 0).
- `drafts.test.ts::does not break writing when storage is unavailable`: when storage throws, nothing throws and loads give "".

### src/i18n/messages.test.ts (4)
- `messages.test.ts::keeps the German catalog in lockstep with English`: identical key sets.
- `messages.test.ts::does not keep the unused favorites.marked key`: `favorites.marked` is absent and `favorites.markedAt` present.
- `messages.test.ts::prefers Favoriten over Lieblinge in German favourites copy`: 8 DE strings contain "Favorit" and not "Lieblinge"; the heading is "Lieblingsbücher".
- `messages.test.ts::keeps {placeholder} sets and | plural arity in lockstep`: per key, the same `{x}` set and the same number of `|` parts.

### src/pages/BookDetailPage.test.ts (2)
- `BookDetailPage.test.ts::adds a favourite when there is a free slot`: calls `addFavorite(4)`; the button reads "Favourite · 1" with `aria-pressed=true`; toast "Added to favourites".
- `BookDetailPage.test.ts::shows a clear message instead of replacing the last slot when full`: on a 409 the button stays "Favourite"/false; toast "You already have 3 favourites. Remove one first, or reorder in Settings."

### src/pages/ClubPage.test.ts (4)
- `ClubPage.test.ts::does not leak a spoiler-flagged excerpt when the viewer is behind`: `.feed-body.shielded` shows "spoiler-flagged" and "Spoiler-flagged note"; the secret is absent from the whole HTML.
- `ClubPage.test.ts::still shows the excerpt to the writer`: `mine` means not shielded and the secret is shown.
- `ClubPage.test.ts::unfolds a long note in place instead of only linking away`:
  - shows "…" and hides the second paragraph;
  - `.feed-more` toggles `aria-expanded` and the full text;
  - `a.feed-head` href is `/book/OL1W#entry-11`.
- `ClubPage.test.ts::offers nothing to unfold for short notes or shielded spoilers`: no `.feed-more`; the secret is absent.

### src/pages/DiscoverPage.test.ts (24)
- `DiscoverPage.test.ts::does not load browse shelves when a query is already in the URL`: `?q=circe` calls search only, not trending or subject; placeholder "Title, author, or ISBN".
- `DiscoverPage.test.ts::shows a short-query state for It and does not call search`: "That search is too short", "3 characters", "Add your own book"; search is not called.
- `DiscoverPage.test.ts::keeps visible hits when a later page fails`: a 502 on page 2 keeps "Circe" and shows "Couldn't load more", not the unavailable panel.
- `DiscoverPage.test.ts::distinguishes a rate-limit from a miss`: a 429 shows "The library is busy…20 seconds", not "Nothing matched".
- `DiscoverPage.test.ts::offers to clear the subject filter and add a book on a true miss`: "Nothing matched", "Clear Fantasy", "Add your own book", "ISBN".
- `DiscoverPage.test.ts::starts two subject shelves before waiting on the rest`: subject calls `[fiction, science_fiction]`, then mystery after one resolves.
- `DiscoverPage.test.ts::keeps browse tiles visible when remounting Discover`: cached tiles show immediately while the refetch is pending; no "Still loading shelves".
- `DiscoverPage.test.ts::retries a failed subject shelf instead of only offering search`: "Try again" and "Search it instead"; a retry loads "Dune".
- `DiscoverPage.test.ts::does not report successful empty shelves as errors in %s` (it.each en, de): 5 `.row-empty`, 0 `.row-fallback`, the "nothing trending" text.
- `DiscoverPage.test.ts::clears a subject error when retry succeeds with an empty list`: after the retry, 0 fallbacks and 1 empty row.
- `DiscoverPage.test.ts::does not empty visible results after they land while the route catches up`: results stay identical after the router settles; exactly 1 page-1 search and 1 trending call.
- `DiscoverPage.test.ts::keeps the current result list visible when a newer search is in flight`: "Dune" stays until "Circe" lands; no "Nothing matched" flash.
- `DiscoverPage.test.ts::does not flash a previous search after Clear`: Clear removes "Dune" at once; the new search shows nothing until it resolves.
- `DiscoverPage.test.ts::still shows the empty state when a reset search returns nothing`: "Nothing matched", 0 tiles.
- `DiscoverPage.test.ts::keeps current hits visible while a sort change refetches`: "Popular" keeps "Dune" until the new page lands.
- `DiscoverPage.test.ts::says still searching after a couple of seconds`: after 2.1 s, "Still searching the library" and "2s so far"; the note disappears on resolve.
- `DiscoverPage.test.ts::ignores a stale slower response after a newer search`: a late "Dune" does not replace "Circe".
- `DiscoverPage.test.ts::shows five recent terms, expands, and reruns a selected search`: 5 entries, then 7 after "Show all"; a click remembers "Book 1", searches, fills the input and closes the dropdown.
- `DiscoverPage.test.ts::filters recent terms while typing and hides the dropdown when none match`: `"  bOOK 2  "` gives `["Book 2"]`; "Circe" hides the dropdown; empty input shows 5; no search.
- `DiscoverPage.test.ts::dismisses with Escape, outside pointer presses, and focus leaving the search`: covers all three dismissals; focus moving into the dropdown keeps it open.
- `DiscoverPage.test.ts::stays closed when history finishes loading after focus has left`: a late load does not reopen; the next focus shows "Circe".
- `DiscoverPage.test.ts::closes the dropdown when a new search is submitted`: closes and remembers "Book".
- `DiscoverPage.test.ts::saves only submitted valid terms, even with zero results or history failure`: typing alone saves nothing; "it" is not saved; "Circe" is saved even when the history POST rejects; the miss state shows.
- `DiscoverPage.test.ts::deletes one term or all terms without changing search results`: `delete(1)` removes the row; "Clear history" calls `delete(undefined)` and closes the dropdown.

### src/pages/MemberPhysicalShelfPage.test.ts (3)
- `MemberPhysicalShelfPage.test.ts::shows their arrangement without anything to move, remove or restyle`:
  - calls `friendShelf("grace")` and `physicalShelfLayout("grace")`;
  - rows `[["2","decor:globe-1"],["1"]]`, material black, lights shown;
  - no row-remove, no material picker; the decoration is disabled;
  - the book sheet shows the title, the take, and a link to `/book/OL1W`;
  - no restyle; never saves.
- `MemberPhysicalShelfPage.test.ts::arranges the books itself when the member never set up a bookshelf`: an empty layout renders 2 spines without saving.
- `MemberPhysicalShelfPage.test.ts::says so when there is nothing on the shelf`: no shelf element; "grace has no books".

### src/pages/MemberShelfPage.test.ts (2)
- `MemberShelfPage.test.ts::hides the portrait when the member has no favourites`: no `.portrait`, no own-shelf hint.
- `MemberShelfPage.test.ts::shows the member's favourite covers under the avatar`: 1 link, `aria-label` "1. Circe", no role.

### src/pages/PhysicalShelfPage.test.ts (18)
- `PhysicalShelfPage.test.ts::restores the saved rows and book order without writing during page load`: rows render as saved; walnut pressed; no save and no `patchShelf`.
- `PhysicalShelfPage.test.ts::prevents edits if loading the saved layout failed, so a default cannot overwrite it`: a layout GET failure gives add-row disabled or absent, no spines, an error text and no save.
- `PhysicalShelfPage.test.ts::moves books with the keyboard controls across rows and leaves reading status untouched`: right saves rows `[[1,2],[3],[]]`; row select 2 saves `[[1],[3],[2]]`; no `patchShelf`; the status label is unchanged.
- `PhysicalShelfPage.test.ts::rolls back a failed save and blocks overlapping layout changes`: white is pressed while saving; add-row is disabled and a click does not save; on reject, walnut is restored and add-row enabled.
- `PhysicalShelfPage.test.ts::keeps spine edits local until they are applied, then persists colour and width together`: no save before submit; one save with `spines["2"]` matching `{color:"#aabbcc", width:52}` and the rows unchanged.
- `PhysicalShelfPage.test.ts::persists a dragged book into the destination row and restores both rows if the save fails`: saves `[[1],[3],[2]]` once; on reject the original rows are restored.
- `PhysicalShelfPage.test.ts::removes occupied boards without losing objects, and keeps the saved single board on reload`:
  - removing board 2 merges into `[[2,"decor:plant-1",1,3,"decor:frame-1"],[]]`, keeping decorations and spines;
  - a second removal leaves 1 row;
  - after a reload the single board's remove button is disabled and no save happens.
- `PhysicalShelfPage.test.ts::restores an occupied board and its decoration after a failed removal`: while pending, add-decoration and every row-remove are disabled; on reject everything is restored, and the save had carried the decorations.
- `PhysicalShelfPage.test.ts::shows the book first and only opens the styling controls on request`: the book sheet shows "Finished" and `/book/OL2W`; the editor opens only via "edit" and "back" closes it; no save.
- `PhysicalShelfPage.test.ts::stores how a book is placed and lets colour and thickness follow the book again`: flat plus auto colour and width saves `{"2":{display:"flat"}}`; cover saves both; returning to spine removes the entry.
- `PhysicalShelfPage.test.ts::switches the string lights and the black finish without touching the rows`: lights saves `{lights:true, rows}` and shows the lights; black saves `{lights:true, material:"black"}`.
- `PhysicalShelfPage.test.ts::adds a %s decoration to the chosen board and persists its identity` (it.each over 6 kinds):
  - the id matches `^decor:[\da-f-]+$` and the record is `{kind, color:#hex6}`;
  - the rows become `[[2,1],[3,id],[]]`, with the spines unchanged and the picker closed;
  - the element's `data-id` is the id; no `patchShelf`.
- `PhysicalShelfPage.test.ts::restores the existing objects and decoration records when adding decoration fails`: the save attempt carries 3 decorations; on reject the original 2 are restored and add is re-enabled.
- `PhysicalShelfPage.test.ts::persists a decoration dragged between mixed rows together with its appearance`: saves rows `[[2,1],[3,"decor:plant-1","decor:frame-1"],[]]` with the decorations and spines intact.
- `PhysicalShelfPage.test.ts::keeps exactly one decoration node after the real drag library restores and updates mixed rows`: with the real vue-draggable-plus, one save and exactly one decoration node, 3 spines.
- `PhysicalShelfPage.test.ts::edits decoration color locally before saving it without changing books or other decorations`: saves only the plant's colour; the editor closes.
- `PhysicalShelfPage.test.ts::moves decoration through the accessible position controls`: right gives `[[2,1,"decor:plant-1"],…]`; row 3 gives `[[2,1],[3,"decor:frame-1"],["decor:plant-1"]]`.
- `PhysicalShelfPage.test.ts::removes decoration while preserving every book, book style and other decoration`: the rows and the decoration map shrink by one; spines and books are unchanged.

### src/pages/ShelfPage.test.ts (8)
- `ShelfPage.test.ts::shows a subtle empty hint when you have no favourites`: the toggle shows "No favourites yet" without a portrait.
- `ShelfPage.test.ts::renders favourite covers under the profile header`: the portrait is hidden until the toggle; `aria-expanded` follows; 2 links with no role; the toggle hides it again.
- `ShelfPage.test.ts::allows direct reordering and rolls back failed saves while blocking overlapping moves`:
  - no drag handle or organize button;
  - a pending patch disables the board and shows the optimistic order;
  - a second move is ignored; the body is `patchShelf(2,{shelf_position:0})`;
  - on reject the board is restored and re-enabled.
- `ShelfPage.test.ts::saves a cross-status reorder without changing status, then lets status be changed separately`: `patchShelf(2,{shelf_position:0})` keeps status, rating and `finished_at`; then the status sheet sends `patchShelf(1,{status:"currently_reading", position:0})`.
- `ShelfPage.test.ts::keeps reading-date editing available from the visible status button`: day precision sends `{finished_on:"2026-06-15"}`; the board shows the new `finished_at`; `<time>` reads "June 2026".
- `ShelfPage.test.ts::saves a month/year without requiring a day`: prefilled "2026-09"; "2025-12" sends `{finished_on:"2025-12-01"}`; the text shows "December 2025".
- `ShelfPage.test.ts::saves cross-month order and completion together, and rolls both back on failure`:
  - `{shelf_position:0, finished_on:"2026-08-01"}`; a reject restores the item exactly;
  - a success adopts the saved item ("August 2026");
  - a same-month repeat sends `{shelf_position:0}` only.
- `ShelfPage.test.ts::renders shelf hints in both languages instead of showing translation keys`: `.shelf-drag-hint` equals `shelf.chronologicalHint` in de and en; never the raw key.

### src/physicalShelf.test.ts (23)
- `physicalShelf.test.ts::prunes deleted books and duplicates while preserving placements and adding new books in shelf order`: gives `[[3,1],[],[2,4,5]]`; the unknown spine 99 is dropped; the input is not mutated; spine objects are copied.
- `physicalShelf.test.ts::packs initial books into rows with room for their widths and defaults to three rows`: 20 books of 900 pages pack as 8/8/4; empty gives 3 empty rows and `decorations {}`; a missing `rows` field gives 3 rows.
- `physicalShelf.test.ts::preserves shelves reduced to one or two boards`: `[[]]` and `[[1],[]]` are kept; removing a row then reconciling gives `[[1,2]]`.
- `physicalShelf.test.ts::preserves mixed object order and only keeps placed decoration records`: missing, unprefixed, duplicate and unknown ids are dropped; unused records are dropped; records are copied.
- `physicalShelf.test.ts::includes decoration widths when finding room for new books`: a 470 px spine plus a plant pushes the next book to a new row; the plant/vase/candle/frame widths are 76/58/50/74.
- `physicalShelf.test.ts::uses custom widths when packing additions and lets the final row overflow at twelve rows`: 100 books give 12 rows with 12 in the last and the order preserved; a 550 px custom width pushes book 2 down.
- `physicalShelf.test.ts::preserves overflow placements by folding extra rows into the last permitted row`: 14 rows fold to 12, with the last row `[12,13,14]`.
- `physicalShelf.test.ts::moves within a row using the destination index after removal`: `[2,3,1,4]` and `[1,4,2,3]`; the input is not mutated.
- `physicalShelf.test.ts::moves between rows, removes duplicate placements, and clamps the insertion index`: covers three cases including index 100 and -2.
- `physicalShelf.test.ts::moves decorations among books without sharing appearance maps`: deduped move; spines and decorations are equal but not the same objects.
- `physicalShelf.test.ts::ignores nonexistent books or invalid destination rows without changing the layout`: returns the identical object for 5 invalid cases.
- `physicalShelf.test.ts::removes a board and appends all its books and decorations to the previous board`: gives `[[1,"decor:frame-1",2],[3]]`; the material is kept; appearance is copied; the input is unchanged.
- `physicalShelf.test.ts::keeps contents of the first board on the next board and lets an empty board be removed`: covers 3 cases.
- `physicalShelf.test.ts::ignores invalid board indices and keeps the final board`: -1, 2, 0.5, NaN, Infinity and the last remaining board give the same object.
- `physicalShelf.test.ts::gives a work stable fallback styling independent of shelf ID and metadata changes`: same work key gives an identical spine; width 32–48, height 150–190; ink matches.
- `physicalShelf.test.ts::uses page counts for thickness, with bounds and a fallback for unusable counts`: 10 pages give 26, 360 give 44, 3000 give 64; 0, -20, null, NaN and Infinity give the hash width.
- `physicalShelf.test.ts::prefers the colour sampled from the real cover over the palette`: `spine_color` wins with the same width; null falls back to the palette.
- `physicalShelf.test.ts::chooses light ink on dark colors and dark ink on light colors, including shorthand hex`: `#101010` gives light ink, `#fafafa` dark; `#000` equals `#000000` and `#FFF` equals `#ffffff`.
- `physicalShelf.test.ts::takes the room each placement needs along the board`: 44; custom 30; flat uses the height; cover uses `round(face*2/3)`; face height ≤ height.
- `physicalShelf.test.ts::packs new books around lying and face-out ones by their real footprint`: a flat footprint is larger than the width; all 3 books are placed; spines are kept.
- `physicalShelf.test.ts::carries the string lights through reconciliation`: true stays true; missing becomes false.
- `physicalShelf.test.ts::piles neighbouring lying books and starts a new pile when one gets too high`: checks the exact `{lift, span, level}` slots; an upright book or decoration ends a pile; the height cap is 150.
- `physicalShelf.test.ts::never piles more than five books`: levels `[0,1,2,3,4,0]`.

### src/shelf.test.ts (6)
- `shelf.test.ts::uses the club timezone for offsetless SQLite timestamps`: `2025-12-31T23:30:00` in Berlin gives 2026-01-01; `…01:00:00Z` in New York gives 2026-06-30; null gives "".
- `shelf.test.ts::initially places unread books above completed books and preserves within-status positions`: gives `[4,2,3,1]`.
- `shelf.test.ts::honors a manual order across statuses, with newly added books first`: gives `[3,1,2]`.
- `shelf.test.ts::maps filtered drops to the full shelf while preserving hidden books' relative order`: gives 0 and 2.
- `shelf.test.ts::sorts the finished filter by month across years while retaining manual order within a month`: finished gives `[2,3,1,4]`; all gives `[1,2,3,4]`.
- `shelf.test.ts::labels completion months in the selected language and club timezone`: gives "2026-01", "Januar 2026", "January 2026"; invalid or non-finished gives "".

### src/stores/session.locale.test.ts (3)
- `session.locale.test.ts::adopts the server locale when the login toggle was not used`: the locale becomes "en"; no `savePreferences`.
- `session.locale.test.ts::persists an explicit auth-screen locale after login`: `choose("de")` then login calls `savePreferences({locale:"de"})`; the locale stays de.
- `session.locale.test.ts::counts a click on the already-selected locale as explicit`: `choose("en")` sets explicit; login persists `{locale:"en"}`.

Test infrastructure (`test/setup.ts`):
- i18n is installed globally.
- `RouterLink` is stubbed as `<a :href="to if string else #">`.
- `matchMedia` and `IntersectionObserver` are polyfilled for jsdom.
- `vite.config.ts` sets `restoreMocks: true` and `css: false`.

Tests that **must change** under the port (not just be kept):
- all of `constants.test.ts` "covers and links" and most of `BookCover.test.ts` (OL CDN and Google, see 5.8);
- `client.test.ts` paths (club scoping, 5.3);
- `Avatar.test.ts` sample URLs (media store);
- the `ClubPage`/`Member*` route paths.

---

## 5. Multi-club mapping (what #48 changes in the frontend)

### 5.1 Route map

| Today | Kind | New route [plan #48 / proposal] | Notes |
|---|---|---|---|
| `/login` (guest) | auth | `/login` | add a "Forgot password?" link [plan #39/#48] |
| `/register` (guest, invite required) | auth | `/signup` (open sign-up: username, email, password) [plan] | `/register` redirects to `/signup`, keeping `?code=`; the invite field is removed, because invites now join clubs |
| – | auth | `/verify-email?token=…`, `/forgot-password`, `/reset-password?token=…` [proposal for paths; features are plan] | public, no `guest` redirect for verify (a logged-in user may click the mail link) |
| – | join | `/join/:code` [proposal] | invite link: logged out, it stores the code and goes through login/signup; logged in, it calls join and redirects to `/c/<slug>` |
| `/` Home (club pick hub) | club | `/` becomes the **combined feed across all my clubs** [plan]; the current Home content moves to `/c/:slug` | solo mode, see 5.4 |
| `/discover` | personal | `/discover` | the "Club" badge on tiles needs club context (5.3) |
| `/shelf`, `/shelf/physical` | personal | unchanged [plan] | |
| – | personal | `/diary` (my entries across books) [plan #48] | new page |
| – | personal | `/quotes` [plan #48] | new page over `/api/quotes` |
| `/club` | club | `/c/:slug` (club home) | `/club` redirects to the last-used club, the only club, the club list, or solo prompt |
| (Club page sections) | club | `/c/:slug/members`, `/c/:slug/vote`, `/c/:slug/overlap`, `/c/:slug/discussions`, `/c/:slug/events` [plan #48 lists pick, votes, events, discussions, overlap, members] | exact sub-paths are a proposal |
| `/club/:username` | personal (another user) | `/u/:username` [proposal] | personal data is gated by audience, not by a club, so it should not be club-scoped; `/club/:u` redirects there |
| `/club/:username/shelf` | personal (another user) | `/u/:username/shelf` [proposal] | read-only physical shelf |
| `/book/:workId` | catalog | `/book/:workId` with the new work id format [open]; legacy `OL…W`/`BC…`/`ISBN…`/`GB…` ids redirect via a server lookup | club bits get club context (5.3) |
| `/settings` | account | `/settings` (account) [plan] plus `/c/:slug/settings` (club admin) [proposal] | invites leave account settings |
| – | club | `/clubs/new`, `/clubs` (list and switcher target) [proposal] | |
| Legacy: `/library`, `/friends`, `/friends/:username`, `/overlap`, `/invites`, `/pick` | redirects | `/discover`; `/club` (then the club); `/u/:username`; the overlap of the last-used club; members/invites of the last-used club; `/` | keep [plan "old routes redirect"] |

`nav.ts` NAV_ITEMS today: Home, Discover, Shelf, Club, Settings, all in the bottom bar.
- **Proposal:** Home, Discover, Shelf (with Diary and Quotes reachable from Shelf, or as a sixth sidebar item), Club (current club; the switcher in the header), Settings.
- `isActive` prefix logic must treat `/c/` as the Club tab.

### 5.2 Club context, switcher and session

- **New store `useClubs` [proposal]:**
  - State: `items: {slug, name, role, member_count, timezone}[]` and `currentSlug` (from the route param on `/c/:slug/**`, else last-used).
  - Loaded after hydrate/login, and refreshed after create, join, leave and transfer.
  - Ideally `/api/auth/me` returns `clubs[]` so the guard needs one request [proposal].
- **Last-used club:** `localStorage["bookclub.club"]` = slug [proposal]. It is a per-device convenience, wrapped in try/catch like the other keys.
- **Today's `useClub` store splits in two:**
  - Service branding (name/accent; constant or a slimmed `/api/config`).
  - The current club's `name`, used as the wordmark inside club pages, and its `timezone`.
  - `DiaryEntry`/`ClubPage` read `club.timezone` today; on personal pages (book diary) the viewer's timezone should be used instead (5.9).
- **404 handling [plan #40]:** a non-member gets 404 on every club resource. Club pages need a "club not found" state; there is no 403 and no "request access". On a 404 for the last-used slug, drop it from storage and the switcher.
- **Role-aware UI [plan #40]:**
  - Owner: everything, plus transfer and delete club.
  - Admin: invites, picks, votes, events, removing members.
  - Member: everything else.
  - The UI hides or disables admin actions for members: "Set as club pick", "Confirm winner", invite minting and revoking, the meeting edit, member removal. The server still enforces.
  - Which status and `detail` a member gets for an admin action is **[open]**: 403 with a mapped string is the natural choice, and not 404, because membership is already known.
- **Club limits [plan]:**
  - At most 10 clubs created per user and at most 20 members per club. The join and create flows need mapped error strings.
  - Invites are single-use or expiring, revocable by admins. Today's `Invite` (`code`, `used`, `used_by`) gains `expires_at`, `max_uses`/single-use flag, `revoked`, and a link form.
- **Session [plan #39]:** the web keeps a cookie session (`credentials: include` stays); bearer tokens are for the apps (Phase 4).
  - `client.ts` should get one place to add an `Authorization` header later [proposal].
  - If the server adds CSRF protection (for example a required header or a double-submit token), `request()` must send it on unsafe methods. **[open]**
- **Global 401 [proposal]:** with real server-side sessions (logout everywhere, account deletion, password reset revoking sessions), `request()` should route a 401 on any non-auth call to `/login?next=…` and reset `session.user`, instead of leaving "Not signed in" in page slots.

### 5.3 Every call site, classified

| Today's endpoint | Under the plan | Frontend change |
|---|---|---|
| `GET /api/config` | instance branding goes away; club name and timezone become per club [plan: hosted service] | slim or constant service config; club info from `useClubs` |
| auth: register/login/logout/me/preferences | personal; register becomes open sign-up with email [plan] | `User` gains `email`, `email_verified`, an email-prompt flag [proposal: `needs_email_prompt`], possibly `clubs[]`; the register body drops `invite_code` and adds `email` |
| `PATCH /api/auth/password` (no UI today) | personal [plan: back in the UI] | new form in account settings |
| new: email verification, resend, reset request, reset confirm, change email, delete account, export | personal [plan #39] | new pages and settings sections; delete needs a confirmation step (password or typed username) and must explain ownership transfer first (owner of a club) [plan #40] |
| `GET`/`PATCH /api/auth/notifications` (no UI) | personal [plan #48] | settings section; push itself is Phase 4 |
| avatar PUT/DELETE, `/api/members/{u}/avatar` | personal; blobs move to the media store with content-hashed, long-cached URLs [plan #43] | `avatar_url` becomes an opaque same-origin URL; `Avatar.vue` is unchanged |
| favourites | personal [plan]; ids become work ids [proposal] | `BookDetail.id` becomes the work id; drop or reword the "not in the club yet" copy (see 5.10) |
| search history | personal [plan] | unchanged |
| search/trending/subjects | global catalog; Google dropped; local Postgres FTS plus DNB/OL [plan #41] | `SearchHit` loses `cover_edition_key`, `isbn`-for-OL-CDN, `club_pick` (a single club no longer exists) [proposal: drop, or `club_picks: slug[]`]; the "trending" and "subjects" browse rows depend on what #41 still offers **[open]** |
| book detail/refresh | global catalog work; editions [plan] | `readers` and `club_rating` must be audience-filtered (#46); `club_pick: bool` becomes "current pick in which of my clubs" [proposal: `club_picks: {slug,name}[]`]; refresh goes away or becomes "report wrong data" (admin queue) [plan #41] |
| `POST /api/books/custom` | user-added books and editions, edits via admin queue [plan] | the AddBookSheet copy says "stays in this club only"; it becomes a global catalog addition, possibly with ISBN and edition fields |
| own shelf GET/POST/PATCH/DELETE, import | personal [plan] | the POST body becomes `{work_id, edition_id?, status, …}` with **no client-supplied metadata** (see R-4); add a `rating_visibility` toggle [plan #46] |
| `GET /api/shelf?username=` | personal, audience-gated [plan #46]; strangers get 404 [proposal: same 404 as "no such user" so usernames don't leak] | a route move to `/u/:username` |
| physical layout GET/PUT, spine colours | personal; audience can view, only the owner edits; editing another user's returns 404 [plan #46] | unchanged API shape (keep integer shelf entry ids, see R-6) |
| `GET /api/members` | **club-scoped** [plan] | `/api/clubs/{slug}/members` [proposal]; add `role`; it should probably **include** the viewer, with a `you` flag, to drop the `members.length + 1` trick [proposal] |
| `GET`/`POST /api/invites` | **club-scoped**, admin only [plan] | `/api/clubs/{slug}/invites` plus revoke; the UI moves to the club members page; invite *links* carry `/join/:code` |
| `GET /api/pick`, `/history`, `PUT`/`DELETE /api/pick` | **club-scoped**; picks at work level; setting is admin-only [plan] | `/api/clubs/{slug}/pick…`; the response's `timezone` is the club's; `readers` = club members only; "Set as club pick" from Book, Shelf or Discover opens a **club chooser** (clubs where I am admin) |
| `GET /api/overlap` | **club-scoped**, computed per club [plan #45] | `/api/clubs/{slug}/overlap` |
| `GET /api/vote`, nominations, cast, apply | **club-scoped**; apply is admin-only [plan] | `/api/clubs/{slug}/vote…`; "Nominate" from Book or Shelf opens the club chooser |
| book diary (`/api/books/works/{id}/posts`, `/api/posts/*`) | **diary entries are personal**, audience-visible, with an "only me" option [plan #46] | add `visibility: "audience" \| "only_me"` to the composer and edit; whether replies and reactions belong to the entry's author or are club content is **[open]** |
| `GET /api/diary` feed ("Recently written" on Club) | today it lists the whole instance's entries | per club ("entries by members of this club"), and the combined home feed across clubs [plan: combined home feed] **[open: exact sources of the home feed]** |
| `/api/backup` | dropped [plan #49: "instance backup download → personal export + server backups"] | the Settings "Backup" section becomes "Export my data" (DSGVO export, JSON plus media) [plan #39] |
| pick posts, milestones, activity, stats (no UI today) | club-scoped [plan #45] | new UI under `/c/:slug/discussions` and `/c/:slug/events`; milestones are in **percent or chapter**, not pages [plan] |
| quotes (no UI today) | personal, with "only me" [plan] | new `/quotes` page |
| push (no UI today) | Phase 4 [plan] | nothing in Phase 1 |

**API path scheme for club scoping [proposal]:**
- `/api/clubs/{slug}/…`, mirroring `/c/{slug}/…`.
- If slugs can be renamed (an [open] question for #40), use a stable club id in the API and keep the slug only in the UI route, resolving it via `useClubs`.

### 5.4 Solo mode (zero clubs) [plan]

- **Shelf, physical shelf, Discover, Book, Diary, Quotes and Settings must work without any club endpoint call.** Today, several pages call club endpoints unconditionally:
  - `ShelfPage` calls `GET /api/pick` (for the badge);
  - `BookDetailPage` calls `GET /api/pick` (for the timezone);
  - `HomePage` is entirely club data.
  - These calls must be removed or made conditional on having a club.
- **Home with no clubs:** a personal summary (currently reading with progress sliders, recent own diary entries, favourites) plus a "Create a club / Join with a code" prompt [plan: "prompts the user to create or join a club"].
- **Club actions with no clubs:** "Set as club pick", "Nominate" and "Club" badges are hidden; the Club nav item goes to the create/join screen.
- **Book page with no clubs:** "Other readers" and the club rating show only audience data (friends, from Phase 3) or are hidden. Copy such as "Nobody else in the club has this one yet" needs a neutral variant.

### 5.5 Sign-up, verification, reset, email prompt, account settings [plan #39/#48]

- **Sign-up form:** username (same rule as today: `^[a-z0-9_]{2,32}$` after trim and lower-case [proposal: keep]), email, password (≥ 8).
  - The locale and theme persistence after register stays (2.1).
  - Whether sign-up logs the user in before verification is **[open]**, and so is what an unverified user may do.
- **Verification:** a landing page for the token link; a resend button with a rate-limit message.
- **Reset:**
  - Request: always answers the same way (no account enumeration) [proposal].
  - Confirm: token and new password.
  - No reset is possible for accounts without an email [plan]; the copy must say so.
- **Imported users** (no email): a one-time prompt after login asking them to add an email. It is skippable, and the dismissal is persisted server-side [plan; field name proposal].
- **Account settings sections:**
  - profile/avatar, favourites, language, appearance (kept);
  - email add/change with re-verification;
  - change password (back);
  - notification preferences;
  - Goodreads import (kept, personal);
  - "Export my data";
  - "Delete account" (with an owner-must-transfer explanation);
  - log out.
  - Removed from account settings: invites (they move to the club) and the database backup.

### 5.6 Visibility toggles [plan #46]

- **Per item "only me":**
  - diary entries (composer, reply? **[open]**, edit);
  - quotes;
  - ratings (on the book page star row; the shelf card shows the own rating regardless).
- **What the audience sees:** shelf, ratings, currently reading, stats, diary entries and quotes (all non-"only me") plus the physical layout (read-only).
- **Strangers:** nothing. Member and user pages for a non-audience user should show the same "not found" as an unknown username [proposal].
- The frontend must render `only_me` items for their owner with a lock badge [proposal], and never assume `readers`/`members` include everyone.

### 5.7 Works vs editions, Google removal, covers [plan #41/#43]

- **Work identity:** every place that today sends `ol_work_key` plus client-side `title/authors/cover_id/cover_url/year` must send only the work id (and optional `edition_id`):
  - `POST /api/shelf` (Home `startReading`, Book `setStatus` and undo, Shelf undo)
  - `PUT /api/pick` (Home `saveMeeting`, Book and Shelf `confirmClubPick`)
  - `POST /api/vote/nominations` (Club overlap, Book, Shelf)
  - `POST …/posts` with `book` (first diary entry)
  - These types go away: `ClubPickBook`, `VoteBook`, `DiaryBookIn`, the add-to-shelf body type.
- **Routing:** `bookPath`/`workId()` currently strip `/works/`. The new id format is **[open]**; whatever it is, `bookPath(work)` must be the single source. Legacy `/book/<OL…W|BC…|ISBN…|GB…>` URLs (bookmarks, feed links with `#entry-<id>`) should resolve to the new id, preserving the hash.
- **Editions [plan]:**
  - A shelf entry may point at an owned edition.
  - The book page may offer "which edition do you own" (not required by #48; **[open]** for Phase 1).
  - The physical shelf spine width should use the owned edition's `pages` when set, else a work-level page count [proposal]; `defaultSpine` keeps hashing the work id so the fallback colours stay stable across editions.
- **Covers:**
  - The browser must make **no** request to `covers.openlibrary.org` or Google [plan #43 AC]. `cover_url` becomes a same-origin URL (for example `/media/covers/<hash>.webp`) [plan: served from our domain, content-hashed].
  - `BookCover.vue` must accept same-origin relative URLs. Today `remoteCoverUrl` **rejects** anything that is not an https Google URL, so a relative `/media/...` URL would never render.
  - Size tiers need server-side variants [proposal: `cover_url` plus a `?w=` or separate `cover_sm`/`cover_md`/`cover_lg`].
  - To remove: `coverUrl`, `editionCoverUrl`, `isbnCoverUrl`, `remoteCoverUrl`'s host allowlist, `isGoogleCatalogWorkKey`, `googleBooksUrl`, `isGoogleWorkId`, `isIsbnWorkId` (as a source marker), the `book.viewGoogle` key, and the OL-specific "No cover — refresh from Open Library" prompt.
  - Add: "Report this cover" (notice-and-takedown) and "Add a photo of your copy" (upload; EXIF stripped server-side) [plan #43]. Whether the upload UI is Phase 1 frontend scope is **[open]**.
- **Catalog link:** "View on Open Library" stays for OL-sourced works; a DNB link for DNB-sourced works [proposal]. `book.addedByClub` becomes "Added by a reader".
- **Milestones** are percent or chapter [plan]. Today's diary `spoiler_upto` and shelf `progress` are already percent and edition-independent, so keep them.

### 5.8 Time zones [proposal; plan is silent]

- **Club-scoped times** (meeting, vote deadline, club feed labels) use the **club's** timezone, a club setting with the server default `Europe/Berlin` or the creator's browser zone.
- **Personal dates** use the **user's** timezone, a new user preference initialised from `Intl.DateTimeFormat().resolvedOptions().timeZone` at sign-up. They cover:
  - the finished month and "today" for reading dates (`shelf.timezone` today);
  - diary entry timestamps on book pages.
- Replace `useClub().timezone` in `DiaryEntry.vue` with the viewer's timezone.
- The Rust server must emit RFC 3339 timestamps **with an offset** (`Z`). `clubDate`'s offsetless-means-UTC shim then becomes dead, but harmless. `formatClubDate` becomes correct.

### 5.9 i18n and `apiErrors.ts`

- **Keep** the exact-string mapping mechanism [plan #48: "apiErrors.ts mapped to the new server's error strings"].
  - **Proposal:** the server keeps every legacy English string that survives and returns `{"detail": "<English>"}`.
  - Optionally it adds a machine `code`, but `request()` keeps reading `detail`.
  - The parity test moves to the server: a Rust test that reads `frontend/src/i18n/apiErrors.ts` and greps `server/src`.
- **Keys whose meaning changes or that should go:**
  - `errors.inviteInvalid` (now for club joins, not sign-up);
  - `errors.bookNotInClub` (no "club catalog" any more; reword the server string and the key);
  - `errors.noPicture` (dead);
  - `auth.haveInvite`, `auth.joinLede`, `auth.inviteCode` (sign-up no longer needs an invite);
  - `settings.backup*`, `settings.downloadDatabase`;
  - `settings.invites*` (move to a club namespace);
  - `club.onlyYouBody` ("Mint an invite code…");
  - `book.viewGoogle`, `book.addedByClub`, `addBook.lede` ("stays in this club only"), `addBook.notes` ("Notes for the club"), `addBook.added` ("Added to the club catalog");
  - `favorites.settingsBlurb` ("Any book already in the club…");
  - `memberShelf.back` ("← Club");
  - `home.lede`, `home.loadFailed`, `vote.blurb`, `vote.confirmBlurb`, `shelf.pickBlurb`, `book.pickBlurb` ("replaces the current pick for everyone" becomes "for <club>");
  - `settings.importBlurb`/`importWait` ("looked up at Open Library", now DNB/OL).
- **New strings to map (examples; the exact server strings are for the auth and club specs to fix):**
  - sign-up: email taken, invalid email, weak password;
  - verification token invalid or expired;
  - reset token invalid or expired;
  - "No email on this account";
  - rate limits for sign-up and reset;
  - club: name or slug taken, slug format, "You can create at most 10 clubs", "This club is full (20 members)", invite expired, revoked or already used, "Transfer ownership before leaving", not allowed for your role;
  - account deletion blocked while owning a club;
  - visibility enum errors.
- **Keep untranslated-detail awareness:** every user-visible string the server can return should be mapped. Today's unmapped strings (listed per endpoint in section 1) show in English in the German UI. That is acceptable for parity but should be revisited as strings are rewritten.
- **Hard-coded English in templates to fix along the way:** `SettingsPage.vue` `"Untitled"`; the Goodreads skip `reason` (server text); `aria-label="Main"` in `AppShell.vue`.

### 5.10 What no longer makes sense

- **`GET /api/config` as instance branding:** the name, accent, `public_url` and timezone come from env vars of a single club. `brand.applyBrand` rewriting the manifest from config becomes a static manifest; the backend's `brand_index_html` rewriting goes away.
- **"Members" as "everyone on the instance except me"**, and `overlap`/`readers`/`club_rating`/`diary feed` as instance-wide aggregates.
- **The Settings invite list** ("invites I created", never expiring) and invite-gated registration.
- **The SQLite backup download.**
- **Google Books** UI paths, the `GB…`/`ISBN…` work-key heuristics, and the cover host allowlist.
- **Client-supplied book metadata** in shelf, pick, vote and diary payloads.
- **`DELETE /api/pick`** client function (dead).
- **`STATUS_LABEL`/`STATUS_SHORT`** English constants (dead).
- **`ClubConfig.public_url`** (unused).

### 5.11 Build and serving

- **`vite.config.ts`:**
  - `build.outDir` is `../backend/app/static` with `emptyOutDir: true`. The `backend/` directory is deleted at cutover [plan], so the build output needs a new home. **[open]**
  - **Proposal:** `frontend/dist`, copied into the server image and served by axum with:
    - `index.html` as `Cache-Control: no-store` (today's rule; prevents stale chunk references, complementing `assetRecovery`);
    - hashed `/assets/*` with a long immutable cache;
    - an SPA fallback for every non-`/api` path;
    - a JSON `{"detail":"Not found"}` for unknown `/api/*` paths.
- **Dev proxy:** it targets `http://127.0.0.1:8000`, but the Rust server's default `BIND_ADDR` is `127.0.0.1:8080`. Change the proxy (or make it env-driven) when the frontend switches servers.
- **Generated types [plan #47]:** `frontend/src/api/generated.ts` from `/api/openapi.json` (openapi-typescript); `types.ts` is replaced step by step, and CI fails if it is stale. Keep `request<T>()` as the single fetch path so typing can be swapped in centrally.
- **Capacitor (Phase 4):** keep relative `/api` for the web build. The app build will need a configurable API origin and bearer tokens, so centralise this in `client.ts` now [proposal].

---

## 6. Port risks (behaviours a reimplementation is likely to get wrong)

- **R-1. Every error must be JSON with `detail`.** `request()` calls `JSON.parse` on every non-204 body.
  - axum's default extractor rejections (`Json` / `Query` / `Path` / `Multipart`, 415/422/400) are **plain text**. So are its default 404/405 and the body-limit 413.
  - Any of those makes the client throw a `SyntaxError` instead of an `ApiError`. The user sees only the page's generic fallback, and status-based branching (2.2) silently breaks.
  - Map all of them to JSON, and reproduce Python's "validation errors are **400** with one `"; "`-joined `detail`". Discover treats a 400 as "short query", so a 422 would land in the generic panel.
- **R-2. 409 on add-to-shelf must carry the full `item`.** It must be a complete `ShelfItemOut`. Home and BookDetail depend on `item.id`/`item.status`, and BookDetail retries as a PATCH.
- **R-3. Status codes the UI branches on:**
  - 401 for not signed in (logged-out detection);
  - 409 for favourites full (fixed copy);
  - 400 / 429 / 502 for search (three different panels, plus the readinglog-to-relevance fallback on 502);
  - 204 (not 200 with an empty body; an empty 200 yields `null`, not `undefined`) for login, logout, delete shelf item, delete diary entry, delete search history.
- **R-4. Client-supplied metadata overwrites the catalog today.** `upsert_book` sets `title` (and authors, cover, year when present) from any shelf, pick, vote or diary payload. In a public multi-user service that lets anyone rename a global work. The new API must take ids only and route edits to the admin queue [plan #41]. Every caller listed in 5.7 must change in the same step.
- **R-5. Covers:**
  - `BookCover` ignores any `cover_url` that is not an https Google URL, and builds OL CDN URLs from `cover_id`.
  - Simply returning same-origin URLs from the server will show **no covers at all**, and leaving `cover_id` in responses will keep hitting openlibrary.org (a DSGVO problem, and it breaks the #43 AC).
  - The component and its tests must be rewritten together with the media store.
- **R-6. Physical layout keys are shelf entry ids, and the code distinguishes books from decorations by `typeof id === "number"`** (`physicalShelf.ts`, `PhysicalShelf.vue`, `PhysicalShelfPage.vue`, the server schema).
  - If shelf entries get UUID or string ids, the whole feature breaks.
  - If the importer renumbers shelf entries, it must rewrite every layout's `rows` and `spines` keys.
  - Keep integer ids (i64 within the JS safe range) or redesign the layout format.
- **R-7. Timestamps.**
  - The Python API emits offsetless ISO strings. `clubDate` assumes UTC; `formatClubDate` and `relativeDay` read them as local time.
  - The Rust server must emit an explicit `Z`/offset everywhere. Tests in `shelf.test.ts` encode the offsetless case, so keep `clubDate`'s shim.
  - `meeting_at` input stays offsetless "club-local" and must be parsed in the **club** timezone. `meeting_local` must be `"YYYY-MM-DDTHH:MM"` in club tz for the `datetime-local` prefill.
- **R-8. Partial PATCH semantics.**
  - The client relies on absent versus `null` being different: `{rating:null}` clears, while an omitted rating is untouched. Likewise `{finished_on:null}` clears the date, and `{spoiler_upto:null}` on a diary edit clears the flag.
  - serde needs `Option<Option<T>>` (or `#[serde(default, with = "::serde_with::rust::double_option")]`) for these fields.
- **R-9. Status change side effects:**
  - The server clears `rating`/`take`/`dnf_reason`/`progress` on status changes and stamps or clears `started_at`/`finished_at`.
  - `ShelfPage` mirrors the per-status `position` renumbering client-side, so the server's `place_item` must renumber exactly: insert at the clamped `position`, compact 0..n-1 per `(user, status)`.
  - `orderedShelf` mirrors `place_on_shelf`; a different tie-break order makes the board jump after save.
- **R-10. `GET /api/vote` and `GET /api/pick` have side effects** today (auto-closing a vote past its deadline, scheduling meeting-reminder pushes). Keep them idempotent and safe when moved to club scope, or move them to a background job; the UI never expects a GET to change the pick.
- **R-11. Re-PUTting the current pick wipes the note.** `HomePage.saveMeeting` re-PUTs the current pick without `note`, and Python resets `note=""` and `set_by=<editor>`. Parity would keep the bug. A separate "edit meeting" endpoint (PATCH meeting only, admin-only) is the clean fix [proposal]; the frontend change is one call site.
- **R-12. The members list excludes the viewer; `readers` on book detail excludes the viewer; `ClubPick.readers` includes the viewer** (Home finds "me" in it to drive the progress slider). Changing any of these silently breaks counts ("N members", "N of M finished") or the Home progress slider.
- **R-13. Diary spoilers are client-side only.**
  - The full body is sent to everyone and is present in the DOM on book pages (blurred).
  - The club feed must never put a shielded body in the DOM (tested).
  - If visibility rules move server-side (only-me), "only me" entries must be filtered **server-side**, never by hiding them in the client.
- **R-14. Reactions:** the toggle identity is the exact emoji string, and `❤️` includes U+FE0F. Comparing normalised or stripped strings breaks both toggling and the picker's "already used" filter. The server returns reactions sorted by count desc, then emoji.
- **R-15. Diary feed pagination is keyset on entry id** (`before` = an id, `id < before`, ordered id desc, deleted rows excluded, `has_more` via `limit+1`). Postgres must order by the same id, not by `created_at`, or "Show more" skips and duplicates rows.
- **R-16. Search history:**
  - The server dedupes by **casefold** but stores the latest spelling.
  - It returns the full list (≤ 20, `last_used_at` desc, `id` desc) from POST.
  - DELETE of another user's id is a 404 with the mapped string.
  - Postgres `lower()` is not casefold (for example `ß`); decide whether to match Python exactly.
- **R-17. Discover dedupes by `ol_work_key` across pages and stops on an empty page.** The new catalog's search must keep stable ids across pages, and local-plus-remote merged results must not repeat a work on later pages [plan #41 AC: "remote merged without duplicates"].
- **R-18. `/api/config` is awaited before mount with no timeout.** Whatever replaces it must be public, instant and never require a session, or the app shows a blank page.
- **R-19. `test_api_error_copy.py` is the only thing that keeps `apiErrors.ts` honest.** Port it as a Rust test, or the map silently rots and German users see English. Also note `errors.noPicture` maps a string that is only ever served to `<img>`.
- **R-20. Avatar upload size:** the client allows up to 50 MiB, but axum's `DefaultBodyLimit` is 2 MB. Raise the limit on that route (and on the Goodreads import, 1.5 MB plus multipart overhead), and return the mapped `TOO_LARGE` JSON with status 413, not axum's plain-text 413.
- **R-21. Multipart field names:** `file` for both the avatar and the Goodreads CSV. No other fields are sent.
- **R-22. Query parameter encoding:** `include_reading=true` (a literal `true`), `limit`/`page` as decimal strings, a percent-encoded subject in the path, and an `encodeURIComponent`ed `username` in the query. Rust `Query<bool>` must accept `true` (and Python also accepts `1`, `yes`, `on`). Search sends `sort=relevance` explicitly in most cases.
- **R-23. `spine-colors` keys are book ids as JSON object keys (strings).** The client indexes with `String(item.book.id)`. With works and editions, decide whether the colour belongs to the work or the edition and keep the key consistent with `item.book.id`.
- **R-24. Undo after delete loses the reading dates** (the undo calls `POST /api/shelf` without `finished_at`). Parity keeps this; an improved API could accept `finished_on` on create. Not a server bug, but don't "fix" it halfway (for example by accepting dates only on one of the two undo paths).
- **R-25. Vote leader inconsistency:** Home and the Club page resolve ties differently (2.4). If the server adds `leader_id` (it already exists, unused), use it in both places.
- **R-26. Cookie and same-origin assumptions:**
  - The SPA uses relative `/api` and `credentials: include`.
  - `<img src="/api/members/…/avatar">` needs the cookie on image requests (`SameSite=Lax`, same origin).
  - If media moves to a separate host, avatars must become public content-hashed URLs, or they 401.
- **R-27. Abort handling:** Discover and BookDetail abort in-flight requests on route change. Server handlers must tolerate client disconnects mid-request: no partial writes on GET, and nothing assumes the response was delivered.
- **R-28. Route guard order:** `hydrate()` runs before any route resolves. A slow `/api/auth/me` delays every navigation. The new `/me` (possibly with `clubs[]`) must stay cheap.

---

## 7. Open questions (not settled by plan.md, phase1-issues.md or Forgejo #40/#45/#46/#48)

1. **Book diary under multi-club:** diary entries are personal and audience-visible [plan], but today they carry replies and reactions from other users, and they *are* the club pick discussion on Home.
   - Does the club home show (a) the personal diary of the pick's work filtered to this club's members, or (b) club-scoped **pick posts** (`club_pick_posts`, with milestones) [plan lists pick posts as club data]?
   - Who owns a reply on someone's entry, and is it visible to the replier's audience or the entry author's?
   - How are legacy `book_posts` that were imported from pick posts (`legacy_post_id`) and `pick_id`-tagged entries mapped?
2. **Combined home feed:** which item types (diary entries, pick changes, votes, activity events, meetings) and from whom (club members only, or the whole audience)?
3. **"Events" in #48/#45:** the activity log (`events` table) or scheduled meetings? This decides the `/c/:slug/events` page.
4. **API club addressing:** by slug or by stable id, and can a slug be renamed (and do old slugs redirect)?
5. **Role enforcement responses:** for a member lacking a role (for example a non-admin setting the pick), is it 403 or 404, and with which mapped `detail`? Can members still nominate and vote [presumably yes: "member: everything else"]?
6. **Other users' pages:** `/u/:username` (personal, audience-gated) or `/c/:slug/members/:username`? Does a stranger get 404 identical to "no such user"?
7. **Work id format** in URLs and the API, and how legacy `/book/OL…W|BC…|ISBN…|GB…` links resolve.
8. **Is an edition picker** ("my edition") in Phase 1 frontend scope, and does the physical shelf use edition pages and covers?
9. **Is the cover photo upload and report UI** part of #48 or a later issue?
10. **Personal timezone:** a new user preference, or always the browser's zone? And what timezone does a new club default to?
11. **Sign-up before verification:** is the user logged in immediately, and what is blocked until the email is verified (joining clubs, posting)?
12. **CSRF protection** for cookie sessions: does the server require a header or token that `client.ts` must send?
13. **Does `/api/config` survive** (service branding until #50 picks the public name), or does the name become a build-time constant?
14. **Search history normalisation:** Python casefold versus Postgres `lower()`.
15. **Does the club rating** on a book page become per club, per audience, or global (catalog-wide) with a minimum count?
16. **Discover browse:** do "Trending" and the subject rows survive without live Open Library browse calls (#41 is local-first), or does Discover become search-only plus "new German releases" from the DNB harvest?
17. **Where does the built SPA live** after `backend/` is deleted, and does axum serve it or the reverse proxy?
18. **Goodreads import:** do skip `reason` strings become codes so they can be translated?
19. **Diary feed exclusions:** should "Recently written" exclude "only me" entries of others (obviously) and also entries on books the viewer has marked as spoiler-sensitive? Today there is no such setting.
