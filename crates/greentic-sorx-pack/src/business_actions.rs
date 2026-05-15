use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BusinessActionCatalog {
    pub schema: String,
    #[serde(default)]
    pub actions: Vec<BusinessAction>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BusinessAction {
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub execution: BusinessActionExecution,
    #[serde(default)]
    pub input_schema: Option<Value>,
    #[serde(default)]
    pub output_schema: Option<Value>,
    #[serde(default)]
    pub input_bindings: Vec<BusinessActionInputBinding>,
    #[serde(default)]
    pub risk: Option<BusinessActionRisk>,
    #[serde(default)]
    pub approval: Option<BusinessActionApproval>,
    #[serde(default)]
    pub idempotency: Option<BusinessActionIdempotency>,
    #[serde(default)]
    pub designer: Option<Value>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusinessActionRef {
    pub id: String,
    pub version: String,
    pub contract_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BusinessActionExecution {
    #[serde(default)]
    pub endpoint_id: Option<String>,
    #[serde(default)]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BusinessActionLock {
    pub schema: String,
    #[serde(default)]
    pub entries: Vec<BusinessActionLockEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusinessActionLockEntry {
    pub id: String,
    pub version: String,
    pub contract_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusinessActionContract {
    pub id: String,
    pub version: String,
    pub contract_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BusinessActionInputBinding {
    pub name: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusinessActionRisk {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusinessActionApproval {
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusinessActionIdempotency {
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BusinessActionInspectSummary {
    pub present: bool,
    pub count: usize,
    pub lock_present: bool,
    pub hashes_valid: bool,
    pub execution_targets_valid: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BusinessActionAssets {
    pub catalog: BusinessActionCatalog,
    pub lock: Option<BusinessActionLock>,
    pub hashes_valid: bool,
    pub execution_targets_valid: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BusinessActionValidationContext {
    pub endpoint_ids: BTreeSet<String>,
    pub operation_ids: BTreeSet<String>,
    pub tool_names: BTreeSet<String>,
}

impl BusinessActionValidationContext {
    pub fn from_agent_gateway_and_mcp_tools(
        agent_gateway: &Value,
        mcp_tools: Option<&Value>,
    ) -> Self {
        let endpoints = agent_gateway
            .get("endpoints")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let endpoint_ids = endpoints
            .iter()
            .filter_map(|endpoint| {
                endpoint
                    .get("endpoint_id")
                    .or_else(|| endpoint.get("id"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
            .collect();
        let operation_ids = endpoints
            .iter()
            .filter_map(|endpoint| {
                endpoint
                    .get("operation_id")
                    .or_else(|| endpoint.get("operationId"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
            .collect();
        let tool_names = mcp_tools
            .and_then(|value| value.get("tools"))
            .and_then(Value::as_array)
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|tool| tool.get("name").and_then(Value::as_str))
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            endpoint_ids,
            operation_ids,
            tool_names,
        }
    }
}

impl BusinessActionAssets {
    pub fn inspect_summary(&self) -> BusinessActionInspectSummary {
        BusinessActionInspectSummary {
            present: true,
            count: self.catalog.actions.len(),
            lock_present: self.lock.is_some(),
            hashes_valid: self.hashes_valid,
            execution_targets_valid: self.execution_targets_valid,
        }
    }
}

pub fn validate_business_actions(
    catalog: BusinessActionCatalog,
    lock: Option<BusinessActionLock>,
    context: &BusinessActionValidationContext,
) -> (BusinessActionAssets, Vec<String>) {
    let mut errors = Vec::new();
    if catalog.schema != "greentic.sorla.business-actions.v1" {
        errors.push("assets/sorla/business-actions.json has unsupported schema".to_string());
    }

    let mut action_keys = BTreeSet::new();
    let mut actions_by_key = BTreeMap::new();
    let mut execution_targets_valid = true;
    for action in &catalog.actions {
        let key = (action.id.clone(), action.version.clone());
        if !action_keys.insert(key.clone()) {
            errors.push(format!(
                "duplicate business action id/version `{}` `{}`",
                action.id, action.version
            ));
        }
        actions_by_key.insert(key, action);
        if !valid_execution_target(action, context) {
            execution_targets_valid = false;
            errors.push(format!(
                "business action `{}` version `{}` references an unknown execution target",
                action.id, action.version
            ));
        }
        if let Some(input_schema) = &action.input_schema
            && !input_schema.is_object()
        {
            errors.push(format!(
                "business action `{}` version `{}` input_schema must be an object",
                action.id, action.version
            ));
        }
        if let Some(output_schema) = &action.output_schema
            && !output_schema.is_object()
        {
            errors.push(format!(
                "business action `{}` version `{}` output_schema must be an object",
                action.id, action.version
            ));
        }
        if action.execution.endpoint_id.is_none()
            && action.execution.operation_id.is_none()
            && action.execution.tool_name.is_none()
        {
            errors.push(format!(
                "business action `{}` version `{}` must define an execution target",
                action.id, action.version
            ));
        }
    }

    if lock.is_none() {
        errors.push(
            "assets/sorla/business-actions.lock.json is required when business actions are present"
                .to_string(),
        );
    }

    let mut hashes_valid = true;
    if let Some(lock) = &lock {
        if lock.schema != "greentic.sorla.business-actions.lock.v1" {
            errors
                .push("assets/sorla/business-actions.lock.json has unsupported schema".to_string());
        }
        let mut lock_keys = BTreeSet::new();
        for entry in &lock.entries {
            let key = (entry.id.clone(), entry.version.clone());
            if !lock_keys.insert(key.clone()) {
                errors.push(format!(
                    "duplicate business action lock entry `{}` `{}`",
                    entry.id, entry.version
                ));
            }
            let Some(action) = actions_by_key.get(&key) else {
                errors.push(format!(
                    "business action lock references unknown action `{}` version `{}`",
                    entry.id, entry.version
                ));
                continue;
            };
            let actual = contract_hash(action);
            if entry.contract_hash != actual {
                hashes_valid = false;
                errors.push(format!(
                    "business action `{}` version `{}` contract hash mismatch",
                    action.id, action.version
                ));
            }
        }
        for key in action_keys {
            if !lock_keys.contains(&key) {
                errors.push(format!(
                    "business action `{}` version `{}` is missing a lock entry",
                    key.0, key.1
                ));
            }
        }
    }

    errors.extend(secret_warnings_as_errors(&catalog, lock.as_ref()));

    (
        BusinessActionAssets {
            catalog,
            lock,
            hashes_valid,
            execution_targets_valid,
        },
        errors,
    )
}

pub fn contract_hash(action: &BusinessAction) -> String {
    let canonical = json!({
        "id": action.id,
        "version": action.version,
        "execution": action.execution,
        "input_schema": action.input_schema,
        "output_schema": action.output_schema,
        "input_bindings": action.input_bindings,
        "risk": action.risk,
        "approval": action.approval,
        "idempotency": action.idempotency,
    });
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

fn valid_execution_target(
    action: &BusinessAction,
    context: &BusinessActionValidationContext,
) -> bool {
    action
        .execution
        .endpoint_id
        .as_ref()
        .is_some_and(|endpoint_id| context.endpoint_ids.contains(endpoint_id))
        || action
            .execution
            .operation_id
            .as_ref()
            .is_some_and(|operation_id| context.operation_ids.contains(operation_id))
        || action
            .execution
            .tool_name
            .as_ref()
            .is_some_and(|tool_name| context.tool_names.contains(tool_name))
}

fn secret_warnings_as_errors(
    catalog: &BusinessActionCatalog,
    lock: Option<&BusinessActionLock>,
) -> Vec<String> {
    let mut errors = Vec::new();
    let catalog_value = serde_json::to_value(catalog).unwrap_or(Value::Null);
    collect_secret_markers(
        "assets/sorla/business-actions.json",
        &catalog_value,
        &mut errors,
    );
    if let Some(lock) = lock {
        let lock_value = serde_json::to_value(lock).unwrap_or(Value::Null);
        collect_secret_markers(
            "assets/sorla/business-actions.lock.json",
            &lock_value,
            &mut errors,
        );
    }
    errors
}

fn collect_secret_markers(name: &str, value: &Value, errors: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            for marker in [
                "BEGIN PRIVATE KEY",
                "api_key:",
                "access_token:",
                "refresh_token:",
                "client_secret:",
                "password:",
            ] {
                if text.contains(marker) {
                    errors.push(format!("`{name}` contains secret-like marker `{marker}`"));
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_secret_markers(name, value, errors);
            }
        }
        Value::Object(map) => {
            for value in map.values() {
                collect_secret_markers(name, value, errors);
            }
        }
        _ => {}
    }
}
