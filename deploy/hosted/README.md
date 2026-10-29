# Bookclub hosted: operator runbook

This directory runs the hosted service on one VM: `compose.yaml` starts the app and its Postgres, and `env.example` lists every setting. `backup.sh` and `restore-test.sh` back it up with restic, from the systemd timers in `systemd/`. The commands below run as root on the VM, in `/opt/bookclub`, unless a step says otherwise.

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
  | `/opt/bookclub/backup.sh`, `restore-test.sh`, `backup-lib.sh` | the backup scripts, from `deploy/hosted/` |
  | `/opt/bookclub/backup.env`, `restic-password` | their settings and the repository password, mode 0600 |
  | `/var/lib/bookclub-backup` | the newest dump (`db/`), the restic cache, the last run's log; mode 0700 |
  | `/etc/systemd/system/bookclub-*` | the backup and restore-test units and timers, from `deploy/hosted/systemd/` |

## Requirements

- Debian 13 with Docker Engine 25 or newer and the compose plugin 2.24 or newer, and `curl` for the backup heartbeats.
- `docker login git.tillhome.de` with a read-only package token.
- **An SMTP account, for staging too.** Without `DEV_MODE` the server does not start without `SMTP_URL` and `EMAIL_FROM`, and `EMAIL_TRANSPORT=log` needs `DEV_MODE`, which this setup never sets. Until the mail provider is chosen, staging uses an interim account. `SMTP_URL` must use TLS: `smtps://…:465`, or `smtp://…:587?tls=required`.
- The reverse proxy (Caddy) terminates TLS and forwards to `BOOKCLUB_LISTEN:BOOKCLUB_PORT`. Only the proxy may reach that port.

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

   The admin mints further invites on the Invites tab of `/admin`.

The admin page `/admin` holds the catalog requests and the picture reports, each with its age, an "Overdue" mark after 72 hours, and a "Decided" view with the history and the 90-day figures. "Edit as admin", on a book page and in the queue, changes a book's facts at once. "Take down a picture by URL", above the open reports, records an emailed notice with its arrival time. Instance admins with a verified address get a mail when new reports arrive (at most one an hour) and a digest of new catalog requests (at most one a day), through `SMTP_URL`.

## Updates and rollback

`BOOKCLUB_TAG=stable` follows the highest release (see "Releases"). To update:

```sh
docker compose pull app
docker compose up -d --wait app
```

The new version applies its migrations when it starts. Migrations only go forward, so run a backup first (`/opt/bookclub/backup.sh`).

To roll back, or to hold updates back, pin an exact version in `.env`, for example `BOOKCLUB_TAG=0.11.0`, and run the same two commands. `docker compose images app` shows the tag that runs.

A version cannot run on a database that a newer version has migrated: it stops at startup with `cannot apply the database migrations: migration <N> was previously applied but is missing in the resolved migrations`. A rollback across a migration therefore needs the backup from before the update (see "Restoring for real").

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

There is no `latest` and no floating major. A tag that is not `vX.Y.Z` or `vX.Y.Z-<pre-release>` fails the run. The publish job runs on the Forgejo runner with the label `docker-host` (`docs/phase2/notes/dep.md`, "The runner") and logs in with the action secret `REGISTRY_TOKEN`, a Forgejo token with `write:package`. It is a secret of the user `rafiistcool`, which the repository's own secret list does not show. The old Python image `rafiistcool/bookclub` is never touched.

Every action in the Forgejo workflows is pinned to a commit, with its release in a comment, because the publish job hands its actions that token and the host's Docker daemon. A pin moves only on purpose; the comment above `jobs:` in `release.yml` says how, and `server/tests/deploy_env.rs` fails on an unpinned `uses:`.

## Staging

Staging runs the same files on the same VM, on a temporary hostname and with test data only. It needs its own `PUBLIC_URL` and the SMTP account (see Requirements). Wipe it before the import:

