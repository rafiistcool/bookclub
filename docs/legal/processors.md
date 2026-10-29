# Processors and other recipients

Who outside the operator receives data from the hosted Bookclub service, and whether each one is a processor (Art. 28 DSGVO) that needs a data processing agreement (DPA, Auftragsverarbeitungsvertrag). Rafael files every DPA (work plan §5 step 18). The providers are the ones Rafael chose on 2026-10-29 (`privacy-facts.md` §0). "To sign" marks a DPA that is not signed yet, because the account only comes once the service goes live; "to confirm" marks a value that follows then, or a point for the legal source.

`server/tests/legal_docs.rs` checks that every outbound host in `server/src/config.rs` and `server/src/outbound.rs` appears here with a verdict.

## Summary

| Recipient | Host | Verdict | DPA |
|---|---|---|---|
| Mail provider | Brevo (Q4); the SMTP host and account follow with the domain | processor | needed: to sign |
| Backup target | Hetzner Storage Box (Hetzner Online GmbH), with the box's own snapshots (Q9) | processor (stores only ciphertext) | needed: to sign |
| Uptime check | Better Stack or UptimeRobot (Q10 A; which one is to confirm) | not a processor of users' data: it receives `{"status":"ok"}` | none for users' data |
| Heartbeats | healthchecks.io (Q10 A) | not a processor of users' data: picture ids are blanked out of the pings before they are sent; to confirm with the lawyer | none for users' data (below) |
| Open Library | `openlibrary.org`, `covers.openlibrary.org`, redirects to `archive.org` | not a processor: an independent public catalog that receives no personal data | none |
| Deutsche Nationalbibliothek | `services.dnb.de` | not a processor: an independent public catalog that receives no personal data | none |
| Hosting, reverse proxy, registry | the operator's own hardware | not a recipient: the operator runs them | none |
| Apple Push Notification service, Firebase Cloud Messaging | later (Phase 4, #60) | processor | needed then |
| Codemagic | later (Phase 4, #60) | builds the apps; receives no user data | none expected |

## Processors

### Mail provider: Brevo (Q4)

- **What it does:** delivers the service's mails over SMTP (`SMTP_URL`, `EMAIL_FROM`; `server/src/email/`). The server sends only over TLS: `smtps://`, or STARTTLS that must succeed (`server/src/config.rs`, `check_smtp_tls`).
- **Data:** the recipient address, the username, the text in the user's language: verification and reset links, the "email changed" and "address in use" notices, the admin notifications to instance admins (counts and ages only, no reporter; ADM), and after an admin decides reports (Q12 A, ADM2) the decision to each reporter and the statement of reasons to whoever uploaded or added what was removed: what was reported and the outcome or ground, never the other side's name or the report's details (`privacy-facts.md` §5). Every mail to a user ends with links to the imprint and the privacy policy. The provider also sees delivery metadata (time, status, bounces).
- **Provider:** Brevo, France (the legal entity as its DPA names it; Sendinblue SAS before the rename, to confirm). SMTPS or STARTTLS, DKIM on the service's domain, a standard DPA, a free tier for beta volume.
- **Third countries:** none planned; to confirm from the DPA's sub-processor list.
- **To sign:** the DPA, once the account exists. The account, the domain verification and the SMTP credentials come with the domain (work plan §5 step 8).
- **To fill in then:** the DPA date, the sub-processor list, the retention of sending logs.
- **Until then:** staging needs an interim SMTP account, which is a processor too and needs a DPA if real addresses go through it (work plan §0 item 9).

### Backup target: Hetzner Storage Box (Q9)

- **What it does:** holds the restic repository of the nightly backups: the database dump and the media files, encrypted by restic on the VM before they leave it, over SFTP (`deploy/hosted/backup.sh`; every snapshot of the last 14 days, then 7 daily, 4 weekly and 12 monthly snapshots, a weekly restore test with `deploy/hosted/restore-test.sh`).
- **Storage Box snapshots:** Hetzner's automatic snapshots of the box, set up in Hetzner's console, which the VM's SFTP user cannot delete (the Phase 2 security review, `docs/phase1/security-review.md`, finding 3, option A, chosen on 2026-10-29; DEP2 documents the setup in `deploy/hosted/README.md`). They hold the same ciphertext as the box.
- **Data:** every table and picture, only as ciphertext; the key stays with the operator (password manager, work plan §5 step 12). Hetzner cannot read it, but stores it on the operator's behalf, so it is a processor.
- **Provider:** Hetzner Online GmbH, Germany.
- **Third countries:** none, as long as the box is ordered in an EU location (to confirm when ordering).
- **To sign:** Hetzner's DPA, when the box is ordered.
- **Snapshots kept:** one a day, in as many slots as the box's plan has (BX11: 10, so about 10 days back; `deploy/hosted/README.md`, "Storage Box snapshots", DEP2).
- **To fill in then:** the DPA date, the box's plan and location.

## Monitoring (Q10 A)

The operator gets alerts when the service or one of its timers stops. Both services hold the operator's own account (address, phone number for alerts), which is not users' data.

### Uptime check: Better Stack or UptimeRobot (which one is to confirm)

- **What it does:** calls `GET /api/health` every few minutes and alerts the operator when the answer is not `{"status":"ok"}` (`server/src/health.rs`).
- **Data:** none of users. The answer is `{"status":"ok"}`, or `{"status":"unavailable"}` while the database does not answer.
- **Verdict:** not a processor of users' data, so no DPA is needed for them. If the provider offers a DPA anyway (for the operator's account), file it.
- **To fill in:** the provider, its location.

### Heartbeats: healthchecks.io

- **What it does:** receives pings from the backup, update and restore-test timers on the VM, from the VM's address: one when a run starts, one with the exit status when it ends, and alerts the operator when a ping is missing or reports a failure.
- **Data:** the end ping of the backup, the restore test and an update carries the last 50 lines of the run's log (`deploy/hosted/backup-lib.sh` (`heartbeat`, `log_tail`, `supervise`), `deploy/hosted/update.sh`): row counts per table, restic's summary, durations, and for an update the image and the backup before it. No username, address or content. Rarely, a line of the log names the storage key of a picture the run could not read, for example `avatars/ab/<public id>/a`: a picture deleted while the backup ran, or one a restore test missed. A public id is the secret part of the picture's URL (`privacy-facts.md` §2), so with the service's host it loads that picture. A failed restore test can also hit an error whose Postgres detail quotes a row (a duplicated address, a failing row of `users`).
- **Chosen for now (2026-10-29, as Rafael asked: take the recommended option, decide what matters at go-live):** the pings keep the log, which tells the operator why a run failed, but every public id (26 characters of `a-z` and `0-9`) is blanked out of it before it is sent, and so is every line Postgres adds to an error (`DETAIL`, `CONTEXT` and the like, which quote rows); the restore test keeps `pg_restore`'s full messages in a file on the VM and logs only its own error lines, with quoted values withheld. They then carry no personal data, so healthchecks.io is not a processor of users' data and needs no DPA for them. `log_tail` in `deploy/hosted/backup-lib.sh` does it for the backup, the restore test and an update; `server/tests/deploy_env.rs` (`update_pings_carry_no_public_id`, `supervise_pings_carry_no_public_id`, `supervise_pings_withhold_postgres_detail`, `a_failed_restore_test_pings_no_row`) checks that a public id or a row in the log does not reach the ping. The other option, signing healthchecks.io's DPA, stays open if the lawyer prefers it.
- **To fill in:** the location, and the lawyer's confirmation of the verdict.

## Not processors

### Open Library (Internet Archive)

- **Hosts:** `openlibrary.org` (search, works, subjects, trending; the cover search by title and author and the cover check by ISBN (`/isbn/`), also from the background cover job), `covers.openlibrary.org` (covers), which redirects to `archive.org` storage (`server/src/config.rs`, `DEFAULT_OPENLIBRARY_URL`, `DEFAULT_OPENLIBRARY_COVERS_URL`; `server/src/outbound.rs`, `REDIRECT_DOMAINS`).
- **Verdict:** not a processor. Open Library is an independent, public book catalog; the service uses it as a data source and processes nothing on Open Library's behalf, and Open Library processes nothing on the service's behalf. The requests carry no personal data of users.
- **What is sent:** the server asks with its own IP address and a User-Agent naming the service (`PUBLIC_URL`) and its operator contact (`CONTACT_EMAIL`, `server/src/outbound.rs:97-103`), as Open Library asks API users to. Each request holds a search text, an Open Library id, an ISBN, a subject, a title and author from a Goodreads import row, or, for the cover search (#74), a catalog book's title and its first author's surname (never a book a reader added; nothing says who reads it), never the user's IP address, cookie, account id or username. The browser never contacts Open Library (`privacy-facts.md` §6).
- **To confirm with the lawyer:** that a search text a user typed, sent without any reference to the user, is not personal data (it could contain a name only if the user searches for a person, which is an author in a book search).
- **Location:** USA (Internet Archive). No transfer of personal data, given the point above.

### Deutsche Nationalbibliothek

- **Host:** `services.dnb.de`: the SRU interface for ISBN lookups and the OAI-PMH interface for the optional new-releases harvest (`server/src/config.rs`, `DEFAULT_DNB_SRU_URL`, `DEFAULT_DNB_OAI_URL`).
- **Verdict:** not a processor: an independent public catalog used as a data source.
- **What is sent:** an ISBN (SRU), or the harvest's set and date range (OAI, no user input at all), with the same User-Agent. No personal data of users.
- **Location:** Germany.

### Hosting, reverse proxy and registry

- **Verdict:** not recipients. The service runs on the operator's own hardware: the app VM, the Caddy reverse proxy that terminates TLS (and keeps an access log with IP addresses for at most 7 days, `privacy-facts.md` §4), and the Forgejo registry that holds the image. Nobody else operates them.
- **Domain and DNS:** the registrar and the DNS provider see no data of users (to confirm once the domain is registered, Q1).

## Later (Phase 4, #60)

- **Apple Push Notification service and Firebase Cloud Messaging:** the iOS and Android apps will receive pushes through them. They will process device push tokens and the notification text: processors with their own DPAs (Apple, Google), with transfers to the USA (EU-US Data Privacy Framework; to confirm then).
- **Codemagic:** builds and signs the apps; it receives source code and signing keys, no user data. Not a processor of users' data (to confirm then).
- **Web Push for the browser app:** would add the browsers' push services (Mozilla, Google, Apple); decided with #60.
