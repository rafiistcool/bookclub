# Bookclub

A tiny private bookclub for 2–5 people. Each person has an account and their own shelf:

**Want to read → Reading → Finished / Did not finish**

Search [Open Library](https://openlibrary.org), add a book, drag it between columns (or use **Move to…**). Friends can look at each other’s shelves. Signup is invite-only.

No Redis, no Postgres. One Python process and a SQLite file. Anyone can self-host it.

---

## Self-host

The default Docker Compose file is a production-style install, not a local demo.

```bash
git clone <this-repo>
cd bookclub
cp deploy/env.example .env
docker compose up --build -d
docker compose logs bookclub
```

Open `http://<host>:8000`. The first-run invite is printed in the logs and stored in `data/.bootstrap_invite`. Register with that code, then mint more from **Invites**.

Secrets: leave `SECRET_KEY` and `BOOKCLUB_BOOTSTRAP_INVITE` empty and the app writes strong values into `./data` on first start. `DEBUG=0` is the default.

HTTPS: put any reverse proxy in front (Caddy, nginx, Tailscale Serve, Cloudflare Tunnel). Leave `BOOKCLUB_HTTPS=auto` so the session cookie is `Secure` on HTTPS. Optional bundled Caddy:

```bash
# in .env: BOOKCLUB_DOMAIN=books.example.com
docker compose --profile proxy up --build -d
```

Full notes (LAN, NAS `PUID`/`PGID`, bare metal, backup, systemd): **[SELFHOST.md](SELFHOST.md)**.

Local demo overlay (debug docs + invite `DEV-ONLY` — not for a shared host):

```bash
docker compose -f docker-compose.yml -f docker-compose.dev.yml up --build
```

---

## Develop

Day-to-day work uses **two processes**: FastAPI on `:8000` and Vite on `:5173`. Vite proxies `/api` to the backend, so the browser only talks to `localhost:5173`.

### What you need

- Python **3.12+** (`python3 --version`)
- Node **20+** (`node --version`)
- Two terminals

On macOS with Homebrew, `python3` may be older than 3.12. Use a newer one if so (`python3.12`, `python3.13`, `python3.14`, …).

### 1. Clone and env file

From the repo root:

```bash
cp .env.example .env
```

The defaults in `.env` are fine for local work (`DEBUG=1`, invite `DEV-ONLY`).

### 2. Backend

```bash
cd backend
python3 -m venv .venv

# bash / zsh
source .venv/bin/activate
# fish
# source .venv/bin/activate.fish

pip install -e ".[dev]"
uvicorn app.main:app --reload --port 8000
```

Leave this running. Reload picks up Python changes.

Check: [http://127.0.0.1:8000/api/health](http://127.0.0.1:8000/api/health) should return `{"ok":true}`.

While `DEBUG=1`, API docs are at [http://127.0.0.1:8000/api/docs](http://127.0.0.1:8000/api/docs).

### 3. Frontend

In a second terminal:

```bash
cd frontend
npm install
npm run dev
```

Open **[http://localhost:5173](http://localhost:5173)** — not port 8000. That’s the hot-reloading Vue app.

### First account

1. Go to **Create an account**.
2. Invite code: `DEV-ONLY` (only works on an empty database).
3. Username: `a–z`, `0–9`, `_`, 2–32 characters.
4. Password: at least 8 characters.

After that, `DEV-ONLY` is spent. Mint more codes from the account menu → **Invites**, or:

```bash
cd backend
.venv/bin/python -m app.create_invite
```

That prints a new code.

### Everyday loop

| You change | Where it shows up |
|---|---|
| Vue / CSS / TS | Instantly in Vite (`:5173`) |
| Python | Uvicorn `--reload` restarts the API |
| `.env` | Restart uvicorn; settings are read at process start |

Tests (from `backend/` with the venv active):

```bash
pytest
```

Or without activating:

```bash
cd backend && .venv/bin/pytest
```

One-process run without Docker (build the Vue app, then serve API + UI from uvicorn):

```bash
cd frontend && npm install && npm run build
cd ../backend
source .venv/bin/activate
uvicorn app.main:app --host 0.0.0.0 --port 8000
```

Open **[http://127.0.0.1:8000](http://127.0.0.1:8000)**. For a machine you share, use the [self-host](#self-host) defaults (`DEBUG=0`, generated secrets, HTTPS in front).

---

## How the app works

### Pages

| Path | What it is |
|---|---|
| `/login` | Sign in |
| `/register` | Create an account with an invite |
| `/library` | Browse and search Open Library as a cover grid, add a book |
| `/shelf` | Your four-column board (drag and drop) |
| `/friends` | Other members + what they’re reading |
| `/friends/:username` | Their shelf, read-only |
| `/invites` | Create / copy invite codes (any member) |

On a phone, swipe the board sideways. Hold a card briefly, then drag it to another column. **Move to…** on the `···` menu does the same thing without dragging.

### Accounts

- Invite-only. No email, no password reset.
- Any signed-in member can mint invites (account menu → **Invites**).
- Treat unused codes like passwords.
- The first unused invite is created only when the database has none (`BOOKCLUB_BOOTSTRAP_INVITE` or a generated `data/.bootstrap_invite`).

### Data

Everything is in **`data/bookclub.db`** (SQLite, WAL mode).

- Backup: `python -m app.backup [outfile]` (or copy the file; stop writes first if you want to be picky; WAL is usually fine).
- Reset local data: stop the server and delete `data/bookclub.db` plus `data/bookclub.db-wal` / `data/bookclub.db-shm` if they exist. Next start creates a fresh DB and the bootstrap invite again.

Book search is proxied to Open Library (no API key). Only books someone actually adds are stored. Covers are loaded from `covers.openlibrary.org`.

### Repo layout

```
backend/           FastAPI app, tests, venv
  app/             Python package (uvicorn app.main:app)
  tests/           pytest
frontend/          Vue 3 + Vite + Pinia
  src/pages/       Screens
  src/components/  Cards, board, sheets
deploy/            Self-host templates (Caddy, systemd, env example)
data/              SQLite file (gitignored except .gitkeep)
```

`npm run build` writes into `backend/app/static/` (also gitignored). Uvicorn serves that folder in “run” mode.

---

## Environment

Loaded from the repo-root `.env` (see `.env.example` for development and `deploy/env.example` for self-host).

| Variable | Default | Meaning |
|---|---|---|
| `SECRET_KEY` | `dev-secret-change-me` locally; empty in Compose | Signs the session cookie. Changing it logs everyone out. When `DEBUG=0`, placeholders and short keys are replaced by a generated `data/.secret_key`. |
| `BOOKCLUB_BOOTSTRAP_INVITE` | `DEV-ONLY` locally; empty in Compose | First invite, only if the DB has none. `DEV-ONLY` is rejected when `DEBUG=0`; an empty value generates `data/.bootstrap_invite`. |
| `DEBUG` | `1` locally; `0` in Compose | `1`: CORS for Vite, `/api/docs`. `0`: production checks, no docs. |
| `BOOKCLUB_HTTPS` | `auto` | `auto`: session cookie is `Secure` only on HTTPS (including `X-Forwarded-Proto`). `1`: always. `0`: never. |
| `BOOKCLUB_TRUSTED_PROXIES` | `*` | Who may set `X-Forwarded-*`. `*` is correct behind a private reverse proxy. |
| `DATABASE_PATH` | `<repo>/data/bookclub.db` | Absolute path if you want it elsewhere. Docker uses `/data/bookclub.db`. |
| `BOOKCLUB_PORT` | `8000` | Host port published by Compose. |
| `BOOKCLUB_DOMAIN` | `localhost` | Hostname for the optional Caddy profile. |
| `PUID` / `PGID` | `1000` | Runtime user for bind-mounted `./data`. |

---

## API (for debugging)

Cookie session: `bookclub_session`. Send it with `credentials: include` / curl `-c` / `-b`.

| Method | Path | Notes |
|---|---|---|
| `GET` | `/api/health` | No auth |
| `POST` | `/api/auth/register` | `{ username, password, invite_code }` — also signs you in |
| `POST` | `/api/auth/login` | `{ username, password }` |
| `POST` | `/api/auth/logout` | |
| `GET` | `/api/auth/me` | Current user |
| `GET` / `POST` | `/api/invites` | List yours / mint one |
| `GET` | `/api/books/search` | Paginated Open Library browse/search (`q`, `subject`, `sort`, `page`, `limit`) |
| `GET` | `/api/shelf` | Your shelf |
| `GET` | `/api/shelf?username=` | Someone else’s shelf |
| `POST` | `/api/shelf` | Add a book |
| `PATCH` | `/api/shelf/{id}` | `{ status, position }` — move / reorder |
| `DELETE` | `/api/shelf/{id}` | Remove from *your* shelf |
| `GET` | `/api/members` | Everyone except you |

Shelf stages: `want_to_read`, `currently_reading`, `finished`, `did_not_finish`.

---

## Troubleshooting

**`DEV-ONLY` is not valid**  
Someone already registered on this database, or you are on `DEBUG=0` (that code is refused). Mint a new invite (UI, `python -m app.create_invite`, or the code in `data/.bootstrap_invite` on a fresh DB).

**Vite loads but login/search fails**  
Backend isn’t running on `:8000`. Start uvicorn first. Vite only proxies `/api`.

**Port 8000 already in use**  
A leftover uvicorn from earlier. Stop it, or pick another port and point Vite’s `server.proxy` at that port.

**`SECRET_KEY is missing or too weak`**  
`DEBUG=0` and the data directory was not writable, so a key could not be generated. Set a long random `SECRET_KEY` or fix permissions on `./data` (`PUID`/`PGID` in Compose).

**`BOOKCLUB_BOOTSTRAP_INVITE` error on start**  
`DEBUG=0` and the DB is empty, but the bootstrap invite is still `DEV-ONLY` and nothing could be generated. Set a real code or allow writes to `./data`.

**Logged in on HTTP, not on HTTPS**  
The reverse proxy is not forwarding `X-Forwarded-Proto`. Or you set `BOOKCLUB_HTTPS=1` while still using plain HTTP (the browser will not store a `Secure` cookie). Leave `BOOKCLUB_HTTPS=auto`.

**Open Library search errors**  
Need outbound HTTPS. The shelf still works if search is down.

**Forgot every invite, nobody can join**  
If the DB already has users, run `python -m app.create_invite` from `backend/` (or `docker compose exec bookclub python -m app.create_invite`). If it has *no* users and no invites, set `BOOKCLUB_BOOTSTRAP_INVITE` and restart.

**Lost your password**  
There is no reset. Delete that row (or the whole DB on a toy install) and register again with a new invite.
