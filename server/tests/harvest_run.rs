//! The DNB OAI-PMH client and its cursor (#42, work-plan HAR): the requests, paging with
//! resumption tokens, the cursor in `harvest_state` (first run, resume after a crash, restart
//! after a rejected or stale token, `harvested_until`), failures, deadlocks, the schedule and the
//! job under its advisory lock. Pages come from `tests/fixtures/dnb_oai/` (recorded on
//! 2026-10-27, trimmed; see its README) through wiremock.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bookclub_server::error::is_deadlock;
use bookclub_server::harvest::oai::{self, Answer};
use bookclub_server::harvest::run::{self, PAGE_ATTEMPTS};
use bookclub_server::harvest::{HarvestError, SetReport, harvest_due, harvest_set, run_forever};
use bookclub_server::jobs::lock_ids;
use bookclub_server::outbound::{HostThrottle, Outbound, UpstreamError};
use common::*;
use sqlx::PgPool;
use time::macros::format_description;
use time::{OffsetDateTime, PrimitiveDateTime};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

const PAGE_1: &str = include_str!("fixtures/dnb_oai/reiheN_page1.xml");
const PAGE_2: &str = include_str!("fixtures/dnb_oai/reiheN_page2.xml");
const REIHE_A: &str = include_str!("fixtures/dnb_oai/reiheA_complete.xml");
const BAD_TOKEN: &str = include_str!("fixtures/dnb_oai/bad_resumption_token.xml");
const NO_RECORDS: &str = include_str!("fixtures/dnb_oai/no_records_match.xml");

const SET_N: &str = "dnb-all:reiheN";
const SET_A: &str = "dnb:reiheA";

fn xml(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.as_bytes().to_vec(), "text/xml;charset=UTF-8")
}

/// The resumption token a recorded page carries.
fn token_of(page: &str) -> String {
    match oai::parse_list_records(page).unwrap() {
        Answer::Page(page) => page.resumption_token.expect("the page has a token"),
        other => panic!("not a page: {other:?}"),
    }
}

/// Answers the first request of a run of `set` (the one with `set` and `from`).
async fn on_list(app: &TestApp, set: &str, answer: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path("/oai/repository"))
        .and(query_param("verb", "ListRecords"))
        .and(query_param("set", set))
        .respond_with(answer)
        .mount(&app.upstream)
        .await;
}

/// Answers the request that continues with `token`.
async fn on_token(app: &TestApp, token: &str, answer: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path("/oai/repository"))
        .and(query_param("verb", "ListRecords"))
        .and(query_param("resumptionToken", token))
        .respond_with(answer)
        .mount(&app.upstream)
        .await;
}

/// reiheN: page 1 with its token, then page 2, the end of the list.
async fn mount_reihe_n(app: &TestApp) {
    on_list(app, SET_N, xml(PAGE_1)).await;
    on_token(app, &token_of(PAGE_1), xml(PAGE_2)).await;
}

async fn harvest(app: &TestApp, set: &str) -> Result<SetReport, HarvestError> {
    let mut conn = app.db.acquire().await.unwrap();
    harvest_set(&app.state, &mut conn, set, &CancellationToken::new()).await
}

/// The query of every OAI request so far, in order.
async fn oai_requests(app: &TestApp) -> Vec<HashMap<String, String>> {
    app.upstream_requests()
        .await
        .into_iter()
        .filter(|request| request.url.path() == "/oai/repository")
        .map(|request| request.url.query_pairs().into_owned().collect())
        .collect()
}

#[derive(Debug)]
struct Cursor {
    harvested_until: Option<OffsetDateTime>,
    resumption_token: Option<String>,
    token_issued_at: Option<OffsetDateTime>,
    run_started_at: Option<OffsetDateTime>,
    run_from: Option<OffsetDateTime>,
    last_success_at: Option<OffsetDateTime>,
    last_error: Option<String>,
    last_error_at: Option<OffsetDateTime>,
    records_seen: i64,
}

