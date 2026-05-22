use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{ProviderBindingConfig, SorxRuntimeConfig};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilityRequirement {
    pub id: String,
    pub capability: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedProviderBinding {
    pub requirement: String,
    pub provider_id: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCompatibilityReport {
    pub status: ProviderCompatibilityStatus,
    pub bindings: Vec<ResolvedProviderBinding>,
    pub issues: Vec<ProviderCompatibilityIssue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderCompatibilityStatus {
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCompatibilityIssue {
    pub category: ProviderCompatibilityIssueCategory,
    pub requirement: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCompatibilityIssueCategory {
    MissingProvider,
    MissingCapability,
    IncompatibleContractVersion,
    UnsupportedOntologySchema,
    UnsupportedRetrievalBindingSchema,
    AmbiguousProvider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderResolutionMode {
    DryRun,
    RuntimeStartup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCompatibilityInput {
    pub ontology_present: bool,
    pub ontology_schema_supported: bool,
    pub retrieval_bindings_present: bool,
    pub retrieval_bindings_schema_supported: bool,
    pub requires_entity_link: bool,
    pub required_capabilities: Vec<String>,
}

impl ProviderCompatibilityInput {
    pub fn none() -> Self {
        Self {
            ontology_present: false,
            ontology_schema_supported: true,
            retrieval_bindings_present: false,
            retrieval_bindings_schema_supported: true,
            requires_entity_link: false,
            required_capabilities: Vec::new(),
        }
    }
}

pub fn resolve_provider_compatibility(
    config: &SorxRuntimeConfig,
    input: &ProviderCompatibilityInput,
    _mode: ProviderResolutionMode,
) -> ProviderCompatibilityReport {
    let mut requirements = Vec::new();
    let mut issues = Vec::new();

    if input.ontology_present && !input.ontology_schema_supported {
        issues.push(issue(
            ProviderCompatibilityIssueCategory::UnsupportedOntologySchema,
            "ontology.schema",
            "ontology graph schema is not supported",
        ));
    }
    if input.retrieval_bindings_present && !input.retrieval_bindings_schema_supported {
        issues.push(issue(
            ProviderCompatibilityIssueCategory::UnsupportedRetrievalBindingSchema,
            "retrieval_bindings.schema",
            "retrieval binding schema is not supported",
        ));
    }
    if input.retrieval_bindings_present {
        requirements.push(ProviderCapabilityRequirement {
            id: "evidence.query".to_string(),
            capability: "ontology-scoped-evidence-query".to_string(),
            provider_id: None,
        });
    }
    if input.requires_entity_link {
        requirements.push(ProviderCapabilityRequirement {
            id: "entity.link".to_string(),
            capability: "entity-link".to_string(),
            provider_id: None,
        });
    }
    for capability in &input.required_capabilities {
        requirements.push(ProviderCapabilityRequirement {
            id: capability.clone(),
            capability: capability.clone(),
            provider_id: None,
        });
    }

    let providers = config
        .providers
        .iter()
        .map(|(id, provider)| (id.clone(), provider))
        .collect::<BTreeMap<_, _>>();
    let mut bindings = Vec::new();
    for requirement in requirements {
        match resolve_requirement(&providers, &requirement) {
            Ok(binding) => bindings.push(binding),
            Err(error) => issues.push(error),
        }
    }
    bindings.sort_by(|left, right| left.requirement.cmp(&right.requirement));
    issues.sort_by(|left, right| {
        (
            left.requirement.as_str(),
            issue_category_name(left.category),
            left.message.as_str(),
        )
            .cmp(&(
                right.requirement.as_str(),
                issue_category_name(right.category),
                right.message.as_str(),
            ))
    });

    ProviderCompatibilityReport {
        status: if issues.is_empty() {
            ProviderCompatibilityStatus::Passed
        } else {
            ProviderCompatibilityStatus::Failed
        },
        bindings,
        issues,
    }
}

fn resolve_requirement(
    providers: &BTreeMap<String, &ProviderBindingConfig>,
    requirement: &ProviderCapabilityRequirement,
) -> Result<ResolvedProviderBinding, ProviderCompatibilityIssue> {
    if let Some(provider_id) = requirement.provider_id.as_deref() {
        let Some(provider) = providers.get(provider_id) else {
            return Err(issue(
                ProviderCompatibilityIssueCategory::MissingProvider,
                &requirement.id,
                format!("provider `{provider_id}` is not configured"),
            ));
        };
        return provider_binding(provider_id, provider, requirement);
    }

    let matches = providers
        .iter()
        .filter(|(_, provider)| provider.capabilities.contains(&requirement.capability))
        .collect::<Vec<_>>();
    if matches.is_empty() {
        let category = if providers.is_empty() {
            ProviderCompatibilityIssueCategory::MissingProvider
        } else {
            ProviderCompatibilityIssueCategory::MissingCapability
        };
        return Err(issue(
            category,
            &requirement.id,
            format!(
                "no configured provider exposes capability `{}`",
                requirement.capability
            ),
        ));
    }
    if matches.len() > 1 {
        return Err(issue(
            ProviderCompatibilityIssueCategory::AmbiguousProvider,
            &requirement.id,
            format!(
                "multiple providers expose capability `{}`",
                requirement.capability
            ),
        ));
    }
    let (provider_id, provider) = matches[0];
    provider_binding(provider_id, provider, requirement)
}

fn provider_binding(
    provider_id: &str,
    provider: &ProviderBindingConfig,
    requirement: &ProviderCapabilityRequirement,
) -> Result<ResolvedProviderBinding, ProviderCompatibilityIssue> {
    if !provider.capabilities.contains(&requirement.capability) {
        return Err(issue(
            ProviderCompatibilityIssueCategory::MissingCapability,
            &requirement.id,
            format!(
                "provider `{provider_id}` does not expose capability `{}`",
                requirement.capability
            ),
        ));
    }
    if let Some(contract_version) = provider.contract_version.as_deref()
        && !contract_version.starts_with("greentic.sorx.provider.v1")
        && contract_version != "1"
    {
        return Err(issue(
            ProviderCompatibilityIssueCategory::IncompatibleContractVersion,
            &requirement.id,
            format!("provider `{provider_id}` has incompatible contract `{contract_version}`"),
        ));
    }
    Ok(ResolvedProviderBinding {
        requirement: requirement.id.clone(),
        provider_id: provider_id.to_string(),
        capabilities: provider.capabilities.clone(),
    })
}

fn issue(
    category: ProviderCompatibilityIssueCategory,
    requirement: impl Into<String>,
    message: impl Into<String>,
) -> ProviderCompatibilityIssue {
    ProviderCompatibilityIssue {
        category,
        requirement: requirement.into(),
        message: message.into(),
    }
}

fn issue_category_name(category: ProviderCompatibilityIssueCategory) -> &'static str {
    match category {
        ProviderCompatibilityIssueCategory::MissingProvider => "missing_provider",
        ProviderCompatibilityIssueCategory::MissingCapability => "missing_capability",
        ProviderCompatibilityIssueCategory::IncompatibleContractVersion => {
            "incompatible_contract_version"
        }
        ProviderCompatibilityIssueCategory::UnsupportedOntologySchema => {
            "unsupported_ontology_schema"
        }
        ProviderCompatibilityIssueCategory::UnsupportedRetrievalBindingSchema => {
            "unsupported_retrieval_binding_schema"
        }
        ProviderCompatibilityIssueCategory::AmbiguousProvider => "ambiguous_provider",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{normalize_start_answers, runtime_config_from_answers};

    use super::*;

    fn config(providers: serde_json::Value) -> SorxRuntimeConfig {
        let answers = json!({
            "tenant": {"tenant_id": "tenant-a", "environment": "local"},
            "server": {"bind": "127.0.0.1:0", "public_base_url": "http://127.0.0.1:0"},
            "providers": providers,
            "policy": {"approvals": {}},
            "audit": {"sink": "disabled"},
            "deployment": {
                "tenant_id": "tenant-a",
                "sor_name": "landlord",
                "environment": "local",
                "deployment_mode": "local_single",
                "api_version_label": "local",
                "base_path": "/"
            },
            "exposure": {
                "default_visibility": "private",
                "require_validation_suite": false,
                "auto_promote_on_validation_pass": false,
                "public_aliases_allowed": []
            },
            "ghcr": {"enable_publish_webhook": false, "allowed_repositories": [], "require_exact_digest": true}
        });
        let normalized = normalize_start_answers(&crate::default_start_schema(), &answers, true)
            .expect("answers normalize");
        runtime_config_from_answers("landlord", &normalized.answers).expect("config")
    }

    fn ontology_with_retrieval() -> ProviderCompatibilityInput {
        ProviderCompatibilityInput {
            ontology_present: true,
            ontology_schema_supported: true,
            retrieval_bindings_present: true,
            retrieval_bindings_schema_supported: true,
            requires_entity_link: false,
            required_capabilities: Vec::new(),
        }
    }

    #[test]
    fn compatible_provider_passes() {
        let config = config(json!({
            "store": {"kind": "memory"},
            "rag": {
                "kind": "memory",
                "capabilities": ["ontology-scoped-evidence-query"],
                "contract_version": "greentic.sorx.provider.v1"
            }
        }));
        let report = resolve_provider_compatibility(
            &config,
            &ontology_with_retrieval(),
            ProviderResolutionMode::DryRun,
        );
        assert_eq!(report.status, ProviderCompatibilityStatus::Passed);
        assert_eq!(report.bindings[0].provider_id, "rag");
    }

    #[test]
    fn missing_evidence_provider_fails() {
        let config = config(json!({"store": {"kind": "memory"}}));
        let report = resolve_provider_compatibility(
            &config,
            &ontology_with_retrieval(),
            ProviderResolutionMode::DryRun,
        );
        assert_eq!(report.status, ProviderCompatibilityStatus::Failed);
        assert_eq!(
            report.issues[0].category,
            ProviderCompatibilityIssueCategory::MissingCapability
        );
    }

    #[test]
    fn missing_entity_link_capability_fails() {
        let config = config(json!({
            "store": {"kind": "memory"},
            "rag": {"kind": "memory", "capabilities": ["ontology-scoped-evidence-query"]}
        }));
        let mut input = ontology_with_retrieval();
        input.requires_entity_link = true;
        let report =
            resolve_provider_compatibility(&config, &input, ProviderResolutionMode::DryRun);
        assert_eq!(report.status, ProviderCompatibilityStatus::Failed);
        assert!(report.issues.iter().any(|issue| {
            issue.category == ProviderCompatibilityIssueCategory::MissingCapability
                && issue.requirement == "entity.link"
        }));
    }

    #[test]
    fn incompatible_contract_version_fails() {
        let config = config(json!({
            "store": {"kind": "memory"},
            "rag": {
                "kind": "memory",
                "capabilities": ["ontology-scoped-evidence-query"],
                "contract_version": "greentic.sorx.provider.v2"
            }
        }));
        let report = resolve_provider_compatibility(
            &config,
            &ontology_with_retrieval(),
            ProviderResolutionMode::DryRun,
        );
        assert_eq!(
            report.issues[0].category,
            ProviderCompatibilityIssueCategory::IncompatibleContractVersion
        );
    }

    #[test]
    fn ambiguous_provider_reports_deterministic_issue() {
        let config = config(json!({
            "store": {"kind": "memory"},
            "a": {"kind": "memory", "capabilities": ["ontology-scoped-evidence-query"]},
            "b": {"kind": "memory", "capabilities": ["ontology-scoped-evidence-query"]}
        }));
        let report = resolve_provider_compatibility(
            &config,
            &ontology_with_retrieval(),
            ProviderResolutionMode::DryRun,
        );
        assert_eq!(
            report.issues[0].category,
            ProviderCompatibilityIssueCategory::AmbiguousProvider
        );
    }

    #[test]
    fn unsupported_schemas_fail() {
        let config = config(json!({"store": {"kind": "memory"}}));
        let input = ProviderCompatibilityInput {
            ontology_present: true,
            ontology_schema_supported: false,
            retrieval_bindings_present: true,
            retrieval_bindings_schema_supported: false,
            requires_entity_link: false,
            required_capabilities: Vec::new(),
        };
        let report =
            resolve_provider_compatibility(&config, &input, ProviderResolutionMode::DryRun);
        assert!(report.issues.iter().any(|issue| {
            issue.category == ProviderCompatibilityIssueCategory::UnsupportedOntologySchema
        }));
        assert!(report.issues.iter().any(|issue| {
            issue.category == ProviderCompatibilityIssueCategory::UnsupportedRetrievalBindingSchema
        }));
    }
}
