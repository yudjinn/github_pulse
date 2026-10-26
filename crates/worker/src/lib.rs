use worker::*;

/// Phase 0 placeholder until the renderer lands in `pulse-core`.
const PLACEHOLDER_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="840" height="80" viewBox="0 0 840 80">
  <rect width="840" height="80" rx="8" fill="#0d1117"/>
  <text x="16" y="32" fill="#7ee787" font-family="ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', monospace" font-size="14">$ dmesg -T --follow</text>
  <text x="16" y="56" fill="#c9d1d9" font-family="ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', monospace" font-size="14">[    0.000000] pulse: booting…</text>
</svg>
"##;

#[event(fetch)]
async fn fetch(req: Request, _env: Env, _ctx: Context) -> Result<Response> {
    if req.method() != Method::Get && req.method() != Method::Head {
        return Response::error("method not allowed", 405);
    }
    match req.path().as_str() {
        "/dmesg.svg" => svg_response(PLACEHOLDER_SVG),
        "/health" => Response::ok("ok"),
        _ => Response::error("not found", 404),
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