async fn cursor(app: &TestApp, set: &str) -> Cursor {
    sqlx::query_as!(
        Cursor,
        "SELECT harvested_until, resumption_token, token_issued_at, run_started_at, run_from,
                last_success_at, last_error, last_error_at, records_seen
         FROM harvest_state WHERE source = $1",
        format!("dnb:{set}")
    )
    .fetch_one(&app.db)
    .await
    .unwrap()
}

async fn db_now(app: &TestApp) -> OffsetDateTime {
    sqlx::query_scalar!(r#"SELECT now() AS "now!""#)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// `from` as the OAI request carries it.
fn parse_from(value: &str) -> OffsetDateTime {
    PrimitiveDateTime::parse(
        value,
        format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z"),
    )
    .unwrap_or_else(|error| panic!("from {value:?}: {error}"))
    .assume_utc()
}

fn oai_from(at: OffsetDateTime) -> String {
    at.format(format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second]Z"
    ))
    .unwrap()
}

async fn editions_with_idn(app: &TestApp) -> i64 {
    app.scalar_i64("SELECT count(*) FROM editions WHERE dnb_idn IS NOT NULL")
        .await
}

/// Waits (≤ 10 s) until `check` holds.
async fn wait_for<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..1000 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

/// A run pages through the list with the resumption token and ingests every record: the first
/// request names the format, the set and `from`; the next ones carry only the token. The end
/// of the list writes `harvested_until` and closes the run.
#[sqlx::test]
async fn a_run_pages_through_the_list(db: PgPool) {
    let app = TestApp::new(db).await;
    mount_reihe_n(&app).await;

    let report = harvest(&app, SET_N).await.unwrap();
    assert_eq!(
        report,
        SetReport {
            pages: 2,
            records: 8,
            ingested: 7,
            withdrawn: 1,
            skipped: 0,
            restarts: 0,
            completed: true,
        }
    );

    let requests = oai_requests(&app).await;
    assert_eq!(requests.len(), 2);
    let first = &requests[0];
    assert_eq!(first["verb"], "ListRecords");
    assert_eq!(first["metadataPrefix"], "MARC21-xml");
    assert_eq!(first["set"], SET_N);
    assert!(first.contains_key("from"));
    assert!(!first.contains_key("resumptionToken"));
    let second = &requests[1];
    assert_eq!(second.len(), 2, "{second:?}");
    assert_eq!(second["verb"], "ListRecords");
    assert_eq!(second["resumptionToken"], token_of(PAGE_1));
    for request in app.upstream_requests().await {
        let agent = request.headers.get("user-agent").unwrap().to_str().unwrap();
        assert!(agent.starts_with("Bookclub/1.0 (+"), "{agent}");
    }

    // 7 live records, each with one edition per ISBN group; the deleted one inserts nothing.
    assert_eq!(editions_with_idn(&app).await, 7);
    let state = cursor(&app, SET_N).await;
    assert_eq!(state.resumption_token, None);
    assert_eq!(state.token_issued_at, None);
    assert_eq!(state.run_started_at, None);
    assert_eq!(state.run_from, None);
    assert_eq!(state.records_seen, 8);
    assert_eq!(state.last_error, None);
    let now = db_now(&app).await;
    let success = state.last_success_at.expect("last_success_at");
    assert!(now - success < time::Duration::minutes(1));
    // harvested_until = the run's start minus one hour.
    let until = state.harvested_until.expect("harvested_until");
    let behind = now - until;
    assert!(
        behind >= time::Duration::minutes(60) && behind < time::Duration::minutes(61),
        "{behind}"
    );
}

/// The first run of a set starts DNB_HARVEST_START_DAYS (default 90) days back; the next run
/// starts at `harvested_until`, an hour before the previous run started.
#[sqlx::test]
async fn runs_start_ninety_days_back_then_at_harvested_until(db: PgPool) {
    let app = TestApp::new(db).await;
    on_list(&app, SET_A, xml(REIHE_A)).await;

    let started = db_now(&app).await;
    harvest(&app, SET_A).await.unwrap();
    let from = parse_from(&oai_requests(&app).await[0]["from"]);
    let expected = started - time::Duration::days(90);
    assert!(
        (from - expected).abs() < time::Duration::minutes(1),
        "{from} vs {expected}"
    );

    let until = cursor(&app, SET_A).await.harvested_until.unwrap();
    harvest(&app, SET_A).await.unwrap();
    let requests = oai_requests(&app).await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1]["from"], oai_from(until));
}

