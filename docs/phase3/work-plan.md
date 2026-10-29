# Phase 3 work plan: scanning, friends, and the private move

This plan covers Phase 3 (#56, #57) and the move of the current users onto the Rust server (#52, the parts of #51 the move needs, and CUT-R). The contracts are the same as in Phases 1 and 2:

- `docs/plan.md`: the product decisions
- `docs/phase1/conventions.md`: architecture and coding rules
- `docs/phase1/endpoint-map.md`: routes, owners, DTOs and error strings
- `server/migrations/*.sql`: the schema

Each package writes its contract changes into its own notes file, `docs/phase3/notes/<pkg>.md`. After the merge, the integrator moves them into `conventions.md`, `endpoint-map.md`, `CLAUDE.md`, `README.md` and `docs/plan.md`, and keeps the notes as the record.

In this plan, "I" means the implementing agent: it has the repo and a shell on the `agents` VM. **[Rafael]** marks what only the owner can decide or do.

---

## 1. Status and the new order

**Where things stand (2026-10-29).**
- Phase 1 is merged. Phase 2 Waves 1 and 2 are merged into `main` (PR #73): the admin queue, the image, compose, releases, backups and automatic updates, the legal machinery (pages, terms, retention), suspension, and the import tooling. The import rehearsal on a copy of the production database passed (CUT2, #38 closed).
- Not done from Phase 2: BRAND, the cutover (#52), CUT-R, and everything §5 of `docs/phase2/work-plan.md` lists for the VM, the domain, mail, legal texts and open sign-up.
- COVER (#74, covers for every catalog and imported book) is being built on `phase2/cover` (worktree `../bookclub-cover`, migrations 2300–2399, Postgres 5458, browser 8087). It merges into `phase-2`.
- Production still runs the Python app (v0.10.0, SQLite) with 2 accounts.

**The new order.** Rafael, on 2026-10-29: "das ist mir aktuell schon viel zu professionell. lass uns erstmal die anwendung bauen und dann meine aktuellen nutzer weiternutzen lassen und dann machen wir das rechtliche" ("This is already far too professional for me right now. Let's build the application first, then let my current users carry on using it, and then we do the legal part.").

1. **COVER (#74)**, already running. It merges into `phase-2`.
2. **Phase 3, Wave 1:** SCAN (#56, barcode scanning on the web), FRIENDS (#57) and MOVE's code (M1), in parallel.
3. **Phase 3, Wave 2: the slim private move (MOVE).** The 2 current accounts move from the Python app to the Rust server: invite-only, the name stays "Bookclub", on the existing homelab host, behind the existing reverse proxy, under the existing hostname. No new external accounts, no legal texts, no paperwork. Only what the server needs to run safely. This is #52, the parts of #51 the move needs, and CUT-R.
4. **Later, before any stranger can sign up:** the public launch (#50 name and domain, #53 legal texts and DPAs, #55 open sign-up, the mail provider, offsite backups, the VLAN, monitoring, the GATE checks). It comes before Phase 4, because the App Store needs a privacy policy. §9 lists it.

**Why this order is safe.** The move changes the software, not the exposure: the same host, the same address, the same proxy and the same two people as the Python app today. Sign-up stays invite-only, and only Rafael mints invites. The server refuses `SIGNUP_MODE=open` without the six legal files (LEG), and after MOVE also without mail (§5.2), so nobody opens sign-up by accident. With mail off, local backups and no external monitor, no processor handles anyone's data, so there is no DPA to sign. Open Library and the DNB get catalog queries without any user identity, as `docs/legal/processors.md` records.

### For Rafael

- Your to-dos are in §8, the three questions in §7. Wave 1 needs nothing from you.
- What you and the other member will notice after the move (§6, answer 47):
  - Everyone signs in again once, with the old username and password.
  - "Forgot password" says a link is on its way, but no mail leaves yet. They ask you, and you run one command from the runbook (§5.2).
  - Only club admins invite new people: you, or whoever you make an admin on the club's members page. In the old app any member could.
  - Nobody can create a new club until mail exists: a new club needs a confirmed address.
  - New: scanning a barcode, and friends.

---

## 2. Packages

```
COVER #74 (phase2/cover, running) ─► phase-2 ─► merged into phase-3
Wave 1  SCAN #56 ─ FRIENDS #57 ─ MOVE code (M1)      parallel
          │  (all three merged into phase-3)
Wave 2  MOVE: PR phase-3 → main ─► v0.11.0 ─► host ─► move day ─► phone test ─► CUT-R
          │  (#52, the parts of #51 it needs)
Later   the public launch: BRAND #50, #53, #55, the rest of #51 (§9)
          │
Phase 4 apps (#58–#62)
```

### 2.1 Ground rules: what differs from Phase 2

Phase 2's ground rules (`docs/phase2/work-plan.md` §2.1) and its amendments 1–5 and 27 (§0) apply. These are the differences:

1. **Branches.**
   - The integration branch is `phase-3`, cut from `phase-2` at 7793faf (this plan's parent).
   - When COVER has merged into `phase-2`, the integrator merges `phase-2` into `phase-3`.
   - Worktrees: `git worktree add ../bookclub-<pkg> -b phase3/<pkg> phase-3`.
   - **SCAN and MOVE branch only after COVER is in `phase-3`**: SCAN reuses COVER's `label` prop on `CoverPhotoButton.vue` and must not fork COVER's SPA files; MOVE writes its runbook against `cutover.md` as COVER leaves it (§5.6).
   - FRIENDS may branch before that. Once COVER is in `phase-3`, it merges `phase-3` into its branch before its final checks. Its overlaps with COVER are add-only or regenerated: `parity_schema.rs` (`GLOBAL`), `parity-checklist.md` #37, `client.ts`, `apiErrors.ts`, `en.json`/`de.json`, `generated.ts`, `server/.sqlx/`.
   - Packages merge locally into `phase-3`, one at a time (Phase 2 amendment 4).
   - **One Forgejo PR `phase-3` → `main`**, after Wave 1 and M1's boot test on the merged `phase-3` (§2.3), with the `forgejo` MCP tools. **[Rafael]** approves the merge.
2. **Ownership.** Every file belongs to one package (lists below). Shared files:

   | File | Edited by | Rule |
   |---|---|---|
   | `frontend/src/router.ts`, `router.test.ts` | SCAN (the `/scan` route), FRIENDS (the `/friends` page replaces the redirect at `:118`, and `router.test.ts:56`) | Separate hunks |
   | `frontend/src/api/client.ts`, `frontend/src/types.ts`, `frontend/src/i18n/apiErrors.ts` | SCAN, FRIENDS | Add lines only |
   | `frontend/src/test/fixtures.ts` | SCAN (an `isbnHit()` fixture), FRIENDS (`invitee: null` and `personal: false` in `invite()`, which `typecheck` needs once `InviteOut` gains them) | Add lines only |
   | `frontend/src/i18n/{en,de}.json` | SCAN (`scan.*`, new keys under `addBook` and `addEdition`), FRIENDS (`friends.*`, `nav.friends`, new `errors.*`, the copy of `errors.noMember`) | The integrator resolves textual conflicts; `messages.test.ts` checks key parity |
   | `server/src/lib.rs` | SCAN (`loggable_path` and its unit test, `:262-321`), FRIENDS (`pub mod friends;` and the `friends::routes()` merge line, `:122-135`), MOVE (the `EmailTransport` arm, `:94-97`) | Separate hunks |
   | `server/src/error.rs` (`msg` constants) | FRIENDS | Add lines only |
   | `frontend/src/api/generated.ts`, `server/.sqlx/` | each package for its own checks (Phase 2 amendment 1) | The integrator regenerates them after a merge when they conflict |
   | `docs/phase1/parity-checklist.md` | FRIENDS (the #37 and #46 tables, which tests read) | Its rows only |
   | `docs/legal/vvt.md` | FRIENDS (two table rows and a stub heading; `legal_docs.rs` requires the rows, §4.7) | Its lines only |
   | `deploy/hosted/README.md` | SCAN (the `Permissions-Policy` and log lines, `:278`, `:285`), FRIENDS (one query in step 7 of the restore section, §4.7), MOVE (mail off, `set-password`, a pointer to the private-move runbook) | Separate hunks |
   | `docs/phase1/{conventions,endpoint-map}.md`, `CLAUDE.md`, `README.md`, `docs/plan.md` | integrator | Applied from the package notes |

3. **Dependencies.** No new crate. One new npm package: `zxing-wasm`, exact version `3.1.6` (SCAN, §3.3), added with `npm install --save-exact`. The integrator adds nothing else.
4. **Migrations.**
   - Reserved blocks: SCAN 3000–3099 (it needs none), FRIENDS 3100–3199, MOVE 3200–3299 (it needs none), the integrator 3900–3999.
   - Phase 2 used 2100–2299, COVER 2300–2399, the integrator 2900.
   - A merged migration is never edited. No squash.
5. **New paths.**

   | Path | Package |
   |---|---|
   | SPA `/scan` | SCAN |
   | `GET /api/friends`, `POST /api/friends/requests`, `POST /api/friends/requests/{username}/accept`, `DELETE /api/friends/requests/{username}`, `DELETE /api/friends/list/{username}`, `PUT` and `DELETE /api/friends/blocks/{username}`, `DELETE /api/friends/club-invites/{invite_id}`; `kind: "friend"` on `POST /api/clubs/{slug}/invites`; SPA `/friends` | FRIENDS |
   | CLI `bookclub-server set-password <username>`; `EMAIL_TRANSPORT=none`; the file `/sw.js` | MOVE |

6. **Worktrees, databases and ports.**

   | Package | Worktree, branch | Postgres | Browser |
   |---|---|---|---|
   | SCAN | `../bookclub-scan`, `phase3/scan` | `bookclub-pg-scan` on 5461 | 8091 (as `localhost:8091` for the fake-camera run, §3.9) |
   | FRIENDS | `../bookclub-friends`, `phase3/friends` | `bookclub-pg-friends` on 5462 | 8094 |
   | MOVE | `../bookclub-move`, `phase3/move` | `bookclub-pg-move` on 5463 | 8095 (the image boot test) |
   | integrator | `phase-3` in the main checkout | the compose server on 5432 | 8096 (the browser run after Wave 1) |

   - COVER holds 5458 and 8087. Phase 2 used 5451–5458 and 8080–8087. 8092 and 8093 stay free.
   - Ports above 9999 work only on 127.0.0.1 (the VM's firewall).
   - The rest of Phase 2's rule 6 applies: one Postgres per worktree, the untracked `.cargo/config.toml` copied in with its own `DATABASE_URL`, at most 5 builds at once, `docker rm -f bookclub-pg-<pkg>` after the merge.
7. **Done** is Phase 2's rule 7. Extras: SCAN runs `caddy validate` (CLAUDE.md, "Deployment files"); MOVE runs shellcheck and `docker compose … config -q`.
8. **Integration** is Phase 2's rule 8. After Wave 1, the integrator also runs one browser pass on 8096 over the merged `phase-3` with a `securitypolicyviolation` listener: Discover, `/scan` with Chromium's fake camera (§3.9), `/friends`, a club's members page. Zero CSP violations is the bar.
9. **Real data** stays out of the repo, Forgejo and committed logs (Phase 2 rule 9). MOVE handles production data on the host only.
10. **`backend/`** stays untouched until CUT-R merges, as before.
11. **No public-launch work in Phase 3.** The name stays "Bookclub". Nothing touches DNS, a mail provider, the legal texts, the Storage Box or the VLAN. The only legal file a package edits is `docs/legal/vvt.md`, because `server/tests/legal_docs.rs::every_table_with_personal_data_is_in_the_record` requires every table that references `users` to be named there (FRIENDS). Facts for the later privacy text stay in the package notes for #53.
12. **Before Wave 1 branches,** the integrator commits one change on `phase-3`, so the legal documents' citations cannot drift silently (the Phase 2 rule behind `EDITED_ELSEWHERE`):
    - `server/tests/legal_docs.rs`: a Phase 3 block in `EDITED_ELSEWHERE` with `server/src/ratelimit.rs`, `server/src/auth/export.rs`, `server/src/auth/mail.rs`, `server/src/auth/reset.rs`, `server/src/config.rs`, `server/src/catalog/isbn_lookup.rs`, `server/src/clubs/`, `server/src/activity/feed.rs`, `server/src/visibility.rs`, `server/src/error.rs`, and the array's length.
    - The line citations into those files become names: `vvt.md:23` and `privacy-facts.md:87` cite `` `server/src/ratelimit.rs` (`RateLimits`, `ip_key`, `unknown_identifier_key`) ``; `privacy-facts.md:139` cites `` `server/src/auth/export.rs` (`export_me`) ``; `privacy-facts.md:53,118` cite `` `server/src/auth/mail.rs` (`issue`) ``; `vvt.md:109` cites `` `server/src/clubs/mod.rs` (`MAX_MEMBERS`) ``.
    - `cargo test --test legal_docs` passes.

### 2.2 Wave 1

SCAN, FRIENDS and MOVE's code (M1, §2.3) need no answer from Rafael. The suggested merge order is whichever finishes first. The phone test of the scanner runs after the move, on the real hostname (§5.8); SCAN merges on the Chromium run (§3.9).

#### SCAN: barcode scanning on the web (#56)

- **Size:** M. **Dependencies:** COVER in `phase-3`.
- **Design:** §3.
- **Scope:**
  - Server: the CSP gains `'wasm-unsafe-eval'`; `GET /api/books/isbn/{isbn}` takes the per-account catalog turn; ISBN paths are redacted in the app log.
  - Deploy: `Permissions-Policy` allows the camera on our own origin; the Caddy log filter redacts ISBN paths.
  - SPA: ISBN validation, the decoder, the camera, `BarcodeScanner.vue`, the `/scan` page, its entry point in Discover, ISBN prefill in the add forms.
- **Owns:**
  - Server: `server/src/spa.rs`, `server/src/catalog/isbn_lookup.rs`, `server/tests/platform.rs`, and the `catalog_requests_are_limited_per_account` test in `server/tests/catalog_search.rs` (`:1117`).
  - Deploy: `deploy/hosted/Caddyfile.example`.
  - SPA, new: `frontend/src/isbn.ts`, `frontend/src/scan/{decoder,zxing,camera}.ts`, `frontend/src/components/BarcodeScanner.vue`, `frontend/src/pages/ScanPage.vue`, and a test next to each (`isbn.test.ts`, `scan/decoder.test.ts`, `scan/zxing.test.ts`, `scan/camera.test.ts`, `BarcodeScanner.test.ts`, `ScanPage.test.ts`); `frontend/src/components/AddEditionForm.test.ts` (it has none yet).
  - SPA, changed: `frontend/src/pages/DiscoverPage.vue`, `frontend/src/components/{AddBookSheet,AddEditionForm}.vue` with their tests, `frontend/package.json`, `frontend/package-lock.json`, and `frontend/vite.config.ts` only if the `?url` import needs it (§3.3).
- **Shared:** `lib.rs` (`loggable_path`), `router.ts`, `client.ts`, `types.ts`, `apiErrors.ts`, `fixtures.ts`, `en.json`/`de.json`, `deploy/hosted/README.md` (`:278`, `:285`).
- **Tests it adds:** §3.9.
- **Notes:** `docs/phase3/notes/scan.md`, with: the CSP and `Permissions-Policy` texts for `conventions.md:195,214`, `endpoint-map.md:135,222` and CLAUDE.md's "No third party, no inline script" rule; the `/isbn` limit for `conventions.md` §10.1 (`:1285`); for #53 only, the facts for the privacy text (ISBN paths are redacted in the logs; camera frames are decoded in the browser and never leave it, only the ISBN goes to the server); the bundle sizes (main entry before and after, the wasm asset); later ideas ("Scan your copy" in the edition picker).
- **Acceptance:**
  - `cargo test --test platform --test catalog_search --test catalog_isbn` and the `lib.rs` unit tests pass.
  - `npx vitest run src/isbn.test.ts src/scan src/components/BarcodeScanner.test.ts src/pages/ScanPage.test.ts src/pages/DiscoverPage.test.ts src/components/AddBookSheet.test.ts src/components/AddEditionForm.test.ts src/i18n/messages.test.ts src/thirdParty.test.ts` passes.
  - `caddy validate` passes on `Caddyfile.example`.
  - `npm run build`: the main entry chunk does not contain `zxing`, and `dist/assets/` holds one `zxing_reader-<hash>.wasm`.
  - The Chromium fake-camera run (§3.9) scans a known ISBN to the hit card and a made-up valid ISBN to the miss flow, with no CSP violation.
  - The phones are tested after the move (§5.8); #56 closes then.

#### FRIENDS: friends (#57)

- **Size:** L. **Dependencies:** none to start; COVER in `phase-3` before its final checks (§2.1 rule 1).
- **One package, not split into server and SPA.** The SPA's types come from `generated.ts`, which comes from the server's OpenAPI document, so a separate SPA package would wait on the server anyway. ADM did both halves the same way in Phase 2.
- **Design:** §4.
- **Scope:** the tables and the visibility functions, the friend and block routes, friend invites into clubs, every account-reference rule the new foreign keys trigger, the visibility matrix's friend viewer, and the SPA (`/friends`, the badge, the Home row, bound invites on the club's members page).
- **Owns:**
  - Server, new: `server/migrations/3100_friends.sql`, `server/src/friends/{mod,requests,blocks,out}.rs`.
  - Server, changed: `server/src/visibility.rs` (doc comments `:18,40`), `server/src/activity/feed.rs`, `server/src/clubs/{invites,members,out}.rs`, `server/src/auth/{mod,deletion,export}.rs`, `server/src/admin/users.rs`, `server/src/ratelimit.rs`; the utoipa doc-comment hunks that say "they share a club" in `server/src/media/reports.rs` (`:2`, `:295`) and `server/src/posts/out.rs` (`:147`).
  - Server tests, new: `server/tests/{friends_requests,friends_blocks,friends_invites,friends_visibility}.rs`.
  - Server tests, changed: `parity_visibility.rs`, `visibility_policy.rs`, `club_visibility.rs`, `clubs_invites.rs` (only where `InviteOut` gains `invitee` and `personal`), `auth_export.rs`, `jobs.rs`, `auth_delete.rs`, `parity_schema.rs`, `common/visibility.rs` (an amendment to a W1-frozen helper, recorded in the notes), and any existing test that the new viewers in `common/visibility.rs` break.
  - SPA, new: `frontend/src/pages/FriendsPage.vue`, `frontend/src/stores/friends.ts`, `frontend/src/components/InviteFriendSheet.vue`, each with a test.
  - SPA, changed: `frontend/src/nav.ts` (+ `nav.test.ts`), `frontend/src/components/NavIcon.vue`, `frontend/src/layouts/AppShell.vue`, `frontend/src/pages/HomePage.vue`, `frontend/src/pages/club/ClubMembersPage.vue`, with their tests; and the two tests that mount the real `AppShell`, which now asks `GET /api/friends`: `frontend/src/acceptance.test.ts` (a handler in each fake server, or `server.unknown` is not empty) and `frontend/src/pages/LegalPage.test.ts` (a `friends` mock).
  - Deploy: one line in the header comment of `deploy/hosted/revoke-access.sql` and one query in step 7 of the README's restore section: accepted `friendships` rows are reviewed by hand after a compromise, like memberships (a planted one grants the same view).
  - Docs: its rows in `docs/phase1/parity-checklist.md` and `docs/legal/vvt.md`.
- **Shared:** `lib.rs`, `error.rs`, `router.ts`, `client.ts`, `types.ts`, `apiErrors.ts`, `fixtures.ts`, `en.json`/`de.json`, `deploy/hosted/README.md`.
- **Tests it adds:** §4.11.
- **Notes:** `docs/phase3/notes/friends.md`, with the texts for `conventions.md` §5.6 (the cascade, `lock_clubs`), §5.7 (the new export keys, the exempt column, 34 foreign keys, TRACES as "a trace of signing in or another account's act", "club mates and friends"), §6.1, §6.2 (`bc_friends` and the new bodies), §6.4 (the home feed), §6.5 (the friend viewer), §10.1 (the new limit); a Friends section for `endpoint-map.md` with §3 (`kind: "friend"`, `InviteOut.invitee` and `personal`) and §11 (the new strings); the CLAUDE.md wording for TRACES; `docs/plan.md:31`, which becomes "…can invite each other into clubs they administer"; later ideas (friend actions on a member's page, inviting from a club's members page, a "Friends are reading" strip on Home).
- **Acceptance:**
  - `cargo test --test friends_requests --test friends_blocks --test friends_invites --test friends_visibility --test parity_visibility --test parity_checklist --test visibility_policy --test club_visibility --test auth_export --test jobs --test auth_delete --test auth_sessions --test parity_schema --test legal_docs --test api_error_copy --test openapi_contract` pass, and then the full suite.
  - The `EXPLAIN` test (§4.4) shows the policy functions still inline.
  - `npx vitest run src/pages/FriendsPage.test.ts src/stores/friends.test.ts src/components/InviteFriendSheet.test.ts src/nav.test.ts src/router.test.ts src/pages/HomePage.test.ts src/pages/club/ClubMembersPage.test.ts src/acceptance.test.ts src/pages/LegalPage.test.ts src/i18n/messages.test.ts` passes.
  - A browser run on 8094 with two accounts in two browser contexts: request, accept, see each other's shelf; the admin invites the friend into a club from `/friends`; the friend sees the invitation on `/friends`, joins through `/join/<code>`, and appears on the club's members page; block, then the request from the blocked account does not show.

### 2.3 MOVE, the slim private move (#52, the parts of #51 it needs, CUT-R)

M1, the code, is a Wave 1 package. M2–M6 are Wave 2: operations, no new code.

- **Size:** M (code S). **Dependencies:** M1: COVER in `phase-3`. M2: Wave 1 (SCAN, FRIENDS, M1) merged into `phase-3`.
- **Design:** §5.
- **Steps:**
  - **M1, code** (worktree `../bookclub-move`): mail off (`EMAIL_TRANSPORT=none`), `set-password`, the old service worker's kill switch at `/sw.js`, `deploy/cutover/host-facts.sh`, and the runbook `docs/phase3/private-move.md`. Merged into `phase-3`.
  - **M2, release:** the M1 boot test once more on the merged `phase-3`; the PR `phase-3` → `main` (**[Rafael]** approves the merge); then the tag `v0.11.0` on `main`, with no second approval (nothing follows `stable` until the update timer is on, §5.8). `bookclub-server` must then hold `0.11.0`, `0.11` and `stable` with one digest (`docs/phase2/notes/dep2.md`, "Not verified").
  - **M3, the host:** facts, the stack installed next to the Python app, backups and one restore test (§5.4–§5.5). The read-only facts may run as soon as M1 is merged.
  - **M4, move day:** the runbook (§5.6), ending with the phone test (§5.8).
  - **M5, afterwards:** the harvest on, the update timer on, the import copy deleted, CUT-R merged (**[Rafael]** approves), the issues closed, my SSH key removed from the host (§5.8).
  - **M6, after 30 days:** the old container and its data deleted, once the backups are proven (§5.8).
- **Owns (M1):**
  - Server: `server/src/config.rs`, `server/src/email/mod.rs`, `server/src/auth/mail.rs`, `server/src/main.rs`; the `set-password` logic as `pub async fn set_password` in `server/src/auth/reset.rs`, sharing the confirm step's transaction body; `server/tests/auth_set_password.rs`.
  - SPA: `frontend/public/sw.js`, `frontend/src/swKillSwitch.test.ts`.
  - Deploy: `deploy/hosted/env.example`, `deploy/cutover/host-facts.sh`.
  - Docs: `docs/phase3/private-move.md`, a two-line pointer at the top of `docs/phase2/cutover.md`.
- **Shared:** `lib.rs` (the transport arm), `deploy/hosted/README.md`.
- **Owns (CUT-R):** the branch `cutover/remove-python` (§5.8).
- **Notes:** `docs/phase3/notes/move.md`: the `conventions.md` §9, §11 and §16 texts for `none` and `set-password`, `endpoint-map.md` §10, the facts found on the host (no secrets, no usernames), the move's counts against the CUT2 baseline, the phone test's result.
- **Acceptance:**
  - M1: `cargo test --test auth_set_password --test deploy_env` and the `config.rs` unit tests pass. `npx vitest run src/swKillSwitch.test.ts src/thirdParty.test.ts` passes. shellcheck is clean on `host-facts.sh`.
  - M1 boot test on 8095, with names that are neither production's nor CI's:
    - the image: `docker build --build-arg CARGO_JOBS=3 -t git.tillhome.de/rafiistcool/bookclub-server:move-test .`;
    - an env file in the scratchpad with `COMPOSE_PROJECT_NAME=bookclub-move`, `BOOKCLUB_TAG=move-test`, `BOOKCLUB_ENV_FILE=<that file>`, `BOOKCLUB_DATA_DIR=<scratchpad>/move-data` (its `media` and `legal` made first with `install -d`; the VM user is uid 1000, so the app can write them), `BOOKCLUB_LISTEN=127.0.0.1`, `BOOKCLUB_PORT=8095`, `EMAIL_TRANSPORT=none`, no `SMTP_URL` or `EMAIL_FROM`, `LEGAL_DIR=/app/legal` (empty), and the other required values as in `ci.env`;
    - `docker compose --env-file <file> -f deploy/hosted/compose.yaml up -d --wait --pull never` exits 0, and `/api/health` answers `{"status":"ok"}`;
    - `send-test-mail x@example.com` exits 1 with `mail is off (EMAIL_TRANSPORT=none)`;
    - `beta-invite`, then `POST /api/auth/register` with that code answers 201 although no mail leaves; `set-password <that name>` prints a password that signs in;
    - `curl -sI …/sw.js` shows a JavaScript content type;
    - `SIGNUP_MODE=open` in the env file stops the app at startup with the mail message;
    - then `down -v`, `docker image rm` of the `move-test` tag, the data directory removed, and `docker builder prune -f`.
  - M4: the counts match (`compare.sh` exit 0), Rafael signs in with his old password and sees his data, `verify-limits.sh` gets its 429s while Rafael's phone signs in, the phone test is recorded, the other member signs in.
  - M5: CUT-R's acceptance (`docs/phase2/work-plan.md` §2.4) and Forgejo CI green; my key gone from root's `authorized_keys`.

### 2.4 Dependencies and concurrency

| Package | Hard dependencies | Runs alongside | Notes |
|---|---|---|---|
| COVER | Phase 2 Wave 2 | before Wave 1 | Phase 2 package; blocks SCAN and M1 |
| SCAN | COVER in `phase-3` | FRIENDS, M1 | Phones are tested after the move |
| FRIENDS | none (COVER before its final checks) | SCAN, M1 | The biggest package |
| MOVE M1 | COVER in `phase-3` | SCAN, FRIENDS | Small; no file overlap beyond separate hunks |
| MOVE M2–M6 | Wave 1 merged | alone | Need the host; Rafael approves the merge and CUT-R |

---

## 3. Design: barcode scanning (#56)

### 3.1 The flow

1. **Entry point.** Discover: a camera button in the search bar ("Scan a barcode") opens `/scan`. It is always shown; manual entry is the fallback on every device. No bottom-bar item, and no second entry in v1: the hit card already sets a shelved book's edition (§3.2).
2. **Capture.** `BarcodeScanner` starts the camera at once (the tap that opened it is the user gesture), or shows why it cannot (§3.4). Manual entry is always visible below the video.
3. **First valid code.** The first decoded text that passes `isbnFromScan` (EAN-13, prefix 978 or 979, check digit) stops the camera and emits the ISBN-13. Nothing else is looked up: one read, one lookup.
4. **Lookup.** `ScanPage` calls `GET /api/books/isbn/{isbn}` with an `AbortController`. It shows the ISBN, and after 2 s the elapsed seconds (the book page's load-clock pattern), with "Cancel" back to capture. A German lookup can take up to about 30 s.
5. **Answer:**
   - **200 `IsbnHitOut` (hit):** the result card (§3.2).
   - **404 `No book found for that ISBN` (miss):** the miss flow (§3.5).
   - **429** (busy, or the account's catalog turn) and **502** `Could not look that ISBN up right now. Try again.`: the mapped message and "Try again", which repeats the lookup with the same ISBN.
   - **400** cannot come from the scanner; manual entry validates first.

### 3.2 The hit card

- Cover (or the placeholder), title, authors, year and pages from `IsbnHitOut`.
- **Not on the shelf:** the four status buttons of `STATUSES` (`frontend/src/constants.ts:5`), each `POST /api/shelf {work_id, edition_id, status}`. A 409 with `err.item` (the book was shelved elsewhere meanwhile) becomes `PATCH /api/shelf/{id} {edition_id, status, position: 0}`, as `BookDetailPage.vue` `setStatus` does.
- **On the shelf (`on_shelf`, `shelf_id`):** "On your shelf", and "Use this edition", which sends `PATCH /api/shelf/{shelf_id} {edition_id}`. It is idempotent, so no DTO needs to say which edition the entry has now.
- **No cover (`cover_url` null):** "Add a photo of the cover", `CoverPhotoButton` with `workId`, `editionId` and COVER's `label` prop. A 200 `CoverSet` updates the card; a 202 says an admin decides. COVER's `CoverPrompt` on the book page stays the place for the big prompt.
- Always: "Open book" (the book page) and "Scan another" (back to capture).
- Nothing is added to the shelf without a tap. A hit already put the work and edition into the catalog (the lookup persists every remote hit, `service::persist`).

### 3.3 ISBN rules and the decoder

**`frontend/src/isbn.ts`** mirrors `server/src/catalog/shared/isbn.rs`:
- `normalizeIsbnInput(raw)`: NFKC (full-width digits become ASCII), spaces and `-` removed, `x` uppercased.
- `parseIsbn(raw): string | null`: for manual entry. ISBN-10 (check digit, `X` allowed last) or ISBN-13 (check digit), returned as the ISBN-13; only 978 and 979. The same rules as `parse_isbn` (`isbn.rs:75`).
- `isbnFromScan(text): string | null`: for decoded barcodes. Exactly 13 digits, 978 or 979, check digit. No ISBN-10: barcodes are EAN-13.
- `9790…` (ISMN, sheet music) passes, as on the server. A lookup simply misses.
- `extractIsbn` in `constants.ts` stays as it is (Discover's shape-only query rule).

**Decoder** (`frontend/src/scan/decoder.ts`): `createDecoder(): Promise<Decoder>`, where `Decoder` is `{kind: "native" | "zxing", detect(canvas: HTMLCanvasElement): Promise<string[]>}`.
- **Native** when `globalThis.BarcodeDetector` exists and `await BarcodeDetector.getSupportedFormats()` includes `"ean_13"`: `new BarcodeDetector({formats: ["ean_13"]})`, `detect(canvas)` returns the `rawValue`s. That is Android Chrome with Play Services. `getSupportedFormats()` can be `[]`; then zxing.
- **Otherwise zxing** (iOS Safari 16+, desktop browsers): `await import("./zxing")`, so neither the glue nor the wasm is in the main bundle.
- The `BarcodeDetector` types are declared locally in `decoder.ts`, not globally (the DOM lib has none).

**`frontend/src/scan/zxing.ts`:**
- Imports `prepareZXingModule` and `readBarcodes` from `zxing-wasm/reader`, and `wasmUrl from "zxing-wasm/reader/zxing_reader.wasm?url"`. Vite emits the file as `/assets/zxing_reader-<hash>.wasm`. `spa.rs` serves it as `application/wasm` (mime_guess), immutable and same-origin.
- **Always** calls `prepareZXingModule({overrides: {locateFile: (path, prefix) => (path.endsWith(".wasm") ? wasmUrl : prefix + path)}, fireImmediately: true})` before the first read. Without it, the package fetches the wasm from `fastly.jsdelivr.net`. The CSP blocks that, and the scanner would fail with nothing in `src/` to catch it.
- `readBarcodes(imageData, {formats: ["EAN13"], tryHarder: true, maxNumberOfSymbols: 1})`; it keeps the `text` of results whose `isValid` is true.
- Version: `zxing-wasm` exactly `3.1.6` (MIT; the wasm bundles zxing-cpp, Apache-2.0). Before installing, `npm view zxing-wasm@3.1.6 scripts` must show no install script. If dev prebundling breaks the `?url` import, add `optimizeDeps.exclude: ["zxing-wasm"]` to `vite.config.ts` (SCAN's hunk).

**Decode loop** (in `BarcodeScanner.vue`): on the main thread, one decode every 200 ms (`setTimeout`, never overlapping). Each pass draws the centre band of the video (full width, the middle 40 % of the height, scaled to at most 1280 px wide) into a `<canvas>` whose 2D context has `willReadFrequently: true`. No worker: only if a phone shows jank, recorded in the notes. No `requestVideoFrameCallback` (Safari 15.4+) and no `OffscreenCanvas` (16.4+).

### 3.4 Camera and permissions

**`frontend/src/scan/camera.ts`:**
- `cameraSupport(): "ok" | "insecure" | "unsupported"`: `insecure` when `window.isSecureContext` is false (on `http://agents.lan:<port>`, `navigator.mediaDevices` is `undefined` on phones); `unsupported` when `navigator.mediaDevices?.getUserMedia` is missing.
- `openCamera(video): Promise<MediaStream>`: `getUserMedia({audio: false, video: {facingMode: {ideal: "environment"}, width: {ideal: 1280}, height: {ideal: 720}}})`, attached to a `<video playsinline muted autoplay>`.
- `stopCamera(stream)` stops every track.
- `cameraError(err): "denied" | "nocamera" | "busy" | "failed"`: `NotAllowedError` and `SecurityError` are `denied`; `NotFoundError` and `OverconstrainedError` are `nocamera`; `NotReadableError` and `AbortError` are `busy`; anything else is `failed`.

**`BarcodeScanner.vue`** (emits `isbn: [isbn13: string]` and `cancel: []`):

| State | What it shows |
|---|---|
| `insecure` | "The camera needs a secure connection." Manual entry, focused. |
| `unsupported`, `nocamera` | "No camera here." Manual entry. |
| `denied` | Where to allow the camera (iPhone: Settings → Safari → Camera, or "aA" → Website Settings; Android Chrome: the icon left of the address → Permissions → Camera), "Try again", and manual entry. |
| `busy` | "The camera is in use by another app." "Try again". |
| `live` | The video, a guide band, "Hold the barcode 15–20 cm away", manual entry. |

- **Manual entry:** a text field (`autocomplete="off"`, `enterkeyhint="search"`; not `inputmode="numeric"`, which hides `X` on iOS). `parseIsbn` validates it. An invalid value shows the existing `invalidIsbn` copy; a valid one emits the ISBN-13.
- **The camera stops** on a result, on cancel, on unmount, and on `visibilitychange` to hidden (iOS ends the stream in the background). Coming back visible in `live` restarts it. Only one stream is ever open. It is always stopped before a file input opens, or the iOS photo camera can come up black.
- **No torch or zoom** in v1 (Android only). No `capture` attribute on the photo input, so the photo library stays available.
- **iOS 16 is the floor** for the zxing path (`'wasm-unsafe-eval'`); older devices fail in `createDecoder` and fall back to manual entry with "This browser cannot read barcodes."

### 3.5 Miss flow, adding, and the photo

The miss card says the ISBN is not in the catalog and offers, in this order:
1. **"Find the book by title"** (avoids a duplicate work when another edition is already known): a search field using `GET /api/books/search`, a list of works, and on a choice `AddEditionForm` with `workId` and `initialIsbn`.
2. **"Add it as a new book"**: `AddBookSheet` with `initialIsbn` and `stay`.

After either save, `ScanPage` calls `GET /api/books/isbn/{isbn}` again. It is now a local hit (no outbound call). The result is the hit card of §3.2, with the photo button, because a new edition has no cover. A reader's own book (`source='user'`) is never cover-searched (COVER), so this photo is its only way to a cover.

**Component changes:**
- `AddEditionForm.vue`: an optional `initialIsbn` prop seeds `isbn` (`:28`). Its 409 handling stays. A new `AddEditionForm.test.ts` covers the prop.
- `AddBookSheet.vue`:
  - an optional `initialIsbn` prop seeds `isbn` (`:24`);
  - an optional `stay` prop: on success it emits `added: [book: BookDetailOut]` and closes instead of navigating (`:31-34`, `:57`);
  - without `stay` it navigates as today.
- `DiscoverPage.vue` (`:1013-1017`): when `extractIsbn(submittedQuery)` matches, it opens `AddBookSheet` with `:initial-isbn` and an empty title. Today an ISBN search followed by "Add your own book" puts the ISBN into the title.
- **No server change.** Saving creates the edition at once: `POST /api/books/works/{work_id}/editions` returns 201 `EditionOut`, and `POST /api/books/custom` creates the work and its edition. `CustomBookIn` gets no `format` in v1. A save after a clean miss asks upstream once more (`claim_isbn`; misses are not cached); a 429 or 502 there keeps the form and its values.

### 3.6 Server and proxy changes

1. **CSP** (`server/src/spa.rs`, `SHELL_CSP`): `script-src 'self' 'wasm-unsafe-eval'`. Everything else stays. The doc comment above it gains one line: WebAssembly compiles from our own `/assets` file only, for the barcode decoder; `'unsafe-eval'` stays out. The pin in `server/tests/platform.rs` (`:81`) changes with it, and `the_spa_carries_the_shell_policy` gains an assertion that the policy contains no `'unsafe-eval'` and no `'unsafe-inline'` in `script-src`. One policy for every page: the SPA is one document, and a client-side route cannot change its policy. Without `'wasm-unsafe-eval'`, Chrome 97+, Firefox 102+ and Safari 16+ refuse `WebAssembly.instantiate`.
2. **Permissions-Policy** (`deploy/hosted/Caddyfile.example:36-38`): `camera=()` becomes `camera=(self)`, and the comment says the app uses the camera on its own origin for barcode scanning (#56). The rest stays. `camera=()` makes `getUserMedia` fail with `NotAllowedError` and no prompt. The header stays the proxy's (`conventions.md:195`). `deploy/hosted/README.md:278` changes the same way. MOVE checks the existing proxy's header (§5.4).
3. **The per-account catalog turn on `/isbn`** (`server/src/catalog/isbn_lookup.rs`, `lookup_isbn`): `let _turn = upstream_turn(&state, user.id).await?;` right after `parse_isbn`, so an invalid ISBN costs nothing. The utoipa 429 text adds `Slow down a little. Try again later.` A local hit takes the turn too: it is cheap, and 60 a minute is far above one scan at a time. Today `/isbn` is the only catalog read that can call Open Library without it, and scans share `ol_api` with COVER's job.
4. **Log redaction:**
   - `loggable_path` (`server/src/lib.rs:262`) turns `/api/books/isbn/<isbn>` into `/api/books/isbn/{isbn}`, with a case in its unit test (`:307-321`).
   - The Caddy filter (`Caddyfile.example:83`) becomes `^(/(?:api/invites|api/books/isbn|join)/)[^/?]+|([?]).*$`, so the path logs as `/api/books/isbn/{redacted}`.
   - `deploy/hosted/README.md:285` names it.
   - An ISBN says what someone reads, the same as a search query, which is already redacted.

### 3.7 SPA routes, client and copy

- `router.ts`: `{ path: "scan", component: () => import("./pages/ScanPage.vue") }` inside the AppShell. Back closes it.
- `client.ts`: ``lookupIsbn: (isbn: string, signal?: AbortSignal) => request<IsbnHitOut>(`/api/books/isbn/${enc(isbn)}`, { signal })``. `addToShelf` already takes `edition_id` (`ShelfAddIn`).
- `types.ts`: re-export `IsbnHitOut`.
- `apiErrors.ts`: map `No book found for that ISBN` (to a new `errors.isbnNotFound`, used only outside the scan flow, which handles 404 itself).
- i18n: `scan.*` (title, the states of §3.4, manual entry, the lookup clock, the hit card, the miss card) and `addBook`/`addEdition` keys for the prefilled ISBN, in `en.json` and `de.json`.

### 3.8 The phone test

`getUserMedia` works only on HTTPS or `localhost`. The agents VM has no certificate a phone trusts, and the move brings the real hostname with a real certificate soon after Wave 1. So the phones are tested after the move, signed in as Rafael (§5.8). SCAN merges on the Chromium run (§3.9), which covers the decoder, the CSP, the hit and the miss flow. A scanner bug found on a phone ships as a patch release through the update timer.

An optional early check on Android only, which trusts no certificate: `chrome://flags/#unsafely-treat-insecure-origin-as-secure` with `http://agents.lan:8091`. iOS has no equivalent.

### 3.9 SCAN's tests

- **`isbn.test.ts`** (the acceptance's "unit tests cover the ISBN validation"): the vectors of `isbn.rs:150-235`, including full-width digits and the 977 EAN; 978 and 979 accepted; a wrong check digit refused; ISBN-10 accepted by `parseIsbn` and refused by `isbnFromScan`; `X` only as the last ISBN-10 character.
- **`scan/decoder.test.ts`:** native is chosen only when `getSupportedFormats()` includes `ean_13` (stubbed); `[]` and a missing `BarcodeDetector` choose zxing (the dynamic import mocked).
- **`scan/zxing.test.ts`:** with `zxing-wasm/reader` mocked, `prepareZXingModule` gets a `locateFile` that returns the `?url` path for `zxing_reader.wasm`: same-origin, starting with `/`, with no `://`.
- **`scan/camera.test.ts`:** `cameraSupport` for an insecure context and a missing `mediaDevices`; `cameraError` for each `DOMException` name.
- **`BarcodeScanner.test.ts`** (jsdom, with camera and decoder mocked): the insecure, denied, no-camera and busy states each show manual entry; a valid manual ISBN-10 emits its ISBN-13; an invalid one shows the error; a decoded 977 EAN or bad check digit is ignored and a valid one emits once; the tracks stop on emit, on unmount and on `visibilitychange`.
- **`ScanPage.test.ts`:**
  - a hit adds with `edition_id`;
  - a hit with `on_shelf` sends PATCH `edition_id`;
  - a 409 on add becomes a PATCH;
  - a 404 shows the miss card; "Find the book by title" leads to `AddEditionForm` with the ISBN; "Add it as a new book" to `AddBookSheet` with the ISBN, then the hit card with the photo button;
  - 429 and 502 keep the ISBN and retry;
  - "Cancel" aborts the request.
- **`AddBookSheet.test.ts`, `DiscoverPage.test.ts`, `AddEditionForm.test.ts`:** `initialIsbn` fills the ISBN, not the title; `AddBookSheet`'s `stay` emits `added` and does not navigate.
- **Server:** the `platform.rs` pin and the new assertion; `/api/books/isbn/{isbn}` in `catalog_requests_are_limited_per_account` (the 61st request in a minute is 429 `Slow down a little. Try again later.`); the `loggable_path` case.
- **Browser, automated:** Chromium through the `playwright-cli` skill on `http://localhost:8091` (`localhost` is a secure context; the server runs with `PUBLIC_URL=http://localhost:8091` for this run), launched with `--use-fake-ui-for-media-stream --use-fake-device-for-media-stream --use-file-for-fake-video-capture=<scratchpad>/ean.mjpeg`. The file is a still EAN-13 rendered from a PNG (for example with `ffmpeg` in a container). Linux Chromium has no `BarcodeDetector`, so this runs the zxing path. Cases: one known ISBN to the hit card, one valid but unknown ISBN to the miss flow; zero `securitypolicyviolation` events.
- **Manual:** the phones, after the move (§3.8, §5.8).

---

## 4. Design: friends (#57)

### 4.1 Tables (`server/migrations/3100_friends.sql`)

The header comment says: friendships join the audience through `bc_can_see_user` and `bc_visible_owners` (as `0001_init.sql:880-887` planned), and the home feed is the one other place, deliberately (§4.4).

```sql
-- One row per pair. accepted_at NULL = a pending request from requester to addressee.
CREATE TABLE friendships (
    requester_id bigint NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    addressee_id bigint NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at   timestamptz NOT NULL DEFAULT now(),
    accepted_at  timestamptz NULL,
    PRIMARY KEY (requester_id, addressee_id),
    CONSTRAINT friendships_not_self CHECK (requester_id <> addressee_id),
    CONSTRAINT friendships_accepted_after CHECK (accepted_at IS NULL OR accepted_at >= created_at)
);
CREATE UNIQUE INDEX friendships_pair_key
    ON friendships ((least(requester_id, addressee_id)), (greatest(requester_id, addressee_id)));
CREATE INDEX friendships_addressee_idx ON friendships (addressee_id, requester_id);

-- user_id blocked blocked_user_id. Independent of friendships: a block deletes the pair's row.
CREATE TABLE user_blocks (
    user_id         bigint NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    blocked_user_id bigint NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, blocked_user_id),
    CONSTRAINT user_blocks_not_self CHECK (user_id <> blocked_user_id)
);
CREATE INDEX user_blocks_blocked_idx ON user_blocks (blocked_user_id);

-- A friend invite: a single-use club invite only this account can redeem.
ALTER TABLE club_invites ADD COLUMN invitee_id bigint NULL REFERENCES users (id) ON DELETE CASCADE;
ALTER TABLE club_invites ADD CONSTRAINT club_invites_bound
    CHECK (invitee_id IS NULL OR (club_id IS NOT NULL AND max_uses = 1));
CREATE INDEX club_invites_invitee_idx ON club_invites (invitee_id) WHERE invitee_id IS NOT NULL;
```

- Add `COMMENT ON` lines for each table and for `club_invites.invitee_id`.
- `invitee_id` is `ON DELETE CASCADE`, not `SET NULL`: NULL would turn an invite bound to one person into an open single-use code.
- The migration also replaces the two policy functions and adds `bc_friends` (§4.4).

### 4.2 Requests: states and transitions

For a pair A, B there is at most one `friendships` row. The states are: **none**; **pending A→B**; **friends**. Blocks are separate rows (§4.3). "Hidden" below means the addressee has blocked the requester: the row exists, the requester sees it as outgoing, and the addressee's lists and badge leave it out.

| Action (by A, about B) | From | To | Answer |
|---|---|---|---|
| send | none | pending A→B (hidden if B blocked A) | 200 |
| send | pending A→B | unchanged | 200 |
| send | pending B→A | friends (`accepted_at = now()`) | 200 |
| send | friends | unchanged | 200 |
| send | A blocked B | unchanged | 409 `You blocked this account` |
| send | B is A | – | 400 `That is your own account` |
| send | B unknown, deleted, or a name with NUL | – | 404 `No account with that name` |
| accept | pending B→A, visible to A | friends | 200 |
| accept | anything else (also a hidden B→A) | unchanged | 404 `No such friend request` |
| decline | pending B→A, visible to A | none | 200 |
| cancel | pending A→B (hidden or not) | none | 200 |
| decline or cancel | anything else | unchanged | 404 `No such friend request` |
| remove | friends | none; the pair's open bound club invites revoked | 200 |
| remove | anything else | unchanged | 404 `No friend with that name` |

- Decline and cancel are the same route (`DELETE /api/friends/requests/{username}`): it deletes the pending row in whichever direction the caller may act on.
- Declined requests leave no trace. The sender sees the request disappear, exactly as after a cancel, and may ask again. Block is the lasting remedy. Pending requests never expire.
- Suspension changes nothing here (ADM2's rule: a suspension leaves the account's content as it is). A suspended friend stays a friend, without a marker, as in a club's member list. Requests from and to a suspended account stay as they are; it cannot sign in to send more, and a bound invite to it waits unused.
- **Concurrency.** Send runs `INSERT … ON CONFLICT ((least(requester_id, addressee_id)), (greatest(requester_id, addressee_id))) DO NOTHING`, then reads the pair's row `FOR UPDATE` and acts on what it finds. Two people sending to each other at once therefore end as friends. A foreign-key violation (the other account was deleted meanwhile) maps to 404 `No account with that name`.

### 4.3 Blocking

`PUT /api/friends/blocks/{username}` (A blocks B), idempotent, in one transaction:
1. `INSERT INTO user_blocks … ON CONFLICT DO NOTHING`.
2. Delete the pair's `friendships` row, in any state: the friendship ends, and pending requests in either direction go.
3. Revoke the pair's open bound invites (`UPDATE club_invites SET revoked_at = clock_timestamp() WHERE revoked_at IS NULL AND ((created_by = A AND invitee_id = B) OR (created_by = B AND invitee_id = A))`).

Lock order: the `user_blocks` row, then the pair's row, then the invite rows. No club row, so no cycle with `clubs::invites::consume` (club, then inviter, then invite).

What a block does and does not do:
- **Silent.** B is never told. B's send still answers 200 and shows as outgoing (hidden from A). B's friend-invite attempts answer 404 `No friend with that name`, like any non-friend's. No answer B gets differs from the answers of an account that was simply never accepted.
- **Visibility.** It ends the friendship, so friend visibility ends. It does not hide anything in clubs A and B share, now or later: if B joins one of A's clubs (through any code, from any admin), they are co-members and audience again, and A is not told. Accepted (answer 21); `/friends` says so under the blocked list, and `friends_blocks.rs` pins it.
- **Reports** are separate: a block files nothing and shows nothing to admins.
- **Unblock** (`DELETE /api/friends/blocks/{username}`, idempotent): deletes the block and the hidden pending requests B sent to A while blocked. Unblocking never surfaces old requests. It acts only on the caller's own rows and always answers 200 with `FriendsOut`, for an unknown name too (§4.9).
- A blocked account can still block back (its own row). An account that blocked B must unblock B before sending to B (409).

### 4.4 Visibility

A friend is `Audience` (`visibility.rs` `Relation`). There is no new variant; only the doc comments at `visibility.rs:18,40` change ("they share at least one club or are friends").

**SQL** (in `3100_friends.sql`; `CREATE OR REPLACE` with the same names, parameter names and OUT columns, never `DROP`, because `bc_visible_shelf`, `bc_visible_quotes`, `bc_visible_posts` and `bc_can_see_item` depend on them):

```sql
CREATE FUNCTION bc_friends(viewer bigint) RETURNS TABLE (user_id bigint, since timestamptz)
    LANGUAGE sql STABLE PARALLEL SAFE
    BEGIN ATOMIC
        SELECT f.addressee_id, f.accepted_at FROM friendships AS f
        WHERE f.requester_id = viewer AND f.accepted_at IS NOT NULL
        UNION ALL
        SELECT f.requester_id, f.accepted_at FROM friendships AS f
        WHERE f.addressee_id = viewer AND f.accepted_at IS NOT NULL;
    END;

CREATE OR REPLACE FUNCTION bc_can_see_user(viewer bigint, owner bigint) RETURNS boolean
    LANGUAGE sql STABLE PARALLEL SAFE
    RETURN coalesce(viewer = owner, false) OR EXISTS (
        SELECT 1 FROM club_memberships AS a
        JOIN club_memberships AS b ON b.club_id = a.club_id
        WHERE a.user_id = viewer AND b.user_id = owner
    ) OR EXISTS (
        SELECT 1 FROM friendships AS f
        WHERE f.accepted_at IS NOT NULL
          AND ((f.requester_id = viewer AND f.addressee_id = owner)
               OR (f.requester_id = owner AND f.addressee_id = viewer))
    );

CREATE OR REPLACE FUNCTION bc_visible_owners(viewer bigint) RETURNS TABLE (user_id bigint, is_self boolean)
    LANGUAGE sql STABLE PARALLEL SAFE
    BEGIN ATOMIC
        SELECT viewer, true
        UNION ALL
        SELECT DISTINCT o.user_id, false
        FROM (SELECT b.user_id FROM club_memberships AS a
              JOIN club_memberships AS b ON b.club_id = a.club_id
              WHERE a.user_id = viewer
              UNION ALL
              SELECT fr.user_id FROM bc_friends(viewer) AS fr) AS o
        WHERE o.user_id <> viewer;
    END;
```

- **No duplicates:** the `DISTINCT` covers both sources. A friend who is also a co-member appears once, so reader lists, rating means, counts and reaction names do not double.
- **NULL safety:** `bc_can_see_user` stays false for a NULL owner. It uses the explicit pair form, never `least`/`greatest`, which ignore NULLs.
- **A pending request grants nothing:** every friend branch filters on `accepted_at IS NOT NULL`.
- **Inlining:** a new test, `visibility_policy.rs::policy_functions_still_inline`, runs `EXPLAIN (FORMAT JSON)` on `SELECT count(*) FROM bc_visible_shelf($1)`, `bc_visible_owners($1)` and `bc_friends($1)` and asserts there is no `Function Scan` node. If `bc_friends` does not inline inside `bc_visible_owners`, write its two branches into `bc_visible_owners` directly and keep `bc_friends` for the other callers.

**What changes by itself** (each gets a test through the matrix): shelf, favourites, layout (read-only), quotes, stats and diary for friends; the book page's readers, rating and quote count (the field keeps its name `club_rating`); reaction `users` lists and `author_avatar_url`; avatar report rights (`media/reports.rs:334-344`), which keeps the rule that a reader only sees pictures they can report; `?username=` on the home feed.

**What stays club-only:** overlap, suggestions, pick readers, club stats, the members' previews, the club feed and club posts. They join `club_memberships`, which is correct.

**The home feed** (`activity/feed.rs:223-228`) is the one other place. Personal events of friends appear from the moment of acceptance, mirroring `joined_at`. The friend branch sits inside the `e.club_id IS NULL` group, so a friend's club events never reach a viewer outside that club:

```sql
WHERE (e.club_id IN (SELECT mc.club_id FROM bc_member_clubs($1) mc)
       OR (e.club_id IS NULL
           AND (e.actor_id = $1
                OR EXISTS (SELECT 1 FROM bc_member_clubs($1) mc
                           JOIN club_memberships m ON m.club_id = mc.club_id
                           WHERE m.user_id = e.actor_id AND m.joined_at <= e.created_at)
                OR EXISTS (SELECT 1 FROM bc_friends($1) f
                           WHERE f.user_id = e.actor_id AND f.since <= e.created_at))))
```

Without it, friends' events never reach Home, and `/api/activity?username=<friend>` would pass `bc_can_see_user` and return nothing. The migration header and `conventions.md` §6.4 record this exception.

**Copy:** `errors.noMember` (`en.json:642`, "Nobody with that name shares a club with you") becomes "Nobody by that name in your clubs or among your friends", in English and German. The server string `No member with that name` does not change.

### 4.5 API (tag `friends`, module `server/src/friends/`, merged in `lib.rs`)

Every write answers 200 with the caller's whole `FriendsOut`, so the SPA's store just replaces its state. The `{username}` path parameter is normalised like every username lookup (`auth::rules::normalize_username`), and a NUL character counts as unknown.

| Route | Body | Errors (besides 401 `Not signed in`) |
|---|---|---|
| `GET /api/friends` | – | – |
| `POST /api/friends/requests` | `FriendRequestIn {username}` | 400 `Field required`; 400 `That is your own account`; 404 `No account with that name`; 409 `You blocked this account`; 429 `Slow down a little. Try again later.` |
| `POST /api/friends/requests/{username}/accept` | – | 404 `No such friend request` |
| `DELETE /api/friends/requests/{username}` | – | 404 `No such friend request` |
| `DELETE /api/friends/list/{username}` | – | 404 `No friend with that name` |
| `PUT /api/friends/blocks/{username}` | – | 400 `That is your own account`; 404 `No account with that name`; 429 |
| `DELETE /api/friends/blocks/{username}` | – | – (always 200, §4.9) |
| `DELETE /api/friends/club-invites/{invite_id}` | – | 404 `No such invite` (`clubs::invites::NO_SUCH_INVITE`) |

- Removal lives at `/api/friends/list/{username}`, not `/api/friends/{username}`: usernames such as `requests` or `blocks` are valid, and would collide with the static segments of the other routes.
- New `msg` constants in `server/src/error.rs`: `SELF_ACCOUNT` (`That is your own account`), `YOU_BLOCKED` (`You blocked this account`), `NO_FRIEND_REQUEST` (`No such friend request`), `NO_FRIEND` (`No friend with that name`), `FRIEND_IN_CLUB` (`That friend is already in this club`). `NO_ACCOUNT` (`No account with that name`) moves from `admin/users.rs:48` into `msg`. Each is mapped in `apiErrors.ts` (`server/tests/api_error_copy.rs`).
- Operation ids: `friends_list`, `friends_send_request`, `friends_accept_request`, `friends_delete_request`, `friends_remove`, `friends_block`, `friends_unblock`, `friends_decline_club_invite`.

**DTOs** (`friends::out`; none in endpoint-map §0; the names are unique crate-wide):

```text
FriendsOut { friends: FriendOut[], incoming: FriendRequestOut[], outgoing: FriendRequestOut[],
             blocked: BlockedOut[], club_invites: FriendClubInviteOut[] }
FriendOut { user: UserSummary, since: timestamp, currently_reading_count: i64,
            currently_reading: FriendReadingOut[] }          // at most 3, in shelf order
FriendReadingOut { book: BookOut, progress: i32|null }
FriendRequestOut { user: UserSummary, created_at: timestamp }   // avatar_url null unless bc_can_see_user
BlockedOut { username, created_at }
FriendClubInviteOut { id, club: ClubRef, invited_by: string|null, code, created_at, expires_at }
```

- `friends`: sorted by username. Avatars always shown (friends are audience).
- `incoming`: newest first; hidden rows left out (the caller blocked the requester).
- `outgoing`: newest first, hidden ones included.
- `blocked`: newest first, without avatars.
- `club_invites`: the caller's active bound invites (not revoked, not expired, not used) to clubs the caller is not in, newest first.
- **Avatars of requests:** `FriendRequestOut.user.avatar_url` is null unless `bc_can_see_user(viewer, other)`. That keeps the ADM2 invariant (`conventions.md` §6.3): a reader only sees a picture they could report.
- **Currently reading** follows `clubs/out.rs` `member_outs` (`:174-250`): one query for the people, one windowed query through `bc_visible_shelf` (the review rule), then `book_outs()`. No N+1:

  ```sql
  SELECT r.user_id, r.work_id, r.edition_id, r.progress, r.total FROM (
    SELECT s.user_id, s.work_id, s.edition_id, s.progress,
           row_number() OVER (PARTITION BY s.user_id ORDER BY s.position, s.id) AS rn,
           count(*)     OVER (PARTITION BY s.user_id) AS total
    FROM bc_visible_shelf($1) s JOIN bc_friends($1) f ON f.user_id = s.user_id
    WHERE s.status = 'currently_reading') r
  WHERE r.rn <= 3 ORDER BY r.user_id, r.rn
  ```

- No rating fields anywhere, so `RATING_SCHEMAS` does not grow. No friend list is ever served to anyone else, and there are no counts of friends.
- No cap on the number of friends: the request limit bounds growth.

### 4.6 Friend invites into a club

- **Route:** `POST /api/clubs/{slug}/invites` with `{kind: "friend", username}`. `InviteKind` gains `Friend`, and `InviteIn` gains `username: Option<String>`. It is required for `friend` (400 `Field required`) and refused for the other kinds (400 `Only a friend invite names a person`, a new `msg` constant, mapped).
- **Who:** club admins, as for every invite today (`ClubAdmin`). Any member, as the legacy app allowed, is a later widening.
- **Transaction:** `lock_club`, re-read the caller's role (the existing pattern), then the pair's `friendships` row `FOR SHARE`. That conflicts with an unfriend's or a block's `DELETE`, so an invite cannot slip past a concurrent unfriend.
  - Not friends, unknown, or blocked either way: 404 `No friend with that name`.
  - Already a member: 409 `That friend is already in this club`.
  - An active bound invite for the same friend and club already exists: 201 with that invite, no second row.
  - Otherwise insert `max_uses = 1`, `expires_at = now() + 30 days` (or `expires_in_days`, 1–30), `invitee_id`.
- **`InviteOut` gains `invitee: string|null` and `personal: bool`** (`clubs/out.rs:76`, the `invite_outs` query). `personal` is true for every bound invite. `invitee` (the username) is filled only for the invite's creator: a co-admin would otherwise learn whom the creator is friends with (answer 24). The club's invite list shows "for <name>", or "personal invite" to other admins. Revoking works as for every invite.
- **Redemption:** `consume` (`clubs/invites.rs:85-120`) adds `AND (invitee_id IS NULL OR invitee_id = $2)` to its atomic `UPDATE`, with the redeeming user's id as `$2`.
  - A forwarded code is useless to anyone else.
  - Sign-up passes the new account's id, so a bound code can never sign anyone up (400 `That invite code is not valid`).
  - The 20-member cap, the removal rule, `invited_by` and the `member_joined` event work unchanged.
  - The invitee joins through the existing `/join/{code}` and `POST /api/invites/{code}/accept`. An admin never adds anyone directly: joining widens the joiner's audience to every member (§6.1), so the friend must agree.
- **Revocation:**
  - A friend declines with `DELETE /api/friends/club-invites/{invite_id}` (sets `revoked_at` where `invitee_id` is the caller).
  - Unfriend and block revoke the pair's bound invites (§4.2, §4.3).
  - A member's removal (`clubs/members.rs` `revoke_known_invites`, `:225-250`) adds `OR invitee_id = $2`, so a removed member cannot walk back in with one.
  - A suspension adds nothing: it already revokes the invites the account created, and one bound to it needs no revoking, since only the invitee can redeem it and a suspended account cannot sign in.

### 4.7 Account references (CLAUDE.md "Cross-cutting rules")

FRIENDS adds 5 foreign-key columns to `users`. COVER adds none.

| Column | Export (`auth_export.rs` `SOURCES`/`EXEMPT`; the count `:212` goes 29 → 34) | Purge (`auth::empty_sign_ups`) and `tests/jobs.rs` | `parity_visibility.rs` `USER_COLUMNS` |
|---|---|---|---|
| `friendships.requester_id` | key `friends` | `NOT EXISTS (SELECT 1 FROM friendships x WHERE x.requester_id = u.id)`; **DATA** (the account sent a request) | `Kinds(&[Kind::Friends])` |
| `friendships.addressee_id` | key `friends` | `NOT EXISTS (… WHERE x.addressee_id = u.id AND x.accepted_at IS NOT NULL)`; **DATA** (accepted). Also **TRACES** for a pending request received: "another account's act, never answered" | `Kinds(&[Kind::Friends])` |
| `user_blocks.user_id` | key `blocks`: `[{username, created_at}]` | `NOT EXISTS`; **DATA** | `Kinds(&[Kind::Friends])` |
| `user_blocks.blocked_user_id` | **EXEMPT**: "who blocked the account; telling it would undo the block (Art. 15(4), like the reporter of a report)" | **TRACES**: "another account's block; it goes with the account" | `Not("never served: the blocked account is not told (friends_blocks.rs)")` |
| `club_invites.invitee_id` | key `club_invites_received`: `[{club_name, invited_by, created_at, expires_at, used, revoked}]` | **TRACES**: "an invite another account sent" | `Not("invite data: the invite's creator and the invitee (clubs_invites.rs, friends_invites.rs)")` |

- **The `friends` export key** is `{friends: [{username, since}], requests_sent: [{username, created_at}], requests_received: [{username, created_at}]}`. Both directions are listed, hidden rows included: each row is the account's own act or an act addressed to it. The `reports` key is the precedent for an object.
- **`invites_created`** (`auth/export.rs:345-368`) serialises `to_jsonb(i) - 'created_by'`, which would now carry another account's id. Subtract `'invitee_id'` too and add `'invitee'` as the username.
- **`tests/jobs.rs`:** `DATA` 27 → 30, `TRACES` 3 → 6. `friendships.addressee_id` appears in both, and the completeness test takes the union. TRACES fixtures create rows that leave the account an empty sign-up (the sign-up 31 days old). The bound-invite fixture is `INSERT INTO club_invites (club_id, code, created_by, max_uses, expires_at, invitee_id) VALUES ({club}, 'BOUND{u}', {keeper}, 1, now() + interval '30 days', {u})`. `USERS_COLUMNS` does not change: no `users` column is added. The test name `every_reference_to_an_account_is_data_or_a_trace_of_signing_in` stays; the docs widen "trace of signing in" to "a trace of signing in or another account's act".
- **Account deletion** (`auth/deletion.rs` `lock_clubs`, `:142-177`): add `UNION ALL SELECT club_id FROM club_invites WHERE invitee_id = $1`, and a case in `auth_delete.rs::the_deletion_locks_every_club_it_writes_in_first`. `friendships` and `user_blocks` cascade without a club and need no lock.
- **`parity_schema.rs`:** `friendships` joins `GLOBAL` (17 after COVER → 18): it is a relation between two accounts with no single `user_id`. `user_blocks` joins `PERSONAL` (6 → 7): `user_id NOT NULL → users`, never read by others, like `search_history`. The checklist's #37 table (`parity-checklist.md:96-111`) gets both.
- **`docs/legal/vvt.md`:** only what `legal_docs.rs` needs: the rows `friendships | 16` and `user_blocks | 16`, `club_invites` gaining 16, and a stub heading "16. Friends (written with #53)".
- **After a compromise** (`revoke-access.sql`'s header, step 7 of the README's restore section): accepted `friendships` rows are reviewed by hand with the memberships, `SELECT … FROM friendships WHERE accepted_at >= :since`. A planted friendship grants the same view as a planted membership.
- **No new foreign key to `media`**, so `MEDIA_REFERENCES` stays.

### 4.8 The visibility matrix and its viewers

- **`parity_visibility.rs`:**
  - `Case::Friend` joins `CASES` (4 → 5). Every kind function then fails to compile until it handles it.
  - `every_personal_data_kind_has_its_four_viewer_cases` keeps its name, because `parity_checklist.rs` fails when the checklist (`:124`) names a test that does not exist. Its doc comment and the checklist's prose say five cases.
  - `Kind::Friends` (label "Friends") covers `GET /api/friends` and the username routes. Like `SearchHistory`, it is its owner's alone.
  - `USER_COLUMNS` gains the five rows of §4.7.
  - `USERNAME_ROUTES` (11 rows today) gains five, all `Kinds(&[Kind::Friends])`: `POST /api/friends/requests/{username}/accept`, `DELETE /api/friends/requests/{username}`, `DELETE /api/friends/list/{username}`, `PUT /api/friends/blocks/{username}`, `DELETE /api/friends/blocks/{username}`.
- **The Friend case per kind:**

  | Kind | Friend of the owner |
  |---|---|
  | Shelf, Favourites, Quotes, Stats, Diary | like a co-member: `only_me` items redacted. The owner's club posts stay invisible. |
  | Layout | GET 200; PUT 404, like a co-member |
  | Book page | the owner is a reader; an `only_me` rating is left out of the reader, the mean and the count |
  | Activity | the owner's personal events on the home feed from `accepted_at` on (an earlier event stays hidden), redacted; the club `shared` feed is 404 `No such club` |
  | Pick readers, Club shelves | like a stranger: 404 `No such club`, and another club's views leave the owner out |
  | Search history, Import jobs | hidden |
  | Friends | the friend sees the owner in their own list, never the owner's other friends, requests or blocks |

- **`tests/common/visibility.rs`** (W1-frozen; this is a recorded amendment). Friendships are made through the routes:
  - `friend`: a friend of the owner, in no club. `audience()` returns `[co_member, friend]`.
  - `strangers()` adds `asked` (sent the owner a request, not accepted), `requested` (the owner sent them a request, not accepted), `exfriend` (was a friend, removed) and `blocked` (the owner blocked them after a friendship). `asked` and `requested` test each direction's branch of `bc_can_see_user` and `bc_friends`.
  - `stranger` becomes a friend of `friend`: friendship is not transitive.
- **`club_visibility.rs:258`** loops `audience()` over the club feed, where a friend gets 404. That loop switches to `&v.co_member`. The audience loops in `shelf_visibility.rs`, `posts_visibility.rs` and `catalog_visibility.rs` cover personal data only and stay.
- **`visibility_policy.rs`:** `relation_and_can_see_user_follow_shared_clubs` (`:59`) gains: friend → `Audience`; `asked`, `requested`, `exfriend`, `blocked` and the friend of a friend → `Stranger` and absent from `bc_visible_owners`; a NULL owner → false; a friend who is also a co-member appears once in `bc_visible_owners` (`rating_count` 1, one reaction name). Plus the `EXPLAIN` test of §4.4.
- **`parity-checklist.md` #46** (`:119-140`): a "Friend" column in the table, the "Friends" row, and the prose counts: 34 columns, 16 routes, 11 rating schemas, five viewer cases. The test reads only the first column.

### 4.9 Rate limit

- `ratelimit.rs`: `friend_request_user: SlidingWindow::new(20, DAY)`, keyed `u:<id>`. Over the limit: 429 `Slow down a little. Try again later.` with `Retry-After`.
- It counts every `POST /api/friends/requests`, unknown names and repeats included, and every `PUT /api/friends/blocks/{username}` unless the caller already has a `friendships` row with that account (a request or a friendship has disclosed the name already). So blocking any number of incoming requesters works.
- It bounds probing for names: 404 `No account with that name` reveals whether an account exists, as sign-up already does (`auth/signup.rs:120-122`, 5 per IP an hour). Only these two routes tell names apart:

  | Route | Unknown name | Existing name |
  |---|---|---|
  | `POST /api/friends/requests` | 404 `No account with that name` (counted) | 200 (counted) |
  | `PUT /api/friends/blocks/{username}` | 404 `No account with that name` (counted) | 200 (counted unless a row exists) |
  | accept, decline/cancel | 404 `No such friend request` | the same without a request |
  | `DELETE /api/friends/list/{username}`, `kind: "friend"` invites | 404 `No friend with that name` | the same without a friendship |
  | `DELETE /api/friends/blocks/{username}` | 200 | 200 |
- Accept, decline, cancel, remove, unblock and friend invites are not counted. Invites are bounded by friends times clubs.
- The struct, `new()`, `thresholds_match_the_table` (`ratelimit.rs:467`) and `conventions.md` §10.1 change together.

### 4.10 SPA

- **`router.ts`:** `{ path: "friends", component: FriendsPage }` replaces the redirect at `:118`; `router.test.ts:56` changes to expect the page. `/friends/:username` → `/u/:username` stays (`:119`, `router.test.ts:85`).
- **`nav.ts`:** `NavIconName` gains `"friends"`; a new item `{to: "/friends", labelKey: "nav.friends", icon: "friends", bottomBar: false}` after Club. It is in the sidebar only: the phone tab bar already has five tabs. Its icon goes in `NavIcon.vue`; `nav.test.ts` follows.
- **`stores/friends.ts`** (Pinia):
  - `state: FriendsOut | null`, `load()`, and one action per route, each replacing the state with the answer;
  - `badge = incoming.length + club_invites.length`;
  - a failed `load()` leaves the state as it was and shows no error (the badge is a hint).
  - `AppShell.vue` calls `load()` on mount and again on `visibilitychange` to visible, at most once a minute, and shows the badge on the Friends item (the pattern of `components/admin/AdminLink.vue`).
  - No `UserOut` field: §0 is frozen and pinned by `openapi_contract.rs`.
- **`pages/FriendsPage.vue`**, top to bottom:
  1. "Add a friend": an exact-username field and "Send request". Copy: "Type their exact username."
  2. Requests to you: Accept, Decline, Block (with a confirm).
  3. Club invitations: "<name> invited you to <club>", Join (navigates to `/join/<code>`) and Decline.
  4. Friends: avatar, name (link to `/u/<name>`), "friends since", up to three current books (cover thumb, title, progress) and "+N more" (link to `/u/<name>/shelf`). Each friend's menu: "Invite to a club…" (`InviteFriendSheet`), Remove (confirm), Block (confirm).
  5. Requests you sent: Cancel.
  6. Blocked: Unblock, and the line "Blocking ends the friendship and hides their requests. It does not hide anything in clubs you share now or join later. To stop that, leave the club or ask its admin to remove them."
  - Empty states for each list.
  - Every new server string is mapped (`apiErrors.ts`).
- **`components/InviteFriendSheet.vue`:** prop `{username: string}`. It lists the caller's clubs where they are owner or admin (from the session's `clubs`), calls `api.createInvite(slug, {kind: "friend", username})`, shows "Invitation sent", and maps 404 and 409.
- **`HomePage.vue`:** a compact "Friends" row (a link to `/friends`, with the badge count when there are requests or invitations, and "Add friends" with none). This is the phone's way to `/friends`.
- **`pages/club/ClubMembersPage.vue`:** bound invites show "for <name>", or "personal invite" when `invitee` is null (§4.6).
- **i18n:** `friends.*`, `nav.friends`, the new `errors.*`, and the changed `errors.noMember`, in both files.

### 4.11 FRIENDS' tests

- **`friends_requests.rs`:**
  - every row of the §4.2 table;
  - mutual send becomes friends;
  - a concurrent mutual send (two tasks) ends as one accepted row;
  - repeats are idempotent;
  - the 21st request in a day is 429, unknown names counted;
  - normalisation (`" Ada "` is `ada`) and a NUL name are 404.
- **`friends_blocks.rs`:**
  - a block in each pair state (none, pending either way, friends) deletes the row;
  - a request sent after the block is hidden from the blocker and looks unchanged to the sender (200, in `outgoing`);
  - `You blocked this account`;
  - unblock deletes hidden requests; unblocking an unknown name and an existing name you have not blocked give identical answers;
  - blocking 25 incoming requesters in one day works;
  - a block revokes bound invites in both directions;
  - a block changes nothing in a shared club (`relation` is still `Audience`), and B joining one of A's clubs later through a third admin's friend invite makes B `Audience` again (accepted, answer 21).
- **`friends_invites.rs`** (the acceptance's end to end):
  - an admin invites a friend; the friend sees it in `club_invites`, previews `/api/invites/{code}`, accepts, and is a member with `invited_by`;
  - a non-admin gets 403;
  - a non-friend, an unknown name and a blocked friend get the same 404;
  - an existing member gets 409;
  - a second invite returns the first;
  - another account accepting the code and a sign-up with it get 400 `That invite code is not valid`;
  - the 20-member cap holds;
  - decline, unfriend, block and removal from the club revoke;
  - a 21-day-old invite still works and a 31-day-old one does not;
  - `InviteOut.invitee` names the friend for the creator only; a co-admin's list shows `personal: true` and `invitee: null`.
- **`friends_visibility.rs`:**
  - requesters' avatars are null unless visible;
  - `GET /api/friends` never shows another account's friends;
  - currently reading shows at most 3 per friend, in shelf order, with the count;
  - an `only_me` rating never appears;
  - a friend's club event, in a club the viewer is not in, never reaches the viewer's home feed, with or without `?username=`.
- **Updates:** `parity_visibility.rs`, `visibility_policy.rs`, `club_visibility.rs`, `auth_export.rs` (34 and the three sections, plus `invites_created` without `invitee_id`), `jobs.rs`, `auth_delete.rs`, `parity_schema.rs`, `clubs_invites.rs`, the `ratelimit.rs` unit test.
- **SPA:** `FriendsPage.test.ts`, `stores/friends.test.ts` (the badge, replacing state, the visibility reload, a failed load), `InviteFriendSheet.test.ts`, `router.test.ts`, `nav.test.ts`, `HomePage.test.ts`, `club/ClubMembersPage.test.ts`, and the fixes in `acceptance.test.ts` and `LegalPage.test.ts`.

---

## 5. Design: the slim private move (MOVE)

### 5.1 What the server needs to run

Outside `DEV_MODE` (`server/src/config.rs`):

| Setting | Value for the move |
|---|---|
| `DATABASE_URL`, `POSTGRES_PASSWORD` | as in `env.example`; `openssl rand -hex 32` for the password |
| `PUBLIC_URL` | `https://<the existing hostname>` |
| `TRUSTED_PROXIES` | the address the app sees for the proxy, read before the move (§5.4) |
| `CONTACT_EMAIL` | the address of Rafael's Forgejo account, unless he names another (Q-M3). It is public in `/api/config` and on the sign-in page, and goes into the User-Agent that Open Library and the DNB see |
| `EMAIL_TRANSPORT` | `none` (§5.2); no `SMTP_URL`, no `EMAIL_FROM` |
| `SIGNUP_MODE` | `invite` |
| `LEGAL_DIR` | `/app/legal`, mounted from an **empty** `/srv/bookclub/legal` (it must exist: `compose.yaml` sets `create_host_path: false`). No terms, no terms prompt; `/imprint` shows "missing"; the footer links stay hidden |
| `COOKIE_SECURE` | default (true). The cookie is `__Host-bookclub_session`; members sign in again once |
| `DNB_HARVEST` | `false` until the import is done, then `true` |
| `BOOKCLUB_TAG` | `stable` (v0.11.0 from M2) |
| `BOOKCLUB_LISTEN`, `BOOKCLUB_PORT` | the Python app's published binding and port, or the network alias of §5.4 |
| `SERVICE_NAME` | default `Bookclub` |

Everything else in `env.example` keeps its default.

### 5.2 Mail off

Nobody needs mail today. The Python app has none (`README.md:162`), and imported users have no address (`importer/users.rs:73-76`). The server does not start without `SMTP_URL` and `EMAIL_FROM` unless `DEV_MODE`, and `EMAIL_TRANSPORT=log` writes reset links to the log, which production refuses. So MOVE adds an explicit off switch:

- **`EmailTransport::None`** (`config.rs:48-55,174-195`):
  - `EMAIL_TRANSPORT=none` is allowed without `DEV_MODE`. Unset still means `smtp` outside `DEV_MODE`, so the secure default stays. The error text becomes `EMAIL_TRANSPORT must be log, smtp or none, not {other:?}`.
  - With `none`, `SMTP_URL` and `EMAIL_FROM` are not required.
  - `SIGNUP_MODE=open` with `none` stops startup with `SIGNUP_MODE=open needs mail: set EMAIL_TRANSPORT=smtp`, checked in `Config::from_env`, so before the legal texts' check. Strangers need verification and password reset.
- **`email::NoEmail`** (`email/mod.rs`): `send` returns `Err` with `email::MAIL_OFF` = `mail is off (EMAIL_TRANSPORT=none)`. It logs no recipient, subject, body or link, so reset links never reach a log outside `DEV_MODE` (`conventions.md` §11). Callers treat the mail as not sent: a reset link is voided, and `admin/notify.rs` sets no `notified_at`.
- **`auth::mail::deliver`** (`auth/mail.rs:181-217`): when the error is `MAIL_OFF`, it logs once at INFO (`mail not sent: mail is off`, with the kind only) instead of WARN. `admin/notify.rs` and `admin/notices.rs` mail only verified addresses, and with mail off no address can be verified, so they never get there.
- **`lib.rs:94-97`:** `EmailTransport::None => Arc::new(email::NoEmail)`.
- **`main.rs`:** `send-test-mail` with `none` exits 1 and prints `mail is off (EMAIL_TRANSPORT=none)` to stderr.
- **`bookclub-server set-password <username>`:** the password reset without mail.
  - It generates 20 characters of the invite-code alphabet (Crockford base32) and prints only the password, to stdout. The log goes to stderr, as for every command (`main.rs:86-89`).
  - In one transaction it does what the reset confirm does (`auth/reset.rs:205-219`): the new `password_hash` (the same hasher as sign-up), `password_changed_at`, every session and bearer token deleted, the open reset and verify links voided. It is `auth::reset::set_password(conn, username) -> sqlx::Result<Option<String>>`, sharing that body with the confirm.
  - An unknown name exits 1 with `No account with that name`. A suspended account gets the password and stays suspended.
  - It cannot clear the running server's in-memory login-failure buckets; they empty within 10 minutes.
  - The member changes the password in Settings the same day (`PATCH /api/auth/password`).
  - During the move its output goes to a root-only file that Rafael reads and deletes (§5.6). Afterwards Rafael runs it himself: `docker compose exec app bookclub-server set-password <name>` in the stack's directory.
- **`env.example`:** the mail comment explains `none` ("until a mail provider exists: no mail leaves, reset links are voided, password resets go through `bookclub-server set-password`") and keeps `#EMAIL_TRANSPORT=smtp`. `deploy_env.rs` is unaffected.
- **Tests:**
  - `config.rs` unit tests `none_needs_no_smtp`, `open_signup_needs_mail` and `unknown_transport_names_all_three`;
  - `server/tests/auth_set_password.rs`: the new password signs in and the old one does not; the account's earlier sessions get 401; an open reset link is voided; an unknown name gives `None`.
- **Known limits, accepted for two users who know Rafael** ("For Rafael" in §1, answer 47):
  - "Forgot password" and the email prompt still say a link is on its way. The move message tells both members (§5.6).
  - The prompt stays and can be dismissed. An address entered now is stored unverified and can be verified with "resend" once mail exists.
  - A new club needs a verified address (`VerifiedUser`, `clubs/manage.rs:132-133`), so no new club can be created until mail exists. The imported club and friend invites into it work.
  - Sign-up still asks for an address; with mail off the account is created unverified (the boot test of §2.3 checks the 201).

### 5.3 The old service worker

Users who installed the Python app before PR #3 may still run its offline worker. Today `/sw.js` returns the shell, so the browser's update check fails and the worker stays (risk R22, `docs/phase1/spec/platform.md:1375-1378`). On the same hostname only the app can fix that:
- `frontend/public/sw.js` is a copy of `deploy/legacy-origin/sw.js` (skip waiting, delete every cache, unregister, reload the open pages). Its header comment says why it lives here. It has no fetch handler.
- `spa.rs` serves it as an existing file, with a JavaScript type and the shell's CSP. Browsers bypass the HTTP cache for a worker's main script, so it needs no special header.
- `frontend/src/swKillSwitch.test.ts` reads `public/sw.js` and asserts that it calls `registration.unregister()` and `caches.delete`, and has no `fetch` listener.
- `thirdParty.test.ts` already scans `public/`.

### 5.4 The host and the switch

**Facts first.** `deploy/cutover/host-facts.sh <old compose directory> <hostname>` is read-only and prints no secret:
- the OS, `uname -m` (the image is amd64 only), Docker Engine (≥ 25) and compose (≥ 2.24) versions, and systemd;
- `docker compose ls`;
- the old container's image, published ports and binding, and data mount;
- `BOOKCLUB_NAME`, `BOOKCLUB_TZ`, `BOOKCLUB_PORT` and `BOOKCLUB_PUBLIC_URL` from the old `.env`, by exact key (never `SECRET_KEY`);
- whether root's Docker config holds `git.tillhome.de` (presence only);
- the old container's established peers on its port (`nsenter` plus `ss`, as `deploy/hosted/README.md` check 3), to run while someone uses the app;
- `curl -sI https://<hostname>/` for HTTPS, `Strict-Transport-Security` and `Permissions-Policy`;
- free disk under `/srv` and `/var/lib/docker`, the disks (`lsblk -o NAME,SIZE,MOUNTPOINT`) and mounted NAS shares (`findmnt -t nfs,nfs4,cifs,smb3`), for the backups (§5.5);
- every container (`docker ps -a --format '{{.Names}} {{.Image}}'`), flagging auto-updaters such as watchtower or ouroboros;
- whether `sqlite3` is installed;
- whether `/opt/bookclub` and `/srv/bookclub` exist.

I run it over SSH (Q-M1).

**Layout.**
- The new stack goes to `/opt/bookclub` and `/srv/bookclub` as `deploy/hosted/README.md` describes ("First boot" 1–4, without step 5's test mail).
- If the old app occupies either path, the new one goes to `/opt/bookclub-server` and `/srv/bookclub-server`: set `BOOKCLUB_DATA_DIR` in `.env`, `COMPOSE_DIR` and `MEDIA_DIR` in `backup.env`, and change the three units' `ExecStart` with `sed` at install. Record it in the notes.
- **Project name:** `compose.yaml` says `name: bookclub`. If `docker compose ls` shows the old project under that name, the new `.env` sets `COMPOSE_PROJECT_NAME=bookclub-server`. Otherwise the stopped Python container becomes an orphan of the new project, and a `down` or `--remove-orphans` could remove it.

**The switch: port takeover.**
- The new app publishes on the Python app's binding and port (`BOOKCLUB_LISTEN`, `BOOKCLUB_PORT`; the Python default is `0.0.0.0:8000`). The proxy needs no edit, the hostname stays, and a rollback is two commands.
- Before the move day only `postgres` runs. To check the app, start it once on a spare loopback port and stop it again: `BOOKCLUB_LISTEN=127.0.0.1 BOOKCLUB_PORT=18080 docker compose up -d --wait app`, `healthcheck`, then `docker compose stop app`.
- The importer accepts a migrated empty database. The target must hold no users and no invites: no `beta-invite`, no sign-up.
- If the proxy runs on the same host and reaches the app over loopback, set `BOOKCLUB_LISTEN=127.0.0.1`. Otherwise keep the old binding.
- **No published port.** `SELFHOST.md` lets a proxy in the same Compose setup reach `bookclub:8000` on a Docker network instead. Then a `compose.override.yaml` next to the new `compose.yaml` attaches `app` to that network (external) with the alias the proxy uses, and `.env` sets `BIND_ADDR=0.0.0.0:8000` (the image's healthcheck reads it too) and `BOOKCLUB_LISTEN=127.0.0.1`. Compose loads the override by itself, so `update.sh` and the backup scripts are unaffected, and the rollback stays two commands. The check before move day then runs with `-f compose.yaml` only, so the new app never answers under the alias early. If that does not fit the proxy, I send Rafael a one-line upstream change, and reverting it joins the rollback.
- **Auto-updaters.** If one runs, I exclude the new stack from it before M3 (watchtower: the label `com.centurylinklabs.watchtower.enable=false` on `app` and `postgres` in `compose.override.yaml`) and make sure it does not revive stopped containers, or I stop it. `update.sh`, which backs up first, is the new stack's only updater.

**`TRUSTED_PROXIES`** is the peer `host-facts.sh` saw on the Python container, as `/32`. Through Docker's published port, a proxy on another host shows its own LAN address. A proxy on the same host connecting to `127.0.0.1` shows the bridge gateway, which is fine only with `BOOKCLUB_LISTEN=127.0.0.1`. A wrong value makes the app take the proxy for the client: everyone lands in one rate-limit bucket, and one stranger's failed attempts lock both members out of sign-in for 10 minutes and out of reset and sign-up for an hour. A wide one lets LAN clients forge `X-Forwarded-For`. Move day checks it from two sides (§5.6, the §3.5 row).

**Checks that stop the move:**
- `uname -m` is not `x86_64`.
- Docker or compose is too old.
- The hostname does not answer over HTTPS (the cookie is `Secure`).
- The proxy sends `Permissions-Policy` with `camera=()`. That blocks the scanner only, not the move: I send Rafael the exact line to change (`camera=(self)`), or the scanner waits.

**Registry:** the package is private. If root is not logged in to `git.tillhome.de`, **[Rafael]** creates a token with only `read:package` (Forgejo → Settings → Applications) and runs `docker login git.tillhome.de` with it as root on the host. The token never passes through my terminal.

### 5.5 Backups and updates

- **Backups:** restic with `deploy/hosted/backup.sh`, `backup-lib.sh` and `restore-test.sh`, and the units in `deploy/hosted/systemd/` (nightly 03:30, Sunday restore test).
  - `RESTIC_REPOSITORY` is an absolute path, chosen from the host facts: a mounted NAS share if there is one; otherwise a directory on another physical disk; otherwise `/srv/bookclub-backup` on the same disk. In the last case I tell Rafael that the backups then guard against mistakes, not against losing the disk, until the Storage Box comes with the public launch. Never under `/tmp` (the units use `PrivateTmp`).
  - `restic init` once. I generate the password file without printing it: `openssl rand -base64 32 | install -m 600 /dev/stdin /opt/bookclub/restic-password`. **[Rafael]** copies it into his password manager from his own SSH session. Without it the backups are unreadable.
  - The three heartbeat URLs point at the monitor Q-M2 names, if there is one. Otherwise they stay empty, and a failed backup or restore test stays silent until someone looks. That is accepted until the public launch brings a monitor; M6 checks the backups before it deletes the old copy (§5.8).
  - Run `backup.sh` and `restore-test.sh` once before the move, and once on move day after the check.
- **Updates:** `update.sh` and `bookclub-update.timer` following `stable`, enabled after the rollback window closes (§5.7). Rafael never runs a deploy. `update.sh` backs up before every update and stops when the backup fails, so a failing backup shows as a release that does not arrive.
- **No external monitor.** Docker's healthcheck and `restart: unless-stopped` restart a crashed app. A check from outside comes with the public launch (Q10).

### 5.6 The runbook (`docs/phase3/private-move.md`)

MOVE writes it as a copy-paste runbook built on `docs/phase2/cutover.md` as COVER leaves it (COVER's notes §8.2 step 10: the cover search inside `import-legacy`, its report lines, and a review of cover suggestions). Variables: `HOST` (the hostname), `OWNER` (Rafael's legacy username; CUT2 assumed the user with id 1), `CLUB_NAME` and `CLUB_TZ` (from the old `.env`; CUT2 assumed "Bookclub" and Europe/Berlin), `SLUG`, `IMPORT=/srv/bookclub/import`, `OLD=<old compose directory>`. `SLUG=bookclub-8rqp` is CUT2's; if the old `BOOKCLUB_NAME` is not "Bookclub", I generate a new one by D5 (the slugified name plus 4 random characters) from the host facts, before the message goes out.

**What may reach my terminal:** counts, statuses and exit codes only. No command whose output holds a secret, a password hash, user rows, titles or client addresses runs there:
- the restic password is generated into its file (§5.5), and Rafael runs `docker login` himself (§5.4);
- `set-password`, if needed, writes to a root-only file (`umask 077`) that Rafael reads and deletes;
- the import reports stay on the host; I read `compare.sh`'s result and the report's entity, `lookup wait` and `cover search` lines, never the `isbn keys` or `without a cover` sections;
- the proxy's address for `TRUSTED_PROXIES` is not a client's.

| `cutover.md` | For the private move |
|---|---|
| §1 checklist | Keep: the stack on `stable` with `SIGNUP_MODE=invite` and `DNB_HARVEST=false`; backups set up and `restore-test.sh` passed once; no auto-updater touches either stack (§5.4); an empty target (no `beta-invite`, no sign-up); `install -d -o 1000 -g 1000 -m 700 "$IMPORT"`; room in the app's `/tmp`. Drop: name and domain, mail, legal texts, the uptime monitor, staging, the Caddy blocks, the gate address. `compare.sh` needs `sqlite3`: install it, or use CUT2's container (`notes/cut2.md`, "Commands") with `docker compose exec -T postgres psql …` as the psql command |
| §2 announcement | Replaced by one short chat message, DE and EN, a few days ahead (below) |
| §3.1 stop, snapshot | Keep `docker compose stop bookclub` in `$OLD` (stop, never down) and the `-journal` check. Drop the maintenance block, the checksums and the gpg copy: the stopped container and its untouched data directory are the rollback copy for 30 days |
| §3.2 transfer | `cp "$OLD/data/bookclub.db"` (and `-wal` if present) into `$IMPORT`, `chown 1000:1000`, `chmod 600` |
| §3.3 dry run, go/no-go, real run | As COVER leaves it (the update timer is not enabled yet). The go is CUT2's entity counts: users 2 → 2, 50 works, 36 editions, 146 events. Each run takes about 20 s plus about a minute of cover search (at most 5 minutes). `lookup wait` may be above 0 from the cover pass, and the `cover search` line and `without a cover` section may differ between the runs; neither is a no-go |
| §3.4 `compare.sh` | Unchanged; exit 0 is the go |
| §3.5 start and check | No gate. `docker compose up -d --wait app` (the Python container is stopped, so the port is free), `healthcheck`, `make-admin "$OWNER"`. Rafael signs in on `https://$HOST` with his old password and checks the club, the shelves, the layouts, the pick, the votes, the quotes and the feed, and opens an old `/works/OL…W` link. Then `rematch-legacy` if `isbn unmatched` or `isbn error` is not 0. Drop the email step. COVER's review of cover suggestions waits until after "done": a missing cover is not a no-go, and Rafael decides them later on Admin → Covers. `curl -sI https://$HOST/sw.js` shows JavaScript. `TRUSTED_PROXIES`: I run `deploy/hosted/verify-limits.sh https://$HOST` from the agents VM (it creates nothing and uses up only the VM's budgets) and must get its 429s; then Rafael signs in on his phone with Wi-Fi off and runs the phone test (§5.8). A 429 on the phone means `TRUSTED_PROXIES` or the proxy's `X-Forwarded-For` is wrong; I fix `.env` and restart the app. A scanner failure is no reason to roll back |
| §3.6 open | No Caddy step: the port takeover was the switch. Run `backup.sh` once, then Rafael sends "done" |
| §4 rollback | §5.7 |
| §5 afterwards | §5.8 |
| §6 preserving Python | Done (Q6 A) |

**The message**, a few days ahead (MOVE writes the final text in the runbook):
- DE: "Bookclub bekommt am <Tag> ab <Uhrzeit> ein großes Update, etwa eine Stunde Pause. Adresse, Benutzername und Passwort bleiben gleich, alles zieht mit. Danach einmal neu anmelden. Nicht abgeschickte Notizen bitte vorher abschicken. Die App fragt nach einer E-Mail-Adresse: eintragen oder überspringen, Mails verschickt sie vorerst keine. Passwort vergessen? Sag mir Bescheid. Neue Leute lade ab jetzt ich ein. Ich melde mich, wenn es fertig ist."
- EN: the same.
- "Done": "Fertig, Bookclub läuft wieder unter der gewohnten Adresse." / "Done, Bookclub is back at the usual address."

### 5.7 Rollback

The window lasts until the other member writes to the new app. No gate is needed: the other member waits for the "done" message.

1. In the new directory: `docker compose stop app`.
2. In `$OLD`: `docker compose start bookclub`, wait until it is healthy, and sign in once. If §5.4 needed a proxy change, Rafael reverts it.
3. In the new directory: `docker compose down -v` and empty the media directory, so the next attempt starts from an empty target.
4. Tell the other member the update is postponed, and record why in #52.

Python's `restart: unless-stopped` keeps a stopped container stopped after a reboot. Starting it while the new app holds the port fails with "port already allocated", which is safe.

### 5.8 Afterwards, and CUT-R

- **Move day, after the check (§5.6, the §3.5 row):** the phone test. **[Rafael]** on his iPhone with Wi-Fi off (and an Android phone with Chrome if one is at hand): open `https://$HOST`, sign in, scan a German book, an English book and one the app does not know, add a photo to the unknown one, and deny the camera once to see manual entry. An iPhone older than iOS 16 shows manual entry only. The result goes into `move.md`; a failure becomes a patch release.
- **Same day:**
  - `DNB_HARVEST=true` in `.env`, then `docker compose up -d app`.
  - Delete `$IMPORT/bookclub.db*` (it holds every password hash), `dry.txt` and `real.txt`, once the counts are in #52 (counts only).
- **Next day, or when the other member has used the app:** `systemctl enable --now bookclub-update.timer` (README, "Updates and rollback", "Setting up"). The rollback window has closed.
- **CUT-R:**
  - The branch `cutover/remove-python` is cut from `main` after v0.11.0. The deletes are exactly `docs/phase2/work-plan.md` §2.4.
  - Docs: the README describes the private invite-only instance and drops `README.md:162` (that closes #35's README criterion); `SELFHOST.md` goes; CLAUDE.md loses "In transition" and the Python commands; `docs/plan.md` gets a status line with the move's date.
  - **[Rafael]** approves the merge after the move is verified.
- **Issues:**
  - Close #52 with the counts, and #56 after the phone test.
  - Close #35: the password change is in Settings (`AccountSettings.vue`, `PATCH /api/auth/password`) and runs in production from the move on.
  - Close #34 as superseded, after adding "Web Push for the browser app" to #60 (D9).
  - #51 stays open for the public launch, with a comment on what the move delivered.
- **Last step of M5:** I delete my key from root's `authorized_keys` on the host.
- **After 30 days (Q14 A), M6:** **[Rafael]** pastes the runbook's cleanup block on the host. It deletes only when `journalctl -u bookclub-restore-test --since '8 days ago'` shows a passing run and `restic snapshots --latest 1` shows last night's snapshot; then `docker compose down` in `$OLD`, its `data/` and the Python image go. Otherwise it stops and deletes nothing.

### 5.9 What waits for the public launch, and why that is safe now

| Deferred | Why it is safe while sign-up is invite-only |
|---|---|
| Name and domain (#50, BRAND) | The members already use the current hostname; the redirect from it comes with the new name (`deploy/legacy-origin/`) |
| Legal texts, VVT signature, DPAs (#53) | No stranger can sign up; the startup refuses `SIGNUP_MODE=open` without the six files. With mail off, local backups and no external monitor, there is no processor |
| Mail provider (Q4 Brevo), mail DNS, `send-test-mail` | Nobody receives mail; `set-password` covers resets; `SIGNUP_MODE=open` is refused without mail |
| Storage Box, snapshots, offsite copy | Encrypted NAS or local backups with a weekly restore test. Losing the host's disk is the remaining risk, as with the Python app today; on a one-disk host without a NAS the backups share that disk (§5.5) |
| Own VM in its own VLAN, firewall rules | The exposure is the Python app's today: same host, same proxy, same port |
| Uptime monitor, outside check, heartbeats (unless Q-M2 names a monitor) | Two users notice an outage; the update stops on a failing backup. A failed backup stays silent until someone looks; M6 checks the backups before the old copy goes |
| HSTS, the `Caddyfile.example` site block, the GATE checks | The existing proxy terminates TLS. The in-app limits work once `TRUSTED_PROXIES` is right, which move day checks with `verify-limits.sh` (§5.6) |
| Open sign-up (#55), Q11 C (reports on texts with friends) | Invite-only; friends are mutual consent, and block and unfriend end access |

---

## 6. Answers (taken 2026-10-29)

Each decision with the alternative rejected.

**Order and process**
1. **Order:** COVER → Phase 3 → private move → public launch → Phase 4. Rejected: Phase 2's cutover first, on a new domain with mail and legal texts.
2. **FRIENDS is one package.** Rejected: a server package and an SPA package (the SPA's types come from the server's OpenAPI document).
3. **Packages merge locally into `phase-3`; one Forgejo PR `phase-3` → `main` after Wave 1.** Rafael approves the merge; the `v0.11.0` tag follows without a second approval. Rejected: a PR per package or per wave, and an approval per tag (more approvals for Rafael, nothing gained).
4. **The move ships as v0.11.0; v1.0.0 marks the public launch.** Rejected: v1.0.0 at the move (`cutover.md:43`, Q5).

**Scanning (#56)**

5. **Decoder:** native `BarcodeDetector` when it lists `ean_13`, else `zxing-wasm/reader` 3.1.6, self-served with a `locateFile` override. Rejected: the `barcode-detector` polyfill (an extra layer over the same wasm, fragile `?url` under hoisting); `@zxing/library` (pure JS, slower on 1-D codes, in maintenance mode); native only (no iOS).
6. **Decoding:** main thread, every 200 ms, the centre band, the first valid read ends it. Rejected: a worker (only if a phone shows jank); continuous scanning of a whole shelf (the native app, #60).
7. **CSP:** `'wasm-unsafe-eval'` in the one shell policy. Rejected: a policy only for `/scan` (one document, client-side routes); `'unsafe-eval'`.
8. **Permissions-Policy:** `camera=(self)`, still set by the proxy. Rejected: the app setting the header.
9. **Lookup:** `GET /api/books/isbn/{isbn}` under the per-account catalog turn, with no DTO change. Rejected: the search endpoint (its hits have no `edition_id`).
10. **ISBN paths are redacted** in the app log and the Caddy log. Rejected: logging them (they show what someone reads).
11. **Structure:** `BarcodeScanner.vue` plus a `/scan` page, entered from Discover. Rejected: a bottom-bar item; a modal without a route (Back would not close it); "Scan your copy" in the edition picker in v1 (the hit card already sets a shelved book's edition; a later idea).
12. **A miss without a work:** "Find the book by title" first, then "Add it as a new book". Rejected: always a new book (duplicate works).
13. **The photo comes after the save**, on the hit card, with `CoverPhotoButton`. Rejected: picking the photo inside the form before the save (more state, and a failed upload after a successful save).
14. **Manual entry is always shown**; it takes ISBN-10 and ISBN-13. The scanner takes EAN-13 978/979 only. ISMN `9790` simply misses. Misses are not cached; a save after a miss asks upstream again, accepted.
15. **Phone test:** on the real hostname on move day, signed in as Rafael (§5.8). Rejected: a local CA installed on the phones (a fully trusted CA whose key sits on the agents VM, and the most work for Rafael); Tailscale (not on the VM).

**Friends (#57)**

16. **Data:** one `friendships` row per pair (pending or accepted), a separate `user_blocks`, `club_invites.invitee_id` for friend invites. Rejected: two directed rows per friendship; a `blocked` state in the friendship row.
17. **A friend is `Audience`;** `bc_can_see_user` and `bc_visible_owners` replaced, `bc_friends` added. Rejected: a fourth `Relation`.
18. **Home feed:** friends' personal events from acceptance on. Rejected: no friends' events on Home.
19. **Decline deletes;** requests never expire; the sender can ask again. Rejected: a stored `declined` state.
20. **Mutual send becomes a friendship.**
21. **Block:** silent; ends the friendship; future requests stored and hidden; the pair's bound invites revoked; clubs untouched, now and later (a blocked account that joins one of the blocker's clubs is audience again; the copy says so); unblock drops what was hidden. Rejected: telling the blocked account, or refusing its requests with an error; blocks that keep people out of clubs (a club rule, not a friend rule).
22. **Friend invites:** club admins only; a bound single-use invite for 30 days; the friend joins through `/join/{code}`. `docs/plan.md:31` becomes "clubs they administer". Rejected: any member inviting (cheap to widen later); adding a friend directly without consent.
23. **Exact username only;** unknown names get 404 `No account with that name` on send and block only, both counted (20 a day; a block of someone who already asked is free). Unblock always answers 200. Rejected: the same answer for unknown and existing names on send (it would need fake pending rows; sign-up already reveals names).
24. **Friend lists are never served to others;** a bound invite names its invitee to its creator only; no counts, no cap.
25. **The badge comes from `GET /api/friends`** in a store. Rejected: a `UserOut` field (§0 frozen); mail or push for requests (no mail now; push is #60).
26. **Navigation:** a sidebar item and a compact Home row. Rejected: a sixth bottom tab; in v1, a "Friends are reading" strip on Home, friend actions on member pages and inviting from a club's members page (later ideas).
27. **Suspension changes nothing about friends:** friendships, requests and bound invites stay (ADM2's rule: a suspension leaves content as it is). Rejected: hiding a suspended account's requests and revoking invites bound to it (it cannot sign in, and only it can redeem them).
28. **Removal at `DELETE /api/friends/list/{username}`.** Rejected: `/api/friends/{username}` (a user named `requests` or `blocks` would collide).
29. **Q11 C** (reports on texts) stays with #55.

**The private move**

30. **Same host, same hostname, same proxy; switch by port takeover,** or by the old network alias when the old app publishes no port. Rejected: a new VM and domain now; changing the proxy's upstream (Rafael's work, and a second rollback step).
31. **Mail off** with `EMAIL_TRANSPORT=none` and `set-password`, and open sign-up refused without mail. Rejected: an app password on Rafael's personal mail account (it opens the whole mailbox, sits in a `.env`, and every mail would link legal pages that say "missing"); a dummy loopback `SMTP_URL` (works by accident, a WARN per mail); Brevo now (an account, DNS and a DPA).
32. **Legal:** an empty `LEGAL_DIR`. Rejected: placeholder texts in production (refused at startup).
33. **Backups:** restic to a NAS share, else another disk, else the same disk (Rafael is told); nightly backup and weekly restore test; heartbeats only to a monitor Rafael already runs (Q-M2). M6 deletes the old copy only after a passing restore test. Rejected: the Storage Box now (an account and a DPA); a monthly manual check for Rafael.
34. **Updates:** the update timer on `stable`, from the day after the move. Rejected: manual deploys.
35. **No external monitor.**
36. **No gate:** the other member waits for "done". Rejected: Caddy's gate from `cutover.md` 3.5 (a proxy edit for two users).
37. **No gpg snapshot:** the stopped container and its data directory are the rollback copy for 30 days.
38. **The app serves the worker kill switch at `/sw.js`.**
39. **Imported users keep the email prompt** and may skip it. Rejected: hiding mail texts through a `/api/config` flag (DTO, `generated.ts` and i18n changes for two users).
40. **Club:** "Bookclub", slug `bookclub-8rqp`. If the old `BOOKCLUB_NAME` differs, I generate a new slug by D5 from the host facts; no question to Rafael.
41. **CUT-R merges after the move is verified.**
42. **Compose project `bookclub`,** or `bookclub-server` when the old project has that name.
43. **Issues:** close #35 and #34 at the move (§5.8); #51 stays open for the public launch.
44. **MOVE's code (M1) is a Wave 1 package;** Wave 2 holds only operations. Rejected: M1 after Wave 1 (it waits for nothing there).
45. **COVER's review of cover suggestions waits until after "done".** Rejected: a review before "done" (more for Rafael on move day; a missing cover is not a no-go).
46. **Secrets never pass through my terminal** (§5.6): the restic password is generated on the host, Rafael runs `docker login`, `set-password` writes to a root-only file. Rejected: Rafael generating the password and writing it to the host himself (one more step, no gain).
47. **What changes for the members:** everyone signs in again once; resets go through Rafael (`set-password`); only club admins invite (Rafael, or whom he makes an admin on the members page); no new club until mail exists. Rejected: making the other member a club admin at import (Rafael can, in two taps, if he wants).
48. **SSH:** a key made for the move, limited to the agents VM with `from=`, removed as the last step of M5; M6 is Rafael's copy-paste block. Rejected: the VM's everyday key until M6.
49. **`TRUSTED_PROXIES` is checked on move day** from two sides: `verify-limits.sh` from the agents VM, then Rafael's phone sign-in on mobile data.
50. **No auto-updater touches the new stack;** `update.sh`, which backs up first, is its only updater.

---

## 7. Questions for Rafael

Only facts about his hosts, and one preference. "Take the recommendations" answers all of them.

**Q-M1. May I log in to the production host over SSH for the move, and which address do your members open?**
- The address is needed only if the old `.env` does not name it (`BOOKCLUB_PUBLIC_URL`).
- **Recommendation:** yes. I make a key just for the move and send you one line to add to root's `authorized_keys` on the host. It works only from the agents VM (`from=`), and I remove it myself when the move is done, a few days later. Then I read the facts, install the stack and run the move day with you watching. The merges still wait for your approval.

**Q-M2. Does your homelab already run a monitor that takes healthchecks.io-style pings (a self-hosted healthchecks, for example)?**
- **Recommendation:** if yes, give me three ping URLs (backup, restore test, update) and I set them. If not, nothing: a failed backup then shows only when someone looks, until the public launch brings a monitor.

**Q-M3. Which email address may show on the sign-in page?** (`CONTACT_EMAIL`; it is public in `/api/config` and goes into the User-Agent that Open Library and the DNB see.)
- **Recommendation:** the email address of your Forgejo account (I read it there). Name another if you prefer.

---

## 8. What Rafael does

**Wave 1:** nothing.

**The move:**
1. Answer Q-M1 to Q-M3 (or "take the recommendations"), and add the line I send to root's `authorized_keys` on the host.
2. Only if I report that the host is not logged in to the registry: create a Forgejo token with only `read:package`, and run `docker login git.tillhome.de` with it as root on the host.
3. When I say the backups are set up: on the host, as root, `cat` the password file I name (normally `/opt/bookclub/restic-password`) and save it in your password manager.
4. Approve the merge into `main`. The release follows without another approval.
5. Only if I report that your proxy blocks the camera: change `camera=()` to `camera=(self)` in its `Permissions-Policy` line (I send you the exact line).
6. Pick a time and send the other member the short message from the runbook.
7. On move day: sign in with your old password, look at your shelf and the club, and tell me it looks right. Then, on your iPhone with Wi-Fi off: sign in, scan a German book, an English book and one the app does not know, add a photo to the unknown one, and deny the camera once. Then, when I say the backup ran, send the other member the "done" message.
8. Approve the merge that removes the Python code.
9. After 30 days: paste the cleanup block from the runbook on the host. It checks the backups first and deletes nothing if they fail.

---

## 9. Later, before the public launch

Before any stranger can sign up, and before Phase 4 (the App Store needs a privacy policy). `docs/phase2/work-plan.md` §5 holds the details. These items wait until after the private move:
- **#50, BRAND:** the name (Q1, `docs/phase2/name-check.md`), the domain and DNS (§5 items 6–7), and BRAND's files (§0). The current hostname then redirects through `deploy/legacy-origin/` (CUT's block, with its `/sw.js`).
- **Mail:** Brevo (Q4): the account, the DPA, SPF, DKIM and DMARC, `EMAIL_TRANSPORT=smtp`, and `send-test-mail` to three providers (items 8, 23). The SPA's "link sent" copy becomes true then.
- **#53:** the contact mailbox and Impressum address (item 9), the legal texts, the VVT signature and the DPAs (items 17–19). The privacy text also names the camera (on-device decoding) and the redacted ISBN paths, both from SCAN's notes, and friendships.
- **#51, the rest:** its own VM in its own VLAN (item 10), the Storage Box with snapshots (item 12), the monitor, heartbeats and the outside check (item 13), the Caddy site block from `Caddyfile.example` with HSTS (item 14), and the alert test (item 16).
- **#55:** `verify-limits.sh` from outside, a takedown run, and `SIGNUP_MODE=open` (items 24–26); Q11 C, reports on texts, now that friends exist; a cap on pending incoming friend requests per account (or their expiry) and a way for admins to see request spam.
- **The GATE checks** against the real stack, and the first `v1.0.0`.
