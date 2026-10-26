//! `hwmon: helsinki 11.8°C, wind 9 m/s, daylight 9h 12m ↓` from Open-Meteo (no key).

use serde::Deserialize;

use super::Error;
use crate::line::{Level, Line};

pub const FACILITY: &str = "hwmon";

pub fn url(latitude: f64, longitude: f64, timezone: &str) -> String {
    format!(
        "https://api.open-meteo.com/v1/forecast?latitude={latitude}&longitude={longitude}\
         &current=temperature_2m,wind_speed_10m&daily=daylight_duration\
         &wind_speed_unit=ms&timezone={}&forecast_days=2",
        timezone.replace('/', "%2F").replace('+', "%2B")
    )
}

#[derive(Deserialize)]
struct Response {
    current: Current,
    daily: Daily,
}

#[derive(Deserialize)]
struct Current {
    temperature_2m: f64,
    wind_speed_10m: f64,
}

#[derive(Deserialize)]
struct Daily {
    /// Seconds of daylight; today first, then tomorrow.
    daylight_duration: Vec<f64>,
}

pub fn parse(json: &str, place: &str, now: i64) -> Result<Line, Error> {
    let r: Response = serde_json::from_str(json)?;
    let today = *r
        .daily
        .daylight_duration
        .first()
        .ok_or_else(|| Error("no daylight data".into()))?;
    let trend = match r.daily.daylight_duration.get(1) {
        Some(&tomorrow) if tomorrow > today + 1.0 => " ↑",
        Some(&tomorrow) if tomorrow < today - 1.0 => " ↓",
        _ => "",
    };
    let minutes = (today / 60.0).round() as i64;
    let msg = format!(
        "{place} {:.1}°C, wind {:.0} m/s, daylight {}h {:02}m{trend}",
        r.current.temperature_2m,
        r.current.wind_speed_10m,
        minutes / 60,
        minutes % 60,
    );
    Ok(Line::new(now, Level::Info, FACILITY, msg).boot())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../../tests/fixtures/open-meteo.json");

    #[test]
    fn parses_fixture() {
        let l = parse(FIXTURE, "helsinki", 0).unwrap();
        assert_eq!(l.msg, "helsinki 11.8°C, wind 9 m/s, daylight 9h 12m ↓");
        assert!(l.boot);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("{}", "x", 0).is_err());
        assert!(parse("<html>", "x", 0).is_err());
    }

    #[test]
    fn url_encodes_timezone() {
        assert!(url(60.17, 24.94, "Europe/Helsinki").contains("timezone=Europe%2FHelsinki&"));
    }
}
