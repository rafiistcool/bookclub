# DEP2 (#51): automatic deploys and real values

Owner: DEP2. These notes record what DEP2 built for Rafael's answers of 2026-10-29 (work plan §4: Q8 = A, Q7 = A, backups option A), the check of the first release run, what was verified and how, and what the integrator moves into `CLAUDE.md`, `docs/phase2/cutover.md` and `docs/phase1/security-review.md`.

No routes, DTOs, `error::msg` strings, migrations, server code, server variables or frontend files changed. `server/.sqlx/` and `frontend/src/api/generated.ts` are unchanged (`cargo sqlx prepare` wrote no difference).

## Files

| File | What |
|---|---|
| `deploy/hosted/update.sh` | **New.** The pull deploy (Q8 = A): follow the tag, back up, update, wait for healthy, ping. |
| `deploy/hosted/systemd/bookclub-update.{service,timer}` | **New.** The update check every 3 minutes. |
| `deploy/hosted/backup-lib.sh` | `load_config` reads `UPDATE_HEARTBEAT_URL` and `UPDATE_WAIT_SECONDS`; new `app_container` (see "A fix in `backup.sh`"). |
| `deploy/hosted/backup.sh` | `check_media_mount` uses `app_container`. |
| `deploy/hosted/backup.env.example` | A section "Automatic updates (update.sh)"; the header names `update.sh`. |
| `deploy/hosted/env.example` | `BOOKCLUB_TAG`, `BOOKCLUB_LISTEN` and `TRUSTED_PROXIES` comments for Q7 and the timer. Values unchanged (`TRUSTED_PROXIES=CHANGE_ME` until the VM exists). |
| `deploy/hosted/README.md` | "Updates and rollback" rewritten (setting up, by hand, rollback with the timer); new "The VM's network" (Q7); new "Storage Box snapshots" and "Restoring from a Storage Box snapshot"; the import and the restore stop and start the timer; "Still to come" removed. |
| `server/tests/deploy_env.rs` | 7 new tests (23 in all, 16 before), one constant removed (below). |

## `update.sh` (Q8 = A)

Every run, from `bookclub-update.timer` or by hand, in `COMPOSE_DIR`:

1. **Guards.** One run at a time (`flock -n` on `BACKUP_DIR/.update.lock`; a second run logs `another update.sh is running` and exits 0). The app container is the app service's container without the one-off containers of `docker compose run` (`app_container`). With no app container (the stack is down, as in a restore) or an app that is not `running` or `restarting` (stopped by hand, as for the legacy import), it logs why and exits 0, without pulling, starting anything or pinging. **It never starts an app that is not running.**
2. **`docker compose pull --quiet app`.** The image is the app's line of `docker compose config --images app` that contains `/bookclub-server:`, as in `smoke-test.sh` (compose prints the dependencies' images too). While the tag has not moved, the pull is a manifest request and downloads nothing. Compose prints progress even with `--quiet`, so the output goes to the journal only on a failure. A failed pull logs `error: cannot pull <image>; nothing was changed` and exits 1 **without a ping**: a short registry outage (a Forgejo restart) alerts only when it outlasts the heartbeat check's grace time.
3. **The decision:** the ID of the image the tag names after the pull (`docker image inspect -f '{{.Id}}'`) against the image the app container runs (`docker inspect -f '{{.Image}}'`). Comparing with what runs, not with a remembered digest, means that a new pin in `.env` is an update like a release, and that a failed update is not forgotten.
4. **Nothing to do** (same image): `running healthy` pings `<url>/0` and logs `<image> is up to date (<repo>@sha256:…), and the app is healthy`; `running starting` exits 0 without a ping (the next run decides); anything else (`running unhealthy`, `restarting …`) logs `error: the app runs <image> (…), but is <status>`, pings `<url>/1` with that line, and exits 1. A failed update therefore keeps the check down until it is fixed, and a crash-looping app is caught although Docker resets its health to `starting` on each restart.
5. **The update**, logged to the journal and to `BACKUP_DIR/update.log` (mode 0600):
   1. `<url>/start`;
   2. `backup.sh` (its own lock, heartbeat and log). A failure stops the update with `the backup failed, so the app was not updated`; the app keeps running the old image, and the next run tries again;
   3. `docker compose up -d --no-deps --wait --wait-timeout $UPDATE_WAIT_SECONDS app`: only the app is recreated (`--no-deps`, so postgres is never recreated by the timer). On failure: `the app did not turn healthy on <image> (docker compose logs app); the backup from before the update is the latest snapshot`;
   4. a check that the container runs the new image and is `running healthy`;
   5. the image the app ran before is removed when no tag names it any more (`docker image inspect -f '{{join .RepoTags " "}}'` is empty); a pinned or older tag that still names it keeps it;
   6. `<url>/<exit status>` with the last 50 lines of the log.

