//! Time zones, meeting-time parsing and labels on top of jiff's bundled tzdb
//! (docs/phase1/conventions.md §12, docs/phase1/spec/club.md §2.1).

use std::fmt;

use jiff::Timestamp;
use jiff::civil::{Date as CivilDate, DateTime as CivilDateTime};
use jiff::tz::{Offset, TimeZone};
use time::{Date, Month, OffsetDateTime};

use crate::error::msg;
use crate::text::py_strip;

/// The name is not an IANA time zone in the bundled tzdb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidZone;

impl fmt::Display for InvalidZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(msg::INVALID_TIME_ZONE)
    }
}

impl std::error::Error for InvalidZone {}

/// Validates an IANA zone name on write. Case-sensitive, like Python's `ZoneInfo` on Linux:
/// only the canonical spelling is accepted, so stored names are always canonical.
pub fn parse_zone(name: &str) -> Result<TimeZone, InvalidZone> {
    let zone = TimeZone::get(name).map_err(|_| InvalidZone)?;
    if zone.iana_name() == Some(name) {
        Ok(zone)
    } else {
        Err(InvalidZone)
    }
}

/// The zone for a stored name; a corrupt value falls back to UTC.
pub fn zone_or_utc(name: &str) -> TimeZone {
    TimeZone::get(name).unwrap_or(TimeZone::UTC)
}

/// Parses `meeting_at`, `closes_at` and `due_at`.
///
/// - `None`, `""` or whitespace → `Ok(None)`.
/// - `YYYY-MM-DD` (midnight) or `YYYY-MM-DD[T|t| ]HH[:MM[:SS[.f|,f]]]`, optionally followed by
///   `Z`, `±HH:MM` or `±HHMM`.
/// - A value without an offset is local time in `zone`; gaps and overlaps resolve like Python's
///   `fold=0` (jiff's compatible disambiguation).
///
/// The result is UTC. `Err(())` means the caller answers 400 `error::msg::MEETING_AT_FORMAT`.
#[allow(clippy::result_unit_err)] // frozen signature (conventions §12); the caller owns the message
pub fn parse_meeting(value: Option<&str>, zone: &TimeZone) -> Result<Option<OffsetDateTime>, ()> {
    let Some(value) = value else {
        return Ok(None);
    };
    let text = py_strip(value);
    if text.is_empty() {
        return Ok(None);
    }
    let (local, offset) = parse_iso(text).ok_or(())?;
    let timestamp = match offset {
        Some(offset) => offset.to_timestamp(local).map_err(|_| ())?,
        None => zone
            .to_ambiguous_timestamp(local)
            .compatible()
            .map_err(|_| ())?,
    };
    Ok(Some(from_jiff(timestamp)))
}

/// `"%Y-%m-%dT%H:%M"` in `zone`, the value of a `datetime-local` input.
pub fn local_minute(dt: OffsetDateTime, zone: &TimeZone) -> String {
    to_jiff(dt)
        .to_zoned(zone.clone())
        .strftime("%Y-%m-%dT%H:%M")
        .to_string()
}

/// `"%a %d %b %Y, %H:%M %Z"` in `zone`, always English, e.g. `"Wed 01 Jul 2026, 20:00 CEST"`.
pub fn label(dt: OffsetDateTime, zone: &TimeZone) -> String {
    to_jiff(dt)
        .to_zoned(zone.clone())
        .strftime("%a %d %b %Y, %H:%M %Z")
        .to_string()
}

/// Today's date in `zone`.
pub fn today(zone: &TimeZone) -> Date {
    to_time_date(Timestamp::now().to_zoned(zone.clone()).date())
}

/// 12:00 local time on `date` in `zone`, as UTC (how personal dates such as `finished_on` are
/// stored, so they read back as the same date in that zone).
pub fn noon_local_utc(date: Date, zone: &TimeZone) -> OffsetDateTime {
    let local = CivilDate::new(
        date.year() as i16,
        u8::from(date.month()) as i8,
        date.day() as i8,
    )
    .map(|civil| civil.at(12, 0, 0, 0));
    match local.and_then(|local| zone.to_ambiguous_timestamp(local).compatible()) {
        Ok(timestamp) => from_jiff(timestamp),
        // Years outside jiff's range (beyond ±9999): fall back to noon UTC.
        Err(_) => date.midnight().assume_utc() + time::Duration::hours(12),
    }
}

fn to_jiff(dt: OffsetDateTime) -> Timestamp {
    Timestamp::from_nanosecond(dt.unix_timestamp_nanos()).unwrap_or(if dt.year() < 0 {
        Timestamp::MIN
    } else {
        Timestamp::MAX
    })
}

fn from_jiff(timestamp: Timestamp) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(timestamp.as_nanosecond())
        .expect("jiff's timestamp range lies inside time's")
}

fn to_time_date(date: CivilDate) -> Date {
    let month = Month::try_from(date.month() as u8).expect("jiff months are 1..=12");
    Date::from_calendar_date(i32::from(date.year()), month, date.day() as u8)
        .expect("jiff dates are valid calendar dates")
}

