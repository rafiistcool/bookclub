# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Bookclub is a self-hosted, invite-only book club for 2–5 people. A FastAPI backend and a Vue 3 SPA ship as one container with one SQLite file. The README covers features, self-hosting, configuration, and the local demo (`app.seed_demo`). SELFHOST.md covers proxies, backups, and upgrades.

**In transition:** Bookclub is becoming a hosted multi-club service with a Rust server in `server/` and Postgres. `docs/plan.md` records the decisions, and Forgejo issues #36–#67 track the work. Until the cutover (#52), `backend/` is still what runs in production. Port its behaviour; don't extend it.

**Phase 1 design:** `docs/phase1/` holds the binding design for `server/`: `conventions.md` (architecture, frozen interfaces, coding rules), `endpoint-map.md` (routes, owners, shared DTOs, error strings), `legacy-mapping.md` (importer), `work-plan.md` (waves, packages, file ownership) and `spec/` (behaviour of the Python app per area). The schema is `server/migrations/0001_init.sql` plus the later files there; a merged migration is never edited.

**Phase 2** (hosted launch, #50–#55): `docs/phase2/work-plan.md` holds the waves, packages, file ownership and migration number blocks. Each package records its contract changes in `docs/phase2/notes/<pkg>.md`; the integrator moves them into `conventions.md`, `endpoint-map.md`, this file and the READMEs, and keeps the notes as the record. `deploy/hosted/README.md` is the operator runbook (image, compose, releases, automatic updates (`update.sh`, `bookclub-update.timer`), the VM's network, Caddy, backups with Storage Box snapshots, restore, legacy import, legal texts, open sign-up); `docs/phase2/cutover.md` is the cutover runbook. The public name is still open (Q1, first pass in `docs/phase2/name-check.md`): keep "Bookclub" until BRAND puts the chosen one in.

**Phase 3** (#56 scanning, #57 friends, then the private move): `docs/phase3/work-plan.md` holds the order, the packages, file ownership, migration blocks (SCAN 3000–3099, FRIENDS 3100–3199, MOVE 3200–3299, integrator 3900–3999) and ports (Postgres SCAN 5461, FRIENDS 5462, MOVE 5463; browser SCAN 8091, FRIENDS 8094, MOVE 8095, integrator 8096). Since 2026-10-29 the order is: COVER (#74), then Phase 3 (SCAN, FRIENDS and MOVE's code in one wave), then the private move (MOVE: the 2 current accounts onto the Rust server on the existing host and hostname, invite-only, with mail off through `EMAIL_TRANSPORT=none`, which MOVE adds; #52, the parts of #51 it needs, and CUT-R; runbook `docs/phase3/private-move.md`), then the public launch (#50, #53, #55 and the rest of #51, `docs/phase2/work-plan.md` §5), then Phase 4. Package notes go to `docs/phase3/notes/<pkg>.md`. Until the public launch, add no legal, mail-provider or branding work, except the `docs/legal/vvt.md` rows that `server/tests/legal_docs.rs` requires for a new table that references `users`.

## Commands

Setup:

- Backend: `cd backend && python3 -m venv .venv && .venv/bin/pip install -e ".[dev]"`. Python 3.12+ is required. If the system Python has no `venv` module (as on the `agents` VM), create the venv with `mise exec python@3.12 -- python -m venv .venv`.
- Frontend: `cd frontend && npm ci` (Node 20+, CI uses 22).
- Server (Rust, stable toolchain): start Postgres with `docker compose -f server/compose.yaml up -d --wait` and `export DATABASE_URL=postgres://bookclub:bookclub@127.0.0.1:5432/bookclub`. Install sqlx-cli once (`cargo install sqlx-cli --version 0.9.0 --locked --no-default-features --features rustls,postgres`), then `cd server && cargo sqlx migrate run`: the `query!` macros check against the live schema at compile time. The tests need the server too: `#[sqlx::test]` creates a throwaway database per test on it.

Server worktrees (parallel packages; `docs/phase2/work-plan.md` §2.1 rule 6):

- `#[sqlx::test]` names its databases after the test path only, so two worktrees on one Postgres server drop each other's databases. Each worktree runs its own server: `docker run -d --name bookclub-pg-<pkg> -p 127.0.0.1:<port>:5432 -e POSTGRES_USER=bookclub -e POSTGRES_PASSWORD=bookclub -e POSTGRES_DB=bookclub postgres:17 -c max_connections=200 -c shared_buffers=64MB`, with `DATABASE_URL` on that port. Phase 2 ports: ADM 5451, LEG 5452, DEP 5453, GATE 5454, CUT 5455, the CUT2 rehearsal 5456 (Phase 1 used 5441–5449); the integrator uses the compose server on 5432. `docker rm -f bookclub-pg-<pkg>` after the merge.
- A package that checks pages in a browser serves the built SPA from its server (`STATIC_DIR=../frontend/dist BIND_ADDR=0.0.0.0:<port> PUBLIC_URL=http://agents.lan:<port>`): ADM 8083, LEG 8084, CUT 8085, the DEP staging stack 8081, the GATE Caddy stack 8082, the CUT2 rehearsal 8080.
- Worktrees live next to the repo (`git worktree add ../bookclub-<pkg> -b phase2/<pkg> phase-2`), never under `/tmp`.
- Up to 5 worktrees build or test at once (the VM has 12 CPUs, 17.6 GiB RAM and 4 GiB swap). Each worktree root, and the main checkout, has an untracked `.cargo/config.toml` (git-excluded via `.git/info/exclude`) with `jobs = 3`, debug info off and that worktree's `DATABASE_URL`. With full debug info, 47 test binaries at ~125 MB each filled the disk. sqlx-cli ignores that file, so export `DATABASE_URL` for `cargo sqlx`.
- `server/.cargo/config.toml` (committed) caps test threads at 4: every `#[sqlx::test]` pool draws from one 20-connection pool per test binary, and more concurrent race tests time out waiting for each other's connections.

Checks (CI runs all of them). The Python backend and the frontend have no linter or formatter. The server uses rustfmt and clippy with warnings as errors:

```bash
cd backend && .venv/bin/python -m pytest -q
cd frontend && npm run typecheck && npm test && npm run build && npm run check:api
cd server && cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked && cargo sqlx prepare --check -- --all-targets
```

Deployment files (CI runs only the image smoke test; run the rest after touching `deploy/`, the workflows or a Caddyfile):

```bash
CARGO_JOBS=3 deploy/hosted/smoke-test.sh; docker builder prune -f   # CI's image job: build, boot with compose.yaml + ci.env, check, tear down
docker run --rm -v "$PWD":/mnt -w /mnt koalaman/shellcheck:v0.10.0 deploy/hosted/*.sh deploy/cutover/*.sh
docker run --rm -v "$PWD":/repo -w /repo rhysd/actionlint:1.7.7 .forgejo/workflows/*.yml .github/workflows/*.yml
docker compose -f deploy/hosted/compose.yaml --env-file deploy/hosted/env.example config -q
docker run --rm -v "$PWD/deploy/hosted":/etc/caddy:ro -e BOOKCLUB_HOST=bookclub.test caddy:2.10 caddy validate --config /etc/caddy/Caddyfile.example --adapter caddyfile
```

`smoke-test.sh` accepts only CI names (project `bookclub-ci…`, tag `ci…`, data directory `/tmp/bookclub-ci…`), first removes what cancelled runs left behind (older than `STALE_MINUTES`, default 120), and `--down` only tears down.

After adding or changing a `query!` macro, run `cargo sqlx prepare -- --all-targets` in `server/` and commit `server/.sqlx/`. With `SQLX_OFFLINE=true` (set in the root `Dockerfile`) the server builds from that data without a database. Run `cargo sqlx prepare` against an empty, freshly migrated database: data from a browser run can change sqlx's nullability inference (`cargo sqlx database reset -y` fixes it).

`frontend/src/api/generated.ts` is written from the server's OpenAPI document, never by hand: after changing a DTO or a utoipa description, run `npm run gen:api` in `frontend/` and commit the file (`check:api` fails otherwise).

Run a single test:

- Backend: `.venv/bin/python -m pytest -q tests/test_shelf.py::test_name` (or `-k pattern`)
- Frontend: `npx vitest run src/shelf.test.ts` (`-t "name"` filters by test name)
- Server: `cargo test --test health` (one test file) or `cargo test name_substring`. Some test files read docs and deploy files: `openapi_contract` (`endpoint-map.md` §0, `conventions.md` §14), `api_error_copy` (`apiErrors.ts`), `parity_checklist` (`docs/phase1/parity-checklist.md`), `legal_docs` (`docs/legal/`), `deploy_env` (`deploy/hosted/env.example`, `ci.env`, `smoke-test.sh`, `backup.env.example` and `backup-lib.sh`, `systemd/`, and `update.sh` and `backup.sh` against stand-ins for docker and curl), `cutover_counts` (`deploy/cutover/*.sql`), `auth_sessions` (`deploy/hosted/revoke-access.sql`).

Dev servers:

- Backend: `cd backend && .venv/bin/uvicorn app.main:app --reload --port 8000`
- Frontend: `cd frontend && npm run dev`
- Server: `cd server && DEV_MODE=true cargo run`. It applies migrations at startup and listens on `127.0.0.1:8080` (`BIND_ADDR` overrides). Without `DEV_MODE` it needs `PUBLIC_URL`, `TRUSTED_PROXIES`, `CONTACT_EMAIL`, `SMTP_URL` and `EMAIL_FROM` (all variables: `conventions.md` §11). `cargo run -- openapi` prints the OpenAPI document without a database; `cargo run -- --help` lists the other commands. `cargo run -- healthcheck` asks a running server's `/api/health` on `BIND_ADDR` (exit 0 only on 200), and `cargo run -- send-test-mail <address>` sends one mail through the configured transport (the log in DEV_MODE); neither needs a database. The log goes to stderr, a command's result to stdout.
  - Production image (the server and the built SPA, root `Dockerfile`): `docker build --build-arg CARGO_JOBS=3 -t bookclub:local .` (drop `CARGO_JOBS` when nothing else builds on the VM), then `docker builder prune -f`. `docker run --rm bookclub:local bookclub-server openapi` checks it without a database; `deploy/hosted/compose.yaml` runs it with Postgres. There is no `server/Dockerfile`; `--target server` builds only the binary.
  - Legal texts: DEV_MODE without `LEGAL_DIR` serves marked placeholders with no terms version, so sign-ups need no acceptance. `LEGAL_DIR=legal-placeholder` (from `server/`) serves the same files with terms version `placeholder`, to try the terms prompt. Outside DEV_MODE, `SIGNUP_MODE=open` refuses to start without all six files (`conventions.md` §5.9).
  - On the `agents` VM, also set `PUBLIC_URL` to the origin the browser uses. `PUBLIC_URL` is the origin the origin check allows and the email links use. In DEV_MODE it defaults to `http://localhost:5173`, so without it every cookie-authenticated write after login gets 403 `Cross-site request refused`.
  - Server opened directly (with `STATIC_DIR` set to a built SPA): `BIND_ADDR=0.0.0.0:8080 PUBLIC_URL=http://agents.lan:8080`, then open `http://agents.lan:8080`.
  - Behind Vite, which forwards the browser's `Origin` unchanged: `PUBLIC_URL=http://agents.lan:5173`.
  - Sign-up needs an invite unless `SIGNUP_MODE=open`. For the first account, `DEV_MODE=true cargo run -- beta-invite` (the same environment as the server) prints a sign-up invite code and its join link (single-use, 30 days; `--uses N --days D` change that). `DEV_MODE=true cargo run -- make-admin <username>` makes an instance admin, who can mint more under `/api/admin/invites`; `revoke-admin <username>` ends those rights and `unsuspend <username>` lifts a suspension (a suspension ends admin rights for good).

Vite on `:5173` proxies `/api` and `/media` to the Rust server on `127.0.0.1:8080`. On the `agents` VM: `cd frontend && npm run dev -- --host`, the server started as `PUBLIC_URL=http://agents.lan:5173 DEV_MODE=true cargo run`, then open `http://agents.lan:5173`. The server can stay on 127.0.0.1. The first account needs a `beta-invite` code (above).

## Architecture

### Request flow

The Vue app always calls relative `/api` and `/media`; there is never a separate API origin. It speaks only the Rust server's API: running the Python backend with the current SPA is not supported.

- `npm run build` writes `frontend/dist/` (gitignored; `emptyOutDir` wipes it). The Rust server serves it as `STATIC_DIR` (`/app/static` in the image); its fallback (`server/src/spa.rs`) answers every other non-`/api`, non-`/media` path with `index.html` and `Cache-Control: no-store`, so clients never keep a shell that points to old hashed assets. Every fallback answer carries the shell's CSP (`spa::SHELL_CSP`).
- Production still runs the Python app (v0.10.x) until the cutover (#52), with the SPA its image was built with. The root `Dockerfile` now builds the Rust image, so no Python release can be cut from this branch: a `v*` tag publishes `bookclub-server` (see "CI and releases"), and the Python image `bookclub` gets no new versions. `docker-compose.dev.yml` (the Python dev overlay, `build: .`) no longer works; CUT-R deletes it.

### Backend (`backend/app/`)

- `main.py:create_app()` builds the app. It loads settings, initialises the DB, adds middleware, and includes one router per feature from `routers/`.
- Tables live in `models.py` (SQLModel). Request and response bodies live in `schemas.py` (Pydantic). Logic shared between routers lives in `serialize.py` and the `*_ops.py` modules.
- Auth is a signed cookie session (`bookclub_session`, Starlette `SessionMiddleware`). `deps.get_current_user` reads `user_id` from it.
- Request validation errors return 400 with one flattened `detail` string, not FastAPI's default 422.

### Settings

- `config.Settings` reads env vars, and `apply_dotenv()` copies unset keys from the repo-root `.env` (`BOOKCLUB_ENV_FILE` overrides the path). Restart uvicorn after editing `.env`.
- `DEBUG` defaults to **true** when unset. With `DEBUG=0`, `runtime.prepare_environment()` refuses placeholder secrets and the `DEV-ONLY` invite. It generates `.secret_key`, `.bootstrap_invite`, and the VAPID key under the data dir.

### Schema changes (no Alembic)

- `SQLModel.metadata.create_all` creates new tables on boot.
- A new column on an existing table must also be added to `COLUMN_MIGRATIONS` in `db.py`, with SQLite DDL and a `DEFAULT` for existing rows. Existing databases never get the column otherwise.
- Data migrations run on every boot and must be idempotent; `import_legacy_pick_posts` is the pattern. `tests/test_migrations.py` covers this.

### Catalog providers

- `GOOGLE_BOOKS_API_KEY` picks the provider at startup: Google Books if the key is set, otherwise Open Library. Neither falls back to the other.
- Books are keyed by work id, and the prefix says where a book came from (regexes and helpers in `works.py`):
  - `OL…W`: Open Library
  - `ISBN…`: Google Books volume that has an ISBN
  - `GB…`: Google Books volume without an ISBN
  - `BC…`: club-only "add your own" book that never calls a catalog
- Books already in the DB are served locally first.
- Search, details, and Google responses have in-process caches.

### Frontend (`frontend/src/`)

- Stack: Vue 3 + Pinia + vue-router + vue-i18n. `@` is an alias for `src/`.
- Every HTTP call goes through `api/client.ts` (`request<T>()`, `ApiError`). Shared response types live in `types.ts`.
- Pages map 1:1 to routes in `router.ts`; old paths redirect there. Stores are in `stores/`. The legal pages `/imprint`, `/privacy` and `/terms` (`LegalPage.vue`, German aliases redirect) are top-level routes outside the AppShell, public signed in and out.
- One report sheet serves every kind of report (cover, profile picture, a book or edition a reader added): `components/ReportSheet.vue` with its `ReportTarget` prop. The admin page's tabs are query values of `/admin`, not routes: `?tab=requests` (the default), `reports`, `invites`, and `users` for the "Accounts" tab with the suspensions.
- Server HTML (the operator's legal texts) goes through `sanitizeLegalHtml` (exported from `pages/LegalPage.vue`) before `v-html`; any new place that renders HTML from the server uses it too.
- Domain logic that isn't UI sits in plain modules with colocated tests, e.g. `shelf.ts`, `physicalShelf.ts`, `diary.ts`, `drafts.ts`.

### Backend features without a frontend

The editorial-redesign merge (PR #3) removed the service worker (`frontend/public/sw.js`) and the Web Push settings. The Python push endpoints (`routers/push.py`) still exist and are tested; the Rust server drops `/api/push/*` until Phase 4 (#60). Settings keeps the notification switches (`/api/auth/notifications`) for that, but nothing sends a push. The password-change form is back in Settings (`PATCH /api/auth/password` on the Rust server). The README does not advertise push.

## Cross-cutting rules enforced by tests

- **UI copy:** every string goes into both `i18n/en.json` and `i18n/de.json`. `i18n/messages.test.ts` requires identical keys and `{placeholders}` in both. Web Push copy lives in `backend/app/i18n.py`.
- **API errors:** the server returns English `detail` strings, and `frontend/src/i18n/apiErrors.ts` maps exact strings to i18n keys. If you change a mapped server message, update the mapping too. `server/tests/api_error_copy.rs` fails if a mapped string no longer appears as a string literal in `server/src` (shared strings are `error::msg` constants).
- **Backend test isolation:** each test that uses the `client` fixture (`tests/conftest.py`) gets a fresh app and a temp DB. Outbound HTTP is faked by monkeypatching `httpx.AsyncClient` on the module under test, and the fixture resets the module-level caches and rate limits. A new module-level cache needs a reset in that fixture.
- **No third party, no inline script:** the browser loads nothing from another origin, and the shell's CSP (`server/src/spa.rs`) allows scripts from `'self'` only. `frontend/src/thirdParty.test.ts` fails on an external `url()`, `@import`, `<link>`, `<script>` or `fetch` in the sources and on an inline script or `on…` handler in `index.html`; the theme boot script is `frontend/public/theme-init.js`, a synchronous classic script in `<head>`. `server/tests/platform.rs` pins the policy.
- **References to accounts and pictures:** a new foreign key to `users` needs a section in the export (`server/tests/auth_export.rs`, `conventions.md` §5.7), plus a `NOT EXISTS` in `auth::empty_sign_ups` and a `DATA` row in `server/tests/jobs.rs`, or a `TRACES` row with the reason (`every_reference_to_an_account_is_data_or_a_trace_of_signing_in`). A new column of `users` needs a `USERS_COLUMNS` row there, and a condition in `empty_sign_ups` when it records a sign-in or keeps the account. A new foreign key to `media` needs a check in `auth::delete_rejected_cover_photos` and a `MEDIA_REFERENCES` row (`rejected_cover_photos_go_with_their_request`). A new foreign key to `users`, an operation with a `username` parameter or a schema with a rating or take field also needs a row in `server/tests/parity_visibility.rs`, with the kinds that serve it or the reason it is not personal data (`every_personal_column_route_and_rating_field_is_in_the_matrix`).
- **Mail templates:** every `pub fn … -> Email` in `server/src/email/templates.rs` goes through `mail()`, which adds the legal footer (imprint and privacy links); a source scan in `every_template_ends_with_the_legal_footer` fails otherwise.
- **Deployment settings:** a variable the server reads (`config.rs`, or `env::var` anywhere in `server/src`) needs a line in `deploy/hosted/env.example` (`#NAME=default` when production needs no value); `server/tests/deploy_env.rs` fails otherwise.
- **Python dependencies:** the released Python image installed from `backend/requirements.txt`, local setup and CI from `pyproject.toml`; keep both in step until CUT-R removes `backend/`. Nothing on this branch builds the Python image any more.

## CI and releases

- `.forgejo/workflows/` runs on the self-hosted Forgejo runner and is the active CI. Its `server` job reaches the Postgres service by the hostname `postgres`; it installs sqlx-cli, runs `cargo sqlx migrate run` before compiling, and checks `server/.sqlx/` with `cargo sqlx prepare --check`. The GitHub copy uses `localhost` with a port mapping. `.github/workflows/` is disabled on GitHub; keep it roughly in sync but don't rely on it.
- The `image` job (`ci.yml`) builds the root `Dockerfile` and boots it with `deploy/hosted/compose.yaml` and `ci.env` (`deploy/hosted/smoke-test.sh`, see "Deployment files"). It and `publish` (`release.yml`) run on the runner labelled `docker-host`, the only one that hands job containers the Docker daemon; `image` skips pull requests from forks.
- To release, push a `vX.Y.Z` tag to Forgejo (`deploy/hosted/README.md`, "Releases"). `release.yml` runs the checks and pushes the amd64 image `git.tillhome.de/rafiistcool/bookclub-server:X.Y.Z`; `deploy/hosted/release-tags.sh` then moves `X.Y` and `stable` only when the tag is the highest release of its minor line or of all (`vX.Y.Z-rc.N` moves nothing); there is no `latest`. It logs in with the secret `REGISTRY_TOKEN`. The Python image `rafiistcool/bookclub` (and older releases on GHCR) gets no new versions.