**Exit status:** 0 when nothing was to do, the app is starting, the run was skipped, or the update succeeded; 1 on a failed update, an unhealthy app, or an unreachable registry; 2 on a usage error.

### Decisions

- **Settings in `backup.env`, not a new `update.env`.** `update.sh` runs `backup.sh`, which reads `backup.env`; both must name the same `COMPOSE_DIR`, and a second file would let the two drift apart. `update.sh` reuses `load_config`, so it also refuses to run without a working backup setup (`RESTIC_REPOSITORY`, a readable `RESTIC_PASSWORD_FILE`), which every update needs anyway. New settings: `UPDATE_HEARTBEAT_URL` (empty: no ping) and `UPDATE_WAIT_SECONDS` (default 300; a positive integer, else `load_config` fails).
- **The ping policy:** `/0` after every healthy check (every 3 minutes), `/start` and `/<status>` around an update, `/1` for an unhealthy app, nothing for a registry error or a skipped run. The README recommends a healthchecks.io check "update" with a period of 5 minutes and a grace time of 1 hour: an update can wait for the backup lock (the nightly backup, the Sunday check and restore test) after its `/start`.
- **No automatic rollback.** A failed update leaves the app on the new image, unhealthy; rolling back automatically would start an older version on a database the newer one may have migrated. The operator pins `BOOKCLUB_TAG` (or restores), and the heartbeat stays down until then.
- **Rollback by pinning keeps working with the timer:** a pin is a tag change like a release; the next run backs up and starts that version, and later releases leave it alone. Tested below.
- **The timer:** `OnCalendar=*:0/3`, `AccuracySec=15s`, not `Persistent` (a missed check needs no catching up). A run that is still busy when the timer elapses is not started twice (oneshot). A failed run leaves the unit `failed`, and the next elapse starts it again (tested).
- **The service:** `Type=oneshot`, `Requires=`/`After=docker.service`, `network-online.target`, `TimeoutStartSec=4h` (up to 3 h waiting for the backup lock), `PrivateTmp`, `ProtectSystem=full`, `NoNewPrivileges`, like the backup units, and `Environment=DOCKER_CONFIG=/root/.docker`: a system service without `User=` gets no `HOME` from systemd, and the pull needs root's `docker login`. Not niced: an update should finish quickly; its backup is short.

### A fix in `backup.sh` (found while building `update.sh`)

`docker compose ps -a -q app` also lists the one-off containers of `docker compose run app …` (checked with Compose v5.6.1: the label `com.docker.compose.oneoff=True`). `backup.sh`'s `check_media_mount` passed both IDs, joined by a newline, to `docker inspect`, so **a backup failed while any `docker compose run app` container existed**: on `phase-2`, `backup.sh` exited 1 with `Error: no such object: <id>\n<id>` while a one-off `sleep` container ran next to the app. The importer, `rematch-legacy` and the README's `df -h /tmp` check are all such containers. `app_container` in `backup-lib.sh` now skips one-off containers, and both scripts use it. With the fix, the same backup exited 0.

