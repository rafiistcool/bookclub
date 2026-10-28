# DNB OAI-PMH fixtures (HAR, #42)

`ListRecords` answers of `https://services.dnb.de/oai/repository` with `metadataPrefix=MARC21-xml`, recorded on 2026-10-27 between 23:26 and 23:27 UTC. Eight requests were sent, at least 3 s apart, with the User-Agent `Bookclub/1.0 (+http://localhost:5173)` (the server's development User-Agent). The recorded pages had 50 or 29 records (300 to 530 KB each). They were trimmed to a few records each by cutting whole `<record>` elements out of the raw text, so every kept byte is as delivered. The DNB repository's `Identify` reports the granularity `YYYY-MM-DDThh:mm:ssZ` and `deletedRecord` `transient`.

| File | Request | Kept |
|---|---|---|
| `reiheN_page1.xml` | `set=dnb-all:reiheN&from=2026-10-27T00:00:00Z` | Records 1, 2, 3, 10 and 11 of 50, with the real resumption token (`cursor="50" completeListSize="4112"`). Record 2 (IDN 1168979633, "Traktor Oldtimer Katalog Nr. 8") has `<header status="deleted">` and still carries its metadata. Record 1 (Meister Eckhart) has the placeholder year 2099 in 264 `$c`. Record 11 ("Yachting 2028") is in English. Every live record is an announcement with 263. |
| `reiheN_page2.xml` | `resumptionToken=<the token of page 1>` | Records 1, 12 and 16 of 50. **Hand-edited:** the real next token is replaced by the empty `<resumptionToken cursor="100" completeListSize="4112"/>` that ends a list, so this page is the last one. |
| `reiheA_complete.xml` | `set=dnb:reiheA&from=2026-10-27T22:00:00Z` | Records 1, 24 and 27 of 29. The whole list fitted on one page, so the recording has no `<resumptionToken>` element at all. Record 24 ("Die kleine Glitzerblume") has two 978-3 ISBNs; record 27 ("Bluey") has a 979-8 ISBN. |
| `bad_resumption_token.xml` | `resumptionToken=expired0token` | Verbatim: `<error code="badResumptionToken">ResumptionToken is incomplete</error>`. |
| `no_records_match.xml` | `set=dnb-all:reiheN&from=2026-10-28T23:00:00Z` | Verbatim: `<error code="noRecordsMatch"/>`. |

Hand-made from the recordings (`retitled.xml` and `deleted.xml`, built by a script from the bytes of `reiheN_page1.xml` and the envelope of `reiheA_complete.xml`):

| File | What it is |
|---|---|
| `retitled.xml` | A reiheA page that re-delivers "Bondbreaker" (IDN 1423074580, ISBN 9783690513029, from `reiheN_page1.xml`) once published: the same IDN and ISBNs, header identifier and setSpec `dnb:reiheA`, 245 `$a` changed to the final title "Bondbreaker – Bund der Schatten", and the 263 field removed (published records carry none). No resumption token. |
| `deleted.xml` | A reiheN page with the same "Bondbreaker" record under `<header status="deleted">`, metadata kept as the DNB sends it. No resumption token. |

The MARC mapping of single records has its own fixtures in `../dnb_marc/` (W1).
