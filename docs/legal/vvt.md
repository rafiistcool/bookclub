# Record of processing activities (Verzeichnis von Verarbeitungstätigkeiten)

The record of Art. 30 (1) DSGVO for the hosted Bookclub service, built from the code. Rafael signs it as the controller (work plan §5 step 18). The legal bases are proposals for the lawyer to confirm; every "to confirm" is an open decision (`privacy-facts.md` §0).

`server/tests/legal_docs.rs` checks that every table with a foreign key to `users` appears in this file, so a new table with personal data fails the build until it is recorded here.

## Controller

| | |
|---|---|
| Controller | Rafael (name and postal address as in the imprint; to confirm) |
| Contact | `CONTACT_EMAIL` on the service's domain (to confirm, Q1) |
| Data protection officer | none: fewer than 20 people process personal data (§ 38 BDSG; to confirm) |
| Hosting | own hardware: a Proxmox VM behind the operator's Caddy reverse proxy, in Germany (to confirm) |
| Version | 2026-10-28, from branch `phase-2` (draft) |

## Technical and organisational measures (Art. 32)

Shared by every activity below. Sources are in `privacy-facts.md` and `docs/phase1/conventions.md`.

- **Transport:** TLS at the reverse proxy with HSTS (DEP, #51). Mail to the provider only over TLS: `smtps://` or STARTTLS that must succeed (`server/src/config.rs`, `check_smtp_tls`).
- **Authentication:** passwords hashed with argon2id (conventions §5.4); session secrets and mail-link tokens stored only as SHA-256 hashes (§5.1, §5.5); the session cookie is `__Host-`, `HttpOnly`, `Secure`, `SameSite=Lax`; cross-site writes are refused (§5.8); a password change ends every other session.
- **Abuse limits:** sign-up, login, reset, search, upload, report and export are rate limited in memory (`server/src/ratelimit.rs:273-302`).
- **Access control:** strangers see nothing of a user; the audience sees only what the user did not mark "only me"; club data only the club's members (conventions §6). Instance-admin routes sit behind one guard (`server/src/lib.rs`, `InstanceAdmin`).
- **Data minimisation:** no IP addresses in the app log or database; no third-party requests from the browser; pictures re-encoded without metadata; the Goodreads rows are emptied after the import.
- **Isolation:** the app VM sits in its own VLAN, accepts connections only from the reverse proxy, and cannot reach the LAN (work plan §5 step 10; to confirm at #51 acceptance).
- **Availability and recovery:** nightly encrypted backups with a weekly restore test (Q9, to confirm); an uptime monitor (Q10, to confirm).
- **Data subject rights:** in-app export and deletion (conventions §5.6, §5.7).

## Processing activities

Each activity lists its tables. "Retention" refers to `privacy-facts.md` §7.

### 1. Accounts, sign-in and sessions

| | |
|---|---|
| Purpose | Provide personal accounts: sign-up (by invite, later open), sign-in, sessions in the browser and the apps, password change and reset |
| Legal basis | Art. 6 (1) (b): performance of the contract (the terms of use) |
| Data subjects | users |
| Data | username, email address, verification time, password hash, preferences (theme, colour mode, language, time zone, notification switches), admin flag, sign-up and password change times; per session: the secret's hash, the kind, the device label (User-Agent or app name), created, last used, expiry; per mail link: the token's hash, the address, created, expiry, used |
| Tables | `users`, `sessions`, `email_tokens` |
| Recipients | none; mails go through the mail provider (activity 2) |
| Third countries | none |
| Retention | the account until the user deletes it; sessions 30 days idle (app tokens 90 days); mail links 48 h or 1 h, removed hourly once used or expired |
| Source | `server/src/auth/` (`signup.rs`, `login.rs`, `session.rs`, `tokens.rs`, `reset.rs`, `address.rs`, `account.rs`) |

### 2. Transactional email

| | |
|---|---|
| Purpose | Verify addresses, send password reset links, warn about an email change and about attempts to use a verified address; later: admin notifications (ADM) and takedown decisions (ADM2) |
| Legal basis | Art. 6 (1) (b); the security notices Art. 6 (1) (f) (protecting the account) |
| Data subjects | users with an email address |
| Data | email address, username, language, the link or notice |
| Tables | `users`, `email_tokens` |
| Recipients | the mail provider as processor (to confirm, Q4; `processors.md`) |
| Third countries | none if an EU provider is chosen (to confirm) |
| Retention | the provider's sending logs (to confirm with its DPA); nothing extra in the app |
| Source | `server/src/email/`, `server/src/auth/mail.rs` |

### 3. Acceptance of the terms of use

| | |
|---|---|
| Purpose | Record which version of the terms a user accepted and when, as evidence of the contract; ask again after a new version |
| Legal basis | Art. 6 (1) (b); keeping the evidence Art. 6 (1) (f) |
| Data subjects | users |
| Data | accepted terms version, time of acceptance |
| Tables | `users` (`terms_version`, `terms_accepted_at`, `server/migrations/2200_users_terms.sql`) |
| Recipients | none |
| Third countries | none |
| Retention | with the account |
| Source | `server/src/legal.rs`, `server/src/auth/signup.rs`, `server/src/auth/account.rs` (`accept_terms`) |

### 4. Personal reading data

| | |
|---|---|
| Purpose | The user's shelf: books, status, ratings and takes, reading dates and progress, favourites, quotes, the physical shelf layout, recent searches, the Goodreads import |
| Legal basis | Art. 6 (1) (b) |
| Data subjects | users |
| Data | the entries and their visibility; search texts (newest 20); the rows of an uploaded Goodreads CSV while it is imported, then counts and up to 25 skipped titles |
| Tables | `shelf_entries`, `user_favorites`, `quotes`, `physical_shelf_layouts`, `search_history`, `import_jobs` |
| Recipients | the user's audience (people who share a club), except "only me" items; search texts and imported titles and ISBNs go to Open Library and the DNB without the user's identity (`processors.md`) |
| Third countries | Open Library (Internet Archive, USA) receives search texts, titles and ISBNs, but no personal reference (`processors.md`; to confirm that this is no transfer of personal data) |
| Retention | until the user deletes them or the account |
| Source | `server/src/shelf/`, `server/src/catalog/search.rs` |

### 5. Diary entries and discussions

| | |
|---|---|
| Purpose | Diary entries on books, discussions of a club's pick, replies and reactions |
| Legal basis | Art. 6 (1) (b) |
| Data subjects | users; people a user writes about (free text) |
| Data | text, spoiler marks, visibility, progress at the time of writing, reactions |
| Tables | `posts`, `post_reactions` |
| Recipients | personal entries: the audience or only the author; club posts: the club's members |
| Third countries | none |
| Retention | until the author deletes them or the account; a post others replied to stays as an anonymous tombstone without text |
| Source | `server/src/posts/` |

### 6. Clubs

| | |
|---|---|
| Purpose | Reading clubs of up to 20 members (`server/src/clubs/mod.rs:29`): membership and roles, invites, the current pick and its milestones, voting on the next book, the activity feed |
| Legal basis | Art. 6 (1) (b) |
| Data subjects | users who create, join or are invited to clubs |
| Data | membership, role, who invited whom, removals, invite codes and their uses, picks with meeting notes, milestones, nominations and ballots, activity events |
| Tables | `clubs`, `club_memberships`, `club_removals`, `club_invites`, `club_invite_uses`, `club_picks`, `milestones`, `next_up_nominations`, `next_up_ballots`, `events` |
| Recipients | the club's members |
| Third countries | none |
| Retention | memberships, ballots and events with the account; picks, nominations, milestones and invites stay with the club without the name (conventions §5.6); everything with the club when its owner deletes it |
| Source | `server/src/clubs/`, `server/src/picks/`, `server/src/votes/`, `server/src/activity/` |

### 7. Profile pictures

| | |
|---|---|
| Purpose | A picture next to the username |
| Legal basis | Art. 6 (1) (b) (optional feature the user chooses) |
| Data subjects | users |
| Data | the picture, re-encoded as a 256×256 WebP without metadata |
| Tables | `media` (`kind = 'avatar'`), `users.avatar_media_id` |
| Recipients | the user's audience |
| Third countries | none |
| Retention | until replaced or removed, or with the account |
| Source | `server/src/media/avatar.rs`, `server/src/media/pipeline.rs` |

### 8. Shared catalog contributions

| | |
|---|---|
| Purpose | Books and editions readers add, cover photos they upload, and their requests to correct, merge or replace catalog data |
| Legal basis | Art. 6 (1) (b) |
| Data subjects | users who contribute |
| Data | the contributed records and pictures, the contributor's id; requests with their note, the deciding admin and the decision |
| Tables | `works`, `editions`, `media` (`kind = 'cover'`, `uploaded_by`), `catalog_edit_requests` |
| Recipients | the books and covers: every signed-in user, without the contributor's name; the requests: instance admins |
| Third countries | none |
| Retention | the contributions stay as catalog content; the contributor's id becomes empty when the account is deleted; decided requests 3 years (to confirm, Q14) |
| Source | `server/src/catalog/`, `server/src/media/cover.rs` |

### 9. Notices about content and moderation

| | |
|---|---|
| Purpose | Receive and decide notices about content (in the app and by mail), remove illegal or infringing content, keep the record, and (planned) tell the reporter and the uploader the decision (`takedown.md`) |
| Legal basis | Art. 6 (1) (c) with Art. 16, 17 DSA (notice and action, statement of reasons; to confirm that they apply); otherwise Art. 6 (1) (f) (keeping the service lawful) |
| Data subjects | reporters, uploaders, people a notice names |
| Data | the report (target, reason, details, reporter, time), the decision (admin, time, outcome); hashes of removed pictures; notices received by mail |
| Tables | `reports`, `media_removed_hashes`, `media` (`removed_at`) |
| Recipients | instance admins; the reporter and the uploader for the decision (planned, ADM2) |
| Third countries | none |
| Retention | reports 3 years after the decision (to confirm, Q14); hashes as long as the block must hold |
| Source | `server/src/media/reports.rs`, ADM and ADM2 (#54, #55) |

### 10. Security and abuse prevention

| | |
|---|---|
| Purpose | Keep the service available and accounts safe: rate limits, request logs, the reverse proxy log |
| Legal basis | Art. 6 (1) (f) (security of the service, Recital 49) |
| Data subjects | every visitor |
| Data | in memory: IP addresses (IPv6 by /64), account ids and hashed unknown login names with their counters; in the app log: method, path, status, latency, request id (no IP); in the proxy log: IP address, time, request, status, User-Agent |
| Tables | none (memory and log files) |
| Recipients | none |
| Third countries | none |
| Retention | counters at most 24 h and gone at restart; app log per the Docker log options (to confirm); proxy log 7 days (to confirm) |
| Source | `server/src/ratelimit.rs`, `server/src/client_ip.rs`, `server/src/lib.rs` (`TraceLayer`); DEP's Caddyfile |

### 11. Requests of data subjects

| | |
|---|---|
| Purpose | Answer access, portability and erasure requests: in the app (export, deletion) and by mail for accounts that cannot sign in |
| Legal basis | Art. 6 (1) (c) with Art. 12 to 22 DSGVO |
| Data subjects | users |
| Data | every table above; the request mail |
| Tables | all of the above (the export reads every foreign key to `users`, conventions §5.7) |
| Recipients | the requesting user |
| Third countries | none |
| Retention | the export is built on request and not kept; request mails 3 years (to confirm) |
| Source | `server/src/auth/export.rs`, `server/src/auth/deletion.rs` |

### 12. Backups

| | |
|---|---|
| Purpose | Restore the service after a failure |
| Legal basis | Art. 6 (1) (f) and Art. 32 (1) (c) |
| Data subjects | all users |
| Data | an encrypted copy of the database and the media files |
| Tables | all |
| Recipients | the backup target as processor (to confirm, Q9; `processors.md`) |
| Third countries | none if Hetzner (DE) is chosen (to confirm) |
| Retention | 7 daily, 4 weekly, 12 monthly snapshots (to confirm, Q9) |
| Source | DEP (#51) |

### 13. Availability monitoring

| | |
|---|---|
| Purpose | Alert the operator when the service or a timer stops |
| Legal basis | Art. 6 (1) (f) |
| Data subjects | none: the health check and the heartbeats carry no personal data |
| Data | `{"status":"ok"}`; heartbeat pings |
| Tables | none |
| Recipients | the monitoring service (to confirm, Q10; `processors.md`) |
| Third countries | depends on the service (to confirm) |
| Retention | the service's check history |
| Source | `server/src/health.rs` |

### 14. Migration of the old app's data (one-off, at the cutover)

| | |
|---|---|
| Purpose | Move the club's accounts, shelves, posts and pictures from the Python app into the new service, so the members keep their data |
| Legal basis | Art. 6 (1) (b) (continuing the service the members use); to confirm with the lawyer whether imported members must be informed beforehand (the cutover announcement, work plan §5 step 20) |
| Data subjects | members of the existing club |
| Data | everything the old app held, mapped per `docs/phase1/legacy-mapping.md` |
| Tables | all of the above; the importer's mapping `legacy_work_keys` holds no personal data |
| Recipients | none |
| Third countries | none |
| Retention | the dev-VM copy is deleted right after the rehearsal; the final snapshot (encrypted) and the stopped Python container 30 days after the cutover (to confirm, Q14) |
| Source | `server/src/importer/` |

### 15. Instance administration

| | |
|---|---|
| Purpose | Let the operator mint sign-up invites and work the admin queue |
| Legal basis | Art. 6 (1) (f) |
| Data subjects | instance admins; users who use a sign-up invite |
| Data | the admin flag; sign-up invites (code, creator, uses) |
| Tables | `users` (`is_admin`), `club_invites` (`club_id IS NULL`), `club_invite_uses` |
| Recipients | instance admins |
| Third countries | none |
| Retention | invites until revoked or used up; the record of uses with the account |
| Source | `server/src/admin/invites.rs`, `server/src/main.rs` (`make-admin`, `beta-invite`) |

## Tables with personal data

Every table that references `users`, and the activities that use it.

| Table | Activities |
|---|---|
| `users` | 1, 2, 3, 7, 15 |
| `sessions` | 1 |
| `email_tokens` | 1, 2 |
| `shelf_entries` | 4 |
| `user_favorites` | 4 |
| `quotes` | 4 |
| `physical_shelf_layouts` | 4 |
| `search_history` | 4 |
| `import_jobs` | 4 |
| `posts` | 5 |
| `post_reactions` | 5 |
| `clubs` | 6 |
| `club_memberships` | 6 |
| `club_removals` | 6 |
| `club_invites` | 6, 15 |
| `club_invite_uses` | 6, 15 |
| `club_picks` | 6 |
| `milestones` | 6 |
| `next_up_nominations` | 6 |
| `next_up_ballots` | 6 |
| `events` | 6 |
| `media` | 7, 8, 9 |
| `works` | 8 |
| `editions` | 8 |
| `catalog_edit_requests` | 8, 9 |
| `reports` | 9 |
| `media_removed_hashes` | 9 (no reference to a user, but tied to a picture's uploader through `media`) |
