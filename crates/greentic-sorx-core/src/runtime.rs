use std::sync::Arc;
use std::time::Instant;

use serde_json::{Value, json};

use crate::{
    ApprovalBroker, ApprovalRequest, ApprovalStatus, AuditSink, CreateOp, DeleteOp,
    DisabledAuditSink, EndpointDefinition, EndpointInvocation, EndpointResult, EndpointRouter,
    EndpointStatus, GetOp, IndexQueryOp, LocalPendingBroker, OperationKind, PolicyAction,
    PolicyConfig, PolicyDecision, PolicyEngine, ProviderNamespace, ProviderRegistry, QueryOp,
    RuntimePack, SorxAuditEvent, SorxError, SorxEvent, SorxResult, SorxRuntimeConfig, TraverseOp,
    UpdateOp, ViewTransform,
};

#[derive(Clone)]
pub struct SorxRuntime {
    pub pack: RuntimePack,
    pub config: SorxRuntimeConfig,
    pub router: EndpointRouter,
    pub providers: ProviderRegistry,
    pub policy: PolicyEngine,
    approval_broker: Arc<dyn ApprovalBroker>,
    audit_sink: Arc<dyn AuditSink>,
}

impl SorxRuntime {
    pub fn new(
        pack: RuntimePack,
        config: SorxRuntimeConfig,
        router: EndpointRouter,
        providers: ProviderRegistry,
    ) -> Self {
        let policy = PolicyEngine::new(PolicyConfig::from_modes(&config.policy));
        Self {
            pack,
            config,
            router,
            providers,
            policy,
            approval_broker: Arc::new(LocalPendingBroker),
            audit_sink: Arc::new(DisabledAuditSink),
        }
    }

    pub fn with_policy(mut self, policy: PolicyEngine) -> Self {
        self.policy = policy;
        self
    }

    pub fn with_approval_broker(mut self, approval_broker: Arc<dyn ApprovalBroker>) -> Self {
        self.approval_broker = approval_broker;
        self
    }

    pub fn with_audit_sink(mut self, audit_sink: Arc<dyn AuditSink>) -> Self {
        self.audit_sink = audit_sink;
        self
    }

