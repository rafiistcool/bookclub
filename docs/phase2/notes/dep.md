# DEP (#51): production image, compose, release and backups

Owner: DEP. These notes record what DEP built, the decisions behind it, the contract changes the integrator moves into `conventions.md`, `CLAUDE.md` and `README.md`, and what is left. DEP runs in two parts; this file covers part 1 (the image and its runtime). Part 2 (release and CI workflows, backups, the rest of the runbook) adds its own section.

## Part 1: the image and its runtime

### Files

| File | What |
|---|---|
| `Dockerfile` (root) | The production image. It replaces the Python Dockerfile (Q6 = A). |
| `.dockerignore` (root) | An allowlist: only `frontend/` and `server/` enter the context. |
| `server/Dockerfile`, `server/.dockerignore` | **Deleted** (see "Decisions"). |
| `server/src/healthcheck.rs` | `bookclub-server healthcheck`, `mod healthcheck;` in `main.rs` (amendment 6). |
| `server/src/main.rs` | The two new commands, the `MEDIA_DIR` check in `serve` and `import-legacy`, and the log on stderr. |
| `server/src/media/shared/store.rs` | `FsMediaStore::check_writable`. |
| `server/src/email/mod.rs` | The explicit `Message-ID` on every SMTP mail, and `email::test_mail`. |
| `deploy/hosted/compose.yaml` | The app and Postgres for one VM. |
| `deploy/hosted/env.example` | Every setting, for compose and the server. |
| `deploy/hosted/README.md` | The first runbook: first boot, the first account, updates and rollback, staging, start-up errors. |
| `server/tests/deploy_env.rs` | The deployment contract, checked without Docker (13 tests). |

### The image (`Dockerfile`)

- **Stages:**
  1. `node:22-trixie-slim` runs `npm ci` and `npm run build` (`vite build`) in `frontend/`. Debian rather than Alpine, so npm installs the same glibc builds of the native packages (rollup, esbuild) as CI.
  2. `rust:1.99-slim-trixie` runs `cargo build --release --locked` with `SQLX_OFFLINE=true`. The dependencies build first, in a layer of their own, against stub `main.rs` and `lib.rs`; a source change then rebuilds only the crate. This layer is what part 2's registry build cache can reuse (BuildKit cache mounts are not exported to a registry cache). `--build-arg CARGO_JOBS=N` caps the rustc jobs; unset means one per CPU.
  3. `debian:trixie-slim` with `ca-certificates`, the binary at `/usr/local/bin/bookclub-server`, and `frontend/dist` at `/app/static`.
- **Runtime:**
  - `ENV STATIC_DIR=/app/static MEDIA_DIR=/data/media BIND_ADDR=0.0.0.0:8080`, `WORKDIR /app`, `USER 1000:1000` (numeric), `EXPOSE 8080`, `CMD ["bookclub-server", "serve"]`.
  - A `bookclub` user and group with id 1000. `/data/media` belongs to 1000:1000 with mode 0750; `/data` stays root's.
  - `HEALTHCHECK --interval=20s --timeout=5s --start-period=60s --start-interval=2s --retries=3 CMD ["bookclub-server", "healthcheck"]`. Three misses in a row mark the container unhealthy, between 40 and about 65 s after the database goes away (measured: 49 s). `--start-interval` needs Docker Engine 25 or newer.
  - OCI labels: `title`, `description`, and `source`, `url` and `documentation` pointing at `https://git.tillhome.de/rafiistcool/bookclub` (the README under `src/branch/main/deploy/hosted/README.md`). `version`, `revision` and `created` come from part 2's `metadata-action`. The repo has no licence file, so there is no `licenses` label.
- The binary links only glibc, libm and libgcc_s (`ldd`): rusqlite and libwebp are built in statically. The image is 187 MB.

### `.dockerignore`

It excludes everything (`*`) and lets in `frontend/` and `server/`. Inside them it drops `frontend/node_modules`, `frontend/dist`, `server/target`, `server/.cargo`, `server/data` and `server/.env`. `.git`, the root `.cargo/` (the untracked worktree settings), `backend/`, `docs/`, `deploy/` and `data/` never enter the context. The build context measured 5.94 MB on this VM, where `server/target` alone is many GB.

