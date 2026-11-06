<div align="center">

# Bookclub

**An invite-only book club and reading tracker.** Shelves, clubs with a shared pick, a spoiler-shielded diary, and friends. One Rust server on Postgres serves the API and the Vue app.

Today it runs as a private instance on the homelab for one club: invite-only, with mail off and no legal texts. The hosted public service comes with the public launch. Until then the name stays "Bookclub".

</div>

- **Shelves** — Cover grids with a visible reading status on every book. Hold and drag to arrange books; tap a status to change it. Finished books are grouped by completion month, newest first, in both All and Finished views. Drag within a month to reorder, or into another month to correct the completion month. Empty months can be added as drop targets. Order and date changes sync across devices. Under the expanded book actions, enter a month/year or an exact date; month-only changes use the first day of that month in your time zone.
- **Clubs** — Create up to 10 and join any number, with an owner, admins and members. Each club has a current pick with everyone's progress and milestones, a meeting with "Add to calendar", a next-up vote, discussions, TBR overlap and club stats.
- **Diary** — Short notes at a reading position ("at 45%", "Finished ★★★★☆"). Spoiler-shielded for readers who are behind; one level of replies and emoji reactions. Your own diary, quotes and reading stats have their own pages, and a note can be "Only me".
- **Friends** — Ask someone by their exact username; once they accept, you see each other's shelves, ratings, quotes and notes, as club mates do, and what they are reading. Club admins invite a friend into a club with a personal invite only that friend can use. Blocking is silent and ends the friendship; it does not reach into clubs you share.
- **Discover** — Search by title, author, or ISBN in our own catalog, filled from [Open Library](https://openlibrary.org) and the German National Library (DNB). Private search history follows your account across devices (last 20 terms, individually or fully deletable). Trending, new German releases, subject shelves, and **add your own** when the catalog misses. **Scan a barcode** to find a book by its ISBN.
- **Covers** — Every catalog and imported book gets a cover where one can be found:
  - from Open Library, by title and author, or by a translation's original title from the DNB;
  - from a reader's photo of their copy, which the book page asks for when a book on your shelf has none;
  - on the private instance only, from the publisher via the DNB portal.

  A book without a picture shows a cloth binding with its title and author stamped on it. Every picture is stored and served from our own origin. Instance admins review cover suggestions and the books still without a cover under **Admin → Covers**, and can set a cover directly on the book page.
- **Account** — Email, **password change**, signed-in devices and a per-account time zone in Settings, plus the Goodreads import, "Export my data" and deleting the account.
- **Looks** — Four palettes (Paper, Slate, Forest, Ink) in light / dark / system, per account. Optional profile photo; initials if you skip it. English or German, toggled in Settings.
- **One image** — The Rust server (axum, sqlx) serves the API and the built Vue app; Postgres runs beside it. amd64.

---

## Running it

- **The private instance** follows [`docs/phase3/private-move.md`](docs/phase3/private-move.md). Updates are applied by hand, each after a VM backup (§9). A forgotten password is reset on the host (§10).
- **The image and the hosted stack** (compose with Postgres, Caddy, backups, updates, the legacy import) are in [`deploy/hosted/README.md`](deploy/hosted/README.md), the operator runbook.
- **Releases:** a `vX.Y.Z` tag on Forgejo publishes `git.tillhome.de/rafiistcool/bookclub-server:X.Y.Z` and moves `X.Y` and `stable` when it is the highest release (there is no `latest`). The registry needs a sign-in. The old Python app is kept on the branch `legacy-python` (tag `legacy-python-final`); its image `rafiistcool/bookclub` gets no new versions.

---

## Develop

Two processes: the Rust server on `127.0.0.1:8080` and Vite on `:5173`. Vite proxies `/api` and `/media` to the server, so the browser only talks to `localhost:5173`.

Needs the stable Rust toolchain, Node **20+** and Docker (for Postgres). [CLAUDE.md](CLAUDE.md) has every command, including parallel worktrees and the agents VM.

**Postgres and the schema** (once):

```bash
docker compose -f server/compose.yaml up -d --wait
export DATABASE_URL=postgres://bookclub:bookclub@127.0.0.1:5432/bookclub
cargo install sqlx-cli --version 0.9.0 --locked --no-default-features --features rustls,postgres
cd server && cargo sqlx migrate run
```

The `query!` macros check against the live schema at compile time, so `DATABASE_URL` must be set when you build.

**Server** (leave this running):

```bash
cd server
DEV_MODE=true cargo run
```

It applies the migrations at startup. [http://127.0.0.1:8080/api/health](http://127.0.0.1:8080/api/health) should return `{"status":"ok"}`.

**Frontend** (second terminal):

```bash
cd frontend
npm ci
npm run dev
```

Open **[http://localhost:5173](http://localhost:5173)**, not port 8080.

**First account.** Sign-up needs an invite. `cd server && DEV_MODE=true cargo run -- beta-invite` prints a sign-up code and its join link; `DEV_MODE=true cargo run -- make-admin <username>` makes that account an instance admin, who can mint more under **Admin → Invites**. Username: `a–z`, `0–9`, `_`, 2–32 characters. Password: 8+ characters. `cargo run -- --help` lists the other commands.

| You change | Where it shows up |
|---|---|
| Vue / CSS / TS | Instantly in Vite (`:5173`) |
| Rust | Restart `cargo run` |
| A `query!` macro | Run `cargo sqlx prepare -- --all-targets` in `server/` and commit `server/.sqlx/` |
| A DTO | Run `npm run gen:api` in `frontend/` and commit `src/api/generated.ts` |

**One process.** `cd frontend && npm run build` writes `frontend/dist/`; start the server with `STATIC_DIR=../frontend/dist` and open [http://127.0.0.1:8080](http://127.0.0.1:8080).

**Tests.** CI runs all of these on every push and pull request:

```bash
cd frontend && npm run typecheck && npm test && npm run build && npm run check:api
cd server && cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked && cargo sqlx prepare --check -- --all-targets
```

---

## How it works

```mermaid
flowchart LR
  Browser -->|"same origin /api, /media"| Server["Rust server + Vue app"]
  Server --> PG[("Postgres")]
  Server --> Media[("media store")]
  Server -.->|"search, covers"| OL[Open Library]
  Server -.->|"ISBN lookup, harvest, original titles"| DNB[DNB]
```

The catalog is our own, in Postgres: works with editions, served locally first. A lookup asks Open Library, and the DNB for German ISBNs that Open Library does not know; the DNB harvester adds new German releases. Books from the old app keep their links: `/book/OL…W`, and the legacy keys `BC…`, `ISBN…` and `GB…`, still open the right work. The browser loads nothing from another origin: covers and profile photos are stored by the server and served under `/media`.

Sign-up needs an invite: a club admin's invite link (`/join/<code>`), or a sign-up invite from an instance admin (Admin → Invites, or `beta-invite`). Change your password under **Settings → Password**. On the private instance mail is off, so "Forgot password" sends nothing; see [Troubleshooting](#troubleshooting).

| Path | What it is |
|---|---|
| `/` | Your clubs, currently reading, favourites, and the feed |
| `/discover` | Search and browse the catalog; add your own when it misses |
| `/scan` | Scan a book's barcode (or type its ISBN), then shelve it, set your edition, or add the book when the catalog has none |
| `/shelf` | Your cover grid, reading-status filters, and saved book order |
| `/shelf/physical` | A personal bookshelf with draggable spines, shelf finishes, and saved placements |
| `/diary`, `/quotes`, `/stats` | Your diary, quotes and reading stats |
| `/clubs`, `/clubs/new` | Your clubs; create one |
| `/c/:slug` | A club: the pick, everyone's progress, next up, past picks |
| `/c/:slug/discussions`, `vote`, `members`, `overlap`, `events`, `stats` | The club's other tabs |
| `/club` | Goes to your last club |
| `/u/:username`, `/u/:username/shelf`, `/u/:username/stats` | Another reader's shelf, bookshelf and stats, for friends and club mates |
| `/book/:id` | Synopsis, ratings, editions, shelf actions, diary, cover prompt |
| `/friends` | Friends, requests, club invitations, blocks |
| `/join/:code` | An invite link |
| `/settings` | Profile, email, password, devices, notifications, favourites, language, time zone, theme, Goodreads import, data export, delete account |
| `/admin` | Instance admins only: Requests, Reports, Covers, Invites, Accounts |
| `/imprint`, `/privacy`, `/terms` | The operator's legal texts |

Old paths (`/library`, `/friends/:username`, `/club/:username`, `/club/:username/shelf`, `/overlap`, `/invites`, `/pick`, `/register`) still redirect.

Choose **Bookshelf** in the view switch on the Shelf tab to arrange books across wooden
shelves. On a phone, hold a spine briefly before dragging. Tap or keyboard-focus
a book to see it, then restyle it: colour, thickness, and whether it stands
upright, lies in a stack with its neighbours, or faces out with its cover. Move
it with the position controls. Spines take their colour from the book's cover
when it has one (the server samples the cover image once) and from a stable
palette otherwise, with thickness from the page count; they are generated
rather than photographs of an edition's actual spine. Placements, custom spines,
and the shelf finish save to your account without changing reading statuses or
completion dates. A failed save restores the previous arrangement.
Add plants, vases, candles, picture frames, bookends, or a globe and drag them
between the books, or switch on the string lights. Decorations have their own
colour and position controls. Friends and club mates can look at your bookshelf
(`/u/<name>/shelf`); only you can change it. Remove a shelf board
using its bin button; all its books and decorations move to an adjacent shelf.
At least one shelf remains, and Undo restores the last successful change.

```
frontend/              Vue 3 + Vite + Pinia; npm run build writes frontend/dist/
server/                The Rust server, its Postgres migrations and tests
deploy/hosted/         The image's compose stack, Caddy, backups and updates (its README is the runbook)
deploy/cutover/        The legacy import's checks and the host facts of the move
deploy/legacy-origin/  The kill switch for the old app's service worker
docs/                  The plan, the phase designs and notes
```

---

## Configure

Every setting is in [`deploy/hosted/env.example`](deploy/hosted/env.example), with its default; `docs/phase1/conventions.md` §11 documents them. Outside `DEV_MODE` the server refuses to start without the ones production needs, and names the missing one. The private instance sets these:

| Variable | Meaning |
|---|---|
| `BOOKCLUB_TAG` | The image version, pinned and raised by hand |
| `PUBLIC_URL` | The origin the browser uses. The origin check allows only this one, so a wrong value gives 403 `Cross-site request refused` |
| `TRUSTED_PROXIES` | The reverse proxy's address, whose `X-Forwarded-For` the rate limits trust |
| `SIGNUP_MODE=invite` | Sign-up only with an invite |
| `EMAIL_TRANSPORT=none` | Mail off: no mail leaves, and password resets go through `set-password` |
| `DNB_HARVEST` | The DNB harvester for new German releases |
| `DNB_PORTAL_COVERS` | Publisher covers from the DNB portal; private instance only, and `SIGNUP_MODE=open` refuses it |

---

## API

Cookie session: `bookclub_session`, or `__Host-bookclub_session` when the cookie is `Secure` (`credentials: include`); apps can use bearer tokens (`/api/auth/tokens`). The OpenAPI document is at `GET /api/openapi.json`, or offline from `cargo run -- openapi`. `GET /api/health` is unauthenticated and answers `{"status":"ok"}`, or 503 when Postgres does not answer within 2 s. Shelf stages: `want_to_read`, `currently_reading`, `finished`, `did_not_finish`.

---

## Troubleshooting

**Forgot a password while mail is off** — "Forgot password" sends nothing, though it still says a link is on its way. The member asks the operator, who runs `docker compose exec -T app bookclub-server set-password <username>` on the host ([`private-move.md`](docs/phase3/private-move.md) §10). It prints a new password and ends every session of the account; the member changes it under **Settings → Password** the same day. After too many failed sign-ins the app still holds the account for up to 10 minutes.

**Vite loads but sign-in or search fails** — the Rust server isn't running on `127.0.0.1:8080`. Start it first (see [Develop](#develop)). Vite only proxies `/api` and `/media`.

**403 `Cross-site request refused` after signing in** — `PUBLIC_URL` is not the origin in the browser's address bar. In `DEV_MODE` it defaults to `http://localhost:5173`; set it when you open the app under another host or port.

**The server does not start outside `DEV_MODE`** — it names the missing or refused setting in its last log line; see "When it does not start" in [`deploy/hosted/README.md`](deploy/hosted/README.md).

**`query!` fails to compile** — `DATABASE_URL` is not set, or the database is behind the migrations: export it and run `cargo sqlx migrate run` in `server/`.

**`/scan` offers only typing the ISBN** — the camera needs a secure context: HTTPS, or `localhost`.
