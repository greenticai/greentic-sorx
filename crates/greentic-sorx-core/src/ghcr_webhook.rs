use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    CreateDeploymentRequest, DeploymentRegistry, DeploymentStatus, DeploymentVisibility,
    PackArtifact, StateMode,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhcrWebhookConfig {
    pub enabled: bool,
    pub public_path: String,
    pub signature_secret_ref: String,
    pub allowed_repositories: Vec<String>,
    pub allowed_oci_prefixes: Vec<String>,
    pub allowed_workflows: Vec<String>,
    pub allowed_environments: Vec<String>,
    pub default_promotion_policy: PromotionPolicy,
    pub require_exact_digest: bool,
}

impl GhcrWebhookConfig {
    pub fn local_test(secret_ref: impl Into<String>) -> Self {
        Self {
            enabled: true,
            public_path: "/v1/sorx/webhooks/github/ghcr-published".to_string(),
            signature_secret_ref: secret_ref.into(),
            allowed_repositories: vec![
                "greenticai/greentic-sorla".to_string(),
                "greenticai/greentic-sorla-providers".to_string(),
            ],
            allowed_oci_prefixes: vec![
                "ghcr.io/greenticai/sorla/".to_string(),
                "ghcr.io/greenticai/sorla-providers/".to_string(),
            ],
            allowed_workflows: vec!["publish-gtpack.yml".to_string(), "publish.yml".to_string()],
            allowed_environments: vec![
                "local".to_string(),
                "test".to_string(),
                "dev".to_string(),
                "staging".to_string(),
                "production".to_string(),
            ],
            default_promotion_policy: PromotionPolicy::ValidateThenPrivate,
            require_exact_digest: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionPolicy {
    #[serde(alias = "pending_only")]
    ManualOnly,
    ValidateThenPrivate,
    ValidateThenPublicPreview,
    ValidateThenPublicAlias,
}

impl PromotionPolicy {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "manual_only" | "pending_only" => Some(Self::ManualOnly),
            "validate_then_private" => Some(Self::ValidateThenPrivate),
            "validate_then_public_preview" => Some(Self::ValidateThenPublicPreview),
            "validate_then_public_alias" => Some(Self::ValidateThenPublicAlias),
            _ => None,
        }
    }

    pub fn requests_validation(self) -> bool {
        !matches!(self, Self::ManualOnly)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubWebhookHeaders {
    pub signature_256: String,
    pub event: String,
    pub delivery: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhcrPublishedMetadata {
    pub repository: String,
    pub workflow: String,
    pub conclusion: String,
    pub artifact_kind: String,
    pub oci_ref: String,
    pub digest: String,
    pub pack_name: String,
    pub pack_version: String,
    pub tenant_id: String,
    pub sor_name: String,
    pub environment: String,
    pub api_version_label: String,
    #[serde(default)]
    pub promotion_policy: Option<PromotionPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OciReference {
    pub value: String,
}

impl OciReference {
    pub fn registry_reference(&self) -> &str {
        self.value.strip_prefix("oci://").unwrap_or(&self.value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedOciArtifact {
    pub original_ref: String,
    pub resolved_digest: String,
    pub media_type: String,
    pub size: u64,
    pub annotations: BTreeMap<String, String>,
    pub local_cache_path: Option<PathBuf>,
}

pub trait OciArtifactResolver {
    fn resolve(&self, reference: &OciReference) -> Result<ResolvedOciArtifact, GhcrWebhookError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GhcrWebhookOutcome {
    pub schema: String,
    pub delivery: String,
    pub deployment_id: String,
    pub status: DeploymentStatus,
    pub validation_job_requested: bool,
    pub public_exposure_started: bool,
    pub resolved: ResolvedOciArtifact,
}

pub fn handle_ghcr_published_webhook(
    config: &GhcrWebhookConfig,
    registry: &mut DeploymentRegistry,
    headers: &GithubWebhookHeaders,
    body: &[u8],
    secret: &[u8],
    resolver: &dyn OciArtifactResolver,
) -> Result<GhcrWebhookOutcome, GhcrWebhookError> {
    if !config.enabled {
        return Err(GhcrWebhookError::new(
            "webhook_disabled",
            "GHCR webhook handling is disabled",
        ));
    }
    if headers.event != "repository_dispatch" && headers.event != "workflow_run" {
        return Err(GhcrWebhookError::new(
            "unsupported_event",
            format!("unsupported GitHub event `{}`", headers.event),
        ));
    }
    if headers.delivery.trim().is_empty() {
        return Err(GhcrWebhookError::new(
            "missing_delivery",
            "X-GitHub-Delivery is required",
        ));
    }
    if registry.has_webhook_delivery(&headers.delivery) {
        return Err(GhcrWebhookError::new(
            "webhook_replay",
            format!(
                "webhook delivery `{}` was already processed",
                headers.delivery
            ),
        ));
    }
    verify_github_signature(secret, body, &headers.signature_256)?;

    let metadata = parse_ghcr_published_metadata(body)?;
    validate_metadata(config, &metadata)?;

    let reference = OciReference {
        value: metadata.oci_ref.clone(),
    };
    let resolved = resolver.resolve(&reference)?;
    if config.require_exact_digest && resolved.resolved_digest != metadata.digest {
        return Err(GhcrWebhookError::new(
            "digest_mismatch",
            format!(
                "resolved digest `{}` does not match payload digest `{}`",
                resolved.resolved_digest, metadata.digest
            ),
        ));
    }

    let promotion_policy = metadata
        .promotion_policy
        .unwrap_or(config.default_promotion_policy);
    let base_path = format!(
        "/sorx/{}/{}/{}",
        metadata.tenant_id, metadata.sor_name, metadata.api_version_label
    );
    let deployment = registry
        .create_deployment(CreateDeploymentRequest {
            artifact: PackArtifact {
                source: metadata.oci_ref,
                name: metadata.pack_name,
                version: metadata.pack_version,
                digest: resolved.resolved_digest.clone(),
                signature: None,
                signature_ref: Some(config.signature_secret_ref.clone()),
            },
            tenant_id: metadata.tenant_id,
            sor_name: metadata.sor_name,
            environment: metadata.environment,
            api_version_label: metadata.api_version_label.clone(),
            base_path,
            visibility: DeploymentVisibility::Private,
            state_mode: StateMode::Isolated,
            state_namespace: None,
            deployment_id: None,
            allow_api_version_conflict: false,
            allow_shared_state_conflict: false,
        })
        .map_err(|err| GhcrWebhookError::new(err.code, err.message))?;
    registry
        .record_webhook_delivery(headers.delivery.clone())
        .map_err(|err| GhcrWebhookError::new(err.code, err.message))?;

    Ok(GhcrWebhookOutcome {
        schema: "greentic.sorx.ghcr-webhook.outcome.v1".to_string(),
        delivery: headers.delivery.clone(),
        deployment_id: deployment.deployment_id,
        status: deployment.status,
        validation_job_requested: promotion_policy.requests_validation(),
        public_exposure_started: false,
        resolved,
    })
}

pub fn parse_ghcr_published_metadata(
    body: &[u8],
) -> Result<GhcrPublishedMetadata, GhcrWebhookError> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|err| GhcrWebhookError::new("invalid_payload", err.to_string()))?;
    if let Some(client_payload) = value.get("client_payload")
        && client_payload.is_object()
    {
        return serde_json::from_value(client_payload.clone())
            .map_err(|err| GhcrWebhookError::new("invalid_payload", err.to_string()));
    }
    serde_json::from_value(value)
        .map_err(|err| GhcrWebhookError::new("invalid_payload", err.to_string()))
}

pub fn verify_github_signature(
    secret: &[u8],
    body: &[u8],
    signature: &str,
) -> Result<(), GhcrWebhookError> {
    let Some(hex_signature) = signature.strip_prefix("sha256=") else {
        return Err(GhcrWebhookError::new(
            "bad_signature_format",
            "X-Hub-Signature-256 must start with sha256=",
        ));
    };
    let expected = hmac_sha256_hex(secret, body);
    if constant_time_eq(expected.as_bytes(), hex_signature.as_bytes()) {
        Ok(())
    } else {
        Err(GhcrWebhookError::new(
            "bad_signature",
            "GitHub webhook signature did not match",
        ))
    }
}

pub fn github_signature(secret: &[u8], body: &[u8]) -> String {
    format!("sha256={}", hmac_sha256_hex(secret, body))
}

fn validate_metadata(
    config: &GhcrWebhookConfig,
    metadata: &GhcrPublishedMetadata,
) -> Result<(), GhcrWebhookError> {
    if metadata.conclusion != "success" {
        return Err(GhcrWebhookError::new(
            "workflow_not_successful",
            format!("workflow conclusion was `{}`", metadata.conclusion),
        ));
    }
    if !config
        .allowed_repositories
        .iter()
        .any(|repository| repository == &metadata.repository)
    {
        return Err(GhcrWebhookError::new(
            "repository_untrusted",
            format!("repository `{}` is not allowed", metadata.repository),
        ));
    }
    if !config
        .allowed_workflows
        .iter()
        .any(|workflow| workflow == &metadata.workflow)
    {
        return Err(GhcrWebhookError::new(
            "workflow_untrusted",
            format!("workflow `{}` is not allowed", metadata.workflow),
        ));
    }
    if !config
        .allowed_environments
        .iter()
        .any(|environment| environment == &metadata.environment)
    {
        return Err(GhcrWebhookError::new(
            "environment_untrusted",
            format!("environment `{}` is not allowed", metadata.environment),
        ));
    }
    if !config.allowed_oci_prefixes.iter().any(|prefix| {
        metadata
            .oci_ref
            .trim_start_matches("oci://")
            .starts_with(prefix)
    }) {
        return Err(GhcrWebhookError::new(
            "oci_prefix_untrusted",
            format!("OCI reference `{}` is not allowed", metadata.oci_ref),
        ));
    }
    if config.require_exact_digest && !metadata.digest.starts_with("sha256:") {
        return Err(GhcrWebhookError::new(
            "digest_required",
            "payload must include an exact sha256 digest",
        ));
    }
    Ok(())
}

fn hmac_sha256_hex(secret: &[u8], body: &[u8]) -> String {
    const BLOCK_SIZE: usize = 64;
    let mut key = if secret.len() > BLOCK_SIZE {
        Sha256::digest(secret).to_vec()
    } else {
        secret.to_vec()
    };
    key.resize(BLOCK_SIZE, 0);

    let mut outer_key_pad = [0x5c; BLOCK_SIZE];
    let mut inner_key_pad = [0x36; BLOCK_SIZE];
    for (index, byte) in key.iter().enumerate() {
        outer_key_pad[index] ^= byte;
        inner_key_pad[index] ^= byte;
    }

    let mut inner = Sha256::new();
    inner.update(inner_key_pad);
    inner.update(body);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(outer_key_pad);
    outer.update(inner_hash);
    hex_lower(&outer.finalize())
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |acc, (left, right)| acc | (left ^ right))
        == 0
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhcrWebhookError {
    pub code: String,
    pub message: String,
}

impl GhcrWebhookError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for GhcrWebhookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for GhcrWebhookError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct FakeResolver {
        digest: String,
    }

    impl OciArtifactResolver for FakeResolver {
        fn resolve(
            &self,
            reference: &OciReference,
        ) -> Result<ResolvedOciArtifact, GhcrWebhookError> {
            Ok(ResolvedOciArtifact {
                original_ref: reference.value.clone(),
                resolved_digest: self.digest.clone(),
                media_type: "application/vnd.greentic.sorla.gtpack".to_string(),
                size: 1234,
                annotations: BTreeMap::new(),
                local_cache_path: None,
            })
        }
    }

    fn body(overrides: &[(&str, &str)]) -> Vec<u8> {
        let mut value = serde_json::json!({
            "repository": "greenticai/greentic-sorla",
            "workflow": "publish-gtpack.yml",
            "conclusion": "success",
            "artifact_kind": "sorla-gtpack",
            "oci_ref": "oci://ghcr.io/greenticai/sorla/landlord-tenant-sor:1.1.0",
            "digest": "sha256:abc123",
            "pack_name": "landlord-tenant-sor",
            "pack_version": "1.1.0",
            "tenant_id": "acme",
            "sor_name": "landlord-tenant",
            "environment": "staging",
            "api_version_label": "v1.1",
            "promotion_policy": "validate_then_private"
        });
        for (key, replacement) in overrides {
            value[*key] = Value::String((*replacement).to_string());
        }
        serde_json::to_vec(&value).unwrap()
    }

    fn headers(delivery: &str, body: &[u8], secret: &[u8]) -> GithubWebhookHeaders {
        GithubWebhookHeaders {
            signature_256: github_signature(secret, body),
            event: "repository_dispatch".to_string(),
            delivery: delivery.to_string(),
        }
    }

    fn test_webhook_secret() -> &'static [u8] {
        &[116, 101, 115, 116, 45, 115, 101, 99, 114, 101, 116]
    }

    #[test]
    fn valid_event_creates_pending_deployment_without_public_exposure() {
        let secret = test_webhook_secret();
        let body = body(&[]);
        let headers = headers("delivery-1", &body, secret);
        let mut registry = DeploymentRegistry::default();
        let outcome = handle_ghcr_published_webhook(
            &GhcrWebhookConfig::local_test("secret://test"),
            &mut registry,
            &headers,
            &body,
            secret,
            &FakeResolver {
                digest: "sha256:abc123".to_string(),
            },
        )
        .unwrap();
        assert_eq!(outcome.status, DeploymentStatus::Pending);
        assert!(outcome.validation_job_requested);
        assert!(!outcome.public_exposure_started);
        assert_eq!(registry.deployments.len(), 1);
        assert_eq!(
            registry.deployments[0].artifact.source,
            "oci://ghcr.io/greenticai/sorla/landlord-tenant-sor:1.1.0"
        );
    }

    #[test]
    fn failed_workflow_is_rejected() {
        let secret = test_webhook_secret();
        let body = body(&[("conclusion", "failure")]);
        let err = handle_ghcr_published_webhook(
            &GhcrWebhookConfig::local_test("secret://test"),
            &mut DeploymentRegistry::default(),
            &headers("delivery-1", &body, secret),
            &body,
            secret,
            &FakeResolver {
                digest: "sha256:abc123".to_string(),
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "workflow_not_successful");
    }

    #[test]
    fn bad_hmac_is_rejected() {
        let secret = test_webhook_secret();
        let body = body(&[]);
        let mut headers = headers("delivery-1", &body, secret);
        headers.signature_256 = format!("sha256={}", "0".repeat(64));
        let err = handle_ghcr_published_webhook(
            &GhcrWebhookConfig::local_test("secret://test"),
            &mut DeploymentRegistry::default(),
            &headers,
            &body,
            secret,
            &FakeResolver {
                digest: "sha256:abc123".to_string(),
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "bad_signature");
    }

    #[test]
    fn untrusted_repository_is_rejected() {
        let secret = test_webhook_secret();
        let body = body(&[("repository", "someone/else")]);
        let err = handle_ghcr_published_webhook(
            &GhcrWebhookConfig::local_test("secret://test"),
            &mut DeploymentRegistry::default(),
            &headers("delivery-1", &body, secret),
            &body,
            secret,
            &FakeResolver {
                digest: "sha256:abc123".to_string(),
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "repository_untrusted");
    }

    #[test]
    fn untrusted_oci_prefix_is_rejected() {
        let secret = test_webhook_secret();
        let body = body(&[("oci_ref", "oci://ghcr.io/someone/else/pkg:1.0.0")]);
        let err = handle_ghcr_published_webhook(
            &GhcrWebhookConfig::local_test("secret://test"),
            &mut DeploymentRegistry::default(),
            &headers("delivery-1", &body, secret),
            &body,
            secret,
            &FakeResolver {
                digest: "sha256:abc123".to_string(),
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "oci_prefix_untrusted");
    }

    #[test]
    fn replay_delivery_id_is_rejected() {
        let secret = test_webhook_secret();
        let body = body(&[]);
        let headers = headers("delivery-1", &body, secret);
        let config = GhcrWebhookConfig::local_test("secret://test");
        let resolver = FakeResolver {
            digest: "sha256:abc123".to_string(),
        };
        let mut registry = DeploymentRegistry::default();
        handle_ghcr_published_webhook(&config, &mut registry, &headers, &body, secret, &resolver)
            .unwrap();
        let err = handle_ghcr_published_webhook(
            &config,
            &mut registry,
            &headers,
            &body,
            secret,
            &resolver,
        )
        .unwrap_err();
        assert_eq!(err.code, "webhook_replay");
    }

    #[test]
    fn digest_mismatch_is_rejected() {
        let secret = test_webhook_secret();
        let body = body(&[]);
        let err = handle_ghcr_published_webhook(
            &GhcrWebhookConfig::local_test("secret://test"),
            &mut DeploymentRegistry::default(),
            &headers("delivery-1", &body, secret),
            &body,
            secret,
            &FakeResolver {
                digest: "sha256:different".to_string(),
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "digest_mismatch");
    }
}
