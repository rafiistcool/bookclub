# DEP (#51): production image, compose, release and backups

Owner: DEP. These notes record what DEP built, the decisions behind it, the contract changes the integrator moves into `conventions.md`, `CLAUDE.md` and `README.md`, and what is left. DEP ran in two parts: part 1 is the image and its runtime, part 2 the release and CI workflows, the backups and the rest of the runbook.

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
| `server/tests/deploy_env.rs` | The deployment contract, checked without Docker (15 tests after part 2 and the review). |

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
- The rest as planned: `read_only`, `tmpfs: /tmp:size=64m` (the server writes no temporary files), `cap_drop: [ALL]`, `no-new-privileges`, json-file logs (10 MB × 5), `stop_grace_period: 30s`, `restart: ${BOOKCLUB_RESTART:-unless-stopped}` (a variable since the review, so CI can set `no`), `depends_on: postgres: service_healthy`, and a `pg_isready` healthcheck with `start_interval: 1s`.
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
    - healthcheck: a 200 passes; a 503, a 30 s delay (it gives up after 3 s) and a closed port fail. The mock server is wiremock on `127.0.0.1`, reached through `BIND_ADDR=0.0.0.0:<port>`. Every run has `HTTP_PROXY`, `http_proxy` and `ALL_PROXY` set to `http://127.0.0.1:1`, where nothing listens, so the 200 case passes only while `healthcheck.rs` keeps `.no_proxy()` (reqwest 0.13.5 does not bypass loopback on its own; without the call the 200 and the 3 s cases fail).
    - send-test-mail against a fake SMTP server in the test, with no DEV_MODE: one message with `Message-ID: <uuid@mail.example.com>`, `Date`, `From`, `To` and `Subject`, and stdout `sent a test mail to …`. A 550 on `RCPT` exits non-zero with `SMTP send failed` and `550`. A mailbox such as `Ada <ada@example.test>` is refused.
    - the log on stderr, without escape codes, and only the result on stdout.
    - `serve` and `import-legacy` with `MEDIA_DIR` below a regular file, and with `MEDIA_DIR=/proc`, exit non-zero with the message, before the database. `/proc` exists but takes no new file even from uid 0, as a root-owned bind mount is for uid 1000: only the probe file catches it, so a `check_writable` cut down to `create_dir_all` fails this case (it then runs into the 20 s limit). The test fails at its 20 s limit if the check ever moves after the connect retry.
  - **Unit tests:**
    - `healthcheck::tests`: the URL for each `BIND_ADDR` form, and a bad `BIND_ADDR` names the variable.
    - `email::tests`: the `Message-ID` shape, domain and uniqueness, and the test mail's text.
    - `media::shared::store::tests`: the check creates the root and leaves nothing behind, its error is exactly `MEDIA_DIR <path> is not writable`, and `MEDIA_DIR /proc is not writable` for an existing root that refuses the probe file (Linux only).
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

## Part 2: workflows, backups and the runbook

### Files

| File | What |
|---|---|
| `.forgejo/workflows/release.yml` | Rewritten: the hosted image, `bookclub-server`. |
| `.github/workflows/release.yml` | Its GitHub copy (disabled on GitHub): the same jobs, `ghcr.io/<owner>/bookclub-server`, a `gha` cache. |
| `.forgejo/workflows/ci.yml`, `.github/workflows/ci.yml` | A new `image` job, identical in both. The Python `backend` job stays. |
| `deploy/hosted/smoke-test.sh` | The `image` job's steps, runnable locally. |
| `deploy/hosted/release-tags.sh` | The release's floating tags `X.Y` and `stable`, moved after the push (review fix). |
| `deploy/hosted/ci.env` | The settings the smoke test boots `compose.yaml` with. |
| `deploy/hosted/backup.sh`, `restore-test.sh`, `backup-lib.sh` | The nightly backup, the weekly restore test, and what both share. |
| `deploy/hosted/backup.env.example` | Their settings. |
| `deploy/hosted/systemd/bookclub-{backup,restore-test}.{service,timer}` | The units and timers. |
| `deploy/hosted/README.md` | Completed: releases, the reverse proxy (pointer), the legacy import, backups, restore, open sign-up. |
| `server/tests/deploy_env.rs` | `ci_env_is_a_production_configuration`; the env-file parser now takes a path; `smoke_test_refuses_names_that_are_not_ci_names` (review fix; 15 tests). |

### The release workflow (`.forgejo/workflows/release.yml`)

- **Jobs:**
  - `frontend` is unchanged.
  - `server` is a byte-for-byte copy of `ci.yml`'s server job (`diff` against `ci.yml` is empty in both directories).
  - `publish` needs both and runs on the runner labelled `docker-host` (see "The runner"). The `backend` (pytest) job is gone, and line 4's GHCR comment now names the Forgejo registry.
