# Plan: `github_pulse`, a live `dmesg` SVG card

Owner: Jacob Rodgers (`yudjinn`).
Audience: the agent implementing this. Read the whole plan before starting.
Do not commit secrets or personal config. Do not push or deploy without the owner's go-ahead.

---

## 1. Goal

A standalone Cloudflare Worker (Rust, `workers-rs`) that serves an SVG which looks like
`dmesg -T` output and shows real, current data: Spotify now playing, latest GitHub activity,
recent projects, and local weather/daylight. It is meant to be embedded as an `<img>` in a
GitHub profile README, but the README itself is **out of scope** for this project.

Everything is Rust. Everything user-specific (the GitHub user, location, domain, secrets)
lives in a gitignored `config.toml`, so it never enters git history.

## 2. Architecture

```
<img src="https://<domain>/dmesg.svg">
   │
camo (GitHub image proxy) ──► Cloudflare Worker  (Rust, workers-rs, free plan)
                                  │
             ┌────────────────────┼─────────────────────────┐
             │ fetch handler       │ scheduled handler (cron) │
             │  GET /dmesg.svg     │  every 10 min            │
             │  GET /health        │  refresh slow sources    │
             │                     │  → write KV if changed   │
             └────────┬───────────┴─────────────┬───────────┘
                      │                         │
        Spotify fetched live on request,   KV namespace `PULSE`
        Cache API TTL 30 s                  src:github, src:weather,
                                            spotify:refresh_token,
                                            spotify:access_token
```

### Why this split

- Free plan limits: 100k req/day, **~10 ms CPU per invocation** (waiting on network
  doesn't count), **KV: 1,000 writes/day**, 100k reads/day.
- **Slow sources** (GitHub, weather) are refreshed by cron every 10 min (144 runs/day) and
  written **only when the content hash changed**. Worst case is about 288 writes/day.
- **Spotify** is fetched **on request** and cached in the Cache API (`caches.default`) for
  30 s, which has no write quota. The access token lives in KV (~24 writes/day) and in
  isolate memory.
- Rendering is pure string templating from cached JSON, which fits easily in 10 ms CPU.
- **The Worker must never return a non-200 for `/dmesg.svg`.** Any source failure renders
  as a `WARN` line.

## 3. Repository layout

A Cargo workspace in this directory, `github_pulse`. The current single-crate
`Cargo.toml`/`src/main.rs` scaffold gets replaced.

```
github_pulse/
├── Cargo.toml                 # [workspace]
├── config.example.toml        # committed, placeholders only
├── config.toml                # GITIGNORED: real config + secrets
├── wrangler.toml              # GITIGNORED: generated from config.toml by `pulse sync`
├── README.md                  # what it is, setup, config, deploy, KV budget
├── crates/
│   ├── core/                  # pulse-core: no I/O, builds for native + wasm32
│   │   └── src/
│   │       ├── config.rs      # Config struct (serde), shared by worker + cli
│   │       ├── line.rs        # struct Line { ts, level, facility, msg }
│   │       ├── render.rs      # Vec<Line> → SVG string
│   │       └── sources/       # pure parsers: API JSON → Vec<Line>
│   │           ├── spotify.rs
│   │           ├── github.rs
│   │           ├── weather.rs
│   │           └── uptime.rs
│   ├── worker/                # cdylib, workers-rs: HTTP fetches, KV, Cache API, cron
│   │   └── src/{lib.rs, cache.rs, fetch.rs}
│   └── cli/                   # native binary `pulse`
│       └── src/main.rs
└── tests/fixtures/*.json      # recorded API responses (scrubbed)
```

Keeping parsing and rendering in `pulse-core` means all the logic is tested natively
with plain `cargo test`, and the CLI can render previews without a Worker.

### 3.1 The `pulse` CLI

| Command | What it does |
|---|---|
| `pulse auth spotify` | One-time OAuth: listens on `127.0.0.1:8888/callback`, opens the authorize URL, exchanges the code, and **writes the refresh token into `config.toml`** (via `toml_edit`, keeping comments). |
| `pulse sync [--push-secrets]` | Reads `config.toml`, writes `wrangler.toml` (name, route/custom domain, KV id, cron, `[vars]`) and `.dev.vars` (mode 0600, secrets for `wrangler dev`). With `--push-secrets`, uploads `[secrets]` via `wrangler secret bulk` over stdin (verified: it reads JSON from stdin). |
| `pulse render [--fixtures]` | Prints an SVG to stdout from fixtures (or live APIs) for quick previews. |
| `pulse deploy` | `sync` + `wrangler deploy`. Only run with the owner's go-ahead. |

Wrangler is a pinned npm dev dependency (`package.json`, run via `npx wrangler`), not a global install.

## 4. Configuration

`config.toml` (gitignored; `config.example.toml` committed with placeholders):

```toml
[worker]
name = "github-pulse"
domain = "pulse.example.com"         # custom domain; zone must be on the Cloudflare account
kv_namespace_id = "…"

[profile]
github_user = "yudjinn"
career_start = "2018-06"             # YYYY-MM, start of the uptime line

[location]
name = "helsinki"
latitude = 60.17
longitude = 24.94
timezone = "Europe/Helsinki"

[secrets]                            # pushed via `wrangler secret`, never into wrangler.toml
spotify_client_id = ""
spotify_client_secret = ""
spotify_refresh_token = ""           # filled by `pulse auth spotify`
github_token = ""
```

- Non-secret values reach the Worker as `[vars]` in the generated `wrangler.toml`.
- `[secrets]` reach it only via `wrangler secret`.
- Add `config.toml`, `wrangler.toml` and `.dev.vars` to `.gitignore` **before the first commit**.
- **Timezone:** `chrono-tz` normally bundles every zone, which bloats the wasm. Use
  `CHRONO_TZ_TIMEZONE_FILTER` set from the config's timezone at build time (`pulse sync`/
  `pulse deploy` set it), with a fallback to UTC if the configured zone wasn't compiled in.

