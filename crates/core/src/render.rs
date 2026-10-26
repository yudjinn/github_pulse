//! `Vec<Line>` → a terminal-styled SVG that looks like `dmesg -T --follow`.

use std::fmt::Write;

use chrono::{DateTime, TimeZone};
use chrono_tz::Tz;
use unicode_width::UnicodeWidthChar;

use crate::line::{Level, Line};

const WIDTH: u32 = 840;
const PAD_X: u32 = 16;
const PAD_Y: u32 = 14;
const LINE_H: u32 = 20;
const FONT_SIZE: f32 = 13.0;
/// Monospace advance is roughly 0.6 em; used only to decide where to truncate.
const CHAR_W: f32 = FONT_SIZE * 0.6;
const COLS: usize = ((WIDTH - 2 * PAD_X) as f32 / CHAR_W) as usize;

const STYLE: &str = r#"
text { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace; font-size: 13px; fill: #c9d1d9; white-space: pre; }
.p { fill: #7ee787; }
.ts { fill: #8b949e; }
.fac { fill: #79c0ff; }
.ok { fill: #7ee787; }
.warn, .warn .fac { fill: #d29922; }
.stale, .stale .ts, .stale .fac { fill: #6e7681; }
.cur { animation: blink 1.1s steps(1) infinite; }
@keyframes blink { 50% { fill-opacity: 0; } }
"#;

pub struct RenderOptions<'a> {
    pub tz: Tz,
    /// Shown on the final `<login> login: ▌` prompt.
    pub login: &'a str,
}

/// Parse an IANA zone name, falling back to UTC (e.g. if the zone was filtered out
/// of the build by `CHRONO_TZ_TIMEZONE_FILTER`).
pub fn parse_tz(name: &str) -> Tz {
    name.parse().unwrap_or(Tz::UTC)
}

pub fn render(lines: &[Line], opts: &RenderOptions) -> String {
    let lines = ordered(lines);
    let rows = lines.len() as u32 + 2; // + header and login prompt
    let height = 2 * PAD_Y + rows * LINE_H;

    let mut svg = String::with_capacity(1024 + lines.len() * 256);
    let _ = write!(
        svg,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{height}" viewBox="0 0 {WIDTH} {height}" xml:space="preserve"><style>{STYLE}</style><rect width="{WIDTH}" height="{height}" rx="8" fill="#0d1117"/>"##
    );

    let mut row = 0;
    let mut text = |svg: &mut String, class: Option<&str>, body: &str| {
        let y = PAD_Y + row * LINE_H + LINE_H * 3 / 4;
        let class = class.map(|c| format!(r#" class="{c}""#)).unwrap_or_default();
        let _ = write!(svg, r#"<text x="{PAD_X}" y="{y}"{class}>{body}</text>"#);
        row += 1;
    };

    text(&mut svg, None, r#"<tspan class="p">$</tspan> dmesg -T --follow"#);

    for line in &lines {
        let ts = format_ts(line.ts, opts.tz);
        let prefix_cols = ts.chars().count() + 1 + line.facility.chars().count() + 2;
        let msg = truncate(&line.msg, COLS.saturating_sub(prefix_cols));
        let class = match line.level {
            Level::Ok => Some("ok"),
            Level::Info => None,
            Level::Warn => Some("warn"),
            Level::Stale => Some("stale"),
        };
        let body = format!(
            r#"<tspan class="ts">{}</tspan> <tspan class="fac">{}:</tspan> {}"#,
            escape(&ts),
            escape(&line.facility),
            escape(&msg),
        );
        text(&mut svg, class, &body);
    }

    let prompt = format!(
        r#"{} login: <tspan class="cur">▌</tspan>"#,
        escape(&truncate(opts.login, COLS - 10))
    );
    text(&mut svg, None, &prompt);

    svg.push_str("</svg>\n");
    svg
}

/// Boot lines first in their given order, then the rest newest first.
fn ordered(lines: &[Line]) -> Vec<&Line> {
    let (mut boot, mut feed): (Vec<_>, Vec<_>) = lines.iter().partition(|l| l.boot);
    feed.sort_by_key(|l| std::cmp::Reverse(l.ts));
    boot.append(&mut feed);
    boot
}

/// `[Sat Oct 25 14:02]`, like `dmesg -T` without seconds and year.
fn format_ts(ts: i64, tz: Tz) -> String {
    let dt: DateTime<Tz> = tz
        .timestamp_opt(ts, 0)
        .single()
        .unwrap_or_else(|| tz.timestamp_opt(0, 0).unwrap());
    dt.format("[%a %b %e %H:%M]").to_string()
}

/// Cut `s` to at most `max_cols` terminal columns, ending in `…` if anything was dropped.
/// Wide (e.g. CJK) characters count as two columns.
pub fn truncate(s: &str, max_cols: usize) -> String {
    let width = |c: char| c.width().unwrap_or(0);
    if s.chars().map(width).sum::<usize>() <= max_cols {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = width(c);
        if used + w + 1 > max_cols {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// XML-escape text for element content and attribute values.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Control characters are invalid in XML 1.0.
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-10-26 15:00:00 UTC, 17:00 in Helsinki.
    const NOW: i64 = 1_793_026_800;

    fn opts() -> RenderOptions<'static> {
        RenderOptions {
            tz: parse_tz("Europe/Helsinki"),
            login: "yudjinn",
        }
    }

    #[test]
    fn escapes_dynamic_strings() {
        assert_eq!(
            escape(r#"Rock & Roll <Live> "x" 'y'"#),
            "Rock &amp; Roll &lt;Live&gt; &quot;x&quot; &apos;y&apos;"
        );
        let svg = render(
            &[Line::new(NOW, Level::Info, "spotify", "Rock & Roll <Live>")],
            &opts(),
        );
        assert!(svg.contains("Rock &amp; Roll &lt;Live&gt;"));
        assert!(!svg.contains("<Live>"));
    }

    #[test]
    fn truncates_by_display_width() {
        assert_eq!(truncate("hello", 5), "hello");
        assert_eq!(truncate("hello world", 6), "hello…");
        // Each CJK char is two columns: 2 chars (4 cols) + ellipsis fit in 6.
        assert_eq!(truncate("東京事変の曲", 6), "東京…");
        assert_eq!(truncate("東京", 4), "東京");
    }

    #[test]
    fn boot_lines_pinned_then_newest_first() {
        let lines = [
            Line::new(NOW - 600, Level::Info, "git", "older"),
            Line::new(NOW, Level::Info, "uptime", "boot").boot(),
            Line::new(NOW - 60, Level::Info, "git", "newer"),
        ];
        let order: Vec<_> = ordered(&lines).iter().map(|l| l.msg.as_str()).collect();
        assert_eq!(order, ["boot", "newer", "older"]);
    }

    #[test]
    fn timestamps_use_configured_zone() {
        assert_eq!(format_ts(NOW, parse_tz("Europe/Helsinki")), "[Mon Oct 26 17:00]");
        assert_eq!(format_ts(NOW, parse_tz("Not/AZone")), "[Mon Oct 26 15:00]");
    }

    #[test]
    fn snapshot() {
        let lines = [
            Line::new(NOW, Level::Info, "uptime", "career up 8y 4m").boot(),
            Line::new(NOW - 900, Level::Info, "hwmon", "helsinki 11.8°C, wind 9 m/s, daylight 9h 12m ↓").boot(),
            Line::new(NOW - 120, Level::Ok, "spotify", "▶ \"Rock & Roll <Live>\" — Led Zeppelin"),
            Line::new(NOW - 3600, Level::Info, "git", "push yudjinn/github_pulse"),
            Line::new(NOW - 7200, Level::Stale, "git", "active 東京事変 ".repeat(12)),
            Line::warn(NOW - 60, "github", "fetch failed: timeout"),
        ];
        insta::assert_snapshot!(render(&lines, &opts()));
    }
}
