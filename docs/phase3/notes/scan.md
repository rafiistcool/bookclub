# SCAN (#56): barcode scanning on the web

Owner: SCAN. Branch `phase3/scan`, worktree `../bookclub-scan`, Postgres `bookclub-pg-scan` on 127.0.0.1:5461, browser port 8091. No migration: the block 3000–3099 stays unused. The design is `docs/phase3/work-plan.md` §3, with answers 5–15 in §6. These notes record what was built, the decisions and deviations, and what the integrator moves into `conventions.md`, `endpoint-map.md` and `CLAUDE.md`; the facts for the privacy text are for #53 only (§2.1 rule 11).

## Part 1: the server

Plan §3.6 and the server tests of §3.9. The SPA (the decoder, the camera, `BarcodeScanner.vue`, `/scan`, the add forms) is Part 2.

### What was built

| Where | What |
|---|---|
| `server/src/spa.rs` | `SHELL_CSP`: `script-src 'self' 'wasm-unsafe-eval'`; everything else unchanged. The doc comment says WebAssembly compiles from our own `/assets` file only, for the barcode decoder, and `'unsafe-eval'` stays out. |
| `server/src/catalog/isbn_lookup.rs` | `lookup_isbn` takes `upstream_turn(&state, user.id)` right after `parse_isbn`. The utoipa 429 text adds `Slow down a little. Try again later.` (60 catalog requests per account a minute); the handler's description says a valid ISBN counts like a search, also when the answer is local. |
| `server/src/lib.rs` | `loggable_path` writes `/api/books/isbn/<anything>` as `/api/books/isbn/{isbn}`, beside `{code}` for invite codes; the doc comment gives both reasons. |
| `deploy/hosted/Caddyfile.example` | `Permissions-Policy`: `camera=()` became `camera=(self)`, the rest unchanged; the comment says the app uses the camera on its own origin for barcode scanning (#56). The log filter is `^(/(?:api/invites|api/books/isbn|join)/)[^/?]+|([?]).*$`, so the path logs as `/api/books/isbn/{redacted}`; the comment above `log` names it. |
| `deploy/hosted/README.md` | The two lines of the plan (`:278` the `Permissions-Policy`, `:285` the log filter); nothing else. |
| `frontend/src/api/generated.ts` | Regenerated (`npm run gen:api`): the ISBN route's 429 text and description. |

Tests:
- `server/tests/platform.rs`: the `SHELL_CSP` pin, and `the_spa_carries_the_shell_policy` reads the policy a served shell answer (`/scan`) carries: its `script-src` has `'wasm-unsafe-eval'` and neither `'unsafe-eval'` nor `'unsafe-inline'` (helper `script_sources`; tokens are compared whole).
- `server/tests/catalog_search.rs`, `catalog_requests_are_limited_per_account`: 59 searches and one ISBN lookup of a local edition (200) spend the minute; then search, trending, a subject and the ISBN lookup each get 429 `Slow down a little. Try again later.` with `Retry-After`, and an invalid ISBN still gets 400 `That is not a valid ISBN-10 or ISBN-13`, because it is checked before the turn.
- `server/src/lib.rs`, unit test `invite_codes_stay_out_of_the_log`: an ISBN-13, a hyphenated ISBN-10 and an empty segment become `/api/books/isbn/{isbn}`; `/api/books/search` stays as it is.
- Each of the three failed before the change: the old pin, the 61st request (a search) answered 200, and the ISBN path was logged as it came.

### Decisions taken while building

1. **The turn is held until the request ends**, as in `search`: it covers the local step, the remote route and the storing of the hit. The plan's place for it (after `parse_isbn`, before `resolve_isbn`) is kept.
2. **The limit test spends the minute partly with an ISBN lookup** (59 searches and one local hit) instead of only adding the route to the refused list. So it shows both halves: an ISBN lookup counts, including a local hit, and it is refused at the 61st request. The invalid-ISBN assertion shows the order (no turn for a bad ISBN).
3. **The shell assertion reads the served header,** not the constant, on `/scan`, the client route the decoder will run on.
4. **`loggable_path` redacts whatever follows `/api/books/isbn/`** up to the next `/`, valid or not, and an empty segment too, as it does for invite codes: a mistyped ISBN says as much about the reader.
5. **The unit test keeps its name** (`invite_codes_stay_out_of_the_log`): `docs/phase1/review-log.md:231` cites it.

### Deviations from the design

None from §3.6. The files left out on purpose are under "For the integrator".

### Checked beyond the gate

The Caddy block itself, not only `caddy validate`: `caddy:2.10` ran `Caddyfile.example` (`BOOKCLUB_HOST=:8080`) in front of a stub upstream (busybox `httpd` serving `/api/health`), all inside throwaway containers.
- The answer carried `Permissions-Policy: accelerometer=(), browsing-topics=(), camera=(self), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), payment=(), usb=()`.
- The access log held `/api/books/isbn/{redacted}`, `/api/books/isbn/{redacted}?{redacted}` (for `/api/books/isbn/978-0-306-40615-7?x=1`), `/api/books/search?{redacted}`, `/join/{redacted}`, `/api/invites/{redacted}/accept`, and `/api/books/works/42` unchanged.

### The gate (2026-10-29, on 9370dbc)

All green in the worktree, against `bookclub-pg-scan` on 5461:
- `cargo sqlx database reset -y`, `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked` (1494 passed, 0 failed), `cargo sqlx prepare --check -- --all-targets` (no `query!` changed; `server/.sqlx/` untouched).
- `npm run typecheck`, `npm test` (73 files, 503 tests), `npm run build`, `npm run check:api`.
- shellcheck on `deploy/hosted/*.sh deploy/cutover/*.sh`, and `caddy validate` on `Caddyfile.example`.

### For the integrator

**`conventions.md`**
- `:195` (§3, layer 2), the last sentence becomes: "HSTS and `Permissions-Policy` are the reverse proxy's (#51, `deploy/hosted/Caddyfile.example`), which terminates TLS. The `Permissions-Policy` allows the camera on the own origin only (`camera=(self)`, the barcode scanner #56) and denies accelerometer, Topics, location, gyroscope, magnetometer, microphone, payment and USB."
- `:214` (§3, "Fallbacks"): the policy becomes `default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; manifest-src 'self' blob:; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'`. After "…no plugins, no framing." add: "WebAssembly may compile (`'wasm-unsafe-eval'`, for the barcode decoder's `zxing_reader-<hash>.wasm` from `/assets`, #56); `'unsafe-eval'` and `'unsafe-inline'` stay out of `script-src`." The last sentence becomes: "`tests/platform.rs::the_spa_carries_the_shell_policy` checks both, and that `script-src` has `'wasm-unsafe-eval'` but neither `'unsafe-eval'` nor `'unsafe-inline'`."
- §10.1, the `catalog_user` row of the limits table (`:1320`; the plan's `:1285` is the `RateLimits` struct above it): "catalog requests that may call Open Library (search, trending, subjects, an ISBN lookup, also a local hit, an unknown `OL…W` import, a stub's first view) per account". The `catalog_gate` row ("the same requests") needs no change.
- §19 (`:1896`), "Never log": "The access log's `path` (the `TraceLayer` span in `lib.rs`) replaces the code in `/api/invites/{code}…` and `/join/{code}` with `{code}`, and the ISBN in `/api/books/isbn/{isbn}` with `{isbn}` (it says what someone reads, like a search query, which the log never holds; #56)."

**`endpoint-map.md`**
- `:135` (the SPA catch-all), the GATE sentence: "(scripts, styles, pictures, fonts, fetches and the manifest from the own origin only; WebAssembly from the own `/assets` (`'wasm-unsafe-eval'`, the barcode decoder #56; never `'unsafe-eval'`); `data:` and `blob:` pictures, a `blob:` manifest, inline styles; no plugins, no framing; conventions.md §3 "Fallbacks")".
- `:210` (the catalog section's intro): "Search, trending, subjects, an ISBN lookup (also a local hit; #56), the import of an unknown `OL…W` and a stub's first view may call Open Library…".
- `:222` (`GET /api/books/isbn/{isbn}`), replace "429 when every source is throttled." with: "429 when every source asked is busy (the catalog's busy strings), or `Slow down a little. Try again later.` (`catalog_user`). A valid ISBN takes the account's catalog turn (`catalog::upstream_turn`) after the check, so an invalid one costs nothing, and also when the edition is local (#56). The app log writes the path as `/api/books/isbn/{isbn}`, the proxy's log as `/api/books/isbn/{redacted}`."

**`CLAUDE.md`**, "No third party, no inline script": "…and the shell's CSP (`server/src/spa.rs`) allows scripts from `'self'` only, plus `'wasm-unsafe-eval'` for the barcode decoder's WebAssembly from our own `/assets` (never `'unsafe-eval'`)."

**Doc comments outside SCAN's files** (`catalog/mod.rs` belongs to no Phase 3 package, `ratelimit.rs` to FRIENDS): the request lists of `upstream_turn` (`server/src/catalog/mod.rs:114`, "search, trending, a subject, the import or first view of a book") and of `RateLimits::catalog_user` (`server/src/ratelimit.rs:264`, FRIENDS') should name the ISBN lookup too. Behaviour does not depend on them.

**`deploy/hosted/README.md`, step 6 of the Caddy section** (`:273`, not in SCAN's hunks): it could also open `https://<host>/api/books/isbn/9780306406157` and grep the log for `9780306406157` (expecting 0), like the invite code check.

### For #53 only: facts for the privacy text

- ISBN lookups, so every scan, are logged without the ISBN: the app's request log writes `/api/books/isbn/{isbn}` (`server/src/lib.rs` (`loggable_path`)), the proxy's access log `/api/books/isbn/{redacted}` (`deploy/hosted/Caddyfile.example`). `docs/legal/privacy-facts.md` §4 (the app log and the reverse proxy bullets) names only invite codes today.
- Unchanged and worth stating: when the ISBN is not in the catalog yet, the outbound request to the DNB or Open Library is logged with its URL, which holds the ISBN, but not who asked, exactly as for a search text (`server/src/outbound.rs`; privacy-facts.md §4 says so for search texts and ISBNs already).
- The camera facts (frames decoded in the browser, never sent; only the ISBN reaches the server) come with Part 2.

### Bundle sizes (for Part 2's comparison)

The main entry before any SCAN SPA code (`npm run build` on this branch after Part 1, which changes no SPA code): `dist/assets/index-<hash>.js` 325.16 kB, 115.20 kB gzip.
