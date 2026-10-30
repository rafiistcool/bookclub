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

**The request id in the proxy log** (review round, finding 3; for the operator to choose, then for #53's text): the app log writes an upstream ISBN lookup's or search's outbound URL on a line with the request id, and the proxy log keeps that id with the client's IP address (see "For #53 only" below). Either of these breaks the link:
- `resp_headers>X-Request-Id delete` in the `format filter` of `deploy/hosted/Caddyfile.example` (SCAN's file), after `request>headers>Referer delete`. Checked with `caddy:2.10`: `caddy validate` passes, the client still gets `X-Request-Id`, and the access line no longer holds it. It covers search texts too. The cost: a proxy line can be matched to the app's lines only by time and path, no longer by id.
- Or `server/src/outbound.rs` logs catalog URLs without their query string and ISBN at `info` (the full URL at `debug`). That file belongs to no Phase 3 package. The cost: less to go on when an upstream misbehaves.

### For #53 only: facts for the privacy text

- The request line of an ISBN lookup, so of every scan, is logged without the ISBN: the app's request log writes `/api/books/isbn/{isbn}` (`server/src/lib.rs` (`loggable_path`)), the proxy's access log `/api/books/isbn/{redacted}` (`deploy/hosted/Caddyfile.example`). `docs/legal/privacy-facts.md` §4 (the app log and the reverse proxy bullets) names only invite codes today.
- Not redacted, unchanged by SCAN and the same as for a search text: when the ISBN is not in the catalog yet, the lookup asks the DNB or Open Library within the request, and the app log writes that outbound URL, which holds the ISBN, at `info` (`server/src/outbound.rs`). The line carries the request's span, `request{method=GET path=/api/books/isbn/{isbn} request_id=<uuid>}`, so it names the route and the request id.
- That request id also reaches the proxy's log: the app returns it as `X-Request-Id` (`PropagateRequestIdLayer`, `server/src/lib.rs`), and Caddy's access log keeps the response headers, beside the client's IP address, for up to 7 days (checked in the review round). So while both logs exist, an ISBN that went upstream, or a search text, can be tied to the address that asked. `privacy-facts.md` §4 (`:85`, "but not who asked") holds for the app log alone, not with the proxy log beside it. The remedies are under "For the integrator" above; the privacy text should say what the chosen one leaves.
- The camera facts (frames decoded in the browser, never sent; only the ISBN reaches the server) come with Part 2.

### Bundle sizes (for Part 2's comparison)

The main entry before any SCAN SPA code (`npm run build` on this branch after Part 1, which changes no SPA code): `dist/assets/index-<hash>.js` 325.16 kB, 115.20 kB gzip.

## Part 2: the SPA

Plan §3.1–§3.5, §3.7 and the SPA tests of §3.9, with answers 5–15. The phones are tested after the move (§3.8, §5.8); #56 stays open until then.

### What was built

| Where | What |
|---|---|
| `frontend/package.json`, `package-lock.json` | `zxing-wasm` exactly `3.1.6` (`npm install --save-exact`). `npm view zxing-wasm@3.1.6 scripts` lists no `preinstall`, `install`, `postinstall` or `prepare`. It brings `@types/emscripten` 1.41.6 (MIT), `type-fest` 5.10.0 (MIT or CC0-1.0, Node >= 20, nested under `zxing-wasm/node_modules`) and, through it, `tagged-tag` 1.0.0 (MIT, Node >= 20): types only, none with an install script. MIT; the wasm bundles zxing-cpp (Apache-2.0). |
| `frontend/src/isbn.ts` | `normalizeIsbnInput` (NFKC, whitespace and `-` dropped, uppercased), `parseIsbn` (manual entry: ISBN-10 or ISBN-13, returned as ISBN-13, 978/979 only, check digits), `isbnFromScan` (exactly 13 digits, 978/979, check digit; no ISBN-10). `extractIsbn` in `constants.ts` is unchanged. |
| `frontend/src/scan/camera.ts` | `cameraSupport()` (`insecure` when `isSecureContext` is false, `unsupported` without `getUserMedia`), `openCamera()` (rear camera, about 720p, no audio; only the request), `showCamera(video, stream)` (attaches and plays; if `play()` fails it stops the stream and clears `srcObject` only while it still holds that stream), `stopCamera(stream)`, `cameraError(err)` (the `DOMException` names of §3.4; a plain object's `name` too, since Chrome's `OverconstrainedError` is not a `DOMException`). |
| `frontend/src/scan/decoder.ts` | `createDecoder()`: native `BarcodeDetector` only when `getSupportedFormats()` lists `ean_13` (a throwing or empty list falls through), else `await import("./zxing")`. The `BarcodeDetector` types are local. Also `scanBand(width, height)` and `drawScanBand(video, canvas)`: the centre band (full width, the middle 40 %, at most 1280 px wide) drawn into a canvas whose context is created with `willReadFrequently: true`. |
| `frontend/src/scan/zxing.ts` | `prepareZXingModule({overrides: {locateFile}, fireImmediately: true})` once, before the first read, with one overrides object (zxing-wasm compares overrides shallowly; a new function per call would instantiate the module again). `locateFile` returns `wasmUrl` (`zxing-wasm/reader/zxing_reader.wasm?url`) for every `.wasm`. A failed start is purged (`purgeZXingModule`), so a later scan tries again. `readBarcodes(imageData, {formats: ["EAN13"], tryHarder: true, maxNumberOfSymbols: 1})`, keeping `text` where `isValid`. |
| `frontend/src/components/BarcodeScanner.vue` | Emits `isbn: [isbn13]` and `cancel: []`. States `starting`, `live`, `insecure`, `unsupported`, `nocamera`, `denied`, `busy`, `failed`, `nodecoder`. One decode every 200 ms with `setTimeout`, the next scheduled only when the pass has ended. Manual entry (`autocomplete="off"`, `enterkeyhint="search"`, no `inputmode`) in every state, focused in `insecure`. The camera stops on a result, on cancel, on unmount and on `visibilitychange` to hidden, and starts again when the page is visible. A run counter drops a stream that opens after a stop (unmount, or hidden and back, while the permission prompt is open) before it reaches the video, so only one stream is ever open and a stale request cannot interrupt the current one's `play()` (review round). |
| `frontend/src/pages/ScanPage.vue` | `/scan`: the scanner, then one `GET /api/books/isbn/{isbn}` with an `AbortController`; the ISBN and, from 2 s on, the seconds; "Cancel" aborts and goes back to the camera. The hit card, the miss card and the error panel of §3.1–§3.5 (details below). |
| `frontend/src/components/AddBookSheet.vue` | `initialIsbn` seeds the ISBN field; `stay` emits `added: [book, isbn]` (the ISBN sent, as typed, or null) and `close` instead of navigating. |
| `frontend/src/components/AddEditionForm.vue` | `initialIsbn` seeds the ISBN field. Its 409 handling is unchanged, except that `existing` now also carries the ISBN the 409 was for (`existing: [editionId, isbn]`; review round). |
| `frontend/src/pages/DiscoverPage.vue` | A scan button (a link to `/scan`, "Scan a barcode") at the right end of the search field, always shown. After an ISBN search, "Add your own book" opens `AddBookSheet` with `:initial-isbn` and an empty title. |
| `frontend/src/router.ts` | `{ path: "scan", component: () => import("./pages/ScanPage.vue") }` in the AppShell, after `discover`. |
| `client.ts`, `types.ts`, `apiErrors.ts`, `fixtures.ts` | Add-only: `api.lookupIsbn(isbn, signal?)`, the `IsbnHitOut` re-export, `"No book found for that ISBN": "errors.isbnNotFound"`, and `isbnHit()`. |
| `frontend/src/i18n/{en,de}.json` | `scan.*` (37 keys), `addBook.isbnFilled`, `editions.isbnFilled`, `errors.isbnNotFound`; add-only. |

The hit card: the cover (or the cloth placeholder), title, authors, year, pages and ISBN. Not on the shelf: the four statuses as the book page's segmented control (short labels, the full one as `title`), each `POST /api/shelf {work_id, edition_id, status}`; a 409 with `err.item` becomes `PATCH /api/shelf/{id} {edition_id, status, position: 0}`. On the shelf: "On your shelf" and "Use this edition" (`PATCH /api/shelf/{shelf_id} {edition_id}`); after either write the card says "This edition is on your shelf." No cover: "Add a photo of the cover", `CoverPhotoButton` with `workId`, `editionId` and COVER's `label` prop; a 200 puts the new cover on the card, a 202 is the button's own toast. Always "Open book" and "Scan another".

The miss card: "Find the book by title" (a search field, `GET /api/books/search?q=…&limit=10`, a list of works; a choice shows `AddEditionForm` with `workId` and `initialIsbn`), then "Add it as a new book" (`AddBookSheet` with `initialIsbn` and `stay`), then "Scan another". After a save (also after `AddEditionForm`'s `existing`) the page looks up the ISBN that was saved, which the reader may have changed in the form, and shows the hit card; a save without an ISBN opens the book, and so does a saved ISBN that still misses (review round).

Errors: 404 is the miss card. 429 (the busy strings and `Slow down a little. Try again later.`), 502 and anything else show the mapped message, the ISBN, "Try again" (the same ISBN) and "Scan another".

### Tests

- `src/isbn.test.ts`: the vectors of `isbn.rs` (ISBN-10 to 978, ISBN-13 kept, 979, all zeros, wrong check digits, the 977 EAN, `X` only last, 12 and 14 digits), full-width digits, hyphens and `Ｘ`, the ISMN `9790…`, and `isbnFromScan` refusing ISBN-10, other EANs, hyphens and padding.
- `src/scan/decoder.test.ts`: native only with `ean_13` listed; `[]`, a list without it, a throwing `getSupportedFormats()` and no `BarcodeDetector` load zxing (mocked); a failing zxing rejects; `scanBand` and `drawScanBand`.
- `src/scan/zxing.test.ts`: with `zxing-wasm/reader` mocked, `prepareZXingModule` is called once, with `fireImmediately`, before any read, and its `locateFile` turns `zxing_reader.wasm` (with jsDelivr as the prefix) into a path that starts with `/` and has no `://`; a failed start is purged and tried again; `readBarcodes` gets the options above and only valid results come back. Under Vitest the real `?url` import is used (`/node_modules/zxing-wasm/dist/reader/zxing_reader.wasm`).
- `src/scan/camera.test.ts`: `cameraSupport` for an insecure context (also with `mediaDevices` present), missing `mediaDevices` or `getUserMedia`, and ok; `cameraError` for every name; `openCamera`'s constraints and its stop on a failed `play()`; `stopCamera`.
- `src/components/BarcodeScanner.test.ts` (fake timers, camera and decoder mocked, the real `stopCamera`): insecure (manual entry focused, no camera and no decoder started), unsupported, no camera, denied (both how-tos, "Try again" reaches `live`), busy, no decoder (the stream is stopped); a typed ISBN-10 emits its ISBN-13; a wrong one shows `invalidIsbn`; a 977 EAN and a wrong check digit are ignored and the valid one emits once, with no pass after it; passes never overlap; the tracks stop on emit, unmount, a stream that opens after unmount, `visibilitychange` (and restart when visible), and cancel.
- `src/pages/ScanPage.test.ts` (the real add forms, `CoverPhotoButton` and `ApiError`; the scanner stubbed): a hit adds with `edition_id`; `on_shelf` sends PATCH `edition_id`; a 409 becomes a PATCH with `position: 0`; the photo button with and without a cover; a 404 shows the miss card with "Find the book by title" before "Add it as a new book", the first leads through the search to `AddEditionForm` with the ISBN and back to the hit card, the second to `AddBookSheet` with the ISBN (title empty, no navigation) and then the hit card with the photo button; 429 (both kinds) and 502 keep the ISBN and look it up again; the clock after 2 s, and "Cancel" aborts the request.
- `src/components/AddBookSheet.test.ts`: `initialIsbn` fills the ISBN field, not the title, and its hint; the usual hint once it is changed; `stay` emits `added` and `close` and does not navigate; a failed save with `stay` keeps the sheet and its values.
- `src/components/AddEditionForm.test.ts` (new): empty by default; `initialIsbn` fills the ISBN and is sent; the 409 "Choose that edition" still emits `existing`.
- `src/pages/DiscoverPage.test.ts`: the scan link in the search field; an ISBN search gives `AddBookSheet` `initialIsbn` and an empty title; any other search still fills the title.
- `src/router.test.ts`: `/scan` opens in the app shell (SCAN's hunk of the shared file).

### The Chromium run (2026-10-30, §3.9)

- The built SPA from the Rust server on `http://localhost:8091` (`DEV_MODE=true`, `PUBLIC_URL=http://localhost:8091`, `STATIC_DIR=../frontend/dist`, `MEDIA_DIR` in the scratchpad, `DNB_HARVEST=false`, `bookclub-pg-scan`), one synthetic account (`scantester`, from a `beta-invite` code).
- `/usr/bin/chromium` headless through `playwright-cli`, launched with `--use-fake-ui-for-media-stream --use-fake-device-for-media-stream --use-file-for-fake-video-capture=<scratchpad>/video/ean-<isbn>.y4m`, viewport 390 × 844 and then 360 × 740, and a `securitypolicyviolation` listener installed with `addInitScript` before the first page.
- The clips are still EAN-13 barcodes written as Y4M by a short Python script (bars of 9 px on white, 1280 × 720), one browser per clip.
- Open Library resets TLS connections from this VM, so `OPENLIBRARY_URL` and `OPENLIBRARY_COVERS_URL` pointed at a local stub on 127.0.0.1 that answers every `search.json` with no docs and everything else with 404. The DNB was the real one. The outbound User-Agent was the server's DEV_MODE one, `Bookclub/1.0 (+http://localhost:8091)`.

Results:
- **Known ISBN:** Discover → the scan button → `/scan` → the camera decoded `9783895614194` on the zxing path (`'BarcodeDetector' in window` was false; requests: `ScanPage-*.js`, `zxing-*.js`, `/assets/zxing_reader-*.wasm`, `/api/books/isbn/9783895614194`, all same-origin) → the DNB answered → the hit card ("Ich kann nicht vergeben", Rudolf Vrba, 496 pages). "Want" shelved it with the edition; "Scan another" read it again (now local, on the shelf), "Use this edition" answered "Edition saved"; a synthetic PNG through "Add a photo of the cover" became the cover on the card. The app log wrote the path as `/api/books/isbn/{isbn}`.
- **Made-up ISBN:** `9780000000002` → the stub had nothing → 404 → the miss card. "Find the book by title" found the work above locally; `AddEditionForm` opened with the ISBN and "The ISBN you looked up."; "Add edition" → the second lookup → the hit card.
- **Denied and manual entry:** with `getUserMedia` replaced by a `NotAllowedError` in the page, "Scan another" showed the denied state with both how-tos, "Try again" and manual entry; `978-0-306-40615-7` typed → miss → "Add it as a new book" opened the sheet with the ISBN and an empty title → saved → the hit card with "Add a photo of the cover".
- **Zero `securitypolicyviolation` events** in all three browsers, on every page above.
- **360 px:** a third browser at 360 × 740 walked Discover, the live scanner, the hit card, the denied state, the miss card, the list of works, the edition form and the add-book sheet; `document.documentElement.scrollWidth` was 360 on each, so nothing scrolls sideways, and no CSP violation either.

### Decisions taken while building

1. **The decode band's geometry lives in `decoder.ts`** (`scanBand`, `drawScanBand`) so it is unit-tested; the loop itself is in `BarcodeScanner.vue`, as planned.
2. **Two more scanner states than the table:** `failed` ("The camera did not start.", with "Try again") for an error name §3.4 does not list, and `nodecoder` ("This browser cannot read barcodes.", §3.4's last bullet), which also follows five decode passes in a row that throw. `starting` shows "Starting the camera…" over the black video box.
3. **The decoder is created in parallel with the permission prompt**, and only when the camera is supported, so an insecure or camera-less page loads no wasm. Its promise is marked handled at once, so a rejection while the prompt is open is not an unhandled rejection.
4. **`isbnFromScan` takes the decoded text as it is** (no trimming); a barcode's text has no spaces or hyphens.
5. **Full-width digits** pass `parseIsbn` after NFKC, as the plan says, although the server's `parse_isbn` refuses them (`isbn.rs` test vector `978031676948８`): the SPA sends the ASCII ISBN-13, so the server never sees them.
6. **The hit card after a save** shows the photo button only when the hit has no cover. `IsbnHitOut` falls back to the work's cover, so a new edition of a work that already has one shows that cover and no button (seen in the Chromium run); a reader's own new book has none and gets the button.
7. **After a save in the miss flow, `AddEditionForm`'s `existing`** (the ISBN turned out to be an edition of this work) also leads to the second lookup and the hit card, of the ISBN the 409 was for (review round).
8. **`AddBookSheet` with `stay`** still navigates for "Open that book" after a 409: that is the reader's explicit choice.
9. **The hint under a prefilled ISBN** ("The ISBN you looked up.") shows only while the field still holds it.
10. **Discover's scan button is 40 px wide** and sits on the field's right padding, and the browser's own search cancel button (a second × next to "Clear") is hidden, so the field keeps as much room as it can at 360 px. The placeholder "Title, author, or ISBN" still loses its last characters at 360 px (it fits from about 375 px); see "Later ideas".
11. **`vite.config.ts` is unchanged:** the dev server serves the `?url` import as `/node_modules/zxing-wasm/dist/reader/zxing_reader.wasm` with `application/wasm` (checked with `vite --host 127.0.0.1`), so `optimizeDeps.exclude` is not needed.
12. **The jsDelivr default stays in the zxing chunk** (the package's own `locateFile`, `dist/assets/zxing-*.js`). It is never used: a decoder exists only after `createZxingDecoder` prepared the module with ours, and every later read reuses that module (zxing-wasm keeps the cached overrides). A `readBarcodes` before any preparation would take the default, so nothing else may call zxing-wasm directly. The Chromium run made no request to another origin.

### Deviations from the design

1. **No `addEdition` namespace exists:** `AddEditionForm.vue` uses `editions.*`, so its new key is `editions.isbnFilled` (the plan's §3.7 and §2.1 rule 2 say `addEdition`).
2. **The fake camera clip** is a Y4M written directly by a Python script instead of an MJPEG made from a PNG with ffmpeg (§3.9 said "for example"); Chromium reads both.
3. **The Chromium run used a local Open Library stub** (above), because openlibrary.org is not reachable from this VM. The DNB lookup was real.
4. **Discover's native search cancel button is hidden** (decision 10), which the plan does not mention.

### Bundle sizes (`npm run build`, 2026-10-30)

| Asset | Before SCAN's SPA code | After |
|---|---|---|
| Main entry `index-<hash>.js` | 325.16 kB (115.20 kB gzip) | 329.44 kB (116.54 kB gzip) |
| `ScanPage-<hash>.js` / `.css` | – | 15.37 kB (5.70 kB gzip) / 2.96 kB (0.92 kB gzip) |
| `zxing-<hash>.js` (glue, only on the zxing path) | – | 36.20 kB (13.07 kB gzip) |
| `zxing_reader-<hash>.wasm` (only on the zxing path) | – | 946.86 kB (411.57 kB gzip) |

- The main entry grows by 4.3 kB: the new copy in both catalogues (the i18n JSON is in the entry) and the route. It contains no `zxing` (`grep -c -i zxing` is 0).
- `dist/assets/` holds exactly one `zxing_reader-<hash>.wasm`; only `zxing-<hash>.js` names it, and only `ScanPage-<hash>.js` imports that chunk.
- The server answers the wasm with `Content-Type: application/wasm` and `Cache-Control: public, max-age=31536000, immutable`, so a phone downloads it once per release. On Android with Play Services the native detector is used and neither file is loaded.

### The gate (2026-10-30, on 6bba1fd)

All green in the worktree, against `bookclub-pg-scan` on 5461:
- `cargo sqlx database reset -y`, `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked` (94 test binaries, 1494 passed, 0 failed), `cargo sqlx prepare --check -- --all-targets` (no `query!` changed; `server/.sqlx/` untouched).
- `npm run typecheck`, `npm test` (80 files, 577 tests; Part 1 had 73 and 503), `npm run build`, `npm run check:api` (`generated.ts` unchanged by Part 2).
- shellcheck on `deploy/hosted/*.sh deploy/cutover/*.sh`, and `caddy validate` on `Caddyfile.example`.
- The plan's acceptance set (`npx vitest run src/isbn.test.ts src/scan src/components/BarcodeScanner.test.ts src/pages/ScanPage.test.ts src/pages/DiscoverPage.test.ts src/components/AddBookSheet.test.ts src/components/AddEditionForm.test.ts src/i18n/messages.test.ts src/thirdParty.test.ts`): 11 files, 115 tests passed.

### For the integrator (Part 2)

**`CLAUDE.md`**
- "Frontend" section, a new bullet: "The barcode scanner (#56) is `/scan` (`pages/ScanPage.vue`, entered from Discover): `components/BarcodeScanner.vue` with `scan/camera.ts`, `scan/decoder.ts` (native `BarcodeDetector` when it lists `ean_13`, else zxing-wasm) and `scan/zxing.ts`; `isbn.ts` mirrors the server's `parse_isbn`. zxing-wasm is only loaded through `createDecoder`'s dynamic import and is always prepared with `scan/zxing.ts`'s `locateFile` first: its default fetches the wasm from jsDelivr, which the CSP blocks."
- "Dev servers", after the `agents` VM lines: "The camera needs a secure context, so on `http://agents.lan:<port>` the scanner offers manual entry only. Try the camera on `http://localhost:<port>` (Chromium with `--use-fake-device-for-media-stream --use-file-for-fake-video-capture=<clip>.y4m`), or on a phone over HTTPS."
- The "No third party, no inline script" text from Part 1 stands; it may add: "zxing-wasm's wasm comes from our own `/assets` (`scan/zxing.ts`)".

**`README.md`**, the table of pages: a row "`/scan` | Scan a book's barcode (or type its ISBN), then shelve it, set your edition, or add the book when the catalog has none". The table still describes the Python app's pages; the integrator decides whether it is updated now or with CUT-R.

**`docs/plan.md`**, if it lists what Phase 3 shipped: "barcode scanning on the web (`/scan`), with manual ISBN entry everywhere".

**Shared files, for the merge with FRIENDS:** every SCAN hunk in `client.ts`, `types.ts`, `apiErrors.ts`, `fixtures.ts` and `en.json`/`de.json` is add-only; `router.ts` and `router.test.ts` have one hunk each (after `discover`, and a new test after "leaves every known page to its own route"). `scan` is a new top-level key after `discover` in both catalogues.

### For #53 only: the camera

- The camera is used only on `/scan`, after the reader opens it, and only while that page is visible; the stream stops on a result, on cancel, when leaving the page and when the page is hidden (`frontend/src/components/BarcodeScanner.vue`).
- Frames are decoded in the browser, by the browser's own barcode detector or by zxing-wasm running from the service's own `/assets`; no frame and no picture leaves the device. Only the ISBN read from the barcode is sent, as `GET /api/books/isbn/{isbn}` (`frontend/src/pages/ScanPage.vue`). The app log and the proxy log write that path without the ISBN; a lookup that goes upstream logs its outbound URL, with the ISBN, on a line with the request id (Part 1, "For #53 only").
- Nothing about the camera is stored: no permission flag, no setting, no browser storage. The permission itself is the browser's.
- The decoder's wasm is a first-party file like the scripts (`privacy-facts.md` §6 stays true: no CDN, no third-party request). The `Permissions-Policy` allows the camera on the own origin only (`camera=(self)`, Part 1).
- A photo added from the hit card is the existing cover upload (`privacy-facts.md` as for COVER): metadata stripped, re-encoded.

### Noticed, not changed

- `npm install` reports 5 high-severity advisories, all in `vue-i18n` and its `@intlify/*` packages (`escapeParameter`); they were there before SCAN, and `zxing-wasm` adds none. Not SCAN's to fix (§2.1 rule 3: no other npm change).

### Later ideas

- "Scan your copy" in the edition picker (`EditionPicker.vue`), for a shelved book (answer 11).
- The hit card could say which edition and status the shelf entry has, without a tap, if `IsbnHitOut` named the entry's edition (a DTO change).
- Torch and zoom on Android (`MediaStreamTrack.applyConstraints`), if dim shelves need them.
- A worker for zxing, only if a phone shows jank (answer 6).
- Discover's "Clear" as an icon, so the placeholder fits at 360 px too.
- Continuous scanning of a whole shelf belongs to the native app (#60).

## Review round (2026-10-30)

Six findings, each checked against the code first. All six were real; none was rejected. The rows above that describe the changed code (`camera.ts`, `BarcodeScanner.vue`, the add forms, the miss card, decision 7, the dependencies and the #53 facts) now say what is built.

| # | Finding | Verdict | Commit |
|---|---|---|---|
| 1 | The miss flow looked up the scanned ISBN again even when the reader saved another one or none | Real (`ScanPage.vue` `onSaved` ignored the payload) | e1381e6 |
| 2 | Two overlapping `openCamera` calls on one `<video>` ended in a false "busy", with two streams open | Real (reproduced by a test, below) | 02c90c3 |
| 3 | The #53 fact "logged without the ISBN … but not who asked" overstated the redaction | Real (checked in the code and with Caddy) | this section's commit |
| 4 | German miss card: "diese" pointed at the wrong edition | Real | b83d552 |
| 5 | Caddy's compression comment did not name the wasm under `/assets` | Real | f41675d |
| 6 | The dependency list left out `tagged-tag` | Real (`package-lock.json`, through `type-fest` 5) | this section's commit |

### 1. The miss flow follows what was saved

- Both forms leave the prefilled ISBN editable: `AddEditionForm` saves with the ISBN cleared when another field is filled, `AddBookSheet` without any ISBN. `ScanPage` then looked up the scanned ISBN, got 404, and showed the miss card again, which invited a duplicate book.
- Now the forms say what they sent: `AddBookSheet`'s `added: [book, isbn]` (`BookDetailOut` has no ISBN) and `AddEditionForm`'s `existing: [editionId, isbn]`, the ISBN the 409 was for (not what the field holds when the reader taps "Choose that edition"). `EditionPicker` ignores the new argument.
- `ScanPage.showSaved(workId, isbn)`: an edition's `isbn13`, or the sent ISBN through `parseIsbn` (an ISBN-10 becomes its ISBN-13, as the server stores it), is looked up and shows the hit card; no ISBN opens the book (`/book/{id}`). If the saved ISBN still misses, the page opens the saved book instead of the miss card (`savedWork`, cleared by a new scan and "Scan another").
- Tests (`ScanPage.test.ts`, "when the reader changes the scanned ISBN in the form"): an edition saved with another ISBN looks that one up; an edition saved without one opens the book; "Choose that edition" after a 409 looks up the ISBN the 409 was for; a new book saved with another ISBN, typed as an ISBN-10, looks up its ISBN-13; a new book saved without one opens it; and a saved ISBN that still misses opens the book. All six failed before the fix (the scanned ISBN looked up again, or the miss card). `AddBookSheet.test.ts` and `AddEditionForm.test.ts` check the new payloads.

### 2. Only the current start touches the video

- `openCamera(video)` attached the stream before the scanner could check its run counter, and its `catch` cleared `srcObject` whatever it held. Hidden and visible again while the prompt was open, both requests resolved on one `<video>`: each `srcObject` interrupted the other's `play()` with `AbortError`, the older call's cleanup cleared the newer stream, and the current start showed "The camera is in use by another app."
- Now `openCamera()` only requests the stream and `showCamera(video, stream)` attaches and plays it. `BarcodeScanner.start()` checks `current === run` between the two: a stale stream is stopped without reaching the video. The stream is `stream` from before `play()`, so a stop during `play()` ends it. `showCamera` clears `srcObject` only while it still holds its own stream.
- Tests: `BarcodeScanner.test.ts`, "keeps one stream when the page comes back while the permission prompt is open", runs the real camera module against a `getUserMedia` that waits for the prompt and a `<video>` that behaves like a browser's (a new `srcObject` interrupts a pending `play()`; jsdom has neither). Before the fix it ended in the busy state; now the first stream is stopped, the second shows and decodes. Also "says another app has the camera when the video cannot start, and tries again" (the new `showCamera` failure branch), and in `camera.test.ts` the split functions and "leaves a newer stream on the video when its own play() is interrupted".

### 3. The privacy facts for #53

- Checked: `lookup_isbn` awaits `resolve_isbn` in the handler (no `spawn`), so `outbound.rs`'s `info` line (`url=…`) is written inside the `TraceLayer` span with `request_id` (`tracing_subscriber::fmt()`'s default format prints the span and its fields). `PropagateRequestIdLayer` returns the id as `X-Request-Id`.
- Checked with `caddy:2.10` (`Caddyfile.example` with `BOOKCLUB_HOST=:8080`, in front of a stub Caddy that answers with an `X-Request-Id`, throwaway containers): the access line held `/api/books/isbn/{redacted}` and `resp_headers.X-Request-Id`. With `resp_headers>X-Request-Id delete` added to the filter, `caddy validate` passed, the client still got the header, and the line no longer held it.
- The Part 1 bullets for #53 are corrected, the camera bullet points at them, and Part 1's "For the integrator" names the two remedies. Neither is applied: the plan's §3.6.4 filter is built as written, and the choice (losing the id match in the proxy log, or less detail on upstream failures) is the operator's. `privacy-facts.md:85` makes the same claim for search texts; it is not SCAN's file (§2.1 rule 11).

### 4–6

- `de.json` `scan.missBody`: "Kennt der Katalog schon eine andere Ausgabe des Buchs, füg diese ISBN dort als Ausgabe hinzu. Sonst leg es als neues Buch an."
- `Caddyfile.example`: "Only the hashed build output under /assets is compressed: the JS and CSS to about a third, the barcode decoder's wasm (#56) to under half." `README.md:280` ("zstd and gzip for /assets/* only") was right already.
- The dependency row names `tagged-tag` 1.0.0 (MIT, Node >= 20, types only) and the versions and licences of the other two.

### Deviations from the design

1. **`camera.ts`'s interface:** §3.4 has `openCamera(video): Promise<MediaStream>`, which attaches the stream itself. It is now `openCamera(): Promise<MediaStream>` plus `showCamera(video, stream)`, so the scanner can drop a stale stream before it touches the video. The intent (rear camera, about 720p, stopped on a failed `play()`) is unchanged.
2. **The add forms' events:** §3.5 has `added: [book: BookDetailOut]`; `AddBookSheet`'s is now `added: [book, isbn]`, and `AddEditionForm`'s `existing` gains the ISBN. §3.5's "the ISBN is looked up again" became "the ISBN that was saved is looked up", which keeps its intent (the hit card after a save) when the reader changed the ISBN.

### The gate (2026-10-30, on b0942df)

All green in the worktree, against `bookclub-pg-scan` on 5461:
- `cargo sqlx database reset -y`, `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked` (94 test binaries, 1494 passed, 0 failed), `cargo sqlx prepare --check -- --all-targets`. The round changed no server code. The first run's log shared its file name with another package's run in the shared scratchpad, so the server half ran again into a file of its own; the numbers are from that run.
- `npm run typecheck`, `npm test` (80 files, 588 tests; 577 before the round), `npm run build` (the main entry has no `zxing`, `dist/assets/` holds one `zxing_reader-<hash>.wasm`), `npm run check:api` (`generated.ts` unchanged).
- shellcheck on `deploy/hosted/*.sh deploy/cutover/*.sh`, and `caddy validate` on `Caddyfile.example`.
- The plan's acceptance set (`npx vitest run src/isbn.test.ts src/scan …`, as in Part 2): 11 files, 126 tests passed.
