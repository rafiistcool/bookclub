# Cutover runbook (#52)

This runbook moves the club from the Python app (v0.10.0, SQLite) on the old host to the hosted
Rust server (Postgres) on the new host. It builds on the rehearsal procedure in
`docs/phase1/notes/imp.md` ("Manual step") and on the operator runbook in
`deploy/hosted/README.md`. The rehearsal on a copy of the production database (CUT2, #38) comes
first: it fixes the club slug, measures the run time and shows what the report of the real data
looks like.

Every command is meant to be copied. Values in `<angle brackets>` and the variables below are
filled in once.

- **Old host:** the machine that runs the Python app's compose project (service `bookclub`, data
  directory `./data` with `bookclub.db`).
- **App VM:** the machine that runs `deploy/hosted/compose.yaml` (services `postgres` and `app`).
- **Caddy host:** the Caddy that serves the old and the new hostname.

```bash
# On the app VM. Fill these in from the rehearsal (#38) and the old .env.
NEW_HOST=<new.example>            # the final public host (Q1)
OLD_HOST=<old.example>            # the old public host
OWNER=<username>                  # Rafael's legacy username: becomes the club owner
CLUB_NAME='<BOOKCLUB_NAME>'       # BOOKCLUB_NAME of the old .env
CLUB_TZ=<BOOKCLUB_TZ>             # BOOKCLUB_TZ of the old .env, e.g. Europe/Berlin
SLUG=<slug>                       # the slug of the rehearsal (D5): the club URL never changes
IMPORT=/srv/bookclub/import       # the legacy copy on the app VM, never in a repo
```

## 1. Checklist before the cutover

Everything here is done before the announcement goes out, and checked again the day before.

**Decisions and accounts**

- [ ] The name and the domain are chosen (Q1), and DNS for `NEW_HOST` points at the business line.
- [ ] Mail works on the final domain (Q4): `docker compose exec app bookclub-server send-test-mail
      <your address>` arrives, and SPF, DKIM and DMARC pass.
- [ ] The legal texts are in `/srv/bookclub/legal` (Q3), and `/imprint`, `/privacy` and `/terms`
      show them logged out on staging.

