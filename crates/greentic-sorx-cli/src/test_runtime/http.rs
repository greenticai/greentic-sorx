//! URL parsing and HTTP response decoding for the Sorx manager card probe.

use serde_json::Value;

#[derive(Debug, Clone)]
pub(super) struct ParsedHttpUrl {
    pub(super) base: String,
    pub(super) host: String,
    pub(super) port: u16,
}

impl ParsedHttpUrl {
    pub(super) fn parse(input: &str, label: &str) -> Result<Self, String> {
        let base = input.trim().trim_end_matches('/').to_string();
        let rest = base
            .strip_prefix("http://")
            .ok_or_else(|| format!("{label} must look like http://host:port"))?;
        let host_port = rest
            .split('/')
            .next()
            .ok_or_else(|| format!("{label} must look like http://host:port"))?;
        let (host, port) = host_port
            .rsplit_once(':')
            .ok_or_else(|| format!("{label} must include a port"))?;
        let host = host.to_string();
        let port = port
            .parse::<u16>()
            .map_err(|_| format!("{label} has an invalid port"))?;
        if host.trim().is_empty() {
            return Err(format!("{label} must include a host"));
        }
        Ok(Self { base, host, port })
    }

    pub(super) fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

pub(super) fn parse_http_json_response(bytes: &[u8]) -> Result<Value, String> {
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "HTTP response did not include headers".to_string())?;
    let headers = String::from_utf8_lossy(&bytes[..split]);
    let status_line = headers
        .lines()
        .next()
        .ok_or_else(|| "HTTP response was empty".to_string())?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| format!("invalid HTTP status line: {status_line}"))?;
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status}: {status_line}"));
    }
    serde_json::from_slice(&bytes[split + 4..])
        .map_err(|err| format!("HTTP response body is invalid JSON: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response(status_line: &str, body: &str) -> Vec<u8> {
        format!("{status_line}\r\nContent-Type: application/json\r\n\r\n{body}").into_bytes()
    }

    #[test]
    fn parse_accepts_a_plain_host_and_port() {
        let url = ParsedHttpUrl::parse("http://127.0.0.1:8788", "--sorx-url").expect("parses");
        assert_eq!(url.base, "http://127.0.0.1:8788");
        assert_eq!(url.host, "127.0.0.1");
        assert_eq!(url.port, 8788);
        assert_eq!(url.bind_addr(), "127.0.0.1:8788");
    }

    #[test]
    fn parse_trims_whitespace_and_a_trailing_slash() {
        let url = ParsedHttpUrl::parse("  http://localhost:9000/  ", "--sorx-url").expect("parses");
        assert_eq!(url.base, "http://localhost:9000");
        assert_eq!(url.bind_addr(), "localhost:9000");
    }

    #[test]
    fn parse_ignores_a_path_when_deriving_host_and_port() {
        let url =
            ParsedHttpUrl::parse("http://localhost:9000/v1/sorx", "--sorx-url").expect("parses");
        assert_eq!(url.host, "localhost");
        assert_eq!(url.port, 9000);
        // `base` keeps the path; only host/port are extracted from the authority.
        assert_eq!(url.base, "http://localhost:9000/v1/sorx");
    }

    #[test]
    fn parse_rejects_a_non_http_scheme() {
        let err = ParsedHttpUrl::parse("https://localhost:9000", "--sorx-url").unwrap_err();
        assert_eq!(err, "--sorx-url must look like http://host:port");
    }

    #[test]
    fn parse_rejects_a_missing_port() {
        let err = ParsedHttpUrl::parse("http://localhost", "--webchat-url").unwrap_err();
        assert_eq!(err, "--webchat-url must include a port");
    }

    #[test]
    fn parse_rejects_a_non_numeric_or_out_of_range_port() {
        assert_eq!(
            ParsedHttpUrl::parse("http://localhost:abc", "--sorx-url").unwrap_err(),
            "--sorx-url has an invalid port"
        );
        assert_eq!(
            ParsedHttpUrl::parse("http://localhost:70000", "--sorx-url").unwrap_err(),
            "--sorx-url has an invalid port"
        );
    }

    #[test]
    fn parse_rejects_an_empty_host() {
        let err = ParsedHttpUrl::parse("http://:9000", "--sorx-url").unwrap_err();
        assert_eq!(err, "--sorx-url must include a host");
    }

    #[test]
    fn parse_takes_the_last_colon_so_ipv6_style_hosts_keep_their_port() {
        let url = ParsedHttpUrl::parse("http://[::1]:8080", "--sorx-url").expect("parses");
        assert_eq!(url.host, "[::1]");
        assert_eq!(url.port, 8080);
    }

    #[test]
    fn parse_http_json_response_decodes_a_200_body() {
        let bytes = response("HTTP/1.1 200 OK", r#"{"type":"AdaptiveCard"}"#);
        let value = parse_http_json_response(&bytes).expect("decodes");
        assert_eq!(value, json!({"type": "AdaptiveCard"}));
    }

    #[test]
    fn parse_http_json_response_accepts_the_whole_2xx_range() {
        let bytes = response("HTTP/1.1 299 Weird", "[]");
        assert_eq!(
            parse_http_json_response(&bytes).expect("decodes"),
            json!([])
        );
    }

    #[test]
    fn parse_http_json_response_surfaces_the_status_for_non_2xx() {
        let bytes = response("HTTP/1.1 404 Not Found", "{}");
        let err = parse_http_json_response(&bytes).unwrap_err();
        assert_eq!(err, "HTTP 404: HTTP/1.1 404 Not Found");
        // refresh_sorx_dashboard_card matches on this substring to skip optional cards.
        assert!(err.contains("HTTP 404"));
    }

    #[test]
    fn parse_http_json_response_rejects_a_300_status() {
        let bytes = response("HTTP/1.1 301 Moved", "{}");
        assert!(
            parse_http_json_response(&bytes)
                .unwrap_err()
                .starts_with("HTTP 301")
        );
    }

    #[test]
    fn parse_http_json_response_requires_a_header_terminator() {
        let err = parse_http_json_response(b"HTTP/1.1 200 OK").unwrap_err();
        assert_eq!(err, "HTTP response did not include headers");
    }

    #[test]
    fn parse_http_json_response_rejects_a_response_with_no_status_line() {
        let err = parse_http_json_response(b"\r\n\r\n{}").unwrap_err();
        assert_eq!(err, "HTTP response was empty");
    }

    #[test]
    fn parse_http_json_response_rejects_a_malformed_status_line() {
        let bytes = response("GARBAGE", "{}");
        assert_eq!(
            parse_http_json_response(&bytes).unwrap_err(),
            "invalid HTTP status line: GARBAGE"
        );
    }

    #[test]
    fn parse_http_json_response_rejects_an_invalid_json_body() {
        let bytes = response("HTTP/1.1 200 OK", "not json");
        assert!(
            parse_http_json_response(&bytes)
                .unwrap_err()
                .starts_with("HTTP response body is invalid JSON:")
        );
    }
}
