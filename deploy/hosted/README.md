# Bookclub hosted: operator runbook

This directory runs the hosted service on one VM: `compose.yaml` starts the app and its Postgres, and `env.example` lists every setting. `backup.sh` and `restore-test.sh` back it up with restic, and `update.sh` follows the release tag, all from the systemd timers in `systemd/`. The commands below run as root on the VM, in `/opt/bookclub`, unless a step says otherwise.

## What runs where

- **`app`:** the image `git.tillhome.de/rafiistcool/bookclub-server:<BOOKCLUB_TAG>`, built from the repo's root `Dockerfile`. It holds the server binary and the web app (`/app/static`), runs as uid 1000 on a read-only root file system, and listens on 8080 inside the container. It applies the database migrations at startup.
- **`postgres`:** `postgres:17` with the data in the Docker volume `bookclub_postgres-data`. It publishes no port, and its network has no route out.
- **On the host:**

  | Path | What |
  |---|---|
  | `/opt/bookclub/compose.yaml` | from `deploy/hosted/` |
  | `/opt/bookclub/.env` | the settings, mode 0600 (from `env.example`) |
  | `/srv/bookclub/media` | pictures (covers, avatars), mounted at `/data/media`, owned by uid 1000 |
  | `/srv/bookclub/legal` | the legal texts, mounted read-only at `/app/legal`, owned by uid 1000 |
  | `/opt/bookclub/backup.sh`, `restore-test.sh`, `backup-lib.sh`, `update.sh` | the backup and update scripts, from `deploy/hosted/` |
  | `/opt/bookclub/backup.env`, `restic-password` | their settings and the repository password, mode 0600 |
  | `/var/lib/bookclub-backup` | the newest dump (`db/`), the restic cache, the last runs' logs; mode 0700 |
  | `/etc/systemd/system/bookclub-*` | the backup, restore-test and update units and timers, from `deploy/hosted/systemd/` |

## Requirements

- Debian 13 with Docker Engine 25 or newer and the compose plugin 2.24 or newer, and `curl` for the heartbeats.
- `docker login git.tillhome.de` with a read-only package token, as root: the update timer pulls with it.
- **An SMTP account, for staging too.** Without `DEV_MODE` the server does not start without `SMTP_URL` and `EMAIL_FROM`, and `EMAIL_TRANSPORT=log` needs `DEV_MODE`, which this setup never sets. Until the mail provider is chosen, staging uses an interim account. `SMTP_URL` must use TLS: `smtps://…:465`, or `smtp://…:587?tls=required`.
- The reverse proxy (Caddy) terminates TLS and forwards to `BOOKCLUB_LISTEN:BOOKCLUB_PORT`. Only the proxy may reach that port (see "The VM's network").

## First boot

1. Copy the files from a checkout of the repo and create the settings:

   ```sh
   install -d -m 0750 /opt/bookclub
   cp deploy/hosted/compose.yaml /opt/bookclub/
   install -m 0600 deploy/hosted/env.example /opt/bookclub/.env
   cd /opt/bookclub
   ```

2. Edit `.env` and replace every `CHANGE_ME`. Set `PUBLIC_URL`, `CONTACT_EMAIL` and `EMAIL_FROM` too. `openssl rand -hex 32` makes a `POSTGRES_PASSWORD` that needs no escaping in `DATABASE_URL`.

3. Create the two host directories, owned by uid 1000:

   ```sh
   install -d -o 1000 -g 1000 -m 0750 /srv/bookclub/media /srv/bookclub/legal
   ```

   The image's own `/data/media` belongs to uid 1000, but a bind mount brings the host directory's owner with it:
   - Without the directories, `docker compose up` stops with `bind source path does not exist: /srv/bookclub/media`. `compose.yaml` does not let Docker create them, because Docker would create them owned by root.
   - With the wrong owner, the app stops at startup with `MEDIA_DIR /data/media is not writable: Permission denied (os error 13)`.

4. Start, and wait until both services are healthy:

   ```sh
   docker compose up -d --wait
   docker compose ps
   docker compose exec app bookclub-server healthcheck
   ```

   The healthcheck prints `GET http://127.0.0.1:8080/api/health answered 200 OK`. Docker runs the same check every 20 s: the app turns `unhealthy` after three misses in a row, for example when the database is gone.

5. Check the mail settings:

   ```sh
   docker compose exec app bookclub-server send-test-mail you@example.com
   ```

   It sends one mail through `SMTP_URL` and exits non-zero with the SMTP server's answer if that fails. The mail carries a `Message-ID` on the domain of `EMAIL_FROM`.

6. Set up the backups (see "Backups") before the first real data arrives.

7. Then the automatic updates, which back up before every update (see "Updates and rollback", "Setting up").

## The first account

In production, only after the legacy import, which needs an empty database (see "The legacy import"). Staging is wiped before the import anyway.

1. Mint a sign-up invite. It prints the code and the join link, is single-use and expires in 30 days (`--uses N` and `--days D` change that):

   ```sh
   docker compose exec app bookclub-server beta-invite
   ```

2. Open the link and sign up.

3. Make that account an instance admin:

   ```sh
   docker compose exec app bookclub-server make-admin <username>
   ```

   The admin mints further invites on the Invites tab of `/admin`, and withdraws one there. `bookclub-server revoke-admin <username>` ends an admin's rights, and `bookclub-server unsuspend <username>` lifts a suspension, both from the host, for when no admin can use the admin page (an only admin suspended by another, for one).

The admin page `/admin` holds the catalog requests and the reports, each with its age, an "Overdue" mark after 72 hours, and a "Decided" view with the history and the 90-day figures. Readers report covers, the profile pictures of people they share a club with, and books and editions readers added: "Take down" removes a picture for good, "Remove the text" withdraws what a reader typed and keeps the book on everyone's shelves (`docs/legal/takedown.md` §4). "Edit as admin", on a book page and in the queue, changes a book's facts at once. "Take down a picture by URL", above the open reports, records an emailed notice with its arrival time. Each decision mails the reporters the outcome and, when something was removed, whoever uploaded or added it a statement of reasons. The "Covers" tab lists the cover suggestions ("Use this cover" or "Not this book", which keeps that Open Library book out for good) and the catalog and imported books without a cover, each with "Search now" and "Upload a picture"; an admin also sets any book's or edition's cover on its book page. The server looks for covers by itself: every 10 minutes it searches Open Library by title and author for a few books without one (books readers use first, one request every 2 seconds, pausing while Open Library is down) and fetches again Open Library pictures that failed to load; a clear match becomes the cover, the rest waits on the Covers tab. No setting turns it on or off, and it never replaces a cover a reader or an admin set. The "Accounts" tab suspends an account (no sign-in, every session ended, admin rights ended for good, the invites it knows revoked, content kept) and lifts a suspension. A suspended account cannot export or delete its data in the app: its owner writes to `CONTACT_EMAIL`, and the operator follows `docs/legal/takedown.md` §7. Instance admins with a verified address get a mail when new reports arrive (at most one an hour) and a digest of new catalog requests (at most one a day), through `SMTP_URL`.

