use std::time::Duration;

use futures_util::future::{Either, select};
use worker::{AbortController, Delay, Error, Fetch, Headers, Method, Request, RequestInit, Result};

const USER_AGENT: &str = concat!("github_pulse/", env!("CARGO_PKG_VERSION"));

/// Every upstream call gets this long, including reading the body.
pub const TIMEOUT: Duration = Duration::from_secs(2);

/// GET `url` and return the body, failing on non-2xx or after [`TIMEOUT`].
pub async fn get_text(url: &str, headers: &[(&str, &str)]) -> Result<String> {
    let h = Headers::new();
    h.set("User-Agent", USER_AGENT)?;
    h.set("Accept", "application/json")?;
    for (k, v) in headers {
        h.set(k, v)?;
    }
    let mut init = RequestInit::new();
    init.with_method(Method::Get).with_headers(h);
    let req = Request::new_with_init(url, &init)?;

    let controller = AbortController::default();
    let signal = controller.signal();
    let fetch = async {
        let mut resp = Fetch::Request(req).send_with_signal(&signal).await?;
        let status = resp.status_code();
        if !(200..300).contains(&status) {
            return Err(Error::RustError(format!("HTTP {status}")));
        }
        resp.text().await
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
