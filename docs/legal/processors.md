# Processors and other recipients

Who outside the operator receives data from the hosted Bookclub service, and whether each one is a processor (Art. 28 DSGVO) that needs a data processing agreement (DPA, Auftragsverarbeitungsvertrag). Rafael files every DPA (work plan §5 step 18). Entries marked "to confirm" wait for a decision (`privacy-facts.md` §0).

`server/tests/legal_docs.rs` checks that every outbound host in `server/src/config.rs` and `server/src/outbound.rs` appears here with a verdict.

## Summary

| Recipient | Host | Verdict | DPA |
|---|---|---|---|
| Mail provider | to confirm (Q4: Brevo recommended) | processor | needed |
| Backup target | to confirm (Q9: Hetzner Storage Box recommended) | processor | needed |
| Uptime monitor and heartbeats | to confirm (Q10: Better Stack or UptimeRobot, healthchecks.io) | no personal data of users; processor only if the provider says so | to confirm |
| Open Library | `openlibrary.org`, `covers.openlibrary.org`, redirects to `archive.org` | not a processor: an independent public catalog that receives no personal data | none |
| Deutsche Nationalbibliothek | `services.dnb.de` | not a processor: an independent public catalog that receives no personal data | none |
| Hosting, reverse proxy, registry | the operator's own hardware | not a recipient: the operator runs them | none |
| Apple Push Notification service, Firebase Cloud Messaging | later (Phase 4, #60) | processor | needed then |
| Codemagic | later (Phase 4, #60) | builds the apps; receives no user data | none expected |

## Processors

### Mail provider (to confirm, Q4)

- **What it does:** delivers the service's mails over SMTP (`SMTP_URL`, `EMAIL_FROM`; `server/src/email/`).
- **Data:** recipient address, username, the text in the user's language, verification and reset links; later admin notifications (ADM) and takedown decisions (ADM2). The provider also sees delivery metadata (time, status, bounces).
- **Recommendation:** Brevo (Sendinblue SAS, France): SMTPS or STARTTLS, DKIM on the service's domain, a standard DPA, a free tier for beta volume. Alternatives: Scaleway Transactional Email (FR), Mailjet (FR); a mailbox provider's SMTP (mailbox.org, Posteo) is not meant for transactional mail.
- **Third countries:** none with an EU provider (to confirm the provider's sub-processors).
- **To fill in:** provider, DPA date, sub-processor list, retention of sending logs.
- **Until Q4 is answered:** staging needs an interim SMTP account, which is a processor too and needs a DPA if real addresses go through it (work plan §0 item 9).

### Backup target (to confirm, Q9)

- **What it does:** stores the nightly backups: a database dump and the media files, encrypted with restic before they leave the VM (DEP, #51).
- **Data:** every table and picture, but only as ciphertext; the key stays with the operator (password manager, work plan §5 step 12).
- **Recommendation:** Hetzner Storage Box (Hetzner Online GmbH, Germany) with restic: 7 daily, 4 weekly, 12 monthly snapshots, a weekly restore test.
- **Third countries:** none with Hetzner (to confirm).
- **To fill in:** provider, DPA date, location, retention.

## Probably not processors

### Uptime monitor and heartbeats (to confirm, Q10)

- **What it does:** calls `GET /api/health` (answer `{"status":"ok"}`, `server/src/health.rs`) and receives heartbeat pings from the backup, update and restore-test timers on the VM.
- **Data:** no personal data of users. The pings come from the VM's address; the alerts go to the operator.
- **Verdict:** no processing of users' personal data, so no DPA is required for them. If the provider offers a DPA anyway (for the operator's own account data), file it.
- **Recommendation:** a hosted HTTP keyword check (Better Stack or UptimeRobot, free tier) plus healthchecks.io heartbeats.
- **To fill in:** provider, location.

## Not processors

### Open Library (Internet Archive)

- **Hosts:** `openlibrary.org` (search, works, subjects, trending), `covers.openlibrary.org` (covers), which redirects to `archive.org` storage (`server/src/config.rs`, `DEFAULT_OPENLIBRARY_URL`, `DEFAULT_OPENLIBRARY_COVERS_URL`; `server/src/outbound.rs`, `REDIRECT_DOMAINS`).
- **Verdict:** not a processor. Open Library is an independent, public book catalog; the service uses it as a data source and processes nothing on Open Library's behalf, and Open Library processes nothing on the service's behalf. The requests carry no personal data of users.
- **What is sent:** the server asks with its own IP address and a User-Agent naming the service (`PUBLIC_URL`) and its operator contact (`CONTACT_EMAIL`, `server/src/outbound.rs:97-103`), as Open Library asks API users to. Each request holds a search text, an Open Library id, an ISBN, a subject, or a title and author from a Goodreads import row, never the user's IP address, cookie, account id or username. The browser never contacts Open Library (`privacy-facts.md` §6).
- **To confirm with the lawyer:** that a search text a user typed, sent without any reference to the user, is not personal data (it could contain a name only if the user searches for a person, which is an author in a book search).
- **Location:** USA (Internet Archive). No transfer of personal data, given the point above.

### Deutsche Nationalbibliothek

- **Host:** `services.dnb.de`: the SRU interface for ISBN lookups and the OAI-PMH interface for the optional new-releases harvest (`server/src/config.rs`, `DEFAULT_DNB_SRU_URL`, `DEFAULT_DNB_OAI_URL`).
- **Verdict:** not a processor: an independent public catalog used as a data source.
- **What is sent:** an ISBN (SRU), or the harvest's set and date range (OAI, no user input at all), with the same User-Agent. No personal data of users.
- **Location:** Germany.

### Hosting, reverse proxy and registry

- **Verdict:** not recipients. The service runs on the operator's own hardware: the app VM, the Caddy reverse proxy that terminates TLS (and keeps a 7-day access log with IP addresses, to confirm), and the Forgejo registry that holds the image. Nobody else operates them.
- **Domain and DNS:** the registrar and the DNS provider see no data of users (to confirm once the domain is registered, Q1).

## Later (Phase 4, #60)

- **Apple Push Notification service and Firebase Cloud Messaging:** the iOS and Android apps will receive pushes through them. They will process device push tokens and the notification text: processors with their own DPAs (Apple, Google), with transfers to the USA (EU-US Data Privacy Framework; to confirm then).
- **Codemagic:** builds and signs the apps; it receives source code and signing keys, no user data. Not a processor of users' data (to confirm then).
- **Web Push for the browser app:** would add the browsers' push services (Mozilla, Google, Apple); decided with #60.
