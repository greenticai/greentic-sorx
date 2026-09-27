//! An OAuth access token for pulling a pack from Google Artifact Registry,
//! read from the GCP metadata server for the attached service account.
//!
//! Ported from greentic-start `src/bundle_ref.rs` (`artifact_registry_token`),
//! so a Cloud Run sorx service pulls its pack keylessly exactly the way a
//! Cloud Run worker pulls its bundle. The endpoint and timeout are
//! PARAMETERS of [`artifact_registry_token_from`] so tests can point it at a
//! loopback mock; there is deliberately no environment override, because
//! anything able to set the environment could then redirect a request that
//! returns a credential.

use std::time::Duration;

/// Host suffix identifying a Google Artifact Registry reference.
pub(crate) const ARTIFACT_REGISTRY_SUFFIX: &str = "-docker.pkg.dev";

/// The GCP metadata server's token endpoint for the attached service account.
pub(crate) const METADATA_TOKEN_URL: &str =
    "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token";

/// Upper bound on the TCP connect and reads of the metadata request. Off GCP
/// the name does not resolve and this is never reached; it matters for a
/// link-local route that accepts and then stalls, where boot must continue
/// anonymously. Note: this bound does not cover DNS resolution of
/// `metadata.google.internal`, which on GCP-less hosts may take several seconds.
pub(crate) const METADATA_TIMEOUT: Duration = Duration::from_secs(5);

/// The basic-auth username Artifact Registry expects alongside an OAuth
/// access token as the password.
pub(crate) const AR_USERNAME: &str = "oauth2accesstoken";

/// Whether `host` (a registry host as `pack_ref::reference_host` resolves it)
/// is a Google Artifact Registry Docker host. Matching is case-sensitive; a
/// `host:port` does not match (the port suffix is not stripped). When this
/// returns false, the pull falls back to anonymous HTTPS — both the match
/// failure and the absence of an accessible metadata server fail closed.
pub(crate) fn is_artifact_registry_host(host: &str) -> bool {
    host.ends_with(ARTIFACT_REGISTRY_SUFFIX)
}

/// An access token for pulling from `host`, or `None` when that does not
/// apply: `host` is not Artifact Registry, or the metadata server does not
/// answer usefully — the normal case off GCP. `None`, never an error, so every
/// existing pull keeps working unchanged and an AR pull without a token still
/// fails at the registry with the registry's own message.
pub(crate) fn artifact_registry_token(host: &str) -> Option<String> {
    artifact_registry_token_from(host, METADATA_TOKEN_URL, METADATA_TIMEOUT)
}

/// [`artifact_registry_token`] with the endpoint and timeout supplied, so the
/// behaviour is testable against a loopback server.
///
/// The token never leaves this function except as its return value: nothing
/// here logs, and the error paths discard the response.
pub(crate) fn artifact_registry_token_from(
    host: &str,
    token_url: &str,
    timeout: Duration,
) -> Option<String> {
    if !is_artifact_registry_host(host) {
        return None;
    }
    // No redirects: the `Metadata-Flavor` header must reach the metadata
    // server and nothing it might point at.
    let agent = ureq::AgentBuilder::new()
        .timeout(timeout)
        .redirects(0)
        .build();
    let response = agent
        .get(token_url)
        .set("Metadata-Flavor", "Google")
        .call()
        .ok()?;
    if response.status() != 200 {
        return None;
    }
    let body: serde_json::Value = response.into_json().ok()?;
    let token = body.get("access_token")?.as_str()?.trim();
    (!token.is_empty()).then(|| token.to_string())
}

#[cfg(test)]
mod tests {
    use super::mock_metadata::{hang, refused, serve_once};
    use super::*;
    use std::time::Instant;

    const AR_HOST: &str = "europe-west1-docker.pkg.dev";

    #[test]
    fn host_matching_accepts_only_artifact_registry_hosts() {
        assert!(is_artifact_registry_host("europe-west1-docker.pkg.dev"));
        assert!(is_artifact_registry_host("us-docker.pkg.dev"));
        assert!(!is_artifact_registry_host("docker.pkg.dev"));
        assert!(!is_artifact_registry_host("pkg.dev"));
        assert!(!is_artifact_registry_host("ghcr.io"));
        assert!(!is_artifact_registry_host("index.docker.io"));
        assert!(!is_artifact_registry_host("x-docker.pkg.dev.evil.example"));
        // A port is not stripped — mirrors greentic-start, which compares the
        // raw leading segment.
        assert!(!is_artifact_registry_host("us-docker.pkg.dev:443"));
        assert!(!is_artifact_registry_host(""));
    }

