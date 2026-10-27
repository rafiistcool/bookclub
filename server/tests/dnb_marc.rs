//! The DNB MARC21-xml mapping on recorded records (docs/phase1/conventions.md §7.5).
//!
//! Fixtures in `tests/fixtures/dnb_marc/`, each one `<record>` copied byte for byte from a DNB
//! response recorded on 2026-10-27 (envelopes dropped):
//! - `hardback_263.xml`: IDN 1422605647, OAI `dnb-all:reiheN`. Hardback, 263, GND author, no 300.
//! - `inverted_authors.xml`: IDN 1209562634, reiheN. Two `aut` names (one with GND), ISBN-13 and
//!   ISBN-10 of one book, 653 Paperback against a "Festeinband" binding, the 9999 placeholder year.
//! - `translators_gnd.xml`: IDN 1209150832, reiheN. Two translators and two editors with GND ids,
//!   plus `aui`, `waw` and `ctb` roles that are not mapped.
//! - `translator_no_gnd.xml`: IDN 1422369358, OAI `dnb:reiheA`. A translator without GND id, a
//!   binding but no 653.
//! - `multi_isbn.xml`: IDN 850639050, SRU (1985). Two co-publishers' ISBNs, each with its own
//!   binding and `$q` publisher.
//! - `with_520_856.xml`: IDN 1422821722, reiheN, with 856 links to VLB texts. DNB's MARC21-xml
//!   carries blurbs only as 856 links, and no recorded record had a 520, so one 520 field was
//!   added to this copy by hand (marked VLB-BLURB-MUST-NOT-BE-STORED).
//! - `deleted_header.xml`: one OAI `<record>` with `<header status="deleted">`, reiheN. DNB still
//!   sends the metadata of deleted records; the caller must check the header.

use bookclub_server::catalog::shared::dnb_marc::{DnbRecord, map_record};
use bookclub_server::catalog::shared::ingest::{self, ContributorRole, Format, Mode, WorkGrouping};
use bookclub_server::extract::Patch;
use serde_json::Value;
use sqlx::PgPool;
use time::macros::date;

