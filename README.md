# github_pulse

A Cloudflare Worker, written in Rust with `workers-rs`, that serves a live SVG styled
like `dmesg -T --follow`. It shows what you're listening to on Spotify, recent GitHub
activity, local weather and daylight, and a career "uptime". It's meant to be embedded in
a GitHub profile README:

```html
<img src="https://pulse.example.com/dmesg.svg" alt="Live dmesg feed" width="100%" />
```

`/dmesg.svg` always returns 200. If a source fails, its line becomes a yellow WARN line,
or its last good value is shown dimmed.

## Endpoints

| Route | Returns |
|---|---|
| `GET /dmesg.svg` | `image/svg+xml`: the card. Always 200. |
| `GET /health` | `text/plain`: `ok`. |
| `GET /` | `text/plain`: a description of these endpoints and an embed snippet. |

## Layout

| Crate | What it is |
|---|---|
| `crates/core` (`pulse-core`) | Config, line model, SVG renderer and API parsers. No I/O; tested natively. |
| `crates/worker` (`pulse-worker`) | The Worker: HTTP, KV, Cache API and the cron. |
| `crates/cli` (`pulse`) | Local tooling: config sync, Spotify auth and previews. |

## Setup

1. Run `npm install`. This installs a pinned `wrangler` locally.
2. Run `cp config.example.toml config.toml && chmod 600 config.toml`, then fill it in.
   `config.toml` is gitignored and holds both settings and secrets.
3. Run `npx wrangler login`, then `npx wrangler kv namespace create PULSE`. Put the
   namespace id in `worker.kv_namespace_id`.
4. Create a Spotify app at developer.spotify.com with the redirect URI
   `http://127.0.0.1:8888/callback`. Put its id and secret in `[secrets]`, then run:

   ```sh
   cargo run -p pulse-cli -- auth spotify
   ```

5. Create a fine-grained GitHub token (public repositories, read-only) and put it in
   `secrets.github_token`.
6. Run `cargo run -p pulse-cli -- sync`. It generates `wrangler.toml` and `.dev.vars`, both
   gitignored.

## Develop

```sh
cargo test --workspace
cargo run -p pulse-cli -- render > preview.svg          # preview from fixtures
npx wrangler dev --test-scheduled                       # http://localhost:8787/dmesg.svg
curl "localhost:8787/__scheduled?cron=*/10+*+*+*+*"     # run the cron once
```

Renderer snapshots use `insta`. After an intended change, run
`INSTA_UPDATE=always cargo test -p pulse-core` and review the diff.

## Deploy

```sh
cargo run -p pulse-cli -- sync --push-secrets   # uploads [secrets] via `wrangler secret bulk`
npx wrangler deploy
```

If `worker.domain` is set, the Worker is served on that custom domain. The domain's zone
must be on your Cloudflare account.

## How data flows

| Source | Refreshed | Stored |
|---|---|---|
| Weather (Open-Meteo, no key) | cron, every 10 min | KV `src:weather` |
| GitHub events + recent repos | cron, every 10 min | KV `src:github` |
| Spotify now playing / last played | on request | Cache API, 30 s |
| Uptime | computed per request | none |

The cron writes a key only when its content changed, or once an hour so an unchanged
value can prove it's still fresh. A value that hasn't refreshed in 90 minutes renders
dimmed.

On `*.workers.dev`, the Cache API is a no-op, so every request fetches Spotify. A custom
domain gets the 30-second cache.

### KV write budget (free plan: 1,000/day)

| Key | Writes/day |
|---|---|
| `src:weather` | ≤ 96 changes (Open-Meteo updates every 15 min) + ≤ 24 hourly rewrites; at most 144 |
| `src:github` | changes plus ≤ 24 hourly rewrites; usually ~25, at most 144 |
| `spotify:access_token` | ~24 (one-hour tokens), a few more if isolates race |
| `spotify:refresh_token` | only when Spotify rotates it; rare |
| **Total** | about 150 typical, about 320 worst case |
