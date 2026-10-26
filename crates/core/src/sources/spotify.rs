//! `spotify:` line from the currently-playing or recently-played endpoints.

use chrono::DateTime;
use serde::Deserialize;

use super::Error;
use crate::line::{Level, Line};

pub const FACILITY: &str = "spotify";
pub const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
pub const AUTHORIZE_URL: &str = "https://accounts.spotify.com/authorize";
pub const CURRENTLY_PLAYING_URL: &str = "https://api.spotify.com/v1/me/player/currently-playing";
pub const RECENTLY_PLAYED_URL: &str = "https://api.spotify.com/v1/me/player/recently-played?limit=1";
pub const SCOPES: &str = "user-read-currently-playing user-read-recently-played";

/// `Authorization` header value for the token endpoint.
pub fn basic_auth(client_id: &str, client_secret: &str) -> String {
    use base64::Engine;
    let creds = base64::engine::general_purpose::STANDARD.encode(format!("{client_id}:{client_secret}"));
    format!("Basic {creds}")
}

#[derive(Debug, Deserialize)]
pub struct Token {
    pub access_token: String,
    pub expires_in: i64,
    /// Present when Spotify rotates the refresh token; persist it when it is.
    pub refresh_token: Option<String>,
}

pub fn parse_token(json: &str) -> Result<Token, Error> {
    Ok(serde_json::from_str(json)?)
}

#[derive(Deserialize)]
struct Track {
    name: String,
    artists: Vec<Artist>,
}

#[derive(Deserialize)]
struct Artist {
    name: String,
}

impl Track {
    fn describe(&self) -> String {
        let artists: Vec<_> = self.artists.iter().map(|a| a.name.as_str()).collect();
        format!("\"{}\" — {}", self.name, artists.join(", "))
    }
}

#[derive(Deserialize)]
struct CurrentlyPlaying {
    is_playing: bool,
    currently_playing_type: String,
    item: Option<Track>,
}

/// `Some(line)` if a music track is playing right now. Podcasts, ads and paused
/// playback return `None` so the caller falls through to recently-played.
/// An empty body (HTTP 204) also means nothing is playing.
pub fn parse_currently_playing(json: &str, now: i64) -> Result<Option<Line>, Error> {
    if json.trim().is_empty() {
        return Ok(None);
    }
    let cp: CurrentlyPlaying = serde_json::from_str(json)?;
    Ok(match cp.item {
        Some(track) if cp.is_playing && cp.currently_playing_type == "track" => Some(Line::new(
            now,
            Level::Ok,
            FACILITY,
            format!("▶ {}", track.describe()),
        )),
        _ => None,
    })
}

#[derive(Deserialize)]
struct RecentlyPlayed {
    items: Vec<PlayHistory>,
}

#[derive(Deserialize)]
struct PlayHistory {
    track: Track,
    played_at: String,
}

pub fn parse_recently_played(json: &str) -> Result<Option<Line>, Error> {
    let rp: RecentlyPlayed = serde_json::from_str(json)?;
    let Some(play) = rp.items.into_iter().next() else {
        return Ok(None);
    };
    let ts = DateTime::parse_from_rfc3339(&play.played_at)
        .map_err(|e| Error(format!("bad played_at: {e}")))?
        .timestamp();
    Ok(Some(Line::new(
        ts,
        Level::Info,
        FACILITY,
        format!("last played {}", play.track.describe()),
    )))
}

// Fixtures are shaped after the Spotify Web API reference, trimmed to the fields
// we read plus a few neighbours, not recorded from a live account.
#[cfg(test)]
mod tests {
    use super::*;

    const PLAYING: &str = include_str!("../../../../tests/fixtures/spotify-currently-playing.json");
    const PODCAST: &str = include_str!("../../../../tests/fixtures/spotify-currently-playing-episode.json");
    const RECENT: &str = include_str!("../../../../tests/fixtures/spotify-recently-played.json");
    const TOKEN: &str = include_str!("../../../../tests/fixtures/spotify-token.json");

    #[test]
    fn currently_playing_track() {
        let line = parse_currently_playing(PLAYING, 100).unwrap().unwrap();
        assert_eq!(line.msg, "▶ \"Rock & Roll - Remaster\" — Led Zeppelin");
        assert_eq!(line.level, Level::Ok);
        assert_eq!(line.ts, 100);
    }

    #[test]
    fn podcast_paused_and_empty_fall_through() {
        assert!(parse_currently_playing(PODCAST, 0).unwrap().is_none());
        assert!(parse_currently_playing("", 0).unwrap().is_none());
        let paused = PLAYING.replace("\"is_playing\": true", "\"is_playing\": false");
        assert!(parse_currently_playing(&paused, 0).unwrap().is_none());
    }

    #[test]
    fn recently_played() {
        let line = parse_recently_played(RECENT).unwrap().unwrap();
        assert_eq!(line.msg, "last played \"Kyoto\" — Phoebe Bridgers");
        assert_eq!(line.ts, 1_793_019_600); // 2026-10-26T13:00:00Z
        assert!(parse_recently_played(r#"{"items":[]}"#).unwrap().is_none());
    }

    #[test]
    fn basic_auth_header() {
        // RFC 7617 example credentials.
        assert_eq!(basic_auth("Aladdin", "open sesame"), "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
    }

    #[test]
    fn token_with_and_without_rotation() {
        let t = parse_token(TOKEN).unwrap();
        assert_eq!(t.expires_in, 3600);
        assert_eq!(t.refresh_token.as_deref(), Some("new-refresh-token"));
        let t = parse_token(r#"{"access_token":"a","token_type":"Bearer","expires_in":3600}"#).unwrap();
        assert!(t.refresh_token.is_none());
    }
}
