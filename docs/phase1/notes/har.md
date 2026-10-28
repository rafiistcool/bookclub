# HAR (#42) implementation notes

Owner: HAR. These notes record what the DNB harvester package built, where it fills gaps the design left open, and what FE, PAR and the integrator need to know. The design documents themselves are unchanged.

## What was built

### Modules (`server/src/harvest/`)

| File | Contents |
|---|---|
| `mod.rs` | `routes()`, `harvest_due` (the sets that are due), `run_pass` (`harvest_due` under the advisory lock), `next_wait` and `wait_after` (the sleep between passes), `run_forever` (the frozen entry point), `RECHECK` |
| `oai.rs` | The OAI-PMH envelope only: `list_records_url` (`verb=ListRecords&metadataPrefix=MARC21-xml&set=…&from=YYYY-MM-DDThh:mm:ssZ`), `resume_url` (`verb` and `resumptionToken` only, the exclusive argument of OAI-PMH 2.0), `parse_list_records` → `Answer::{Page, NoRecordsMatch, BadResumptionToken, Error}`, `OaiRecord` (identifier, datestamp, deleted flag, the MARC `<record>` inside `<metadata>` as text) and `OaiRecord::idn` |
| `run.rs` | `harvest_set` (one set, its cursor in `harvest_state`), `SetReport`, `HarvestError`, `refuses_record` (which ingest errors skip a record), the schedule (`interval`, `is_due`, `next_due_in`) and the constants `TOKEN_MAX_AGE` (6 h), `OVERLAP` (1 h), `PAGE_ATTEMPTS` (3), `MAX_RESTARTS` (3), `MIN_INTERVAL` (30 min), `RESUME_RETRY` (30 min), `MERGED_RETRIES` (2) |
| `new_releases.rs` | `GET /api/books/new-releases`, `NewReleasesParams`, `new_releases_page` |

HAR never reads a MARC field. Each live record's `<metadata>` content goes to `catalog::shared::dnb_marc::map_record` and the result to `ingest::upsert_dnb_record(…, Mode::Replace)`. That function ingests the first edition with `upsert_edition_with_work(…, WorkGrouping::DnbTitleAuthor, Mode::Replace)` and the record's other editions with `WorkGrouping::Into` that work (conventions §7.2 and §7.5).

### Endpoint

`GET /api/books/new-releases` (operation id `harvest_new_releases`, tag `harvest`) answers a `SearchPage`.

### Tests

- `tests/harvest_run.rs` (27 tests) covers the OAI client and the cursor:
  - request parameters and the User-Agent
  - paging with the token
  - the first run 90 days back, or `DNB_HARVEST_START_DAYS`, and later runs from `harvested_until`
  - acceptance: the crash between pages
  - acceptance: the rejected token
  - a token rejected again and again
  - a page that hands back the token it was asked with
  - a stale token, and one just under 6 h
  - a failed page keeps the cursor
  - 503 `Retry-After`
  - `noRecordsMatch`
  - OAI errors, HTML and a truncated page
  - at least 1 s between pages with the production throttle
  - the deadlock retry, and its failure after three attempts
  - one transaction per page on a page with a token: a page that keeps deadlocking stores neither its records nor its token, and the next run asks for the list again; a page retried after two deadlocks counts its records once
  - records the mapping or ingest refuses (a data error) are skipped, not the page
  - a lock timeout (55P03) or a cancelled statement (57014) fails the page, and the next run stores the record
  - one cursor per set
  - a failing set does not stop the others
  - the interval and the failure back-off
  - with an interval of 8 h, a failed run with a fresh token is due again after 30 min and resumes from its token; with a stale token or none it waits the interval
  - the sleep between passes (`next_wait`): one minute when a set is due, about the interval after a pass
  - the job under the advisory lock; a pass (`run_pass`) with the lock held elsewhere sends nothing, and the next one harvests
  - shutdown between pages