/// DNB_HARVEST_START_DAYS moves the first run's start.
#[sqlx::test]
async fn the_first_run_follows_the_start_days(db: PgPool) {
    let app = TestApp::with_env(db, &[("DNB_HARVEST_START_DAYS", "7")]).await;
    on_list(&app, SET_A, xml(REIHE_A)).await;
    let started = db_now(&app).await;
    harvest(&app, SET_A).await.unwrap();
    let from = parse_from(&oai_requests(&app).await[0]["from"]);
    let expected = started - time::Duration::days(7);
    assert!(
        (from - expected).abs() < time::Duration::minutes(1),
        "{from} vs {expected}"
    );
}

/// Acceptance (#42): the task is aborted between pages (page 1 committed, page 2 in flight);
/// the next run resumes with the stored token, without asking for page 1 again.
#[sqlx::test]
async fn a_crash_between_pages_resumes_from_the_cursor(db: PgPool) {
    let app = TestApp::new(db).await;
    let token = token_of(PAGE_1);
    on_list(&app, SET_N, xml(PAGE_1)).await;
    on_token(&app, &token, xml(PAGE_2).set_delay(Duration::from_secs(60))).await;

    let state = app.state.clone();
    let task = tokio::spawn(async move {
        let mut conn = state.db.acquire().await.unwrap();
        harvest_set(&state, &mut conn, SET_N, &CancellationToken::new()).await
    });
    wait_for("the request for page 2", || async {
        oai_requests(&app).await.len() == 2
    })
    .await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());

    // Page 1 is committed with its token; nothing of page 2 is there.
    let crashed = cursor(&app, SET_N).await;
    assert_eq!(crashed.resumption_token.as_deref(), Some(token.as_str()));
    assert!(crashed.token_issued_at.is_some());
    assert!(crashed.run_started_at.is_some());
    let run_from = crashed.run_from.expect("run_from");
    assert_eq!(crashed.records_seen, 5);
    assert_eq!(crashed.harvested_until, None);
    assert_eq!(editions_with_idn(&app).await, 4);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM editions WHERE dnb_idn = '1423076036'")
            .await,
        0
    );

    app.upstream.reset().await;
    on_token(&app, &token, xml(PAGE_2)).await;
    let report = harvest(&app, SET_N).await.unwrap();
    assert_eq!(report.pages, 1);
    assert!(report.completed);
    let requests = oai_requests(&app).await;
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0]["resumptionToken"], token);

    let done = cursor(&app, SET_N).await;
    assert_eq!(done.records_seen, 8);
    assert_eq!(editions_with_idn(&app).await, 7);
    // harvested_until comes from the interrupted run's start, not the resume.
    let until = done.harvested_until.unwrap();
    assert_eq!(
        until,
        crashed.run_started_at.unwrap() - time::Duration::hours(1)
    );
    assert!(until > run_from);
}

/// Acceptance (#42): `badResumptionToken` clears the token and restarts the run from
/// `run_from`.
#[sqlx::test]
async fn a_rejected_token_restarts_from_run_from(db: PgPool) {
    let app = TestApp::new(db).await;
    let token = token_of(PAGE_1);
    on_list(&app, SET_N, xml(PAGE_1)).await;
    on_token(&app, &token, ResponseTemplate::new(503)).await;
    harvest(&app, SET_N).await.unwrap_err();
    let failed = cursor(&app, SET_N).await;
    assert_eq!(failed.resumption_token.as_deref(), Some(token.as_str()));
    let run_from = failed.run_from.unwrap();

    // Meanwhile the DNB forgot the token.
    app.upstream.reset().await;
    on_token(&app, &token, xml(BAD_TOKEN)).await;
    on_list(&app, SET_N, xml(PAGE_2)).await;
    let report = harvest(&app, SET_N).await.unwrap();
    assert_eq!(report.restarts, 1);
    assert!(report.completed);

    let requests = oai_requests(&app).await;
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert_eq!(requests[0]["resumptionToken"], token);
    assert_eq!(requests[1]["set"], SET_N);
    assert_eq!(requests[1]["from"], oai_from(run_from));
    let done = cursor(&app, SET_N).await;
    assert_eq!(done.resumption_token, None);
    // records_seen counts the restarted run only.
    assert_eq!(done.records_seen, 3);
    assert_eq!(
        done.harvested_until.unwrap(),
        failed.run_started_at.unwrap() - time::Duration::hours(1)
    );
}