    pub fn invoke(&self, mut invocation: EndpointInvocation) -> SorxResult<EndpointResult> {
        let started = Instant::now();
        let endpoint = self.router.endpoint(&invocation.endpoint_id)?;
        self.audit(endpoint, &invocation, "sorx.endpoint.invoked", None, None)?;
        if endpoint.operation_id != invocation.operation_id {
            let err = SorxError::new(
                "operation_mismatch",
                format!(
                    "endpoint `{}` maps to operation `{}`, not `{}`",
                    endpoint.endpoint_id, endpoint.operation_id, invocation.operation_id
                ),
            );
            self.audit(endpoint, &invocation, "sorx.endpoint.failed", None, None)?;
            return Err(err);
        }
        if endpoint.view.read_only && endpoint.operation.is_mutating() {
            let err = SorxError::new(
                "view_read_only",
                format!(
                    "endpoint `{}` is read-only in this view",
                    endpoint.endpoint_id
                ),
            );
            self.audit(endpoint, &invocation, "sorx.endpoint.failed", None, None)?;
            return Err(err);
        }
        invocation.input = view_to_canonical(&endpoint.view, invocation.input);
        if let Err(err) = validate_input(endpoint, &invocation.input) {
            self.audit(endpoint, &invocation, "sorx.endpoint.failed", None, None)?;
            return Err(err);
        }
        let policy_decision = self.policy.decide(endpoint);
        self.audit(
            endpoint,
            &invocation,
            "sorx.policy.decided",
            Some(policy_decision_label(&policy_decision)),
            None,
        )?;
        match policy_decision.action {
            PolicyAction::Execute => {}
            PolicyAction::Deny => {
                let result = EndpointResult {
                    status: EndpointStatus::Denied,
                    output: json!({
                        "status": "denied",
                        "risk": format!("{:?}", endpoint.risk).to_ascii_lowercase(),
                        "reason": policy_decision.reason
                    }),
                    events: vec![event(endpoint, &invocation, "policy.denied")],
                };
                self.audit(
                    endpoint,
                    &invocation,
                    "sorx.endpoint.completed",
                    Some("denied"),
                    Some(started.elapsed().as_millis() as u64),
                )?;
                return Ok(result);
            }
            PolicyAction::RequireApproval => {
                let approval = self.approval_broker.decide(ApprovalRequest {
                    request_id: approval_request_id(endpoint, &invocation),
                    tenant_id: invocation.tenant_id.clone(),
                    endpoint_id: endpoint.endpoint_id.clone(),
                    operation_id: endpoint.operation_id.clone(),
                    risk: endpoint.risk,
                    reason: policy_decision.reason.clone(),
                    caller: invocation.caller.clone(),
                })?;
                self.audit(
                    endpoint,
                    &invocation,
                    "sorx.approval.requested",
                    Some(approval_status_label(approval.status)),
                    None,
                )?;
                match approval.status {
                    ApprovalStatus::Approved => {}
                    ApprovalStatus::Denied => {
                        let result = EndpointResult {
                            status: EndpointStatus::Denied,
                            output: json!({
                                "status": "denied",
                                "approval": approval
                            }),
                            events: vec![event(endpoint, &invocation, "approval.denied")],
                        };
                        self.audit(
                            endpoint,
                            &invocation,
                            "sorx.endpoint.completed",
                            Some("denied"),
                            Some(started.elapsed().as_millis() as u64),
                        )?;
                        return Ok(result);
                    }
                    ApprovalStatus::Pending => {
                        let result = EndpointResult {
                            status: EndpointStatus::ApprovalRequired,
                            output: json!({
                                "status": "approval_required",
                                "approval": {
                                    "request_id": approval.request_id,
                                    "risk": format!("{:?}", endpoint.risk).to_ascii_lowercase(),
                                    "reason": approval.reason
                                }
                            }),
                            events: vec![event(endpoint, &invocation, "approval.requested")],
                        };
                        self.audit(
                            endpoint,
                            &invocation,
                            "sorx.endpoint.completed",
                            Some("approval_required"),
                            Some(started.elapsed().as_millis() as u64),
                        )?;
                        return Ok(result);
                    }
                }
            }
        }

        let binding = self.config.bindings.resolve(endpoint)?;
        let provider = self.providers.store(&binding.provider_id)?;
        let namespace = ProviderNamespace {
            tenant_id: invocation.tenant_id.clone(),
            sor_name: self.config.deployment.sor_name.clone(),
        };

        self.audit(
            endpoint,
            &invocation,
            "sorx.provider.operation.started",
            None,
            None,
        )?;
        let result = match endpoint.operation {
            OperationKind::Create => {
                let record = provider.create(CreateOp {
                    namespace: namespace.clone(),
                    entity: binding.entity.clone(),
                    collection: binding.collection.clone(),
                    input: invocation.input.clone(),
                    idempotency_key: invocation
                        .idempotency_key
                        .as_ref()
                        .map(|key| format!("{}:{key}", endpoint.operation_id)),
                })?;
                EndpointResult {
                    status: EndpointStatus::Created,
                    output: canonical_to_view(
                        &endpoint.view,
                        serde_json::to_value(record)
                            .map_err(|err| SorxError::new("encode_failed", err.to_string()))?,
                    ),
                    events: vec![event(endpoint, &invocation, "entity.created")],
                }
            }
            OperationKind::Get => {
                let id = required_string(&invocation.input, "id")?;
                let record = provider.get(GetOp {
                    namespace: namespace.clone(),
                    entity: binding.entity.clone(),
                    collection: binding.collection.clone(),
                    id,
                })?;
                EndpointResult {
                    status: if record.is_some() {
                        EndpointStatus::Ok
                    } else {
                        EndpointStatus::NotFound
                    },
                    output: canonical_to_view(
                        &endpoint.view,
                        serde_json::to_value(record)
                            .map_err(|err| SorxError::new("encode_failed", err.to_string()))?,
                    ),
                    events: vec![event(endpoint, &invocation, "entity.get")],
                }
            }
            OperationKind::Update => {
                let id = required_string(&invocation.input, "id")?;
                let patch = invocation
                    .input
                    .get("patch")
                    .cloned()
                    .unwrap_or_else(|| invocation.input.clone());
                let record = provider.update(UpdateOp {
                    namespace: namespace.clone(),
                    entity: binding.entity.clone(),
                    collection: binding.collection.clone(),
                    id,
                    patch,
                })?;
                EndpointResult {
                    status: EndpointStatus::Ok,
                    output: canonical_to_view(
                        &endpoint.view,
                        serde_json::to_value(record)
                            .map_err(|err| SorxError::new("encode_failed", err.to_string()))?,
                    ),
                    events: vec![event(endpoint, &invocation, "entity.updated")],
                }
            }
            OperationKind::Query => {
                let filter = invocation
                    .input
                    .get("filter")
                    .cloned()
                    .unwrap_or_else(|| invocation.input.clone());
                let output = if let Some(index) = &endpoint.query_plan.index {
                    let canonical = self.providers.canonical_store(&binding.provider_id)?;
                    serde_json::to_value(canonical.query_index(IndexQueryOp {
                        namespace: namespace.clone(),
                        entity: binding.entity.clone(),
                        collection: binding.collection.clone(),
                        index: index.name.clone(),
                        filter,
                    })?)
                    .map_err(|err| SorxError::new("encode_failed", err.to_string()))?
                } else if let Some(traversal) = &endpoint.query_plan.traversal {
                    let canonical = self.providers.canonical_store(&binding.provider_id)?;
                    let root_id = required_string(&invocation.input, "id")?;
                    serde_json::to_value(canonical.traverse(TraverseOp {
                        namespace: namespace.clone(),
                        root_entity: binding.entity.clone(),
                        root_collection: binding.collection.clone(),
                        root_id,
                        max_depth: traversal.max_depth,
                        relationships: traversal.relationships.clone(),
                    })?)
                    .map_err(|err| SorxError::new("encode_failed", err.to_string()))?
                } else {
                    serde_json::to_value(provider.query(QueryOp {
                        namespace: namespace.clone(),
                        entity: binding.entity.clone(),
                        collection: binding.collection.clone(),
                        filter,
                    })?)
                    .map_err(|err| SorxError::new("encode_failed", err.to_string()))?
                };
                EndpointResult {
                    status: EndpointStatus::Ok,
                    output: canonical_to_view(&endpoint.view, output),
                    events: vec![event(endpoint, &invocation, "entity.queried")],
                }
            }
            OperationKind::Delete => {
                let id = required_string(&invocation.input, "id")?;
                let deleted = provider.delete(DeleteOp {
                    namespace,
                    entity: binding.entity.clone(),
                    collection: binding.collection.clone(),
                    id,
                })?;
                EndpointResult {
                    status: if deleted.deleted {
                        EndpointStatus::Deleted
                    } else {
                        EndpointStatus::NotFound
                    },
                    output: serde_json::to_value(deleted)
                        .map_err(|err| SorxError::new("encode_failed", err.to_string()))?,
                    events: vec![event(endpoint, &invocation, "entity.deleted")],
                }
            }
        };
        self.audit(
            endpoint,
            &invocation,
            "sorx.provider.operation.completed",
            None,
            None,
        )?;
        self.audit(
            endpoint,
            &invocation,
            "sorx.endpoint.completed",
            Some("executed"),
            Some(started.elapsed().as_millis() as u64),
        )?;
        Ok(result)
    }

