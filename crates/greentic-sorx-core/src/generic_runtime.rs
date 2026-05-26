use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{SorxError, SorxResult};

pub const RUNTIME_INFO_SCHEMA: &str = "greentic.runtime.info.v1";
pub const RUNTIME_HEALTH_SCHEMA: &str = "greentic.runtime.health.v1";
pub const RUNTIME_CAPABILITIES_SCHEMA: &str = "greentic.capabilities.v1";
pub const RUNTIME_DEPLOYMENTS_SCHEMA: &str = "greentic.runtime.deployments.v1";
pub const RUNTIME_TRAFFIC_SCHEMA: &str = "greentic.runtime.traffic.v1";
pub const RUNTIME_ADMIN_SURFACES_SCHEMA: &str = "greentic.runtime.admin-surfaces.v1";

pub const CAP_RUNTIME_HOST_V1: &str = "greentic.cap.runtime.host.v1";
pub const CAP_SECRETS_V1: &str = "greentic.cap.secrets.v1";
pub const CAP_TELEMETRY_V1: &str = "greentic.cap.telemetry.v1";
pub const CAP_EXTENSION_CONTROL_V1: &str = "greentic.cap.extension.control.v1";
pub const CAP_EXTENSION_OBSERVER_V1: &str = "greentic.cap.extension.observer.v1";
pub const CAP_EXTENSION_ADMIN_V1: &str = "greentic.cap.extension.admin.v1";

pub const CONTRACT_RUNTIME_ADMIN_V1: &str = "greentic.runtime.admin.v1";
pub const CONTRACT_RUNTIME_HEALTH_V1: &str = "greentic.runtime.health.v1";
pub const CONTRACT_RUNTIME_DEPLOYMENTS_V1: &str = "greentic.runtime.deployments.v1";
pub const CONTRACT_RUNTIME_TRAFFIC_V1: &str = "greentic.runtime.traffic.v1";
pub const CONTRACT_RUNTIME_INVOKE_V1: &str = "greentic.runtime.invoke.v1";
pub const CONTRACT_CONTROL_PRE_CALL_V1: &str = "greentic.control.pre-call.v1";
pub const CONTRACT_CONTROL_POST_CALL_V1: &str = "greentic.control.post-call.v1";
pub const CONTRACT_CONTROL_PRE_ADMIN_V1: &str = "greentic.control.pre-admin.v1";
pub const CONTRACT_CONTROL_POST_ADMIN_V1: &str = "greentic.control.post-admin.v1";
pub const CONTRACT_OBSERVER_PRE_CALL_V1: &str = "greentic.observer.pre-call.v1";
pub const CONTRACT_OBSERVER_POST_CALL_V1: &str = "greentic.observer.post-call.v1";
pub const CONTRACT_OBSERVER_CALL_FAILED_V1: &str = "greentic.observer.call-failed.v1";
pub const CONTRACT_OBSERVER_CONTROL_DENIED_V1: &str = "greentic.observer.control-denied.v1";
pub const CONTRACT_OBSERVER_ADMIN_EVENT_V1: &str = "greentic.observer.admin-event.v1";
pub const CONTRACT_ADMIN_SURFACE_V1: &str = "greentic.admin.surface.v1";

pub const SORX_DEPLOYER_DESCRIPTOR: &str = "greentic.deployer.sorx@0.1.0";
pub const SORX_DEPLOYER_DESCRIPTOR_PATH: &str = "greentic.deployer.sorx";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInfo {
    pub schema: String,
    pub runtime_id: String,
    pub runtime_kind: String,
    pub implementation: String,
    pub version: String,
    pub contracts: Vec<String>,
}