/// A token the DNB keeps rejecting fails the run instead of looping.
#[sqlx::test]
async fn a_token_rejected_again_and_again_fails_the_run(db: PgPool) {
    let app = TestApp::new(db).await;
    on_list(&app, SET_N, xml(PAGE_1)).await;
    on_token(&app, &token_of(PAGE_1), xml(BAD_TOKEN)).await;
    let error = harvest(&app, SET_N).await.unwrap_err();
    assert!(matches!(error, HarvestError::TokenRejected(4)), "{error}");
    // Four tries of page 1 after the first: a list request per restart.
    assert_eq!(oai_requests(&app).await.len(), 2 + 2 * 3);
    let state = cursor(&app, SET_N).await;
    assert!(
        state
            .last_error
            .unwrap()
            .contains("rejected the resumption token")
    );
}

/// A stored token older than six hours is not sent: the run restarts from `run_from`.
#[sqlx::test]
async fn a_stale_token_restarts_from_run_from(db: PgPool) {
    let app = TestApp::new(db).await;
    let token = token_of(PAGE_1);
    on_list(&app, SET_N, xml(PAGE_1)).await;
    on_token(&app, &token, ResponseTemplate::new(500)).await;
    harvest(&app, SET_N).await.unwrap_err();
    let run_from = cursor(&app, SET_N).await.run_from.unwrap();

    app.exec("UPDATE harvest_state SET token_issued_at = now() - interval '6 hours 1 minute'")
        .await;
    app.upstream.reset().await;
    on_list(&app, SET_N, xml(PAGE_2)).await;
    let report = harvest(&app, SET_N).await.unwrap();
    assert_eq!(report.restarts, 1);
    let requests = oai_requests(&app).await;
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0]["from"], oai_from(run_from));

    // A token younger than six hours is still used.
    sqlx::query!(
        "UPDATE harvest_state SET run_started_at = now(), run_from = now(), resumption_token = $1,
                token_issued_at = now() - interval '5 hours 59 minutes'",
        token
    )
    .execute(&app.db)
    .await
    .unwrap();
    app.upstream.reset().await;
    on_token(&app, &token, xml(PAGE_2)).await;
    harvest(&app, SET_N).await.unwrap();
    assert_eq!(oai_requests(&app).await[0]["resumptionToken"], token);
}

/// A failing page keeps the cursor at the last committed page and records the error; the
/// next run resumes there.
#[sqlx::test]
async fn a_failed_page_keeps_the_cursor(db: PgPool) {
    let app = TestApp::new(db).await;
    let token = token_of(PAGE_1);
    on_list(&app, SET_N, xml(PAGE_1)).await;
    on_token(&app, &token, ResponseTemplate::new(500)).await;

    let error = harvest(&app, SET_N).await.unwrap_err();
    assert!(
        matches!(error, HarvestError::Upstream(UpstreamError::Status(500))),
        "{error}"
    );
    let failed = cursor(&app, SET_N).await;
    assert_eq!(failed.resumption_token.as_deref(), Some(token.as_str()));
    assert_eq!(failed.records_seen, 5);
    assert!(failed.last_error.unwrap().contains("500"));
    assert!(failed.last_error_at.is_some());
    assert_eq!(failed.last_success_at, None);
    assert_eq!(editions_with_idn(&app).await, 4);

    app.upstream.reset().await;
    on_token(&app, &token, xml(PAGE_2)).await;
    harvest(&app, SET_N).await.unwrap();
    let done = cursor(&app, SET_N).await;
    assert_eq!(done.last_error, None);
    assert_eq!(done.last_error_at, None);
    assert!(done.last_success_at.is_some());
    assert_eq!(oai_requests(&app).await.len(), 1);
}

