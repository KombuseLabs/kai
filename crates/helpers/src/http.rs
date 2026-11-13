//! HTTP clients. Every crate that talks HTTP starts its client here, so that
//! all of kai's requests share one TLS setup and one rule for plain HTTP.
//!
//! TLS is rustls with ring as its crypto provider, rather than reqwest's
//! default aws-lc, which is a C build and brings more licences. The
//! system's certificates are trusted.

use std::error::Error;

/// A client builder with kai's TLS setup and `user_agent`. The caller adds
/// its own timeouts and redirect policy.
pub fn builder(user_agent: &str) -> reqwest::ClientBuilder {
    // reqwest is built without a crypto provider of its own; ring is
    // installed for the process. One installed before stays, hence the
    // ignored result.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder().user_agent(user_agent)
}

/// Whether kai may send requests to `url`: `https://`, or plain `http://`
/// to this machine only (`127.0.0.1`, `localhost`, `[::1]`, with an
/// optional port), which is what tests serve from.
///
/// ```
/// use helpers::http::allowed_url;
///
/// assert!(allowed_url("https://api.github.com"));
/// assert!(allowed_url("http://127.0.0.1:8080/graphql"));
/// assert!(!allowed_url("http://example.com"));
/// assert!(!allowed_url("http://127.0.0.1.example.com"));
/// ```
pub fn allowed_url(url: &str) -> bool {
    url.starts_with("https://") || is_loopback_http(url)
}

/// Whether `url` is plain HTTP to this machine: a loopback host, an
/// optional port, and no user part that could hide another host.
fn is_loopback_http(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or("");
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if port.bytes().all(|b| b.is_ascii_digit()) => host,
        _ => authority,
    };
    !authority.contains('@') && matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

/// `err` and every error that caused it, as one line: reqwest's own message
/// alone often says only "error sending request".
pub fn error_chain(err: &dyn Error) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