## Q7 = A: `env.example` and the README

- `env.example`: `TRUSTED_PROXIES` explains the value (`<Caddy IP>/32`, Caddy reaches the VM without source NAT) and the reason (a wrong value puts every user into Caddy's one rate-limit bucket; a wider one lets the subnet forge `X-Forwarded-For`). It stays `CHANGE_ME`, so the shipped file still does not start (`env_example_filled_in_is_a_production_configuration`). `BOOKCLUB_LISTEN` says that the "only Caddy" rule belongs in the Proxmox firewall.
- README, new section "The VM's network": `TRUSTED_PROXIES`; inbound only Caddy to 8080 (plus SSH from the admin host), **in the Proxmox firewall or at the router, not nftables/ufw on the VM**, because Docker routes a published port's packets before the host's `INPUT` chain (on the VM only a `DOCKER-USER` rule works); outbound to git.tillhome.de by split DNS (VLAN DNS or `/etc/hosts`) and one pinhole VM → Caddy:443, no other RFC 1918 destination; four checks (`getent`/`curl …/v2/` showing Caddy's LAN address and 401, `docker login`, `docker compose pull app`; no answer on 8080 from elsewhere in the LAN; the peer addresses the app sees through `nsenter … ss`; `verify-limits.sh`). "Reverse proxy" step 2 points there.
- Sources: Docker, "Packet filtering and firewalls" (https://docs.docker.com/engine/network/packet-filtering-firewalls/): published-port packets "are diverted before it reaches the `INPUT` and `OUTPUT` chains that ufw uses"; Docker, "Docker with iptables" (https://docs.docker.com/engine/network/firewall-iptables/): `DOCKER-USER` is "a placeholder for user-defined rules that will be processed before rules in the `DOCKER-FORWARD` and `DOCKER` chains", and restricting sources means "a negated rule at the top of the `DOCKER-USER` filter chain".

## Backups, option A: Storage Box snapshots

The Wave 1 security review's finding 3 (`docs/phase1/security-review.md`, "Phase 2 Wave 1", "The VM can delete its own off-site backups") offered three fixes; Rafael chose A. It needs no code: README "Backups" gained "Storage Box snapshots" (set-up in the console, how far back, space, credentials) and "Restore" gained "Restoring from a Storage Box snapshot" (read-only restore with `--no-lock`, then a rollback or a new repository). "Setting up" step 1 now also turns on SSH support (port 23) and "External Reachability".

Hetzner's documentation, read on 2026-10-29:

- "Storage Box Snapshots", https://docs.hetzner.com/storage/storage-box/snapshots/ (last change 2026-09-02): automatic snapshots on a schedule, manual ones kept until deleted; slots per plan BX11 10, BX21 20, BX31 30, BX41 40, and once the automatic slots are full the oldest is deleted as a new one is taken; snapshots are free but use the box's capacity, a new one takes no space and grows by later changes and deletions; they live in `/.zfs/snapshot`, or `/home/.zfs/snapshot` over SSH on port 23, one subfolder per snapshot that mirrors the box; "It is not possible to write to the `/.zfs` directory or its subfolder"; "Display snapshot directory" in the console shows or hides it; a restore undoes all changes since the snapshot, newer data and newer snapshots included; snapshots are "not full backups" and restore only the same box.
- "Creating Storage Box snapshots", https://docs.hetzner.com/storage/storage-box/getting-started/creating-snapshots/ (last change 2026-09-18): console path "Storage Boxes" → the box → "Snapshots" → "Enable automatic snapshots", choose when and the maximum number of slots, "Save".
- "Access with SSH/rsync/BorgBackup", https://docs.hetzner.com/storage/storage-box/access/access-ssh-rsync-borg/ (last change 2026-09-18): port 23, the restricted shell's commands (including `ls`), only `/home/` writable, relative paths recommended.
- "Storage Box Access Overview", https://docs.hetzner.com/storage/storage-box/access/access-overview/ (last change 2026-09-18): "External Reachability" must be enabled in the console ("Change settings") to connect from outside Hetzner's network; port 23 (SFTP, rsync, Borg) is activated in the console.
- Hetzner API changelog, https://docs.hetzner.cloud/changelog: Storage Boxes moved to the API at `api.hetzner.com` on 2025-06-25 (the Robot Storage Box endpoints were removed on 2025-07-30); on 2025-10-21, "Enable Snapshot Plan" made `hour` and `minute` required, and "Rollback Snapshot" takes `snapshot` (name or ID). So an API token can roll back or delete: the README forbids Console logins and API tokens on the VM.

What the README derives from that: the VM's SFTP key cannot delete or alter a snapshot; daily snapshots after the 03:30 backup (06:00 suggested; the console's time zone is to be checked, I found no statement of it); all of the plan's slots (BX11: 10 days back); the extra space is about 10 days of database dumps plus the pictures deleted in that time, because `forget --prune` deletes what the snapshots then keep; restoring reads the repository at `sftp:bookclub-backup:.zfs/snapshot/<snapshot>/bookclub` (relative to the home directory, which is `/home` on port 23) with `--no-lock`, through a copy of `backup.env` in `BACKUP_ENV`.

**For the integrator:** `docs/phase1/security-review.md` (GATE's file in Wave 1) can mark finding 3 as "Option A chosen (2026-10-29); set-up and restore in `deploy/hosted/README.md`, 'Storage Box snapshots'. Takes effect when Rafael enables the snapshot plan (work plan §5, item 12)."

## The release path: v0.11.0-rc.1

The orchestrator pushed `v0.11.0-rc.1` at 739ec9d at 09:39. Checked read-only with the forgejo MCP tools:

- **Run 13** (`release.yml`, database ID 22, ref `v0.11.0-rc.1`, commit 739ec9d): **success**, 09:39:37 to 09:51:07 (11 min 30 s). Jobs: `frontend` success, `server` success, `Build and push image` (on `docker-host`, runner v13.2.0, Docker 29.9.2, buildx 0.36.1, Compose 5.4.0) success, 08:46:06Z to 08:51:06Z.
- The publish log ends with `v0.11.0-rc.1 moves no floating tag: a pre-release, or a newer release exists` from `release-tags.sh`, so `git ls-remote` with the checkout's credentials worked, and the build summary shows `image.name` `git.tillhome.de/rafiistcool/bookclub-server:0.11.0-rc.1`, manifest `sha256:75e9fceccdd71d5043d5c3da8a4396fd0dc4deaf01147af5d3e48c712b83d8b6` (an OCI image manifest of 1245 bytes, amd64, no index), config `sha256:a746b62d…`, and the cache manifest `sha256:0ed85cb6…`.
- **Packages of `rafiistcool`:**
  - `bookclub-server`: exactly `0.11.0-rc.1` (09:50:13; 7 files: the manifest, the config and 5 layers, 47.5 MB) and `buildcache` (09:51:03). No `0.11`, no `stable`, no `latest`.
  - `bookclub`: exactly `0.10.0`, `0.10` and `latest`, all created 2026-10-18 16:29:02, untouched.
- This closes the open items of `dep.md` ("Still open after the review"): metadata-action, the login with the user secret `REGISTRY_TOKEN`, Forgejo's registry taking the BuildKit cache manifest, and `git ls-remote` all worked on Forgejo. `imagetools create` was not exercised there (a pre-release moves nothing); it ran against a local `registry:2.8.3` in DEP's review and in the test below. The first final release (`v0.11.0`) exercises it on Forgejo: afterwards `bookclub-server` must hold `0.11.0`, `0.11` and `stable` with the same manifest digest.
- Nothing failed, so nothing was fixed. No tag was pushed by DEP2.

## Tests (`server/tests/deploy_env.rs`, 23 tests)

- **Removed** the constant `READ_BY_A_LATER_PACKAGE` (`LEGAL_DIR`): `config.rs` reads `LEGAL_DIR` since LEG merged, as the constant's comment foresaw.
- `backup_env_example_names_every_script_setting`: every `${NAME…}` in `load_config`'s body (except `BACKUP_ENV`) is named in `backup.env.example`, and the example names nothing else apart from `RESTIC_*` and `AWS_*`.
- `systemd_units_start_scripts_that_exist` (unix): every timer has a service of the same name; every service's `ExecStart=/opt/bookclub/<script>` exists in `deploy/hosted/` and is executable; `backup.sh`, `restore-test.sh` and `update.sh` each have a unit.
- `update.sh` against a stand-in (`UpdateRig`): copies of `update.sh` and `backup-lib.sh` in a temp directory, a stub `backup.sh` that logs its run, and `docker` and `curl` as bash functions imported from the environment (as `smoke_test_refuses_names_that_are_not_ci_names` does) that log every call and answer from state files. No Docker, no network.
  - `update_leaves_an_up_to_date_app_alone`: pull, no backup, no `compose up`, ping `0`; a starting app gets no ping.
  - `update_backs_up_before_it_replaces_the_app`: `/start` < `backup.sh` < `compose up -d --no-deps --wait --wait-timeout 300 app` < `/0`; the old image is removed.
  - `update_reports_an_app_that_does_not_turn_healthy`: backup before `up`, pings `start`, `1`, no image removed; later runs ping `1` without another backup, for `running unhealthy` and `restarting unhealthy`.
  - `update_never_replaces_the_app_without_a_backup`: a failed backup means no `compose up`, pings `start`, `1`; the next run updates.
  - `update_leaves_a_stopped_app_and_a_dead_registry_alone`: an exited app gets no pull, no `up`, no ping; a failed pull exits 1 with `cannot pull`, no backup, no ping.
- **Mutations** (each reverted): ignoring a failed backup, dropping the running/restarting guard, updating on every run, and backing up after `compose up` each fail at least one of these tests; deleting `UPDATE_WAIT_SECONDS` from `backup.env.example` fails the settings test, and `chmod -x update.sh` the units test.

## Verified by hand (on the agents VM)

Docker 29.9.2 (containerd image store), Compose v5.6.1, buildx v0.38.1, systemd 257.

**The acceptance run** (a local registry instead of git.tillhome.de):
- One image of the root `Dockerfile` from this branch, built once (`CARGO_JOBS=3`, 2 min 34 s): `bookclub-dep2:a`. Derived with two-line Dockerfiles: B (a label), B′ (`HEALTHCHECK --interval=3s --retries=2 CMD ["false"]`), B-crash (`CMD ["bookclub-server", "no-such-command"]`), and later C and D (labels).
- `registry:2.8.3` on 127.0.0.1:5000. Tags moved in the registry with `docker buildx imagetools create --prefer-index=false`, as `release-tags.sh` does.
- Staging `bookclub-staging-dep2` from `compose.yaml`, `env.example` filled in as the README says, `BOOKCLUB_PORT=8081`, data in `/tmp/bookclub-dep2-staging`, and a `compose.override.yaml` that named `127.0.0.1:5000/rafiistcool/bookclub-server:${BOOKCLUB_TAG:-stable}`. The scripts copied in, `backup.env` from the example, a local restic repository (`backup.sh restic init`), one account through `beta-invite` and `POST /api/auth/register` (201). A Python listener on 127.0.0.1:18099 stood in for healthchecks.io.
- **A, unchanged tag:** exit 0, `… is up to date (…@sha256:1a37…), and the app is healthy`, ping `GET /ping/update/0`.
- **A → B as `stable`:** exit 0 in 6 s. Pings in order: `update/start`, `backup/start`, `backup/0` (`backup done`), `update/0` (`updated: the app runs …@sha256:0423… and is healthy`). The container carried B's label and was healthy; the next run was a no-op with `update/0`.
- **B → B′ as `stable`:** the backup ran first (`backup/start`, `backup/0`), then `container bookclub-staging-dep2-app-1 is unhealthy`, exit 1, `POST /ping/update/1` with the log tail. The next run: exit 1, `error: … but is running unhealthy`, `update/1`, no backup.
- **Rollback by pinning** `BOOKCLUB_TAG=0.11.1` (B) while on B′: backup, healthy on B, `update/0`, `docker compose images app` showed `0.11.1`.
- **B-crash** (pinned `0.11.3`): `up --wait` failed within 2 s (`is unhealthy`), exit 1, `update/1`; the next run `… but is restarting unhealthy`, `update/1`.
- **Stopped app** (`docker compose stop app`): `the app is exited, so nothing is updated`, exit 0, no ping; the same with a one-off `docker compose run` container running next to it. With the app running and a one-off container, the normal check (`update/0`).
- **Registry stopped:** compose's error on stderr, `error: cannot pull …; nothing was changed`, exit 1, no ping.
- **Backup fails** (a `BACKUP_ENV` copy with an uninitialised repository): `Fatal: repository does not exist`, `error: restic backup exited 10`, exit 1, pings `backup/1` and `update/1`; the app stayed on B, healthy. The next run (real `backup.env`) backed up and updated.
- **Old image removal:** A → C → D through `stable`, with C held by no local tag: after C → D, `removed the previous image`, and `docker image inspect` of C failed. A and B, which local tags still named, were kept.
- **Lock:** a second `update.sh` started while the C → D update ran logged `another update.sh is running (…/.update.lock)` and exited 0.
- **The read-only snapshot restore** (`--no-lock`): a copy of the repository made read-only (owned by uid 1000, `chmod -R a-w`), read as uid 1000 through a `BACKUP_ENV` copy as in the README. Without `--no-lock`: `unable to create lock in backend: … permission denied`. With it: `snapshots` listed both snapshots, and `restore latest --host bookclub --tag bookclub` restored the dump and `counts.tsv`. `--retry-lock 10m` (always added by `restic_`) and `--no-lock` together are accepted by restic 0.17.3.
- **`backup.sh` with a one-off container** (the fix above): `phase-2`'s version exited 1 with `Error: no such object: <id>\n<id>`, this branch's exited 0. `restore-test.sh` afterwards: `restore test passed: 34 tables, 11 rows, migration 2200, 0 media files`, `restore-test/0`.
- **The `nsenter … ss` check** of "The VM's network", against the staging app with one open connection from the VM's LAN address: it printed `172.19.0.2:8080 192.168.178.14:46986`, the client's own address (no source NAT on that path).

**systemd** (a stand-in for the VM, as in DEP's review): a Debian 13 container (`debian:trixie-slim` with `systemd`, `curl`) as PID 1, privileged, its own cgroup namespace, the host network (so that the listener on 127.0.0.1 was reachable), `tmp.mount` masked, a `docker.service` stand-in, the host's Docker socket, CLI and plugins mounted, `/opt/bookclub` bound to the staging directory, and a drop-in `BindPaths=/tmp/bookclub-dep2-staging` (the staging is under `/tmp`, which `PrivateTmp` hides; on the VM nothing lies there).
- The README's commands as written: `cp deploy/hosted/systemd/bookclub-update.* /etc/systemd/system/`, `systemctl daemon-reload`, `systemctl enable --now bookclub-update.timer`, `systemctl list-timers 'bookclub-*'`.
- With B released as `stable` in the registry, the 09:06 run updated D → B (backup, healthy, `removed the previous image`, `update/0`); the 09:09 run was a no-op (`update/0`). `Environment=DOCKER_CONFIG=/root/.docker`, `PrivateTmp=yes`, `ProtectSystem=full`, `NoNewPrivileges=yes`, `Result=success`.
- B-crash released as `stable`: the 09:12 run failed (`Failed with result 'exit-code'`, `update/1`), and the 09:15 run started anyway from the `failed` state and pinged `update/1` again.
- The README's rollback, as written: `sed -i 's/^BOOKCLUB_TAG=.*/BOOKCLUB_TAG=0.11.1/' .env`, `systemctl start bookclub-update` (exit 0 after the update), `docker compose images app` showed `0.11.1`, the unit was `inactive` again.
- `systemd-analyze verify` on both units: clean. `systemd-analyze calendar '*:0/3'`: every 3 minutes.

**Lint and checks:** shellcheck v0.10.0 on `deploy/hosted/*.sh deploy/cutover/*.sh`, with and without `-x`, and from `deploy/hosted/` with `-x`: clean. actionlint 1.7.7 on all workflows: clean. `docker compose -f deploy/hosted/compose.yaml --env-file deploy/hosted/env.example config -q`: exit 0. `caddy validate` on `Caddyfile.example`: valid. Server: `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked` (1338 passed, 0 failed), `cargo sqlx prepare -- --all-targets` (no change) and `cargo sqlx prepare --check -- --all-targets`. The frontend checks were not run: nothing in `frontend/` or the server's DTOs changed.

**Cleanup:** the staging stack (`down -v`), the systemd stand-in and its image, the registry, the test images (A, B, B′, B-crash, C, D and the pulled tags), `/tmp/bookclub-dep2-staging`, the listener, then `docker builder prune -f`.

## Not verified (needs the VM or accounts)

- A real Storage Box: the snapshot plan in the console, `ssh bookclub-backup ls .zfs/snapshot`, the restic path `.zfs/snapshot/<snapshot>/bookclub` over SFTP, the console's rollback, its time zone, and how much space the snapshots take. The `--no-lock` restore ran only against a local read-only copy.
- healthchecks.io itself (a local listener stood in), and `docker login git.tillhome.de` plus `docker compose pull` from the VM: the agents VM has no package token, so the registry was a local one.
- Q7 on the network: the Proxmox firewall rule, the split DNS and the pinhole, and `TRUSTED_PROXIES` with Caddy's real address. The README's checks are written for when the VM exists.
- systemd on the production VM itself (a container with systemd 257 stood in).

## For the integrator

- **`CLAUDE.md`**, the deployment pointer: `deploy/hosted/README.md` also covers the automatic updates (`update.sh`, `bookclub-update.timer` every 3 minutes, settings in `backup.env`), the VM's network (Q7) and the Storage Box snapshots. "Deployment files": `server/tests/deploy_env.rs` now also checks `backup.env.example` against `backup-lib.sh`, the systemd units, and `update.sh` against a stand-in.
- **`docs/phase2/cutover.md`** (CUT's): before the import, `systemctl stop bookclub-update.timer` next to `docker compose stop app`, and `systemctl start bookclub-update.timer` after `docker compose up -d --wait` (as in the README's "The legacy import"). `update.sh` leaves a stopped app alone anyway; the stop keeps an update from starting between the steps. The same for the rollback window, if the old app is to be restarted.
- **`docs/phase1/security-review.md`**: finding 3, see "Backups, option A" above.
- **Work plan §5** (Rafael's steps): item 10, the inbound rule belongs in the Proxmox firewall (Docker's published ports pass by the VM's own `INPUT` rules); item 12, enable the snapshot plan (all slots, daily after the backup) and External Reachability; item 13, the heartbeat "update" has a period of 5 minutes and a grace time of 1 hour; item 15.1, also `cp update.sh` and `systemctl enable --now bookclub-update.timer` after the backups.
- No new server variable, CLI command, route, DTO, error string or migration. New files: `deploy/hosted/update.sh`, `deploy/hosted/systemd/bookclub-update.{service,timer}`, `docs/phase2/notes/dep2.md`.