The app deletes old records itself, every hour, with nothing to configure (Q14, `docs/legal/privacy-facts.md` §7): decided reports and catalog requests 3 years after the decision, the photo of a rejected cover request with its request, and sign-ups that stayed empty (no verified address, no data, not an admin, not suspended) 30 days after their last sign-in or the lifting of a suspension. Open reports and requests stay until they are decided. The app log shows only the counts.

## Updates and rollback

`BOOKCLUB_TAG` in `.env` names the image tag the app follows: `stable` follows the highest release (see "Releases"), and an exact version such as `0.11.0` follows only that version's tag, which no release moves. `update.sh`, which `bookclub-update.timer` starts every 3 minutes, carries out every change of that tag:

1. `docker compose pull app`. While the tag has not moved, that is one request to the registry.
2. If the app already runs the tag's image, nothing happens.
3. Otherwise `backup.sh` runs first: the new version applies its database migrations when it starts, and migrations only go forward. If the backup fails, the app stays as it is.
4. `docker compose up -d --no-deps --wait app` replaces the app container, and nothing else, and waits up to `UPDATE_WAIT_SECONDS` (300) for it to turn healthy. `update.sh` then checks that the app runs the new image, and removes the old image once no tag names it.
5. The heartbeat `UPDATE_HEARTBEAT_URL` of `backup.env`:
   - `<url>/0` after every check that finds the app up to date and healthy;
   - `<url>/start` before an update, and `<url>/<exit status>` with the end of its log after it (every public id blanked, as for the backups);
   - `<url>/1` while the app runs the tag's image but is unhealthy or restarting, so a failed update keeps the check down until it is fixed;
   - `<url>/1` while a failed update waits for its next try (below);
   - nothing when the registry does not answer (the run exits 1): only an outage that outlasts the check's grace time alerts.

A failed update of an image (the backup failed, or the app did not come up on it) is tried again an hour later, not every 3 minutes, because every try starts with a full backup. `/var/lib/bookclub-backup/update.failed` holds the image and the time; the runs in between log `the next try comes after …` and ping 1. A new image for the tag is tried at once.

`update.sh` never starts an app that is not running. While the stack is down (a restore) or the app is stopped (`docker compose stop app`, the legacy import), it changes nothing and pings nothing. If the app is stopped while an update's backup runs, which can wait hours for the nightly backup or the restore test, the update stops before it touches the app, and pings 1.

**Who can deploy.** Whoever can push an image to `git.tillhome.de/rafiistcool/bookclub-server` deploys it to production within about 3 minutes, with nobody in the loop: the timer starts whatever image the followed tag names now, and the app container gets `.env` (the database password, the SMTP credentials) and a route out. That is the user `rafiistcool`, every token with `write:package` on that user (`REGISTRY_TOKEN`, which the publish job hands its job image and BuildKit, both pulled by tag: `docs/phase1/security-review.md`, Phase 2 Wave 1, finding 2), and whoever controls the Forgejo instance. Nothing checks a digest or a signature. Registry tags can be moved, exact versions too: `release.yml` pushes each one once, but the registry takes a second push under the same tag, and the timer deploys it like a release. A pinned exact version is therefore no freeze; a pinned digest is (see "Rollback").

Stop the timer for such work anyway, so that no update starts in between, and wait for a run that is already busy: stopping the timer does not stop it, and its backup would dump the database in the middle of the work. The `flock` returns once no `update.sh` runs (it takes that lock); `systemctl is-active` does not help, because a running oneshot service is `activating`, not `active`. Afterwards, start the timer again:

```sh
systemctl stop bookclub-update.timer
flock /var/lib/bookclub-backup/.update.lock true
# … the work …
systemctl start bookclub-update.timer
```

### Setting up

After the backups (see "Backups", "Setting up"), because every update starts with one:

1. The script, next to the backup scripts. It reads `backup.env` too, so the backup it runs covers the same stack:

   ```sh
   cp deploy/hosted/update.sh /opt/bookclub/
   ```

2. The heartbeat: a third check at healthchecks.io, "update", with a period of 5 minutes and a grace time of 1 hour. Its ping URL goes into `UPDATE_HEARTBEAT_URL` in `backup.env`. The grace time covers an update that waits for the nightly backup or the Sunday restore test, which hold the backup lock; it alerts when the checks stop, when the registry stays out of reach, and at once on a failed update.

3. One run by hand. It prints a line such as `git.tillhome.de/rafiistcool/bookclub-server:stable is up to date (…@sha256:…), and the app is healthy`, or updates the app:

   ```sh
   /opt/bookclub/update.sh
   ```

4. The timer:

   ```sh
   cp deploy/hosted/systemd/bookclub-update.* /etc/systemd/system/
   systemctl daemon-reload
   systemctl enable --now bookclub-update.timer
   systemctl list-timers 'bookclub-*'
   ```

### By hand

- A check now, as the timer runs it: `systemctl start bookclub-update`. The runs: `journalctl -u bookclub-update`; the last update's log: `/var/lib/bookclub-backup/update.log`.
- After fixing what failed an update, try it again now instead of after the hour: `rm /var/lib/bookclub-backup/update.failed`, then `systemctl start bookclub-update`.
- The same steps without the script:

  ```sh
  /opt/bookclub/backup.sh
  docker compose pull app
  docker compose up -d --wait app
  ```

### Rollback

To roll back, or to hold updates back, pin an exact version in `.env`. The timer carries the pin out like a release: its next run backs up and starts that version. `systemctl start bookclub-update` does it now, and `docker compose images app` shows the tag that runs:

```sh
sed -i 's/^BOOKCLUB_TAG=.*/BOOKCLUB_TAG=0.11.0/' .env
systemctl start bookclub-update
docker compose images app
```

Later releases leave a pinned app alone. `BOOKCLUB_TAG=stable` follows the releases again, from the next run on.

A pinned version is still pulled every 3 minutes, so an image pushed again under its tag replaces the app (see "Who can deploy" above). To freeze the app, for example while a leak of the registry token is being looked into, pin the digest of the image it runs:

```sh
docker image inspect -f '{{index .RepoDigests 0}}' "$(docker inspect -f '{{.Image}}' "$(docker compose ps -q app)")"
# git.tillhome.de/rafiistcool/bookclub-server@sha256:…
sed -i 's/^BOOKCLUB_TAG=.*/BOOKCLUB_TAG=0.11.0@sha256:…/' .env
```

Compose then pulls that image by its digest and ignores the tag. With a local registry, a `0.11.0` pushed again with other content left an app pinned this way on its old image, and `update.sh` handles the form unchanged. Or stop the timer (`systemctl disable --now bookclub-update.timer`); its heartbeat check then alerts after its grace time.

A version cannot run on a database that a newer version has migrated: it stops at startup with `cannot apply the database migrations: migration <N> was previously applied but is missing in the resolved migrations`, and `update.sh` reports a failed update. A rollback across a migration therefore needs the backup from before the update: stop the timer and wait for a running update (see above), and restore the snapshot that `update.sh` took right before the newer version started (see "Restoring for real"; `backup.sh restic snapshots` lists them with their times). `backup.sh` keeps every snapshot of the last 14 days, so that one is still there after later backups the same day, the rollback's own among them. Whatever was written since then is lost.

