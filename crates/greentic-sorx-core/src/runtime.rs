use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::{
    AdminActionContext, AdminActionRequest, AdminActionResponse, AdminObserverEvent, AppendEventOp,
    ApprovalBroker, ApprovalRequest, ApprovalStatus, AuditSink, AuthorizationPolicyInput,
    AuthorizationPolicyResource, AuthorizationRequirement, CommandOrderDirection, CommandSpec,
    CommandStep, ControlDecision, ControlDecisionAction, ControlHook, CreateOp, DeleteOp,
    DisabledAuditSink, EndpointDefinition, EndpointInvocation, EndpointResult, EndpointRouter,
    EndpointStatus, GetOp, IndexQueryOp, LocalPendingBroker, NoopControlHook, NoopObserverHook,
    ObserverEvent, ObserverHook, OperationKind, PolicyAction, PolicyConfig, PolicyDecision,
    PolicyEngine, ProviderBinding, ProviderNamespace, ProviderRegistry, QueryOp, QueryOrder,
    QueryOrderDirection, RuntimePack, SorStoreProvider, SorxAuditEvent, SorxError, SorxEvent,
    SorxResult, SorxRuntimeConfig, StackCallContext, StackCallRequest, StackCallResponse,
    TraverseOp, UniqueConflictBehavior, UniqueIndex, UpdateOp, ViewTransform, apply_value_patch,
};

