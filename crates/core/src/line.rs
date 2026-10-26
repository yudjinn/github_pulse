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
}

impl Line {
    pub fn new(ts: i64, level: Level, facility: impl Into<String>, msg: impl Into<String>) -> Self {
        Self {
            ts,
            level,
            facility: facility.into(),
            msg: msg.into(),
            boot: false,
        }
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