- `tests/harvest_catalog.rs` (15 tests) covers the catalog side and the endpoint:
  - harvested records as DNB editions, with the placeholder year
  - a deleted header is never mapped
  - a multi-ISBN record stays one work
  - no VLB text or URL stored
  - acceptance: a deleted record withdraws its edition, drops it from new-releases and still resolves by ISBN; a later live record revives it
  - the same for a record with two ISBNs ("Die kleine Glitzerblume" of the reiheA fixture, made an announcement with a 263 in the test): the work leaves new-releases with both editions, other titles stay, and a live record brings it back
  - a record ingested while an admin merge of its work runs waits for the merge and lands on the surviving work
  - acceptance: a retitled record replaces the provisional title, and a reiheA record finds the reiheN edition
  - an admin title survives a retitled record
  - acceptance: an announced title is searchable (`/api/books/search`) and resolves through `CatalogLookup::resolve_isbn` and `/api/books/isbn/{isbn}` without upstream requests
  - new-releases: the window and its ends, one hit per work with its earliest edition, the order, the language, paging, parameter errors, 401, the viewer's shelf and club picks, the viewer's time zone
- Unit tests: `harvest::oai` (6) on the recorded fixtures, `harvest::run` (4), `harvest` (1, `wait_after`).
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
   - The last page's transaction, the one that stores its records, also writes `harvested_until` = `run_started_at` − 1 h (the start of the interrupted run, also after a resume or a restart), `last_success_at`, and NULL for the token, `run_started_at`, `run_from`, `last_error` and `last_error_at`.
3. **`records_seen`** counts the records of the current run, deleted ones included. It is reset when a run starts or restarts, and stays at the last run's total afterwards.
4. **Failures** set `last_error` (at most 1000 characters) and `last_error_at`. The cursor stays at the last committed page. Errors include:
   - transport errors and statuses (after the `RetryPolicy::JOB` retries)
   - OAI errors other than `noRecordsMatch` and `badResumptionToken`
   - a page that is not OAI-PMH, or is truncated (no closing root element)
   - a page that deadlocks three times
   - a token rejected more than three times in one run (`HarvestError::TokenRejected`)
   - a page whose next token is the token it was asked with, which would page forever (checked before the page is committed)
5. **Skipped records.** A record that the mapping refuses (no 001, no usable title) is skipped with a WARN line. Each record's ingest runs in a savepoint. A record that ingest refuses with an error that is a property of the record is skipped too, logged at ERROR, because it would fail its page on every later run (`run::refuses_record`). These errors are:
   - SQLSTATE class 22 (data exception) or 23 (integrity constraint violation)
   - a value sqlx cannot decode (`Decode`, `ColumnDecode`)
   - ingest's own refusal (`Protocol`)
   - `RowNotFound`: a row ingest's own statements should have found. Works and editions are never deleted, and the record's advisory locks keep concurrent ingests out, so a retry would meet it again. This one is not in the review's list; without it, such a record would stop the harvest for good.

   Every other error fails the page, and the cursor stays at the last committed page, so the record is tried again: a lock timeout (55P03), a cancelled statement (57014), a lost connection, a full pool. A deadlock (40P01) rolls back and retries the whole page (conventions §7.2).
