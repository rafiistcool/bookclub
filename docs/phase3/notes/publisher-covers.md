# Publisher covers from the DNB portal

Added on 2026-11-05, after the original titles (`original-title.md`). 17 of the club's imported books still had no cover. Most are German originals, which have no MARC 240, so Open Library cannot find them by an original title.

## The measurement behind it

- **The research.** It found one source that holds covers of German publisher titles: the VLB of the Börsenverein (MVB).
  - The official route is the VLB REST API, which needs credentials and a licence for the covers.
  - The DNB portal shows the same covers without a key at `https://portal.dnb.de/opac/mvb/cover?isbn=<ISBN-13>`. It answers 200 `image/jpeg` when it has a cover, and 404 with an HTML page when it has none. The DNB's Calibre plugin uses this endpoint.
  - The terms for third parties are not stated anywhere.
- **The count on the host (2026-11-05).** The admin asked the portal for the ISBNs of the 17 imported books without a cover, one request a second:
  - 7 answered 200;
  - 10 answered 404;
  - none failed to connect.

## The decision

Rafael chose on 2026-11-05 to build it for the private instance and to store the pictures. Every cover is stored this way: an Open Library cover is fetched once and served from our own media store too, because the browser loads nothing from another origin.

**Legal (not legal advice):**
- Showing the covers to 2–5 invited people is probably not making them available to the public under §19a UrhG (BGH I ZR 119/20).
- Storing them is a reproduction (§16) of its own.
- A public service counts all its users together, so the argument ends at the public launch.

## What changed

- **Settings** (`config.rs`):
  - `DNB_PORTAL_COVERS` (default `false`).
  - `DNB_PORTAL_URL` (default `https://portal.dnb.de/opac/mvb/cover`).
  - `SIGNUP_MODE=open` refuses to start with `DNB_PORTAL_COVERS=true`. It says so before it names missing legal texts.
- **Outbound:** the portal has its own throttle, 1 request a second. Requests use the media policy: 1.5 MB at most, one retry on 502, 503 or 504.
- **Lookup:** `CatalogLookup::publisher_cover(isbn)` returns the bytes or `None`; the default is `None`. `CatalogService` asks only for German-group ISBNs, and only with the setting on. What the answer means:
  - 200 with an `image/*` type gives the bytes.
  - 404, 410, a redirect it refuses, a body that is too large, or a 200 that is not an image means none.
  - 429 or the throttle means throttled.
  - Anything else means unavailable.
- **Schema** (migration 3901):
  - `media.source` takes `dnb_portal`.
  - A one-off `UPDATE` makes every `no_match` book with a 978-3 ISBN due again, as 3900 did.
- **Media:**
  - `MediaService::store_found_cover` runs the bytes through the pipeline, which makes the WebP variants `l` and `s` and samples the spine colour. It refuses a taken-down picture: it checks `media_removed_hashes`, and the original bytes of a removed row. It stores the variants under a new public id; the caller registers the row.
  - `discard_cover` deletes the variants again when that transaction does not commit.
- **Cover search** (`search_one`), when `Context::media` is set: the job and an admin's search set it. The import and `rematch-legacy` pass `None` and skip the portal.
  1. **When it asks.** After the inherit step and Open Library's ISBN check for a DNB work. Before the DNB original-title check and the title search.
  2. **Which editions.** Up to 3 of the work's editions with distinct German-group ISBNs, in edition order.
  3. **One portal answer with a picture.** The picture is stored. Then one transaction runs in this order:
     - The media row is registered, and the work is locked and checked again.
     - A takedown on the book, or a cover the provenance rules keep, ends the step without a cover, and the bytes are deleted.
     - Otherwise the picture becomes the edition's and the work's cover with provenance `dnb`. The `cover_search` row records outcome `isbn`, and no request to Open Library follows.
  4. **No picture, no answer, or throttled.** The search goes on by title as before. The portal is a source on top, so it never records a failure.
- **Reach.** The book's next search asks the portal again, so a book whose title search found nothing asks once per backoff step (30 days, then longer).

## Tests

- **`server/tests/publisher_cover.rs`:**
  - The publisher's cover becomes the cover with stored variants, and no title search follows.
  - With no cover or no answer at the portal, the title search follows and nothing is stored.
  - Only German-group ISBNs are asked, and only by the job and an admin's search.
  - A taken-down picture is not stored again.
- **`catalog::service::tests`** (wiremock):
  - The portal's 200 image gives the bytes.
  - 404 or a 200 HTML page gives none.
  - 500 means unavailable, and an English ISBN is never asked.
  - Nothing is asked with the setting off.
- **`config::tests::open_signup_refuses_portal_covers`.**

## Turning it on (the private instance)

The admin adds `DNB_PORTAL_COVERS=true` to the host's `.env` in the update block of the release that contains migration 3901. Migration 3901 makes the 17 books due again, so the next runs of the job (every 10 minutes, 30 books each) fetch the 7 covers.

## The result on the host (2026-11-06)

The admin updated the host to v0.11.3 with `DNB_PORTAL_COVERS=true`, after a VM backup. The count about 20 minutes later:

- **7 portal covers:** 7 `dnb_portal` media rows, and 7 searches since the update with the outcome `isbn`. They are the 7 ISBNs the portal had answered 200 for.
- **10 imported books without a cover** (17 before). 4 of them were searched again since the update and found nothing. The portal had answered 404 for all 10. Those books keep the cloth cover until a reader adds a photo or an admin sets one.
- **No open cover suggestions.**

## Before the public launch

`docs/phase3/work-plan.md` §5.9 lists it.
1. Turn the setting off.
2. Remove every `dnb_portal` cover: clear the covers that show one, delete the rows and their bytes, and make those books due for the search again.
3. Use the licensed VLB API or none.

The startup check alone does not remove the stored covers.
