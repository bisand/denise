//! The wall clock, in local time, without a date library.
//!
//! `std` gives the number of seconds since 1970 and nothing else: no calendar,
//! no time zone. The usual answer is a crate, and for an application that is the
//! right answer. For one label on one example it is a poor trade — a widget
//! gallery that pulls in a date library has changed what it is a demonstration
//! of.
//!
//! So there are two small pieces here instead. Neither is clever and both are
//! standard; they are written out because the alternative is a dependency, not
//! because they were interesting to write.
//!
//! # What this deliberately does not do
//!
//! There is no formatting beyond one shape, no parsing, no arithmetic on dates,
//! and no leap seconds — the offset table has them and they are skipped, because
//! Unix time does not count them and neither does anything downstream of this.

use std::time::{SystemTime, UNIX_EPOCH};

/// Where a Linux or macOS system keeps the rules for its local time.
const ZONEINFO: &str = "/etc/localtime";

/// A moment, formatted, and how far into its second it is.
pub struct Now {
    /// `YYYY-MM-DD HH:MM:SS`, in local time.
    pub text: String,
    /// Milliseconds elapsed in the current second, so a caller can ask to be
    /// woken exactly when the next one starts rather than polling for it.
    pub sub_ms: u32,
}

/// Reads the clock, or `None` if the system has no idea what time it is.
pub fn now() -> Option<Now> {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    let utc = since.as_secs() as i64;
    // A machine with no zoneinfo shows UTC rather than nothing. Being an hour
    // out is a smaller failure than a blank where a clock should be.
    let local = utc + i64::from(utc_offset(utc).unwrap_or(0));

    let (year, month, day) = civil_from_days(local.div_euclid(86_400));
    let secs = local.rem_euclid(86_400);
    Some(Now {
        text: format!(
            "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
            secs / 3600,
            (secs / 60) % 60,
            secs % 60
        ),
        sub_ms: since.subsec_millis(),
    })
}

/// The year, month and day containing `days` days after 1970-01-01.
///
/// Howard Hinnant's `civil_from_days`, which is the one everybody uses: it moves
/// the epoch to March so that the leap day lands at the end of a year and the
/// month lengths become a straight line rather than a table.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Counts from a TZif header, which is the same six numbers in both versions.
struct Counts {
    isut: usize,
    isstd: usize,
    leap: usize,
    times: usize,
    types: usize,
    chars: usize,
    version: u8,
}

/// The offset from UTC in force at `at`, in seconds.
///
/// Reads `/etc/localtime` directly, which is a TZif file (RFC 8536): a list of
/// instants at which the offset changed, and a list of the offsets. Finding the
/// one in force is a search for the last transition at or before now — which is
/// why this handles summer time correctly without knowing what summer time is.
fn utc_offset(at: i64) -> Option<i32> {
    let data = std::fs::read(ZONEINFO).ok()?;
    let first = counts(&data, 0)?;

    if first.version == b'1' {
        return offset_from_block(&data, 44, &first, 4, at);
    }

    // Version 2 and later repeat the whole thing with 64-bit transition times,
    // and the second copy is the authoritative one — the first exists so that a
    // reader that only knows version 1 finds something it understands. Skipping
    // it means walking past every field, which is the only reason the sizes
    // below are spelled out.
    let after_v1 = 44
        + first.times * 5
        + first.types * 6
        + first.chars
        + first.leap * 8
        + first.isstd
        + first.isut;
    let second = counts(&data, after_v1)?;
    offset_from_block(&data, after_v1 + 44, &second, 8, at)
}

fn counts(data: &[u8], at: usize) -> Option<Counts> {
    let head = data.get(at..at + 44)?;
    if &head[0..4] != b"TZif" {
        return None;
    }
    Some(Counts {
        version: head[4],
        isut: be32(head, 20)?,
        isstd: be32(head, 24)?,
        leap: be32(head, 28)?,
        times: be32(head, 32)?,
        types: be32(head, 36)?,
        chars: be32(head, 40)?,
    })
}

/// The offset in force at `at`, from one data block.
///
/// `time_size` is 4 in a version 1 block and 8 in the block that follows it.
fn offset_from_block(
    data: &[u8],
    at: usize,
    counts: &Counts,
    time_size: usize,
    when: i64,
) -> Option<i32> {
    let times = data.get(at..at + counts.times * time_size)?;
    let indices = data
        .get(at + counts.times * time_size..)?
        .get(..counts.times)?;
    let types_at = at + counts.times * time_size + counts.times;

    // The last transition at or before `when`. They are sorted, so this could be
    // a binary search; with a few hundred entries read once a second it is not
    // worth the chance of getting the boundary wrong.
    let mut chosen: Option<usize> = None;
    for i in 0..counts.times {
        let start = i * time_size;
        let transition = if time_size == 8 {
            i64::from_be_bytes(times.get(start..start + 8)?.try_into().ok()?)
        } else {
            i64::from(i32::from_be_bytes(
                times.get(start..start + 4)?.try_into().ok()?,
            ))
        };
        if transition > when {
            break;
        }
        chosen = Some(usize::from(*indices.get(i)?));
    }

    // Before the first transition — or a zone that has never changed — the
    // convention is the first entry that is not daylight saving.
    let index = match chosen {
        Some(index) => index,
        None => (0..counts.types).find(|i| data[types_at + i * 6 + 4] == 0)?,
    };
    let entry = data.get(types_at + index * 6..types_at + index * 6 + 6)?;
    Some(i32::from_be_bytes(entry[0..4].try_into().ok()?))
}

fn be32(data: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_epoch_is_the_first_of_january() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn a_leap_day_is_a_day() {
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        assert_eq!(civil_from_days(19_783), (2024, 3, 1));
    }

    /// 2000 is a leap year and 1900 was not, which is the rule that catches
    /// implementations written from memory.
    #[test]
    fn the_hundred_year_exception_and_its_exception() {
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(-25_509), (1900, 2, 28));
        assert_eq!(civil_from_days(-25_508), (1900, 3, 1));
    }

    #[test]
    fn the_clock_reads_as_a_timestamp() {
        let now = now().expect("a system clock");
        assert_eq!(now.text.len(), 19, "{}", now.text);
        assert_eq!(&now.text[4..5], "-");
        assert_eq!(&now.text[10..11], " ");
        assert_eq!(&now.text[13..14], ":");
        assert!(now.sub_ms < 1000);
    }
}