impl RuntimeInfo {
    pub fn sorx(runtime_id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            schema: RUNTIME_INFO_SCHEMA.to_string(),
            runtime_id: runtime_id.into(),
            runtime_kind: "runtime-host".to_string(),
            implementation: "sorx".to_string(),
            version: version.into(),
            contracts: runtime_contracts(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeHealth {
    pub schema: String,
    pub runtime_id: String,
    pub status: String,
    pub deployment_count: usize,
    pub ready_revision_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityOffer {
    pub capability: String,
    pub contracts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityRequirement {
    pub capability: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contracts: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCapabilities {
    pub schema: String,
    pub offers: Vec<CapabilityOffer>,
    pub requires: Vec<CapabilityRequirement>,
}

impl RuntimeCapabilities {
    pub fn sorx_runtime_host() -> Self {
        Self {
            schema: RUNTIME_CAPABILITIES_SCHEMA.to_string(),
            offers: vec![CapabilityOffer {
                capability: CAP_RUNTIME_HOST_V1.to_string(),
                contracts: runtime_contracts(),
            }],
            requires: vec![
                optional_requirement(CAP_SECRETS_V1),
                optional_requirement(CAP_TELEMETRY_V1),
                optional_requirement(CAP_EXTENSION_CONTROL_V1),
                optional_requirement(CAP_EXTENSION_OBSERVER_V1),
                optional_requirement(CAP_EXTENSION_ADMIN_V1),
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeRevisionLifecycle {
    Inactive,
    Staged,
    Warming,
    Ready,
    Draining,
    Failed,
    Archived,
}

impl RuntimeRevisionLifecycle {
    pub fn allows_new_traffic(self) -> bool {
        matches!(self, Self::Ready)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeRevision {
    pub revision_id: String,
    pub deployment_id: String,
    pub bundle_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_id: Option<String>,
    pub lifecycle: RuntimeRevisionLifecycle,
    #[serde(default)]
    pub weight_bps: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_uri: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeDeployment {
    pub deployment_id: String,
    pub bundle_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_id: Option<String>,
    pub revisions: Vec<RuntimeRevision>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeDeployments {
    pub schema: String,
    pub deployments: Vec<RuntimeDeployment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeTrafficEntry {
    pub revision_id: String,
    pub weight_bps: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeTrafficSplit {
    pub schema: String,
    pub deployment_id: String,
    pub entries: Vec<RuntimeTrafficEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageDeploymentRequest {
    pub deployment_id: String,
    pub revision_id: String,
    pub bundle_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_uri: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrafficUpdateRequest {
    pub entries: Vec<RuntimeTrafficEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub schema: String,
    pub env_id: String,
    pub revisions: Vec<RevisionRuntimeBlock>,
    #[serde(default, skip_serializing_if = "RuntimeExtensions::is_empty")]
    pub extensions: RuntimeExtensions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionRuntimeBlock {
    pub deployment_id: String,
    pub revision_id: String,
    pub bundle_id: String,
    #[serde(default)]
    pub pack_list_refs: Vec<String>,
    #[serde(default)]
    pub pack_config_refs: Vec<String>,
    pub weight_bps: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeExtensions {
    #[serde(default, skip_serializing_if = "RuntimeControlExtensions::is_empty")]
    pub control: RuntimeControlExtensions,
    #[serde(default, skip_serializing_if = "RuntimeObserverExtensions::is_empty")]
    pub observer: RuntimeObserverExtensions,
    #[serde(default, skip_serializing_if = "RuntimeAdminExtensions::is_empty")]
    pub admin: RuntimeAdminExtensions,
}

impl RuntimeExtensions {
    pub fn is_empty(&self) -> bool {
        self.control.is_empty() && self.observer.is_empty() && self.admin.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeControlExtensions {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hooks: BTreeMap<String, Vec<RuntimeExtensionBinding>>,
}

impl RuntimeControlExtensions {
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeObserverExtensions {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub subscriptions: BTreeMap<String, Vec<RuntimeExtensionBinding>>,
}

impl RuntimeObserverExtensions {
    pub fn is_empty(&self) -> bool {
        self.subscriptions.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeAdminExtensions {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub surfaces: Vec<RuntimeAdminSurfaceBinding>,
}

impl RuntimeAdminExtensions {
    pub fn is_empty(&self) -> bool {
        self.surfaces.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeExtensionBinding {
    pub id: String,
    pub contract: String,
    pub pack_ref: String,
    #[serde(default)]
    pub fail_mode: ExtensionFailMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeAdminSurfaceBinding {
    pub id: String,
    pub contract: String,
    pub pack_ref: String,
    pub mount: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtensionFailMode {
    Open,
    #[default]
    Closed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StackCallContext {
    pub environment_id: String,
    pub runtime_id: String,
    pub tenant_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    pub deployment_id: String,
    pub stack_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    pub route_id: String,
    pub call_id: String,
    pub trace_id: String,
    pub actor: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StackCallRequest {
    pub operation_id: String,
    pub input: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StackCallResponse {
    pub status: String,
    pub output: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlDecisionAction {
    Allow,
    Deny,
    AllowWithPatch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlDecision {
    pub action: ControlDecisionAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<Value>,
}

impl ControlDecision {
    pub fn allow() -> Self {
        Self {
            action: ControlDecisionAction::Allow,
            reason: None,
            patch: None,
        }
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        Self {
            action: ControlDecisionAction::Deny,
            reason: Some(reason.into()),
            patch: None,
        }
    }

    pub fn allow_with_patch(patch: Value) -> Self {
        Self {
            action: ControlDecisionAction::AllowWithPatch,
            reason: None,
            patch: Some(patch),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObserverEvent {
    pub event_type: String,
    pub context: StackCallContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_decision: Option<ControlDecision>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminSurface {
    pub surface_id: String,
    pub kind: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_pack_ref: Option<String>,
    #[serde(default)]
    pub required_permissions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeAdminSurfaces {
    pub schema: String,
    pub surfaces: Vec<AdminSurface>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminActionContext {
    pub environment_id: String,
    pub runtime_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    pub action_id: String,
    pub path: String,
    pub call_id: String,
    pub trace_id: String,
    pub actor: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminActionRequest {
    pub method: String,
    pub path: String,
    pub input: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminActionResponse {
    pub status: u16,
    pub output: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminObserverEvent {
    pub event_type: String,
    pub context: AdminActionContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_decision: Option<ControlDecision>,
}

pub trait ControlHook: std::fmt::Debug + Send + Sync {
    fn pre_call(
        &self,
        _context: &StackCallContext,
        _request: &StackCallRequest,
    ) -> SorxResult<ControlDecision> {
        Ok(ControlDecision::allow())
    }

    fn post_call(
        &self,
        _context: &StackCallContext,
        _request: &StackCallRequest,
        _response: &StackCallResponse,
    ) -> SorxResult<ControlDecision> {
        Ok(ControlDecision::allow())
    }

    fn pre_admin(
        &self,
        _context: &AdminActionContext,
        _request: &AdminActionRequest,
    ) -> SorxResult<ControlDecision> {
        Ok(ControlDecision::allow())
    }

    fn post_admin(
        &self,
        _context: &AdminActionContext,
        _request: &AdminActionRequest,
        _response: &AdminActionResponse,
    ) -> SorxResult<ControlDecision> {
        Ok(ControlDecision::allow())
    }
}

#[derive(Debug, Default)]
pub struct NoopControlHook;

impl ControlHook for NoopControlHook {}

pub trait ObserverHook: std::fmt::Debug + Send + Sync {
    fn observe(&self, _event: &ObserverEvent) -> SorxResult<()> {
        Ok(())
    }

    fn observe_admin(&self, _event: &AdminObserverEvent) -> SorxResult<()> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct NoopObserverHook;

impl ObserverHook for NoopObserverHook {}

pub trait RuntimeExtensionAdapter: std::fmt::Debug + Send + Sync {
    fn control(
        &self,
        _hook: &str,
        _binding: &RuntimeExtensionBinding,
        _request: &Value,
        _response: Option<&Value>,
    ) -> SorxResult<ControlDecision> {
        Ok(ControlDecision::allow())
    }

    fn observe(
        &self,
        _subscription: &str,
        _binding: &RuntimeExtensionBinding,
        _event: &Value,
    ) -> SorxResult<()> {
        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
pub struct RuntimeExtensionRegistry {
    adapters: BTreeMap<String, Arc<dyn RuntimeExtensionAdapter>>,
}

impl RuntimeExtensionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_adapter(
        mut self,
        pack_ref: impl Into<String>,
        adapter: Arc<dyn RuntimeExtensionAdapter>,
    ) -> Self {
        self.adapters.insert(pack_ref.into(), adapter);
        self
    }

    fn adapter(&self, pack_ref: &str) -> SorxResult<Arc<dyn RuntimeExtensionAdapter>> {
        self.adapters.get(pack_ref).cloned().ok_or_else(|| {
            SorxError::new(
                "runtime_extension_adapter_missing",
                format!("extension pack `{pack_ref}` is not loaded"),
            )
        })
    }
}

#[derive(Debug, Clone)]
pub struct BoundControlHook {
    extensions: RuntimeExtensions,
    registry: RuntimeExtensionRegistry,
}

impl BoundControlHook {
    pub fn new(extensions: RuntimeExtensions, registry: RuntimeExtensionRegistry) -> Self {
        Self {
            extensions,
            registry,
        }
    }

    fn run(
        &self,
        hook: &str,
        request: &Value,
        response: Option<&Value>,
    ) -> SorxResult<ControlDecision> {
        let Some(bindings) = self.extensions.control.hooks.get(hook) else {
            return Ok(ControlDecision::allow());
        };
        let mut patch = Value::Null;
        for binding in bindings {
            let decision = match self
                .registry
                .adapter(&binding.pack_ref)
                .and_then(|adapter| adapter.control(hook, binding, request, response))
            {
                Ok(decision) => decision,
                Err(_) if binding.fail_mode == ExtensionFailMode::Open => continue,
                Err(err) => return Err(err),
            };
            match decision.action {
                ControlDecisionAction::Allow => {}
                ControlDecisionAction::AllowWithPatch => {
                    if let Some(next_patch) = decision.patch {
                        apply_value_patch(&mut patch, &next_patch);
                    }
                }
                ControlDecisionAction::Deny => return Ok(decision),
            }
        }
        if patch.is_null() {
            Ok(ControlDecision::allow())
        } else {
            Ok(ControlDecision::allow_with_patch(patch))
        }
    }
}

impl ControlHook for BoundControlHook {
    fn pre_call(
        &self,
        _context: &StackCallContext,
        request: &StackCallRequest,
    ) -> SorxResult<ControlDecision> {
        let value = serde_json::to_value(request)
            .map_err(|err| SorxError::new("runtime_extension_request_invalid", err.to_string()))?;
        self.run("pre_call", &value, None)
    }

    fn post_call(
        &self,
        _context: &StackCallContext,
        request: &StackCallRequest,
        response: &StackCallResponse,
    ) -> SorxResult<ControlDecision> {
        let value = serde_json::to_value(request)
            .map_err(|err| SorxError::new("runtime_extension_request_invalid", err.to_string()))?;
        self.run("post_call", &value, Some(&response.output))
    }

    fn pre_admin(
        &self,
        _context: &AdminActionContext,
        request: &AdminActionRequest,
    ) -> SorxResult<ControlDecision> {
        let value = serde_json::to_value(request)
            .map_err(|err| SorxError::new("runtime_extension_request_invalid", err.to_string()))?;
        self.run("pre_admin", &value, None)
    }

    fn post_admin(
        &self,
        _context: &AdminActionContext,
        request: &AdminActionRequest,
        response: &AdminActionResponse,
    ) -> SorxResult<ControlDecision> {
        let value = serde_json::to_value(request)
            .map_err(|err| SorxError::new("runtime_extension_request_invalid", err.to_string()))?;
        self.run("post_admin", &value, Some(&response.output))
    }
}

#[derive(Debug, Clone)]
pub struct BoundObserverHook {
    extensions: RuntimeExtensions,
    registry: RuntimeExtensionRegistry,
}

impl BoundObserverHook {
    pub fn new(extensions: RuntimeExtensions, registry: RuntimeExtensionRegistry) -> Self {
        Self {
            extensions,
            registry,
        }
    }

    fn notify(&self, subscription: &str, event: Value) -> SorxResult<()> {
        let Some(bindings) = self.extensions.observer.subscriptions.get(subscription) else {
            return Ok(());
        };
        for binding in bindings {
            match self
                .registry
                .adapter(&binding.pack_ref)
                .and_then(|adapter| adapter.observe(subscription, binding, &event))
            {
                Ok(()) => {}
                Err(_) if binding.fail_mode == ExtensionFailMode::Open => {}
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }
}

impl ObserverHook for BoundObserverHook {
    fn observe(&self, event: &ObserverEvent) -> SorxResult<()> {
        let subscription = match event.event_type.as_str() {
            "stack.call.started" => "pre_call",
            "stack.call.completed" => "post_call",
            "stack.call.failed" => "call_failed",
            "stack.call.denied" => "control_denied",
            _ => return Ok(()),
        };
        let value = serde_json::to_value(event)
            .map_err(|err| SorxError::new("runtime_extension_event_invalid", err.to_string()))?;
        self.notify(subscription, value)
    }

    fn observe_admin(&self, event: &AdminObserverEvent) -> SorxResult<()> {
        let value = serde_json::to_value(event)
            .map_err(|err| SorxError::new("runtime_extension_event_invalid", err.to_string()))?;
        self.notify("admin_event", value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSnapshot {
    runtime_id: String,
    deployments: BTreeMap<String, RuntimeDeployment>,
    admin_surfaces: BTreeMap<String, AdminSurface>,
}

impl RuntimeSnapshot {
    pub fn new(runtime_id: impl Into<String>) -> Self {
        Self {
            runtime_id: runtime_id.into(),
            deployments: BTreeMap::new(),
            admin_surfaces: BTreeMap::new(),
        }
    }

    pub fn from_runtime_config(
        runtime_id: impl Into<String>,
        config: RuntimeConfig,
    ) -> SorxResult<Self> {
        let mut snapshot = Self::new(runtime_id);
        snapshot.apply_runtime_config(config)?;
        Ok(snapshot)
    }

    pub fn runtime_id(&self) -> &str {
        &self.runtime_id
    }

    pub fn deployments(&self) -> RuntimeDeployments {
        RuntimeDeployments {
            schema: RUNTIME_DEPLOYMENTS_SCHEMA.to_string(),
            deployments: self.deployments.values().cloned().collect(),
        }
    }

    pub fn deployment(&self, deployment_id: &str) -> SorxResult<RuntimeDeployment> {
        self.deployments
            .get(deployment_id)
            .cloned()
            .ok_or_else(|| missing_deployment(deployment_id))
    }

    pub fn health(&self) -> RuntimeHealth {
        RuntimeHealth {
            schema: RUNTIME_HEALTH_SCHEMA.to_string(),
            runtime_id: self.runtime_id.clone(),
            status: "ok".to_string(),
            deployment_count: self.deployments.len(),
            ready_revision_count: self
                .deployments
                .values()
                .flat_map(|deployment| deployment.revisions.iter())
                .filter(|revision| revision.lifecycle == RuntimeRevisionLifecycle::Ready)
                .count(),
        }
    }

    pub fn admin_surfaces(&self) -> RuntimeAdminSurfaces {
        RuntimeAdminSurfaces {
            schema: RUNTIME_ADMIN_SURFACES_SCHEMA.to_string(),
            surfaces: self.admin_surfaces.values().cloned().collect(),
        }
    }

    pub fn admin_surface_for_path(&self, path: &str) -> Option<AdminSurface> {
        self.admin_surfaces
            .values()
            .filter(|surface| {
                path == surface.path || path.starts_with(&format!("{}/", surface.path))
            })
            .max_by_key(|surface| surface.path.len())
            .cloned()
    }

    pub fn register_admin_surface(&mut self, surface: AdminSurface) -> SorxResult<AdminSurface> {
        if surface.surface_id.trim().is_empty()
            || surface.kind.trim().is_empty()
            || surface.path.trim().is_empty()
        {
            return Err(SorxError::new(
                "runtime_admin_surface_invalid",
                "surface_id, kind, and path are required",
            ));
        }
        validate_admin_mount(&surface.path)?;
        if self.admin_surfaces.values().any(|existing| {
            existing.surface_id != surface.surface_id && existing.path == surface.path
        }) {
            return Err(SorxError::new(
                "runtime_admin_surface_mount_collision",
                format!(
                    "admin surface path `{}` is already registered",
                    surface.path
                ),
            ));
        }
        self.admin_surfaces
            .insert(surface.surface_id.clone(), surface.clone());
        Ok(surface)
    }

    pub fn stage(&mut self, request: StageDeploymentRequest) -> SorxResult<RuntimeDeployment> {
        if request.deployment_id.trim().is_empty() || request.revision_id.trim().is_empty() {
            return Err(SorxError::new(
                "runtime_stage_invalid",
                "deployment_id and revision_id are required",
            ));
        }
        let deployment = self
            .deployments
            .entry(request.deployment_id.clone())
            .or_insert_with(|| RuntimeDeployment {
                deployment_id: request.deployment_id.clone(),
                bundle_id: request.bundle_id.clone(),
                stack_id: request.stack_id.clone(),
                revisions: Vec::new(),
            });
        if deployment.bundle_id != request.bundle_id {
            return Err(SorxError::new(
                "runtime_deployment_bundle_mismatch",
                format!(
                    "deployment `{}` is already bound to bundle `{}`",
                    request.deployment_id, deployment.bundle_id
                ),
            ));
        }
        if deployment
            .revisions
            .iter()
            .any(|revision| revision.revision_id == request.revision_id)
        {
            return Err(SorxError::new(
                "runtime_revision_exists",
                format!("revision `{}` already exists", request.revision_id),
            ));
        }
        deployment.revisions.push(RuntimeRevision {
            revision_id: request.revision_id,
            deployment_id: request.deployment_id,
            bundle_id: request.bundle_id,
            stack_id: request.stack_id,
            lifecycle: RuntimeRevisionLifecycle::Staged,
            weight_bps: 0,
            artifact_uri: request.artifact_uri,
        });
        deployment.revisions.sort_by(|left, right| {
            left.revision_id
                .cmp(&right.revision_id)
                .then(left.bundle_id.cmp(&right.bundle_id))
        });
        Ok(deployment.clone())
    }

    pub fn warm(&mut self, deployment_id: &str) -> SorxResult<RuntimeDeployment> {
        self.transition_deployment_revision(
            deployment_id,
            &[
                RuntimeRevisionLifecycle::Staged,
                RuntimeRevisionLifecycle::Warming,
            ],
            RuntimeRevisionLifecycle::Ready,
        )
    }

    pub fn activate(&mut self, deployment_id: &str) -> SorxResult<RuntimeDeployment> {
        self.transition_deployment_revision(
            deployment_id,
            &[
                RuntimeRevisionLifecycle::Staged,
                RuntimeRevisionLifecycle::Warming,
            ],
            RuntimeRevisionLifecycle::Ready,
        )
    }

    pub fn drain(
        &mut self,
        deployment_id: &str,
        revision_id: &str,
    ) -> SorxResult<RuntimeDeployment> {
        let deployment = self.deployment_mut(deployment_id)?;
        let revision = deployment
            .revisions
            .iter_mut()
            .find(|revision| revision.revision_id == revision_id)
            .ok_or_else(|| missing_revision(revision_id))?;
        revision.weight_bps = 0;
        revision.lifecycle = RuntimeRevisionLifecycle::Draining;
        Ok(deployment.clone())
    }

    pub fn deactivate(&mut self, deployment_id: &str) -> SorxResult<RuntimeDeployment> {
        let deployment = self.deployment_mut(deployment_id)?;
        for revision in &mut deployment.revisions {
            revision.weight_bps = 0;
            if revision.lifecycle == RuntimeRevisionLifecycle::Ready {
                revision.lifecycle = RuntimeRevisionLifecycle::Inactive;
            }
        }
        Ok(deployment.clone())
    }

    pub fn set_traffic(
        &mut self,
        deployment_id: &str,
        request: TrafficUpdateRequest,
    ) -> SorxResult<RuntimeTrafficSplit> {
        validate_traffic_entries(&request.entries)?;
        let deployment = self.deployment_mut(deployment_id)?;
        let known: BTreeSet<String> = deployment
            .revisions
            .iter()
            .map(|revision| revision.revision_id.clone())
            .collect();
        for entry in &request.entries {
            if !known.contains(&entry.revision_id) {
                return Err(missing_revision(&entry.revision_id));
            }
        }
        for revision in &mut deployment.revisions {
            revision.weight_bps = request
                .entries
                .iter()
                .find(|entry| entry.revision_id == revision.revision_id)
                .map(|entry| entry.weight_bps)
                .unwrap_or(0);
        }
        Ok(RuntimeTrafficSplit {
            schema: RUNTIME_TRAFFIC_SCHEMA.to_string(),
            deployment_id: deployment_id.to_string(),
            entries: request.entries,
        })
    }

    pub fn selectable_revisions(&self, deployment_id: &str) -> SorxResult<Vec<RuntimeRevision>> {
        Ok(self
            .deployment(deployment_id)?
            .revisions
            .into_iter()
            .filter(|revision| revision.lifecycle.allows_new_traffic() && revision.weight_bps > 0)
            .collect())
    }

    pub fn select_revision(
        &self,
        deployment_id: &str,
        selector_bps: u32,
    ) -> SorxResult<RuntimeRevision> {
        let selectable = self.selectable_revisions(deployment_id)?;
        if selectable.is_empty() {
            return Err(SorxError::new(
                "runtime_traffic_no_ready_revision",
                format!("deployment `{deployment_id}` has no ready revision with positive traffic"),
            ));
        }
        let total_weight = selectable
            .iter()
            .map(|revision| revision.weight_bps)
            .sum::<u32>();
        let mut cursor = selector_bps % total_weight;
        for revision in selectable {
            if cursor < revision.weight_bps {
                return Ok(revision);
            }
            cursor -= revision.weight_bps;
        }
        Err(SorxError::new(
            "runtime_traffic_selection_failed",
            format!("deployment `{deployment_id}` traffic selection failed"),
        ))
    }

    pub fn apply_runtime_config(&mut self, config: RuntimeConfig) -> SorxResult<()> {
        validate_runtime_config(&config)?;
        self.deployments.clear();
        self.admin_surfaces.clear();
        for surface in &config.extensions.admin.surfaces {
            self.register_admin_surface(AdminSurface {
                surface_id: surface.id.clone(),
                kind: "extension".to_string(),
                path: surface.mount.clone(),
                source_pack_ref: Some(surface.pack_ref.clone()),
                required_permissions: Vec::new(),
            })?;
        }
        for block in config.revisions {
            let deployment = self
                .deployments
                .entry(block.deployment_id.clone())
                .or_insert_with(|| RuntimeDeployment {
                    deployment_id: block.deployment_id.clone(),
                    bundle_id: block.bundle_id.clone(),
                    stack_id: Some(block.bundle_id.clone()),
                    revisions: Vec::new(),
                });
            deployment.revisions.push(RuntimeRevision {
                revision_id: block.revision_id,
                deployment_id: block.deployment_id,
                bundle_id: block.bundle_id,
                stack_id: deployment.stack_id.clone(),
                lifecycle: RuntimeRevisionLifecycle::Ready,
                weight_bps: block.weight_bps,
                artifact_uri: None,
            });
        }
        Ok(())
    }

    pub fn apply_runtime_config_json(&mut self, value: Value) -> SorxResult<()> {
        let config = serde_json::from_value::<RuntimeConfig>(value)
            .map_err(|err| SorxError::new("runtime_config_invalid", err.to_string()))?;
        self.apply_runtime_config(config)
    }

    pub fn apply_runtime_config_str(&mut self, raw: &str) -> SorxResult<()> {
        let config = serde_json::from_str::<RuntimeConfig>(raw)
            .map_err(|err| SorxError::new("runtime_config_invalid", err.to_string()))?;
        self.apply_runtime_config(config)
    }

    fn transition_deployment_revision(
        &mut self,
        deployment_id: &str,
        from: &[RuntimeRevisionLifecycle],
        to: RuntimeRevisionLifecycle,
    ) -> SorxResult<RuntimeDeployment> {
        let deployment = self.deployment_mut(deployment_id)?;
        let Some(revision) = deployment
            .revisions
            .iter_mut()
            .rev()
            .find(|revision| from.contains(&revision.lifecycle))
        else {
            return Err(SorxError::new(
                "runtime_revision_transition_invalid",
                format!("deployment `{deployment_id}` has no revision eligible for transition"),
            ));
        };
        revision.lifecycle = to;
        Ok(deployment.clone())
    }

    fn deployment_mut(&mut self, deployment_id: &str) -> SorxResult<&mut RuntimeDeployment> {
        self.deployments
            .get_mut(deployment_id)
            .ok_or_else(|| missing_deployment(deployment_id))
    }
}

pub fn runtime_contracts() -> Vec<String> {
    vec![
        CONTRACT_RUNTIME_ADMIN_V1.to_string(),
        CONTRACT_RUNTIME_HEALTH_V1.to_string(),
        CONTRACT_RUNTIME_DEPLOYMENTS_V1.to_string(),
        CONTRACT_RUNTIME_TRAFFIC_V1.to_string(),
        CONTRACT_RUNTIME_INVOKE_V1.to_string(),
    ]
}

pub fn validate_runtime_config(config: &RuntimeConfig) -> SorxResult<()> {
    if config.schema != "greentic.runtime-config.v1" {
        return Err(SorxError::new(
            "runtime_config_schema_mismatch",
            format!(
                "expected greentic.runtime-config.v1, got `{}`",
                config.schema
            ),
        ));
    }
    let mut by_deployment: BTreeMap<&str, (&str, u32, BTreeSet<&str>)> = BTreeMap::new();
    for block in &config.revisions {
        validate_relative_refs(&block.pack_list_refs)?;
        validate_relative_refs(&block.pack_config_refs)?;
        let entry = by_deployment.entry(&block.deployment_id).or_insert((
            &block.bundle_id,
            0,
            BTreeSet::new(),
        ));
        if entry.0 != block.bundle_id {
            return Err(SorxError::new(
                "runtime_config_mixed_bundles",
                format!("deployment `{}` mixes bundles", block.deployment_id),
            ));
        }
        if !entry.2.insert(&block.revision_id) {
            return Err(SorxError::new(
                "runtime_config_duplicate_revision",
                format!(
                    "deployment `{}` repeats revision `{}`",
                    block.deployment_id, block.revision_id
                ),
            ));
        }
        entry.1 = entry.1.saturating_add(block.weight_bps);
    }
    validate_runtime_extensions(&config.extensions)?;
    for (deployment_id, (_, weight_sum, _)) in by_deployment {
        if weight_sum != 10_000 {
            return Err(SorxError::new(
                "runtime_config_invalid_traffic_sum",
                format!(
                    "deployment `{deployment_id}` revision weights sum to {weight_sum} bps, must equal 10000"
                ),
            ));
        }
    }
    Ok(())
}

pub fn validate_runtime_extensions(extensions: &RuntimeExtensions) -> SorxResult<()> {
    for (hook, bindings) in &extensions.control.hooks {
        let expected_contract = match hook.as_str() {
            "pre_call" => CONTRACT_CONTROL_PRE_CALL_V1,
            "post_call" => CONTRACT_CONTROL_POST_CALL_V1,
            "pre_admin" => CONTRACT_CONTROL_PRE_ADMIN_V1,
            "post_admin" => CONTRACT_CONTROL_POST_ADMIN_V1,
            _ => {
                return Err(SorxError::new(
                    "runtime_extension_hook_invalid",
                    format!("control hook `{hook}` is not supported"),
                ));
            }
        };
        validate_extension_bindings("control", hook, expected_contract, bindings)?;
    }
    for (subscription, bindings) in &extensions.observer.subscriptions {
        let expected_contract = match subscription.as_str() {
            "pre_call" => CONTRACT_OBSERVER_PRE_CALL_V1,
            "post_call" => CONTRACT_OBSERVER_POST_CALL_V1,
            "call_failed" => CONTRACT_OBSERVER_CALL_FAILED_V1,
            "control_denied" => CONTRACT_OBSERVER_CONTROL_DENIED_V1,
            "admin_event" => CONTRACT_OBSERVER_ADMIN_EVENT_V1,
            _ => {
                return Err(SorxError::new(
                    "runtime_extension_subscription_invalid",
                    format!("observer subscription `{subscription}` is not supported"),
                ));
            }
        };
        validate_extension_bindings("observer", subscription, expected_contract, bindings)?;
    }
    let mut mounts = BTreeSet::new();
    for surface in &extensions.admin.surfaces {
        validate_binding_id(&surface.id, "admin surface")?;
        if surface.contract != CONTRACT_ADMIN_SURFACE_V1 {
            return Err(SorxError::new(
                "runtime_extension_contract_mismatch",
                format!(
                    "admin surface `{}` declares contract `{}`, expected `{}`",
                    surface.id, surface.contract, CONTRACT_ADMIN_SURFACE_V1
                ),
            ));
        }
        validate_pack_ref(&surface.pack_ref)?;
        validate_admin_mount(&surface.mount)?;
        if !mounts.insert(surface.mount.as_str()) {
            return Err(SorxError::new(
                "runtime_admin_surface_mount_collision",
                format!(
                    "admin surface mount `{}` is declared more than once",
                    surface.mount
                ),
            ));
        }
    }
    Ok(())
}

fn validate_traffic_entries(entries: &[RuntimeTrafficEntry]) -> SorxResult<()> {
    let mut seen = BTreeSet::new();
    let mut sum = 0_u32;
    for entry in entries {
        if !seen.insert(&entry.revision_id) {
            return Err(SorxError::new(
                "runtime_traffic_duplicate_revision",
                format!("traffic repeats revision `{}`", entry.revision_id),
            ));
        }
        sum = sum.saturating_add(entry.weight_bps);
    }
    if sum != 10_000 {
        return Err(SorxError::new(
            "runtime_traffic_invalid_sum",
            format!("traffic weights sum to {sum} bps, must equal 10000"),
        ));
    }
    Ok(())
}

pub fn apply_value_patch(value: &mut Value, patch: &Value) {
    match (value, patch) {
        (Value::Object(target), Value::Object(patch)) => merge_object_patch(target, patch),
        (target, patch) => *target = patch.clone(),
    }
}

fn merge_object_patch(target: &mut Map<String, Value>, patch: &Map<String, Value>) {
    for (key, value) in patch {
        if value.is_null() {
            target.remove(key);
        } else if let (Some(Value::Object(existing)), Value::Object(nested)) =
            (target.get_mut(key), value)
        {
            merge_object_patch(existing, nested);
        } else {
            target.insert(key.clone(), value.clone());
        }
    }
}

fn validate_relative_refs(refs: &[String]) -> SorxResult<()> {
    for raw in refs {
        if raw.starts_with('/') || raw.split('/').any(|part| part == "..") {
            return Err(SorxError::new(
                "runtime_config_path_invalid",
                format!(
                    "runtime-config ref `{raw}` must be env-relative and stay under the environment directory"
                ),
            ));
        }
    }
    Ok(())
}

fn validate_extension_bindings(
    extension_kind: &str,
    extension_point: &str,
    expected_contract: &str,
    bindings: &[RuntimeExtensionBinding],
) -> SorxResult<()> {
    let mut ids = BTreeSet::new();
    for binding in bindings {
        validate_binding_id(&binding.id, extension_kind)?;
        if !ids.insert(binding.id.as_str()) {
            return Err(SorxError::new(
                "runtime_extension_duplicate",
                format!(
                    "{extension_kind} extension `{}` is bound more than once for `{extension_point}`",
                    binding.id
                ),
            ));
        }
        if binding.contract != expected_contract {
            return Err(SorxError::new(
                "runtime_extension_contract_mismatch",
                format!(
                    "{extension_kind} extension `{}` for `{extension_point}` declares contract `{}`, expected `{expected_contract}`",
                    binding.id, binding.contract
                ),
            ));
        }
        validate_pack_ref(&binding.pack_ref)?;
    }
    Ok(())
}

fn validate_binding_id(id: &str, label: &str) -> SorxResult<()> {
    if id.trim().is_empty() {
        return Err(SorxError::new(
            "runtime_extension_id_missing",
            format!("{label} extension id is required"),
        ));
    }
    Ok(())
}

fn validate_pack_ref(pack_ref: &str) -> SorxResult<()> {
    if pack_ref.trim().is_empty() {
        return Err(SorxError::new(
            "runtime_extension_pack_ref_missing",
            "extension pack_ref is required",
        ));
    }
    if pack_ref.starts_with('/') || pack_ref.split('/').any(|part| part == "..") {
        return Err(SorxError::new(
            "runtime_extension_pack_ref_invalid",
            format!("extension pack_ref `{pack_ref}` must not escape the environment directory"),
        ));
    }
    Ok(())
}

fn validate_admin_mount(mount: &str) -> SorxResult<()> {
    if !mount.starts_with('/') || mount.contains("//") || mount.split('/').any(|part| part == "..")
    {
        return Err(SorxError::new(
            "runtime_admin_surface_mount_invalid",
            format!("admin surface mount `{mount}` must be an absolute normalized path"),
        ));
    }
    Ok(())
}

fn optional_requirement(capability: &str) -> CapabilityRequirement {
    CapabilityRequirement {
        capability: capability.to_string(),
        contracts: Vec::new(),
        optional: true,
    }
}

fn missing_deployment(deployment_id: &str) -> SorxError {
    SorxError::new(
        "runtime_deployment_missing",
        format!("deployment `{deployment_id}` does not exist"),
    )
}

fn missing_revision(revision_id: &str) -> SorxError {
    SorxError::new(
        "runtime_revision_missing",
        format!("revision `{revision_id}` does not exist"),
    )
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct TestExtensionAdapter {
        control: Mutex<BTreeMap<String, SorxResult<ControlDecision>>>,
        observed: Mutex<Vec<String>>,
    }

    impl TestExtensionAdapter {
        fn with_control(hook: &str, decision: SorxResult<ControlDecision>) -> Arc<Self> {
            let adapter = Arc::new(Self::default());
            adapter
                .control
                .lock()
                .unwrap()
                .insert(hook.to_string(), decision);
            adapter
        }

        fn observed(&self) -> Vec<String> {
            self.observed.lock().unwrap().clone()
        }
    }

    impl RuntimeExtensionAdapter for TestExtensionAdapter {
        fn control(
            &self,
            hook: &str,
            _binding: &RuntimeExtensionBinding,
            _request: &Value,
            _response: Option<&Value>,
        ) -> SorxResult<ControlDecision> {
            self.control
                .lock()
                .unwrap()
                .get(hook)
                .cloned()
                .unwrap_or_else(|| Ok(ControlDecision::allow()))
        }

        fn observe(
            &self,
            subscription: &str,
            _binding: &RuntimeExtensionBinding,
            _event: &Value,
        ) -> SorxResult<()> {
            self.observed.lock().unwrap().push(subscription.to_string());
            Ok(())
        }
    }

    #[test]
    fn runtime_metadata_and_capabilities_are_generic() {
        let info = RuntimeInfo::sorx("runtime-main", "0.1.0");
        assert_eq!(info.schema, RUNTIME_INFO_SCHEMA);
        assert_eq!(info.implementation, "sorx");
        assert!(
            info.contracts
                .contains(&CONTRACT_RUNTIME_HEALTH_V1.to_string())
        );

        let caps = RuntimeCapabilities::sorx_runtime_host();
        assert_eq!(caps.offers[0].capability, CAP_RUNTIME_HOST_V1);
        assert!(caps.requires.iter().all(|requirement| requirement.optional));
    }

    #[test]
    fn runtime_config_rejects_bad_paths_duplicates_mixed_bundles_and_bad_sums() {
        let mut config = valid_runtime_config();
        config.revisions[0].pack_config_refs = vec!["../secret.json".to_string()];
        assert_eq!(
            validate_runtime_config(&config).unwrap_err().code,
            "runtime_config_path_invalid"
        );

        let mut config = valid_runtime_config();
        config.revisions.push(config.revisions[0].clone());
        assert_eq!(
            validate_runtime_config(&config).unwrap_err().code,
            "runtime_config_duplicate_revision"
        );

        let mut config = valid_runtime_config();
        config.revisions[0].bundle_id = "other".to_string();
        assert_eq!(
            validate_runtime_config(&config).unwrap_err().code,
            "runtime_config_mixed_bundles"
        );

        let mut config = valid_runtime_config();
        config.revisions[0].weight_bps = 5_000;
        assert_eq!(
            validate_runtime_config(&config).unwrap_err().code,
            "runtime_config_invalid_traffic_sum"
        );
    }

    #[test]
    fn traffic_selects_only_ready_positive_weight_revisions() {
        let mut snapshot = RuntimeSnapshot::new("runtime-main");
        snapshot
            .stage(StageDeploymentRequest {
                deployment_id: "deployment-a".to_string(),
                revision_id: "rev-a".to_string(),
                bundle_id: "bundle-a".to_string(),
                stack_id: None,
                artifact_uri: None,
            })
            .unwrap();
        snapshot.activate("deployment-a").unwrap();
        snapshot
            .stage(StageDeploymentRequest {
                deployment_id: "deployment-a".to_string(),
                revision_id: "rev-b".to_string(),
                bundle_id: "bundle-a".to_string(),
                stack_id: None,
                artifact_uri: None,
            })
            .unwrap();
        snapshot.activate("deployment-a").unwrap();
        snapshot
            .set_traffic(
                "deployment-a",
                TrafficUpdateRequest {
                    entries: vec![
                        RuntimeTrafficEntry {
                            revision_id: "rev-a".to_string(),
                            weight_bps: 10_000,
                        },
                        RuntimeTrafficEntry {
                            revision_id: "rev-b".to_string(),
                            weight_bps: 0,
                        },
                    ],
                },
            )
            .unwrap();

        let selectable = snapshot.selectable_revisions("deployment-a").unwrap();
        assert_eq!(selectable.len(), 1);
        assert_eq!(selectable[0].revision_id, "rev-a");
        snapshot.drain("deployment-a", "rev-a").unwrap();
        assert!(
            snapshot
                .selectable_revisions("deployment-a")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn deterministic_revision_selection_uses_basis_point_weights() {
        let snapshot =
            RuntimeSnapshot::from_runtime_config("runtime-main", valid_runtime_config()).unwrap();

        assert_eq!(
            snapshot
                .select_revision("deployment-a", 0)
                .unwrap()
                .revision_id,
            "rev-a"
        );
        assert_eq!(
            snapshot
                .select_revision("deployment-a", 8_999)
                .unwrap()
                .revision_id,
            "rev-a"
        );
        assert_eq!(
            snapshot
                .select_revision("deployment-a", 9_000)
                .unwrap()
                .revision_id,
            "rev-b"
        );
        assert_eq!(
            snapshot
                .select_revision("deployment-a", 19_000)
                .unwrap()
                .revision_id,
            "rev-b"
        );
    }

    #[test]
    fn runtime_config_json_replaces_snapshot() {
        let mut snapshot = RuntimeSnapshot::new("runtime-main");
        snapshot
            .stage(StageDeploymentRequest {
                deployment_id: "old".to_string(),
                revision_id: "old-rev".to_string(),
                bundle_id: "old-bundle".to_string(),
                stack_id: None,
                artifact_uri: None,
            })
            .unwrap();

        let value = serde_json::to_value(valid_runtime_config()).unwrap();
        snapshot.apply_runtime_config_json(value).unwrap();

        assert!(snapshot.deployment("old").is_err());
        assert_eq!(
            snapshot.deployment("deployment-a").unwrap().revisions.len(),
            2
        );
    }

    #[test]
    fn admin_surfaces_can_be_registered_and_listed_deterministically() {
        let mut snapshot = RuntimeSnapshot::new("runtime-main");
        snapshot
            .register_admin_surface(AdminSurface {
                surface_id: "settings.page".to_string(),
                kind: "page".to_string(),
                path: "/admin/settings".to_string(),
                source_pack_ref: None,
                required_permissions: vec!["settings.read".to_string()],
            })
            .unwrap();
        snapshot
            .register_admin_surface(AdminSurface {
                surface_id: "health.api".to_string(),
                kind: "api".to_string(),
                path: "/admin/health".to_string(),
                source_pack_ref: None,
                required_permissions: Vec::new(),
            })
            .unwrap();

        let surfaces = snapshot.admin_surfaces();
        assert_eq!(surfaces.schema, RUNTIME_ADMIN_SURFACES_SCHEMA);
        assert_eq!(surfaces.surfaces[0].surface_id, "health.api");
        assert_eq!(surfaces.surfaces[1].surface_id, "settings.page");
    }

    #[test]
    fn runtime_config_validates_extension_bindings_and_registers_admin_surfaces() {
        let mut config = valid_runtime_config();
        config.extensions.control.hooks.insert(
            "pre_call".to_string(),
            vec![RuntimeExtensionBinding {
                id: "policy".to_string(),
                contract: CONTRACT_CONTROL_PRE_CALL_V1.to_string(),
                pack_ref: "extensions/policy.gtpack".to_string(),
                fail_mode: ExtensionFailMode::Closed,
            }],
        );
        config.extensions.observer.subscriptions.insert(
            "post_call".to_string(),
            vec![RuntimeExtensionBinding {
                id: "audit".to_string(),
                contract: CONTRACT_OBSERVER_POST_CALL_V1.to_string(),
                pack_ref: "ghcr.io/greenticai/extensions/observer/audit:1.0.0".to_string(),
                fail_mode: ExtensionFailMode::Open,
            }],
        );
        config
            .extensions
            .admin
            .surfaces
            .push(RuntimeAdminSurfaceBinding {
                id: "stack-console".to_string(),
                contract: CONTRACT_ADMIN_SURFACE_V1.to_string(),
                pack_ref: "extensions/admin/stack-console.gtpack".to_string(),
                mount: "/admin/stacks".to_string(),
            });

        validate_runtime_config(&config).unwrap();
        let snapshot = RuntimeSnapshot::from_runtime_config("runtime-main", config).unwrap();
        let surfaces = snapshot.admin_surfaces();
        assert_eq!(surfaces.surfaces[0].surface_id, "stack-console");
        assert_eq!(surfaces.surfaces[0].path, "/admin/stacks");
        assert_eq!(
            surfaces.surfaces[0].source_pack_ref.as_deref(),
            Some("extensions/admin/stack-console.gtpack")
        );
    }

    #[test]
    fn runtime_config_rejects_invalid_extension_metadata() {
        let mut config = valid_runtime_config();
        config.extensions.control.hooks.insert(
            "during_call".to_string(),
            vec![RuntimeExtensionBinding {
                id: "policy".to_string(),
                contract: CONTRACT_CONTROL_PRE_CALL_V1.to_string(),
                pack_ref: "extensions/policy.gtpack".to_string(),
                fail_mode: ExtensionFailMode::Closed,
            }],
        );
        assert_eq!(
            validate_runtime_config(&config).unwrap_err().code,
            "runtime_extension_hook_invalid"
        );

        let mut config = valid_runtime_config();
        config.extensions.observer.subscriptions.insert(
            "post_call".to_string(),
            vec![RuntimeExtensionBinding {
                id: "audit".to_string(),
                contract: CONTRACT_OBSERVER_PRE_CALL_V1.to_string(),
                pack_ref: "extensions/audit.gtpack".to_string(),
                fail_mode: ExtensionFailMode::Open,
            }],
        );
        assert_eq!(
            validate_runtime_config(&config).unwrap_err().code,
            "runtime_extension_contract_mismatch"
        );

        let mut config = valid_runtime_config();
        config
            .extensions
            .admin
            .surfaces
            .push(RuntimeAdminSurfaceBinding {
                id: "console-a".to_string(),
                contract: CONTRACT_ADMIN_SURFACE_V1.to_string(),
                pack_ref: "../console.gtpack".to_string(),
                mount: "/admin/stacks".to_string(),
            });
        assert_eq!(
            validate_runtime_config(&config).unwrap_err().code,
            "runtime_extension_pack_ref_invalid"
        );

        let mut config = valid_runtime_config();
        config
            .extensions
            .admin
            .surfaces
            .push(RuntimeAdminSurfaceBinding {
                id: "console-a".to_string(),
                contract: CONTRACT_ADMIN_SURFACE_V1.to_string(),
                pack_ref: "extensions/console-a.gtpack".to_string(),
                mount: "/admin/stacks".to_string(),
            });
        config
            .extensions
            .admin
            .surfaces
            .push(RuntimeAdminSurfaceBinding {
                id: "console-b".to_string(),
                contract: CONTRACT_ADMIN_SURFACE_V1.to_string(),
                pack_ref: "extensions/console-b.gtpack".to_string(),
                mount: "/admin/stacks".to_string(),
            });
        assert_eq!(
            validate_runtime_config(&config).unwrap_err().code,
            "runtime_admin_surface_mount_collision"
        );
    }

    #[test]
    fn bound_control_extensions_deny_patch_and_honor_fail_modes() {
        let mut extensions = RuntimeExtensions::default();
        extensions.control.hooks.insert(
            "pre_call".to_string(),
            vec![RuntimeExtensionBinding {
                id: "policy".to_string(),
                contract: CONTRACT_CONTROL_PRE_CALL_V1.to_string(),
                pack_ref: "extensions/policy.gtpack".to_string(),
                fail_mode: ExtensionFailMode::Closed,
            }],
        );
        extensions.control.hooks.insert(
            "post_call".to_string(),
            vec![RuntimeExtensionBinding {
                id: "redactor".to_string(),
                contract: CONTRACT_CONTROL_POST_CALL_V1.to_string(),
                pack_ref: "extensions/redactor.gtpack".to_string(),
                fail_mode: ExtensionFailMode::Closed,
            }],
        );
        let registry = RuntimeExtensionRegistry::new()
            .with_adapter(
                "extensions/policy.gtpack",
                TestExtensionAdapter::with_control(
                    "pre_call",
                    Ok(ControlDecision::deny("blocked by extension")),
                ),
            )
            .with_adapter(
                "extensions/redactor.gtpack",
                TestExtensionAdapter::with_control(
                    "post_call",
                    Ok(ControlDecision::allow_with_patch(json!({
                        "data": { "name": "redacted" }
                    }))),
                ),
            );
        let hook = BoundControlHook::new(extensions, registry);

        let denied = hook
            .pre_call(
                &stack_context(),
                &StackCallRequest {
                    operation_id: "tenant.create".to_string(),
                    input: json!({ "id": "tenant-a" }),
                },
            )
            .unwrap();
        assert_eq!(denied.action, ControlDecisionAction::Deny);
        assert_eq!(denied.reason.as_deref(), Some("blocked by extension"));

        let patch = hook
            .post_call(
                &stack_context(),
                &StackCallRequest {
                    operation_id: "tenant.create".to_string(),
                    input: json!({}),
                },
                &StackCallResponse {
                    status: "ok".to_string(),
                    output: json!({ "data": { "name": "Acme" } }),
                },
            )
            .unwrap();
        assert_eq!(patch.action, ControlDecisionAction::AllowWithPatch);
        assert_eq!(patch.patch.unwrap()["data"]["name"], "redacted");

        let mut extensions = RuntimeExtensions::default();
        extensions.control.hooks.insert(
            "pre_call".to_string(),
            vec![RuntimeExtensionBinding {
                id: "missing-open".to_string(),
                contract: CONTRACT_CONTROL_PRE_CALL_V1.to_string(),
                pack_ref: "extensions/missing.gtpack".to_string(),
                fail_mode: ExtensionFailMode::Open,
            }],
        );
        let decision = BoundControlHook::new(extensions, RuntimeExtensionRegistry::new())
            .pre_call(
                &stack_context(),
                &StackCallRequest {
                    operation_id: "tenant.create".to_string(),
                    input: json!({}),
                },
            )
            .unwrap();
        assert_eq!(decision.action, ControlDecisionAction::Allow);
    }

    #[test]
    fn bound_observer_extensions_receive_selected_events_and_honor_fail_modes() {
        let adapter = Arc::new(TestExtensionAdapter::default());
        let mut extensions = RuntimeExtensions::default();
        extensions.observer.subscriptions.insert(
            "pre_call".to_string(),
            vec![RuntimeExtensionBinding {
                id: "audit".to_string(),
                contract: CONTRACT_OBSERVER_PRE_CALL_V1.to_string(),
                pack_ref: "extensions/audit.gtpack".to_string(),
                fail_mode: ExtensionFailMode::Closed,
            }],
        );
        extensions.observer.subscriptions.insert(
            "call_failed".to_string(),
            vec![RuntimeExtensionBinding {
                id: "audit".to_string(),
                contract: CONTRACT_OBSERVER_CALL_FAILED_V1.to_string(),
                pack_ref: "extensions/audit.gtpack".to_string(),
                fail_mode: ExtensionFailMode::Closed,
            }],
        );
        extensions.observer.subscriptions.insert(
            "admin_event".to_string(),
            vec![RuntimeExtensionBinding {
                id: "audit".to_string(),
                contract: CONTRACT_OBSERVER_ADMIN_EVENT_V1.to_string(),
                pack_ref: "extensions/audit.gtpack".to_string(),
                fail_mode: ExtensionFailMode::Closed,
            }],
        );
        let hook = BoundObserverHook::new(
            extensions,
            RuntimeExtensionRegistry::new()
                .with_adapter("extensions/audit.gtpack", adapter.clone()),
        );

        hook.observe(&ObserverEvent {
            event_type: "stack.call.started".to_string(),
            context: stack_context(),
            status: None,
            duration_ms: None,
            control_decision: None,
        })
        .unwrap();
        hook.observe(&ObserverEvent {
            event_type: "stack.call.failed".to_string(),
            context: stack_context(),
            status: Some("failed".to_string()),
            duration_ms: Some(1),
            control_decision: None,
        })
        .unwrap();
        hook.observe_admin(&AdminObserverEvent {
            event_type: "admin.action.completed".to_string(),
            context: admin_context(),
            status: Some("200".to_string()),
            duration_ms: None,
            control_decision: None,
        })
        .unwrap();

        assert_eq!(
            adapter.observed(),
            vec!["pre_call", "call_failed", "admin_event"]
        );
    }

    fn valid_runtime_config() -> RuntimeConfig {
        RuntimeConfig {
            schema: "greentic.runtime-config.v1".to_string(),
            env_id: "local".to_string(),
            revisions: vec![
                RevisionRuntimeBlock {
                    deployment_id: "deployment-a".to_string(),
                    revision_id: "rev-a".to_string(),
                    bundle_id: "bundle-a".to_string(),
                    pack_list_refs: vec!["locks/rev-a.json".to_string()],
                    pack_config_refs: vec!["configs/rev-a.json".to_string()],
                    weight_bps: 9_000,
                },
                RevisionRuntimeBlock {
                    deployment_id: "deployment-a".to_string(),
                    revision_id: "rev-b".to_string(),
                    bundle_id: "bundle-a".to_string(),
                    pack_list_refs: Vec::new(),
                    pack_config_refs: Vec::new(),
                    weight_bps: 1_000,
                },
            ],
            extensions: RuntimeExtensions::default(),
        }
    }

    fn stack_context() -> StackCallContext {
        StackCallContext {
            environment_id: "local".to_string(),
            runtime_id: "runtime-main".to_string(),
            tenant_id: "tenant-a".to_string(),
            team_id: None,
            deployment_id: "deployment-a".to_string(),
            stack_id: "stack-a".to_string(),
            revision_id: Some("rev-a".to_string()),
            route_id: "tenant.create".to_string(),
            call_id: "call-a".to_string(),
            trace_id: "trace-a".to_string(),
            actor: "tester".to_string(),
        }
    }

    fn admin_context() -> AdminActionContext {
        AdminActionContext {
            environment_id: "local".to_string(),
            runtime_id: "runtime-main".to_string(),
            tenant_id: None,
            team_id: None,
            action_id: "GET /admin/v1/health".to_string(),
            path: "/admin/v1/health".to_string(),
            call_id: "admin-a".to_string(),
            trace_id: "admin-a".to_string(),
            actor: "tester".to_string(),
        }
    }
}
