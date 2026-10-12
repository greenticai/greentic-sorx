//! Admin-backed roles overlay.
//!
//! When configured, a deployment's policy roles are sourced from the admin
//! system-of-record (the authoritative store of which user holds which role)
//! rather than from the self-asserted `x-greentic-caller-role` request header.
//! This closes the trust hole where any caller could assert its own roles.
//!
//! The overlay pulls a tenant's `(user-email -> roles)` map from the admin API
//! and caches it per tenant with a TTL. Lookups are keyed by the caller's
//! authenticated email (the trusted router sets `x-greentic-caller-email`).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;

/// Resolves a tenant's `(user-email -> roles)` map from the admin
/// system-of-record. Emails are lowercased for case-insensitive matching.
pub trait RolesResolver: Send + Sync {
    /// Fetches the role assignments for every user in `tenant_slug`.
    ///
    /// On a network or parse error returns `Err(String)`; the caller decides
    /// how to fall back (e.g. serve a stale cached map).
    fn tenant_user_roles(&self, tenant_slug: &str) -> Result<HashMap<String, Vec<String>>, String>;
}

/// Shape of the admin API response:
/// `GET /api/v1/designer/tenant/me/user-roles`
/// `{ "users": [ { "user_id", "email", "display_name", "roles": [...] } ] }`.
#[derive(Debug, Deserialize)]
struct UserRolesResponse {
    #[serde(default)]
    users: Vec<UserRolesEntry>,
}

#[derive(Debug, Deserialize)]
struct UserRolesEntry {
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    roles: Vec<String>,
}

/// Parses the admin user-roles JSON body into a lowercased-email -> roles map.
///
/// Factored out so the parse logic is unit-testable without real HTTP.
fn parse_user_roles(body: &str) -> Result<HashMap<String, Vec<String>>, String> {
    let parsed: UserRolesResponse =
        serde_json::from_str(body).map_err(|err| format!("admin user-roles parse error: {err}"))?;
    let mut map = HashMap::new();
    for entry in parsed.users {
        if let Some(email) = entry.email {
            let key = email.trim().to_ascii_lowercase();
            if !key.is_empty() {
                map.insert(key, entry.roles);
            }
        }
    }
    Ok(map)
}

/// HTTP client that resolves roles from the admin system-of-record over a
/// blocking `ureq` agent (no async runtime).
pub struct AdminRolesClient {
    endpoint: String,
    service_key: String,
    agent: ureq::Agent,
}

impl AdminRolesClient {
    /// Builds a client for the admin API rooted at `endpoint`
    /// (e.g. `https://admin.example.com`). The `service_key` is the
    /// `gts_<...>` token presented as a bearer credential.
    pub fn new(endpoint: impl Into<String>, service_key: impl Into<String>) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(10))
            .build();
        Self {
            endpoint: endpoint.into(),
            service_key: service_key.into(),
            agent,
        }
    }
}

impl RolesResolver for AdminRolesClient {
    fn tenant_user_roles(&self, tenant_slug: &str) -> Result<HashMap<String, Vec<String>>, String> {
        let url = format!(
            "{}/api/v1/designer/tenant/me/user-roles",
            self.endpoint.trim_end_matches('/')
        );
        let response = self
            .agent
            .get(&url)
            .set("Authorization", &format!("Bearer {}", self.service_key))
            .set("X-Greentic-Tenant", tenant_slug)
            .set("Cache-Control", "no-store")
            .call()
            .map_err(|err| format!("admin user-roles request failed: {err}"))?;
        let body = response
            .into_string()
            .map_err(|err| format!("admin user-roles body read failed: {err}"))?;
        parse_user_roles(&body)
    }
}

/// Env var holding the admin API root URL (e.g. `https://admin.example.com`).
pub const ENDPOINT_ENV: &str = "SORX_ADMIN_ROLES_ENDPOINT";
/// Env var holding the `gts_<...>` service key.
pub const SERVICE_KEY_ENV: &str = "SORX_ADMIN_ROLES_SERVICE_KEY";
/// Env var holding the per-tenant cache TTL in seconds (default 300).
pub const TTL_SECS_ENV: &str = "SORX_ADMIN_ROLES_TTL_SECS";

/// Default per-tenant cache TTL when [`TTL_SECS_ENV`] is unset or unparsable.
const DEFAULT_TTL_SECS: u64 = 300;

/// Reads an env var, returning `None` when unset or blank after trimming.
fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// A cached tenant map together with the instant it was fetched.
type CacheEntry = (Instant, HashMap<String, Vec<String>>);

/// Per-tenant, TTL-bounded overlay over a [`RolesResolver`].
pub struct AdminRolesOverlay {
    resolver: Box<dyn RolesResolver>,
    ttl: Duration,
    cache: Mutex<HashMap<String, CacheEntry>>,
}

