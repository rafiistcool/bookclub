# MOVE (#52): the slim private move

Owner: MOVE. Plan: `docs/phase3/work-plan.md` §2.3, §5 and answers 30–50. M1, the code, is a Wave 1 package on `phase3/move`; M2–M6 are Wave 2 operations that follow `docs/phase3/private-move.md`. This file records what M1 built, its decisions and deviations, the texts the integrator moves into `conventions.md`, `endpoint-map.md` and `CLAUDE.md`, and the checks. The sections for M2–M6 (the facts found on the host, the move's counts, the phone test) are filled in when they run.

## M1: the code

### Files

| File | What |
|---|---|
| `server/src/config.rs` | `EmailTransport::None`; `EMAIL_TRANSPORT=none` without `DEV_MODE` and without `SMTP_URL`/`EMAIL_FROM`; `SIGNUP_MODE=open` with `none` refused before the legal texts' check; the new error text for an unknown transport; unit tests `none_needs_no_smtp`, `open_signup_needs_mail`, `unknown_transport_names_all_three`. |
| `server/src/email/mod.rs` | `NoEmail`, `MAIL_OFF`, `is_mail_off`; unit test `the_none_transport_sends_nothing`. |
| `server/src/auth/mail.rs` | `deliver` logs `mail not sent: mail is off` at INFO with the kind only, instead of WARN, when the transport is off. |
| `server/src/lib.rs` | Only the `EmailTransport::None => Arc::new(email::NoEmail)` arm in `AppState::new`. |
| `server/src/auth/reset.rs` | `replace_password` (the confirm step's transaction body, now shared) and `pub async fn set_password`; the password generator with a unit test. |
| `server/src/main.rs` | `set-password <username>`; `send-test-mail` with `none` exits 1 with `error: mail is off (EMAIL_TRANSPORT=none)`; the CLI parse test. |
| `server/tests/auth_set_password.rs` | **New.** Eight tests: six from M1 (below), two from the review round. |
| `server/.sqlx/` | One new query (`SELECT id FROM users WHERE username = $1 FOR UPDATE`). The confirm's queries moved into `replace_password` unchanged, so their files stayed. |
| `frontend/public/sw.js` | **New.** The kill switch for the Python app's offline worker: `deploy/legacy-origin/sw.js` with its own header comment. |
| `frontend/src/swKillSwitch.test.ts` | **New.** Three tests (below). |
| `deploy/cutover/host-facts.sh` | **New.** The read-only facts of §5.4, shellcheck-clean. |
| `server/tests/private_move.rs` | **New** (the checks round, below). Five tests that read the runbook's blocks and run them, and run `host-facts.sh`, with stand-ins. |
| `deploy/hosted/env.example` | The mail comment explains `none`; `#EMAIL_TRANSPORT=smtp` stays. |
| `deploy/hosted/README.md` | MOVE's hunks only: a pointer to the private-move runbook under the intro; "Requirements" gains "Or mail off"; "First boot" step 5 says `send-test-mail` exits 1 with `none`; "The first account" gains "A forgotten password while mail is off" with the `set-password` command; "When it does not start" gains the mail error. |
| `docs/phase3/private-move.md` | **New.** The runbook (§5.6). |
| `docs/phase2/cutover.md` | Two lines at the top pointing to the private-move runbook. |

No route, DTO, utoipa text, `error::msg` string, migration or i18n key changed. `frontend/src/api/generated.ts` is unchanged (`npm run check:api` passes).

### What was built

**Mail off (§5.2).**
- `EMAIL_TRANSPORT=none` parses outside `DEV_MODE` too. Unset still means `smtp` outside `DEV_MODE` (`log` in it), so the secure default stays. `SMTP_URL` and `EMAIL_FROM` are required only for `smtp`.
- `SIGNUP_MODE=open` with `none` stops `Config::from_lookup` with `SIGNUP_MODE=open needs mail: set EMAIL_TRANSPORT=smtp`, right after the transport is parsed and before `LegalTexts::configure`, so a stack without either names mail first (the boot test checks that order).
- An unknown value: `EMAIL_TRANSPORT must be log, smtp or none, not "…"`.
- `email::NoEmail::send` returns `Err(anyhow::Error::msg(MAIL_OFF))` and logs nothing: no recipient, subject, body or link. `email::is_mail_off(&error)` tells that refusal apart from a failed send: a downcast to `&'static str` that equals `MAIL_OFF`, so an SMTP failure (a lettre error under the context `SMTP send failed`) never counts, whatever its text.
- `auth::mail::deliver` checks `is_mail_off` first and logs `mail not sent: mail is off` at INFO with `kind`; every other failure still WARNs. It returns false as before, so every caller treats the mail as not sent: the reset request voids its link (tested), sign-up still answers 201 and leaves its verification token open, unreceived, until it expires or "resend" replaces it.
- `admin::notify` and `admin::notices` send through `state.email` directly, only to verified addresses. With mail off no address can be verified (the verification link never arrives), so they never send; if a verified address existed, their existing failure paths log a WARN and stamp nothing.
- `send-test-mail` picks `NoEmail` for `none`; `main` prints `error: mail is off (EMAIL_TRANSPORT=none)` and exits 1.

**`set-password` (§5.2).**
- `auth::reset::replace_password(conn, user_id, new_hash)`: the confirm step's body, unchanged in what it does: `password_hash` and `password_changed_at`, `DELETE FROM sessions` (cookie sessions and bearer tokens share the table), the open `reset_password` and `verify_email` tokens voided. `confirm_reset` calls it inside its transaction.
- `auth::reset::set_password(conn, passwords, username) -> anyhow::Result<Option<String>>`: 20 characters of the invite codes' Crockford alphabet (`0-9A-Z` without I, L, O, U; 100 random bits from `auth::random`), hashed before the transaction (as the confirm does, so the row lock waits for no argon2), then in one transaction the user row `FOR UPDATE` by the normalised name and `replace_password`. `None` for an unknown name. A suspended account gets the password and stays suspended (login answers 403 `This account is suspended`, tested).
- `bookclub-server set-password <username>` reads the configuration like the other account commands, connects (and migrates), hashes with `Passwords::new()` (the production parameters `AppState::new` uses for sign-up), prints only the password on stdout, and logs `password set from the command line; every session of the account ended` on stderr without naming the account. An unknown name exits 1 with `error: No account with that name`.
- It cannot clear the running server's in-memory login-failure buckets (another process); they empty within 10 minutes. The README and the runbook say so.

**The old service worker (§5.3).** `frontend/public/sw.js` is the code of `deploy/legacy-origin/sw.js` under a header that says why it lives here: install → `skipWaiting`; activate → `clients.claim`, delete every cache, `registration.unregister()`, navigate every open window to its own URL; no fetch handler. Vite copies it to `dist/sw.js`, and `spa.rs` serves it as an existing file with `text/javascript` and `SHELL_CSP` (no change to SCAN's file). The SPA registers no worker and uses no Cache API (checked with `grep`), so only a browser that still runs the old worker fetches it.

**`host-facts.sh` (§5.4).** `deploy/cutover/host-facts.sh OLD_COMPOSE_DIR HOSTNAME`, as root on the host. Read-only; prints, in sections: OS, `uname -m`, systemd, Docker Engine and compose versions, Docker's root; `docker compose ls -a`; every container with image and status, auto-updaters (watchtower, ouroboros, shepherd, what's-up-docker) flagged; the old container (found by its compose labels for `OLD_COMPOSE_DIR` and service `bookclub`, else `docker compose ps`): image, state and health, restart policy, compose project and files, the watchtower label, published ports with bindings, networks with aliases and the containers on them, mounts, the sizes of `bookclub.db` and its `-wal`/`-journal`; `BOOKCLUB_NAME`, `BOOKCLUB_TZ`, `BOOKCLUB_PORT`, `BOOKCLUB_PUBLIC_URL` by exact key from the old `.env` and from the container's environment (no other key ever leaves either); whether root's Docker config names `git.tillhome.de`; the old container's established peers on its port (`nsenter -n` + `ss`), sampled once a second for `PEER_SECONDS` (10) and counted; who listens on 80 and 443; `curl -sI https://HOSTNAME/` (status line, `Strict-Transport-Security`, `Permissions-Policy`); free disk under `/srv` and Docker's root, `lsblk`, network shares (`findmnt -t nfs,nfs4,cifs,smb3`, without options, which can hold a user name); `sqlite3`; `/opt/bookclub` and `/srv/bookclub`. It ends with `note:` lines for the runbook and `STOP:` lines, and exits 1 when a check stops the move (not x86_64, Docker < 25, compose < 2.24, no HTTPS answer, no systemd, no curl), 2 on a usage error.

Tried on the agents VM against a stand-in compose project (busybox under the service name `bookclub`, a `.env` with `SECRET_KEY`): every section printed, the secret appeared nowhere, the bad-argument exits were 2, and the HTTPS stop fired for `localhost:1`. As uid 1000 it says the peers and root's Docker config were not read; the peer and listener parsing was checked on sample `ss` lines. The real run is M3's.

**The runbook (§5.6).** `docs/phase3/private-move.md`, built on `cutover.md` as COVER left it: the variables (in `/root/bookclub-move/vars.sh` on the host, with a `summary` filter for the import reports), "What may reach my terminal", the keep/drop table, M2's digest check, the key with `from=`, the facts and what they decide, the install next to the Python app (`set_env` for `.env`, the alias override, the watchtower labels), the check start on 18080, backups and the units, the checklist, the DE and EN messages, move day 7.1–7.6, the rollback, the afterwards steps with CUT-R and the issues, `set-password`, and the M6 cleanup block. The bash blocks pass `bash -n` (apart from their `<placeholders>`), and the cleanup block passes shellcheck; its four outcomes (no restore test, an old snapshot, no `bookclub.db`, all fine) were run with stand-ins for `journalctl`, `backup.sh` and `docker`.

### Decisions taken while building

- **`is_mail_off` by type and text.** `deliver` must not mistake an SMTP error that happens to quote the words for mail off; the downcast checks that the error is a bare `&'static str` equal to `MAIL_OFF`, which only `NoEmail` builds.
- **Open sign-up refuses `none` in DEV_MODE too.** The plan states the rule without a mode; a developer who wants open sign-up locally keeps the default `log`.
- **The mail-off flow test lives in `auth_set_password.rs`.** It is the only test file MOVE owns. It boots a production configuration with `EMAIL_TRANSPORT=none` through `AppState::new` (so the `lib.rs` arm runs), signs up with a sign-up invite (201), and checks that a reset request's link is voided.
- **The kill switch's code is pinned to the old hostname's copy** by `swKillSwitch.test.ts`, so the two cannot drift apart; the test also runs the worker against stand-ins for `self`, `caches` and `clients` and checks the order of its calls.
- **`main.rs` spells `No account with that name` as a literal.** The constant is `admin::users::NO_ACCOUNT` on `phase-3`, and FRIENDS moves it to `error::msg::NO_ACCOUNT`; either reference would break one side of the merge. After both merges the integrator may use `msg::NO_ACCOUNT`.
- **Runbook: `TRUSTED_PROXIES` by the proxy's place.** A proxy on another host: the peer `host-facts.sh` saw, as `/32` (unchanged by the move). A proxy on the same host over loopback: the gateway of the **new** compose network, read after the first start; the old network's gateway, which `host-facts.sh` sees, does not carry over. A proxy container on a shared network (the alias case): its address as `/32`, or that network's subnet when the proxy container gets a new address on each re-creation (only containers on that network reach the app there). Move day checks the peers of the new app with `nsenter` + `ss` before `verify-limits.sh`.
- **Runbook: `CLUB_TZ` is the old app's effective value**, from the old `.env`, else the container's environment (the Python default `UTC` when it was never set): the import then keeps the times as the members saw them. Both the club's and each account's zone can be changed in the app afterwards. CUT2 had assumed `Europe/Berlin`; M3 records which it is.
- **Runbook: reports and `compare.sh` stay in root-only files** (`umask 077`), and my terminal sees `summary` (entity lines, `lookup wait`, `cover search`, each section as a line count) and `compare.sh`'s `MISMATCH`, `accounts:` and `result:` lines. The dry/real diff compares the summaries. The `notes` section is a no-go above 0 lines (CUT2 had none), and then Rafael reads `dry.txt` on the host.
- **Runbook: the `sqlite3` stand-in.** CUT2's container ran `compare.sh` whole, with `psql` inside, which cannot reach the new stack's Postgres (an internal network, no published port). The runbook instead builds a small `sqlite3` image and points `compare.sh`'s `SQLITE3` at a wrapper; `psql` stays `docker compose exec -T postgres psql`. Installing `sqlite3` with apt stays the first choice.
- **Runbook: the restic repository** is created with `install -d -m 0700` first (Docker would create a missing bind source owned by root with the default mode), and a NAS share must take a `touch` as root (NFS root squashing).
- **The English message** is mine, from the German text of §5.6.

### Deviations from the plan

1. **`set_password`'s signature** is `set_password(conn: &mut PgConnection, passwords: &Passwords, username: &str) -> anyhow::Result<Option<String>>`, not `(conn, username) -> sqlx::Result<Option<String>>`: hashing fails with `anyhow` (`Busy` inside it), and taking the hasher lets the tests use the cheap test parameters (`app.state.passwords`) while the CLI passes `Passwords::new()`, the hasher sign-up uses in production. The intent (one function, one transaction, shared with the confirm) is unchanged.
2. **Additions beyond the listed tests:** `email::is_mail_off` and its unit test; `auth::reset`'s generator test; the CLI parse test for `set-password`; `auth_set_password.rs` has six tests where §5.2 lists four (plus a suspended account and the mail-off flow), and eight after the review round; `swKillSwitch.test.ts` also runs the worker and compares it with `deploy/legacy-origin/sw.js`.
3. **`host-facts.sh` prints more than §5.4 lists:** the four keys from the container's environment as well as from `.env` (the Python compose file passes them with defaults, so an unset `.env` key still has an effective value), the networks with aliases and their containers (for the alias case), the database size (for the 64 MB `/tmp`), the listeners on 80 and 443 (is the proxy on this host?), Docker's root, and the stop checks with an exit status.
4. **`TRUSTED_PROXIES` is not always the old peer as `/32`** (see the decision above): for a proxy on the same host over loopback the new network's gateway, and for a proxy container possibly its network's subnet.

### For the integrator

**`docs/phase1/conventions.md` §3** (the `AppState::new` sentence): "`LogEmail`, `SmtpEmail` or `NoEmail` by `EMAIL_TRANSPORT`" instead of "`LogEmail` or `SmtpEmail` by `EMAIL_TRANSPORT`".

**`conventions.md` §9**, in the code block after `SmtpEmail`:

```rust
pub struct NoEmail;         // EMAIL_TRANSPORT=none (MOVE #52): every send fails with MAIL_OFF and logs nothing
pub const MAIL_OFF: &str = "mail is off (EMAIL_TRANSPORT=none)";
pub fn is_mail_off(error: &anyhow::Error) -> bool;   // NoEmail's refusal: a bare &'static str equal to MAIL_OFF
```

and a bullet after "Headers":

- **Mail off** (MOVE #52, the private move): `EMAIL_TRANSPORT=none` sends nothing until a mail provider exists. `NoEmail` logs no recipient, subject, body or link, so a reset link never reaches a log outside DEV_MODE (§19). `auth::mail::deliver` logs `mail not sent: mail is off` at INFO with the kind only (other failures WARN) and returns false, so callers treat the mail as not sent: the reset request voids its link, sign-up still answers 201 with its verification token unreceived. `admin::notify` and `admin::notices` mail verified addresses only, and with mail off none can be verified. Password resets go through `bookclub-server set-password` (§16).

**`conventions.md` §11**, the `EMAIL_TRANSPORT` row:

| `EMAIL_TRANSPORT` | `log` if `DEV_MODE`, else `smtp` | `log` is refused at startup without `DEV_MODE`: `LogEmail` writes every mail, reset links included, to the log (§19). `none` (MOVE #52) sends nothing (`NoEmail`, §9), also without `DEV_MODE`, and needs no `SMTP_URL` or `EMAIL_FROM`. Another value: `EMAIL_TRANSPORT must be log, smtp or none, not "…"` |

the `SIGNUP_MODE` row: "`invite` (closed beta) or `open` (#55). `open` needs mail: with `EMAIL_TRANSPORT=none` startup stops with `SIGNUP_MODE=open needs mail: set EMAIL_TRANSPORT=smtp`, checked before the legal texts (`LEGAL_DIR`)"; and in "Details fixed in Wave 1": "`SMTP_URL` and `EMAIL_FROM` are required when the transport is `smtp`, and ignored for `none`."

**`conventions.md` §16**, in "The CLI" after `unsuspend`: "`set-password <username>` (MOVE #52, the password reset without mail): `auth::reset::set_password(conn, passwords, username)` gives the account 20 random characters of the invite-code alphabet, hashed with `Passwords::new()`, and in one transaction does what the reset confirm does (`replace_password`: `password_hash`, `password_changed_at`, every session and bearer token deleted, the open reset and verification links voided). It prints only the password on stdout; an unknown name exits 1 with `error: No account with that name`; a suspended account stays suspended. The running server's in-memory login failures stay (another process) and empty within 10 minutes." And in the `send-test-mail` bullet: "… and exits 1 with `error: SMTP send failed: …` when the server refuses, or with `error: mail is off (EMAIL_TRANSPORT=none)` when mail is off."

**`docs/phase1/endpoint-map.md` §10**, two lines:

- `send-test-mail <address>` (DEP #51: one test mail through the configured transport; exit 1 when the SMTP server refuses, or with `EMAIL_TRANSPORT=none` (MOVE #52); no database)
- `set-password <username>` (MOVE #52: the password reset without mail; prints only a new random password of 20 characters, ends every session and app token of the account and voids its open links; exit 1 `No account with that name`)

**`CLAUDE.md`**, in "Run a single test", the list of test files that read docs and deploy files: "`private_move` (`docs/phase3/private-move.md`, and `deploy/cutover/host-facts.sh` against stand-ins for docker and curl)". In the "Sign-up needs an invite" bullet after `unsuspend`: "… and `set-password <username>` prints a new password for an account (the reset without mail; `EMAIL_TRANSPORT=none` turns mail off outside DEV_MODE)". In the Phase 3 paragraph, if one is added: "`docs/phase3/private-move.md` is the runbook of the private move."

**`README.md`** (top level, the integrator's): nothing from M1; CUT-R rewrites it (§5.8).

**For #53 only** (the later privacy text): with mail off no processor receives an address; an address entered in the prompt is stored unverified and is used only once mail exists.

### Checks (2026-10-30)

**The gate**, in `../bookclub-move` with `DATABASE_URL` on 5463 (`bookclub-pg-move`), all green:

- `cd server && cargo sqlx database reset -y && cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked && cargo sqlx prepare --check -- --all-targets`: 1505 tests passed, 0 failed (367 library unit tests, 6 binary unit tests, 92 test files).
- `cd frontend && npm run typecheck && npm test && npm run build && npm run check:api`: 74 files, 506 tests passed; `dist/sw.js` is in the build; `generated.ts` unchanged.
- `docker run --rm -v "$PWD":/mnt -w /mnt koalaman/shellcheck:v0.10.0 deploy/hosted/*.sh deploy/cutover/*.sh`: clean.
- `docker compose -f deploy/hosted/compose.yaml --env-file deploy/hosted/env.example config -q`: exit 0.
- M1's acceptance subset: `cargo test --test auth_set_password --test deploy_env` and the `config.rs` unit tests pass; `npx vitest run src/swKillSwitch.test.ts src/thirdParty.test.ts` passes (11 tests).

**The M1 boot test on 8095** (§2.3), with the image built from this branch (`docker build --build-arg CARGO_JOBS=3 -t git.tillhome.de/rafiistcool/bookclub-server:move-test .`) and an env file in the scratchpad: `COMPOSE_PROJECT_NAME=bookclub-move`, `BOOKCLUB_TAG=move-test`, `BOOKCLUB_ENV_FILE` and `BOOKCLUB_DATA_DIR` in the scratchpad (`media` and `legal` made with `install -d`), `BOOKCLUB_LISTEN=127.0.0.1`, `BOOKCLUB_PORT=8095`, `BOOKCLUB_RESTART=no`, `EMAIL_TRANSPORT=none`, no `SMTP_URL` or `EMAIL_FROM`, `LEGAL_DIR=/app/legal` (empty), `TRUSTED_PROXIES=none`, the rest as in `ci.env` with `.invalid` names.

| Check | Result |
|---|---|
| `docker compose --env-file … -f deploy/hosted/compose.yaml up -d --wait --pull never` | exit 0, both services healthy |
| `GET /api/health` | `{"status":"ok"}` |
| `send-test-mail x@example.com` | exit 1, `error: mail is off (EMAIL_TRANSPORT=none)` |
| `beta-invite`, then `POST /api/auth/register` with its code | 201, `email_verified: false`; the log has one INFO `mail not sent: mail is off kind=Link` and no address |
| `set-password <that name>` | exit 0, 20 characters on stdout; the log line names no account |
| login with that password / with the old one | 204 / 401 |
| `set-password nobody` | exit 1, `error: No account with that name` |
| `curl -sI …/sw.js` | 200, `content-type: text/javascript`, the shell's CSP |
| `SIGNUP_MODE=open` in the env file, `up -d --wait app` | the app exits 1 at startup with `error: SIGNUP_MODE=open needs mail: set EMAIL_TRANSPORT=smtp` |
| teardown | `down -v`, `docker image rm` of `move-test`, the data directory removed, `docker builder prune -f` |

## Review round (2026-10-30)

The review of M1 had 14 findings, on mail off and on the runbook. Each was checked against the code first. All 14 are fixed. Two carried a sub-claim that the code contradicts; the fix follows the code (rows 2 and 3).

| # | Finding | Verdict | Fix |
|---|---|---|---|
| 1 | README: mail off voids verification links (nit) | Confirmed: only `auth::reset::request_reset` voids its token when `deliver` returns false; sign-up, resend and the address change ignore the result | "Requirements", "Or mail off": reset links are voided, a verification link stays open, unreceived, until it expires |
| 2 | No test for `deliver`'s mail-off arm or for `admin::notify` with `NoEmail` (nit) | Both gaps confirmed. Not so: "`admin_notify.rs` has no failed-send case": `news_waits_until_a_mail_goes_out` runs the job with a refusing transport. Only `NoEmail` itself was never run there | Two tests in `auth_set_password.rs` (MOVE's only test file; `admin_notify.rs` is not MOVE's). `with_mail_off_a_mail_is_one_info_line_with_the_kind_only` captures the log of one `deliver` call with a state from `AppState::new` and `EMAIL_TRANSPORT=none`: one INFO line with `kind=Link`, no WARN, address, subject or link. `with_mail_off_admin_news_stays_unstamped` runs `admin::notify::run` with mail off (no mail, nothing stamped), then with a transport that delivers (the digest goes out). With the arm deleted, the first fails (`WARN … sending mail failed`); with `notify::send` counting a failed send as sent, the second fails |
| 3 | The go/no-go asks for CUT2's exact counts (major) | Confirmed: CUT2 read an older copy. Not so: "an `isbn error` row gets no edition": `unmatched_isbn` gives it a bare-facts edition | 7.3 and the keep/drop row: exactly `users 2 → 2` and the club with `$SLUG` and 2 members; every other line carries its source over or names the difference (`merged`, `dropped`, `with a dangling reference`); works are books minus `merged` (`importer/books.rs`: each row is merged or owns a new work); editions at most the `isbn keys` lines; CUT2's counts are a floor. A lookup that answered in one run only may move `books`, `editions`, `merges` and `differences` between the runs; the real run then goes through the go once more. The old text also let the `isbn keys` line count differ between runs, which it cannot (one line per ISBN row) |
| 4 | M6's `docker compose down` takes the old project's other services and networks (major) | Confirmed: `SELFHOST.md` puts the proxy in the same compose project | `docker compose rm --stop --force bookclub`. Section 8 and the M6 text add what follows when `$OLD` keeps other services: a `compose up` there brings the Python app back, and in the alias case no port conflict stops it |
| 5 | The loopback binding is chosen from the peer (major) | Confirmed: a proxy container that dials a host address shows a Docker gateway too (Docker masquerades it), and a `127.0.0.1` binding refuses it | 2.3: `BOOKCLUB_LISTEN=127.0.0.1` only for an upstream on `127.0.0.1` or `localhost`, from a proxy process on the host (not `docker-proxy`); the upstream is read as addresses only (`grep -rhoE "[][A-Za-z0-9.:-]+:<old port>\b"`, tried on sample nginx and Caddy files with credentials: only `host:port` tokens print) or Rafael names it. Every other proxy keeps the old binding; `[::1]` gets the one-line upstream change. `TRUSTED_PROXIES` still follows the peer: a Docker gateway becomes the new network's gateway (also with the old binding kept), another address of this host stays |
| 6 | "not searched (database error)" is not on the `cover search` line (minor) | Confirmed: it is in the per-book lines of `without a cover` (`importer/covers.rs`) | `grep -c ': not searched (database error)$'` on `dry.txt` and `real.txt` must print 0 |
| 7 | The importer's log reaches my terminal with ISBNs (minor) | Confirmed (`importer/books.rs:556`, `importer/rematch.rs:94,101,104`; also `catalog/service.rs:166`, `catalog/detail.rs:360`) | stderr goes to root-only `dry.log`, `real.log`, `rematch-dry.log`, `rematch.log`; the terminal sees `grep '^error:'` and `grep -c ' WARN '`; section 9 deletes them with the reports |
| 8 | `SQLITE3` is not set on move day (minor) | Confirmed: `compare.sh` falls back to `sqlite3` and stops | The stand-in's path is appended to `vars.sh` |
| 9 | The watchtower check prints its whole command line (minor) | Confirmed | Only `--label-enable`, `--include-stopped`, `-S` and `--revive-stopped` print (tried on a sample with a token and a notification URL) |
| 10 | Alias case: a proxy that resolved the alias at its start keeps the old address (minor) | Confirmed for nginx without a `resolver` | The alias branch says so, and the reload joins move day and the rollback. 7.5 checks `https://$HOST/api/health` through the proxy before Rafael signs in, which also catches a binding that does not fit (row 5) |
| 11 | The M2 digest check needs a registry login (minor) | Confirmed: the VM has no `~/.docker/config.json` | The forgejo MCP's `list_package_files` for `0.11.0`, `0.11` and `stable`: each version lists `manifest.json` with the manifest digest as its `sha256` (read on `0.11.0-rc.1`) |
| 12 | Host blocks depend on an earlier `cd` (nit) | Confirmed | `cd "$NEW"` in 3.2's gateway block and 7.5's rematch and peer blocks; the checklist's query says "in `$NEW`" |
| 13 | M6 removes the image by ID (nit) | Confirmed: refused with more than one tag | Every tag in one `docker image rm`, the ID when there is none |
| 14 | M6 accepts last week's passing restore test (nit) | Confirmed. The proposed "last `restore test` line" does not work: a failed run logs `error: …` without those words | The block runs `systemctl start bookclub-restore-test` and deletes only when that run passes; the 8-day journal check stays, to show the timer runs |

**Deviations from the plan added by this round** (numbered on from the list above):

5. **M6** (§5.8: "then `docker compose down` in `$OLD`, its `data/` and the Python image go"): only the `bookclub` service's container goes, the image by its tags, and the block runs a restore test itself besides the 8-day check.
6. **The binding** (§5.4: "If the proxy runs on the same host and reaches the app over loopback"): "over loopback" is read from the proxy's upstream, never from the peer. `TRUSTED_PROXIES` for a Docker gateway peer is the new network's gateway also when the old binding stays.
7. **The go** (§5.6, the §3.3 row: "The go is CUT2's entity counts"): CUT2's counts are a floor; the go is that every line carries its source over.
8. **The alias case** (§5.4: "the rollback stays two commands"): with a proxy that keeps the alias's old address, a reload joins move day and the rollback.

**Checks of this round:** the gate, in `../bookclub-move` with `DATABASE_URL` on 5463, all green: the server chain from `cargo sqlx database reset -y` to `cargo sqlx prepare --check -- --all-targets` exits 0 with 1518 tests passed and 0 failed in 96 test binaries; the frontend chain exits 0 (74 files, 506 tests, `check:api` unchanged); shellcheck on `deploy/hosted/*.sh deploy/cutover/*.sh` is clean; `docker compose … config -q` exits 0. The runbook's 28 bash blocks pass `bash -n` (placeholders replaced). The M6 block passes shellcheck and was run with stand-ins for `journalctl`, `systemctl`, `docker` and `backup.sh`: no pass in 8 days, an old snapshot, no `bookclub.db` and a failing restore test each stop with the data kept and no `docker` call that removes anything; all fine removes the `bookclub` container, the data and the image by both of its tags, or by ID when it has none.

## Checks (2026-10-30)

After the review round, two checks ran on 6d5fb11: the image boot test and a security review of the runbook.

**The image boot test** (M1's acceptance, §2.3): passed, no failed step, no bug. In short: the image built from the branch as `move-test`; the stack up on 8095 with `EMAIL_TRANSPORT=none` and no `SMTP_URL` or `EMAIL_FROM`; `/api/health` answers `{"status":"ok"}`; `send-test-mail` exits 1 with the mail-off message; `beta-invite`, then a sign-up with its code answers 201; `set-password` prints a password that signs in; `/sw.js` comes as JavaScript; `SIGNUP_MODE=open` stops the app at startup with the mail message; teardown (`down -v`, the tag, the data directory, `docker builder prune -f`). No step failed, so none was rerun.

**The security review**: five candidates, each confirmed by two of three verifiers, none rejected. Each was checked against the branch before the fix.

| # | Candidate | Verdict | Fix | Commit |
|---|---|---|---|---|
| 1 | The rollback's step 3 runs `docker compose down -v` and `find "$DATA/media" -mindepth 1 -delete` as prose, with no `vars.sh`, `cd` or guard (filed medium) | Confirmed, at low: it takes a session without `vars.sh`, against section 0's rule. Not so: "the only destructive step not in a block that sources `vars.sh`": section 7's blocks rely on the same per-session rule. What sets step 3 apart is that an empty `$DATA` turns it into `find /media … -delete` as root, an existing directory where a NAS share with the backups may be mounted; elsewhere an empty variable gives a harmless path or a failing command | Every rollback step is a block that loads `vars.sh` and changes into its own directory (`cd "${NEW:?}"`, `cd "${OLD:?}"`). The delete is a `bash -s` block under `set -euo pipefail` that stops unless `$NEW` holds `backup.env` and `compose.yaml` and its `MEDIA_DIR` is `$DATA/media` | 534ebb3 |
| 2 | A rollback keeps the root key and the import copy on the host for as long as the move waits (low) | Confirmed for the key: it has no passphrase, every session on the agents VM can read it, and only M5's last step removed it. The import copy, reports and logs add little (the live `bookclub.db` next to them holds the same and more) but go too | Step 6: the copy, the reports and the logs are deleted once the reason is in #52; the key goes unless the next attempt has a date within 7 days, checked from the agents VM as in section 9 | 534ebb3 |
| 3 | The rollback deletes the new database without checking that the window is open (low) | Confirmed: "until the other member writes" had no check, and the only backup of the imported data was 7.6's, from before "done" | Step 1 counts, without names, the accounts other than the club's owner that signed in (a session, or `last_seen_at` for one that ended); above 0 the rollback waits for Rafael, who asks the member. The delete block runs `backup.sh` before `down -v`, so a wrong judgement can be undone from the backup | 534ebb3 |
| 4 | The M6 cleanup checks only that `OLD_DATA` holds `bookclub.db` before `rm -rf` (low) | Confirmed, for an unusual layout: `OLD_DATA` is the old `/data` mount source, and `BOOKCLUB_DATA=/srv` holds `/srv/bookclub` and the fallback repository `/srv/bookclub-backup`. `host-facts.sh` checked only whether the old data lay inside `/srv/bookclub`, not the reverse. The Postgres volume would survive; the pictures and a local repository would not | The block stops, before its restore test and before any removal, when `OLD_DATA` equals or holds `$NEW`, a path of `backup.env` (`COMPOSE_DIR`, `MEDIA_DIR`, `BACKUP_DIR`, a local `RESTIC_REPOSITORY`, `RESTIC_PASSWORD_FILE`; `backup-lib.sh`'s defaults when unset) or Docker's root, compared after `realpath`. Section 2.3 says the layout and the repository go outside the old data directory (`DATA=/var/lib/bookclub` then). `host-facts.sh` prints a note when the old data directory holds `/opt/bookclub`, `/srv/bookclub`, their `-server` forms or `/srv/bookclub-backup` | e5ac968 |
| 5 | `summary` prints the part of a book title after a line break (low) | Confirmed in a narrower form; the code contradicts the filed path. A legacy or Open Library title cannot carry a line break into the report: every work the importer writes goes through ingest's `CleanWork` (`clean_line`, `collapse_ws(nfc(…))`), and the report reads titles back from `works` (`covers::summarize`, `isbn_work`, `rematch::upgraded`). Left: `ingest::admin_text` keeps inner line breaks, so an admin edit sent as a hand-made API request (the SPA's fields are single-line) between `make-admin` and `rematch-legacy` on move day could reach a summary. The old filter also took any line ending in `:` for a section title | `summary` became an allow-list: before the first known section title it prints only entity lines (`^[a-z_ ]+ (-\|[0-9]+) → `), `normalised`, `lookup wait`, `cover search` and a dry run's header; from the first title on only titles and counts. Anything else is `other lines N`, and Rafael reads the report on the host. Escaping control characters in `importer/report.rs` would close the path at its source, but that file is not MOVE's; it is left to the importer's owner | f3e0f23 |

**Tests.** `server/tests/private_move.rs` reads the blocks from the runbook as they stand and runs them with real `bash`, `awk`, `find` and `realpath`, and `docker`, `journalctl`, `systemctl` and `curl` as bash functions that log their calls:

- `summary_prints_no_line_of_a_section`: a report with titles and authors broken over lines, from a file and from stdin, and a dry run's header as `importer::Report` prints it;
- `the_rollback_window_counts_sign_ins_besides_the_owner` (`#[sqlx::test]`): 0 on an empty target, after the import, and with Rafael's session; 1 with the other member's session and after it ended;
- `the_rollback_deletes_only_the_new_stack_and_only_after_its_backup`: `backup.sh` before `docker compose down -v`, the media emptied, the import copy kept for step 6; nothing deleted and no `docker` call when the backup fails, `vars.sh` is missing or lacks `DATA`, `$NEW` is the old app's directory, or `MEDIA_DIR` is not `$DATA/media`;
- `the_cleanup_keeps_an_old_data_directory_that_holds_the_new_stack`: deletes with a separate layout and with an `sftp:` repository; stops with the data kept, no restore test started and no removal when the old data directory holds the pictures, the repository, the new stack, `BACKUP_DIR`, Docker's root, or all of them;
- `host_facts_notes_an_old_data_directory_that_holds_the_new_layout`: `/srv` holds three of the five paths, `/` all five, a directory of its own none.

All five fail on the runbook and `host-facts.sh` of 6d5fb11 (checked by putting those two files back).

**Deviations added by this round** (numbered on from the review round):

9. **The rollback** (§5.7: four steps, "No gate is needed"): six steps, each a block. First a counts-only check that the window is open, then the two stops and starts, a backup before `down -v`, the message, and the leftovers. §5.8 and answer 48 remove my key as M5's last step; after a rollback without a date within 7 days it goes earlier, and the next attempt starts at 2.1 with a new key. The 7 days are my choice; Rafael can name another limit.
10. **M6** (§5.8): besides the backup checks, the block stops when the old data directory holds a path of the new stack or Docker's root. Section 2.3 and `host-facts.sh` catch the layout before anything is installed.
11. **A second test file**, `server/tests/private_move.rs`, which §2.3 does not list. The runbook's blocks are code that runs as root on the production host, and `auth_set_password.rs` is about the server; a new file conflicts with no other package.

**The gate after the fixes**, in `../bookclub-move` with `DATABASE_URL` on 5463: all green on e5ac968. The server chain from `cargo sqlx database reset -y` to `cargo sqlx prepare --check -- --all-targets` exits 0: 1512 tests passed, 0 failed (367 library unit tests, 6 binary unit tests, 1139 in 93 test files, among them `private_move`'s 5). The frontend chain exits 0 (74 files, 506 tests; `dist/sw.js` built; `check:api` leaves `generated.ts` unchanged). shellcheck on `deploy/hosted/*.sh deploy/cutover/*.sh` is clean, and `docker compose … config -q` exits 0. The runbook's 33 bash blocks pass `bash -n` (placeholders replaced); the rollback's blocks and the bodies of its `bash -s` block and of the cleanup block pass shellcheck (SC1091, the sourced `vars.sh`, excluded). The review round's 1518 was a miscount: `server/` differs from 6d5fb11 only by `private_move.rs`, so 6d5fb11 runs 1507, and every test file runs as many tests as it declares.

## M2–M6

Filled in when they run: the M1 boot test on the merged `phase-3`, the release's digests, the facts found on the host (no secrets, no usernames), the layout and project name chosen, the backup target, the move's counts against the CUT2 baseline, the phone test's result, CUT-R, and the cleanup.