```sh
docker compose down -v
find /srv/bookclub/media -mindepth 1 -delete
```

To run a second stack on the same Docker host, from another directory, give its `.env` its own `COMPOSE_PROJECT_NAME`, `BOOKCLUB_PORT` and `BOOKCLUB_DATA_DIR`. Compose takes the project name from `.env` too, so the commands in this file work unchanged in that directory.

## Reverse proxy

TLS ends at the Caddy that already serves git.tillhome.de, which forwards to `BOOKCLUB_LISTEN:BOOKCLUB_PORT`. `Caddyfile.example` next to this file is its site block.

1. Copy the block into that Caddyfile, or `import` the file. Set two variables in Caddy's environment:
   - `BOOKCLUB_HOST`: the public host name.
   - `BOOKCLUB_UPSTREAM`: the app VM's address and published port, e.g. `10.0.20.5:8080`. Without it the block proxies to `127.0.0.1:8080`.
2. In the app's `.env`, set `TRUSTED_PROXIES` to this Caddy's address as the app VM sees it, with `/32` (Q7), and run `docker compose up -d --wait app`. If it is wrong, every user lands in Caddy's one rate-limit bucket, and one client can lock everyone out of sign-up and reset.
3. Install the logrotate rule from the comment above the block's `log` as `/etc/logrotate.d/bookclub-access` (root-owned, mode 0644; logrotate ignores a group- or world-writable file), on the host that holds `/var/log/caddy`. If Caddy runs in a container, bind-mount that directory from the host. Check it with `logrotate -d /etc/logrotate.d/bookclub-access`. If logrotate refuses the directory's permissions, add `su <caddy user> <caddy group>` to the rule.
4. Check and load: `caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile`, then `caddy reload`.
5. From home, run `verify-limits.sh` (next section), then make one sign-up attempt from a second network, such as a phone on mobile data. That attempt must not get 429.
6. Open `https://<host>/login?next=/join/TEST` and `https://<host>/join/TEST` in a browser, then check that `grep -c TEST /var/log/caddy/bookclub-access.log` prints 0.

What the block does:

- `request_body max_size 51MiB` (53,477,376 bytes). The app answers 413 itself, from `Content-Length` and before reading the body, above 52,432,896 bytes, so a client always gets the app's JSON 413. Caddy's limit is the backstop for bodies it has to read, such as chunked ones. Caddy's `50MB` (50,000,000 bytes) would cut off uploads the app accepts.
- HSTS `max-age=31536000` without `includeSubDomains` (add it, and consider `preload`, once the final domain is in place), a `Permissions-Policy` that denies camera, microphone, location, payment and motion sensors (clipboard and Web Share keep their defaults, because the invite pages use them), and no `Server` header.
- It does not repeat the headers the app sets: CSP, `X-Frame-Options`, `X-Content-Type-Options`, `Referrer-Policy`, `Cache-Control`.
- zstd and gzip for `/assets/*` only. API answers mix personal data with text a client sent, the pattern compression side channels need.
- `reverse_proxy` with an active health check on `/api/health` every 15 s. While the app is down or restarting, Caddy answers 503 for up to one interval after the app is back.
- An access log in `/var/log/caddy/bookclub-access.log`, kept at most 7 days. It holds client addresses, which the privacy text must state.
  - Caddy 2.10 rolls a log file only by size (it has no `roll_interval`), and `roll_keep_for` counts from the roll, not from the oldest line. At beta traffic a 50 MiB file would hold lines for weeks. So the block sets `roll_disabled`, and logrotate on the host rolls the file every day, empty or not (`daily`, `rotate 5`, `maxage 7`, `copytruncate`). The live file holds at most a day, the 5 old files the 5 days before, so no line is older than 6 days plus the timer's delay.
  - `copytruncate`, because Caddy keeps the file open. Caddy opens it in append mode, so after the truncate it writes at the start of the empty file. Lines written between the copy and the truncate can be lost.
  - The URI keeps only the path, as in the app's own log, which logs the path without the query. A query becomes `?{redacted}`: search terms, `/signup?code=…`, and `/login?next=/join/<code>`, which the app sends a signed-out reader to from an invite. Invite codes in the path become `/join/{redacted}` and `/api/invites/{redacted}/…`.
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

