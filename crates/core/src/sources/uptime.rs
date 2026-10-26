//! `uptime: career up 8y 4m`, computed from `profile.career_start`.

use chrono::{Datelike, TimeZone};
use chrono_tz::Tz;

use super::Error;
use crate::line::{Level, Line};

pub const FACILITY: &str = "uptime";

pub fn line(career_start: &str, now: i64, tz: Tz) -> Line {
    match months_since(career_start, now, tz) {
        Ok(months) => Line::new(
            now,
            Level::Info,
            FACILITY,
            format!("career up {}y {}m", months / 12, months % 12),
        ),
        Err(e) => Line::warn(now, FACILITY, e.to_string()),
    }
    .boot()
}

fn months_since(start: &str, now: i64, tz: Tz) -> Result<i32, Error> {
    let bad = || Error(format!("bad career_start {start:?}, expected YYYY-MM"));
    let (year, month) = start.split_once('-').ok_or_else(bad)?;
    let year: i32 = year.parse().map_err(|_| bad())?;
    let month: i32 = month.parse().map_err(|_| bad())?;
    if !(1..=12).contains(&month) {
        return Err(bad());
    }
    let today = tz.timestamp_opt(now, 0).single().ok_or_else(bad)?;
    let months = (today.year() - year) * 12 + (today.month() as i32 - month);
    Ok(months.max(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_793_026_800; // 2026-10-26

    #[test]
    fn counts_years_and_months() {
        let l = line("2018-06", NOW, Tz::UTC);
        assert_eq!(l.msg, "career up 8y 4m");
        assert!(l.boot);
    }

    #[test]
    fn bad_input_warns() {
        for bad in ["2018", "2018-13", "June 2018"] {
            assert_eq!(line(bad, NOW, Tz::UTC).level, Level::Warn, "{bad}");
        }
    }
}
