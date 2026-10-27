# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Bookclub is a self-hosted, invite-only book club for 2–5 people. A FastAPI backend and a Vue 3 SPA ship as one container with one SQLite file. The README covers features, self-hosting, configuration, and the local demo (`app.seed_demo`). SELFHOST.md covers proxies, backups, and upgrades.

**In transition:** Bookclub is becoming a hosted multi-club service with a Rust server in `server/` and Postgres. `docs/plan.md` records the decisions, and Forgejo issues #36–#67 track the work. Until the cutover (#52), `backend/` is still what runs in production. Port its behaviour; don't extend it.

**Phase 1 design:** `docs/phase1/` holds the binding design for `server/`: `conventions.md` (architecture, frozen interfaces, coding rules), `endpoint-map.md` (routes, owners, shared DTOs, error strings), `legacy-mapping.md` (importer), `work-plan.md` (waves, packages, file ownership) and `spec/` (behaviour of the Python app per area). The schema is `server/migrations/0001_init.sql`.

## Commands

Setup:

- Backend: `cd backend && python3 -m venv .venv && .venv/bin/pip install -e ".[dev]"`. Python 3.12+ is required. If the system Python has no `venv` module (as on the `agents` VM), create the venv with `mise exec python@3.12 -- python -m venv .venv`.
- Frontend: `cd frontend && npm ci` (Node 20+, CI uses 22).
- Server (Rust, stable toolchain): start Postgres with `docker compose -f server/compose.yaml up -d --wait` and `export DATABASE_URL=postgres://bookclub:bookclub@127.0.0.1:5432/bookclub`. Install sqlx-cli once (`cargo install sqlx-cli --version 0.9.0 --locked --no-default-features --features rustls,postgres`), then `cd server && cargo sqlx migrate run`: the `query!` macros check against the live schema at compile time. The tests need the server too: `#[sqlx::test]` creates a throwaway database per test on it.

Server worktrees (Phase 1 Wave 2, `work-plan.md` ground rule 5):

- `#[sqlx::test]` names its databases after the test path only, so two worktrees on one Postgres server drop each other's databases. Each worktree runs its own server: `docker run -d --name bookclub-pg-<pkg> -p 127.0.0.1:<port>:5432 -e POSTGRES_USER=bookclub -e POSTGRES_PASSWORD=bookclub -e POSTGRES_DB=bookclub postgres:17 -c max_connections=200 -c shared_buffers=64MB`, with `DATABASE_URL` on that port. Ports: CAT 5441, HAR 5442, MED 5443, SHELF 5444, POSTS 5445, CLUB 5446, IMP 5447, OAPI 5448, PAR 5449; the integrator, W1 and FE use the compose server on 5432. `docker rm -f bookclub-pg-<pkg>` after the merge.
- Worktrees live next to the repo (`git worktree add ../bookclub-<pkg> -b phase1/<pkg> phase-1`), never under `/tmp`.
- At most 3 worktrees build or test at once, each with `CARGO_BUILD_JOBS=2` (the VM has 6 CPUs and 7.8 GiB RAM, no swap).

Checks (CI runs all of them). The Python backend and the frontend have no linter or formatter. The server uses rustfmt and clippy with warnings as errors:

```bash
cd backend && .venv/bin/python -m pytest -q
cd frontend && npm run typecheck && npm test && npm run build
cd server && cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked && cargo sqlx prepare --check -- --all-targets
```

After adding or changing a `query!` macro, run `cargo sqlx prepare -- --all-targets` in `server/` and commit `server/.sqlx/`. With `SQLX_OFFLINE=true` (set in the Dockerfile) the server builds from that data without a database.

Run a single test:

- Backend: `.venv/bin/python -m pytest -q tests/test_shelf.py::test_name` (or `-k pattern`)
- Frontend: `npx vitest run src/shelf.test.ts` (`-t "name"` filters by test name)
- Server: `cargo test --test health` (one test file) or `cargo test name_substring`

Dev servers:

