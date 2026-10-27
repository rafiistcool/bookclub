# Plan: from one club to a hosted multi-club service

Decided on 2026-10-27. This file records what was decided and why. The work itself is tracked in Forgejo issues #36–#67, grouped into five milestones.

## Goal

Many people want their own club or a personal reading tracker. Bookclub becomes a **hosted service**: one instance where anyone can sign up, track their reading alone, and create or join any number of private clubs. There will be store apps for iOS and Android.

Self-hosting ends. The only existing instance, on the homelab, is replaced in place, and **every current user keeps all their data**.

## Decisions

### Product and operations

| Topic | Decision | Why |
|---|---|---|
| Model | Hosted service run by Rafael. Self-hosting ends. | People who want a club won't run a Docker container. |
| Scale target | About 10k accounts on one server. No horizontal scaling. | The bottleneck is the data model and catalog, not throughput. |
| Database | Postgres | Many concurrent writers, full-text and trigram search, real migrations. |
| Hosting | A dedicated VM on the homelab, behind the existing reverse proxy on the business line's static IP. Its own VLAN with no LAN access. Proxy rate limits, encrypted offsite backups, external uptime monitoring. A Hetzner VPS in Germany later, if it grows. | It already works for `git.tillhome.de`. VLAN isolation keeps strangers' traffic away from the LAN. |
| Sign-up | Open sign-up with email and password, email verification and reset through a transactional email provider (EU-hosted, with a DPA). No social login. | Offering social login on iOS would also require Sign in with Apple. |
| Account rights | In-app account deletion and a DSGVO data export from day one. | Required by the App Store and the DSGVO. |
| Money | Free, no payments in v1. If costs need covering later: a web-only supporter option through a merchant of record that unlocks no features, after talking to a Steuerberater. | In-app purchase rules, VAT and registering a business make payments expensive to add. |
| Operator identity | An Impressum with Rafael's real name and an address service. Legal texts from a generator or a lawyer. | Required for a public service operated from Germany. Keeps the home address private. |
| Public name | Open. Decided in #50 before the legal texts and store listings. "Bookclub" stays the working name for the code. | |

### Accounts, clubs, friends

- **Accounts are global.** Personal data belongs to the user: shelf, ratings, diary, quotes, favourites, physical shelf, search history. Club data belongs to a club: picks, milestones, pick posts, events, next-up votes, invites. With zero clubs the app is a complete solo reading tracker.
- **Clubs are private and invite-only.** Roles are owner, admin and member. At most 20 members per club, and at most 10 clubs created per user. Invites are single-use or expiring. Ownership must be transferred before the owner leaves. There is no public directory, and non-members get 404 for anything in a club.
- **Friends are mutual** (request and accept). They see each other's reading and can invite each other into clubs. No public profiles and no followers.
- **Visibility:**
  - Friends and members of a shared club see a user's shelf, ratings, currently reading, stats, diary entries and quotes.
  - Diary entries, quotes and ratings can each be set to "only me".
  - The physical shelf layout is visible to the same audience, but only its owner can edit it.
  - Strangers see nothing.
  - Club content is visible only inside the club.
- **UI:** club pages under `/c/<club-slug>/…`, personal pages unscoped, a combined home feed, and a club switcher.

### Catalog

- **Our own catalog in Postgres**, the way BookWyrm and Hardcover work. Every book anyone searches, scans or shelves is copied in permanently. Works have many editions. A shelf entry points to a work and optionally to the edition the user owns. Club picks point to works, and milestones are in percent or chapters, because page counts differ between editions.
- **Sources (CC0 only):**
  - **DNB** for German books. A background harvester reads the OAI-PMH set `dnb-all:reiheN` (publishers' VLB announcements), so new German titles appear about 2 months before publication. Only the CC0 fields are stored; blurbs are VLB's and are not stored. Live DNB SRU lookups are a fallback only, because they rate-limit hard.
  - **Open Library** for English books, and for covers wherever it has one.
  - **Google Books is dropped.** Its terms forbid building databases or permanent copies.
- **Covers:** Open Library covers are fetched by cover ID and served from our own domain. That avoids the ISBN-lookup limit and the DSGVO problem of clients calling a third party. Users can photograph their copy: EXIF is stripped and the image stored as WebP. The first upload becomes the edition's cover, and replacements go to the admin queue. Every cover can be reported. Showing covers rests on tolerated practice, not the quotation right, so notice-and-takedown is mandatory.
- **Missing or wrong data:** users add missing books and editions, for example from a failed scan. Edits to existing records go to an admin queue that Rafael reviews. Librarian roles come later.
- **Free first.** Paid feeds are considered only if feedback demands them. The research notes are on #64: VLB only licenses bookshops, ISBNdb data must be deleted when you cancel, Amazon needs affiliate sales, and Nielsen and Bowker are sales-call only.

### Technology

- **Monorepo:** `server/` (Rust), `frontend/` (Vue, plus Capacitor later), and `backend/` (Python), which is deleted at cutover and preserved by a tag and a `legacy-python` branch.
- **Server:** axum, sqlx with compile-time-checked queries, Postgres, numbered sqlx migrations, and utoipa generating the OpenAPI spec that the frontend's TypeScript types are generated from. Cookie sessions for the web, bearer tokens for the apps.
  - Why Rust: Rafael knows it better than Python or Java.
  - The Python test suite is the parity checklist (#49).
- **Mobile:** Capacitor wraps the Vue app, so there's one UI codebase. Native features (ML Kit scanning, APNs/FCM push, an offline shelf) make it more than a website, which App Store guideline 4.2 requires.
  - **iOS first.** There's no Mac, so builds run on Codemagic and ship through TestFlight. A used M1 Mac mini comes once native debugging is needed.
  - Android follows, built on the Forgejo runner.

### Migration

- The homelab instance is the only one, and it is replaced in place.
- A one-off command-line importer (#38) moves the SQLite data into Postgres:
  - It shows a dry run with per-entity counts first.
  - It's tested on a copy of the production database that is never committed.
  - The instance becomes one club.
- **Logins carry over unchanged:** users keep their username and password, because the argon2 hashes are copied as they are.
- Users are asked to add an email after logging in.
- Google-sourced books are re-matched by ISBN, and keep only the bare facts when there's no match.
- Push subscriptions are dropped, because they're tied to the old origin.
- The old domain redirects to the new one.

## Phases

| Milestone | Issues |
|---|---|
| **Phase 1: Rust backend** | #36 scaffold, #37 schema, #38 legacy importer, #39 auth, #40 clubs, #41 catalog core, #42 DNB harvester, #43 media store, #44 personal features, #45 club features, #46 visibility, #47 OpenAPI types, #48 frontend, #49 parity checklist |
| **Phase 2: Beta and launch** | #50 name and domain, #51 deployment, #52 cutover, #53 legal texts, #54 admin queue, #55 open sign-up |
| **Phase 3: Scanning and friends** | #56 web barcode scanning, #57 friends |
| **Phase 4: Apps (iOS first)** | #58 Capacitor, #59 iOS pipeline, #60 native features, #61 App Store, #62 Android |
| **Phase 5: Later** | #63 dump backfill, #64 paid feeds, #65 VPS and S3, #66 supporter payments, #67 librarians |

Inside Phase 1, the schema and importer come early, right after the scaffold, so the "nobody loses data" requirement is proven against real data before any features are ported.