/// The accepted ISO 8601 subset (see [`parse_meeting`]). Returns the local date-time and the
/// explicit offset, if any.
fn parse_iso(text: &str) -> Option<(CivilDateTime, Option<Offset>)> {
    let bytes = text.as_bytes();
    let mut cursor = Cursor { bytes, pos: 0 };

    let year = cursor.digits(4)?;
    cursor.expect(b'-')?;
    let month = cursor.digits(2)?;
    cursor.expect(b'-')?;
    let day = cursor.digits(2)?;
    let date = CivilDate::new(year as i16, month as i8, day as i8).ok()?;
    if cursor.done() {
        return Some((date.at(0, 0, 0, 0), None));
    }

    if !matches!(cursor.next()?, b'T' | b't' | b' ') {
        return None;
    }
    let hour = cursor.digits(2)?;
    let (mut minute, mut second, mut nanos) = (0, 0, 0);
    if cursor.peek() == Some(b':') {
        cursor.pos += 1;
        minute = cursor.digits(2)?;
        if cursor.peek() == Some(b':') {
            cursor.pos += 1;
            second = cursor.digits(2)?;
            if matches!(cursor.peek(), Some(b'.' | b',')) {
                cursor.pos += 1;
                nanos = cursor.fraction()?;
            }
        }
    }
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let local = date.at(hour as i8, minute as i8, second as i8, nanos);

    let offset = match cursor.peek() {
        None => None,
        Some(b'Z') => {
            cursor.pos += 1;
            Some(Offset::UTC)
        }
        Some(sign @ (b'+' | b'-')) => {
            cursor.pos += 1;
            let hours = cursor.digits(2)?;
            if cursor.peek() == Some(b':') {
                cursor.pos += 1;
            }
            let minutes = cursor.digits(2)?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            let seconds = (hours * 3600 + minutes * 60) as i32;
            let seconds = if sign == b'-' { -seconds } else { seconds };
            Some(Offset::from_seconds(seconds).ok()?)
        }
        Some(_) => return None,
    };
    cursor.done().then_some((local, offset))
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn done(&self) -> bool {
        self.pos == self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.pos += 1;
        Some(byte)
    }

    fn expect(&mut self, byte: u8) -> Option<()> {
        (self.next()? == byte).then_some(())
    }

    /// Exactly `count` ASCII digits.
    fn digits(&mut self, count: usize) -> Option<u32> {
        let end = self.pos.checked_add(count)?;
        let slice = self.bytes.get(self.pos..end)?;
        if !slice.iter().all(u8::is_ascii_digit) {
            return None;
        }
        self.pos = end;
        Some(slice.iter().fold(0, |n, d| n * 10 + u32::from(d - b'0')))
    }

    /// 1 to 9 fraction digits, as nanoseconds.
    fn fraction(&mut self) -> Option<i32> {
        let start = self.pos;
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
        }
        let digits = &self.bytes[start..self.pos];
        if digits.is_empty() || digits.len() > 9 {
            return None;
        }
        let value = digits
            .iter()
            .fold(0i32, |n, d| n * 10 + i32::from(d - b'0'));
        Some(value * 10i32.pow(9 - digits.len() as u32))
    }
}

#[cfg(test)]
mod tests {
    use time::macros::{date, datetime};

    use super::*;

    fn zone(name: &str) -> TimeZone {
        parse_zone(name).unwrap()
    }

    fn parse(value: &str, zone_name: &str) -> Option<OffsetDateTime> {
        parse_meeting(Some(value), &zone(zone_name)).unwrap()
    }

    #[test]
    fn zone_names_are_validated() {
        assert!(parse_zone("Europe/Berlin").is_ok());
        assert!(parse_zone("UTC").is_ok());
        assert!(parse_zone("America/Sao_Paulo").is_ok());
        for bad in [
            "utc",
            "europe/berlin",
            "Mars/Olympus",
            "",
            "../etc/passwd",
            " Europe/Berlin",
        ] {
            assert_eq!(parse_zone(bad).err(), Some(InvalidZone), "{bad:?}");
        }
        assert_eq!(InvalidZone.to_string(), "That is not a valid time zone");
        assert_eq!(zone_or_utc("Not/AZone"), TimeZone::UTC);
        assert_eq!(
            zone_or_utc("Asia/Kolkata").iana_name(),
            Some("Asia/Kolkata")
        );
    }

    #[test]
    fn empty_values_are_none() {
        let utc = TimeZone::UTC;
        assert_eq!(parse_meeting(None, &utc), Ok(None));
        assert_eq!(parse_meeting(Some(""), &utc), Ok(None));
        assert_eq!(parse_meeting(Some("  \t"), &utc), Ok(None));
    }