2. Stop the app, so nobody signs up while the import runs. The importer runs in a one-off container of the same image, which `docker compose run` starts (with Postgres, if it is not running). `-T` keeps the report free of terminal line endings. First a dry run, which rolls everything back:

   ```sh
   docker compose stop app
   docker compose run --rm -T -v /srv/bookclub/import:/import:ro app \
       bookclub-server import-legacy /import/bookclub.db --owner <username> \
       --club-name '<club name>' --timezone Europe/Berlin --club-slug <slug> \
       --allow-unmatched --dry-run > dry-run.txt
   ```

   `--owner` is the legacy username that becomes the club owner, `--club-name` and `--timezone` are `BOOKCLUB_NAME` and `BOOKCLUB_TZ` of the old `.env`, and `--club-slug` is the slug the rehearsal used, so the club's URL never changes. Read `dry-run.txt`, then run the same command without `--dry-run` and start the app:

   ```sh
   docker compose run --rm -T -v /srv/bookclub/import:/import:ro app \
       bookclub-server import-legacy /import/bookclub.db --owner <username> \
       --club-name '<club name>' --timezone Europe/Berlin --club-slug <slug> \
       --allow-unmatched > import.txt
   docker compose up -d --wait
   ```

3. `--allow-unmatched` keeps going when the DNB or Open Library cannot answer, and keeps those ISBN rows as bare facts. Resolve them later, with the app running:

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
3. `restic forget --prune`, keeping 7 daily, 4 weekly and 12 monthly snapshots.
4. On Sundays, `restic check --read-data-subset=5%`, which reads back 5 % of the stored data.
5. A heartbeat ping when it starts, and one with its exit status and the end of its log when it ends.

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

   Upload the public key to the box as Hetzner's Storage Box documentation describes, and compare the scanned host key with the fingerprints Hetzner publishes. `RESTIC_REPOSITORY` is then `sftp:bookclub-backup:bookclub`. A local path or an `s3:` URL also works (`backup.env.example`).

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

## Restore

### The restore test

`restore-test.sh` restores the newest snapshot into `/var/lib/bookclub-backup/restore-test` and checks it without changing the running stack:

1. restic verifies every restored file against the repository.
2. A throwaway `postgres:17` without a network runs `pg_restore --exit-on-error` on the dump.
3. The row count of every table and the newest migration must equal the backup's `counts.tsv`.
4. Every stored picture the restored database references (`media.storage_key` and its variants, except taken-down rows) must be in the restored media, and not empty. A missing one that the running database no longer references either was deleted while the backup ran (see "Backups"): the test logs it as `deleted after the backup` and does not fail. A missing one that the running database still references fails the test, and so does every missing one while the running database does not answer.

It removes the container and the restored files afterwards, and ends with a line such as `restore test passed: 31 tables, 1830 rows, migration 1900, 52 media files`, or `… 51 media files (1 deleted after the backup)`. Any other difference fails it, and the heartbeat alerts. The test only reads from the running stack. By hand: `/opt/bookclub/restore-test.sh`, or `systemctl start bookclub-restore-test`.

### Restoring for real

When the database or the pictures are lost, on this VM or on a new one set up as in "First boot", with `.env`, `backup.env`, the SSH key and the restic password from the password manager:

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

5. Start the app, with the version that made the backup or a newer one (`BOOKCLUB_TAG`; the last line of `counts.tsv` names the newest migration), and remove the restored files:

   ```sh
   docker compose up -d --wait
   rm -rf /var/lib/bookclub-backup/restore
   ```

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

## Still to come

Automatic updates with a timer that follows `stable` (DEP2, Q8).