The old file dropped `*.md` everywhere. The new one keeps every file under `server/`, so LEG's `server/legal-placeholder/` is in the context whatever its extension (work-plan line 378 assumes the old rule; nothing needs to change).

### Decisions

- **`server/Dockerfile` is deleted, with `server/.dockerignore`.** The root image holds the same binary. A second Dockerfile would need its own copy of the runtime stage (uid 1000, `/data/media`, the healthcheck, the labels, `STATIC_DIR`), which would drift from the one that ships. No workflow, script or test builds it: `tests/catalog_no_google.rs` lists `Dockerfile` among the files it scans but skips a missing file. The CUT2 rehearsal builds with cargo. Whoever needs only the binary can build the root file's `server` stage: `docker build --target server -t bookclub-build .`, with the binary at `/usr/local/bin/bookclub-server`.
- **Logs go to stderr.** `main.rs` now sets `with_writer(std::io::stderr)` and colours only on a terminal (`with_ansi(stderr().is_terminal())`). Before, the log went to stdout with ANSI codes: `beta-invite` printed sqlx's `relation "_sqlx_migrations" already exists` notice before the code, so `docker compose exec app bookclub-server beta-invite | head -1` returned a log line, and `docker logs` held escape codes. `tests/clubs_invites.rs` passed only because it sets `RUST_LOG=error`.
- **The `Message-ID` applies to every mail**, not only the test mail: `SmtpEmail::message` sets `<uuid@domain>`, where the domain is that of `EMAIL_FROM` and the UUID is version 4 from `auth::random` (no `uuid` crate). The work plan assumed lettre's default names the host; in lettre 0.11.23 `Message::builder().build()` adds no `Message-ID` at all, only `Date`. Until now every mail went out without one, and the provider or the receiving server added its own.
- **The `MEDIA_DIR` check** runs in `serve` and `import-legacy`, after `Config::from_env` and before the database connection, so a wrong mount fails at once instead of after the 60 s connect retry. `rematch-legacy`, `make-admin` and `beta-invite` store no media and skip it. `FsMediaStore::check_writable` creates the directory, writes `.write-probe.<16 hex>` and removes it. The error is `MEDIA_DIR <path> is not writable: <cause>`.
- **`healthcheck`** reads only `BIND_ADDR` (default `127.0.0.1:8080`, the same as `config.rs`), maps `0.0.0.0` to `127.0.0.1` and `[::]` to `[::1]`, and GETs `/api/health` with reqwest: 3 s for the whole request, no proxy (an `HTTP_PROXY` in the environment must not catch it). It never calls `Config::from_env` and never connects to the database. Exit 0 and `GET <url> answered 200 OK` on stdout only on 200; otherwise exit 1 and `error: GET <url> answered 503 Service Unavailable` (or the connection error) on stderr, which `docker inspect` shows in the health log.
- **`send-test-mail <address>`** reads the full configuration, as `serve` does, but never connects to the database. The address must pass `auth::rules::valid_email`. It sends through the configured transport (`LogEmail` in DEV_MODE, `SmtpEmail` otherwise) and exits 1 with `error: SMTP send failed: …` when the SMTP server refuses. The mail is English: subject `<SERVICE_NAME> test mail`, a body naming the service and `PUBLIC_URL`, and no link that does anything. It is built in `email/mod.rs` (`email::test_mail`), not in ADM's `templates.rs` (amendment 6).

### `deploy/hosted/compose.yaml`

As in the work plan, plus these choices:

- `name: bookclub`, so the volume is `bookclub_postgres-data` wherever the file lies. `-p bookclub-staging` still overrides it.
- **Postgres** sits only on the network `db`, which is `internal: true` (no route out, no published port). The app is on `db` and `default`.
- The app's port is `${BOOKCLUB_LISTEN:-0.0.0.0}:${BOOKCLUB_PORT:-8080}:8080` (amendment 7).
- **The bind mounts** use the long syntax with `create_host_path: false`, under `${BOOKCLUB_DATA_DIR:-/srv/bookclub}` (`/media` → `/data/media`, `/legal` → `/app/legal`, read-only). A missing directory stops `docker compose up` instead of being created as root, and `BOOKCLUB_DATA_DIR` lets staging, CI and tests use another directory.
- **`env_file`** is `${BOOKCLUB_ENV_FILE:-.env}` with `required: false`. Compose reads `.env` next to `compose.yaml` both for interpolation and, through `env_file`, as the app's environment. `required: false` lets `docker compose -f deploy/hosted/compose.yaml --env-file deploy/hosted/env.example config -q` pass in the repo, where no `.env` exists. On the VM a missing `.env` already stops compose at `${POSTGRES_PASSWORD:?…}`, and the server refuses to start without `DATABASE_URL`. Part 2's CI sets `BOOKCLUB_ENV_FILE=ci.env` inside `ci.env`.
- `DATABASE_URL=postgres://bookclub:${POSTGRES_PASSWORD}@postgres:5432/bookclub` in the env file: compose interpolates `${…}` inside `env_file` files from the project environment (checked with Compose v5.6.1), so the password is set once.
- The rest as planned: `read_only`, `tmpfs: /tmp:size=64m` (the server writes no temporary files), `cap_drop: [ALL]`, `no-new-privileges`, json-file logs (10 MB × 5), `stop_grace_period: 30s`, `restart: unless-stopped`, `depends_on: postgres: service_healthy`, and a `pg_isready` healthcheck with `start_interval: 1s`.
- It needs Compose 2.24 or newer (`env_file` with `path` and `required`) and Engine 25 or newer (`start_interval`). The README says so.

### `deploy/hosted/env.example` and `tests/deploy_env.rs`

- Every variable `config.rs` reads is named: set where production needs a value, otherwise as `#NAME=default` (no space after the `#`). It also names `RUST_LOG`, `LEGAL_DIR=/app/legal`, `POSTGRES_PASSWORD`, `DNB_HARVEST=false`, `SERVICE_NAME=Bookclub`, and the compose variables `BOOKCLUB_TAG`, `BOOKCLUB_LISTEN`, `BOOKCLUB_PORT`, `BOOKCLUB_DATA_DIR` and `#BOOKCLUB_ENV_FILE`.
- Placeholders are `CHANGE_ME`. `TRUSTED_PROXIES=CHANGE_ME` makes an unfilled copy fail at startup (`TRUSTED_PROXIES entry "CHANGE_ME" is not a CIDR or an IP address`). There is no valid-looking default: a wrong value would quietly put every user in one rate-limit bucket.
- `tests/deploy_env.rs`:
  - **env.example:**
    - `env_example_names_every_variable_the_server_reads`: `get("NAME")` in `config.rs`, `env::var("NAME")` anywhere in `server/src`, and `RUST_LOG`.
    - `env_example_names_only_variables_someone_reads`: the server, `compose.yaml`'s `${NAME}`, or `LEGAL_DIR`. That allowlist entry can go once LEG's `config.rs` reads `LEGAL_DIR`.
    - `env_example_names_every_compose_variable`.
    - `env_example_sets_what_production_needs`: no name twice, the required variables set, `DNB_HARVEST=false`, `LEGAL_DIR=/app/legal`, and `DATABASE_URL` built from `POSTGRES_PASSWORD`.
    - `env_example_filled_in_is_a_production_configuration`: as shipped, `Config::from_lookup` refuses it over `TRUSTED_PROXIES`. With that one value filled in it is accepted, with no DEV_MODE, Secure cookies, SMTP and no harvest.
  - **The binary**, run with an empty environment:
    - healthcheck: a 200 passes; a 503, a 30 s delay (it gives up after 3 s) and a closed port fail. The mock server is wiremock on `127.0.0.1`, reached through `BIND_ADDR=0.0.0.0:<port>`.
    - send-test-mail against a fake SMTP server in the test, with no DEV_MODE: one message with `Message-ID: <uuid@mail.example.com>`, `Date`, `From`, `To` and `Subject`, and stdout `sent a test mail to …`. A 550 on `RCPT` exits non-zero with `SMTP send failed` and `550`. A mailbox such as `Ada <ada@example.test>` is refused.
    - the log on stderr, without escape codes, and only the result on stdout.
    - `serve` and `import-legacy` with `MEDIA_DIR` below a regular file exit non-zero with the message, before the database. The test fails at its 20 s limit if the check ever moves after the connect retry.
  - **Unit tests:**
    - `healthcheck::tests`: the URL for each `BIND_ADDR` form, and a bad `BIND_ADDR` names the variable.
    - `email::tests`: the `Message-ID` shape, domain and uniqueness, and the test mail's text.
    - `media::shared::store::tests`: the check creates the root and leaves nothing behind, and its error is exactly `MEDIA_DIR <path> is not writable`.
    - `main.rs`: the new commands parse.

