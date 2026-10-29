# COVER (#74): covers for every catalog and imported book

Owner: COVER. Branch `phase2/cover`, worktree `../bookclub-cover`, Postgres `bookclub-pg-cover` on 127.0.0.1:5458, browser port 8087, migrations in the block 2300–2399. These notes hold the design first; the implementation adds what it built, its deviations and what the integrator moves into `conventions.md`, `endpoint-map.md`, `CLAUDE.md`, the READMEs and the legal documents (§8.3).

Scope: #74. Rafael's requirement (2026-10-29): every book from a catalog or the legacy import ends up with a real cover; a book a reader added (`works.source = 'user'`) may stay without one. Google covers stay out (`docs/plan.md`, "Catalog"). The sources are Open Library (fetched by cover id, served from our domain) and pictures people upload.

## Status

Design only (this file). Nothing is built yet.

## Summary

- **Who is searched:** every work that is not a reader's own book, has no usable cover (none, or an Open Library cover that Open Library answered 404 for), is not merged away, and has no open `cover_replace` request. A work whose edition has a cover takes that one first, without a search (§1).
- **How:** one Open Library `search.json` call per work, by title and the first author's surname, 10 results, with each result's best edition in the book's language. A **clear** match (same main title, same numbers, same first author, one book) attaches the cover to the work with provenance `openlibrary`. An **uncertain** one becomes a cover suggestion for the admin. Nothing else attaches (§2).
- **Never:** a `user` or `admin` cover is replaced; a reader's own book is changed automatically; a taken-down picture comes back; a rejected candidate is suggested again for that book. After a takedown of a book's cover, its search only suggests (§1, §2.5).
- **State:** migration `2300_cover_search.sql` adds `cover_search` (per work: when, outcome, backoff, takedown) and `cover_suggestions` (the admin's review list, which also remembers rejections for good). No foreign key to `users` or `media` (§3).
- **Where:** inside `import-legacy` (in its transaction, so the dry run reports what the real run does), in `rematch-legacy`, in a background job every 10 minutes (30 works a run, one search every 2 s, books readers use first), and on an admin's "Search now" (§4).
- **Gaps visible:** the book page asks a reader who has the book on their shelf for a photo when it has no cover; a new admin tab "Covers" lists the suggestions and the books without a cover; an instance admin sets any work's or edition's cover directly (§5).
- **Cutover:** the import report gets a `cover search` line and a `without a cover` section (titles only in the private report file); `cutover.md` gets the review step (§6, §8.2 step 9).

## 1. Candidates

A work is a **candidate** when all of these hold:

1. `merged_into_id IS NULL`.
2. `source <> 'user'`. A reader's own book is never searched automatically (#74 AC 3). Only an admin's "Search now" may search one, and then it only suggests (§2.3, reason `reader_book`).
3. **It has no usable cover:** `cover_media_id IS NULL`, or it points at an Open Library media row that is dead (`storage_key IS NULL AND fetch_failed_at IS NOT NULL`: Open Library answered 404 or 410, or sent no usable picture; `media::service`). A dead cover shows the cloth after one failed request, so for the reader it is no cover. A cover whose bytes were never fetched yet counts as a cover. One SQL function in the migration, `bc_cover_missing(cover_media_id bigint) RETURNS boolean` (STABLE), is the only definition; the job, the admin list, the summary and the report all use it.
4. **No admin left it empty on purpose:** not (`cover_media_id IS NULL` and `field_sources->>'cover_media_id' = 'admin'`). No code path writes that today, but `ingest::decide` already treats an admin-cleared field as closed, and the search follows it.
5. **No open `cover_replace` request** on the work or one of its editions. A reader already offered a picture (one exists only where a cover existed, so this is a dead cover with a waiting photo): the admin decides that first.

Rules for the special cases the task names:

- **Editions have covers, the work field is empty** (a takedown cleared the work's own cover, or a merge left it empty): the work takes the cover of its lowest-id edition whose cover is usable, through `ingest::set_cover(Work, media, by = that edition's cover provenance, only_if_empty = true)`, without asking Open Library. Outcome `inherited`. This also runs for a work under takedown (§2.5): the edition's picture is a different picture of the same book.
- **Removed (taken-down) pictures:** `ingest::ol_cover_media` already returns `None` for a removed row, so a taken-down Open Library cover id is never attached or suggested again; the search treats such a candidate cover as absent. A dead cover id (point 3) is skipped the same way.
- **A takedown of the book's cover** (any picture: a reader's photo, an imported cover, one the search attached): `ingest::clear_media` also records `cover_search.takedown_at` for every work it cleared. From then on the search never attaches automatically for that work (or a work merged into it); a clear match becomes a suggestion with reason `after_takedown`. The takedown answered a report ("wrong book", "offensive", "copyright"); whether another picture of the same Open Library record is right is for a person to say.
- **A reader's photo:** a work with a reader's photo as its cover is no candidate (it has a cover). A reader's photo on an edition fills the empty work field (rule above).
- **Readers' own books:** never candidates; the import report counts them separately (§6), and the book page keeps only the small "Add a photo of your copy" link for them (§5.4).

**Tiers.** The job searches **tier 1** first: candidates **in use**, meaning a row in `shelf_entries`, `user_favorites`, `quotes`, `club_picks`, `next_up_nominations`, `posts` or `legacy_work_keys` names the work. **Tier 2** is the other `dnb` and `legacy` candidates, except DNB works not published yet (every edition has a `planned_publication_date` in the future: Open Library does not have them, and the search would only cost requests). Open Library works nobody uses are not searched: Open Library's own record had no cover, and only a duplicate record could help. A tier-2 search never creates a suggestion: an uncertain result is recorded as `uncertain` and searched again as soon as the work is in use (§4.4), so the admin's list holds only books people read.

## 2. The matching rule

Pure functions in `server/src/catalog/covers/matching.rs`, unit-tested with tables. Conservative throughout: a wrong cover is worse than none.

### 2.1 The query

One request per work and attempt:

```
GET {OPENLIBRARY_URL}/search.json?title=<main title>&author=<surname>&limit=10
    &fields=key,title,subtitle,author_name,cover_i,edition_count,first_publish_year,language,
            editions,editions.key,editions.title,editions.cover_i,editions.language
    [&lang=<ISO 639-1>]
```

- `title` is the work's main title as stored (brackets dropped, cut before the subtitle, §2.2), not folded: Open Library does its own matching. `author` is the first author's surname as written; the full comparison happens locally (§2.3). Without an author, `author` is left out.
- `lang` (Open Library: "influences but doesn't exclude"; it makes `editions` hold the edition in that language when the work has one) comes from the work's editions' `language` (the most frequent MARC code, mapped to ISO 639-1: ger→de, eng→en, fre→fr, spa→es, ita→it, dut→nl, and a few more), else from the ISBN's registration group (978-3 → de, 978-0 and 978-1 → en, 978-2 and 979-10 → fr), else nothing.
- `editions` holds **one** edition per work, the one Open Library ranks best for the query and `lang` (its documentation: "Currently only one edition is displayed").
- 10 results: enough to see Open Library's duplicate records of one book (§2.3 "one book") in a single request; the matching work is normally among the first.
- Through `CatalogService::ol_json`: the `ol_api` throttle (3 a second, shared with readers' searches, at most 2 s of waiting), the 429 cooldown, one retry on 502/503/504, the 18-hour response cache (key `cover|<folded title>|<folded author>|<lang>`), empty answers not cached.

The answer is mapped leniently like every Open Library payload (spec R13): non-objects, keys that are not `/works/OL…W`, untitled docs and duplicates are skipped; cover ids must be positive.

### 2.2 Normalisation

`fold(text)`, for titles:

1. Unicode NFKD, combining marks dropped (diacritics: é → e, ü → u), then `text::casefold` (ß → ss).
2. `&`, `und` and `et` become `and`.
3. An apostrophe after a word-initial `l`, `d` or `j` becomes a space (`L'Étranger` → `l etranger`); every other apostrophe is dropped (`Hitchhiker's` → `hitchhikers`). Every other character that is not a letter or digit becomes a space; whitespace collapses.
4. One leading article is dropped: the, a, an, der, die, das, ein, eine, le, la, les, l, un, une, el, los, las, il, lo, gli, de, het, een.

`main(title)`: the parts in `(…)` and `[…]` are dropped, the title is cut at the first `:`, ` - `, ` – `, ` — `, `;` or ` / `, then folded. A main key that folds to nothing never matches.

`numbers(title, subtitle)`: the set of numbers in the whole title and subtitle, brackets included, after folding: digit runs (leading zeros dropped) and the whole-word Roman numerals ii to xx. `Band 2`, `#2`, `Volume II` and `(The Expanse, 3)` all give {2} or {3}. Years count too: `1984` must agree with `1984`.

`name(text)`, for an author: folded like a title, with periods and hyphens as spaces; a name with exactly one comma is inverted (`Le Guin, Ursula K.` → `ursula k le guin`). The **surname** is the last word; the **initial** is the first letter of the first other word, if any. Our first author is the first `work_authors` record (with `authors.sort_name`, the part before its comma is the surname, as `ingest::author_surname` reads it), else the first `", "`-separated name of `works.author_names`. Two names **agree** when the surnames are equal and the initials are equal or one of them is missing (`J. R. R. Tolkien` = `John Ronald Reuel Tolkien` = `Tolkien`; `Stephen King` ≠ `Tabitha King`).

Title variants: ours are the work title and each distinct edition title (a translation's own title); theirs are the doc's title and its best edition's title. Our subtitle never counts as a title on its own: DNB subtitles are often "Roman" or "Thriller".

### 2.3 Clear, uncertain, no match

For each doc, in Open Library's order:

- **identity:** the doc's key is the work's own `ol_work_id` (Open Library added a cover since our stub). Clear, whatever the titles say.
- **title equal:** some pair of variants has equal, non-empty main keys, and the `numbers` of that pair (each with its subtitle) are equal (`1984` / `1984`).
- **numbers differ:** some pair has main keys that are equal and non-empty once their number words are deleted, but the pair's `numbers` differ (`Die Tribute von Panem` / `Die Tribute von Panem 1`; `Harry Potter und der Stein der Weisen` / the same with `(Band 1)`).
- **title similar:** neither, but every word of the shorter main key is a word of the longer one (`Dune` / `Dune Messiah`).
- **author agrees:** our first author agrees with one of the doc's `author_name` entries (Open Library may list an illustrator or translator too).

The verdict for the work:

| Verdict | When | What happens |
|---|---|---|
| **clear** | identity; or title equal + author agrees, our book has an author, and the doc is the only such Open Library work or **dominates** them (its `edition_count` is at least 3 times that of every other one) | attach (§4.1), unless the work is under takedown (→ suggestion `after_takedown`) or the search is an admin's on a reader's book (→ suggestion `reader_book`) |
| **uncertain** | title equal + author agrees, but no doc dominates (`several_works`: the one with the most editions is suggested); numbers differ + author agrees (`number_differs`); title similar + author agrees (`title_differs`); title equal and our book has no author at all (`no_author`) | one suggestion, the best by that order of reasons, then `edition_count`, then Open Library's order (tier 2: recorded as `uncertain`, no suggestion) |
| **no match** | anything else, including the same title by another author | nothing; searched again later (§4.4) |

Candidates are filtered before the verdict: an Open Library work rejected for this book (or for a book merged into it) is out, and a cover id whose media row is removed or dead is treated as absent. A doc with no usable cover still counts for "dominates" (it is a duplicate record of the same book or another book of the same name), but a verdict whose chosen work has no usable cover is **no match**.

### 2.4 Which cover of the matched work

1. The best edition's cover (`editions.docs[0].cover_i`), when usable and the edition is in the book's language (§2.1) or its title is the matched title variant: the German edition's cover for a German book, the translation's for a translation.
2. Else the work's cover (`cover_i`, Open Library's pick, usually the most common edition).
3. Else the best edition's cover in another language.

The suggestion and `cover_search` record the Open Library work id, the edition id when the cover is an edition's, and the cover id.

The cover goes on the **work**, never on one of our editions: it shows another edition of the same book, so it would be wrong as the cover of a specific ISBN. `BookOut` in an edition context falls back to the work's cover, so every edition shows it until it has its own.

### 2.5 False positives considered

- **Same title, different book:** the first author must agree; a single-word title (`Momo`, `Circe`) is fine with the author. Two different books with one title by one author (collections such as `Gedichte`, `Briefe`) are the case "dominates" exists for: Open Library's duplicate records of one book are a main record and stubs with one or two editions, while two real books both have several. No dominance means a suggestion.
- **Volumes and series:** `numbers` must agree for a clear match, so volume 2 never gets volume 1's cover automatically. A number on one side only (`Band 1` missing on ours) is `number_differs`, a suggestion.
- **Subtitles:** only the main title counts, so `Dune: Der Wüstenplanet` matches `Dune`; but a subtitle never stands in for a title.
- **Translations:** a German title matches through the German edition of an English work (title variant of the best edition, with `lang=de`), and that edition's cover is chosen. A German title with no German edition on Open Library is no match (not a guess through the English title).
- **New translations or editions of an old work:** they get the old edition's cover. That is what #74 asks for ("the cover of another edition"); the reader's photo or an admin's upload can still replace it (rank `user`/`admin` > `openlibrary`).
- **Books without an author:** never clear; an exact title only suggests.
- **After a wrong cover:** a reader reports it ("wrong book"), the admin takes it down; the picture never comes back (§1), and the book only gets suggestions from then on.
- **A reader is faster:** the search re-checks under the work's row lock that the work still lacks a usable cover; `set_cover(…, only_if_empty)` for an empty cover and the rank rule for a dead one never replace a `user` or `admin` cover.

## 3. Data model: `server/migrations/2300_cover_search.sql`

```sql
CREATE FUNCTION bc_cover_missing(cover_media_id bigint) RETURNS boolean
    LANGUAGE sql STABLE PARALLEL SAFE
    RETURN cover_media_id IS NULL
        OR EXISTS (SELECT 1 FROM media m
                   WHERE m.id = cover_media_id
                     AND (m.removed_at IS NOT NULL
                          OR (m.storage_key IS NULL AND m.fetch_failed_at IS NOT NULL)));

-- One row per work the search has seen; no row = never searched (due).
CREATE TABLE cover_search (
    work_id              bigint PRIMARY KEY REFERENCES works (id) ON DELETE CASCADE,
    searched_at          timestamptz NULL,      -- the last search Open Library answered
    searched_key         text NULL,             -- works.match_key at that search
    outcome              text NULL CHECK (outcome IN
                             ('attached', 'suggested', 'uncertain', 'no_match', 'inherited')),
    misses               integer NOT NULL DEFAULT 0 CHECK (misses >= 0),
    failures             integer NOT NULL DEFAULT 0 CHECK (failures >= 0),
    next_search_at       timestamptz NULL,      -- not before; NULL = due
    last_error           text NULL CHECK (char_length(last_error) <= 300),  -- never a title
    takedown_at          timestamptz NULL,      -- ingest::clear_media: suggestions only from now on
    attached_ol_work_id  text COLLATE "C" NULL CHECK (attached_ol_work_id ~ '^OL[0-9]+W$'),
    attached_ol_cover_id bigint NULL CHECK (attached_ol_cover_id > 0)
);
CREATE INDEX cover_search_next_idx ON cover_search (next_search_at);

CREATE TABLE cover_suggestions (
    id            bigint GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,
    work_id       bigint NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    ol_work_id    text COLLATE "C" NOT NULL CHECK (ol_work_id ~ '^OL[0-9]+W$'),
    ol_edition_id text COLLATE "C" NULL CHECK (ol_edition_id ~ '^OL[0-9]+M$'),
    ol_cover_id   bigint NOT NULL CHECK (ol_cover_id > 0),   -- the media row: media.ol_cover_id
    title         text NOT NULL CHECK (char_length(title) BETWEEN 1 AND 1000),
    authors       text NOT NULL DEFAULT '' CHECK (char_length(authors) <= 2000),
    year          integer NULL CHECK (year BETWEEN -5000 AND 3000),
    language      text NULL CHECK (language ~ '^[a-z]{3}$'),
    reason        text NOT NULL CHECK (reason IN ('title_differs', 'number_differs', 'several_works',
                                                  'no_author', 'after_takedown', 'reader_book')),
    status        text NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'approved', 'rejected', 'closed')),
    created_at    timestamptz NOT NULL DEFAULT now(),
    decided_at    timestamptz NULL,
    CONSTRAINT cover_suggestions_decided CHECK ((status = 'open') = (decided_at IS NULL)),
    CONSTRAINT cover_suggestions_candidate_key UNIQUE (work_id, ol_work_id)
);
CREATE UNIQUE INDEX cover_suggestions_one_open_key ON cover_suggestions (work_id) WHERE status = 'open';
CREATE INDEX cover_suggestions_open_idx ON cover_suggestions (created_at, id) WHERE status = 'open';
CREATE INDEX cover_suggestions_decided_idx ON cover_suggestions (decided_at DESC, id DESC) WHERE status <> 'open';
-- The candidate scan of the job and the admin list.
CREATE INDEX works_cover_missing_idx ON works (id)
    WHERE cover_media_id IS NULL AND merged_into_id IS NULL AND source <> 'user';
```

Plus a `COMMENT ON` for every table and column, as the other migrations do.

Decisions:

1. **A table of its own for suggestions, not a `catalog_edit_requests` kind.** Suggestions are made by the server, not filed by a reader: they have no `created_by`, must not count against the readers' request limit, the 72-hour response target, the overdue badge or the daily digest mail (`admin::notify`), and a rejection must outlive the 3-year retention that `auth::cleanup` applies to decided requests. A new kind would also have to loosen `catalog_edit_requests_cover` (`media_id` only for `cover_replace`) and the takedown's request rejection. Approve and reject reuse the queue's patterns (`QueueQuery`, `REQUEST_DECIDED`, the lock order).
2. **Statuses.** `open`; `approved` (an admin attached it); `rejected` (an admin said "not this book": that Open Library work never comes back for this book, or for a book merged into it, because `merge_works` points every older loser at the new winner); `closed` (the server closed it: the book got a cover, was merged away, or the picture was taken down). Only `rejected` blocks: a `closed` candidate may be suggested again later (the unique key is reused with `ON CONFLICT … DO UPDATE … WHERE status = 'closed'`). At most one open suggestion per book keeps the list short; after a rejection the book is due at once, so the next candidate follows.
3. **Rejection is per Open Library work, not per cover id.** Suggestions exist only for uncertain matches, and the uncertain part is "is this the same book". Blocking only the picture would bring the same wrong book back with its next cover. A right book with a bad picture costs the admin an upload instead.
4. **No foreign key to `users`.** Who approved is not stored (the cover's provenance `admin` says an admin did); so the export (§5.7 and `auth_export.rs`'s pinned count), the purge of empty sign-ups (`jobs.rs`) and `parity_visibility.rs` need no change.
5. **No foreign key to `media`.** A suggestion names its picture by `ol_cover_id`; the media row (created by `ingest::ol_cover_media` before the suggestion is written) is found through the unique `media.ol_cover_id`. Open Library media rows are never deleted (`delete_rejected_cover_photos` deletes only uploads of rejected `cover_replace` requests), so `delete_rejected_cover_photos` and `MEDIA_REFERENCES` need no change.
6. **No personal data.** Both tables are catalog state: `parity_schema.rs`'s `GLOBAL` list and `docs/phase1/parity-checklist.md`'s "Neither" row gain them. No retention rule: rows are small, and rejections must stay.
7. **`searched_key`** is `works.match_key` (title and first surname, kept by ingest) at the search: an admin's correction or a `rematch-legacy` upgrade changes it, and the book is due again at once, with `misses` reset.

## 4. Where it runs

### 4.1 One search of one work

`catalog::covers::search` has three steps, so each caller decides where its transaction ends:

1. **facts** (read): the work's titles, first author, language hint, `ol_work_id`, current cover state, takedown flag (its own or a merged-away work's), rejected Open Library works, open `cover_replace`. `None` when it is no candidate (§1). The inherit step (§1) needs no Open Library call and returns at once.
2. **remote:** `CatalogLookup::cover_candidates(&CoverQuery)` (§4.6), outside any transaction for the job, the admin and `rematch-legacy`; inside the import's transaction for `import-legacy` (it already makes its ISBN lookups there).
3. **apply** (one transaction, or the import's): media first, as everywhere (conventions §7.2): `ingest::ol_cover_media(conn, cover_id)` locks the picture `FOR SHARE` (`None`: taken down meanwhile → no match); then the work `FOR UPDATE`, re-checked (still a candidate, same `match_key`; otherwise nothing is written and the book stays due); then `ingest::set_cover(Work, media, Source::OpenLibrary, only_if_empty = cover_media_id IS NULL)` or the suggestion insert (a conflict on either unique key writes nothing; a `closed` row of the same Open Library work is reopened); then the `cover_search` upsert. A clear match commits and then, in the server process, queues the picture for `MediaService::prefetch`, so the bytes arrive (or the dead id shows) before a reader opens the book.

**Lock order: media → works → cover_suggestions → cover_search.** A takedown takes media `FOR UPDATE` → works (`clear_media`, which now writes `cover_search`) → reports → requests; approval reads the suggestion without a lock (its picture and work never change), then takes the picture `FOR SHARE` → the work `FOR UPDATE` → the suggestion `FOR UPDATE` (still `open`?); rejection takes only the suggestion and `cover_search`. No path takes them the other way round. A race test per pair (the job against an approval of the same book, a takedown against an attach).

`cover_search` after each outcome:

| Outcome | Written | Due again |
|---|---|---|
| attached / inherited | `outcome`, `misses = 0`, `failures = 0`, and for attached `attached_*` | when the work loses its cover |
| suggested | `outcome`, `misses = 0` | after the suggestion is decided (rejected: at once) or closed |
| uncertain (tier 2) | `outcome`, `misses + 1`, `next_search_at` as for no match | as soon as the work is in use, else then |
| no match | `misses + 1`, `next_search_at` = now + 30 days × 2^(misses − 1), at most 180 days | then |
| Open Library unavailable | `failures + 1`, `last_error`, `next_search_at` = now + 1 h × 2^(failures − 1), at most 24 h | then |
| throttled | nothing | at once (the caller stops or waits) |
| title or first author changed (`searched_key` ≠ `match_key`) | – | at once, `misses` reset |

### 4.2 `import-legacy`

- **When:** right after the books step (`import_books`), inside the import's transaction, over every imported work that is a candidate, in legacy book id order (deterministic report). User works (`BC…`) are counted, not searched.
- **Dry run:** the transaction is rolled back like everything else, so the dry run reports exactly what the real run will do. The dry and real reports stay equal unless Open Library's answer changes in between (the same caveat as the ISBN lookups, `cutover.md` 3.3).
- **Errors never abort the import:** a cover is not part of the data. Throttled: wait `--retry-wait-ms` (default 60 s), counted in `lookup wait`, at most 3 waits per work; once one work gives up, the rest are "not searched". Unavailable: that work is "not searched", whatever `--allow-unmatched` says. A not-searched work has no `cover_search` row, so the job takes it first after the cutover (tier 1, `legacy_work_keys`).
- **`--offline`:** no search; the report says so.
- **Downtime:** one request per coverless book at most 3 a second. For the rehearsal's library (51 books; at least the 15 unmatched ISBN rows and the 2 Google rows had no cover) that is under 10 s per run, twice. The prefetch does not run here (no server process); the first view fetches the bytes.

### 4.3 `rematch-legacy`

After the ISBN pass, every imported (`legacy_work_keys`) candidate is searched, each in its own transaction (rolled back with `--dry-run`), ignoring `next_search_at` and `misses` (the operator asked for it now), but respecting rejections, open suggestions and takedowns. Throttling is waited out as for its lookups. The report gets the same `cover search` line and `without a cover` section (§6). This is the operator's way to run the search again for the imported books, for example after Open Library was down during the import.

### 4.4 The background job

- `jobs::cover_search_loop`, started by `jobs::spawn_all` (not in tests), every `COVER_SEARCH_EVERY` = 10 minutes (the first tick at startup), one `jobs::run_locked` run under the new `lock_ids::COVER_SEARCH = 0x6263_0004`, so a second instance skips it.
- A run first closes moot suggestions (`closed`: the book has a usable cover, was merged away, or the picture was taken down), then takes up to `COVER_SEARCH_BATCH` = 30 due candidates: tier 1 before tier 2, never searched first, then `next_search_at`, then work id. Due = no row, or `next_search_at` is NULL or past, or `searched_key` changed, or outcome `uncertain` and now in use; and no open suggestion.
- Between two Open Library searches it waits `COVER_SEARCH_PACE` = 2 s, so the job uses at most a sixth of the shared 3 requests a second while it runs (about a minute and a half per run, at most 4,320 searches a day). A throttled search (the 2 s throttle wait was not enough, or a cooldown after a 429) ends the run with nothing recorded: readers' searches go first, and the next run resumes. An unavailable answer records the failure on that work (backoff) and ends the run, so an outage costs one request per run.
- Each work commits on its own, so a crash or a restart loses at most the work in flight.
- The log has counts and work ids only (`cover search: searched`, `attached`, `suggested`, `no_match`, `failed`, `stopped`), never titles.
- No setting turns it on or off: it is paced, backs off and yields to readers, and with nothing to search it sends nothing (CI's smoke test boots an empty database). No new environment variable.

### 4.5 No separate CLI command

`find-covers` is not added: `import-legacy` and `rematch-legacy` cover the imported books (the only batch an operator reviews), the job covers the rest, and the admin page searches one book on demand. One fewer command to document and test.

### 4.6 The Open Library seam: `CatalogLookup::cover_candidates`

`server/src/catalog/shared/lookup.rs` (frozen, W1; an additive change the integrator records in conventions §7.4):

```rust
/// What the cover search asks Open Library (#74).
pub struct CoverQuery {
    pub title: String,               // the main title as stored (§2.2), not folded
    pub author: Option<String>,      // the first author's surname as written
    pub language: Option<String>,    // ISO 639-1, sent as `lang`
}
/// One Open Library work of the answer, with its best edition for the query.
pub struct CoverDoc {
    pub ol_work_id: String, pub title: String, pub subtitle: String, pub authors: Vec<String>,
    pub cover_id: Option<i64>, pub edition_count: i32, pub first_publish_year: Option<i32>,
    pub languages: Vec<String>,      // MARC codes
    pub edition: Option<CoverEdition>,
}
pub struct CoverEdition { pub ol_edition_id: String, pub title: String, pub cover_id: Option<i64>,
                          pub languages: Vec<String> }

trait CatalogLookup {
    // … the three existing methods …
    /// `search.json` by title and author, at most 10 works; persists nothing.
    /// Ok(vec![]) = Open Library answered with none. Err(Throttled/Unavailable) = no answer.
    async fn cover_candidates(&self, query: &CoverQuery) -> Result<Vec<CoverDoc>, LookupError>;
}
```

- `CatalogService` implements it with `ol_cover_search` in `catalog/openlibrary.rs` (`COVER_SEARCH_FIELDS`, `COVER_SEARCH_LIMIT = 10`, `map_cover_docs`).
- On the trait rather than a second one, because `AppState`'s fields are frozen and the importer, `rematch-legacy` and the job already hold `Arc<dyn CatalogLookup>`: production uses the real client, tests the programmable `FakeCatalog`, and #74's acceptance criterion asks for a FakeCatalog test.

## 5. API and SPA

### 5.1 Routes (all `/api/admin/*` behind `InstanceAdmin`, tag `admin`)

| Route | Request → Response | Notes |
|---|---|---|
| `GET /api/admin/cover-suggestions` | `QueueQuery` (`?status=open\|resolved&page=n`) → `CoverSuggestionList {items, page, has_more}` | `open`: oldest first, hiding moot ones (book has a usable cover, merged away, picture removed); `resolved`: newest decision first. 50 a page. Operation `catalog_admin_list_cover_suggestions`. |
| `POST /api/admin/cover-suggestions/{suggestion_id}/approve` | → `CoverSuggestionOut` | Attaches the picture to the current work (merges followed) through `set_cover(…, Source::Admin, false)`, only while it still lacks a usable cover; records `attached_*`; prefetches. 404 `Not found`, 404 `No such book.`, 400 `This picture was removed`, 409 `This book has a cover now`, 409 `This request was already decided`. |
| `POST /api/admin/cover-suggestions/{suggestion_id}/reject` | no body → `CoverSuggestionOut` | "Not this book": `rejected`, the book due at once. 404, 409 as above. |
| `GET /api/admin/missing-covers` | `?scope=in_use\|all&page=n` → `MissingCoverList {items, page, has_more}` | Candidates (§1) and readers' books excluded. `in_use` (default): tier 1. `all`: every catalog and imported work without a usable cover. Order: in use first, then `lower(bc_unaccent(title))`, then id. 400 `Input should be 'in_use' or 'all'`. |
| `POST /api/admin/works/{work_id}/cover-search` | no body → `CoverSearchOut` | Searches this book now: ignores the schedule and misses, respects rejections and takedowns; a reader's book only gets a suggestion (`reader_book`). Counts against `upstream_turn` (`catalog_user`, `catalog_gate`). 404 `No such book.`, 409 `This book has a cover now`, 429 the catalog's busy strings, 502 `Could not search the library right now. Try again.` |
| `PUT /api/admin/works/{work_id}/cover` | multipart `file`, optional `edition_id` → `CoverSet {book}` | The admin's picture becomes the cover of the work or edition **at once**, replacing any cover (`set_cover(…, Source::Admin, false)`), through the reader upload's pipeline (`media::cover`: metadata stripped, WebP `l`/`s`, removed hashes refused). Same 400/413/429 as `POST /api/books/works/{work_id}/cover`, plus 404. Operation `media_admin_set_cover`; the body limit of the upload routes. |

`AdminSummary` (not a §0 DTO) gains `covers: AdminCoverSummary {suggestions: i64, missing: i64}` (open, non-moot suggestions; tier-1 works without a usable cover). The tab badge shows `suggestions`; no overdue mark (no response target).

The admin's existing tools already cover the rest: "Edit as admin" corrects a title or author (which makes the book due again), a merge joins a bare-facts book to its real record.

### 5.2 DTOs (`catalog::covers::admin`, none in §0)

```text
CoverSuggestionOut { id, status: "open"|"approved"|"rejected"|"closed", work_id,
                     book: BookOut|null,            // the work now (merges followed): its cover is null while it has none
                     cover_url, cover_thumb_url, cover_public_id,   // the candidate, from our /media
                     ol_work_id, ol_edition_id|null, title, authors, year|null, language|null,
                     reason: "title_differs"|"number_differs"|"several_works"|"no_author"|"after_takedown"|"reader_book",
                     created_at, decided_at|null }
CoverSuggestionList { items: CoverSuggestionOut[], page, has_more }
MissingCoverOut { book: BookOut, in_use: bool, imported: bool, dead_cover: bool,
                  searched_at|null, outcome: "attached"|"suggested"|"uncertain"|"no_match"|"inherited"|null,
                  next_search_at|null, failures, after_takedown: bool, suggestion_id|null }
MissingCoverList { items: MissingCoverOut[], page, has_more }
CoverSearchOut { outcome: "attached"|"suggested"|"no_match"|"inherited", book: BookOut,
                 suggestion: CoverSuggestionOut|null }
```

### 5.3 Detail strings

One new string, a constant in `catalog/mod.rs`, mapped in `apiErrors.ts` to `errors.coverExists` (en "This book has a cover now.", de "Dieses Buch hat inzwischen ein Cover."): **`This book has a cover now`** (409 on approve and on "Search now"). The rest are reused: `This request was already decided`, `This picture was removed`, `No such book.`, `Not found`, the catalog's busy and unavailable strings, the upload strings. `Input should be 'in_use' or 'all'` is admin-only and unmapped, like `STATUS_CHOICES`.

### 5.4 Book page: the photo prompt

**Who sees it:** a signed-in reader on a book page whose shown book (the viewer's edition, else the work) has no cover URL, who has the book on their shelf (`BookDetailOut.on_shelf` set, any status), when the book is not a reader's own (`custom` false) and not withdrawn. It replaces the small "Add a photo of your copy" link under the cover (one upload control, not two); everyone else keeps the link as today. The upload is the existing `POST /api/books/works/{work_id}/cover` with the viewer's edition, so the photo becomes that edition's cover and fills the work's empty one (`fill_work_from_edition`).

A reader's own book does not get the prompt: #74 lets it stay with the cloth and monogram, and its reader still has the link.

A dead Open Library cover still has URLs in `BookOut` (the cloth shows after the failed request), so the prompt does not show for it; the search replaces such covers instead. Hiding dead covers in `book_outs` would change a frozen loader for a rare case.

Wording (`cover.prompt.*`, new keys in both files):

| Key | en | de |
|---|---|---|
| `title` | This book has no cover yet | Dieses Buch hat noch kein Cover |
| `body` | It's on your shelf. If you have your copy at hand, a photo of its front gives the book a cover for everyone. Photograph only the cover. | Es steht in deinem Regal. Wenn du dein Exemplar zur Hand hast, gibt ein Foto der Vorderseite dem Buch ein Cover – für alle. Fotografiere nur das Cover. |
| `action` | Add a photo of the cover | Foto vom Cover hinzufügen |

Component: `frontend/src/components/CoverPrompt.vue` (the text and a `CoverPhotoButton` with a button look; `CoverPhotoButton` gets an optional `label` prop). `BookDetailPage.vue` shows it in `.hero-cover` and adopts the new cover as today (`adoptCover`).

**Admin on the book page:** beside "Edit as admin", "Set cover as admin" (and, when the page shows an edition, "Set this edition's cover"): a file input calling `PUT /api/admin/works/{work_id}/cover`.

### 5.5 Admin page: the "Covers" tab

`frontend/src/components/admin/AdminCovers.vue`, a fifth tab `covers` in `AdminPage.vue` (badge from `summary.covers.suggestions`), with two views:

- **Suggestions** (`useQueue`, open/decided): our book (cover or cloth, title, authors, source) beside the candidate (its picture from `/media`, Open Library's title, authors, year, language, a "View on Open Library" link that opens a new tab without a referrer like the book page's), the reason in words, and **Use this cover** / **Not this book**.
- **Without a cover** (`in_use`/`all` switch, pages of 50): each book with its state in words (never searched, no match on <date> and next try <date>, a suggestion waiting, Open Library's picture is gone, after a takedown: suggestions only), and **Search now** (shows the outcome; an attached cover or a new suggestion updates the list), **Upload a picture** (the direct cover), **Open the book**.

The reason and state labels are pure functions in `frontend/src/admin.ts` with tests. i18n keys: `admin.tabs.covers`, `admin.covers.*` (en and de; `messages.test.ts` keeps them in step).

## 6. The import report

`importer::Report` gains `covers: Option<CoverSearchLine>` (counts) and `without_cover: Vec<String>`; `ReportBuilder::lines()` renders them after `lookup wait` and the `isbn keys` section:

```
cover search           20 searched: 12 attached, 8 still without a cover (3 suggested, 4 no match, 1 not searched); 2 books readers added left as they are
...
without a cover:
  book 12 "<title>" by <authors>: suggested, review it on the admin page
  book 14 "<title>" by <authors>: no match on Open Library
  book 15 "<title>" by <authors>: not searched (Open Library unavailable)
```

- "still without a cover" is counted with `bc_cover_missing` over the imported works at the end of the step, not added up, so it is the database's truth. The `without a cover` lines name the legacy book id (stable between dry and real run; work ids are not) and, as the `isbn keys` section already does, titles: **only in the private report file**, which `cutover.md` keeps out of #38 and deletes after the counts are posted. With `--offline` the line reads `cover search           skipped (--offline): 20 without a cover; rematch-legacy searches them`.
- **Not an entity line.** Its name has a space and it has no `→`, so `compare.sh` (whose regex reads `^[a-z_]+ +[-0-9]+ → `) and `cutover_counts.rs` (which compares every entity line with `legacy-counts.sql` and `target-counts.sql`) ignore it; `deploy/cutover/` needs no change. The counts depend on Open Library, not on the legacy data, so they are no count check.
- `rematch-legacy` prints the same line and section for its search.

## 7. Tests

**Unit** (`catalog/covers/matching.rs`): `fold` (diacritics, ß, articles in six languages, `&`/und/et, apostrophes, punctuation), `main` (subtitle separators, brackets), `numbers` (digits, Roman ii–xx, years, `#3`), names (inverted, initials, particles, hyphens), and the verdict table: clear, identity, dominance (3× and just under), `several_works`, `number_differs`, `title_differs`, `no_author`, same title other author, no usable cover, rejected and removed candidates, edition-in-language vs work cover vs other-language edition.

**`FakeCatalog`** (`tests/common/catalog.rs`, add-only): `on_covers(title, author, docs)`, `fail_covers(err, times)` and `cover_calls()`. Cover searches are recorded apart from `calls()`, so the existing importer tests that compare `calls()` keep passing. Unprogrammed queries answer `Ok(vec![])`; `fail_all` applies to them too.

**`server/tests/cover_search.rs`** (FakeCatalog):
- a legacy, a DNB and an Open Library work without a cover each get another edition's cover on a clear match (#74 AC 1), with provenance `openlibrary` on the work and none on our editions;
- the edition in the book's language wins over the work cover;
- an uncertain match becomes one suggestion; approving attaches it with provenance `admin`; rejecting keeps that Open Library work out for good: on the next job run, an admin's "Search now", and after the book is merged into another (#74 AC 2);
- a `user` cover, an `admin` cover and a reader's book are never touched, also when a reader's upload lands between the remote step and apply (#74 AC 3);
- a taken-down cover id never comes back, and a takedown turns the book to suggestions only;
- a dead Open Library cover is replaced; a work takes its edition's cover without a search; an open `cover_replace` keeps the book out;
- the job: throttled ends the run with nothing recorded and the next run resumes; unavailable backs off (1 h, then 2 h) and ends the run; no match waits 30 then 60 days; a changed title is due at once; tier 1 before tier 2; tier 2 never suggests and becomes due when used; moot suggestions close; a second run while the lock is held is skipped (#74 AC 4);
- race tests: a takedown during an attach, the job against an approval of the same book.

**`server/tests/cover_search_client.rs`** (wiremock, `TestApp::with_real_lookup`): the request's `title`, `author`, `fields`, `limit` and `lang`, the User-Agent; the payload mapping (editions, junk docs, cover ids ≤ 0); the 18-hour cache; a 429 gives `Throttled` and the cooldown (the throttle's pacing itself is `outbound`'s test).

**`server/tests/cover_admin.rs`**: every route's success and error strings; the missing list's scopes, order and pages; "Search now" outcomes, 409, 429 and 502; the direct upload replaces a reader's cover, sets an edition's, refuses a removed hash, keeps the upload limits; the summary counts.

**`server/tests/cover_import.rs`** (the legacy fixture harness): `import-legacy` searches the coverless imported works, the report has the line and the section, the dry run's lines equal the real run's, `--offline` skips, an unavailable Open Library does not abort and leaves the works unsearched, readers' books are counted not searched; `rematch-legacy` searches and reports (#74 AC 6).

**Shared checks:** `admin_guard.rs` (`ADMIN_OPERATIONS` gains the six routes, 22 in all; `WATCHED_TABLES` gains `cover_search` and `cover_suggestions`), `parity_schema.rs` (`GLOBAL`), `auth_required.rs` and `openapi_contract.rs` (unchanged lists, they walk the document), `api_error_copy.rs` (the new mapped string). `auth_export.rs` and `jobs.rs` must pass unchanged (no new foreign key to `users` or `media`).

**SPA (Vitest):** `CoverPrompt.test.ts` (shown only with a shelf entry, no cover, not a reader's book; uploads with the edition; adopts the cover), `BookDetailPage.test.ts` (the prompt replaces the link; the admin's "Set cover as admin"), `AdminCovers.test.ts` (approve, reject, a 409, "Search now" outcomes, the scope switch, upload), `AdminPage.test.ts` (the tab and its badge), `admin.test.ts` (reason and state labels). `messages.test.ts` and `thirdParty.test.ts` must pass unchanged.

**By hand:** the server with the built SPA on `http://agents.lan:8087` against a database with synthetic works, and the real Open Library: a few well-known books get covers, a book with a numbered title gets a suggestion, the prompt and the Covers tab in en and de, on a phone-wide window.

## 8. Files and order

### 8.1 Files

New, COVER's: `server/migrations/2300_cover_search.sql`; `server/src/catalog/covers/{mod,matching,search,job,admin}.rs`; `server/src/importer/covers.rs`; `server/tests/{cover_search,cover_search_client,cover_admin,cover_import}.rs`; `frontend/src/components/CoverPrompt.vue`, `CoverPrompt.test.ts`; `frontend/src/components/admin/AdminCovers.vue`, `AdminCovers.test.ts`; this file.

Changed, with the hunk COVER owns:

| File | Change |
|---|---|
| `server/src/catalog/shared/lookup.rs` (frozen) | `CoverQuery`, `CoverDoc`, `CoverEdition`, `cover_candidates` (additive) |
| `server/src/catalog/shared/ingest.rs` (frozen) | `clear_media` also upserts `cover_search.takedown_at` for the works it cleared (body only; signature unchanged) |
| `server/src/catalog/openlibrary.rs`, `service.rs`, `mod.rs` | `ol_cover_search` and its mapping; the trait impl; `pub mod covers`, `COVER_EXISTS`, the admin routes |
| `server/src/media/cover.rs`, `media/mod.rs` | the direct admin upload (the reader upload's steps shared, not copied); its route in `media::admin_routes` |
| `server/src/admin/summary.rs` | `AdminSummary.covers` |
| `server/src/jobs.rs` | `lock_ids::COVER_SEARCH`, `cover_search_loop` |
| `server/src/importer/{mod,report,rematch}.rs` | the search step, `Report.covers`/`without_cover`, the rematch pass |
| `server/tests/common/catalog.rs` | `FakeCatalog` cover programming (add-only) |
| `server/tests/{admin_guard,parity_schema}.rs`, `docs/phase1/parity-checklist.md` | the add-only rows the shared checks require |
| `frontend/src/api/client.ts`, `i18n/apiErrors.ts`, `i18n/{en,de}.json`, `admin.ts`, `admin.test.ts` | client methods, the mapping, `cover.prompt.*`, `admin.covers.*`, labels |
| `frontend/src/components/CoverPhotoButton.vue`, `pages/BookDetailPage.vue`, `pages/AdminPage.vue` (+ their tests) | the `label` prop, the prompt and admin upload, the tab |
| `server/.sqlx/`, `frontend/src/api/generated.ts` | regenerated (amendment 1) |
| `docs/phase2/cutover.md` | the review step (below) |
| `docs/phase2/work-plan.md` | the migration block and ports (done with this design) |

### 8.2 Order

Server first, each step with its tests and the full server checks:

1. Migration 2300 on a fresh database; `cargo sqlx prepare -- --all-targets`.
2. `matching.rs` test-first (pure).
3. The seam: `lookup.rs`, `openlibrary.rs`, `service.rs`, `FakeCatalog`; the client tests.
4. `search.rs` (facts, apply, the inherit step), `clear_media`, `job.rs` and `jobs.rs`; `cover_search.rs`.
5. The admin routes, the summary and the direct upload; `cover_admin.rs`, the shared guard and schema rows.
6. The importer step, the report and the rematch pass; `cover_import.rs`.
7. `cargo run -- openapi`, then the SPA: `npm run gen:api`, client, i18n, `CoverPrompt`, `BookDetailPage`, `AdminCovers`, `AdminPage`, their tests.
8. The browser check on 8087.
9. `docs/phase2/cutover.md`: in 3.3, the go/no-go also reads the `cover search` line and the `without a cover` section against the dry run (a different count is fine when Open Library changed in between); the downtime window of section 1 includes the search (one request per coverless book, both runs); in 3.5, before 3.6, **review the covers**: open Admin → Covers, decide every suggestion, and for the books still without a cover upload a picture where one is at hand or leave them to the readers' prompt; a missing cover is not a no-go. If Open Library was unavailable during the import, `rematch-legacy` (already in 3.5) searches again. Titles stay out of #38: counts only.
10. These notes: what was built, deviations, and the integrator's list completed.

### 8.3 For the integrator

- `conventions.md`: §7.2 (`clear_media` records `cover_search.takedown_at`; the lock order media → works → `cover_suggestions` → `cover_search`), §7.3 (a rule "Covers from the cover search": work-level only, provenance `openlibrary` for a clear match and `admin` for an approved suggestion, never over `user`/`admin`, suggestions only after a takedown), §7.4 (`cover_candidates`, `CoverQuery`, `CoverDoc`), §13 (`bc_cover_missing`), §16 (the job, `COVER_SEARCH = 0x6263_0004`, its constants; `import-legacy` and `rematch-legacy` search covers), §17.2 (`FakeCatalog` cover programming).
- `endpoint-map.md`: §4 the five catalog admin routes and the summary field, §6 `PUT /api/admin/works/{work_id}/cover`, §10 the report line and section, §11 `This book has a cover now`.
- `docs/phase1/legacy-mapping.md`: §0 "Report" (the line and section), §3.3 step 4 (GB rows are still never matched as works; the cover search only looks for a picture and never changes a work's facts or identity).
- `CLAUDE.md`: COVER's ports and block in "Server worktrees", the new test files that read docs (none), the job in the architecture notes if useful.
- `README.md`: covers for catalog and imported books, the photo prompt.
- Legal documents: `privacy-facts.md` §5's `openlibrary.org` row and `processors.md` "What is sent" gain "a catalog book's title and author (the cover search)"; `vvt.md` line 86 likewise. Still no personal reference: no user identity is sent, and which books are searched first ("in use") is not visible to Open Library beyond the order of requests. `takedown.md`: a takedown of a book's cover also turns that book's cover search to suggestions only.

## Decisions taken here (with the reason in the section named)

1. Dead Open Library covers count as missing (§1): for the reader they are no cover.
2. Work-level covers only, never an edition's (§2.4).
3. Clear needs title, numbers, author and one book; dominance at 3× editions (§2.3, §2.5).
4. Suggestions in their own table, rejection per Open Library work, kept forever (§3).
5. No foreign keys to `users` or `media` (§3).
6. The search runs inside the import's transaction and never aborts it (§4.2).
7. Tiers; tier 2 never suggests; unpublished DNB works and unused Open Library works are not searched (§1, §4.4).
8. No environment variable and no `find-covers` command (§4.4, §4.5).
9. The prompt only for catalog and imported books on the viewer's shelf (§5.4).
10. No admin mail for suggestions: nothing is urgent or legal about them, and the tab badge shows them.

## Open questions

None for Rafael: #74 settles the product choices (automatic clear matches, a review list for the rest, Open Library and uploads as the only sources, readers' books left alone), and the rest is decided above. Defaults he may want to overrule later: the background search of DNB books nobody reads (tier 2), and that a takedown turns a book's search to suggestions for good.
