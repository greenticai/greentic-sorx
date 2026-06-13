use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::BindingResolver;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SorxStartAnswers {
    pub normalized: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SorxNormalizedAnswers {
    pub schema: String,
    pub answers: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SorxRuntimeConfig {
    pub tenant_id: String,
    pub environment: String,
    pub server: ServerConfig,
    pub mcp: McpConfig,
    pub providers: BTreeMap<String, ProviderBindingConfig>,
    pub bindings: BindingResolver,
    pub policy: BTreeMap<String, String>,
    pub audit: AuditConfig,
    pub events: EventsConfig,
    pub deployment: DeploymentConfig,
    pub exposure: ExposureConfig,
    pub ghcr: GhcrConfig,
    pub ghcr_webhook: GhcrWebhookAnswerConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub bind: String,
    pub public_base_url: Option<String>,
    pub auth: ServerAuthConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerAuthConfig {
    pub mode: String,
    pub shared_secret_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpConfig {
    pub enabled: bool,
    pub bind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderBindingConfig {
    pub kind: String,
    pub config_ref: Option<String>,
    pub config: Option<Value>,
    pub capabilities: Vec<String>,
    pub contract_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditConfig {
    pub sink: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventsConfig {
    /// Business event sink: "disabled" | "stdout" | "nats".
    pub sink: String,
    pub nats_url: Option<String>,
    pub subject_prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentConfig {
    pub tenant_id: String,
    pub sor_name: String,
    pub environment: String,
    pub deployment_mode: String,
    pub api_version_label: String,
    pub base_path: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExposureConfig {
    pub default_visibility: String,
    pub require_validation_suite: bool,
    pub auto_promote_on_validation_pass: bool,
    pub public_aliases_allowed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhcrConfig {
    pub enable_publish_webhook: bool,
    pub allowed_repositories: Vec<String>,
    pub require_exact_digest: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhcrWebhookAnswerConfig {
    pub enabled: bool,
    pub bind: String,
    pub public_path: String,
    pub signature_secret_ref: Option<String>,
    pub allowed_repositories: Vec<String>,
    pub allowed_oci_prefixes: Vec<String>,
    pub allowed_workflows: Vec<String>,
    pub default_promotion_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SorxStartupError {
    pub code: String,
    pub message: String,
    pub issues: Vec<SorxStartupIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SorxStartupIssue {
    pub path: String,
    pub message: String,
}

impl SorxStartupError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            issues: Vec::new(),
        }
    }

    fn with_issues(
        code: impl Into<String>,
        message: impl Into<String>,
        issues: Vec<SorxStartupIssue>,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            issues,
        }
    }
}

impl fmt::Display for SorxStartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.issues.is_empty() {
            write!(f, "{}", self.message)
        } else {
            let paths = self
                .issues
                .iter()
                .map(|issue| issue.path.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            write!(f, "{}: {}", self.message, paths)
        }
    }
}

impl std::error::Error for SorxStartupError {}

pub fn default_start_schema() -> Value {
    json!({
        "schema": "greentic.sorx.start.schema.v1",
        "title": "Sorx startup answers",
        "type": "object",
        "required": ["tenant", "server", "providers", "policy", "audit", "deployment", "exposure", "ghcr"],
        "properties": {
            "tenant": {
                "type": "object",
                "required": ["tenant_id", "environment"],
                "properties": {
                    "tenant_id": { "type": "string" },
                    "environment": { "type": "string", "enum": ["local", "test", "dev", "staging", "production"], "default": "local" }
                }
            },
            "server": {
                "type": "object",
                "required": ["bind", "public_base_url"],
                "properties": {
                    "bind": { "type": "string", "default": "127.0.0.1:8787" },
                    "public_base_url": { "type": "string" },
                    "auth": {
                        "type": "object",
                        "properties": {
                            "mode": { "type": "string", "enum": ["none", "shared_secret"], "default": "none" },
                            "shared_secret_ref": { "type": "string" }
                        }
                    }
                }
            },
            "mcp": {
                "type": "object",
                "properties": {
                    "enabled": { "type": "boolean", "default": false },
                    "bind": { "type": "string", "default": "127.0.0.1:8790" }
                }
            },
            "providers": {
                "type": "object",
                "default": {
                    "store": {
                        "kind": "foundationdb",
                        "config_ref": "providers.foundationdb.local"
                    }
                },
                "required": ["store"],
                "properties": {
                    "store": {
                        "type": "object",
                        "default": {
                            "kind": "foundationdb",
                            "config_ref": "providers.foundationdb.local"
                        },
                        "required": ["kind"],
                        "properties": {
                            "kind": { "type": "string", "enum": ["memory", "foundationdb"], "default": "foundationdb" },
                            "config_ref": { "type": "string" },
                            "capabilities": {
                                "type": "array",
                                "items": { "type": "string" },
                                "default": []
                            },
                            "contract_version": { "type": "string" },
                            "config": {}
                        }
                    }
                },
                "additionalProperties": {
                    "type": "object",
                    "required": ["kind"],
                    "properties": {
                        "kind": { "type": "string", "enum": ["memory", "foundationdb"] },
                        "config_ref": { "type": "string" },
                        "capabilities": {
                            "type": "array",
                            "items": { "type": "string" },
                            "default": []
                        },
                        "contract_version": { "type": "string" },
                        "config": {}
                    }
                }
            },
            "bindings": {
                "type": "object",
                "properties": {
                    "entities": {
                        "type": "object",
                        "additionalProperties": {
                            "type": "object",
                            "required": ["provider", "collection"],
                            "properties": {
                                "provider": { "type": "string" },
                                "collection": { "type": "string" }
                            }
                        }
                    }
                }
            },
            "policy": {
                "type": "object",
                "required": ["approvals"],
                "properties": {
                    "approvals": {
                        "type": "object",
                        "properties": {
                            "low": { "type": "string", "enum": ["auto", "require_approval", "deny"], "default": "auto" },
                            "medium": { "type": "string", "enum": ["auto", "require_approval", "deny"], "default": "auto" },
                            "high": { "type": "string", "enum": ["auto", "require_approval", "deny"], "default": "require_approval" },
                            "critical": { "type": "string", "enum": ["auto", "require_approval", "deny"], "default": "deny" }
                        }
                    }
                }
            },
            "audit": {
                "type": "object",
                "properties": {
                    "sink": { "type": "string", "enum": ["stdout", "file", "disabled"], "default": "stdout" }
                }
            },
            "events": {
                "type": "object",
                "properties": {
                    "sink": { "type": "string", "enum": ["disabled", "stdout", "nats"], "default": "disabled" },
                    "nats_url": { "type": "string" },
                    "subject_prefix": { "type": "string", "default": "greentic.events" }
                }
            },
            "deployment": {
                "type": "object",
                "required": ["tenant_id", "sor_name", "environment", "deployment_mode", "api_version_label", "base_path"],
                "properties": {
                    "tenant_id": { "type": "string" },
                    "sor_name": { "type": "string" },
                    "environment": { "type": "string", "enum": ["local", "test", "dev", "staging", "production"], "default": "local" },
                    "deployment_mode": { "type": "string", "enum": ["local_single", "versioned_registry"], "default": "local_single" },
                    "api_version_label": { "type": "string", "default": "local" },
                    "base_path": { "type": "string", "default": "/" },
                    "alias": { "type": "string" }
                }
            },
            "exposure": {
                "type": "object",
                "required": ["default_visibility", "require_validation_suite", "auto_promote_on_validation_pass", "public_aliases_allowed"],
                "properties": {
                    "default_visibility": { "type": "string", "enum": ["private", "internal", "public"], "default": "private" },
                    "require_validation_suite": { "type": "boolean", "default": false },
                    "auto_promote_on_validation_pass": { "type": "boolean", "default": false },
                    "public_aliases_allowed": {
                        "type": "array",
                        "default": ["stable", "latest", "preview"],
                        "items": { "type": "string", "enum": ["stable", "latest", "preview"] }
                    }
                }
            },
            "ghcr": {
                "type": "object",
                "required": ["enable_publish_webhook", "allowed_repositories", "require_exact_digest"],
                "properties": {
                    "enable_publish_webhook": { "type": "boolean", "default": false },
                    "allowed_repositories": {
                        "type": "array",
                        "default": ["ghcr.io/greenticai/sorla-packages/*"],
                        "items": { "type": "string" }
                    },
                    "require_exact_digest": { "type": "boolean", "default": true }
                }
            },
            "ghcr_webhook": {
                "type": "object",
                "properties": {
                    "enabled": { "type": "boolean", "default": false },
                    "bind": { "type": "string", "default": "127.0.0.1:8081" },
                    "public_path": { "type": "string", "default": "/v1/sorx/webhooks/github/ghcr-published" },
                    "signature_secret_ref": { "type": "string" },
                    "allowed_repositories": {
                        "type": "array",
                        "default": ["greenticai/greentic-sorla", "greenticai/greentic-sorla-providers"],
                        "items": { "type": "string" }
                    },
                    "allowed_oci_prefixes": {
                        "type": "array",
                        "default": ["ghcr.io/greenticai/sorla/", "ghcr.io/greenticai/sorla-providers/"],
                        "items": { "type": "string" }
                    },
                    "allowed_workflows": {
                        "type": "array",
                        "default": ["publish-gtpack.yml", "publish.yml"],
                        "items": { "type": "string" }
                    },
                    "default_promotion_policy": { "type": "string", "enum": ["pending_only", "validate_then_private"], "default": "validate_then_private" }
                }
            }
        }
    })
}

pub fn normalize_start_answers(
    schema: &Value,
    answers: &Value,
    non_interactive: bool,
) -> Result<SorxNormalizedAnswers, SorxStartupError> {
    let mut issues = Vec::new();
    let answers = qa_answer_payload(answers);
    let mut normalized = normalize_value(schema, answers, "", true, &mut issues);
    apply_provider_defaults(&mut normalized);
    issues.extend(secret_issues(&normalized, ""));
    issues.extend(provider_config_mode_issues(&normalized));

    if issues.is_empty() {
        Ok(SorxNormalizedAnswers {
            schema: "greentic.sorx.start.answers.normalized.v1".to_string(),
            answers: normalized,
        })
    } else if issues
        .iter()
        .any(|issue| issue.message == "required answer is missing")
    {
        let message = if non_interactive {
            "missing required startup answers in non-interactive mode"
        } else {
            "missing required startup answers; interactive greentic-qa prompting is not wired in this adapter yet"
        };
        Err(SorxStartupError::with_issues(
            "missing_answers",
            message,
            issues,
        ))
    } else {
        Err(SorxStartupError::with_issues(
            "invalid_answers",
            "startup answers failed validation",
            issues,
        ))
    }
}

fn apply_provider_defaults(value: &mut Value) {
    let Some(root) = value.as_object_mut() else {
        return;
    };
    let providers = root
        .entry("providers".to_string())
        .or_insert_with(|| json!({}))
        .as_object_mut();
    let Some(providers) = providers else {
        return;
    };
    let store = providers
        .entry("store".to_string())
        .or_insert_with(|| json!({}))
        .as_object_mut();
    let Some(store) = store else {
        return;
    };
    let kind = store
        .entry("kind".to_string())
        .or_insert_with(|| json!("foundationdb"))
        .as_str()
        .unwrap_or("foundationdb")
        .to_string();
    if store.get("config_ref").and_then(Value::as_str).is_some() || store.contains_key("config") {
        return;
    }
    let config_ref = match kind.as_str() {
        "memory" => "providers.memory.local",
        "foundationdb" => "providers.foundationdb.local",
        _ => return,
    };
    store.insert("config_ref".to_string(), json!(config_ref));
}

fn qa_answer_payload(value: &Value) -> &Value {
    let Some(object) = value.as_object() else {
        return value;
    };
    if object.contains_key("form_id") && object.contains_key("spec_version") {
        object.get("answers").unwrap_or(value)
    } else {
        value
    }
}

pub fn runtime_config_from_answers(
    pack_name: &str,
    answers: &Value,
) -> Result<SorxRuntimeConfig, SorxStartupError> {
    let object = answers.as_object().ok_or_else(|| {
        SorxStartupError::new(
            "invalid_answers",
            "normalized startup answers must be an object",
        )
    })?;
    let tenant = object_value(object, "tenant")?;
    let server = object_value(object, "server")?;
    let server_auth = server.get("auth").and_then(Value::as_object);
    let mcp = object.get("mcp").and_then(Value::as_object);
    let providers = object_value(object, "providers")?;
    let bindings = object.get("bindings").and_then(Value::as_object);
    let policy = object_value(object, "policy")?;
    let audit = object.get("audit").and_then(Value::as_object);
    let events = object.get("events").and_then(Value::as_object);
    let deployment = object.get("deployment").and_then(Value::as_object);
    let exposure = object.get("exposure").and_then(Value::as_object);
    let ghcr = object.get("ghcr").and_then(Value::as_object);
    let ghcr_webhook = object.get("ghcr_webhook").and_then(Value::as_object);

    Ok(SorxRuntimeConfig {
        tenant_id: string_at(tenant, "tenant_id").unwrap_or_default(),
        environment: string_at(tenant, "environment").unwrap_or_else(|| "local".to_string()),
        server: ServerConfig {
            bind: string_at(server, "bind").unwrap_or_else(|| "127.0.0.1:8787".to_string()),
            public_base_url: string_at(server, "public_base_url"),
            auth: ServerAuthConfig {
                mode: string_at_opt(server_auth, "mode").unwrap_or_else(|| "none".to_string()),
                shared_secret_ref: string_at_opt(server_auth, "shared_secret_ref"),
            },
        },
        mcp: McpConfig {
            enabled: bool_at(mcp, "enabled").unwrap_or(false),
            bind: string_at_opt(mcp, "bind").unwrap_or_else(|| "127.0.0.1:8790".to_string()),
        },
        providers: provider_configs(providers),
        bindings: BindingResolver::from_config(
            &bindings.cloned().unwrap_or_default(),
            first_provider_id(providers),
        )
        .map_err(|err| SorxStartupError::new(err.code, err.message))?,
        policy: approval_policy(policy),
        audit: AuditConfig {
            sink: string_at_opt(audit, "sink").unwrap_or_else(|| "stdout".to_string()),
        },
        events: EventsConfig {
            sink: string_at_opt(events, "sink").unwrap_or_else(|| "disabled".to_string()),
            nats_url: string_at_opt(events, "nats_url"),
            subject_prefix: string_at_opt(events, "subject_prefix")
                .unwrap_or_else(|| "greentic.events".to_string()),
        },
        deployment: DeploymentConfig {
            tenant_id: string_at_opt(deployment, "tenant_id")
                .or_else(|| string_at(tenant, "tenant_id"))
                .unwrap_or_default(),
            sor_name: string_at_opt(deployment, "sor_name")
                .unwrap_or_else(|| pack_name.to_string()),
            environment: string_at_opt(deployment, "environment")
                .or_else(|| string_at(tenant, "environment"))
                .unwrap_or_else(|| "local".to_string()),
            deployment_mode: string_at_opt(deployment, "deployment_mode")
                .unwrap_or_else(|| "local_single".to_string()),
            api_version_label: string_at_opt(deployment, "api_version_label")
                .unwrap_or_else(|| "local".to_string()),
            base_path: string_at_opt(deployment, "base_path").unwrap_or_else(|| "/".to_string()),
            alias: string_at_opt(deployment, "alias"),
        },
        exposure: ExposureConfig {
            default_visibility: string_at_opt(exposure, "default_visibility")
                .unwrap_or_else(|| "private".to_string()),
            require_validation_suite: bool_at(exposure, "require_validation_suite")
                .unwrap_or(false),
            auto_promote_on_validation_pass: bool_at(exposure, "auto_promote_on_validation_pass")
                .unwrap_or(false),
            public_aliases_allowed: string_array_at(exposure, "public_aliases_allowed")
                .unwrap_or_else(|| {
                    vec![
                        "stable".to_string(),
                        "latest".to_string(),
                        "preview".to_string(),
                    ]
                }),
        },
        ghcr: GhcrConfig {
            enable_publish_webhook: bool_at(ghcr, "enable_publish_webhook").unwrap_or(false),
            allowed_repositories: string_array_at(ghcr, "allowed_repositories")
                .unwrap_or_else(|| vec!["ghcr.io/greenticai/sorla-packages/*".to_string()]),
            require_exact_digest: bool_at(ghcr, "require_exact_digest").unwrap_or(true),
        },
        ghcr_webhook: GhcrWebhookAnswerConfig {
            enabled: bool_at(ghcr_webhook, "enabled").unwrap_or(false),
            bind: string_at_opt(ghcr_webhook, "bind")
                .unwrap_or_else(|| "127.0.0.1:8081".to_string()),
            public_path: string_at_opt(ghcr_webhook, "public_path")
                .unwrap_or_else(|| "/v1/sorx/webhooks/github/ghcr-published".to_string()),
            signature_secret_ref: string_at_opt(ghcr_webhook, "signature_secret_ref"),
            allowed_repositories: string_array_at(ghcr_webhook, "allowed_repositories")
                .unwrap_or_else(|| {
                    vec![
                        "greenticai/greentic-sorla".to_string(),
                        "greenticai/greentic-sorla-providers".to_string(),
                    ]
                }),
            allowed_oci_prefixes: string_array_at(ghcr_webhook, "allowed_oci_prefixes")
                .unwrap_or_else(|| {
                    vec![
                        "ghcr.io/greenticai/sorla/".to_string(),
                        "ghcr.io/greenticai/sorla-providers/".to_string(),
                    ]
                }),
            allowed_workflows: string_array_at(ghcr_webhook, "allowed_workflows").unwrap_or_else(
                || vec!["publish-gtpack.yml".to_string(), "publish.yml".to_string()],
            ),
            default_promotion_policy: string_at_opt(ghcr_webhook, "default_promotion_policy")
                .unwrap_or_else(|| "validate_then_private".to_string()),
        },
    })
}

pub fn build_startup_plan(
    pack_name: &str,
    pack_version: &str,
    answers: &Value,
) -> Result<Value, SorxStartupError> {
    let config = runtime_config_from_answers(pack_name, answers)?;
    let providers = config
        .providers
        .iter()
        .map(|(id, provider)| {
            json!({
                "id": id,
                "kind": provider.kind,
                "config_ref": provider.config_ref,
                "has_direct_config": provider.config.is_some(),
                "capabilities": provider.capabilities,
                "contract_version": provider.contract_version
            })
        })
        .collect::<Vec<_>>();
    let bindings = config
        .bindings
        .entity_bindings
        .values()
        .map(|binding| {
            json!({
                "entity": binding.entity,
                "provider": binding.provider_id,
                "collection": binding.collection
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "schema": "greentic.sorx.start.plan.v1",
        "pack": {
            "name": pack_name,
            "version": pack_version
        },
        "server": {
            "bind": config.server.bind,
            "public_base_url": config.server.public_base_url,
            "auth": {
                "mode": config.server.auth.mode,
                "shared_secret_configured": config.server.auth.shared_secret_ref.is_some()
            }
        },
        "mcp": {
            "enabled": config.mcp.enabled,
            "bind": config.mcp.bind
        },
        "providers": providers,
        "bindings": {
            "entities": bindings
        },
        "policy": config.policy,
        "audit": {
            "sink": config.audit.sink
        },
        "events": {
            "sink": config.events.sink,
            "nats_url": config.events.nats_url,
            "subject_prefix": config.events.subject_prefix
        },
        "deployment": config.deployment,
        "exposure": config.exposure,
        "ghcr": config.ghcr,
        "ghcr_webhook": config.ghcr_webhook
    }))
}

fn normalize_value(
    schema: &Value,
    value: &Value,
    path: &str,
    present: bool,
    issues: &mut Vec<SorxStartupIssue>,
) -> Value {
    if !present && let Some(default) = schema.get("default") {
        return default.clone();
    }

    match schema.get("type").and_then(Value::as_str) {
        Some("object") => normalize_object(schema, value, path, present, issues),
        Some("array") => normalize_array(schema, value, path, present, issues),
        Some(expected) => {
            if present {
                validate_scalar(schema, value, expected, path, issues);
                value.clone()
            } else {
                Value::Null
            }
        }
        None => {
            if present {
                value.clone()
            } else {
                Value::Null
            }
        }
    }
}

fn normalize_object(
    schema: &Value,
    value: &Value,
    path: &str,
    present: bool,
    issues: &mut Vec<SorxStartupIssue>,
) -> Value {
    let mut out = Map::new();
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();

    let input = if present {
        match value.as_object() {
            Some(object) => object,
            None => {
                issues.push(issue(path, "expected object"));
                return value.clone();
            }
        }
    } else {
        static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
        EMPTY.get_or_init(Map::new)
    };

    let additional_properties = schema.get("additionalProperties");

    for key in input.keys() {
        if !properties.contains_key(key) && additional_properties.is_none() {
            issues.push(issue(&join_path(path, key), "unknown startup answer"));
        }
    }

    for (key, child_schema) in &properties {
        let child_path = join_path(path, key);
        let child_present = input.contains_key(key);
        if !child_present
            && required.contains(&key.as_str())
            && child_schema.get("default").is_none()
        {
            issues.push(issue(&child_path, "required answer is missing"));
            continue;
        }
        let fallback = Value::Null;
        let child_value = input.get(key).unwrap_or(&fallback);
        let normalized = normalize_value(
            child_schema,
            child_value,
            &child_path,
            child_present,
            issues,
        );
        if !normalized.is_null() || child_present || child_schema.get("default").is_some() {
            out.insert(key.clone(), normalized);
        }
    }

    if let Some(additional_schema) = additional_properties {
        for (key, value) in input {
            if properties.contains_key(key) {
                continue;
            }
            if additional_schema == &Value::Bool(false) {
                issues.push(issue(&join_path(path, key), "unknown startup answer"));
            } else if additional_schema == &Value::Bool(true) {
                out.insert(key.clone(), value.clone());
            } else {
                let normalized = normalize_value(
                    additional_schema,
                    value,
                    &join_path(path, key),
                    true,
                    issues,
                );
                out.insert(key.clone(), normalized);
            }
        }
    }

    Value::Object(out)
}

fn normalize_array(
    schema: &Value,
    value: &Value,
    path: &str,
    present: bool,
    issues: &mut Vec<SorxStartupIssue>,
) -> Value {
    if !present {
        return schema.get("default").cloned().unwrap_or(Value::Null);
    }
    let Some(values) = value.as_array() else {
        issues.push(issue(path, "expected array"));
        return value.clone();
    };
    let Some(item_schema) = schema.get("items") else {
        return value.clone();
    };
    Value::Array(
        values
            .iter()
            .enumerate()
            .map(|(index, item)| {
                normalize_value(item_schema, item, &format!("{path}[{index}]"), true, issues)
            })
            .collect(),
    )
}

fn validate_scalar(
    schema: &Value,
    value: &Value,
    expected: &str,
    path: &str,
    issues: &mut Vec<SorxStartupIssue>,
) {
    let valid_type = match expected {
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "integer" => value.as_i64().is_some(),
        "number" => value.as_f64().is_some(),
        _ => true,
    };
    if !valid_type {
        issues.push(issue(path, format!("expected {expected}")));
        return;
    }
    if let Some(enum_values) = schema.get("enum").and_then(Value::as_array)
        && !enum_values.iter().any(|candidate| candidate == value)
    {
        issues.push(issue(path, "value is not in the allowed enum"));
    }
}

fn secret_issues(value: &Value, path: &str) -> Vec<SorxStartupIssue> {
    let mut issues = Vec::new();
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                let child_path = join_path(path, key);
                if is_secret_key(key)
                    && let Some(text) = value.as_str()
                    && !looks_like_secret_ref(text)
                {
                    issues.push(issue(
                        &child_path,
                        "secret-like answer must be a reference, not an inline value",
                    ));
                }
                issues.extend(secret_issues(value, &child_path));
            }
        }
        Value::Array(values) => {
            for (index, item) in values.iter().enumerate() {
                issues.extend(secret_issues(item, &format!("{path}[{index}]")));
            }
        }
        _ => {}
    }
    issues
}

fn provider_config_mode_issues(value: &Value) -> Vec<SorxStartupIssue> {
    let environment = value
        .get("tenant")
        .and_then(|tenant| tenant.get("environment"))
        .and_then(Value::as_str)
        .unwrap_or("local");
    if matches!(environment, "local" | "test") {
        return Vec::new();
    }

    value
        .get("providers")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|providers| providers.iter())
        .filter_map(|(provider_id, provider)| {
            provider.get("config")?;
            Some(issue(
                &format!("providers.{provider_id}.config"),
                "direct provider config is only allowed in local/test mode; use config_ref",
            ))
        })
        .collect()
}

fn is_secret_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower.contains("secret")
        || lower.contains("password")
        || lower.contains("access_token")
        || lower.contains("refresh_token")
        || lower == "token"
        || lower == "api_key"
        || lower == "private_key"
}

fn looks_like_secret_ref(value: &str) -> bool {
    value.starts_with("secret:")
        || value.starts_with("secrets.")
        || value.starts_with("env:")
        || value.starts_with("ref:")
        || value.starts_with("vault:")
        || value.starts_with("${")
}

fn provider_configs(object: &Map<String, Value>) -> BTreeMap<String, ProviderBindingConfig> {
    object
        .iter()
        .filter_map(|(id, value)| {
            let provider = value.as_object()?;
            let mut capabilities =
                string_array_at(Some(provider), "capabilities").unwrap_or_default();
            capabilities.sort();
            capabilities.dedup();
            Some((
                id.clone(),
                ProviderBindingConfig {
                    kind: string_at(provider, "kind").unwrap_or_default(),
                    config_ref: string_at(provider, "config_ref"),
                    config: provider.get("config").cloned(),
                    capabilities,
                    contract_version: string_at(provider, "contract_version"),
                },
            ))
        })
        .collect()
}

fn first_provider_id(object: &Map<String, Value>) -> String {
    object
        .keys()
        .next()
        .cloned()
        .unwrap_or_else(|| "store".to_string())
}

fn approval_policy(object: &Map<String, Value>) -> BTreeMap<String, String> {
    object
        .get("approvals")
        .and_then(Value::as_object)
        .map(|approvals| {
            approvals
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn object_value<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a Map<String, Value>, SorxStartupError> {
    object.get(key).and_then(Value::as_object).ok_or_else(|| {
        SorxStartupError::new(
            "invalid_answers",
            format!("normalized startup answers are missing object `{key}`"),
        )
    })
}

fn string_at(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key)?.as_str().map(ToString::to_string)
}

fn string_at_opt(object: Option<&Map<String, Value>>, key: &str) -> Option<String> {
    object?.get(key)?.as_str().map(ToString::to_string)
}

fn bool_at(object: Option<&Map<String, Value>>, key: &str) -> Option<bool> {
    object?.get(key)?.as_bool()
}

fn string_array_at(object: Option<&Map<String, Value>>, key: &str) -> Option<Vec<String>> {
    Some(
        object?
            .get(key)?
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect(),
    )
}

fn join_path(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_string()
    } else {
        format!("{parent}.{key}")
    }
}

fn issue(path: &str, message: impl Into<String>) -> SorxStartupIssue {
    SorxStartupIssue {
        path: path.to_string(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_answers() -> Value {
        json!({
            "tenant": {
                "tenant_id": "tenant-a"
            },
            "server": {
                "public_base_url": "http://127.0.0.1:8787"
            },
            "providers": {
                "store": {
                    "kind": "memory",
                    "config_ref": "providers.memory.local"
                }
            },
            "policy": {
                "approvals": {}
            },
            "audit": {},
            "deployment": {
                "tenant_id": "tenant-a",
                "sor_name": "landlord",
                "environment": "local"
            },
            "exposure": {},
            "ghcr": {}
        })
    }

    #[test]
    fn defaults_are_applied_to_answers() {
        let normalized =
            normalize_start_answers(&default_start_schema(), &full_answers(), true).unwrap();
        assert_eq!(normalized.answers["tenant"]["environment"], "local");
        assert_eq!(normalized.answers["server"]["bind"], "127.0.0.1:8787");
        assert_eq!(normalized.answers["server"]["auth"]["mode"], "none");
        assert_eq!(
            normalized.answers["policy"]["approvals"]["high"],
            "require_approval"
        );
        assert_eq!(normalized.answers["ghcr"]["require_exact_digest"], true);
    }

    #[test]
    fn foundationdb_store_is_defaulted_when_provider_answers_are_missing() {
        let answers = json!({
            "tenant": {
                "tenant_id": "tenant-a"
            },
            "server": {
                "public_base_url": "http://127.0.0.1:8787"
            },
            "policy": {
                "approvals": {}
            },
            "audit": {},
            "deployment": {
                "tenant_id": "tenant-a",
                "sor_name": "landlord",
                "environment": "local"
            },
            "exposure": {},
            "ghcr": {}
        });
        let normalized = normalize_start_answers(&default_start_schema(), &answers, true).unwrap();
        assert_eq!(
            normalized.answers["providers"]["store"]["kind"],
            "foundationdb"
        );
        assert_eq!(
            normalized.answers["providers"]["store"]["config_ref"],
            "providers.foundationdb.local"
        );
    }

    #[test]
    fn memory_store_answer_gets_memory_config_ref_default() {
        let mut answers = full_answers();
        answers["providers"]["store"] = json!({ "kind": "memory" });
        let normalized = normalize_start_answers(&default_start_schema(), &answers, true).unwrap();
        assert_eq!(normalized.answers["providers"]["store"]["kind"], "memory");
        assert_eq!(
            normalized.answers["providers"]["store"]["config_ref"],
            "providers.memory.local"
        );
    }

    #[test]
    fn missing_answers_are_reported_as_paths() {
        let answers = json!({
            "tenant": {},
            "server": {},
            "providers": {},
            "policy": {},
            "audit": {},
            "deployment": {},
            "exposure": {},
            "ghcr": {}
        });
        let err = normalize_start_answers(&default_start_schema(), &answers, true).unwrap_err();
        assert_eq!(err.code, "missing_answers");
        assert!(
            err.issues
                .iter()
                .any(|issue| issue.path == "tenant.tenant_id")
        );
        assert!(
            !err.issues
                .iter()
                .any(|issue| issue.path.starts_with("providers.store"))
        );
    }

    #[test]
    fn invalid_provider_kind_fails() {
        let mut answers = full_answers();
        answers["providers"]["store"]["kind"] = json!("postgres");
        let err = normalize_start_answers(&default_start_schema(), &answers, true).unwrap_err();
        assert_eq!(err.code, "invalid_answers");
        assert!(
            err.issues
                .iter()
                .any(|issue| issue.path == "providers.store.kind")
        );
    }

    #[test]
    fn invalid_approval_mode_fails() {
        let mut answers = full_answers();
        answers["policy"]["approvals"]["high"] = json!("maybe");
        let err = normalize_start_answers(&default_start_schema(), &answers, true).unwrap_err();
        assert_eq!(err.code, "invalid_answers");
        assert!(
            err.issues
                .iter()
                .any(|issue| issue.path == "policy.approvals.high")
        );
    }

    #[test]
    fn inline_secret_like_values_are_rejected() {
        let mut answers = full_answers();
        answers["providers"]["store"]["client_secret"] = json!("plain-secret-value");
        let err = normalize_start_answers(&default_start_schema(), &answers, true).unwrap_err();
        assert_eq!(err.code, "invalid_answers");
        assert!(
            err.issues
                .iter()
                .any(|issue| issue.path == "providers.store.client_secret")
        );
    }

    #[test]
    fn env_secret_refs_are_allowed_for_http_auth() {
        let mut answers = full_answers();
        answers["server"]["auth"] = json!({
            "mode": "shared_secret",
            "shared_secret_ref": "env:SORX_HTTP_INGEST_SECRET"
        });
        let normalized = normalize_start_answers(&default_start_schema(), &answers, true).unwrap();
        let config = runtime_config_from_answers("landlord", &normalized.answers).unwrap();
        assert_eq!(config.server.auth.mode, "shared_secret");
        assert_eq!(
            config.server.auth.shared_secret_ref.as_deref(),
            Some("env:SORX_HTTP_INGEST_SECRET")
        );
    }

    #[test]
    fn dry_run_plan_is_stable() {
        let normalized =
            normalize_start_answers(&default_start_schema(), &full_answers(), true).unwrap();
        let plan = build_startup_plan("landlord", "0.1.0", &normalized.answers).unwrap();
        assert_eq!(plan["schema"], "greentic.sorx.start.plan.v1");
        assert_eq!(plan["pack"]["name"], "landlord");
        assert_eq!(plan["providers"][0]["id"], "store");
    }

    #[test]
    fn qa_answer_set_envelope_is_accepted() {
        let envelope = json!({
            "form_id": "greentic.sorx.start",
            "spec_version": "0.1.0",
            "answers": full_answers()
        });
        let normalized = normalize_start_answers(&default_start_schema(), &envelope, true).unwrap();
        assert_eq!(normalized.answers["tenant"]["tenant_id"], "tenant-a");
        assert_eq!(normalized.answers["server"]["bind"], "127.0.0.1:8787");
    }

    #[test]
    fn events_config_defaults_to_disabled() {
        let normalized =
            normalize_start_answers(&default_start_schema(), &full_answers(), true).unwrap();
        let config = runtime_config_from_answers("pack", &normalized.answers).unwrap();
        assert_eq!(config.events.sink, "disabled");
        assert_eq!(config.events.nats_url, None);
        assert_eq!(config.events.subject_prefix, "greentic.events");
    }

    #[test]
    fn events_config_parses_nats_settings() {
        let mut answers = full_answers();
        answers["events"] = json!({
            "sink": "nats",
            "nats_url": "nats://localhost:4222",
            "subject_prefix": "custom.events"
        });
        let normalized = normalize_start_answers(&default_start_schema(), &answers, true).unwrap();
        let config = runtime_config_from_answers("pack", &normalized.answers).unwrap();
        assert_eq!(config.events.sink, "nats");
        assert_eq!(
            config.events.nats_url.as_deref(),
            Some("nats://localhost:4222")
        );
        assert_eq!(config.events.subject_prefix, "custom.events");
    }

    #[test]
    fn events_config_round_trips_through_answers() {
        let mut answers = full_answers();
        answers["events"] = json!({
            "sink": "stdout",
            "subject_prefix": "greentic.events"
        });
        let normalized = normalize_start_answers(&default_start_schema(), &answers, true).unwrap();
        let config = runtime_config_from_answers("pack", &normalized.answers).unwrap();
        let plan = build_startup_plan("pack", "0.1.0", &normalized.answers).unwrap();
        assert_eq!(plan["events"]["sink"], "stdout");
        assert_eq!(config.events.subject_prefix, "greentic.events");
    }
}
