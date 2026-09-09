# Self-host Bookclub

Anyone can run this. There is no hosted service, no extra database, and no
account at a cloud provider. One container (or one Python process) and a
SQLite file.

This guide is generic: no personal domain, no vendor lock-in. Point DNS or a
tailnet hostname at whatever machine you have.

## What you get

- Invite-only accounts for a small group (one club per instance)
- One current club pick on Home, with an optional meeting and a next-up vote
- A reading diary on every book: entries carry the writer's position, are
  spoiler-shielded for members who are behind, and take replies and reactions
- Personal shelves (Want to read / Reading / Finished / Did not finish), ratings
  with a club average per book, and reading progress
- TBR overlap and a Goodreads CSV import
- Download a SQLite backup from Settings → Backup (no restore-from-upload; replace the files as below)
- Your name and optional theme colors from env
- Data in one directory you can copy
- Session cookies that become `Secure` automatically behind HTTPS
- A first-run invite printed in the logs if you do not set one yourself

## 1. Requirements

- Docker Engine with Compose v2, **or** Python 3.12+ and Node 20+ (bare metal)
- Outbound HTTPS so catalog search works ([Open Library](https://openlibrary.org),
  and optionally [Google Books](https://developers.google.com/books) if you set
  `GOOGLE_BOOKS_API_KEY`). The shelf still works if search is down; club-only
  books never call a catalog. Enabling the key later can create ISBN-keyed
  duplicates of books already stored as Open Library `/works/OL…` keys.
- A place for `./data` (a few megabytes)
- Your own reverse proxy if you want HTTPS (see below). The image does not
  bundle one.

The image is published for amd64 and arm64 (Raspberry Pi 4+, most NAS boxes).

## 2. Docker Compose (recommended)

Releases are published to `ghcr.io/rafiistcool/bookclub`. You do not need to
clone the repository: the compose file in the repo is an example to copy and
edit. In an empty directory:

```bash
curl -fsSLO https://raw.githubusercontent.com/rafiistcool/bookclub/main/docker-compose.yml
curl -fsSL  https://raw.githubusercontent.com/rafiistcool/bookclub/main/.env.example -o .env
# set BOOKCLUB_NAME (and optional BOOKCLUB_THEME / BOOKCLUB_PUBLIC_URL)
# set SECRET_KEY and BOOKCLUB_BOOTSTRAP_INVITE, or leave them empty
docker compose up -d
docker compose logs -f bookclub
```

Open `http://<that-machine>:8000`.

The example pins `ghcr.io/rafiistcool/bookclub:latest`. To control when you
take upgrades, pin a minor (`:0.1`) or an exact version (`:0.1.0`) instead;
see [Releases](#7-releases) for how tags are produced.

On first start the app writes two files next to the database:

| File | Purpose |
|---|---|
| `data/bookclub.db` | All club data (WAL mode) |
| `data/.secret_key` | Session signing key (generated if `SECRET_KEY` is empty) |
| `data/.bootstrap_invite` | First invite (generated if `BOOKCLUB_BOOTSTRAP_INVITE` is empty) |

The logs print the first invite once. Share it with the first person, then mint
more from **Settings → Invites** in the app. Treat unused codes like passwords.

`DEBUG=0` is the Compose default. Demo defaults (`DEBUG=1`, invite `DEV-ONLY`)
are only used if you build from source with `docker-compose.dev.yml`.

### Reverse proxy and HTTPS (your job)

The container serves plain HTTP on `:8000` and ships no TLS. Put whatever
you already run in front of it — Caddy, nginx, Traefik, Tailscale Serve, a
Cloudflare Tunnel — and manage certificates there. Rules that make it work:

- **Same origin.** The UI calls relative `/api`, so the browser must reach the
  app and the API through one hostname. Set `BOOKCLUB_PUBLIC_URL` to that
  public origin (`https://books.example.com`).
- **Forward the scheme.** Send `X-Forwarded-Proto: https` and leave
  `BOOKCLUB_HTTPS=auto`; the session cookie becomes `Secure` automatically.
- **Name the proxy.** `BOOKCLUB_TRUSTED_PROXIES` is who may set
  `X-Forwarded-*`. `*` is fine when `:8000` is not reachable from the internet;
  use `127.0.0.1` if the proxy shares the host.
- **Do not expose `:8000` publicly.** If the proxy runs on the same machine,
  bind the port to loopback in your compose file:
  `ports: ["127.0.0.1:8000:8000"]`. If it runs in the same Compose project,
  drop `ports` and proxy to `bookclub:8000` on the Compose network.

Where the proxy lives is up to you:

- **LAN only, no proxy:** keep `:8000` published, leave `BOOKCLUB_HTTPS=auto`
  (or `0` to force non-Secure cookies).
- **Tailscale Serve / Funnel:** `tailscale serve 8000`. Serve provides HTTPS
  on the tailnet; Funnel exposes it on the internet.
- **Cloudflare Tunnel:** point the tunnel at `http://127.0.0.1:8000`.
- **Caddy / nginx / Traefik:** a one-line `reverse_proxy 127.0.0.1:8000`
  (or the equivalent) with automatic certificates. Configure and run it the
  way you do for your other services; nothing in this repo needs to match it.

### NAS / Unraid / home server

Set `PUID` and `PGID` in `.env` to the account that owns the data folder so
bind-mounted files are writable:

```bash
PUID=1000
PGID=1000
BOOKCLUB_DATA=/volume1/docker/bookclub
```

Put `BOOKCLUB_DATA` on a **local disk**. SQLite WAL on NFS or another
network filesystem can corrupt the database. If `chmod 600` on
`.secret_key` fails (common on CIFS/NFS), the app logs a warning and
continues — that is not a license to store the DB on a share.

## 3. Bare metal

```bash
cp .env.example .env
# or: cp deploy/env.example .env
# both files set DEBUG=0 and empty secrets (generated on first start)
# set BOOKCLUB_NAME; optionally BOOKCLUB_PUBLIC_URL and BOOKCLUB_TRUSTED_PROXIES=127.0.0.1

cd frontend && npm ci && npm run build && cd ..

cd backend
python3 -m venv .venv
source .venv/bin/activate
pip install -e .
# bind loopback; put Caddy/nginx/Tailscale Serve in front
uvicorn app.main:app --host 127.0.0.1 --port 8000
```

Put HTTPS in front of `127.0.0.1:8000`. A sample systemd unit is
`deploy/bookclub.service` (edit paths, create a `bookclub` user).

Mint extra invites:

```bash
cd backend && .venv/bin/python -m app.create_invite
```

## 4. Backup, restore, update

Backup (safe while the app is running):

```bash
docker compose exec bookclub python -m app.backup /data/bookclub-backup.db
# then copy ./data/bookclub-backup.db off the machine
```

Or stop writes and copy `data/bookclub.db` plus `-wal` / `-shm` if they exist.
Also copy `data/.secret_key` — changing it logs everyone out. Any signed-in
member can also download `bookclub.db` from Settings → Backup.

Restore: stop the container, replace the db files (and `.secret_key` if you
want existing sessions), start again. There is no upload/restore in the UI.

Update:

```bash
docker compose pull
docker compose up -d
```

The data directory is a bind mount; a new image does not wipe it. Schema
changes are applied on start (`COLUMN_MIGRATIONS` in `backend/app/db.py`),
so take a backup before a major-version bump.

Reset a toy install: stop the app and delete `data/bookclub.db*` (keep or
delete `.secret_key` / `.bootstrap_invite` as you prefer). An empty database
recreates the bootstrap invite from `BOOKCLUB_BOOTSTRAP_INVITE` or
`data/.bootstrap_invite`.

## 5. Environment

Every key is commented in [`.env.example`](.env.example) (or `deploy/env.example`).
The README lists the handful that matter on first start. Full set:

| Variable | Default | Meaning |
|---|---|---|
| `BOOKCLUB_NAME` | `Bookclub` | Club name in the UI, tab title, manifest, API title, and Open Library User-Agent. |
| `BOOKCLUB_TZ` | `UTC` | IANA timezone for club-pick meeting labels. Invalid names fall back to UTC. |
| `BOOKCLUB_THEME` | `#b44a2a` | Optional accent (`#rgb` / `#rrggbb`). |
| `BOOKCLUB_THEME_DARK` | shaded accent | Optional darker accent. |
| `BOOKCLUB_PUBLIC_URL` | empty | Canonical origin of this instance (`https://books.example.com`). Same-origin reverse proxy is the supported path; the UI does not talk to a split API host. Vite `localhost:5173` CORS is only when `DEBUG=1`. |
| `SECRET_KEY` | empty in Compose | Signs the session cookie. Changing it logs everyone out. Empty + `DEBUG=0` writes `data/.secret_key`. Example / short values are refused when `DEBUG=0`. |
| `BOOKCLUB_BOOTSTRAP_INVITE` | empty in Compose | First invite, only if the DB has none. Empty + `DEBUG=0` writes `data/.bootstrap_invite`. `DEV-ONLY` is refused when `DEBUG=0`. |
| `DEBUG` | `0` in Compose | `1`: CORS for Vite, `/api/docs`. `0`: no docs. |
| `BOOKCLUB_HTTPS` | `auto` | `auto`: session cookie is `Secure` only on HTTPS (including `X-Forwarded-Proto`). `1`: always. `0`: never. |
| `BOOKCLUB_TRUSTED_PROXIES` | `*` | Who may set `X-Forwarded-*`. `*` is correct behind a private reverse proxy. |
| `DATABASE_PATH` | `<repo>/data/bookclub.db` | Absolute path if you want it elsewhere. Docker uses `/data/bookclub.db`. |
| `VAPID_PRIVATE_KEY` | empty | PEM private key for Web Push. Empty: a key pair is generated into `data/.vapid_private.pem` on first use. Changing it invalidates every device subscription. |
| `GOOGLE_BOOKS_API_KEY` | empty | Optional. Prefer Google Books for Discover search (and ISBN / Goodreads lookups), with Open Library as fallback. Create a key in Google Cloud Console → Credentials and enable the Books API (`books.googleapis.com`). Restrict by IP if you can. Default free quota is enough for a small club; billing is not required. Leave empty for Open Library only. |
| `VAPID_SUBJECT` | `BOOKCLUB_PUBLIC_URL` or `mailto:bookclub@localhost` | Contact claim sent to push services (`mailto:` or `https://`). |
| `BOOKCLUB_PORT` | `8000` | Host port published by Compose. |
| `PUID` / `PGID` | `1000` | Runtime user for bind-mounted `./data`. |

Production rules:

- `DEBUG=0` hides `/api/docs`.
- Empty `SECRET_KEY` / `BOOKCLUB_BOOTSTRAP_INVITE` generate files under `./data`.
- Explicit placeholders (`dev-secret-change-me`, `DEV-ONLY`, short keys) are
  refused when `DEBUG=0`.
- `BOOKCLUB_HTTPS=auto` (default) marks the session cookie `Secure` only when
  the request is HTTPS, including after `X-Forwarded-Proto`.
- `BOOKCLUB_PUBLIC_URL` is the public origin of this same instance (Open
  Library contact / docs). The UI uses relative `/api`; put TLS on the
  same host. `DEBUG=1` also allows Vite on `localhost:5173`.
- `BOOKCLUB_TRUSTED_PROXIES` is who may set `X-Forwarded-*`. The image no
  longer forces `--forwarded-allow-ips *`.

## 6. Security notes

- This is a private club, not a public website. Invite codes are the access
  control. Do not publish unused codes.
- There is no email and no password reset. If everyone is locked out, mint an
  invite from the server (`python -m app.create_invite`) or restore a backup.
- Publish port 8000 only on a trusted network, or put TLS in front and do not
  expose 8000 at all.
- The image drops to uid `PUID` after fixing `/data` ownership. Do not run
  extra sidecars that write into that directory as root unless you chown.

## 7. Releases

Images are built by `.github/workflows/release.yml` only when a `v*` git tag
is pushed — branch pushes and pull requests never publish anything. A tag
`v1.2.3` produces these tags on `ghcr.io/rafiistcool/bookclub`:

| Tag | Moves when |
|---|---|
| `1.2.3` | never |
| `1.2` | the next patch release of 1.2 |
| `1` | any 1.x release (not produced while the major is `0`) |
| `latest` | any stable release (pre-releases such as `v1.3.0-rc.1` only get their exact version) |

Cutting a release from `main`:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The workflow runs the backend and frontend test suites first, then builds
and pushes a multi-arch image. `backend/pyproject.toml` carries a `version`
for the Python package; it is informational and does not have to match the
tag.
