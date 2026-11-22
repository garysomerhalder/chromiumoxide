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
    let s = match t {
        Ticks::Millis(v) => return Ok(*v),
        Ticks::Text(s) if s.is_empty() => return Ok(0.0),
        Ticks::Text(s) => s,
    };
    let valid = regex::Regex::new(r"^(\d\d:){0,2}\d\d?$").expect("valid regex");
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() > 3 || !valid.is_match(s) {
        return Err(Error::InvalidArgument(
            "Clock only understands numbers, 'mm:ss' and 'hh:mm:ss'".into(),
        ));
    }
    let mut ms = 0.0;
    for (i, part) in parts.iter().rev().enumerate() {
        let v: f64 = part.parse().unwrap_or_default();
        if v >= 60.0 {
            return Err(Error::InvalidArgument(format!("Invalid time {s}")));
        }
        ms += v * 60f64.powi(i as i32);
    }
    Ok(ms * 1000.0)
}

/// Playwright's `parseTime` for ISO 8601 dates (`YYYY-MM-DD`, optionally
/// `THH:MM[:SS[.sss]]` and `Z` / `±HH:MM`). Without an offset a date-time is
/// read as UTC (JavaScript would use the local zone).
pub fn parse_time(t: &ClockTime) -> Result<f64> {
    let s = match t {
        ClockTime::Millis(v) => return Ok(*v),
        ClockTime::Text(s) => s,
    };
    let invalid = || Error::InvalidArgument(format!("Invalid date: {s}"));
    let re = regex::Regex::new(
        r"^(\d{4})-(\d{2})-(\d{2})(?:T(\d{2}):(\d{2})(?::(\d{2})(?:\.(\d{1,3}))?)?(Z|[+-]\d{2}:\d{2})?)?$",
    )
    .expect("valid regex");
    let c = re.captures(s.trim()).ok_or_else(invalid)?;
    let num = |i: usize| -> i64 { c.get(i).map_or(0, |m| m.as_str().parse().unwrap_or(0)) };
    let (y, mo, d) = (num(1), num(2), num(3));
    let (h, mi, sec) = (num(4), num(5), num(6));
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 24 || mi > 59 || sec > 59 {
        return Err(invalid());
    }
    let ms_frac = c.get(7).map_or(0, |m| {
        let digits = format!("{:0<3}", m.as_str());
        digits.parse::<i64>().unwrap_or(0)
    });
    let offset_min = match c.get(8).map(|m| m.as_str()) {
        None | Some("Z") => 0,
        Some(o) => {
            let sign = if o.starts_with('-') { -1 } else { 1 };
            let hh: i64 = o[1..3].parse().unwrap_or(0);
            let mm: i64 = o[4..6].parse().unwrap_or(0);
            sign * (hh * 60 + mm)
        }
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let (yy, m) = if mo <= 2 {
        (y - 1, mo + 9)
    } else {
        (y, mo - 3)
    };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + h * 3600 + mi * 60 + sec - offset_min * 60;
    Ok((secs * 1000 + ms_frac) as f64)
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
