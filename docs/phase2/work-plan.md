# Phase 2 work plan: waves and work packages

This plan covers Phase 2 (#50–#55) and the last open item of #38. The contracts are the same as in Phase 1:

- `docs/plan.md`: the product decisions
- `docs/phase1/conventions.md`: architecture and coding rules
- `docs/phase1/endpoint-map.md`: routes, owners, DTOs and error strings
- `server/migrations/*.sql`: the schema

Each package writes its contract changes into its own notes file, `docs/phase2/notes/<pkg>.md`. After the merge, the integrator moves them into `conventions.md`, `endpoint-map.md`, `CLAUDE.md` and `README.md`.

In this plan, "I" means the implementing agent: it has the repo and a shell on the `agents` VM, but no access to Proxmox, Caddy, DNS or registrars. **[Rafael]** marks what only the owner can decide or do.

## 0. Amendments from the review (these override the sections below)

A completeness review checked this plan against the code at 84006bd. Where this list and a later section disagree, this list wins.

**Ground rules**

1. **Generated files.** Each package regenerates and commits `server/.sqlx/` (`cargo sqlx prepare -- --all-targets`) and `frontend/src/api/generated.ts` (`npm run gen:api`) itself, so its own checks pass. The integrator regenerates them again after a merge only when they conflict. This replaces the "nobody by hand" row in §2.1 rule 2.
2. **`endpoint-map.md` §0.** `server/tests/openapi_contract.rs::section_0_objects_match_the_document` requires every §0 DTO to have exactly the fields that `docs/phase1/endpoint-map.md` §0 lists. A package that adds a field to a §0 DTO (for example LEG's `UserOut.needs_terms`) adds the matching line to §0 in the same commit. It only adds lines. Everything else in `endpoint-map.md` and `conventions.md` stays with the integrator.
3. **Shared, add-only.** `server/tests/common/**` is shared by ADM, LEG and GATE: they add helpers, and any change to an existing helper is recorded in their notes. `server/src/catalog/shared/ingest.rs` stays frozen (conventions §7): ADM calls `apply_admin_fields` and `clear_media`, and anything it needs changed there goes to the integrator as `blocked_on_frozen`.
4. **Branches.** In Wave 1 the packages merge locally into `phase-2`, one at a time, as in Phase 1. At the end of the wave, one Forgejo PR merges `phase-2` into `main`. Its CI run is the first run of DEP's image smoke job on the runner. This replaces the per-package PRs in §2.1 rule 1.
5. **Browser checks** use the `playwright-cli` skill or `npx` outside the repo. Nothing is added to `frontend/package.json`.

**DEP**

6. The `send-test-mail` message is built in `server/src/main.rs` or `server/src/email/mod.rs`, never in `templates.rs` (ADM's). `healthcheck` is `mod healthcheck;` in `main.rs` (the binary crate), so DEP does not need `lib.rs`.
7. The compose port is `${BOOKCLUB_LISTEN:-0.0.0.0}:${BOOKCLUB_PORT:-8080}:8080`. `${BOOKCLUB_LISTEN:-0.0.0.0}:8080` would publish container port 8080 on a random host port.
8. `deploy/hosted/README.md` creates the bind-mount directories owned by uid 1000 (`install -d -o 1000 -g 1000 /srv/bookclub/media /srv/bookclub/legal`). The image's `chown` does not apply to bind mounts.
9. Outside `DEV_MODE` the server does not start without `SMTP_URL` and `EMAIL_FROM` (`server/src/config.rs:167-188`), and `EMAIL_TRANSPORT=log` needs `DEV_MODE`. The README says that staging needs an SMTP account: an interim one until Q4 is answered.
10. `env.example` includes `SERVICE_NAME` (`server/src/config.rs:232`). `deploy_env.rs` enforces this anyway.

**GATE**

11. The CSP adds `manifest-src 'self' blob:`. `frontend/src/brand.ts:31` sets a `blob:` manifest, which `default-src 'self'` would block.
12. The header layer in `server/src/lib.rs:214-217` uses `SetResponseHeaderLayer::overriding`, which would replace the CSP that `spa.rs` sets on the shell. GATE's hunk in `lib.rs` switches that layer to `if_not_present`.
13. The app answers 413 only when `Content-Length` is above 52,432,896 bytes (`server/src/media/upload.rs`). Caddy's `max_size 51MiB` is 53,477,376 bytes. The end-to-end upload therefore uses a body between those two sizes, so that the app answers, not Caddy. A 51 MB body (51,000,000 bytes) gets no 413 at all.
14. `frontend/src/thirdParty.test.ts` checks sources only: `index.html`, `public/manifest.webmanifest`, and `src/**/*.{css,vue,ts}` for external `url(…)`, `@import` and `<link>` origins. CI runs `npm test` before `npm run build`, so the test cannot read `dist/`.
15. Search is limited per account in the app (the `catalog_user` limit, `server/src/ratelimit.rs:287`, used in `server/src/catalog/search.rs`). `verify-limits.sh` does not cover search. The amendment to #51 (Q16) says so.
16. GATE's instructions for Caddy and `verify-limits.sh` go into `docs/phase2/notes/gate.md`. The integrator moves them into `deploy/hosted/README.md`, which DEP owns.

**LEG, ADM, ADM2, LEG2**

17. LEG needs no code in `server/src/auth/export.rs`: the profile section is `to_jsonb(u) - 'password_hash'` (`export.rs:151-160`), so new `users` columns are exported automatically. LEG adds only a test assertion, and `export.rs` leaves LEG's shared list.
18. LEG's rule "refuse open sign-up without the legal texts" lives in `config.rs` or `legal.rs`, not in `main.rs` (DEP's).
19. ADM2 also owns `server/tests/auth_export.rs`: any new foreign key to `users` changes its pinned table count (`in_schema.len() == 28`, `:204`).
20. A suspended account cannot log in, so it loses in-app export and deletion (DSGVO Art. 15, 17 and 20). ADM2 documents a manual path through `CONTACT_EMAIL` in `docs/legal/takedown.md`, and the privacy facts state it.
21. LEG2 owns `server/src/auth/mod.rs` (`cleanup`), `server/tests/jobs.rs` and `docs/legal/**`, plus `server/src/auth/deletion.rs` if the purge of empty sign-ups reuses it.

**New Wave 2 task: BRAND (after Q1).** The chosen name goes into:
- `frontend/index.html`
- `frontend/public/manifest.webmanifest` (its description, "A tiny private bookclub…", is also out of date)
- `frontend/src/brand.ts`
- the strings in `en.json` and `de.json`
- `SERVICE_NAME` in `deploy/hosted/env.example`

**Facts settled since the draft**

22. The registry already holds `rafiistcool/bookclub:{0.10.0,0.10,latest}`, pushed on 2026-10-18 but not by a Forgejo run. The v0.10.0 compose pulls `ghcr.io/rafiistcool/bookclub:latest`. Q5's recommendation (a new image name) therefore stands.
23. The repo has no `REGISTRY_TOKEN` action secret, though `release.yml` logs in with it. It may be set at user level. This is a question for Rafael before the first release.
24. Whether the self-hosted runner can `docker build` inside a job is still unknown. The Wave 1 PR's CI run answers it.
25. Q6 (the `legacy-python` branch and tag) waits for Rafael's answer. CUT puts the commands into the runbook and pushes nothing.
26. Corrections:
    - `server/src/ratelimit.rs:273-302` are the thresholds; the limit tests are in `server/tests/auth_*.rs`.
    - The `ci.yml` server job is lines 52-108.
    - The staging deploy in §3 depends on A7, A8 (or an interim SMTP account) and A10–A15, not on A6.

---

## 1. Status

Phase 1 is merged into `main` as 84006bd (PR #70). On Forgejo, #36, #37 and #39–#49 are closed. #38 stays open for one item: the dry run and the real run on a copy of the production database. Production still runs the Python v0.10.0 image.

Phase 1 already delivered part of Phase 2:

- **#54 (admin queue), most of it:**
  - the instance-admin flag (`server/migrations/0001_init.sql:79`) and the `make-admin` CLI (`server/src/main.rs`)
  - one `InstanceAdmin` route layer over every `/api/admin/*` route (`server/src/lib.rs:136-160`), with a 404 walk test (`server/tests/admin_guard.rs`)
  - nine admin operations: sign-up invites, edit, merge and cover-replace requests, and cover reports and takedowns. A takedown blocks the picture's hashes from coming back (`server/src/admin/invites.rs`, `server/src/catalog/admin.rs`, `server/src/media/reports.rs`, `server/migrations/1300_media_removed_hashes.sql`).
  - an `/admin` page with requests, reports and invites tabs (`frontend/src/pages/AdminPage.vue`)
- **#55 (open sign-up):** `SIGNUP_MODE=invite|open` works end to end (`server/src/config.rs:136-140`, `server/src/auth/signup.rs:117-122`). The in-app limits on login, sign-up, reset and search are tested (`server/src/ratelimit.rs:273-302`).
- **#51 (deployment):**
  - a server image that holds only the binary (`server/Dockerfile`)
  - `GET /api/health` (`server/src/health.rs`)
  - production config validation (`server/src/config.rs:113-262`)
  - migrations at startup and a graceful shutdown (`server/src/main.rs:81-154`)
- **#52 (cutover):**
  - the importer is complete (`server/src/importer/`)
  - old URLs resolve (`frontend/src/router.ts`, `legacy_work_keys`)
  - imported users get the email prompt (`frontend/src/components/EmailPrompt.vue`)
  - the rehearsal procedure is drafted (`docs/phase1/notes/imp.md:209-246`)
- **#53 (legal texts):** no code beyond the cover report button. The facts its privacy text needs are favourable: the browser contacts no third party, the app log holds no IP addresses, and export and deletion exist.
- **#50 (name and domain):** not started.

Two things stop any release from `main` today:

- The root `Dockerfile` still builds the Python image and cannot build: `Dockerfile:28` copies `backend/app/static`, which Vite no longer writes (`README.md:51` says so).
- `.forgejo/workflows/release.yml` still runs pytest and builds that Dockerfile.

---

## 2. Packages

```
Wave 1  DEP #51 ─ GATE #55 ─ ADM #54 ─ LEG #53 ─ CUT #52/#38            parallel, start now
          │  (Wave 1 merged, phase-2 → main; Rafael's answers to §4)
Wave 2  DEP2 #51 ─ ADM2 #54/#55 ─ LEG2 #53 ─ CUT2 #38                     parallel
          │  (Rafael: domain, mail, VM, legal texts live)
Cutover CUT-R: branch cutover/remove-python, merged only on cutover day   (#52)
          │
#55     the SIGNUP_MODE=open switch (Rafael), once every gate is green
```

### 2.1 Ground rules for parallel work

1. **Branches.**
   - The integration branch is `phase-2`, cut from `main` at 84006bd.
   - Each package gets a worktree next to the repo, never under `/tmp`: `git worktree add ../bookclub-<pkg> -b phase2/<pkg> phase-2`.
   - Each package opens its PR into `phase-2` on Forgejo, with the `forgejo` MCP tools, never on GitHub.
   - At the end of each wave, one PR merges `phase-2` into `main`, after Forgejo CI is green.
   - CUT-R is the only exception (§2.4).

2. **Ownership.** Every file belongs to one package, as listed below. A package edits only its own files. Shared files are listed per package, and only the integrator merges them:

   | File | Edited by | Rule |
   |---|---|---|
   | `server/src/lib.rs` | ADM (the `admin_router` merge line), LEG (the `legal::routes()` line), GATE (the header layer at `:208-224`) | Separate hunks |
   | `server/src/auth/export.rs` | LEG (the terms columns in the account section), ADM2 (the reports section at `:467-524`) | Separate hunks |
   | `frontend/src/pages/SettingsPage.vue` | ADM (the admin link at `:238-244` only), LEG (a new Legal section at the end) | Separate hunks |
   | `frontend/src/api/client.ts`, `server/src/error.rs` (`msg` constants), `frontend/src/i18n/apiErrors.ts` | ADM, LEG | Add lines only |
   | `frontend/src/i18n/{en,de}.json` | ADM (`admin.*`, `cover.*`), LEG (new top-level `legal`, `terms`) | The integrator resolves the textual conflicts; `messages.test.ts` checks key parity |
   | `frontend/src/api/generated.ts`, `server/.sqlx/` | nobody by hand | The integrator regenerates them after each merge |
   | `docs/phase1/{conventions,endpoint-map}.md`, `CLAUDE.md`, `README.md`, `docs/plan.md` | integrator | Applied from the package notes |

3. **Crates.** I expect no new crate:
   - `healthcheck` uses reqwest, which is already a dependency.
   - LEG serves HTML fragments as the generator exports them, so it needs no Markdown renderer.

   If a crate is needed after all, the integrator adds it on `phase-2`.

4. **Migrations.**
   - Reserved blocks: ADM and ADM2 use 2100–2199, LEG and LEG2 use 2200–2299, the integrator uses 2900–2999.
   - There is no squash (default D7).
   - A migration merged into `phase-2` is never edited afterwards.
   - A new foreign key to `users` needs a section in the export, or the `pg_constraint` completeness check in `server/tests/auth_export.rs` fails (conventions.md §5.7).

5. **New paths.** Each new path belongs to one package:

   | Path | Package |
   |---|---|
   | `PATCH /api/admin/works/{work_id}`, `PATCH /api/admin/editions/{edition_id}`, `GET /api/admin/summary`, `?status=&page=` on `GET /api/admin/reports` and `GET /api/admin/edit-requests`, an optional body on `DELETE /api/admin/media/{public_id}` | ADM |
   | `POST /api/books/works/{work_id}/reports`, `POST /api/books/editions/{edition_id}/reports`, `POST /api/admin/users/{username}/suspend` and `/unsuspend`; avatars go through the existing `POST /api/media/{public_id}/reports` | ADM2 |
   | `GET /api/legal/{doc}` (public), `POST /api/auth/terms`, the `/api/config` fields `legal`, `terms_version` and `contact_email`; SPA routes `/imprint`, `/privacy`, `/terms` | LEG |
   | CLI `bookclub-server healthcheck` and `bookclub-server send-test-mail <address>` | DEP |

6. **Worktrees, databases and build resources.**
   - Each package runs its own Postgres server: `docker run -d --name bookclub-pg-<pkg> -p 127.0.0.1:<port>:5432 … postgres:17 -c max_connections=200 -c shared_buffers=64MB`.
   - Ports: ADM 5451, LEG 5452, DEP 5453, GATE 5454, CUT 5455, the CUT2 rehearsal 5456. The integrator uses the compose server on 5432.
   - Each package that checks pages in a browser runs the built SPA from the server (`STATIC_DIR=../frontend/dist`) with `BIND_ADDR=0.0.0.0:<port> PUBLIC_URL=http://agents.lan:<port>`. Ports: ADM 8083, LEG 8084, CUT 8085, the DEP staging stack 8081, the GATE Caddy stack 8082, the CUT2 rehearsal 8080.
   - Copy the main checkout's untracked `.cargo/config.toml` (`jobs = 3`, debug info off) into each worktree and change its `DATABASE_URL`. Also export `DATABASE_URL` for `cargo sqlx`.
   - At most 5 worktrees build at once (the VM has 12 CPUs, 17 GiB RAM and 113 GB free disk today).
   - DEP's `docker build`s count as one build slot. Run `docker builder prune -f` after its image tests.
   - Before starting, run `docker ps -a --filter name=bookclub-pg-` and remove any Phase 1 containers still there.

7. **Done** means all of these pass in the worktree:
   - Server: `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked && cargo sqlx prepare --check -- --all-targets`
   - Frontend: `npm run typecheck && npm test && npm run build && npm run check:api`
   - Package extras where they apply:
     - shellcheck (`koalaman/shellcheck:v0.10.0`)
     - actionlint (`rhysd/actionlint:1.7.7`)
     - `caddy validate` (`caddy:2.10`)
     - `docker compose … config -q`

8. **Integration.** The integrator merges one PR at a time. After each merge it:
   1. runs `cargo sqlx prepare -- --all-targets`
   2. runs `npm run gen:api`
   3. runs the full suite
   4. applies the package's notes to the docs

9. **Real data.** Production data never enters the repo, a Forgejo comment or a committed log. #38 gets counts only. The rehearsal copy lives outside the repo and is deleted afterwards.

10. **`backend/`.** Nothing on `main` touches `backend/` until CUT-R merges, and the Python CI job (`.forgejo/workflows/ci.yml:14-29`) keeps running until then.

### 2.2 Wave 1: packages I can build now

All five run in parallel. They need no answer from Rafael. Where an answer is still open, they build the recommended option, and the alternative changes only a path or a constant. The suggested merge order is **DEP, CUT, LEG, ADM, GATE**:

- DEP goes first, because GATE's end-to-end checks and CUT's runbook use its image and compose file.
- GATE goes last, so its CSP browser run covers LEG's and ADM's new pages.

#### DEP: production image, compose, release and backups (#51)

- **Size:** L. **Dependencies:** none.
- **Soft blocks:** Q5 (image name) and Q6 (Dockerfile location). If Rafael picks other options, only names and paths change.
- **Scope:**
  - **Production image at the repo root.** It replaces the Python Dockerfile, which cannot build on `main` anyway (Q6; the alternative location is `deploy/hosted/Dockerfile`).
    - Stages: `node:22` runs `npm ci` and `vite build`. `rust:1.99-slim-trixie` runs `cargo build --release --locked` with `SQLX_OFFLINE=true`. The runtime is `debian:trixie-slim` with `ca-certificates`.
    - The runtime copies the binary and `frontend/dist` to `/app/static`, and sets `STATIC_DIR=/app/static`, `MEDIA_DIR=/data/media` and `BIND_ADDR=0.0.0.0:8080`.
    - It creates `/data/media` owned by uid 1000. Without that, the default `./data/media` would resolve to `/data/media`, which uid 1000 cannot create (`server/Dockerfile` sets no `WORKDIR`).
    - `HEALTHCHECK ["bookclub-server","healthcheck"]`, and OCI labels that point at git.tillhome.de.
  - **Root `.dockerignore`:** exclude `server/target/` (17 GB on this VM), `.cargo/`, `backend/`, `frontend/node_modules`, `frontend/dist` and `.git`. The current file excludes neither `server/target/` nor `.cargo/`.
  - **`bookclub-server healthcheck`** (`server/src/healthcheck.rs`):
    - it derives the local URL from `BIND_ADDR` (`0.0.0.0` → `127.0.0.1`, `[::]` → `[::1]`)
    - it GETs `/api/health` with reqwest and a 3 s timeout, and exits 0 only on 200
    - it never calls `Config::from_env` and never connects to the database
  - **Startup probe for `MEDIA_DIR`:** `serve` and `import-legacy` create the directory, write and remove a probe file, and fail with `MEDIA_DIR <path> is not writable`. Today `FsMediaStore` creates directories lazily (`server/src/media/shared/store.rs:58`), so a wrong mount shows up only at the first upload.
  - **`bookclub-server send-test-mail <address>`:** sends one mail through the configured transport and exits non-zero on an SMTP error. Also set an explicit `Message-ID` of `<uuid@EMAIL_FROM domain>` (`server/src/email/mod.rs:42-84`); lettre's default uses the hostname, which inside a container is the container id.
  - **`deploy/hosted/compose.yaml`:**
    - `postgres:17` with no published port, a data volume, a `pg_isready` healthcheck and `restart: unless-stopped`
    - an `app` service from `git.tillhome.de/rafiistcool/bookclub-server:${BOOKCLUB_TAG:-stable}`, with `env_file` and `depends_on: service_healthy`
    - the app's port is `${BOOKCLUB_LISTEN:-0.0.0.0}:8080`
    - bind mounts `/srv/bookclub/media:/data/media` and `/srv/bookclub/legal:/app/legal:ro`
    - `read_only` with a `/tmp` tmpfs, `cap_drop: ALL`, `no-new-privileges`, json-file log rotation and `stop_grace_period: 30s`
  - **`deploy/hosted/env.example`:** every variable of conventions.md §11, including `LEGAL_DIR=/app/legal` (LEG adds that variable; the name is fixed here so `deploy_env.rs` passes after LEG merges). It also includes `POSTGRES_PASSWORD`, and `DNB_HARVEST=false` until after the import.
  - **`server/tests/deploy_env.rs`:** fails when a variable that `server/src/config.rs` reads is missing from `env.example`.
  - **Release workflow rewrite** (`.forgejo/workflows/release.yml`, mirrored to `.github/workflows/release.yml`):
    - a frontend job, and a server job copied from `ci.yml:51-101`
    - the publish job pushes to `git.tillhome.de/rafiistcool/bookclub-server` with the tags `{{version}}` and `{{major}}.{{minor}}`, plus `stable` for non-prerelease versions
    - `flavor: latest=false`
    - registry build cache
    - drop the Python job and fix the stale GHCR comment (`release.yml:4`)
  - **CI image smoke job** (`.forgejo/workflows/ci.yml`, mirrored):
    - builds the root Dockerfile without pushing
    - boots it with `deploy/hosted/compose.yaml` and a new `deploy/hosted/ci.env`
    - curls `/api/health` and `/` from the compose network
  - **Backups:**
    - `deploy/hosted/backup.sh`:
      1. `pg_dump -Fc --no-owner` inside the postgres container, then `counts.tsv`
      2. then the media, so every key the dump references exists
      3. `restic backup`, then `forget --keep-daily 7 --keep-weekly 4 --keep-monthly 12 --prune`
      4. a weekly `restic check --read-data-subset=5%`
      5. a heartbeat ping on success or failure
    - `deploy/hosted/restore-test.sh`: restores into a temp dir and a throwaway `postgres:17`, runs `pg_restore --exit-on-error`, compares the counts and the migration version, and spot-checks the `media.storage_key` files.
    - `backup.env.example`, plus systemd units and timers.
  - **`deploy/hosted/README.md`:** the operator runbook:
    - the first boot
    - `docker compose exec app bookclub-server beta-invite` and `make-admin`
    - the import through `docker compose run --rm -v …:/import:ro app bookclub-server import-legacy …`
    - backup and restore
    - rollback by pinning `BOOKCLUB_TAG`
    - the switch to `SIGNUP_MODE=open`
- **Owns:**
  - `Dockerfile`, `.dockerignore`, `server/Dockerfile` (deleted or kept for server-only builds), `server/.dockerignore`
  - `deploy/hosted/**` except `Caddyfile.example` and `verify-limits.sh`
  - `.forgejo/workflows/{ci,release}.yml`, `.github/workflows/{ci,release}.yml`
  - `server/src/main.rs`, `server/src/healthcheck.rs`, `server/src/media/shared/store.rs`, `server/src/email/mod.rs`
  - `server/tests/deploy_env.rs`
- **Shared:** none. The pointers in `CLAUDE.md` and `README.md:51` go through the integrator.
- **Acceptance:**
  - Image basics:
    - `docker build -t bookclub:local .` keeps the build context under about 300 MB.
    - `docker run --rm bookclub:local bookclub-server openapi` works without a database.
  - Boot test against a throwaway `postgres:17` on 127.0.0.1:5450, with the image run `--read-only`:
    - `curl -fsS http://agents.lan:8081/api/health` answers `{"status":"ok"}`.
    - `/` is `text/html` with `Cache-Control: no-store`, and `/assets/*.js` is immutable.
    - `docker inspect -f '{{.State.Health.Status}}'` says `healthy`.
    - After stopping Postgres, the container turns unhealthy within about 90 s.
    - `beta-invite`, a sign-up and an avatar upload work in the browser.
    - A root-owned media bind mount exits non-zero with the `MEDIA_DIR` message.
  - Compose and env file:
    - `docker compose -f deploy/hosted/compose.yaml --env-file deploy/hosted/env.example config -q` passes.
    - `cargo test --test deploy_env` passes.
    - `docker compose -p bookclub-staging up -d --wait` exits 0, then `down -v`.
  - Workflows: actionlint is clean, and the smoke job is green on a Forgejo PR and red on a deliberately broken Dockerfile.
  - Backups: `backup.sh` and `restore-test.sh` exit 0 against the staging stack with a local restic repo outside the repo, using `restic/restic:0.17.3`. A truncated dump or a deleted media file makes `restore-test.sh` fail. shellcheck is clean.
  - `send-test-mail` to Mailpit (127.0.0.1:1025) shows Date, Message-ID and From at `http://agents.lan:8025`.
  - Every command in `deploy/hosted/README.md` runs as written against `bookclub-staging`.

#### GATE: hardening before strangers sign up (#55, part of #51)

- **Size:** M. **Dependencies:** none for the Rust and frontend parts. The end-to-end checks need DEP merged, so GATE merges last in Wave 1.
- **Scope:**
  - **A CSP for the SPA shell:**
    - Move the inline theme script (`frontend/index.html:15-43`) to `frontend/public/theme-init.js`, loaded synchronously in `<head>`.
    - `server/src/spa.rs` sets `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'` on the shell.
    - API responses keep `frame-ancestors 'none'` (`server/src/lib.rs:217`).
    - Confirm that vue-i18n needs no `'unsafe-eval'`.
  - **`frontend/src/thirdParty.test.ts`:** `index.html`, `manifest.webmanifest` and the built CSS reference no external origin. This enforces the privacy text's claim that the browser contacts no third party.
  - **`server/tests/proxy_client_ip.rs`:**
    - with a trusted peer, sign-up, login and reset count per `X-Forwarded-For` client (A gets 429, B does not)
    - `X-Forwarded-For` from an untrusted peer counts against the peer
    - with two trusted hops, the right-most untrusted address is the client
    - today only unit tests cover this (`server/src/client_ip.rs:94-140`)
  - **`deploy/hosted/Caddyfile.example`:**
    - `request_body max_size 51MiB`, because Caddy's `50MB` is below the app's 52,432,896 B
    - HSTS (no `includeSubDomains` until the final domain), `Permissions-Policy`, `-Server`
    - `reverse_proxy` with `health_uri /api/health`
    - a log with `roll_keep_for 168h`
    - a commented `caddy-ratelimit` block for Q16
    - it does not repeat the headers the app already sets
  - **`deploy/hosted/verify-limits.sh <base-url>`:** checks the thresholds without creating anything: register 5×400 then 429, unknown-name login 429 on the 11th, IP login 429 on the 31st, reset 5×204 then 429.
  - **Login lockout record:** add the outcome of Q17 to `docs/phase1/security-review.md` §3 (`:72-83`). The code changes only if Rafael picks per-address counting, and then in ADM2.
- **Owns:** `server/src/spa.rs`, `server/tests/platform.rs`, `server/tests/proxy_client_ip.rs`, `frontend/index.html`, `frontend/public/theme-init.js`, `frontend/src/thirdParty.test.ts`, `deploy/hosted/Caddyfile.example`, `deploy/hosted/verify-limits.sh`, `docs/phase1/security-review.md`.
- **Shared:** `server/src/lib.rs` (the header layer).
- **Acceptance:**
  - `cargo test --test platform --test proxy_client_ip` pass. A mutation that swaps `ClientIp` for the raw `ConnectInfo` peer makes them fail.
  - `docker run --rm -v $PWD/deploy/hosted:/etc/caddy:ro -e BOOKCLUB_HOST=bookclub.test caddy:2.10 caddy validate --config /etc/caddy/Caddyfile.example --adapter caddyfile` passes.
  - Local end to end on `agents.lan:8082`: Caddy with `tls internal` in front of DEP's staging stack, and `TRUSTED_PROXIES` set to the Caddy container's IP/32.
    - HSTS and the app's headers are present.
    - A 51 MB upload gets the app's 413, not Caddy's.
    - Two client containers each get their own sign-up budget.
    - `verify-limits.sh` exits 0 there and non-zero against `python3 -m http.server`.
  - A Playwright run against the built SPA covers login, search, a book page, an avatar upload, a diary note, `/imprint` and `/admin`, with no CSP violations, and the theme applies before first paint.

#### ADM: finish the admin queue (#54)

- **Size:** L. **Dependencies:** none. The constants for Q12 (72 h) and D1 (notification cadence) can change later.
- **Scope**, in this order:
  1. **A stricter guard test.** Today `admin_guard.rs` fills path ids with `1` and `x` (`server/tests/common/routes.rs:54-60`), so for approve, reject and the media DELETE it cannot tell a guarded route from an unguarded one.
     - Seed an open edit request, an open report and a live media row, and use their real ids.
     - Assert the full list of admin operations.
     - Assert that the seeded rows are unchanged after the non-admin walk.
  2. **Three small queue defects:**
     - A takedown now rejects the open `cover_replace` requests for that picture, with a note. Today, approving one later fails with 400 `This picture was removed` (`catalog/admin.rs:280`).
     - Repeat reports of the same picture by the same user, and reports of a picture already removed, are answered idempotently (`reports.rs:148-151`).
     - "dismiss" resolves every open report of the picture, not just one (`reports.rs:262-273`).
  3. **Current values on edit requests:** `EditRequestOut.current` holds the record's present value for each proposed field, and the cards show before and after.
  4. **Direct admin edit:** `PATCH /api/admin/works/{work_id}` and `PATCH /api/admin/editions/{edition_id}` apply fields through `ingest::apply_admin_fields`. An "Edit as admin" sheet opens from the queue and from `BookDetailPage.vue`.
  5. **A record for direct takedowns:** an optional body `{details, received_at}` on `DELETE /api/admin/media/{public_id}` writes a resolved `removed` report row. Today a direct takedown leaves no record; `media` has only `removed_at`. Add a "Take down a picture by URL" form; this is how emailed notices enter the queue (D2).
  6. **Summary and history:**
     - `GET /api/admin/summary` returns the open counts, the oldest open item per queue, and 90-day report stats (median and max hours to resolution, and the count over the target).
     - Add `?status=open|resolved&page=` to both list endpoints. Today both return open items only (`media/reports.rs:217`, `catalog/admin.rs:336`).
     - Load both tab badges on mount (today only the mounted tab shows a count, `AdminPage.vue:26,44-46`), show each item's age, flag overdue items (72 h for now), and add a history view.
     - Show the open count on the admin link in Settings.
  7. **Admin notification:**
     - a job under `jobs::lock_ids::ADMIN_NOTIFY = 0x6263_0003` (`server/src/jobs.rs:19-22`)
     - it mails every instance admin with a verified address: reports at most hourly, catalog requests as a daily digest
     - templates in en and de, with a link to `{PUBLIC_URL}/admin?tab=reports`
     - the state lives in migration `2100_admin_notify_state.sql`, as a key/timestamp row with **no** foreign key to `users`, so the export check doesn't apply
- **Owns:**
  - Server: `server/src/admin/**`, `server/src/catalog/{admin,requests,mod}.rs`, `server/src/media/{reports,mod}.rs`, `server/src/email/templates.rs`, `server/src/jobs.rs`, `server/migrations/21*.sql`
  - Server tests: `server/tests/{admin_guard,admin_summary,admin_notify,catalog_admin_edit,media_reports,catalog_requests}.rs`
  - Frontend: `frontend/src/pages/AdminPage.vue` (and its test), `frontend/src/components/admin/**`, `frontend/src/admin.ts` (and its test), `frontend/src/components/EditRequestSheet.vue` (or a new `AdminEditSheet.vue`), `frontend/src/pages/BookDetailPage.vue`, `frontend/src/components/ReportCoverSheet.vue`
- **Shared:** `lib.rs` (`admin_router` merges `admin::admin_routes()`), `client.ts`, `error.rs`, `apiErrors.ts`, `en.json`/`de.json`, `SettingsPage.vue` (`:238-244`).
- **Acceptance:**
  - `cargo test --test admin_guard` passes. Removing the route layer (`lib.rs:156-159`) and approve's `InstanceAdmin` argument makes it fail (a mutation check, reverted).
  - `cargo test --test media_reports --test catalog_requests` cover the three defects, the `current` values, and a takedown with a body: one `removed` row with `created_at = received_at` and `resolved_by` the admin. Without a body it still answers 204.
  - `cargo test --test catalog_admin_edit`: the fields get `field_sources = admin`, and a later refresh or harvest leaves them alone. Invalid input gets the same 400s as approve.
  - `cargo test --test admin_summary`: a report resolved after 50 h gives a median of 50 and `over_target = 0`. `?status=resolved` lists newest first, with paging.
  - `cargo test --test admin_notify`:
    - one mail per verified admin, in their locale, and none to unverified admins or non-admins
    - a second run with nothing new sends nothing
    - a new report sends again, within the throttle
  - `npx vitest run src/pages/AdminPage.test.ts src/admin.test.ts`: both badges show on first load, the overdue flag, the history view, before and after values, and the takedown form calls DELETE.

#### LEG: legal pages, terms acceptance and the legal paperwork (#53)

- **Size:** L. **Dependencies:** none.
- **Soft blocks:** Q13 (how terms are accepted), D3 (`LEGAL_DIR` with HTML, and one contact address).
- **Scope:**
  1. **First, within days, because Rafael needs these for eRecht24 or a lawyer:**
     - **`docs/legal/privacy-facts.md`:**
       - the storage table: the session cookie (`server/src/auth/mod.rs:33-39`), `localStorage` and `sessionStorage` keys
       - logs: no IPs in the app (`server/src/lib.rs:192-206`), the Caddy log
       - outbound calls, which carry no user identity
       - that the browser contacts no third party
       - retention (`server/src/jobs.rs`, `server/src/auth/mod.rs:165-170`, `server/src/auth/deletion.rs`)
       - export and deletion
     - **`docs/legal/vvt.md`:** the Art. 30 record, one row per processing activity.
     - **`docs/legal/processors.md`:**
       - the mail provider and backup target (to be filled in, DPA needed)
       - the monitor
       - Codemagic, APNs and FCM later
       - Open Library and DNB as non-processors, with the reason
     - Optional: `server/tests/legal_docs.rs` checks that every table with a foreign key to `users` appears in `vvt.md`.
  2. **`docs/legal/takedown.md`:**
     - how notices come in: the in-app report, and the contact address
     - the targets from Q12, marked as "to confirm"
     - triage by reason
     - removal per content type, with the endpoints ADM builds
     - decision notices, counter-notices and record keeping
  3. **Legal texts on the server:**
     - An optional `LEGAL_DIR` holds six operator files, `{imprint,privacy,terms}.{de,en}.html`, plus `manifest.json` (`terms_version`, and `updated` per document).
     - A public `GET /api/legal/{doc}?locale=de|en` serves them. `doc` is a fixed enum, so there is no path traversal. A missing locale falls back to the other language, and the response says which one it served.
     - `/api/config` gains `legal`, `terms_version` and `contact_email`. The contact address is `CONTACT_EMAIL`, which today only goes into the User-Agent (`server/src/outbound.rs:98-103`).
     - Outside `DEV_MODE`, startup refuses `SIGNUP_MODE=open` unless all six files are present. That makes the legal texts a hard gate for #55.
     - `DEV_MODE` serves marked placeholders from `server/legal-placeholder/*.html`. They are `.html`, not `.md`, because the root `.dockerignore` drops `*.md`.
  4. **Legal pages in the SPA:**
     - Public routes `/imprint`, `/privacy` and `/terms`, declared **outside** the AppShell. Otherwise the catch-all inside the AppShell sends logged-out visitors to `/login` (`frontend/src/router.ts:24-133`).
     - Redirects from `/impressum`, `/datenschutz` and `/nutzungsbedingungen`.
     - `LegalPage.vue` renders the server's HTML.
     - `LegalLinks.vue` goes on Login, Signup, ForgotPassword, ResetPassword, VerifyEmail, Join, NotFound and the AppShell, plus a Legal section in Settings with a contact mailto.
     - Add a privacy link to `EmailPrompt.vue`, which collects new addresses at the cutover.
  5. **Terms acceptance:**
     - Migration `2200_users_terms.sql` adds `users.terms_version` and `terms_accepted_at`.
     - `RegisterIn.terms_version` is required, and must be the current version, whenever legal texts are configured; otherwise 400 with a new `msg` constant, mapped in `apiErrors.ts`.
     - `UserOut.needs_terms` covers imported users and later terms versions.
     - `POST /api/auth/terms` and a `TermsPrompt.vue` in the AppShell, shown the way `EmailPrompt` is.
     - Both columns go into the export.
- **Owns:**
  - Server: `server/src/legal.rs`, `server/src/config.rs`, `server/src/auth/{mod,signup,account}.rs`, `server/src/dto.rs`, `server/legal-placeholder/**`, `server/migrations/22*.sql`
  - Server tests: `server/tests/{legal,legal_docs,auth_config,auth_register,auth_account,auth_export,auth_required}.rs`
  - Docs: `docs/legal/**`
  - Frontend: `frontend/src/router.ts` (and its test), `frontend/src/pages/{LegalPage,LoginPage,SignupPage,ForgotPasswordPage,ResetPasswordPage,VerifyEmailPage,JoinPage}.vue`, the NotFound page, `frontend/src/pages/{LegalPage,AuthPages}.test.ts`, `frontend/src/components/{LegalLinks,TermsPrompt,EmailPrompt}.vue`, `frontend/src/layouts/AppShell.vue`, `frontend/src/stores/{service,session}.ts`, `frontend/src/types.ts`
- **Shared:** `lib.rs`, `auth/export.rs` (account section), `error.rs`, `apiErrors.ts`, `client.ts`, `en.json`/`de.json`, `SettingsPage.vue` (new section at the end).
- **Acceptance:**
  - `cargo test --test legal`: every document and locale is served anonymously, the fallback locale is reported, an unknown document gets 404, and the manifest version appears in `/api/config`.
  - `cargo test --test auth_required --test auth_config`: the anonymous allow-list is updated, open sign-up without texts is refused outside `DEV_MODE`, `DEV_MODE` serves the placeholders, and `contact_email` is in `/api/config`.
  - `cargo test --test auth_register --test auth_account --test auth_export --test api_error_copy`:
    - a missing or stale `terms_version` gets 400
    - a valid one stores the version and the timestamp
    - `needs_terms` flips after `POST /api/auth/terms`
    - the export includes both columns
  - `npx vitest run src/router.test.ts src/pages/LegalPage.test.ts src/pages/AuthPages.test.ts src/i18n/messages.test.ts`:
    - the legal routes render signed out without a redirect, and the German aliases redirect
    - every auth page links the imprint and privacy policy, and sign-up also links the terms
  - The legal docs:
    - every table with personal data appears in `vvt.md`
    - every outbound host in `server/src/config.rs:17-20` appears in `processors.md` with a verdict
    - every removal path in `takedown.md` points to an existing endpoint or UI
  - Manual check: `http://agents.lan:8084/imprint` opens signed out.

#### CUT: cutover preparation (#52, #38)

- **Size:** M. **Dependencies:** none.
- **Soft block:** Q6 for the legacy branch and tag.
- **Scope:**
  - **Import of a database in WAL mode:**
    - The production database runs in WAL mode, and an `app.backup` copy keeps the WAL header.
    - A read-only open in a directory uid 1000 cannot write fails with `attempt to write a readonly database` (`server/src/importer/source.rs:261-268`).
    - Fix: copy `<db>` and `<db>-wal` into a private temp dir and open that copy read-only. Never use `immutable=1`, which silently ignores committed `-wal` content.
  - **Independent count check:**
    - `deploy/cutover/legacy-counts.sql` (SQLite) and `deploy/cutover/target-counts.sql` (Postgres) count every entity, plus per-user counts.
    - `deploy/cutover/compare.sh` runs both side by side and diffs `id|username|password_hash`. That diff is valid only before anyone logs in, because login re-hashes legacy hashes.
    - `server/tests/cutover_counts.rs` runs both files around the `fresh.sql` fixture import, so the queries can't go stale against the schema.
  - **Old domain:**
    - `deploy/legacy-origin/Caddyfile.example`: `handle /sw.js` serves a self-unregistering worker with `no-store`, and every other path goes through `handle { redir https://<NEW_HOST>{uri} permanent }`. The redir must sit inside `handle`, because `redir` runs before `handle`.
    - `deploy/legacy-origin/sw.js` is that worker. This covers risk R22 (`docs/phase1/spec/platform.md:1374-1378`).
  - **`docs/phase2/cutover.md`**, a copy-paste runbook built on `docs/phase1/notes/imp.md:209-246`:
    - the checklist before the cutover
    - the announcement in DE and EN (new address, same login, add an email, add the app to the home screen again, finish unsent drafts, old invite links stop working, push is gone until the app)
    - the cutover-day commands
    - the rollback window: until users write to the new instance
    - the steps afterwards: `DNB_HARVEST=true`, closing the issues, deleting legacy data
  - **Forgejo housekeeping:** close #27 now: it was live in v0.10.0 (`frontend/src/pages/DiscoverPage.vue:81`, `server/src/shelf/search_history.rs`). #29 follows D9.
  - **After Q6:** create branch `legacy-python` and annotated tag `legacy-python-final` at 1c5efe3, and point `server/tests/fixtures/legacy/README.md` at the branch.
  - **CUT-R:** open the cutover branch as a draft (§2.4).
- **Owns:** `server/src/importer/source.rs`, `server/tests/importer_run.rs`, `server/tests/cutover_counts.rs`, `server/tests/fixtures/legacy/README.md`, `deploy/cutover/**`, `deploy/legacy-origin/**`, `docs/phase2/cutover.md`.
- **Acceptance:**
  - `cargo test --test importer_run wal` passes: a WAL database with uncheckpointed rows, in a `chmod 555` directory, imports with matching counts. Without the fix it fails.
  - `cargo test --test cutover_counts` passes.
  - `shellcheck deploy/cutover/compare.sh` is clean.
  - `caddy validate` passes on the redirect config.
  - Locally on `agents.lan:8085`:
    - `curl -sI …/shelf?x=1` gives a 301 to `https://<NEW_HOST>/shelf?x=1`
    - `/sw.js` gives 200, a JavaScript type and `no-store`
  - `git ls-remote origin refs/heads/legacy-python` shows 1c5efe3, and no release run started.

### 2.3 Wave 2: after the Wave 1 merge and Rafael's answers

#### ADM2: reports on user content, decision notices, suspension (#54, #55)

- **Size:** L. **Dependencies:** ADM and LEG merged; Q11, Q12 and Q15, and Q17 only if per-address counting is chosen.
- **Scope:**
  - **Report targets (Q11 = B):**
    - migration `2101_reports_targets.sql` widens `reports.target_kind` (today `CHECK (target_kind IN ('cover'))`, `0001_init.sql:517`) to `avatar`, `work` and `edition`, with target columns
    - avatars go through the existing `POST /api/media/{public_id}/reports`: drop `AND kind = 'cover'` (`reports.rs:149`) and require that the reporter can see the owner
    - user-added works and editions get `POST /api/books/works/{work_id}/reports` and `POST /api/books/editions/{edition_id}/reports`
    - the reports section of the export
    - `ReportCoverSheet.vue` becomes `ReportSheet.vue`, with a "Report" action on `MemberShelfPage.vue` and on user-added books
    - a line "no account? write to {contact_email}"
    - `AdminReports` renders every kind
  - **Decision notices (Q12):** en and de mails, sent after the commit: the decision to the reporter, and a statement of reasons to the uploader or avatar owner.
  - **Legal footer:** one line linking `{PUBLIC_URL}/imprint` and `/privacy` in every template (`server/src/email/templates.rs:39,73,110,154`, plus ADM's templates).
  - **Suspension (Q15):**
    - migration `2102_users_suspended_at.sql`
    - suspended accounts are refused in `server/src/auth/extract.rs` (the resolve query at `:76`) and at login, and their sessions are deleted
    - `POST /api/admin/users/{username}/suspend` and `/unsuspend`, and `AdminUsers.vue`
  - **Per-address lockout:** only if Q17 picks it.
- **Owns:** additionally `server/src/auth/{extract,login}.rs`, `server/src/admin/users.rs`, `server/src/catalog/reports.rs`, `frontend/src/pages/MemberShelfPage.vue`, `frontend/src/components/ReportSheet.vue`, `server/tests/{catalog_reports,admin_users}.rs`.
- **Acceptance:**
  - `cargo test --test media_reports --test catalog_reports --test admin_users --test admin_guard --test auth_required --test auth_export` cover:
    - an avatar report, then "remove": `avatar_url` is null everywhere, and `/media` answers 410
    - a report on a user-added work shows up in the queue
    - "remove" mails the reporter and the uploader, "dismiss" mails only the reporter, and a rollback sends nothing
    - after a suspension, the user's cookie and bearer token get 401, login is refused, and an unsuspend restores login
  - The template unit tests check the footer.

#### LEG2: retention and final texts (#53)

- **Size:** S. **Dependencies:** LEG and ADM merged; Q14.
- **Scope:**
  - **Retention in `auth::cleanup`:**
    - delete resolved reports and decided edit requests after the period
    - delete empty sign-ups that never verified, per Q14
    - fill in the decided periods and provider names in `docs/legal/*.md`
- **Acceptance:** `cargo test --test jobs`: rows past the period are deleted, and rows inside it, imported accounts, and accounts with data stay.

#### DEP2: automatic deploys and real values (#51)

- **Size:** M. **Dependencies:** DEP merged; Q5, Q7 and Q8.
- **Scope:**
  - **Pull variant (Q8):** `deploy/hosted/update.sh` plus a timer every 2–5 minutes. If the digest changed, it runs `pull`, then `backup.sh` (migrations only go forward), then `up -d app`, waits for healthy, and pings the heartbeat.
  - **Push variant:** a deploy job over SSH with a key restricted by `command=`.
  - Put the values from Q7 into `env.example` and the README.
  - Push `v0.11.0-rc.1` to Forgejo and check the registry tags.
- **Acceptance:**
  - With a local `registry:2` on 127.0.0.1:5000: image A, then image B as `stable`. The backup runs first, the app comes up healthy on B, and the success ping arrives. A failing B' exits non-zero and sends the failure ping.
  - The rc tag creates `bookclub-server:0.11.0-rc.1` and changes no tag of `rafiistcool/bookclub`.

#### CUT2: rehearsal on a copy of the production database (#38)

- **Size:** M. **Dependencies:** CUT merged; Rafael's snapshot and production facts (A2, A4).
- **Scope:**
  1. Start a dedicated `bookclub-pg-rehearsal` on 5456, and keep `MEDIA_DIR` outside the repo.
  2. Build with `SQLX_OFFLINE=true cargo build --release`.
  3. Run the dry run and the real run with a fixed `--club-slug` (D5), then `compare.sh`.
  4. Start the server at `http://agents.lan:8080` for Rafael's check.
  5. Record the run time and the lookup wait.
  6. Post counts only to #38, then remove everything with `docker rm -fv`, and delete the copy and the media.
- **Acceptance:**
  - Both runs exit 0.
  - `diff <(tail -n +2 dry.txt) real.txt` differs at most in the lookup-wait line.
  - The counts and the hash comparison match.
  - Rafael logs in with his old password and sees his data.
  - #38 is closed.

### 2.4 Cutover branch CUT-R (#52), merged only on cutover day

- **Branch:** `cutover/remove-python`, cut from `main` after Wave 1 lands there. It has a draft PR into `main`, is rebased whenever `main` moves, and is **merged only after the real import is verified** (the counts match and Rafael has checked in the browser).
- **Deletes:**
  - `backend/` (81 files)
  - `docker-compose.yml`, `docker-compose.dev.yml`, `.env.example`
  - `deploy/{bookclub.service,docker-entrypoint.sh,env.example}`
  - `data/.gitkeep`
  - `.cursor/{environment.json,install.sh}` (or rewrites them for cargo and Vite)
  - `server/tests/parity_checklist.rs`, as its own comment asks (`:21-23`)
  - the backend job in `.forgejo/workflows/ci.yml:14-29`
  - `.github/workflows/` (D8)
  - the Python lines in `.gitignore`. It also adds `server/data/` to `.gitignore`.
- **Docs:**
  - `README.md` describes the hosted service and the dev setup, and mentions the password change again (the README criterion of #35).
  - `SELFHOST.md` is deleted.
  - `CLAUDE.md` loses the "In transition" paragraph and the Python commands and sections.
  - `docs/plan.md` gets a status line with the cutover date.
- **Acceptance:**
  - Forgejo CI is green.
  - `git ls-files backend | wc -l` is 0.
  - `cd server && cargo test --locked` passes.
  - `grep -rnE 'backend/|uvicorn|pytest|app\.backup' --exclude-dir=docs --exclude-dir=node_modules --exclude-dir=target .` finds only historical mentions.
  - Every command in `CLAUDE.md` runs as written.

### 2.5 Dependencies and concurrency summary

| Package | Hard dependencies | Runs alongside | Notes |
|---|---|---|---|
| DEP | none | Wave 1 | Merges first; its Docker builds count as a build slot |
| CUT | none | Wave 1 | Small |
| LEG | none | Wave 1 | The `docs/legal/` facts come first, for Rafael's generator |
| ADM | none | Wave 1 | Owns the email templates in Wave 1 |
| GATE | DEP merged (end-to-end only) | Wave 1 | Merges last; its browser run covers LEG's and ADM's pages |
| ADM2 | ADM, LEG; Q11, Q12, Q15 (Q17) | Wave 2 | Owns `auth/login.rs` and `auth/extract.rs` |
| LEG2 | LEG, ADM; Q14 | Wave 2 | |
| DEP2 | DEP; Q5, Q7, Q8 | Wave 2 | |
| CUT2 | CUT; A2, A4 | Wave 2 | Needs real data; touches no repo files |
| CUT-R | Wave 1 in `main` | rebased continuously | Merged on cutover day only |

---

## 3. Blocked items

| Item | Waits on | Unblocks |
|---|---|---|
| ADM2: reports on avatars and user-added books | Q11 | #54 "reports on covers and user content"; the promise in `endpoint-map.md:189` |
| ADM2: decision notices | Q12; real delivery also needs Q4 | DSA Art. 16(5) and 17 duties before #55 |
| ADM2: suspension | Q15 | Abuse handling before #55 |
| ADM2: per-address login lockout | Q17 | Security review §3 |
| LEG2: retention jobs | Q14 | The retention statements in the VVT and privacy policy |
| DEP2: auto-deploy | Q8 | #51 "tags build the image and deploy it" |
| DEP2: first release and registry tags | Q5 | Any release from `main`; the staging deploy |
| CUT: `legacy-python` branch and tag | Q6 | #52 step 8; a Python hotfix path |
| CUT2: rehearsal | A2, A4 (snapshot and facts) | Closing #38; the size of the downtime window |
| Admin and decision mails in production | Q4, and A21 (a verified admin address) | #54 response time, #55 |
| Staging deploy on the VM | A6–A13 | #51 acceptance (LAN isolation, restore, monitor) |
| Legal texts live | Q1, A15–A18, LEG merged | #53; #52 if Q3 = at cutover; #55 |
| Cutover | Q1, Q2, Q3, Q4, #51 done, CUT2 done | #52, then the beta invites |
| Open sign-up | Everything above, A23–A26 | #55 |
| Phase 1 "Needs Rafael" (`docs/phase1/work-plan.md:589-600`) | The mail provider is Q4; the production copy is A4; Open Library descriptions are Q18; the other defaults stand unless overridden | |

---

## 4. Questions for Rafael

Ordered by how much work each answer unblocks. "Take the recommendations" is a valid answer to all of them.

**Q1. Which public name (#50)?**
- **Options:** Eselsohr, Shelfmates, Marginalia, Lesezirkel, or another name.
- **Recommendation:** let me first prepare `docs/phase2/name-check.md`: domains (.de, .app, .com), App Store and Play Store, and a first DPMA and EUIPO search. Then you decide and confirm the trademark search. The name gates mail DNS, the Impressum, `PUBLIC_URL`, the cutover and #55.

**Q2. In what order do things happen?**
- **Options:**
  - A: deploy the VM now on a temporary hostname with test data only, and wipe it before the import. Cut over once, on the final domain, after the mail provider works. The import is the first real data, and beta invites start only after it.
  - B: cut over sooner on the old hostname with an interim SMTP setup, and move the domain later.
  - C: open the closed beta before the import. The importer refuses any user or invite (`server/src/importer/mod.rs:240-305`), so this needs a merge mode in the importer (size L).
- **Recommendation:** A. It needs no importer change, users move only once, and the infrastructure checks don't need the name.

**Q3. Must the legal texts be live at the cutover (#52) or only at #55?**
- **Options:** all three at #52; imprint and privacy at #52 and terms at #55; all at #55.
- **Recommendation:** all at #52. The new public domain shows a login page to anyone, and `EmailPrompt` collects new personal data on day one. The terms come from the same generator at no extra effort.

**Q4. Which EU transactional mail provider?**
- **Options:** Brevo (FR), Scaleway Transactional Email (FR), Mailjet (FR), or a mailbox provider's SMTP (mailbox.org, Posteo; not meant for transactional volume).
- **Recommendation:** Brevo. It offers SMTPS or STARTTLS with DKIM on your domain and a standard DPA, and its free tier covers beta volume.
- **What it unblocks:** the production server does not start without `SMTP_URL` and `EMAIL_FROM` (`server/src/config.rs:167-188`); also deliverability, admin mails and the processor list.

**Q5. Under which name and tags is the Rust and SPA image published?**
- **Options:**
  - A: a new name, `git.tillhome.de/rafiistcool/bookclub-server`, with tags X.Y.Z, X.Y and `stable`, never `latest`. Versions continue at v0.11.0, and v1.0.0 marks the cutover.
  - B: the same name `bookclub`, starting at v1.0.0, after production is pinned to an exact tag.
  - C: the same name and the same numbering.
- **Recommendation:** A. Whatever the Python host follows (`:latest` on GHCR per v0.10.0's compose, or on git.tillhome.de per `docker-compose.yml:21`), a new name cannot replace it. `metadata-action` adds `latest` today (`release.yml:68-80`).
- The two gap reports disagreed here; this reconciles them.

**Q6. May I preserve the Python app now and replace the root Dockerfile now?** That means creating branch `legacy-python` and annotated tag `legacy-python-final` at 1c5efe3 (v0.10.0-3), and letting DEP replace the root `Dockerfile` with the Rust and SPA image.
- **Options:**
  - A: both now.
  - B: keep the root Dockerfile until #52 and put the image at `deploy/hosted/Dockerfile`.
  - C: tag `v0.10.1` at 1c5efe3, which publishes one more Python image.
  - D: nothing until #52.
- **Recommendation:** A. `main` cannot build the Python image anyway (`Dockerfile:28`), and a tag without a `v` starts no release run. Forgejo runs the workflow of the tagged commit, so a hotfix tag on `legacy-python` still builds Python.

**Q7. Which source address does the app VM see on connections from Caddy, and how does the VM reach git.tillhome.de?**
- **Options:**
  - A: no SNAT. `TRUSTED_PROXIES=<Caddy IP>/32`, the VM accepts 8080 only from Caddy, and the VM reaches git.tillhome.de through one pinhole to Caddy:443, with split DNS.
  - B: the same, but the VM uses hairpin NAT through the public IP.
  - C: SNAT on the router, with `TRUSTED_PROXIES=<router VLAN address>/32`.
- **Recommendation:** A. It opens no wider surface than any internet client gets, and no other NATed flow becomes trusted.
- **Why it matters:** a wrong value puts every user in Caddy's rate-limit bucket.

**Q8. How does a release reach the VM?**
- **Options:**
  - A: pull. A timer on the VM follows `stable`, backs up, updates, waits for healthy and pings a heartbeat.
  - B: push. A deploy job SSHes in with a key restricted by `command=`. This needs a firewall rule from the LAN to the VM's port 22.
  - C: a second Forgejo runner on the VM.
- **Recommendation:** A. There is no new firewall hole and no credential on the LAN side.

**Q9. Where do the encrypted offsite backups go?**
- **Options:** Hetzner Storage Box with restic; Backblaze B2 EU or another EU S3 bucket with restic; Storage Box or BorgBase with borg; additionally a LAN NAS.
- **Recommendation:** Hetzner Storage Box with restic, nightly, keeping 7 daily, 4 weekly and 12 monthly snapshots, plus a weekly restore test. It is a German provider with a DPA and cheap, and restic also speaks S3 for the VPS later (#65).

**Q10. Which uptime monitor?**
- **Options:**
  - A: a hosted HTTP keyword check on `https://<host>/api/health` expecting `"ok"` (Better Stack or UptimeRobot free).
  - B: Uptime Kuma on an external VPS.
  - Either way, healthchecks.io heartbeats for the backup, update and restore-test timers.
- **Recommendation:** A plus the heartbeats, alerting your phone. There is no server to maintain, and the health endpoint carries no personal data.

**Q11. Which content gets an in-app "Report" button before #55?**
- **Options:**
  - A: covers only.
  - B: covers, profile pictures and user-added works and editions: everything people outside a club can see.
  - C: B plus posts, replies, quotes and rating takes, with text snapshots shown to you.
- **Recommendation:** B now, and C together with friends (#57). Until then, every reader of a user's text was let in by a club admin, who can already delete club posts.

**Q12. What takedown targets do the legal texts and the admin page state, and does the app mail decisions?**
- **Options:**
  - A: acknowledge emailed notices within 24 h, decide within 72 h, and the app mails the decision to the reporter and a statement of reasons to the uploader.
  - B: the same targets, with the mails sent by hand.
  - C: decide within 7 days, by hand.
- **Recommendation:** A. A mail the app sends cannot be forgotten, and it uses the existing `EmailSender`. By hand, the admin page would have to show users' email addresses, which `AdminReports` doesn't today.
- Have #53's legal source confirm the DSA Art. 16(5) and 17 wording.

**Q13. How is acceptance of the terms recorded?**
- **Options:**
  - A: a sentence with links above the sign-up button, with the version and time stored. Imported users, and everyone after a new terms version, get a one-time prompt.
  - B: a required checkbox, stored the same way.
  - C: links only, nothing stored.
- **Recommendation:** A. It is enough evidence without friction; switch to a checkbox only if the lawyer or generator asks for one.

**Q14. Retention periods.**
- **Options:**
  - A:
    - delete resolved reports and decided edit requests after 3 years
    - delete empty sign-ups (no verified address, no data, no sign-in for 30 days) after 30 days
    - legacy data: delete the dev-VM copy right after the rehearsal, and delete the final snapshot (encrypted) and the stopped Python container 30 days after the cutover
  - B: keep legacy data 90 days, and anonymise reporters after 12 months.
  - C: keep everything.
- **Recommendation:** A. It covers defending claims and is simple to state.
- **Note:** an earlier proposal ("purge unverified accounts after 30 days") would have deleted active users. Only club creation needs a verified address, and imported users may type an address without verifying it. The rule above is narrowed to avoid that.

**Q15. Can the instance admin suspend an account?**
- **Options:** no (SQL when needed); a suspend flag (login refused, sessions dropped, content stays); suspend and also hide the account's content.
- **Recommendation:** the suspend flag. It is small and reversible. Hash blocking stops only identical bytes, and a bad username cannot be changed today.

**Q16. Do the in-app limits count as #51's "proxy rate limits"?**
- **Options:**
  - A: yes, and amend #51. The proxy does TLS, HSTS and the body size.
  - B: rebuild the shared Caddy with `caddy-ratelimit` through xcaddy.
  - C: per-source connection limits on the firewall.
- **Recommendation:** A. The in-app limits key on the real client and are tested, while a custom Caddy build adds upkeep to the proxy that also serves Forgejo. Revisit if the logs show floods.

**Q17. Do login failures keep counting per account (security review §3, `docs/phase1/security-review.md:72-83`)?**
- **Options:**
  - A: per account, as today: 10 per 10 minutes, with username and verified address in separate buckets.
  - B: per account and client /64, plus a per-account ceiling of about 100 an hour.
- **Recommendation:** A for launch. Strangers cannot see usernames, and a password reset clears both buckets.

**Q18. Do Open Library descriptions stay (open since Phase 1)?**
- **Options:**
  - A: keep them, and remove one on notice with ADM's direct admin edit.
  - B: keep them, with a "report description" button.
  - C: stop storing them.
- **Recommendation:** A. It needs no extra code once ADM lands, and the descriptions are useful. `takedown.md` documents the path.

### Defaults I will use unless you object

- **D1. Admin notifications:** mails to every instance admin's verified address; reports at most hourly, catalog requests as a daily digest. No new config variable.
- **D2. Notices from people without an account** enter through ADM's admin-only "take down by URL" form, which records a report with the received time. There is no public unauthenticated form.
- **D3. Legal texts:**
  - `LEGAL_DIR` on the VM holds HTML fragments as the generator exports them, served at `/api/legal/{doc}`.
  - There is one public contact and takedown address: `CONTACT_EMAIL`, on the new domain, shown in `/api/config`.
- **D4. Overdue threshold** on the admin page: 72 h until Q12 says otherwise.
- **D5. Imported club slug:** the slugified club name plus 4 random characters, generated once and used in both the rehearsal and the cutover (`--club-slug`).
- **D6. Cutover-day import:** `--allow-unmatched`, then `rematch-legacy`, so a DNB or Open Library outage never holds the downtime window hostage.
- **D7. Migrations:** no squash; new migrations use the reserved blocks.
- **D8.** `.github/workflows/` is deleted at the cutover; it never runs. Until then DEP keeps it roughly in sync.
- **D9. Old issues:**
  - Close #27 now.
  - Close #29 now: drag-and-drop shipped in v0.10.0 (24a61a3, ab016a6), and keyboard reordering is not planned.
  - Close #35 at the cutover.
  - Close #34 at the cutover as superseded, after adding "Web Push for the browser app" to #60.

---

## 5. Actions only Rafael can take, in order

**Now: these unblock Wave 2 and the rehearsal**
1. Answer Q1–Q10, or say "take the recommendations".
2. Send me the production facts that are not in the repo:
   - the host, and the compose directory or systemd unit
   - the exact image reference (`docker compose images` or `docker inspect`), and whether an auto-updater (for example watchtower) runs
   - the data directory
   - `BOOKCLUB_NAME` and `BOOKCLUB_TZ`
   - the old public hostname, and whether the Caddy that serves git.tillhome.de also serves it
   - your username for `--owner`
3. Pin the production Python service to its exact image tag and turn off any auto-updater.
4. Snapshot for the rehearsal:
   1. Run `docker compose exec bookclub python -m app.backup /data/bookclub-rehearsal.db` (bare metal: `cd backend && .venv/bin/python -m app.backup <path>`).
   2. Copy the file to the agents VM outside the repo, for example `~/bookclub-prod/` with `chmod 700`.
   3. Delete it from the production data directory.
5. When I start the rehearsal server, log in at `http://agents.lan:8080` with your old password, check your data, and tell me to delete everything.

**Name, domain and mail (#50, #39)**

6. Choose the name. Document the DPMA and EUIPO search (Nice classes 9, 41, 42). Register the domain.
7. DNS: an A record (and AAAA if the line has IPv6) for the final host, pointing at the business line's static IP. Now, also a temporary staging host under tillhome.de.
8. Mail: open the provider account and sign its DPA. Verify the domain, publish the SPF include, DKIM and DMARC (`p=none` with `rua` reports), and create the SMTP credentials (`smtps://` or `?tls=required`).
9. Set up the contact and takedown mailbox on the new domain, and book an Impressum address service.

**Infrastructure (#51)**

10. Proxmox: create the app VM (Debian 13, about 2 vCPU, 4 GB RAM, 40 GB disk, Docker Engine and the compose plugin) in its own VLAN.
    - Inbound: only Caddy's IP to 8080, plus SSH from your admin host.
    - Outbound: DNS, NTP, 443 to the internet, git.tillhome.de per Q7, SMTP 465/587, and the backup port. Deny RFC1918 addresses.
    - Check the #51 acceptance criterion: from the VM, `nc -vz -w3 <nas-ip> 445` and a curl to Forgejo's LAN address both fail.
11. Forgejo: create a read-only package token (ideally a bot account with `read:package`), run `docker login git.tillhome.de` on the VM, and check that the `REGISTRY_TOKEN` repo secret is still valid.
12. Backups: create the backup target and sign its DPA. Add the VM's SSH key, and keep the restic password in your password manager, not only on the VM.
13. Monitoring: set up the uptime check on `/api/health` and heartbeat checks for the backup, update and restore-test timers, all alerting your phone.
14. Caddy: add the site block from `deploy/hosted/Caddyfile.example` for the staging host and reload. Rebuild with `caddy-ratelimit` only if Q16 = B.
15. On the VM:
    1. Install `deploy/hosted/` (compose, the filled env, the systemd timers).
    2. Run `docker compose up -d --wait`, then `docker compose exec app bookclub-server beta-invite`, and sign up on staging.
    3. Wipe staging before the cutover (`docker compose down -v`, empty `/srv/bookclub/media`).
16. #51 acceptance: run `restore-test.sh` and keep the output. Run `docker compose stop app`, confirm that the monitor alerts, then start the app again.

**Legal (#53)**

17. Give `docs/legal/privacy-facts.md` and `docs/legal/vvt.md` to eRecht24 or a lawyer. Generate the Impressum (§ 5 DDG) and the Datenschutzerklärung; it names the mail provider, the backup host, the monitor, and the 7-day Caddy log with IPs. Also generate Nutzungsbedingungen with moderation and notice-and-action clauses, in DE and EN. Have them confirm the DSA hosting duties (Art. 11/12, 14, 16, 17) and the Q12 targets.
18. Sign the VVT as the controller, and file every DPA (mail, backup).
19. Put the six files and `manifest.json` into `/srv/bookclub/legal` on the VM, and check `/imprint`, `/privacy` and `/terms` logged out.

**Cutover (#52)**

20. Announce the downtime window to the club at least a week ahead, with the DE/EN text from `docs/phase2/cutover.md`.
21. On cutover day, follow the runbook:
    1. Stop the old app, take the snapshot, and transfer it.
    2. Dry run, go/no-go, real run.
    3. Run `compare.sh` before anyone logs in.
    4. Run `make-admin <you>`, then add and verify an email on your admin account.
    5. Check in the browser.
    6. Add the redirect and the `/sw.js` block for the old host, then `caddy reload`.
22. Approve merging `cutover/remove-python`, then mint the closed-beta invites.

**Open sign-up (#55)**

23. Run `bookclub-server send-test-mail` to Gmail, Outlook and GMX, and check mail-tester: SPF, DKIM and DMARC must pass.
24. Run `deploy/hosted/verify-limits.sh https://<host>` from home, plus one sign-up attempt from a second network (a phone on mobile data). If that attempt also gets 429, `TRUSTED_PROXIES` is wrong. The run blocks your home IP from sign-up and reset for an hour.
25. Run a takedown end to end on the live instance: report a cover, remove it, confirm `/media` answers 410 and the mails arrive, and check the time against the target.
26. Set `SIGNUP_MODE=open` in the env and run `docker compose up -d app`. Outside `DEV_MODE`, startup refuses this until the legal texts are present.

**Afterwards**

27. When the Q14 period ends, delete the final snapshot, the stopped Python container and its data directory.