    #[test]
    fn a_token_is_fetched_with_the_metadata_flavor_header() {
        let server = serve_once(
            200,
            r#"{"access_token":"ya29.test-token","expires_in":3599,"token_type":"Bearer"}"#,
        );
        let token = artifact_registry_token_from(AR_HOST, &server.url, Duration::from_secs(5));
        assert_eq!(token.as_deref(), Some("ya29.test-token"));
        let seen = server
            .seen
            .recv_timeout(Duration::from_secs(2))
            .expect("request seen");
        assert!(
            seen.request_line
                .starts_with("GET /computeMetadata/v1/instance/service-accounts/default/token "),
            "{}",
            seen.request_line
        );
        assert!(
            seen.headers.iter().any(|h| h == "metadata-flavor: google"),
            "{:?}",
            seen.headers
        );
    }

    #[test]
    fn a_non_artifact_registry_host_never_contacts_the_metadata_server() {
        let server = serve_once(200, r#"{"access_token":"ya29.must-not-leak"}"#);
        assert_eq!(
            artifact_registry_token_from("ghcr.io", &server.url, Duration::from_secs(5)),
            None
        );
        assert!(
            server
                .seen
                .recv_timeout(Duration::from_millis(300))
                .is_err(),
            "the metadata server must not be asked for a non-AR host"
        );
    }

    #[test]
    fn an_unreachable_metadata_server_yields_no_token() {
        assert_eq!(
            artifact_registry_token_from(AR_HOST, &refused(), Duration::from_secs(5)),
            None
        );
    }

    #[test]
    fn a_non_success_status_yields_no_token() {
        let server = serve_once(403, r#"{"error":"forbidden"}"#);
        assert_eq!(
            artifact_registry_token_from(AR_HOST, &server.url, Duration::from_secs(5)),
            None
        );
    }

    #[test]
    fn a_malformed_body_yields_no_token() {
        let server = serve_once(200, "not json");
        assert_eq!(
            artifact_registry_token_from(AR_HOST, &server.url, Duration::from_secs(5)),
            None
        );
    }

    #[test]
    fn a_missing_or_empty_access_token_yields_no_token() {
        let missing = serve_once(200, r#"{"expires_in":3599}"#);
        assert_eq!(
            artifact_registry_token_from(AR_HOST, &missing.url, Duration::from_secs(5)),
            None
        );
        let empty = serve_once(200, r#"{"access_token":"   "}"#);
        assert_eq!(
            artifact_registry_token_from(AR_HOST, &empty.url, Duration::from_secs(5)),
            None
        );
    }

    #[test]
    fn a_hanging_metadata_server_is_bounded_by_the_timeout() {
        let url = hang();
        let started = Instant::now();
        assert_eq!(
            artifact_registry_token_from(AR_HOST, &url, Duration::from_millis(300)),
            None
        );
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "took {:?}; the timeout must bound the whole request",
            started.elapsed()
        );
    }

    #[test]
    fn the_production_constants_match_greentic_start() {
        assert_eq!(METADATA_TIMEOUT, Duration::from_secs(5));
        assert_eq!(ARTIFACT_REGISTRY_SUFFIX, "-docker.pkg.dev");
        assert_eq!(AR_USERNAME, "oauth2accesstoken");
        assert_eq!(
            METADATA_TOKEN_URL,
            "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token"
        );
    }
}

/// A one-shot HTTP/1.1 server on 127.0.0.1 standing in for the GCP metadata
/// server. Test-only: nothing here is compiled into the binary.
#[cfg(test)]
pub(crate) mod mock_metadata {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    /// One request the mock received: the request line and its header lines,
    /// lowercased and trimmed.
    pub(crate) struct Seen {
        pub(crate) request_line: String,
        pub(crate) headers: Vec<String>,
    }

    pub(crate) struct MockServer {
        pub(crate) url: String,
        pub(crate) seen: mpsc::Receiver<Seen>,
    }

    const TOKEN_PATH: &str = "/computeMetadata/v1/instance/service-accounts/default/token";

    /// Answer exactly one connection with `status` and a JSON `body`.
    pub(crate) fn serve_once(status: u16, body: &'static str) -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
        let addr = listener.local_addr().expect("mock addr");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut request_line = String::new();
            let _ = reader.read_line(&mut request_line);
            let mut headers = Vec::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                let line = line.trim_end().to_ascii_lowercase();
                if line.is_empty() {
                    break;
                }
                headers.push(line);
            }
            let _ = tx.send(Seen {
                request_line: request_line.trim_end().to_string(),
                headers,
            });
            let reason = if status == 200 { "OK" } else { "Error" };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let mut stream = stream;
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        });
        MockServer {
            url: format!("http://{addr}{TOKEN_PATH}"),
            seen: rx,
        }
    }

    /// Accept one connection and never answer it.
    pub(crate) fn hang() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
        let addr = listener.local_addr().expect("mock addr");
        thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                thread::sleep(Duration::from_secs(10));
                drop(stream);
            }
        });
        format!("http://{addr}{TOKEN_PATH}")
    }

    /// A loopback URL nothing listens on: the connection is refused.
    pub(crate) fn refused() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
        let addr = listener.local_addr().expect("mock addr");
        drop(listener);
        format!("http://{addr}{TOKEN_PATH}")
    }
}