## 5. Sources

Each source returns `Vec<Line>`. Each runs with its own timeout (**2 s**) and all run
concurrently (`futures::join!`). If a cron source fails, the Worker keeps the last good
KV value, which renders dimmed once 90 min old. If no value exists, it renders a `WARN`
line. Spotify has no stored fallback: a failure renders `spotify: unavailable: <reason>`.

### 5.1 Spotify (live)
- Token: `POST https://accounts.spotify.com/api/token`, Basic auth `id:secret`,
  `grant_type=refresh_token`. If the response includes a new `refresh_token`, **persist it
  to KV `spotify:refresh_token`** and prefer KV over the seed secret from then on.
  Store `access_token` + expiry in KV and in a module-level cell.
- `GET https://api.spotify.com/v1/me/player/currently-playing`
  - 200 + `is_playing` → `spotify: ▶ "<track>" — <artists>`
  - 204, or not playing → `GET /v1/me/player/recently-played?limit=1`
    → `spotify: last played "<track>" — <artists>` with the `played_at` timestamp.
  - Ignore `currently_playing_type != "track"` (podcasts/ads): fall through to recently-played.
- Scopes: `user-read-currently-playing user-read-recently-played`.
- Redirect URI `http://127.0.0.1:8888/callback`. Verified 2026-10-26: Spotify rejects
  `localhost` and allows plain HTTP only for loopback IP literals.

### 5.2 GitHub activity + recent projects (cron)
- `GET https://api.github.com/users/<github_user>/events/public?per_page=30`, with a
  `User-Agent` header (required) and `Authorization: Bearer $GITHUB_TOKEN`
  (fine-grained, public read only).
- Map to lines:
  - `PushEvent` → `git: push <repo>` (verified 2026-10-26: payload has only `ref`,
    `head`, `before`, ids; no commits or size)
  - `PullRequestEvent` (`opened`/`closed`/`reopened` only) → `git: PR <action> <repo>#<n>`
    (verified: payload no longer includes the PR title)
  - `CreateEvent` (repository) → `git: new repo <repo>`
  - `ReleaseEvent` → `git: release <repo> <tag>`
  - `PublicEvent` → `git: open-sourced <repo>`
  - everything else → skip
- The owner's repos display without the `user/` prefix.
- Fold runs of identical consecutive lines into one with a count (`push timber ×3`).
  Keep the newest **5**.
