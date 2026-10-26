//! `pulse auth spotify`: one-time authorization-code flow that writes the refresh
//! token into config.toml.

use std::{
    fs,
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail, ensure};
use pulse_core::{Config, sources::spotify};
use url::Url;

/// Spotify rejects `localhost`; the loopback IP must be registered exactly like this.
const REDIRECT_URI: &str = "http://127.0.0.1:8888/callback";
const LISTEN_ADDR: &str = "127.0.0.1:8888";

pub fn spotify(config: &Config, config_path: &Path) -> Result<()> {
    let s = &config.secrets;
    ensure!(
        !s.spotify_client_id.is_empty() && !s.spotify_client_secret.is_empty(),
        "set spotify_client_id and spotify_client_secret in [secrets] first"
    );

    let state = random_state()?;
    let authorize = Url::parse_with_params(
        spotify::AUTHORIZE_URL,
        [
            ("response_type", "code"),
            ("client_id", s.spotify_client_id.as_str()),
            ("scope", spotify::SCOPES),
            ("redirect_uri", REDIRECT_URI),
            ("state", state.as_str()),
        ],
    )?;

    // Bind before opening the browser so the redirect can't race us.
    let listener = TcpListener::bind(LISTEN_ADDR)
        .with_context(|| format!("listening on {LISTEN_ADDR} (is something else using it?)"))?;
    println!("Make sure {REDIRECT_URI} is a redirect URI on your Spotify app.");
    println!("Opening the authorization page. If no browser opens, visit:\n\n  {authorize}\n");
    open_browser(authorize.as_str());

    let code = wait_for_code(&listener, &state)?;
    let refresh_token = exchange(config, &code)?;
    save_refresh_token(config_path, &refresh_token)?;

    println!(
        "Saved spotify_refresh_token to {}. Run `pulse sync` to update .dev.vars.",
        config_path.display()
    );
    Ok(())
}

fn random_state() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("getrandom: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn open_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let _ = Command::new(opener)
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Serve requests until the `/callback` redirect arrives; ignore anything else
/// (e.g. the browser asking for `/favicon.ico`).
fn wait_for_code(listener: &TcpListener, state: &str) -> Result<String> {
    for stream in listener.incoming() {
        let mut stream = stream?;
        let Some(target) = request_target(&stream)? else {
            continue;
        };
        let url = Url::parse("http://127.0.0.1")?.join(&target)?;
        if url.path() != "/callback" {
            respond(&mut stream, "404 Not Found", "Not found.")?;
            continue;
        }
        let param = |name| {
            url.query_pairs()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.into_owned())
        };

        if param("state").as_deref() != Some(state) {
            respond(&mut stream, "400 Bad Request", "State mismatch. Rerun `pulse auth spotify`.")?;
            bail!("state mismatch in callback; not using this code");
        }
        if let Some(err) = param("error") {
            respond(&mut stream, "400 Bad Request", "Authorization was denied.")?;
            bail!("Spotify returned error: {err}");
        }
        let Some(code) = param("code") else {
            respond(&mut stream, "400 Bad Request", "No code in callback.")?;
            bail!("callback had no code");
        };
        respond(&mut stream, "200 OK", "github_pulse is authorized. You can close this tab.")?;
        return Ok(code);
    }
    bail!("listener closed before the callback arrived")
}

/// The path and query from `GET <target> HTTP/1.1`.
fn request_target(stream: &TcpStream) -> Result<Option<String>> {
    let mut first = String::new();
    BufReader::new(stream).read_line(&mut first)?;
    let mut parts = first.split_whitespace();
    Ok(match (parts.next(), parts.next()) {
        (Some("GET"), Some(target)) => Some(target.to_string()),
        _ => None,
    })
}

fn respond(stream: &mut TcpStream, status: &str, message: &str) -> Result<()> {
    let body = format!("<!doctype html><title>github_pulse</title><p>{message}</p>\n");
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    Ok(())
}

fn exchange(config: &Config, code: &str) -> Result<String> {
    let s = &config.secrets;
    let body = ureq::post(spotify::TOKEN_URL)
        .header(
            "Authorization",
            spotify::basic_auth(&s.spotify_client_id, &s.spotify_client_secret),
        )
        .send_form([
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT_URI),
        ])
        .context("exchanging the authorization code")?
        .body_mut()
        .read_to_string()?;
    spotify::parse_token(&body)?
        .refresh_token
        .context("Spotify's token response had no refresh_token")
}

/// Edit only `secrets.spotify_refresh_token`, keeping the rest of the file (comments,
/// ordering, permissions) intact.
fn save_refresh_token(config_path: &Path, token: &str) -> Result<()> {
    let text = fs::read_to_string(config_path)?;
    let mut doc: toml_edit::DocumentMut = text.parse()?;
    doc["secrets"]["spotify_refresh_token"] = toml_edit::value(token);
    fs::write(config_path, doc.to_string())
        .with_context(|| format!("writing {}", config_path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_token_keeps_comments_and_other_values() {
        let dir = std::env::temp_dir().join(format!("pulse-auth-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(
            &path,
            "# top comment\n[secrets]\nspotify_client_id = \"id\" # keep me\nspotify_refresh_token = \"\"\n",
        )
        .unwrap();

        save_refresh_token(&path, "new-token").unwrap();
        let out = fs::read_to_string(&path).unwrap();
        fs::remove_dir_all(&dir).unwrap();

        assert!(out.contains("# top comment"));
        assert!(out.contains("spotify_client_id = \"id\" # keep me"));
        assert!(out.contains("spotify_refresh_token = \"new-token\""));
    }
}
