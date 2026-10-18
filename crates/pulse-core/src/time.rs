//! UTC calendar helpers without a date library, so the WebAssembly module stays small.
//!
//! Timestamps are Unix seconds and dates are days since 1970-01-01.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const SECONDS_PER_DAY: i64 = 86_400;

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
pub fn days_from_civil(year: i32, month: u32, day: u32) -> i32 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = (year - era * 400) as u32;
    let month_index = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era as i32 - 719_468
}

pub fn civil_from_days(days: i32) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u32;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era as i32 + era * 400 + i32::from(month <= 2);
    (year, month, day)
}

pub fn day_of(timestamp: i64) -> i32 {
    timestamp.div_euclid(SECONDS_PER_DAY) as i32
}

/// `YYYY-MM-DD`
pub fn format_date(days: i32) -> String {
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Parse the leading `YYYY-MM-DD` of a date or timestamp string.
pub fn parse_date(value: &str) -> Option<i32> {
    let bytes = value.as_bytes();
    if bytes.len() < 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let year: i32 = value.get(0..4)?.parse().ok()?;
    let month: u32 = value.get(5..7)?.parse().ok()?;
    let day: u32 = value.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day))
}

/// Parse RFC 3339 / ISO 8601 timestamps such as `2026-10-18T06:02:37.945611Z`,
/// `2025-08-19T10:31:52-07:00`, or `2026-10-18 06:02:37+00:00`.
pub fn parse_timestamp(value: &str) -> Option<i64> {
    let value = value.trim();
    let days = parse_date(value)?;
    let rest = &value[10..];
    if rest.is_empty() {
        return Some(i64::from(days) * SECONDS_PER_DAY);
    }
    let rest = rest.strip_prefix(['T', 't', ' '])?;
    let time = rest.get(0..8)?;
    let hour: i64 = time.get(0..2)?.parse().ok()?;
    let minute: i64 = time.get(3..5)?.parse().ok()?;
    let second: i64 = time.get(6..8)?.parse().ok()?;
    let mut tail = &rest[8..];
    if let Some(fraction) = tail.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        tail = &fraction[digits..];
    }
    let offset = match tail {
        "" | "Z" | "z" => 0,
        _ => {
            let sign = match tail.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let digits: String = tail[1..].chars().filter(char::is_ascii_digit).collect();
            if digits.len() != 4 {
                return None;
            }
            let hours: i64 = digits[0..2].parse().ok()?;
            let minutes: i64 = digits[2..4].parse().ok()?;
            sign * (hours * 3600 + minutes * 60)
        }
    };
    Some(i64::from(days) * SECONDS_PER_DAY + hour * 3600 + minute * 60 + second - offset)
}

fn split_timestamp(timestamp: i64) -> (String, i64, i64, i64) {
    let days = day_of(timestamp);
    let seconds = timestamp - i64::from(days) * SECONDS_PER_DAY;
    (format_date(days), seconds / 3600, seconds % 3600 / 60, seconds % 60)
}

/// `YYYY-MM-DDTHH:MM:SSZ`, used in stored state.
pub fn format_timestamp(timestamp: i64) -> String {
    let (date, hour, minute, second) = split_timestamp(timestamp);
    format!("{date}T{hour:02}:{minute:02}:{second:02}Z")
}

/// `YYYY-MM-DDTHH:MM:SS+00:00`, the timestamp format of API responses.
pub fn format_iso(timestamp: i64) -> String {
    let (date, hour, minute, second) = split_timestamp(timestamp);
    format!("{date}T{hour:02}:{minute:02}:{second:02}+00:00")
}

/// A UTC timestamp stored as Unix seconds and serialized as an RFC 3339 string.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ts(pub i64);

impl Ts {
    pub fn day(self) -> i32 {
        day_of(self.0)
    }
}

impl Serialize for Ts {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format_timestamp(self.0))
    }
}

impl<'de> Deserialize<'de> for Ts {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        parse_timestamp(&value).map(Ts).ok_or_else(|| serde::de::Error::custom(format!("invalid timestamp {value:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trip() {
        for days in [-1000, 0, 1, 365, 19_000, 20_744, 30_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(format_date(0), "1970-01-01");
        assert_eq!(parse_date("2026-10-18"), Some(days_from_civil(2026, 10, 18)));
    }

    #[test]
    fn timestamps() {
        let base = parse_timestamp("2026-10-18T06:02:37Z").unwrap();
        assert_eq!(parse_timestamp("2026-10-18T06:02:37.945611Z"), Some(base));
        assert_eq!(parse_timestamp("2026-10-17T23:02:37-07:00"), Some(base));
        assert_eq!(parse_timestamp("2026-10-18 06:02:37+00:00"), Some(base));
        assert_eq!(format_timestamp(base), "2026-10-18T06:02:37Z");
        assert_eq!(format_iso(base), "2026-10-18T06:02:37+00:00");
        assert_eq!(parse_timestamp("nope"), None);
    }
}
