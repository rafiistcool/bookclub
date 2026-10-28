# Privacy facts

These are the facts a privacy policy (Datenschutzerklärung) for the hosted Bookclub service needs, taken from the code with file references. They are input for eRecht24 or a lawyer, not legal text and not legal advice. Every "to confirm" is a decision Rafael still has to make, or a value a later package fills in.

The facts describe the Rust server (`server/`) and the Vue app (`frontend/`) on branch `phase-2`. Until the cutover (#52), production still runs the Python app in `backend/`; these facts apply from the cutover on.

Companion documents:
- `docs/legal/vvt.md`: the record of processing activities (Art. 30 DSGVO)
- `docs/legal/processors.md`: processors and the services that are not processors
- `docs/legal/takedown.md`: how notices about content are handled

## 0. Open decisions (to confirm)

| Decision | Recommendation (work plan §4) | Affects |
|---|---|---|
| Public name and domain (Q1, #50) | not decided | controller details, cookie origin, mail sender, `CONTACT_EMAIL` |
| Mail provider (Q4) | Brevo (FR), with a DPA | §5, processors |
| Backup target and retention (Q9) | Hetzner Storage Box with restic: 7 daily, 4 weekly, 12 monthly snapshots | §5, §7, processors |
| Uptime monitor (Q10) | a hosted HTTP check (Better Stack or UptimeRobot) plus healthchecks.io heartbeats | §5, processors |
| Takedown targets (Q12) | acknowledge within 24 h, decide within 72 h, the app mails the decision | `takedown.md` |
| Retention periods (Q14) | reports and edit requests 3 years; empty sign-ups 30 days; legacy data 30 days after the cutover | §7 |
| Reverse proxy log retention | 7 days, with IP addresses (work plan §5 step 17) | §4 |
| App log retention | the Docker log options of DEP's compose file | §4 |
| Minimum age | not in the code; the terms state it | terms |

## 1. Controller, hosting and contact

- **Controller:** Rafael, as the operator. Name and postal address go into the imprint (§ 5 DDG). To confirm: the address service (work plan §5 step 9).
- **Hosting:** the server runs on Rafael's own hardware (a Proxmox VM behind his own Caddy reverse proxy, on a business line with a static IP). No hosting provider processes the data. To confirm: the location (Germany) for the privacy text.
- **Contact:** one public contact and takedown address, `CONTACT_EMAIL` (default D3). The server publishes it in `GET /api/config` as `contact_email` (`server/src/auth/mod.rs`, `service_config`) and sends it in the User-Agent of its catalog requests (`server/src/outbound.rs:97-103`).
- **Legal texts:** the operator puts the imprint, privacy policy and terms (German and English) into `LEGAL_DIR`; the server serves them at `GET /api/legal/{doc}` and the app shows them at `/imprint`, `/privacy` and `/terms`, signed out too (`server/src/legal.rs`). Without them, the server refuses `SIGNUP_MODE=open` outside development.

## 2. What the server stores

Everything lives in one Postgres database (schema: `server/migrations/*.sql`) and one media directory (`MEDIA_DIR`). The tables are listed per processing activity in `vvt.md`.

| Category | Data | Where | Who can see it |
|---|---|---|---|
| Account | username, email address (optional until verified), verification time, password hash (argon2id), admin flag, theme, colour mode, locale, time zone, notification switches, profile picture reference, sign-up time, password change time, the time the email prompt was dismissed | `users` | the user; username and picture: the user's audience (people who share a club) |
| Terms acceptance | the accepted terms version and the time | `users.terms_version`, `users.terms_accepted_at` (`server/migrations/2200_users_terms.sql`) | the user (export) |
| Sessions | per signed-in device: a hash of the session secret, the kind (cookie or app token), the label (the browser's User-Agent for a cookie session, the name the app chose for a token), created, last used, expiry | `sessions` (`server/src/auth/session.rs:152-155`, `device_label` at `:196`) | the user (Settings, session list) |
| Mail links | for an open verification or reset link: a hash of the token, the address it goes to, created, expiry, used | `email_tokens` (`server/src/auth/mail.rs:90-93`) | nobody; the address of a pending email change is shown to the user |
| Reading data | shelf entries (status, rating, take, reading dates, progress, edition, did-not-finish reason, visibility of the rating), favourites, quotes, the physical shelf layout, the last 20 searches | `shelf_entries`, `user_favorites`, `quotes`, `physical_shelf_layouts`, `search_history` | the user; the audience, except items marked "only me" (docs/phase1/conventions.md §6.1) |
| Goodreads import | the parsed rows of an uploaded CSV while the import runs; afterwards the counts and up to 25 skipped rows (title and reason) | `import_jobs` (`server/src/shelf/goodreads.rs:609-611`, `:667`) | the user |
| Diary and discussions | posts (diary entries, pick discussions, replies), their visibility and spoiler marks, reactions | `posts`, `post_reactions` | personal entries: the audience or only the author; club posts: the club's members |
| Clubs | memberships with role and who invited whom, removals, clubs created, invite codes and their uses, picks, milestones, nominations and ballots, the activity feed | `club_memberships`, `club_removals`, `clubs`, `club_invites`, `club_invite_uses`, `club_picks`, `milestones`, `next_up_nominations`, `next_up_ballots`, `events` | the club's members |
| Catalog contributions | works and editions a user added, cover photos a user uploaded, edit, merge and cover requests | `works.created_by`, `editions.created_by`, `media.uploaded_by`, `catalog_edit_requests` | the books: every signed-in user, without the contributor's name; the requests: instance admins |
| Pictures | profile pictures and cover photos, re-encoded as WebP without metadata (EXIF and GPS are dropped, `server/src/media/pipeline.rs:12-14`) | `media`, files under `MEDIA_DIR` | profile picture: the audience; covers: everyone signed in |
| Reports and takedowns | who reported which picture, the reason and details, who resolved it and when; hashes of removed pictures | `reports`, `media_removed_hashes` | instance admins |

Not stored: IP addresses (see §4), payment data, location, contacts, tracking or advertising identifiers.

## 3. Storage in the browser

The app sets one cookie and uses `localStorage` and `sessionStorage` for a few preferences and drafts. There is no tracking, no analytics and no third-party cookie. To confirm with the lawyer: all of these are strictly necessary for the service the user asked for or store a choice the user made (§ 25 Abs. 2 Nr. 2 TDDDG), so no consent banner is needed.

| Name | Kind | Content | Lifetime | Source |
|---|---|---|---|---|
| `__Host-bookclub_session` (`bookclub_session` in development) | cookie, `HttpOnly`, `Secure`, `SameSite=Lax`, `Path=/` | a random session secret (32 bytes, base64url) | `Max-Age` 400 days; the server ends the session after 30 days without use, at logout, password change, reset or account deletion | `server/src/auth/mod.rs` (`SESSION_COOKIE`, `SECURE_SESSION_COOKIE`), `server/src/auth/session.rs:29-30` |
| `bookclub.theme`, `bookclub.mode` | `localStorage` | the chosen palette and light/dark mode, so the page paints in them before the app starts | until changed or cleared | `frontend/src/stores/theme.ts:20-21`, `frontend/index.html:21-22` |
| `bookclub.locale` | `localStorage` | the chosen language | until changed or cleared | `frontend/src/i18n/index.ts:8` |
| `bookclub.club` | `localStorage` | the slug of the club opened last | until changed, or the club is gone | `frontend/src/stores/clubs.ts:8` |
| `bookclub.draft.<username>.…` | `localStorage` | the text of an unsent diary entry or reply | until sent or emptied; all drafts are removed when the user signs out on this device or deletes the account | `frontend/src/drafts.ts:3-41` |
| `bookclub.join` | `sessionStorage` | an invite code opened while signed out, to join after sign-in | the browser tab | `frontend/src/invites.ts:4` |
| `bookclub.import-job` | `sessionStorage` | the id of a running Goodreads import, to show its progress | the browser tab | `frontend/src/goodreads.ts:8` |
| `bookclub.asset-reload-at` | `sessionStorage` | the time of the last automatic reload after a release, so a stale page reloads at most once a minute | the browser tab | `frontend/src/assetRecovery.ts:4` |

The browser also keeps the app's own files (HTML, scripts, styles, icons) in its HTTP cache. The page shell is never cached (`Cache-Control: no-store`, `server/src/spa.rs:17`), and no API answer is cached (`no-store` on every API route, `server/src/lib.rs`).

## 4. Logs and IP addresses

- **The app log** (standard output of the container) has one line per request with the method, the path, the status, the latency and a random request id (`server/src/lib.rs`, `TraceLayer`). It has **no IP address, no User-Agent, no cookie and no user id**. Invite codes in paths are replaced by `{code}` (`loggable_path`). Outbound catalog requests are logged with their URL (`server/src/outbound.rs`), which contains the search text or the ISBN, but not who asked. Errors are logged without request bodies. Mails are never logged in production: the log transport, which writes every mail into the log, is refused outside development (`server/src/config.rs`, `EMAIL_TRANSPORT`). Retention: the Docker log options of the deployment (to confirm, DEP).
- **The reverse proxy** (Caddy, shared with Forgejo) logs every request with the client IP address, time, request line, status and User-Agent. Planned retention: 7 days (to confirm, DEP's `deploy/hosted/Caddyfile.example`).
- **Rate limiting** uses the client IP address (an IPv4 address, or the /64 prefix of an IPv6 address, `server/src/ratelimit.rs:314-322`), taken from `X-Forwarded-For` only when the proxy is trusted (`server/src/client_ip.rs:41-65`), plus account ids and SHA-256 hashes of login names that match no account (`:327-331`). These counters live in memory only, for at most 24 hours (the longest window, `:273-302`), and are gone after a restart. They are never written to disk or to the log.

## 5. Connections the server makes

The browser never talks to these hosts; the server does, from its own IP address, without the user's IP address, cookies, account id or username.

| Host | When | What is sent | Source |
|---|---|---|---|
| `openlibrary.org` | a catalog search, a book page seen for the first time, a Goodreads import, trending and subject lists | the search text, an Open Library work id, an ISBN, or a title and author from an imported row; a User-Agent with the service's `PUBLIC_URL` and `CONTACT_EMAIL` (the operator's, not the user's) | `server/src/catalog/openlibrary.rs:375-525`, `server/src/outbound.rs:97-103` |
| `covers.openlibrary.org`, redirected to `archive.org` | a cover is fetched once and then served from the server's own media store | the cover id | `server/src/media/service.rs:394`, `server/src/outbound.rs:31-33` |
| `services.dnb.de` (SRU) | an ISBN lookup | the ISBN | `server/src/catalog/dnb_sru.rs:26-31` |
| `services.dnb.de` (OAI-PMH) | the optional new-releases harvest (`DNB_HARVEST`) | nothing from users | `server/src/harvest/run.rs:170-185` |
| the mail provider (to confirm, Q4) | verification links, password reset links, the "email changed" and "address in use" notices; later admin notifications and takedown decisions | the recipient address, the username, the link, the text in the user's language | `server/src/email/templates.rs:39,73,110,154` |
| the backup target (to confirm, Q9) | nightly | the encrypted database dump and media files | DEP (#51) |
| the monitor (to confirm, Q10) | the monitor calls `GET /api/health`; the backup and update timers ping heartbeats | nothing personal: the health answer is `{"status":"ok"}` | `server/src/health.rs` |

## 6. The browser contacts no third party

- The app is one origin: the page, its scripts and styles, the API (`/api`) and every picture (`/media`) come from the service's own host. Covers from Open Library are fetched and stored by the server first (§5), so the browser never loads them from Open Library.
- No web fonts are loaded: the font stacks name fonts the device may have installed and fall back to system fonts (`frontend/src/styles/tokens.css:6-7`); the app ships no font files.
- No analytics, no error tracking, no CDN, no embedded media, no social plugins. GATE (#55) adds a Content-Security-Policy that limits the page to its own origin.
- The only way out is a link the user clicks: "View on Open Library" on a book page, which opens in a new tab without a referrer (`frontend/src/pages/BookDetailPage.vue:412-420`).
- The web app manifest and icons are local files (`frontend/public/`).

## 7. Retention

| Data | Deleted | Source |
|---|---|---|
| Sessions | after 30 days without use (app tokens: 90 days), at logout, at a password change (every other session), at a password reset (all), with the account; expired rows are removed by the hourly cleanup | `server/src/auth/session.rs:154`, `server/src/auth/mod.rs` (`cleanup`), `server/src/jobs.rs:24` |
| Mail links | valid 48 hours (verification) or 1 hour (reset); removed by the hourly cleanup once used or expired | `server/src/auth/mail.rs:90-93`, `server/src/auth/mod.rs` (`cleanup`) |
| Search history | only the newest 20 per account are kept; the user can delete single entries or all | `server/src/shelf/search_history.rs:21`, `:105-115` |
| Goodreads rows | the uploaded rows are emptied when the import ends; counts and up to 25 skipped titles stay until the account is deleted | `server/src/shelf/goodreads.rs:609-611`, `:667` |
| Account and everything in it | when the user deletes the account (§8). There is no deletion for inactivity yet. Planned (Q14, LEG2): delete sign-ups that never verified an address and hold no data after 30 days (to confirm) | `server/src/auth/deletion.rs` |
| Posts with replies by others | kept as an anonymous tombstone without text when the author's account is deleted | `docs/phase1/conventions.md` §5.6 |
| Club records (picks, nominations, milestones, invites) | kept without the name when the account is deleted | §5.6 |
| Cover photos a user uploaded and books a user added | kept as catalog content without the name when the account is deleted | §5.6 |
| Reports and decided edit requests | kept; the reporter or requester becomes anonymous when their account is deleted. Planned (Q14, LEG2): delete after 3 years (to confirm) | `server/migrations/0001_init.sql` (`reports`, `catalog_edit_requests`) |
| Removed pictures | the files are deleted at the takedown; the media row (with the uploader) and the hashes of the files stay, so the same picture cannot be uploaded again | `server/src/media/reports.rs:333-425` |
| Terms acceptance | with the account | `users` |
| Rate-limit counters | at most 24 hours, in memory | §4 |
| App log | the Docker log options (to confirm, DEP) | §4 |
| Proxy log | 7 days (to confirm, DEP) | §4 |
| Backups | 7 daily, 4 weekly and 12 monthly snapshots (to confirm, Q9): deleted data can stay in a backup for up to about 12 months | DEP (#51) |
| Data of the old Python app | the dev-VM copy right after the rehearsal; the final snapshot (encrypted) and the stopped Python container 30 days after the cutover (to confirm, Q14) | CUT (#52) |

## 8. Rights of the user

- **Access and portability (Art. 15, 20):** Settings → "Export my data" downloads a ZIP with `data.json` (every row that belongs to the user, from every table that references the account) and the pictures the user uploaded (`GET /api/auth/me/export`, `server/src/auth/export.rs:77`; contents in `docs/phase1/conventions.md` §5.7). Rows of clubs the user has left keep only what the user wrote. Five exports per hour.
- **Erasure (Art. 17):** Settings → "Delete account" with the password (`DELETE /api/auth/me`, `server/src/auth/deletion.rs:92`). Everything the user owns is deleted in one transaction: sessions, links, shelf, favourites, quotes, layout, search history, imports, memberships, reactions, ballots, events, profile pictures and posts (posts with replies by others become anonymous tombstones). An owner of a club with other members must hand it over or delete it first. Details: `docs/phase1/conventions.md` §5.6.
- **Rectification (Art. 16):** the user changes the email address, language, time zone and display settings in Settings. The username cannot be changed in the app; the operator changes it on request (to confirm how).
- **Suspended accounts:** a suspended account (planned, ADM2, Q15) cannot sign in and so cannot export or delete itself. Its owner writes to `CONTACT_EMAIL`, and the operator runs the export or deletion (ADM2 documents the steps in `takedown.md`).
- **Objection, restriction, complaint:** by mail to `CONTACT_EMAIL`; the privacy text names the supervisory authority (to confirm).

## 9. Terms acceptance

Recommended option of Q13: a sentence with links above the sign-up button, with the version and the time stored.

- The operator's `manifest.json` names the current terms version; `GET /api/config` publishes it as `terms_version` (`server/src/legal.rs`).
- A sign-up must send that version whenever the terms are configured; the server stores it with the time (`users.terms_version`, `users.terms_accepted_at`; `server/src/auth/signup.rs`).
- Accounts without an acceptance of the current version (imported accounts, and everyone after a new version) see a one-time prompt (`UserOut.needs_terms`), which records the acceptance (`POST /api/auth/terms`, `server/src/auth/account.rs`).
- Both columns are part of the export (§8).

## 10. Inside the service: who sees whose data

- **Strangers see nothing** of a user. People who share a club (the audience) see the username, the profile picture, the shelf, ratings and takes, diary entries, quotes, favourites, stats and the physical shelf, except items the user marked "only me" (`docs/phase1/conventions.md` §6.1).
- **Club members** see the club's posts, picks, votes, members and activity.
- **Instance admins** see the admin queue: reports (with the reporter's username), edit, merge and cover requests (with the requester's username), and the sign-up invites. No admin screen shows email addresses (to confirm when ADM2 adds decision mails, Q12).
- **Email addresses** are never shown to other users. Sign-up and the email change never reveal whether an address is in use (`docs/phase1/conventions.md` §5.5).
