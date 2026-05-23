use std::collections::BTreeMap;

use serde_json::Value;

use crate::{
    ApprovalRequirement, CommandSpec, CommandStep, EndpointDefinition, EndpointMethod,
    IndexRequirement, OperationKind, QueryPlan, RiskLevel, SorxError, SorxResult,
    TraversalRequirement, ViewTransform,
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
    let mut operation = string_field(value, "operation")
        .and_then(|value| OperationKind::parse(&value))
        .or_else(|| infer_operation(&operation_id))
        .ok_or_else(|| {
            SorxError::at_path(
                "invalid_gateway",
                format!("cannot infer operation kind for `{operation_id}`"),
                &path,
            )
        })?;
    if matches!(operation, OperationKind::Command(_)) {
        operation = OperationKind::Command(parse_command_spec(value.get("command"), &path)?);
    }
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
    let view = parse_view_transform(value.get("view"));
    let query_plan = parse_query_plan(value.get("requires").or_else(|| value.get("query_plan")));

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
        view,
        query_plan,
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

fn parse_view_transform(value: Option<&Value>) -> ViewTransform {
    let Some(object) = value.and_then(Value::as_object) else {
        return ViewTransform::default();
    };
    ViewTransform {
        read_only: object
            .get("read_only")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        input_field_map: string_map(object.get("view_to_canonical")),
        output_field_map: string_map(object.get("canonical_to_view")),
    }
}

fn parse_query_plan(value: Option<&Value>) -> QueryPlan {
    let Some(object) = value.and_then(Value::as_object) else {
        return QueryPlan::default();
    };
    QueryPlan {
        index: object.get("index").and_then(parse_index_requirement),
        traversal: object
            .get("traversal")
            .and_then(parse_traversal_requirement),
    }
}

fn parse_command_spec(value: Option<&Value>, path: &str) -> SorxResult<CommandSpec> {
    let Some(value) = value else {
        return Ok(CommandSpec::default());
    };
    let object = value
        .as_object()
        .ok_or_else(|| SorxError::at_path("invalid_gateway", "command must be an object", path))?;
    let steps_value = object
        .get("steps")
        .or_else(|| object.get("effects"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut steps = Vec::new();
    for (index, step) in steps_value.iter().enumerate() {
        steps.push(parse_command_step(
            step,
            &format!("{path}.command.steps[{index}]"),
        )?);
    }
    Ok(CommandSpec {
        kind: string_field(value, "kind"),
        action: string_field(value, "action"),
        target: string_field(value, "target"),
        idempotency: string_field(value, "idempotency"),
        steps,
        return_value: object
            .get("return")
            .or_else(|| object.get("output"))
            .cloned(),
    })
}

fn parse_command_step(value: &Value, path: &str) -> SorxResult<CommandStep> {
    let object = value.as_object().ok_or_else(|| {
        SorxError::at_path("invalid_gateway", "command step must be an object", path)
    })?;
    let op = string_field(value, "op")
        .or_else(|| string_field(value, "type"))
        .ok_or_else(|| {
            SorxError::at_path("invalid_gateway", "command step is missing op/type", path)
        })?;
    match op.as_str() {
        "create" => Ok(CommandStep::Create {
            as_name: string_field(value, "as"),
            entity: string_field(value, "entity"),
            collection: string_field(value, "collection"),
            input: object
                .get("input")
                .or_else(|| object.get("values"))
                .cloned(),
        }),
        "delete_where" => Ok(CommandStep::DeleteWhere {
            as_name: string_field(value, "as"),
            entity: string_field(value, "entity"),
            collection: string_field(value, "collection"),
            r#where: object
                .get("where")
                .or_else(|| object.get("filter"))
                .cloned()
                .unwrap_or_else(|| Value::Object(Default::default())),
        }),
        "update_where" => Ok(CommandStep::UpdateWhere {
            as_name: string_field(value, "as"),
            entity: string_field(value, "entity"),
            collection: string_field(value, "collection"),
            r#where: object
                .get("where")
                .or_else(|| object.get("filter"))
                .cloned()
                .unwrap_or_else(|| Value::Object(Default::default())),
            set: object
                .get("set")
                .or_else(|| object.get("patch"))
                .cloned()
                .unwrap_or_else(|| Value::Object(Default::default())),
        }),
        "query" => Ok(CommandStep::Query {
            as_name: string_field(value, "as"),
            entity: string_field(value, "entity"),
            collection: string_field(value, "collection"),
            r#where: object
                .get("where")
                .or_else(|| object.get("filter"))
                .cloned()
                .unwrap_or_else(|| Value::Object(Default::default())),
        }),
        "find_one" | "query_one" => Ok(CommandStep::FindOne {
            as_name: string_field(value, "as"),
            entity: string_field(value, "entity"),
            collection: string_field(value, "collection"),
            r#where: object
                .get("where")
                .or_else(|| object.get("filter"))
                .cloned()
                .unwrap_or_else(|| Value::Object(Default::default())),
            required: object
                .get("required")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }),
        "emit_event" => {
            let event = string_field(value, "event").ok_or_else(|| {
                SorxError::at_path("invalid_gateway", "emit_event step is missing event", path)
            })?;
            Ok(CommandStep::EmitEvent {
                as_name: string_field(value, "as"),
                event,
                payload: object
                    .get("payload")
                    .or_else(|| object.get("data"))
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Default::default())),
                stream: string_field(value, "stream"),
            })
        }
        "foreach" => {
            let items = object.get("items").cloned().ok_or_else(|| {
                SorxError::at_path("invalid_gateway", "foreach step is missing items", path)
            })?;
            let nested = object
                .get("do")
                .or_else(|| object.get("steps"))
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    SorxError::at_path("invalid_gateway", "foreach step is missing do steps", path)
                })?;
            let mut steps = Vec::new();
            for (index, step) in nested.iter().enumerate() {
                steps.push(parse_command_step(step, &format!("{path}.do[{index}]"))?);
            }
            Ok(CommandStep::Foreach {
                as_name: string_field(value, "as"),
                items,
                steps,
            })
        }
        _ => Err(SorxError::at_path(
            "invalid_gateway",
            format!("unsupported command step `{op}`"),
            path,
        )),
    }
}

