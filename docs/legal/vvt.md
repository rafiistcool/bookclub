# Record of processing activities (Verzeichnis von Verarbeitungstätigkeiten)

The record of Art. 30 (1) DSGVO for the hosted Bookclub service, built from the code. Rafael signs it as the controller (work plan §5 step 18). The legal bases are proposals for the lawyer to confirm. The options are the ones Rafael decided on 2026-10-29 (`privacy-facts.md` §0); every "to confirm" is a value that follows once the service goes live, or a point for the legal source, and "to sign" a data processing agreement still to be signed.

`server/tests/legal_docs.rs` checks that every table with a foreign key to `users` appears in this file, so a new table with personal data fails the build until it is recorded here.

## Controller

| | |
|---|---|
| Controller | Rafael (name and postal address as in the imprint; to confirm) |
| Contact | `CONTACT_EMAIL` on the service's domain (to confirm, Q1) |
| Data protection officer | none: fewer than 20 people process personal data (§ 38 BDSG; to confirm) |
| Hosting | own hardware: a Proxmox VM behind the operator's Caddy reverse proxy, in Germany (to confirm) |
| Version | 2026-10-29, from branch `phase-2` (draft) |

## Technical and organisational measures (Art. 32)

Shared by every activity below. Sources are in `privacy-facts.md` and `docs/phase1/conventions.md`.