## Releases

A release is a version tag pushed to Forgejo, from a commit on `main`:

```sh
git tag v0.11.0
git push origin v0.11.0
```

`.forgejo/workflows/release.yml` runs the frontend and server checks, builds the root `Dockerfile` for amd64 and pushes `git.tillhome.de/rafiistcool/bookclub-server` with these tags:

| Tag | From `v1.2.3` | From `v1.3.0-rc.1` |
|---|---|---|
| exact version | `1.2.3`, never moves | `1.3.0-rc.1`, never moves |
| minor | `1.2`, if `v1.2.3` is the highest `v1.2.*` release | none |
| `stable` | if `v1.2.3` is the highest release of all | none |

The exact version is pushed first. Then `release-tags.sh` compares the tag with the release tags (`vX.Y.Z`, no pre-releases) that the repository has at that moment, and points the floating tags at the new image only if it is the highest. A backport such as `v0.10.5` after `v0.11.0` therefore moves `0.10` but not `stable`, and of two release runs that overlap, the older version never moves `stable` back, whichever run ends last. If the newest release's run fails, `stable` stays where it was until that version is released again.

Once `stable` has moved, the VM's update timer picks the release up within about 3 minutes (see "Updates and rollback"). A pre-release never reaches a stack that follows `stable`; to try one on staging, pin its exact version there (`BOOKCLUB_TAG=0.11.0-rc.1`).

There is no `latest` and no floating major. A tag that is not `vX.Y.Z` or `vX.Y.Z-<pre-release>` fails the run. The publish job runs on the Forgejo runner with the label `docker-host` (`docs/phase2/notes/dep.md`, "The runner") and logs in with the action secret `REGISTRY_TOKEN`, a Forgejo token with `write:package`. It is a secret of the user `rafiistcool`, which the repository's own secret list does not show. The old Python image `rafiistcool/bookclub` is never touched.

Every action in the Forgejo workflows is pinned to a commit, with its release in a comment, because the publish job hands its actions that token and the host's Docker daemon. A pin moves only on purpose; the comment above `jobs:` in `release.yml` says how, and `server/tests/deploy_env.rs` fails on an unpinned `uses:`.

## Staging

Staging runs the same files on the same VM, on a temporary hostname and with test data only. It needs its own `PUBLIC_URL` and the SMTP account (see Requirements). Wipe it before the import:

```sh
docker compose down -v
find /srv/bookclub/media -mindepth 1 -delete
```

To run a second stack on the same Docker host, from another directory, give its `.env` its own `COMPOSE_PROJECT_NAME`, `BOOKCLUB_PORT` and `BOOKCLUB_DATA_DIR`. Compose takes the project name from `.env` too, so the commands in this file work unchanged in that directory. Its backups and updates need their own copy of the scripts with a `backup.env` for that directory, and copies of the units under other names that start them there.

## The VM's network

The app VM sits in a VLAN of its own, and the Caddy that serves git.tillhome.de forwards to it (Q7, option A). Caddy's connections reach the VM without source NAT, so the app sees Caddy's own address as the peer of every request, and the client's address in Caddy's `X-Forwarded-For`. The real addresses go in once the VM exists (work plan §5, item 10); until then `TRUSTED_PROXIES=CHANGE_ME` keeps the app from starting.

- **`TRUSTED_PROXIES=<Caddy IP>/32`** in `.env`: Caddy's address as the app VM sees it, and nothing wider. The app reads `X-Forwarded-For` only from these addresses and counts its rate limits per client from it. With a wrong value, the app takes Caddy for the client, and every user lands in one rate-limit bucket: one client's failed attempts lock everyone out of sign-up and password reset for an hour, and out of login for 10 minutes. A wider value, such as the whole VLAN, lets any machine in it forge `X-Forwarded-For` and slip past the limits.
- **Inbound:** only Caddy may reach the published port (`BOOKCLUB_PORT`, 8080), plus SSH from the admin host. Put the rule into the Proxmox firewall of the VM, or the VLAN's router, not into nftables or ufw on the VM: Docker routes the packets of a published port before the host's `INPUT` chain, so such a rule never sees them. On the VM itself only a rule in Docker's `DOCKER-USER` chain would work.
- **Outbound to git.tillhome.de:** the VM pulls the image from Forgejo's registry behind the same Caddy. It does not go out to the public address and back in (hairpin NAT), but takes the LAN path:
  - split DNS: on the VM, `git.tillhome.de` resolves to Caddy's LAN address, from the VLAN's DNS server or from one line in `/etc/hosts` (`<Caddy IP> git.tillhome.de`);
  - one pinhole in the VLAN's firewall: from the VM to Caddy, TCP 443.

  The name stays the same, and so does Caddy's certificate for it.
- **Every other private (RFC 1918) address** stays closed to the VM (work plan §5, item 10), with one exception: the VLAN's DNS resolver and time server, usually its gateway, on UDP and TCP 53 and UDP 123. Split DNS by the VLAN's resolver needs that exception. Without it, the VM uses public resolvers and time servers, and `/etc/hosts` for git.tillhome.de.

Checks, once the VM and the rules exist:

1. On the VM, the registry by the LAN path. `getent hosts git.tillhome.de` shows Caddy's LAN address, and the `curl` prints that address and `401` (the registry wants a login). Then log in with the read-only token and pull:

   ```sh
   getent hosts git.tillhome.de
   curl -sS -o /dev/null -w '%{remote_ip} %{http_code}\n' https://git.tillhome.de/v2/
   docker login git.tillhome.de
   docker compose pull app
   ```

2. From another machine in the LAN, `curl -m 5 http://<VM address>:8080/api/health` gets no answer; from Caddy's host it gets `{"status":"ok"}`.

3. The address the app sees, while Caddy forwards to it (its health check every 15 s is enough). The `Peer Address:Port` column must hold Caddy's address and nothing else; that address with `/32` is `TRUSTED_PROXIES`:

   ```sh
   nsenter -n -t "$(docker inspect -f '{{.State.Pid}}' "$(docker compose ps -q app)")" \
       ss -tn state established '( sport = :8080 )'
   ```

4. `verify-limits.sh` and one sign-up from a second network ("Reverse proxy", step 5).

## Reverse proxy

TLS ends at the Caddy that already serves git.tillhome.de, which forwards to `BOOKCLUB_LISTEN:BOOKCLUB_PORT`. `Caddyfile.example` next to this file is its site block.

1. Copy the block into that Caddyfile, or `import` the file. Set two variables in Caddy's environment:
   - `BOOKCLUB_HOST`: the public host name.
   - `BOOKCLUB_UPSTREAM`: the app VM's address and published port, e.g. `10.0.20.5:8080`. Without it the block proxies to `127.0.0.1:8080`.