/// A page that hands back the token it was asked with would page forever: the run fails
/// before committing it, and the cursor stays.
#[sqlx::test]
async fn a_repeated_token_fails_the_run(db: PgPool) {
    let app = TestApp::new(db).await;
    let token = token_of(PAGE_1);
    on_list(&app, SET_N, xml(PAGE_1)).await;
    on_token(&app, &token, xml(PAGE_1)).await;
    let error = harvest(&app, SET_N).await.unwrap_err();
    assert!(
        matches!(&error, HarvestError::Envelope(message) if message.contains("same resumption token")),
        "{error}"
    );
    assert_eq!(oai_requests(&app).await.len(), 2);
    let state = cursor(&app, SET_N).await;
    assert_eq!(state.resumption_token, Some(token));
    assert_eq!(state.records_seen, 5);
}

/// The job policy waits out a 503 with `Retry-After`, up to three times (a request path would
/// give up after one retry).
#[sqlx::test]
async fn a_503_retry_after_is_waited_out(db: PgPool) {
    let app = TestApp::new(db).await;
    Mock::given(method("GET"))
        .and(path("/oai/repository"))
        .respond_with(ResponseTemplate::new(503).insert_header("retry-after", "1"))
        .up_to_n_times(2)
        .with_priority(1)
        .mount(&app.upstream)
        .await;
    on_list(&app, SET_A, xml(REIHE_A)).await;

    let started = Instant::now();
    let report = harvest(&app, SET_A).await.unwrap();
    assert!(report.completed);
    assert!(started.elapsed() >= Duration::from_secs(2));
    assert_eq!(oai_requests(&app).await.len(), 3);
}

/// `noRecordsMatch` is an empty, complete list.
#[sqlx::test]
async fn no_records_match_completes_the_run(db: PgPool) {
    let app = TestApp::new(db).await;
    on_list(&app, SET_N, xml(NO_RECORDS)).await;
    let report = harvest(&app, SET_N).await.unwrap();
    assert_eq!(report.pages, 1);
    assert_eq!(report.records, 0);
    assert!(report.completed);
    let state = cursor(&app, SET_N).await;
    assert!(state.harvested_until.is_some());
    assert!(state.last_success_at.is_some());
}

