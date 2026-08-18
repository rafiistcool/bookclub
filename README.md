# Bookclub

A tiny private bookclub for 2–5 people. Each person has an account and their own shelf:

**Want to read → Reading → Finished / Did not finish**

Search [Open Library](https://openlibrary.org), add a book, drag it between columns (or use **Move to…**). Friends can look at each other’s shelves. Signup is invite-only.

No Redis, no Postgres. One Python process and a SQLite file.

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

---

## Run

Two ways to actually *use* the site (no Vite). Pick one.

### Option A — one process, no Docker

Build the Vue app into `backend/app/static`, then serve API + UI from uvicorn.

```bash
cd frontend
npm install
npm run build

cd ../backend
# use the same venv you created for develop
source .venv/bin/activate   # or activate.fish
uvicorn app.main:app --host 0.0.0.0 --port 8000
```

Open **[http://127.0.0.1:8000](http://127.0.0.1:8000)**.

For a machine you share with friends, set real secrets first:

```bash
DEBUG=0 \
SECRET_KEY='paste-a-long-random-string' \
BOOKCLUB_BOOTSTRAP_INVITE='a-secret-you-share-once' \
BOOKCLUB_HTTPS=1 \
uvicorn app.main:app --host 0.0.0.0 --port 8000
```

`DEBUG=0` refuses to start if `SECRET_KEY` is still the example value, and refuses the `DEV-ONLY` bootstrap invite.

Put HTTPS in front (Caddy, Tailscale Serve, Cloudflare Tunnel). Set `BOOKCLUB_HTTPS=1` so the session cookie is `Secure`.

### Option B — Docker

From the repo root (needs a `.env` or you can rely on compose defaults):

```bash
docker compose up --build
```

Open **[http://127.0.0.1:8000](http://127.0.0.1:8000)**.

The database lives in `./data/bookclub.db` on the host. Compose defaults to `DEBUG=1` and invite `DEV-ONLY` so a first run works. For a real deploy, put this in `.env`:

```bash
DEBUG=0
SECRET_KEY=paste-a-long-random-string
BOOKCLUB_BOOTSTRAP_INVITE=a-secret-you-share-once
BOOKCLUB_HTTPS=1
```

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
- The first unused invite is created only when the database has none (`BOOKCLUB_BOOTSTRAP_INVITE`).

### Data

Everything is in **`data/bookclub.db`** (SQLite, WAL mode).

- Backup: copy that file (stop writes first if you want to be picky; WAL is usually fine).
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
data/              SQLite file (gitignored except .gitkeep)
```

`npm run build` writes into `backend/app/static/` (also gitignored). Uvicorn serves that folder in “run” mode.

---

## Environment

Loaded from the repo-root `.env` (see `.env.example`).

| Variable | Default | Meaning |
|---|---|---|
| `SECRET_KEY` | `dev-secret-change-me` | Signs the session cookie. Changing it logs everyone out. Required to be non-default when `DEBUG=0`. |
| `BOOKCLUB_BOOTSTRAP_INVITE` | `DEV-ONLY` | First invite, only if the DB has none. `DEV-ONLY` is rejected when `DEBUG=0`. |
| `DEBUG` | `1` | `1`: CORS for Vite, `/api/docs`. `0`: production checks, no docs. |
| `BOOKCLUB_HTTPS` | `0` | `1`: session cookie is `Secure` (use behind HTTPS). |
| `DATABASE_PATH` | `<repo>/data/bookclub.db` | Absolute path if you want it elsewhere. Docker uses `/data/bookclub.db`. |

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
Someone already registered on this database. Mint a new invite (UI or `python -m app.create_invite`), or delete `data/bookclub.db*` and start over.

**Vite loads but login/search fails**  
Backend isn’t running on `:8000`. Start uvicorn first. Vite only proxies `/api`.

**Port 8000 already in use**  
A leftover uvicorn from earlier. Stop it, or pick another port and point Vite’s `server.proxy` at that port.

**`SECRET_KEY must be set when DEBUG=0`**  
You’re in run/production mode with the example secret. Set a long random `SECRET_KEY`.

**`BOOKCLUB_BOOTSTRAP_INVITE` error on start**  
`DEBUG=0` and the DB is empty, but the bootstrap invite is still `DEV-ONLY`. Set a real code.

**Open Library search errors**  
Need outbound HTTPS. The shelf still works if search is down.

**Forgot every invite, nobody can join**  
If the DB already has users, run `python -m app.create_invite` from `backend/`. If it has *no* users and no invites, set `BOOKCLUB_BOOTSTRAP_INVITE` and restart.

**Lost your password**  
There is no reset. Delete that row (or the whole DB on a toy install) and register again with a new invite.