2. In the app's `.env`, set `TRUSTED_PROXIES` to this Caddy's address as the app VM sees it, with `/32` (see "The VM's network", check 3), and run `docker compose up -d --wait app`. If it is wrong, every user lands in Caddy's one rate-limit bucket, and one client can lock everyone out of sign-up and reset.
3. Install the logrotate rule from the comment above the block's `log` as `/etc/logrotate.d/bookclub-access` (root-owned, mode 0644; logrotate ignores a group- or world-writable file), on the host that holds `/var/log/caddy`. If Caddy runs in a container, bind-mount that directory from the host. Check it with `logrotate -d /etc/logrotate.d/bookclub-access`. If logrotate refuses the directory's permissions, add `su <caddy user> <caddy group>` to the rule.
4. Check and load: `caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile`, then `caddy reload`.
5. From home, run `verify-limits.sh` (next section), then make one sign-up attempt from a second network, such as a phone on mobile data. That attempt must not get 429.
6. Open `https://<host>/login?next=/join/TEST` and `https://<host>/join/TEST` in a browser, then check that `grep -c TEST /var/log/caddy/bookclub-access.log` prints 0.

What the block does:

- `request_body max_size 51MiB` (53,477,376 bytes). The app answers 413 itself, from `Content-Length` and before reading the body, above 52,432,896 bytes, so a client always gets the app's JSON 413. Caddy's limit is the backstop for bodies it has to read, such as chunked ones. Caddy's `50MB` (50,000,000 bytes) would cut off uploads the app accepts.
- HSTS `max-age=31536000` without `includeSubDomains` (add it, and consider `preload`, once the final domain is in place), a `Permissions-Policy` that allows the camera on the app's own origin only, for barcode scanning (`camera=(self)`, #56; `camera=()` makes every camera request fail without a prompt), and denies microphone, location, payment and motion sensors (clipboard and Web Share keep their defaults, because the invite pages use them), and no `Server` header.
- It does not repeat the headers the app sets: CSP, `X-Frame-Options`, `X-Content-Type-Options`, `Referrer-Policy`, `Cache-Control`.
- zstd and gzip for `/assets/*` only. API answers mix personal data with text a client sent, the pattern compression side channels need.
- `reverse_proxy` with an active health check on `/api/health` every 15 s. While the app is down or restarting, Caddy answers 503 for up to one interval after the app is back.
- An access log in `/var/log/caddy/bookclub-access.log`, kept at most 7 days. It holds client addresses, which the privacy text must state.
  - Caddy 2.10 rolls a log file only by size (it has no `roll_interval`), and `roll_keep_for` counts from the roll, not from the oldest line. At beta traffic a 50 MiB file would hold lines for weeks. So the block sets `roll_disabled`, and logrotate on the host rolls the file every day, empty or not (`daily`, `rotate 5`, `maxage 7`, `copytruncate`). The live file holds at most a day, the 5 old files the 5 days before, so no line is older than 6 days plus the timer's delay.
  - `copytruncate`, because Caddy keeps the file open. Caddy opens it in append mode, so after the truncate it writes at the start of the empty file. Lines written between the copy and the truncate can be lost.
  - The URI keeps only the path, as in the app's own log, which logs the path without the query. A query becomes `?{redacted}`: search terms, `/signup?code=…`, and `/login?next=/join/<code>`, which the app sends a signed-out reader to from an invite. Invite codes in the path become `/join/{redacted}` and `/api/invites/{redacted}/…`, and an ISBN lookup (a scan, #56) `/api/books/isbn/{redacted}`: an ISBN says what someone reads, like a search term. The app's own log writes it as `/api/books/isbn/{isbn}`.
  - `Referer`, which repeats such URLs, is dropped. Caddy leaves out `Cookie` and `Authorization` by default.
- A commented `rate_limit` block for Q16 option B. It needs a Caddy built with `github.com/mholt/caddy-ratelimit` (xcaddy) and `order rate_limit before basicauth` in the global options.
- Caddy replaces any `X-Forwarded-For` an untrusted client sends with the client's address. The app reads it only from `TRUSTED_PROXIES` and takes the right-most untrusted address.
- For a local test, uncomment `tls internal`, or put `local_certs` in the global options.

The cutover adds a temporary gate and later a redirect for the old host (`docs/phase2/cutover.md` §3.5–3.6, `deploy/legacy-origin/Caddyfile.example`). The gate's `respond` runs before this block's bare `reverse_proxy`; if `reverse_proxy` ever moves into a `handle`, the gate must become `handle @cutover_closed { respond … }`.

## Checking the limits (`verify-limits.sh`)

From a checkout, on a machine outside the VM's network:

```sh
deploy/hosted/verify-limits.sh https://<host>
# with `tls internal`:
VERIFY_LIMITS_CURL_FLAGS=--insecure deploy/hosted/verify-limits.sh https://<host>
```

