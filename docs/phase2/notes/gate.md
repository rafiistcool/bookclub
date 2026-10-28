# GATE notes (#55, part of #51)

Owner: GATE. These notes record what the hardening package built, the contract changes, what the integrator moves into the docs, and the Caddy and `verify-limits.sh` instructions for `deploy/hosted/README.md` (work plan §0 item 16).

## What changed

- **A CSP for the SPA shell.**
  - `server/src/spa.rs`: every answer of the SPA fallback (the shell, `/assets/*`, the other built files, and a missing asset's JSON 404) carries `spa::SHELL_CSP`:

    ```
    default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; manifest-src 'self' blob:; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'
    ```

    `manifest-src 'self' blob:` is there for the `blob:` manifest that `frontend/src/brand.ts` builds (§0 item 11). `img-src` allows `data:` and `blob:` as the work plan specifies; the app uses neither for pictures today (only `brand.ts` calls `URL.createObjectURL`, for the manifest), so a local picture preview could be added without touching the policy.
  - `server/src/lib.rs`: the outer `Content-Security-Policy: frame-ancestors 'none'` layer is `SetResponseHeaderLayer::if_not_present` instead of `overriding` (§0 item 12). The SPA's answers keep the shell policy, which also says `frame-ancestors 'none'`; every other answer gets the framing-only policy as before. `X-Frame-Options`, `X-Content-Type-Options` and `Referrer-Policy` stay `overriding`.
  - `frontend/index.html`: the inline theme script moved to `frontend/public/theme-init.js`, loaded with `<script src="/theme-init.js"></script>` in `<head>`: a classic script, neither `async`, `defer` nor a module, so it runs before `<body>` is parsed. The code is unchanged. Vite copies it to `dist/` as is and leaves the tag alone (no build warning).
  - vue-i18n needs no `'unsafe-eval'`. vue-i18n 11.4.10 compiles messages to an AST and interprets them (JIT); neither its bundles (`vue-i18n.mjs`, `@intlify/core-base`, `@intlify/message-compiler`, `@intlify/shared`) nor any chunk of the built `dist/assets/*.js` contains `new Function` or `eval(`. The browser run below rendered every page, with interpolated messages, under `script-src 'self'` without a violation.
- **`frontend/src/thirdParty.test.ts`** (sources only, §0 item 14). It scans `index.html`, every `.js`, `.svg`, `.html` and `.css` file in `public/`, the manifest's strings, and `src/**/*.{css,vue,ts}` without the tests and `src/test/`. A load from another origin fails the test: `url(…)`, `@import`, `<link>`, `<script>`, the media elements' `src`, `srcset` and `poster`, `<use href>`, and `fetch`, `WebSocket`, `EventSource`, workers, beacons and dynamic `import()` with an absolute or protocol-relative URL. A link the reader clicks (`<a href>`, the Open Library page) is not a load. A self-test pins what the scanner finds. Two more checks guard the CSP: `index.html` has no inline script, no `on…` attribute and no `javascript:` URL, and the theme script is a synchronous classic script in `<head>` that reads the keys the stores write. 8 tests.
- **`server/tests/proxy_client_ip.rs`**: 6 tests through the router with a `ConnectInfo` peer.
  - `sign_ups_count_per_forwarded_client`, `logins_count_per_forwarded_client`, `reset_requests_count_per_forwarded_client`: behind a trusted proxy, client A gets 429 and client B, behind the same proxy, does not.
  - `forwarded_for_from_an_untrusted_peer_counts_against_the_peer`, `without_trusted_proxies_forwarded_for_is_ignored`: a fresh `X-Forwarded-For` per request from an untrusted peer buys no fresh budget.
  - `two_trusted_hops_name_the_right_most_untrusted_address`: forged left-hand entries are ignored, the inner proxy is skipped, and the chain may come in several headers.
  - Mutation checks, each reverted (`server/src/client_ip.rs` is unchanged in the commit):
    - `ClientIp` = the raw `ConnectInfo` peer: 4 of 6 fail (the sign-up, login, reset and two-hop tests). The two untrusted-peer tests pass, as they should: for them the peer is the right answer.
    - every peer trusted: 3 fail (both untrusted-peer tests and the two-hop test).
    - left-most hop instead of right-most untrusted: the two-hop test fails.
- **`server/tests/platform.rs`**: `the_spa_carries_the_shell_policy` (new) checks exactly one `Content-Security-Policy` header with the shell policy on `/`, `/index.html`, a client route, a nested `index.html`, a file, an asset, a missing asset and a HEAD, and the framing-only policy on `/api/health`, an unknown API path, `/media/…` and the 405. `assert_security_headers` now also requires a single policy header. With the lib.rs layer back on `overriding` the new test fails (the shell gets `frame-ancestors 'none'` only), and with `appending` too (two headers). 14 tests.
- **`deploy/hosted/Caddyfile.example`** and **`deploy/hosted/verify-limits.sh`**: see the instructions below.
- **`docs/phase1/security-review.md`**: the Q17 record in §3 (open, recommendation A, waiting for Rafael) and a line under §10 "Not taken" saying that Phase 2 adds the shell CSP.

## Contract changes

- Headers only: the SPA fallback's answers carry `spa::SHELL_CSP` instead of `frame-ancestors 'none'`. API, `/media` and fallback JSON answers are unchanged.
- No new route, DTO field, `detail` string, environment variable, CLI command or migration. `server/.sqlx/` and `frontend/src/api/generated.ts` are unchanged (regenerated, no diff).
- No change to `server/tests/common/**`.

## For the integrator: doc updates

- **`conventions.md` §3, layer 2:** "and, overriding too, `Content-Security-Policy: frame-ancestors 'none'`" becomes: "`Content-Security-Policy: frame-ancestors 'none'` only where the answer has no policy yet (`if_not_present`): the SPA fallback sets the full shell policy (`spa::SHELL_CSP`, #55), which also forbids framing."
- **`conventions.md` §3, Fallbacks**, and **`endpoint-map.md` §1, the SPA catch-all row** (`:114`): add "Every answer of the SPA fallback carries `Content-Security-Policy: <SHELL_CSP>` (scripts, styles, pictures, fonts, fetches and the manifest from the own origin only; `data:` and `blob:` pictures, a `blob:` manifest, inline styles; no plugins, no framing)."
- **`CLAUDE.md`, "Cross-cutting rules enforced by tests":** suggested bullet: "**No third party, no inline script:** the browser loads nothing from another origin, and the shell's CSP (`server/src/spa.rs`) allows scripts from `'self'` only. `frontend/src/thirdParty.test.ts` fails on an external `url()`, `@import`, `<link>`, `<script>` or `fetch` in the sources and on an inline script in `index.html`; the theme boot script is `frontend/public/theme-init.js`."
- **`frontend/src/stores/theme.ts:70`** says "the boot script in index.html": now `public/theme-init.js`, loaded by `index.html`. Not GATE's file; a one-word fix for whoever touches it next.
- **`docs/phase1/spec/frontend.md:571`** describes the inline script of the Python-era app. It is a spec of the old behaviour; leave it, or add "(now `public/theme-init.js`)".
- **`deploy/hosted/README.md`** (DEP's): add the two sections below.
- **#51 / Q16 amendment** (§0 item 15): `verify-limits.sh` checks sign-up, login and reset per address. Search is limited per account in the app (`catalog_user`, 60 a minute, `server/src/catalog/search.rs`), so the script does not cover it.

## Caddy (for `deploy/hosted/README.md`)

`deploy/hosted/Caddyfile.example` is the site block for the Caddy that already serves git.tillhome.de.

1. Copy the block into that Caddyfile, or `import` the file. Set two variables in Caddy's environment:
   - `BOOKCLUB_HOST`: the public host name.
   - `BOOKCLUB_UPSTREAM`: the app VM's address and published port, e.g. `10.0.20.5:8080`. Without it the block proxies to `127.0.0.1:8080`.
2. In the app's env file, set `TRUSTED_PROXIES` to this Caddy's address as the app VM sees it, with `/32` (Q7). If it is wrong, every user lands in Caddy's one rate-limit bucket, and one client can lock everyone out of sign-up and reset.
3. Check and load: `caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile`, then `caddy reload`.
4. From home, run `verify-limits.sh` (next section), then make one sign-up attempt from a second network, such as a phone on mobile data. That attempt must not get 429.

What the block does:
- `request_body max_size 51MiB` (53,477,376 bytes). The app answers 413 itself, from `Content-Length` and before reading the body, above 52,432,896 bytes, so a client always gets the app's JSON 413. Caddy's limit is the backstop for bodies it has to read, such as chunked ones. Caddy's `50MB` (50,000,000 bytes) would cut off uploads the app accepts.
- HSTS `max-age=31536000` without `includeSubDomains` (add it, and consider `preload`, once the final domain is in place), a `Permissions-Policy` that denies camera, microphone, location, payment and motion sensors (clipboard and Web Share keep their defaults, because the invite pages use them), and no `Server` header.
- It does not repeat the headers the app sets: CSP, `X-Frame-Options`, `X-Content-Type-Options`, `Referrer-Policy`, `Cache-Control`.
- zstd and gzip for `/assets/*` only. API answers mix personal data with text a client sent, the pattern compression side channels need.
- `reverse_proxy` with an active health check on `/api/health` every 15 s. While the app is down or restarting, Caddy answers 503 for up to one interval after the app is back.
- An access log in `/var/log/caddy/bookclub-access.log`, rolled at 50 MiB and kept 7 days (`roll_keep_for 168h`). It holds client addresses, which the privacy text must state. Caddy leaves out `Cookie` and `Authorization`. The filter replaces invite codes in the URI with `{code}` (`/join/{code}`, `/api/invites/{code}/…`, `?code={code}` on `/signup` and `/register`) and drops `Referer`, which repeats such URLs. The app's own log already leaves codes out.
- A commented `rate_limit` block for Q16 option B. It needs a Caddy built with `github.com/mholt/caddy-ratelimit` (xcaddy) and `order rate_limit before basicauth` in the global options.
- Caddy replaces any `X-Forwarded-For` an untrusted client sends with the client's address. The app reads it only from `TRUSTED_PROXIES` and takes the right-most untrusted address.
- For a local test, uncomment `tls internal`, or put `local_certs` in the global options.

## `verify-limits.sh` (for `deploy/hosted/README.md`)

```bash
deploy/hosted/verify-limits.sh https://<host>
# with `tls internal`:
VERIFY_LIMITS_CURL_FLAGS=--insecure deploy/hosted/verify-limits.sh https://<host>
```

- It needs bash and curl, and creates nothing:
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
- **It uses up the caller's budgets:** sign-up and reset for an hour, login for 10 minutes. Run it once, and do not run it from an address that people need to sign in from right away. A second run within the hour fails at the first check with a hint that the address is already limited.
- Search is limited per account inside the app, not per address, and is not checked.

## Q17 (security review §3)

Recorded in `docs/phase1/security-review.md` §3 as open, with recommendation A (per account, as today) and the alternative B (per account and client address, under a per-account ceiling). Under A nothing changes. Under B, ADM2 changes `auth/login.rs` and `ratelimit.rs` with tests in `auth_login.rs`. Record the answer and its date there.

## What I verified by hand

**Browser run** against the built SPA from this worktree's server (`STATIC_DIR=../frontend/dist BIND_ADDR=0.0.0.0:8082 PUBLIC_URL=http://agents.lan:8082 DEV_MODE=true`, first account from `beta-invite`, made admin with `make-admin`), headless Chromium through the `playwright-cli` skill. An init script logged every `securitypolicyviolation` event and the theme attributes at the moment `<body>` was inserted. The clean scripted pass:
- **Pages:** the join link, `/signup`, `/forgot-password`, `/login`, sign-in, a search on `/discover` (Open Library), a book page (`/book/37`), marking it Reading, a diary note (201), `/shelf`, `/settings` with an avatar upload (200), `/admin`, `/stats`, `/club`, `/clubs`, `/clubs/new` and `/`.
- **Earlier passes** also covered sign-up through the form, a cover photo upload (202) and the theme switch in Settings.
- **Result:** 0 CSP violations. The only console errors were the 5 expected 401s of `/api/auth/me` on the logged-out pages.
- **Manifest:** `Page.getAppManifest` (CDP) loaded the `blob:` manifest under the shell policy (name "Bookclub"). As a control, an added `manifest-src 'self'` policy blocked it with a violation, so the `blob:` source is needed.
- **Theme before first paint:** with `bookclub.theme=ink` and `bookclub.mode=dark` stored, every page had `data-theme=ink data-mode=dark` (and a black body background) when `<body>` was inserted, before the app's module script ran. As a control, with `/theme-init.js` blocked the same check saw `paper light`.

**Caddy smoke test**, not the end-to-end run with DEP's image:
- **Setup:** `caddy:2.10` on the host network with `Caddyfile.example` imported, `local_certs`, `BOOKCLUB_HOST=agents.lan:8082`, in front of this worktree's server on `127.0.0.1:18082` with `TRUSTED_PROXIES=127.0.0.1/32`.
- **Headers:** HSTS, `Permissions-Policy` and the app's headers present, the shell CSP on `/`, no `Server` header (Caddy adds `Via: 1.1 Caddy`).
- **Compression:** the JS bundle zstd-encoded, `/api/config` not.
- **413:** bodies of 52,500,000 and 53,500,000 bytes to `PUT /api/auth/me/avatar` both got the app's JSON 413 `That picture is too large (50 MB max)`.
- **`verify-limits.sh`:** exit 0 through Caddy. Afterwards a second client, a container on the docker bridge, still got 400 for sign-up, 204 for reset and 204 for login, while the first client got 429.
- **Access log:** `/join/{code}`, `/api/invites/{code}…`, `/signup?code={code}&lang=de`, no `Referer`, `?q=barcode=1` untouched.

**`verify-limits.sh` against other servers:**
- the server directly: exit 0;
- `python3 -m http.server`: exit 1 at the health check;
- a server with `TRUSTED_PROXIES=0.0.0.0/0, ::/0`, which trusts the forged header: exit 1, with the sign-up, IP login and reset checks failing;
- an address already used within the hour: exit 1 with the "already limited" hint;
- no argument or a non-HTTP URL: exit 2.

**Other checks:**
- `caddy validate` with the command from the work plan: valid. `caddy fmt` leaves the file unchanged.
- shellcheck (`koalaman/shellcheck:v0.10.0`) is clean.

## Left for after the Wave 1 merge

- The end-to-end run on `agents.lan:8082` with Caddy (`tls internal`) in front of DEP's staging stack and `TRUSTED_PROXIES` set to the Caddy container's IP/32, including two client containers and `verify-limits.sh` there (work plan GATE acceptance).
- The Playwright CSP run over LEG's `/imprint`, `/privacy`, `/terms` and ADM's new admin pages once they are on `phase-2`. LEG's legal fragments are served as HTML from `/api/legal/{doc}`. If the SPA inserts them with `v-html`, the CSP still applies: inline scripts and `on…` handlers in a fragment would be blocked, and inline `style` attributes stay allowed.

## Observations outside GATE's files

- `brand.ts` builds the `blob:` manifest with `start_url: "/"`, which Chrome ignores in a `blob:` manifest ("property 'start_url' ignored, URL is invalid"). An absolute `start_url` (`location.origin + "/"`) and `scope` would fix it. This is for the BRAND task or FE.
- `/theme-init.js`, like `favicon.svg` and the manifest, is served without `Cache-Control`, so browsers cache it heuristically. That is harmless while its code stays the same. If it ever changes together with the stores' keys, consider `no-cache` for the non-hashed files in `spa.rs`.
- The dev database's query plans can change sqlx's nullability inference. After the browser run had put data into the worktree's database, `cargo clippy` failed with `Option<…>` mismatches in `auth/mod.rs` (a `LEFT JOIN media` query). `cargo sqlx database reset -y` fixed it. Run `cargo sqlx prepare` against an empty, freshly migrated database.
