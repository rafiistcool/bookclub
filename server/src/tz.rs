//! Time zones, meeting-time parsing and labels on top of jiff's bundled tzdb
//! (docs/phase1/conventions.md §12, docs/phase1/spec/club.md §2.1).

use std::fmt;

use jiff::Timestamp;
use jiff::civil::{Date as CivilDate, DateTime as CivilDateTime};
use jiff::tz::{Offset, TimeZone, TimeZoneDatabase};
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

/// Validates an IANA zone name on write, against jiff's bundled tzdb. (`TimeZone::get` reads
/// the host's tzdb first, and which old names that has varies: without `tzdata-legacy`, Debian
/// 13, the server image's base, lacks most of them, `Asia/Calcutta` among them.)
///
/// Every name the bundled tzdb knows passes, in its exact spelling only: case-sensitive like
/// Python's `ZoneInfo` on Linux (`Europe/Berlin`, not `europe/berlin`). An old name, a link in
/// `LINKS` such as `Asia/Calcutta`, passes too, as it did in Python, and the zone returned is
/// the one it links to: its `iana_name()` is the current name (`Asia/Kolkata`), which callers
/// store, so stored names are always current ones. [`canonical_zone_name`] returns that name.
pub fn parse_zone(name: &str) -> Result<TimeZone, InvalidZone> {
    let db = TimeZoneDatabase::bundled();
    let zone = db.get(name).map_err(|_| InvalidZone)?;
    if zone.iana_name() != Some(name) {
        return Err(InvalidZone);
    }
    match link_target(name) {
        Some(current) => db.get(current).map_err(|_| InvalidZone),
        None => Ok(zone),
    }
}

/// The name to store for a zone name from a request or a flag: [`parse_zone`]'s zone's
/// `iana_name()`, so an old name becomes the current one (`Asia/Calcutta` → `Asia/Kolkata`).
pub fn canonical_zone_name(name: &str) -> Result<String, InvalidZone> {
    let zone = parse_zone(name)?;
    zone.iana_name().map(str::to_owned).ok_or(InvalidZone)
}

/// The zone for a stored name: the host's tzdb first (`TimeZone::get`), else the bundled one,
/// which knows every name [`parse_zone`] accepts; a corrupt value falls back to UTC.
pub fn zone_or_utc(name: &str) -> TimeZone {
    TimeZone::get(name)
        .or_else(|_| TimeZoneDatabase::bundled().get(name))
        .unwrap_or(TimeZone::UTC)
}

/// The current name an old one links to, if `name` is in [`LINKS`].
fn link_target(name: &str) -> Option<&'static str> {
    LINKS
        .binary_search_by_key(&name, |&(old, _)| old)
        .ok()
        .map(|index| LINKS[index].1)
}