- **`publish`:**
  1. The tag must match `^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$`, else the run fails. metadata-action only warns about a tag it cannot parse, such as `v1.2`, and would then leave the build without a tag.
  2. `docker/setup-buildx-action@v4`: the docker-container driver. The default `docker` driver cannot export a registry cache.
  3. `docker/login-action@v4` to `git.tillhome.de` with `${{ github.repository_owner }}` and `secrets.REGISTRY_TOKEN`, as before.
  4. `docker/metadata-action@v6` for `git.tillhome.de/rafiistcool/bookclub-server` (the workflow-level `env.IMAGE`), with `flavor: latest=false` and the one tag `type=semver,pattern={{version}}`. The floating tags follow in step 6. There is no `{{major}}` tag. Its labels set `title`, `description` and `documentation` to the Dockerfile's values; metadata-action adds `version`, `revision`, `created`, `source` and `url`.
  5. `docker/build-push-action@v7`: `linux/amd64`, `provenance: false` and `sbom: false`, so each tag is one plain image manifest without an attestation index next to it. The cache is `cache-from: type=registry,ref=…:buildcache`, and `cache-to` the same with `mode=max,image-manifest=true,oci-mediatypes=true`.
     - `mode=max` keeps the stages' layers, including the Rust dependency layer that part 1 split out.
     - The two OCI options store the cache as a plain OCI image, which registries without BuildKit's cache manifest type accept.
     - The cache appears as the tag `buildcache` of `bookclub-server` in the Forgejo package list.
  6. **Floating tags** (review fix): `deploy/hosted/release-tags.sh "$IMAGE" "$GITHUB_REF_NAME" <digest of step 5>`.
     - It lists the release tags `vX.Y.Z` (no pre-releases) on the remote at that moment (`git ls-remote --tags --refs origin 'v*'`, so the shallow checkout is enough), sorted with `sort -V`.
     - `X.Y` moves when the tag is the highest of its minor line, `stable` when it is the highest of all, and a pre-release moves nothing.
     - It moves them with `docker buildx imagetools create --prefer-index=false --tag IMAGE:<name> IMAGE@<digest>`: a registry-side copy of the plain manifest, no pull, no index around it.
     - **Why after the push, from the remote's tags:** before, `stable` was set by every non-pre-release tag, and two runs (`concurrency` is per ref) could end in either order. A hotfix `v0.11.1` minutes after `v0.11.0`, or a backport `v0.10.5`, could leave `stable` on the older version, and a pull of `stable` after a migration then stops at startup (`migration N was previously applied but is missing`). Now a run that is no longer the highest moves nothing, whichever run ends last. The decision falls seconds before the retag, while a newer release's run needs its full checks and build first.
     - **Not a workflow-wide `concurrency` group:** a newer pending run cancels an older pending one in the same group, which would drop that release. The per-ref group stays, with a comment saying so.
     - If the newest release's run fails, `stable` stays on the release before it until that version is released again (README, "Releases").