- **Transport:** TLS at the reverse proxy with HSTS (DEP, #51). Mail to the provider only over TLS: `smtps://` or STARTTLS that must succeed (`server/src/config.rs`, `check_smtp_tls`).
- **Authentication:** passwords hashed with argon2id (conventions §5.4); session secrets and mail-link tokens stored only as SHA-256 hashes (§5.1, §5.5); the session cookie is `__Host-`, `HttpOnly`, `Secure`, `SameSite=Lax`; cross-site writes are refused (§5.8); a password change ends every other session.
- **Abuse limits:** sign-up, login, reset, search, upload, report and export are rate limited in memory (`server/src/ratelimit.rs:273-302`).
- **Access control:** strangers see nothing of a user in the app (pictures are served to whoever has their unguessable URL, `privacy-facts.md` §2); the audience sees only what the user did not mark "only me"; club data only the club's members (conventions §6). Instance-admin routes sit behind one guard (`server/src/lib.rs`, `InstanceAdmin`).
- **Data minimisation:** no IP addresses in the app log or database; no third-party requests from the browser; pictures re-encoded without metadata; the Goodreads rows are emptied after the import.
- **Isolation:** the app VM sits in its own VLAN, accepts connections only from the reverse proxy, and cannot reach the LAN (work plan §5 step 10; to confirm at #51 acceptance).
- **Availability and recovery:** nightly backups of the database and the media to a Hetzner Storage Box, encrypted by restic on the VM before they leave it (the key stays with the operator), with a weekly restore test; the Storage Box's own snapshots, which the VM cannot delete (DEP2 sets them up), keep the backups safe from someone who takes over the VM (Q9; Phase 2 security review, finding 3, option A). An uptime check on `/api/health` and heartbeats from the backup, update and restore-test timers alert the operator (Q10 A).
- **Data subject rights:** in-app export and deletion (conventions §5.6, §5.7); by mail to `CONTACT_EMAIL` for an account that cannot sign in, such as a suspended one (activity 11).
- **Storage limitation:** the hourly cleanup deletes expired sessions and mail links, reports and catalog requests 3 years after their decision (a rejected cover request with its photo), and sign-ups that stayed empty for 30 days (Q14; `server/src/auth/mod.rs` (`cleanup`, `delete_rejected_cover_photos`), `privacy-facts.md` §7).

## Processing activities

Each activity lists its tables. "Retention" refers to `privacy-facts.md` §7.

### 1. Accounts, sign-in and sessions

| | |
|---|---|
| Purpose | Provide personal accounts: sign-up (by invite, later open), sign-in, sessions in the browser and the apps, password change and reset |
| Legal basis | Art. 6 (1) (b): performance of the contract (the terms of use) |
| Data subjects | users |
| Data | username, email address, verification time, password hash, preferences (theme, colour mode, language, time zone, notification switches), admin flag, sign-up and password change times, the last use of a session that ended before it expired (`last_seen_at`), the time of a suspension and of its lifting (`suspended_at`, `unsuspended_at`, activity 9); per session: the secret's hash, the kind, the device label (User-Agent or app name), created, last used, expiry; per mail link: the token's hash, the address, created, expiry, used |
| Tables | `users`, `sessions`, `email_tokens` |
| Recipients | none; mails go through the mail provider (activity 2) |
| Third countries | none |
| Retention | the account until the user deletes it; an empty sign-up (no verified address, no data, no sign-in for 30 days, not suspended) 30 days after its last sign-in or the lifting of a suspension (Q14); sessions 30 days idle (app tokens 90 days); mail links 48 h or 1 h, removed hourly once used or expired |
| Source | `server/src/auth/` (`signup.rs`, `login.rs`, `session.rs`, `tokens.rs`, `reset.rs`, `address.rs`, `account.rs`) |

### 2. Transactional email

| | |
|---|---|
| Purpose | Verify addresses, send password reset links, warn about an email change and about attempts to use a verified address, tell instance admins about new reports and catalog requests; after a decision on reports (Q12 A, ADM2), tell each reporter the decision and, when something was removed, whoever uploaded or added it the statement of reasons (activity 9). Every mail to a user ends with links to the imprint and the privacy policy |
| Legal basis | Art. 6 (1) (b); the security notices Art. 6 (1) (f) (protecting the account) |
| Data subjects | users with an email address |
| Data | email address, username, language, the link or notice; for a decision, what was reported (a cover with its book's title, a profile picture, a book or edition by its title), the outcome or the ground, never the other side's name or the report's details |
| Tables | `users`, `email_tokens`, `reports` |
| Recipients | Brevo (FR) as processor (Q4; DPA to sign; `processors.md`) |
| Third countries | none planned: Brevo is an EU provider (to confirm from its DPA and sub-processor list) |
| Retention | the provider's sending logs (to confirm from its DPA); nothing extra in the app |
| Source | `server/src/email/`, `server/src/auth/mail.rs`, `server/src/admin/notices.rs` (`decided`) |

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
| Data | the picture, re-encoded as a 256×256 WebP without metadata; one imported from the old app stays the 256×256 JPEG it stored, already without metadata (`server/src/importer/users.rs` (`import_avatar`)) |
| Tables | `media` (`kind = 'avatar'`), `users.avatar_media_id` |
| Recipients | in the app, the user's audience; the file is served without sign-in to whoever has its unguessable URL, and browsers may cache it for up to a year (`privacy-facts.md` §2) |
| Third countries | none |
| Retention | until replaced or removed, or with the account (copies in browser caches: up to a year); one replaced or deleted while it is reported is kept off the profile until an admin decides, then taken down or deleted (`privacy-facts.md` §7) |
| Source | `server/src/media/avatar.rs`, `server/src/media/pipeline.rs` |

### 8. Shared catalog contributions

| | |
|---|---|
| Purpose | Books and editions readers add, cover photos they upload, and their requests to correct, merge or replace catalog data |
| Legal basis | Art. 6 (1) (b) |
| Data subjects | users who contribute |
| Data | the contributed records and pictures, the contributor's id; requests with their note, the deciding admin and the decision |
| Tables | `works`, `editions`, `media` (`kind = 'cover'`, `uploaded_by`), `catalog_edit_requests` |
| Recipients | the books and covers: every signed-in user, without the contributor's name; the cover files also whoever has their URL, without sign-in (`privacy-facts.md` §2); the requests: instance admins |
| Third countries | none |
| Retention | the contributions stay as catalog content; the contributor's id becomes empty when the account is deleted; requests 3 years after their decision, open ones until decided (Q14); the photo of a rejected cover request, which never became a cover, with its request |
| Source | `server/src/catalog/`, `server/src/media/cover.rs` |

### 9. Notices about content and moderation

| | |
|---|---|
| Purpose | Receive and decide notices about content (in the app and by mail), remove illegal or infringing content, keep the record, tell the reporter and the uploader the decision (Q12 A, ADM2), and suspend accounts (Q15, ADM2) (`takedown.md`) |
| Legal basis | Art. 6 (1) (c) with Art. 16, 17 DSA (notice and action, statement of reasons; to confirm that they apply); otherwise Art. 6 (1) (f) (keeping the service lawful) |
| Data subjects | reporters; uploaders of reported covers, owners of reported profile pictures, readers who added reported books and editions; people a notice names; suspended users |
| Data | the report: its target (a cover, a profile picture with whose it is (`target_user_id`), or a work or edition a reader added (`target_work_id`)), reason, details, reporter, how it came in (`via`: the app or a notice by mail), time; the decision (admin, time, outcome, closed on arrival); hashes of removed pictures; a reported profile picture its owner replaced, held until the decision; notices received by mail; a suspension (its time, and the time it was lifted; the account's sessions are deleted) |
| Tables | `reports`, `media_removed_hashes`, `media` (`removed_at`), `users` (`suspended_at`, `server/migrations/2102_users_suspended_at.sql`; `unsuspended_at`, `server/migrations/2900_users_unsuspended_at.sql`) |
| Recipients | instance admins; by mail through Brevo (ADM2): each reporter the decision, and whoever uploaded or added removed content the statement of reasons, neither naming the other |
| Third countries | none |
| Retention | reports 3 years after the decision, open ones until decided (Q14); hashes as long as the block must hold; a suspension until it is lifted, or with the account; the time of its lifting with the account |
| Source | `server/src/media/reports.rs`, `server/src/catalog/reports.rs`, `server/src/admin/notices.rs`, `server/src/admin/users.rs`; ADM and ADM2 (#54, #55) |

### 10. Security and abuse prevention

| | |
|---|---|
| Purpose | Keep the service available and accounts safe: rate limits, request logs, the reverse proxy log |
| Legal basis | Art. 6 (1) (f) (security of the service, Recital 49) |
| Data subjects | every visitor |
| Data | in memory: IP addresses (IPv6 by /64), account ids, session ids, hashed unknown login names and recipient email addresses (case-folded, the keys of the per-address mail limits: 6 mails of each kind a day, also for addresses of people without an account) with their counters; in the app log: method, path, status, latency, request id (no IP); in the proxy log: IP address, time, method, path (query and invite codes redacted), status, request headers with the User-Agent (no cookies, no Referer) |
| Tables | none (memory and log files) |
| Recipients | none |
| Third countries | none |
| Retention | counters at most 24 h and gone at restart; app log by size, at most 5 files of 10 MB (no time limit; to confirm with the lawyer); proxy log at most 7 days |
| Source | `server/src/ratelimit.rs`, `server/src/client_ip.rs`, `server/src/auth/mail.rs` (`target_key`, `deliver`), `server/src/lib.rs` (`TraceLayer`); `deploy/hosted/compose.yaml` (`x-logging`), `deploy/hosted/Caddyfile.example` |

### 11. Requests of data subjects

| | |
|---|---|
| Purpose | Answer access, portability and erasure requests: in the app (export, deletion), and by mail to `CONTACT_EMAIL` for accounts that cannot sign in. A suspended account (Q15, ADM2) cannot sign in, so the operator answers its owner's export and deletion requests by hand (`takedown.md` §7; `privacy-facts.md` §8) |
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
| Data | a copy of the database (a dump) and the media files, encrypted by restic on the VM; the provider holds only ciphertext |
| Tables | all |
| Recipients | Hetzner Online GmbH (Storage Box) as processor (Q9; DPA to sign; `processors.md`) |
| Third countries | none, as long as the box is ordered in an EU location (to confirm when ordering) |
| Retention | restic: every snapshot of the last 14 days, then 7 daily, 4 weekly, 12 monthly snapshots; the Storage Box's own snapshots on top, one a day in as many slots as the box's plan has (BX11: 10; the plan is to confirm), so a deleted item can stay up to about 12 months plus the age of the oldest Storage Box snapshot (`privacy-facts.md` §7) |
| Source | `deploy/hosted/backup.sh`, `deploy/hosted/restore-test.sh`, `deploy/hosted/README.md` ("Backups") |

### 13. Availability monitoring

| | |
|---|---|
| Purpose | Alert the operator when the service or a timer stops |
| Legal basis | Art. 6 (1) (f) |
| Data subjects | none: picture ids are blanked out of the heartbeats before they are sent, so they name no user (`privacy-facts.md` §0, §5) |
| Data | the health answer (`{"status":"ok"}`); per heartbeat the job's exit status and the last 50 lines of its log (row counts, restic's summary), with every picture's public id blanked out (`privacy-facts.md` §5) |
| Tables | none |
| Recipients | the uptime check (Better Stack or UptimeRobot, to confirm) and healthchecks.io (Q10 A; `processors.md`) |
| Third countries | to confirm with the chosen services |
| Retention | the services' check and ping history (to confirm) |
| Source | `server/src/health.rs`; `deploy/hosted/backup-lib.sh` (`heartbeat`, `log_tail`), `deploy/hosted/update.sh` |

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
| Retention | the copy of the production database on the development VM is deleted right after the rehearsal; the final snapshot taken at the cutover (kept encrypted) and the stopped Python container with its data are deleted 30 days after the cutover (Q14) |
| Source | `server/src/importer/` |

### 15. Instance administration

| | |
|---|---|
| Purpose | Let the operator mint sign-up invites, work the admin queue and suspend accounts (activity 9) |
| Legal basis | Art. 6 (1) (f) |
| Data subjects | instance admins; users who use a sign-up invite |
| Data | the admin flag; sign-up invites (code, creator, uses) |
| Tables | `users` (`is_admin`), `club_invites` (`club_id IS NULL`), `club_invite_uses` |
| Recipients | instance admins |
| Third countries | none |
| Retention | invites until revoked or used up; the record of uses with the account |
| Source | `server/src/admin/invites.rs`, the `make-admin` and `beta-invite` commands of `server/src/main.rs` (`MakeAdmin`, `BetaInvite`) |

## Tables with personal data

Every table that references `users`, and the activities that use it.

| Table | Activities |
|---|---|
| `users` | 1, 2, 3, 7, 9, 15 |
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
