# HAR (#42) implementation notes

Owner: HAR. These notes record what the DNB harvester package built, where it fills gaps the design left open, and what FE, PAR and the integrator need to know. The design documents themselves are unchanged.

## What was built

### Modules (`server/src/harvest/`)

| File | Contents |
|---|---|
| `mod.rs` | `routes()`, `harvest_due` (one pass of the job), `run_forever` (the frozen entry point), `RECHECK` |
| `oai.rs` | The OAI-PMH envelope only: `list_records_url` (`verb=ListRecords&metadataPrefix=MARC21-xml&set=…&from=YYYY-MM-DDThh:mm:ssZ`), `resume_url` (`verb` and `resumptionToken` only, the exclusive argument of OAI-PMH 2.0), `parse_list_records` → `Answer::{Page, NoRecordsMatch, BadResumptionToken, Error}`, `OaiRecord` (identifier, datestamp, deleted flag, the MARC `<record>` inside `<metadata>` as text) and `OaiRecord::idn` |
| `run.rs` | `harvest_set` (one set, its cursor in `harvest_state`), `SetReport`, `HarvestError`, the schedule (`interval`, `is_due`, `next_due_in`) and the constants `TOKEN_MAX_AGE` (6 h), `OVERLAP` (1 h), `PAGE_ATTEMPTS` (3), `MAX_RESTARTS` (3), `MIN_INTERVAL` (30 min) |
| `new_releases.rs` | `GET /api/books/new-releases`, `NewReleasesParams`, `new_releases_page` |

HAR never reads a MARC field. Each live record's `<metadata>` content goes to `catalog::shared::dnb_marc::map_record` and the result to `ingest::upsert_dnb_record(…, Mode::Replace)`. That function ingests the first edition with `upsert_edition_with_work(…, WorkGrouping::DnbTitleAuthor, Mode::Replace)` and the record's other editions with `WorkGrouping::Into` that work (conventions §7.2 and §7.5).

### Endpoint

`GET /api/books/new-releases` (operation id `harvest_new_releases`, tag `harvest`) answers a `SearchPage`.

### Tests

- `tests/harvest_run.rs` (21 tests) covers the OAI client and the cursor:
  - request parameters and the User-Agent
  - paging with the token
  - the first run 90 days back, or `DNB_HARVEST_START_DAYS`, and later runs from `harvested_until`
  - acceptance: the crash between pages
  - acceptance: the rejected token
  - a token rejected again and again
  - a stale token, and one just under 6 h
  - a failed page keeps the cursor
  - 503 `Retry-After`
  - `noRecordsMatch`
  - OAI errors, HTML and a truncated page
  - at least 1 s between pages with the production throttle
  - the deadlock retry, and its failure after three attempts
  - records the mapping or ingest refuses are skipped, not the page
  - one cursor per set
  - a failing set does not stop the others
  - the interval and the failure back-off
  - the job under the advisory lock, skipping while another instance holds it
  - shutdown between pages
- `tests/harvest_catalog.rs` (13 tests) covers the catalog side and the endpoint:
  - harvested records as DNB editions, with the placeholder year
  - a deleted header is never mapped
  - a multi-ISBN record stays one work
  - no VLB text or URL stored
  - acceptance: a deleted record withdraws its edition, drops it from new-releases and still resolves by ISBN; a later live record revives it
  - acceptance: a retitled record replaces the provisional title, and a reiheA record finds the reiheN edition
  - an admin title survives a retitled record
  - acceptance: an announced title is searchable (`/api/books/search`) and resolves through `CatalogLookup::resolve_isbn` and `/api/books/isbn/{isbn}` without upstream requests
  - new-releases: the window and its ends, one hit per work with its earliest edition, the order, the language, paging, parameter errors, 401, the viewer's shelf and club picks, the viewer's time zone
- Unit tests: `harvest::oai` (6) on the recorded fixtures, `harvest::run` (2).
- The generic walkers (`auth_required`, `openapi_smoke`, `clubs_non_member`) pick up the new route, and pass.

### Fixtures

`server/tests/fixtures/dnb_oai/` holds the fixtures; its `README.md` lists what each file is and which parts are hand-made.

- **Recorded** on 2026-10-27, eight polite requests:
  - two reiheN pages joined by a real resumption token
  - a complete reiheA list
  - the DNB's `badResumptionToken` and `noRecordsMatch` answers
- **Trimmed:** the pages were cut to whole `<record>` elements.
- **Hand-made:**
  - the empty end-of-list token on page 2
  - `retitled.xml`: the same IDN re-delivered in reiheA with the final title and no 263
  - `deleted.xml`: the same record under a deleted header

The not-yet-published title is "Bondbreaker" (263 `202803`) of the recorded page 1.

## Decisions where the design left room

1. **Ingest entry point.** Each record goes through `ingest::upsert_dnb_record(…, Mode::Replace)`, as work-plan HAR and conventions §7.2 require. It is the record-level wrapper of `upsert_edition_with_work(…, DnbTitleAuthor, Replace)`; calling the latter per edition could split a multi-ISBN record across works.
2. **Run state.**
   - A run is in progress while `run_started_at` is set. The next call resumes it with the stored token when the token is younger than 6 h; otherwise it restarts the run from `run_from`.
   - A run that failed before its first page has no token, so the next call restarts it from `run_from`.
   - The last page writes, in its own transaction: `harvested_until` = `run_started_at` − 1 h (the start of the interrupted run, also after a resume or a restart), `last_success_at`, and NULL for the token, `run_started_at`, `run_from`, `last_error` and `last_error_at`.
