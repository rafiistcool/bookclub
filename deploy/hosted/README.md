# Bookclub hosted: operator runbook

This directory runs the hosted service on one VM: `compose.yaml` starts the app and its Postgres, and `env.example` lists every setting. The commands below run as root on the VM, in `/opt/bookclub`.

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

## Requirements

- Debian 13 with Docker Engine 25 or newer and the compose plugin 2.24 or newer.
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

## The first account

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

The new version applies its migrations when it starts. Migrations only go forward.

To roll back, or to hold updates back, pin an exact version in `.env`, for example `BOOKCLUB_TAG=0.11.0`, and run the same two commands. `docker compose images app` shows the tag that runs.

A version cannot run on a database that a newer version has migrated: it stops at startup with `cannot apply the database migrations: migration <N> was previously applied but is missing in the resolved migrations`. A rollback across a migration therefore needs the backup from before the update.

## Staging

Staging runs the same files on the same VM, on a temporary hostname and with test data only. It needs its own `PUBLIC_URL` and the SMTP account (see Requirements). Wipe it before the import:

```sh
docker compose down -v
find /srv/bookclub/media -mindepth 1 -delete
```

## When it does not start

`docker compose logs app` shows the reason in its last line, `error: …`. Each one names the setting:

- `MEDIA_DIR /data/media is not writable: …`: `chown 1000:1000 /srv/bookclub/media`.
- `bind source path does not exist: …` (from `docker compose up`): create the directory as in "First boot".
- `TRUSTED_PROXIES entry "CHANGE_ME" is not a CIDR or an IP address`: a value in `.env` is still the placeholder.
- `SMTP_URL must use TLS without DEV_MODE: …`: use `smtps://` or `?tls=required`.
- `cannot reach the database (gave up after 60 s)`: check `docker compose ps postgres` and `POSTGRES_PASSWORD`.

## Still to come

Later packages add the legacy import (`import-legacy`), backups and the restore test, the reverse proxy example, the switch to `SIGNUP_MODE=open`, and automatic updates.
