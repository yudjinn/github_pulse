//! Live Spotify line: fetched on request, cached for 30 s in the Cache API (no write
//! quota). Access tokens live in isolate memory and KV; a rotated refresh token is
//! persisted to KV and preferred over the seed secret from then on.

use std::cell::RefCell;

use pulse_core::{
    Line,
    config::secret,
    sources::spotify::{self, FACILITY},
};
use serde::{Deserialize, Serialize};
use worker::{
    Cache, Env, Error, Headers, Method, Response, Result, console_error, kv::KvStore,
    url::form_urlencoded,
};

use crate::fetch;

const KEY_ACCESS: &str = "spotify:access_token";
const KEY_REFRESH: &str = "spotify:refresh_token";
/// Cache API key; never fetched, only used to address the cached line.
const CACHE_URL: &str = "https://github-pulse.internal/spotify-line";
const CACHE_SECS: u32 = 30;
/// Refresh this long before Spotify's stated expiry.
const EXPIRY_MARGIN: i64 = 60;

#[derive(Clone, Serialize, Deserialize)]
struct AccessToken {
    token: String,
    expires_at: i64,
}

thread_local! {
    static ACCESS: RefCell<Option<AccessToken>> = const { RefCell::new(None) };
}

/// The Spotify line, or a WARN line. Never fails.
pub async fn line(env: &Env, kv: Option<&KvStore>, now: i64) -> Line {
    let cache = Cache::default();
    if let Ok(Some(mut resp)) = cache.get(CACHE_URL, false).await
        && let Ok(line) = resp.json::<Line>().await
    {
        return line;
    }

    let line = match fetch_line(env, kv, now).await {
        Ok(line) => line,
        Err(e) => {
            console_error!("spotify: {e}");
            Line::warn(now, FACILITY, warn_msg(&e))
        }
    };
    // Cache failures too, so a broken token doesn't hit Spotify on every request.
    if let Err(e) = put_cached(&cache, &line).await {
        console_error!("spotify: cache put failed: {e}");
    }
    line
}

fn warn_msg(e: &Error) -> String {
    match e {
        Error::RustError(msg) => format!("unavailable: {msg}"),
        _ => "unavailable".into(),
    }
}

async fn put_cached(cache: &Cache, line: &Line) -> Result<()> {
    let headers = Headers::new();
    headers.set("Cache-Control", &format!("max-age={CACHE_SECS}"))?;
    let resp = Response::from_json(line)?.with_headers(headers);
    cache.put(CACHE_URL, resp).await
}

async fn fetch_line(env: &Env, kv: Option<&KvStore>, now: i64) -> Result<Line> {
    let token = access_token(env, kv, now).await?;
    let auth = format!("Bearer {token}");
    let headers = [("Authorization", auth.as_str())];

    let (status, body) =
        fetch::request(Method::Get, spotify::CURRENTLY_PLAYING_URL, &headers, None).await?;
    let body = match status {
        200 => body,
        204 => String::new(),
        401 => {
            forget_access_token(kv).await;
            return Err(Error::RustError("token rejected".into()));
        }
        s => return Err(Error::RustError(format!("HTTP {s}"))),
    };
    if let Some(line) = spotify::parse_currently_playing(&body, now).map_err(rust_err)? {
        return Ok(line);
    }

    let body = fetch::get_text(spotify::RECENTLY_PLAYED_URL, &headers).await?;
    spotify::parse_recently_played(&body)
        .map_err(rust_err)?
        .ok_or_else(|| Error::RustError("nothing played yet".into()))
}

async fn access_token(env: &Env, kv: Option<&KvStore>, now: i64) -> Result<String> {
    let fresh = |t: &AccessToken| t.expires_at - EXPIRY_MARGIN > now;

    if let Some(t) = ACCESS.with_borrow(|a| a.clone()).filter(fresh) {
        return Ok(t.token);
    }
    if let Some(kv) = kv
        && let Ok(Some(t)) = kv.get(KEY_ACCESS).json::<AccessToken>().await
        && fresh(&t)
    {
        ACCESS.set(Some(t.clone()));
        return Ok(t.token);
    }
    refresh(env, kv, now).await
}

async fn refresh(env: &Env, kv: Option<&KvStore>, now: i64) -> Result<String> {
    let secret = |name| {
        env.secret(name)
            .map(|s| s.to_string())
            .ok()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::RustError("not configured".into()))
    };
    let client_id = secret(secret::SPOTIFY_CLIENT_ID)?;
    let client_secret = secret(secret::SPOTIFY_CLIENT_SECRET)?;

    let stored = match kv {
        Some(kv) => kv.get(KEY_REFRESH).text().await.ok().flatten(),
        None => None,
    };
    let refresh_token = match stored {
        Some(t) => t,
        None => secret(secret::SPOTIFY_REFRESH_TOKEN)?,
    };

    let body = form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "refresh_token")
        .append_pair("refresh_token", &refresh_token)
        .finish();
    let auth = spotify::basic_auth(&client_id, &client_secret);
    let (status, resp) = fetch::request(
        Method::Post,
        spotify::TOKEN_URL,
        &[
            ("Authorization", &auth),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ],
        Some(&body),
    )
    .await?;
    if status != 200 {
        return Err(Error::RustError(format!("token refresh HTTP {status}")));
    }
    let token = spotify::parse_token(&resp).map_err(rust_err)?;

    let access = AccessToken {
        token: token.access_token,
        expires_at: now + token.expires_in,
    };
    ACCESS.set(Some(access.clone()));

    if let Some(kv) = kv {
        if let Some(rotated) = token.refresh_token.filter(|t| *t != refresh_token)
            && let Err(e) = put(kv, KEY_REFRESH, rotated, None).await
        {
            // Losing a rotated token can lock us out until `pulse auth spotify` is rerun.
            console_error!("spotify: failed to persist rotated refresh token: {e}");
        }
        let ttl = (token.expires_in - EXPIRY_MARGIN).max(60) as u64;
        if let Err(e) = put(kv, KEY_ACCESS, serde_json::to_string(&access)?, Some(ttl)).await {
            console_error!("spotify: failed to cache access token: {e}");
        }
    }
    Ok(access.token)
}

async fn put(kv: &KvStore, key: &str, value: String, ttl: Option<u64>) -> Result<()> {
    let mut put = kv.put(key, value)?;
    if let Some(ttl) = ttl {
        put = put.expiration_ttl(ttl);
    }
    Ok(put.execute().await?)
}

async fn forget_access_token(kv: Option<&KvStore>) {
    ACCESS.set(None);
    if let Some(kv) = kv
        && let Err(e) = kv.delete(KEY_ACCESS).await
    {
        console_error!("spotify: failed to drop rejected token: {e}");
    }
}

fn rust_err(e: impl std::fmt::Display) -> Error {
    Error::RustError(e.to_string())
}
