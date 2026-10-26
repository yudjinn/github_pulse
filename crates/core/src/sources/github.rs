//! `git:` lines from the public events feed and the most recently pushed repos.
//!
//! GitHub trims event payloads: `PushEvent` has no commits/size and `PullRequestEvent`
//! has no title, so lines only use fields that are still present.

use chrono::DateTime;
use serde::Deserialize;

use super::Error;
use crate::line::{Level, Line};

pub const FACILITY: &str = "git";
const MAX_EVENTS: usize = 5;
const MAX_REPOS: usize = 3;

pub fn events_url(user: &str) -> String {
    format!("https://api.github.com/users/{user}/events/public?per_page=30")
}

pub fn repos_url(user: &str) -> String {
    format!("https://api.github.com/users/{user}/repos?sort=pushed&per_page=10")
}

#[derive(Deserialize)]
struct Event {
    #[serde(rename = "type")]
    kind: String,
    repo: EventRepo,
    created_at: String,
    #[serde(default)]
    payload: serde_json::Value,
}

#[derive(Deserialize)]
struct EventRepo {
    name: String,
}

#[derive(Deserialize)]
struct Repo {
    full_name: String,
    fork: bool,
    language: Option<String>,
    description: Option<String>,
    pushed_at: String,
}

/// Up to five lines for recent events, then up to three "active" repos that the
/// event lines don't already mention.
pub fn parse(events_json: &str, repos_json: &str, user: &str) -> Result<Vec<Line>, Error> {
    let events: Vec<Event> = serde_json::from_str(events_json)?;
    let repos: Vec<Repo> = serde_json::from_str(repos_json)?;

    let mut lines: Vec<(String, Line)> = Vec::new();
    let mut repeats = 1;
    for event in &events {
        let Some((before, after)) = event_msg(event) else {
            continue;
        };
        let line = repo_line(
            timestamp(&event.created_at)?,
            &event.repo.name,
            user,
            (&before, &after),
        );
        // The feed is newest first; fold runs of identical lines into one with a count.
        // Appending the count keeps the link's byte range valid.
        if let Some((_, last)) = lines.last_mut()
            && strip_count(&last.msg) == line.msg
        {
            repeats += 1;
            last.msg = format!("{} ×{repeats}", line.msg);
            continue;
        }
        if lines.len() == MAX_EVENTS {
            break;
        }
        repeats = 1;
        lines.push((event.repo.name.clone(), line));
    }

    let active = repos
        .iter()
        .filter(|r| !r.fork && !lines.iter().any(|(name, _)| *name == r.full_name))
        .take(MAX_REPOS)
        .map(|r| {
            let mut after = String::new();
            if let Some(lang) = &r.language {
                after.push_str(&format!(" ({lang})"));
            }
            if let Some(desc) = r.description.as_deref().filter(|d| !d.is_empty()) {
                after.push_str(&format!(" — {desc}"));
            }
            Ok(repo_line(
                timestamp(&r.pushed_at)?,
                &r.full_name,
                user,
                ("active ", &after),
            ))
        })
        .collect::<Result<Vec<_>, Error>>()?;

    Ok(lines.into_iter().map(|(_, l)| l).chain(active).collect())
}

/// `before <repo> after`, with the repo name linking to the repo on GitHub.
fn repo_line(ts: i64, full_name: &str, user: &str, (before, after): (&str, &str)) -> Line {
    Line::linked(
        ts,
        Level::Info,
        FACILITY,
        (before, display_repo(full_name, user), after),
        format!("https://github.com/{full_name}"),
    )
}

/// The text around the repo name for events worth showing.
fn event_msg(event: &Event) -> Option<(String, String)> {
    let p = &event.payload;
    let (before, after) = match event.kind.as_str() {
        "PushEvent" => ("push ".into(), String::new()),
        "PullRequestEvent" => {
            let action = p["action"].as_str()?;
            if !matches!(action, "opened" | "closed" | "reopened") {
                return None;
            }
            (format!("PR {action} "), format!("#{}", p["number"].as_u64()?))
        }
        "CreateEvent" if p["ref_type"] == "repository" => ("new repo ".into(), String::new()),
        "ReleaseEvent" => (
            "release ".into(),
            p["release"]["tag_name"]
                .as_str()
                .map(|tag| format!(" {tag}"))
                .unwrap_or_default(),
        ),
        "PublicEvent" => ("open-sourced ".into(), String::new()),
        _ => return None,
    };
    Some((before, after))
}