3. **`records_seen`** counts the records of the current run, deleted ones included. It is reset when a run starts or restarts, and stays at the last run's total afterwards.
4. **Failures** set `last_error` (at most 1000 characters) and `last_error_at`. The cursor stays at the last committed page. Errors include:
   - transport errors and statuses (after the `RetryPolicy::JOB` retries)
   - OAI errors other than `noRecordsMatch` and `badResumptionToken`
   - a page that is not OAI-PMH, or is truncated (no closing root element)
   - a page that deadlocks three times
   - a token rejected more than three times in one run (`HarvestError::TokenRejected`)
5. **Skipped records.** A record that the mapping refuses (no 001, no usable title) is skipped with a WARN line. So is a record that ingest refuses with any error other than a deadlock: each record runs in a savepoint, and the failure is logged at ERROR. Otherwise one bad record would fail its page on every later run. A deadlock still rolls back and retries the whole page (conventions §7.2).
6. **Deleted headers** take the IDN from the OAI identifier's last segment (`oai:dnb.de/dnb-all:reiheN/1168979633`), because the metadata is never mapped. An identifier without a usable IDN is skipped. `withdraw_dnb_edition` inserts nothing for an unknown IDN.
7. **End of a list:** no `<resumptionToken>` element (the DNB's answer when the list fits on one page), an empty one, and `noRecordsMatch` all complete the run.
8. **At least 1 s between pages** comes from the DNB OAI host throttle in `outbound.rs` (1 request per second, evenly spaced). HAR adds no sleep of its own. `pages_are_at_least_a_second_apart` runs two pages with the production throttle.
9. **503 `Retry-After`** is `RetryPolicy::JOB` in `outbound::get_bytes`: up to three waits of at most 5 min, then the single 5xx retry.
10. **Schedule.**
    - `run_forever` runs a pass under `jobs::run_locked(…, lock_ids::DNB_HARVEST, …)`, then sleeps until the next set is due, at least 60 s (`RECHECK`) and at most the interval.
    - The interval is `DNB_HARVEST_INTERVAL_MINUTES`, never below 30 min (`run::interval`; the configuration already refuses less).
    - A set is due when it has had no failure within the interval, and either a run is in progress (interrupted, so it resumes at once) or no run completed within the interval.
    - Because this state lives in `harvest_state`, a restart or a second instance never harvests a set early, and a failing set waits a whole interval before it tries again.
    - The first pass starts right after startup.
11. **Shutdown** is checked between pages and also stops a page request in flight. The page transaction is never cancelled. The run returns `completed: false`, and the next start resumes it.
12. **Sets** run one after the other in `DNB_HARVEST_SETS` order. A failing set does not stop the others, and the pass then fails with every set's error (logged by `run_locked`).
13. **new-releases parameters:**
    - `language` defaults to `ger` and must be three lowercase letters. Otherwise the answer is 400 `String should match pattern '^[a-z]{3}$'`, the pydantic wording CAT uses for the same field.
    - `page` is 1 or more, and `limit` is 1 to 40 with default 24 (the search bounds).
    - Errors use `Validator`'s pydantic wording.
14. **new-releases content:**
    - "Today" is in the caller's time zone (`users.timezone`), like other personal dates. The window is today − 30 to today + 120 days, both ends included.
    - One hit per work: the edition with the earliest date in the window, the lowest edition id on a tie.
    - The hit is built with that edition as the `BookRef` context, so the edition's own title and cover win (`books::book_outs`). `isbn` is that edition's ISBN-13.
    - Merged-away works are excluded.
    - `on_shelf`, `shelf_id` and `club_picks` use the same queries as CAT's search hits (`bc_member_clubs`), duplicated in `new_releases.rs` because `catalog::hits` is not a frozen interface. After Wave 2, the integrator may point both at one function.

## For FE (#48)

- New route: `GET /api/books/new-releases?language=ger&page=1&limit=24` → `SearchPage`. `has_more` is set, and every hit has `planned_publication_date` (`YYYY-MM-DD`). The route makes no remote calls and needs a session (401 `Not signed in`).
- No new detail strings to map. The only new 400 is the pydantic pattern message for `language`, which the app never triggers when it sends `ger`.
- The list is empty until the harvester has run (`DNB_HARVEST=true`).

## For PAR (#49)

- No Python tests to port or reason away: the Python app has no DNB harvester and no new-releases route.
- To check by hand against the real DNB: `DNB_HARVEST=true` (with `DNB_HARVEST_START_DAYS=1` for a short first run), then `SELECT * FROM harvest_state` and `GET /api/books/new-releases`.

## Open points

- None blocked. No file outside HAR's ownership was changed. No migration was needed: `harvest_state` and `editions.withdrawn_at` and `planned_publication_date` come from `0001_init.sql`.
