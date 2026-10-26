use std::time::Duration;

use futures_util::future::{Either, select};
use worker::{
    AbortController, Delay, Error, Fetch, Headers, Method, Request, RequestInit, Result,
    wasm_bindgen::JsValue,
};

const USER_AGENT: &str = concat!("github_pulse/", env!("CARGO_PKG_VERSION"));

/// Every upstream call gets this long, including reading the body.
pub const TIMEOUT: Duration = Duration::from_secs(2);

/// Send a request and return `(status, body)` for any status, failing only on
/// network errors or after [`TIMEOUT`].
pub async fn request(
    method: Method,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> Result<(u16, String)> {
    let h = Headers::new();
    h.set("User-Agent", USER_AGENT)?;
    h.set("Accept", "application/json")?;
    for (k, v) in headers {
        h.set(k, v)?;
    }
    let mut init = RequestInit::new();
    init.with_method(method)
        .with_headers(h)
        .with_body(body.map(JsValue::from_str));
    let req = Request::new_with_init(url, &init)?;

    let controller = AbortController::default();
    let signal = controller.signal();
    let fetch = async {
        let mut resp = Fetch::Request(req).send_with_signal(&signal).await?;
        let status = resp.status_code();
        Ok((status, resp.text().await?))
    };
    let fetch = std::pin::pin!(fetch);
    let timeout = Delay::from(TIMEOUT);

    match select(fetch, timeout).await {
        Either::Left((result, _)) => result,
        Either::Right(((), _)) => {
            controller.abort();
            Err(Error::RustError("timeout".into()))
        }
    }
}

/// GET `url` and return the body, failing on non-2xx.
pub async fn get_text(url: &str, headers: &[(&str, &str)]) -> Result<String> {
    match request(Method::Get, url, headers, None).await? {
        (200..=299, body) => Ok(body),
        (status, _) => Err(Error::RustError(format!("HTTP {status}"))),
    }
}
