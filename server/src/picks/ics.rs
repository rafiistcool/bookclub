//! The meeting download (`GET /api/clubs/{slug}/pick/meeting.ics`) and its RFC 5545 rendering:
//! one VEVENT, no dependencies (docs/phase1/spec/club.md §1.12, §2.12).

use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use time::macros::format_description;
use time::{Duration, OffsetDateTime, UtcOffset};

use super::NO_MEETING;
use crate::AppState;
use crate::catalog::shared::books::{BookRef, book_out};
use crate::clubs::extract::ClubMember;
use crate::error::{ApiResult, AppError, ErrorBody};

/// Every meeting lasts this long.
const MEETING_LENGTH: Duration = Duration::minutes(90);
/// RFC 5545 §3.1: lines are folded at 75 octets.
const FOLD_OCTETS: usize = 75;

/// What the calendar entry says.
#[derive(Debug, Clone)]
pub struct Meeting<'a> {
    pub pick_id: i64,
    pub club_name: &'a str,
    pub title: &'a str,
    pub authors: &'a str,
    pub note: &'a str,
    pub starts_at: OffsetDateTime,
    /// The book's page: `{PUBLIC_URL}/book/{work_id}`.
    pub book_url: &'a str,
    /// The club's page `{PUBLIC_URL}/c/{slug}`: the last line of the description and `URL:`.
    /// `None` leaves both out (the legacy app without a public URL).
    pub club_url: Option<&'a str>,
}

/// The `.ics` text: CRLF line ends, lines folded at 75 octets without splitting a UTF-8
/// sequence, `DTSTAMP` = `now`.
pub fn build_meeting_ics(meeting: &Meeting<'_>, now: OffsetDateTime) -> String {
    let summary = format!("{}: {}", meeting.club_name, meeting.title);
    let mut description = vec![if meeting.authors.is_empty() {
        meeting.title.to_owned()
    } else {
        format!("{} — {}", meeting.title, meeting.authors)
    }];
    if !meeting.note.is_empty() {
        description.push(meeting.note.to_owned());
    }
    description.push(meeting.book_url.to_owned());
    if let Some(url) = meeting.club_url {
        description.push(url.to_owned());
    }
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_owned(),
        "VERSION:2.0".to_owned(),
        format!("PRODID:-//{}//Bookclub//EN", escape(meeting.club_name)),
        "CALSCALE:GREGORIAN".to_owned(),
        "METHOD:PUBLISH".to_owned(),
        "BEGIN:VEVENT".to_owned(),
        format!("UID:pick-{}@bookclub", meeting.pick_id),
        format!("DTSTAMP:{}", stamp(now)),
        format!("DTSTART:{}", stamp(meeting.starts_at)),
        format!("DTEND:{}", stamp(meeting.starts_at + MEETING_LENGTH)),
        format!("SUMMARY:{}", escape(&summary)),
        format!("DESCRIPTION:{}", escape(&description.join("\n"))),
    ];
    if let Some(url) = meeting.club_url {
        lines.push(format!("URL:{url}"));
    }
    lines.push("END:VEVENT".to_owned());
    lines.push("END:VCALENDAR".to_owned());
    let mut text = lines
        .iter()
        .map(|line| fold(line))
        .collect::<Vec<_>>()
        .join("\r\n");
    text.push_str("\r\n");
    text
}

/// TEXT escaping, in this order: `\`, `;`, `,`, CRLF, LF. A lone CR and colons stay (parity).
fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace("\r\n", "\\n")
        .replace('\n', "\\n")
}

/// UTC `YYYYMMDDTHHMMSSZ`.
fn stamp(at: OffsetDateTime) -> String {
    at.to_offset(UtcOffset::UTC)
        .format(format_description!(
            "[year][month][day]T[hour][minute][second]Z"
        ))
        .expect("a UTC timestamp in the years 1 to 9999 formats")
}

/// Folds a line longer than 75 octets: a first chunk of at most 75 bytes, then continuation
/// chunks of at most 74 bytes, each after CRLF and one space. A chunk never ends inside a UTF-8
/// sequence: it backs off while the next byte is a continuation byte.
fn fold(line: &str) -> String {
    if line.len() <= FOLD_OCTETS {
        return line.to_owned();
    }
    let mut chunks: Vec<&str> = Vec::new();
    let mut rest = line;
    while !rest.is_empty() {
        let limit = if chunks.is_empty() {
            FOLD_OCTETS
        } else {
            FOLD_OCTETS - 1
        };
        let mut cut = limit.min(rest.len());
        while !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        chunks.push(&rest[..cut]);
        rest = &rest[cut..];
    }
    chunks.join("\r\n ")
}