- Backend: `cd backend && .venv/bin/uvicorn app.main:app --reload --port 8000`
- Frontend: `cd frontend && npm run dev`
- Server: `cd server && DEV_MODE=true cargo run`. It applies migrations at startup and listens on `127.0.0.1:8080` (`BIND_ADDR` overrides). Without `DEV_MODE` it needs `PUBLIC_URL`, `TRUSTED_PROXIES`, `CONTACT_EMAIL`, `SMTP_URL` and `EMAIL_FROM` (all variables: `conventions.md` §11). `cargo run -- openapi` prints the OpenAPI document without a database; `cargo run -- --help` lists the other commands. Image: `docker build -t bookclub-server server`.
  - On the `agents` VM, also set `PUBLIC_URL` to the origin the browser uses. `PUBLIC_URL` is the origin the origin check allows and the email links use. In DEV_MODE it defaults to `http://localhost:5173`, so without it every cookie-authenticated write after login gets 403 `Cross-site request refused`.
  - Server opened directly (with `STATIC_DIR` set to a built SPA): `BIND_ADDR=0.0.0.0:8080 PUBLIC_URL=http://agents.lan:8080`, then open `http://agents.lan:8080`.
  - Behind Vite, which forwards the browser's `Origin` unchanged: `PUBLIC_URL=http://agents.lan:5173`.
  - Sign-up needs an invite unless `SIGNUP_MODE=open`. For the first account, `DEV_MODE=true cargo run -- beta-invite` (the same environment as the server) prints a sign-up invite code and its join link (single-use, 30 days; `--uses N --days D` change that). `DEV_MODE=true cargo run -- make-admin <username>` makes an instance admin, who can mint more under `/api/admin/invites`.

Vite on `:5173` proxies `/api` to `127.0.0.1:8000`. On the `agents` VM, add `-- --host` to the Vite command and open `http://agents.lan:5173`. The backend can stay on 127.0.0.1. First login on an empty DB uses invite `DEV-ONLY`, which needs `DEBUG=1`.

## Architecture

### Request flow

The Vue app always calls relative `/api`; there is never a separate API origin.

- In production, uvicorn serves both the API and the built SPA. `npm run build` writes into `backend/app/static/` (gitignored; `emptyOutDir` wipes it).
- The catch-all in `app/main.py` serves `index.html` with `Cache-Control: no-store`, so clients never keep a shell that points to old hashed assets.

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
- Pages map 1:1 to routes in `router.ts`; old paths redirect there. Stores are in `stores/`.
- Domain logic that isn't UI sits in plain modules with colocated tests, e.g. `shelf.ts`, `physicalShelf.ts`, `diary.ts`, `drafts.ts`.

### Backend features without a frontend

The editorial-redesign merge (PR #3) removed the service worker (`frontend/public/sw.js`), the Web Push settings, and the password-change form. Their backend endpoints still exist and are tested: `routers/push.py` and `PATCH /api/auth/password`. The README no longer advertises either feature.

## Cross-cutting rules enforced by tests

- **UI copy:** every string goes into both `i18n/en.json` and `i18n/de.json`. `i18n/messages.test.ts` requires identical keys and `{placeholders}` in both. Web Push copy lives in `backend/app/i18n.py`.
- **API errors:** the backend returns English `detail` strings, and `frontend/src/i18n/apiErrors.ts` maps exact strings to i18n keys. If you change a mapped backend message, update the mapping too. `backend/tests/test_api_error_copy.py` fails if a mapped string no longer appears in `backend/app`, and `server/tests/api_error_copy.rs` if it no longer appears as a string literal in `server/src` (shared strings are `error::msg` constants).
- **Backend test isolation:** each test that uses the `client` fixture (`tests/conftest.py`) gets a fresh app and a temp DB. Outbound HTTP is faked by monkeypatching `httpx.AsyncClient` on the module under test, and the fixture resets the module-level caches and rate limits. A new module-level cache needs a reset in that fixture.
- **Dependencies:** the Docker image installs from `backend/requirements.txt`, while local setup and CI install from `pyproject.toml`. Add a new runtime dependency to both files. No test catches a dependency missing from `requirements.txt`: CI stays green, and the image only breaks when it is built on a release tag.

## CI and releases

- `.forgejo/workflows/` runs on the self-hosted Forgejo runner and is the active CI. Its `server` job reaches the Postgres service by the hostname `postgres`; it installs sqlx-cli, runs `cargo sqlx migrate run` before compiling, and checks `server/.sqlx/` with `cargo sqlx prepare --check`. The GitHub copy uses `localhost` with a port mapping. `.github/workflows/` is disabled on GitHub; keep it roughly in sync but don't rely on it.
- To release, push a `vX.Y.Z` tag to Forgejo. The release workflow pushes an amd64 image to the `git.tillhome.de` registry. Older releases are still on GHCR, which no longer receives new versions.