static GENERATED_NONCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct SorxRuntime {
    pub pack: RuntimePack,
    pub config: SorxRuntimeConfig,
    pub router: EndpointRouter,
    pub providers: ProviderRegistry,
    pub policy: PolicyEngine,
    approval_broker: Arc<dyn ApprovalBroker>,
    audit_sink: Arc<dyn AuditSink>,
    control_hook: Arc<dyn ControlHook>,
    observer_hook: Arc<dyn ObserverHook>,
    observer_fail_open: bool,
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
            control_hook: Arc::new(NoopControlHook),
            observer_hook: Arc::new(NoopObserverHook),
            observer_fail_open: true,
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

    pub fn with_control_hook(mut self, control_hook: Arc<dyn ControlHook>) -> Self {
        self.control_hook = control_hook;
        self
    }

    pub fn with_observer_hook(
        mut self,
        observer_hook: Arc<dyn ObserverHook>,
        fail_open: bool,
    ) -> Self {
        self.observer_hook = observer_hook;
        self.observer_fail_open = fail_open;
        self
    }

    pub fn admin_action_context(
        &self,
        action_id: impl Into<String>,
        path: impl Into<String>,
        actor: impl Into<String>,
        tenant_id: Option<String>,
    ) -> AdminActionContext {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let call_id = format!("admin-{now}");
        AdminActionContext {
            environment_id: self.config.environment.clone(),
            runtime_id: "runtime-main".to_string(),
            tenant_id,
            team_id: None,
            action_id: action_id.into(),
            path: path.into(),
            call_id: call_id.clone(),
            trace_id: call_id,
            actor: actor.into(),
        }
    }

    pub fn observe_admin(&self, event: AdminObserverEvent) -> SorxResult<()> {
        match self.observer_hook.observe_admin(&event) {
            Ok(()) => Ok(()),
            Err(_) if self.observer_fail_open => Ok(()),
            Err(err) => Err(err),
        }
    }

    pub fn pre_admin(
        &self,
        context: &AdminActionContext,
        request: &AdminActionRequest,
    ) -> SorxResult<ControlDecision> {
        self.control_hook.pre_admin(context, request)
    }

    pub fn post_admin(
        &self,
        context: &AdminActionContext,
        request: &AdminActionRequest,
        response: &AdminActionResponse,
    ) -> SorxResult<ControlDecision> {
        self.control_hook.post_admin(context, request, response)
    }

    pub fn audit_metric(
        &self,
        tenant_id: impl Into<String>,
        caller_id: impl Into<String>,
        event_name: impl Into<String>,
        metric_name: impl Into<String>,
        decision: Option<String>,
        details: serde_json::Map<String, Value>,
    ) -> SorxResult<()> {
        let metric_name = metric_name.into();
        self.audit_sink.emit(SorxAuditEvent {
            event: event_name.into(),
            pack: self.pack.name.clone(),
            version: self.pack.version.clone(),
            tenant_id: tenant_id.into(),
            endpoint_id: format!("metrics.{metric_name}"),
            operation_id: format!("metrics.query.{metric_name}"),
            risk: crate::RiskLevel::Low,
            caller_id: caller_id.into(),
            decision,
            duration_ms: None,
            idempotency_key_present: false,
            details,
        })
    }

    pub fn invoke(&self, mut invocation: EndpointInvocation) -> SorxResult<EndpointResult> {
        let started = Instant::now();
        let endpoint = self.router.endpoint(&invocation.endpoint_id)?;
        let call_context = self.stack_call_context(endpoint, &invocation);
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
        if let Some(reason) = self.authorization_denial(endpoint, &invocation) {
            let result = EndpointResult {
                status: EndpointStatus::Denied,
                output: json!({
                    "status": "denied",
                    "reason": reason,
                    "authorization": "denied"
                }),
                events: vec![event(endpoint, &invocation, "authorization.denied")],
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
        let mut stack_request = StackCallRequest {
            operation_id: invocation.operation_id.clone(),
            input: invocation.input.clone(),
        };
        self.observe(ObserverEvent {
            event_type: "stack.call.started".to_string(),
            context: call_context.clone(),
            status: None,
            duration_ms: None,
            control_decision: None,
        })?;
        let pre_decision = self.control_hook.pre_call(&call_context, &stack_request)?;
        match pre_decision.action {
            ControlDecisionAction::Allow => {}
            ControlDecisionAction::AllowWithPatch => {
                if let Some(patch) = &pre_decision.patch {
                    apply_value_patch(&mut invocation.input, patch);
                    stack_request.input = invocation.input.clone();
                }
            }
            ControlDecisionAction::Deny => {
                let result = EndpointResult {
                    status: EndpointStatus::Denied,
                    output: json!({
                        "status": "denied",
                        "reason": pre_decision.reason.clone().unwrap_or_else(|| "control denied stack call".to_string())
                    }),
                    events: vec![event(endpoint, &invocation, "control.denied")],
                };
                self.observe(ObserverEvent {
                    event_type: "stack.call.denied".to_string(),
                    context: call_context,
                    status: Some("denied".to_string()),
                    duration_ms: Some(started.elapsed().as_millis() as u64),
                    control_decision: Some(pre_decision),
                })?;
                self.audit(
                    endpoint,
                    &invocation,
                    "sorx.endpoint.completed",
                    Some("denied"),
                    Some(started.elapsed().as_millis() as u64),
                )?;
                return Ok(result);
            }
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

        let provider_result = (|| -> SorxResult<EndpointResult> {
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
            let result = match &endpoint.operation {
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
                        unique_indexes: self
                            .unique_indexes_for(&binding.entity, &binding.collection),
                        unique_behavior: UniqueConflictBehavior::Reject,
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
                        unique_indexes: self
                            .unique_indexes_for(&binding.entity, &binding.collection),
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
                            order_by: Vec::new(),
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
                OperationKind::Command(spec) => {
                    self.execute_command(endpoint, spec, &invocation, &namespace, &binding)?
                }
            };
            self.audit(
                endpoint,
                &invocation,
                "sorx.provider.operation.completed",
                None,
                None,
            )?;
            Ok(result)
        })();
        let mut result = match provider_result {
            Ok(result) => result,
            Err(err) => {
                self.observe(ObserverEvent {
                    event_type: "stack.call.failed".to_string(),
                    context: call_context,
                    status: Some("failed".to_string()),
                    duration_ms: Some(started.elapsed().as_millis() as u64),
                    control_decision: Some(pre_decision),
                })?;
                self.audit(endpoint, &invocation, "sorx.endpoint.failed", None, None)?;
                return Err(err);
            }
        };
        let mut stack_response = StackCallResponse {
            status: format!("{:?}", result.status).to_ascii_lowercase(),
            output: result.output.clone(),
        };
        let post_decision =
            self.control_hook
                .post_call(&call_context, &stack_request, &stack_response)?;
        match post_decision.action {
            ControlDecisionAction::Allow => {}
            ControlDecisionAction::AllowWithPatch => {
                if let Some(patch) = &post_decision.patch {
                    apply_value_patch(&mut result.output, patch);
                    stack_response.output = result.output.clone();
                }
            }
            ControlDecisionAction::Deny => {
                result = EndpointResult {
                    status: EndpointStatus::Denied,
                    output: json!({
                        "status": "denied",
                        "reason": post_decision.reason.clone().unwrap_or_else(|| "control denied stack response".to_string())
                    }),
                    events: vec![event(endpoint, &invocation, "control.denied")],
                };
                stack_response.status = "denied".to_string();
                stack_response.output = result.output.clone();
            }
        }
        self.observe(ObserverEvent {
            event_type: if result.status == EndpointStatus::Denied {
                "stack.call.denied".to_string()
            } else {
                "stack.call.completed".to_string()
            },
            context: call_context,
            status: Some(stack_response.status),
            duration_ms: Some(started.elapsed().as_millis() as u64),
            control_decision: Some(post_decision),
        })?;
        self.audit(
            endpoint,
            &invocation,
            "sorx.endpoint.completed",
            Some("executed"),
            Some(started.elapsed().as_millis() as u64),
        )?;
        Ok(result)
    }

    fn authorization_denial(
        &self,
        endpoint: &EndpointDefinition,
        invocation: &EndpointInvocation,
    ) -> Option<String> {
        if let Some(auth) = &endpoint.authorization
            && !authorization_matches(auth, &invocation.caller.roles)
        {
            return Some("endpoint authorization roles did not match principal".to_string());
        }
        if let Some(auth) = &endpoint.authorization
            && let Some(reason) = self.authorization_policy_denial(
                auth,
                AuthorizationPolicyResource::Endpoint {
                    endpoint_id: endpoint.endpoint_id.clone(),
                },
                operation_access_name(&endpoint.operation),
                invocation,
            )
        {
            return Some(reason);
        }
        let record = endpoint.entity.as_deref()?;
        let access = self.pack.record_access.get(record)?;
        let auth = match endpoint.operation {
            OperationKind::Get | OperationKind::Query => access.read.as_ref(),
            OperationKind::Create => access.create.as_ref(),
            OperationKind::Update => access.update.as_ref(),
            OperationKind::Delete => access.delete.as_ref(),
            OperationKind::Command(_) => None,
        };
        if let Some(auth) = auth
            && !authorization_matches(auth, &invocation.caller.roles)
        {
            return Some(format!(
                "record `{record}` {} access roles did not match principal",
                operation_access_name(&endpoint.operation)
            ));
        }
        if let Some(auth) = auth
            && let Some(reason) = self.authorization_policy_denial(
                auth,
                AuthorizationPolicyResource::Record {
                    record: record.to_string(),
                },
                operation_access_name(&endpoint.operation),
                invocation,
            )
        {
            return Some(reason);
        }
        None
    }

    fn authorization_policy_denial(
        &self,
        auth: &AuthorizationRequirement,
        resource: AuthorizationPolicyResource,
        operation: &str,
        invocation: &EndpointInvocation,
    ) -> Option<String> {
        let decision = self.policy.decide_authorization(&AuthorizationPolicyInput {
            principal_subject: invocation.caller.subject.clone(),
            principal_roles: invocation.caller.roles.clone(),
            resource,
            operation: operation.to_string(),
            policies: auth.policies.clone(),
            conditions: auth.conditions.clone(),
        });
        if decision.action == PolicyAction::Deny {
            Some(decision.reason)
        } else {
            None
        }
    }

    fn observe(&self, event: ObserverEvent) -> SorxResult<()> {
        match self.observer_hook.observe(&event) {
            Ok(()) => Ok(()),
            Err(_) if self.observer_fail_open => Ok(()),
            Err(err) => Err(err),
        }
    }

    fn stack_call_context(
        &self,
        endpoint: &EndpointDefinition,
        invocation: &EndpointInvocation,
    ) -> StackCallContext {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let call_id = format!("call-{now}");
        StackCallContext {
            environment_id: self.config.environment.clone(),
            runtime_id: "runtime-main".to_string(),
            tenant_id: invocation.tenant_id.clone(),
            team_id: None,
            deployment_id: self.config.deployment.sor_name.clone(),
            stack_id: self.pack.name.clone(),
            revision_id: None,
            route_id: endpoint.endpoint_id.clone(),
            call_id: call_id.clone(),
            trace_id: call_id,
            actor: invocation.caller.subject.clone(),
        }
    }

    fn execute_command(
        &self,
        endpoint: &EndpointDefinition,
        spec: &CommandSpec,
        invocation: &EndpointInvocation,
        namespace: &ProviderNamespace,
        binding: &ProviderBinding,
    ) -> SorxResult<EndpointResult> {
        if spec.idempotency.as_deref() == Some("required") && invocation.idempotency_key.is_none() {
            return Err(SorxError::new(
                "idempotency_key_required",
                format!(
                    "command `{}` requires an idempotency key",
                    endpoint.operation_id
                ),
            ));
        }
        let provider = self.providers.store(&binding.provider_id)?;
        let mut deleted_count = 0usize;
        let mut created_count = 0usize;
        let mut updated_count = 0usize;
        let mut step_outputs = Vec::new();
        let mut command = CommandExecutionContext::new(endpoint, invocation);

        for step in &spec.steps {
            let (as_name, output) = self.execute_command_step(
                step,
                endpoint,
                spec,
                invocation,
                namespace,
                binding,
                &provider,
                &mut command,
                &mut created_count,
                &mut updated_count,
                &mut deleted_count,
            )?;
            command.record_step(as_name.as_deref(), output.clone());
            step_outputs.push(output);
        }

        let result = if let Some(return_value) = &spec.return_value {
            command.resolve_value(return_value)?
        } else {
            json!({
                "deleted": deleted_count,
                "deleted_count": deleted_count,
                "created": created_count,
                "created_count": created_count,
                "updated": updated_count,
                "updated_count": updated_count,
                "steps": step_outputs
            })
        };
        Ok(EndpointResult {
            status: EndpointStatus::Ok,
            output: canonical_to_view(
                &endpoint.view,
                json!({
                    "ok": true,
                    "endpoint_id": endpoint.endpoint_id,
                    "action": spec.action,
                    "result": result
                }),
            ),
            events: vec![event(endpoint, invocation, "command.executed")],
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_command_step(
        &self,
        step: &CommandStep,
        endpoint: &EndpointDefinition,
        spec: &CommandSpec,
        invocation: &EndpointInvocation,
        namespace: &ProviderNamespace,
        binding: &ProviderBinding,
        provider: &Arc<dyn SorStoreProvider>,
        command: &mut CommandExecutionContext<'_>,
        created_count: &mut usize,
        updated_count: &mut usize,
        deleted_count: &mut usize,
    ) -> SorxResult<(Option<String>, Value)> {
        if !command.should_run_step(step_when(step))? {
            return Ok((step_as_name(step).cloned(), skipped_step_output(step)));
        }
        match step {
            CommandStep::DeleteWhere {
                as_name,
                when: _,
                entity,
                collection,
                r#where,
            } => {
                let entity =
                    resolve_step_string(entity, command)?.unwrap_or_else(|| binding.entity.clone());
                let collection = resolve_step_string(collection, command)?
                    .unwrap_or_else(|| binding.collection.clone());
                let filter = command.resolve_value(r#where)?;
                let records = provider.query(QueryOp {
                    namespace: namespace.clone(),
                    entity: entity.clone(),
                    collection: collection.clone(),
                    filter,
                    order_by: Vec::new(),
                })?;
                let mut step_deleted = 0usize;
                let mut affected_ids = Vec::new();
                for record in records.records {
                    let id = record.id;
                    if provider
                        .delete(DeleteOp {
                            namespace: namespace.clone(),
                            entity: entity.clone(),
                            collection: collection.clone(),
                            id: id.clone(),
                        })?
                        .deleted
                    {
                        step_deleted += 1;
                        affected_ids.push(id);
                    }
                }
                *deleted_count += step_deleted;
                Ok((
                    as_name.clone(),
                    json!({
                        "op": "delete_where",
                        "collection": collection,
                        "deleted": step_deleted,
                        "deleted_count": step_deleted,
                        "affected_ids": affected_ids
                    }),
                ))
            }
            CommandStep::Create {
                as_name,
                when: _,
                entity,
                collection,
                input,
            } => {
                let entity =
                    resolve_step_string(entity, command)?.unwrap_or_else(|| binding.entity.clone());
                let collection = resolve_step_string(collection, command)?
                    .unwrap_or_else(|| binding.collection.clone());
                let input_template = input.as_ref().unwrap_or(&invocation.input);
                let generated_fields = generated_field_refs(input_template);
                let unique_indexes = self.unique_indexes_for(&entity, &collection);
                let unique_behavior = command_unique_behavior(spec, &entity, &unique_indexes);
                let mut last_err = None;
                let mut record = None;
                for _ in 0..6 {
                    let input = command.resolve_value(input_template)?;
                    match provider.create(CreateOp {
                        namespace: namespace.clone(),
                        entity: entity.clone(),
                        collection: collection.clone(),
                        input,
                        idempotency_key: invocation
                            .idempotency_key
                            .as_ref()
                            .map(|key| format!("{}:{collection}:{key}", endpoint.operation_id)),
                        unique_indexes: unique_indexes.clone(),
                        unique_behavior: unique_behavior.clone(),
                    }) {
                        Ok(created) => {
                            record = Some(created);
                            break;
                        }
                        Err(err)
                            if err.code == "unique_constraint_violation"
                                && command
                                    .reset_generated_for_conflict(&err, &generated_fields) =>
                        {
                            last_err = Some(err);
                        }
                        Err(err) => return Err(err),
                    }
                }
                let record = match record {
                    Some(record) => record,
                    None => {
                        return Err(last_err.unwrap_or_else(|| {
                            SorxError::new(
                                "unique_constraint_retry_exhausted",
                                "failed to generate a unique value for create command",
                            )
                        }));
                    }
                };
                *created_count += 1;
                Ok((
                    as_name.clone(),
                    json!({
                        "op": "create",
                        "collection": collection,
                        "id": record.id,
                        "record": record
                    }),
                ))
            }
            CommandStep::UpdateWhere {
                as_name,
                when: _,
                entity,
                collection,
                r#where,
                set,
            } => {
                let entity =
                    resolve_step_string(entity, command)?.unwrap_or_else(|| binding.entity.clone());
                let collection = resolve_step_string(collection, command)?
                    .unwrap_or_else(|| binding.collection.clone());
                let filter = command.resolve_value(r#where)?;
                let generated_fields = generated_field_refs(set);
                let unique_indexes = self.unique_indexes_for(&entity, &collection);
                let records = provider.query(QueryOp {
                    namespace: namespace.clone(),
                    entity: entity.clone(),
                    collection: collection.clone(),
                    filter,
                    order_by: Vec::new(),
                })?;
                let mut updated_records = Vec::new();
                let mut affected_ids = Vec::new();
                for record in records.records {
                    let mut last_err = None;
                    let mut updated_record = None;
                    for _ in 0..6 {
                        let patch = command.resolve_value(set)?;
                        match provider.update(UpdateOp {
                            namespace: namespace.clone(),
                            entity: entity.clone(),
                            collection: collection.clone(),
                            id: record.id.clone(),
                            patch,
                            unique_indexes: unique_indexes.clone(),
                        }) {
                            Ok(updated) => {
                                updated_record = Some(updated);
                                break;
                            }
                            Err(err)
                                if err.code == "unique_constraint_violation"
                                    && command
                                        .reset_generated_for_conflict(&err, &generated_fields) =>
                            {
                                last_err = Some(err);
                            }
                            Err(err) => return Err(err),
                        }
                    }
                    let updated = match updated_record {
                        Some(updated) => updated,
                        None => {
                            return Err(last_err.unwrap_or_else(|| {
                                SorxError::new(
                                    "unique_constraint_retry_exhausted",
                                    "failed to generate a unique value for update command",
                                )
                            }));
                        }
                    };
                    affected_ids.push(updated.id.clone());
                    updated_records.push(
                        serde_json::to_value(updated)
                            .map_err(|err| SorxError::new("encode_failed", err.to_string()))?,
                    );
                }
                *updated_count += updated_records.len();
                Ok((
                    as_name.clone(),
                    json!({
                        "op": "update_where",
                        "collection": collection,
                        "updated": updated_records.len(),
                        "updated_count": updated_records.len(),
                        "affected_ids": affected_ids,
                        "records": updated_records
                    }),
                ))
            }
            CommandStep::IncrementWhere {
                as_name,
                when: _,
                entity,
                collection,
                r#where,
                increments,
            } => {
                let entity =
                    resolve_step_string(entity, command)?.unwrap_or_else(|| binding.entity.clone());
                let collection = resolve_step_string(collection, command)?
                    .unwrap_or_else(|| binding.collection.clone());
                let filter = command.resolve_value(r#where)?;
                let increments = command.resolve_value(increments)?;
                let increments = increments.as_object().cloned().ok_or_else(|| {
                    SorxError::new(
                        "command_increments_invalid",
                        "increment_where increments must resolve to an object",
                    )
                })?;
                let unique_indexes = self.unique_indexes_for(&entity, &collection);
                let records = provider.query(QueryOp {
                    namespace: namespace.clone(),
                    entity: entity.clone(),
                    collection: collection.clone(),
                    filter,
                    order_by: Vec::new(),
                })?;
                let mut updated_records = Vec::new();
                let mut affected_ids = Vec::new();
                for record in records.records {
                    let patch = increment_patch(&record.data, &increments)?;
                    let updated = provider.update(UpdateOp {
                        namespace: namespace.clone(),
                        entity: entity.clone(),
                        collection: collection.clone(),
                        id: record.id,
                        patch,
                        unique_indexes: unique_indexes.clone(),
                    })?;
                    affected_ids.push(updated.id.clone());
                    updated_records.push(
                        serde_json::to_value(updated)
                            .map_err(|err| SorxError::new("encode_failed", err.to_string()))?,
                    );
                }
                *updated_count += updated_records.len();
                Ok((
                    as_name.clone(),
                    json!({
                        "op": "increment_where",
                        "collection": collection,
                        "updated": updated_records.len(),
                        "updated_count": updated_records.len(),
                        "affected_ids": affected_ids,
                        "records": updated_records
                    }),
                ))
            }
            CommandStep::Query {
                as_name,
                when: _,
                entity,
                collection,
                r#where,
                order_by,
            } => {
                let entity =
                    resolve_step_string(entity, command)?.unwrap_or_else(|| binding.entity.clone());
                let collection = resolve_step_string(collection, command)?
                    .unwrap_or_else(|| binding.collection.clone());
                let filter = command.resolve_value(r#where)?;
                let records = provider.query(QueryOp {
                    namespace: namespace.clone(),
                    entity,
                    collection: collection.clone(),
                    filter,
                    order_by: command_order_by(order_by),
                })?;
                Ok((
                    as_name.clone(),
                    json!({
                        "op": "query",
                        "collection": collection,
                        "count": records.records.len(),
                        "records": records.records
                    }),
                ))
            }
            CommandStep::FindOne {
                as_name,
                when: _,
                entity,
                collection,
                r#where,
                required,
            } => {
                let entity =
                    resolve_step_string(entity, command)?.unwrap_or_else(|| binding.entity.clone());
                let collection = resolve_step_string(collection, command)?
                    .unwrap_or_else(|| binding.collection.clone());
                let filter = command.resolve_value(r#where)?;
                let records = provider.query(QueryOp {
                    namespace: namespace.clone(),
                    entity,
                    collection: collection.clone(),
                    filter,
                    order_by: Vec::new(),
                })?;
                let record = records.records.into_iter().next();
                if *required && record.is_none() {
                    return Err(SorxError::new(
                        "command_record_not_found",
                        format!("required command record was not found in `{collection}`"),
                    ));
                }
                let output = if let Some(record) = record {
                    serde_json::to_value(record)
                        .map_err(|err| SorxError::new("encode_failed", err.to_string()))?
                } else {
                    Value::Null
                };
                Ok((as_name.clone(), output))
            }
            CommandStep::EmitEvent {
                as_name,
                when: _,
                event,
                payload,
                stream,
            } => {
                let canonical = self.providers.canonical_store(&binding.provider_id)?;
                let data = command.resolve_value(payload)?;
                let record = canonical.append_event(AppendEventOp {
                    namespace: namespace.clone(),
                    stream: stream.clone().unwrap_or_else(|| {
                        spec.action
                            .clone()
                            .unwrap_or_else(|| endpoint.operation_id.clone())
                    }),
                    event_type: event.clone(),
                    capability: Some(business_event_capability(&self.pack.name, event)),
                    producer: Some(format!("sorx:{}:{}", self.pack.name, self.pack.version)),
                    subject_entity: binding.entity.clone(),
                    subject_id: command
                        .resolve_string("$input.id")
                        .or_else(|| command.resolve_string("$item.id"))
                        .unwrap_or_else(|| endpoint.endpoint_id.clone()),
                    data,
                })?;
                Ok((
                    as_name.clone(),
                    serde_json::to_value(record)
                        .map_err(|err| SorxError::new("encode_failed", err.to_string()))?,
                ))
            }
            CommandStep::Foreach {
                as_name,
                when: _,
                items,
                steps,
            } => {
                let items = command.resolve_value(items)?;
                let items = items.as_array().cloned().ok_or_else(|| {
                    SorxError::new(
                        "command_items_invalid",
                        "foreach items must resolve to an array",
                    )
                })?;
                let mut local_created = 0usize;
                let mut local_updated = 0usize;
                let mut local_deleted = 0usize;
                let mut records = Vec::new();
                let mut iteration_outputs = Vec::new();
                for (index, item) in items.iter().cloned().enumerate() {
                    command.push_iteration(item, index);
                    let mut per_iteration = Vec::new();
                    for nested in steps {
                        let (nested_as, output) = self.execute_command_step(
                            nested,
                            endpoint,
                            spec,
                            invocation,
                            namespace,
                            binding,
                            provider,
                            command,
                            &mut local_created,
                            &mut local_updated,
                            &mut local_deleted,
                        )?;
                        command.record_step(nested_as.as_deref(), output.clone());
                        collect_command_records(&output, &mut records);
                        per_iteration.push(output);
                    }
                    command.pop_iteration();
                    iteration_outputs.push(json!({
                        "index": index,
                        "steps": per_iteration
                    }));
                }
                *created_count += local_created;
                *updated_count += local_updated;
                *deleted_count += local_deleted;
                Ok((
                    as_name.clone(),
                    json!({
                        "op": "foreach",
                        "count": items.len(),
                        "created_count": local_created,
                        "updated_count": local_updated,
                        "deleted_count": local_deleted,
                        "records": records,
                        "items": iteration_outputs
                    }),
                ))
            }
        }
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

    fn unique_indexes_for(&self, entity: &str, collection: &str) -> Vec<UniqueIndex> {
        self.pack
            .operational_indexes
            .iter()
            .filter(|index| {
                index.unique
                    && index.record == entity
                    && index
                        .collection
                        .as_deref()
                        .is_none_or(|index_collection| index_collection == collection)
            })
            .map(|index| UniqueIndex {
                id: index.id.clone(),
                record: index.record.clone(),
                collection: index
                    .collection
                    .clone()
                    .unwrap_or_else(|| collection.to_string()),
                fields: index.fields.clone(),
            })
            .collect()
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

fn resolve_step_string(
    value: &Option<String>,
    command: &mut CommandExecutionContext<'_>,
) -> SorxResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    Ok(Some(
        command
            .resolve_string(value)
            .unwrap_or_else(|| value.clone()),
    ))
}

fn collect_command_records(output: &Value, records: &mut Vec<Value>) {
    if let Some(record) = output.get("record") {
        records.push(record.clone());
    }
    if let Some(values) = output.get("records").and_then(Value::as_array) {
        records.extend(values.iter().cloned());
    }
}

fn step_when(step: &CommandStep) -> Option<&Value> {
    match step {
        CommandStep::Create { when, .. }
        | CommandStep::DeleteWhere { when, .. }
        | CommandStep::UpdateWhere { when, .. }
        | CommandStep::IncrementWhere { when, .. }
        | CommandStep::Query { when, .. }
        | CommandStep::FindOne { when, .. }
        | CommandStep::EmitEvent { when, .. }
        | CommandStep::Foreach { when, .. } => when.as_ref(),
    }
}

fn step_as_name(step: &CommandStep) -> Option<&String> {
    match step {
        CommandStep::Create { as_name, .. }
        | CommandStep::DeleteWhere { as_name, .. }
        | CommandStep::UpdateWhere { as_name, .. }
        | CommandStep::IncrementWhere { as_name, .. }
        | CommandStep::Query { as_name, .. }
        | CommandStep::FindOne { as_name, .. }
        | CommandStep::EmitEvent { as_name, .. }
        | CommandStep::Foreach { as_name, .. } => as_name.as_ref(),
    }
}

fn skipped_step_output(step: &CommandStep) -> Value {
    let op = match step {
        CommandStep::Create { .. } => "create",
        CommandStep::DeleteWhere { .. } => "delete_where",
        CommandStep::UpdateWhere { .. } => "update_where",
        CommandStep::IncrementWhere { .. } => "increment_where",
        CommandStep::Query { .. } => "query",
        CommandStep::FindOne { .. } => "find_one",
        CommandStep::EmitEvent { .. } => "emit_event",
        CommandStep::Foreach { .. } => "foreach",
    };
    json!({ "op": op, "skipped": true })
}

fn command_order_by(order_by: &[crate::model::CommandOrderBy]) -> Vec<QueryOrder> {
    order_by
        .iter()
        .map(|order| QueryOrder {
            field: order.field.clone(),
            direction: match order.direction {
                CommandOrderDirection::Asc => QueryOrderDirection::Asc,
                CommandOrderDirection::Desc => QueryOrderDirection::Desc,
            },
        })
        .collect()
}

fn increment_patch(data: &Value, increments: &serde_json::Map<String, Value>) -> SorxResult<Value> {
    let mut patch = serde_json::Map::new();
    for (field, amount) in increments {
        let amount = amount.as_f64().ok_or_else(|| {
            SorxError::at_path(
                "command_increment_invalid",
                "increment amount must be numeric",
                field,
            )
        })?;
        let current = lookup_path(data, field)
            .and_then(Value::as_f64)
            .unwrap_or_default();
        let next = current + amount;
        patch.insert(field.clone(), json_number(next));
    }
    Ok(Value::Object(patch))
}

fn json_number(value: f64) -> Value {
    if value.fract() == 0.0 && value >= i64::MIN as f64 && value <= i64::MAX as f64 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

fn command_unique_behavior(
    spec: &CommandSpec,
    entity: &str,
    indexes: &[UniqueIndex],
) -> UniqueConflictBehavior {
    let Some(idempotency) = &spec.constraints.idempotency else {
        return UniqueConflictBehavior::Reject;
    };
    let mode = idempotency.mode.as_deref().or(spec.idempotency.as_deref());
    if mode != Some("return_existing") {
        return UniqueConflictBehavior::Reject;
    }
    let Some(index) = idempotency.index.as_ref() else {
        return UniqueConflictBehavior::Reject;
    };
    if let Some(unique) = indexes
        .iter()
        .find(|candidate| candidate.id == *index && candidate.record == entity)
    {
        return UniqueConflictBehavior::ReturnExisting {
            index: unique.id.clone(),
            fields: unique.fields.clone(),
        };
    }
    UniqueConflictBehavior::Reject
}

fn generated_field_refs(value: &Value) -> BTreeMap<String, String> {
    let mut refs = BTreeMap::new();
    collect_generated_field_refs(value, "", &mut refs);
    refs
}

fn collect_generated_field_refs(value: &Value, path: &str, refs: &mut BTreeMap<String, String>) {
    match value {
        Value::String(text) => {
            if let Some(name) = text.strip_prefix("$generated.") {
                refs.insert(path.to_string(), name.to_string());
            }
        }
        Value::Object(object) => {
            for (key, value) in object {
                let child = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                collect_generated_field_refs(value, &child, refs);
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                let child = if path.is_empty() {
                    index.to_string()
                } else {
                    format!("{path}.{index}")
                };
                collect_generated_field_refs(value, &child, refs);
            }
        }
        _ => {}
    }
}

struct CommandExecutionContext<'a> {
    endpoint: &'a EndpointDefinition,
    invocation: &'a EndpointInvocation,
    generated: BTreeMap<String, Value>,
    steps: Vec<Value>,
    named_steps: serde_json::Map<String, Value>,
    local_named_steps: Vec<serde_json::Map<String, Value>>,
    item: Option<Value>,
    index: Option<usize>,
    now: Value,
    nonce: u64,
}

impl<'a> CommandExecutionContext<'a> {
    fn new(endpoint: &'a EndpointDefinition, invocation: &'a EndpointInvocation) -> Self {
        Self {
            endpoint,
            invocation,
            generated: BTreeMap::new(),
            steps: Vec::new(),
            named_steps: serde_json::Map::new(),
            local_named_steps: Vec::new(),
            item: None,
            index: None,
            now: Value::String(now_string()),
            nonce: GENERATED_NONCE.fetch_add(1, Ordering::Relaxed),
        }
    }

    fn record_step(&mut self, as_name: Option<&str>, output: Value) {
        if let Some(as_name) = as_name {
            if let Some(local) = self.local_named_steps.last_mut() {
                local.insert(as_name.to_string(), output.clone());
            } else {
                self.named_steps.insert(as_name.to_string(), output.clone());
            }
        }
        if self.local_named_steps.is_empty() {
            self.steps.push(output);
        }
    }

    fn push_iteration(&mut self, item: Value, index: usize) {
        self.item = Some(item);
        self.index = Some(index);
        self.local_named_steps.push(serde_json::Map::new());
    }

    fn pop_iteration(&mut self) {
        self.local_named_steps.pop();
        self.item = None;
        self.index = None;
    }

    fn resolve_value(&mut self, value: &Value) -> SorxResult<Value> {
        match value {
            Value::String(text) => self.resolve_string_value(text),
            Value::Array(values) => values
                .iter()
                .map(|value| self.resolve_value(value))
                .collect::<SorxResult<Vec<_>>>()
                .map(Value::Array),
            Value::Object(object) => {
                if let Some(values) = object.get("coalesce").and_then(Value::as_array) {
                    for value in values {
                        let resolved = self.resolve_optional_value(value)?;
                        if !is_absent_command_value(&resolved) {
                            return Ok(resolved);
                        }
                    }
                    return Ok(Value::Null);
                }
                if let Some(rank) = object.get("rank") {
                    return self.resolve_rank(rank);
                }
                let mut out = serde_json::Map::new();
                for (key, value) in object {
                    out.insert(key.clone(), self.resolve_optional_value(value)?);
                }
                Ok(Value::Object(out))
            }
            _ => Ok(value.clone()),
        }
    }

    fn resolve_optional_value(&mut self, value: &Value) -> SorxResult<Value> {
        match self.resolve_value(value) {
            Ok(value) => Ok(value),
            Err(err) if err.code == "command_value_missing" => Ok(Value::Null),
            Err(err) => Err(err),
        }
    }

    fn resolve_string(&mut self, value: &str) -> Option<String> {
        self.resolve_string_value(value).ok().and_then(|value| {
            value
                .as_str()
                .map(ToString::to_string)
                .or_else(|| value.as_i64().map(|number| number.to_string()))
        })
    }

    fn should_run_step(&mut self, when: Option<&Value>) -> SorxResult<bool> {
        let Some(when) = when else {
            return Ok(true);
        };
        match when {
            Value::Bool(value) => Ok(*value),
            Value::String(_) => Ok(!is_absent_command_value(
                &self.resolve_optional_value(when)?,
            )),
            Value::Object(object) => {
                if let Some(value) = object.get("present") {
                    return Ok(!is_absent_command_value(
                        &self.resolve_optional_value(value)?,
                    ));
                }
                if let Some(value) = object.get("absent") {
                    return Ok(is_absent_command_value(
                        &self.resolve_optional_value(value)?,
                    ));
                }
                if let Some(values) = object.get("all").and_then(Value::as_array) {
                    for value in values {
                        if !self.should_run_step(Some(value))? {
                            return Ok(false);
                        }
                    }
                    return Ok(true);
                }
                if let Some(values) = object.get("any").and_then(Value::as_array) {
                    for value in values {
                        if self.should_run_step(Some(value))? {
                            return Ok(true);
                        }
                    }
                    return Ok(false);
                }
                if let Some(value) = object.get("eq").and_then(Value::as_array)
                    && value.len() == 2
                {
                    return Ok(self.resolve_optional_value(&value[0])?
                        == self.resolve_optional_value(&value[1])?);
                }
                Ok(!is_absent_command_value(
                    &self.resolve_optional_value(when)?,
                ))
            }
            _ => Ok(!is_absent_command_value(
                &self.resolve_optional_value(when)?,
            )),
        }
    }

    fn resolve_rank(&mut self, rank: &Value) -> SorxResult<Value> {
        let object = rank
            .as_object()
            .ok_or_else(|| SorxError::new("command_rank_invalid", "rank must be an object"))?;
        let records = object
            .get("records")
            .ok_or_else(|| SorxError::new("command_rank_invalid", "rank.records is required"))?;
        let records = self.resolve_value(records)?;
        let records = records.as_array().ok_or_else(|| {
            SorxError::new(
                "command_rank_invalid",
                "rank.records must resolve to an array",
            )
        })?;
        let where_value = object
            .get("where")
            .ok_or_else(|| SorxError::new("command_rank_invalid", "rank.where is required"))?;
        let filter = self.resolve_value(where_value)?;
        let filter = filter.as_object().ok_or_else(|| {
            SorxError::new(
                "command_rank_invalid",
                "rank.where must resolve to an object",
            )
        })?;
        for (index, record) in records.iter().enumerate() {
            if filter
                .iter()
                .all(|(path, expected)| lookup_path(record, path) == Some(expected))
            {
                return Ok(json!(index + 1));
            }
        }
        Ok(Value::Null)
    }

    fn resolve_string_value(&mut self, text: &str) -> SorxResult<Value> {
        if text == "$input" {
            return Ok(self.invocation.input.clone());
        }
        if text == "$item" {
            return Ok(self.item.clone().unwrap_or(Value::Null));
        }
        if text == "$index" {
            return Ok(json!(self.index.unwrap_or(0)));
        }
        if text == "$now" {
            return Ok(self.now.clone());
        }
        if let Some(path) = text.strip_prefix("$input.") {
            return lookup_path(&self.invocation.input, path)
                .cloned()
                .ok_or_else(|| missing_command_path("input", path));
        }
        if let Some(name) = text.strip_prefix("$generated.") {
            return Ok(self.generated_value(name));
        }
        if let Some(path) = text.strip_prefix("$item.") {
            return self
                .item
                .as_ref()
                .and_then(|item| lookup_path(item, path))
                .cloned()
                .ok_or_else(|| missing_command_path("item", path));
        }
        if let Some(path) = text.strip_prefix("$steps.") {
            return self
                .step_value(path)
                .cloned()
                .ok_or_else(|| missing_command_path("steps", path));
        }
        Ok(Value::String(text.to_string()))
    }

    fn step_value(&self, path: &str) -> Option<&Value> {
        let (head, tail) = path.split_once('.').unwrap_or((path, ""));
        let base = if let Ok(index) = head.parse::<usize>() {
            self.steps.get(index)?
        } else if let Some(local) = self.local_named_steps.last()
            && let Some(value) = local.get(head)
        {
            value
        } else {
            self.named_steps.get(head)?
        };
        if tail.is_empty() {
            Some(base)
        } else {
            lookup_path(base, tail)
        }
    }

    fn generated_value(&mut self, name: &str) -> Value {
        if let Some(value) = self.generated.get(name) {
            return value.clone();
        }
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let seed = format!(
            "{}:{}:{}:{}:{}:{}",
            self.endpoint.endpoint_id,
            self.endpoint.operation_id,
            self.invocation
                .idempotency_key
                .as_deref()
                .unwrap_or("no-key"),
            self.generated.len(),
            self.nonce,
            time
        );
        let value = match name {
            "uuid" => Value::String(generated_uuid(&seed)),
            "short_code" | "invitation_code" => Value::String(generated_short_code(&seed, 8)),
            "referral_code" => Value::String(generated_short_code(&seed, 10)),
            value if value.ends_with("_token") => {
                Value::String(generated_short_code(&format!("{seed}:{name}"), 16))
            }
            value if value.ends_with("_code") => {
                Value::String(generated_short_code(&format!("{seed}:{name}"), 10))
            }
            _ => Value::String(generated_short_code(&format!("{seed}:{name}"), 12)),
        };
        self.generated.insert(name.to_string(), value.clone());
        value
    }

    fn reset_generated_for_conflict(
        &mut self,
        err: &SorxError,
        generated_fields: &BTreeMap<String, String>,
    ) -> bool {
        let mut reset = false;
        for (field, name) in generated_fields {
            if err.message.contains(field) {
                self.generated.remove(name);
                reset = true;
            }
        }
        if reset {
            self.nonce = GENERATED_NONCE.fetch_add(1, Ordering::Relaxed);
        }
        reset
    }
}

fn lookup_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in path.split('.') {
        current = if let Ok(index) = part.parse::<usize>() {
            current.get(index)?
        } else {
            current.get(part)?
        };
    }
    Some(current)
}

fn missing_command_path(scope: &str, path: &str) -> SorxError {
    SorxError::at_path(
        "command_value_missing",
        format!("command {scope} path `{path}` is missing"),
        path,
    )
}

fn is_absent_command_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(value) => value.is_empty(),
        _ => false,
    }
}

fn now_string() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn generated_short_code(seed: &str, len: usize) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(seed.as_bytes());
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    (0..len)
        .map(|index| {
            let byte = digest[index % digest.len()] as usize;
            ALPHABET[byte % ALPHABET.len()] as char
        })
        .collect()
}

fn generated_uuid(seed: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(seed.as_bytes());
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-4{:02x}{:02x}-8{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        digest[0],
        digest[1],
        digest[2],
        digest[3],
        digest[4],
        digest[5],
        digest[6] & 0x0f,
        digest[7],
        digest[8] & 0x3f,
        digest[9],
        digest[10],
        digest[11],
        digest[12],
        digest[13],
        digest[14],
        digest[15]
    )
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

fn business_event_capability(pack_name: &str, event_type: &str) -> String {
    format!(
        "cap://greentic/events/{}/{}",
        clean_capability_segment(pack_name),
        clean_capability_segment(event_type)
    )
}

fn clean_capability_segment(value: &str) -> String {
    let cleaned = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if cleaned.is_empty() {
        "unnamed".to_string()
    } else {
        cleaned
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

fn authorization_matches(auth: &AuthorizationRequirement, principal_roles: &[String]) -> bool {
    if auth.roles.any_of.is_empty() && auth.roles.all_of.is_empty() {
        return true;
    }
    let any_ok = auth.roles.any_of.is_empty()
        || auth
            .roles
            .any_of
            .iter()
            .any(|role| principal_roles.iter().any(|principal| principal == role));
    let all_ok = auth
        .roles
        .all_of
        .iter()
        .all(|role| principal_roles.iter().any(|principal| principal == role));
    any_ok && all_ok
}

fn operation_access_name(operation: &OperationKind) -> &'static str {
    match operation {
        OperationKind::Get | OperationKind::Query => "read",
        OperationKind::Create => "create",
        OperationKind::Update => "update",
        OperationKind::Delete => "delete",
        OperationKind::Command(_) => "execute",
    }
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
        operational_indexes: Vec::new(),
        record_access: BTreeMap::new(),
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
