use std::collections::BTreeMap;

use serde_json::Value;

use crate::{
    ApprovalRequirement, EndpointDefinition, EndpointMethod, OperationKind, RiskLevel, SorxError,
    SorxResult,
};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EndpointRouter {
    pub endpoints: BTreeMap<String, EndpointDefinition>,
}

impl EndpointRouter {
    pub fn new(endpoints: impl IntoIterator<Item = EndpointDefinition>) -> SorxResult<Self> {
        let mut by_id = BTreeMap::new();
        for endpoint in endpoints {
            if by_id
                .insert(endpoint.endpoint_id.clone(), endpoint)
                .is_some()
            {
                return Err(SorxError::new(
                    "duplicate_endpoint",
                    "endpoint router contains a duplicate endpoint id",
                ));
            }
        }
        Ok(Self { endpoints: by_id })
    }

    pub fn from_agent_gateway(gateway: &Value) -> SorxResult<Self> {
        Self::from_agent_gateway_with_options(gateway, false)
    }

    pub fn from_agent_gateway_strict(gateway: &Value) -> SorxResult<Self> {
        Self::from_agent_gateway_with_options(gateway, true)
    }

    fn from_agent_gateway_with_options(gateway: &Value, strict_risk: bool) -> SorxResult<Self> {
        let endpoints = gateway
            .get("endpoints")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                SorxError::new(
                    "invalid_gateway",
                    "agent gateway metadata must contain an endpoints array",
                )
            })?;

        let mut definitions = Vec::new();
        for (index, value) in endpoints.iter().enumerate() {
            definitions.push(parse_endpoint(value, index, strict_risk)?);
        }
        Self::new(definitions)
    }

    pub fn endpoint(&self, endpoint_id: &str) -> SorxResult<&EndpointDefinition> {
        self.endpoints.get(endpoint_id).ok_or_else(|| {
            SorxError::new(
                "unknown_endpoint",
                format!("unknown endpoint `{endpoint_id}`"),
            )
        })
    }
}

fn parse_endpoint(
    value: &Value,
    index: usize,
    strict_risk: bool,
) -> SorxResult<EndpointDefinition> {
    let path = format!("endpoints[{index}]");
    let object = value.as_object().ok_or_else(|| {
        SorxError::at_path(
            "invalid_gateway",
            "endpoint metadata must be an object",
            &path,
        )
    })?;
    let endpoint_id = string_field(value, "endpoint_id")
        .or_else(|| string_field(value, "id"))
        .ok_or_else(|| missing(&path, "endpoint_id"))?;
    let operation_id = string_field(value, "operation_id")
        .or_else(|| string_field(value, "operationId"))
        .unwrap_or_else(|| endpoint_id.clone());
    let operation = string_field(value, "operation")
        .and_then(|value| OperationKind::parse(&value))
        .or_else(|| infer_operation(&operation_id))
        .ok_or_else(|| {
            SorxError::at_path(
                "invalid_gateway",
                format!("cannot infer operation kind for `{operation_id}`"),
                &path,
            )
        })?;
    let method = string_field(value, "method")
        .and_then(|value| EndpointMethod::parse(&value))
        .ok_or_else(|| missing(&path, "method"))?;
    let route_path = string_field(value, "path").ok_or_else(|| missing(&path, "path"))?;
    let entity = string_field(value, "entity");
    let collection = string_field(value, "collection")
        .or_else(|| entity.as_ref().map(|entity| default_collection(entity)))
        .unwrap_or_else(|| "records".to_string());
    let provider_binding =
        string_field(value, "provider_binding").unwrap_or_else(|| "store".to_string());
    let risk_value = string_field(value, "risk");
    if strict_risk && risk_value.is_none() && operation.is_mutating() {
        return Err(SorxError::at_path(
            "risk_missing",
            "mutating endpoint metadata is missing risk",
            &path,
        ));
    }
    let risk = risk_value
        .and_then(|value| RiskLevel::parse(&value))
        .unwrap_or_else(|| {
            if operation.is_mutating() {
                RiskLevel::High
            } else {
                RiskLevel::Low
            }
        });
    let approval = parse_approval(value.get("approval"));

    Ok(EndpointDefinition {
        endpoint_id,
        operation_id,
        operation,
        method,
        path: route_path,
        entity,
        collection,
        provider_binding,
        risk,
        approval,
        input_schema: object.get("input_schema").cloned(),
        output_schema: object.get("output_schema").cloned(),
    })
}

fn parse_approval(value: Option<&Value>) -> Option<ApprovalRequirement> {
    let object = value?.as_object()?;
    Some(ApprovalRequirement {
        required: object
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        roles: object
            .get("roles")
            .and_then(Value::as_array)
            .map(|roles| {
                roles
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        reason_required: object
            .get("reason_required")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(ToString::to_string)
}

fn infer_operation(operation_id: &str) -> Option<OperationKind> {
    operation_id
        .rsplit(['.', ':', '_'])
        .next()
        .and_then(OperationKind::parse)
}

fn default_collection(entity: &str) -> String {
    format!("{}s", entity.to_ascii_lowercase())
}

fn missing(path: &str, field: &str) -> SorxError {
    SorxError::at_path(
        "invalid_gateway",
        format!("endpoint metadata is missing `{field}`"),
        path,
    )
}
