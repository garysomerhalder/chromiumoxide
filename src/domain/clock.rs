//! `page.clock` inputs: Playwright's `parseTicks` and `parseTime`.

use crate::domain::error::{Error, Result};

/// A duration for `fast_forward` / `run_for`: milliseconds, or `"mm:ss"` /
/// `"hh:mm:ss"`.
#[derive(Debug, Clone, PartialEq)]
pub enum Ticks {
    Millis(f64),
    Text(String),
}

impl From<u64> for Ticks {
    fn from(v: u64) -> Self {
        Ticks::Millis(v as f64)
    }
}
impl From<i32> for Ticks {
    fn from(v: i32) -> Self {
        Ticks::Millis(v as f64)
    }
}
impl From<f64> for Ticks {
    fn from(v: f64) -> Self {
        Ticks::Millis(v)
    }
}
impl From<&str> for Ticks {
    fn from(v: &str) -> Self {
        Ticks::Text(v.to_owned())
    }
}

/// A point in time: milliseconds since the epoch, or an ISO 8601 date.
#[derive(Debug, Clone, PartialEq)]
pub enum ClockTime {
    Millis(f64),
    Text(String),
}

impl From<u64> for ClockTime {
    fn from(v: u64) -> Self {
        ClockTime::Millis(v as f64)
    }
}
impl From<i64> for ClockTime {
    fn from(v: i64) -> Self {
        ClockTime::Millis(v as f64)
    }
}
impl From<f64> for ClockTime {
    fn from(v: f64) -> Self {
        ClockTime::Millis(v)
    }
}
impl From<&str> for ClockTime {
    fn from(v: &str) -> Self {
        ClockTime::Text(v.to_owned())
    }
}

/// Playwright's `parseTicks`.
pub fn parse_ticks(t: &Ticks) -> Result<f64> {
    unimplemented!()
}

/// Playwright's `parseTime` for ISO 8601 dates (`YYYY-MM-DD`, optionally
/// `THH:MM[:SS[.sss]]` and `Z` / `±HH:MM`). Without an offset a date-time is
/// read as UTC (JavaScript would use the local zone).
pub fn parse_time(t: &ClockTime) -> Result<f64> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_like_playwright() {
        assert_eq!(parse_ticks(&1500u64.into()).unwrap(), 1500.0);
        assert_eq!(parse_ticks(&"01:00".into()).unwrap(), 60_000.0);
        assert_eq!(parse_ticks(&"02:03:04".into()).unwrap(), 7_384_000.0);
        assert_eq!(parse_ticks(&"5".into()).unwrap(), 5_000.0);
        assert_eq!(parse_ticks(&"".into()).unwrap(), 0.0);
        assert_eq!(
            parse_ticks(&"1:2:3:4".into()).unwrap_err().to_string(),
            "Clock only understands numbers, 'mm:ss' and 'hh:mm:ss'"
        );
        assert_eq!(
            parse_ticks(&"01:61".into()).unwrap_err().to_string(),
            "Invalid time 01:61"
        );
    }

    #[test]
    fn times_like_javascript_dates() {
        assert_eq!(parse_time(&1_700_000_000_000u64.into()).unwrap(), 1.7e12);
        assert_eq!(
            parse_time(&"2024-02-02".into()).unwrap(),
            1_706_832_000_000.0
        );
        assert_eq!(
            parse_time(&"2024-02-02T08:00:00Z".into()).unwrap(),
            1_706_860_800_000.0
        );
        assert_eq!(
            parse_time(&"2024-02-02T08:00:00.250+02:00".into()).unwrap(),
            1_706_853_600_250.0
        );
        assert_eq!(parse_time(&"1970-01-01T00:00".into()).unwrap(), 0.0);
        assert_eq!(
            parse_time(&"not a date".into()).unwrap_err().to_string(),
            "Invalid date: not a date"
        );
    }
}