/// Old zone names and the current names they link to, sorted by old name (byte order).
///
/// These are the `Link` lines of tzdb 2026e (jiff-tzdb 0.1.9, the bundled copy) as built with
/// `backzone` for the names in `zone.tab` (`PACKRATLIST=zone.tab`, the `tzdata.zi` Debian
/// ships), less two kinds of names that stay as they are:
///
/// - the 11 links listed in `zone.tab` itself, each a country's own name that browsers report
///   and every tzdata install has (`Europe/Bratislava`, `Europe/Podgorica`,
///   `Arctic/Longyearbyen`, …);
/// - `UTC` (a link to `Etc/UTC`), the column default and the name every client sends.
///
/// The build with `backzone` keeps every `zone.tab` name a zone, so `Pacific/Truk` resolves to
/// `Pacific/Chuuk` (as the SPA's `currentZoneName` maps it), not to `Pacific/Port_Moresby`, and
/// `Europe/Oslo` stays. After a jiff-tzdb update, compare this list with the new `tzdata.zi`.
static LINKS: &[(&str, &str)] = &[
    ("Africa/Asmera", "Africa/Asmara"),
    ("Africa/Timbuktu", "Africa/Bamako"),
    (
        "America/Argentina/ComodRivadavia",
        "America/Argentina/Catamarca",
    ),
    ("America/Atka", "America/Adak"),
    ("America/Buenos_Aires", "America/Argentina/Buenos_Aires"),
    ("America/Catamarca", "America/Argentina/Catamarca"),
    ("America/Coral_Harbour", "America/Atikokan"),
    ("America/Cordoba", "America/Argentina/Cordoba"),
    ("America/Ensenada", "America/Tijuana"),
    ("America/Fort_Wayne", "America/Indiana/Indianapolis"),
    ("America/Godthab", "America/Nuuk"),
    ("America/Indianapolis", "America/Indiana/Indianapolis"),
    ("America/Jujuy", "America/Argentina/Jujuy"),
    ("America/Knox_IN", "America/Indiana/Knox"),
    ("America/Louisville", "America/Kentucky/Louisville"),
    ("America/Mendoza", "America/Argentina/Mendoza"),
    ("America/Montreal", "America/Toronto"),
    ("America/Nipigon", "America/Toronto"),
    ("America/Pangnirtung", "America/Iqaluit"),
    ("America/Porto_Acre", "America/Rio_Branco"),
    ("America/Rainy_River", "America/Winnipeg"),
    ("America/Rosario", "America/Argentina/Cordoba"),
    ("America/Santa_Isabel", "America/Tijuana"),
    ("America/Shiprock", "America/Denver"),
    ("America/Thunder_Bay", "America/Toronto"),
    ("America/Virgin", "America/St_Thomas"),
    ("America/Yellowknife", "America/Edmonton"),
    ("Antarctica/South_Pole", "Antarctica/McMurdo"),
    ("Asia/Ashkhabad", "Asia/Ashgabat"),
    ("Asia/Calcutta", "Asia/Kolkata"),
    ("Asia/Choibalsan", "Asia/Ulaanbaatar"),
    ("Asia/Chongqing", "Asia/Shanghai"),
    ("Asia/Chungking", "Asia/Shanghai"),
    ("Asia/Dacca", "Asia/Dhaka"),
    ("Asia/Harbin", "Asia/Shanghai"),
    ("Asia/Istanbul", "Europe/Istanbul"),
    ("Asia/Kashgar", "Asia/Urumqi"),
    ("Asia/Katmandu", "Asia/Kathmandu"),
    ("Asia/Macao", "Asia/Macau"),
    ("Asia/Rangoon", "Asia/Yangon"),
    ("Asia/Saigon", "Asia/Ho_Chi_Minh"),
    ("Asia/Tel_Aviv", "Asia/Jerusalem"),
    ("Asia/Thimbu", "Asia/Thimphu"),
    ("Asia/Ujung_Pandang", "Asia/Makassar"),
    ("Asia/Ulan_Bator", "Asia/Ulaanbaatar"),
    ("Atlantic/Faeroe", "Atlantic/Faroe"),
    ("Atlantic/Jan_Mayen", "Europe/Oslo"),
    ("Australia/ACT", "Australia/Sydney"),
    ("Australia/Canberra", "Australia/Sydney"),
    ("Australia/Currie", "Australia/Hobart"),
    ("Australia/LHI", "Australia/Lord_Howe"),
    ("Australia/NSW", "Australia/Sydney"),
    ("Australia/North", "Australia/Darwin"),
    ("Australia/Queensland", "Australia/Brisbane"),
    ("Australia/South", "Australia/Adelaide"),
    ("Australia/Tasmania", "Australia/Hobart"),
    ("Australia/Victoria", "Australia/Melbourne"),
    ("Australia/West", "Australia/Perth"),
    ("Australia/Yancowinna", "Australia/Broken_Hill"),
    ("Brazil/Acre", "America/Rio_Branco"),
    ("Brazil/DeNoronha", "America/Noronha"),
    ("Brazil/East", "America/Sao_Paulo"),
    ("Brazil/West", "America/Manaus"),
    ("CET", "Europe/Brussels"),
    ("CST6CDT", "America/Chicago"),
    ("Canada/Atlantic", "America/Halifax"),
    ("Canada/Central", "America/Winnipeg"),
    ("Canada/Eastern", "America/Toronto"),
    ("Canada/Mountain", "America/Edmonton"),
    ("Canada/Newfoundland", "America/St_Johns"),
    ("Canada/Pacific", "America/Vancouver"),
    ("Canada/Saskatchewan", "America/Regina"),
    ("Canada/Yukon", "America/Whitehorse"),
    ("Chile/Continental", "America/Santiago"),
    ("Chile/EasterIsland", "Pacific/Easter"),
    ("Cuba", "America/Havana"),
    ("EET", "Europe/Athens"),
    ("EST", "America/Panama"),
    ("EST5EDT", "America/New_York"),
    ("Egypt", "Africa/Cairo"),
    ("Eire", "Europe/Dublin"),
    ("Etc/GMT+0", "Etc/GMT"),
    ("Etc/GMT-0", "Etc/GMT"),
    ("Etc/GMT0", "Etc/GMT"),
    ("Etc/Greenwich", "Etc/GMT"),
    ("Etc/UCT", "Etc/UTC"),
    ("Etc/Universal", "Etc/UTC"),
    ("Etc/Zulu", "Etc/UTC"),
    ("Europe/Belfast", "Europe/London"),
    ("Europe/Kiev", "Europe/Kyiv"),
    ("Europe/Nicosia", "Asia/Nicosia"),
    ("Europe/Tiraspol", "Europe/Chisinau"),
    ("Europe/Uzhgorod", "Europe/Kyiv"),
    ("Europe/Zaporozhye", "Europe/Kyiv"),
    ("GB", "Europe/London"),
    ("GB-Eire", "Europe/London"),
    ("GMT", "Etc/GMT"),
    ("GMT+0", "Etc/GMT"),
    ("GMT-0", "Etc/GMT"),
    ("GMT0", "Etc/GMT"),
    ("Greenwich", "Etc/GMT"),
    ("HST", "Pacific/Honolulu"),
    ("Hongkong", "Asia/Hong_Kong"),
    ("Iceland", "Atlantic/Reykjavik"),
    ("Iran", "Asia/Tehran"),
    ("Israel", "Asia/Jerusalem"),
    ("Jamaica", "America/Jamaica"),
    ("Japan", "Asia/Tokyo"),
    ("Kwajalein", "Pacific/Kwajalein"),
    ("Libya", "Africa/Tripoli"),
    ("MET", "Europe/Brussels"),
    ("MST", "America/Phoenix"),
    ("MST7MDT", "America/Denver"),
    ("Mexico/BajaNorte", "America/Tijuana"),
    ("Mexico/BajaSur", "America/Mazatlan"),
    ("Mexico/General", "America/Mexico_City"),
    ("NZ", "Pacific/Auckland"),
    ("NZ-CHAT", "Pacific/Chatham"),
    ("Navajo", "America/Denver"),
    ("PRC", "Asia/Shanghai"),
    ("PST8PDT", "America/Los_Angeles"),
    ("Pacific/Enderbury", "Pacific/Kanton"),
    ("Pacific/Johnston", "Pacific/Honolulu"),
    ("Pacific/Ponape", "Pacific/Pohnpei"),
    ("Pacific/Samoa", "Pacific/Pago_Pago"),
    ("Pacific/Truk", "Pacific/Chuuk"),
    ("Pacific/Yap", "Pacific/Chuuk"),
    ("Poland", "Europe/Warsaw"),
    ("Portugal", "Europe/Lisbon"),
    ("ROC", "Asia/Taipei"),
    ("ROK", "Asia/Seoul"),
    ("Singapore", "Asia/Singapore"),
    ("Turkey", "Europe/Istanbul"),
    ("UCT", "Etc/UTC"),
    ("US/Alaska", "America/Anchorage"),
    ("US/Aleutian", "America/Adak"),
    ("US/Arizona", "America/Phoenix"),
    ("US/Central", "America/Chicago"),
    ("US/East-Indiana", "America/Indiana/Indianapolis"),
    ("US/Eastern", "America/New_York"),
    ("US/Hawaii", "Pacific/Honolulu"),
    ("US/Indiana-Starke", "America/Indiana/Knox"),
    ("US/Michigan", "America/Detroit"),
    ("US/Mountain", "America/Denver"),
    ("US/Pacific", "America/Los_Angeles"),
    ("US/Samoa", "Pacific/Pago_Pago"),
    ("Universal", "Etc/UTC"),
    ("W-SU", "Europe/Moscow"),
    ("WET", "Europe/Lisbon"),
    ("Zulu", "Etc/UTC"),
];