### Contract changes for the integrator

No routes, DTOs, `error::msg` strings, migrations or frontend files changed. `generated.ts` and `server/.sqlx/` are unchanged.

- **`conventions.md` §3, "Startup (`serve`)":** after step 2, add "2a. `FsMediaStore::new(media_dir).check_writable()`: create `MEDIA_DIR`, write and remove a probe file, else exit with `MEDIA_DIR <path> is not writable` (also `import-legacy`)".
- **`conventions.md` §9:** `SmtpEmail` sets `Message-ID: <uuid-v4@EMAIL_FROM's domain>`; lettre adds `Date`. `email::test_mail(config, to)` is the `send-test-mail` message.
- **`conventions.md` §11:** the `BIND_ADDR` row's "(the Docker image sets `0.0.0.0:8080`)" now refers to the root `Dockerfile`. The image also sets `STATIC_DIR=/app/static` and `MEDIA_DIR=/data/media`. `deploy/hosted/env.example` lists every variable, and `tests/deploy_env.rs` keeps it complete.
- **`conventions.md` §16, "The CLI":** add `healthcheck` (reads only `BIND_ADDR`, GETs `/api/health` with a 3 s timeout, exit 0 only on 200; no config, no database) and `send-test-mail <address>` (full config, no database; exit 1 on an SMTP error).
- **`conventions.md` §19:** the log goes to stderr, with colours only on a terminal; stdout carries only command results.
- **`CLAUDE.md`:**
  - "Image: `docker build -t bookclub-server server`" becomes `docker build -t bookclub:local .` (the root `Dockerfile`: the server and the SPA). Add `--build-arg CARGO_JOBS=3` when other builds run on the VM, and run `docker builder prune -f` afterwards.
  - "With `SQLX_OFFLINE=true` (set in the Dockerfile)" now means the root `Dockerfile`.
  - Request flow: "Do not cut a Python release from `phase-1`: the root `Dockerfile` still copies `backend/app/static`…" is out of date. The root `Dockerfile` now builds the Rust image, and a `v*` tag must not run the old `release.yml` before part 2 rewrites it.
  - The server CLI list: `cargo run -- healthcheck` and `cargo run -- send-test-mail <address>`.
  - A pointer to `deploy/hosted/README.md` for deployment.