/// The current pick's meeting as an `.ics` file (`text/calendar`). `PRODID` and `SUMMARY` use
/// the club's name; the description links the book's page and the club's page, which is also
/// `URL:`.
#[utoipa::path(
    get,
    path = "/api/clubs/{slug}/pick/meeting.ics",
    operation_id = "picks_meeting_ics",
    tag = "picks",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    responses(
        (status = 200, content_type = "text/calendar", body = String, description = "The meeting as an iCalendar file, `attachment; filename=\"bookclub-{pick_id}.ics\"`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such club`, `No meeting scheduled`")
    )
)]
pub async fn meeting_ics(
    member: ClubMember,
    State(state): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let mut conn = state.db.acquire().await?;
    let pick = sqlx::query!(
        "SELECT id, work_id, note, meeting_at FROM club_picks
         WHERE club_id = $1 AND ended_at IS NULL",
        member.club.id
    )
    .fetch_optional(&mut *conn)
    .await?;
    let Some((pick, starts_at)) = pick.and_then(|pick| pick.meeting_at.map(|at| (pick, at))) else {
        return Err(AppError::not_found(NO_MEETING));
    };
    let book = book_out(&mut conn, BookRef::work(pick.work_id))
        .await?
        .ok_or_else(|| AppError::not_found(NO_MEETING))?;
    let public_url = &state.config.public_url;
    let book_url = format!("{public_url}/book/{}", book.id);
    let club_url = format!("{public_url}/c/{}", member.club.slug);
    let body = build_meeting_ics(
        &Meeting {
            pick_id: pick.id,
            club_name: &member.club.name,
            title: &book.title,
            authors: &book.authors,
            note: &pick.note,
            starts_at,
            book_url: &book_url,
            club_url: Some(&club_url),
        },
        OffsetDateTime::now_utc(),
    );
    Ok((
        [
            (
                header::CONTENT_TYPE,
                "text/calendar; charset=utf-8".to_owned(),
            ),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"bookclub-{}.ics\"", pick.id),
            ),
        ],
        body,
    ))
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    /// The golden output of docs/phase1/spec/club.md §2.12, rendered by the Python builder:
    /// with the legacy Open Library link and public URL as inputs, the bytes are identical.
    #[test]
    fn matches_the_legacy_golden_output() {
        let meeting = Meeting {
            pick_id: 7,
            club_name: "Bookclub",
            title: "Circe",
            authors: "Madeline Miller",
            note: "Bring snacks; wine, too",
            starts_at: datetime!(2026-09-10 19:30 UTC),
            book_url: "https://openlibrary.org/works/OL1W",
            club_url: Some("https://books.example.com"),
        };
        let golden = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Bookclub//Bookclub//EN\r\n\
            CALSCALE:GREGORIAN\r\nMETHOD:PUBLISH\r\nBEGIN:VEVENT\r\nUID:pick-7@bookclub\r\n\
            DTSTAMP:20260901T120000Z\r\nDTSTART:20260910T193000Z\r\nDTEND:20260910T210000Z\r\n\
            SUMMARY:Bookclub: Circe\r\n\
            DESCRIPTION:Circe — Madeline Miller\\nBring snacks\x5c; wine\\, too\\nhttps://o\r\n \
            penlibrary.org/works/OL1W\\nhttps://books.example.com\r\n\
            URL:https://books.example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        assert_eq!(
            build_meeting_ics(&meeting, datetime!(2026-09-01 12:00 UTC)),
            golden
        );
    }

    /// The UTF-8 fold vector of spec §2.12 (Python output): `Ä` is two bytes, so a chunk ends
    /// before a character rather than inside it.
    #[test]
    fn matches_the_legacy_utf8_fold_vector() {
        let title = "Ä".repeat(40);
        let meeting = Meeting {
            pick_id: 7,
            club_name: "Bücherclub",
            title: &title,
            authors: "",
            note: "",
            starts_at: datetime!(2026-01-01 18:00 UTC),
            book_url: "https://openlibrary.org/works/OL7W",
            club_url: None,
        };
        let golden = format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Bücherclub//Bookclub//EN\r\n\
             CALSCALE:GREGORIAN\r\nMETHOD:PUBLISH\r\nBEGIN:VEVENT\r\nUID:pick-7@bookclub\r\n\
             DTSTAMP:20260101T000000Z\r\nDTSTART:20260101T180000Z\r\nDTEND:20260101T193000Z\r\n\
             SUMMARY:Bücherclub: {}\r\n {}\r\n\
             DESCRIPTION:{}\r\n {}\\nhttps://openlibrary.org/works/OL7W\r\n\
             END:VEVENT\r\nEND:VCALENDAR\r\n",
            "Ä".repeat(27),
            "Ä".repeat(13),
            "Ä".repeat(31),
            "Ä".repeat(9)
        );
        assert_eq!(
            build_meeting_ics(&meeting, datetime!(2026-01-01 00:00 UTC)),
            golden
        );
    }

    /// Escape order (backslash first), CRLF and LF become `\n`, a lone CR stays, and a long line
    /// folds into 74-byte continuations (Python output).
    #[test]
    fn escapes_and_folds_like_the_legacy_builder() {
        let note = "x".repeat(200);
        let meeting = Meeting {
            pick_id: 9,
            club_name: "Lese, Kreis; \\ Nord",
            title: "Line\r\nbreak\nand\rcr",
            authors: "A, B",
            note: &note,
            starts_at: datetime!(2026-03-29 01:30 UTC),
            book_url: "https://openlibrary.org/works/OL9W",
            club_url: Some("https://x.example"),
        };
        let golden = format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Lese\\, Kreis\x5c; \\\\ Nord//Bookclub//EN\r\n\
             CALSCALE:GREGORIAN\r\nMETHOD:PUBLISH\r\nBEGIN:VEVENT\r\nUID:pick-9@bookclub\r\n\
             DTSTAMP:20260301T080509Z\r\nDTSTART:20260329T013000Z\r\nDTEND:20260329T030000Z\r\n\
             SUMMARY:Lese\\, Kreis\x5c; \\\\ Nord: Line\\nbreak\\nand\rcr\r\n\
             DESCRIPTION:Line\\nbreak\\nand\rcr — A\\, B\\n{}\r\n {}\r\n {}\r\n \
             {}\\nhttps://openlibrary.org/works/OL9W\\nhttps://x.exampl\r\n e\r\n\
             URL:https://x.example\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
            "x".repeat(32),
            "x".repeat(74),
            "x".repeat(74),
            "x".repeat(20)
        );
        assert_eq!(
            build_meeting_ics(&meeting, datetime!(2026-03-01 08:05:09 UTC)),
            golden
        );
    }

    /// parity: test_notes_and_meeting.py::test_ics_folds_long_lines_without_breaking_utf8
    #[test]
    fn folds_long_lines_without_breaking_utf8() {
        let title = "Ä".repeat(120);
        let meeting = Meeting {
            pick_id: 7,
            club_name: "Bücherclub",
            title: &title,
            authors: "",
            note: "",
            starts_at: datetime!(2026-01-01 18:00 UTC),
            book_url: "https://books.example.com/book/7",
            club_url: Some("https://books.example.com/c/buecherclub-ab12"),
        };
        let text = build_meeting_ics(&meeting, OffsetDateTime::now_utc());
        for line in text.split("\r\n") {
            assert!(line.len() <= 75, "{line:?} is {} bytes", line.len());
        }
        let unfolded = text.replace("\r\n ", "");
        assert!(unfolded.contains(&format!("SUMMARY:Bücherclub: {}", "Ä".repeat(120))));
    }

    #[test]
    fn short_lines_and_exact_fits_stay_whole() {
        let exact = "x".repeat(75);
        assert_eq!(fold(&exact), exact);
        let one_more = "x".repeat(76);
        assert_eq!(fold(&one_more), format!("{}\r\n x", "x".repeat(75)));
        // A three-byte character straddling byte 75 moves to the next line whole.
        let euro = format!("{}€", "x".repeat(73));
        assert_eq!(fold(&euro), format!("{}\r\n €", "x".repeat(73)));
    }
}
