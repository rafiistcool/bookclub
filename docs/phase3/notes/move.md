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
| `server/tests/auth_set_password.rs` | **New.** Six tests (below). |
| `server/.sqlx/` | One new query (`SELECT id FROM users WHERE username = $1 FOR UPDATE`). The confirm's queries moved into `replace_password` unchanged, so their files stayed. |
| `frontend/public/sw.js` | **New.** The kill switch for the Python app's offline worker: `deploy/legacy-origin/sw.js` with its own header comment. |
| `frontend/src/swKillSwitch.test.ts` | **New.** Three tests (below). |
| `deploy/cutover/host-facts.sh` | **New.** The read-only facts of §5.4, shellcheck-clean. |
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
2. **Additions beyond the listed tests:** `email::is_mail_off` and its unit test; `auth::reset`'s generator test; the CLI parse test for `set-password`; `auth_set_password.rs` has six tests where §5.2 lists four (plus a suspended account and the mail-off flow); `swKillSwitch.test.ts` also runs the worker and compares it with `deploy/legacy-origin/sw.js`.
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

**`CLAUDE.md`**, in the "Sign-up needs an invite" bullet after `unsuspend`: "… and `set-password <username>` prints a new password for an account (the reset without mail; `EMAIL_TRANSPORT=none` turns mail off outside DEV_MODE)". In the Phase 3 paragraph, if one is added: "`docs/phase3/private-move.md` is the runbook of the private move."

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

## M2–M6

Filled in when they run: the M1 boot test on the merged `phase-3`, the release's digests, the facts found on the host (no secrets, no usernames), the layout and project name chosen, the backup target, the move's counts against the CUT2 baseline, the phone test's result, CUT-R, and the cleanup.
