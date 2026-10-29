# COVER (#74): covers for every catalog and imported book

Owner: COVER. Branch `phase2/cover`, worktree `../bookclub-cover`, Postgres `bookclub-pg-cover` on 127.0.0.1:5458, browser port 8087, migrations in the block 2300–2399. These notes hold the design first; the implementation adds what it built, its deviations and what the integrator moves into `conventions.md`, `endpoint-map.md`, `CLAUDE.md`, the READMEs and the legal documents (§8.3).

Scope: #74. Rafael's requirement (2026-10-29): every book from a catalog or the legacy import ends up with a real cover; a book a reader added (`works.source = 'user'`) may stay without one. Google covers stay out (`docs/plan.md`, "Catalog"). The sources are Open Library (fetched by cover id, served from our domain) and pictures people upload.

## Status

Design revised after the review of 2026-10-29 ("Design review" lists every finding and what became of it). **Part 1, the server, is built** ("Part 1: the server" at the end: what was built, the decisions and deviations, and what the integrator moves). **Part 2, the SPA, is built and checked in a browser** ("Part 2: the SPA": §5.4, §5.5, the `apiErrors.ts` mapping, the i18n keys, §8.2 steps 8–9). `cutover.md` (§8.2 step 10) follows.

## Summary

- **Who is searched:** every work that is not a reader's own book, has no cover, is not merged away and has no reader's photo waiting for it (§1). A DNB work whose one ISBN cover check never ran gets that exact check first; a work whose edition has a cover takes that one. Only then does the title search run.
- **Dead pictures:** an Open Library picture whose fetch failed is fetched again after a day; a 404 or 410 then marks it **gone** (unless Open Library's cover server is failing for pictures we know exist). A gone picture is cleared from every work and edition and never attached again, and the works it left empty are searched (§1.1).
- **How:** one Open Library `search.json` call per work, `q=title:(…) author:(…)`, in the book's language, 10 works with their best edition. A **clear** match (same full title with subtitles, same numbers, same first author, one book) attaches the cover to the work with provenance `openlibrary`. An **uncertain** one becomes a cover suggestion for the admin. Nothing else attaches (§2).
- **Never:** the search replaces a `user` or `admin` cover; a reader's own book is searched; a taken-down picture comes back, not even as the same bytes under another cover id; a rejected or approved candidate is suggested again for that book. After a takedown of a book's cover, its search only suggests (§1, §2.5).
- **State:** migration `2300_cover_search.sql` adds `cover_search` (per work: when, outcome, backoff, takedown), `cover_suggestions` (the admin's review list, which also remembers rejections for good), `media.ol_gone_at` and `bc_cover_missing` (§3).
- **Where:** inside `import-legacy` (in its transaction, each book in a savepoint, stopping at the first outage), in `rematch-legacy` (paced like the job), in a background job every 10 minutes (dead check, then 30 books a run, one Open Library request every 2 s, books readers use first) and on an admin's "Search now" (§4).
- **Gaps visible:** the book page asks a reader who has the book on their shelf for a photo when it has no cover; a new admin tab "Covers" lists the suggestions and the books without a cover; an instance admin sets any work's or edition's cover directly (§5).
- **Cutover:** the import report gets a `cover search` line and a `without a cover` section (titles only in the private report file); `cutover.md` adds the search to the downtime window, allows the new lines in the dry/real diff and gets a time-boxed review step (§6, §8.2 step 9).

## 1. Candidates

A work is a **candidate** when all of these hold:

1. `merged_into_id IS NULL`.
2. `source <> 'user'`. A reader's own book is never searched, not even on an admin's request (#74 AC 3; §5.1): its title may be private, and the admin can upload a picture instead.
3. **It has no cover:** `bc_cover_missing(cover_media_id)`, true for NULL and for a removed or gone media row (§3). A picture whose last fetch failed (*failing*, §1.1) still counts as a cover until the dead check confirms it gone; that check then clears it, so the field is NULL. A cover whose bytes were never fetched counts as a cover. The SQL function is the only definition; the job, the admin list, the summary and the report all use it.
4. **No admin left it empty on purpose:** not (`cover_media_id IS NULL` and `field_sources->>'cover_media_id' = 'admin'`). No code path writes that today, but `ingest::decide` already treats an admin-cleared field as closed, and the search follows it.
5. **No reader's photo waits for the work:** no open `cover_replace` request with this `work_id` and `edition_id IS NULL`. The admin decides that photo first. A request on one of its editions does not keep the work out: it only says that edition had a cover.

A candidate goes through these steps, in this order (§4.1 "facts"):

- **Inherit:** when an edition has a usable cover (not removed, gone or failing), the work takes the cover of its lowest-id such edition through `ingest::set_cover(Work, media, by = that edition's cover provenance, only_if_empty = true)`, without asking Open Library. Outcome `inherited`. This also runs for a work under takedown (below): the edition's picture is a different picture of the same book. The work field is always NULL here (gone pictures are cleared, §1.1), so `only_if_empty` does not refuse it.
- **ISBN check:** a DNB work whose one Open Library cover check by ISBN never ran (`details_fetched_at IS NULL`: harvested works until their first detail view) gets that check first, through `CatalogLookup::cover_by_isbn` (§4.6) and `catalog::service::attach_checked_cover`, as `catalog::detail::dnb_cover_check` does on the first view: the exact cover goes on that edition and fills the work's empty field (`fill_work_from_edition`), outcome `isbn`. Open Library has no cover for the ISBN: the check is marked done and the title search follows in the same attempt. No answer: as an unavailable search. The exact picture so always comes before another edition's, and the first detail view finds the check done.
- **Title search** (§2).

Rules for the special cases the task names:

- **Removed (taken-down) pictures:** `ingest::ol_cover_media` already returns `None` for a removed row, so a taken-down Open Library cover id is never attached or suggested again; the search treats such a candidate cover as absent. A gone cover id is skipped the same way (§1.1). The same bytes under another cover id are refused when they are fetched (§1.2).
- **A takedown of the book's cover** (any picture: a reader's photo, an imported cover, one the search attached): `ingest::clear_media` also records `cover_search.takedown_at` for every work it cleared. From then on the search never attaches automatically for that work (or a work merged into it); a clear match becomes a suggestion with reason `after_takedown`. The takedown answered a report ("wrong book", "offensive", "copyright"); whether another picture of the same Open Library record is right is for a person to say.
- **A reader's photo:** a work with a reader's photo as its cover is no candidate (it has a cover). A reader's photo on an edition fills the empty work field (inherit, above).
- **Readers' own books:** never candidates; the import report counts them separately (§6), and the book page keeps only the small "Add a photo of your copy" link for them (§5.4).

**Tiers.** The job searches **tier 1** first: candidates **in use**, meaning a row in `shelf_entries`, `user_favorites`, `quotes`, `club_picks`, `next_up_nominations`, `posts` or `legacy_work_keys` names the work. **Tier 2** is the other `dnb` and `legacy` candidates, except DNB works not published yet (every edition has a `planned_publication_date` in the future: Open Library does not have them, and the search would only cost requests). Open Library works nobody uses are not searched: Open Library's own record had no cover, and only a duplicate record could help. A tier-2 search never creates a suggestion: an uncertain result is recorded as `uncertain`. After **two misses** (`no_match`, `uncertain` or `blocked`) a tier-2 work is not searched again until it is in use (§4.4), so the yearly DNB harvest does not turn into a growing stream of requests that almost never match, and the admin's list holds only books people read.

### 1.1 Dead pictures

- **Failing:** an Open Library row with `storage_key IS NULL AND fetch_failed_at IS NOT NULL AND removed_at IS NULL AND ol_gone_at IS NULL`. Its last fetch ended without a picture (`media::service`: 404, 410, a refused redirect, a body over 1.5 MB or bytes that do not decode; asked again after 7 days). The reader sees the cloth. It is **not** missing for the search: a refused redirect depends on `REDIRECT_DOMAINS`, an error page sent with 200 does not decode, and one bad moment must not replace a book's own cover.
- **The dead check** (the job, every run, before the searches; §4.4): up to `COVER_RECHECK_BATCH` = 20 failing rows that a work or an edition names and whose failure is at least 24 h old, in-use books first, then the oldest failure. Each is fetched again at once, ignoring the 7-day window, through `MediaService::recheck_cover` (new, in MED's `media/service.rs`, sharing the fetch and store steps of `ensure_cover`):
  - a picture: stored as on any fetch; the row is a cover again;
  - 404 or 410: **gone**, if the run's probe passed (below);
  - a refused redirect, too large or undecodable: stays failing (`fetch_failed_at` = now). These can come from our configuration or a format change, so they are never cleared automatically; the admin's list names them ("Open Library's picture cannot be used", §5.5), and the admin uploads one or presses "Search now";
  - throttled or failed (5xx, transport): nothing changes, and the run's dead check stops.
- **The probe:** before the first row of a run is marked gone, the job fetches the Open Library picture stored most recently (one we have bytes for) the same way, without storing it. Anything but a picture means Open Library's cover server is broken or has moved, and nothing is marked gone in that run. A changed URL scheme cannot clear the covers of the whole catalog.
- **Gone:** in one transaction (media `FOR UPDATE`, then works, then editions, the takedown's order): `media.ol_gone_at = now()` and the new `ingest::clear_gone_media(conn, media_id)`, which clears every works and editions reference to the row and its provenance, like `clear_media` but without the takedown flag. Editions then fall back to the work's cover in `BookOut`; works left empty are candidates (inherit, then search). `ingest::ol_cover_medias` skips gone rows like removed ones (a body-only change of a frozen function), so no ingest, refresh, import, ISBN check or search attaches the picture again: a `Mode::Refresh` of the Open Library record keeps the search's cover, because `value_only` sees no incoming cover.
- **The prefetch:** each run also queues up to `COVER_PREFETCH_BATCH` = 100 never-fetched Open Library pictures of in-use works and editions for `MediaService::prefetch` (its queue holds 500 and drops the rest), so the imported cover ids are tried within the first runs after the start, and dead ones show up as failing.
- **"Search now"** (§5.1) re-checks a failing cover of that work at once, without the 24-hour wait, with the probe.

Readers on a shelf whose edition carries a failing picture see the cloth until the check confirms it gone (one to two days); then the edition falls back to the work's cover, which the search fills.

### 1.2 Taken-down bytes under another cover id

Open Library often keeps one picture under several cover ids (one per edition), and the search looks at exactly those other editions. The takedown blocks the hashes of the bytes it removed (`media_removed_hashes`), but today only uploads check them. **`MediaService` checks every Open Library fetch too** (`try_fetch_and_store`, MED's file; the recheck shares it): after the fetch and before anything is stored, bytes whose SHA-256 is a removed hash (the query of the upload check, `media::avatar::refuse_removed`) are settled as taken down. In one transaction the row gets `removed_at` (no bytes are stored) and `ingest::clear_media` clears its references, which records `takedown_at` for those works: the same picture was taken down for another book, so a person decides from now on. `/media/{public_id}` answers 410 as for any removed picture. That makes `takedown.md`'s sentence "The hash block also stops the server from fetching it again" true for other cover ids as well. Left open: an Open Library cover taken down before its first fetch has no hash (`record_removed_hashes`), so only its cover id is blocked.

## 2. The matching rule

Pure functions in `server/src/catalog/covers/matching.rs`, unit-tested with tables. Conservative throughout: a wrong cover is worse than none.

### 2.1 The query

One request per work and attempt:

```
GET {OPENLIBRARY_URL}/search.json?q=title:(<main title>) author:(<surname>)&limit=10
    &fields=key,title,subtitle,author_name,cover_i,edition_count,first_publish_year,language,
            editions,editions.key,editions.title,editions.subtitle,editions.cover_i,editions.language
    [&lang=<ISO 639-1>]
```

- **The fielded `q`**, not the `title=` and `author=` parameters: those match work titles only, so a German title would never find the English work through its German edition. Checked against Open Library on 2026-10-29: `title=Der Distelfink&author=Tartt&lang=de` answers 0 docs; `q=title:(Der Distelfink) author:(Tartt)&lang=de` answers OL16809803W "The Goldfinch" with the edition "Der Distelfink" / "Roman" and cover 15099932. That payload becomes a fixture of the client test. Edition titles in the results make series collisions likelier, which the full-title rule (§2.2, §2.3) handles.
- **Escaping:** in both values the Lucene special characters `+ - & | ! ( ) { } [ ] ^ " ~ * ? : \ /` become spaces and whitespace collapses. They never count for matching (§2.2 folds them away), and a stray quote or slash must not turn into an error answer. A title empty after that is not searched (`no_match`).
- `<main title>` is the work's main title as stored (§2.2), not folded: Open Library does its own matching. `<surname>` is the first author's surname as written (§2.2). Without an author, `author:(…)` is left out.
- `lang` (Open Library: "influences but doesn't exclude"; it makes `editions` hold the edition in that language when the work has one) comes from the work's editions' `language` (the most frequent MARC code, mapped to ISO 639-1: ger→de, eng→en, fre→fr, spa→es, ita→it, dut→nl, and a few more), else from the ISBN's registration group (978-3 → de, 978-0 and 978-1 → en, 978-2 and 979-10 → fr), else nothing.
- `editions` holds **one** edition per work, the one Open Library ranks best for the query and `lang` (its documentation: "Currently only one edition is displayed").
- 10 results: enough to see Open Library's duplicate records of one book in a single request. `numFound` is kept, because "the only such work" (§2.3) is only true when nothing was cut off.
- Through `CatalogService`'s `json` cache and the `ol_api` throttle (3 a second, shared with readers' searches, at most 2 s of waiting), the 429 cooldown, one retry on 502/503/504, the 18-hour response cache (key `cover|<folded title>|<folded author>|<lang>`), empty answers not cached.

The answer is mapped leniently like every Open Library payload (spec R13): non-objects, keys that are not `/works/OL…W`, docs whose cleaned title is empty and duplicates are skipped; cover ids must be positive; a missing `edition_count` stays unknown (`None`), never 0. What counts as which answer:

| Open Library answers | `cover_candidates` returns | The work |
|---|---|---|
| 200 with a `docs` array | `Ok(CoverAnswer {num_found, docs})` | the verdict (§2.3) |
| 200 without a `docs` array, or with an `error` key (a search-index hiccup) | `Err(Unavailable)` | not "no match" |
| 429, or the throttle or a cooldown | `Err(Throttled)` | §4.1 |
| another 4xx (a query Open Library refuses, 404, 422) | `Ok` with no docs, inside the fetch closure, so `Fetch` stays as it is and the empty answer is not cached | `no_match` with the miss backoff, the status in `last_error`; the run goes on |
| 5xx after the retry, transport, timeout | `Err(Unavailable)` | §4.1 |

### 2.2 Normalisation

`fold(text)`, for titles:

1. Unicode NFKD, combining marks dropped (diacritics: é → e, ü → u), then `text::casefold` (ß → ss).
2. `&`, `und` and `et` become `and`.
3. An apostrophe after a word-initial `l`, `d` or `j` becomes a space (`L'Étranger` → `l etranger`); every other apostrophe is dropped (`Hitchhiker's` → `hitchhikers`). Every other character that is not a letter or digit becomes a space; whitespace collapses.
4. One leading article is dropped: the, a, an, der, die, das, ein, eine, le, la, les, l, un, une, el, los, las, il, lo, gli, de, het, een.

**A title variant** is a pair (title, subtitle). `split` reads it as:

- `main`: the title with the parts in `(…)` and `[…]` dropped, cut at the first `:`, ` - `, ` – `, ` — `, `;` or ` / `, folded. A main key that folds to nothing never matches.
- `rest`: the **segments** after that cut (split at the same separators) plus the subtitle, each folded, with **generic segments** dropped, joined. A segment is generic when its whole folded text is on a short list of genre and format words: roman, ein roman, thriller, krimi, kriminalroman, erzahlung, erzahlungen, novelle, a novel, novel, a thriller, a mystery, jugendbuch, kinderbuch, deutsche erstausgabe, taschenbuch, paperback, (extended in the tests as cases come up). A remainder that is the series name, a volume title or "Gesamtausgabe im Schuber" is not generic.
- `full`: `main` and `rest` together.
- `numbers`: the set of numbers in the whole title and subtitle, brackets included, after folding: digit runs (leading zeros dropped) and the whole-word Roman numerals ii to xx. `Band 2`, `#2`, `Volume II` and `(The Expanse, 3)` all give {2} or {3}. Years count too: `1984` must agree with `1984`.

Ours are the work's (title, subtitle) and each edition's (title, subtitle) that differs (a translation's own title); theirs are the doc's (title, subtitle) and its best edition's (title, subtitle). Legacy Google rows keep their title without a subtitle (`importer::books`), DNB titles carry `$n`/`$p` joined into the title (`dnb_marc`); both go through the same `split`.

`name(text)`, for an author: folded like a title, with periods and hyphens as spaces; a name with exactly one comma is inverted (`Le Guin, Ursula K.` → `ursula k le guin`). Our first author is the first `work_authors` record, else the first `", "`-separated name of `works.author_names`. **One rule for both sides:** the **surname word** is the last word of the surname part: of the part before the comma of `authors.sort_name` when we have one (`Le Guin` → `guin`, `Saint-Exupéry` → `exupery`, `Müller-Westernhagen` → `westernhagen`), else of the folded natural-order name (`Ursula K. Le Guin` → `guin`, `Antoine de Saint-Exupéry` → `exupery`). The **initial** is the first letter of the first forename word (after the comma of `sort_name`, else the first word of a name with more than one), if any. Two names **agree** when the surname words are equal and the initials are equal or one of them is missing (`J. R. R. Tolkien` = `John Ronald Reuel Tolkien` = `Tolkien`; `Stephen King` ≠ `Tabitha King`; `Le Guin, Ursula K.` = `Ursula K. Le Guin`). The query's `author:(…)` sends the surname part as written (`Le Guin`, `Saint Exupéry` after escaping).

### 2.3 Clear, uncertain, no match

The docs are filtered first: an Open Library work rejected or approved for this book (or for a book merged into it) is dropped, and a cover id whose media row is removed or gone counts as absent. Then, for each doc in Open Library's order, over the pairs of our and their title variants:

- **identity:** the doc's key is the work's own `ol_work_id` (Open Library added a cover since our stub). Clear, whatever the titles say.
- **title equal:** some pair has equal, non-empty `main` keys, equal `rest` keys (both empty counts as equal) and equal `numbers`.
- **rest differs:** some pair has equal `main` keys and equal `numbers`, but different `rest` keys, also when one side has none (`Percy Jackson – Im Bann des Zyklopen` / `Percy Jackson`; `Die Chroniken von Narnia – Der König von Narnia` / `Die Chroniken von Narnia: Gesamtausgabe im Schuber TB`; `Dune: House Harkonnen` / `Dune`).
- **numbers differ:** some pair has `main` keys that are equal and non-empty once their number words are deleted, but different `numbers` (`Die Tribute von Panem` / `Die Tribute von Panem 1`; `Harry Potter und der Stein der Weisen` / the same with `(Band 1)`).
- **title similar:** none of the above, but every word of the shorter `full` key is a word of the longer one (`Dune` / `Dune Messiah`; `Percy Jackson – Im Bann des Zyklopen` / `Im Bann des Zyklopen: Percy Jackson 2`).
- **author agrees:** our first author agrees with one of the doc's `author_name` entries (Open Library may list an illustrator or translator too).
- **generic title:** our `main` key is on a stop-list of collection titles that one author has many distinct books of: gedichte, erzahlungen, briefe, tagebucher, werke, gesammelte werke, ausgewahlte werke, samtliche werke, marchen, kurzgeschichten, novellen, essays, poems, selected poems, collected poems, stories, short stories, collected stories, letters, selected letters.

The verdict for the work:

| Verdict | When | What happens |
|---|---|---|
| **clear** | identity; or title equal + author agrees, our book has an author, the title is not generic, and the doc is either the **only** title-equal Open Library work (and `numFound` ≤ 10, so none was cut off) or **dominates** the other title-equal ones (its `edition_count` is at least 3 times and at least 5 more than each other's, and every compared doc has a known `edition_count` ≥ 1) | attach (§4.1), unless the work is under takedown (→ suggestion `after_takedown`) |
| **uncertain** | title equal + author agrees, but neither only nor dominating (`several_works`); title equal + author agrees on a generic title (`generic_title`); numbers differ + author agrees (`number_differs`); rest differs or title similar + author agrees (`title_differs`); title equal and our book has no author at all (`no_author`) | one suggestion: the first reason in that order, then `edition_count`, then Open Library's order (tier 2: recorded as `uncertain`, no suggestion) |
| **no match** | anything else, including the same title by another author | nothing; searched again later (§4.4) |

Dominance only ever compares title-equal docs, so it can never promote a doc whose rest or numbers differ: the most-edited volume of a series, its box set or an omnibus with the series name as its title stays a suggestion. A doc with no usable cover still counts for "only" and "dominates" (it is a duplicate record of the same book or another book of the same name), but a verdict whose chosen work has no usable cover is **no match**.

### 2.4 Which cover of the matched work

1. The best edition's cover (`editions.docs[0].cover_i`), when usable and the edition is in the book's language (§2.1) or its variant is the matched one: the German edition's cover for a German book, the translation's for a translation.
2. Else the work's cover (`cover_i`, Open Library's pick, usually the most common edition).
3. Else the best edition's cover in another language.

The suggestion and `cover_search` record the Open Library work id, the edition id when the cover is an edition's, and the cover id.

The cover goes on the **work**, never on one of our editions: it shows another edition of the same book, so it would be wrong as the cover of a specific ISBN. `BookOut` in an edition context falls back to the work's cover, so every edition without its own shows it. (The ISBN check of §1 is the exception: its picture is the edition's own.)

### 2.5 False positives considered

- **Same title, different book:** the first author must agree; a single-word title (`Momo`, `Circe`) is fine with the author. Collections one author has many of (`Gedichte`, `Briefe`, `Selected Poems`) only ever suggest. Other books of one title by one author: Open Library's duplicate records of one book are a main record and stubs with one or two editions, while two real books both have several, so a margin of 3 times and 5 more is needed; without it, a suggestion.
- **Volumes and series:** `numbers` must agree for a clear match, so volume 2 never gets volume 1's cover automatically. Series whose volumes share a prefix and differ only after the separator (`Gregs Tagebuch - Jetzt reicht's!`, `Sherlock Holmes - Der Hund der Baskervilles`, `Der Herr der Ringe - Die Gefährten`, `Harry Potter: Der Gefangene von Askaban`) need an equal rest too, so neither volume 1 nor the box set nor an anthology titled after the series attaches; they are `title_differs` suggestions at most. Checked on 2026-10-29 for Narnia: with `editions.subtitle` the box set's German edition reads "Die Chroniken von Narnia" / "Gesamtausgabe im Schuber TB", whose rest differs.
- **Subtitles:** the full title counts, minus genre words: `Der Distelfink` matches the edition `Der Distelfink` / `Roman`, and `Die Tribute von Panem – Gefährliche Liebe` matches Catching Fire's German edition (`Die Tribute von Panem` / `Gefährliche Liebe`), not Hunger Games' or Mockingjay's.
- **Translations:** a German title matches through the German edition of an English work (title variant of the best edition, with `lang=de` and the fielded query), and that edition's cover is chosen. A German title with no German edition on Open Library is no match (not a guess through the English title). The edition's `language` is often missing in the answer; rule 1 of §2.4 then holds through the matched variant.
- **New translations or editions of an old work:** they get the old edition's cover. That is what #74 asks for ("the cover of another edition"); the reader's photo or an admin's upload can still replace it (rank `user`/`admin` > `openlibrary`).
- **Books without an author:** never clear; an exact title only suggests.
- **Names with particles or hyphens:** one surname rule for both sides (§2.2), so `Le Guin`, `de Beauvoir`, `Saint-Exupéry` and German double names agree with Open Library's natural-order names.
- **After a wrong cover:** a reader reports it ("wrong book"), the admin takes it down; the picture never comes back (§1, §1.2), and the book only gets suggestions from then on.
- **A reader is faster:** the search re-checks under the work's row lock (§4.1); `set_cover(…, only_if_empty = true)` never replaces any cover.

## 3. Data model: `server/migrations/2300_cover_search.sql`

```sql
-- MED's table, additive (recorded for conventions §13): Open Library confirmed that this cover
-- id has no picture (§1.1). Ingest never attaches such a row again.
ALTER TABLE media ADD COLUMN ol_gone_at timestamptz NULL;
ALTER TABLE media ADD CONSTRAINT media_ol_gone CHECK (ol_gone_at IS NULL OR ol_cover_id IS NOT NULL);

CREATE FUNCTION bc_cover_missing(cover_media_id bigint) RETURNS boolean
    LANGUAGE sql STABLE PARALLEL SAFE
    RETURN cover_media_id IS NULL
        OR EXISTS (SELECT 1 FROM media m
                   WHERE m.id = cover_media_id
                     AND (m.removed_at IS NOT NULL OR m.ol_gone_at IS NOT NULL));

-- One row per work the search has seen; no row = never searched (due).
CREATE TABLE cover_search (
    work_id              bigint PRIMARY KEY REFERENCES works (id) ON DELETE CASCADE,
    searched_at          timestamptz NULL,      -- the start of the last attempt Open Library answered
    searched_key         text NULL,             -- works.match_key at that search
    outcome              text NULL CHECK (outcome IN
                             ('attached', 'inherited', 'isbn', 'suggested', 'uncertain',
                              'no_match', 'blocked')),
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
                                                  'generic_title', 'no_author', 'after_takedown')),
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
-- The dead check: failing Open Library rows, oldest failure first.
CREATE INDEX media_failing_idx ON media (fetch_failed_at)
    WHERE storage_key IS NULL AND fetch_failed_at IS NOT NULL AND removed_at IS NULL AND ol_gone_at IS NULL;
```

Plus a `COMMENT ON` for every table and column, as the other migrations do.

Decisions:

1. **A table of its own for suggestions, not a `catalog_edit_requests` kind.** Suggestions are made by the server, not filed by a reader: they have no `created_by`, must not count against the readers' request limit, the 72-hour response target, the overdue badge or the daily digest mail (`admin::notify`), and a rejection must outlive the 3-year retention that `auth::cleanup` applies to decided requests. A new kind would also have to loosen `catalog_edit_requests_cover` (`media_id` only for `cover_replace`) and the takedown's request rejection. Approve and reject reuse the queue's patterns (`QueueQuery`, `REQUEST_DECIDED`, the lock order).
2. **Statuses.** `open`; `approved` (an admin attached it); `rejected` (an admin said "not this book": that Open Library work never comes back for this book, or for a book merged into it, because `merge_works` points every older loser at the new winner); `closed` (the server closed it: the book got a cover, was merged away, the picture was removed or gone, or the Open Library work was rejected for a book merged into this one). `rejected` and `approved` both keep that Open Library work out of later searches for the book (an approved work had its chance; another picture of it is the admin's upload). A `closed` candidate may be suggested again later. At most one open suggestion per book keeps the list short; after a rejection the book is due at once, so the next candidate follows.
3. **Writing a suggestion** (under the work's `FOR UPDATE` lock, which every writer takes first): an open suggestion for the work exists → nothing is written, outcome `suggested`; else `UPDATE … SET status = 'open', … WHERE work_id = $1 AND ol_work_id = $2 AND status = 'closed'` reopens a closed one; else `INSERT … ON CONFLICT DO NOTHING` without a conflict target, which covers both unique keys. Nothing written (a rejected or approved row the facts step missed) → outcome `blocked` with the miss backoff, never an error.
4. **Rejection is per Open Library work, not per cover id.** Suggestions exist only for uncertain matches, and the uncertain part is "is this the same book". Blocking only the picture would bring the same wrong book back with its next cover. A right book with a bad picture costs the admin an upload instead.
5. **No foreign key to `users`.** Who approved is not stored (the cover's provenance `admin` says an admin did); so the export (§5.7 and `auth_export.rs`'s pinned count), the purge of empty sign-ups (`jobs.rs`) and `parity_visibility.rs` need no change.
6. **No foreign key to `media`.** A suggestion names its picture by `ol_cover_id`; the media row (created by `ingest::ol_cover_media` before the suggestion is written) is found through the unique `media.ol_cover_id`. Open Library media rows are never deleted (`delete_rejected_cover_photos` deletes only uploads of rejected `cover_replace` requests), so `delete_rejected_cover_photos` and `MEDIA_REFERENCES` need no change.
7. **No personal data.** Both tables are catalog state: `parity_schema.rs`'s `GLOBAL` list and `docs/phase1/parity-checklist.md`'s "Neither" row gain them. No retention rule: rows are small, and rejections must stay. `media.ol_gone_at` is catalog state too.
8. **`searched_key`** is `works.match_key` (title and first surname, kept by ingest) at the search: an admin's correction or a `rematch-legacy` upgrade changes it, and the book is due again at once, with `misses` reset.
9. **`ol_gone_at` on `media`** rather than clearing `fetch_failed_at` or setting `removed_at`: `removed_at` means a takedown (410, reports, `takedown.md`), and `fetch_failed_at` alone cannot tell a passing failure from Open Library's "no such cover".

## 4. Where it runs

### 4.1 One search of one work

`catalog::covers::search` has three steps, so each caller decides where its transaction ends:

1. **facts** (read): the work's title variants, first author, language hint, `ol_work_id`, current cover state, takedown flag (its own or a merged-away work's), the Open Library works rejected or approved for it (and for works merged into it), an open suggestion, an open work-level `cover_replace`. `None` when it is no candidate (§1). The inherit step needs no Open Library call and returns at once; the ISBN check (§1) runs here for a DNB work that needs it.
2. **remote:** `CatalogLookup::cover_candidates(&CoverQuery)` (§4.6), outside any transaction for the job, the admin and `rematch-legacy`; inside the import's transaction for `import-legacy` (it already makes its ISBN lookups there).
3. **apply** (one transaction, or a savepoint of the import's): media first, as everywhere (conventions §7.2): `ingest::ol_cover_media(conn, cover_id)` locks the picture `FOR SHARE` (`None`: removed or gone meanwhile → no match); then the work `FOR UPDATE`, **re-checked**: still a candidate with the same `match_key` (else nothing is written and the work, if still a candidate, is due again with its new key); no open suggestion now (else outcome `suggested`); the takedown flags, own and merged-away, read again (a takedown that landed during the remote step turns an attach into an `after_takedown` suggestion); the editions' usable covers read again (a reader's photo on an edition that landed meanwhile: inherit instead of attach). Then `ingest::set_cover(Work, media, Source::OpenLibrary, only_if_empty = true)` or the suggestion write (§3 decision 3); then the `cover_search` upsert. A clear match commits and then, in the server process, queues the picture for `MediaService::prefetch`, so the bytes arrive (or the failure shows) before a reader opens the book.

**Every outcome writes `cover_search`**, so no book stays at the head of the queue: when `set_cover` or the suggestion write changes nothing for a reason the re-check did not catch, the outcome is `blocked`, with the miss backoff. The upsert only applies when the stored `searched_at` is older than this attempt's start, so two searches of one book at once (the job, "Search now" and `rematch-legacy` in its own process) record one result and count one miss.

**Values from Open Library are cleaned before any insert** with ingest's cleaners (`ingest::clean_line` and `nfc` become `pub(crate)` rather than copied): NFC, control characters (U+0000 among them) dropped, whitespace collapsed; the title cut to 1000 characters, the joined authors to 2000; a year outside −5000..3000 and a language that is not three lowercase letters become NULL. A doc whose cleaned title is empty is skipped in the mapping (§2.1).

**Lock order: media → works → cover_suggestions → cover_search.** A takedown takes media `FOR UPDATE` → works (`clear_media`, which now writes `cover_search`) → reports → requests; the gone step and the removed-hash settle (§1.1, §1.2) take media `FOR UPDATE` → works → editions; approval reads the suggestion without a lock (its picture and work never change), then takes the picture `FOR SHARE` → the work `FOR UPDATE` → the suggestion `FOR UPDATE` (still `open`?); rejection takes the work `FOR UPDATE` → the suggestion → `cover_search`. No path takes them the other way round. A race test per pair (the job against an approval of the same book, a takedown against an attach, the gone step against an attach).

`cover_search` after each outcome:

| Outcome | Written | Due again |
|---|---|---|
| attached / inherited / isbn | `outcome`, `misses = 0`, `failures = 0`, and for attached `attached_*` | when the work loses its cover |
| suggested | `outcome`, `misses = 0` | after the suggestion is decided (rejected: at once) or closed |
| uncertain (tier 2) | `outcome`, `misses + 1`, `next_search_at` as for no match | as soon as the work is in use; else then, but not after 2 misses |
| no match / blocked | `outcome`, `misses + 1`, `next_search_at` = now + 30 days × 2^(misses − 1), at most 180 days | then (tier 2: not after 2 misses until in use) |
| Open Library unavailable | `failures + 1`, `last_error`, `next_search_at` = now + 1 h × 2^(failures − 1), at most 24 h | then; the job's breaker also pauses the whole search (§4.4) |
| throttled | nothing | at once (the caller stops or waits) |
| title or first author changed (`searched_key` ≠ `match_key`) | – | at once, `misses` reset |

### 4.2 `import-legacy`

- **When:** right after the books step (`import_books`), inside the import's transaction, over every imported work that is a candidate, in legacy book id order (deterministic report). User works (`BC…`) are counted, not searched.
- **Each book in a savepoint** (`tx.begin()` on the import's transaction): a database error in one book's cover step rolls back only that step and reports the book "not searched (database error)"; the import goes on. A cover is not part of the data, so nothing in the cover step can make the import exit 1.
- **The breaker:** the first unavailable answer, or the first book whose throttle waits give up (`--retry-wait-ms`, default 60 s, counted in `lookup wait`, at most 3 waits per book), stops the search for this run: the remaining books are "not searched". A run also stops searching once the step has taken `IMPORT_COVER_BUDGET` = 5 minutes. A not-searched book has no `cover_search` row, so the job takes it first after the cutover (tier 1, `legacy_work_keys`).
- **Dry run:** the transaction is rolled back like everything else, so the dry run reports what the real run will do, unless Open Library's answers change in between, it throttles, or it fails (the cover lines may then differ; `cutover.md` 3.3 says so).
- **`--offline`:** no search; the report says so.
- **Downtime:** a healthy `search.json` with the editions sub-query takes 1–3 s, and the books are searched one after the other, so the estimate is **about 3 s per book without a cover, per run, twice**. For the rehearsal's library (51 books, at least 17 without a cover) that is about a minute per run. During an outage the first request costs its 15 s timeout and one retry, then the breaker stops; the budget caps a slow but answering Open Library at 5 minutes per run. `cutover.md` section 1 adds this to the window (§8.2 step 9).
- **Covers the import brings are not fetched here** (no server process, no media service): the report counts the imported cover ids not fetched yet (§6), and the job's prefetch and dead check take them after the start (§1.1).

### 4.3 `rematch-legacy`

After the ISBN pass, every imported (`legacy_work_keys`) candidate is searched, each in its own transaction (rolled back with `--dry-run`), ignoring `next_search_at`, `misses` and the tier-2 stop (the operator asked for it now), but respecting rejections, approvals, open suggestions and takedowns. It runs as its own process with its own `ol_api` throttle next to the server's job and readers' searches on the same address, so it **paces itself like the job**: 2 s between two Open Library requests. Throttling is waited out as for its lookups; an unavailable answer stops the cover pass (the rest "not searched"). No dead check (the importer has no media service; the server's job does it). The report gets the same `cover search` line and `without a cover` section (§6). This is the operator's way to run the search again for the imported books, for example after Open Library was down during the import.

### 4.4 The background job

- `jobs::cover_search_loop`, started by `jobs::spawn_all` (not in tests) with the shutdown token, every `COVER_SEARCH_EVERY` = 10 minutes (the first tick at startup), one `jobs::run_locked` run under the new `lock_ids::COVER_SEARCH = 0x6263_0004`, so a second instance skips it. The job's closure holds a clone of the token and checks it between steps, between works and during every pace sleep (`select!` on `cancelled()`), so a stop ends the run after the work in flight, well within `stop_grace_period: 30s`, as `harvest::run_pass` does between pages.
- **A run:**
  1. closes moot suggestions (`closed`: the book has a cover, was merged away, the picture was removed or is gone, or the Open Library work is rejected for the book or a work merged into it);
  2. queues never-fetched pictures of in-use books for the prefetch (§1.1);
  3. runs the dead check (§1.1);
  4. takes up to `COVER_SEARCH_BATCH` = 30 due candidates: tier 1 before tier 2, never searched first, then `next_search_at`, then work id. **Due** = no row, or `next_search_at` is NULL or past, or `searched_key` changed, or outcome `uncertain` and now in use; and no open suggestion; and for tier 2, fewer than 2 misses.
- **Pace:** between two Open Library requests (title searches and ISBN checks alike) it waits `COVER_SEARCH_PACE` = 2 s, so the job uses at most a sixth of the shared 3 requests a second while it runs (about a minute and a half per run). The dead check's fetches go through the `ol_covers` throttle, which readers' cover views share, with the same pace.
- **Throttled** (the 2 s throttle wait was not enough, or a cooldown after a 429) ends the run with nothing recorded: readers' searches go first, and the next run resumes.
- **Unavailable** records the failure on that work (its own backoff, for a book Open Library fails on every time) and ends the run, and the **breaker** pauses the search: after the k-th unavailable run in a row, the loop skips runs until 10 minutes × 2^(k − 1), at most 6 hours, have passed; any answer resets it. It lives in the loop's memory: a restart tries once at once. So a long outage costs one request every few hours and backs off one book per try, and after the recovery the queue continues where it stopped.
- Each work commits on its own, so a crash or a restart loses at most the work in flight.
- The log has counts and work ids only (`cover search: searched`, `attached`, `isbn`, `suggested`, `no_match`, `failed`, `gone`, `stopped`), never titles.
- No setting turns it on or off: it is paced, backs off and yields to readers, and with nothing to search it sends nothing (CI's smoke test boots an empty database). No new environment variable.

### 4.5 No separate CLI command

`find-covers` is not added: `import-legacy` and `rematch-legacy` cover the imported books (the only batch an operator reviews), the job covers the rest, and the admin page searches one book on demand. One fewer command to document and test.

### 4.6 The Open Library seam: `CatalogLookup::cover_candidates` and `cover_by_isbn`

`server/src/catalog/shared/lookup.rs` (frozen, W1; an additive change the integrator records in conventions §7.4). Both new methods have **default bodies**, so the implementors that never search covers (`Scripted` in `tests/importer_books.rs`, `SlowLookup` in `tests/shelf_import.rs`) compile and behave as before:

```rust
/// What the cover search asks Open Library (#74).
pub struct CoverQuery {
    pub title: String,               // the main title as stored (§2.2), not folded, not escaped
    pub author: Option<String>,      // the first author's surname part as written
    pub language: Option<String>,    // ISO 639-1, sent as `lang`
}
/// Open Library's answer: how many works matched, and the first 10.
#[derive(Default)]
pub struct CoverAnswer { pub num_found: i64, pub docs: Vec<CoverDoc> }
/// One Open Library work of the answer, with its best edition for the query.
pub struct CoverDoc {
    pub ol_work_id: String, pub title: String, pub subtitle: String, pub authors: Vec<String>,
    pub cover_id: Option<i64>, pub edition_count: Option<i32>, pub first_publish_year: Option<i32>,
    pub languages: Vec<String>,      // MARC codes
    pub edition: Option<CoverEdition>,
}
pub struct CoverEdition { pub ol_edition_id: String, pub title: String, pub subtitle: String,
                          pub cover_id: Option<i64>, pub languages: Vec<String> }

trait CatalogLookup {
    // … the three existing methods …
    /// `search.json` by title and author (§2.1), at most 10 works; persists nothing.
    /// Err(Throttled/Unavailable) = no answer. Default: no match.
    async fn cover_candidates(&self, query: &CoverQuery) -> Result<CoverAnswer, LookupError> {
        Ok(CoverAnswer::default())
    }
    /// The one Open Library cover check by ISBN (`/isbn/{isbn}.json`): a cover id, or None
    /// when Open Library has no such edition or no cover. Persists nothing. Default: None.
    async fn cover_by_isbn(&self, isbn: &Isbn13) -> Result<Option<i64>, LookupError> {
        Ok(None)
    }
}
```

- `CatalogService` implements them with `ol_cover_search` in `catalog/openlibrary.rs` (`COVER_SEARCH_FIELDS`, `COVER_SEARCH_LIMIT = 10`, `map_cover_docs`, the escaping of §2.1) and the existing `ol_isbn_cover`, errors through `lookup_error`. The 4xx mapping of §2.1 happens inside `ol_cover_search`'s fetch closure, because `Fetch::Failed` does not keep the status.
- On the trait rather than a second one, because `AppState`'s fields are frozen and the importer, `rematch-legacy` and the job already hold `Arc<dyn CatalogLookup>`: production uses the real client, tests the programmable `FakeCatalog`, and #74's acceptance criterion asks for a FakeCatalog test.

## 5. API and SPA

### 5.1 Routes (all `/api/admin/*` behind `InstanceAdmin`, tag `admin`)

| Route | Request → Response | Notes |
|---|---|---|
| `GET /api/admin/cover-suggestions` | `QueueQuery` (`?status=open\|resolved&page=n`) → `CoverSuggestionList {items, page, has_more}` | `open`: oldest first, hiding moot ones (§4.4 step 1); `resolved`: newest decision first. 50 a page. Operation `catalog_admin_list_cover_suggestions`. |
| `POST /api/admin/cover-suggestions/{suggestion_id}/approve` | → `CoverSuggestionOut` | Attaches the picture to the current work (merges followed) through `set_cover(…, Source::Admin, false)`, only while it still has no cover (`bc_cover_missing`); records `attached_*`; prefetches. 404 `Not found`, 404 `No such book.`, 400 `This picture was removed` (removed or gone: Open Library removed it too), 409 `This book has a cover now`, 409 `This request was already decided`. |
| `POST /api/admin/cover-suggestions/{suggestion_id}/reject` | no body → `CoverSuggestionOut` | "Not this book": `rejected`, the book due at once. 404, 409 as above. |
| `GET /api/admin/missing-covers` | `?scope=in_use\|all&page=n` → `MissingCoverList {items, page, has_more}` | Candidates (§1), works with a reader's photo waiting (rule 5), and in-use works whose cover or in-use edition's cover is failing (§1.1); readers' books excluded. `in_use` (default): tier 1. `all`: every catalog and imported work. Order: in use first, then `lower(bc_unaccent(title))`, then id. 400 `Input should be 'in_use' or 'all'`. |
| `POST /api/admin/works/{work_id}/cover-search` | no body → `CoverSearchOut` | Searches this book now: ignores the schedule, misses and the tier-2 stop, respects rejections, approvals and takedowns; a failing cover is re-checked at once (§1.1); an open suggestion is returned as it is (outcome `suggested`, no search). Counts against `upstream_turn` (`catalog_user`, `catalog_gate`). 404 `No such book.` (also for a reader's own book: this route is for catalog and imported books), 409 `This book has a cover now`, 429 the catalog's busy strings, 502 `Could not search the library right now. Try again.` |
| `PUT /api/admin/works/{work_id}/cover` | multipart `file`, optional `edition_id` → `CoverSet {book}` | The admin's picture becomes the cover of the work or edition **at once**, replacing any cover (`set_cover(…, Source::Admin, false)`), through the reader upload's pipeline (`media::cover`: metadata stripped, WebP `l`/`s`, removed hashes refused). Also for a reader's own book. Same 400/413/429 as `POST /api/books/works/{work_id}/cover`, plus 404. Operation `media_admin_set_cover`; the body limit of the upload routes. |

`AdminSummary` (not a §0 DTO) gains `covers: AdminCoverSummary {suggestions: i64, missing: i64}` (open, non-moot suggestions; tier-1 works without a cover). The tab badge shows `suggestions`; no overdue mark (no response target).

The admin's existing tools already cover the rest: "Edit as admin" corrects a title or author (which makes the book due again), a merge joins a bare-facts book to its real record.

### 5.2 DTOs (`catalog::covers::admin`, none in §0)

```text
CoverSuggestionOut { id, status: "open"|"approved"|"rejected"|"closed", work_id,
                     book: BookOut|null,            // the work now (merges followed): its cover is null while it has none
                     cover_url, cover_thumb_url, cover_public_id,   // the candidate, from our /media
                     ol_work_id, ol_edition_id|null, title, authors, year|null, language|null,
                     reason: "title_differs"|"number_differs"|"several_works"|"generic_title"|"no_author"|"after_takedown",
                     created_at, decided_at|null }
CoverSuggestionList { items: CoverSuggestionOut[], page, has_more }
MissingCoverOut { book: BookOut, in_use: bool, imported: bool,
                  state: "none"|"failing"|"photo_waiting", failing_edition_id|null,
                  searched_at|null,
                  outcome: "attached"|"inherited"|"isbn"|"suggested"|"uncertain"|"no_match"|"blocked"|null,
                  next_search_at|null, failures, after_takedown: bool, suggestion_id|null }
MissingCoverList { items: MissingCoverOut[], page, has_more }
CoverSearchOut { outcome: "attached"|"inherited"|"isbn"|"suggested"|"no_match", book: BookOut,
                 suggestion: CoverSuggestionOut|null }
```

### 5.3 Detail strings

One new string, a constant in `catalog/mod.rs`, mapped in `apiErrors.ts` to `errors.coverExists` (en "This book has a cover now.", de "Dieses Buch hat inzwischen ein Cover."): **`This book has a cover now`** (409 on approve and on "Search now"). The rest are reused: `This request was already decided`, `This picture was removed`, `No such book.`, `Not found`, the catalog's busy and unavailable strings, the upload strings. `Input should be 'in_use' or 'all'` is admin-only and unmapped, like `STATUS_CHOICES`.

### 5.4 Book page: the photo prompt

**Who sees it:** a signed-in reader on a book page whose shown book (the viewer's edition, else the work) has no cover URL, who has the book on their shelf (`BookDetailOut.on_shelf` set, any status), when the book is not a reader's own (`custom` false) and not withdrawn. It replaces the small "Add a photo of your copy" link under the cover (one upload control, not two); everyone else keeps the link as today. The upload is the existing `POST /api/books/works/{work_id}/cover` with the viewer's edition, so the photo becomes that edition's cover and fills the work's empty one (`fill_work_from_edition`).

A reader's own book does not get the prompt: #74 lets it stay with the cloth and monogram, and its reader still has the link.

A gone picture is cleared from the rows (§1.1), so the prompt shows for such a book once the dead check confirmed it, and the reader's upload is then a first cover, not a `cover_replace` request. A failing picture keeps its URLs for the day or two until the check (the cloth shows after the failed request), and the link stays. `book_outs` stays as it is: clearing gone pictures in the rows reaches every loader (`book_outs`, the edition list, search hits, spines) without changing each one.

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

- **Suggestions** (`useQueue`, open/decided): our book (cover or cloth, title, authors, source) beside the candidate (its picture from `/media`, Open Library's title and subtitle, authors, year, language, a "View on Open Library" link that opens a new tab without a referrer like the book page's), the reason in words, and **Use this cover** / **Not this book**.
- **Without a cover** (`in_use`/`all` switch, pages of 50): each book with its state in words (never searched, no match on <date> and next try <date>, a suggestion waiting, a reader's photo waiting in the requests tab, Open Library's picture failed to load on <date> (with the edition when it is an edition's), Open Library's picture cannot be used, after a takedown: suggestions only), and **Search now** (shows the outcome; an attached cover or a new suggestion updates the list), **Upload a picture** (the direct cover), **Open the book**.

The reason and state labels are pure functions in `frontend/src/admin.ts` with tests. i18n keys: `admin.tabs.covers`, `admin.covers.*` (en and de; `messages.test.ts` keeps them in step).

## 6. The import report

`importer::Report` gains `covers: Option<CoverSearchLine>` (counts) and `without_cover: Vec<String>`; `ReportBuilder::lines()` renders them after `lookup wait` and the `isbn keys` section:

```
cover search           20 searched: 12 attached, 8 still without a cover (3 suggested, 4 no match, 1 not searched); 2 books readers added left as they are; 9 imported cover ids not fetched yet (checked once the server runs)
...
without a cover:
  book 12 "<title>" by <authors>: suggested, review it on the admin page
  book 14 "<title>" by <authors>: no match on Open Library
  book 15 "<title>" by <authors>: not searched (Open Library unavailable)
  book 16 "<title>" by <authors>: not searched (stopped after an outage)
```

- "still without a cover" is counted with `bc_cover_missing` over the imported works at the end of the step, not added up, so it is the database's truth. The `without a cover` lines name the legacy book id (stable between dry and real run; work ids are not) and, as the `isbn keys` section already does, titles: **only in the private report file**, which `cutover.md` keeps out of #38 and deletes after the counts are posted. "Not searched" names its cause: Open Library unavailable, stopped after an outage, the time budget, or a database error. With `--offline` the line reads `cover search           skipped (--offline): 20 without a cover; rematch-legacy searches them`.
- "imported cover ids not fetched yet" counts the imported works whose cover is an Open Library row without bytes: the import cannot check them (§4.2), so the review revisits the Covers tab after the start (§8.2 step 9).
- **Not an entity line.** Its name has a space and it has no `→`, so `compare.sh` (whose regex reads `^[a-z_]+ +[-0-9]+ → `) and `cutover_counts.rs` (which compares every entity line with `legacy-counts.sql` and `target-counts.sql`) ignore it; `deploy/cutover/` needs no change. The counts depend on Open Library, not on the legacy data, so they are no count check.
- `rematch-legacy` prints the same line and section for its search.

## 7. Tests

**Unit** (`catalog/covers/matching.rs`): `fold` (diacritics, ß, articles in six languages, `&`/und/et, apostrophes, punctuation), `split` (separators, brackets, subtitles, generic segments), `numbers` (digits, Roman ii–xx, years, `#3`), names (inverted, initials, `Le Guin`, `de Beauvoir`, `Saint-Exupéry`, `Müller-Westernhagen`), the query's escaping (each special character, an unbalanced quote, a title that escapes to nothing), and the verdict table: clear, identity, dominance (3× and 5 more, just under each, an unknown or zero `edition_count`), `several_works`, `generic_title`, `number_differs`, `title_differs` (rest and similar), `no_author`, `numFound` above 10, same title other author, no usable cover, rejected, approved, removed and gone candidates, edition-in-language vs work cover vs other-language edition. **Series fixtures that must not attach:** `Percy Jackson – Im Bann des Zyklopen`, `Gregs Tagebuch - Jetzt reicht's!`, `Dune: House Harkonnen`, `Sherlock Holmes - Der Hund der Baskervilles`, `Der Herr der Ringe - Die Gefährten`, `Die Chroniken von Narnia - Der König von Narnia` (against the box set's "Die Chroniken von Narnia" / "Gesamtausgabe im Schuber TB"), `Harry Potter: Der Gefangene von Askaban`; and `Die Tribute von Panem` / `Gefährliche Liebe`, which must pick Catching Fire, not Hunger Games.

**`FakeCatalog`** (`tests/common/catalog.rs`, add-only): `on_covers(title, author, answer)`, `on_isbn_cover(isbn, cover)`, `fail_covers(err, times)` and `cover_calls()`. Cover searches and ISBN checks are recorded apart from `calls()`, so the existing importer tests that compare `calls()` keep passing. Unprogrammed queries answer no match and no cover; `fail_all` applies to them too.

**`server/tests/cover_search.rs`** (FakeCatalog):
- a legacy, a DNB and an Open Library work without a cover each get another edition's cover on a clear match (#74 AC 1), with provenance `openlibrary` on the work and none on our editions;
- a harvested DNB work gets its exact ISBN cover on its edition, without a title search; Open Library knowing no cover for the ISBN leads to the title search in the same attempt;
- the edition in the book's language wins over the work cover;
- an uncertain match becomes one suggestion; approving attaches it with provenance `admin`; rejecting keeps that Open Library work out for good: on the next job run, an admin's "Search now", and after the book is merged into another, where the winner's open suggestion for the same Open Library work is closed (#74 AC 2);
- a `user` cover, an `admin` cover and a reader's book are never touched, also when a reader's upload lands between the remote step and apply (the work then inherits it) (#74 AC 3);
- a taken-down cover id never comes back; a takedown, also one that lands during the remote step, turns the book to suggestions only; **bytes identical to a taken-down picture under another cover id are never stored or served**, and the works that named it get the takedown flag;
- **the dead check:** a failing picture at least a day old answering 404 is gone: cleared from the work and from a reader's edition, which then shows the work's new cover; a refresh of the Open Library record does not bring it back; a probe that fails marks nothing gone; a refused redirect or an undecodable picture stays failing; a picture that loads again is stored; a never-fetched in-use picture is queued for the prefetch;
- a work takes its edition's cover without a search; an open work-level `cover_replace` keeps the book out, one on an edition does not;
- **the job:** throttled ends the run with nothing recorded and the next run resumes; unavailable backs off the work (1 h, then 2 h), ends the run and opens the breaker, which the next answer closes; a payload without `docs` is unavailable, a 400 is a no match that does not end the run; no match waits 30 then 60 days; `blocked` gets the same; a changed title is due at once; tier 1 before tier 2; tier 2 never suggests, stops after 2 misses and becomes due when used; moot suggestions close; a second run while the lock is held is skipped; a cancelled token ends a run between works (#74 AC 4);
- two searches of one book at once record one result and one miss; "Search now" on a book with an open suggestion returns it;
- race tests: a takedown during an attach, the gone step during an attach, the job against an approval of the same book.

**`server/tests/cover_search_client.rs`** (wiremock, `TestApp::with_real_lookup`): the request's `q` (pinned: `title:(…) author:(…)`, escaped), `fields` (with both subtitles), `limit` and `lang`, the User-Agent; the payload mapping (editions, junk docs, cover ids ≤ 0, a missing `edition_count`, U+0000 and over-long values cleaned) on the recorded Goldfinch answer; the 18-hour cache; a 429 gives `Throttled` and the cooldown; a 400 gives an empty answer that is not cached; a payload without `docs` gives `Unavailable` (the throttle's pacing itself is `outbound`'s test).

**`server/tests/cover_admin.rs`**: every route's success and error strings; the missing list's scopes, states, order and pages; "Search now" outcomes, a reader's book (404), an open suggestion, a failing cover re-checked, 409, 429 and 502; approve refuses a gone picture; the direct upload replaces a reader's cover, sets an edition's, refuses a removed hash, keeps the upload limits; the summary counts.

**`server/tests/cover_import.rs`** (the legacy fixture harness): `import-legacy` searches the coverless imported works, the report has the line and the section, the dry run's lines equal the real run's, `--offline` skips, an unavailable Open Library stops the search after the first book and does not abort the import, a database error in one book's step leaves that book "not searched" and the import exits 0, the time budget stops the search (the constant is a parameter in tests), readers' books are counted not searched, unfetched cover ids are counted; `rematch-legacy` searches, paces and reports (#74 AC 6).

**Shared checks and other packages' tests** (the edits recorded here, for the owners):
- `admin_guard.rs` (ADM): `ADMIN_OPERATIONS` gains the six routes (22 in all); `WATCHED_TABLES` gains `cover_search` and `cover_suggestions`; the seed gains an open suggestion (`suggestion_id`) and a body for `PUT /api/admin/works/{work_id}/cover`. `Seeded::body` holds JSON only: the test either learns to send a multipart body or records that `InstanceAdmin` refuses before the body is read, decided when it is built.
- `admin_summary.rs` (ADM): `an_empty_instance_has_nothing_waiting` compares the whole summary, so its expected JSON gains `"covers": {"suggestions": 0, "missing": 0}`.
- `parity_importer.rs` (PAR): `rematch_legacy_asks_each_legacy_isbn_once_along_the_real_route` counts every upstream request (7, then 11), and its catch-all `/search.json` mock now also answers the cover pass. The counts are kept for the ISBN requests (the `q=isbn:…` searches, `/isbn/…`, SRU), and the cover searches (`q` starting with `title:(`) are asserted on their own.
- `parity_schema.rs` (`GLOBAL`), `auth_required.rs` and `openapi_contract.rs` (unchanged lists, they walk the document), `api_error_copy.rs` (the new mapped string), `legal_docs.rs` (the citation, §8.3). `auth_export.rs`, `jobs.rs`, `importer_books.rs` and `shelf_import.rs` must pass unchanged (no new foreign key to `users` or `media`; the trait's default methods).

**SPA (Vitest):** `CoverPrompt.test.ts` (shown only with a shelf entry, no cover, not a reader's book; uploads with the edition; adopts the cover), `BookDetailPage.test.ts` (the prompt replaces the link; the admin's "Set cover as admin"), `AdminCovers.test.ts` (approve, reject, a 409, "Search now" outcomes, the states, the scope switch, upload), `AdminPage.test.ts` (the tab and its badge), `admin.test.ts` (reason and state labels). `messages.test.ts` and `thirdParty.test.ts` must pass unchanged.

**By hand:** the server with the built SPA on `http://agents.lan:8087` against a database with synthetic works, and the real Open Library: a few well-known books get covers, a German translation finds its German edition, a numbered or series title gets a suggestion, the prompt and the Covers tab in en and de, on a phone-wide window.

## 8. Files and order

### 8.1 Files

New, COVER's: `server/migrations/2300_cover_search.sql`; `server/src/catalog/covers/{mod,matching,search,dead,job,admin}.rs`; `server/src/importer/covers.rs`; `server/tests/{cover_search,cover_search_client,cover_admin,cover_import}.rs`, `server/tests/fixtures/openlibrary/cover_search_goldfinch.json`; `frontend/src/components/CoverPrompt.vue`, `CoverPrompt.test.ts`; `frontend/src/components/admin/AdminCovers.vue`, `AdminCovers.test.ts`; this file.

Changed, with the hunk COVER owns:

| File | Change |
|---|---|
| `server/src/catalog/shared/lookup.rs` (frozen) | `CoverQuery`, `CoverAnswer`, `CoverDoc`, `CoverEdition`, `cover_candidates` and `cover_by_isbn` with default bodies (additive) |
| `server/src/catalog/shared/ingest.rs` (frozen) | `clear_media` also upserts `cover_search.takedown_at` for the works it cleared (body only); `ol_cover_medias` skips gone rows (body only); new `clear_gone_media`; `clean_line` and `nfc` become `pub(crate)` |
| `server/src/media/service.rs` (MED) | `recheck_cover` (the fetch ignoring the 7-day window, telling 404/410 apart from the other no-cover outcomes) and the probe fetch; every Open Library fetch refuses removed hashes and settles such a row as taken down (§1.2) |
| `server/src/catalog/openlibrary.rs`, `service.rs`, `mod.rs` | `ol_cover_search` and its mapping; the trait impl; `pub mod covers`, `COVER_EXISTS`, the admin routes |
| `server/src/media/cover.rs`, `media/mod.rs` | the direct admin upload (the reader upload's steps shared, not copied); its route in `media::admin_routes` |
| `server/src/admin/summary.rs` | `AdminSummary.covers` |
| `server/src/jobs.rs` | `lock_ids::COVER_SEARCH`, `cover_search_loop` with the shutdown token |
| `server/src/importer/{mod,report,rematch}.rs` | the search step with savepoints, the breaker and the budget, `Report.covers`/`without_cover`, the paced rematch pass |
| `server/tests/common/catalog.rs` | `FakeCatalog` cover programming (add-only) |
| `server/tests/{admin_guard,admin_summary,parity_importer,parity_schema}.rs`, `docs/phase1/parity-checklist.md` | the edits of §7 "Shared checks" |
| `frontend/src/api/client.ts`, `i18n/apiErrors.ts`, `i18n/{en,de}.json`, `admin.ts`, `admin.test.ts` | client methods, the mapping, `cover.prompt.*`, `admin.covers.*`, labels |
| `frontend/src/components/CoverPhotoButton.vue`, `pages/BookDetailPage.vue`, `pages/AdminPage.vue` (+ their tests) | the `label` prop, the prompt and admin upload, the tab |
| `server/.sqlx/`, `frontend/src/api/generated.ts` | regenerated (amendment 1) |
| `docs/phase2/cutover.md` | the window, the diff text and the review step (§8.2 step 9) |
| `docs/phase2/work-plan.md` | the migration block, ports and the frozen and shared files COVER touches (done with this design) |

### 8.2 Order

Server first, each step with its tests and the full server checks:

1. Migration 2300 on a fresh database; `cargo sqlx prepare -- --all-targets`.
2. `matching.rs` test-first (pure), with the series fixtures.
3. The seam: `lookup.rs`, `openlibrary.rs`, `service.rs`, `FakeCatalog`; the client tests with the recorded payload.
4. `media/service.rs` (the removed-hash check, `recheck_cover`, the probe) and `ingest.rs` (`clear_media`, `clear_gone_media`, `ol_cover_medias`, the cleaners).
5. `search.rs` (facts with inherit and the ISBN check, apply), `dead.rs`, `job.rs` and `jobs.rs`; `cover_search.rs`.
6. The admin routes, the summary and the direct upload; `cover_admin.rs`, the shared guard, summary and schema rows.
7. The importer step, the report and the rematch pass; `cover_import.rs`, `parity_importer.rs`.
8. `cargo run -- openapi`, then the SPA: `npm run gen:api`, client, i18n, `CoverPrompt`, `BookDetailPage`, `AdminCovers`, `AdminPage`, their tests.
9. The browser check on 8087.
10. `docs/phase2/cutover.md`:
    - Section 1: the downtime window becomes the run time twice, plus the lookup wait, plus the cover search (about 3 s per book without a cover, twice, at most 5 minutes per run), plus one hour for the checks.
    - 3.3, go/no-go: the dry run's `cover search` line and `without a cover` section hold nothing unexpected (no database errors; "not searched" only with an outage). After the real run: the diff may show only the `lookup wait` line, an ISBN whose upstream record changed, and the `cover search` line and `without a cover` section (Open Library's answers, throttling and outages differ between two runs).
    - 3.5, before 3.6, **review the suggestions** (at most 15 minutes, members see the "moving" page meanwhile): Admin → Covers, decide every suggestion. Uploads of pictures and the books still without a cover wait until after 3.6: the readers' prompt, the job and the admin's later uploads take them, and a missing cover is not a no-go. If the search stopped for an outage, `rematch-legacy` (already in 3.5) searches again.
    - Section 5, afterwards: a day after the start, open Admin → Covers → Without a cover again: the imported cover ids the import could not check have been fetched, and those Open Library no longer has are gone and searched by then (§1.1).
    - Titles stay out of #38: counts only.
11. These notes: what was built, deviations, and the integrator's list completed.

### 8.3 For the integrator

- `conventions.md`: §7.2 (`clear_media` records `cover_search.takedown_at`; `clear_gone_media`; `ol_cover_medias` skips gone rows; the lock order media → works → `cover_suggestions` → `cover_search`), §7.3 (a rule "Covers from the cover search": work-level only, provenance `openlibrary` for a clear match and `admin` for an approved suggestion, only into an empty field, suggestions only after a takedown), §7.4 (`cover_candidates`, `cover_by_isbn`, `CoverQuery`, `CoverAnswer`, `CoverDoc`, the default bodies), §8 (MED: Open Library fetches refuse removed hashes; `recheck_cover`; failing and gone pictures), §13 (`bc_cover_missing`, `media.ol_gone_at`), §16 (the job, `COVER_SEARCH = 0x6263_0004`, its constants and breaker; `import-legacy` and `rematch-legacy` search covers), §17.2 (`FakeCatalog` cover programming).
- `endpoint-map.md`: §4 the five catalog admin routes and the summary field, §6 `PUT /api/admin/works/{work_id}/cover`, §10 the report line and section, §11 `This book has a cover now`.
- `docs/phase1/legacy-mapping.md`: §0 "Report" (the line and section), §3.3 step 4 (GB rows are still never matched as works; the cover search only looks for a picture and never changes a work's facts or identity).
- `CLAUDE.md`: COVER's ports and block in "Server worktrees", the new test files that read docs (none), the job in the architecture notes if useful.
- `README.md`: covers for catalog and imported books, the photo prompt.
- Legal documents: `privacy-facts.md` §5's `openlibrary.org` row and `processors.md` "What is sent" gain "a catalog book's title and author (the cover search)" (never a reader's own book: §1 rule 2); `vvt.md` line 86 likewise. The row's source citation `server/src/catalog/openlibrary.rs:375-525` moves with `ol_cover_search`: update the range or cite the functions by name, `ol_cover_search` among them (`legal_docs.rs` only checks that the line exists). Still no personal reference: no user identity is sent, and which books are searched first ("in use") is not visible to Open Library beyond the order of requests. `takedown.md`: a takedown of a book's cover also turns that book's cover search to suggestions only, and the hash block now also stops the same bytes under another Open Library cover id (an Open Library cover taken down before its first fetch has no hash).

## Decisions taken here (with the reason in the section named)

1. A failing Open Library picture is not missing; only one confirmed gone (404 or 410 again a day later, with a working cover server) is, and it is cleared everywhere (§1.1): one bad moment or a configuration change must not replace or clear covers.
2. Work-level covers only, never an edition's, except the exact ISBN check, which runs before the title search (§1, §2.4).
3. Clear needs the full title with subtitles, numbers, author and one book; dominance only among title-equal books at 3× and 5 more; collection titles only suggest (§2.3, §2.5).
4. Suggestions in their own table, rejection per Open Library work, kept forever; an approved Open Library work is not suggested again either (§3).
5. No foreign keys to `users` or `media`; `media.ol_gone_at` is the one column on another package's table (§3).
6. The search runs inside the import's transaction, each book in a savepoint, stops at the first outage or after 5 minutes, and never aborts the import (§4.2).
7. Tiers; tier 2 never suggests and stops after 2 misses until the book is in use; unpublished DNB works and unused Open Library works are not searched (§1, §4.4).
8. No environment variable and no `find-covers` command (§4.4, §4.5).
9. The prompt only for catalog and imported books on the viewer's shelf; readers' own books are never sent to Open Library (§5.1, §5.4).
10. No admin mail for suggestions: nothing is urgent or legal about them, and the tab badge shows them.
11. At the cutover, only the suggestions are reviewed before the go; uploads come after (§8.2 step 10).

## Open questions

For Rafael (#74 settles the rest: automatic clear matches, a review list for the rest, Open Library and uploads as the only sources, readers' books left alone):

1. **The cutover window.** The rehearsal copy is deleted, so the search's share of the window is an estimate (about 3 s per book without a cover, twice; at most 5 minutes per run), and the cutover's own dry run measures it. Recommended: no second rehearsal. A short `import-legacy --dry-run` on a fresh snapshot would replace the estimate with a measurement.
2. **The cutover review.** #74 asks to "review the rest before the go". Recommended reading: the suggestions are decided before 3.6 (time-boxed, while members see the "moving" page), and photo uploads for the books still without a cover come after the go.

Defaults he may want to overrule later: the background search of DNB books nobody reads (tier 2, now at most two tries each until someone uses the book), and that a takedown turns a book's search to suggestions for good.

## Design review (2026-10-29)

Two critics reviewed the first design (84a53fa). Each finding was checked against the code; the Open Library claims were checked against the live API on 2026-10-29. All but one part of one finding are accepted; the design above includes them.

| # | Finding | Verdict and reason |
|---|---|---|
| 1 | Cutting titles at `:`/` - ` and ignoring subtitles makes series volumes clear matches for another volume's cover (blocker) | Accepted: `main()` and "dominates" did exactly that, and `bare_work`/`unmatched_isbn` store Google titles with the series first; clear now needs equal rests with both subtitles fetched, and dominance only compares title-equal docs (§2.2, §2.3). |
| 2 | Existing checks break: `parity_importer.rs` request counts, `admin_summary.rs`'s whole-JSON compare, `Scripted` and `SlowLookup` (blocker) | Accepted: all four confirmed (`requests.len() == 7`/`11`, `assert_eq!` on the summary, two more `impl CatalogLookup`); default trait bodies, and the two test edits recorded for their owners (§4.6, §7). |
| 3 | A taken-down picture can come back under another Open Library cover id | Accepted: `try_fetch_and_store` never reads `media_removed_hashes`, only uploads do, while `takedown.md` claims the block stops fetches; Open Library fetches now refuse removed hashes (§1.2). |
| 4 | Dead edition covers shadow the work-level fix on readers' shelves, and the prompt stays hidden | Accepted: `book_outs` prefers any non-removed edition cover; solved by clearing confirmed-gone pictures from every row rather than changing `book_outs`, because the edition list, search hits and spines read covers on their own too (§1.1, §5.4). |
| 5 | "Dead" is not permanent, and refreshes put the dead id back | Accepted: `fetch_failed_at` also covers refused redirects, oversize and undecodable bytes and expires after 7 days, and `value_only` at equal rank writes the record's cover back; gone now needs a second 404/410 a day later behind a probe, and `ol_cover_medias` skips gone rows (§1.1). |
| 6 | Endless re-search when apply writes nothing | Accepted: `set_cover` refuses by rank or `only_if_empty` and the suggestion insert can write nothing, leaving no row; every outcome now writes `cover_search` (`blocked`), gone pictures are cleared with their provenance, approved works are filtered and approve refuses gone pictures (§3, §4.1, §5.1). |
| 7 | The suggestion insert cannot do "a conflict on either unique key writes nothing" | Accepted: `ON CONFLICT (work_id, ol_work_id)` has one arbiter and the partial index raises; an open-suggestion check under the work lock, a reopening UPDATE and a target-less `ON CONFLICT DO NOTHING`, and the `searched_at` guard against double misses (§3 decision 3, §4.1). |
| 8 | The import-time search sits in the downtime path without a breaker, and an SQL error aborts the import | Accepted: `RetryPolicy::REQUEST` gives 15 s plus a retry per request, and the import runs in one transaction; a breaker, a 5-minute budget, a savepoint per book and cleaned values, and a re-estimated window (§4.2). |
| 9 | Cutover documents and the window formula are not updated for the new lines and the review step | Accepted for `cutover.md` (window, 3.3 diff text, time-boxed 3.5 review, uploads after 3.6; §8.2 step 10); rejected for CUT2's acceptance in `work-plan.md`: CUT2 is done, and its text records what was checked then. |
| 10 | The apply re-check is too narrow: takedown flag and edition covers | Accepted: the takedown flag and edition covers were only read before the remote step; the re-check under the work lock now reads both and downgrades or inherits (§4.1). |
| 11 | Two surname definitions: hyphenated and particle names never agree | Accepted: `author_surname` keeps the whole part before the comma while natural names gave the last word; one rule, the last word of the surname part on both sides (§2.2). |
| 12 | Dominance turns vacuous with a missing `edition_count`, and generic titles get no guard | Accepted: a missing count mapped to 0 made 0 ≥ 3·0 true; counts must be known and ≥ 1, the margin is 3× and 5 more, and collection titles only suggest (§2.3). |
| 13 | Error answers from Open Library get the wrong backoff | Accepted: a payload without `docs` is unavailable, other 4xx are a no match that does not end the run, and a job-level breaker sits above the per-work backoff (kept for a book Open Library fails on every time) (§2.1, §4.4). |
| 14 | Candidate rule 5 is too broad for edition requests | Accepted: an edition's `cover_replace` says nothing about the work's field; only work-level requests keep a work out, and the missing list shows "photo waiting" (§1, §5.1). |
| 15 | The cover job does not observe shutdown | Accepted: `run_locked` gets no token and `stop_grace_period` is 30 s; the job's closure holds the token and checks it between works and in the pace sleep (§4.4). |
| 16 | A rejection on a merged-away book leaves the winner's open suggestion for the same Open Library work | Accepted: the moot close did not cover it; it now closes suggestions whose Open Library work is rejected for the book or a work merged into it (§4.4). |
| 17 | "Search now" on a reader's own book sends its title to Open Library outside the legal texts | Accepted: the route answers 404 for a reader's book, and the admin uploads instead (§5.1). |
| 18 | Matching on the main title alone attaches the wrong volume's cover (Narnia box set, Harry Potter, Panem) (blocker) | Accepted together with 1: confirmed live (the box set's German edition carries the subtitle "Gesamtausgabe im Schuber TB"); full titles with subtitles, `numFound` ≤ 10 for "only", and the cases as fixtures (§2.3, §7). |
| 19 | The `title=`/`author=` query never matches translated edition titles | Accepted: confirmed live (0 docs for `title=Der Distelfink&author=Tartt`, OL16809803W with edition "Der Distelfink" for the fielded `q`); fielded, escaped query and the payload as a fixture (§2.1). |
| 20 | DNB works get the fuzzy title search instead of the existing exact ISBN cover check | Accepted: harvested works keep `details_fetched_at` NULL until a view; the search runs the ISBN check first through a new default-bodied `cover_by_isbn` and `attach_checked_cover` (§1, §4.6). |
| 21 | An apply that writes nothing leaves the work searched first in every run | Accepted with 6: `only_if_empty` refused any dead cover; gone pictures are cleared, attach and inherit only fill an empty field, and every outcome writes a row (§1.1, §4.1). |
| 22 | An Open Library value or constraint error in the cover step aborts the whole cutover import | Accepted with 7 and 8: the CHECKs on `cover_suggestions` and U+0000 are real; ingest's cleaners, a savepoint per book and the conflict-proof write (§3, §4.1, §4.2). |
| 23 | The new trait method breaks two other implementors and the admin guard seed | Accepted: default bodies leave `Scripted` and `SlowLookup` alone; `Seeded::body` panics without a body, so the seed additions are listed (§4.6, §7). |
| 24 | Dead edition covers stay invisible, and imported cover ids are never checked | Accepted: the dead check covers rows that editions name, the missing list shows failing in-use editions, the job prefetches unfetched in-use pictures, and the report and the cutover say legacy cover ids are not verified at import (§1.1, §5.1, §6, §8.2). |
| 25 | Particle surnames never agree | Accepted with 11 (§2.2). |
| 26 | A definite 4xx answer is treated as an outage | Accepted with 13: `lookup_error` maps every non-throttled `Fetch` to `Unavailable`; the 4xx mapping happens inside `ol_cover_search`'s fetch closure (§2.1, §4.6). |
| 27 | Tier 2 never stops searching unread DNB books, and `rematch-legacy` has its own throttle | Accepted: tier 2 stops after 2 misses until in use, and the rematch pass paces itself at 2 s (§1, §4.3). |
| 28 | The `reader_book` path cannot be reached from the UI | Accepted with 17: the reason value, its label and its tests are dropped (§3, §5.2). |
| 29 | A legal citation goes stale when `ol_cover_search` is added | Accepted: `privacy-facts.md` cites `openlibrary.rs:375-525` and `legal_docs.rs` checks only that the line exists; listed for the integrator (§8.3). |

## Part 1: the server

Built on `phase2/cover` in the order of §8.2 steps 1–8 (the SPA itself is Part 2). Everything in §1–§4, §5.1–§5.3 and §6 is server code now; the shared checks of §7 are edited as recorded there.

### What was built

| Where | What |
|---|---|
| `server/migrations/2300_cover_search.sql` | `media.ol_gone_at` (+ CHECK), `bc_cover_missing`, `cover_search`, `cover_suggestions` with its indexes, `works_cover_missing_idx`, `media_failing_idx`, and two helper functions (deviation 1). Comments on every new table, column and function. |
| `catalog/covers/matching.rs` | `fold`, `TitleKeys` (main, rest, full, numbers), `relate`, `Person`, `escape`, `Language`/`language_hint`, `Book::query`, `verdict` with `choose_cover`; 18 table tests with the series fixtures of §7. |
| `catalog/shared/lookup.rs` | `CoverQuery`, `CoverAnswer` (+ `refused`, deviation 2), `CoverDoc`, `CoverEdition`; `cover_candidates` and `cover_by_isbn` with default bodies. |
| `catalog/openlibrary.rs`, `service.rs` | `COVER_SEARCH_FIELDS`, `COVER_SEARCH_LIMIT`, `map_cover_docs`, `ol_cover_search` (escaping, the 4xx/`docs`/`error` mapping inside the fetch closure, cache key `cover\|<title>\|<author>\|<lang>`); the trait impl through `lookup_error`. |
| `catalog/shared/ingest.rs` | `clear_media` upserts `cover_search.takedown_at` for the works it cleared; `clear_gone_media`; `ol_cover_medias` skips gone rows; `nfc` and `clean_line` are `pub(crate)`. |
| `media/service.rs` | `recheck_cover` → `RecheckOutcome`, `probe_covers`, `CoverFetch::Gone` (404/410 apart from refused redirects and oversize), the removed-hash refusal of every Open Library fetch (`settle_taken_down` → `clear_media`), `ensure_cover` never fetches a gone row. |
| `catalog/covers/search.rs` | `facts`, `search_one` (inherit, the ISBN check through `attach_checked_cover`, the title search, `apply` with the re-check under the work lock), `write_suggestion` (§3 decision 3), `record`/`record_failure` (the §4.1 table), `Pacer`, `Context`, `Outcome`. |
| `catalog/covers/dead.rs` | `due_rows`, `failing_rows_of`, `recheck` (probe once per run, before the first row is marked gone), `mark_gone`. |
| `catalog/covers/job.rs`, `jobs.rs` | `run` (moot close, prefetch, dead check, 30 due works), `due_works` (tiers, the 2-miss stop, unpublished DNB works out), `Breaker`, `run_once`, `JobSettings` with the constants of §4.4; `lock_ids::COVER_SEARCH = 0x6263_0004` and `cover_search_loop` with the breaker. |
| `catalog/covers/admin.rs`, `catalog/mod.rs` | The five catalog routes of §5.1, `COVER_EXISTS`, `SCOPE_CHOICES`, the DTOs of §5.2 (+ `MissingCoverOut.failed_at`, deviation 4), `cover_counts` for the summary. |
| `media/cover.rs`, `media/mod.rs` | `PUT /api/admin/works/{work_id}/cover` (`media_admin_set_cover`): the reader upload's steps moved into `receive`, shared by both routes; the upload body limit on the admin route. |
| `admin/summary.rs` | `AdminSummary.covers: AdminCoverSummary {suggestions, missing}`. |
| `importer/covers.rs`, `mod.rs`, `report.rs`, `rematch.rs` | The import step (savepoint per book, breaker, 3 throttle waits, budget), the rematch pass (paced, a transaction per book), `CoverSearchLine`, `Report.covers` and `without_cover`, the line and the section; hidden flags `--cover-budget-ms` and `--cover-pace-ms` for tests. |
| `server/.sqlx/`, `frontend/src/api/generated.ts` | Regenerated (fresh database; `npm run gen:api`). |

Tests: `cover_search.rs` (25), `cover_search_client.rs` (7), `cover_admin.rs` (8), `cover_import.rs` (7), the unit tests in `matching.rs`, `job.rs` and `report.rs`; the helpers they share are `tests/common_cover/mod.rs` (`#[path]`-included, like the legacy harness). Mutation checks: without the gone filter of `ol_cover_medias`, the removed-hash refusal or the `searched_at` guard, `a_picture_open_library_no_longer_has_is_gone`, `a_taken_down_picture_never_comes_back_under_another_cover_id` and `two_searches_at_once_count_one_miss` fail.

Shared checks edited as §7 records: `admin_guard.rs` (22 operations, the two tables watched, an open suggestion seeded, a JSON body for the multipart PUT because `InstanceAdmin` refuses before the body is read, and the approval among the requests that act for an admin), `admin_summary.rs` (the whole summary gains `covers`), `parity_importer.rs` (cover searches, `q` starting with `title:(`, counted apart: 7 and 11 stay; each rematch searches the same books again), `parity_schema.rs` and `docs/phase1/parity-checklist.md` (both tables under "Neither"), `importer_books.rs` and the legacy harness (the new hidden flags), `frontend/src/pages/AdminPage.test.ts` (the summary fixture gains `covers`). `auth_export.rs`, `jobs.rs`, `importer_*.rs`, `shelf_import.rs`, `api_error_copy.rs`, `openapi_contract.rs` and `legal_docs.rs` pass unchanged.

### Decisions taken while building

1. **Dominance needs another title-equal work.** With one title-equal work on the page, "only" decides, and it needs `numFound` ≤ 10; a single title-equal work among more than 10 results is `several_works`. Otherwise the `numFound` rule of §2.3 would never apply.
2. **Matching details** the design left open: the generic segments also cover `mystery`, `a memoir`, `psychothriller`, `historischer roman` and `roman taschenbuch`; the comparison without numbers also drops volume words (`band`, `bd`, `volume`, `vol`, `teil`, `tome`, `tomo`, `nr`, `no`, `folge`, `part`, `livre`, `libro`, `deel`); Roman numerals are ii–xx including `v` and `x` (`i` is a word too often); `jr`, `sr` and `ii`/`iii` after a surname are skipped; a leading article is only dropped when another word follows (`Die` stays a title); brackets are dropped from the rest as well as the main title (they still count for the numbers). The suggestion's `language` is the edition's when the picture is an edition's, else null.
3. **"Search now"** checks a reader's own book (404) before taking the account's `upstream_turn`, re-checks every failing Open Library picture of the work and its editions at once (with the probe), and answers 409 `This book has a cover now` when the work still has a cover after that (a picture that is still failing counts as a cover, §1). It ignores candidate rules 4 and 5 (an admin's empty field, a reader's photo waiting): an admin asked. `uncertain` and `blocked` answer as `no_match`.
4. **The admin's direct cover** keeps the reader upload's limits too (one upload at a time, 30 a day per account), as §5.1 says "same 400/413/429".
5. **An unconfirmed 404** (the probe failed) sets `fetch_failed_at` to now, so the row waits another day instead of being fetched again on every run. No stored Open Library picture to probe with counts as a failed probe: nothing is marked gone on an instance that never stored one.
6. **The import's unavailable book** is rolled back with its savepoint like the books after it, so none of them has a `cover_search` row and the job takes them first. `rematch-legacy` runs against the live server and keeps the failure row (the book backs off), except in a dry run.
7. **The approval's `cover_search` row** gets `outcome = attached`, the attached ids and reset counters, but not `searched_at` (no search answered).

### Deviations from the design

1. **Two more SQL functions in 2300:** `bc_cover_in_use(work_id)` (tier 1: the seven tables of §1) and `bc_cover_suggestion_moot(suggestion_id)` (§4.4 step 1), so the job, the admin lists, the summary and the dead check share one definition each, like `bc_cover_missing`. Plus the index `cover_suggestions_ol_work_idx` for the decided-work filter.
2. **`CoverAnswer.refused: Option<u16>`:** the status of a refused query (4xx other than 429), so it reaches `cover_search.last_error` as §2.1 asks; `Fetch::Failed` keeps no status, so the fetch closure stores it in an uncached marker answer.
3. **`cover_search.searched_at` is when the result was recorded** (`clock_timestamp()` at the write), not the attempt's start. The guard stays "applies only when the stored `searched_at` is older than this attempt's start", which with the write time means "no other attempt recorded a result since this one started": two searches at once record one result and count one miss. With the start stored, a search that started first and finished last would have counted a second miss.
4. **`MissingCoverOut.failed_at`:** when the failing picture last failed, for §5.5's "failed to load on <date>".
5. **`RecheckOutcome` and `CoverFetch::Gone`** in MED's `media/service.rs`: the recheck's outcomes, and 404/410 apart from the other "no cover" answers (`NoCover` now means a refused redirect or a body over 1.5 MB). `ensure_cover` maps them back to `CoverOutcome` unchanged.
6. **`facts` reads no takedown flag:** `apply` reads it under the work lock anyway (§4.1), so reading it before the remote step too would be a second query that decides nothing.
7. **Report counts:** "imported cover ids not fetched yet" counts imported works whose cover is an Open Library picture without bytes, the ones the search attached included (counted from the outcomes, so a rolled-back rematch reports what the real one does); `rematch-legacy` counts "still without a cover" from its outcomes for the same reason. `uncertain` and `blocked` count as "no match" in the line and the section; a book whose throttle waits gave up reads "not searched (Open Library busy)", the books after it "stopped after an outage".
8. **Not tested on its own:** `blocked` getting the miss backoff. It shares `record`'s miss branch with `no_match` (tested), and no test reaches it without a race the re-check under the lock already catches.

### For the integrator (in addition to §8.3)

- `conventions.md` §7.4 also names `CoverAnswer.refused`; §8 names `RecheckOutcome`, `CoverFetch::Gone`, `MediaService::recheck_cover` and `probe_covers`, and that `ensure_cover` never fetches a gone row; §13 also lists `bc_cover_in_use` and `bc_cover_suggestion_moot`; §17.2 lists `FakeCatalog::on_covers`, `on_isbn_cover`, `fail_covers`, `cover_calls` and `tests/common_cover/mod.rs` (`Hook`: a lookup that acts during the remote step).
- `endpoint-map.md` §4: operation ids `catalog_admin_list_cover_suggestions`, `catalog_admin_approve_cover_suggestion`, `catalog_admin_reject_cover_suggestion`, `catalog_admin_list_missing_covers`, `catalog_admin_search_cover`; §6 `media_admin_set_cover`; §11 `This book has a cover now` (to be mapped in `apiErrors.ts` in Part 2) and the admin-only `Input should be 'in_use' or 'all'`.
- `CLAUDE.md`: no new test file reads docs; the server's background jobs now include the cover search (every 10 minutes, `lock_ids::COVER_SEARCH`).
- `import-legacy` gained the hidden `--cover-budget-ms`, `rematch-legacy` the hidden `--cover-pace-ms` (tests only, like `--retry-wait-ms`).

## Part 2: the SPA

Built on `phase2/cover` in the order of §8.2 steps 8–9: §5.4 and §5.5 are SPA code now, against the types Part 1 generated (`frontend/src/api/generated.ts` is unchanged; `check:api` passes).

### What was built

| Where | What |
|---|---|
| `api/client.ts` | Add-only: `adminCoverSuggestionPage`, `approveCoverSuggestion`, `rejectCoverSuggestion`, `adminMissingCovers`, `adminSearchCover`, `adminSetCover` (multipart `file` and optional `edition_id`, as `uploadCover`). |
| `i18n/apiErrors.ts` | `This book has a cover now` → `errors.coverExists` (en "This book has a cover now.", de "Dieses Buch hat inzwischen ein Cover."). The other strings the routes answer were mapped already; `Input should be 'in_use' or 'all'` stays unmapped (admin-only). |
| `i18n/en.json`, `de.json` | `cover.prompt.{title,body,action}` with the §5.4 wording; `errors.coverExists`; `admin.tabs.covers`; `admin.setCover`, `admin.setEditionCover`, `admin.coverSet`; `admin.covers.*` (views, blurbs, the six reasons, statuses, decisions, the missing list's states, the outcomes of "Search now"); `admin.lede` names the covers. |
| `covers.ts` | `asksForCoverPhoto(book)`: a book not added by a reader (`custom` false), on the viewer's shelf in any status, without a cover URL. The prompt and the book page share it. |
| `admin.ts` | `COVER_REASONS`, `coverReasonLabel`, `coverStatusLabel`, `coverCandidateUrl` (the candidate's Open Library edition, else its work; only `OL…M`/`OL…W` ids), `coverSearchMessage`, `coverFound`, `missingCoverNotes(item, day, now)` (the §5.5 states as lines). |
| `components/CoverPhotoButton.vue` | New optional props: `label`; `look` (`fine`, the link under a cover as before; `inline`; `primary`/`ghost`, a small button); `admin` (calls `adminSetCover`: the picture replaces any cover at once, toast `admin.coverSet`). The hidden file input's keyboard focus now shows on the label. |
| `components/CoverPrompt.vue` | The §5.4 prompt: heading, body, and a primary `CoverPhotoButton` uploading for the viewer's edition; renders nothing unless `asksForCoverPhoto`. |
| `pages/BookDetailPage.vue` | The prompt in the side column above "Your shelf" (deviation 1); the small link under the cover is hidden while the prompt shows; "Set cover as admin" and, with the viewer's edition, "Set your edition's cover as admin" beside "Edit as admin". |
| `components/admin/AdminCovers.vue` | The Covers tab: "Suggestions" (open/decided through `useQueue`; our book beside the candidate, the candidate's picture from our `/media`, Open Library's title, authors, year and language, a "View on Open Library" link in a new tab without a referrer, the reason; "Use this cover" and "Not this book", each after a second tap with a sentence on what it does) and "Without a cover" (`in_use`/`all`, 50 a page; badges "Readers use it" and "Imported"; the state lines; "Search now", "Upload a picture", "Open the book"). The counts of `AdminSummary.covers` on the two views. |
| `components/admin/useQueue.ts` | Add-only: the view's type is a parameter with a first view (default `open`), so the missing list pages like the queues (deviation 6). |
| `pages/AdminPage.vue` | A fifth tab `covers` between Reports and Invites (the queues together; the first two tabs keep their place), badge `summary.covers.suggestions`, never marked overdue. On a phone the five tabs are rows of two, the last across both. |

Tests (Vitest): `CoverPrompt.test.ts` (4: who sees it, the edition upload, the hand-over of the new cover, German), `AdminCovers.test.ts` (10: the comparison and the `/media` picture, the second taps, a 409 that reads the list again, a gone picture, the decided view, the missing list's states, pages and scopes, "Search now" with each outcome, a 409 and a 502, the uploads for the book and for a failing edition, German), `BookDetailPage.test.ts` (+3: the prompt in place of the link and its upload, a reader's own book, the admin's two uploads), `AdminPage.test.ts` (the five tabs and the badge, never overdue; the tab opened, a decision re-reading the summary), `CoverPhotoButton.test.ts` (+1: the admin mode), `admin.test.ts` (+5: labels, the link, the outcomes, the state lines in both languages), `covers.test.ts` (+1), `apiErrors.test.ts` (+1 assertion). `messages.test.ts` and `thirdParty.test.ts` pass unchanged (the Open Library link is an `<a href>` a person clicks, not a load).

### The browser check (§8.2 step 9)

The server with the built SPA on `http://agents.lan:8087` (`DEV_MODE`, `STATIC_DIR=../frontend/dist`, the package's database freshly reset), against a **local stand-in for Open Library** on 127.0.0.1 (`OPENLIBRARY_URL` and `OPENLIBRARY_COVERS_URL` pointed at a small script in the scratchpad that answers `search.json` for fictional titles and draws a gradient PNG for any cover id), so nothing left the VM. Synthetic catalog: nine works with fictional titles and authors (legacy, DNB and one reader's book), legacy keys and shelf entries of one fictional admin account. A restart's first cover-search run searched eight of them (the reader's book left out) and logged `searched=8 attached=1 suggested=4 no_match=3`: one clear match, and one suggestion for each of `number_differs`, `generic_title`, `several_works` and `title_differs`.

Headless Chromium at 360 × 780 and at 1280 × 900, in English and German:

- Admin → Covers, both views and both languages: `document.documentElement.scrollWidth` stayed 360 at 360 px, and no element of the tab reached past the right edge. The candidate pictures came from `/media/…` (fetched by the server from the stand-in). The second tap, "Use it for everyone", attached the cover; the tab badge and the suggestions count went from 4 to 3, and the missing count from 5 to 4 (6 before, minus the photo uploaded on the book page below).
- "Search now" on a book with no match: toast "Kein Treffer bei Open Library.", and the line under the book read the new date.
- The book page of a shelved legacy book without a cover showed the prompt and no small link; a generated picture uploaded through it became the cover at once ("Your photo is the cover now"), the prompt left, "Suggest a photo of your copy" came back under the cover. The page stayed 360 px wide.

Two fixes came out of it: the admin's two upload labels on the book page are laid out like the text buttons beside them (they flowed as plain text), and the comparison of a suggestion is at most 34 rem wide (at 1280 px the candidate sat at the far edge of the card).

### Decisions taken while building

1. **Both decisions on a suggestion need a second tap**, as the takedown and the suspension do: an approval puts the picture on the book for everyone, and a rejection keeps that Open Library work out for good; undoing either takes an upload or a takedown. The first tap shows what the decision does.
2. **After an error on a decision** (400 picture gone, 404, 409 decided or the book has a cover) the suggestions are read again: the server hides the moot ones, so the list shows what is left.
3. **"Search now"** drops the book from the list on `attached`, `inherited` and `isbn`, and on 409 or 404; on `suggested` it notes the suggestion and marks the suggestions view for a re-read; on `no_match` it records the search on the row (today, no next try shown: the server's backoff is not in the answer).
4. **The direct upload in the missing list** goes to the edition whose Open Library picture fails (`failing_edition_id`, labelled "Upload for edition {id}"), else to the book: replacing the work's cover would not fix a failing edition cover, which the edition's readers see.
5. **The edition's admin upload on the book page** reads the book again afterwards (`adminEditionSaved`, as the edition edit does): the page shows the work's cover, which the edition's picture may or may not be.
6. **The Settings link to the admin page** still counts reports and requests only: suggestions have no response target, and the Covers tab shows their count.

### Deviations from the design

1. **The prompt's place:** in the side column above "Your shelf" (a full-width row under the cover and the title on a phone), not inside `.hero-cover`: that column is 120 px wide on a phone, too narrow for the heading, the sentence and the button. It still replaces the small link under the cover.
2. **No separate "Open Library's picture cannot be used" state:** `MissingCoverOut` does not tell a refused redirect, an oversize or an undecodable picture from a passing failure (all are `failing` with `failed_at`), so every one reads "Open Library's picture (of edition N) failed to load on <date>". Telling them apart needs a field on the server; nothing is lost meanwhile, since the dead check settles the 404s and the admin uploads for the rest.
3. **Wording on the book page:** "Set your edition's cover as admin" instead of "Set this edition's cover", in line with "Edit your edition as admin" (the page's edition is the viewer's).
4. **`CoverPhotoButton` gains `look` and `admin`** besides the `label` the design names: the prompt needs a button look, the admin's uploads the direct route.
5. **The `reason` and state labels** are functions in `admin.ts` as planned; the date formatter is a parameter of `missingCoverNotes`, so the tests pin the lines without a time zone.
6. **`useQueue.ts` (ADM's file) changed, add-only:** a type parameter for the view and an optional first view. Its callers are unchanged.

### For the integrator (Part 2)

- `CLAUDE.md`, "Frontend": the admin page's tabs gain `?tab=covers` ("Covers": the cover suggestions and the books without a cover); the book page asks a reader with a coverless catalog or imported book on their shelf for a photo (`components/CoverPrompt.vue`, rule in `covers.ts`).
- `README.md`: the photo prompt and the admin's Covers tab (with §8.3's line on covers for catalog and imported books).
- `endpoint-map.md` §11: `This book has a cover now` is mapped (`errors.coverExists`).
- Nothing for `conventions.md` beyond §8.3 and Part 1.

