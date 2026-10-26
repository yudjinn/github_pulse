use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    Info,
    Warn,
    /// Last known value, shown dimmed because its source hasn't refreshed recently.
    Stale,
}

/// A clickable span of a line's `msg`. Only works when the SVG is opened directly;
/// browsers ignore links inside an SVG shown via `<img>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    /// Byte range of the linked text within `msg`.
    pub start: usize,
    pub len: usize,
    pub href: String,
}

/// One `dmesg` row: `[Sat Oct 25 14:02] facility: msg`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Line {
    /// Unix seconds.
    pub ts: i64,
    pub level: Level,
    pub facility: String,
    pub msg: String,
    /// Boot lines (uptime, hwmon) are pinned above the time-sorted feed.
    #[serde(default)]
    pub boot: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<Link>,
}

impl Line {
    pub fn new(ts: i64, level: Level, facility: impl Into<String>, msg: impl Into<String>) -> Self {
        Self {
            ts,
            level,
            facility: facility.into(),
            msg: msg.into(),
            boot: false,
            link: None,
        }
    }

    /// A line whose `msg` is `before + text + after`, with `text` linking to `href`.
    /// Only `https://` links are kept, so nothing from an API can inject `javascript:`.
    pub fn linked(
        ts: i64,
        level: Level,
        facility: impl Into<String>,
        (before, text, after): (&str, &str, &str),
        href: impl Into<String>,
    ) -> Self {
        let href = href.into();
        let mut line = Self::new(ts, level, facility, format!("{before}{text}{after}"));
        if href.starts_with("https://") && !text.is_empty() {
            line.link = Some(Link {
                start: before.len(),
                len: text.len(),
                href,
            });
        }
        line
    }

    pub fn warn(ts: i64, facility: impl Into<String>, msg: impl Into<String>) -> Self {
        Self::new(ts, Level::Warn, facility, msg)
    }

    pub fn boot(mut self) -> Self {
        self.boot = true;
        self
    }

    pub fn stale(mut self) -> Self {
        // A warning stays a warning; everything else dims.
        if self.level != Level::Warn {
            self.level = Level::Stale;
        }
        self
    }
}