fn parse_index_requirement(value: &Value) -> Option<IndexRequirement> {
    if let Some(name) = value.as_str() {
        return Some(IndexRequirement {
            name: name.to_string(),
            capability: "exact-index-query".to_string(),
        });
    }
    let object = value.as_object()?;
    Some(IndexRequirement {
        name: object.get("name")?.as_str()?.to_string(),
        capability: object
            .get("capability")
            .and_then(Value::as_str)
            .unwrap_or("exact-index-query")
            .to_string(),
    })
}

fn parse_traversal_requirement(value: &Value) -> Option<TraversalRequirement> {
    if let Some(name) = value.as_str() {
        return Some(TraversalRequirement {
            name: name.to_string(),
            capability: "bounded-graph-traversal".to_string(),
            max_depth: 2,
            relationships: Vec::new(),
        });
    }
    let object = value.as_object()?;
    Some(TraversalRequirement {
        name: object.get("name")?.as_str()?.to_string(),
        capability: object
            .get("capability")
            .and_then(Value::as_str)
            .unwrap_or("bounded-graph-traversal")
            .to_string(),
        max_depth: object
            .get("max_depth")
            .and_then(Value::as_u64)
            .unwrap_or(2)
            .min(u8::MAX as u64) as u8,
        relationships: object
            .get("relationships")
            .and_then(Value::as_array)
            .map(|relationships| {
                relationships
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

fn string_map(value: Option<&Value>) -> BTreeMap<String, String> {
    value
        .and_then(Value::as_object)
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
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