const FIXTURES: [&str; 7] = [
    "hardback_263.xml",
    "inverted_authors.xml",
    "translators_gnd.xml",
    "translator_no_gnd.xml",
    "multi_isbn.xml",
    "with_520_856.xml",
    "deleted_header.xml",
];

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/dnb_marc/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn map(name: &str) -> DnbRecord {
    map_record(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn isbns(rec: &DnbRecord) -> Vec<&str> {
    rec.editions
        .iter()
        .map(|e| e.isbn13.as_ref().map_or("", |i| i.as_str()))
        .collect()
}

#[test]
fn single_isbn_hardback_with_a_planned_date() {
    let rec = map("hardback_263.xml");
    assert_eq!(rec.idn, "1422605647");
    assert_eq!(
        rec.work.title,
        "Die Odyssee eines Outlaw-Journalisten. Gonzo-Briefe 1958–1976"
    );
    assert_eq!(rec.work.subtitle, None);
    assert_eq!(rec.work.author_names, "Hunter S. Thompson");
    let author = &rec.work.authors[0];
    assert_eq!(author.sort_name.as_deref(), Some("Thompson, Hunter S."));
    assert_eq!(author.gnd_id.as_deref(), Some("119232162"));
    assert_eq!(
        isbns(&rec),
        ["9783312015689"],
        "the ISBN-10 is the same book"
    );
    let e = &rec.editions[0];
    assert_eq!(e.dnb_idn.as_deref(), Some("1422605647"));
    assert_eq!(e.format, Some(Format::Hardcover));
    assert_eq!(e.publisher.as_deref(), Some("Nagel & Kimche"));
    assert_eq!(e.publication_year, Some(2028));
    assert_eq!(
        e.planned_publication_date,
        Patch::Value(date!(2028 - 09 - 01))
    );
    assert_eq!(e.language.as_deref(), Some("ger"));
    assert_eq!(e.pages, None);
    assert!(e.contributors.is_empty());
    assert_eq!(rec.work.first_publish_year, Some(2028));
}

#[test]
fn inverted_multi_author_names_come_in_natural_order() {
    let rec = map("inverted_authors.xml");
    assert_eq!(rec.work.author_names, "Thomas Steiger, Werner Amelsberg");
    let authors: Vec<(&str, Option<&str>, Option<&str>)> = rec
        .work
        .authors
        .iter()
        .map(|a| (a.name.as_str(), a.sort_name.as_deref(), a.gnd_id.as_deref()))
        .collect();
    assert_eq!(
        authors,
        [
            ("Thomas Steiger", Some("Steiger, Thomas"), Some("124422705")),
            ("Werner Amelsberg", Some("Amelsberg, Werner"), None),
        ]
    );
    assert_eq!(isbns(&rec), ["9783648139776"]);
    let e = &rec.editions[0];
    assert_eq!(
        e.format,
        Some(Format::Paperback),
        "653 wins over the binding"
    );
    assert_eq!(e.publication_year, None, "9999 is a placeholder");
    assert_eq!(
        e.planned_publication_date,
        Patch::Value(date!(2026 - 08 - 01))
    );
    assert_eq!(
        rec.work.first_publish_year,
        Some(2026),
        "from the planned date"
    );
    assert_eq!((e.pages, rec.work.pages), (Some(290), Some(290)));
    assert_eq!(e.publisher.as_deref(), Some("Haufe-Lexware"));
}

#[test]
fn translators_and_editors_with_gnd_ids() {
    let rec = map("translators_gnd.xml");
    assert_eq!(rec.work.title, "Ich kann nicht vergeben");
    assert_eq!(
        rec.work.subtitle.as_deref(),
        Some("Meine Flucht aus Auschwitz")
    );
    assert_eq!(
        rec.work.author_names, "Rudolf Vrba",
        "aui is not an author role"
    );
    let contributors: Vec<(&str, ContributorRole, Option<&str>)> = rec.editions[0]
        .contributors
        .iter()
        .map(|(a, role)| (a.name.as_str(), *role, a.gnd_id.as_deref()))
        .collect();
    assert_eq!(
        contributors,
        [
            (
                "Dagi Knellessen",
                ContributorRole::Editor,
                Some("1194960073")
            ),
            ("Werner Renz", ContributorRole::Editor, Some("1026228093")),
            (
                "Sigrid Ruschmeier",
                ContributorRole::Translator,
                Some("133971783")
            ),
            (
                "Brigitte Walitzek",
                ContributorRole::Translator,
                Some("133972100")
            ),
        ]
    );
    let e = &rec.editions[0];
    assert_eq!(e.format, Some(Format::Hardcover));
    assert_eq!(e.pages, Some(496));
    assert_eq!(e.publisher.as_deref(), Some("Schöffling"));
}

#[test]
fn a_translator_without_gnd_id_and_a_binding_without_653() {
    let rec = map("translator_no_gnd.xml");
    assert_eq!(rec.work.title, "Der Mörder von Whitechapel");
    assert_eq!(
        rec.work.subtitle.as_deref(),
        Some("ein Fall für Dr Lewis & Inspector Tennant")
    );
    assert_eq!(rec.work.author_names, "Patrice McDonough");
    let (translator, role) = &rec.editions[0].contributors[0];
    assert_eq!(*role, ContributorRole::Translator);
    assert_eq!(translator.name, "Jeanette Mey");
    assert_eq!(translator.sort_name.as_deref(), Some("Mey, Jeanette"));
    assert_eq!(translator.gnd_id, None);
    let e = &rec.editions[0];
    assert_eq!(e.format, Some(Format::Paperback), "Broschur");
    assert_eq!(e.publication_year, Some(2026), "September 2026");
    assert_eq!(e.planned_publication_date, Patch::Missing, "no 263");
    assert_eq!(e.pages, Some(350));
    assert_eq!(e.publisher.as_deref(), Some("dp Verlag"));
}

#[test]
fn a_multi_isbn_record_gives_one_edition_per_isbn() {
    let rec = map("multi_isbn.xml");
    assert_eq!(rec.idn, "850639050");
    assert_eq!(isbns(&rec), ["9783787306503", "9783857680540"]);
    let publishers: Vec<Option<&str>> = rec
        .editions
        .iter()
        .map(|e| e.publisher.as_deref())
        .collect();
    assert_eq!(publishers, [Some("Meiner"), Some("Kugler")]);
    let formats: Vec<Option<Format>> = rec.editions.iter().map(|e| e.format).collect();
    assert_eq!(formats, [Some(Format::Paperback), Some(Format::Paperback)]);
    let idns: Vec<Option<&str>> = rec.editions.iter().map(|e| e.dnb_idn.as_deref()).collect();
    assert_eq!(idns, [Some("850639050"), None]);
    assert_eq!(rec.editions[1].pages, Some(337), "XXI, 337 S.");
    assert_eq!(rec.editions[1].publication_year, Some(1985));
    assert_eq!(rec.work.author_names, "Ignaz Paul Vital Troxler");
}

#[test]
fn blurbs_and_links_are_never_mapped() {
    let xml = fixture("with_520_856.xml");
    assert!(xml.contains(r#"tag="520""#) && xml.contains("VLB-BLURB-MUST-NOT-BE-STORED"));
    assert!(
        xml.contains(r#"tag="856""#)
            && xml.contains("https://services.dnb.de/plus/idn/1422821722/blurb/")
    );
    let rec = map("with_520_856.xml");
    assert_eq!(rec.work.title, "Tante Poldi und der Gesang der Sirenen");
    assert_eq!(rec.work.description, None);
    let mapped = format!("{rec:?}");
    for forbidden in [
        "VLB-BLURB",
        "Klappentext",
        "services.dnb.de",
        "http",
        "X:MVB",
    ] {
        assert!(!mapped.contains(forbidden), "{forbidden} leaked: {mapped}");
    }
}

/// Every text column of the catalog tables, as one string.
async fn catalog_text(db: &PgPool) -> String {
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(w) FROM works w
         UNION ALL SELECT to_jsonb(e) FROM editions e
         UNION ALL SELECT to_jsonb(a) FROM authors a",
    )
    .fetch_all(db)
    .await
    .unwrap();
    serde_json::to_string(&rows).unwrap()
}

#[sqlx::test]
async fn every_fixture_ingests_without_vlb_text(db: PgPool) {
    let mut conn = db.acquire().await.unwrap();
    for name in FIXTURES {
        let rec = map(name);
        for edition in &rec.editions {
            ingest::upsert_edition_with_work(
                &mut conn,
                &rec.work,
                edition,
                WorkGrouping::DnbTitleAuthor,
                Mode::Replace,
            )
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }
    let text = catalog_text(&db).await;
    for forbidden in [
        "VLB-BLURB",
        "Klappentext",
        "services.dnb.de",
        "http",
        "X:MVB",
    ] {
        assert!(!text.contains(forbidden), "{forbidden} stored");
    }
    let works: i64 = sqlx::query_scalar("SELECT count(*) FROM works")
        .fetch_one(&db)
        .await
        .unwrap();
    let editions: i64 = sqlx::query_scalar("SELECT count(*) FROM editions")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!((works, editions), (7, 8));
    // Mapping a record again changes nothing.
    let before = catalog_text(&db).await;
    let rec = map("translators_gnd.xml");
    ingest::upsert_edition_with_work(
        &mut conn,
        &rec.work,
        &rec.editions[0],
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    let after = catalog_text(&db).await;
    let strip = |s: &str| {
        regex::Regex::new(r#""updated_at": ?"[^"]*""#)
            .unwrap()
            .replace_all(s, "")
            .into_owned()
    };
    assert_eq!(strip(&before), strip(&after));
}

#[sqlx::test]
async fn a_deleted_header_withdraws_the_edition(db: PgPool) {
    let xml = fixture("deleted_header.xml");
    // What HAR does with the envelope: read the status and the IDN from the header.
    assert!(xml.contains(r#"<header status="deleted">"#));
    let identifier = xml
        .split("<identifier>")
        .nth(1)
        .and_then(|rest| rest.split("</identifier>").next())
        .unwrap();
    let idn = identifier.rsplit('/').next().unwrap();
    assert_eq!(idn, "1168979633");

    // The metadata is still there (and maps), which is why the caller must check the header.
    let earlier = map("deleted_header.xml");
    assert_eq!(earlier.work.authors[0].name, "Udo Paulitz");
    assert_eq!(
        earlier.work.authors[0].sort_name.as_deref(),
        Some("Paulitz, Udo")
    );
    let mut conn = db.acquire().await.unwrap();
    let (_, edition) = ingest::upsert_edition_with_work(
        &mut conn,
        &earlier.work,
        &earlier.editions[0],
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();

    ingest::withdraw_dnb_edition(&mut conn, idn).await.unwrap();
    let withdrawn: bool =
        sqlx::query_scalar("SELECT withdrawn_at IS NOT NULL FROM editions WHERE id = $1")
            .bind(edition)
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(withdrawn);
}