    #[test]
    fn accepted_forms() {
        // docs/phase1/spec/club.md §2.1, probed against Python 3.12 fromisoformat.
        let berlin_19 = Some(datetime!(2026-08-22 17:00 UTC));
        assert_eq!(
            parse("2026-08-22", "UTC"),
            Some(datetime!(2026-08-22 0:00 UTC))
        );
        assert_eq!(
            parse("2026-08-22", "Europe/Berlin"),
            Some(datetime!(2026-08-21 22:00 UTC))
        );
        assert_eq!(parse("2026-08-22T19", "Europe/Berlin"), berlin_19);
        assert_eq!(parse("2026-08-22T19:00", "Europe/Berlin"), berlin_19);
        assert_eq!(parse("2026-08-22 19:00", "Europe/Berlin"), berlin_19);
        assert_eq!(parse("2026-08-22t19:00", "Europe/Berlin"), berlin_19);
        assert_eq!(parse(" 2026-08-22T19:00:00 ", "Europe/Berlin"), berlin_19);
        assert_eq!(
            parse("2026-08-22T19:00:00.5", "UTC"),
            Some(datetime!(2026-08-22 19:00:00.5 UTC))
        );
        assert_eq!(
            parse("2026-08-22T19:00:00,25", "UTC"),
            Some(datetime!(2026-08-22 19:00:00.25 UTC))
        );
        assert_eq!(
            parse("2026-08-22T19:00Z", "Europe/Berlin"),
            Some(datetime!(2026-08-22 19:00 UTC))
        );
        assert_eq!(
            parse("2026-08-22T19:00+02:00", "America/Sao_Paulo"),
            Some(datetime!(2026-08-22 17:00 UTC))
        );
        assert_eq!(
            parse("2026-08-22T19:00-0330", "UTC"),
            Some(datetime!(2026-08-22 22:30 UTC))
        );
    }

    #[test]
    fn rejected_forms() {
        for bad in [
            "2026-234",
            "2026-08-22T24:00",
            "2026-8-2",
            "08/22/2026",
            "next Tuesday",
            "2026-02-30",
            "2026-08-22T19:60",
            "2026-08-22T19:00:00.",
            "2026-08-22T19:00+2",
            "2026-08-22X19:00",
            "2026-08-22T19:00 Z",
            "2026-08-22Z",
            "２０２６-08-22",
        ] {
            assert_eq!(parse_meeting(Some(bad), &TimeZone::UTC), Err(()), "{bad:?}");
        }
    }

    #[test]
    fn dst_gaps_and_overlaps_behave_like_fold_zero() {
        // Spring-forward gap: the pre-transition offset (+01:00) applies.
        assert_eq!(
            parse("2026-03-29T02:30", "Europe/Berlin"),
            Some(datetime!(2026-03-29 01:30 UTC))
        );
        // Fall-back overlap: the first occurrence (+02:00 CEST).
        assert_eq!(
            parse("2026-10-25T02:30", "Europe/Berlin"),
            Some(datetime!(2026-10-25 00:30 UTC))
        );
        let berlin = zone("Europe/Berlin");
        assert_eq!(
            label(datetime!(2026-03-29 01:30 UTC), &berlin),
            "Sun 29 Mar 2026, 03:30 CEST"
        );
    }

    #[test]
    fn labels_match_strftime_in_the_c_locale() {
        assert_eq!(
            label(datetime!(2026-08-22 19:00 UTC), &TimeZone::UTC),
            "Sat 22 Aug 2026, 19:00 UTC"
        );
        assert_eq!(
            label(datetime!(2026-07-01 18:00 UTC), &zone("Europe/Berlin")),
            "Wed 01 Jul 2026, 20:00 CEST"
        );
        assert_eq!(
            label(datetime!(2026-08-22 22:00 UTC), &zone("America/Sao_Paulo")),
            "Sat 22 Aug 2026, 19:00 -03"
        );
        assert_eq!(
            label(datetime!(2026-08-22 22:00 UTC), &zone("Asia/Kolkata")),
            "Sun 23 Aug 2026, 03:30 IST"
        );
    }

    #[test]
    fn local_minute_drops_seconds() {
        assert_eq!(
            local_minute(datetime!(2026-08-22 17:00:59 UTC), &zone("Europe/Berlin")),
            "2026-08-22T19:00"
        );
    }

    #[test]
    fn noon_local_round_trips_the_date() {
        let kiritimati = zone("Pacific/Kiritimati");
        let stored = noon_local_utc(date!(2026 - 01 - 01), &kiritimati);
        assert_eq!(stored, datetime!(2025-12-31 22:00 UTC));
        assert_eq!(local_minute(stored, &kiritimati), "2026-01-01T12:00");
        assert_eq!(
            noon_local_utc(date!(2026 - 07 - 01), &zone("America/New_York")),
            datetime!(2026-07-01 16:00 UTC)
        );
    }

    #[test]
    fn today_is_in_the_zone() {
        let utc_today = today(&TimeZone::UTC);
        let east = today(&zone("Pacific/Kiritimati"));
        let west = today(&zone("Pacific/Pago_Pago"));
        assert!(east >= utc_today && west <= utc_today);
        assert!((east - west).whole_days() <= 2);
    }
}