impl AdminRolesOverlay {
    /// Builds an overlay that refreshes each tenant map after `ttl`.
    pub fn new(resolver: Box<dyn RolesResolver>, ttl: Duration) -> Self {
        Self {
            resolver,
            ttl,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Builds an overlay backed by an [`AdminRolesClient`] from environment
    /// configuration. Returns `None` when either [`ENDPOINT_ENV`] or
    /// [`SERVICE_KEY_ENV`] is unset/blank, which preserves the legacy
    /// header-roles behavior (config is opt-in, default-OFF).
    ///
    /// [`TTL_SECS_ENV`] overrides the cache TTL (default
    /// [`DEFAULT_TTL_SECS`] seconds); an unparsable value falls back to the
    /// default rather than failing startup.
    pub fn from_env() -> Option<Self> {
        let endpoint = non_empty_env(ENDPOINT_ENV)?;
        let service_key = non_empty_env(SERVICE_KEY_ENV)?;
        let ttl_secs = std::env::var(TTL_SECS_ENV)
            .ok()
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .filter(|secs| *secs > 0)
            .unwrap_or(DEFAULT_TTL_SECS);
        let client = AdminRolesClient::new(endpoint, service_key);
        Some(Self::new(Box::new(client), Duration::from_secs(ttl_secs)))
    }

    /// Effective roles for a caller.
    ///
    /// Returns `None` when the overlay cannot resolve roles (no caller email,
    /// or the admin is unreachable and no cached map exists) so the caller can
    /// decide its own fallback. A known user with no granted roles yields
    /// `Some(vec![])`.
    ///
    /// On a resolver error a stale cached map is served (with a warning) when
    /// one is present; otherwise `None` is returned.
    pub fn roles_for(&self, tenant_slug: &str, caller_email: Option<&str>) -> Option<Vec<String>> {
        let email = caller_email?.trim().to_ascii_lowercase();
        if email.is_empty() {
            return None;
        }
        let map = self.tenant_map(tenant_slug)?;
        Some(map.get(&email).cloned().unwrap_or_default())
    }

    /// Returns a fresh-or-cached tenant map, refreshing on a TTL miss.
    ///
    /// On a resolver error, serves a stale cached map when present, else `None`.
    fn tenant_map(&self, tenant_slug: &str) -> Option<HashMap<String, Vec<String>>> {
        let now = Instant::now();
        {
            let cache = self.cache.lock().ok()?;
            if let Some((fetched_at, map)) = cache.get(tenant_slug)
                && now.duration_since(*fetched_at) < self.ttl
            {
                return Some(map.clone());
            }
        }

        match self.resolver.tenant_user_roles(tenant_slug) {
            Ok(map) => {
                if let Ok(mut cache) = self.cache.lock() {
                    cache.insert(tenant_slug.to_string(), (now, map.clone()));
                }
                Some(map)
            }
            Err(err) => {
                // Admin is unreachable or returned a bad body. Prefer serving a
                // stale map (availability) over dropping all asserted roles.
                if let Ok(cache) = self.cache.lock()
                    && let Some((_, map)) = cache.get(tenant_slug)
                {
                    eprintln!(
                        "warning: admin roles refresh failed for tenant `{tenant_slug}`, \
                         serving stale cache: {err}"
                    );
                    return Some(map.clone());
                }
                eprintln!(
                    "warning: admin roles resolution failed for tenant `{tenant_slug}` \
                     and no cache is available: {err}"
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// Shared, observable state behind a [`FakeResolver`]. Tests hold an `Arc`
    /// to this so they can read the call count / flip the failure flag while
    /// the overlay owns the resolver itself.
    struct FakeState {
        map: HashMap<String, Vec<String>>,
        calls: AtomicUsize,
        fail: AtomicBool,
    }

    /// In-memory resolver for tests; no real HTTP.
    #[derive(Clone)]
    struct FakeResolver {
        state: Arc<FakeState>,
    }

    impl FakeResolver {
        fn new(pairs: &[(&str, &[&str])]) -> Self {
            let map = pairs
                .iter()
                .map(|(email, roles)| {
                    (
                        email.to_ascii_lowercase(),
                        roles.iter().map(|r| r.to_string()).collect::<Vec<_>>(),
                    )
                })
                .collect();
            Self {
                state: Arc::new(FakeState {
                    map,
                    calls: AtomicUsize::new(0),
                    fail: AtomicBool::new(false),
                }),
            }
        }

        fn call_count(&self) -> usize {
            self.state.calls.load(Ordering::SeqCst)
        }

        fn set_fail(&self, fail: bool) {
            self.state.fail.store(fail, Ordering::SeqCst);
        }
    }

    impl RolesResolver for FakeResolver {
        fn tenant_user_roles(
            &self,
            _tenant_slug: &str,
        ) -> Result<HashMap<String, Vec<String>>, String> {
            self.state.calls.fetch_add(1, Ordering::SeqCst);
            if self.state.fail.load(Ordering::SeqCst) {
                return Err("simulated admin outage".to_string());
            }
            Ok(self.state.map.clone())
        }
    }

    fn overlay_with(resolver: FakeResolver, ttl: Duration) -> AdminRolesOverlay {
        AdminRolesOverlay::new(Box::new(resolver), ttl)
    }

    #[test]
    fn overlay_returns_caller_roles() {
        let resolver = FakeResolver::new(&[("alice@x", &["sorla_composer"])]);
        let overlay = overlay_with(resolver, Duration::from_secs(300));
        assert_eq!(
            overlay.roles_for("t", Some("alice@x")),
            Some(vec!["sorla_composer".to_string()])
        );
    }

    #[test]
    fn overlay_caseinsensitive_email() {
        let resolver = FakeResolver::new(&[("alice@x", &["sorla_composer"])]);
        let overlay = overlay_with(resolver, Duration::from_secs(300));
        assert_eq!(
            overlay.roles_for("t", Some("Alice@X")),
            Some(vec!["sorla_composer".to_string()])
        );
    }

    #[test]
    fn overlay_unknown_user_empty() {
        let resolver = FakeResolver::new(&[("alice@x", &["sorla_composer"])]);
        let overlay = overlay_with(resolver, Duration::from_secs(300));
        // Known tenant, unknown user => resolved-but-empty (Some(empty)), so the
        // seam yields the safe ["local"] fallback rather than admin roles.
        assert_eq!(overlay.roles_for("t", Some("nobody@x")), Some(Vec::new()));
    }

    #[test]
    fn overlay_no_email_none() {
        let resolver = FakeResolver::new(&[("alice@x", &["sorla_composer"])]);
        let overlay = overlay_with(resolver, Duration::from_secs(300));
        assert_eq!(overlay.roles_for("t", None), None);
        // Blank email is treated like no email.
        assert_eq!(overlay.roles_for("t", Some("   ")), None);
    }

    #[test]
    fn overlay_ttl_refetch() {
        let resolver = FakeResolver::new(&[("alice@x", &["sorla_composer"])]);
        let probe = resolver.clone();
        let overlay = overlay_with(resolver, Duration::from_millis(40));

        // First lookup fetches.
        let _ = overlay.roles_for("t", Some("alice@x"));
        assert_eq!(probe.call_count(), 1);

        // Second lookup within TTL is served from cache (no new fetch).
        let _ = overlay.roles_for("t", Some("alice@x"));
        assert_eq!(probe.call_count(), 1);

        // After TTL expiry the resolver is called again.
        std::thread::sleep(Duration::from_millis(60));
        let _ = overlay.roles_for("t", Some("alice@x"));
        assert_eq!(probe.call_count(), 2);
    }

    #[test]
    fn overlay_stale_on_error() {
        let resolver = FakeResolver::new(&[("alice@x", &["sorla_composer"])]);
        let probe = resolver.clone();
        // Tiny TTL so the second lookup forces a refetch.
        let overlay = overlay_with(resolver, Duration::from_millis(10));

        // Prime the cache successfully.
        assert_eq!(
            overlay.roles_for("t", Some("alice@x")),
            Some(vec!["sorla_composer".to_string()])
        );

        // Flip the resolver to error and expire the cache.
        probe.set_fail(true);
        std::thread::sleep(Duration::from_millis(20));

        // The stale cached map is served instead of None.
        assert_eq!(
            overlay.roles_for("t", Some("alice@x")),
            Some(vec!["sorla_composer".to_string()])
        );
    }

    #[test]
    fn overlay_error_no_cache_none() {
        let resolver = FakeResolver::new(&[("alice@x", &["sorla_composer"])]);
        resolver.set_fail(true);
        let overlay = overlay_with(resolver, Duration::from_secs(300));
        // First-ever fetch errors with no cache => None.
        assert_eq!(overlay.roles_for("t", Some("alice@x")), None);
    }

    #[test]
    fn parse_user_roles_lowercases_and_skips_blank() {
        let body = r#"{
            "users": [
                { "user_id": "1", "email": "Alice@X", "display_name": "A", "roles": ["sorla_composer", "reviewer"] },
                { "user_id": "2", "email": "", "roles": ["ignored"] },
                { "user_id": "3", "roles": ["no_email"] }
            ]
        }"#;
        let map = parse_user_roles(body).expect("parse");
        assert_eq!(map.len(), 1);
        assert_eq!(
            map.get("alice@x"),
            Some(&vec!["sorla_composer".to_string(), "reviewer".to_string()])
        );
    }

    #[test]
    fn parse_user_roles_empty_users() {
        let map = parse_user_roles(r#"{"users":[]}"#).expect("parse");
        assert!(map.is_empty());
        let map = parse_user_roles(r#"{}"#).expect("parse missing users");
        assert!(map.is_empty());
    }

    #[test]
    fn parse_user_roles_invalid_json_errors() {
        assert!(parse_user_roles("not json").is_err());
    }
}