    fn audit(
        &self,
        endpoint: &EndpointDefinition,
        invocation: &EndpointInvocation,
        event_name: &str,
        decision: Option<&str>,
        duration_ms: Option<u64>,
    ) -> SorxResult<()> {
        self.audit_sink.emit(SorxAuditEvent {
            event: event_name.to_string(),
            pack: self.pack.name.clone(),
            version: self.pack.version.clone(),
            tenant_id: invocation.tenant_id.clone(),
            endpoint_id: endpoint.endpoint_id.clone(),
            operation_id: endpoint.operation_id.clone(),
            risk: endpoint.risk,
            caller_id: invocation.caller.subject.clone(),
            decision: decision.map(ToString::to_string),
            duration_ms,
            idempotency_key_present: invocation.idempotency_key.is_some(),
            details: serde_json::Map::from_iter([(
                "source".to_string(),
                json!(format!("{:?}", invocation.source).to_ascii_lowercase()),
            )]),
        })
    }
}

fn validate_input(endpoint: &EndpointDefinition, input: &Value) -> SorxResult<()> {
    let Some(schema) = &endpoint.input_schema else {
        return Ok(());
    };
    validate_schema(schema, input, "")
}

fn view_to_canonical(view: &ViewTransform, input: Value) -> Value {
    rename_object_fields(input, &view.input_field_map)
}

fn canonical_to_view(view: &ViewTransform, output: Value) -> Value {
    if view.output_field_map.is_empty() {
        return output;
    }
    match output {
        Value::Object(mut object) => {
            if let Some(data) = object.remove("data") {
                object.insert(
                    "data".to_string(),
                    rename_object_fields(data, &view.output_field_map),
                );
            }
            if let Some(Value::Array(records)) = object.get_mut("records") {
                for record in records {
                    if let Value::Object(record_object) = record
                        && let Some(data) = record_object.remove("data")
                    {
                        record_object.insert(
                            "data".to_string(),
                            rename_object_fields(data, &view.output_field_map),
                        );
                    }
                }
            }
            Value::Object(object)
        }
        other => other,
    }
}

