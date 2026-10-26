mod cache;
mod fetch;
mod spotify;

use pulse_core::{
    Line,
    config::{KV_BINDING, secret, var},
    render::{self, RenderOptions},
    sources::{github, uptime, weather},
};
use worker::*;

const KEY_WEATHER: &str = "src:weather";
const KEY_GITHUB: &str = "src:github";

/// Non-secret settings from `[vars]`, with fallbacks so a missing var degrades
/// to a WARN line instead of an error.
struct Vars {
    github_user: String,
    career_start: String,
    location_name: String,
    latitude: Option<f64>,
    longitude: Option<f64>,
    timezone: String,
}

impl Vars {
    fn from_env(env: &Env) -> Self {
        let get = |name| env.var(name).map(|v| v.to_string()).unwrap_or_default();
        Self {
            github_user: get(var::GITHUB_USER),
            career_start: get(var::CAREER_START),
            location_name: get(var::LOCATION_NAME),
            latitude: get(var::LATITUDE).parse().ok(),
            longitude: get(var::LONGITUDE).parse().ok(),
            timezone: get(var::TIMEZONE),
        }
    }
}

fn now() -> i64 {
    (Date::now().as_millis() / 1000) as i64
}

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    if req.method() != Method::Get && req.method() != Method::Head {
        return Response::error("method not allowed", 405);
    }
    match req.path().as_str() {
        "/dmesg.svg" => svg_response(&dmesg_svg(&env).await),
        "/health" => Response::ok("ok"),
        _ => Response::error("not found", 404),
    }
}

#[event(scheduled)]
async fn scheduled(_event: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    let vars = Vars::from_env(&env);
    let now = now();
    let kv = match env.kv(KV_BINDING) {
        Ok(kv) => kv,
        Err(e) => return console_error!("cron: no KV binding {KV_BINDING}: {e}"),
    };
    let (weather, github) = futures_util::join!(
        refresh_weather(&vars, now),
        refresh_github(&env, &vars),
    );
    futures_util::join!(
        store(&kv, KEY_WEATHER, weather.map(|l| vec![l]), now),
        store(&kv, KEY_GITHUB, github, now),
    );
}

/// On failure, keep the last good value; it dims once stale.
async fn store(kv: &kv::KvStore, key: &str, lines: Result<Vec<Line>>, now: i64) {
    match lines {
        Ok(lines) => match cache::store(kv, key, lines, now).await {
            Ok(wrote) => console_log!("cron: {key} ok, kv write: {wrote}"),
            Err(e) => console_error!("cron: {key} kv write failed: {e}"),
        },
        Err(e) => console_error!("cron: {key} failed: {e}"),
    }
}

async fn refresh_weather(vars: &Vars, now: i64) -> Result<Line> {
    let (Some(lat), Some(lon)) = (vars.latitude, vars.longitude) else {
        return Err(Error::RustError("LATITUDE/LONGITUDE not set".into()));
    };
    let body = fetch::get_text(&weather::url(lat, lon, &vars.timezone), &[]).await?;
    weather::parse(&body, &vars.location_name, now).map_err(|e| Error::RustError(e.to_string()))
}

async fn refresh_github(env: &Env, vars: &Vars) -> Result<Vec<Line>> {
    let user = &vars.github_user;
    if user.is_empty() {
        return Err(Error::RustError("GITHUB_USER not set".into()));
    }
    // Unauthenticated works too, at 60 requests/hour; the cron needs 12.
    let auth = env
        .secret(secret::GITHUB_TOKEN)
        .map(|t| format!("Bearer {t}"))
        .ok();
    let mut headers = vec![
        ("Accept", "application/vnd.github+json"),
        ("X-GitHub-Api-Version", "2022-11-28"),
    ];
    if let Some(auth) = &auth {
        headers.push(("Authorization", auth));
    }
    let (events_url, repos_url) = (github::events_url(user), github::repos_url(user));
    let (events, repos) = futures_util::join!(
        fetch::get_text(&events_url, &headers),
        fetch::get_text(&repos_url, &headers),
    );
    github::parse(&events?, &repos?, user).map_err(|e| Error::RustError(e.to_string()))
}

/// Never fails: every problem becomes a WARN line so the image never breaks.
async fn dmesg_svg(env: &Env) -> String {
    let vars = Vars::from_env(env);
    let now = now();
    let tz = render::parse_tz(&vars.timezone);

    let mut lines = vec![uptime::line(&vars.career_start, now, tz)];
    let kv = env.kv(KV_BINDING).ok();
    match &kv {
        Some(kv) => {
            let (weather, github, spotify) = futures_util::join!(
                load_or_warn(kv, KEY_WEATHER, weather::FACILITY, now, true),
                load_or_warn(kv, KEY_GITHUB, github::FACILITY, now, false),
                spotify::line(env, Some(kv), now),
            );
            lines.extend(weather);
            lines.extend(github);
            lines.push(spotify);
        }
        None => {
            lines.push(Line::warn(now, "pulse", "KV not bound, no cached sources"));
            lines.push(spotify::line(env, None, now).await);
        }
    }

    let login = if vars.github_user.is_empty() { "pulse" } else { &vars.github_user };
    render::render(&lines, &RenderOptions { tz, login })
}

async fn load_or_warn(
    kv: &kv::KvStore,
    key: &str,
    facility: &str,
    now: i64,
    boot: bool,
) -> Vec<Line> {
    let warn = |msg| {
        let line = Line::warn(now, facility, msg);
        vec![if boot { line.boot() } else { line }]
    };
    match cache::load(kv, key, now).await {
        Ok(Some(lines)) => lines,
        Ok(None) => warn("no data yet"),
        Err(e) => {
            console_error!("kv read {key}: {e}");
            warn("cache unavailable")
        }
    }
}

fn svg_response(svg: &str) -> Result<Response> {
    let headers = Headers::new();
    headers.set("Content-Type", "image/svg+xml; charset=utf-8")?;
    headers.set(
        "Cache-Control",
        "no-cache, no-store, max-age=0, must-revalidate",
    )?;
    headers.set("ETag", &etag(svg))?;
    Ok(Response::ok(svg)?.with_headers(headers))
}

/// Strong ETag from a 64-bit FNV-1a hash of the body.
fn etag(body: &str) -> String {
    let hash = body.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    format!("\"{hash:016x}\"")
}
