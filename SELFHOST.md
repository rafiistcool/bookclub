# Self-host Bookclub

Anyone can run this. There is no hosted service, no extra database, and no
account at a cloud provider. One container (or one Python process) and a
SQLite file.

This guide is generic: no personal domain, no vendor lock-in. Point DNS or a
tailnet hostname at whatever machine you have.

## What you get

- Invite-only accounts for a small group (one club per instance)
- One current club pick on Home, with notes, an optional meeting, and a next-up vote
- Personal shelves (Want to read / Reading / Finished / Did not finish), finish notes, ratings, and reading progress
- TBR overlap and a Goodreads CSV import
- Download a SQLite backup from Settings → Backup (no restore-from-upload; replace the files as below)
- Your name and optional theme colors from env
- Data in one directory you can copy
- Session cookies that become `Secure` automatically behind HTTPS
- A first-run invite printed in the logs if you do not set one yourself

## 1. Requirements

- Docker Engine with Compose v2, **or** Python 3.12+ and Node 20+ (bare metal)
- Outbound HTTPS so [Open Library](https://openlibrary.org) search works
- A place for `./data` (a few megabytes)

Works on amd64 and arm64 (Raspberry Pi 4+, most NAS boxes).

## 2. Docker Compose (recommended)

From the repo root:

```bash
cp .env.example .env
# set BOOKCLUB_NAME (and optional BOOKCLUB_THEME / BOOKCLUB_PUBLIC_URL)
# set SECRET_KEY and BOOKCLUB_BOOTSTRAP_INVITE, or leave them empty
docker compose up --build -d
docker compose logs -f bookclub
```

Open `http://<that-machine>:8000`.

On first start the app writes two files next to the database:

| File | Purpose |
|---|---|
| `data/bookclub.db` | All club data (WAL mode) |
| `data/.secret_key` | Session signing key (generated if `SECRET_KEY` is empty) |
| `data/.bootstrap_invite` | First invite (generated if `BOOKCLUB_BOOTSTRAP_INVITE` is empty) |

The logs print the first invite once. Share it with the first person, then mint
more from **Settings → Invites** in the app. Treat unused codes like passwords.

`DEBUG=0` is the Compose default. Demo defaults (`DEBUG=1`, invite `DEV-ONLY`)
are only used if you start with `docker-compose.dev.yml`.

### Optional HTTPS with Caddy

If the machine has a public DNS name:

```bash
# in .env
BOOKCLUB_DOMAIN=books.example.com
BOOKCLUB_PUBLIC_URL=https://books.example.com
```

```bash
docker compose -f docker-compose.yml -f docker-compose.proxy.yml up --build -d
```

Caddy listens on 80/443 and proxies to the app on the Docker network.
`:8000` is **not** published on the host. Set A/AAAA records to the server
and open 80/443. Let's Encrypt is automatic. `BOOKCLUB_DOMAIN` must be the
real hostname (not `localhost`) — this file binds 80/443.

You can also put your own reverse proxy in front of port 8000 on the same
host. The UI calls relative `/api`, so the browser must see one origin.
Send `X-Forwarded-Proto: https` and leave `BOOKCLUB_HTTPS=auto` so the
session cookie is marked `Secure`. Set `BOOKCLUB_TRUSTED_PROXIES` to the
proxy (or `127.0.0.1` if it shares the host).

### LAN, Tailscale, Cloudflare Tunnel

- **LAN only:** keep port 8000, leave `BOOKCLUB_HTTPS=auto` (or `0` if you
  want to force non-Secure cookies).
- **Tailscale Serve / Funnel:** proxy to `http://127.0.0.1:8000`. Serve
  provides HTTPS; Funnel exposes it on the internet.
- **Cloudflare Tunnel / nginx / Caddy on the host:** same as any other
  reverse proxy. Do not set a hardcoded hostname in this repo.

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
git pull
docker compose up --build -d
```

The data directory is a bind mount; rebuilding the image does not wipe it.

Reset a toy install: stop the app and delete `data/bookclub.db*` (keep or
delete `.secret_key` / `.bootstrap_invite` as you prefer). An empty database
recreates the bootstrap invite from `BOOKCLUB_BOOTSTRAP_INVITE` or
`data/.bootstrap_invite`.

## 5. Environment

See `.env.example` and the table in the README. Important production rules:

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
