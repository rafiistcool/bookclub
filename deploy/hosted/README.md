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

## Updates and rollback

`BOOKCLUB_TAG=stable` follows the newest release. To update:

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
| minor | `1.2`, moves with each patch release | none |
| `stable` | moves with each release | none |

There is no `latest` and no floating major. A tag that is not `vX.Y.Z` or `vX.Y.Z-<pre-release>` fails the run. The workflow logs in with the repository secret `REGISTRY_TOKEN`, a Forgejo token with `write:package`. The old Python image `rafiistcool/bookclub` is never touched.

## Staging

Staging runs the same files on the same VM, on a temporary hostname and with test data only. It needs its own `PUBLIC_URL` and the SMTP account (see Requirements). Wipe it before the import:

```sh
docker compose down -v
find /srv/bookclub/media -mindepth 1 -delete
```

To run a second stack on the same Docker host, from another directory, give its `.env` its own `COMPOSE_PROJECT_NAME`, `BOOKCLUB_PORT` and `BOOKCLUB_DATA_DIR`. Compose takes the project name from `.env` too, so the commands in this file work unchanged in that directory.

## Reverse proxy

TLS ends at the Caddy that already serves git.tillhome.de, which forwards to `BOOKCLUB_LISTEN:BOOKCLUB_PORT`. In `.env`, `TRUSTED_PROXIES` is that Caddy's address as the VM sees it, with `/32` (Q7). If it is wrong, every user lands in one rate-limit bucket. The site block is `Caddyfile.example` next to this file, and `verify-limits.sh` checks the app's limits through it.

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
2. `restic backup` of that directory and of `/srv/bookclub/media`. The dump comes first, so every picture it references was on disk before the media were read.
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

`restore-test.sh` restores the newest snapshot into `/var/lib/bookclub-backup/restore-test` and checks it without touching the running stack:

1. restic verifies every restored file against the repository.
2. A throwaway `postgres:17` without a network runs `pg_restore --exit-on-error` on the dump.
3. The row count of every table and the newest migration must equal the backup's `counts.tsv`.
4. Every stored picture the restored database references (`media.storage_key` and its variants, except taken-down rows) must be in the restored media, and not empty.

It removes the container and the restored files afterwards, and ends with a line such as `restore test passed: 31 tables, 1830 rows, migration 1900, 52 media files`. Any difference fails it, and the heartbeat alerts. By hand: `/opt/bookclub/restore-test.sh`, or `systemctl start bookclub-restore-test`.

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

## Open sign-up

`SIGNUP_MODE=invite` is the closed beta: sign-up needs an invite. Switch to `open` (#55) only when every gate is green (work plan §5, steps 23 to 25):

- The legal texts are in `/srv/bookclub/legal`, and `/imprint`, `/privacy` and `/terms` show them signed out. Outside `DEV_MODE` the server refuses `SIGNUP_MODE=open` while one of the files is missing, and names the missing ones.
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
- `cannot reach the database (gave up after 60 s)`: check `docker compose ps postgres` and `POSTGRES_PASSWORD`.

## Still to come

Automatic updates with a timer that follows `stable` (DEP2, Q8).
