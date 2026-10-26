//! KV storage for cron-refreshed sources, written only when content changes to stay
//! well under the free plan's 1,000 writes/day.

use pulse_core::Line;
use serde::{Deserialize, Serialize};
use worker::kv::KvStore;

/// Rewrite unchanged content this often so `fetched_at` proves the source is alive.
const REFRESH_SECS: i64 = 60 * 60;
/// Older than this (no successful refresh) and lines render dimmed.
const STALE_SECS: i64 = 90 * 60;

#[derive(Serialize, Deserialize)]
struct Stored {
    fetched_at: i64,
    lines: Vec<Line>,
}

/// Content identity, ignoring timestamps, so a re-fetch of the same data is a no-op.
fn same_content(a: &[Line], b: &[Line]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.level == y.level && x.facility == y.facility && x.msg == y.msg)
}

/// Store `lines` under `key` if they changed or the stored copy is due for a refresh.
/// Unchanged lines keep their original timestamps, i.e. when the value last changed.
/// Returns whether a KV write happened.
pub async fn store(kv: &KvStore, key: &str, lines: Vec<Line>, now: i64) -> worker::Result<bool> {
    let existing: Option<Stored> = kv.get(key).json().await.ok().flatten();
    let lines = match existing {
        Some(old) if same_content(&old.lines, &lines) => {
            if now - old.fetched_at < REFRESH_SECS {
                return Ok(false);
            }
            old.lines
        }
        _ => lines,
    };
    let value = serde_json::to_string(&Stored {
        fetched_at: now,
        lines,
    })?;
    kv.put(key, value)?.execute().await?;
    Ok(true)
}

/// Lines stored under `key`, dimmed if stale. `None` if nothing was ever stored.
pub async fn load(kv: &KvStore, key: &str, now: i64) -> worker::Result<Option<Vec<Line>>> {
    let Some(stored) = kv.get(key).json::<Stored>().await? else {
        return Ok(None);
    };
    let stale = now - stored.fetched_at > STALE_SECS;
    Ok(Some(
        stored
            .lines
            .into_iter()
            .map(|l| if stale { l.stale() } else { l })
            .collect(),
    ))
}