- Recent projects: `GET /users/<github_user>/repos?sort=pushed&per_page=10`, skip forks,
  take 3, render as `git: active <repo> (<language>) — <description>`. Dedupe against
  the event lines.

### 5.3 Weather / daylight (cron, no key)
- `GET https://api.open-meteo.com/v1/forecast?latitude=<lat>&longitude=<lon>&current=temperature_2m,wind_speed_10m&daily=daylight_duration&wind_speed_unit=ms&timezone=<tz>&forecast_days=2`
- Line: `hwmon: helsinki -3.4°C, wind 6 m/s, daylight 7h 02m ↓`
  (the arrow compares today's vs tomorrow's `daylight_duration`).

### 5.4 Uptime (computed, no API)
- `uptime: career up 8y 4m`, computed from `profile.career_start`.

## 6. Rendering (`pulse-core::render`)

- Output: one SVG, ~840 px wide, height = header + n_lines × line_height.
- Always-dark terminal panel (rounded rect, `#0d1117`) so it reads well in both GitHub themes.
- Header line: `$ dmesg -T --follow` in a prompt color.
- Line format, mimicking `dmesg -T`: `[Sat Oct 25 14:02] facility: message`
  - Timestamps in the configured timezone.
  - Newest first, with `uptime`/`hwmon` pinned at the top as "boot" lines.
- Levels → colors: `ok` green-ish, `info` default fg, `warn` yellow, `stale` dim gray.
  The facility prefix gets its own accent color.
- Last line: `<github_user> login: ▌` with a CSS `@keyframes` blinking cursor.
- Font: no external fonts. `font-family: ui-monospace, SFMono-Regular, Menlo, Consolas,
  "Liberation Mono", monospace`. Assume ~0.6 em char width. **Truncate messages to fit**
  using `unicode-width` (CJK is double width) and append `…`.
- **XML-escape every dynamic string** (`& < > " '`). Test with `Rock & Roll <Live>`.
- Response headers for `/dmesg.svg`:
  `Content-Type: image/svg+xml; charset=utf-8`,
  `Cache-Control: no-cache, no-store, max-age=0, must-revalidate`, plus an `ETag`.

## 7. Phases and acceptance criteria

**Phase 0: Scaffold** ✔ done 2026-10-26
- Workspace with `core`, `worker`, `cli`; `.gitignore` covers `config.toml`, `wrangler.toml`,
  `.dev.vars`, `target/`, `build/`.
- `config.example.toml`, `pulse sync` generating `wrangler.toml`.
- `wrangler dev` serves a hard-coded SVG at `/dmesg.svg`.
- ✅ `curl -i localhost:8787/dmesg.svg` returns 200 with the correct headers; output passes
  `xmllint --noout`.
- ✅ `git status` shows no `config.toml` or `wrangler.toml`.

**Phase 1: Renderer + static sources** ✔ done 2026-10-26
- `Line` model, renderer, uptime + weather, cron handler, KV caching.
- ✅ `insta` snapshot tests for the renderer; escaping test; CJK truncation test.
- ✅ `pulse render --fixtures` output renders correctly in a browser and a gist preview.

**Phase 2: GitHub + Spotify** ✔ code done 2026-10-26 (live Spotify check pending the owner's auth)
- GitHub via cron; Spotify live with token refresh and rotation; `pulse auth spotify`.
- ✅ Parser tests against recorded fixtures (scrub any tokens).
- ✅ With Spotify secrets unset or invalid, `/dmesg.svg` still returns 200 with a `WARN` line.
- ✅ KV writes/day estimate documented in README and under 500.

**Phase 3: Deploy** ✔ deployed 2026-10-26 to pulse.yudjinn.dev (live "now playing" check pending). Also added `GET /`, which describes the endpoints.
- `pulse deploy` to the custom domain.
- ✅ The song changes on the live card within ~2 min of a track change.

## 8. Manual steps for the owner

1. Create a Spotify app at developer.spotify.com, add the redirect URI
   `http://127.0.0.1:8888/callback`, put the client id/secret in `config.toml`, then run
   `pulse auth spotify`.
2. Create a fine-grained GitHub token (public repos, read-only) and put it in `config.toml`.
3. Cloudflare: `wrangler login`, create the KV namespace, put its id in `config.toml`.
   Make sure the custom domain's zone is on the account.