/// Parses `meeting_at`, `closes_at` and `due_at`.
///
/// - `None`, `""` or whitespace → `Ok(None)`.
/// - `YYYY-MM-DD` (midnight) or `YYYY-MM-DD[T|t| ]HH[:MM[:SS[.f|,f]]]`, optionally followed by
///   `Z`, `±HH:MM` or `±HHMM`.
/// - A value without an offset is local time in `zone`; gaps and overlaps resolve like Python's
///   `fold=0` (jiff's compatible disambiguation).
///
/// The result is UTC, in the years 1 to 9999 (Python's `datetime` range; the RFC 3339
/// serialiser of every DTO handles no other): year 0000, and a time whose UTC instant leaves
/// that range (`0001-01-01T00:00+01:00`), are refused. `Err(())` means the caller answers 400
/// `error::msg::MEETING_AT_FORMAT`.
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
    let utc = from_jiff(timestamp);
    if !(1..=9999).contains(&utc.year()) {
        return Err(());
    }
    Ok(Some(utc))
}

/// Whether a personal date (`finished_on`, a Goodreads "Date Read") may be stored with
/// [`noon_local_utc`]: 0001-01-02 to 9999-12-30. Noon on such a date is in the UTC years 1 to
/// 9999 in every zone, so it always serialises; a `time::Date` body field also accepts year 0
/// and negative years, which must not be stored. Callers answer 400 `Invalid value` otherwise.
pub fn personal_date_in_range(date: Date) -> bool {
    let first = Date::from_calendar_date(1, Month::January, 2).expect("a valid date");
    let last = Date::from_calendar_date(9999, Month::December, 30).expect("a valid date");
    (first..=last).contains(&date)
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
/// stored, so they read back as the same date in that zone). Callers check
/// [`personal_date_in_range`] first; outside it the result may not serialise.
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

    // Year 0000 is valid ISO 8601 but not a Python date (MINYEAR is 1).
    let year = cursor.digits(4).filter(|year| *year >= 1)?;
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

    /// An old name (a tzdb link such as `Asia/Calcutta`, which ICU browsers still report) is
    /// accepted, as Python's `ZoneInfo` accepted it, and resolves to the zone it names, so
    /// callers store the current name. Case still matters, and made-up names stay refused.
    #[test]
    fn old_names_resolve_to_the_current_name() {
        for (old, current) in [
            ("Asia/Calcutta", "Asia/Kolkata"),
            ("Europe/Kiev", "Europe/Kyiv"),
            ("America/Buenos_Aires", "America/Argentina/Buenos_Aires"),
            ("Pacific/Truk", "Pacific/Chuuk"),
            ("US/Eastern", "America/New_York"),
            ("Etc/Zulu", "Etc/UTC"),
        ] {
            assert_eq!(parse_zone(old).unwrap().iana_name(), Some(current), "{old}");
            assert_eq!(canonical_zone_name(old).as_deref(), Ok(current), "{old}");
        }
        // Current names, a country's own name in zone.tab (a tzdb link to Europe/Prague) and
        // `UTC` (the default, a link to Etc/UTC) are stored as they are.
        for name in [
            "Asia/Kolkata",
            "Europe/Berlin",
            "Europe/Oslo",
            "Europe/Bratislava",
            "Arctic/Longyearbyen",
            "UTC",
            "Etc/UTC",
        ] {
            assert_eq!(canonical_zone_name(name).as_deref(), Ok(name), "{name}");
        }
        for bad in [
            "asia/calcutta",
            "ASIA/CALCUTTA",
            "europe/kiev",
            "us/eastern",
            "Asia/Atlantis",
            "Etc/Unknown",
        ] {
            assert_eq!(parse_zone(bad).err(), Some(InvalidZone), "{bad:?}");
            assert_eq!(canonical_zone_name(bad), Err(InvalidZone), "{bad:?}");
        }
    }

    /// A stored name the host's tzdb lacks (Debian 13 installs no old names without
    /// `tzdata-legacy`) still reads as that zone, not as UTC.
    #[test]
    fn stored_names_fall_back_to_the_bundled_tzdb() {
        assert_eq!(
            zone_or_utc("Asia/Calcutta").iana_name(),
            Some("Asia/Calcutta")
        );
        assert_eq!(
            zone_or_utc("asia/kolkata").iana_name(),
            Some("Asia/Kolkata")
        );
    }

    /// [`LINKS`] is sorted for the binary search, and every old name and its target exist in
    /// the bundled tzdb with that spelling, are the same zone (same offsets from 1970 on), and
    /// the target is a current name itself.
    #[test]
    fn the_link_table_matches_the_bundled_tzdb() {
        let db = TimeZoneDatabase::bundled();
        assert!(LINKS.windows(2).all(|pair| pair[0].0 < pair[1].0));
        for &(old, current) in LINKS {
            let old_zone = db.get(old).unwrap();
            let current_zone = db.get(current).unwrap();
            assert_eq!(old_zone.iana_name(), Some(old));
            assert_eq!(current_zone.iana_name(), Some(current));
            assert_eq!(link_target(current), None, "{old} -> {current}");
            for year in 1970..2040 {
                for month in [1, 7] {
                    let at = CivilDate::new(year, month, 1)
                        .unwrap()
                        .at(12, 0, 0, 0)
                        .to_zoned(TimeZone::UTC)
                        .unwrap()
                        .timestamp();
                    assert_eq!(
                        old_zone.to_offset(at),
                        current_zone.to_offset(at),
                        "{old} -> {current} at {at}"
                    );
                }
            }
        }
        assert_eq!(LINKS.len(), 150);
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
            // Reasoned change (conventions §12): Python accepted ISO week dates and the basic
            // format; the Rust server does not.
            "2026-W34-6",
            "2026-W34-6T19:00",
            "20260822T1900",
            "20260822",
        ] {
            assert_eq!(parse_meeting(Some(bad), &TimeZone::UTC), Err(()), "{bad:?}");
        }
    }

    /// Every accepted value serialises as RFC 3339: the UTC instant stays in the years 1 to
    /// 9999, like Python's `datetime` (which raises OverflowError beyond it).
    #[test]
    fn times_outside_the_years_1_to_9999_are_refused() {
        for (bad, zone_name) in [
            ("0000-01-01", "UTC"),
            ("0000-01-01", "Europe/Berlin"),
            ("0000-06-01T12:00Z", "UTC"),
            ("0000-01-01T00:00+01:00", "Europe/Berlin"),
            ("0001-01-01T00:00+01:00", "UTC"),
            ("0001-01-01", "Europe/Berlin"),
            ("9999-12-31T23:00-05:00", "UTC"),
            ("9999-12-31T23:59", "America/New_York"),
        ] {
            assert_eq!(
                parse_meeting(Some(bad), &zone(zone_name)),
                Err(()),
                "{bad:?} in {zone_name}"
            );
        }
        let first = parse("0001-01-01T00:00Z", "UTC").unwrap();
        assert_eq!(first, datetime!(0001-01-01 0:00 UTC));
        let last = parse("9999-12-30T12:00", "Pacific/Kiritimati").unwrap();
        for at in [first, last] {
            let text = at.format(&time::format_description::well_known::Rfc3339);
            assert!(text.is_ok(), "{at}");
        }
    }

    #[test]
    fn personal_dates_stay_inside_the_serialisable_years() {
        for date in [
            date!(-0001 - 01 - 01),
            date!(0000 - 01 - 01),
            date!(0001 - 01 - 01),
            date!(9999 - 12 - 31),
        ] {
            assert!(!personal_date_in_range(date), "{date}");
        }
        for zone_name in ["Pacific/Kiritimati", "Pacific/Pago_Pago", "Etc/GMT+12"] {
            for date in [date!(0001 - 01 - 02), date!(9999 - 12 - 30)] {
                assert!(personal_date_in_range(date));
                let stored = noon_local_utc(date, &zone(zone_name));
                assert!((1..=9999).contains(&stored.year()), "{date} {zone_name}");
            }
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
        // UTC+14 and UTC-11 are 25 hours apart, so their dates always differ by 1 or 2 days;
        // a today() that ignored its zone would give the same date for both.
        let apart = (east - west).whole_days();
        assert!((1..=2).contains(&apart), "{apart}");
    }
}
