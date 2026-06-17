//! OAuth Resource-Server pieces for the MCP transport: config from env,
//! Protected Resource Metadata (RFC 9728), bearer extraction, the
//! `WWW-Authenticate` challenge, and JWT/JWKS verification (Task 6).

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use jsonwebtoken::jwk::{AlgorithmParameters, JwkSet};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
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

// ---------------------------------------------------------------------------
// JWT / JWKS verification (Task 6)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawClaims {
    iss: String,
    sub: String,
    tenant_id: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    roles: Vec<String>,
}

/// Verified, projected JWT claims ready for use by the MCP handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedClaims {
    pub tenant_id: String,
    pub sub: String,
    pub email: Option<String>,
    pub roles: Vec<String>,
}

/// Abstraction over JWKS retrieval — allows both the production HTTP client
/// and a static in-process fixture to be used interchangeably.
pub trait JwksSource {
    fn jwks(&self, issuer: &str) -> Result<JwkSet, String>;
}

/// Validate `token` against the issuer allow-list and audience in `cfg`.
///
/// Flow:
/// 1. Decode the header to extract `kid`.
/// 2. Peek claims with signature validation disabled (read-only; issuer check).
/// 3. Reject any issuer not in `cfg.issuers`.
/// 4. Fetch the JWKS for the issuer, find the JWK matching `kid`.
/// 5. Full RS256 decode with audience + leeway.
/// 6. Project to `VerifiedClaims`; reject empty `tenant_id` or `sub`.
#[allow(dead_code)]
pub fn verify_token(
    token: &str,
    cfg: &McpAuthConfig,
    jwks: &dyn JwksSource,
) -> Result<VerifiedClaims, String> {
    let header = decode_header(token).map_err(|e| format!("bad token header: {e}"))?;
    let kid = header.kid.ok_or_else(|| "token missing kid".to_string())?;

    // Peek the (unverified) issuer to pick the JWKS + enforce the allow-list.
    let unverified: RawClaims = {
        let mut v = Validation::new(Algorithm::RS256);
        v.insecure_disable_signature_validation();
        v.validate_aud = false;
        v.validate_exp = false;
        decode::<RawClaims>(token, &DecodingKey::from_secret(b"x"), &v)
            .map_err(|e| format!("undecodable claims: {e}"))?
            .claims
    };
    if !cfg.issuers.iter().any(|i| i == &unverified.iss) {
        return Err(format!("untrusted issuer {}", unverified.iss));
    }

    let set = jwks.jwks(&unverified.iss)?;
    let jwk = set
        .keys
        .iter()
        .find(|k| k.common.key_id.as_deref() == Some(kid.as_str()))
        .ok_or_else(|| format!("no JWK for kid {kid}"))?;
    let decoding = match &jwk.algorithm {
        AlgorithmParameters::RSA(rsa) => DecodingKey::from_rsa_components(&rsa.n, &rsa.e)
            .map_err(|e| format!("bad RSA jwk: {e}"))?,
        _ => return Err("unsupported JWK type".to_string()),
    };
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_required_spec_claims(&["exp", "aud"]);
    validation.set_audience(&[cfg.audience.clone()]);
    validation.leeway = cfg.leeway_secs;
    let data = decode::<RawClaims>(token, &decoding, &validation)
        .map_err(|e| format!("jwt verification failed: {e}"))?;
    let c = data.claims;
    if c.tenant_id.is_empty() || c.sub.is_empty() {
        return Err("token missing tenant_id/sub".to_string());
    }
    Ok(VerifiedClaims {
        tenant_id: c.tenant_id,
        sub: c.sub,
        email: c.email.filter(|e| !e.is_empty()),
        roles: c.roles,
    })
}

/// Production JWKS client with TTL-based in-memory cache.
///
/// Fetches `{issuer}/jwks.json` via `ureq` (blocking) and caches the result
/// for `ttl`. Used by the MCP HTTP handler (Task 7); not exercised by unit
/// tests, which use `StaticJwks` instead.
#[allow(dead_code)]
pub struct UreqJwks {
    ttl: Duration,
    cache: Mutex<Option<(String, Instant, JwkSet)>>,
}

#[allow(dead_code)]
impl UreqJwks {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            cache: Mutex::new(None),
        }
    }
}

impl JwksSource for UreqJwks {
    fn jwks(&self, issuer: &str) -> Result<JwkSet, String> {
        let url = format!("{}/jwks.json", issuer.trim_end_matches('/'));
        {
            let guard = self
                .cache
                .lock()
                .map_err(|_| "jwks cache poisoned".to_string())?;
            if let Some((u, at, set)) = guard.as_ref() {
                if u == &url && at.elapsed() < self.ttl {
                    return Ok(set.clone());
                }
            }
        }
        let set: JwkSet = ureq::get(&url)
            .call()
            .map_err(|e| format!("jwks fetch {url}: {e}"))?
            .into_json()
            .map_err(|e| format!("jwks decode {url}: {e}"))?;
        if let Ok(mut guard) = self.cache.lock() {
            *guard = Some((url, Instant::now(), set.clone()));
        }
        Ok(set)
    }
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