- **The GitHub copy** pushes to `ghcr.io/${{ github.repository_owner }}/bookclub-server` with `GITHUB_TOKEN`, caches with `type=gha`, and runs `server` against `localhost`, as `ci.yml` there does. It is amd64 only now (it was amd64 and arm64 through QEMU, which a Rust release build would make very slow).
- **The GitHub copy** runs `publish` on `ubuntu-latest` (GitHub's hosted VMs have their own daemon) and the same `release-tags.sh` step.
- **`REGISTRY_TOKEN`** (item 23, done 2026-10-29): an action secret of the user `rafiistcool`, a token with `write:package`, set on 2026-10-18. The repository's secret list does not show user secrets. Without the secret, `publish` would fail at the login, after the tests.

### The CI image smoke job (`image` in `ci.yml`)

- **What it runs:** `deploy/hosted/smoke-test.sh`:
  1. `docker build --pull` of the root `Dockerfile`, tagged with the image of `compose.yaml` (`git.tillhome.de/rafiistcool/bookclub-server:$BOOKCLUB_TAG`, read from `docker compose config --images`). Nothing is pushed.
  2. The data directory with `media/` and `legal/` for uid 1000.
  3. `docker compose -f deploy/hosted/compose.yaml --env-file deploy/hosted/ci.env up -d --wait --wait-timeout 180`.
  4. From a `curlimages/curl:8.11.1` container on `<project>_default`:
     - `/api/health` must answer exactly `{"status":"ok"}`.
     - `/` must be `text/html` with `Cache-Control: no-store`.
     - The first `/assets/*.js` the shell names must be `immutable`.
  5. `docker compose exec -T app bookclub-server healthcheck`.
  6. On a failure: `compose ps -a` and the last 200 log lines. Always: `down -v --remove-orphans`, the data directory removed if this run made it, and the built image removed.
- **Why everything goes through the daemon:** the Forgejo runner runs jobs in a container (`catthehacker/ubuntu:act-24.04`, oapi notes). With the host's Docker socket, bind-mount sources and published ports belong to the daemon's host, not to the job container.
  - A `busybox:1.37.0` helper container therefore creates (`install -d -o 1000 -g 1000 -m 0750`) and removes the data directory, which must be an absolute path on the daemon's host.
  - The requests come from the compose network, as the work plan asked.
- **Per-run names:** the job sets `COMPOSE_PROJECT_NAME=bookclub-ci-<run_id>`, `BOOKCLUB_TAG=ci-<run_id>` and `BOOKCLUB_DATA_DIR=/tmp/bookclub-ci-<run_id>`. Compose gives the environment precedence over `--env-file`, and the script does the same. `ci.env` publishes on `127.0.0.1` port `0`, a free port, so parallel runs on one daemon never collide.
- **Only CI names** (review fix): the teardown runs `rm -rf` on the data directory as root, `down -v` on the project and `docker image rm` on the tag, all named by the environment. Before it asks Docker anything, the script therefore refuses a `COMPOSE_PROJECT_NAME` that does not match `^bookclub-ci([-_][a-z0-9_-]*)?$`, a `BOOKCLUB_TAG` that does not match `^ci([-_.][A-Za-z0-9_.-]*)?$`, and a `BOOKCLUB_DATA_DIR` (trailing slashes stripped) that does not match `^/tmp/bookclub-ci[A-Za-z0-9_.-]*$`, for `--down` too. A shell that loaded the production `.env` (`BOOKCLUB_DATA_DIR=/srv/bookclub`, `BOOKCLUB_TAG=stable`, `COMPOSE_PROJECT_NAME=bookclub`) or `BOOKCLUB_DATA_DIR=/tmp/` now stops there. The data directory is made with `mkdir` (no `-p`): one that already exists stops the run (`remove it with smoke-test.sh --down`), and the teardown removes the directory only if this run made it, or with `--down`. `deploy_env.rs` checks the refusal with `docker` replaced by a bash function from the environment, so no daemon is involved.
- **A cancelled run** (review fix): a last step with `if: cancelled()` runs `smoke-test.sh --down`. Whether Forgejo's runner runs `cancelled()` steps is not known, and a killed job container runs no trap. Three things now keep leftovers from piling up on the runner's daemon:
  - `ci.env` sets `BOOKCLUB_RESTART=no` (compose.yaml: `restart: ${BOOKCLUB_RESTART:-unless-stopped}`), so a leftover stack does not come back with the daemon.
  - Before it builds, `smoke-test.sh` sweeps what earlier runs left: stacks of `bookclub-ci…` projects whose every container is older than `STALE_MINUTES` (default 120), through `docker compose -p <project> down -v --remove-orphans` with compose.yaml; `ci…` tags of the image that no container uses and that were tagged before that (`.Metadata.LastTagTime`; a daemon that records no time keeps them); and `/tmp/bookclub-ci…` directories older than that on the daemon's host, removed through the busybox helper. Its own names are never swept.
  - The job has `timeout-minutes: 60`, so a stack older than 120 minutes is never a running job's.
- **`ci.env`:**
  - `COMPOSE_PROJECT_NAME=bookclub-ci` (never `bookclub`), `BOOKCLUB_TAG=ci`, `BOOKCLUB_LISTEN=127.0.0.1`, `BOOKCLUB_PORT=0`, `BOOKCLUB_DATA_DIR=/tmp/bookclub-ci`, `BOOKCLUB_ENV_FILE=ci.env` and `BOOKCLUB_RESTART=no`.
  - A production configuration without `DEV_MODE`: `TRUSTED_PROXIES=none`, `SMTP_URL=smtp://localhost:2525` (loopback, accepted without TLS; nothing listens and no mail is sent), and `.invalid` addresses.
  - Compose reads `COMPOSE_PROJECT_NAME` from an `--env-file` too (checked with Compose v5.6.1), so a local `docker compose --env-file deploy/hosted/ci.env …` never touches a `bookclub` project.
- **`ci_env_is_a_production_configuration`** (`deploy_env.rs`):
  - `ci.env` names only what `env.example` names, plus `COMPOSE_PROJECT_NAME`, and only sets values.
  - It points compose at itself, is not the `bookclub` project, and has an absolute data directory.
  - It sets every compose variable and everything `env.example` must set.
  - `Config::from_lookup` accepts it, without DEV_MODE, with SMTP and no harvest.
  - It sets `BOOKCLUB_RESTART=no`.
  - Mutations that each fail it: a non-loopback plain `SMTP_URL`, `BOOKCLUB_ENV_FILE=.env`, `COMPOSE_PROJECT_NAME=bookclub`, an added `DEV_MODE=true`, and an unknown name.
- **`smoke_test_refuses_names_that_are_not_ci_names`** (`deploy_env.rs`, review fix): `smoke-test.sh --down` with `docker` as a bash function imported from the environment (`BASH_FUNC_docker%%`) that only records a call. ci.env's names and CI's per-run names (with a trailing slash) reach `docker`. `/srv/bookclub`, `/tmp/`, `/tmp`, `/tmp/bookclub-ci-1/media`, `/tmp/bookclub-ci-1/..`, `/tmp/../srv/bookclub-ci`, the projects `bookclub` and `bookclub-staging`, and the tags `stable` and `0.11.0` each exit 1 with `<NAME>=… is not a CI …` and never call it. With the guard removed, the test fails.
- **The runner (item 24; changed by the review):** the Docker daemon is root on its host, so only one runner hands it to job containers, and only `image` (ci.yml) and `publish` (release.yml) run there, through `runs-on: docker-host`. The `ubuntu-latest` runner keeps running the other jobs, whose npm and cargo build scripts come from any pull request, without the socket. `image` also skips pull requests from forks: `if: github.event_name == 'push' || github.event.pull_request.head.repo.full_name == github.repository`.
  - **What Rafael set up** (item 24, done 2026-10-29): a second runner registration on VM 101 (its own `.runner` file and `config.yml`, capacity 1, so `image` and `publish` jobs run one after another) with the label `docker-host:docker://ghcr.io/catthehacker/ubuntu:act-24.04`. Only its `config.yml` hands job containers the daemon, in the `container` section: `docker_host` set so that the runner mounts the daemon into job containers (`automount` in current Forgejo runners; check the runner version's config reference), or `valid_volumes: ["/var/run/docker.sock"]` with `options: -v /var/run/docker.sock:/var/run/docker.sock`. Its daemon needs Compose 2.24 or newer. The existing runner (`ubuntu-latest`, `docker`) no longer has the socket; before, it handed it to every job. The `image` job of main's run 10 (531c3fa) passed on the new runner.
  - **Why not `docker`** (integrator, after PR #71's first run): forgejo-runner registers every runner with a default label `docker` (here `docker://node:22-bookworm`), so the existing runner took the `image` job and failed with `docker: command not found`. The label had to be one no runner has by default.
  - Without that runner, the `image` job waits for one instead of failing. `smoke-test.sh` stops with "there is no docker command here" or "the Docker daemon does not answer" when a job lands on a runner without the daemon; the second means the `container` settings above are missing.
  - Every run builds the image without a registry cache; only the daemon's own build cache helps. The run time on the runner is not known. Locally, the build took about 2 minutes with the dependency layer cached and about 4 minutes cold (part 1).

### Backups

- **`backup.sh`:**
  1. **The dump.** One `psql` session in the postgres container (`docker compose exec -T postgres`):
     - It opens `BEGIN ISOLATION LEVEL REPEATABLE READ, READ ONLY`, exports its snapshot (`pg_export_snapshot()`, `\setenv`), and runs `pg_dump --format=custom --no-owner --snapshot=…` through `\!`. The dump is psql's stdout, streamed to the host.
     - In the same transaction it writes the counts through `\o` to a file in the container, which the script copies out and removes.
     - `\if :SHELL_ERROR` (psql 16 and newer) turns a failed `pg_dump` into an SQL error, and `ON_ERROR_STOP` into exit 3.
     - The dump and its counts therefore always describe the same moment. A plain `pg_dump` followed by a count would make the restore test fail at random whenever anyone wrote in between.
     - The pair goes to `BACKUP_DIR/db.new`, is checked (non-empty dump, a `(migration)` line, `pg_restore --list` reads it back), and then replaces `BACKUP_DIR/db`.
  2. **`counts.tsv`** is `table<TAB>rows` for every table in `public`, ordered by name, then `(migration)<TAB><max successful _sqlx_migrations.version>`. `counts_sql` in `backup-lib.sh` writes the query for both scripts.
  3. **`restic backup --tag bookclub BACKUP_DIR/db MEDIA_DIR`**, excluding the media store's temporary files (`.*.tmp`, `.write-probe.*`). The dump comes first, so the media are read after it. That covers pictures added meanwhile (the file is on disk before its row commits), not pictures deleted meanwhile (review fix):
     - The app deletes a picture's files right after the commit that deletes its row or takes it down: a replaced or deleted avatar (`media/avatar.rs`), a takedown (`media/reports.rs`), a deleted account (`auth/deletion.rs`). One that lands between the dump and restic's read leaves the dump referencing files the snapshot does not hold.
     - When restic listed the directory before the deletion, it reports `error: lstat <path>: no such file or directory` (seen in a race test with restic 0.17.3), saves the snapshot without the file and exits 3. Before, `set -e` then stopped `backup.sh` before `forget` and the check, and the heartbeat reported a failure.
     - Now restic's stderr goes to a file (and on to the log). Exit 3 counts as a warning when every `error:` line has the form `error: <op> <path>: no such file or directory` with `<path>` below `MEDIA_DIR` and no longer there: `warning: the app deleted the pictures above while restic read the media; the snapshot was saved without them`. Any other unreadable file stops the backup with `restic could not read the files above (exit 3; …)`, any other exit with `restic backup exited N`.
  4. **`restic forget --host BACKUP_HOST --tag bookclub --keep-daily 7 --keep-weekly 4 --keep-monthly 12 --prune`.** While the history is shorter than the policy, restic also keeps the oldest snapshot (documented restic behaviour; seen as 2 snapshots on one day).
  5. **`restic check --read-data-subset=5%`** on `CHECK_WEEKDAY` (7, Sunday), or with `--check`.
  6. **Before it starts,** `MEDIA_DIR` must exist and equal the `/data/media` mount source of the app container (`docker inspect`). Without an app container, as during a restore, it only warns.
- **`restore-test.sh`:**
  1. `restic restore latest --host BACKUP_HOST --tag bookclub --verify` into `BACKUP_DIR/restore-test/files`.
  2. `RESTORE_POSTGRES_IMAGE` (`postgres:17`) runs with `--network none`. The script waits for `pg_isready -h 127.0.0.1`, because the image's first start runs a temporary server on the socket only. Then `pg_restore --exit-on-error --no-owner`.
  3. `diff -u` of `counts_sql` on the copy against the snapshot's `counts.tsv`.
  4. Every `media.storage_key/<variant>` of the rows that are not taken down must be in the restored `MEDIA_DIR` and not empty. It checks **every** key, not a sample: the restore has all files anyway, and a sample could miss the one deleted file.
     - **Deleted during the backup** (review fix): for the missing keys, the script asks the running stack's database (`docker compose exec -T postgres psql …` in `COMPOSE_DIR`, read only) which keys it references now. A missing key it no longer references was deleted by the app after the dump (see `backup.sh` step 3): logged as `deleted after the backup, no longer referenced: <key>`, counted in the last line (`… 1 media files (1 deleted after the backup)`), no failure. A missing key it still references is lost: `missing or empty: <key>` and exit 1. When the database does not answer (the stack is down, as during a restore), every missing key counts as lost.
     - The README no longer says that every picture the dump references is in the snapshot.
  5. An EXIT trap removes the container and the scratch directory, whatever the outcome.
- **Shared (`backup-lib.sh`):**
  - `load_config` reads `backup.env` next to the scripts, or `BACKUP_ENV`.
  - `restic_` runs `restic/restic:0.17.3` with the caller's uid. Its host name is `BACKUP_HOST`, which keeps snapshots, `forget` and `latest` in one group. `BACKUP_DIR` is mounted read-write and `MEDIA_DIR` read-only, at their own paths, so snapshot paths are host paths. The container also gets the password file, a local repository path, and for `sftp:` repositories `RESTIC_SSH_DIR` at `/root/.ssh` (read-only). It adds `--retry-lock 10m`.
  - The heartbeat follows healthchecks.io's URL scheme: `GET <url>/start`, then `POST <url>/<exit status>` with the last 50 lines of the log. An empty URL means no ping.
  - `supervise` holds the lock (`flock` on `BACKUP_DIR/.lock`), which both scripts share; a second run waits up to 3 h. It writes the log to the journal and to `BACKUP_DIR/<name>.log`.
- **`backup.sh restic ARGS…`** runs restic with this setup: `init`, `snapshots`, `stats`, `check`, and a `restore` into `BACKUP_DIR`. The README's restore uses it.
- **Decisions:**
  - **restic from its image, not the host.** It is pinned at 0.17.3 like the tests, and nothing needs installing. The host needs only Docker, `curl` and util-linux's `flock`. Running a host `restic` instead would change only `restic_`.
  - **The scripts write with `umask 077`.** The dump and the restored files hold personal data.
  - **A snapshot records `BACKUP_DIR/db` and `MEDIA_DIR` under their paths, and the restore test looks there.** After either changes, run `backup.sh` once before the next restore test (stated in `backup.env.example`).
  - **For S3,** `restic_` passes `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and `AWS_DEFAULT_REGION` (Q9's alternative). sftp needs uid 0 in the container, as under systemd: ssh refuses a uid without a passwd entry.
- **Units:**
  - `bookclub-backup.timer` runs daily at 03:30 with `RandomizedDelaySec=30m`. `bookclub-restore-test.timer` runs on Sundays at 05:30, with the same delay. Both are `Persistent=true`.
  - The services are `Type=oneshot`, `Requires=` and `After=docker.service`, with `Nice=10`, best-effort I/O priority 7, `PrivateTmp`, `ProtectSystem=full` and `NoNewPrivileges`. Their start timeouts are 4 h and 6 h; the restore test may first wait for a running backup.
  - No path that a container mounts lies under `/tmp`, so `PrivateTmp` does not hide anything the Docker daemon needs.

### The runbook (`deploy/hosted/README.md`)

New sections: "Releases", "Reverse proxy", "The legacy import", "Backups" (setting up, by hand), "Restore" (the test, restoring for real) and "Open sign-up". Existing sections were extended: "First boot" step 6, the host table, "Requirements" (`curl`), "Staging" (a second stack on one host), and "Updates" (back up first).

Parts that rest on other Wave 1 packages, for the integrator to check after their merges:
- **CUT:** the import directory, `-T`, and the paragraph on the importer's private copy in `$TMPDIR` (the 64 MB `/tmp` tmpfs, and `-e TMPDIR=/import-tmp -v …`). It follows `docs/phase2/notes/cut.md` "For the docs"; on this branch the importer still opens the file in place. The README points at `docs/phase2/cutover.md`, which CUT adds.
- **LEG:** "Outside DEV_MODE the server refuses SIGNUP_MODE=open while one of the files is missing, and names the missing ones" (from `docs/phase2/notes/leg.md`). On this branch open sign-up starts without them.
- **GATE (amendment 16):** "Reverse proxy" is a short pointer to `Caddyfile.example` and `verify-limits.sh`. The integrator moves GATE's two README sections from `docs/phase2/notes/gate.md` there.

### Contract changes for the integrator (part 2)

No routes, DTOs, `error::msg` strings, migrations, server code or frontend files changed. `generated.ts` and `server/.sqlx/` are unchanged. There are no new server variables; `ci.env` and `backup.env.example` are not server settings. The review added one compose variable, `BOOKCLUB_RESTART` (`#BOOKCLUB_RESTART=unless-stopped` in `env.example`, `no` in `ci.env`), and no server code (only tests).

- **`CLAUDE.md`, "CI and releases":**
  - The `image` job builds the root `Dockerfile` and boots it with `deploy/hosted/compose.yaml` and `ci.env`. Locally: `deploy/hosted/smoke-test.sh`; on the agents VM `CARGO_JOBS=3 deploy/hosted/smoke-test.sh`, then `docker builder prune -f`. It accepts only CI names (`bookclub-ci…`, `ci…`, `/tmp/bookclub-ci…`) and first removes what cancelled runs left behind (`STALE_MINUTES`, default 120).
  - `image` (ci.yml) and `publish` (release.yml) run on the runner labelled `docker-host`, the only one that hands job containers the Docker daemon; `image` skips pull requests from forks.
  - A `vX.Y.Z` tag publishes `git.tillhome.de/rafiistcool/bookclub-server` with `X.Y.Z`; `deploy/hosted/release-tags.sh` then moves `X.Y` and `stable` only when the tag is the highest release of its minor line or of all (`X.Y.Z-rc.N`: only the exact tag), never `latest`. The secret is `REGISTRY_TOKEN`.
  - "Older releases are still on GHCR" stays true for the Python image.
- **`CLAUDE.md`, "Request flow":** replace "Do not cut a Python release from `phase-1`…" with: a `v*` tag now publishes the Rust image `bookclub-server`, and the Python image `bookclub` gets no new versions.
- **`CLAUDE.md`, deployment pointer:** `deploy/hosted/README.md` is the operator runbook (backups, restore, import, open sign-up); the scripts are shellcheck-clean (`koalaman/shellcheck:v0.10.0 deploy/hosted/*.sh`, with or without `-x`), the workflows actionlint-clean (`rhysd/actionlint:1.7.7`, which needs the runner label `docker-host` declared; see the next item).
- **New file for the integrator: `.github/actionlint.yaml`.** actionlint knows only GitHub's runner labels and reports `label "docker-host" is unknown` for `runs-on: docker-host`. It reads its list of self-hosted labels from `.github/actionlint.yaml`, which is not among DEP's files, so DEP did not add it. Content:

  ```yaml
  self-hosted-runner:
    labels:
      - docker-host
  ```

  Until it exists: `docker run --rm -v "$PWD":/repo -w /repo rhysd/actionlint:1.7.7 -ignore 'label "docker-host" is unknown' .forgejo/workflows/*.yml .github/workflows/*.yml`.
- **`SELFHOST.md` §7 "Releases"** describes the Python image's tags (`1.2.3`, `1.2`, `1`, `latest` on `rafiistcool/bookclub`). That is no longer what a tag publishes. CUT-R deletes `SELFHOST.md`; until then, a pointer to `deploy/hosted/README.md`, "Releases".
- **`docs/phase2/cutover.md` (CUT):** its compose directory and paths agree with the README's (`/opt/bookclub`, `/srv/bookclub/import`). Its checklist item "Backups run, and `restore-test.sh` has passed once (`deploy/hosted/README.md`)" now has the commands behind it.

### Verified by hand (part 2, on the agents VM)

**Lint**
- `koalaman/shellcheck:v0.10.0` on `backup.sh`, `restore-test.sh`, `backup-lib.sh` and `smoke-test.sh`: clean with and without `-x`, run both from the repo root and from `deploy/hosted/` (`source=SCRIPTDIR/…`).
- `rhysd/actionlint:1.7.7` on all four workflow files: clean.
- `systemd-analyze verify` (systemd 257) on the four units, with `ExecStart` pointed at the worktree's scripts: clean. `systemd-analyze calendar` gives the next runs as Thu 03:30 and Sun 05:30.

**The smoke test, locally** (Docker 29.9.2, Compose v5.6.1, per-run names as in CI, `CARGO_JOBS=3`)
- A full build, then a green run: both services healthy, `/api/health`, `/`, `/assets/index-….js` and the healthcheck passed. Afterwards no container, volume, network, data directory or image of the run was left.
- **Red on a broken Dockerfile:** a copy of the tree with `CMD ["bookclub-server", "no-such-command"]` built from cache, and `up --wait` failed. The script printed `app-1 | error: unrecognized subcommand 'no-such-command'` from the logs, exited 1, and tore everything down.
- `--no-build` against an already tagged image: green, and the image was kept.
- `--down` against a stack left running, as after a cancelled run: it removed the containers, the volume, the data directory and the image.
- Compose takes the project name from `.env` or `--env-file` (it overrides `name: bookclub`), and host port `0` published a free port (32768 and 32769).

**Staging** (`bookclub-staging`, app on 8081)
- The setup:
  - every README command ran as written, from the staging directory, with `/opt/bookclub`, `/srv/bookclub` and `/var/lib/bookclub-backup` replaced by directories outside the repo
  - `COMPOSE_PROJECT_NAME=bookclub-staging` in its `.env`, as "Staging" describes
  - the image built from this branch as `…/bookclub-server:dep-local`
  - Mailpit running in the app container's network namespace, so that `smtp://localhost:1025` reached it
- **First boot:**
  - `up -d --wait`, `ps` and the healthcheck passed (`GET http://127.0.0.1:8080/api/health answered 200 OK`).
  - `send-test-mail you@example.com` arrived in Mailpit with `Message-ID: <uuid@staging.example.com>`.
- **The legacy import:**
  - `bookclub.db` was built from the synthetic `fresh.sql` with Python's sqlite3.
  - Step 1's `install`, `cp`, `chown` and `chmod` ran, then `docker compose stop app`.
  - The dry run exited 0 (38 report lines, no CR), and so did the real run. `diff <(tail -n +2 dry-run.txt) import.txt` was empty.
  - `up -d --wait` worked. The two legacy avatars landed in the media directory.
  - Both `rematch-legacy` commands exited 0: 1 unmatched ISBN, as the fixture intends.
  - `df -h /tmp` showed the 64 MB tmpfs.
  - `DNB_HARVEST=true` and back reached the container.
- **The first account:**
  - `beta-invite` printed exactly 2 lines.
  - A sign-up with its code through `POST /api/auth/register` returned 201, an avatar upload 200.
  - `make-admin grace` worked.
- **Updates:** `docker compose pull app` failed with `unauthorized` (see below). Pinning `BOOKCLUB_TAG` to a second local tag and back, each time with `up -d --wait app`, worked, and `docker compose images app` showed the tag.
- **Backups:**
  - The repository was a local directory outside the repo. A local HTTP listener stood in for healthchecks.io.
  - `cp` of the scripts, `install` of `backup.env`, and the `umask 077` password ran as written, then `backup.sh restic init`.
  - `backup.sh` exited 0: `dump 176K, 34 tables, 207 rows, migration 1900`, then the snapshot and `forget`.
  - The listener saw `GET /ping/backup/start`, then `POST /ping/backup/0` with the log's last lines.
  - The dump and counts were mode 0600 after the `umask` fix (0664 before; the directory was 0700).
  - `backup.sh restic snapshots`, `stats` and `check`, and `backup.sh --check` (`no errors were found`), all exited 0.
- **Restore test, green:** exit 0, `restore test passed: 34 tables, 207 rows, migration 1900, 3 media files`, and `POST /ping/restore-test/0`. No container and no scratch directory were left.
- **Restore test, red:**
  - **A truncated dump** (halved, then backed up through `backup.sh restic backup` with the same paths) gave exit 1: `pg_restore: error: could not read from input file: end of file`, and `/ping/restore-test/1`.
  - **A deleted media file** (an avatar removed before `backup.sh`) gave exit 1: `missing or empty: avatars/lr/…/a` and `1 of 3 media files are missing or empty`. With the file back and a new backup, the test was green again.
  - **A changed count** in `counts.tsv` gave exit 1 with the diff `-users 999` / `+users 6`.
- **`backup.sh` guards:**
  - A `MEDIA_DIR` other than the app's mount gave exit 1 with `MEDIA_DIR is …/legal, but the app container mounts …/media at /data/media`, and `/ping/backup/1`.
  - With the lock held by another process, a run logged `waiting for the other backup or restore test to finish` and went on 6 s later, when the lock was released.
- **The dump and its counts under writes:**
  - A procedure committed one row every 10 ms into a probe table during `backup.sh`. The table grew from 55 to 259 rows over the run.
  - The snapshot's `counts.tsv` said 63, and the restore test matched it exactly.
- **Restoring for real** (steps 1 to 5, as written):
  - `restore latest --host bookclub --tag bookclub` into the backup directory, then `down -v`, `up -d --wait postgres`, and `pg_restore` from stdin (exit 0).
  - The media replaced with `find`, `cp -a` and `chown`.
  - `backup.sh`, with the expected warning that there is no app container, then `diff` of the two `counts.tsv`: empty.
  - `up -d --wait` and the `rm`.
  - After the restore, the new account and an imported one (`demo`, with its legacy password) could log in (204), and the avatar URL served `image/webp`.
- **Open sign-up:** the `sed`, `up -d --wait app` and `curl …/api/config` showed `"signup":"open"`, and the same commands with `invite` switched it back.
- **The sftp path, as root:**
  - Set up:
    - a local `atmoz/sftp` server on `172.17.0.1:2323`
    - the README's `Host bookclub-backup` block, with that address and port
    - a key and the scanned host key, all root-owned 0600 in a directory used as `RESTIC_SSH_DIR`
  - Under `sudo`, `backup.sh restic init`, `backup.sh` and `restore-test.sh` all exited 0. The dump was `0600 root`, and restic logged the snapshot as `by root@bookclub`. This covers the mount of `/root/.ssh`, the ssh config alias, a non-standard port and `known_hosts`.
- **The units' sandbox:** `sudo systemd-run --wait` with the service's properties (`Type=oneshot`, `Nice`, I/O priority, `PrivateTmp`, `ProtectSystem=full`, `NoNewPrivileges`, `Requires=` and `After=docker.service`) ran `backup.sh` and `restore-test.sh` as root. Both exited 0. `BindPaths=` exposed the staging directories, which lie under `/tmp` only on this VM.
- **Teardown:** the README's staging wipe (`down -v`, then `find … -mindepth 1 -delete`). Then the repositories, staging directories, the sftp server, the listener and the test images were removed, and `docker builder prune -f` ran.

**Not verified** (see "Review round 1" below for what the review added):
- **`docker compose pull` from `git.tillhome.de`:** no `bookclub-server` tag is published yet, and this VM has no `docker login` there (`unauthorized`). The same commands ran against a local registry.
- **The release workflow:** it has never run, because no tag was pushed. Also untested: metadata-action and the registry cache against Forgejo's registry, and the `REGISTRY_TOKEN` secret.
- **The `image` job on the Forgejo runner:** its first run is the Wave 1 PR (amendment 4; item 24). It landed on the existing runner (see "Why not `docker`"); it now runs on the runner labelled `docker-host`, where it first passed in main's run 10 (531c3fa, 2026-10-29).
- **The README commands that need the production VM or accounts:**
  - `systemctl` and `journalctl` on the VM itself (they ran in a systemd 257 container, see below)
  - the Storage Box key upload and its host-key fingerprints
  - healthchecks.io
  - `git tag` and `git push` of a release

## Review round 1

A review of DEP raised ten findings; all were checked against the code. The fixes are described in place above; this section lists them and what was run.

### Fixed

| Finding | Fix |
|---|---|
| A cancelled CI run leaves its stack, image and data directory on the runner's daemon (two findings) | `BOOKCLUB_RESTART=no` in `ci.env`; `smoke-test.sh` sweeps stale `bookclub-ci…` stacks, unused `ci…` tags and `/tmp/bookclub-ci…` directories older than `STALE_MINUTES`; `timeout-minutes: 60` on the job ("The CI image smoke job") |
| `smoke-test.sh` deletes whatever the environment names (two findings) | Names must look like CI's, checked before any `docker` call; `mkdir` without `-p`, and the teardown removes only a directory the run made; `smoke_test_refuses_names_that_are_not_ci_names` |
| The PR-triggered `image` job needs the host daemon on the shared runner | `runs-on: docker-host` for `image` and `publish`; `image` skips fork pull requests ("The runner") |
| `stable` and `X.Y` can move back to an older version | The exact version is pushed first; `release-tags.sh` moves the floating tags only for the highest release, from the remote's tags at that moment ("The release workflow", step 6) |
| A picture deleted between the dump and restic's read fails the backup (restic exit 3) or the restore test (two findings) | `backup.sh` treats exit 3 as a warning when every unreadable file is a vanished picture below `MEDIA_DIR`; `restore-test.sh` excuses missing pictures the live database no longer references; README reworded ("Backups") |
| No test covers the `MEDIA_DIR` write probe | `/proc` cases in `store.rs` and `deploy_env.rs` |
| The healthcheck's `.no_proxy()` is untested | `deploy_env.rs` runs the healthcheck with `HTTP_PROXY`, `http_proxy` and `ALL_PROXY` pointing at a dead port |

### Verified by hand (review round 1, on the agents VM)

Docker 29.9.2, Compose v5.6.1, buildx v0.38.1.

**Tests against mutations** (each restored afterwards):
- Without `.no_proxy()` in `healthcheck.rs`: `healthcheck_succeeds_only_on_a_200` and `healthcheck_gives_up_after_three_seconds` fail.
- `check_writable` cut down to `create_dir_all`: `the_writability_check_writes_into_an_existing_root` fails, and `serve_and_import_legacy_refuse_a_media_dir_they_cannot_write` runs into its 20 s limit.
- The name checks removed from `smoke-test.sh`: `smoke_test_refuses_names_that_are_not_ci_names` fails.

**`smoke-test.sh`**
- Leftovers: a stack `bookclub-ci-left` (both containers with restart policy `no`), its tag `ci-left`, an unused tag `ci-orphan`, and the directories `/tmp/bookclub-ci-left` and `/tmp/bookclub-ci-orphan`.
  - A `--no-build` run with the default 120 minutes passed and left all of them alone.
  - A run with `STALE_MINUTES=0` and a build removed the stack with its volume and networks, the three unused `ci…` tags (one of them the earlier `--no-build` run's image) and both directories. It then built, passed and tore down. No container, volume, network, image or directory of any run was left.
- An existing `/tmp/bookclub-ci-dep2` with a file in it: exit 1 with `/tmp/bookclub-ci-dep2 already exists on the daemon's host (remove it with deploy/hosted/smoke-test.sh --down)`; the directory and its file were still there.
- `--down` against a stack left running: the containers, the volume, both networks, the data directory and the image were removed.

**Releases** (no tag pushed, nothing sent to Forgejo)
- `release-tags.sh --plan` against a local bare repository: `v0.11.0` alone gives `0.11 stable`; after `v0.11.1`, `v0.11.0` gives nothing and `v0.11.1` gives `0.11 stable`; a backport `v0.10.5` gives `0.10`; `v0.11.10` sorts above `v0.11.1`; pre-releases and `phase1-green` give nothing; a tag the remote does not have is an error.
- A local `registry:2.8.3` and a `docker-container` buildx builder (`network=host`):
  - The root Dockerfile built and pushed as `0.11.1` with the workflow's options (`provenance=false`, `sbom=false`, `cache-to type=registry,…:buildcache,mode=max,image-manifest=true,oci-mediatypes=true`): 5 min 57 s cold, 58 s of it the cache export. The cache is an OCI image manifest with the config type `application/vnd.buildkit.cacheconfig.v0` and 20 layers.
  - After `docker buildx prune -af` on the builder, a build as `0.11.2` took 12 s: every step `CACHED` from the registry.
  - Overlapping runs: `v0.11.2`'s step ended first and pointed `0.11` and `stable` at it; `v0.11.1`'s step ended last and moved nothing. `0.11` and `stable` had the manifest bytes of `0.11.2` (an OCI image manifest, no index).
- Staging, with a `compose.override.yaml` that named `localhost:5001/…` instead of `git.tillhome.de/…`: the README's `docker compose pull app` and `docker compose up -d --wait app` ran `stable` (the image of 0.11.2). With `BOOKCLUB_TAG=0.11.1`, the same two commands rolled back to 0.11.1 (`docker compose images app`), healthy.

**Backups** (`bookclub-staging` from this branch, two accounts with avatars, a local restic repository)
- Baseline: `backup.sh` and `restore-test.sh` exit 0.
- **The race's outcome:** the dump from before a deletion and the media from after it, put into one snapshot with `backup.sh restic backup --tag bookclub <BACKUP_DIR>/db <MEDIA_DIR>`. A replaced avatar, a deleted avatar and an admin takedown each gave exit 0, `deleted after the backup, no longer referenced: avatars/…/a` and `… (1 deleted after the backup)`. The previous `restore-test.sh` failed the same snapshot with `1 of 2 media files are missing or empty`.
- A file removed from disk while its row stayed: exit 1, `missing or empty: avatars/…/a`.
- The race's snapshot with the stack's postgres stopped: exit 1 after `warning: the live database does not answer, so every missing file counts`.
- **restic exit 3**, from stub images (`RESTIC_IMAGE`) that run restic 0.17.3 and then print one `error:` line and exit 3:
  - `error: lstat <MEDIA_DIR>/avatars/zz/zzgone/a: no such file or directory` and `error: open <MEDIA_DIR>/avatars/zz/zzgone: no such file or directory`: the warning, then `forget`, `backup done`, exit 0.
  - `permission denied`, a path that still exists, a path below `BACKUP_DIR`, and an `error:` line of another form: exit 1 with `restic could not read the files above`.
- A real race (restic 0.17.3 reading 8,000 files while a process deleted and rewrote directories) printed `error: lstat <path>: no such file or directory` and `Warning: at least one source file could not be read`, and exited 3: the form the script expects.

**systemd** (a stand-in for the VM)
- A Debian 13 container with systemd 257 as PID 1: privileged, its own cgroup namespace, `tmp.mount` masked, the host's Docker socket and CLI mounted, the staging directories at their own paths.
- Two stand-ins: a `docker.service` that only exists (the daemon runs outside the container), and a drop-in `BindPaths=<staging directory>`, because the staging lies under `/tmp`, which `PrivateTmp` hides.
- The README's commands as written: `cp deploy/hosted/systemd/bookclub-* /etc/systemd/system/`, `systemctl daemon-reload`, `systemctl enable --now bookclub-backup.timer bookclub-restore-test.timer`, `systemctl list-timers 'bookclub-*'` (next runs Thu 03:30 and Sun 05:30 plus the random delay), `systemctl start bookclub-backup`, `systemctl start bookclub-restore-test` (both exit 0; `Result=success`, `PrivateTmp=yes`, `ProtectSystem=full`, `NoNewPrivileges=yes`, `Nice=10`, I/O priority 7), and `journalctl -u bookclub-backup -u bookclub-restore-test`, which showed the dump, the snapshot, `backup done` and `restore test passed`.

**Lint**
- shellcheck v0.10.0 on all five scripts, with and without `-x`, from the repo root and from `deploy/hosted/`: clean. Without `-x`, shellcheck follows only files given as input; `backup.sh` and `restore-test.sh` source `backup-lib.sh`, which the `*.sh` glob passes, but `backup-lib.sh` sources `backup.env.example`, which it does not. That `.` line therefore carries `disable=SC1091` next to `source=SCRIPTDIR/backup.env.example`: with `-x` shellcheck still parses the example (a syntax error in it fails the check), and the plain `deploy/hosted/*.sh` stays clean. Each of `backup.sh` and `restore-test.sh` alone still needs `-x` or `backup-lib.sh` beside it on the command line.
- actionlint 1.7.7 on all four workflows: clean with the proposed `.github/actionlint.yaml` (passed as `-config-file`) and with `-ignore 'label "docker" is unknown'`.
- `docker compose -f deploy/hosted/compose.yaml --env-file deploy/hosted/env.example config -q`: exit 0; with `ci.env` both services get `restart: "no"`.

**Cleanup:** the staging stack (`down -v`), the systemd container and image, the stub images, the local registry and its builder, the test images, and `docker builder prune -f`.

### Still open after the review

- **`docker compose pull` from `git.tillhome.de`:** needs a published tag and `docker login` on the VM (Rafael, item 11). Only a local registry stood in.
- **The release workflow on Forgejo:** it runs on the first `v*` tag. Untested there: metadata-action, the login with `REGISTRY_TOKEN` (the user secret of item 23), whether Forgejo's registry takes the cache manifest (config type `application/vnd.buildkit.cacheconfig.v0`) and `imagetools create`, and `git ls-remote` with the checkout's credentials.
- **The runner labelled `docker-host` (item 24):** done on 2026-10-29, on VM 101 ("The runner"); main's run 10 passed the `image` job on it.
- **README commands no VM here can run:** the Storage Box key upload and host-key fingerprints (a local sftp server stood in), the healthchecks.io account (a local listener stood in), `git tag` and `git push` of a release, and `systemctl`/`journalctl` on the production VM itself.
- **GATE's Caddy and `verify-limits.sh` sections** (amendment 16) are not on this branch: they are in `docs/phase2/notes/gate.md` on GATE's branch, and the integrator moves them into `deploy/hosted/README.md` after GATE merges. DEP's "Reverse proxy" section already points at both files.
