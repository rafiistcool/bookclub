# Bookclub

A tiny private bookclub for 2–5 people. Each person has an account and their own shelf:

**Want to read → Reading → Finished / Did not finish**

Search [Open Library](https://openlibrary.org), open a book to see the synopsis, the club’s rating, and who has it, add it, move it between columns. Every book has a **reading diary** the whole club can see: short entries written at a reading position (“at 45%”, “Finished ★★★★☆”), spoiler-shielded for members who are behind, with one level of replies and emoji reactions. Home is the current club pick — meeting countdown, everyone’s progress, and that book’s diary; Club shows who is here, what you have in common, what was written recently, and the next-up vote. Four palettes in light and dark, per member. Signup is invite-only.

No Redis, no Postgres. One Python process and a SQLite file. Anyone can self-host it.

---

## Self-host

One named club per instance. Releases are published as a container image at
`ghcr.io/rafiistcool/bookclub` (amd64 + arm64). No clone needed — grab the
example compose file and an env file:

```bash
mkdir bookclub && cd bookclub
curl -fsSLO https://raw.githubusercontent.com/rafiistcool/bookclub/main/docker-compose.yml
curl -fsSL  https://raw.githubusercontent.com/rafiistcool/bookclub/main/.env.example -o .env
# set BOOKCLUB_NAME, and either set SECRET_KEY + BOOKCLUB_BOOTSTRAP_INVITE
# or leave them empty to generate files under ./data
docker compose up -d
docker compose logs bookclub
```

Open `http://<host>:8000`. Register with the first-run invite (logs and `data/.bootstrap_invite`), then mint more from **Settings → Invites**. `DEBUG=0` is the Compose default, so `/api/docs` stays closed. Update with `docker compose pull && docker compose up -d`.

HTTPS is yours to provide: put any reverse proxy (Caddy, nginx, Tailscale Serve, Cloudflare Tunnel) on the **same host** as the UI in front of `:8000`, forward `X-Forwarded-Proto`, and leave `BOOKCLUB_HTTPS=auto`. The Vue app calls relative `/api`, so there is no split frontend/API origin; `BOOKCLUB_PUBLIC_URL` is this instance’s public origin.

Full notes (reverse proxy, LAN, NAS `PUID`/`PGID`, bare metal, backup, systemd, releases): **[SELFHOST.md](SELFHOST.md)**.

Building the image from source (contributors; also turns on debug docs and invite `DEV-ONLY` — not for a shared host):

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

Uncomment the local Vite block at the bottom of `.env` (`DEBUG=1`, invite `DEV-ONLY`). Leave `DEBUG=0` if you are using Docker Compose.

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

After that, `DEV-ONLY` is spent. Mint more codes from **Settings → Invites**, or:

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

### Tests

Backend (from `backend/`, venv active):

```bash
pytest
```

Frontend (from `frontend/`):

```bash
npm run typecheck   # vue-tsc, includes templates
npm test            # Vitest: API client, constants
npm run build
```

CI (GitHub Actions) runs pytest and the frontend typecheck + unit tests + build on every push and pull request.

One-process run without Docker (build the Vue app, then serve API + UI from uvicorn):

```bash
cd frontend && npm install && npm run build
cd ../backend
source .venv/bin/activate
uvicorn app.main:app --host 127.0.0.1 --port 8000
```

Open **[http://127.0.0.1:8000](http://127.0.0.1:8000)**. Bind `0.0.0.0` only if you intend the LAN to reach the process. For a machine you share, use the [self-host](#self-host) defaults (`DEBUG=0`, generated secrets, HTTPS in front).

---

## How the app works

### Pages

| Path | What it is |
|---|---|
| `/` | Home: club pick hero with meeting countdown, where everyone is (progress, ratings, takes), the pick's diary (latest three entries), next-up vote card, past picks |
| `/login` | Sign in |
| `/register` | Create an account with an invite |
| `/discover` | Permanent search field, trending row, curated subject shelves |
| `/book/:workId` | Book detail: synopsis, club rating, inline shelf status / progress / stars / take, club-pick and nominate actions, the full diary with composer, other readers |
| `/shelf` | Phone: segmented status filter over a cover grid (stars on finished books). Desktop: drag columns |
| `/club` | Members, recently written diary entries across all books, TBR overlap, next-up vote |
| `/club/:username` | A member's shelf with their ratings |
| `/settings` | Palette and light / dark / system per member, invites, Goodreads import, export, logout |

Old paths (`/library`, `/friends`, `/overlap`, `/invites`, `/pick`) redirect to their new homes.

Every book action — add, move, rate, progress, set as club pick, nominate, save a quote, remove — runs through the same bottom sheet, so it behaves identically on Home, Library, Shelf, the feed, and the detail sheet. Destructive moves show an **Undo** in the toast for a few seconds. Tapping a shelf card’s `···` opens the same sheet; on the desktop board you can also drag.

### Accounts

- Invite-only. No email. Members change their own password under **Settings → Password**; there is no reset link (see Troubleshooting).
- Any signed-in member can mint invites (**Settings → Invites**).
- Treat unused codes like passwords.
- The first unused invite is created only when the database has none (`BOOKCLUB_BOOTSTRAP_INVITE` or a generated `data/.bootstrap_invite`).

### Data

Everything is in **`data/bookclub.db`** (SQLite, WAL mode).

- Backup: `python -m app.backup [outfile]` (or copy the file; stop writes first if you want to be picky; WAL is usually fine). Any member can also download `bookclub.db` from **Settings → Backup**. There is no restore-from-upload; replace the files as below.
- Reset local data: stop the server and delete `data/bookclub.db` plus `data/bookclub.db-wal` / `data/bookclub.db-shm` if they exist. Next start creates a fresh DB and the bootstrap invite again.

Book search is proxied to Open Library (no API key). Only books someone actually adds are stored. Covers are loaded from `covers.openlibrary.org`.

### Repo layout

```
backend/           FastAPI app, tests, venv
  app/             Python package (uvicorn app.main:app)
  tests/           pytest
frontend/          Vue 3 + Vite + Pinia
  src/pages/       Screens
  src/components/  Sheet, cards, board/list, thread, vote, feed, detail sheet
  src/stores/      Pinia: session, club, theme, toast, pick/vote/shelf (SWR), flow (book sheets), push
  public/sw.js     Service worker: offline shell, cover cache, Web Push
deploy/            Self-host templates (container entrypoint, systemd unit, env example)
.github/workflows/ ci.yml (tests on push/PR), release.yml (image to GHCR on v* tags)
data/              SQLite file (gitignored except .gitkeep)
```

`npm run build` writes into `backend/app/static/` (also gitignored). Uvicorn serves that folder in “run” mode.

---

## Environment

Loaded from the repo-root `.env` (copy `.env.example` or `deploy/env.example`). Compose injects the same keys; bare-metal uvicorn now reads that file too.

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
| `VAPID_SUBJECT` | `BOOKCLUB_PUBLIC_URL` or `mailto:bookclub@localhost` | Contact claim sent to push services (`mailto:` or `https://`). |
| `BOOKCLUB_PORT` | `8000` | Host port published by Compose. |
| `PUID` / `PGID` | `1000` | Runtime user for bind-mounted `./data`. |

---

## API (for debugging)

Cookie session: `bookclub_session`. Send it with `credentials: include` / curl `-c` / `-b`.

| Method | Path | Notes |
|---|---|---|
| `GET` | `/api/health` | No auth; also pings SQLite |
| `GET` | `/api/config` | Club name, theme, timezone (no auth) |
| `POST` | `/api/auth/register` | `{ username, password, invite_code }` — also signs you in |
| `POST` | `/api/auth/login` | `{ username, password }` |
| `POST` | `/api/auth/logout` | |
| `GET` | `/api/auth/me` | Current user |
| `PATCH` | `/api/auth/password` | `{ current_password, new_password }` |
| `GET` / `PATCH` | `/api/auth/notifications` | Per-user push toggles (`notify_meeting`, `notify_pick`, `notify_note`) |
| `GET` / `POST` | `/api/invites` | List yours / mint one |
| `GET` | `/api/books/search` | Paginated Open Library browse/search (`q`, `subject`, `sort`, `page`, `limit`); RAM-cached ~18h (empty pages not cached). Short `q` is 400, rate limits 429, other OL failures 502. ISBN-shaped `q` uses `isbn:`. |
| `POST` | `/api/books/custom` | Add a club-only book when Open Library misses (`title`, optional `authors` / `year` / `description`). Key is `/works/BC…` |
| `GET` | `/api/books/works/{OL…W}` | Book page. Local SQLite when the work is already in the club DB; Open Library only for an unknown work (then imported) |
| `POST` | `/api/books/works/{OL…W}/refresh` | Re-fetch Open Library metadata into the local Book row |
| `GET` | `/api/books/work/{OL…W}` | Richer details (pages, OL rating, members). Local when the book is known; no automatic refresh |
| `POST` | `/api/books/work/{OL…W}/refresh` | Same, explicit refresh |
| `GET` | `/api/books/isbn/{isbn}` | Resolve an ISBN-10/13 to a work |
| `GET` | `/api/shelf` | Your shelf (items carry `started_at` / `finished_at`) |
| `GET` | `/api/shelf?username=` | Someone else’s shelf |
| `POST` | `/api/shelf` | Add a book (optional rating / take / dnf_reason / progress) |
| `PATCH` | `/api/shelf/{id}` | `{ status, position, rating, take, dnf_reason, progress }` |
| `POST` | `/api/shelf/import` | Goodreads library export CSV |
| `DELETE` | `/api/shelf/{id}` | Remove from *your* shelf |
| `GET` | `/api/members` | Everyone except you |
| `GET` | `/api/pick` | Current club pick |
| `GET` | `/api/pick/history` | Ended picks |
| `PUT` | `/api/pick` | Set / replace the club pick (optional meeting) |
| `DELETE` | `/api/pick` | Clear the current pick (kept in history) |
| `GET` | `/api/pick/meeting.ics` | Calendar file for the current meeting |
| `GET` | `/api/books/works/{OL…W}/posts` | The book’s diary: top-level entries oldest first with nested `replies`, plus `my_progress` / `my_status` so the client can shield entries flagged past where you are |
| `POST` | `/api/books/works/{OL…W}/posts` | New entry `{ body, spoiler_upto?, parent_id?, book? }`. `book` (title/authors/cover/year) creates the book row if nobody has shelved it yet; `parent_id` must be a top-level entry on the same book. Snapshots your progress and status |
| `PATCH` / `DELETE` | `/api/posts/{id}` | Edit / delete your own entry. Deleting one that has replies leaves a `deleted` tombstone |
| `POST` | `/api/posts/{id}/reactions` | Toggle `{ emoji }` (❤️ 👍 😂 😮 🔥 📚) |
| `GET` | `/api/diary?before&limit` | Club-wide feed of recent entries across all books, newest first (`before` = entry id cursor) |
| `GET` / `POST` | `/api/pick/posts`, `/api/pick/{id}/posts` | Legacy pick notes, imported into the diary on start; no longer used by the UI |
| `GET` / `POST` | `/api/pick/{id}/milestones` | Reading schedule; `PATCH` / `DELETE` `/api/pick/{id}/milestones/{mid}` |
| `GET` | `/api/overlap` | Shared TBR (`include_reading`) |
| `GET` | `/api/vote` | Open next-up vote (with `closes_at`, `not_voted`, `leader_id`); a passed deadline applies the leader |
| `PATCH` | `/api/vote` | `{ closes_at }` set / clear the deadline |
| `POST` | `/api/vote/nominations` | Nominate a book |
| `POST` | `/api/vote/cast` | `{ nomination_id }` — one vote each |
| `POST` | `/api/vote/apply` | Confirm a winner as the club pick |
| `GET` | `/api/vote/suggestions` | Candidates scored from shared TBR and past-pick subjects |
| `GET` | `/api/activity` | Club feed (`before`, `limit`, `username`) |
| `GET` | `/api/stats` | Year in review (`year`) |
| `GET` / `POST` | `/api/quotes` | Saved quotes (`work`, `username`); `PATCH` / `DELETE` `/api/quotes/{id}` |
| `GET` | `/api/push/vapid` | Public key for `PushManager.subscribe` |
| `GET` / `POST` / `DELETE` | `/api/push/subscriptions` | This device’s subscriptions |
| `POST` | `/api/push/test` | Send yourself a test notification |
| `GET` | `/api/backup` | Download a SQLite copy (`bookclub.db`) |

Shelf stages: `want_to_read`, `currently_reading`, `finished`, `did_not_finish`.

---

## Troubleshooting

**`DEV-ONLY` is not valid**  
Someone already registered on this database, or you are on `DEBUG=0` (that code is refused). Mint a new invite (UI, `python -m app.create_invite`, or the code in `data/.bootstrap_invite` on a fresh DB).

**Vite loads but login/search fails**  
Backend isn’t running on `:8000`. Start uvicorn first. Vite only proxies `/api`.

**Port 8000 already in use**  
A leftover uvicorn from earlier. Stop it, or pick another port and point Vite’s `server.proxy` at that port.

**`SECRET_KEY must be a long random string when DEBUG=0`**  
You set the example secret (or a short one) in production. Paste a long random value, or leave `SECRET_KEY` empty so `data/.secret_key` is generated.

**`BOOKCLUB_BOOTSTRAP_INVITE cannot be DEV-ONLY`**  
`DEBUG=0` refuses that demo code. Set a real invite, or leave the variable empty to generate one.

**Logged in on HTTP, not on HTTPS**  
The reverse proxy is not forwarding `X-Forwarded-Proto`. Or you set `BOOKCLUB_HTTPS=1` while still using plain HTTP (the browser will not store a `Secure` cookie). Leave `BOOKCLUB_HTTPS=auto`.

**Open Library search errors**  
Need outbound HTTPS. The shelf still works if search is down.

**Forgot every invite, nobody can join**  
If the DB already has users, run `python -m app.create_invite` from `backend/` (or `docker compose exec bookclub python -m app.create_invite`). If it has *no* users and no invites, set `BOOKCLUB_BOOTSTRAP_INVITE` and restart.

**Lost your password**  
Signed in somewhere? Change it under Settings → Password. Otherwise there is no reset link: delete that row (or the whole DB on a toy install) and register again with a new invite.

**Push notifications never arrive**  
Web Push needs HTTPS (or `localhost`) and, on iPhone, the app added to the Home Screen first. Check Settings → Notifications → *Send a test*. If you rotated `VAPID_PRIVATE_KEY`, every device has to turn notifications off and on again.

**Upgrading from an older database**  
Just start the new version. Missing columns and tables are added on boot (`app/db.py` → `COLUMN_MIGRATIONS`); existing rows keep working. Reading dates (`started_at` / `finished_at`) are only known for books moved after the upgrade.