- **`README.md:51`** ("the root `Dockerfile` fails on purpose…") and **`SELFHOST.md:41`** (`docker build -t bookclub .`): the root `Dockerfile` now builds the Rust image, which the Python setup cannot run. **`docker-compose.dev.yml`** (`build: .`, the Python contributors' overlay) now builds that image too, so the Python dev overlay no longer works. CUT-R deletes it; until then the docs should not point to it.
- **`server/tests/catalog_no_google.rs`** (CAT's file) scans `server/Dockerfile`, which no longer exists. To keep the image in the scan, point it at `../Dockerfile` and `../deploy/hosted/compose.yaml`.

### Verified by hand (part 1, on the agents VM)

**Build**
- `docker build --build-arg CARGO_JOBS=3 -t bookclub:local .` took 4 min 12 s with a context of 5.94 MB. After a change in `server/src` the rebuild took 2 min: the dependency layer was cached. The image is 187 MB.
- `docker run --rm bookclub:local bookclub-server openapi` printed the document (89 paths) without a database.

**Boot with `docker run`**
- Setup:
  - `--read-only --tmpfs /tmp --cap-drop ALL --security-opt no-new-privileges:true`
  - `--network host` with `BIND_ADDR=0.0.0.0:8081`
  - a throwaway `postgres:17` on 127.0.0.1:5450
  - a media bind mount owned by uid 1000
  - production settings (no DEV_MODE; `COOKIE_SECURE=false` only for plain http)
  - `SMTP_URL=smtp://127.0.0.1:1025`, Mailpit (`axllent/mailpit`, SMTP on 127.0.0.1:1025, web on 0.0.0.0:8025)
- Results:
  - The container was `healthy` 2 s after the start, thanks to `--start-interval`.
  - `curl http://agents.lan:8081/api/health` answered `{"status":"ok"}`.
  - `/` and a deep SPA path answered 200 `text/html; charset=utf-8` with `Cache-Control: no-store`. `/assets/index-….js` answered `public, max-age=31536000, immutable`, and a missing asset a JSON 404.
  - `beta-invite`, then a sign-up through the API (201), then an avatar upload (200). `/media/…/a` served a 256×256 WebP, immutable, and the file was in the bind mount.
  - The verification mail and `docker exec … bookclub-server send-test-mail` arrived in Mailpit. Both carried `Message-Id: <uuid@staging.example.com>`, a `Date`, and `From: "Bookclub Staging" <noreply@staging.example.com>`.
  - `send-test-mail` with `SMTP_URL` on a closed port exited 1 with `error: SMTP send failed: Connection error: Connection refused (os error 111)…`.
  - `docker stop` on Postgres turned the container `unhealthy` after 49 s. The health log read `error: GET http://127.0.0.1:8081/api/health answered 503 Service Unavailable`, and `/api/health` answered 503 `{"status":"unavailable"}`. After `docker start` the container was `healthy` again within 8 s.
  - A media bind mount that Docker created (owned by root) made `serve` exit 1 with `error: MEDIA_DIR /data/media is not writable: Permission denied (os error 13)`.

**Compose, with the rebuilt image (log on stderr)**
- `docker compose -f deploy/hosted/compose.yaml --env-file deploy/hosted/env.example config -q` exited 0.
- `-p bookclub-staging` ran with a filled copy of `env.example` outside the repo: tag `dep-local`, port 8081, its own `BOOKCLUB_DATA_DIR` and `BOOKCLUB_ENV_FILE`, `TRUSTED_PROXIES=none`, and `SMTP_URL=smtp://localhost:25`, a loopback address where nothing listens.
  - Without the data directories, `up` failed with `bind source path does not exist: …/media`. With them, `up -d --wait` exited 0 in about 10 s. `DATABASE_URL` came from `${POSTGRES_PASSWORD}` inside the env file.
  - Through `docker compose exec app`, from the README:
    - `bookclub-server healthcheck` printed `GET http://127.0.0.1:8080/api/health answered 200 OK`.
    - `beta-invite` wrote exactly two lines to stdout (code and link), with the sqlx notice on stderr.
    - `send-test-mail` exited 1, as expected without an SMTP server.
    - `make-admin grace` worked.
  - In the browser (playwright-cli, headless Chromium): the join link led to "Create an account" and the sign-up form, and the new account landed on the home page. A profile picture uploaded in Settings showed as `/media/…/a` (256 px), and its file was in the bind mount.
  - The app container ran with `Uid 1000`, `CapEff 0` and `NoNewPrivs 1` on a read-only root (`touch /app/x`: read-only file system).
  - Postgres has no published port and no route out (`/dev/tcp/1.1.1.1/443`: network is unreachable).
  - `docker logs` held no escape codes.
  - `docker compose stop app` exited 0 after 7 s (`shutting down`).
- Cleanup: `down -v`, then `docker rm -fv` of the Postgres and Mailpit containers. The registry-named tag was removed, and `docker builder prune -f` ran.

**Not verified:**
- the pull from `git.tillhome.de`: the tag exists only after part 2's release workflow
- `[::]` as `BIND_ADDR` inside a container (unit-tested only)
- the Compose and Engine versions on the production VM

## Left for part 2

- The release workflow rewrite (`.forgejo/workflows/release.yml`, mirrored to `.github/`): image `git.tillhome.de/rafiistcool/bookclub-server`, tags `X.Y.Z`, `X.Y` and `stable` (not for prereleases), `flavor: latest=false`, the registry build cache, no Python job, and the stale GHCR comment. Until then, a `v*` tag runs the old workflow against the new Dockerfile.
- The CI image smoke job with `deploy/hosted/ci.env` (it sets `BOOKCLUB_ENV_FILE=ci.env`, `BOOKCLUB_DATA_DIR` and `BOOKCLUB_TAG`).
- `backup.sh`, `restore-test.sh`, `backup.env.example`, and the systemd units and timers.
- The README sections on the import, backup and restore, and the `SIGNUP_MODE=open` switch, plus GATE's Caddy section (amendment 16).
- shellcheck and actionlint runs; "every command in the README runs as written against `bookclub-staging`" once the README is complete.