    // ---- Task 6: JWT/JWKS verification tests --------------------------------

    use jsonwebtoken::{EncodingKey, Header, encode, jwk::JwkSet};

    const PRIV_PEM: &str = include_str!("../tests/fixtures/mcp/rsa_test_private.pem");
    const JWKS: &str = include_str!("../tests/fixtures/mcp/rsa_test_jwks.json");

    struct StaticJwks;
    impl JwksSource for StaticJwks {
        fn jwks(&self, _issuer: &str) -> Result<JwkSet, String> {
            serde_json::from_str(JWKS).map_err(|e| e.to_string())
        }
    }

    fn jwt_cfg() -> McpAuthConfig {
        McpAuthConfig {
            issuers: vec!["https://tm.example".into()],
            audience: "sorx-acme-landlord".into(),
            jwks_ttl: std::time::Duration::from_secs(600),
            leeway_secs: 60,
        }
    }

    fn sign(claims: serde_json::Value) -> String {
        let mut header = Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some("test-key".into());
        encode(
            &header,
            &claims,
            &EncodingKey::from_rsa_pem(PRIV_PEM.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn verify_token_accepts_valid_tm_jwt_and_projects_claims() {
        let now = 4_102_444_800i64; // far future
        let token = sign(serde_json::json!({
            "iss": "https://tm.example", "sub": "u1", "aud": "sorx-acme-landlord",
            "tenant_id": "acme", "email": "a@acme.io", "roles": ["sorla_composer"],
            "exp": now
        }));
        let claims = verify_token(&token, &jwt_cfg(), &StaticJwks).unwrap();
        assert_eq!(claims.tenant_id, "acme");
        assert_eq!(claims.sub, "u1");
        assert_eq!(claims.roles, vec!["sorla_composer".to_string()]);
    }

    #[test]
    fn verify_token_rejects_untrusted_issuer() {
        let token = sign(serde_json::json!({
            "iss": "https://evil.example", "sub": "u1", "aud": "sorx-acme-landlord",
            "tenant_id": "acme", "exp": 4_102_444_800i64
        }));
        assert!(verify_token(&token, &jwt_cfg(), &StaticJwks).is_err());
    }

    #[test]
    fn verify_token_rejects_wrong_audience() {
        let token = sign(serde_json::json!({
            "iss": "https://tm.example", "sub": "u1", "aud": "someone-else",
            "tenant_id": "acme", "exp": 4_102_444_800i64
        }));
        assert!(verify_token(&token, &jwt_cfg(), &StaticJwks).is_err());
    }

    #[test]
    fn verify_token_rejects_tampered_signature() {
        let token = sign(serde_json::json!({
            "iss": "https://tm.example", "sub": "u1", "aud": "sorx-acme-landlord",
            "tenant_id": "acme", "exp": 4_102_444_800i64
        }));
        // Split into header.payload.signature and corrupt the signature segment.
        let parts: Vec<&str> = token.splitn(3, '.').collect();
        assert_eq!(parts.len(), 3, "JWT must have three dot-separated parts");
        let mut sig = parts[2].to_string();
        // Flip the last character to a different base64url character.
        let last = sig.pop().unwrap_or('A');
        sig.push(if last == 'A' { 'B' } else { 'A' });
        let tampered = format!("{}.{}.{}", parts[0], parts[1], sig);
        assert!(
            verify_token(&tampered, &jwt_cfg(), &StaticJwks).is_err(),
            "a token with a corrupted signature must be rejected"
        );
    }

    #[test]
    fn verify_token_rejects_expired_token() {
        // exp = 1_000_000_000 is 2001-09-09, well in the past and outside the 60s leeway.
        let token = sign(serde_json::json!({
            "iss": "https://tm.example", "sub": "u1", "aud": "sorx-acme-landlord",
            "tenant_id": "acme", "exp": 1_000_000_000i64
        }));
        assert!(
            verify_token(&token, &jwt_cfg(), &StaticJwks).is_err(),
            "an expired token must be rejected"
        );
    }

    #[test]
    fn verify_token_rejects_missing_audience() {
        // No `aud` field — after Fix 1 (`set_required_spec_claims`) this must fail.
        let token = sign(serde_json::json!({
            "iss": "https://tm.example", "sub": "u1",
            "tenant_id": "acme", "exp": 4_102_444_800i64
        }));
        assert!(
            verify_token(&token, &jwt_cfg(), &StaticJwks).is_err(),
            "a token without an aud claim must be rejected"
        );
    }
}