6. **Deleted headers** take the IDN from the OAI identifier's last segment (`oai:dnb.de/dnb-all:reiheN/1168979633`), because the metadata is never mapped. An identifier without a usable IDN is skipped. `withdraw_dnb_edition` inserts nothing for an unknown IDN.
7. **End of a list:** no `<resumptionToken>` element (the DNB's answer when the list fits on one page), an empty one, and `noRecordsMatch` all complete the run.
8. **At least 1 s between pages** comes from the DNB OAI host throttle in `outbound.rs` (1 request per second, evenly spaced). HAR adds no sleep of its own. `pages_are_at_least_a_second_apart` runs two pages with the production throttle.
9. **503 `Retry-After`** is `RetryPolicy::JOB` in `outbound::get_bytes`: up to three waits of at most 5 min, then the single 5xx retry.
10. **Schedule.**
    - `run_forever` runs a pass under `jobs::run_locked(…, lock_ids::DNB_HARVEST, …)`, then sleeps until the next set is due, at least 60 s (`RECHECK`) and at most the interval.
    - The interval is `DNB_HARVEST_INTERVAL_MINUTES`, never below 30 min (`run::interval`; the configuration already refuses less).
    - A set is due when it has had no failure within its back-off, and either a run is in progress (interrupted, so it resumes at once) or no run completed within the interval.
    - The back-off after a failure is `RESUME_RETRY` (30 min) while the run in progress holds a resumption token younger than 6 h, and a whole interval otherwise. Without the short back-off, any interval of 6 h or more (`DNB_HARVEST_INTERVAL_MINUTES` accepts up to a day) would let the token go stale during the wait, so every failure in the middle of a long run (a 502 or a 429 cooldown on page 3000 of a 90-day first run) would start the run over from `run_from`. A failure before the first page, or with a stale token, waits the interval: that run starts over anyway.
    - The sleep between passes is `wait_after(next_due_in, interval)`: at least `RECHECK` (60 s), so a due set that cannot run never makes the job spin, and at most the interval.
    - Because this state lives in `harvest_state`, a restart or a second instance never harvests a set early.
    - The first pass starts right after startup.
11. **Writes during a merge** (CAT's rule for writes that store a `work_id`, `notes/cat.md` "For SHELF, POSTS and CLUB" and deviation 22). After `upsert_dnb_record`, the record's savepoint locks the works its editions landed on `FOR NO KEY UPDATE`, in id order. A merge that is already moving the loser's editions cannot see an edition inserted meanwhile. The lock waits for that merge; when the work turns out to be merged away, the savepoint is rolled back and the record ingested again, now into the surviving work (ingest's grouping skips merged-away works). It does this twice at most (`MERGED_RETRIES`); after that the record is kept where it landed, with a WARN line. A merge that starts later waits for the page and moves the edition itself. The lock comes after ingest's advisory and edition locks, in the order ingest uses, as in CAT's `insert_user_edition`. A deadlock with a merge is possible, as before (ingest's work update after the edition lock): the page is retried.
12. **Shutdown** is checked between pages and also stops a page request in flight. The page transaction is never cancelled. The run returns `completed: false`, and the next start resumes it.
13. **Sets** run one after the other in `DNB_HARVEST_SETS` order. A failing set does not stop the others, and the pass then fails with every set's error (logged by `run_locked`).
14. **new-releases parameters:**
    - `language` defaults to `ger` and must be three lowercase letters. Otherwise the answer is 400 `String should match pattern '^[a-z]{3}$'`, the pydantic wording CAT uses for the same field.
    - `page` is 1 or more, and `limit` is 1 to 40 with default 24 (the search bounds).
    - Errors use `Validator`'s pydantic wording.
15. **new-releases content:**
    - "Today" is in the caller's time zone (`users.timezone`), like other personal dates. The window is today − 30 to today + 120 days, both ends included.
    - One hit per work: the edition with the earliest date in the window, the lowest edition id on a tie.
    - The hit is built with that edition as the `BookRef` context, so the edition's own title and cover win (`books::book_outs`). `isbn` is that edition's ISBN-13.
    - Merged-away works are excluded.
    - **The other ISBNs of a deleted record** are left out too. A deleted header withdraws only the edition that carries the IDN, the first ISBN's (conventions §7.5, frozen `withdraw_dnb_edition`), and a multi-ISBN record's other editions carry no IDN. So an edition is skipped when it has no IDN, its planned date came from the DNB (`field_sources.planned_publication_date = 'dnb'`), and an edition of the same work with the same planned date is withdrawn. A later live delivery un-withdraws the first edition, and the others come back with it. This is a workaround inside HAR's files until the frozen interface changes (see "Open points"). What it does not cover: another ISBN of the record that ingest found under a different work keeps that work and stays listed; and an IDN-less DNB edition of another record of the same work with the same date is hidden while its work is listed through that record's own first edition.
    - `on_shelf`, `shelf_id` and `club_picks` use the same queries as CAT's search hits (`bc_member_clubs`), duplicated in `new_releases.rs` because `catalog::hits` is not a frozen interface. After Wave 2, the integrator may point both at one function.

## For FE (#48)

- New route: `GET /api/books/new-releases?language=ger&page=1&limit=24` → `SearchPage`. `has_more` is set, and every hit has `planned_publication_date` (`YYYY-MM-DD`). The route makes no remote calls and needs a session (401 `Not signed in`).
- No new detail strings to map. The only new 400 is the pydantic pattern message for `language`, which the app never triggers when it sends `ger`.
- The list is empty until the harvester has run (`DNB_HARVEST=true`).

## Review fixes

The review of HAR found, and this round fixed:

- A deleted multi-ISBN record left its work in new-releases: decision 15 and "Open points".
- With an interval of 6 h or more, a failed run could never resume from its token: decision 10.
- Transient ingest errors skipped a record for good: decision 5.
- `an_announced_title_is_searchable_and_resolvable` compared the fixture date with the clock and would have failed from 2028-03-01. It now asserts the recorded date.
- No test covered the one-transaction rule on a page with a token: two new tests in `harvest_run.rs`.
- The job tests could not see a missing wait between passes and slept 500 ms instead of synchronising. `wait_after` and `next_wait` are tested now, and the skip test calls `run_pass` with the lock held.

The sync with `phase1-green` (POSTS and the FakeCatalog fix) also brought CAT's rule for writes during a merge into view: decision 11.

## For PAR (#49)

- No Python tests to port or reason away: the Python app has no DNB harvester and no new-releases route.
- To check by hand against the real DNB: `DNB_HARVEST=true` (with `DNB_HARVEST_START_DAYS=1` for a short first run), then `SELECT * FROM harvest_state` and `GET /api/books/new-releases`.

## Open points

- **Blocked on a frozen interface: a deleted multi-ISBN record withdraws only its first edition.** `dnb_marc::map_record` gives only the first ISBN's edition the record's IDN (`editions_dnb_idn_key` is unique), and `ingest::withdraw_dnb_edition(conn, idn)` runs `UPDATE editions SET withdrawn_at = now() WHERE dnb_idn = $1`. The other editions of the record stay live (`withdrawn_at` NULL, same language and planned date). HAR cannot fix this without mapping the deleted metadata, which work-plan HAR forbids. HAR's `new_releases.rs` hides those editions (decision 15), but search, book detail and every other reader of `withdrawn_at` still see them as live. **The exact change, for the integrator or a later wave (W1 files `catalog/shared/ingest.rs`, `dnb_marc.rs`, and the schema):**
  1. Record the IDN of the delivering record on **every** edition it maps, in a non-unique column, for example `editions.dnb_record_idn text NULL` with an index (migration in a reserved block, or `0001_init.sql` before the squash). `EditionRecord` gets the field, `map_record` sets it on each edition (`idn.clone()`, not only `first`), and `insert_edition` / `update_edition` write it under the same rule as `dnb_idn` (the newest record's IDN wins).
  2. `withdraw_dnb_edition` withdraws by it: `UPDATE editions SET withdrawn_at = now(), updated_at = now() WHERE (dnb_idn = $1 OR dnb_record_idn = $1) AND withdrawn_at IS NULL`.
  3. HAR then drops the sibling clause from `new_releases_page`. `harvest_catalog.rs::a_deleted_multi_isbn_record_leaves_new_releases` should keep passing, and could also assert that both editions are withdrawn.
- No file outside HAR's ownership was changed. No migration was needed: `harvest_state` and `editions.withdrawn_at` and `planned_publication_date` come from `0001_init.sql`.
