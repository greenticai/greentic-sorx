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
    use std::sync::Mutex;

    /// `from_env()` reads process-global env vars, so the gate tests must not
    /// run concurrently. This guard serializes them.
    static ENV_GUARD: Mutex<()> = Mutex::new(());

    const ENV_KEYS: [&str; 5] = [
        "SORX_MCP_ENABLED",
        "SORX_MCP_ISSUERS",
        "SORX_MCP_AUDIENCE",
        "SORX_MCP_JWKS_TTL_SECS",
        "SORX_MCP_LEEWAY_SECS",
    ];

    /// Snapshot the gate env vars, clear them, run `body`, then restore the
    /// snapshot — so a developer's ambient env cannot leak into the assertions
    /// and the tests cannot leak into each other.
    fn with_clean_env<T>(set: &[(&str, &str)], body: impl FnOnce() -> T) -> T {
        let _lock = ENV_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let saved: Vec<(&str, Option<String>)> = ENV_KEYS
            .iter()
            .map(|&k| (k, std::env::var(k).ok()))
            .collect();
        // SAFETY: all env mutation happens under `ENV_GUARD`, so no other test
        // observes a partially-updated environment, and the snapshot is restored
        // before the lock is released.
        unsafe {
            for &k in &ENV_KEYS {
                std::env::remove_var(k);
            }
            for &(k, v) in set {
                std::env::set_var(k, v);
            }
        }
        let result = body();
        unsafe {
            for (k, v) in saved {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
        result
    }

    #[test]
    fn from_env_none_when_disabled_by_default() {
        let cfg = with_clean_env(&[], McpAuthConfig::from_env);
        assert!(
            cfg.is_none(),
            "MCP must be off when SORX_MCP_ENABLED is absent"
        );
    }

    #[test]
    fn from_env_none_when_explicitly_false() {
        let cfg = with_clean_env(
            &[
                ("SORX_MCP_ENABLED", "false"),
                ("SORX_MCP_ISSUERS", "https://tm.example"),
                ("SORX_MCP_AUDIENCE", "https://sor.example"),
            ],
            McpAuthConfig::from_env,
        );
        assert!(cfg.is_none());
    }

    #[test]
    fn from_env_none_when_issuers_missing() {
        let cfg = with_clean_env(
            &[
                ("SORX_MCP_ENABLED", "1"),
                ("SORX_MCP_AUDIENCE", "https://sor.example"),
            ],
            McpAuthConfig::from_env,
        );
        assert!(cfg.is_none());
    }

    #[test]
    fn from_env_none_when_issuers_empty_after_filter() {
        let cfg = with_clean_env(
            &[
                ("SORX_MCP_ENABLED", "1"),
                ("SORX_MCP_ISSUERS", " , , "),
                ("SORX_MCP_AUDIENCE", "https://sor.example"),
            ],
            McpAuthConfig::from_env,
        );
        assert!(
            cfg.is_none(),
            "a comma-only issuer list must collapse to empty and gate off"
        );
    }

    #[test]
    fn from_env_none_when_audience_empty() {
        let cfg = with_clean_env(
            &[
                ("SORX_MCP_ENABLED", "1"),
                ("SORX_MCP_ISSUERS", "https://tm.example"),
                ("SORX_MCP_AUDIENCE", ""),
            ],
            McpAuthConfig::from_env,
        );
        assert!(cfg.is_none());
    }

    #[test]
    fn from_env_some_with_full_config_and_defaults() {
        let cfg = with_clean_env(
            &[
                ("SORX_MCP_ENABLED", "true"),
                (
                    "SORX_MCP_ISSUERS",
                    "https://tm.example, https://idp.example ",
                ),
                ("SORX_MCP_AUDIENCE", "https://sor.example/acme/landlord"),
            ],
            McpAuthConfig::from_env,
        )
        .expect("full valid config must enable MCP");
        assert_eq!(
            cfg.issuers,
            vec![
                "https://tm.example".to_string(),
                "https://idp.example".to_string(),
            ],
            "issuers are split, trimmed, and order-preserving"
        );
        assert_eq!(cfg.audience, "https://sor.example/acme/landlord");
        assert_eq!(cfg.jwks_ttl, Duration::from_secs(600), "default TTL");
        assert_eq!(cfg.leeway_secs, 60, "default leeway");
    }

    #[test]
    fn from_env_some_with_custom_ttl_and_leeway() {
        let cfg = with_clean_env(
            &[
                ("SORX_MCP_ENABLED", "yes"),
                ("SORX_MCP_ISSUERS", "https://tm.example"),
                ("SORX_MCP_AUDIENCE", "https://sor.example"),
                ("SORX_MCP_JWKS_TTL_SECS", "120"),
                ("SORX_MCP_LEEWAY_SECS", "5"),
            ],
            McpAuthConfig::from_env,
        )
        .expect("config present");
        assert_eq!(cfg.jwks_ttl, Duration::from_secs(120));
        assert_eq!(cfg.leeway_secs, 5);
    }

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
        h.insert(
            "authorization".to_string(),
            "Bearer abc.def.ghi".to_string(),
        );
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