/// Other OAI errors and pages that are no OAI-PMH fail the run, which records why.
#[sqlx::test]
async fn oai_errors_and_garbage_fail_the_run(db: PgPool) {
    let app = TestApp::new(db).await;
    on_list(
        &app,
        SET_N,
        xml(r#"<OAI-PMH xmlns="http://www.openarchives.org/OAI/2.0/"><error code="badArgument">Unknown set</error></OAI-PMH>"#),
    )
    .await;
    let error = harvest(&app, SET_N).await.unwrap_err();
    assert!(
        matches!(&error, HarvestError::Oai { code, message } if code == "badArgument" && message == "Unknown set"),
        "{error}"
    );
    assert!(
        cursor(&app, SET_N)
            .await
            .last_error
            .unwrap()
            .contains("badArgument")
    );

    on_list(
        &app,
        SET_A,
        xml("<html><body>Wartungsarbeiten</body></html>"),
    )
    .await;
    let error = harvest(&app, SET_A).await.unwrap_err();
    assert!(matches!(error, HarvestError::Envelope(_)), "{error}");
    let truncated = &PAGE_1[..PAGE_1.find("</metadata></record>").unwrap() + 20];
    app.upstream.reset().await;
    on_list(&app, SET_A, xml(truncated)).await;
    let error = harvest(&app, SET_A).await.unwrap_err();
    assert!(matches!(error, HarvestError::Envelope(_)), "{error}");
    // Nothing of the truncated page was stored.
    assert_eq!(editions_with_idn(&app).await, 0);
}

/// Requests for the pages of one run are at least a second apart (the production DNB OAI
/// throttle).
#[sqlx::test]
async fn pages_are_at_least_a_second_apart(db: PgPool) {
    let app = TestApp::new(db).await;
    mount_reihe_n(&app).await;
    let mut outbound = Outbound::new(&app.state.config).unwrap();
    outbound.dnb_oai = HostThrottle::per_second(1.0);
    let mut state = app.state.clone();
    state.outbound = Arc::new(outbound);

    let started = Instant::now();
    let mut conn = app.db.acquire().await.unwrap();
    let report = harvest_set(&state, &mut conn, SET_N, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(report.pages, 2);
    assert!(
        started.elapsed() >= Duration::from_millis(990),
        "{:?}",
        started.elapsed()
    );
}

/// Makes the first `failures` inserts into `editions` fail with SQLSTATE 40P01, as when
/// Postgres aborts a page transaction to break a deadlock. A sequence counts the inserts: it is
/// not rolled back with the transaction.
async fn fake_deadlocks(app: &TestApp, failures: i64) {
    app.exec(&format!(
        r#"CREATE SEQUENCE fake_deadlock_calls;
           CREATE FUNCTION fake_deadlock() RETURNS trigger LANGUAGE plpgsql AS $$
           BEGIN
               IF nextval('fake_deadlock_calls') <= {failures} THEN
                   RAISE EXCEPTION 'fake deadlock' USING ERRCODE = '40P01';
               END IF;
               RETURN NEW;
           END $$;
           CREATE TRIGGER fake_deadlock BEFORE INSERT ON editions
               FOR EACH ROW EXECUTE FUNCTION fake_deadlock();"#
    ))
    .await;
}

/// A page transaction aborted by a deadlock is rolled back and tried again as a whole: every
/// record lands once and `records_seen` counts the page once.
#[sqlx::test]
async fn a_deadlocked_page_is_tried_again(db: PgPool) {
    let app = TestApp::new(db).await;
    on_list(&app, SET_A, xml(REIHE_A)).await;
    fake_deadlocks(&app, 2).await;
    let report = harvest(&app, SET_A).await.unwrap();
    assert!(report.completed);
    assert_eq!(report.ingested, 3);
    let state = cursor(&app, SET_A).await;
    assert_eq!(state.records_seen, 3);
    // One record has two ISBNs: four editions.
    assert_eq!(app.scalar_i64("SELECT count(*) FROM editions").await, 4);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM works").await, 3);
}

/// A page that keeps deadlocking fails the run after three attempts, with nothing stored.
#[sqlx::test]
async fn a_page_that_keeps_deadlocking_fails_the_run(db: PgPool) {
    let app = TestApp::new(db).await;
    on_list(&app, SET_A, xml(REIHE_A)).await;
    fake_deadlocks(&app, 1000).await;
    let error = harvest(&app, SET_A).await.unwrap_err();
    match &error {
        HarvestError::Db(db_error) => assert!(is_deadlock(db_error), "{db_error}"),
        other => panic!("{other}"),
    }
    assert_eq!(
        app.scalar_i64("SELECT last_value FROM fake_deadlock_calls")
            .await,
        i64::from(PAGE_ATTEMPTS)
    );
    assert_eq!(app.scalar_i64("SELECT count(*) FROM editions").await, 0);
    let state = cursor(&app, SET_A).await;
    assert_eq!(state.records_seen, 0);
    assert!(state.last_error.unwrap().contains("fake deadlock"));
    assert_eq!(oai_requests(&app).await.len(), 1);
}

/// A record the mapping refuses (here: no 001) and a record ingest refuses (here: a trigger
/// rejects its edition) are skipped with a log line; the rest of the page is stored and the run
/// completes, so one bad record cannot stop every later run at the same page.
#[sqlx::test]
async fn bad_records_are_skipped_not_the_page(db: PgPool) {
    let app = TestApp::new(db).await;
    let without_idn = REIHE_A.replace(r#"<controlfield tag="001">1362732249</controlfield>"#, "");
    assert_ne!(without_idn, REIHE_A);
    on_list(&app, SET_A, xml(&without_idn)).await;
    app.exec(
        r#"CREATE FUNCTION refuse_feuerwehr() RETURNS trigger LANGUAGE plpgsql AS $$
           BEGIN
               IF NEW.isbn13 = '9783690331821' THEN
                   RAISE EXCEPTION 'refused' USING ERRCODE = '23514';
               END IF;
               RETURN NEW;
           END $$;
           CREATE TRIGGER refuse_feuerwehr BEFORE INSERT ON editions
               FOR EACH ROW EXECUTE FUNCTION refuse_feuerwehr();"#,
    )
    .await;

    let report = harvest(&app, SET_A).await.unwrap();
    assert!(report.completed);
    assert_eq!((report.records, report.ingested, report.skipped), (3, 1, 2));
    let isbns: Vec<Option<String>> =
        sqlx::query_scalar!("SELECT isbn13 FROM editions ORDER BY isbn13")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(
        isbns,
        [
            Some("9783693370056".to_owned()),
            Some("9783903300989".to_owned())
        ]
    );
    // The refused record left nothing behind, not even its work.
    assert_eq!(app.scalar_i64("SELECT count(*) FROM works").await, 1);
    let state = cursor(&app, SET_A).await;
    assert_eq!(state.records_seen, 3);
    assert!(state.last_success_at.is_some());
}

/// Every set of DNB_HARVEST_SETS (default reiheN and reiheA) has its own cursor row.
#[sqlx::test]
async fn every_set_has_its_own_cursor(db: PgPool) {
    let app = TestApp::new(db).await;
    assert_eq!(app.config().dnb_harvest_sets, [SET_N, SET_A]);
    mount_reihe_n(&app).await;
    on_list(&app, SET_A, xml(REIHE_A)).await;

    let mut conn = app.db.acquire().await.unwrap();
    harvest_due(&app.state, &mut conn, &CancellationToken::new())
        .await
        .unwrap();
    let sources: Vec<String> =
        sqlx::query_scalar!("SELECT source FROM harvest_state ORDER BY source")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(sources, ["dnb:dnb-all:reiheN", "dnb:dnb:reiheA"]);
    assert_eq!(cursor(&app, SET_N).await.records_seen, 8);
    assert_eq!(cursor(&app, SET_A).await.records_seen, 3);
    let sets: Vec<String> = oai_requests(&app)
        .await
        .into_iter()
        .filter_map(|query| query.get("set").cloned())
        .collect();
    assert_eq!(sets, [SET_N, SET_A]);
}

/// A failing set does not stop the others; the pass reports it.
#[sqlx::test]
async fn a_failing_set_does_not_stop_the_others(db: PgPool) {
    let app = TestApp::new(db).await;
    on_list(&app, SET_N, ResponseTemplate::new(500)).await;
    on_list(&app, SET_A, xml(REIHE_A)).await;
    let mut conn = app.db.acquire().await.unwrap();
    let error = harvest_due(&app.state, &mut conn, &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("dnb-all:reiheN"), "{error}");
    assert!(cursor(&app, SET_N).await.last_error.is_some());
    assert!(cursor(&app, SET_A).await.last_success_at.is_some());
}

/// A set is harvested again only after DNB_HARVEST_INTERVAL_MINUTES, also after a failure; an
/// interrupted run is due at once.
#[sqlx::test]
async fn sets_wait_for_the_interval(db: PgPool) {
    let app = TestApp::with_env(db, &[("DNB_HARVEST_SETS", SET_A)]).await;
    on_list(&app, SET_A, xml(REIHE_A)).await;
    let every = run::interval(app.config());
    let pass = || async {
        let mut conn = app.db.acquire().await.unwrap();
        harvest_due(&app.state, &mut conn, &CancellationToken::new()).await
    };
    let mut conn = app.db.acquire().await.unwrap();
    let sets = app.config().dnb_harvest_sets.clone();
    assert!(run::is_due(&mut conn, SET_A, every).await.unwrap());
    assert_eq!(
        run::next_due_in(&mut conn, &sets, every).await.unwrap(),
        Duration::ZERO
    );

    pass().await.unwrap();
    assert_eq!(oai_requests(&app).await.len(), 1);
    pass().await.unwrap();
    assert_eq!(oai_requests(&app).await.len(), 1, "not due again yet");
    assert!(!run::is_due(&mut conn, SET_A, every).await.unwrap());
    let wait = run::next_due_in(&mut conn, &sets, every).await.unwrap();
    assert!(
        wait > every - Duration::from_secs(60) && wait <= every,
        "{wait:?}"
    );

    app.exec("UPDATE harvest_state SET last_success_at = now() - interval '241 minutes'")
        .await;
    pass().await.unwrap();
    assert_eq!(oai_requests(&app).await.len(), 2);

    // A failure waits the interval too.
    app.exec("UPDATE harvest_state SET last_success_at = now() - interval '241 minutes'")
        .await;
    app.upstream.reset().await;
    on_list(&app, SET_A, ResponseTemplate::new(500)).await;
    pass().await.unwrap_err();
    pass().await.unwrap();
    assert_eq!(oai_requests(&app).await.len(), 1);
    assert!(!run::is_due(&mut conn, SET_A, every).await.unwrap());

    // An interrupted run (no failure recorded) resumes at once.
    app.exec(
        "UPDATE harvest_state SET last_error = NULL, last_error_at = NULL,
                last_success_at = now(), run_started_at = now(), run_from = now()",
    )
    .await;
    assert!(run::is_due(&mut conn, SET_A, every).await.unwrap());
}

/// The job harvests every set under the advisory lock and then sleeps until the next set is
/// due; shutdown stops it.
#[sqlx::test]
async fn the_job_harvests_and_stops_on_shutdown(db: PgPool) {
    let app = TestApp::new(db).await;
    mount_reihe_n(&app).await;
    on_list(&app, SET_A, xml(REIHE_A)).await;
    let shutdown = CancellationToken::new();
    let job = tokio::spawn(run_forever(app.state.clone(), shutdown.clone()));
    wait_for("both sets", || async {
        app.scalar_i64("SELECT count(*) FROM harvest_state WHERE last_success_at IS NOT NULL")
            .await
            == 2
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(oai_requests(&app).await.len(), 3, "one pass only");
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), job)
        .await
        .expect("the job stops")
        .unwrap();
    // The lock was released.
    let locked: bool = sqlx::query_scalar!(
        r#"SELECT pg_try_advisory_lock($1) AS "locked!""#,
        lock_ids::DNB_HARVEST
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(locked);
}

/// While another instance holds the advisory lock, the job sends nothing.
#[sqlx::test]
async fn the_job_skips_while_another_instance_harvests(db: PgPool) {
    let app = TestApp::new(db).await;
    mount_reihe_n(&app).await;
    on_list(&app, SET_A, xml(REIHE_A)).await;
    let mut other = app.db.acquire().await.unwrap();
    let locked: bool = sqlx::query_scalar!(
        r#"SELECT pg_try_advisory_lock($1) AS "locked!""#,
        lock_ids::DNB_HARVEST
    )
    .fetch_one(&mut *other)
    .await
    .unwrap();
    assert!(locked);

    let shutdown = CancellationToken::new();
    let job = tokio::spawn(run_forever(app.state.clone(), shutdown.clone()));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(oai_requests(&app).await.is_empty());
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), job)
        .await
        .expect("the job stops")
        .unwrap();
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM harvest_state").await,
        0
    );
}

/// A shutdown during a page request stops the run at once; the cursor keeps the last
/// committed page.
#[sqlx::test]
async fn shutdown_stops_between_pages(db: PgPool) {
    let app = TestApp::new(db).await;
    on_list(&app, SET_N, xml(PAGE_1)).await;
    on_token(
        &app,
        &token_of(PAGE_1),
        xml(PAGE_2).set_delay(Duration::from_secs(60)),
    )
    .await;
    let shutdown = CancellationToken::new();
    let state = app.state.clone();
    let stop = shutdown.clone();
    let task = tokio::spawn(async move {
        let mut conn = state.db.acquire().await.unwrap();
        harvest_set(&state, &mut conn, SET_N, &stop).await
    });
    wait_for("the request for page 2", || async {
        oai_requests(&app).await.len() == 2
    })
    .await;
    shutdown.cancel();
    let report = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the run stops")
        .unwrap()
        .unwrap();
    assert_eq!(report.pages, 1);
    assert!(!report.completed);
    let state = cursor(&app, SET_N).await;
    assert_eq!(state.resumption_token, Some(token_of(PAGE_1)));
    assert_eq!(state.last_error, None);
}
