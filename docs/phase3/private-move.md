# The private move: runbook (#52)

This runbook moves the two current members from the Python app (v0.10.0, SQLite) to the Rust server (v0.11.0, Postgres) on the **same host, under the same hostname, behind the same reverse proxy**: invite-only, the name stays "Bookclub", mail off, no legal texts, no new external accounts. The design and the decisions are `docs/phase3/work-plan.md` §5 and §6 (answers 30–50); this file is the copy-paste part.

It is built on `docs/phase2/cutover.md` as COVER (#74) left it (the cover search inside `import-legacy`, its report lines, and the review of cover suggestions), and on the operator runbook `deploy/hosted/README.md`. The table below says which steps of `cutover.md` the private move keeps and which it drops. `cutover.md` stays as it is for the public launch.

"I" is the agent that runs the move from the agents VM over SSH; **[Rafael]** marks what only he does. Every block says where it runs: on the agents VM, or on the host as root.

## What may reach my terminal

Counts, statuses and exit codes only. No command whose output holds a secret, a password hash, user rows, titles or client addresses runs in my SSH session:

- The restic password is generated straight into its file (section 4), and **[Rafael]** copies it from his own SSH session. **[Rafael]** runs `docker login` himself when the registry needs it (section 2.3). No token or password passes through my terminal.
- `set-password`, if needed, writes to a root-only file (`umask 077`) that **[Rafael]** reads and deletes (section 10).
- The import reports and `compare.sh`'s output stay on the host, in root-only files. I read `compare.sh`'s exit status and result lines, and the reports through `summary` (section 0): the entity lines, `lookup wait` and `cover search`, and the other sections as line counts. Never the `isbn keys` or `without a cover` sections, which name books; what I need from them I count on the host with `grep -c`.
- The app's log only through `grep '^error:'`: its request lines carry paths. The log of `import-legacy` and `rematch-legacy` goes to a root-only file next to its report, and I read only its `error:` lines and a count of warnings: a throttled or failed lookup logs its ISBN.
- The proxy's address for `TRUSTED_PROXIES` is not a client's. `host-facts.sh` prints the peers of the old app, which behind a proxy is the proxy.

The notes (`docs/phase3/notes/move.md`) get the facts found on the host without secrets or usernames, the counts, and the phone test's result.

## `cutover.md` and the private move

| `cutover.md` | For the private move |
|---|---|
| §1 checklist | **Keep:** the stack on `stable` with `SIGNUP_MODE=invite` and `DNB_HARVEST=false`; backups set up and `restore-test.sh` passed once; no auto-updater touches either stack (§5.4); an empty target (no `beta-invite`, no sign-up); `install -d -o 1000 -g 1000 -m 700 "$IMPORT"`; room in the app's `/tmp`. **Drop:** name and domain, mail, legal texts, the uptime monitor, staging, the Caddy blocks, the gate address. `compare.sh` needs `sqlite3`: install it, or use the container stand-in (section 7.4). Section 5 here |
| §2 announcement | Replaced by one short chat message, DE and EN, a few days ahead (section 6) |
| §3.1 stop, snapshot | **Keep** `docker compose stop bookclub` in `$OLD` (stop, never down) and the `-journal` check. **Drop** the maintenance block, the checksums and the gpg copy: the stopped container and its untouched data directory are the rollback copy for 30 days (section 7.1) |
| §3.2 transfer | `cp` of `bookclub.db` (and `-wal` if present) into `$IMPORT`, `chown 1000:1000`, `chmod 600`: the same host, no `scp` (section 7.2) |
| §3.3 dry run, go/no-go, real run | As COVER leaves it; the update timer is not enabled yet. The go: `users 2 → 2` and the club with 2 members exactly; every other entity line carries its source over or names the difference, with counts at CUT2's (50 works, 36 editions, 146 events) or above, since CUT2 read an older copy and the club kept writing. Each run takes about 20 s plus about a minute of cover search (at most 5 minutes). `lookup wait` may be above 0 from the cover pass, and the `cover search` line and `without a cover` section may differ between the runs; neither is a no-go (section 7.3) |
| §3.4 `compare.sh` | Unchanged; exit 0 is the go (section 7.4) |
| §3.5 start and check | No gate. Start the app on the old port, `healthcheck`, `make-admin "$OWNER"`, Rafael's check, `rematch-legacy` if needed, `/sw.js`, `verify-limits.sh` from the agents VM, the phone test. No email step. COVER's review of cover suggestions waits until after "done" (section 7.5) |
| §3.6 open | No Caddy step: the port takeover was the switch. `backup.sh` and `restore-test.sh` once, then Rafael sends "done" (section 7.6) |
| §4 rollback | Section 8 |
| §5 afterwards | Section 9 |
| §6 preserving Python | Done (Q6 A) |

## 0. Variables

On the host, once the facts are in (section 2), I write them into `/root/bookclub-move/vars.sh`. It holds no secret. Every SSH session on the host starts with `. /root/bookclub-move/vars.sh`.

```bash
HOST=<hostname>                 # what the members open: BOOKCLUB_PUBLIC_URL of the old app, else Q-M1
OLD=<old compose directory>     # the Python app's compose project (service bookclub)
OLD_DATA=<old data directory>   # its mount on /data (host facts, "mounts"); usually $OLD/data
OWNER=<username>                # Rafael's username in the old app (CUT2: the user with id 1; he confirms it)
CLUB_NAME='Bookclub'            # BOOKCLUB_NAME of the old app (host facts)
CLUB_TZ=<zone>                  # BOOKCLUB_TZ of the old app (host facts: .env, else the container's; UTC when never set)
SLUG=bookclub-8rqp              # CUT2's slug; a new one when CLUB_NAME is not "Bookclub" (section 2.3)
NEW=/opt/bookclub               # /opt/bookclub-server when the old app holds /opt/bookclub (section 2.3)
DATA=/srv/bookclub              # /srv/bookclub-server likewise
IMPORT=$DATA/import             # the legacy copy: mode 700, uid 1000
R=/root/bookclub-move           # deploy/ of the release, copied from the agents VM (section 3.1)

# The report without what names people or books: entity lines, lookup wait, cover search, and each
# section as its title and a line count.
summary() {
    awk '/^  / { n++; next }
         title != "" { printf "%s %d lines\n", title, n }
         { title = ""; n = 0 }
         /:$/ { title = $0; next }
         { print }
         END { if (title != "") printf "%s %d lines\n", title, n }' "$@"
}
```

`CLUB_TZ` sets the club's and both imported accounts' time zone (`docs/phase1/legacy-mapping.md`). The old app's value keeps the times as they were; both can be changed in the app afterwards (club settings, Settings).

## 1. Before: the release (M2)

Wave 1 (SCAN, FRIENDS, M1) is merged into `phase-3`, and M1's boot test (`work-plan.md` §2.3) passed once more on it. **[Rafael]** approved the merge of the PR `phase-3` → `main`; the tag `v0.11.0` on `main` follows without a second approval. Once the release workflow has run, the three tags must name one digest. The package is private and the agents VM has no registry login (a token would pass through my terminal), so I read Forgejo's package API with the `forgejo` MCP tools instead of `docker buildx imagetools inspect` (`docs/phase2/notes/dep2.md`, "Not verified"): `list_package_files` with owner `rafiistcool`, type `container`, name `bookclub-server`, for the versions `0.11.0`, `0.11` and `stable`. Each lists a `manifest.json` whose `sha256` is that tag's manifest digest.

Three equal `sha256` values are the go. Nothing follows `stable` on the host until the update timer is on (section 9).

## 2. Access and facts (M3)

### 2.1 The key

On the agents VM, a key made only for the move (answer 48):

```bash
ssh-keygen -t ed25519 -N '' -C bookclub-move -f ~/.ssh/bookclub-move
cat ~/.ssh/bookclub-move.pub
```

**[Rafael]** adds one line to root's `~/.ssh/authorized_keys` on the host: the agents VM's address as the host sees it, then the public key.

```
from="<agents VM address>",no-agent-forwarding,no-port-forwarding,no-X11-forwarding ssh-ed25519 AAAA… bookclub-move
```

I connect with `ssh -i ~/.ssh/bookclub-move root@<host's SSH address>`. Section 9 removes the line again.

### 2.2 The host facts

From the agents VM, a checkout of `main` at `v0.11.0`: copy `deploy/` to the host and run `host-facts.sh` while someone uses the app (its peers sample takes 10 s; `PEER_SECONDS` changes that):

```bash
# On the agents VM, in the checkout:
tar -czf - deploy/hosted deploy/cutover |
    ssh -i ~/.ssh/bookclub-move root@<host> 'install -d -m 700 /root/bookclub-move && tar -C /root/bookclub-move -xzf -'
# On the host:
/root/bookclub-move/deploy/cutover/host-facts.sh <old compose directory> <hostname>
echo "exit $?"
```

It is read-only and prints no secret (its header lists what it reads). Exit 1 means a check stops the move: the `STOP:` lines say which (`uname -m` not `x86_64`, Docker older than 25 or compose older than 2.24, no answer over HTTPS). The move waits until they are fixed.

The facts go into the notes without secrets or usernames: OS, Docker and compose versions, the old image, binding and port, the layout chosen, the project name, the backup target, the proxy's place (same host or not), auto-updaters.

### 2.3 What the facts decide

- **Layout.** `/opt/bookclub` and `/srv/bookclub` free: `NEW=/opt/bookclub`, `DATA=/srv/bookclub`. If the old app holds either (the script's note says so), the new stack goes to `/opt/bookclub-server` and `/srv/bookclub-server`: `BOOKCLUB_DATA_DIR` in `.env`, `COMPOSE_DIR`, `MEDIA_DIR` and `RESTIC_PASSWORD_FILE` in `backup.env`, and the three units' `ExecStart` (section 4).
- **Project name.** `compose.yaml` says `name: bookclub`. If `docker compose ls` shows the old project under that name (a directory named `bookclub` is enough), the new `.env` sets `COMPOSE_PROJECT_NAME=bookclub-server`. Otherwise the stopped Python container becomes an orphan of the new project, and a `down` or `--remove-orphans` could remove it.
- **The binding.** The old container's published binding and port (Python's default `0.0.0.0:8000`) become `BOOKCLUB_LISTEN` and `BOOKCLUB_PORT`: the port takeover. `BOOKCLUB_LISTEN=127.0.0.1` only when the proxy's upstream is `127.0.0.1:<old port>` or `localhost:<old port>`. That takes a proxy that runs as a process on this host: "Listening on 80 and 443" names nginx, caddy, haproxy, apache2 or traefik, not `docker-proxy` and not nothing. Its upstream, read as addresses only (or Rafael names it):

  ```bash
  grep -rhoE "[][A-Za-z0-9.:-]+:<old port>\b" /etc/nginx /etc/caddy /etc/haproxy /etc/apache2 2>/dev/null | sort -u
  ```

  Every other proxy keeps the old binding: a proxy container, and a proxy whose upstream is the host's LAN address, `172.17.0.1` or `host.docker.internal`. A `127.0.0.1` binding takes only connections to `127.0.0.1`, so such a proxy would get "connection refused" after the switch, and the members a 502. The peer cannot tell these apart: loopback through Docker's proxy and a proxy container's connection to a host address, which Docker masquerades, both show a Docker gateway. An upstream on `[::1]` reaches neither binding (the new app publishes IPv4 only): I send Rafael the one-line change to `127.0.0.1:<old port>`, and reverting it joins the rollback.
- **No published port.** If the old container publishes nothing and the proxy, a container on a shared Docker network, reaches it by an alias (the facts' "networks" line), the app joins that network under that alias instead (section 3.1). If that does not fit the proxy, I send Rafael a one-line upstream change for the proxy; reverting it joins the rollback.
- **`TRUSTED_PROXIES`.** From the peer of the old container (the facts' "peers"), as `/32`:
  - a proxy on another host: its LAN address, unchanged for the new app;
  - a Docker gateway, an address of a `docker0` or `br-…` interface (`ip -br addr`): a proxy on this host over loopback, or a proxy container that dials a host address. The value is the gateway of the **new** app's network, not the old one's (each compose network has its own), read after the first start in section 3.2. With `BOOKCLUB_LISTEN=127.0.0.1` only this host reaches the port. With the old binding kept, the host's other containers that dial the port show the same gateway; LAN clients keep their own address;
  - another address of this host (a host proxy that dials the host's LAN address): unchanged;
  - a proxy container on a shared network: its address there as `/32`, or the network's subnet when the proxy container gets a new address on each re-creation (only containers on that network reach the app there).

  A wrong value puts everyone in the proxy's rate-limit bucket: one stranger's failed attempts lock both members out of sign-in for 10 minutes and out of reset and sign-up for an hour. A wide one lets LAN clients forge `X-Forwarded-For`. Move day checks it from two sides (section 7.5).
- **Name, time zone, slug.** `CLUB_NAME` and `CLUB_TZ` from the facts' "old settings" (the `.env` value, else the container's). `SLUG=bookclub-8rqp` (CUT2's) when the name is "Bookclub". Otherwise a new slug by D5, once, before the message goes out (on the agents VM; the importer checks the result):

  ```bash
  base=$(printf '%s' "$CLUB_NAME" | sed 's/[Ää]/ae/g; s/[Öö]/oe/g; s/[Üü]/ue/g; s/ß/ss/g' |
      iconv -f utf-8 -t ascii//TRANSLIT | tr '[:upper:]' '[:lower:]' | tr -cs 'a-z0-9' '-' |
      sed 's/^-*//; s/-*$//' | cut -c 1-50 | sed 's/-*$//')
  echo "${base:-club}-$(LC_ALL=C tr -dc 'a-z0-9' </dev/urandom | head -c 4)"
  ```

- **Backups.** `RESTIC_REPOSITORY` is an absolute path: a mounted NAS share if the facts list one; otherwise a directory on another physical disk; otherwise `/srv/bookclub-backup` on the same disk. In the last case I tell Rafael that the backups then guard against mistakes, not against losing the disk, until the Storage Box comes with the public launch. Never under `/tmp` (the units use `PrivateTmp`). restic runs as root: on an NFS share with root squashing, root cannot write, so the directory must take a `touch` as root first.
- **Auto-updaters.** If one runs, the new stack is excluded before section 3 (watchtower: the label in `compose.override.yaml`, section 3.1), and it must not revive stopped containers (watchtower: no `--include-stopped`, `--revive-stopped`, `WATCHTOWER_INCLUDE_STOPPED` or `WATCHTOWER_REVIVE_STOPPED`), or the stopped Python app would come back during the move. Otherwise I stop it. Its flags, by name only, since its command line and environment can hold tokens (`--http-api-token`, notification URLs): `docker inspect -f '{{range .Config.Cmd}}{{println .}}{{end}}' <name> | grep -E '^(--label-enable|--include-stopped|-S|--revive-stopped)(=.*)?$'` and `docker inspect -f '{{range .Config.Env}}{{println .}}{{end}}' <name> | grep -E '^WATCHTOWER_(LABEL_ENABLE|INCLUDE_STOPPED|REVIVE_STOPPED)='`. `update.sh`, which backs up first, is the new stack's only updater.
- **Registry.** The package is private. If root's Docker config does not name `git.tillhome.de`, **[Rafael]** creates a Forgejo token with only `read:package` (Settings → Applications) and runs `docker login git.tillhome.de` with it as root on the host.
- **The scanner.** If the proxy sends `Permissions-Policy` with `camera=()`, I send Rafael the exact line with `camera=(self)`, or the scanner waits. Not a stop.
- **`sqlite3`.** Missing: install it for `compare.sh` (`apt-get install -y sqlite3`), or use the stand-in of section 7.4.

## 3. Install the stack next to the Python app (M3)

### 3.1 Files and settings

On the host (`deploy/hosted/README.md`, "First boot" steps 1–4, without step 5's test mail):

```bash
. /root/bookclub-move/vars.sh
install -d -m 0750 "$NEW"
cp "$R/deploy/hosted/compose.yaml" "$NEW/"
install -m 0600 "$R/deploy/hosted/env.example" "$NEW/.env"
cd "$NEW"
# Replaces KEY=… or #KEY=… in .env, or adds the line.
set_env() {
    if grep -qE "^#?$1=" .env; then sed -i "s|^#\?$1=.*|$1=$2|" .env; else printf '%s=%s\n' "$1" "$2" >> .env; fi
}
set_env POSTGRES_PASSWORD "$(openssl rand -hex 32)"
set_env PUBLIC_URL "https://$HOST"
set_env TRUSTED_PROXIES '<proxy address>/32'   # a Docker gateway as the old peer: none until section 3.2
set_env CONTACT_EMAIL '<the address of Q-M3>'
set_env EMAIL_TRANSPORT none
sed -i -e 's|^SMTP_URL=|#SMTP_URL=|' -e 's|^EMAIL_FROM=|#EMAIL_FROM=|' .env
set_env BOOKCLUB_LISTEN '<old binding>'
set_env BOOKCLUB_PORT '<old port>'
set_env BOOKCLUB_DATA_DIR "$DATA"
# Only when the old project is named bookclub (section 2.3):
set_env COMPOSE_PROJECT_NAME bookclub-server
grep -c '^[^#].*CHANGE_ME' .env                                     # 0
grep -vE '^(#|$|POSTGRES_PASSWORD=|DATABASE_URL=)' .env             # the settings, without the password
install -d -o 1000 -g 1000 -m 0750 "$DATA/media" "$DATA/legal"
install -d -o 1000 -g 1000 -m 0700 "$IMPORT"
```

The rest keeps `env.example`'s values: `BOOKCLUB_TAG=stable`, `SIGNUP_MODE=invite`, `LEGAL_DIR=/app/legal` (the empty `$DATA/legal`: no texts, no terms prompt, `/imprint` shows "missing", the footer links stay hidden), `DNB_HARVEST=false`, `COOKIE_SECURE` and `SERVICE_NAME` at their defaults (`work-plan.md` §5.1). With `EMAIL_TRANSPORT=none` the server needs neither `SMTP_URL` nor `EMAIL_FROM`, refuses `SIGNUP_MODE=open`, sends nothing and voids reset links; password resets go through `set-password` (section 10).

**Only when the proxy reaches the old app by a network alias** (section 2.3): `.env` gets `BIND_ADDR=0.0.0.0:8000` (the image's healthcheck reads it too) and `BOOKCLUB_LISTEN=127.0.0.1`, and next to `compose.yaml`:

```bash
cat > "$NEW/compose.override.yaml" <<'EOF'
# The private move (#52): the proxy reaches the app as <alias>:8000 on <proxy network>, as it
# reached the Python app. Compose loads this file by itself, so update.sh and the backups see it.
services:
  app:
    networks:
      proxy:
        aliases: [<alias>]
networks:
  proxy:
    name: <proxy network>
    external: true
EOF
```

A proxy that resolves the alias only when it starts, as nginx does with `proxy_pass http://<alias>:8000;` and no `resolver`, keeps dialing the stopped Python container's address: the new container gets another one on that network. It answers 502 after the switch (section 7.5 checks it) until it is reloaded, and again after a rollback. Then the reload is part of move day and of the rollback: `docker exec <proxy container> nginx -s reload` for an nginx container, Rafael's reload for another proxy.

**Only when an auto-updater runs** (watchtower; another one by its own rule), in the same `compose.override.yaml` (create it with just this when there is no alias):

```yaml
services:
  app:
    labels:
      com.centurylinklabs.watchtower.enable: "false"
  postgres:
    labels:
      com.centurylinklabs.watchtower.enable: "false"
```

`docker compose config -q` in `$NEW` checks the result.

### 3.2 Only Postgres, and one check of the app

Before move day only `postgres` runs. The app starts once on a spare loopback port, so the old port stays the Python app's, and stops again. The check runs with `-f compose.yaml` only, so with an alias override the new app never answers under the alias early, and with `--no-deps`, so Postgres keeps the override's labels:

```bash
cd "$NEW"
docker compose pull
docker compose up -d --wait postgres
BOOKCLUB_LISTEN=127.0.0.1 BOOKCLUB_PORT=18080 docker compose -f compose.yaml up -d --wait --no-deps app
docker compose exec app bookclub-server healthcheck        # … answered 200 OK
curl -sS http://127.0.0.1:18080/api/config; echo           # "signup":"invite", no terms version (not with BIND_ADDR=…:8000)
docker compose exec app bookclub-server send-test-mail check@example.com; echo "exit $?"
#   error: mail is off (EMAIL_TRANSPORT=none) / exit 1
docker compose stop app
docker compose ps -a
```

If it does not come up: `docker compose logs --no-log-prefix app | grep '^error:' | tail -n 1` names the setting (README, "When it does not start").

**When the old peer is a Docker gateway** (section 2.3): the app's network exists now, and its gateway is the address the app will see for the proxy:

```bash
cd "$NEW"
network=$(docker inspect -f '{{range $name, $net := .NetworkSettings.Networks}}{{$name}} {{end}}' \
    "$(docker compose ps -aq app)" | tr ' ' '\n' | grep -v -e '_db$' -e '^$' | head -n 1)
gateway=$(docker network inspect -f '{{range .IPAM.Config}}{{.Gateway}}{{end}}' "$network")
echo "$network $gateway"
sed -i "s|^TRUSTED_PROXIES=.*|TRUSTED_PROXIES=$gateway/32|" .env
```

The importer takes a migrated empty database, and this start migrated it. The target must hold no users and no invites: **no `beta-invite`, no sign-up** until the import.

## 4. Backups and the update script (M3)

On the host (README, "Backups", "Setting up", with a local repository):

```bash
. /root/bookclub-move/vars.sh
cp "$R"/deploy/hosted/{backup.sh,restore-test.sh,backup-lib.sh,update.sh} "$NEW/"
install -m 0600 "$R/deploy/hosted/backup.env.example" "$NEW/backup.env"
openssl rand -base64 32 | install -m 600 /dev/stdin "$NEW/restic-password"
REPO=<the repository directory of section 2.3>
install -d -m 0700 "$REPO"
sed -i -e "s|^RESTIC_REPOSITORY=.*|RESTIC_REPOSITORY=$REPO|" \
    -e "s|^COMPOSE_DIR=.*|COMPOSE_DIR=$NEW|" \
    -e "s|^MEDIA_DIR=.*|MEDIA_DIR=$DATA/media|" \
    -e "s|^RESTIC_PASSWORD_FILE=.*|RESTIC_PASSWORD_FILE=$NEW/restic-password|" "$NEW/backup.env"
# Heartbeats only to a monitor Rafael already runs (Q-M2): the three URLs he names, else empty.
grep -E '^(RESTIC_REPOSITORY|COMPOSE_DIR|MEDIA_DIR|RESTIC_PASSWORD_FILE)=' "$NEW/backup.env"
"$NEW/backup.sh" restic init
"$NEW/backup.sh"; echo "exit $?"
"$NEW/restore-test.sh"; echo "exit $?"                      # restore test passed: …
cp "$R"/deploy/hosted/systemd/bookclub-* /etc/systemd/system/
# Only with NEW=/opt/bookclub-server:
sed -i "s|/opt/bookclub/|$NEW/|" /etc/systemd/system/bookclub-{backup,restore-test,update}.service
systemctl daemon-reload
systemctl enable --now bookclub-backup.timer bookclub-restore-test.timer
systemctl list-timers 'bookclub-*'
```

The update timer stays off until the day after the move (section 9). Without heartbeats a failed backup or restore test stays silent until someone looks; that is accepted until the public launch brings a monitor, and section 11 checks the backups before it deletes the old copy.

**[Rafael]**, from his own SSH session as root: `cat <NEW>/restic-password`, into the password manager. Without it the backups are unreadable.

## 5. The checklist before move day

Checked the day before, from my SSH session:

- [ ] The facts' stops are cleared; layout, project name, binding (or alias), `TRUSTED_PROXIES`, slug and time zone are in `vars.sh` and `.env`.
- [ ] `docker compose ps -a` in `$NEW`: `postgres` healthy, `app` exited; `BOOKCLUB_TAG=stable` names v0.11.0; `SIGNUP_MODE=invite`, `DNB_HARVEST=false`, `EMAIL_TRANSPORT=none`.
- [ ] The target is empty, in `$NEW`: `docker compose exec -T postgres psql -U bookclub -d bookclub -Atc 'SELECT (SELECT count(*) FROM users), (SELECT count(*) FROM club_invites)'` prints `0|0`.
- [ ] `backup.sh` and `restore-test.sh` passed once; both timers are listed; Rafael has the restic password.
- [ ] No auto-updater touches either stack (section 2.3).
- [ ] `$IMPORT` exists, mode 700, owner 1000.
- [ ] Room in the app's `/tmp` (a 64 MB tmpfs) for `bookclub.db` and its `-wal`: the facts' "data files" sizes together are well below 64 MB; otherwise set `TMPDIR` as README "The legacy import" says.
- [ ] `sqlite3` is installed, or the stand-in of section 7.4 is ready.
- [ ] The scanner: the proxy's `Permissions-Policy` allows `camera=(self)`, or Rafael knows the scanner waits.
- [ ] The message went out (section 6).

## 6. The message

**[Rafael]** picks the time and sends it a few days ahead, in the chat he shares with the other member.

**Deutsch**

> Bookclub bekommt am \<Tag\> ab \<Uhrzeit\> ein großes Update, etwa eine Stunde Pause. Adresse, Benutzername und Passwort bleiben gleich, alles zieht mit. Danach einmal neu anmelden. Nicht abgeschickte Notizen bitte vorher abschicken. Die App fragt nach einer E-Mail-Adresse: eintragen oder überspringen, Mails verschickt sie vorerst keine. Passwort vergessen? Sag mir Bescheid. Neue Leute lade ab jetzt ich ein. Ich melde mich, wenn es fertig ist.

**English**

> Bookclub gets a big update on \<day\> from \<time\>, with a break of about an hour. The address, your username and your password stay the same, and everything moves with you. Afterwards, sign in once more. Please send any unsent notes before then. The app asks for an email address: add one or skip it, it sends no mail for now. Forgot your password? Tell me. From now on, I invite new people. I'll let you know when it's done.

**Done** (section 7.6):

> Fertig, Bookclub läuft wieder unter der gewohnten Adresse. / Done, Bookclub is back at the usual address.

## 7. Move day (M4)

Every block on the host as root, after `. /root/bookclub-move/vars.sh`, unless it says otherwise. About an hour in all.

### 7.1 Stop the old app

```bash
cd "$OLD"
docker compose stop bookclub            # stop, never down: the container stays for the rollback
ls "$OLD_DATA" | grep '^bookclub\.db'   # after a clean stop, normally no bookclub.db-wal is left
```

If a `bookclub.db-journal` shows, the app did not stop cleanly: `docker compose start bookclub`, wait until `docker compose ps bookclub` says healthy, stop it again and list again. No snapshot, checksum or gpg copy: the stopped container and its untouched data directory are the rollback copy for 30 days.

### 7.2 The copy

```bash
rm -f "$IMPORT"/bookclub.db*            # a -wal left from an earlier attempt would be read with the new copy
cp "$OLD_DATA/bookclub.db" "$IMPORT/"
[ -e "$OLD_DATA/bookclub.db-wal" ] && cp "$OLD_DATA/bookclub.db-wal" "$IMPORT/"
chown 1000:1000 "$IMPORT"/bookclub.db* && chmod 600 "$IMPORT"/bookclub.db*
ls -l "$IMPORT" | awk 'NR > 1 { print $1, $3, $5, $NF }'
```

### 7.3 Dry run, go/no-go, real run

In `$NEW`. The app is stopped (section 3.2), so nobody signs up while the import runs; the update timer is not on yet. `-T` keeps the report free of terminal line endings; `umask 077` keeps the report and the log root's. The log stays in `dry.log`: a throttled or failed lookup logs its ISBN.

```bash
cd "$NEW"
docker compose stop app
(umask 077; docker compose run --rm -T -v "$IMPORT:/import:ro" app \
    bookclub-server import-legacy /import/bookclub.db --owner "$OWNER" \
    --club-name "$CLUB_NAME" --timezone "$CLUB_TZ" --club-slug "$SLUG" \
    --allow-unmatched --dry-run > dry.txt 2> dry.log); echo "exit $?"
grep '^error:' dry.log; grep -c ' WARN ' dry.log
summary dry.txt
grep -c ': not searched (database error)$' dry.txt      # 0
```

Exit 2 means the target is not empty (section 5); exit 1: the `error:` line says why. `--allow-unmatched` (D6) keeps going when the DNB or Open Library cannot answer: those ISBN rows become bare facts (`isbn error`), and `rematch-legacy` resolves them later.

**Go/no-go**, against CUT2's report (`docs/phase2/notes/cut2.md`, "Results"). CUT2 read an older copy of `bookclub.db`, and the club kept writing since, so CUT2's counts are a floor, not the expected values:

- exactly `users 2 → 2`, and `club - → 1` with `$SLUG` and 2 members;
- every other entity line carries its source over: the target equals the source, or the line names the difference (`merged`, `dropped`, `with a dangling reference`; `push_subscriptions … → DROPPED`). `books N → M works`: M is N minus the line's `merged`, and its class counts add up to N. `editions - → E`: E is at most the `isbn keys` section's line count;
- the counts at CUT2's or above (`books 51 → 50 works`, `editions - → 36`, `events 146 → 146`, `shelf 50 → 50`, …). A few below, where a member removed something, is possible; many below is a no-go;
- `notes`: 0 lines (CUT2 had none). Anything there: stop, and Rafael reads `dry.txt` on the host;
- `merges` and `differences` about CUT2's (1 and 19 lines) or more, `isbn keys` about 36 or more;
- no book "not searched (database error)": the reason stands in the `without a cover` section, which names books, so the last command above counts it on the host and must print 0. "Not searched" for an outage, a busy Open Library or the time budget, and books without a cover, are no no-go: `rematch-legacy` and the server's cover search take them later.

Anything unexpected: stop here and roll back (section 8). The target is still empty.

```bash
(umask 077; docker compose run --rm -T -v "$IMPORT:/import:ro" app \
    bookclub-server import-legacy /import/bookclub.db --owner "$OWNER" \
    --club-name "$CLUB_NAME" --timezone "$CLUB_TZ" --club-slug "$SLUG" \
    --allow-unmatched > real.txt 2> real.log); echo "exit $?"
grep '^error:' real.log; grep -c ' WARN ' real.log
summary real.txt
diff <(tail -n +2 dry.txt | summary) <(summary real.txt)
grep -c ': not searched (database error)$' real.txt     # 0
```

The diff may show only the `lookup wait` line, the `cover search` line and the line count of `without a cover`: Open Library answers, throttles or fails differently between two runs, and the time budget may stop the search at another book. When a lookup answered in one run only, the `books` line's `isbn …` counts differ, and `editions`, `merges` and `differences` may move with them; then the real run's summary goes through the go/no-go above once more, and `compare.sh` checks that report against the database.

### 7.4 The count check, before anyone signs in

```bash
cd "$NEW"
(umask 077; "$R/deploy/cutover/compare.sh" --report real.txt "$IMPORT/bookclub.db" -- \
    docker compose exec -T postgres psql -U bookclub -d bookclub > compare.txt); echo "exit $?"
grep -E 'MISMATCH|^accounts:|^result:' compare.txt
```

Exit 0 is the go. It counts both databases with its own queries, checks every count against the report, and compares `id|username|password_hash` of every account without printing a hash. The account comparison is valid only now: a login re-hashes a legacy hash. Its per-user table names people and stays in `compare.txt`.

**Without `sqlite3` on the host**, a stand-in that runs it in a container (built once, before move day; `compare.sh` copies the legacy file into `/tmp` and reads that copy). Its path goes into `vars.sh`, so the move day session has it:

```bash
docker build -t bookclub-sqlite3 - <<'EOF'
FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends sqlite3 && rm -rf /var/lib/apt/lists/*
ENTRYPOINT ["sqlite3"]
EOF
printf '#!/bin/sh\nexec docker run --rm -i --network none -v /tmp:/tmp bookclub-sqlite3 "$@"\n' > "$R/sqlite3"
chmod 700 "$R/sqlite3"
echo "export SQLITE3=$R/sqlite3" >> "$R/vars.sh"   # compare.sh runs it instead of sqlite3
```

Section 9 removes the image (`docker image rm bookclub-sqlite3`).

### 7.5 Start and check

No gate: the other member waits for "done". The Python container is stopped, so its port is free.

```bash
cd "$NEW"
docker compose up -d --wait app                          # on the old binding and port now
docker compose exec app bookclub-server healthcheck      # … answered 200 OK
docker compose exec app bookclub-server make-admin "$OWNER"
curl -s -o /dev/null -w '%{http_code}\n' "https://$HOST/api/health"     # 200: the proxy reaches the new app
```

A 502 or 504 means the proxy does not reach the new app. With the port takeover, the binding does not fit the proxy's upstream: back to the old binding in `.env` (section 2.3) and `docker compose up -d --wait app`. With the alias, the proxy still holds the Python container's address: reload it (section 3.1). Rafael signs in only after a 200.

**[Rafael]**, in his browser on `https://$HOST`: sign in with the old password (everyone signs in again once: the cookie is new). The email prompt may be skipped. Then the club, the shelves, the physical shelf layouts, the pick and its notes, the votes, the quotes and the activity feed are there, and an old `/works/OL…W` link from a chat opens.

If he cannot sign in with the old password: section 10, then the check.

If `isbn unmatched` or `isbn error` in the report's `books` line is not 0, rematch now (it commits per edition and per step of each book's cover search, and is safe on the running app):

```bash
cd "$NEW"
(umask 077; docker compose exec -T app bookclub-server rematch-legacy --dry-run 2> rematch-dry.log | summary)
grep '^error:' rematch-dry.log; grep -c ' WARN ' rematch-dry.log
(umask 077; docker compose exec -T app bookclub-server rematch-legacy 2> rematch.log | summary)
grep '^error:' rematch.log; grep -c ' WARN ' rematch.log
```

COVER's review of cover suggestions (Admin → Covers) waits until after "done": a missing cover is no no-go, and Rafael decides them later (answer 45).

The old worker's kill switch, from anywhere:

```bash
curl -sI "https://$HOST/sw.js" | grep -iE '^(HTTP|content-type)'    # 200, content-type: text/javascript
```

**`TRUSTED_PROXIES`, from two sides** (answer 49). First the peers the new app sees, while Rafael's browser is open (README, "The VM's network", check 3): only the proxy's address, the one in `.env`.

```bash
# The app listens on 8080 in its container (8000 with the alias override's BIND_ADDR).
cd "$NEW"
nsenter -n -t "$(docker inspect -f '{{.State.Pid}}' "$(docker compose ps -q app)")" \
    ss -Htn state established '( sport = :8080 )' |
    awk '{ peer = $NF; sub(/:[0-9]+$/, "", peer); print peer }' | sort | uniq -c
grep '^TRUSTED_PROXIES=' .env
```

Then, on the agents VM, in the checkout:

```bash
deploy/hosted/verify-limits.sh "https://<HOST>"; echo "exit $?"
```

It must get its 429s (exit 0). It creates nothing and uses up only the budgets of the address the proxy sees for the agents VM: when that is also the address of Rafael's home network, signing in from home answers 429 for up to 10 minutes. Then **[Rafael]** signs in on his phone **with Wi-Fi off** (mobile data). A 429 on the phone means `TRUSTED_PROXIES` or the proxy's `X-Forwarded-For` is wrong: I fix `.env`, `docker compose up -d --wait app`, and he tries again.

Then **the phone test** (`work-plan.md` §5.8): **[Rafael]** on his iPhone with Wi-Fi off (and an Android phone with Chrome if one is at hand): open `https://$HOST`, sign in, scan a German book, an English book and one the app does not know, add a photo to the unknown one, and deny the camera once to see manual entry. An iPhone older than iOS 16 shows manual entry only. The result goes into the notes; a failure becomes a patch release, never a rollback.

### 7.6 Open

No proxy step: the port takeover was the switch. One backup of the imported data, and its restore test:

```bash
"$NEW/backup.sh"; echo "exit $?"
"$NEW/restore-test.sh"; echo "exit $?"         # restore test passed: …
```

Then **[Rafael]** sends "done" (section 6). The rollback window closes when the other member writes to the new app.

## 8. Rollback

The window lasts until the other member writes to the new app. Until then the old app's data is untouched (stopped, and the importer read a copy), and the only writes on the new app are Rafael's checks, which a rollback discards.

1. In `$NEW`: `docker compose stop app`.
2. In `$OLD`: `docker compose start bookclub`, wait until `docker compose ps bookclub` says healthy, and sign in once. If section 2.3 needed a proxy change, **[Rafael]** reverts it. A proxy that needed a reload on move day (section 3.1) needs one now too.
3. In `$NEW`: `docker compose down -v` and `find "$DATA/media" -mindepth 1 -delete`, so the next attempt starts from an empty target. Before the next attempt, section 3.2 again (`up -d --wait postgres`, the check on 18080).
4. Rafael tells the other member the update is postponed, and I record why in #52.

Python's `restart: unless-stopped` keeps a stopped container stopped after a reboot. Starting it while the new app holds the port fails with "port already allocated", which is safe. In the alias case no port stops it: it would answer under the same alias as the new app. So nothing runs `docker compose up` or `start` in `$OLD` outside the rollback, also when the proxy is a service of that project.

After the window there is no rollback: the members' writes exist only on the new app. Problems are fixed forward; the backup timer covers the new data from the first night on.

## 9. Afterwards (M5)

**Move day, after "done":**

- The catalog harvest on:

  ```bash
  cd "$NEW"
  sed -i 's/^DNB_HARVEST=.*/DNB_HARVEST=true/' .env
  docker compose up -d --wait app
  ```

- The counts into #52, counts only (no usernames, no titles), then the import copy, the reports and their logs go. The copy holds every password hash:

  ```bash
  rm -f "$IMPORT"/bookclub.db* "$NEW"/{dry,real,compare}.txt "$NEW"/{dry,real,rematch-dry,rematch}.log
  ```

**The next day, or when the other member has used the app** (the rollback window has closed): the update timer (README, "Updates and rollback", "Setting up"; the units are installed, section 4):

```bash
"$NEW/update.sh"; echo "exit $?"        # … is up to date (…), and the app is healthy
systemctl enable --now bookclub-update.timer
systemctl list-timers 'bookclub-*'
```

From then on `stable` reaches the host within minutes of a release, each time after a backup; Rafael never runs a deploy.

**When it suits Rafael:** Admin → Covers: the suggestions ("Use this cover" or "Not this book"), and the books readers use that still have none (`cutover.md` §5, "The day after").

**CUT-R** (`docs/phase2/work-plan.md` §2.4):

- The branch `cutover/remove-python`, cut from `main` after v0.11.0. The deletes are exactly §2.4's.
- Docs: the README describes the private invite-only instance and drops `README.md:162` (that closes #35's README criterion); `SELFHOST.md` goes; CLAUDE.md loses "In transition" and the Python commands; `docs/plan.md` gets a status line with the move's date.
- **[Rafael]** approves the merge after the move is verified, once Forgejo CI is green.

**Issues:**

- Close #52 with the counts, and #56 after the phone test.
- Close #35: the password change is in Settings (`AccountSettings.vue`, `PATCH /api/auth/password`) and runs in production from the move on.
- Close #34 as superseded, after adding "Web Push for the browser app" to #60 (D9).
- #51 stays open for the public launch, with a comment on what the move delivered.

**Last step of M5**, from my SSH session: the move's files and my key go.

```bash
docker image rm bookclub-sqlite3 2>/dev/null
rm -rf /root/bookclub-move
sed -i '/ bookclub-move$/d' /root/.ssh/authorized_keys
```

On the agents VM: `ssh -i ~/.ssh/bookclub-move root@<host> true` now fails with `Permission denied`; then `rm ~/.ssh/bookclub-move ~/.ssh/bookclub-move.pub`.

## 10. A forgotten password

Mail is off, so "Forgot password" sends nothing (it still says a link is on its way). The member asks Rafael. During the move I run it, afterwards **[Rafael]** does, in the stack's directory:

```bash
cd /opt/bookclub                  # or /opt/bookclub-server
(umask 077 && docker compose exec -T app bookclub-server set-password <username> > /root/new-password); echo "exit $?"
```

The file holds a new random password of 20 characters (capital letters and digits) and nothing else. Every session and app token of the account ends, and its open links are voided; an unknown name exits 1 with `No account with that name`. **[Rafael]** reads it from his own session (`cat /root/new-password && rm /root/new-password`), passes it on, and the member changes it in Settings the same day. The app keeps failed sign-ins in memory, so a member it locked out still waits up to 10 minutes.

## 11. After 30 days (M6): the cleanup block

**[Rafael]** pastes this on the host as root, with the three values filled in at the move. It deletes only when the last 8 days hold a passing restore test from the timer, the newest snapshot is from last night, and a restore test run right then passes (about a minute): a pass from last week says nothing about this week's run. Otherwise it stops and deletes nothing.

```bash
bash -s <<'CLEANUP'
set -euo pipefail
NEW=<NEW>                 # e.g. /opt/bookclub
OLD=<OLD>                 # the Python app's compose directory
OLD_DATA=<OLD_DATA>       # its data directory, with bookclub.db
cd "$NEW"
if ! journalctl -u bookclub-restore-test --since '8 days ago' --no-pager | grep -F 'restore test passed' >/dev/null; then
    echo "stop: no passing restore test in the last 8 days (is bookclub-restore-test.timer on?); nothing was deleted"
    exit 1
fi
snapshots=$("$NEW/backup.sh" restic snapshots --latest 1 --json)
latest=$(grep -o '"time":"[^"]*"' <<<"$snapshots" | sed -n '1s/^"time":"\(.*\)"$/\1/p')
if [[ -z $latest ]] || (( $(date +%s) - $(date -d "$latest" +%s) > 36 * 3600 )); then
    echo "stop: the newest snapshot (${latest:-none}) is not from last night; nothing was deleted"
    exit 1
fi
[[ -f $OLD_DATA/bookclub.db ]] || { echo "stop: $OLD_DATA holds no bookclub.db; nothing was deleted"; exit 1; }
if ! systemctl start bookclub-restore-test; then
    echo "stop: the restore test failed just now (journalctl -u bookclub-restore-test); nothing was deleted"
    exit 1
fi
cd "$OLD"
container=$(docker compose ps -aq bookclub)
image=$(docker inspect -f '{{.Image}}' "$container")
# Every tag of the image (two registries, or :latest and :0.10): by ID alone, docker refuses an
# image that has more than one.
mapfile -t tags < <(docker image inspect -f '{{range .RepoTags}}{{println .}}{{end}}' "$image" | grep -v '^$')
# Only the Python service's container: the project's other services (a proxy, as SELFHOST.md
# allows) and its networks stay.
docker compose rm --stop --force bookclub
rm -rf -- "$OLD_DATA"
docker image rm "${tags[@]:-$image}"
echo "done: the Python app, its data and its image are gone (backups: restore test passed just now, newest snapshot $latest)"
CLEANUP
```

`$OLD` itself, with the old `compose.yaml` and `.env`, stays as the record; delete it by hand when it is no longer wanted (its `.env` holds the old app's `SECRET_KEY`, which nothing uses any more). When that project still runs other services, its `compose.yaml` still names `bookclub`, and a later `docker compose up -d` there would create the Python container again: **[Rafael]** removes the `bookclub` service from that file.
