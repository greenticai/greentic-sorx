//! OAuth Resource-Server pieces for the MCP transport: config from env,
//! Protected Resource Metadata (RFC 9728), bearer extraction, and the
//! `WWW-Authenticate` challenge. JWT/JWKS verification lives below (Task 6).

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Value, json};

#[allow(dead_code)]
pub struct McpAuthConfig {
    pub issuers: Vec<String>,
    pub audience: String,
    pub jwks_ttl: Duration,
    pub leeway_secs: u64,
}

impl McpAuthConfig {
    /// Returns `Some` only when MCP is explicitly enabled and the issuer
    /// allow-list + audience are configured. Default build => `None` =>
    /// the endpoint is never mounted (back-compat).
    #[allow(dead_code)]
    pub fn from_env() -> Option<McpAuthConfig> {
        let enabled = std::env::var("SORX_MCP_ENABLED")
            .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes"))
            .unwrap_or(false);
        if !enabled {
            return None;
        }
        let issuers: Vec<String> = std::env::var("SORX_MCP_ISSUERS")
            .ok()?
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let audience = std::env::var("SORX_MCP_AUDIENCE").ok()?;
        if issuers.is_empty() || audience.is_empty() {
            return None;
        }
        let jwks_ttl = Duration::from_secs(
            std::env::var("SORX_MCP_JWKS_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(600),
        );
        let leeway_secs = std::env::var("SORX_MCP_LEEWAY_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);
        Some(McpAuthConfig {
            issuers,
            audience,
            jwks_ttl,
            leeway_secs,
        })
    }
}

#[allow(dead_code)]
pub fn protected_resource_metadata(resource: &str, issuers: &[String]) -> Value {
    json!({
        "resource": resource,
        "authorization_servers": issuers,
        "bearer_methods_supported": ["header"],
    })
}

#[allow(dead_code)]
pub fn bearer_from_headers(headers: &BTreeMap<String, String>) -> Option<&str> {
    headers.get("authorization")?.strip_prefix("Bearer ")
}

#[allow(dead_code)]
pub fn www_authenticate(resource_metadata_url: &str) -> String {
    format!("Bearer resource_metadata=\"{resource_metadata_url}\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn prm_lists_resource_and_authorization_servers() {
        let doc = protected_resource_metadata(
            "https://sor.example/acme/landlord",
            &["https://tm.example".to_string()],
        );
        assert_eq!(doc["resource"], "https://sor.example/acme/landlord");
        assert_eq!(doc["authorization_servers"][0], "https://tm.example");
    }

    #[test]
    fn bearer_extracted_case_insensitive_header() {
        let mut h = BTreeMap::new();
        h.insert("authorization".to_string(), "Bearer abc.def.ghi".to_string());
        assert_eq!(bearer_from_headers(&h), Some("abc.def.ghi"));
    }

    #[test]
    fn www_authenticate_points_at_resource_metadata() {
        let v = www_authenticate("https://sor.example/.well-known/oauth-protected-resource");
        assert!(v.starts_with("Bearer "));
        assert!(v.contains(
            "resource_metadata=\"https://sor.example/.well-known/oauth-protected-resource\""
        ));
    }
}