**The app VM (#51)**

- [ ] The stack runs from the pinned release tag for the cutover (`BOOKCLUB_TAG`, Q5: v1.0.0 marks
      the cutover), with `SIGNUP_MODE=invite` and `DNB_HARVEST=false` until after the import.
- [ ] Backups run, and `restore-test.sh` has passed once (`deploy/hosted/README.md`).
- [ ] The uptime monitor watches `https://<NEW_HOST>/api/health`.
- [ ] Staging is wiped: `docker compose down -v`, then `/srv/bookclub/media` emptied. The target
      must be empty: **no `beta-invite` and no sign-up before the import** (the importer refuses a
      target with users, exit 2, or with invites, exit 1).
- [ ] `sqlite3` is installed for `compare.sh` (`apt-get install sqlite3`), and `deploy/cutover/`
      (compare.sh and both SQL files) is copied to the VM, e.g. to `/srv/bookclub/cutover`.
- [ ] The import directory exists and only uid 1000 (the app container's user) and root can read
      it: `install -d -o 1000 -g 1000 -m 700 /srv/bookclub/import`.
- [ ] The app container's `/tmp` has room for a copy of `bookclub.db` and its `-wal`: the importer
      reads a private copy there, because a WAL database cannot be opened in place from the
      read-only import mount. Compare `docker compose run --rm -T app df -h /tmp` with the size of
      the rehearsal snapshot.

**The rehearsal (CUT2, #38)**

- [ ] The dry run and the real run of the rehearsal exited 0, `compare.sh` matched, and Rafael
      signed in with his old password and saw his data.
- [ ] The slug (D5), the run time and the lookup wait are in #38. The downtime window is the run
      time twice (dry run and real run), plus the lookup wait, plus one hour for the checks.

**The old host and the Caddy host**

- [ ] The Python service is pinned to its exact image tag, and no auto-updater runs (§5 A3).
- [ ] The new host's site block (`deploy/hosted/Caddyfile.example`) is live, and staging answers
      on it.
- [ ] The old host's new site block is prepared from `deploy/legacy-origin/Caddyfile.example`
      with both host names filled in, and `sw.js` is copied to
      `/srv/bookclub-legacy-origin/sw.js` on the Caddy host. It is not active yet.
- [ ] `caddy validate --config <the Caddyfile with the new block> --adapter caddyfile` passes.

**People**

- [ ] The announcement (section 2) went to the club at least a week ahead, with the date, the time
      and the new address.

## 2. The announcement

Send it at least a week ahead, in both languages. Fill in the date, the times and the new
address.

**Deutsch**

> **Bookclub zieht um: \<Datum\>, \<Uhrzeit\> bis etwa \<Uhrzeit\>**
>
> Unser Bookclub bekommt ein neues Zuhause. Während des Umzugs ist die App für etwa \<Dauer\>
> nicht erreichbar. Danach gilt:
>
> - **Neue Adresse:** https://\<neue Adresse\>. Die alte Adresse leitet dorthin weiter.
> - **Gleiche Anmeldung:** Benutzername und Passwort bleiben, alle Regale, Notizen, Zitate,
>   Abstimmungen und das Club-Archiv ziehen mit.
> - **E-Mail-Adresse eintragen:** Nach der ersten Anmeldung fragt die App nach einer
>   E-Mail-Adresse. Bitte eintragen und bestätigen: nur damit kannst du ein vergessenes Passwort
>   zurücksetzen.
> - **App neu zum Startbildschirm hinzufügen:** Das alte Symbol zeigt auf die alte Adresse.
>   Lösche es und füge die neue Adresse wieder zum Startbildschirm hinzu.
> - **Entwürfe vorher abschicken:** Nicht abgeschickte Notizen liegen nur in deinem Browser und
>   ziehen nicht mit. Bitte schick sie vor \<Datum, Uhrzeit\> ab.
> - **Alte Einladungslinks gelten nicht mehr.** Wer neu dazukommen soll, bekommt nach dem Umzug
>   eine neue Einladung.
> - **Keine Push-Benachrichtigungen** bis zur App: Die neue Version verschickt vorerst keine
>   Push-Nachrichten.
>
> Fragen oder Probleme nach dem Umzug: \<Kontakt\>.

**English**

> **Bookclub is moving: \<date\>, \<time\> to about \<time\>**
>
> Our Bookclub is getting a new home. During the move the app is offline for about \<duration\>.
> Afterwards:
>
> - **New address:** https://\<new address\>. The old address redirects there.
> - **Same login:** your username and password stay the same, and all shelves, notes, quotes,
>   votes and the club history move with you.
> - **Add an email address:** after your first sign-in, the app asks for an email address. Please
>   add and confirm one: it is the only way to reset a forgotten password.
> - **Add the app to your home screen again:** the old icon points to the old address. Remove it
>   and add the new address to your home screen.
> - **Send your drafts before the move:** unsent notes live only in your browser and do not move.
>   Please send them before \<date, time\>.
> - **Old invite links stop working.** Anyone new gets a fresh invite after the move.
> - **No push notifications** until the app: the new version sends no push messages for now.
>
> Questions or problems after the move: \<contact\>.

**When it is live** (short, both languages):

> Der Umzug ist fertig: https://\<neue Adresse\>. Melde dich wie gewohnt an und trag deine
> E-Mail-Adresse ein. / The move is done: https://\<new address\>. Sign in as usual and add your
> email address.

## 3. Cutover day

### 3.1 Stop the old app and take the snapshot (old host)

Optional, before stopping: show a maintenance message on the old host instead of Caddy's 502.
Replace the old host's site block for the window and `caddy reload`:

```caddyfile
<OLD_HOST> {
	respond "Bookclub zieht gerade um und ist bald unter der neuen Adresse zurück. / Bookclub is moving and will be back at its new address soon." 503
}
```

Then, in the old compose directory:

```bash
docker compose stop bookclub            # stop, never down: the container stays for a rollback
ls -l data/                             # after a clean stop, normally no bookclub.db-wal is left
install -d -m 700 /root/bookclub-final
cp data/bookclub.db /root/bookclub-final/
[ -e data/bookclub.db-wal ] && cp data/bookclub.db-wal /root/bookclub-final/
sha256sum /root/bookclub-final/* | tee /root/bookclub-final/SHA256SUMS
```

Copy the files, never the `-shm`. If a `bookclub.db-journal` exists, the app did not stop cleanly:
start it once (`docker compose start bookclub`), wait until it is healthy, and stop it again.

Keep an encrypted copy of the final snapshot for the retention period (Q14: 30 days), outside
the repo, with the passphrase in the password manager:

```bash
tar -C /root -cf - bookclub-final | gpg --symmetric --cipher-algo AES256 \
    -o /root/bookclub-final-$(date +%F).tar.gpg
```

### 3.2 Transfer the snapshot (to the app VM)

```bash
# On the old host. The import directory is owned by uid 1000 (section 1).
scp /root/bookclub-final/* <app-vm>:/srv/bookclub/import/
# On the app VM:
cd "$IMPORT" && sha256sum -c SHA256SUMS
chown 1000:1000 "$IMPORT"/bookclub.db* && chmod 600 "$IMPORT"/bookclub.db*
```

### 3.3 Dry run, go/no-go, real run (app VM)

In the compose directory. Stop the app, so nobody can sign up while the import runs;
`docker compose run` starts Postgres if it is not running. `-T` keeps the report free of terminal
line endings (compare.sh reads it).

```bash
docker compose stop app
docker compose run --rm -T -v "$IMPORT:/import:ro" app \
    bookclub-server import-legacy /import/bookclub.db --owner "$OWNER" \
    --club-name "$CLUB_NAME" --timezone "$CLUB_TZ" --club-slug "$SLUG" \
    --allow-unmatched --dry-run > dry.txt
echo "exit $?"; cat dry.txt
```

`--allow-unmatched` (D6) keeps going when DNB or Open Library cannot answer: those ISBN rows become
bare facts (`isbn error`), and `rematch-legacy` resolves them later. Exit 2 means the target is not
empty (wipe it, section 1); exit 1 prints the reason.

**Go/no-go.** Read `dry.txt` against the rehearsal's report in #38:

- every entity line `source → target`: the differences are the listed merges and skips;
- the counts are at least the rehearsal's (the club kept writing since), and nothing new is in
  `notes`;
- `normalised`, `merges`, `differences` and `isbn keys` hold nothing unexpected.

Anything unexpected: stop here and roll back (section 4). The target is still empty.

```bash
docker compose run --rm -T -v "$IMPORT:/import:ro" app \
    bookclub-server import-legacy /import/bookclub.db --owner "$OWNER" \
    --club-name "$CLUB_NAME" --timezone "$CLUB_TZ" --club-slug "$SLUG" \
    --allow-unmatched > real.txt
echo "exit $?"
diff <(tail -n +2 dry.txt) real.txt
```

The diff may show only the `lookup wait` line, or an ISBN whose upstream record changed in
between.

### 3.4 Independent count check, before anyone signs in

```bash
/srv/bookclub/cutover/compare.sh --report real.txt "$IMPORT/bookclub.db" -- \
    docker compose exec -T postgres psql -U bookclub -d bookclub
echo "exit $?"
```

Run it as root (the import directory belongs to uid 1000), from the compose directory. It
counts both databases with its own queries (`legacy-counts.sql`, `target-counts.sql`), checks
every count against the report, prints the per-user counts, and compares
`id|username|password_hash` of every account without printing a hash. Exit 0 is the go.

- The account comparison is valid only now: a login re-hashes a legacy hash.
- A per-user difference must be one of the report's merges or skips: a merged shelf entry or
  favourite, a skipped pick note, an avatar that was not a JPEG or WebP image.
- `-U` and `-d` are `POSTGRES_USER` and `POSTGRES_DB` of the env file.

Post only counts to #38: no usernames, no report lines with titles.

### 3.5 Start the server and check it

```bash
docker compose up -d --wait app
curl -fsS https://$NEW_HOST/api/health                     # {"status":"ok"}
docker compose exec app bookclub-server make-admin "$OWNER"
```

Rafael, in the browser on `https://<NEW_HOST>`:

1. Sign in with the old password. Add an email address and verify it (the admin mails and password
   resets need a verified address).
2. The club, the shelves, the physical shelf layouts, the pick and its notes, the votes, the
   quotes and the activity feed are there.
3. Old links resolve: open a `/works/OL…W` link from an old chat.
4. A second member (or a test sign-in by a member who agreed) gets the email prompt.

If `isbn unmatched` or `isbn error` in `real.txt` is not 0, rematch now or later (it commits per
edition and is safe on the live server):

```bash
docker compose exec -T app bookclub-server rematch-legacy --dry-run
docker compose exec -T app bookclub-server rematch-legacy
```

### 3.6 Redirect the old host (Caddy host)

Replace the old host's site block (or the maintenance block) with the prepared block from
`deploy/legacy-origin/Caddyfile.example`, then:

```bash
caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile
caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile   # or: systemctl reload caddy
curl -sI "https://$OLD_HOST/shelf?x=1" | grep -iE '^(HTTP|location)'
#   HTTP/2 301 / location: https://<NEW_HOST>/shelf?x=1
curl -sI "https://$OLD_HOST/sw.js" | grep -iE '^(HTTP|content-type|cache-control)'
#   HTTP/2 200 / content-type: text/javascript; charset=utf-8 / cache-control: no-store
```

`/sw.js` is not redirected: an installed app from before PR #3 still runs the old offline worker,
and a browser does not follow a redirect when it checks that worker for updates. The new
`sw.js` deletes the old caches, unregisters itself and reloads the page, which then follows the
redirect (risk R22, `docs/phase1/spec/platform.md`).

Then send the "it is live" message (section 2). This is the end of the rollback window.

## 4. Rollback window

Rolling back is possible until members write to the new instance: until the redirect goes live in
3.6 and the "it is live" message is sent. Until then, the old app's data is untouched (it was
stopped, and the importer read a copy), so a rollback loses nothing. Rafael's own checks on the
new instance (3.5) are the only writes there, and they may be lost.

To roll back:

1. Caddy host: restore the old host's original site block and `caddy reload`.
2. Old host: `docker compose start bookclub`, wait for healthy, and sign in once.
3. App VM: `docker compose down -v` and empty `/srv/bookclub/media`, so the next attempt starts
   from an empty target. Keep `$IMPORT` until the next attempt replaces it.
4. Tell the club the move is postponed, and record why in #52.

After the window closes, there is no rollback: members' writes exist only on the new instance.
Problems are fixed forward. The backup timer of `deploy/hosted/` covers the new data from the
first night on.

## 5. Afterwards

- **Same day:**
  - Turn on the catalog harvest: `DNB_HARVEST=true` in the env file, then
    `docker compose up -d app`.
  - Delete the import copy on the app VM: `rm -rf "$IMPORT"/bookclub.db* "$IMPORT"/SHA256SUMS`.
    It holds every password hash.
  - Remove `dry.txt` and `real.txt` once the counts are in #38: the report names users and books.
- **Within days:**
  - Merge `cutover/remove-python` (CUT-R, §2.4 of the work plan) once Forgejo CI is green.
  - Mint the closed-beta invites (`docker compose exec app bookclub-server beta-invite`). Never
    before the import: the importer refuses a target with invites.
  - Close #52, and #38 if the rehearsal has not closed it. Close #35 (the password change is
    back) and #34 as superseded, after adding "Web Push for the browser app" to #60 (D9).
- **After 30 days (Q14):**
  - Old host: `docker compose down` in the old compose directory, then delete `data/` and
    `/root/bookclub-final`.
  - Delete the encrypted final snapshot `/root/bookclub-final-<date>.tar.gpg`.
  - Remove the Python image from the old host: `docker image rm <the pinned image>`.
- **Keep for at least a year:** the redirect of the old host, so old links in chats keep working.

## 6. Preserving the Python app (Q6)

Once Rafael answers Q6 with A, the integrator runs these from a checkout whose remote `origin`
is the Forgejo repository (never the GitHub mirror). 1c5efe3 is v0.10.0-3, the last
state of the Python app on `main`; `backend/` there equals `backend/` on `phase-2` apart from one
removed test file.

```bash
git fetch origin
git branch legacy-python 1c5efe3
git tag -a legacy-python-final 1c5efe3 \
    -m "The Python app (v0.10.0-3) before the cutover to the Rust server (#52)"
git push origin legacy-python refs/tags/legacy-python-final
git ls-remote origin refs/heads/legacy-python refs/tags/legacy-python-final
#   1c5efe3904fe1be6cf263bfdcd06c776a4224b5c  refs/heads/legacy-python
#   <tag object id>                            refs/tags/legacy-python-final
git rev-parse 'legacy-python-final^{commit}'   # 1c5efe3904fe1be6cf263bfdcd06c776a4224b5c
```

The release workflow runs only for `v*` tags (`.forgejo/workflows/release.yml`), so neither push
starts a release. Check that no run started on Forgejo's Actions page. A Python hotfix later is a
commit on `legacy-python` and a `v0.10.x` tag there: Forgejo runs the workflow of the tagged
commit, which still builds the Python image.