/// The owner's own repos without the `user/` prefix; everyone else's in full.
fn display_repo<'a>(full_name: &'a str, user: &str) -> &'a str {
    full_name
        .strip_prefix(user)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(full_name)
}

fn strip_count(msg: &str) -> &str {
    msg.rsplit_once(" ×").map_or(msg, |(base, _)| base)
}

fn timestamp(rfc3339: &str) -> Result<i64, Error> {
    DateTime::parse_from_rfc3339(rfc3339)
        .map(|dt| dt.timestamp())
        .map_err(|e| Error(format!("bad timestamp {rfc3339:?}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVENTS: &str = include_str!("../../../../tests/fixtures/github-events.json");
    const REPOS: &str = include_str!("../../../../tests/fixtures/github-repos.json");

    fn msgs(lines: &[Line]) -> Vec<&str> {
        lines.iter().map(|l| l.msg.as_str()).collect()
    }

    #[test]
    fn parses_recorded_fixtures() {
        let lines = parse(EVENTS, REPOS, "yudjinn").unwrap();
        // The CreateEvent in the fixture is a branch, not a repository, so it's skipped;
        // timber appears as an event, so it's deduped out of the active list.
        assert_eq!(
            msgs(&lines),
            [
                "open-sourced timber",
                "active citizenID (Rust) — OAuth Provider service intended for Star Citizen third party services. Lightweight to run in AWS Lambda with DynamoDB backing.",
                "active fom-simple-combat (Game Maker Language) — Simplified Combat for Fields of Mistria",
                "active heimdall (Go) — Realtime IoT streaming engine for retrofitting analog factory machines for monitoring and OEE metrics.",
            ]
        );
    }

    #[test]
    fn folds_repeats_and_skips_noise() {
        let events = r#"[
          {"type":"PushEvent","repo":{"name":"yudjinn/a"},"created_at":"2026-10-26T12:00:00Z","payload":{}},
          {"type":"PushEvent","repo":{"name":"yudjinn/a"},"created_at":"2026-10-26T11:00:00Z","payload":{}},
          {"type":"PushEvent","repo":{"name":"yudjinn/a"},"created_at":"2026-10-26T10:00:00Z","payload":{}},
          {"type":"WatchEvent","repo":{"name":"rust-lang/rust"},"created_at":"2026-10-26T09:00:00Z","payload":{"action":"started"}},
          {"type":"PullRequestEvent","repo":{"name":"helix-editor/helix"},"created_at":"2026-10-26T08:00:00Z","payload":{"action":"labeled","number":1}},
          {"type":"PullRequestEvent","repo":{"name":"helix-editor/helix"},"created_at":"2026-10-26T07:00:00Z","payload":{"action":"opened","number":42}},
          {"type":"ReleaseEvent","repo":{"name":"yudjinn/a"},"created_at":"2026-10-26T06:00:00Z","payload":{"release":{"tag_name":"v1.0.0"}}}
        ]"#;
        let lines = parse(events, "[]", "yudjinn").unwrap();
        assert_eq!(
            msgs(&lines),
            ["push a ×3", "PR opened helix-editor/helix#42", "release a v1.0.0"]
        );
        assert_eq!(lines[0].ts, 1_793_016_000); // newest of the folded run
    }

    #[test]
    fn repo_names_link_to_github() {
        let lines = parse(EVENTS, REPOS, "yudjinn").unwrap();
        let link = lines[0].link.as_ref().unwrap();
        assert_eq!(&lines[0].msg[link.start..link.start + link.len], "timber");
        assert_eq!(link.href, "https://github.com/yudjinn/timber");

        let pr = r#"[{"type":"PullRequestEvent","repo":{"name":"helix-editor/helix"},"created_at":"2026-10-26T07:00:00Z","payload":{"action":"opened","number":42}}]"#;
        let line = &parse(pr, "[]", "yudjinn").unwrap()[0];
        let link = line.link.as_ref().unwrap();
        assert_eq!(&line.msg[link.start..link.start + link.len], "helix-editor/helix");
        assert_eq!(link.href, "https://github.com/helix-editor/helix");
    }

    #[test]
    fn caps_event_lines() {
        let one = |i: u32| {
            format!(
                r#"{{"type":"PushEvent","repo":{{"name":"yudjinn/r{i}"}},"created_at":"2026-10-26T10:00:00Z","payload":{{}}}}"#
            )
        };
        let events = format!("[{}]", (0..9).map(one).collect::<Vec<_>>().join(","));
        assert_eq!(parse(&events, "[]", "yudjinn").unwrap().len(), MAX_EVENTS);
    }
}
