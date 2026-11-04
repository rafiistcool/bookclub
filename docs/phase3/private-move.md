# The private move: runbook (#52)

This runbook moves the two current members from the Python app (v0.10.0, SQLite) to the Rust server (v0.11.1, Postgres) on the **same host, under the same hostname, behind the same reverse proxy**: invite-only, the name stays "Bookclub", mail off, no legal texts, no new external accounts. The design and the decisions are `docs/phase3/work-plan.md` §5 and §6 (answers 30–50); this file is the copy-paste part.

It is built on `docs/phase2/cutover.md` as COVER (#74) left it (the cover search inside `import-legacy`, its report lines, and the review of cover suggestions), and on the operator runbook `deploy/hosted/README.md`. The table below says which steps of `cutover.md` the private move keeps and which it drops. `cutover.md` stays as it is for the public launch.

**Who runs what (2026-11-04).** The host is the **apps VM** (Proxmox VM 104, `192.168.178.12`). The agents VM cannot reach it, by design: its firewall lets it reach only Forgejo in the LAN, and the apps VM takes SSH only from the Proxmox host. That separation stays: many agents run with sudo on the agents VM, and none of them gets root on production. So:

- **[Admin]** runs every block on the host, as root, over SSH from the Proxmox host (`ssh root@192.168.178.12`), and reports back what the next section allows.
- "I" is the agent on the agents VM: I prepare the files and this runbook, read what the admin reports, decide the go/no-go, and run the few steps that need no host (the release check, `verify-limits.sh`).
- **[Rafael]** marks what only he does. He has no account in the old app: the club's owner is Anni (`anni`), and the second account is the other member. On move day Rafael checks the data and runs the phone test **with Anni's account**; she gives him her password herself, never through the admin's or my terminal. After the import he gets **an account of his own** as the instance admin, outside the club (section 7.5).

Every block says where it runs: on the host as root (the admin), on the Proxmox host, or on the agents VM.

**No backups of our own (2026-11-04).** The admin's Proxmox backup of the whole apps VM covers the new stack: its directory, its data directory and Docker's volumes with the database. The move sets up no restic, no `backup.sh`, no timers and no restore test; instead the admin starts a VM backup by hand as move day's first step and after the import (sections 4, 7.0 and 7.6). Postgres in a snapshot backup is like Postgres after a power cut: it replays its WAL on start. The offsite backups come with the public launch (#51).

## What the admin reports back

Counts, statuses and exit codes only: what the blocks below print. No command whose output holds a secret, a password hash, user rows, titles or client addresses is part of a block, and nothing else goes into the report or into chat:

- **[Rafael]** runs `docker login` himself when the registry needs it (section 2.3). No token or password passes through the admin's or my terminal.
- `set-password`, if needed, writes to a root-only file (`umask 077`) that **[Rafael]** reads and deletes (section 10).
- The import reports and `compare.sh`'s output stay on the host, in root-only files. I read `compare.sh`'s exit status and result lines, and the reports through `summary` (section 0): the entity lines, `lookup wait` and `cover search`, and the sections as line counts. `summary` prints a line only when it looks like one of those, and none after the first section title: a book's title with a line break cannot reach my terminal. Never the `isbn keys` or `without a cover` sections, which name books; what I need from them I count on the host with `grep -c`. `other lines` in a summary is a line `summary` does not know: Rafael reads the report on the host.
- The app's log only through `grep '^error:'`: its request lines carry paths. The log of `import-legacy` and `rematch-legacy` goes to a root-only file next to its report, and I read only its `error:` lines and a count of warnings: a throttled or failed lookup logs its ISBN.
- The proxy's address for `TRUSTED_PROXIES` is not a client's. `host-facts.sh` prints the peers of the old app, which behind a proxy is the proxy.

The notes (`docs/phase3/notes/move.md`) get the facts found on the host without secrets or usernames, the counts, and the phone test's result.

## `cutover.md` and the private move

| `cutover.md` | For the private move |
|---|---|
| §1 checklist | **Keep:** the stack on `0.11.1` with `SIGNUP_MODE=invite` and `DNB_HARVEST=false`; WUD touches neither stack (section 2.3); an empty target (no `beta-invite`, no sign-up); `install -d -o 1000 -g 1000 -m 700 "$IMPORT"`; room in the app's `/tmp`. **Drop:** name and domain, mail, legal texts, the uptime monitor, staging, the Caddy blocks, the gate address. `compare.sh` needs `sqlite3`: install it, or use the container stand-in (section 7.4). Section 5 here |
| §2 announcement | Replaced by one short chat message, DE and EN, a few days ahead (section 6) |
| §3.1 stop, snapshot | **Keep** `docker compose stop bookclub` in `$OLD` (stop, never down) and the `-journal` check. **Drop** the maintenance block, the checksums and the gpg copy: the stopped container and its untouched data directory are the rollback copy for 30 days (section 7.1) |
| §3.2 transfer | `cp` of `bookclub.db` (and `-wal` if present) into `$IMPORT`, `chown 1000:1000`, `chmod 600`: the same host, no `scp` (section 7.2) |
| §3.3 dry run, go/no-go, real run | As COVER leaves it; WUD leaves the new stack alone. The go: `users 2 → 2` and the club with 2 members exactly; every other entity line carries its source over or names the difference, with counts at CUT2's (50 works, 36 editions, 146 events) or above, since CUT2 read an older copy and the club kept writing. Each run takes about 20 s plus about a minute of cover search (at most 5 minutes). `lookup wait` may be above 0 from the cover pass, and the `cover search` line and `without a cover` section may differ between the runs; neither is a no-go (section 7.3) |
| §3.4 `compare.sh` | Unchanged; exit 0 is the go (section 7.4) |
| §3.5 start and check | No gate. Start the app on the old port, `healthcheck`, Rafael's check with Anni's account, Rafael's own admin account (`beta-invite`, sign-up, `make-admin`), `rematch-legacy` if needed, `/sw.js`, `verify-limits.sh` from the agents VM, the phone test. No email step. COVER's review of cover suggestions waits until after "done" (section 7.5) |
| §3.6 open | No Caddy step: the port takeover was the switch. One VM backup, then Rafael sends "done" (section 7.6) |
| §4 rollback | Section 8 |
| §5 afterwards | Section 9 |
| §6 preserving Python | Done (Q6 A) |

## 0. Variables

Once the facts are in (section 2), I send the admin the filled-in block, and the admin writes it into `/root/bookclub-move/vars.sh` on the host. It holds no secret. Every SSH session on the host starts with `. /root/bookclub-move/vars.sh`. The values the admin has already named are filled in; `host-facts.sh` confirms them.

```bash
HOST=buchclub.tillhome.de                     # BOOKCLUB_PUBLIC_URL of the old app
OLD=/opt/stacks/buchclub-anni                 # the Python app's compose project (service bookclub)
OLD_DATA=/srv/appdata/buchclub-anni/data      # its mount on /data (host facts, "mounts")
OWNER=anni                      # the club's owner in the old app (Rafael, 2026-11-04); Rafael has no account there
CLUB_NAME='Bookclub'            # BOOKCLUB_NAME of the old app (host facts, 2026-11-04)
CLUB_TZ=Europe/Berlin           # the old app's BOOKCLUB_TZ is CET (host facts); see below
SLUG=bookclub-8rqp              # CUT2's slug, since CLUB_NAME is "Bookclub" (section 2.3)
NEW=/opt/bookclub               # the new stack: free, the old app lives under /opt/stacks
DATA=/srv/bookclub              # its data directory: free, the old app's is under /srv/appdata
IMPORT=$DATA/import             # the legacy copy: mode 700, uid 1000
R=/root/bookclub-move           # deploy/ of the release, from the agents VM's checkout (section 2.1)

# The report without what names people or books. Before the first section it prints only the
# lines it knows (entity lines, normalised, lookup wait, cover search, a dry run's header); from
# the first section title on it prints each section as its title and a line count. A section's
# lines are never printed, also one that a title with a line break starts without indentation.
# Any other line is counted as "other lines".
summary() {
    awk 'BEGIN { split("merges:|differences:|isbn keys:|without a cover:|notes:", t, "|")
                 for (i in t) known[t[i]] = 1 }
         $0 in known { if (title != "") printf "%s %d lines\n", title, n; title = $0; n = 0; next }
         title != "" { n++; next }
         /^[a-z_ ]+ (-|[0-9]+) → / || /^(normalised|lookup wait|cover search) / ||
             /^DRY RUN: / || /^The cover search looked at / { print; next }
         { other++ }
         END { if (title != "") printf "%s %d lines\n", title, n
               if (other) printf "other lines %d\n", other }' "$@"
}
```

`CLUB_TZ` sets the club's and both imported accounts' time zone (`docs/phase1/legacy-mapping.md`). The old app's value is `CET`. The server accepts it, but stores the zone it links to in tzdb 2026e, `Europe/Brussels` (`server/src/tz.rs`, `LINKS`), and that name would show in the club's settings. `Europe/Berlin` has the same offsets and the same summer time as `CET` for every date the club has, so the times stay as they were, under the name the members expect. Both can be changed in the app afterwards (club settings, Settings).

## 1. Before: the release (M2)

Wave 1 (SCAN, FRIENDS, M1) is merged into `phase-3`, and M1's boot test (`work-plan.md` §2.3) passed once more on it. **[Rafael]** approved the merge of the PR `phase-3` → `main`; the tag `v0.11.0` on `main` follows without a second approval. Once the release workflow has run, the three tags must name one digest. The package is private and the agents VM has no registry login (a token would pass through my terminal), so I read Forgejo's package API with the `forgejo` MCP tools instead of `docker buildx imagetools inspect` (`docs/phase2/notes/dep2.md`, "Not verified"): `list_package_files` with owner `rafiistcool`, type `container`, name `bookclub-server`, for the versions `0.11.0`, `0.11` and `stable`. Each lists a `manifest.json` whose `sha256` is that tag's manifest digest.

Three equal `sha256` values are the go.

**Done (2026-11-04):** `v0.11.0` (2fc06ba) was released on 2026-11-01. `v0.11.1` (fd5b27d, PR #76) followed: with `EMAIL_TRANSPORT=none` it starts without `CONTACT_EMAIL`, which section 3's `.env` comments out, so the move needs v0.11.1 or later. `0.11.1`, `0.11` and `stable` share the manifest digest `sha256:684b3a0ae3f2…`. The host pins `BOOKCLUB_TAG=0.11.1`: the registry has `0.11.1`, `0.11`, `0.11.0` and `stable`, but no `v0.11.1` and no `latest`. Nothing on the host updates by itself: the new stack is kept away from WUD, and updates run by hand (section 9).

## 2. Files and facts (M3)

### 2.1 The files on the host

No key and no firewall opening. The admin carries `deploy/` of the release from the agents VM's checkout to the host, through the Proxmox host. Forgejo answers only signed-in users, so the host cannot fetch it from there itself.

```bash
# On the Proxmox host, with the admin's way into the agents VM as <agents>:
ssh <agents> 'git -C /home/rafi/Projects/bookclub archive --format=tar v0.11.1 deploy' |
    ssh root@192.168.178.12 'install -d -m 700 /root/bookclub-move && tar -C /root/bookclub-move -xf - && ls /root/bookclub-move/deploy'
#   cutover  hosted  legacy-origin
```

`git archive` reads the tag, not the working tree, so the files are the release's whatever the checkout holds.

### 2.2 The host facts

Already known from the admin (2026-11-04, read only):

- Docker 29.9.3 and compose v5.7.0; 2 vCPUs, 3 GB RAM (about 2.2 GB free), 16 of 20 GB disk free.
- The old app: `$OLD` (`compose.yaml` → `Buchclub-Anni.yml`, `.env` → `Buchclub-Anni.env`, and `compose.override.yml` with the bind mount `$OLD_DATA` → `/data`), container `buchclub-anni-bookclub-1`, image `git.tillhome.de/rafiistcool/bookclub:latest`, healthy, `BOOKCLUB_TRUSTED_PROXIES=*` (the default). WUD's auto-deploy labels are on it.
- Port 8000 is published on `0.0.0.0` (`"8000:8000"`), and the firewall `104.fw` lets only `edge` reach it. Also taken: 8081 and 8083 (`0.0.0.0`), 8084 (`192.168.178.12`).
- Caddy on `edge` (`192.168.178.10`) runs in a Docker network there, but its connections to apps come from `192.168.178.10`. Its site block is only `reverse_proxy 192.168.178.12:8000`, with no headers: no `Permissions-Policy`, so the camera is not blocked.
- Root's Docker on apps is logged in to `git.tillhome.de` (no `credsStore`) and can pull the private `bookclub-server`; anonymous pulls get 401.
- The VM backup: section 4.

`host-facts.sh` confirms them and adds what is still open: the old settings (name and time zone), `sqlite3`, and the size of the data files.

**Ran on 2026-11-04, exit 0, no stop:** Debian 13, x86_64, systemd 257; no compose project named `bookclub`; port 8000 published on `0.0.0.0`; the only peer `192.168.178.10` (requests through Caddy on `edge`); `BOOKCLUB_NAME=Bookclub`, `BOOKCLUB_TZ=CET`; root's Docker config names `git.tillhome.de`; `sqlite3` 3.46.1 installed; `bookclub.db` 0.4 MB and its `-wal` 4.1 MB, far below the app's 64 MB `/tmp`; `/opt/bookclub` and `/srv/bookclub` absent; 16 GB free on `/`. `https://$HOST/` answered `405` to the script's `HEAD`: the Python app refuses `HEAD`, and `GET` gets 200. WUD (`nas-monitoring-wud-1`) watches only running containers (`WUD_WATCHER_LOCAL_WATCHALL` unset), and its trigger `eigeneapps` acts only on labelled containers (`INCLUDEBYDEFAULT=false`), so it does not start the stopped Python app. Run it while someone uses the app (its peers sample takes 10 s; `PEER_SECONDS` changes that):

```bash
# On the host as root:
/root/bookclub-move/deploy/cutover/host-facts.sh /opt/stacks/buchclub-anni buchclub.tillhome.de
echo "exit $?"
```

It is read-only and prints no secret (its header lists what it reads), so the admin sends me its whole output. Exit 1 means a check stops the move: the `STOP:` lines say which (`uname -m` not `x86_64`, Docker older than 25 or compose older than 2.24, no answer over HTTPS). The move waits until they are fixed. Its notes about a backup repository (`/srv/bookclub-backup`, mounted shares) do not apply any more. Its peers sample shows Docker's view of `192.168.178.10`, which is the value below.

The facts go into the notes without secrets or usernames.

### 2.3 What the facts decide

- **Layout.** `NEW=/opt/bookclub`, `DATA=/srv/bookclub`: both free, and neither inside `$OLD_DATA`, which M6 deletes whole (section 11). If the facts' note says otherwise, stop and tell me.
- **Project name.** The old project is `buchclub-anni`, so the new one keeps `compose.yaml`'s `name: bookclub`, and no `COMPOSE_PROJECT_NAME` is needed. If `docker compose ls` shows a project named `bookclub` after all, the new `.env` sets `COMPOSE_PROJECT_NAME=bookclub-server`: otherwise the stopped Python container becomes an orphan of the new project, and a `down` or `--remove-orphans` could remove it.
- **The binding.** The proxy runs on another host (`edge`) and dials `192.168.178.12:8000`. The new app takes over the old container's binding and port: `BOOKCLUB_LISTEN=0.0.0.0`, `BOOKCLUB_PORT=8000`; the firewall keeps limiting it to `edge`. Never `127.0.0.1`: `edge` would get "connection refused", and the members a 502.
- **`TRUSTED_PROXIES=192.168.178.10/32`**, the address `edge` connects from. If the facts' "peers" show another address, stop and tell me. A wrong value puts everyone in the proxy's rate-limit bucket: one stranger's failed attempts lock both members out of sign-in for 10 minutes and out of reset and sign-up for an hour. A wide one lets LAN clients forge `X-Forwarded-For`. Move day checks it from two sides (section 7.5).
- **Name, time zone, slug.** `CLUB_NAME` and `CLUB_TZ` from the facts' "old settings" (the `.env` value, else the container's). `SLUG=bookclub-8rqp` (CUT2's) when the name is "Bookclub". Otherwise I make a new slug by D5, once, before the message goes out (on the agents VM; the importer checks the result):

  ```bash
  base=$(printf '%s' "$CLUB_NAME" | sed 's/[Ää]/ae/g; s/[Öö]/oe/g; s/[Üü]/ue/g; s/ß/ss/g' |
      iconv -f utf-8 -t ascii//TRANSLIT | tr '[:upper:]' '[:lower:]' | tr -cs 'a-z0-9' '-' |
      sed 's/^-*//; s/-*$//' | cut -c 1-50 | sed 's/-*$//')
  echo "${base:-club}-$(LC_ALL=C tr -dc 'a-z0-9' </dev/urandom | head -c 4)"
  ```

- **WUD.** It auto-deploys new images. The new stack carries `wud.watch: "false"` on `app` and `postgres` from its first start (section 3.1), so WUD never replaces the app behind the move's back; updates run by hand (section 9). No new Python image is published any more, so WUD has nothing to roll out to the stopped Python container. WUD watches only running containers and does not start stopped ones (2.2), so the Python app stays stopped during the move.
- **Registry.** Root's login pulls `bookclub-server` (2.2). Should `docker compose pull` in section 3.2 still fail with `denied` or `unauthorized`, **[Rafael]** creates a Forgejo token with only `read:package` (Settings → Applications) and runs `docker login git.tillhome.de` with it as root on the host.
- **The scanner.** Caddy sends no `Permissions-Policy` for the site, so the camera works. Should one be added later, it says `camera=(self)`.
- **`sqlite3`.** Missing: install it for `compare.sh` (`apt-get install -y sqlite3`), or use the stand-in of section 7.4.
- **Memory.** About 2.2 GB free is enough for Postgres and the app next to the stopped Python app.

## 3. Install the stack next to the Python app (M3)

### 3.1 Files and settings

On the host as root (`deploy/hosted/README.md`, "First boot" steps 1–4, without step 5's test mail):

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
set_env TRUSTED_PROXIES 192.168.178.10/32
set_env EMAIL_TRANSPORT none
sed -i -e 's|^SMTP_URL=|#SMTP_URL=|' -e 's|^EMAIL_FROM=|#EMAIL_FROM=|' -e 's|^CONTACT_EMAIL=|#CONTACT_EMAIL=|' .env
set_env BOOKCLUB_LISTEN 0.0.0.0
set_env BOOKCLUB_PORT 8000
set_env BOOKCLUB_TAG 0.11.1             # pinned: updates run by hand (section 9)
set_env BOOKCLUB_DATA_DIR "$DATA"
cat > compose.override.yaml <<'YAML'
# The private move (#52): WUD never updates this stack; updates run by hand (private-move.md §9).
services:
  app:
    labels:
      wud.watch: "false"
  postgres:
    labels:
      wud.watch: "false"
YAML
docker compose config -q && echo "config ok"
grep -c '^[^#].*CHANGE_ME' .env                                     # 0
grep -vE '^(#|$|POSTGRES_PASSWORD=|DATABASE_URL=)' .env             # the settings, without the password
install -d -o 1000 -g 1000 -m 0750 "$DATA/media" "$DATA/legal"
install -d -o 1000 -g 1000 -m 0700 "$IMPORT"
```

The rest keeps `env.example`'s values: `SIGNUP_MODE=invite`, `LEGAL_DIR=/app/legal` (the empty `$DATA/legal`: no texts, no terms prompt, `/imprint` shows "missing", the footer links stay hidden), `DNB_HARVEST=false`, `COOKIE_SECURE` and `SERVICE_NAME` at their defaults (`work-plan.md` §5.1). With `EMAIL_TRANSPORT=none` the server needs neither `SMTP_URL`, `EMAIL_FROM` nor `CONTACT_EMAIL`, refuses `SIGNUP_MODE=open`, sends nothing and voids reset links; password resets go through `set-password` (section 10).

Compose loads `compose.override.yaml` by itself, so every `docker compose` command in `$NEW` carries the WUD labels.

### 3.2 Only Postgres, and one check of the app

Before move day only `postgres` runs. The app starts once on a spare loopback port, so port 8000 stays the Python app's, and stops again:

```bash
cd /opt/bookclub
docker compose pull
docker compose up -d --wait postgres
BOOKCLUB_LISTEN=127.0.0.1 BOOKCLUB_PORT=18080 docker compose up -d --wait --no-deps app
docker compose exec app bookclub-server healthcheck        # … answered 200 OK
curl -sS http://127.0.0.1:18080/api/config; echo           # "signup":"invite", no terms version
docker compose exec app bookclub-server send-test-mail check@example.com; echo "exit $?"
#   error: mail is off (EMAIL_TRANSPORT=none) / exit 1
docker compose stop app
docker compose ps -a
docker inspect -f '{{index .Config.Labels "wud.watch"}}' $(docker compose ps -aq)    # false, twice
```

If it does not come up: `docker compose logs --no-log-prefix app | grep '^error:' | tail -n 1` names the setting (README, "When it does not start").

The importer takes a migrated empty database, and this start migrated it. The target must hold no users and no invites: **no `beta-invite`, no sign-up** until the import.

## 4. The VM backup (M3)

No backups of our own (see the top). The admin's Proxmox backup of VM 104 is the way back (2026-11-04):

- vzdump, no PBS: the job `backup-vm100-taeglich`, every day at 03:00, mode `snapshot` with fleecing (`local-lvm`), zstd, to the storage `tank-backup`; kept 7 days, 4 weeks and 3 months; `repeat-missed` on. The backup of 2026-11-04 03:03 was 1.5 GB.
- By hand, on the Proxmox host (section 7.0 and 7.6):

  ```bash
  vzdump 104 --storage tank-backup --mode snapshot --compress zstd; echo "exit $?"
  ```

- **A restore of VM 104 in place takes everything on it back to the backup**, also the apps on 8081, 8083 and 8084. To bring back Bookclub alone, the admin restores the backup as a new VM (a free VM ID, its network off), and copies `/opt/bookclub`, `/srv/bookclub` and the Postgres volume (`bookclub_postgres-data`) out of it into the stopped stack on apps.

No heartbeat: a failed nightly backup shows in Proxmox only. That is accepted until the public launch brings a monitor. There is no update timer either: WUD leaves the stack alone (section 2.3), and updates run by hand after a VM backup (section 9).

## 5. The checklist before move day

Checked the day before: the admin runs the commands on the host and reports, I tick.

- [ ] The facts' stops are cleared; binding, `TRUSTED_PROXIES`, slug and time zone are in `vars.sh` and `.env`.
- [ ] `docker compose ps -a` in `$NEW`: `postgres` healthy, `app` exited; `BOOKCLUB_TAG=0.11.1`, `SIGNUP_MODE=invite`, `DNB_HARVEST=false`, `EMAIL_TRANSPORT=none`.
- [ ] The target is empty, in `$NEW`: `docker compose exec -T postgres psql -U bookclub -d bookclub -Atc 'SELECT (SELECT count(*) FROM users), (SELECT count(*) FROM club_invites)'` prints `0|0`.
- [ ] The nightly VM backup of VM 104 passed (section 4), and the admin knows how Bookclub alone comes back from it.
- [ ] WUD: `wud.watch` is `false` on both new containers (section 3.2), and WUD does not start stopped containers (section 2.3).
- [ ] `$IMPORT` exists, mode 700, owner 1000.
- [ ] Room in the app's `/tmp` (a 64 MB tmpfs) for `bookclub.db` and its `-wal`: the facts' "data files" sizes together are well below 64 MB; otherwise set `TMPDIR` as README "The legacy import" says.
- [ ] `sqlite3` is installed (3.46.1 on 2026-11-04).
- [ ] The message went out (section 6).

## 6. The message

**[Rafael]** picks the time and sends it a few days ahead to both members, Anni and the other member. Anni also hears from him in person that he signs in once with her account on move day to check it, and gives him her password for that.

**Deutsch**

> Bookclub bekommt am \<Tag\> ab \<Uhrzeit\> ein großes Update, etwa eine Stunde Pause. Adresse, Benutzername und Passwort bleiben gleich, alles zieht mit. Danach einmal neu anmelden. Nicht abgeschickte Notizen bitte vorher abschicken. Die App fragt nach einer E-Mail-Adresse: eintragen oder überspringen, Mails verschickt sie vorerst keine. Passwort vergessen? Sag mir Bescheid. Neue Leute laden ab jetzt nur Club-Admins ein: Anni, und wen sie dazu macht. Ich melde mich, wenn es fertig ist.

**English**

> Bookclub gets a big update on \<day\> from \<time\>, with a break of about an hour. The address, your username and your password stay the same, and everything moves with you. Afterwards, sign in once more. Please send any unsent notes before then. The app asks for an email address: add one or skip it, it sends no mail for now. Forgot your password? Tell me. From now on only club admins invite new people: Anni, and whoever she makes one. I'll let you know when it's done.

**Done** (section 7.6):

> Fertig, Bookclub läuft wieder unter der gewohnten Adresse. / Done, Bookclub is back at the usual address.

## 7. Move day (M4)

Every block on the host as root (the admin), after `. /root/bookclub-move/vars.sh`, unless it says otherwise. The admin sends me each block's output, and I give the go for the next. About an hour in all.

### 7.0 A VM backup first

On the Proxmox host, while the old app still runs, the admin starts a backup of VM 104 (section 4) and reports its exit status. Exit 0 is the go for 7.1. The admin may also keep a copy of the old `bookclub.db` of his own; it holds every password hash, so it stays root-only on the Proxmox side and goes when M6 runs (section 11).

```bash
vzdump 104 --storage tank-backup --mode snapshot --compress zstd; echo "exit $?"
```

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

In `$NEW`. The app is stopped (section 3.2), so nobody signs up while the import runs, and WUD leaves it alone. `-T` keeps the report free of terminal line endings; `umask 077` keeps the report and the log root's. The log stays in `dry.log`: a throttled or failed lookup logs its ISBN.

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

No gate: both members wait for "done". The Python container is stopped, so its port is free.

```bash
cd "$NEW"
docker compose up -d --wait app                          # on the old binding and port now
docker compose exec app bookclub-server healthcheck      # … answered 200 OK
curl -s -o /dev/null -w '%{http_code}\n' "https://$HOST/api/health"     # 200: the proxy reaches the new app
```

A 502 or 504 means `edge` does not reach the new app: the binding does not fit the proxy's upstream. Back to the old binding in `.env` (section 2.3) and `docker compose up -d --wait app`. Rafael signs in only after a 200.

**[Rafael]**, in his browser on `https://$HOST`: sign in as `anni` with her old password (everyone signs in again once: the cookie is new). He skips the email prompt, so Anni enters her address herself later. Then the club, the shelves, the physical shelf layouts, the pick and its notes, the votes, the quotes and the activity feed are there, and an old `/works/OL…W` link from a chat opens. He changes nothing.

If the old password does not work: section 10 for `anni` (Anni then gets the new one and changes it in Settings), then the check.

**Rafael's own account**, the instance admin, outside the club. The invite goes to a root-only file, as `set-password`'s password does (section 10):

```bash
cd "$NEW"
(umask 077 && docker compose exec -T app bookclub-server beta-invite > /root/bookclub-invite); echo "exit $?"
```

**[Rafael]** reads it from his own session (`cat /root/bookclub-invite && rm /root/bookclub-invite`), opens the join link, and signs up with a username of his choice. He tells the admin the name, and the admin runs:

```bash
cd "$NEW"
docker compose exec app bookclub-server make-admin '<his username>'
```

Rafael's account joins no club: he sees the admin area (covers, reports, invites, accounts), not the club's shelves.

If `isbn unmatched` or `isbn error` in the report's `books` line is not 0, rematch now (it commits per edition and per step of each book's cover search, and is safe on the running app):

```bash
cd "$NEW"
(umask 077; docker compose exec -T app bookclub-server rematch-legacy --dry-run 2> rematch-dry.log | summary)
grep '^error:' rematch-dry.log; grep -c ' WARN ' rematch-dry.log
(umask 077; docker compose exec -T app bookclub-server rematch-legacy 2> rematch.log | summary)
grep '^error:' rematch.log; grep -c ' WARN ' rematch.log
```

COVER's review of cover suggestions (Admin → Covers, with Rafael's own account) waits until after "done": a missing cover is no no-go, and Rafael decides them later (answer 45).

The old worker's kill switch, from anywhere:

```bash
curl -sI "https://$HOST/sw.js" | grep -iE '^(HTTP|content-type)'    # 200, content-type: text/javascript
```

**`TRUSTED_PROXIES`, from two sides** (answer 49). First the peers the new app sees, while Rafael's browser is open (README, "The VM's network", check 3): only the proxy's address, the one in `.env`.

```bash
# The app listens on 8080 in its container.
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

It must get its 429s (exit 0). It creates nothing and uses up only the budgets of the address the proxy sees for the agents VM. The agents VM reaches `https://$HOST` over the router's public address, so `edge` may see the router, as for every device at home: then signing in from home answers 429 for up to 10 minutes, and Rafael waits that long. Then **[Rafael]** signs in as `anni` on his phone **with Wi-Fi off** (mobile data). A 429 on the phone means `TRUSTED_PROXIES` or the proxy's `X-Forwarded-For` is wrong: the admin fixes `.env`, runs `docker compose up -d --wait app`, and he tries again.

Then **the phone test** (`work-plan.md` §5.8): **[Rafael]**, signed in as `anni`, on his iPhone with Wi-Fi off (and an Android phone with Chrome if one is at hand): open `https://$HOST`, sign in, scan a German book, an English book and one the app does not know, add a photo to the unknown one, and deny the camera once to see manual entry. An iPhone older than iOS 16 shows manual entry only. The result goes into the notes; a failure becomes a patch release, never a rollback.

### 7.6 Open

No proxy step: the port takeover was the switch. The admin starts one backup of VM 104 by hand on the Proxmox host (section 4) and reports its exit status: from now on a backup holds the imported data.

**[Rafael]** signs out of Anni's account in his browser and on his phone (Settings → Sign out). Then the admin notes the moment, which section 8's first step counts from:

```bash
echo "DONE_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)" >> /root/bookclub-move/vars.sh
```

Then **[Rafael]** sends "done" to both members (section 6). The rollback window closes when a member uses the new app; section 8's first step tells whether one has.

## 8. Rollback

The window lasts until a member uses the new app after "done". Until then the old app's data is untouched (stopped, and the importer read a copy), and the only writes on the new app are the import, Rafael's checks with Anni's account and his own admin account: a rollback discards them, and the next attempt imports again from the old data.

Each step is one block on the host as root (the admin) that loads `vars.sh` and changes into its own directory, so it runs as written in a fresh SSH session. A block whose variables are missing stops before it touches anything (`${NEW:?}`, or `set -u` in step 4's `bash -s` block).

1. **Is the window still open?** Counts only: the club's members, Anni included, that used the new app after `DONE_AT` (section 7.6), by a live session or by one that ended (`last_seen_at`). Every write needs a sign-in. Rafael's checks with Anni's account came before `DONE_AT`, and his own account is in no club, so neither counts. Before "done", `DONE_AT` is unset and the count is 0.

   ```bash
   . /root/bookclub-move/vars.sh
   cd "${NEW:?}" && docker compose exec -T postgres psql -U bookclub -d bookclub -At -v done="${DONE_AT:-infinity}" <<'SQL'
   SELECT count(*) FROM users u
   WHERE EXISTS (SELECT 1 FROM club_memberships m WHERE m.user_id = u.id)
     AND (u.last_seen_at > :'done'
          OR EXISTS (SELECT 1 FROM sessions s WHERE s.user_id = u.id AND s.last_used_at > :'done'));
   SQL
   ```

   `0`: on to step 2. Anything else: a member has used the new app, and what they wrote exists only there. Then no rollback without **[Rafael]**: he asks them. If they wrote nothing, the rollback goes on; otherwise problems are fixed forward.
2. **The new app stops.**

   ```bash
   . /root/bookclub-move/vars.sh
   cd "${NEW:?}" && docker compose stop app
   ```

3. **The old app starts.** Run the block again until `ps` says healthy (a second `start` changes nothing), then sign in once.

   ```bash
   . /root/bookclub-move/vars.sh
   cd "${OLD:?}" && docker compose start bookclub && docker compose ps bookclub
   ```

4. **The new stack is emptied**, so the next attempt starts from an empty target. What it holds came from the copy and Rafael's checks, and the VM backup of section 7.6, if it ran, still has it. The block deletes nothing unless `$NEW` holds the new stack (its `compose.yaml` names `bookclub-server`) and its `.env` keeps the data in `$DATA`:

   ```bash
   bash -s <<'ROLLBACK'
   set -euo pipefail
   . /root/bookclub-move/vars.sh
   cd "$NEW"
   if [[ ! -f compose.yaml || ! -f .env ]] || ! grep -q '/bookclub-server:' compose.yaml ||
       ! grep -qx "BOOKCLUB_DATA_DIR=$DATA" .env; then
       echo "stop: $NEW is not the new stack, or its BOOKCLUB_DATA_DIR is not $DATA; nothing was deleted"
       exit 1
   fi
   docker compose down -v
   find "$DATA/media" -mindepth 1 -delete
   echo "done: the new stack is empty"
   ROLLBACK
   ```

   Before the next attempt, section 3.2 again (`up -d --wait postgres`, the check on 18080).
5. Rafael tells both members the update is postponed, and I record why in #52 (counts only).
6. **The leftovers go**, once the reason is in #52 and Rafael has read on the host what he needs: the import copy holds every password hash, the reports and logs name people and books.

   ```bash
   . /root/bookclub-move/vars.sh
   rm -f "${IMPORT:?}"/bookclub.db* "${NEW:?}"/{dry,real,compare}.txt "$NEW"/{dry,real,rematch-dry,rematch}.log
   ```

Python's `restart: unless-stopped` keeps a stopped container stopped after a reboot. Starting it while the new app holds the port fails with "port already allocated", which is safe. Still, nothing runs `docker compose up` or `start` in `$OLD` outside the rollback.

After the window there is no rollback: the members' writes exist only on the new app. Problems are fixed forward; the nightly VM backup covers the new data.

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

**Updates, by hand.** No timer and no WUD: a new version applies its migrations when it starts, and migrations only go forward, so the way back is the VM backup from just before. When I report a release `X.Y.Z`, the admin starts a backup of VM 104 by hand (section 4) and, once it passed, runs on the host:

```bash
cd /opt/bookclub
sed -i 's/^BOOKCLUB_TAG=.*/BOOKCLUB_TAG=X.Y.Z/' .env
docker compose pull app
docker compose up -d --wait --no-deps app
docker compose ps app                                   # healthy
docker compose images app                               # X.Y.Z
```

A failed start: `docker compose logs --no-log-prefix app | grep '^error:' | tail -n 1`, then the previous version in `BOOKCLUB_TAG` and the `up` line again. A new version that already ran its migrations does not go back that way: then Bookclub alone comes back from the VM backup (section 4). Whether WUD takes this over later is the admin's call, with the public launch at the latest.

**When it suits Rafael:** Admin → Covers: the suggestions ("Use this cover" or "Not this book"), and the books readers use that still have none (`cutover.md` §5, "The day after").

**CUT-R** (`docs/phase2/work-plan.md` §2.4):

- The branch `cutover/remove-python`, cut from `main` after v0.11.1. The deletes are exactly §2.4's.
- Docs: the README describes the private invite-only instance and drops `README.md:162` (that closes #35's README criterion); `SELFHOST.md` goes; CLAUDE.md loses "In transition" and the Python commands; `docs/plan.md` gets a status line with the move's date.
- **[Rafael]** approves the merge after the move is verified, once Forgejo CI is green.

**Issues:**

- Close #52 with the counts, and #56 after the phone test.
- Close #35: the password change is in Settings (`AccountSettings.vue`, `PATCH /api/auth/password`) and runs in production from the move on.
- Close #34 as superseded, after adding "Web Push for the browser app" to #60 (D9).
- #51 stays open for the public launch, with a comment on what the move delivered.

**Last step of M5**, on the host as root (the admin): the move's files go.

```bash
docker image rm bookclub-sqlite3 2>/dev/null
rm -rf /root/bookclub-move
```

## 10. A forgotten password

Mail is off, so "Forgot password" sends nothing (it still says a link is on its way). The member asks Rafael, and the admin runs, on the host as root:

```bash
cd /opt/bookclub
(umask 077 && docker compose exec -T app bookclub-server set-password <username> > /root/new-password); echo "exit $?"
```

The file holds a new random password of 20 characters (capital letters and digits) and nothing else. Every session and app token of the account ends, and its open links are voided; an unknown name exits 1 with `No account with that name`. **[Rafael]** reads it from his own session (`cat /root/new-password && rm /root/new-password`), passes it on, and the member changes it in Settings the same day. The app keeps failed sign-ins in memory, so a member it locked out still waits up to 10 minutes.

## 11. After 30 days (M6): the cleanup block

**[Admin]** first checks in Proxmox that the nightly backups of VM 104 ran since the move without a failure, and that the newest one is from last night: nothing on the host can see them. The admin's own copy of the old `bookclub.db` (section 7.0) goes now too. Then the admin pastes this on the host as root. It deletes only when the old data directory holds `bookclub.db` and none of the new stack's paths (`$NEW`, its data directory, Docker's root) and the new app answers its healthcheck. Otherwise it stops and deletes nothing.

```bash
bash -s <<'CLEANUP'
set -euo pipefail
NEW=<NEW>                 # /opt/bookclub
OLD=<OLD>                 # /opt/stacks/buchclub-anni
OLD_DATA=<OLD_DATA>       # /srv/appdata/buchclub-anni/data, with bookclub.db
cd "$NEW"
[[ -f .env ]] || { echo "stop: $NEW holds no .env; nothing was deleted"; exit 1; }
[[ -f $OLD_DATA/bookclub.db ]] || { echo "stop: $OLD_DATA holds no bookclub.db; nothing was deleted"; exit 1; }
# The rm below takes OLD_DATA whole: it must hold nothing of the new stack (its directory, its
# data directory with the pictures) or of Docker's volumes (the database).
old=$(realpath -e -- "$OLD_DATA")
data=$(sed -n 's/^BOOKCLUB_DATA_DIR=//p' .env | tail -n 1)
# compose.yaml's default when .env does not name one.
keep=$(printf '%s\n' "$NEW" "${data:-/srv/bookclub}")
keep+=$'\n'$(docker info -f '{{.DockerRootDir}}')
while IFS= read -r path; do
    real=$(realpath -m -- "$path")
    if [[ $real == "$old" || $real == "${old%/}"/* ]]; then
        echo "stop: $OLD_DATA holds $path; nothing was deleted"
        exit 1
    fi
done <<<"$keep"
if ! docker compose exec -T app bookclub-server healthcheck >/dev/null; then
    echo "stop: the new app does not answer (docker compose ps); nothing was deleted"
    exit 1
fi
cd "$OLD"
container=$(docker compose ps -aq bookclub)
image=$(docker inspect -f '{{.Image}}' "$container")
# Every tag of the image (two registries, or :latest and :0.10): by ID alone, docker refuses an
# image that has more than one.
mapfile -t tags < <(docker image inspect -f '{{range .RepoTags}}{{println .}}{{end}}' "$image" | grep -v '^$')
# Only the Python service's container: the project's other services and its networks stay.
docker compose rm --stop --force bookclub
rm -rf -- "$OLD_DATA"
docker image rm "${tags[@]:-$image}"
echo "done: the Python app, its data and its image are gone"
CLEANUP
```

`$OLD` itself, with the old `compose.yaml` and `.env`, stays as the record; delete it by hand when it is no longer wanted (its `.env` holds the old app's `SECRET_KEY`, which nothing uses any more). When that project still runs other services, its `compose.yaml` still names `bookclub`, and a later `docker compose up -d` there would create the Python container again: the admin removes the `bookclub` service from that file. The stack may also be listed in a stack manager that reads `/opt/stacks`; the admin removes it there too.