- It needs bash (3.2 or later, so macOS's `/bin/bash` will do) and curl, and creates nothing:
  - the sign-ups carry an invite code that does not exist, so each answers 400 and rolls back;
  - the logins use made-up names;
  - the reset asks for an `example.invalid` address that no account holds.
- It first checks `GET /api/health` for 200 `"ok"`, then:
  1. sign-up: 5 × 400, then 429;
  2. login with one unknown name: 10 × 401, then 429 (the per-name limit);
  3. login per address: attempts 12 to 30, with fresh names, 401; the 31st 429;
  4. reset: 5 × 204, then 429.
- Each 429 must carry `Retry-After` and the app's detail `Too many attempts. Wait a few minutes.`
- Every request also sends a different forged `X-Forwarded-For`. If the proxy passed it on and the app trusted it, no 429 would come, and the script says so.
- Exit 0 when every check passes, 1 when one fails, 2 on a usage error.
- **It uses up the caller's budgets:** sign-up and reset for an hour, login for 10 minutes. Run it once, and do not run it from an address that people need to sign in from right away. A second run within the hour fails at the first check with a hint that the address is already limited. So does a sign-in from the same address in the 10 minutes before: the per-address login check then fails at attempt 30 with that hint.
- Search is limited per account inside the app (`catalog_user`, 60 a minute), not per address, and is not checked.

## The legacy import

The cutover (#52) imports the Python app's SQLite database once, into an empty database: no `beta-invite` and no sign-up before it, because the importer refuses a database with users (exit 2) or invites (exit 1). `docs/phase2/cutover.md` is the cutover runbook; these are its import commands.

1. The copy of the legacy database, `bookclub.db` and its `bookclub.db-wal` if there is one, goes into a directory only uid 1000 (the app's user) and root can read:

   ```sh
   install -d -o 1000 -g 1000 -m 0700 /srv/bookclub/import
   cp bookclub.db* /srv/bookclub/import/
   chown 1000:1000 /srv/bookclub/import/bookclub.db*
   chmod 0600 /srv/bookclub/import/bookclub.db*
   ```

2. Stop the update timer, wait for an update that is already running (see "Updates and rollback"), and stop the app, so that nobody signs up and no update starts while the import runs. The importer runs in a one-off container of the same image, which `docker compose run` starts (with Postgres, if it is not running). `-T` keeps the report free of terminal line endings. First a dry run, which rolls everything back:

   ```sh
   systemctl stop bookclub-update.timer
   flock /var/lib/bookclub-backup/.update.lock true
   docker compose stop app
   docker compose run --rm -T -v /srv/bookclub/import:/import:ro app \
       bookclub-server import-legacy /import/bookclub.db --owner <username> \
       --club-name '<club name>' --timezone Europe/Berlin --club-slug <slug> \
       --allow-unmatched --dry-run > dry-run.txt
   ```

   `--owner` is the legacy username that becomes the club owner, `--club-name` and `--timezone` are `BOOKCLUB_NAME` and `BOOKCLUB_TZ` of the old `.env`, and `--club-slug` is the slug the rehearsal used, so the club's URL never changes. The import also searches Open Library for covers of the imported books that have none (at most 5 minutes a run; an outage stops the search, never the import). The report's `cover search` line counts them, and its `without a cover` section names each book still without one with its title, so the report stays as private as the rest of it. Read `dry-run.txt`, then run the same command without `--dry-run` and start the app:

   ```sh
   docker compose run --rm -T -v /srv/bookclub/import:/import:ro app \
       bookclub-server import-legacy /import/bookclub.db --owner <username> \
       --club-name '<club name>' --timezone Europe/Berlin --club-slug <slug> \
       --allow-unmatched > import.txt
   docker compose up -d --wait
   systemctl start bookclub-update.timer
   ```

3. `--allow-unmatched` keeps going when the DNB or Open Library cannot answer, and keeps those ISBN rows as bare facts. Resolve them later, with the app running. `rematch-legacy` then searches covers again for the imported books still without one, 2 seconds apart (the books an outage or the time budget left "not searched" too), and prints the same cover line and section; decide its suggestions on the admin page's Covers tab:

   ```sh
   docker compose run --rm -T app bookclub-server rematch-legacy --dry-run
   docker compose run --rm -T app bookclub-server rematch-legacy
   ```

4. Turn on the DNB harvester, which stays off until the import is done: `DNB_HARVEST=true` in `.env`, then `docker compose up -d --wait app`.

The importer reads a private copy of the database and its `-wal` in the container's `$TMPDIR`, which is `/tmp`, a 64 MB tmpfs (`compose.yaml`). Compare `docker compose run --rm -T app df -h /tmp` with the size of the files. For a bigger database, give the copy a directory owned by uid 1000, as in step 1, and add `-e TMPDIR=/import-tmp -v /srv/bookclub/import-tmp:/import-tmp` to both `docker compose run` commands.

## Backups

`backup.sh` runs every night at 03:30 (plus up to 30 minutes), from `bookclub-backup.timer`:

1. `pg_dump -Fc --no-owner` inside the postgres container, and in the same database snapshot the row count of every table and the newest migration (`counts.tsv`), into `/var/lib/bookclub-backup/db`. A dump and its counts therefore always agree.
2. `restic backup` of that directory and of `/srv/bookclub/media`. The dump comes first: a picture is on disk before its row commits, so the dump never references a picture saved after restic read the media. A picture deleted in between (a replaced profile picture, a takedown, a deleted account) can be missing from the snapshot while the dump still references it. restic then saves the snapshot without it and names it as a file it could not read. `backup.sh` logs that as a warning and goes on when every such file is a picture that no longer exists, and fails otherwise.
3. `restic forget --prune`, keeping every snapshot of the last 14 days, and beyond them 7 daily, 4 weekly and 12 monthly snapshots. The 14 days keep the backup that `update.sh` takes before an update, which a later backup the same day would otherwise prune: `--keep-daily` keeps only the newest snapshot of each day.
4. On Sundays, `restic check --read-data-subset=5%`, which reads back 5 % of the stored data.
5. A heartbeat ping when it starts, and one with its exit status and the end of its log when it ends. The log can name a picture by its path (step 2), and a picture's public id is the secret part of its URL, so the ping carries the last 50 lines with every run of 26 or more lowercase letters and digits replaced by `<id>` (`log_tail` in `backup-lib.sh`); restic's ids are blanked with them. The lines Postgres adds to an error (`DETAIL`, `CONTEXT`, `HINT`, `LINE n`, `QUERY`, `STATEMENT`, `Command was`) quote data, so the ping has `<withheld>` in their place. The full log stays in `/var/lib/bookclub-backup`.

`restore-test.sh` runs every Sunday at 05:30 (see "Restore"). Both scripts read `backup.env`, share one lock, and log to the journal. restic runs from the image `restic/restic:0.17.3`, so nothing needs installing besides `curl`.

### Setting up

1. The repository. For a Hetzner Storage Box (Q9), give root an SSH key and put the box into root's SSH config, with port 23:

   ```sh
   install -d -m 0700 /root/.ssh
   ssh-keygen -t ed25519 -N '' -C bookclub-backup -f /root/.ssh/id_ed25519_backup
   cat >> /root/.ssh/config <<'EOF'
   Host bookclub-backup
       HostName uNNNNNN.your-storagebox.de
       User uNNNNNN
       Port 23
       IdentityFile /root/.ssh/id_ed25519_backup
   EOF
   ssh-keyscan -p 23 uNNNNNN.your-storagebox.de >> /root/.ssh/known_hosts
   ```

   Upload the public key to the box as Hetzner's Storage Box documentation describes, and compare the scanned host key with the fingerprints Hetzner publishes. In the box's settings in the Hetzner Console, turn on SSH support (port 23) and "External Reachability": the VM is outside Hetzner's network. `RESTIC_REPOSITORY` is then `sftp:bookclub-backup:bookclub`. A local path or an `s3:` URL also works (`backup.env.example`). Then set up the box's snapshots (see "Storage Box snapshots").

2. The scripts, their settings and the repository password:

   ```sh
   cp deploy/hosted/backup.sh deploy/hosted/restore-test.sh deploy/hosted/backup-lib.sh /opt/bookclub/
   install -m 0600 deploy/hosted/backup.env.example /opt/bookclub/backup.env
   (umask 077 && openssl rand -base64 32 > /opt/bookclub/restic-password)
   ```

   Edit `backup.env`: `RESTIC_REPOSITORY`, and the two heartbeat URLs. **Put the content of `restic-password` into the password manager**: without it no backup can be read, and it is on the VM only.

3. The heartbeats: two checks at healthchecks.io (or any service with the same URLs), "backup" with a period of 1 day and a grace time of 2 hours, and "restore test" with a period of 7 days and a grace time of 8 hours. Their ping URLs go into `BACKUP_HEARTBEAT_URL` and `RESTORE_TEST_HEARTBEAT_URL`. A check alerts when a run fails and when one does not happen at all.

4. Create the repository, take the first backup, and test it:

   ```sh
   /opt/bookclub/backup.sh restic init
   /opt/bookclub/backup.sh
   /opt/bookclub/backup.sh restic snapshots
   /opt/bookclub/restore-test.sh
   ```

5. The timers:

   ```sh
   cp deploy/hosted/systemd/bookclub-* /etc/systemd/system/
   systemctl daemon-reload
   systemctl enable --now bookclub-backup.timer bookclub-restore-test.timer
   systemctl list-timers 'bookclub-*'
   ```

### By hand

- A backup now, for example before an update: `/opt/bookclub/backup.sh`, or `systemctl start bookclub-backup` to run it as the timer does. `/opt/bookclub/backup.sh --check` also reads back 5 % of the data.
- The last runs: `journalctl -u bookclub-backup -u bookclub-restore-test`, or `/var/lib/bookclub-backup/backup.log`.
- Any restic command with this setup: `/opt/bookclub/backup.sh restic <command>`, for example `snapshots`, `stats` or `check`. restic can write only into `/var/lib/bookclub-backup`, so a restore goes there.

### Storage Box snapshots

The VM's SSH key can write to the restic repository, so it can also delete it: whoever takes over the VM could delete the backups along with the data. The Storage Box's own snapshots narrow that gap (`docs/phase1/security-review.md`, Phase 2 Wave 1, finding 3, option A). Hetzner takes them on a schedule; the box's users, the VM's SFTP login among them, can read them but cannot write to or delete them; only the Hetzner Console and the Hetzner API can delete a snapshot or roll the box back to one. They do not close it on their own: the snapshots rotate, and only a check from outside the VM notices a damaged repository in time (see "How far back" and "A check from outside the VM").

1. In the Hetzner Console, from the admin machine: "Storage Boxes", the box, "Snapshots", "Enable automatic snapshots".
   - Daily, at a time after the nightly backup. That starts at 03:30 on the VM's clock, plus up to 30 minutes, and takes minutes; 06:00 leaves room. Check which time zone the console uses.
   - The maximum number of snapshot slots: all that the plan allows (BX11: 10, BX21: 20, BX31: 30, BX41: 40).
   - Save.
2. Leave "Display snapshot directory" on, so that a restore can read the snapshots in `.zfs/snapshot` (see "Restoring from a Storage Box snapshot").
3. The next day, check that the snapshot is listed in the console, and on the VM:

   ```sh
   ssh bookclub-backup ls .zfs/snapshot
   ```

- **How far back:** one snapshot a day, so 10 slots (BX11) reach back 10 days: a deleted or damaged repository has to be noticed within that time. Once the slots are full, Hetzner deletes the oldest snapshot as it takes a new one. The repository's own history (every backup of the last 14 days, then 7 daily, 4 weekly and 12 monthly ones) is unchanged; the snapshots protect it.
- **Who notices:** everything on the VM that would notice a damaged repository (the nightly backup, the Sunday `restic check` and restore test, their heartbeats) is in the hands of whoever controls the VM. Root can replace the scripts and keep pinging the checks with the URLs in `backup.env`, or give the repository a password of their own (`restic key add`, then `key remove` of the old one), so that the backups and their checks go on while the password in the password manager no longer opens anything. Once the slots have turned over (10 days on a BX11), every snapshot holds the damaged repository, and dropping the database then leaves nothing to restore. Only a check the VM cannot fake catches that.
- **Space:** snapshots cost nothing extra but count against the box's capacity. A new snapshot takes no space; it grows by what is changed or deleted on the box after it. `backup.sh` only adds files to the repository, and its `forget --prune` deletes the files that no backup needs any more: mostly the old database dumps, and pictures deleted in the app. Those stay in the snapshots until the last snapshot that holds them is deleted. So the snapshots take about 10 days' worth of dumps (each `backup.sh` run logs the dump's size) plus the pictures deleted in that time. The console shows the box's usage, snapshots included. A full box fails `backup.sh`, and its heartbeat alerts; fewer slots free space.
- **Credentials:** never put the Hetzner Console login, a Hetzner API token or the box's password on the VM, or anywhere the VM can read them. Whoever holds the login or a token can delete the snapshots and the box. The VM needs only its SSH key for the box, which cannot touch the snapshots.

### A check from outside the VM

**Not set up yet** (work plan §5, item 13): it needs a machine outside the VM that runs every day. Until it runs, the snapshots protect against a repository deleted in one go, which the next backup's heartbeat reports, but not against an attacker who stays on the VM until the slots have turned over.

- In the Hetzner Console, a sub-account of the box that can only read, with SSH support, whose directory is the repository's. Its password or key goes only onto that machine, never onto the VM.
- On that machine, once a day, with restic and the restic password from the password manager: `restic --no-lock snapshots --latest 1 --json` and `restic --no-lock cat config` against the repository through the sub-account (`--no-lock`, because the sub-account cannot write).
- A heartbeat check of its own, which alerts when the newest snapshot is older than 2 days, when the repository's `id` (from `cat config`) is not the one written down at setup, or when the password does not open the repository.

## Restore

### The restore test

`restore-test.sh` restores the newest snapshot into `/var/lib/bookclub-backup/restore-test` and checks it without changing the running stack:

1. restic verifies every restored file against the repository.
2. A throwaway `postgres:17` without a network runs `pg_restore --exit-on-error` on the dump. When it fails, Postgres's messages quote the rows they fail on (a duplicated address, a failing row with its password hash), so they go to `/var/lib/bookclub-backup/pg_restore.err`, readable by root only, and the log (and with it the ping) gets only pg_restore's own `pg_restore: error:` lines, which name the table, index or constraint, with any quoted value withheld. Read the file on the VM to see why, and delete it once the cause is fixed; a passing run deletes it.
3. The row count of every table and the newest migration must equal the backup's `counts.tsv`.
4. Every stored picture the restored database references (`media.storage_key` and its variants, except taken-down rows) must be in the restored media, and not empty. A missing one that the running database no longer references either was deleted while the backup ran (see "Backups"): the test logs it as `deleted after the backup` and does not fail. A missing one that the running database still references fails the test, and so does every missing one while the running database does not answer.

It removes the container and the restored files afterwards, and ends with a line such as `restore test passed: 31 tables, 1830 rows, migration 1900, 52 media files`, or `… 51 media files (1 deleted after the backup)`. Any other difference fails it, and the heartbeat alerts. The test only reads from the running stack. By hand: `/opt/bookclub/restore-test.sh`, or `systemctl start bookclub-restore-test`.

### Restoring for real

When the database or the pictures are lost, on this VM or on a new one set up as in "First boot", with `.env`, `backup.env`, the SSH key and the restic password from the password manager. Stop the update timer first, and wait for an update that is already running (see "Updates and rollback"), so that no update backs up a half-restored database or starts the app in between; step 5 starts the timer again:

```sh
systemctl stop bookclub-update.timer
flock /var/lib/bookclub-backup/.update.lock true
```

1. Restore the snapshot into the backup directory. `latest` is the newest; `snapshots` lists the others by ID:

   ```sh
   /opt/bookclub/backup.sh restic snapshots
   /opt/bookclub/backup.sh restic restore latest --host bookclub --tag bookclub --target /var/lib/bookclub-backup/restore
   ```

2. Replace the database. `down -v` deletes the database volume, so check step 1's output first:

   ```sh
   cd /opt/bookclub
   docker compose down -v
   docker compose up -d --wait postgres
   docker compose exec -T postgres pg_restore --exit-on-error --no-owner -U bookclub -d bookclub \
       < /var/lib/bookclub-backup/restore/var/lib/bookclub-backup/db/bookclub.dump
   ```

3. Replace the pictures:

   ```sh
   find /srv/bookclub/media -mindepth 1 -delete
   cp -a /var/lib/bookclub-backup/restore/srv/bookclub/media/. /srv/bookclub/media/
   chown -R 1000:1000 /srv/bookclub/media
   ```

4. Before the app starts, back up the restored state and compare its counts with the snapshot's. `diff` prints nothing when every table and the migration match:

   ```sh
   /opt/bookclub/backup.sh
   diff /var/lib/bookclub-backup/restore/var/lib/bookclub-backup/db/counts.tsv /var/lib/bookclub-backup/db/counts.tsv
   ```

   `backup.sh` warns that there is no app container to compare `MEDIA_DIR` with; that is expected here.

5. Start the app, with the version that made the backup or a newer one (`BOOKCLUB_TAG`; the last line of `counts.tsv` names the newest migration), remove the restored files, and start the update timer again:

   ```sh
   docker compose up -d --wait
   rm -rf /var/lib/bookclub-backup/restore
   systemctl start bookclub-update.timer
   ```

### Restoring from a Storage Box snapshot

When the repository itself is gone or damaged, for example because someone deleted it from the VM. Treat that VM as compromised. Whoever deleted the repository could read everything on it: `.env` with the SMTP credentials, the package token of `docker login`, `backup.env`, the restic password and the box's SSH key. With that key they may also have added a key of their own to the box's `.ssh/authorized_keys`. The old restic password and everything in the old repository therefore count as known to them. So does the database: every address and password hash, everything people wrote, and the invite codes, which are stored as they are. While they were in, they could also write it (`docker compose exec postgres psql`): plant a session for a token they hold, make an account an instance admin, join a club, change an address, lift a suspension or undo a takedown. The snapshot below is from before the damage, not from before the intrusion, so steps 6 to 8 end that access before the app starts. Restore onto a new VM, set up as in "First boot", steps 1 to 5, with new SMTP credentials and a new read-only package token (revoke the old ones), and with the scripts (`backup.sh`, `restore-test.sh`, `backup-lib.sh`, `update.sh`) and `backup.env` as in "Backups", "Setting up", step 2. Give `backup.env` new heartbeat checks too: the old ping URLs were in the old `backup.env`, and a ping URL is its check's only credential, so whoever has them can keep the old checks green from anywhere while a backup, a restore test or an update on the new VM stops. Create the three checks anew ("Backups", "Setting up", step 3, and "Updates and rollback", "Setting up", step 2), put only their URLs into the new `backup.env`, and delete the old checks. The steps below make the box's key, the restic password and the repository; take the box back before anything else:

1. Take the box back. In the Hetzner Console, reset the box's password. On the new VM, create a new key, root's SSH config and `known_hosts` with the commands of "Backups", "Setting up", step 1. Then, instead of the usual upload, replace the box's key file as a whole with one that holds only the new key (the first `scp` asks for the new password; if the box has no `.ssh` any more, `ssh bookclub-backup mkdir .ssh` first), and read it back. `diff` prints nothing when the new key is the only one:

   ```sh
   scp /root/.ssh/id_ed25519_backup.pub bookclub-backup:.ssh/authorized_keys
   scp bookclub-backup:.ssh/authorized_keys /root/box-keys
   diff /root/.ssh/id_ed25519_backup.pub /root/box-keys && rm /root/box-keys
   ```

2. Find a snapshot from before the damage, in the console or with `ssh bookclub-backup ls .zfs/snapshot`.

3. Read the repository inside that snapshot with the old password, without changing anything on the box. Put the old restic password from the password manager into `/opt/bookclub/restic-password-old` (mode 0600). A copy of `backup.env` points restic at the snapshot and that password, and `--no-lock` keeps restic from writing its lock file into the read-only snapshot. `<snapshot>` is the name `ls` printed:

   ```sh
   (umask 077 && sed -e 's#^RESTIC_REPOSITORY=.*#RESTIC_REPOSITORY=sftp:bookclub-backup:.zfs/snapshot/<snapshot>/bookclub#' \
       -e 's#^RESTIC_PASSWORD_FILE=.*#RESTIC_PASSWORD_FILE=/opt/bookclub/restic-password-old#' \
       /opt/bookclub/backup.env > /opt/bookclub/backup-snapshot.env)
   BACKUP_ENV=/opt/bookclub/backup-snapshot.env /opt/bookclub/backup.sh restic --no-lock snapshots
   BACKUP_ENV=/opt/bookclub/backup-snapshot.env /opt/bookclub/backup.sh restic --no-lock \
       restore latest --host bookclub --tag bookclub --target /var/lib/bookclub-backup/restore
   ```

4. Start a new repository, with a new password, in a new directory of the box, before anything backs up:

   ```sh
   (umask 077 && openssl rand -base64 32 > /opt/bookclub/restic-password)
   sed -i 's#^RESTIC_REPOSITORY=.*#RESTIC_REPOSITORY=sftp:bookclub-backup:bookclub-<date>#' /opt/bookclub/backup.env
   /opt/bookclub/backup.sh restic init
   ```

   **Put the new password into the password manager**, next to the old one: the old one still reads the old repository in the box's snapshots, until the last of them is gone. Rolling the box back to the snapshot instead would bring back the old repository with the old password, and the `.ssh/authorized_keys` of that day, perhaps with the attacker's key in it.

5. Go on with steps 2 to 4 of "Restoring for real": step 3 left the restore where they expect it, and step 4's `backup.sh` takes the first backup into the new repository. Do not start the app yet.

6. End what the attacker can reuse from the database: every session and app token (a session is the SHA-256 of its token, with no secret of the server's, so a planted one works on the new VM too), every open club and sign-up invite, and every email link not used yet. `revoke-access.sql`, from the checkout the new VM was set up from, does it in one transaction and prints how many rows each statement changed. Everyone signs in again with their password; club admins create new invites:

   ```sh
   cd /opt/bookclub
   docker compose exec -T postgres psql -X -v ON_ERROR_STOP=1 -U bookclub -d bookclub \
       < <checkout>/deploy/hosted/revoke-access.sql
   ```

7. Check what they could have changed: the instance admins, the accounts created and the addresses verified or passwords changed since the intrusion may have begun, the club memberships since then, and the moderation decisions. `<date>` is that day; when in doubt, the day of the oldest snapshot of the old box:

   ```sh
   docker compose exec -T postgres psql -X -U bookclub -d bookclub -v since="'<date>'" <<'SQL'
   SELECT username, email FROM users WHERE is_admin ORDER BY username;
   SELECT username, email, created_at, email_verified_at, password_changed_at FROM users
    WHERE greatest(created_at, email_verified_at, password_changed_at) >= :since ORDER BY 3;
   SELECT c.name AS club, u.username, m.role, m.joined_at FROM club_memberships m
     JOIN clubs c ON c.id = m.club_id JOIN users u ON u.id = m.user_id
    WHERE m.joined_at >= :since ORDER BY m.joined_at;
   SELECT username, suspended_at, unsuspended_at FROM users
    WHERE suspended_at IS NOT NULL OR unsuspended_at IS NOT NULL ORDER BY username;
   SELECT public_id, kind, removed_at FROM media WHERE removed_at IS NOT NULL ORDER BY removed_at;
   SQL
   ```

   A direct write leaves no trace in these columns: `UPDATE users SET suspended_at = NULL` lifts a suspension without setting `unsuspended_at` (only the admin page and `bookclub-server unsuspend` do), and a cleared `removed_at` brings a picture back. So compare the suspensions and takedowns with the record in the contact mailbox (`docs/legal/takedown.md` §6), which is off the VM and out of the attacker's reach, not with the database's own timestamps. The same holds for the timestamps of the other queries: a direct write can change an address or a password hash without them, so ask the owners of the accounts that matter (the admins, the owners of clubs) to check their address once the app runs.

   Anything nobody can explain goes before the app starts: an admin flag with `UPDATE users SET is_admin = false WHERE username = '<name>'`; an address its owner did not set with `UPDATE users SET email = NULL, email_verified_at = NULL WHERE username = '<name>'`, so that no reset link goes there (tell the owner, who sets it again); every account the record lists as suspended that is not, with `UPDATE users SET suspended_at = now(), is_admin = false WHERE username = '<name>'` (step 6 already ended its sessions and the invites); a picture the record lists as taken down whose `removed_at` is empty again, with `UPDATE media SET removed_at = now(), storage_key = NULL WHERE public_id = '<id>'` and `INSERT INTO media_removed_hashes (sha256, media_id) SELECT sha256, id FROM media WHERE public_id = '<id>' AND sha256 IS NOT NULL ON CONFLICT DO NOTHING` (the hash of the file as received, so the same file cannot be uploaded again; delete its files under `/srv/bookclub/media` by hand); an account or a membership through the club's owner or, once the app runs, the admin page.

8. Back up again, so that the newest snapshot holds the database as it is now, then start the app as in step 5 of "Restoring for real", and the timers, which this VM does not have yet. Their first runs ping the new checks:

   ```sh
   /opt/bookclub/backup.sh
   docker compose up -d --wait
   rm -rf /var/lib/bookclub-backup/restore
   cp <checkout>/deploy/hosted/systemd/bookclub-* /etc/systemd/system/
   systemctl daemon-reload
   systemctl enable --now bookclub-backup.timer bookclub-restore-test.timer bookclub-update.timer
   ```

9. Remove the copies: `rm /opt/bookclub/backup-snapshot.env /opt/bookclub/restic-password-old`.

10. Tell people. The addresses and password hashes of every account, and everything people wrote, left the VM: a personal data breach. Unless it is unlikely to put anyone at risk, report it to the data protection authority within 72 hours of noticing it (Art. 33 GDPR), and when the risk to people is high, tell them too (Art. 34): what happened, and to choose a new password, here and wherever they used the same one. Write down what happened, what it affected and what was done, whether or not it is reported (Art. 33 (5)).

Without a compromise, when a mistake or a bug on a VM you trust damaged the repository, roll the box back instead: in the console, "Snapshots", the snapshot's menu. A rollback undoes every change on the box since the snapshot: the backups taken after it, and the newer snapshots, are gone. The repository keeps its password and its history; restore from it as in "Restoring for real".

## Legal texts

The app shows the imprint, the privacy policy and the terms of use at `/imprint`, `/privacy` and `/terms`, read from `/srv/bookclub/legal` (`LEGAL_DIR=/app/legal`). While the directory is empty, the app serves and links none, and invite mode works; open sign-up needs all six files.

1. Generate the texts (eRecht24 or a lawyer) from the facts in `docs/legal/`, and export each as an HTML fragment. Six UTF-8 files of at most 512 KiB each: `imprint.de.html`, `imprint.en.html`, `privacy.de.html`, `privacy.en.html`, `terms.de.html`, `terms.en.html`. A missing language falls back to the other. The app shows text, headings, lists, tables and links only; pictures, scripts and styles are dropped.
2. Write `manifest.json` next to them. `terms_version` (1 to 64 letters, digits, `.`, `_` and `-`) is required once a terms file exists; `updated` is optional and shown as "Last updated":

   ```json
   {
     "terms_version": "2026-11-01",
     "updated": {"imprint": "2026-11-01", "privacy": "2026-11-01", "terms": "2026-11-01"}
   }
   ```

3. Install them and restart the app, which reads them only at startup:

   ```sh
   install -o 1000 -g 1000 -m 0640 imprint.*.html privacy.*.html terms.*.html manifest.json /srv/bookclub/legal/
   docker compose restart app
   docker compose ps app
   ```

4. Open `/imprint`, `/privacy` and `/terms` signed out, in both languages.

A new `terms_version` asks every account to accept the terms again (a prompt in the app; new sign-ups accept them with the form), so change it only when the terms change. If the app does not start, `docker compose logs app` names `LEGAL_DIR` and the file: text files without `manifest.json`, an invalid manifest, a file that is empty, too large or not UTF-8, or a development placeholder.

## Open sign-up

`SIGNUP_MODE=invite` is the closed beta: sign-up needs an invite. Switch to `open` (#55) only when every gate is green (work plan §5, steps 23 to 25):

- The legal texts are in `/srv/bookclub/legal` (see "Legal texts"), and `/imprint`, `/privacy` and `/terms` show them signed out. Outside `DEV_MODE` the server refuses `SIGNUP_MODE=open` while one of the files is missing, and names the missing ones.
- `send-test-mail` to Gmail, Outlook and GMX arrives, and mail-tester shows SPF, DKIM and DMARC passing.
- `verify-limits.sh` passed from home, and a sign-up attempt from a second network (a phone on mobile data) was not limited.
- A takedown ran end to end, within the target time.
- Backups run, and the restore test has passed.

Then:

```sh
sed -i 's/^SIGNUP_MODE=.*/SIGNUP_MODE=open/' .env
docker compose up -d --wait app
curl -fsS http://127.0.0.1:8080/api/config
```

The answer holds `"signup":"open"` (the address is `BOOKCLUB_LISTEN` and `BOOKCLUB_PORT` of `.env`). If the app does not come up, `docker compose logs app` names the missing legal texts. Back to the closed beta: `SIGNUP_MODE=invite` and the same commands. Invites keep working in both modes.

## When it does not start

`docker compose logs app` shows the reason in its last line, `error: …`. Each one names the setting:

- `MEDIA_DIR /data/media is not writable: …`: `chown 1000:1000 /srv/bookclub/media`.
- `bind source path does not exist: …` (from `docker compose up`): create the directory as in "First boot".
- `TRUSTED_PROXIES entry "CHANGE_ME" is not a CIDR or an IP address`: a value in `.env` is still the placeholder.
- `SMTP_URL must use TLS without DEV_MODE: …`: use `smtps://` or `?tls=required`.
- `SIGNUP_MODE=open needs the legal texts outside DEV_MODE: …` or `LEGAL_DIR/…`: see "Legal texts".
- `cannot reach the database (gave up after 60 s)`: check `docker compose ps postgres` and `POSTGRES_PASSWORD`.
- `cannot apply the database migrations: migration <N> was previously applied but is missing in the resolved migrations`: a version older than the database; see "Rollback".
