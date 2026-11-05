# Original titles: covers and search for translated books

Added on 2026-11-05, after the private move. After the move, 17 of the club's imported books had no cover. Their covers had come from Google Books, which the importer drops on purpose (legacy-mapping.md §3.2). Open Library knows many German translations only through the original's English editions. A search by the German title finds nothing for those books.

## The measurement behind it

On 2026-11-05 I drew 867 records of fiction translated from English (2019–2024) from the DNB's SRU. 743 of them (86 %) carry the original title in MARC 240. I asked Open Library about a random 120 of those three ways:

| Request | Books with a cover |
|---|---|
| `/isbn/{isbn}.json` | 12 |
| `search.json` by the German title and the author | 18 |
| Either of the two (what the search did before) | 22 (18 %) |
| `search.json` by the original title and the author | 83 |
| Any of the three | 88 (73 %) |

The count is loose: it counts any answer with a cover, while the cover search's verdict is stricter (an unclear match becomes a suggestion). A record of a German original has no 240, so this does nothing for German originals.

## What changed

- **Mapping** (`dnb_marc.rs`): it reads 240 `$a` (CC0) as `EditionRecord.original_title` on every edition of the record.
- **Schema** (migration 3900):
  - `editions.original_title`, and `editions.original_title_checked_at` (when a DNB record last named the original title, or said there is none).
  - A one-off `UPDATE`: every book with outcome `no_match` and a 978-3 ISBN becomes due for the search now, instead of after the usual wait of 30 days or more.
- **Ingest:**
  - Only a DNB record writes `original_title`. DNB records are complete, so a record without 240 clears it, and every DNB write sets `original_title_checked_at`. Other sources leave both alone.
  - `works.search_extra` takes the original titles too, so a search for "The name of the wind" finds "Der Name des Windes".
  - `ingest::record_original_title` stores the answer of the cover search's DNB check. It writes the original title without ingesting the rest of the record, so a background job never retitles a reader's book.
- **Lookup:** `CatalogLookup::original_title(isbn)` (default `None`). `CatalogService` reads the DNB record's 240 through the existing SRU client (`dnb_record`), and only for German-group ISBNs. The SRU throttle and the 18-hour cache apply.
- **Cover search** (`search_one`):
  1. **The DNB check.** It comes after the ISBN check and before the title search. It runs when no edition of the work has an original title and an edition with a 978-3 ISBN is still unchecked. It runs once per edition, because "no record" and "no 240" are answers too. Without an answer it stays open, and the attempt goes on without it.
  2. **The second request.** When the request by the German title gives no match, the search asks by each original title whose main title differs. The original titles count as our titles in the verdict, so Open Library's English work matches.
  3. **The rest is unchanged.** `choose_cover` already took the cover of the work, or of an edition in another language, when there was no German edition.

## Tests

- `server/tests/original_title.rs`:
  - DNB ingest stores and clears the original title, and other sources never touch it.
  - The search finds a book by its original title.
  - An imported book gets the original's cover with the order of requests as described.
  - The DNB check runs once and waits when there is no answer.
  - A DNB edition needs no check.
- `dnb_marc.rs` and `matching.rs` have unit tests for 240 and for the extra requests.

## Limits

- **German originals get nothing.** Their records have no 240. They are left with readers' photos and, later, generated text covers.
- **The series form misses.** Some 240s put a series in front ("Mydworth mysteries - city heat"). The main title is cut at the first separator, so that request asks for the series. The measurement had a few of these among the 32 misses.
- **The second request needs no match first.** A German title that matches a work without a cover counts as no match too, so the second request still runs. An uncertain match by the German title ends the search as before: the suggestion goes to Admin → Covers.