fn rename_object_fields(
    value: Value,
    mapping: &std::collections::BTreeMap<String, String>,
) -> Value {
    if mapping.is_empty() {
        return value;
    }
    let Value::Object(object) = value else {
        return value;
    };
    let mut renamed = serde_json::Map::new();
    for (key, value) in object {
        let mapped = mapping.get(&key).cloned().unwrap_or(key);
        renamed.insert(mapped, value);
    }
    Value::Object(renamed)
}

fn validate_schema(schema: &Value, value: &Value, path: &str) -> SorxResult<()> {
    if let Some(expected) = schema.get("type").and_then(Value::as_str) {
        let valid = match expected {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "boolean" => value.is_boolean(),
            "integer" => value.as_i64().is_some(),
            "number" => value.as_f64().is_some(),
            _ => true,
        };
        if !valid {
            return Err(SorxError::at_path(
                "invalid_input",
                format!("expected {expected}"),
                display_path(path),
            ));
        }
    }

    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for key in required.iter().filter_map(Value::as_str) {
            if value.get(key).is_none() {
                return Err(SorxError::at_path(
                    "invalid_input",
                    format!("missing required input `{key}`"),
                    join_path(path, key),
                ));
            }
        }
    }

    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        for (key, child_schema) in properties {
            if let Some(child) = value.get(key) {
                validate_schema(child_schema, child, &join_path(path, key))?;
            }
        }
    }

    if let Some(enum_values) = schema.get("enum").and_then(Value::as_array)
        && !enum_values.iter().any(|candidate| candidate == value)
    {
        return Err(SorxError::at_path(
            "invalid_input",
            "input value is not in the allowed enum",
            display_path(path),
        ));
    }

    Ok(())
}

fn required_string(value: &Value, key: &str) -> SorxResult<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| {
            SorxError::at_path(
                "invalid_input",
                format!("missing required string `{key}`"),
                key,
            )
        })
}

fn event(
    endpoint: &EndpointDefinition,
    invocation: &EndpointInvocation,
    event_type: &str,
) -> SorxEvent {
    SorxEvent {
        schema: "greentic.sorx.event.v1".to_string(),
        event_type: event_type.to_string(),
        tenant_id: invocation.tenant_id.clone(),
        endpoint_id: endpoint.endpoint_id.clone(),
        operation_id: endpoint.operation_id.clone(),
        provider_binding: endpoint.provider_binding.clone(),
    }
}

fn join_path(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_string()
    } else {
        format!("{parent}.{key}")
    }
}

fn display_path(path: &str) -> String {
    if path.is_empty() {
        "$".to_string()
    } else {
        path.to_string()
    }
}

fn approval_request_id(endpoint: &EndpointDefinition, invocation: &EndpointInvocation) -> String {
    let key = invocation.idempotency_key.as_deref().unwrap_or("pending");
    format!(
        "approval_{}_{}_{}",
        invocation.tenant_id.replace('-', "_"),
        endpoint.endpoint_id.replace(['.', '-'], "_"),
        key.replace(['.', '-', ':'], "_")
    )
}

fn policy_decision_label(decision: &PolicyDecision) -> &'static str {
    match decision.action {
        PolicyAction::Execute => "execute",
        PolicyAction::RequireApproval => "require_approval",
        PolicyAction::Deny => "deny",
    }
}

fn approval_status_label(status: ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Approved => "approved",
        ApprovalStatus::Denied => "denied",
        ApprovalStatus::Pending => "pending",
    }
}

pub fn runtime_pack(name: impl Into<String>, version: impl Into<String>) -> RuntimePack {
    RuntimePack {
        name: name.into(),
        version: version.into(),
        digest: None,
    }
}

pub fn invocation(
    tenant_id: impl Into<String>,
    endpoint_id: impl Into<String>,
    operation_id: impl Into<String>,
    input: Value,
) -> EndpointInvocation {
    EndpointInvocation {
        tenant_id: tenant_id.into(),
        endpoint_id: endpoint_id.into(),
        operation_id: operation_id.into(),
        input,
        caller: crate::CallerContext {
            subject: "test".to_string(),
            roles: vec!["admin".to_string()],
        },
        idempotency_key: None,
        source: crate::InvocationSource::Direct,
    }
}

pub fn empty_object() -> Value {
    json!({})
}
