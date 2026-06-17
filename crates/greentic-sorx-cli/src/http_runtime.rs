use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use greentic_sorx_core::{
    AdminActionRequest, AdminActionResponse, AdminObserverEvent, AdminSurface,
    AuthorizationRequirement, AuthorizationRoles, BusinessEventSink, CallerContext,
    CapabilityOffer, CommandStep, ControlDecisionAction, DeploymentRegistryError,
    EndpointDefinition, EndpointInvocation, EndpointMethod, EndpointRouter, EndpointStatus,
    EntityRecord, FoundationDbProviderAdapter, FoundationDbProviderConfig, InvocationSource,
    LocalDeploymentRegistryStore, ManagerContextDefaults, ManagerFieldRelationshipView,
    ManagerFieldView, ManagerLocaleBundle, ManagerLocaleCatalog, ManagerLocaleContext,
    ManagerNavItem, ManagerPolicyDecision, ManagerPolicyEffect, ManagerPolicySet,
    ManagerRecordView, ManagerRelationshipView, McpToolDefinition, McpToolList,
    MemoryStoreProvider, MetricAggregate, MetricQuery, MetricQueryFilter, MetricQueryResult,
    MetricResultRow, MetricRuntime, MetricRuntimeProvider, OperationKind, PolicyAction,
    ProviderBinding, ProviderNamespace, ProviderRegistry, QueryOp, RecordAccessPolicy, RiskLevel,
    RollbackAliasRequest, RuntimeCapabilities, RuntimeConfig, RuntimeInfo, RuntimeMetric,
    RuntimeMetricCache, RuntimeMetricCatalog, RuntimeMetricDimension, RuntimeMetricKind,
    RuntimeOperationalIndex, RuntimePack, RuntimeSnapshot, SorxDeployment, SorxError, SorxResult,
    SorxRuntime, SorxRuntimeConfig, StageDeploymentRequest, StdoutAuditSink,
    StdoutBusinessEventSink, StoreProviderKind, TrafficUpdateRequest, apply_value_patch,
    command_event_topic, entity_event_topic, filter_manager_view, generate_manager_view,
    humanize_identifier, localize_manager_view, render_dashboard_card, render_record_create_card,
    render_record_detail_card, render_record_picker_card, render_relationship_summary_card,
    resolve_manager_context,
};
use greentic_sorx_pack::{
    BusinessAction, BusinessActionAssets, LoadedSorlaPack, MetricDefinition, contract_hash,
};
use serde::Serialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::admin_roles::AdminRolesOverlay;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RouteList {
    pub schema: String,
    pub routes: Vec<RouteInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RouteInfo {
    pub method: String,
    pub path: String,
    pub endpoint_id: String,
    pub operation_id: String,
    pub risk: String,
    pub deployment_id: String,
    pub pack_name: String,
    pub pack_version: String,
    pub pack_digest: Option<String>,
    pub api_version_label: String,
    pub view_version: String,
    pub canonical_version: String,
    pub state_namespace: String,
    pub exposure: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteVersionMetadata {
    pub api_version_label: String,
    pub view_version: String,
    pub canonical_version: String,
    pub state_namespace: String,
}

impl RouteVersionMetadata {
    pub fn local(config: &SorxRuntimeConfig, pack: &LoadedSorlaPack) -> Self {
        Self {
            api_version_label: config.deployment.api_version_label.clone(),
            view_version: config.deployment.api_version_label.clone(),
            canonical_version: pack.pack_version.clone(),
            state_namespace: format!(
                "sorx/{}/{}",
                clean_route_segment(&config.deployment.tenant_id),
                clean_route_segment(&config.deployment.sor_name)
            ),
        }
    }

    pub fn from_deployment(deployment: &SorxDeployment) -> Self {
        Self {
            api_version_label: deployment.api_version_label.clone(),
            view_version: deployment.view_version.clone(),
            canonical_version: deployment.canonical_version.clone(),
            state_namespace: deployment.state_namespace.clone(),
        }
    }
}

#[derive(Clone)]
pub struct HttpRuntime {
    deployment_id: String,
    admin_api_enabled: bool,
    auth: HttpAuth,
    runtime: Arc<SorxRuntime>,
    routes: Arc<RouteList>,
    tools: Arc<McpToolList>,
    business_actions: Arc<Option<BusinessActionAssets>>,
    manager_title: Arc<String>,
    manager_description: Arc<String>,
    manager_record_descriptions: Arc<BTreeMap<String, String>>,
    manager_model_records: Arc<BTreeMap<String, ManagerModelRecord>>,
    manager_record_fields: Arc<BTreeMap<String, Vec<ManagerFieldView>>>,
    manager_hierarchy: Arc<ManagerRecordHierarchy>,
    manager_ontology: Arc<ManagerOntologyMetadata>,
    metrics: Arc<Option<RuntimeMetricCatalog>>,
    metric_provider: Arc<StoreMetricProvider>,
    runtime_snapshot: Arc<RwLock<RuntimeSnapshot>>,
    /// Path to the persisted deployment registry JSON file. When set, the
    /// `/v1/sorx/*` deployment/alias control-plane endpoints are served from
    /// this store; when `None` they return 501 (back-compat).
    registry_path: Arc<Option<PathBuf>>,
    /// When set, the deployment's policy roles are sourced from the admin
    /// system-of-record instead of the self-asserted `x-greentic-caller-role`
    /// header. `None` (the default) preserves the legacy header-roles behavior.
    admin_roles_overlay: Option<Arc<AdminRolesOverlay>>,
}

#[derive(Clone)]
struct HttpAuth {
    shared_secret: Option<String>,
}

impl HttpRuntime {
    #[allow(dead_code)]
    pub fn from_pack(
        deployment_id: impl Into<String>,
        pack: &LoadedSorlaPack,
        config: SorxRuntimeConfig,
    ) -> SorxResult<Self> {
        Self::from_pack_with_runtime_config(deployment_id, pack, config, None)
    }

    pub fn from_pack_with_runtime_config(
        deployment_id: impl Into<String>,
        pack: &LoadedSorlaPack,
        config: SorxRuntimeConfig,
        runtime_config: Option<RuntimeConfig>,
    ) -> SorxResult<Self> {
        let mut router = EndpointRouter::from_agent_gateway(&pack.sorla_assets.agent_gateway_json)?;
        apply_model_endpoint_authorization(pack, &mut router);
        let providers = provider_registry(&config)?;
        let runtime = configure_runtime_events(
            configure_runtime_audit(
                SorxRuntime::new(
                    RuntimePack {
                        name: pack.pack_name.clone(),
                        version: pack.pack_version.clone(),
                        digest: pack.pack_digest.clone(),
                        operational_indexes: runtime_operational_indexes(pack),
                        record_access: runtime_record_access(pack),
                    },
                    config.clone(),
                    router,
                    providers,
                ),
                &config,
            ),
            &config,
        )?;
        let deployment_id = deployment_id.into();
        let exposure = config.exposure.default_visibility.clone();
        let auth = http_auth_from_config(&config)?;
        let route_versions = RouteVersionMetadata::local(&config, pack);
        let routes = route_list(
            &deployment_id,
            &exposure,
            pack,
            &runtime.router,
            &route_versions,
        );
        let tools = greentic_sorx_core::mcp_tools_from_metadata(
            pack.sorla_assets.mcp_tools_json.as_ref(),
            &runtime.router,
        )?;
        let metrics = runtime_metric_catalog(pack)?;
        let manager_title = manager_title(pack);
        let manager_description = manager_description(pack, &runtime.router);
        let manager_record_descriptions = manager_record_descriptions(pack);
        let manager_model_records = manager_model_records(pack);
        let manager_record_fields = manager_model_records
            .iter()
            .map(|(record, model_record)| (record.clone(), model_record.fields.clone()))
            .collect::<BTreeMap<_, _>>();
        let manager_hierarchy = manager_record_hierarchy(&pack.sorla_assets.agent_gateway_json);
        let manager_ontology = manager_ontology_metadata(pack);
        let tools = with_metric_tools(tools, metrics.as_ref());
        let runtime_snapshot = match runtime_config {
            Some(config) => RuntimeSnapshot::from_runtime_config("runtime-main", config)?,
            None => RuntimeSnapshot::new("runtime-main"),
        };
        let metric_provider = StoreMetricProvider {
            providers: runtime.providers.clone(),
            entity_provider_ids: runtime
                .config
                .bindings
                .entity_bindings
                .iter()
                .map(|(entity, binding)| (entity.clone(), binding.provider_id.clone()))
                .collect(),
            default_provider_id: runtime.config.bindings.default_provider_id().to_string(),
        };
        Ok(Self {
            deployment_id,
            admin_api_enabled: false,
            auth,
            runtime: Arc::new(runtime),
            routes: Arc::new(routes),
            tools: Arc::new(tools),
            business_actions: Arc::new(pack.sorla_assets.business_actions.clone()),
            manager_title: Arc::new(manager_title),
            manager_description: Arc::new(manager_description),
            manager_record_descriptions: Arc::new(manager_record_descriptions),
            manager_model_records: Arc::new(manager_model_records),
            manager_record_fields: Arc::new(manager_record_fields),
            manager_hierarchy: Arc::new(manager_hierarchy),
            manager_ontology: Arc::new(manager_ontology),
            metrics: Arc::new(metrics),
            metric_provider: Arc::new(metric_provider),
            runtime_snapshot: Arc::new(RwLock::new(runtime_snapshot)),
            registry_path: Arc::new(None),
            admin_roles_overlay: None,
        })
    }

    /// Attach a persisted deployment registry file so the `/v1/sorx/*`
    /// deployment/alias control-plane and routing-table endpoints are served
    /// from it. Passing a path enables the admin API surface; passing `None`
    /// disables it again so those endpoints fall back to 501/disabled.
    pub fn with_registry_path(mut self, registry_path: Option<PathBuf>) -> Self {
        self.admin_api_enabled = registry_path.is_some();
        self.registry_path = Arc::new(registry_path);
        self
    }

    /// Attaches an admin-backed roles overlay. When present, the request path
    /// sources caller roles from the admin system-of-record and ignores the
    /// self-asserted `x-greentic-caller-role` header (security: callers must not
    /// be able to grant themselves roles).
    pub fn with_admin_roles_overlay(mut self, overlay: Arc<AdminRolesOverlay>) -> Self {
        self.admin_roles_overlay = Some(overlay);
        self
    }

    #[cfg(test)]
    fn route_list(&self) -> &RouteList {
        &self.routes
    }

    /// Returns a shared handle to the underlying [`SorxRuntime`] so background
    /// tasks (e.g. the NATS event bridge) can invoke endpoints after the HTTP
    /// runtime is built. The handle is reference-counted; cloning it does not
    /// duplicate runtime state.
    #[cfg(feature = "events-nats")]
    pub fn runtime_handle(&self) -> Arc<SorxRuntime> {
        self.runtime.clone()
    }

    pub fn serve(&self, listener: TcpListener) -> std::io::Result<()> {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let runtime = self.clone();
                    std::thread::spawn(move || {
                        let _ = runtime.handle_stream(stream);
                    });
                }
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }

    fn handle_stream(&self, mut stream: TcpStream) -> std::io::Result<()> {
        let response = match read_request(&mut stream) {
            Ok(request) => self.handle_request(request),
            Err(err) => json_response(
                400,
                json!({
                    "ok": false,
                    "error": {
                        "code": "SORX_BAD_REQUEST",
                        "message": err,
                        "details": {}
                    }
                }),
            ),
        };
        let bytes = response.as_bytes();
        stream.write_all(&bytes)
    }

    fn handle_request(&self, request: HttpRequest) -> HttpResponse {
        if request.method == "OPTIONS" {
            return json_response(200, json!({ "ok": true }));
        }

        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/healthz") => return json_response(200, json!({ "ok": true })),
            ("GET", "/readyz") => return json_response(200, json!({ "ok": true })),
            _ => {}
        }

        if let Err(response) = self.authorize(&request) {
            return response;
        }

        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/v1/sorx/routes") => {
                return json_response(200, serde_json::to_value(&*self.routes).unwrap());
            }
            ("GET", "/v1/sorx/public-routes") => {
                let routes = RouteList {
                    schema: "greentic.sorx.public-routes.v1".to_string(),
                    routes: self
                        .routes
                        .routes
                        .iter()
                        .filter(|route| route.exposure == "public")
                        .cloned()
                        .collect(),
                };
                return json_response(200, serde_json::to_value(routes).unwrap());
            }
            ("GET", "/v1/sorx/tools") => {
                return json_response(
                    200,
                    json!({
                        "schema": "greentic.sorx.tools.v1",
                        "tools": &self.tools.tools
                    }),
                );
            }
            _ => {}
        }

        if request.path == "/v1/sorx/metrics" && request.method == "GET" {
            return self.list_metrics(&request);
        }
        if request.path.starts_with("/v1/sorx/metrics/") {
            return self.handle_metric_request(&request);
        }

        if request.path == "/v1/sorx/manager"
            || request.path.starts_with("/v1/sorx/manager/")
            || request.path == "/manager"
            || request.path.starts_with("/manager/")
        {
            return self.handle_manager_request(&request);
        }

        if request.path.starts_with("/admin/v1/") || self.is_admin_surface_request(&request.path) {
            return self.handle_generic_admin_request(&request);
        }

        if request.path == "/v1/sorx/business-actions" && request.method == "GET" {
            return self.list_business_actions();
        }
        if request.path.starts_with("/v1/sorx/business-actions/") {
            return self.handle_business_action_request(&request);
        }

        let deployment_routes_path = format!("/v1/sorx/deployments/{}/routes", self.deployment_id);
        if request.method == "GET" && request.path == deployment_routes_path {
            return json_response(200, serde_json::to_value(&*self.routes).unwrap());
        }

        let promotion_status_path = format!(
            "/v1/sorx/deployments/{}/promotion-status",
            self.deployment_id
        );
        if request.method == "GET" && request.path == promotion_status_path {
            return json_response(
                200,
                json!({
                    "schema": "greentic.sorx.promotion-status.v1",
                    "deployment_id": self.deployment_id,
                    "registry_backed": false,
                    "public_route_count": self.routes.routes.iter().filter(|route| route.exposure == "public").count(),
                    "reason": "local runtime diagnostics do not include deployment registry validation reports"
                }),
            );
        }

        if is_admin_api_path(&request.path) {
            if !self.admin_api_enabled {
                return error_response(404, "SORX_ADMIN_API_DISABLED", "admin API is disabled");
            }
            return match self.registry_path.as_ref() {
                Some(path) => handle_registry_request(
                    &LocalDeploymentRegistryStore::new(path.clone()),
                    &request.method,
                    &request.path,
                    &request.body,
                ),
                None => error_response(
                    501,
                    "SORX_ADMIN_API_NOT_IMPLEMENTED",
                    "admin API storage is provided by the CLI registry in this build",
                ),
            };
        }

        let Some((endpoint, path_params)) = self.match_endpoint(&request.method, &request.path)
        else {
            return error_response(404, "SORX_ROUTE_NOT_FOUND", "route not found");
        };

        let tenant_id = match header_or_local(
            &request.headers,
            "x-greentic-tenant-id",
            &self.runtime.config.tenant_id,
            &self.runtime.config.environment,
        ) {
            Ok(value) => value,
            Err(err) => return sorx_error_response(400, err),
        };
        let caller_id = match header_or_local(
            &request.headers,
            "x-greentic-caller-id",
            "local",
            &self.runtime.config.environment,
        ) {
            Ok(value) => value,
            Err(err) => return sorx_error_response(400, err),
        };
        let header_roles = header_caller_roles(&request.headers);
        // Trusted lookup key for the admin overlay. The router sets this to the
        // authenticated user's email; it is NOT derived from the free-form
        // `x-greentic-caller-id` subject.
        let caller_email = request
            .headers
            .get("x-greentic-caller-email")
            .map(String::as_str);
        let roles = compute_effective_roles(
            self.admin_roles_overlay.as_deref(),
            &tenant_id,
            caller_email,
            header_roles,
        );

        let input = match request_json(&request, &path_params, Some(&endpoint)) {
            Ok(value) => value,
            Err(err) => return error_response(400, "SORX_INVALID_JSON", &err),
        };
        let invocation = EndpointInvocation {
            tenant_id,
            endpoint_id: endpoint.endpoint_id.clone(),
            operation_id: endpoint.operation_id.clone(),
            input,
            caller: CallerContext {
                subject: caller_id,
                roles,
            },
            idempotency_key: request.headers.get("idempotency-key").cloned(),
            source: InvocationSource::Http,
        };

        match self.runtime.invoke(invocation) {
            Ok(result) if result.status == EndpointStatus::ApprovalRequired => json_response(
                202,
                json!({
                    "ok": false,
                    "status": "approval_required",
                    "approval": result.output["approval"]
                }),
            ),
            Ok(result) if result.status == EndpointStatus::Denied => json_response(
                403,
                json!({
                    "ok": false,
                    "error": {
                        "code": "SORX_POLICY_DENIED",
                        "message": result.output["reason"].as_str().unwrap_or("operation denied"),
                        "details": result.output
                    }
                }),
            ),
            Ok(result) if matches!(endpoint.operation, OperationKind::Command(_)) => {
                let mut body = result.output;
                if let Value::Object(object) = &mut body {
                    object.insert("events".to_string(), json!(result.events));
                }
                json_response(200, body)
            }
            Ok(result) => json_response(
                200,
                json!({
                    "ok": true,
                    "endpoint_id": endpoint.endpoint_id,
                    "operation_id": endpoint.operation_id,
                    "result": result.output,
                    "events": result.events
                }),
            ),
            Err(err) => sorx_error_response(400, err),
        }
    }

    fn authorize(&self, request: &HttpRequest) -> Result<(), HttpResponse> {
        let Some(secret) = self.auth.shared_secret.as_deref() else {
            return Ok(());
        };
        if request_bearer_token(&request.headers).is_some_and(|token| token == secret)
            || request
                .headers
                .get("x-greentic-sorx-secret")
                .is_some_and(|token| token == secret)
        {
            return Ok(());
        }
        Err(error_response(
            401,
            "SORX_UNAUTHORIZED",
            "valid HTTP ingest shared secret is required",
        ))
    }

    fn handle_generic_admin_request(&self, request: &HttpRequest) -> HttpResponse {
        let action_id = generic_admin_action_id(request);
        let actor = request
            .headers
            .get("x-greentic-caller-id")
            .cloned()
            .unwrap_or_else(|| "runtime-admin".to_string());
        let tenant_id = request.headers.get("x-greentic-tenant-id").cloned();
        let context =
            self.runtime
                .admin_action_context(action_id, request.path.clone(), actor, tenant_id);
        let mut admin_request = AdminActionRequest {
            method: request.method.clone(),
            path: request.path.clone(),
            input: if request.body.trim().is_empty() {
                Value::Null
            } else {
                serde_json::from_str(&request.body).unwrap_or(Value::Null)
            },
        };

        if let Err(err) = self.runtime.observe_admin(AdminObserverEvent {
            event_type: "admin.action.started".to_string(),
            context: context.clone(),
            status: None,
            duration_ms: None,
            control_decision: None,
        }) {
            return sorx_error_response(500, err);
        }

        let pre_decision = match self.runtime.pre_admin(&context, &admin_request) {
            Ok(decision) => decision,
            Err(err) => return sorx_error_response(403, err),
        };
        match pre_decision.action {
            ControlDecisionAction::Allow => {}
            ControlDecisionAction::AllowWithPatch => {
                if let Some(patch) = &pre_decision.patch {
                    apply_value_patch(&mut admin_request.input, patch);
                }
            }
            ControlDecisionAction::Deny => {
                let response = json_response(
                    403,
                    json!({
                        "ok": false,
                        "error": {
                            "code": "RUNTIME_ADMIN_CONTROL_DENIED",
                            "message": pre_decision.reason.clone().unwrap_or_else(|| "admin action denied".to_string()),
                            "details": {}
                        }
                    }),
                );
                let _ = self.runtime.observe_admin(AdminObserverEvent {
                    event_type: "admin.action.denied".to_string(),
                    context,
                    status: Some("denied".to_string()),
                    duration_ms: None,
                    control_decision: Some(pre_decision),
                });
                return response;
            }
        }

        let effective_request = if admin_request.input == Value::Null {
            request.clone()
        } else {
            let mut request = request.clone();
            request.body = admin_request.input.to_string();
            request
        };
        let mut response = self.handle_generic_admin_request_inner(&effective_request);
        let mut admin_response = AdminActionResponse {
            status: response.status,
            output: response.body.clone(),
        };
        let post_decision = match self
            .runtime
            .post_admin(&context, &admin_request, &admin_response)
        {
            Ok(decision) => decision,
            Err(err) => return sorx_error_response(403, err),
        };
        match post_decision.action {
            ControlDecisionAction::Allow => {}
            ControlDecisionAction::AllowWithPatch => {
                if let Some(patch) = &post_decision.patch {
                    apply_value_patch(&mut response.body, patch);
                    admin_response.output = response.body.clone();
                }
            }
            ControlDecisionAction::Deny => {
                response = json_response(
                    403,
                    json!({
                        "ok": false,
                        "error": {
                            "code": "RUNTIME_ADMIN_CONTROL_DENIED",
                            "message": post_decision.reason.clone().unwrap_or_else(|| "admin response denied".to_string()),
                            "details": {}
                        }
                    }),
                );
                admin_response.status = response.status;
                admin_response.output = response.body.clone();
            }
        }
        if let Err(err) = self.runtime.observe_admin(AdminObserverEvent {
            event_type: generic_admin_terminal_event(response.status).to_string(),
            context,
            status: Some(response.status.to_string()),
            duration_ms: None,
            control_decision: Some(post_decision),
        }) {
            return sorx_error_response(500, err);
        }
        response
    }

    fn handle_generic_admin_request_inner(&self, request: &HttpRequest) -> HttpResponse {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/admin/v1/runtime") => {
                return json_response(
                    200,
                    serde_json::to_value(RuntimeInfo::sorx(
                        "runtime-main",
                        env!("CARGO_PKG_VERSION"),
                    ))
                    .unwrap(),
                );
            }
            ("GET", "/admin/v1/health") => {
                return self.with_snapshot_read(|snapshot| {
                    json_response(200, serde_json::to_value(snapshot.health()).unwrap())
                });
            }
            ("GET", "/admin/v1/capabilities") => {
                return json_response(
                    200,
                    serde_json::to_value(self.runtime_capabilities()).unwrap(),
                );
            }
            ("POST", "/admin/v1/capabilities/invoke") => {
                return self.invoke_capability(request);
            }
            ("GET", "/admin/v1/deployments") => {
                return self.with_snapshot_read(|snapshot| {
                    json_response(200, serde_json::to_value(snapshot.deployments()).unwrap())
                });
            }
            ("POST", "/admin/v1/deployments/stage") => {
                let request =
                    match request_json(request, &BTreeMap::new(), None).and_then(|value| {
                        serde_json::from_value::<StageDeploymentRequest>(value)
                            .map_err(|err| err.to_string())
                    }) {
                        Ok(request) => request,
                        Err(err) => return error_response(400, "RUNTIME_STAGE_INVALID", &err),
                    };
                return self.with_snapshot_write(|snapshot| match snapshot.stage(request) {
                    Ok(deployment) => json_response(200, serde_json::to_value(deployment).unwrap()),
                    Err(err) => sorx_error_response(400, err),
                });
            }
            ("POST", "/admin/v1/runtime-config") => {
                let config = match request_json(request, &BTreeMap::new(), None).and_then(|value| {
                    serde_json::from_value::<RuntimeConfig>(value).map_err(|err| err.to_string())
                }) {
                    Ok(config) => config,
                    Err(err) => return error_response(400, "RUNTIME_CONFIG_INVALID", &err),
                };
                return self.with_snapshot_write(|snapshot| {
                    match snapshot.apply_runtime_config(config) {
                        Ok(()) => json_response(
                            200,
                            serde_json::to_value(snapshot.deployments()).unwrap(),
                        ),
                        Err(err) => sorx_error_response(400, err),
                    }
                });
            }
            ("GET", "/admin/v1/admin-surfaces") => {
                return self.with_snapshot_read(|snapshot| {
                    json_response(
                        200,
                        serde_json::to_value(snapshot.admin_surfaces()).unwrap(),
                    )
                });
            }
            ("POST", "/admin/v1/admin-surfaces") => {
                let surface =
                    match request_json(request, &BTreeMap::new(), None).and_then(|value| {
                        serde_json::from_value::<AdminSurface>(value).map_err(|err| err.to_string())
                    }) {
                        Ok(surface) => surface,
                        Err(err) => {
                            return error_response(400, "RUNTIME_ADMIN_SURFACE_INVALID", &err);
                        }
                    };
                return self.with_snapshot_write(|snapshot| {
                    match snapshot.register_admin_surface(surface) {
                        Ok(surface) => json_response(200, serde_json::to_value(surface).unwrap()),
                        Err(err) => sorx_error_response(400, err),
                    }
                });
            }
            _ => {}
        }

        if let Some(response) = self.handle_admin_surface_request(request) {
            return response;
        }

        let parts = request
            .path
            .trim_start_matches("/admin/v1/deployments/")
            .trim_matches('/')
            .split('/')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        match (request.method.as_str(), parts.as_slice()) {
            ("GET", [deployment_id]) => {
                self.with_snapshot_read(|snapshot| match snapshot.deployment(deployment_id) {
                    Ok(deployment) => json_response(200, serde_json::to_value(deployment).unwrap()),
                    Err(err) => sorx_error_response(404, err),
                })
            }
            ("POST", [deployment_id, "warm"]) => {
                self.with_snapshot_write(|snapshot| match snapshot.warm(deployment_id) {
                    Ok(deployment) => json_response(200, serde_json::to_value(deployment).unwrap()),
                    Err(err) => sorx_error_response(400, err),
                })
            }
            ("POST", [deployment_id, "activate"]) => {
                self.with_snapshot_write(|snapshot| match snapshot.activate(deployment_id) {
                    Ok(deployment) => json_response(200, serde_json::to_value(deployment).unwrap()),
                    Err(err) => sorx_error_response(400, err),
                })
            }
            ("POST", [deployment_id, "traffic"]) => {
                let request =
                    match request_json(request, &BTreeMap::new(), None).and_then(|value| {
                        serde_json::from_value::<TrafficUpdateRequest>(value)
                            .map_err(|err| err.to_string())
                    }) {
                        Ok(request) => request,
                        Err(err) => return error_response(400, "RUNTIME_TRAFFIC_INVALID", &err),
                    };
                self.with_snapshot_write(|snapshot| {
                    match snapshot.set_traffic(deployment_id, request) {
                        Ok(split) => json_response(200, serde_json::to_value(split).unwrap()),
                        Err(err) => sorx_error_response(400, err),
                    }
                })
            }
            ("POST", [deployment_id, "revisions", revision_id, "drain"]) => self
                .with_snapshot_write(
                    |snapshot| match snapshot.drain(deployment_id, revision_id) {
                        Ok(deployment) => {
                            json_response(200, serde_json::to_value(deployment).unwrap())
                        }
                        Err(err) => sorx_error_response(400, err),
                    },
                ),
            ("POST", [deployment_id, "deactivate"]) => {
                self.with_snapshot_write(|snapshot| match snapshot.deactivate(deployment_id) {
                    Ok(deployment) => json_response(200, serde_json::to_value(deployment).unwrap()),
                    Err(err) => sorx_error_response(400, err),
                })
            }
            _ => error_response(
                404,
                "RUNTIME_ADMIN_ROUTE_NOT_FOUND",
                "generic runtime admin route not found",
            ),
        }
    }

    fn handle_admin_surface_request(&self, request: &HttpRequest) -> Option<HttpResponse> {
        let surface = match self.runtime_snapshot.read() {
            Ok(snapshot) => snapshot.admin_surface_for_path(&request.path),
            Err(_) => {
                return Some(error_response(
                    500,
                    "RUNTIME_SNAPSHOT_LOCKED",
                    "runtime snapshot lock poisoned",
                ));
            }
        }?;
        Some(match (request.method.as_str(), request.path.as_str()) {
            ("GET", path) if path == surface.path => json_response(
                200,
                json!({
                    "schema": "greentic.runtime.admin-surface.v1",
                    "surface": surface
                }),
            ),
            _ => error_response(
                501,
                "RUNTIME_ADMIN_SURFACE_UNSUPPORTED_HANDLER",
                "admin surface handler contract is not supported by this runtime build",
            ),
        })
    }

    fn is_admin_surface_request(&self, path: &str) -> bool {
        match self.runtime_snapshot.read() {
            Ok(snapshot) => snapshot.admin_surface_for_path(path).is_some(),
            Err(_) => false,
        }
    }

    fn with_snapshot_read<F>(&self, f: F) -> HttpResponse
    where
        F: FnOnce(&RuntimeSnapshot) -> HttpResponse,
    {
        match self.runtime_snapshot.read() {
            Ok(snapshot) => f(&snapshot),
            Err(_) => error_response(
                500,
                "RUNTIME_SNAPSHOT_LOCKED",
                "runtime snapshot lock poisoned",
            ),
        }
    }

    fn with_snapshot_write<F>(&self, f: F) -> HttpResponse
    where
        F: FnOnce(&mut RuntimeSnapshot) -> HttpResponse,
    {
        match self.runtime_snapshot.write() {
            Ok(mut snapshot) => f(&mut snapshot),
            Err(_) => error_response(
                500,
                "RUNTIME_SNAPSHOT_LOCKED",
                "runtime snapshot lock poisoned",
            ),
        }
    }

    fn runtime_capabilities(&self) -> RuntimeCapabilities {
        let mut capabilities = RuntimeCapabilities::sorx_runtime_host();
        capabilities.offers.extend(self.business_action_offers());
        capabilities
            .offers
            .extend(self.business_event_topic_offers());
        capabilities
    }

    fn business_action_offers(&self) -> Vec<CapabilityOffer> {
        let Some(assets) = self.business_actions.as_ref() else {
            return Vec::new();
        };
        assets
            .catalog
            .actions
            .iter()
            .map(|action| {
                let endpoint = self.execution_endpoint(action);
                let locked_contract_hash = self.locked_contract_hash(action);
                CapabilityOffer {
                    capability: business_action_capability(&self.runtime.pack.name, action),
                    contracts: vec!["greentic.sorx.business-action.invoke.v1".to_string()],
                    metadata: Some(json!({
                        "kind": "business_function",
                        "pack": {
                            "name": self.runtime.pack.name,
                            "version": self.runtime.pack.version,
                            "digest": self.runtime.pack.digest
                        },
                        "action": {
                            "id": action.id,
                            "version": action.version,
                            "label": action.label,
                            "aliases": action.aliases,
                            "contract_hash": locked_contract_hash.unwrap_or_else(|| contract_hash(action)),
                            "risk": action.risk,
                            "approval": action.approval,
                            "idempotency": action.idempotency
                        },
                        "execution": {
                            "endpoint_id": endpoint.map(|endpoint| endpoint.endpoint_id.clone()),
                            "operation_id": endpoint.map(|endpoint| endpoint.operation_id.clone()),
                            "tool_name": action.execution.tool_name
                        }
                    })),
                }
            })
            .collect()
    }

    fn business_event_topic_offers(&self) -> Vec<CapabilityOffer> {
        let mut event_topics = BTreeMap::<String, Value>::new();

        // --- command-emitted (domain) event topics ---
        for endpoint in self.runtime.router.endpoints.values() {
            let OperationKind::Command(spec) = &endpoint.operation else {
                continue;
            };
            let mut events = Vec::new();
            collect_command_event_topics(&spec.steps, &mut events);
            for event in events {
                let topic = command_event_topic(&self.runtime.pack.name, &event);
                event_topics.entry(event.clone()).or_insert_with(|| {
                    json!({
                        "kind": "business_event_topic",
                        "pack": {
                            "name": self.runtime.pack.name,
                            "version": self.runtime.pack.version,
                            "digest": self.runtime.pack.digest
                        },
                        "event_type": event,
                        "topic": topic,
                        "producer": format!("sorx:{}:{}", self.runtime.pack.name, self.runtime.pack.version),
                        "source_endpoint_id": endpoint.endpoint_id,
                        "source_operation_id": endpoint.operation_id
                    })
                });
            }
        }

        // --- entity lifecycle topics (create / update / delete) ---
        for endpoint in self.runtime.router.endpoints.values() {
            let operation_label = match &endpoint.operation {
                OperationKind::Create => "created",
                OperationKind::Update => "updated",
                OperationKind::Delete => "deleted",
                _ => continue,
            };
            let Ok(binding) = self.runtime.config.bindings.resolve(endpoint) else {
                continue;
            };
            let event_type = format!("{}.{operation_label}", binding.entity);
            let topic =
                entity_event_topic(&self.runtime.pack.name, &binding.entity, operation_label);
            event_topics.entry(event_type.clone()).or_insert_with(|| {
                json!({
                    "kind": "business_event_topic",
                    "pack": {
                        "name": self.runtime.pack.name,
                        "version": self.runtime.pack.version,
                        "digest": self.runtime.pack.digest
                    },
                    "event_type": event_type,
                    "topic": topic,
                    "producer": format!("sorx:{}:{}", self.runtime.pack.name, self.runtime.pack.version),
                    "source_endpoint_id": endpoint.endpoint_id,
                    "source_operation_id": endpoint.operation_id
                })
            });
        }

        event_topics
            .into_iter()
            .map(|(event_type, metadata)| CapabilityOffer {
                capability: business_event_capability(&self.runtime.pack.name, &event_type),
                contracts: vec!["greentic.sorx.business-event-topic.v1".to_string()],
                metadata: Some(metadata),
            })
            .collect()
    }

    fn invoke_capability(&self, request: &HttpRequest) -> HttpResponse {
        let body = match request_json(request, &BTreeMap::new(), None) {
            Ok(value) => value,
            Err(err) => return error_response(400, "RUNTIME_CAPABILITY_INVOKE_INVALID", &err),
        };
        let Some(capability) = body.get("capability").and_then(Value::as_str) else {
            return error_response(
                400,
                "RUNTIME_CAPABILITY_INVOKE_INVALID",
                "capability is required",
            );
        };
        let Some(action) = self.find_business_action_by_capability(capability) else {
            return error_response(
                404,
                "RUNTIME_CAPABILITY_NOT_FOUND",
                "capability does not resolve to a business action",
            );
        };
        let Some(endpoint) = self.execution_endpoint(action) else {
            return error_response(
                404,
                "RUNTIME_CAPABILITY_TARGET_MISSING",
                "capability execution target is missing",
            );
        };
        let values = body.get("input").cloned().unwrap_or_else(|| json!({}));
        if let Some(schema) = &action.input_schema
            && let Err(err) = validate_action_schema(schema, &values)
        {
            return error_response(400, "RUNTIME_CAPABILITY_INPUT_INVALID", &err);
        }
        let idempotency_key = body
            .get("idempotency_key")
            .or_else(|| {
                body.get("options")
                    .and_then(|options| options.get("idempotency_key"))
            })
            .and_then(Value::as_str)
            .map(ToString::to_string);
        if action
            .idempotency
            .as_ref()
            .is_some_and(|idempotency| idempotency.required)
            && idempotency_key.is_none()
        {
            return error_response(
                400,
                "RUNTIME_CAPABILITY_IDEMPOTENCY_REQUIRED",
                "idempotency key is required",
            );
        }
        let dry_run = body
            .get("dry_run")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let policy_decision = self.runtime.policy.decide(endpoint);
        if dry_run {
            return json_response(
                200,
                json!({
                    "valid": true,
                    "capability": capability,
                    "canonical_payload": values,
                    "policy_decision": policy_decision_label(&policy_decision.action),
                    "approval_required": matches!(policy_decision.action, PolicyAction::RequireApproval),
                    "execution_target": execution_target_json(action, endpoint)
                }),
            );
        }
        let context = body.get("context").and_then(Value::as_object);
        let tenant_id = context
            .and_then(|context| context.get("tenant_id").and_then(Value::as_str))
            .or_else(|| body.get("tenant_id").and_then(Value::as_str))
            .or_else(|| {
                request
                    .headers
                    .get("x-greentic-tenant-id")
                    .map(String::as_str)
            })
            .unwrap_or(&self.runtime.config.tenant_id)
            .to_string();
        let caller_id = context
            .and_then(|context| context.get("caller_id").and_then(Value::as_str))
            .or_else(|| body.get("caller_id").and_then(Value::as_str))
            .or_else(|| {
                request
                    .headers
                    .get("x-greentic-caller-id")
                    .map(String::as_str)
            })
            .unwrap_or("capability")
            .to_string();
        let roles = context
            .and_then(|context| context.get("roles"))
            .and_then(string_array)
            .filter(|roles| !roles.is_empty())
            .unwrap_or_else(|| request_roles(&request.headers));
        let invocation = EndpointInvocation {
            tenant_id,
            endpoint_id: endpoint.endpoint_id.clone(),
            operation_id: endpoint.operation_id.clone(),
            input: values,
            caller: CallerContext {
                subject: caller_id,
                roles,
            },
            idempotency_key,
            source: InvocationSource::Direct,
        };
        match self.runtime.invoke(invocation) {
            Ok(result) if result.status == EndpointStatus::ApprovalRequired => json_response(
                202,
                json!({
                    "ok": false,
                    "status": "approval_required",
                    "capability": capability,
                    "approval": result.output["approval"],
                    "action_ref": capability_action_ref_json(action, self.locked_contract_hash(action))
                }),
            ),
            Ok(result) if result.status == EndpointStatus::Denied => json_response(
                403,
                json!({
                    "ok": false,
                    "error": {
                        "code": "RUNTIME_CAPABILITY_DENIED",
                        "message": result.output["reason"].as_str().unwrap_or("capability invocation denied"),
                        "details": result.output
                    }
                }),
            ),
            Ok(result) => json_response(
                200,
                json!({
                    "ok": true,
                    "schema": "greentic.sorx.capability-invoke-result.v1",
                    "capability": capability,
                    "action_ref": capability_action_ref_json(action, self.locked_contract_hash(action)),
                    "status": format!("{:?}", result.status).to_ascii_lowercase(),
                    "result": result.output,
                    "events": result.events
                }),
            ),
            Err(err) => sorx_error_response(400, err),
        }
    }

    fn handle_manager_request(&self, request: &HttpRequest) -> HttpResponse {
        let path = request_path_without_query(&request.path);
        let suffix = path
            .trim_start_matches("/v1/sorx/manager")
            .trim_start_matches("/manager")
            .trim_matches('/');
        let parts = if suffix.is_empty() {
            Vec::new()
        } else {
            suffix.split('/').collect::<Vec<_>>()
        };
        match (request.method.as_str(), parts.as_slice()) {
            ("GET", []) => json_response(
                200,
                json!({
                    "schema": "greentic.sorx.manager-shell.v1",
                    "view": "/v1/sorx/manager/view",
                    "dashboard_card": "/v1/sorx/manager/cards/dashboard",
                    "graph": "/v1/sorx/manager/graph.json"
                }),
            ),
            ("GET", ["view"]) => self.manager_view_response(request),
            ("GET", ["cards", "dashboard"]) => self.manager_dashboard_card_response(request),
            ("GET", ["cards", "metrics"]) => self.manager_metrics_card_response(request),
            ("GET", ["cards", "metrics", metric]) => {
                self.manager_metric_detail_card_response(request, metric)
            }
            ("GET", ["cards", "records", record]) => {
                self.manager_record_list_card_response(request, record)
            }
            ("GET", ["cards", "records", record, "create"]) => {
                self.manager_record_create_card_response(request, record)
            }
            ("GET", ["cards", "pickers", record]) => {
                self.manager_card_response(request, |view| render_record_picker_card(view, record))
            }
            ("GET", ["cards", "records", record, id]) => {
                self.manager_record_detail_card_response(request, record, id)
            }
            ("GET", ["cards", "records", record, id, "delete"]) => {
                self.manager_record_delete_card_response(request, record, id)
            }
            ("GET", ["cards", "relationships"]) => self.manager_card_response(request, |view| {
                Some(render_relationship_summary_card(view))
            }),
            ("GET", ["graph.json"]) => self.manager_graph_json_response(request),
            ("GET", ["graph.svg"]) => self.manager_graph_svg_response(request),
            ("GET", ["pickers", record]) => self.manager_picker_response(request, record),
            ("POST", ["submit"]) => self.manager_submit_response(request),
            _ => error_response(
                404,
                "SORX_MANAGER_ROUTE_NOT_FOUND",
                "manager route not found",
            ),
        }
    }

    fn manager_record_list_card_response(
        &self,
        request: &HttpRequest,
        record_name: &str,
    ) -> HttpResponse {
        match self.manager_view(request) {
            Ok(view) => {
                let Some(record) = view
                    .records
                    .iter()
                    .find(|record| record.record == record_name)
                    .cloned()
                else {
                    return error_response(
                        404,
                        "SORX_MANAGER_RECORD_NOT_FOUND",
                        "record not found",
                    );
                };
                let mut rows = self.manager_record_rows(&view.tenant_id, record_name);
                let parent_record = request_query_param(&request.path, "parent_record")
                    .or_else(|| request_query_param(&request.path, "parentRecord"));
                let parent_id = request_query_param(&request.path, "parent_id")
                    .or_else(|| request_query_param(&request.path, "parentId"));
                if let (Some(parent_record), Some(parent_id)) =
                    (parent_record.as_deref(), parent_id.as_deref())
                {
                    self.filter_manager_rows_by_parent_context(
                        &view.tenant_id,
                        &mut rows,
                        record_name,
                        parent_record,
                        parent_id,
                    );
                }
                sort_manager_record_rows(&mut rows);
                let search = request_query_param(&request.path, "q")
                    .or_else(|| request_query_param(&request.path, "search"))
                    .unwrap_or_default();
                let search = search.trim().to_string();
                if !search.is_empty() {
                    rows.retain(|row| manager_record_matches_search(row, &search));
                }
                let page_size = 10usize;
                let page = request_query_usize(&request.path, "page")
                    .unwrap_or(1)
                    .max(1);
                let total = rows.len();
                let start = (page - 1).saturating_mul(page_size).min(total);
                let end = (start + page_size).min(total);
                let visible_rows = rows[start..end].to_vec();
                let can_create = manager_view_has_record_action(&view, record_name, "create");
                let can_update = manager_view_has_record_action(&view, record_name, "update");
                let can_delete = manager_view_has_record_action(&view, record_name, "delete");
                let card = render_runtime_record_list_card(
                    &view,
                    &record,
                    &visible_rows,
                    self.manager_record_descriptions.get(record_name).cloned(),
                    RuntimeRecordListState {
                        page,
                        page_size,
                        total,
                        start,
                        end,
                        search,
                        can_create,
                        can_update,
                        can_delete,
                        parent_context: parent_record.zip(parent_id),
                    },
                );
                json_response(200, card)
            }
            Err(err) => sorx_error_response(400, err),
        }
    }

    fn manager_view_response(&self, request: &HttpRequest) -> HttpResponse {
        match self.manager_view(request) {
            Ok(view) => json_response(200, serde_json::to_value(view).unwrap()),
            Err(err) => sorx_error_response(400, err),
        }
    }

    fn manager_dashboard_card_response(&self, request: &HttpRequest) -> HttpResponse {
        self.manager_card_response(request, |view| {
            let mut card = render_dashboard_card(view);
            let action = manager_open_action(
                localized_manager_static(&view.locale, "Metrics"),
                "metrics",
                Value::Object(Map::new()),
            );
            if let Some(actions) = card.get_mut("actions").and_then(Value::as_array_mut) {
                actions.push(action);
            }
            Some(card)
        })
    }

    fn manager_metrics_card_response(&self, request: &HttpRequest) -> HttpResponse {
        match self.manager_view(request) {
            Ok(view) => {
                let metric_summaries = self
                    .metrics
                    .as_ref()
                    .as_ref()
                    .map(|catalog| {
                        catalog
                            .metrics
                            .iter()
                            .map(|metric| {
                                let query_result = self.manager_metric_query_result(
                                    &view.tenant_id,
                                    &metric.name,
                                    Vec::new(),
                                );
                                RuntimeMetricCardRow {
                                    name: metric.name.clone(),
                                    label: metric.label.clone(),
                                    result: query_result.as_ref().ok().cloned(),
                                    error: query_result
                                        .err()
                                        .map(|err| metric_query_error_message(&err)),
                                }
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let card = render_runtime_metrics_card(&view, &metric_summaries);
                json_response(200, card)
            }
            Err(err) => sorx_error_response(400, err),
        }
    }

    fn manager_metric_detail_card_response(
        &self,
        request: &HttpRequest,
        metric_name: &str,
    ) -> HttpResponse {
        match self.manager_view(request) {
            Ok(view) => {
                let metric = self
                    .metrics
                    .as_ref()
                    .as_ref()
                    .and_then(|catalog| catalog.metric(metric_name).ok());
                let query_result = metric.map(|metric| {
                    self.manager_metric_query_result(
                        &view.tenant_id,
                        metric_name,
                        default_metric_dimensions(metric),
                    )
                });
                let result = query_result
                    .as_ref()
                    .and_then(|result| result.as_ref().ok());
                let error = query_result
                    .as_ref()
                    .and_then(|result| result.as_ref().err())
                    .map(metric_query_error_message);
                let card = render_runtime_metric_detail_card(
                    &view,
                    metric_name,
                    metric,
                    result,
                    error.as_deref(),
                );
                json_response(200, card)
            }
            Err(err) => sorx_error_response(400, err),
        }
    }

    fn manager_card_response<F>(&self, request: &HttpRequest, render: F) -> HttpResponse
    where
        F: FnOnce(&greentic_sorx_core::ManagerViewModel) -> Option<Value>,
    {
        match self.manager_view(request) {
            Ok(view) => match render(&view) {
                Some(card) => json_response(200, card),
                None => error_response(404, "SORX_MANAGER_RECORD_NOT_FOUND", "record not found"),
            },
            Err(err) => sorx_error_response(400, err),
        }
    }

    fn manager_record_create_card_response(
        &self,
        request: &HttpRequest,
        record_name: &str,
    ) -> HttpResponse {
        match self.manager_view(request) {
            Ok(view) => {
                if !view
                    .records
                    .iter()
                    .any(|record| record.record == record_name)
                {
                    return error_response(
                        404,
                        "SORX_MANAGER_RECORD_NOT_FOUND",
                        "record not found",
                    );
                }
                let endpoint = self.runtime.router.endpoints.values().find(|endpoint| {
                    endpoint.entity.as_deref() == Some(record_name)
                        && manager_endpoint_is_create_form(endpoint)
                        && !endpoint.record_selector
                });
                let parent_context = request_query_param(&request.path, "parent_record")
                    .or_else(|| request_query_param(&request.path, "parentRecord"))
                    .zip(
                        request_query_param(&request.path, "parent_id")
                            .or_else(|| request_query_param(&request.path, "parentId")),
                    );
                let Some(endpoint) = endpoint else {
                    if manager_record_action(&view, record_name, "create").is_some() {
                        return match render_record_create_card(&view, record_name) {
                            Some(mut card) => {
                                if let Some((parent_record, parent_id)) = parent_context {
                                    self.apply_manager_parent_create_context(
                                        &view,
                                        &mut card,
                                        record_name,
                                        &parent_record,
                                        &parent_id,
                                    );
                                }
                                json_response(200, card)
                            }
                            None => error_response(
                                404,
                                "SORX_MANAGER_RECORD_NOT_FOUND",
                                "record not found",
                            ),
                        };
                    }
                    return error_response(
                        404,
                        "SORX_MANAGER_CREATE_NOT_FOUND",
                        "create endpoint not found",
                    );
                };
                let context = match self.manager_context(request) {
                    Ok(context) => context,
                    Err(err) => return sorx_error_response(400, err),
                };
                let router = match EndpointRouter::new([endpoint.clone()]) {
                    Ok(router) => router,
                    Err(err) => return sorx_error_response(400, err),
                };
                let mut scoped = generate_manager_view(&context, &router, &self.runtime.policy);
                scoped.title = view.title.clone();
                scoped.description = view.description.clone();
                scoped.locale = view.locale.clone();
                apply_manager_model_records(
                    &mut scoped,
                    &self.manager_model_records,
                    &self.manager_hierarchy,
                    &context.roles,
                );
                apply_manager_record_fields(&mut scoped, &self.manager_record_fields);
                apply_manager_ontology_metadata(&mut scoped, &self.manager_ontology);
                carry_manager_field_relationships(&mut scoped, &view);
                self.populate_manager_relationship_choices(&mut scoped, &context.tenant_id);
                let policies = manager_authorization_policy_set(&self.runtime, &context.roles);
                let mut scoped = filter_manager_view(scoped, &policies);
                let locale = ManagerLocaleContext::new(context.locale.clone(), "en");
                scoped = localize_manager_view(scoped, &locale, &builtin_manager_locale_bundle());
                apply_builtin_manager_text_translations(&mut scoped, &locale.locale);
                match render_record_create_card(&scoped, record_name) {
                    Some(mut card) => {
                        if let Some((parent_record, parent_id)) = parent_context {
                            self.apply_manager_parent_create_context(
                                &scoped,
                                &mut card,
                                record_name,
                                &parent_record,
                                &parent_id,
                            );
                        }
                        json_response(200, card)
                    }
                    None => {
                        error_response(404, "SORX_MANAGER_RECORD_NOT_FOUND", "record not found")
                    }
                }
            }
            Err(err) => sorx_error_response(400, err),
        }
    }

    fn manager_record_detail_card_response(
        &self,
        request: &HttpRequest,
        record_name: &str,
        id: &str,
    ) -> HttpResponse {
        match self.manager_view(request) {
            Ok(mut view) => {
                if !view
                    .records
                    .iter()
                    .any(|record| record.record == record_name)
                {
                    return error_response(
                        404,
                        "SORX_MANAGER_RECORD_NOT_FOUND",
                        "record not found",
                    );
                }
                let Some(row) = self
                    .manager_record_rows(&view.tenant_id, record_name)
                    .into_iter()
                    .find(|row| row.id == id)
                else {
                    return error_response(
                        404,
                        "SORX_MANAGER_RECORD_NOT_FOUND",
                        "record not found",
                    );
                };
                if let Some(record) = view
                    .records
                    .iter_mut()
                    .find(|record| record.record == record_name)
                {
                    for field in &mut record.fields {
                        field.value = row.data.get(&field.name).cloned();
                    }
                }
                let related_sections =
                    self.manager_direct_child_record_sections(&view, record_name, id);
                match render_runtime_record_detail_card(
                    &view,
                    record_name,
                    id,
                    manager_record_action(&view, record_name, "delete"),
                    related_sections,
                ) {
                    Some(card) => json_response(200, card),
                    None => {
                        error_response(404, "SORX_MANAGER_RECORD_NOT_FOUND", "record not found")
                    }
                }
            }
            Err(err) => sorx_error_response(400, err),
        }
    }

    fn manager_record_delete_card_response(
        &self,
        request: &HttpRequest,
        record_name: &str,
        id: &str,
    ) -> HttpResponse {
        match self.manager_view(request) {
            Ok(view) => {
                let Some(record) = view
                    .records
                    .iter()
                    .find(|record| record.record == record_name)
                else {
                    return error_response(
                        404,
                        "SORX_MANAGER_RECORD_NOT_FOUND",
                        "record not found",
                    );
                };
                let Some(action) = manager_record_action(&view, record_name, "delete") else {
                    return error_response(
                        403,
                        "SORX_MANAGER_ACTION_FORBIDDEN",
                        "delete action is not available for this record",
                    );
                };
                let business_id = self
                    .manager_record_rows(&view.tenant_id, record_name)
                    .into_iter()
                    .find(|row| row.id == id)
                    .and_then(|row| manager_business_record_id(record_name, &row.data))
                    .unwrap_or_else(|| id.to_string());
                let card =
                    render_runtime_record_delete_card(&view, record, id, &business_id, action);
                json_response(200, card)
            }
            Err(err) => sorx_error_response(400, err),
        }
    }

    fn manager_graph_json_response(&self, request: &HttpRequest) -> HttpResponse {
        match self.manager_view(request) {
            Ok(view) => {
                let nodes = view
                    .records
                    .iter()
                    .map(|record| {
                        json!({
                            "id": record.record,
                            "label": record.label,
                            "collection": record.collection
                        })
                    })
                    .collect::<Vec<_>>();
                let edges = view
                    .relationships
                    .iter()
                    .map(|relationship| {
                        json!({
                            "id": relationship.id,
                            "from": relationship.from_record,
                            "to": relationship.to_record,
                            "label": relationship.label
                        })
                    })
                    .collect::<Vec<_>>();
                json_response(
                    200,
                    json!({
                        "schema": "greentic.sorx.manager-graph.v1",
                        "nodes": nodes,
                        "edges": edges
                    }),
                )
            }
            Err(err) => sorx_error_response(400, err),
        }
    }

    fn manager_graph_svg_response(&self, request: &HttpRequest) -> HttpResponse {
        match self.manager_view(request) {
            Ok(view) => {
                let height = 40 + (view.records.len().max(1) * 34);
                let mut body = String::new();
                for (index, record) in view.records.iter().enumerate() {
                    let y = 30 + index * 34;
                    body.push_str(&format!(
                        r#"<text x="16" y="{y}" font-family="sans-serif" font-size="14">{}</text>"#,
                        escape_xml(&record.label)
                    ));
                }
                json_response(
                    200,
                    json!({
                        "schema": "greentic.sorx.manager-graph-svg.v1",
                        "svg": format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="480" height="{height}">{body}</svg>"#)
                    }),
                )
            }
            Err(err) => sorx_error_response(400, err),
        }
    }

    fn manager_picker_response(&self, request: &HttpRequest, record: &str) -> HttpResponse {
        match self.manager_view(request) {
            Ok(view)
                if view
                    .records
                    .iter()
                    .any(|candidate| candidate.record == record) =>
            {
                let choices = self.manager_picker_choices(&view.tenant_id, record);
                json_response(
                    200,
                    json!({
                        "schema": "greentic.sorx.manager-picker.v1",
                        "tenant_id": view.tenant_id,
                        "record": record,
                        "choices": choices
                    }),
                )
            }
            Ok(_) => error_response(404, "SORX_MANAGER_RECORD_NOT_FOUND", "record not found"),
            Err(err) => sorx_error_response(400, err),
        }
    }

    fn manager_picker_choices(&self, tenant_id: &str, record: &str) -> Vec<Value> {
        let mut rows = self.manager_record_rows(tenant_id, record);
        sort_manager_record_rows(&mut rows);
        rows.into_iter()
            .take(25)
            .map(|record| {
                let label = picker_choice_label(&record.data, &record.id);
                let value = manager_business_record_id(record.entity.as_str(), &record.data)
                    .unwrap_or_else(|| record.id.clone());
                json!({
                    "title": label,
                    "value": value,
                })
            })
            .collect()
    }

    fn populate_manager_relationship_choices(
        &self,
        view: &mut greentic_sorx_core::ManagerViewModel,
        tenant_id: &str,
    ) {
        for record in &mut view.records {
            for field in &mut record.fields {
                let Some(relationship) = field.relationship.as_ref() else {
                    continue;
                };
                if !is_uuid_field(field.json_type.as_deref()) {
                    continue;
                }
                let choices = self.manager_picker_choices(tenant_id, &relationship.to_record);
                if choices.is_empty() {
                    continue;
                }
                let mut rules = field
                    .rules
                    .take()
                    .and_then(|value| value.as_object().cloned())
                    .unwrap_or_default();
                rules.insert("choices".to_string(), Value::Array(choices));
                field.rules = Some(Value::Object(rules));
            }
        }
    }

    fn manager_record_rows(&self, tenant_id: &str, record: &str) -> Vec<EntityRecord> {
        let Some(binding) = self.manager_record_binding(record) else {
            return Vec::new();
        };
        let Ok(provider) = self.runtime.providers.store(&binding.provider_id) else {
            return Vec::new();
        };
        let namespace = ProviderNamespace {
            tenant_id: tenant_id.to_string(),
            sor_name: self.runtime.config.deployment.sor_name.clone(),
        };
        let Ok(result) = provider.query(QueryOp {
            namespace,
            entity: binding.entity,
            collection: binding.collection,
            filter: Value::Object(Map::new()),
            order_by: Vec::new(),
        }) else {
            return Vec::new();
        };
        result.records
    }

    fn manager_record_binding(&self, record: &str) -> Option<ProviderBinding> {
        if let Some(endpoint) = self
            .runtime
            .router
            .endpoints
            .values()
            .find(|endpoint| endpoint.entity.as_deref() == Some(record))
            && let Ok(binding) = self.runtime.config.bindings.resolve(endpoint)
        {
            return Some(binding);
        }
        self.manager_model_records
            .get(record)
            .map(|model_record| ProviderBinding {
                entity: model_record.record.clone(),
                provider_id: self
                    .runtime
                    .config
                    .bindings
                    .default_provider_id()
                    .to_string(),
                collection: model_record.collection.clone(),
            })
    }

    fn filter_manager_rows_by_parent_context(
        &self,
        tenant_id: &str,
        rows: &mut Vec<EntityRecord>,
        target_record: &str,
        parent_record: &str,
        parent_id: &str,
    ) {
        if target_record == parent_record {
            rows.retain(|row| row.id == parent_id);
            return;
        }
        let Some(path) =
            manager_hierarchy_path(&self.manager_hierarchy, parent_record, target_record)
        else {
            return;
        };
        let mut ids = self.manager_record_context_ids(tenant_id, parent_record, parent_id);
        for step in path {
            let step_rows = self.manager_record_rows(tenant_id, &step.child);
            let next_ids = step_rows
                .iter()
                .filter(|row| {
                    row.data
                        .get(&step.field)
                        .is_some_and(|value| manager_record_value_matches_ids(value, &ids))
                })
                .map(|row| row.id.clone())
                .collect::<BTreeSet<_>>();
            ids = next_ids;
            if ids.is_empty() {
                break;
            }
        }
        rows.retain(|row| ids.contains(&row.id));
    }

    fn manager_record_context_ids(
        &self,
        tenant_id: &str,
        record_name: &str,
        id: &str,
    ) -> BTreeSet<String> {
        let mut ids = BTreeSet::from([id.to_string()]);
        if let Some(row) = self
            .manager_record_rows(tenant_id, record_name)
            .into_iter()
            .find(|row| row.id == id)
        {
            collect_manager_row_identifier_values(record_name, &row.data, &mut ids);
        }
        ids
    }

    fn apply_manager_parent_create_context(
        &self,
        view: &greentic_sorx_core::ManagerViewModel,
        card: &mut Value,
        child_record: &str,
        parent_record: &str,
        parent_id: &str,
    ) {
        let Some(path) =
            manager_hierarchy_path(&self.manager_hierarchy, parent_record, child_record)
        else {
            return;
        };
        let Some(step) = path.first() else {
            return;
        };
        let parent_value =
            self.manager_record_parent_context_value(&view.tenant_id, parent_record, parent_id);
        set_manager_card_submit_parent_context(
            card,
            child_record,
            parent_record,
            parent_id,
            &step.field,
            &parent_value,
        );
        remove_manager_card_input(card, &step.field);
    }

    fn manager_record_parent_context_value(
        &self,
        tenant_id: &str,
        parent_record: &str,
        parent_id: &str,
    ) -> String {
        if let Some(row) = self
            .manager_record_rows(tenant_id, parent_record)
            .into_iter()
            .find(|row| row.id == parent_id)
            && let Some(value) = manager_business_record_id(parent_record, &row.data)
        {
            return value;
        }
        self.manager_record_context_ids(tenant_id, parent_record, parent_id)
            .into_iter()
            .find(|value| value != parent_id)
            .unwrap_or_else(|| parent_id.to_string())
    }

    fn manager_direct_child_record_sections(
        &self,
        view: &greentic_sorx_core::ManagerViewModel,
        parent_record: &str,
        parent_id: &str,
    ) -> Vec<RuntimeRelatedRecordSection> {
        manager_child_record_links(view, &self.manager_hierarchy, parent_record)
            .into_iter()
            .filter_map(|(child_record, _)| {
                let record = view
                    .records
                    .iter()
                    .find(|record| record.record == child_record)?
                    .clone();
                let mut rows = self.manager_record_rows(&view.tenant_id, &child_record);
                self.filter_manager_rows_by_parent_context(
                    &view.tenant_id,
                    &mut rows,
                    &child_record,
                    parent_record,
                    parent_id,
                );
                sort_manager_record_rows(&mut rows);
                let page_size = 10usize;
                let total = rows.len();
                let end = page_size.min(total);
                let rows = rows[..end].to_vec();
                Some(RuntimeRelatedRecordSection {
                    record,
                    rows,
                    state: RuntimeRecordListState {
                        page: 1,
                        page_size,
                        total,
                        start: 0,
                        end,
                        search: String::new(),
                        can_create: manager_view_has_record_action(view, &child_record, "create"),
                        can_update: manager_view_has_record_action(view, &child_record, "update"),
                        can_delete: manager_view_has_record_action(view, &child_record, "delete"),
                        parent_context: Some((parent_record.to_string(), parent_id.to_string())),
                    },
                })
            })
            .collect()
    }

    fn manager_submit_response(&self, request: &HttpRequest) -> HttpResponse {
        let mut context = match self.manager_context(request) {
            Ok(context) => context,
            Err(err) => return sorx_error_response(400, err),
        };
        let body = match serde_json::from_str::<Value>(&request.body) {
            Ok(Value::Object(body)) => body,
            Ok(_) => {
                return error_response(400, "SORX_INVALID_JSON", "request body must be an object");
            }
            Err(err) => return error_response(400, "SORX_INVALID_JSON", &err.to_string()),
        };
        if let Some(roles) = manager_submit_roles(&body) {
            context.roles = roles;
        }
        let endpoint_id = match body.get("endpoint_id").and_then(Value::as_str) {
            Some(value) => value.to_string(),
            None => {
                return error_response(
                    400,
                    "SORX_MANAGER_SUBMIT_INVALID",
                    "manager submit requires endpoint_id",
                );
            }
        };
        let operation_id = body
            .get("operation_id")
            .and_then(Value::as_str)
            .unwrap_or(endpoint_id.as_str())
            .to_string();
        if let Some((record_name, operation)) = self.manager_model_action_record(&endpoint_id) {
            return self.manager_model_action_submit_response(
                &context,
                &body,
                &record_name,
                &endpoint_id,
                &operation,
            );
        }
        let mut input = body
            .get("input")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        if let Some(endpoint) = self.runtime.router.endpoints.get(&endpoint_id)
            && let Some(record) = endpoint.entity.as_deref()
            && let Ok(view) = self.manager_view(request)
        {
            merge_manager_submit_fields(&mut input, &body, &view, record);
            fill_generated_manager_fields(&mut input, &view, record, &endpoint_id);
            combine_manager_datetime_inputs(&mut input, &view, record);
            stamp_manager_created_at(&mut input, endpoint);
        }
        trim_manager_submit_string_values(&mut input);
        let idempotency_key = body
            .get("idempotency_key")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let invocation = EndpointInvocation {
            tenant_id: context.tenant_id.clone(),
            endpoint_id,
            operation_id,
            input,
            caller: context.caller(),
            idempotency_key,
            source: InvocationSource::Http,
        };
        match self.runtime.invoke(invocation) {
            Ok(result) if result.status == EndpointStatus::ApprovalRequired => json_response(
                202,
                json!({
                    "ok": false,
                    "status": "approval_required",
                    "approval": result.output["approval"]
                }),
            ),
            Ok(result) if result.status == EndpointStatus::Denied => json_response(
                403,
                json!({
                    "ok": false,
                    "error": {
                        "code": "SORX_POLICY_DENIED",
                        "message": result.output["reason"].as_str().unwrap_or("operation denied"),
                        "details": result.output
                    }
                }),
            ),
            Ok(result) => json_response(
                200,
                json!({
                    "ok": true,
                    "schema": "greentic.sorx.manager-submit-result.v1",
                    "status": format!("{:?}", result.status).to_ascii_lowercase(),
                    "result": result.output,
                    "events": result.events
                }),
            ),
            Err(err) => sorx_error_response(400, err),
        }
    }

    fn manager_model_action_record(&self, endpoint_id: &str) -> Option<(String, String)> {
        for record in self.manager_model_records.keys() {
            for operation in ["create", "update", "delete"] {
                if endpoint_id == format!("{}.model_{}", manager_key_like(record), operation) {
                    return Some((record.clone(), operation.to_string()));
                }
            }
        }
        None
    }

    fn manager_model_action_submit_response(
        &self,
        context: &greentic_sorx_core::SorxManagerContext,
        body: &Map<String, Value>,
        record_name: &str,
        endpoint_id: &str,
        operation: &str,
    ) -> HttpResponse {
        let Some(model_record) = self.manager_model_records.get(record_name) else {
            return error_response(404, "SORX_MANAGER_RECORD_NOT_FOUND", "record not found");
        };
        if !manager_model_record_can_perform(model_record, &context.roles, operation) {
            return error_response(
                403,
                "SORX_MANAGER_ACTION_FORBIDDEN",
                &format!("{operation} action is not available for this record"),
            );
        }
        let mut input = body
            .get("input")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let Some(object) = input.as_object_mut() else {
            return error_response(
                400,
                "SORX_MANAGER_SUBMIT_INVALID",
                "manager submit input must be an object",
            );
        };
        for field in &model_record.fields {
            if !object.contains_key(&field.name)
                && let Some(value) = body.get(&field.name)
            {
                object.insert(field.name.clone(), value.clone());
            }
            if field.generated
                && object.get(&field.name).is_none()
                && is_uuid_field(field.json_type.as_deref())
            {
                object.insert(
                    field.name.clone(),
                    Value::String(generated_manager_uuid(
                        endpoint_id,
                        record_name,
                        &field.name,
                    )),
                );
            }
        }
        trim_manager_submit_string_values(&mut input);
        let Some(binding) = self.manager_record_binding(record_name) else {
            return error_response(
                404,
                "SORX_MANAGER_RECORD_NOT_FOUND",
                "record provider binding not found",
            );
        };
        let Ok(provider) = self.runtime.providers.store(&binding.provider_id) else {
            return error_response(
                404,
                "SORX_MANAGER_RECORD_NOT_FOUND",
                "record provider not found",
            );
        };
        let namespace = ProviderNamespace {
            tenant_id: context.tenant_id.clone(),
            sor_name: self.runtime.config.deployment.sor_name.clone(),
        };
        match operation {
            "create" => match provider.create(greentic_sorx_core::CreateOp {
                namespace,
                entity: binding.entity,
                collection: binding.collection,
                input,
                idempotency_key: body
                    .get("idempotency_key")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                unique_indexes: Vec::new(),
                unique_behavior: greentic_sorx_core::UniqueConflictBehavior::Reject,
            }) {
                Ok(record) => manager_model_submit_record_response(record),
                Err(err) => sorx_error_response(400, err),
            },
            "update" => {
                let Some(id) = body.get("id").and_then(Value::as_str) else {
                    return error_response(
                        400,
                        "SORX_MANAGER_SUBMIT_INVALID",
                        "manager update requires id",
                    );
                };
                match provider.update(greentic_sorx_core::UpdateOp {
                    namespace,
                    entity: binding.entity,
                    collection: binding.collection,
                    id: id.to_string(),
                    patch: input,
                    unique_indexes: Vec::new(),
                }) {
                    Ok(record) => manager_model_submit_record_response(record),
                    Err(err) => sorx_error_response(400, err),
                }
            }
            "delete" => {
                let Some(id) = body
                    .get("input")
                    .and_then(|input| input.get("id"))
                    .or_else(|| body.get("id"))
                    .and_then(Value::as_str)
                else {
                    return error_response(
                        400,
                        "SORX_MANAGER_SUBMIT_INVALID",
                        "manager delete requires id",
                    );
                };
                match provider.delete(greentic_sorx_core::DeleteOp {
                    namespace,
                    entity: binding.entity,
                    collection: binding.collection,
                    id: id.to_string(),
                }) {
                    Ok(result) => json_response(
                        200,
                        json!({
                            "ok": true,
                            "schema": "greentic.sorx.manager-submit-result.v1",
                            "status": "completed",
                            "result": { "deleted": result.deleted },
                            "events": []
                        }),
                    ),
                    Err(err) => sorx_error_response(400, err),
                }
            }
            _ => error_response(
                400,
                "SORX_MANAGER_SUBMIT_INVALID",
                "unsupported manager model action",
            ),
        }
    }

    fn manager_view(
        &self,
        request: &HttpRequest,
    ) -> SorxResult<greentic_sorx_core::ManagerViewModel> {
        let context = self.manager_context(request)?;
        let mut view = generate_manager_view(&context, &self.runtime.router, &self.runtime.policy);
        view.title = (*self.manager_title).clone();
        view.description = (*self.manager_description).clone();
        apply_manager_model_records(
            &mut view,
            &self.manager_model_records,
            &self.manager_hierarchy,
            &context.roles,
        );
        apply_manager_record_fields(&mut view, &self.manager_record_fields);
        apply_manager_ontology_metadata(&mut view, &self.manager_ontology);
        apply_manager_record_hierarchy(&mut view, &self.manager_hierarchy);
        let policies = manager_authorization_policy_set(&self.runtime, &context.roles);
        let mut view = filter_manager_view(view, &policies);
        let locale = ManagerLocaleContext::new(context.locale.clone(), "en");
        view = localize_manager_view(view, &locale, &builtin_manager_locale_bundle());
        apply_builtin_manager_text_translations(&mut view, &locale.locale);
        Ok(view)
    }

    fn manager_context(
        &self,
        request: &HttpRequest,
    ) -> SorxResult<greentic_sorx_core::SorxManagerContext> {
        resolve_manager_context(
            &request.headers,
            &ManagerContextDefaults::from_runtime_config(&self.runtime.config),
        )
    }

    fn list_business_actions(&self) -> HttpResponse {
        let Some(assets) = self.business_actions.as_ref() else {
            return json_response(
                200,
                json!({
                    "schema": "greentic.sorx.business-actions.v1",
                    "actions": []
                }),
            );
        };
        let mut by_id: BTreeMap<String, Vec<&BusinessAction>> = BTreeMap::new();
        for action in &assets.catalog.actions {
            by_id.entry(action.id.clone()).or_default().push(action);
        }
        let actions = by_id
            .into_iter()
            .map(|(id, versions)| {
                json!({
                    "id": id,
                    "versions": versions.iter().map(|action| action.version.clone()).collect::<Vec<_>>(),
                    "label": versions.first().and_then(|action| action.label.clone()),
                    "aliases": versions.first().map(|action| action.aliases.clone()).unwrap_or_default(),
                    "risk": versions.first().and_then(|action| action.risk.as_ref()).map(|risk| format!("{risk:?}").to_ascii_lowercase()),
                    "approval_required": versions.first().and_then(|action| action.approval.as_ref()).is_some_and(|approval| approval.required),
                    "idempotency_required": versions.first().and_then(|action| action.idempotency.as_ref()).is_some_and(|idempotency| idempotency.required),
                    "designer": versions.first().and_then(|action| action.designer.clone())
                })
            })
            .collect::<Vec<_>>();
        json_response(
            200,
            json!({
                "schema": "greentic.sorx.business-actions.v1",
                "actions": actions
            }),
        )
    }

    fn list_metrics(&self, request: &HttpRequest) -> HttpResponse {
        self.audit_metric_surface(request, "sorx.metric.listed", "list", Some("allow"));
        let metrics = self.metrics.as_ref().as_ref();
        let metric_values = metrics
            .map(|catalog| {
                catalog
                    .metrics
                    .iter()
                    .map(metric_summary_json)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        json_response(
            200,
            json!({
                "schema": "greentic.sorx.metrics.v1",
                "metrics": metric_values
            }),
        )
    }

    fn handle_metric_request(&self, request: &HttpRequest) -> HttpResponse {
        let suffix = request
            .path
            .trim_start_matches("/v1/sorx/metrics/")
            .trim_matches('/');
        let parts = suffix.split('/').collect::<Vec<_>>();
        match (request.method.as_str(), parts.as_slice()) {
            ("GET", [metric_name]) => self.get_metric(metric_name, request),
            ("POST", [metric_name, "query"]) => self.query_metric(metric_name, request),
            _ => error_response(404, "SORX_METRIC_ROUTE_NOT_FOUND", "metric route not found"),
        }
    }

    fn get_metric(&self, metric_name: &str, request: &HttpRequest) -> HttpResponse {
        let Some(metrics) = self.metrics.as_ref().as_ref() else {
            return error_response(404, "SORX_METRIC_NOT_FOUND", "metric not found");
        };
        match metrics.metric(metric_name) {
            Ok(metric) => {
                self.audit_metric_surface(
                    request,
                    "sorx.metric.definition_read",
                    metric_name,
                    Some("allow"),
                );
                json_response(
                    200,
                    json!({
                        "schema": "greentic.sorx.metric-definition.v1",
                        "metric": metric
                    }),
                )
            }
            Err(_) => error_response(404, "SORX_METRIC_NOT_FOUND", "metric not found"),
        }
    }

    fn audit_metric_surface(
        &self,
        request: &HttpRequest,
        event: &str,
        metric_name: &str,
        decision: Option<&str>,
    ) {
        let tenant_id = request
            .headers
            .get("x-greentic-tenant-id")
            .cloned()
            .unwrap_or_else(|| self.runtime.config.tenant_id.clone());
        let caller_id = request
            .headers
            .get("x-greentic-caller-id")
            .cloned()
            .unwrap_or_else(|| "local".to_string());
        let mut details = Map::new();
        details.insert("metric".to_string(), json!(metric_name));
        let _ = self.runtime.audit_metric(
            tenant_id,
            caller_id,
            event,
            metric_name,
            decision.map(ToString::to_string),
            details,
        );
    }

    fn query_metric(&self, metric_name: &str, request: &HttpRequest) -> HttpResponse {
        let Some(metrics) = self.metrics.as_ref().as_ref() else {
            return error_response(404, "SORX_METRIC_NOT_FOUND", "metric not found");
        };
        let body = match request_json(request, &BTreeMap::new(), None) {
            Ok(value) => value,
            Err(err) => return error_response(400, "SORX_METRIC_QUERY_INVALID", &err),
        };
        let tenant_id = match header_or_local(
            &request.headers,
            "x-greentic-tenant-id",
            &self.runtime.config.tenant_id,
            &self.runtime.config.environment,
        ) {
            Ok(value) => value,
            Err(err) => return sorx_error_response(400, err),
        };
        let caller_id = match header_or_local(
            &request.headers,
            "x-greentic-caller-id",
            "local",
            &self.runtime.config.environment,
        ) {
            Ok(value) => value,
            Err(err) => return sorx_error_response(400, err),
        };
        let mut query = metric_query_from_json(
            body,
            ProviderNamespace {
                tenant_id: tenant_id.clone(),
                sor_name: self.runtime.config.deployment.sor_name.clone(),
            },
        );
        if let Ok(metric) = metrics.metric(metric_name) {
            if query.dimensions.is_empty() {
                query.dimensions = default_metric_dimensions(metric);
            }
            if let Some(dimension) = sensitive_requested_dimension(metric, &query) {
                let mut details = Map::new();
                details.insert("metric".to_string(), json!(metric_name));
                details.insert("dimension".to_string(), json!(dimension));
                let _ = self.runtime.audit_metric(
                    tenant_id,
                    caller_id,
                    "sorx.metric.query.rejected",
                    metric_name,
                    Some("denied".to_string()),
                    details,
                );
                return error_response(
                    403,
                    "SORX_METRIC_POLICY_DENIED",
                    "metric query includes a sensitive dimension",
                );
            }
        }
        let runtime = MetricRuntime::new(metrics.clone(), self.metric_provider.as_ref());
        let mut cache_details = Map::new();
        cache_details.insert("metric".to_string(), json!(metric_name));
        let _ = self.runtime.audit_metric(
            tenant_id.clone(),
            caller_id.clone(),
            "sorx.metric.cache_miss",
            metric_name,
            Some("miss".to_string()),
            cache_details,
        );
        match runtime.query(metric_name, query) {
            Ok(result) => {
                let mut details = Map::new();
                details.insert("metric".to_string(), json!(metric_name));
                details.insert("row_count".to_string(), json!(result.rows.len()));
                let _ = self.runtime.audit_metric(
                    tenant_id,
                    caller_id,
                    "sorx.metric.queried",
                    metric_name,
                    Some("allow".to_string()),
                    details,
                );
                json_response(
                    200,
                    json!({
                        "schema": "greentic.sorx.metric-query-result.v1",
                        "result": result
                    }),
                )
            }
            Err(err) if err.code == "metric_missing" => {
                error_response(404, "SORX_METRIC_NOT_FOUND", &err.message)
            }
            Err(err) => sorx_error_response(400, err),
        }
    }

    fn manager_metric_query_result(
        &self,
        tenant_id: &str,
        metric_name: &str,
        dimensions: Vec<String>,
    ) -> SorxResult<MetricQueryResult> {
        let Some(metrics) = self.metrics.as_ref().as_ref() else {
            return Err(SorxError::new("metric_missing", "metric not found"));
        };
        let runtime = MetricRuntime::new(metrics.clone(), self.metric_provider.as_ref());
        runtime.query(
            metric_name,
            MetricQuery {
                namespace: ProviderNamespace {
                    tenant_id: tenant_id.to_string(),
                    sor_name: self.runtime.config.deployment.sor_name.clone(),
                },
                from: None,
                to: None,
                grain: None,
                dimensions,
                filters: Vec::new(),
            },
        )
    }

    fn handle_business_action_request(&self, request: &HttpRequest) -> HttpResponse {
        let suffix = request
            .path
            .trim_start_matches("/v1/sorx/business-actions/")
            .trim_matches('/');
        let parts = suffix.split('/').collect::<Vec<_>>();
        match (request.method.as_str(), parts.as_slice()) {
            ("GET", [id]) => self.get_business_action_versions(id),
            ("GET", [id, "versions", version]) => self.get_business_action_version(id, version),
            ("GET", [id, "versions", version, "schema"]) => {
                self.get_business_action_schema(id, version)
            }
            ("POST", [id, "versions", version, "dry-run"]) => {
                self.run_business_action(id, version, request, true)
            }
            ("POST", [id, "versions", version, "invoke"]) => {
                self.run_business_action(id, version, request, false)
            }
            _ => error_response(404, "unknown_action", "business action route not found"),
        }
    }

    fn get_business_action_versions(&self, id: &str) -> HttpResponse {
        let Some(assets) = self.business_actions.as_ref() else {
            return business_action_error(404, "unknown_action", "business action not found");
        };
        let actions = assets
            .catalog
            .actions
            .iter()
            .filter(|action| action.id == id)
            .map(business_action_json)
            .collect::<Vec<_>>();
        if actions.is_empty() {
            return business_action_error(404, "unknown_action", "business action not found");
        }
        json_response(
            200,
            json!({
                "schema": "greentic.sorx.business-action-versions.v1",
                "id": id,
                "actions": actions
            }),
        )
    }

    fn get_business_action_version(&self, id: &str, version: &str) -> HttpResponse {
        let Some(action) = self.find_business_action(id, version) else {
            return business_action_error(
                404,
                missing_action_code(self.business_actions.as_ref().as_ref(), id),
                "business action not found",
            );
        };
        json_response(200, business_action_json(action))
    }

    fn get_business_action_schema(&self, id: &str, version: &str) -> HttpResponse {
        let Some(action) = self.find_business_action(id, version) else {
            return business_action_error(
                404,
                missing_action_code(self.business_actions.as_ref().as_ref(), id),
                "business action not found",
            );
        };
        json_response(
            200,
            json!({
                "schema": "greentic.sorx.business-action-schema.v1",
                "id": action.id,
                "version": action.version,
                "input_schema": action.input_schema,
                "output_schema": action.output_schema
            }),
        )
    }

    fn run_business_action(
        &self,
        id: &str,
        version: &str,
        request: &HttpRequest,
        dry_run: bool,
    ) -> HttpResponse {
        let Some(action) = self.find_business_action(id, version) else {
            return business_action_error(
                404,
                missing_action_code(self.business_actions.as_ref().as_ref(), id),
                "business action not found",
            );
        };
        let body = match request_json(request, &BTreeMap::new(), None) {
            Ok(value) => value,
            Err(err) => return business_action_error(400, "invalid_payload", &err),
        };
        if let Some(action_ref) = body.get("action_ref").and_then(Value::as_object)
            && (action_ref
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|body_id| body_id != id)
                || action_ref
                    .get("version")
                    .and_then(Value::as_str)
                    .is_some_and(|body_version| body_version != version))
        {
            return business_action_error(
                409,
                "version_mismatch",
                "request action_ref conflicts with URL action reference",
            );
        }
        let Some(expected_hash) = body
            .get("action_ref")
            .and_then(|value| value.get("contract_hash"))
            .and_then(Value::as_str)
        else {
            return business_action_error(400, "missing_contract_hash", "missing contract hash");
        };
        if !valid_contract_hash_format(expected_hash) {
            return business_action_error(
                400,
                "invalid_contract_hash",
                "contract hash must use sha256:<64 lowercase hex characters>",
            );
        }
        if !self.contract_hash_matches(action, expected_hash) {
            return business_action_error(
                409,
                "contract_hash_mismatch",
                "contract hash does not match business action lock",
            );
        }
        let values = body.get("values").cloned().unwrap_or_else(|| json!({}));
        if let Some(schema) = &action.input_schema
            && let Err(err) = validate_action_schema(schema, &values)
        {
            return business_action_error(400, "invalid_payload", &err);
        }
        let Some(endpoint) = self.execution_endpoint(action) else {
            return business_action_error(
                404,
                "execution_target_missing",
                "business action execution target is missing",
            );
        };
        let idempotency_key = body
            .get("options")
            .and_then(|options| options.get("idempotency_key"))
            .and_then(Value::as_str)
            .map(ToString::to_string);
        if action
            .idempotency
            .as_ref()
            .is_some_and(|idempotency| idempotency.required)
            && idempotency_key.is_none()
        {
            return business_action_error(
                400,
                "missing_idempotency_key",
                "idempotency key is required",
            );
        }
        if self
            .runtime
            .config
            .bindings
            .resolve(endpoint)
            .and_then(|binding| {
                self.runtime
                    .providers
                    .store(&binding.provider_id)
                    .map(|_| ())
            })
            .is_err()
        {
            return business_action_error(
                503,
                "provider_unavailable",
                "business action provider is unavailable",
            );
        }
        let policy_decision = self.runtime.policy.decide(endpoint);
        let policy_decision_label = match policy_decision.action {
            PolicyAction::Execute => "allow",
            PolicyAction::Deny => "deny",
            PolicyAction::RequireApproval => "require_approval",
        };
        if dry_run {
            return json_response(
                200,
                json!({
                    "valid": true,
                    "canonical_payload": values,
                    "policy_decision": policy_decision_label,
                    "approval_required": matches!(policy_decision.action, PolicyAction::RequireApproval),
                    "execution_target": execution_target_json(action, endpoint),
                    "explain": {}
                }),
            );
        }

        let tenant_id = match header_or_local(
            &request.headers,
            "x-greentic-tenant-id",
            &self.runtime.config.tenant_id,
            &self.runtime.config.environment,
        ) {
            Ok(value) => value,
            Err(err) => return sorx_error_response(400, err),
        };
        let caller_id = match header_or_local(
            &request.headers,
            "x-greentic-caller-id",
            "local",
            &self.runtime.config.environment,
        ) {
            Ok(value) => value,
            Err(err) => return sorx_error_response(400, err),
        };
        let invocation = EndpointInvocation {
            tenant_id,
            endpoint_id: endpoint.endpoint_id.clone(),
            operation_id: endpoint.operation_id.clone(),
            input: values,
            caller: CallerContext {
                subject: caller_id,
                roles: request_roles(&request.headers),
            },
            idempotency_key,
            source: InvocationSource::Http,
        };
        match self.runtime.invoke(invocation) {
            Ok(result) if result.status == EndpointStatus::ApprovalRequired => json_response(
                202,
                json!({
                    "ok": false,
                    "status": "approval_required",
                    "approval": result.output["approval"],
                    "action_ref": action_ref_json(action, expected_hash),
                    "explain": {}
                }),
            ),
            Ok(result) if result.status == EndpointStatus::Denied => {
                let message = result.output["reason"]
                    .as_str()
                    .unwrap_or("business action denied")
                    .to_string();
                business_action_error_with_details(403, "policy_denied", &message, result.output)
            }
            Ok(result) => json_response(
                200,
                json!({
                    "ok": true,
                    "action_ref": action_ref_json(action, expected_hash),
                    "result": result.output,
                    "audit": {
                        "action_id": action.id,
                        "action_version": action.version,
                        "expected_contract_hash": expected_hash,
                        "validation_result": "passed",
                        "result_status": format!("{:?}", result.status).to_ascii_lowercase(),
                        "events": result.events
                    },
                    "explain": {}
                }),
            ),
            Err(err) => sorx_error_response(400, err),
        }
    }

    fn find_business_action(&self, id: &str, version: &str) -> Option<&BusinessAction> {
        self.business_actions
            .as_ref()
            .as_ref()?
            .catalog
            .actions
            .iter()
            .find(|action| action.id == id && action.version == version)
    }

    fn find_business_action_by_capability(&self, capability: &str) -> Option<&BusinessAction> {
        self.business_actions
            .as_ref()
            .as_ref()?
            .catalog
            .actions
            .iter()
            .find(|action| {
                business_action_capability(&self.runtime.pack.name, action) == capability
            })
    }

    fn locked_contract_hash(&self, action: &BusinessAction) -> Option<String> {
        self.business_actions
            .as_ref()
            .as_ref()
            .and_then(|assets| assets.lock.as_ref())
            .and_then(|lock| {
                lock.entries
                    .iter()
                    .find(|entry| entry.id == action.id && entry.version == action.version)
                    .map(|entry| entry.contract_hash.clone())
            })
    }

    fn contract_hash_matches(&self, action: &BusinessAction, expected_hash: &str) -> bool {
        self.business_actions
            .as_ref()
            .as_ref()
            .and_then(|assets| assets.lock.as_ref())
            .is_some_and(|lock| {
                lock.entries.iter().any(|entry| {
                    entry.id == action.id
                        && entry.version == action.version
                        && entry.contract_hash == expected_hash
                })
            })
    }

    fn execution_endpoint(&self, action: &BusinessAction) -> Option<&EndpointDefinition> {
        if let Some(endpoint_id) = &action.execution.endpoint_id {
            return self.runtime.router.endpoints.get(endpoint_id);
        }
        if let Some(operation_id) = &action.execution.operation_id {
            return self
                .runtime
                .router
                .endpoints
                .values()
                .find(|endpoint| endpoint.operation_id == *operation_id);
        }
        if let Some(tool_name) = &action.execution.tool_name
            && let Some(tool) = self.tools.tools.iter().find(|tool| tool.name == *tool_name)
        {
            return self.runtime.router.endpoints.get(&tool.endpoint_id);
        }
        None
    }

    fn match_endpoint(
        &self,
        method: &str,
        path: &str,
    ) -> Option<(EndpointDefinition, BTreeMap<String, String>)> {
        self.runtime.router.endpoints.values().find_map(|endpoint| {
            if method_from_endpoint(endpoint.method) != method {
                return None;
            }
            match_path(&endpoint.path, path).map(|params| (endpoint.clone(), params))
        })
    }
}

fn http_auth_from_config(config: &SorxRuntimeConfig) -> SorxResult<HttpAuth> {
    match config.server.auth.mode.as_str() {
        "none" => Ok(HttpAuth {
            shared_secret: None,
        }),
        "shared_secret" => {
            let secret_ref = config
                .server
                .auth
                .shared_secret_ref
                .as_deref()
                .ok_or_else(|| {
                    SorxError::at_path(
                        "invalid_startup_config",
                        "server auth mode shared_secret requires shared_secret_ref",
                        "server.auth.shared_secret_ref",
                    )
                })?;
            let secret = resolve_shared_secret(secret_ref)?;
            Ok(HttpAuth {
                shared_secret: Some(secret),
            })
        }
        other => Err(SorxError::at_path(
            "invalid_startup_config",
            format!("unsupported server auth mode {other:?}"),
            "server.auth.mode",
        )),
    }
}

fn resolve_shared_secret(secret_ref: &str) -> SorxResult<String> {
    let Some(env_name) = secret_ref.strip_prefix("env:") else {
        return Err(SorxError::at_path(
            "unsupported_secret_ref",
            "HTTP ingest shared secret refs currently support env:<NAME>",
            "server.auth.shared_secret_ref",
        ));
    };
    std::env::var(env_name).map_err(|_| {
        SorxError::at_path(
            "secret_ref_unresolved",
            format!("environment variable {env_name} is not set"),
            "server.auth.shared_secret_ref",
        )
    })
}

fn request_bearer_token(headers: &BTreeMap<String, String>) -> Option<&str> {
    let value = headers.get("authorization")?;
    let (scheme, token) = value.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty() {
        Some(token.trim())
    } else {
        None
    }
}

fn configure_runtime_audit(runtime: SorxRuntime, config: &SorxRuntimeConfig) -> SorxRuntime {
    match config.audit.sink.as_str() {
        "stdout" => runtime.with_audit_sink(Arc::new(StdoutAuditSink)),
        _ => runtime,
    }
}

fn configure_runtime_events(
    runtime: SorxRuntime,
    config: &SorxRuntimeConfig,
) -> SorxResult<SorxRuntime> {
    match config.events.sink.as_str() {
        "stdout" => Ok(runtime.with_event_sink(Arc::new(StdoutBusinessEventSink))),
        "nats" => {
            let url = config.events.nats_url.clone().ok_or_else(|| {
                SorxError::new(
                    "events_nats_url_missing",
                    "events.sink = \"nats\" requires events.nats_url",
                )
            })?;
            nats_event_sink(&url, &config.events.subject_prefix)
                .map(|sink| runtime.with_event_sink(sink))
        }
        // "disabled" and "" are the explicit no-op values; anything else is
        // treated the same way (schema already constrains the allowed values).
        "disabled" | "" => Ok(runtime),
        _ => Ok(runtime),
    }
}

#[cfg(feature = "events-nats")]
fn nats_event_sink(url: &str, subject_prefix: &str) -> SorxResult<Arc<dyn BusinessEventSink>> {
    crate::nats_events::NatsEventSink::connect(url, subject_prefix)
        .map(|sink| Arc::new(sink) as Arc<dyn BusinessEventSink>)
}

#[cfg(not(feature = "events-nats"))]
fn nats_event_sink(_url: &str, _subject_prefix: &str) -> SorxResult<Arc<dyn BusinessEventSink>> {
    Err(SorxError::new(
        "events_nats_unavailable",
        "this build does not include the events-nats feature",
    ))
}

fn is_admin_api_path(path: &str) -> bool {
    let path = request_path_without_query(path);
    path == "/v1/sorx/deployments"
        || path == "/v1/sorx/aliases"
        || path == "/v1/sorx/routing-table"
        || path.starts_with("/v1/sorx/deployments/")
        || path.starts_with("/v1/sorx/aliases/")
}

/// Serve the registry-backed `/v1/sorx/*` deployment/alias control plane and
/// the resolved routing-table from a persisted [`LocalDeploymentRegistryStore`].
///
/// Every write loads the store, mutates it through the existing
/// [`greentic_sorx_core::DeploymentRegistry`] methods (no alias logic is
/// duplicated here), then persists it again. Lifecycle-creation endpoints
/// (`create`/`validate`/`retire`) are intentionally not served here because
/// they need the pack file; they return 501 and stay CLI-only.
fn handle_registry_request(
    store: &LocalDeploymentRegistryStore,
    method: &str,
    path: &str,
    body: &str,
) -> HttpResponse {
    let route = request_path_without_query(path);
    let tenant = request_query_param(path, "tenant");
    let sor = request_query_param(path, "sor");

    match (method, route) {
        ("GET", "/v1/sorx/deployments") => registry_list_deployments(store, tenant, sor),
        ("GET", "/v1/sorx/aliases") => registry_list_aliases(store, tenant, sor),
        ("GET", "/v1/sorx/routing-table") => registry_routing_table(store, tenant, sor),
        ("PUT", route) if route.starts_with("/v1/sorx/aliases/") => {
            registry_set_alias(store, route, body)
        }
        ("GET", route) if route.starts_with("/v1/sorx/deployments/") => {
            registry_get_deployment(store, route)
        }
        ("POST", route)
            if route.starts_with("/v1/sorx/deployments/") && route.ends_with("/promote") =>
        {
            registry_promote(store, route, body)
        }
        ("POST", route)
            if route.starts_with("/v1/sorx/deployments/") && route.ends_with("/rollback") =>
        {
            registry_rollback(store, route, body)
        }
        ("POST", "/v1/sorx/deployments") | ("POST", "/v1/sorx/validations") => error_response(
            501,
            "SORX_ADMIN_API_NOT_IMPLEMENTED",
            "deployment create/validate/retire are served by the CLI registry, not the HTTP API",
        ),
        _ => error_response(404, "SORX_ROUTE_NOT_FOUND", "route not found"),
    }
}

fn registry_error_response(err: DeploymentRegistryError) -> HttpResponse {
    let status = match err.code.as_str() {
        "deployment_missing" => 404,
        "alias_target_not_routable" | "alias_scope_mismatch" => 409,
        "promotion_blocked" => 409,
        _ => 400,
    };
    error_response(status, &registry_error_code(&err.code), &err.message)
}

fn registry_error_code(code: &str) -> String {
    format!("SORX_{}", code.to_ascii_uppercase())
}

fn registry_load(
    store: &LocalDeploymentRegistryStore,
) -> Result<greentic_sorx_core::DeploymentRegistry, HttpResponse> {
    store.load().map_err(registry_error_response)
}

fn registry_save(
    store: &LocalDeploymentRegistryStore,
    registry: &greentic_sorx_core::DeploymentRegistry,
) -> Result<(), HttpResponse> {
    store.save(registry).map_err(registry_error_response)
}

fn registry_list_deployments(
    store: &LocalDeploymentRegistryStore,
    tenant: Option<String>,
    sor: Option<String>,
) -> HttpResponse {
    let registry = match registry_load(store) {
        Ok(registry) => registry,
        Err(response) => return response,
    };
    let deployments = registry
        .deployments
        .iter()
        .filter(|deployment| tenant.as_deref().is_none_or(|t| deployment.tenant_id == t))
        .filter(|deployment| sor.as_deref().is_none_or(|s| deployment.sor_name == s))
        .cloned()
        .collect::<Vec<_>>();
    json_response(200, json!({ "deployments": deployments }))
}

fn registry_get_deployment(store: &LocalDeploymentRegistryStore, route: &str) -> HttpResponse {
    let Some(deployment_id) = route.strip_prefix("/v1/sorx/deployments/") else {
        return error_response(404, "SORX_ROUTE_NOT_FOUND", "route not found");
    };
    // Only a bare deployment id is a "get deployment"; sub-resources are routed
    // elsewhere (the local-runtime branch handles `/routes` and
    // `/promotion-status`, promote/rollback are matched as POST above).
    if deployment_id.is_empty() || deployment_id.contains('/') {
        return error_response(404, "SORX_ROUTE_NOT_FOUND", "route not found");
    }
    let registry = match registry_load(store) {
        Ok(registry) => registry,
        Err(response) => return response,
    };
    match registry.deployment(deployment_id) {
        Some(deployment) => json_response(200, serde_json::to_value(deployment).unwrap()),
        None => error_response(
            404,
            "SORX_DEPLOYMENT_MISSING",
            &format!("deployment `{deployment_id}` does not exist"),
        ),
    }
}

fn registry_list_aliases(
    store: &LocalDeploymentRegistryStore,
    tenant: Option<String>,
    sor: Option<String>,
) -> HttpResponse {
    let registry = match registry_load(store) {
        Ok(registry) => registry,
        Err(response) => return response,
    };
    let aliases = registry.aliases_for(tenant.as_deref(), sor.as_deref());
    json_response(200, json!({ "aliases": aliases }))
}

fn registry_set_alias(
    store: &LocalDeploymentRegistryStore,
    route: &str,
    body: &str,
) -> HttpResponse {
    let segments = route
        .strip_prefix("/v1/sorx/aliases/")
        .unwrap_or_default()
        .split('/')
        .collect::<Vec<_>>();
    let [tenant, sor, alias] = segments.as_slice() else {
        return error_response(
            404,
            "SORX_ROUTE_NOT_FOUND",
            "alias path must be /v1/sorx/aliases/{tenant}/{sor}/{alias}",
        );
    };
    let payload = match parse_json_object(body) {
        Ok(payload) => payload,
        Err(response) => return response,
    };
    let Some(target) = payload
        .get("target_deployment_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    else {
        return error_response(
            400,
            "SORX_INVALID_BODY",
            "body must include a non-empty target_deployment_id",
        );
    };

    let mut registry = match registry_load(store) {
        Ok(registry) => registry,
        Err(response) => return response,
    };
    let alias = match registry.set_alias(*tenant, *sor, *alias, target) {
        Ok(alias) => alias,
        Err(err) => return registry_error_response(err),
    };
    if let Err(response) = registry_save(store, &registry) {
        return response;
    }
    json_response(200, serde_json::to_value(alias).unwrap())
}

fn registry_promote(store: &LocalDeploymentRegistryStore, route: &str, body: &str) -> HttpResponse {
    let Some(deployment_id) = route
        .strip_prefix("/v1/sorx/deployments/")
        .and_then(|rest| rest.strip_suffix("/promote"))
        .filter(|id| !id.is_empty() && !id.contains('/'))
    else {
        return error_response(404, "SORX_ROUTE_NOT_FOUND", "route not found");
    };
    let payload = match parse_json_object(body) {
        Ok(payload) => payload,
        Err(response) => return response,
    };
    let Some(alias) = payload
        .get("alias")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    else {
        return error_response(
            400,
            "SORX_INVALID_BODY",
            "body must include a non-empty alias",
        );
    };
    let public = match payload.get("visibility").and_then(Value::as_str) {
        Some("public") => true,
        Some("private") | None => false,
        Some(other) => {
            return error_response(
                400,
                "SORX_INVALID_BODY",
                &format!("visibility must be `private` or `public`, got `{other}`"),
            );
        }
    };

    let mut registry = match registry_load(store) {
        Ok(registry) => registry,
        Err(response) => return response,
    };
    let new_alias = match registry.promote_alias(deployment_id, alias, public, "http-api", None) {
        Ok(alias) => alias,
        Err(err) => return registry_error_response(err),
    };
    let deployment = registry
        .deployment(deployment_id)
        .cloned()
        .map(|deployment| serde_json::to_value(deployment).unwrap())
        .unwrap_or(Value::Null);
    if let Err(response) = registry_save(store, &registry) {
        return response;
    }
    json_response(
        200,
        json!({
            "deployment": deployment,
            "alias": new_alias,
        }),
    )
}

fn registry_rollback(
    store: &LocalDeploymentRegistryStore,
    route: &str,
    body: &str,
) -> HttpResponse {
    let Some(deployment_id) = route
        .strip_prefix("/v1/sorx/deployments/")
        .and_then(|rest| rest.strip_suffix("/rollback"))
        .filter(|id| !id.is_empty() && !id.contains('/'))
    else {
        return error_response(404, "SORX_ROUTE_NOT_FOUND", "route not found");
    };
    let payload = match parse_json_object(body) {
        Ok(payload) => payload,
        Err(response) => return response,
    };
    let Some(alias) = payload
        .get("alias")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    else {
        return error_response(
            400,
            "SORX_INVALID_BODY",
            "body must include a non-empty alias",
        );
    };
    let Some(to_deployment_id) = payload
        .get("to_deployment_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    else {
        return error_response(
            400,
            "SORX_INVALID_BODY",
            "body must include a non-empty to_deployment_id",
        );
    };
    let reason = payload
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("rollback requested via HTTP API")
        .to_string();

    let mut registry = match registry_load(store) {
        Ok(registry) => registry,
        Err(response) => return response,
    };
    // The rollback target carries tenant/sor scope; the {deployment_id} in the
    // path identifies the deployment whose alias is being rolled back.
    let Some(scope) = registry.deployment(deployment_id).cloned() else {
        return error_response(
            404,
            "SORX_DEPLOYMENT_MISSING",
            &format!("deployment `{deployment_id}` does not exist"),
        );
    };
    let new_alias = match registry.rollback_alias(RollbackAliasRequest {
        tenant_id: scope.tenant_id.clone(),
        sor_name: scope.sor_name.clone(),
        alias: alias.to_string(),
        to_deployment_id: to_deployment_id.to_string(),
        reason,
        actor: "http-api".to_string(),
        automation_source: None,
    }) {
        Ok(alias) => alias,
        Err(err) => return registry_error_response(err),
    };
    if let Err(response) = registry_save(store, &registry) {
        return response;
    }
    json_response(200, json!({ "alias": new_alias }))
}

fn registry_routing_table(
    store: &LocalDeploymentRegistryStore,
    tenant: Option<String>,
    sor: Option<String>,
) -> HttpResponse {
    let registry = match registry_load(store) {
        Ok(registry) => registry,
        Err(response) => return response,
    };
    let mut routes = Vec::new();
    for alias in registry.aliases_for(tenant.as_deref(), sor.as_deref()) {
        let Some(deployment) =
            registry.resolve_alias(&alias.tenant_id, &alias.sor_name, &alias.alias)
        else {
            continue;
        };
        routes.push(json!({
            "tenant_id": alias.tenant_id,
            "sor_name": alias.sor_name,
            "alias": alias.alias,
            "deployment_id": deployment.deployment_id,
            "pack_name": deployment.pack_name,
            "pack_version": deployment.pack_version,
            "base_path": deployment.base_path,
            "state_namespace": deployment.state_namespace,
            "visibility": deployment.visibility,
            "routable": deployment.status.is_routable(),
            "traffic": deployment.traffic,
        }));
    }
    json_response(
        200,
        json!({
            "schema": greentic_sorx_core::DEPLOYMENT_PUBLIC_ROUTE_TABLE_SCHEMA,
            "routes": routes,
        }),
    )
}

fn parse_json_object(body: &str) -> Result<Map<String, Value>, HttpResponse> {
    if body.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(body) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(error_response(
            400,
            "SORX_INVALID_BODY",
            "request body must be a JSON object",
        )),
        Err(err) => Err(error_response(
            400,
            "SORX_INVALID_BODY",
            &format!("request body is not valid JSON: {err}"),
        )),
    }
}

fn generic_admin_action_id(request: &HttpRequest) -> String {
    format!(
        "{} {}",
        request.method.to_ascii_uppercase(),
        request.path.trim_end_matches('/')
    )
}

fn generic_admin_terminal_event(status: u16) -> &'static str {
    if status == 403 {
        "admin.action.denied"
    } else if status >= 400 {
        "admin.action.failed"
    } else {
        "admin.action.completed"
    }
}

fn runtime_metric_catalog(pack: &LoadedSorlaPack) -> SorxResult<Option<RuntimeMetricCatalog>> {
    let Some(metrics) = &pack.sorla_assets.metrics else {
        return Ok(None);
    };
    let runtime_metrics = metrics
        .catalog
        .metrics
        .iter()
        .map(runtime_metric_from_pack)
        .collect::<SorxResult<Vec<_>>>()?;
    RuntimeMetricCatalog::new(runtime_metrics).map(Some)
}

fn runtime_operational_indexes(pack: &LoadedSorlaPack) -> Vec<RuntimeOperationalIndex> {
    pack.sorla_assets
        .operational_indexes
        .as_ref()
        .map(|assets| {
            assets
                .catalog
                .indexes
                .iter()
                .filter(|index| index.unique)
                .map(|index| RuntimeOperationalIndex {
                    id: index.id.clone(),
                    record: index.record.clone(),
                    collection: index.collection.clone(),
                    kind: index.kind.clone(),
                    fields: index.fields.clone(),
                    unique: index.unique,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn runtime_record_access(pack: &LoadedSorlaPack) -> BTreeMap<String, RecordAccessPolicy> {
    let Ok(model) = ciborium::de::from_reader::<Value, _>(pack.sorla_assets.model_cbor.as_slice())
    else {
        return BTreeMap::new();
    };
    model
        .get("records")
        .and_then(Value::as_array)
        .map(|records| {
            records
                .iter()
                .filter_map(|record| {
                    let name = record
                        .get("name")
                        .or_else(|| record.get("id"))
                        .or_else(|| record.get("record"))
                        .and_then(Value::as_str)?;
                    let access = record.get("access")?.as_object()?;
                    Some((
                        name.to_string(),
                        RecordAccessPolicy {
                            read: parse_access_authorization(access.get("read")),
                            create: parse_access_authorization(access.get("create")),
                            update: parse_access_authorization(access.get("update")),
                            delete: parse_access_authorization(access.get("delete")),
                        },
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn manager_record_descriptions(pack: &LoadedSorlaPack) -> BTreeMap<String, String> {
    let Ok(model) = ciborium::de::from_reader::<Value, _>(pack.sorla_assets.model_cbor.as_slice())
    else {
        return BTreeMap::new();
    };
    model
        .get("records")
        .and_then(Value::as_array)
        .map(|records| {
            records
                .iter()
                .filter_map(|record| {
                    let name = record
                        .get("name")
                        .or_else(|| record.get("id"))
                        .or_else(|| record.get("record"))
                        .and_then(Value::as_str)?;
                    let description = record
                        .get("description")
                        .or_else(|| record.get("summary"))
                        .or_else(|| record.get("help_text"))
                        .or_else(|| record.get("helpText"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())?;
                    Some((name.to_string(), description.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn manager_model_records(pack: &LoadedSorlaPack) -> BTreeMap<String, ManagerModelRecord> {
    let Ok(model) = ciborium::de::from_reader::<Value, _>(pack.sorla_assets.model_cbor.as_slice())
    else {
        return BTreeMap::new();
    };
    model
        .get("records")
        .and_then(Value::as_array)
        .map(|records| {
            records
                .iter()
                .filter_map(|record| {
                    let record_name = record
                        .get("name")
                        .or_else(|| record.get("id"))
                        .or_else(|| record.get("record"))
                        .and_then(Value::as_str)?;
                    let collection = record
                        .get("collection")
                        .or_else(|| record.get("table"))
                        .and_then(Value::as_str)
                        .map(ToString::to_string)
                        .unwrap_or_else(|| manager_default_collection(record_name));
                    let label = record
                        .get("label")
                        .or_else(|| record.get("title"))
                        .and_then(Value::as_str)
                        .map(ToString::to_string)
                        .unwrap_or_else(|| humanize_identifier(record_name));
                    let plural_label = record
                        .get("plural_label")
                        .or_else(|| record.get("pluralLabel"))
                        .or_else(|| record.get("plural"))
                        .and_then(Value::as_str)
                        .map(ToString::to_string)
                        .unwrap_or_else(|| humanize_identifier(&collection));
                    let fields = record
                        .get("fields")
                        .and_then(Value::as_array)
                        .map(|fields| {
                            fields
                                .iter()
                                .filter_map(|field| manager_model_field(record_name, field))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let create_roles = manager_model_access_roles(record, "create");
                    let update_roles = manager_model_access_roles(record, "update");
                    let delete_roles = manager_model_access_roles(record, "delete");
                    Some((
                        record_name.to_string(),
                        ManagerModelRecord {
                            record: record_name.to_string(),
                            collection,
                            label,
                            plural_label,
                            fields,
                            create_roles,
                            update_roles,
                            delete_roles,
                        },
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn manager_model_access_roles(record: &Value, operation: &str) -> Vec<String> {
    record
        .get("access")
        .and_then(|access| access.get(operation))
        .and_then(|create| create.get("roles"))
        .and_then(Value::as_array)
        .map(|roles| {
            roles
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn manager_default_collection(record_name: &str) -> String {
    let snake = manager_snake_case(record_name);
    if snake.ends_with('y') {
        format!("{}ies", snake.trim_end_matches('y'))
    } else if snake.ends_with('s') {
        snake
    } else {
        format!("{snake}s")
    }
}

fn manager_model_field(record_name: &str, field: &Value) -> Option<ManagerFieldView> {
    let name = field
        .get("name")
        .or_else(|| field.get("id"))
        .and_then(Value::as_str)?;
    let sensitive = field
        .get("sensitive")
        .or_else(|| field.get("redacted"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let json_type = manager_model_field_type(name, field);
    let generated = field
        .get("generated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || manager_model_generated_identifier_field(record_name, name, json_type.as_deref());
    Some(ManagerFieldView {
        name: name.to_string(),
        label_key: format!(
            "field.{}.{}.label",
            manager_key_like(record_name),
            manager_key_like(name)
        ),
        label: field
            .get("display_label")
            .or_else(|| field.get("label"))
            .or_else(|| field.get("title"))
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .unwrap_or_else(|| humanize_identifier(name)),
        json_type,
        rules: field.get("rules").cloned(),
        generated,
        relationship: None,
        required: field
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        read_only: field
            .get("read_only")
            .or_else(|| field.get("readOnly"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        redacted: sensitive,
        value: None,
        hidden: field
            .get("hidden")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        display_order: field
            .get("display_order")
            .and_then(Value::as_u64)
            .map(|v| u32::try_from(v).unwrap_or(u32::MAX)),
        display_group: field
            .get("display_group")
            .and_then(Value::as_str)
            .map(str::to_string),
        policy: ManagerPolicyDecision::allow(),
    })
}

fn manager_model_field_type(field_name: &str, field: &Value) -> Option<String> {
    field
        .get("type")
        .or_else(|| field.get("json_type"))
        .or_else(|| field.get("jsonType"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            (field.get("references").is_some()
                || field_name == "id"
                || field_name.ends_with("_id")
                || field_name.ends_with("_uuid"))
            .then(|| "uuid".to_string())
        })
}

fn manager_model_generated_identifier_field(
    record_name: &str,
    field_name: &str,
    field_type: Option<&str>,
) -> bool {
    is_uuid_field(field_type)
        && (field_name == "id"
            || field_name.ends_with("_uuid")
            || field_name == manager_default_parent_field(record_name))
}

fn apply_model_endpoint_authorization(pack: &LoadedSorlaPack, router: &mut EndpointRouter) {
    for (endpoint_id, authorization) in runtime_endpoint_authorization(pack) {
        if let Some(endpoint) = router.endpoints.get_mut(&endpoint_id)
            && endpoint.authorization.is_none()
        {
            endpoint.authorization = Some(authorization);
        }
    }
}

fn runtime_endpoint_authorization(
    pack: &LoadedSorlaPack,
) -> BTreeMap<String, AuthorizationRequirement> {
    let Ok(model) = ciborium::de::from_reader::<Value, _>(pack.sorla_assets.model_cbor.as_slice())
    else {
        return BTreeMap::new();
    };
    model
        .get("agent_endpoints")
        .or_else(|| model.get("agentEndpoints"))
        .and_then(Value::as_array)
        .map(|endpoints| {
            endpoints
                .iter()
                .filter_map(|endpoint| {
                    let endpoint_id = endpoint
                        .get("endpoint_id")
                        .or_else(|| endpoint.get("endpointId"))
                        .or_else(|| endpoint.get("id"))
                        .or_else(|| endpoint.get("operation_id"))
                        .or_else(|| endpoint.get("operationId"))
                        .and_then(Value::as_str)?;
                    let authorization = parse_access_authorization(endpoint.get("authorization"))?;
                    Some((endpoint_id.to_string(), authorization))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn manager_authorization_policy_set(
    runtime: &SorxRuntime,
    principal_roles: &[String],
) -> ManagerPolicySet {
    let mut policies = ManagerPolicySet::default();
    let mut visible_actions_by_record = BTreeMap::<String, usize>::new();
    let mut action_records = BTreeMap::<String, String>::new();

    for endpoint in runtime.router.endpoints.values() {
        let Some(record) = endpoint.entity.clone() else {
            continue;
        };
        action_records.insert(endpoint.endpoint_id.clone(), record.clone());
        if manager_endpoint_visible(runtime, endpoint, principal_roles) {
            *visible_actions_by_record.entry(record).or_default() += 1;
        } else {
            policies.actions.insert(
                endpoint.endpoint_id.clone(),
                ManagerPolicyDecision::with_effect(ManagerPolicyEffect::Hide),
            );
        }
    }

    for record in action_records.values() {
        if !visible_actions_by_record.contains_key(record) {
            policies.records.insert(
                record.clone(),
                ManagerPolicyDecision::with_effect(ManagerPolicyEffect::Hide),
            );
        }
    }

    policies
}

fn manager_endpoint_visible(
    runtime: &SorxRuntime,
    endpoint: &EndpointDefinition,
    principal_roles: &[String],
) -> bool {
    if endpoint
        .authorization
        .as_ref()
        .is_some_and(|auth| !authorization_roles_match(auth, principal_roles))
    {
        return false;
    }
    let Some(record) = endpoint.entity.as_deref() else {
        return true;
    };
    let Some(access) = runtime.pack.record_access.get(record) else {
        return true;
    };
    let auth = match endpoint.operation {
        OperationKind::Get | OperationKind::Query => access.read.as_ref(),
        OperationKind::Create => access.create.as_ref(),
        OperationKind::Update => access.update.as_ref(),
        OperationKind::Delete => access.delete.as_ref(),
        OperationKind::Command(_) => {
            return [
                access.read.as_ref(),
                access.create.as_ref(),
                access.update.as_ref(),
                access.delete.as_ref(),
            ]
            .into_iter()
            .flatten()
            .any(|auth| authorization_roles_match(auth, principal_roles));
        }
    };
    auth.is_none_or(|auth| authorization_roles_match(auth, principal_roles))
}

fn authorization_roles_match(auth: &AuthorizationRequirement, principal_roles: &[String]) -> bool {
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

fn parse_access_authorization(value: Option<&Value>) -> Option<AuthorizationRequirement> {
    let object = value?.as_object()?;
    Some(AuthorizationRequirement {
        roles: parse_auth_roles(object.get("roles")),
        policies: object
            .get("policies")
            .and_then(Value::as_array)
            .map(|policies| {
                policies
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        conditions: object.get("conditions").cloned(),
    })
}

fn parse_auth_roles(value: Option<&Value>) -> AuthorizationRoles {
    let Some(value) = value else {
        return AuthorizationRoles::default();
    };
    if let Some(values) = value.as_array() {
        return AuthorizationRoles {
            any_of: values
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect(),
            all_of: Vec::new(),
        };
    }
    let Some(object) = value.as_object() else {
        return AuthorizationRoles::default();
    };
    AuthorizationRoles {
        any_of: json_string_array(object.get("any_of").or_else(|| object.get("anyOf"))),
        all_of: json_string_array(object.get("all_of").or_else(|| object.get("allOf"))),
    }
}

fn json_string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn manager_title(pack: &LoadedSorlaPack) -> String {
    humanize_identifier(&pack.pack_name)
}

fn manager_description(pack: &LoadedSorlaPack, router: &EndpointRouter) -> String {
    if let Some(description) = pack
        .sorla_assets
        .llms_txt_fragment
        .as_deref()
        .and_then(first_llms_prose_line)
    {
        return description;
    }

    let records = router
        .endpoints
        .values()
        .filter_map(|endpoint| endpoint.entity.as_deref())
        .map(humanize_identifier)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    match records.as_slice() {
        [] => format!("Manage {}.", humanize_identifier(&pack.pack_name)),
        [one] => format!("Manage {one}."),
        [first, second] => format!("Manage {first} and {second}."),
        _ => {
            let last = records.last().cloned().unwrap_or_default();
            let prefix = records[..records.len() - 1].join(", ");
            format!("Manage {prefix}, and {last}.")
        }
    }
}

fn first_llms_prose_line(fragment: &str) -> Option<String> {
    fragment
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("Intent:"))
        .map(|line| line.trim_end_matches('.').to_string() + ".")
}

fn builtin_manager_locale_bundle() -> ManagerLocaleBundle {
    ManagerLocaleBundle::new("en").with_catalog(ManagerLocaleCatalog {
        locale: "es".to_string(),
        messages: BTreeMap::from([
            ("record.building.label".to_string(), "Edificio".to_string()),
            (
                "record.building.plural".to_string(),
                "Edificios".to_string(),
            ),
            (
                "record.landlord.label".to_string(),
                "Arrendador".to_string(),
            ),
            (
                "record.landlord.plural".to_string(),
                "Arrendadores".to_string(),
            ),
            (
                "record.maintenance_request.label".to_string(),
                "Solicitud de mantenimiento".to_string(),
            ),
            (
                "record.maintenance_request.plural".to_string(),
                "Solicitudes de mantenimiento".to_string(),
            ),
            ("record.payment.label".to_string(), "Pago".to_string()),
            ("record.payment.plural".to_string(), "Pagos".to_string()),
            (
                "record.tenancy.label".to_string(),
                "Arrendamiento".to_string(),
            ),
            (
                "record.tenancy.plural".to_string(),
                "Arrendamientos".to_string(),
            ),
            ("record.tenant.label".to_string(), "Inquilino".to_string()),
            ("record.tenant.plural".to_string(), "Inquilinos".to_string()),
            ("record.unit.label".to_string(), "Unidad".to_string()),
            ("record.unit.plural".to_string(), "Unidades".to_string()),
            (
                "field.building.address.label".to_string(),
                "Direccion".to_string(),
            ),
            (
                "field.building.landlord_id.label".to_string(),
                "ID de arrendador".to_string(),
            ),
            (
                "field.landlord.email.label".to_string(),
                "Correo electronico".to_string(),
            ),
            (
                "field.landlord.full_name.label".to_string(),
                "Nombre completo".to_string(),
            ),
            (
                "field.payment.amount.label".to_string(),
                "Importe".to_string(),
            ),
            (
                "field.payment.payment_id.label".to_string(),
                "ID de pago".to_string(),
            ),
            (
                "field.payment.status.label".to_string(),
                "Estado".to_string(),
            ),
            (
                "field.payment.tenancy_id.label".to_string(),
                "ID de arrendamiento".to_string(),
            ),
            (
                "field.tenancy.tenant_id.label".to_string(),
                "ID de inquilino".to_string(),
            ),
            (
                "field.tenancy.unit_id.label".to_string(),
                "ID de unidad".to_string(),
            ),
            (
                "field.tenancy.lease_start.label".to_string(),
                "Inicio del contrato".to_string(),
            ),
            (
                "field.tenancy.lease_end.label".to_string(),
                "Fin del contrato".to_string(),
            ),
            (
                "field.tenancy.rent_amount.label".to_string(),
                "Importe del alquiler".to_string(),
            ),
            (
                "field.tenant.email.label".to_string(),
                "Correo electronico".to_string(),
            ),
            (
                "field.tenant.full_name.label".to_string(),
                "Nombre completo".to_string(),
            ),
            (
                "field.unit.building_id.label".to_string(),
                "ID de edificio".to_string(),
            ),
            (
                "field.unit.address.label".to_string(),
                "Direccion".to_string(),
            ),
        ]),
    })
}

fn apply_builtin_manager_text_translations(
    view: &mut greentic_sorx_core::ManagerViewModel,
    locale: &str,
) {
    if !locale
        .split(['-', '_'])
        .next()
        .unwrap_or(locale)
        .eq_ignore_ascii_case("es")
    {
        return;
    }
    view.title = translate_manager_text_es(&view.title).to_string();
    view.description = translate_manager_text_es(&view.description).to_string();
    for record in &mut view.records {
        record.collection = translate_manager_text_es(&record.collection).to_string();
        for field in &mut record.fields {
            if let Some(relationship) = field.relationship.as_mut() {
                relationship.label = translate_manager_text_es(&relationship.label).to_string();
            }
        }
    }
    for item in &mut view.navigation {
        item.collection = translate_manager_text_es(&item.collection).to_string();
    }
}

fn translate_manager_text_es(value: &str) -> &str {
    match value {
        "Landlord Tenant Sor" => "SOR de arrendadores e inquilinos",
        "This package exposes handoff metadata for business-safe agent endpoints." => {
            "Este paquete expone metadatos de traspaso para endpoints de agentes empresariales seguros."
        }
        "Building" => "Edificio",
        "Buildings" => "Edificios",
        "buildings" => "edificios",
        "Landlord" => "Arrendador",
        "Landlords" => "Arrendadores",
        "landlords" => "arrendadores",
        "Maintenance Request" => "Solicitud de mantenimiento",
        "Maintenance Requests" => "Solicitudes de mantenimiento",
        "maintenance_requests" => "solicitudes_de_mantenimiento",
        "Payment" => "Pago",
        "Payments" => "Pagos",
        "payments" => "pagos",
        "Tenancy" => "Arrendamiento",
        "Tenancies" => "Arrendamientos",
        "tenancies" => "arrendamientos",
        "Tenant" => "Inquilino",
        "Tenants" => "Inquilinos",
        "tenants" => "inquilinos",
        "Unit" => "Unidad",
        "Units" => "Unidades",
        "units" => "unidades",
        _ => value,
    }
}

#[derive(Debug, Clone, Default)]
struct ManagerOntologyMetadata {
    relationships: Vec<ManagerRelationshipView>,
    field_relationships: BTreeMap<String, Vec<ManagerFieldRelationshipView>>,
}

#[derive(Debug, Clone, Default)]
struct ManagerRecordHierarchy {
    records: BTreeMap<String, ManagerRecordHierarchyNode>,
}

#[derive(Debug, Clone, Default)]
struct ManagerRecordHierarchyNode {
    main: bool,
    parents: Vec<ManagerRecordHierarchyParent>,
}

#[derive(Debug, Clone, Default)]
struct ManagerRecordHierarchyParent {
    record: String,
    field: Option<String>,
}

#[derive(Debug, Clone)]
struct ManagerModelRecord {
    record: String,
    collection: String,
    label: String,
    plural_label: String,
    fields: Vec<ManagerFieldView>,
    create_roles: Vec<String>,
    update_roles: Vec<String>,
    delete_roles: Vec<String>,
}

fn manager_record_hierarchy(gateway: &Value) -> ManagerRecordHierarchy {
    let mut records = BTreeMap::new();
    let Some(value) = gateway
        .get("record_hierarchy")
        .or_else(|| gateway.get("recordHierarchy"))
    else {
        return ManagerRecordHierarchy::default();
    };
    if let Some(items) = value.as_array() {
        for item in items {
            if let Some((record, node)) = parse_manager_hierarchy_node(item, None) {
                records.insert(record, node);
            }
        }
    } else if let Some(items) = value.as_object() {
        for (record, item) in items {
            if let Some((record, node)) = parse_manager_hierarchy_node(item, Some(record)) {
                records.insert(record, node);
            }
        }
    }
    ManagerRecordHierarchy { records }
}

fn parse_manager_hierarchy_node(
    value: &Value,
    fallback_record: Option<&str>,
) -> Option<(String, ManagerRecordHierarchyNode)> {
    let object = value.as_object()?;
    let record = object
        .get("record")
        .or_else(|| object.get("name"))
        .or_else(|| object.get("id"))
        .and_then(Value::as_str)
        .or(fallback_record)?
        .to_string();
    let main = object.get("main").and_then(Value::as_bool).unwrap_or(false);
    let mut parents = parse_manager_hierarchy_parents(object);
    let parent = object
        .get("parent")
        .or_else(|| object.get("parent_record"))
        .or_else(|| object.get("parentRecord"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    if let Some(parent) = parent
        && !parents.iter().any(|candidate| candidate.record == parent)
    {
        parents.push(ManagerRecordHierarchyParent {
            record: parent,
            field: object
                .get("parent_field")
                .or_else(|| object.get("parentField"))
                .or_else(|| object.get("field"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
        });
    }
    Some((record, ManagerRecordHierarchyNode { main, parents }))
}

fn parse_manager_hierarchy_parents(
    object: &Map<String, Value>,
) -> Vec<ManagerRecordHierarchyParent> {
    object
        .get("parents")
        .and_then(Value::as_array)
        .map(|parents| {
            parents
                .iter()
                .filter_map(|parent| {
                    let object = parent.as_object()?;
                    let record = object.get("record").and_then(Value::as_str)?.to_string();
                    Some(ManagerRecordHierarchyParent {
                        record,
                        field: object
                            .get("field")
                            .or_else(|| object.get("parent_field"))
                            .or_else(|| object.get("parentField"))
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn apply_manager_record_hierarchy(
    view: &mut greentic_sorx_core::ManagerViewModel,
    hierarchy: &ManagerRecordHierarchy,
) {
    if hierarchy.records.is_empty() {
        return;
    }
    view.navigation.retain(|item| {
        hierarchy
            .records
            .get(&item.record)
            .map(|node| node.main)
            .unwrap_or(false)
    });
}

fn manager_child_record_links(
    view: &greentic_sorx_core::ManagerViewModel,
    hierarchy: &ManagerRecordHierarchy,
    parent: &str,
) -> Vec<(String, String)> {
    if hierarchy.records.is_empty() {
        return Vec::new();
    }
    let mut children = hierarchy
        .records
        .iter()
        .filter(|(_, node)| {
            node.parents
                .iter()
                .any(|candidate| candidate.record == parent)
        })
        .filter_map(|(record, _)| {
            view.records
                .iter()
                .find(|candidate| candidate.record == *record)
                .map(|candidate| (record.clone(), candidate.plural_label.clone()))
        })
        .collect::<Vec<_>>();
    children.sort_by(|left, right| left.1.cmp(&right.1));
    children
}

#[derive(Debug, Clone)]
struct ManagerHierarchyPathStep {
    child: String,
    field: String,
}

fn manager_hierarchy_path(
    hierarchy: &ManagerRecordHierarchy,
    from_record: &str,
    to_record: &str,
) -> Option<Vec<ManagerHierarchyPathStep>> {
    if from_record == to_record {
        return Some(Vec::new());
    }
    let mut queue = VecDeque::from([(from_record.to_string(), Vec::new())]);
    let mut seen = BTreeSet::from([from_record.to_string()]);
    while let Some((record, path)) = queue.pop_front() {
        for (child, node) in &hierarchy.records {
            for parent in &node.parents {
                if parent.record != record {
                    continue;
                }
                let mut next_path = path.clone();
                next_path.push(ManagerHierarchyPathStep {
                    child: child.clone(),
                    field: parent
                        .field
                        .clone()
                        .unwrap_or_else(|| manager_default_parent_field(&parent.record)),
                });
                if child == to_record {
                    return Some(next_path);
                }
                if seen.insert(child.clone()) {
                    queue.push_back((child.clone(), next_path));
                }
            }
        }
    }
    None
}

fn manager_default_parent_field(record: &str) -> String {
    format!("{}_id", manager_snake_case(record))
}

fn manager_snake_case(value: &str) -> String {
    let mut out = String::new();
    let mut prev_is_lower_or_digit = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            if ch.is_ascii_uppercase() {
                if prev_is_lower_or_digit && !out.ends_with('_') {
                    out.push('_');
                }
                out.push(ch.to_ascii_lowercase());
                prev_is_lower_or_digit = false;
            } else {
                out.push(ch.to_ascii_lowercase());
                prev_is_lower_or_digit = ch.is_ascii_lowercase() || ch.is_ascii_digit();
            }
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
            prev_is_lower_or_digit = false;
        }
    }
    out.trim_matches('_').to_string()
}

fn manager_ontology_metadata(pack: &LoadedSorlaPack) -> ManagerOntologyMetadata {
    let Some(ontology) = &pack.sorla_assets.ontology else {
        return ManagerOntologyMetadata::default();
    };

    let mut concept_records = BTreeMap::<String, Vec<String>>::new();
    for concept in &ontology.graph.concepts {
        concept_records
            .entry(concept.id.clone())
            .or_default()
            .extend(concept.records.iter().cloned());
    }
    for record in &ontology.graph.records {
        concept_records
            .entry(record.concept_id.clone())
            .or_default()
            .push(record.id.clone());
    }
    for values in concept_records.values_mut() {
        values.sort();
        values.dedup();
    }

    let mut relationships = Vec::new();
    let mut field_relationships = BTreeMap::<String, Vec<ManagerFieldRelationshipView>>::new();
    for relationship in &ontology.graph.relationships {
        let (Some(from), Some(to)) = (relationship.from.as_deref(), relationship.to.as_deref())
        else {
            continue;
        };
        let from_records = concept_records
            .get(from)
            .cloned()
            .unwrap_or_else(|| vec![from.to_string()]);
        let to_records = concept_records
            .get(to)
            .cloned()
            .unwrap_or_else(|| vec![to.to_string()]);
        for from_record in &from_records {
            for to_record in &to_records {
                let label = relationship
                    .label
                    .clone()
                    .unwrap_or_else(|| humanize_identifier(&relationship.id));
                relationships.push(ManagerRelationshipView {
                    id: relationship.id.clone(),
                    from_record: from_record.clone(),
                    to_record: to_record.clone(),
                    label_key: format!("relationship.{}.label", manager_key_like(&relationship.id)),
                    label: label.clone(),
                    limited_context: false,
                    policy: greentic_sorx_core::ManagerPolicyDecision::allow(),
                });
                let to_hint = ManagerFieldRelationshipView {
                    relationship_id: relationship.id.clone(),
                    to_record: to_record.clone(),
                    label: humanize_identifier(to_record),
                };
                field_relationships
                    .entry(from_record.clone())
                    .or_default()
                    .push(to_hint);
                let from_hint = ManagerFieldRelationshipView {
                    relationship_id: relationship.id.clone(),
                    to_record: from_record.clone(),
                    label: humanize_identifier(from_record),
                };
                field_relationships
                    .entry(to_record.clone())
                    .or_default()
                    .push(from_hint);
            }
        }
    }
    relationships.sort_by(|left, right| left.id.cmp(&right.id));
    relationships.dedup_by(|left, right| {
        left.id == right.id
            && left.from_record == right.from_record
            && left.to_record == right.to_record
    });

    ManagerOntologyMetadata {
        relationships,
        field_relationships,
    }
}

fn apply_manager_ontology_metadata(
    view: &mut greentic_sorx_core::ManagerViewModel,
    metadata: &ManagerOntologyMetadata,
) {
    view.relationships = metadata.relationships.clone();
    let known_records = view
        .records
        .iter()
        .map(|record| (record.record.clone(), record.label.clone()))
        .collect::<Vec<_>>();
    for record in &mut view.records {
        let candidates = metadata
            .field_relationships
            .get(&record.record)
            .cloned()
            .unwrap_or_default();
        for field in &mut record.fields {
            if field.generated
                || field.relationship.is_some()
                || !is_uuid_field(field.json_type.as_deref())
            {
                continue;
            }
            field.relationship = candidates
                .iter()
                .find(|candidate| field_name_matches_record(&field.name, &candidate.to_record))
                .cloned()
                .or_else(|| inferred_field_relationship(&field.name, &known_records));
        }
    }
}

fn apply_manager_model_records(
    view: &mut greentic_sorx_core::ManagerViewModel,
    records_by_name: &BTreeMap<String, ManagerModelRecord>,
    hierarchy: &ManagerRecordHierarchy,
    roles: &[String],
) {
    if hierarchy.records.is_empty() {
        return;
    }
    let mut existing = view
        .records
        .iter()
        .map(|record| record.record.clone())
        .collect::<BTreeSet<_>>();
    for record_name in hierarchy.records.keys() {
        if !existing.insert(record_name.clone()) {
            continue;
        }
        let Some(model_record) = records_by_name.get(record_name) else {
            continue;
        };
        view.records.push(ManagerRecordView {
            record: model_record.record.clone(),
            collection: model_record.collection.clone(),
            label_key: format!("record.{}.label", manager_key_like(&model_record.record)),
            label: model_record.label.clone(),
            plural_label_key: format!("record.{}.plural", manager_key_like(&model_record.record)),
            plural_label: model_record.plural_label.clone(),
            fields: model_record.fields.clone(),
            create_field_names: model_record
                .fields
                .iter()
                .filter(|field| !field.generated && !field.read_only)
                .map(|field| field.name.clone())
                .collect(),
            endpoint_ids: Vec::new(),
            policy: ManagerPolicyDecision::allow(),
        });
    }
    let existing_nav = view
        .navigation
        .iter()
        .map(|item| item.record.clone())
        .collect::<BTreeSet<_>>();
    for record in &view.records {
        if existing_nav.contains(&record.record) {
            continue;
        }
        view.navigation.push(ManagerNavItem {
            record: record.record.clone(),
            label_key: record.plural_label_key.clone(),
            label: record.plural_label.clone(),
            collection: record.collection.clone(),
        });
    }
    for model_record in records_by_name.values() {
        apply_manager_model_record_action(view, model_record, roles, "create");
        apply_manager_model_record_action(view, model_record, roles, "update");
        apply_manager_model_record_action(view, model_record, roles, "delete");
    }
}

fn apply_manager_model_record_action(
    view: &mut greentic_sorx_core::ManagerViewModel,
    model_record: &ManagerModelRecord,
    roles: &[String],
    operation: &str,
) {
    if manager_record_action(view, &model_record.record, operation).is_some()
        || !manager_model_record_can_perform(model_record, roles, operation)
    {
        return;
    }
    let action_id = format!(
        "{}.model_{}",
        manager_key_like(&model_record.record),
        operation
    );
    view.actions.push(greentic_sorx_core::ManagerActionView {
        action_id: action_id.clone(),
        endpoint_id: action_id.clone(),
        operation_id: action_id,
        record: Some(model_record.record.clone()),
        label_key: format!(
            "action.{}.{}.label",
            manager_key_like(&model_record.record),
            operation
        ),
        label: format!("{} {}", humanize_identifier(operation), model_record.label),
        risk: "low".to_string(),
        approval_required: false,
        policy: ManagerPolicyDecision::allow(),
    });
}

fn manager_model_record_can_perform(
    model_record: &ManagerModelRecord,
    roles: &[String],
    operation: &str,
) -> bool {
    let allowed_roles = match operation {
        "create" => &model_record.create_roles,
        "update" => &model_record.update_roles,
        "delete" => &model_record.delete_roles,
        _ => return false,
    };
    allowed_roles.is_empty()
        || allowed_roles
            .iter()
            .any(|role| roles.iter().any(|candidate| candidate == role))
}

fn apply_manager_record_fields(
    view: &mut greentic_sorx_core::ManagerViewModel,
    fields_by_record: &BTreeMap<String, Vec<ManagerFieldView>>,
) {
    for record in &mut view.records {
        let Some(model_fields) = fields_by_record.get(&record.record) else {
            continue;
        };
        let mut existing = record
            .fields
            .iter()
            .map(|field| field.name.clone())
            .collect::<BTreeSet<_>>();
        for field in model_fields {
            if existing.insert(field.name.clone()) {
                record.fields.push(field.clone());
            }
        }
    }
}

fn carry_manager_field_relationships(
    scoped: &mut greentic_sorx_core::ManagerViewModel,
    full: &greentic_sorx_core::ManagerViewModel,
) {
    let relationships = full
        .records
        .iter()
        .flat_map(|record| {
            record.fields.iter().filter_map(|field| {
                field.relationship.clone().map(|relationship| {
                    ((record.record.as_str(), field.name.as_str()), relationship)
                })
            })
        })
        .collect::<BTreeMap<_, _>>();

    for record in &mut scoped.records {
        for field in &mut record.fields {
            if field.relationship.is_none()
                && let Some(relationship) =
                    relationships.get(&(record.record.as_str(), field.name.as_str()))
            {
                field.relationship = Some(relationship.clone());
            }
        }
    }
}

fn is_uuid_field(value: Option<&str>) -> bool {
    value.unwrap_or_default().eq_ignore_ascii_case("uuid")
}

fn inferred_field_relationship(
    field_name: &str,
    records: &[(String, String)],
) -> Option<ManagerFieldRelationshipView> {
    records
        .iter()
        .find(|(record, _)| field_name_matches_record(field_name, record))
        .map(|(record, label)| ManagerFieldRelationshipView {
            relationship_id: format!(
                "inferred_{}_to_{}",
                manager_key_like(field_name),
                manager_key_like(record)
            ),
            to_record: record.clone(),
            label: label.clone(),
        })
}

fn field_name_matches_record(field_name: &str, record_name: &str) -> bool {
    let field = field_name
        .trim_end_matches("_id")
        .trim_end_matches("_uuid")
        .trim_end_matches("_ref");
    manager_key_like(field) == manager_key_like(record_name)
}

fn manager_key_like(value: &str) -> String {
    let mut out = String::new();
    for (index, ch) in value.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

fn runtime_metric_from_pack(metric: &MetricDefinition) -> SorxResult<RuntimeMetric> {
    let dimensions = metric
        .dimensions
        .iter()
        .map(|dimension| RuntimeMetricDimension {
            name: dimension.name.clone(),
            field: dimension.field.clone(),
            sensitive: dimension.sensitive,
        })
        .collect::<Vec<_>>();
    let kind = if let Some(measure) = &metric.measure {
        let source = metric.source.as_ref().ok_or_else(|| {
            SorxError::new(
                "metric_source_missing",
                format!("metric `{}` source is required", metric.name),
            )
        })?;
        RuntimeMetricKind::Aggregate {
            source_entity: source.entity.clone(),
            collection: source
                .collection
                .clone()
                .unwrap_or_else(|| source.entity.to_ascii_lowercase()),
            aggregate: metric_aggregate(&measure.aggregate)?,
            field: measure.field.clone(),
        }
    } else if let Some(formula) = &metric.formula {
        RuntimeMetricKind::Formula {
            expression: formula.expression.clone(),
            dependencies: formula.dependencies.clone(),
        }
    } else {
        return Err(SorxError::new(
            "metric_invalid",
            format!("metric `{}` must define measure or formula", metric.name),
        ));
    };
    Ok(RuntimeMetric {
        name: metric.name.clone(),
        label: metric.label.clone(),
        kind,
        dimensions,
        filters: metric
            .filters
            .iter()
            .map(|filter| MetricQueryFilter {
                field: filter.field.clone(),
                operator: normalize_metric_filter_operator(&filter.operator),
                value: filter.value.clone().unwrap_or(Value::Null),
            })
            .collect(),
        cache: metric.cache.as_ref().map(|cache| RuntimeMetricCache {
            ttl_seconds: cache.ttl_seconds,
            scope: cache.scope.clone(),
        }),
    })
}

fn normalize_metric_filter_operator(operator: &str) -> String {
    let normalized = operator.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "eq" | "equal" | "equals" => "eq",
        "ne" | "not_equal" | "not_equals" => "ne",
        "gt" | "greater_than" => "gt",
        "gte" | "greater_than_or_equal" | "greater_than_or_equals" => "gte",
        "lt" | "less_than" => "lt",
        "lte" | "less_than_or_equal" | "less_than_or_equals" => "lte",
        "in" | "one_of" => "in",
        value => value,
    }
    .to_string()
}

fn metric_aggregate(value: &str) -> SorxResult<MetricAggregate> {
    match value {
        "count" => Ok(MetricAggregate::Count),
        "sum" => Ok(MetricAggregate::Sum),
        "avg" => Ok(MetricAggregate::Avg),
        "min" => Ok(MetricAggregate::Min),
        "max" => Ok(MetricAggregate::Max),
        "distinct_count" => Ok(MetricAggregate::DistinctCount),
        _ => Err(SorxError::new(
            "metric_aggregate_unsupported",
            format!("unsupported metric aggregate `{value}`"),
        )),
    }
}

fn combine_manager_datetime_inputs(
    input: &mut Value,
    view: &greentic_sorx_core::ManagerViewModel,
    record_name: &str,
) {
    let Some(object) = input.as_object_mut() else {
        return;
    };
    let Some(record) = view
        .records
        .iter()
        .find(|candidate| candidate.record == record_name)
    else {
        return;
    };

    for field in &record.fields {
        if !is_datetime_field(field.json_type.as_deref()) {
            continue;
        }
        let date_key = datetime_part_key(&field.name, "date");
        let time_key = datetime_part_key(&field.name, "time");
        let date = object
            .remove(&date_key)
            .and_then(|value| value.as_str().map(str::to_string))
            .filter(|value| !value.is_empty());
        let time = object
            .remove(&time_key)
            .and_then(|value| value.as_str().map(str::to_string))
            .filter(|value| !value.is_empty());
        if let Some(date) = date {
            let time = normalize_time_component(time.as_deref().unwrap_or("00:00"));
            object.insert(field.name.clone(), Value::String(format!("{date}T{time}Z")));
        }
    }
}

fn fill_generated_manager_fields(
    input: &mut Value,
    view: &greentic_sorx_core::ManagerViewModel,
    record_name: &str,
    endpoint_id: &str,
) {
    let Some(object) = input.as_object_mut() else {
        return;
    };
    let Some(record) = view
        .records
        .iter()
        .find(|candidate| candidate.record == record_name)
    else {
        return;
    };

    for field in &record.fields {
        if !field.generated || object.get(&field.name).is_some() {
            continue;
        }
        if is_uuid_field(field.json_type.as_deref()) {
            object.insert(
                field.name.clone(),
                Value::String(generated_manager_uuid(
                    endpoint_id,
                    record_name,
                    &field.name,
                )),
            );
        }
    }
}

fn merge_manager_submit_fields(
    input: &mut Value,
    body: &Map<String, Value>,
    view: &greentic_sorx_core::ManagerViewModel,
    record_name: &str,
) {
    let Some(object) = input.as_object_mut() else {
        return;
    };
    let Some(record) = view
        .records
        .iter()
        .find(|record| record.record == record_name)
    else {
        return;
    };
    let mut field_names = record
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<BTreeSet<_>>();
    field_names.extend(record.create_field_names.iter().map(String::as_str));
    for field_name in field_names {
        if object.contains_key(field_name) {
            continue;
        }
        if let Some(value) = body.get(field_name) {
            object.insert(field_name.to_string(), value.clone());
        }
    }
}

fn trim_manager_submit_string_values(value: &mut Value) {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.len() != text.len() {
                *text = trimmed.to_string();
            }
        }
        Value::Array(items) => {
            for item in items {
                trim_manager_submit_string_values(item);
            }
        }
        Value::Object(object) => {
            for value in object.values_mut() {
                trim_manager_submit_string_values(value);
            }
        }
        _ => {}
    }
}

fn generated_manager_uuid(endpoint_id: &str, record_name: &str, field_name: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let digest = Sha256::digest(format!("{endpoint_id}:{record_name}:{field_name}:{nanos}"));
    let hex = hex::encode(digest);
    format!(
        "{}-{}-4{}-a{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[13..16],
        &hex[17..20],
        &hex[20..32]
    )
}

fn stamp_manager_created_at(input: &mut Value, endpoint: &EndpointDefinition) {
    if !matches!(endpoint.operation, OperationKind::Create)
        || !manager_endpoint_is_create_form(endpoint)
    {
        return;
    }
    let Some(object) = input.as_object_mut() else {
        return;
    };
    let has_sortable_timestamp = [
        "_greentic_manager_created_at",
        "created_at",
        "createdAt",
        "submitted_at",
        "submittedAt",
        "timestamp",
        "date",
    ]
    .iter()
    .any(|field| {
        object
            .get(*field)
            .is_some_and(manager_scalar_has_sort_value)
    });
    if has_sortable_timestamp {
        return;
    }
    object.insert(
        "_greentic_manager_created_at".to_string(),
        Value::String(manager_now_sort_value()),
    );
}

fn manager_now_sort_value() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{nanos:020}")
}

fn picker_choice_label(data: &Value, fallback: &str) -> String {
    [
        "name",
        "title",
        "label",
        "summary",
        "full_name",
        "email",
        "address",
        "unit_number",
        "id",
    ]
    .iter()
    .find_map(|field| data.get(field).and_then(Value::as_str))
    .unwrap_or(fallback)
    .to_string()
}

#[derive(Debug, Clone)]
struct RuntimeRecordListState {
    page: usize,
    page_size: usize,
    total: usize,
    start: usize,
    end: usize,
    search: String,
    can_create: bool,
    can_update: bool,
    can_delete: bool,
    parent_context: Option<(String, String)>,
}

#[derive(Debug, Clone)]
struct RuntimeRelatedRecordSection {
    record: ManagerRecordView,
    rows: Vec<EntityRecord>,
    state: RuntimeRecordListState,
}

fn render_runtime_record_list_card(
    view: &greentic_sorx_core::ManagerViewModel,
    record: &ManagerRecordView,
    rows: &[EntityRecord],
    description: Option<String>,
    state: RuntimeRecordListState,
) -> Value {
    let fields = manager_table_fields(record);
    let mut body = vec![json!({
        "type": "TextBlock",
        "text": record.plural_label,
        "wrap": true,
        "size": "large",
        "weight": "Bolder"
    })];
    if let Some(description) = description.filter(|description| !description.trim().is_empty()) {
        body.push(json!({
            "type": "TextBlock",
            "text": description,
            "wrap": true,
            "isSubtle": true
        }));
    }
    body.push(manager_record_context_search_row(
        record,
        &view.locale,
        &state.search,
        state.parent_context.as_ref(),
    ));
    if let Some(summary) = manager_record_list_summary(&view.locale, &state) {
        body.push(json!({
            "type": "TextBlock",
            "text": summary,
            "wrap": true,
            "isSubtle": true,
            "spacing": "small"
        }));
    }
    body.push(manager_record_table_header(
        &fields,
        state.can_update || state.can_delete,
        &view.locale,
    ));
    if rows.is_empty() {
        body.push(json!({
            "type": "TextBlock",
            "text": localized_manager_static(&view.locale, "No records found."),
            "wrap": true,
            "spacing": "medium",
            "isSubtle": true
        }));
    } else {
        for row in rows {
            body.push(manager_record_table_row(
                record,
                &fields,
                row,
                state.can_update,
                state.can_delete,
                &view.locale,
            ));
        }
    }

    body.push(manager_record_footer_actions(record, &view.locale, &state));

    json!({
        "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
        "type": "AdaptiveCard",
        "version": "1.5",
        "lang": view.locale,
        "metadata": {
            "schema": "greentic.sorx.manager-card.v1",
            "kind": "manager.record.list",
            "locale": view.locale,
            "record": record.record,
            "page": state.page,
            "page_size": state.page_size,
            "total": state.total,
            "query": state.search,
            "parent_record": state.parent_context.as_ref().map(|(record, _)| record),
            "parent_id": state.parent_context.as_ref().map(|(_, id)| id)
        },
        "body": body,
        "actions": []
    })
}

fn render_runtime_record_detail_card(
    view: &greentic_sorx_core::ManagerViewModel,
    record_name: &str,
    id: &str,
    delete_action: Option<&greentic_sorx_core::ManagerActionView>,
    related_sections: Vec<RuntimeRelatedRecordSection>,
) -> Option<Value> {
    let mut card = render_record_detail_card(view, record_name, id)?;
    if let Some(body) = card.get_mut("body").and_then(Value::as_array_mut) {
        if let Some(delete_action) = delete_action {
            append_manager_detail_delete_action(body, view, record_name, id, delete_action);
        }
        for section in related_sections {
            body.push(render_runtime_related_record_section(&view.locale, section));
        }
        body.push(json!({
            "type": "ActionSet",
            "spacing": "medium",
            "actions": [manager_open_action(
            &format!("< {}", localized_manager_static(&view.locale, "Back")),
            &format!("records/{record_name}"),
            json!({ "_action_style": "secondary" }),
            )]
        }));
    }
    Some(card)
}

fn append_manager_detail_delete_action(
    body: &mut [Value],
    view: &greentic_sorx_core::ManagerViewModel,
    record_name: &str,
    id: &str,
    _delete_action: &greentic_sorx_core::ManagerActionView,
) {
    let Some(action_set) = body.iter_mut().find(|item| {
        item.get("type").and_then(Value::as_str) == Some("ActionSet")
            && item
                .get("actions")
                .and_then(Value::as_array)
                .is_some_and(|actions| {
                    actions.iter().any(|action| {
                        action
                            .get("data")
                            .and_then(Value::as_object)
                            .and_then(|data| data.get("action"))
                            .and_then(Value::as_str)
                            == Some("manager_submit")
                    })
                })
    }) else {
        return;
    };
    let Some(actions) = action_set.get_mut("actions").and_then(Value::as_array_mut) else {
        return;
    };
    actions.push(manager_open_action(
        localized_manager_static(&view.locale, "Delete"),
        &format!("records/{record_name}/{id}/delete"),
        json!({ "_action_style": "destructive" }),
    ));
}

fn render_runtime_related_record_section(
    locale: &str,
    section: RuntimeRelatedRecordSection,
) -> Value {
    let fields = manager_table_fields(&section.record);
    let mut items = vec![json!({
        "type": "TextBlock",
        "text": section.record.plural_label,
        "wrap": true,
        "size": "medium",
        "weight": "Bolder"
    })];
    items.push(manager_record_context_search_row(
        &section.record,
        locale,
        "",
        section.state.parent_context.as_ref(),
    ));
    if let Some(summary) = manager_record_list_summary(locale, &section.state) {
        items.push(json!({
            "type": "TextBlock",
            "text": summary,
            "wrap": true,
            "isSubtle": true,
            "spacing": "small"
        }));
    }
    items.push(manager_record_table_header(
        &fields,
        section.state.can_update || section.state.can_delete,
        locale,
    ));
    if section.rows.is_empty() {
        items.push(json!({
            "type": "TextBlock",
            "text": localized_manager_static(locale, "No records found."),
            "wrap": true,
            "spacing": "small",
            "isSubtle": true
        }));
    } else {
        for row in &section.rows {
            items.push(manager_record_table_row(
                &section.record,
                &fields,
                row,
                section.state.can_update,
                section.state.can_delete,
                locale,
            ));
        }
    }
    items.push(manager_related_record_actions(
        &section.record,
        locale,
        &section.state,
    ));
    json!({
        "type": "Container",
        "separator": true,
        "spacing": "large",
        "items": items
    })
}

fn manager_related_record_actions(
    record: &ManagerRecordView,
    locale: &str,
    state: &RuntimeRecordListState,
) -> Value {
    let mut actions = Vec::new();
    if state.total > state.end {
        actions.push(manager_open_action(
            localized_manager_static(locale, "View All"),
            &manager_record_list_target(record, 1, &state.search, state.parent_context.as_ref()),
            Value::Object(Map::new()),
        ));
    }
    if state.can_create {
        let mut target = format!("records/{}/create", record.record);
        if let Some((parent_record, parent_id)) = state.parent_context.as_ref() {
            target.push_str("?parent_record=");
            target.push_str(&percent_encode_query(parent_record));
            target.push_str("&parent_id=");
            target.push_str(&percent_encode_query(parent_id));
        }
        actions.push(manager_open_action(
            &format!(
                "{} {}",
                localized_manager_static(locale, "Add"),
                record.label
            ),
            &target,
            json!({ "_action_style": "positive" }),
        ));
    }
    json!({
        "type": "ActionSet",
        "spacing": "medium",
        "actions": actions
    })
}

fn manager_record_footer_actions(
    record: &ManagerRecordView,
    locale: &str,
    state: &RuntimeRecordListState,
) -> Value {
    let mut actions = Vec::new();
    if state.page > 1 {
        actions.push(manager_open_action(
            localized_manager_static(locale, "Previous"),
            &manager_record_list_target(
                record,
                state.page - 1,
                &state.search,
                state.parent_context.as_ref(),
            ),
            Value::Object(Map::new()),
        ));
    }
    if state.end < state.total {
        actions.push(manager_open_action(
            localized_manager_static(locale, "Next"),
            &manager_record_list_target(
                record,
                state.page + 1,
                &state.search,
                state.parent_context.as_ref(),
            ),
            Value::Object(Map::new()),
        ));
    }
    if state.can_create {
        let mut target = format!("records/{}/create", record.record);
        if let Some((parent_record, parent_id)) = state.parent_context.as_ref() {
            target.push_str("?parent_record=");
            target.push_str(&percent_encode_query(parent_record));
            target.push_str("&parent_id=");
            target.push_str(&percent_encode_query(parent_id));
        }
        actions.push(manager_open_action(
            &format!(
                "{} {}",
                localized_manager_static(locale, "Add"),
                record.label
            ),
            &target,
            json!({ "_action_style": "positive" }),
        ));
    }
    if let Some((parent_record, parent_id)) = state.parent_context.as_ref() {
        let target = format!("records/{parent_record}/{parent_id}");
        actions.push(manager_open_action(
            &format!("< {}", localized_manager_static(locale, "Back")),
            &target,
            json!({ "_action_style": "secondary" }),
        ));
    } else {
        actions.push(manager_open_action(
            &format!("< {}", localized_manager_static(locale, "Main Menu")),
            "dashboard",
            json!({ "_action_style": "secondary" }),
        ));
    }
    json!({
        "type": "ActionSet",
        "spacing": "medium",
        "actions": actions
    })
}

fn render_runtime_record_delete_card(
    view: &greentic_sorx_core::ManagerViewModel,
    record: &ManagerRecordView,
    id: &str,
    business_id: &str,
    action: &greentic_sorx_core::ManagerActionView,
) -> Value {
    let back_target = format!("records/{}", record.record);
    let input = if action.record.as_deref() == Some("Record") {
        json!({
            "record_name": record.record,
            "record_id": business_id,
            "reason": "Deleted from manager WebChat"
        })
    } else {
        json!({ "id": id })
    };
    json!({
        "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
        "type": "AdaptiveCard",
        "version": "1.5",
        "lang": view.locale,
        "metadata": {
            "schema": "greentic.sorx.manager-card.v1",
            "kind": "manager.record.delete",
            "locale": view.locale,
            "record": record.record,
            "id": id
        },
        "body": [
            {
                "type": "TextBlock",
                "text": format!("{} {}", localized_manager_static(&view.locale, "Delete"), record.label),
                "wrap": true,
                "size": "large",
                "weight": "Bolder"
            },
            {
                "type": "TextBlock",
                "text": format!("{} {id}?", localized_manager_static(&view.locale, "Delete this record")),
                "wrap": true
            }
        ],
        "actions": [
            {
                "type": "Action.Submit",
                "title": localized_manager_static(&view.locale, "Delete"),
                "style": "destructive",
                "data": {
                    "record": record.record,
                    "id": id,
                    "endpoint_id": action.endpoint_id,
                    "operation_id": action.operation_id,
                    "input": input,
                    "action": "manager_submit",
                    "manager_target": back_target,
                    "routeToCardId": manager_route_card_id(&back_target),
                    "cardId": manager_route_card_id(&back_target),
                    "step": "submit"
                }
            },
            manager_open_action(
                &format!("< {}", localized_manager_static(&view.locale, "Back")),
                &back_target,
                json!({ "_action_mode": "secondary" })
            )
        ]
    })
}

#[derive(Debug, Clone)]
struct RuntimeMetricCardRow {
    name: String,
    label: Option<String>,
    result: Option<MetricQueryResult>,
    error: Option<String>,
}

fn render_runtime_metrics_card(
    view: &greentic_sorx_core::ManagerViewModel,
    metrics: &[RuntimeMetricCardRow],
) -> Value {
    let mut body = vec![json!({
        "type": "TextBlock",
        "text": localized_manager_static(&view.locale, "Metrics"),
        "wrap": true,
        "size": "large",
        "weight": "Bolder"
    })];
    if metrics.is_empty() {
        body.push(json!({
            "type": "TextBlock",
            "text": localized_manager_static(&view.locale, "No metrics are declared."),
            "wrap": true,
            "isSubtle": true
        }));
    } else {
        body.push(manager_metrics_summary_table(&view.locale, metrics));
    }
    let mut actions = metrics
        .iter()
        .map(|metric| {
            manager_open_action(
                metric.label.as_deref().unwrap_or(&metric.name),
                &format!("metrics/{}", metric.name),
                Value::Object(Map::new()),
            )
        })
        .collect::<Vec<_>>();
    actions.push(manager_open_action(
        &format!("< {}", localized_manager_static(&view.locale, "Main Menu")),
        "dashboard",
        json!({ "_action_style": "secondary" }),
    ));
    body.push(json!({
        "type": "ActionSet",
        "spacing": "medium",
        "actions": actions
    }));
    json!({
        "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
        "type": "AdaptiveCard",
        "version": "1.5",
        "lang": view.locale,
        "metadata": {
            "schema": "greentic.sorx.manager-card.v1",
            "kind": "manager.metrics",
            "locale": view.locale
        },
        "body": body,
        "actions": []
    })
}

fn render_runtime_metric_detail_card(
    view: &greentic_sorx_core::ManagerViewModel,
    metric_name: &str,
    metric: Option<&RuntimeMetric>,
    result: Option<&MetricQueryResult>,
    error: Option<&str>,
) -> Value {
    let title = metric
        .and_then(|metric| metric.label.as_deref())
        .unwrap_or(metric_name);
    let mut body = vec![json!({
        "type": "TextBlock",
        "text": title,
        "wrap": true,
        "size": "large",
        "weight": "Bolder"
    })];
    if let Some(metric) = metric {
        body.push(json!({
            "type": "TextBlock",
            "text": format!("{}: {}", localized_manager_static(&view.locale, "Metric"), metric.name),
            "wrap": true,
            "isSubtle": true
        }));
        if let Some(result) = result {
            body.push(manager_metric_result_table(&view.locale, result));
        } else if let Some(error) = error {
            body.push(json!({
                "type": "TextBlock",
                "text": format!(
                    "{}: {}",
                    localized_manager_static(&view.locale, "Metric query failed."),
                    error
                ),
                "wrap": true,
                "isSubtle": true
            }));
        } else {
            body.push(json!({
                "type": "TextBlock",
                "text": localized_manager_static(&view.locale, "No metric data found."),
                "wrap": true,
                "isSubtle": true
            }));
        }
    } else {
        body.push(json!({
            "type": "TextBlock",
            "text": localized_manager_static(&view.locale, "Metric not found."),
            "wrap": true,
            "isSubtle": true
        }));
    }
    body.push(json!({
        "type": "ActionSet",
        "spacing": "medium",
        "actions": [
            manager_open_action(
                &format!("< {}", localized_manager_static(&view.locale, "Metrics")),
                "metrics",
                json!({ "_action_style": "secondary" })
            )
        ]
    }));
    json!({
        "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
        "type": "AdaptiveCard",
        "version": "1.5",
        "lang": view.locale,
        "metadata": {
            "schema": "greentic.sorx.manager-card.v1",
            "kind": "manager.metric",
            "locale": view.locale,
            "metric": metric_name
        },
        "body": body,
        "actions": []
    })
}

fn metric_query_error_message(err: &SorxError) -> String {
    format!("{} ({})", err.message, err.code)
}

fn manager_metrics_summary_table(locale: &str, metrics: &[RuntimeMetricCardRow]) -> Value {
    let mut rows = vec![metric_table_header(&[
        localized_manager_static(locale, "Metric"),
        localized_manager_static(locale, "Value"),
        localized_manager_static(locale, "Rows"),
    ])];
    for metric in metrics {
        let value = metric
            .result
            .as_ref()
            .and_then(|result| result.rows.first())
            .map(|row| format_metric_value(row.value))
            .or_else(|| metric.error.clone())
            .unwrap_or_else(|| "-".to_string());
        let row_count = metric
            .result
            .as_ref()
            .map(|result| result.rows.len().to_string())
            .unwrap_or_else(|| "-".to_string());
        rows.push(metric_table_row(&[
            metric.label.as_deref().unwrap_or(&metric.name).to_string(),
            value,
            row_count,
        ]));
    }
    json!({
        "type": "Container",
        "spacing": "medium",
        "items": rows
    })
}

fn manager_metric_result_table(locale: &str, result: &MetricQueryResult) -> Value {
    let mut dimension_names = result
        .rows
        .iter()
        .flat_map(|row| row.dimensions.keys().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    dimension_names.sort();
    let mut headers = dimension_names.clone();
    headers.push(localized_manager_static(locale, "Value").to_string());
    let mut rows = vec![metric_table_header(&headers)];
    if result.rows.is_empty() {
        rows.push(metric_table_row(&[localized_manager_static(
            locale,
            "No metric data found.",
        )
        .to_string()]));
    } else {
        for row in result.rows.iter().take(10) {
            let mut cells = dimension_names
                .iter()
                .map(|dimension| {
                    row.dimensions
                        .get(dimension)
                        .map(metric_cell_value)
                        .unwrap_or_else(|| "-".to_string())
                })
                .collect::<Vec<_>>();
            cells.push(format_metric_value(row.value));
            rows.push(metric_table_row(&cells));
        }
    }
    json!({
        "type": "Container",
        "spacing": "medium",
        "items": rows
    })
}

fn metric_table_header(headers: &[impl AsRef<str>]) -> Value {
    json!({
        "type": "ColumnSet",
        "spacing": "small",
        "columns": headers.iter().map(|header| json!({
            "type": "Column",
            "width": "stretch",
            "items": [{
                "type": "TextBlock",
                "text": header.as_ref(),
                "wrap": true,
                "weight": "Bolder"
            }]
        })).collect::<Vec<_>>()
    })
}

fn metric_table_row(cells: &[String]) -> Value {
    json!({
        "type": "ColumnSet",
        "separator": true,
        "spacing": "small",
        "columns": cells.iter().map(|cell| json!({
            "type": "Column",
            "width": "stretch",
            "items": [{
                "type": "TextBlock",
                "text": cell,
                "wrap": true
            }]
        })).collect::<Vec<_>>()
    })
}

fn metric_cell_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null => "-".to_string(),
        value => serde_json::to_string(value).unwrap_or_else(|_| "-".to_string()),
    }
}

fn format_metric_value(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value:.2}")
    }
}

fn manager_record_table_header(
    fields: &[ManagerFieldView],
    include_actions: bool,
    locale: &str,
) -> Value {
    let mut columns = fields
        .iter()
        .map(|field| {
            json!({
                "type": "Column",
                "width": "stretch",
                "items": [{
                    "type": "TextBlock",
                    "text": field.label,
                    "wrap": true,
                    "weight": "Bolder"
                }]
            })
        })
        .collect::<Vec<_>>();
    if include_actions {
        columns.push(json!({
            "type": "Column",
            "width": "auto",
            "items": [{
                "type": "TextBlock",
                "text": localized_manager_static(locale, "Actions"),
                "wrap": true,
                "weight": "Bolder"
            }]
        }));
    }
    columns.push(json!({
        "type": "Column",
        "width": "auto",
        "items": [{
            "type": "TextBlock",
            "text": "",
            "wrap": false
        }]
    }));
    json!({
        "type": "ColumnSet",
        "spacing": "medium",
        "columns": columns
    })
}

fn manager_record_context_search_row(
    record: &ManagerRecordView,
    locale: &str,
    search: &str,
    parent_context: Option<&(String, String)>,
) -> Value {
    let target = manager_record_list_target(record, 1, "", parent_context);
    let input_id = format!("manager_search_{}", manager_route_card_id(&record.record));
    let mut search_action = manager_open_action(
        localized_manager_static(locale, "Search Icon"),
        &target,
        json!({
            "manager_search_input": input_id,
            "manager_page_size": 10
        }),
    );
    search_action["associatedInputs"] = Value::String("auto".to_string());
    json!({
        "type": "ColumnSet",
        "spacing": "medium",
        "columns": [
            {
                "type": "Column",
                "width": "stretch",
                "items": [{
                    "type": "Input.Text",
                    "id": input_id.clone(),
                    "label": localized_manager_static(locale, "Search"),
                    "placeholder": record.plural_label,
                    "value": search
                }]
            },
            {
                "type": "Column",
                "width": "auto",
                "verticalContentAlignment": "bottom",
                "items": [{
                    "type": "ActionSet",
                    "actions": [search_action]
                }]
            }
        ]
    })
}

fn manager_record_table_row(
    record: &ManagerRecordView,
    fields: &[ManagerFieldView],
    row: &EntityRecord,
    include_edit: bool,
    include_delete: bool,
    locale: &str,
) -> Value {
    let target = format!("records/{}/{}", record.record, row.id);
    let mut columns = fields
        .iter()
        .map(|field| {
            json!({
                "type": "Column",
                "width": "stretch",
                "items": [{
                    "type": "TextBlock",
                    "text": manager_table_cell_value(&row.data, field),
                    "wrap": true
                }]
            })
        })
        .collect::<Vec<_>>();
    if include_edit || include_delete {
        let mut actions = Vec::new();
        if include_edit {
            actions.push(manager_open_action(
                localized_manager_static(locale, "Edit Icon"),
                &target,
                json!({
                    "_action_icon": manager_edit_icon_url(),
                    "manager_action": "edit"
                }),
            ));
        }
        if include_delete {
            actions.push(manager_open_action(
                localized_manager_static(locale, "Delete Icon"),
                &format!("{target}/delete"),
                json!({
                    "_action_icon": manager_delete_icon_url(),
                    "manager_action": "delete"
                }),
            ));
        }
        columns.push(json!({
            "type": "Column",
            "width": "auto",
            "items": [{
                "type": "ActionSet",
                "actions": actions
            }]
        }));
    }
    columns.push(json!({
        "type": "Column",
        "width": "auto",
        "verticalContentAlignment": "center",
        "items": [{
            "type": "TextBlock",
            "text": format!("{} >", localized_manager_static(locale, "Open")),
            "wrap": false,
            "color": "Accent",
            "weight": "Bolder",
            "horizontalAlignment": "Right"
        }]
    }));
    json!({
        "type": "ColumnSet",
        "separator": true,
        "selectAction": manager_open_action(
            localized_manager_static(locale, "Open"),
            &target,
            json!({ "manager_action": "open" }),
        ),
        "columns": columns
    })
}

fn manager_table_fields(record: &ManagerRecordView) -> Vec<ManagerFieldView> {
    let mut candidate_fields: Vec<ManagerFieldView> = record
        .fields
        .iter()
        .filter(|field| {
            !field.redacted
                && !field.generated
                && !manager_is_identifier_field(field)
                && !field.hidden
        })
        .cloned()
        .collect();

    // Apply display_order sort when at least one field carries it
    if candidate_fields.iter().any(|f| f.display_order.is_some()) {
        candidate_fields.sort_by(|a, b| {
            a.display_order
                .unwrap_or(u32::MAX)
                .cmp(&b.display_order.unwrap_or(u32::MAX))
                .then_with(|| a.name.cmp(&b.name))
        });
    }

    // Note: manager_prioritized_table_fields re-sorts by domain priority (e.g.
    // invitation_code / waitlist), which can override the display_order sort above.
    // Hint order is therefore not guaranteed on those tables.
    let mut fields = manager_prioritized_table_fields(candidate_fields);
    fields.truncate(4);
    if fields.is_empty() {
        fields = record
            .fields
            .iter()
            .filter(|field| !field.redacted && !field.hidden)
            .take(4)
            .cloned()
            .collect();
    }
    fields
}

fn manager_prioritized_table_fields(mut fields: Vec<ManagerFieldView>) -> Vec<ManagerFieldView> {
    let has_invitation_code = fields.iter().any(|field| field.name == "invitation_code");
    if !has_invitation_code {
        return fields;
    }
    fields.sort_by_key(|field| {
        (
            manager_table_field_priority(field, has_invitation_code),
            field.name.clone(),
        )
    });
    fields
}

fn manager_table_field_priority(field: &ManagerFieldView, has_invitation_code: bool) -> u8 {
    match field.name.as_str() {
        "email" => 0,
        "invitation_code" => 1,
        "name" | "full_name" | "display_name" => 2,
        "referred_count" | "referral_count" => 3,
        "invited_by_code" if has_invitation_code => 8,
        _ => 4,
    }
}

fn manager_is_identifier_field(field: &ManagerFieldView) -> bool {
    let name = field.name.to_ascii_lowercase();
    name == "id"
        || name.ends_with("_id")
        || name.ends_with("_uuid")
        || field
            .json_type
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("uuid"))
}

fn manager_table_cell_value(data: &Value, field: &ManagerFieldView) -> String {
    match data.get(&field.name) {
        Some(Value::String(value)) if !value.is_empty() => value.clone(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Null) | None => String::new(),
        Some(value) => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn collect_manager_row_identifier_values(
    record_name: &str,
    data: &Value,
    ids: &mut BTreeSet<String>,
) {
    let Some(object) = data.as_object() else {
        return;
    };
    let record_id_field = manager_default_parent_field(record_name);
    for (key, value) in object {
        let key = key.to_ascii_lowercase();
        let is_identifier =
            key == "id" || key == "record_id" || key == record_id_field || key.ends_with("_uuid");
        if is_identifier {
            collect_manager_scalar_identifier_values(value, ids);
        }
    }
}

fn manager_business_record_id(record_name: &str, data: &Value) -> Option<String> {
    let mut ids = BTreeSet::new();
    collect_manager_row_identifier_values(record_name, data, &mut ids);
    let record_id_field = manager_default_parent_field(record_name);
    data.as_object()
        .and_then(|object| object.get(&record_id_field))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| ids.into_iter().next())
}

fn collect_manager_scalar_identifier_values(value: &Value, ids: &mut BTreeSet<String>) {
    match value {
        Value::String(value) if !value.is_empty() => {
            ids.insert(value.clone());
        }
        Value::Number(value) => {
            ids.insert(value.to_string());
        }
        Value::Array(values) => {
            for value in values {
                collect_manager_scalar_identifier_values(value, ids);
            }
        }
        Value::Object(object) => {
            for key in ["id", "value", "record_id"] {
                if let Some(value) = object.get(key) {
                    collect_manager_scalar_identifier_values(value, ids);
                }
            }
        }
        _ => {}
    }
}

fn remove_manager_card_input(card: &mut Value, input_id: &str) {
    match card {
        Value::Object(object) => {
            for child in object.values_mut() {
                remove_manager_card_input(child, input_id);
            }
        }
        Value::Array(items) => {
            items.retain(|item| !manager_card_element_contains_input(item, input_id));
            for item in items {
                remove_manager_card_input(item, input_id);
            }
        }
        _ => {}
    }
}

fn manager_card_element_contains_input(value: &Value, input_id: &str) -> bool {
    match value {
        Value::Object(object) => {
            if object
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.starts_with("Input."))
                && object.get("id").and_then(Value::as_str) == Some(input_id)
            {
                return true;
            }
            object
                .values()
                .any(|child| manager_card_element_contains_input(child, input_id))
        }
        Value::Array(items) => items
            .iter()
            .any(|item| manager_card_element_contains_input(item, input_id)),
        _ => false,
    }
}

fn set_manager_card_submit_parent_context(
    card: &mut Value,
    child_record: &str,
    parent_record: &str,
    parent_id: &str,
    parent_field: &str,
    parent_value: &str,
) {
    match card {
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("Action.Submit")
                && let Some(data) = object.get_mut("data").and_then(Value::as_object_mut)
            {
                let target = manager_record_context_target(child_record, parent_record, parent_id);
                let is_submit_for_child =
                    data.get("record").and_then(Value::as_str) == Some(child_record);
                let is_open_to_child_list = data
                    .get("manager_target")
                    .and_then(Value::as_str)
                    .is_some_and(|target| target == format!("records/{child_record}"));
                if is_submit_for_child || is_open_to_child_list {
                    data.insert("manager_target".to_string(), Value::String(target.clone()));
                    data.insert(
                        "routeToCardId".to_string(),
                        Value::String(manager_route_card_id(&target)),
                    );
                    data.insert(
                        "cardId".to_string(),
                        Value::String(manager_route_card_id(&target)),
                    );
                    if is_submit_for_child {
                        let input = data
                            .entry("input")
                            .or_insert_with(|| Value::Object(Map::new()));
                        if let Some(input) = input.as_object_mut() {
                            input.insert(
                                parent_field.to_string(),
                                Value::String(parent_value.to_string()),
                            );
                        }
                    }
                }
            }
            for child in object.values_mut() {
                set_manager_card_submit_parent_context(
                    child,
                    child_record,
                    parent_record,
                    parent_id,
                    parent_field,
                    parent_value,
                );
            }
        }
        Value::Array(items) => {
            for item in items {
                set_manager_card_submit_parent_context(
                    item,
                    child_record,
                    parent_record,
                    parent_id,
                    parent_field,
                    parent_value,
                );
            }
        }
        _ => {}
    }
}

fn manager_record_list_summary(locale: &str, state: &RuntimeRecordListState) -> Option<String> {
    if state.total == 0 {
        return None;
    }
    Some(format!(
        "{} {}-{} {} {}",
        localized_manager_static(locale, "Showing"),
        state.start + 1,
        state.end,
        localized_manager_static(locale, "of"),
        state.total
    ))
}

fn manager_record_list_target(
    record: &ManagerRecordView,
    page: usize,
    search: &str,
    parent_context: Option<&(String, String)>,
) -> String {
    let mut target = format!("records/{}?page={page}", record.record);
    if !search.is_empty() {
        target.push_str("&q=");
        target.push_str(&percent_encode_query(search));
    }
    if let Some((parent_record, parent_id)) = parent_context {
        target.push_str("&parent_record=");
        target.push_str(&percent_encode_query(parent_record));
        target.push_str("&parent_id=");
        target.push_str(&percent_encode_query(parent_id));
    }
    target
}

fn manager_record_context_target(
    child_record: &str,
    parent_record: &str,
    parent_id: &str,
) -> String {
    format!(
        "records/{child_record}?parent_record={}&parent_id={}",
        percent_encode_query(parent_record),
        percent_encode_query(parent_id)
    )
}

fn manager_record_value_matches_ids(value: &Value, ids: &BTreeSet<String>) -> bool {
    match value {
        Value::String(value) => ids.contains(value),
        Value::Array(values) => values
            .iter()
            .any(|value| manager_record_value_matches_ids(value, ids)),
        Value::Object(object) => ["id", "value", "record_id"]
            .iter()
            .filter_map(|key| object.get(*key))
            .any(|value| manager_record_value_matches_ids(value, ids)),
        _ => false,
    }
}

fn manager_open_action(title: &str, target: &str, extra_data: Value) -> Value {
    let route_to_card_id = manager_route_card_id(target);
    let mut data = Map::new();
    let mut action_mode = None;
    let mut action_style = None;
    let mut action_icon = None;
    data.insert(
        "manager_target".to_string(),
        Value::String(target.to_string()),
    );
    data.insert(
        "routeToCardId".to_string(),
        Value::String(route_to_card_id.clone()),
    );
    data.insert(
        "cardId".to_string(),
        Value::String(route_to_card_id.clone()),
    );
    data.insert("step".to_string(), Value::String("open".to_string()));
    if let Value::Object(extra) = extra_data {
        for (key, value) in extra {
            match key.as_str() {
                "_action_mode" => action_mode = value.as_str().map(ToString::to_string),
                "_action_style" => action_style = value.as_str().map(ToString::to_string),
                "_action_icon" => action_icon = value.as_str().map(ToString::to_string),
                _ => {
                    data.insert(key, value);
                }
            }
        }
    }
    let mut action = json!({
        "type": "Action.Submit",
        "title": title,
        "associatedInputs": "none",
        "data": data
    });
    if let Some(mode) = action_mode {
        action["mode"] = Value::String(mode);
    }
    if let Some(style) = action_style {
        action["style"] = Value::String(style);
    }
    if let Some(icon) = action_icon {
        action["iconUrl"] = Value::String(icon);
    }
    action
}

fn manager_edit_icon_url() -> &'static str {
    "data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A//www.w3.org/2000/svg%22%20viewBox%3D%220%200%2024%2024%22%20fill%3D%22none%22%20stroke%3D%22%230f172a%22%20stroke-width%3D%222%22%3E%3Cpath%20d%3D%22M12%2020h9%22/%3E%3Cpath%20d%3D%22M16.5%203.5a2.1%202.1%200%200%201%203%203L7%2019l-4%201%201-4Z%22/%3E%3C/svg%3E"
}

fn manager_delete_icon_url() -> &'static str {
    "data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A//www.w3.org/2000/svg%22%20viewBox%3D%220%200%2024%2024%22%20fill%3D%22none%22%20stroke%3D%22%23b91c1c%22%20stroke-width%3D%222%22%3E%3Cpath%20d%3D%22M3%206h18%22/%3E%3Cpath%20d%3D%22M8%206V4h8v2%22/%3E%3Cpath%20d%3D%22M19%206l-1%2014H6L5%206%22/%3E%3Cpath%20d%3D%22M10%2011v6%22/%3E%3Cpath%20d%3D%22M14%2011v6%22/%3E%3C/svg%3E"
}

fn manager_route_card_id(target: &str) -> String {
    if target == "dashboard" {
        return "sorx_dashboard".to_string();
    }
    target
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn manager_view_has_record_action(
    view: &greentic_sorx_core::ManagerViewModel,
    record: &str,
    operation: &str,
) -> bool {
    manager_record_action(view, record, operation).is_some()
}

fn manager_record_action<'a>(
    view: &'a greentic_sorx_core::ManagerViewModel,
    record: &str,
    operation: &str,
) -> Option<&'a greentic_sorx_core::ManagerActionView> {
    let operations = if operation == "delete" {
        vec!["delete", "remove"]
    } else {
        vec![operation]
    };
    view.actions.iter().find(|action| {
        matches!(action.record.as_deref(), Some(action_record) if action_record == record || action_record == "Record")
            && operations.iter().any(|operation| {
                let marker = format!(".{operation}");
                let underscore_marker = format!("_{operation}");
                let dash_marker = format!("-{operation}");
                let slash_marker = format!("/{operation}");
                action.label_key.ends_with(&format!("{marker}.label"))
                    || action.label_key.contains(&format!("{marker}."))
                    || action.label.eq_ignore_ascii_case(operation)
                    || action.endpoint_id.contains(&marker)
                    || action.endpoint_id.contains(&underscore_marker)
                    || action.endpoint_id.contains(&dash_marker)
                    || action.endpoint_id.contains(&slash_marker)
                    || action.endpoint_id.starts_with(&format!("{operation}_"))
                    || action.endpoint_id.starts_with(&format!("{operation}-"))
                    || action.endpoint_id.ends_with(operation)
                    || action.operation_id.contains(&marker)
                    || action.operation_id.contains(&underscore_marker)
                    || action.operation_id.contains(&dash_marker)
                    || action.operation_id.contains(&slash_marker)
                    || action.operation_id.starts_with(&format!("{operation}_"))
                    || action.operation_id.starts_with(&format!("{operation}-"))
                    || action.operation_id.ends_with(operation)
            })
    })
}

fn manager_endpoint_is_create_form(endpoint: &EndpointDefinition) -> bool {
    if matches!(endpoint.operation, OperationKind::Create) {
        return true;
    }
    if let OperationKind::Command(spec) = &endpoint.operation
        && (spec
            .kind
            .as_deref()
            .is_some_and(|kind| matches!(kind, "record-create" | "record_create"))
            || (spec
                .kind
                .as_deref()
                .is_some_and(|kind| matches!(kind, "record-mutation" | "record_mutation"))
                && spec
                    .steps
                    .iter()
                    .any(|step| command_step_creates_endpoint_record(step, endpoint))))
    {
        return true;
    }
    endpoint_id_matches_operation(endpoint, "create")
}

fn command_step_creates_endpoint_record(step: &CommandStep, endpoint: &EndpointDefinition) -> bool {
    match step {
        CommandStep::Create {
            entity, collection, ..
        } => {
            entity
                .as_deref()
                .is_some_and(|entity| endpoint.entity.as_deref() == Some(entity))
                || endpoint.collection == collection.as_deref().unwrap_or_default()
        }
        CommandStep::Foreach { steps, .. } => steps
            .iter()
            .any(|step| command_step_creates_endpoint_record(step, endpoint)),
        _ => false,
    }
}

fn endpoint_id_matches_operation(endpoint: &EndpointDefinition, operation: &str) -> bool {
    let marker = format!(".{operation}");
    let underscore_marker = format!("_{operation}");
    let dash_marker = format!("-{operation}");
    let slash_marker = format!("/{operation}");
    [&endpoint.endpoint_id, &endpoint.operation_id]
        .into_iter()
        .any(|value| {
            value.contains(&marker)
                || value.contains(&underscore_marker)
                || value.contains(&dash_marker)
                || value.contains(&slash_marker)
                || value.starts_with(&format!("{operation}_"))
                || value.starts_with(&format!("{operation}-"))
                || value.ends_with(operation)
        })
}

fn sort_manager_record_rows(rows: &mut [EntityRecord]) {
    rows.sort_by(|left, right| {
        manager_record_sort_key(right)
            .cmp(&manager_record_sort_key(left))
            .then_with(|| right.version.cmp(&left.version))
            .then_with(|| right.id.cmp(&left.id))
    });
}

fn manager_record_sort_key(record: &EntityRecord) -> String {
    [
        "updated_at",
        "updatedAt",
        "modified_at",
        "modifiedAt",
        "created_at",
        "createdAt",
        "submitted_at",
        "submittedAt",
        "_greentic_manager_created_at",
        "timestamp",
        "date",
    ]
    .iter()
    .find_map(|field| record.data.get(*field).map(manager_scalar_sort_value))
    .unwrap_or_default()
}

fn manager_scalar_has_sort_value(value: &Value) -> bool {
    !manager_scalar_sort_value(value).is_empty()
}

fn manager_scalar_sort_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        _ => String::new(),
    }
}

fn manager_record_matches_search(record: &EntityRecord, query: &str) -> bool {
    let query = query.to_ascii_lowercase();
    record.id.to_ascii_lowercase().contains(&query)
        || manager_value_contains_search(&record.data, &query)
}

fn manager_value_contains_search(value: &Value, query: &str) -> bool {
    match value {
        Value::String(value) => value.to_ascii_lowercase().contains(query),
        Value::Number(value) => value.to_string().to_ascii_lowercase().contains(query),
        Value::Bool(value) => value.to_string().contains(query),
        Value::Array(values) => values
            .iter()
            .any(|value| manager_value_contains_search(value, query)),
        Value::Object(values) => values
            .values()
            .any(|value| manager_value_contains_search(value, query)),
        Value::Null => false,
    }
}

fn request_path_without_query(path: &str) -> &str {
    path.split_once('?').map(|(path, _)| path).unwrap_or(path)
}

fn request_query_param(path: &str, name: &str) -> Option<String> {
    let (_, query) = path.split_once('?')?;
    for part in query.split('&') {
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        if percent_decode_query(key) == name {
            return Some(percent_decode_query(value));
        }
    }
    None
}

fn request_query_usize(path: &str, name: &str) -> Option<usize> {
    request_query_param(path, name)?.parse().ok()
}

fn percent_decode_query(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = &value[index + 1..index + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    index += 3;
                } else {
                    out.push(bytes[index]);
                    index += 1;
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode_query(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn localized_manager_static<'a>(locale: &str, text: &'a str) -> &'a str {
    match text {
        "Delete Icon" => return "X",
        "Edit Icon" => return "Edit",
        "Search Icon" => return "⌕ Search",
        _ => {}
    }
    let language = locale
        .split(['-', '_'])
        .next()
        .unwrap_or(locale)
        .to_ascii_lowercase();
    if language != "es" {
        return text;
    }
    match text {
        "Add" => "Anadir",
        "Actions" => "Acciones",
        "Back" => "Volver",
        "Dashboard" => "Panel",
        "Delete" => "Eliminar",
        "Delete Icon" => "X",
        "Delete this record" => "Eliminar este registro",
        "Edit" => "Editar",
        "Edit Icon" => "Editar",
        "Main Menu" => "Menu principal",
        "Metric" => "Metrica",
        "Metric not found." => "No se encontro la metrica.",
        "Metric query failed." => "Error al consultar la metrica.",
        "Metrics" => "Metricas",
        "Next" => "Siguiente",
        "No metric data found." => "No se encontraron datos de metrica.",
        "No metrics are declared." => "No se han declarado metricas.",
        "No records found." => "No se encontraron registros.",
        "of" => "de",
        "Previous" => "Anterior",
        "Search" => "Buscar",
        "Search Icon" => "⌕ Buscar",
        "Select a metric to inspect or query." => {
            "Selecciona una metrica para inspeccionar o consultar."
        }
        "Showing" => "Mostrando",
        "Rows" => "Filas",
        "Value" => "Valor",
        _ => text,
    }
}

fn is_datetime_field(value: Option<&str>) -> bool {
    matches!(
        value.unwrap_or_default().to_ascii_lowercase().as_str(),
        "datetime" | "timestamp"
    )
}

fn datetime_part_key(field_name: &str, part: &str) -> String {
    format!("{field_name}__sorx_{part}")
}

fn normalize_time_component(value: &str) -> String {
    if value.matches(':').count() == 1 {
        format!("{value}:00")
    } else {
        value.to_string()
    }
}

fn with_metric_tools(
    mut tools: McpToolList,
    metrics: Option<&RuntimeMetricCatalog>,
) -> McpToolList {
    if let Some(metrics) = metrics {
        tools.tools.push(metric_tool(
            "sorx.metrics.list",
            "List declared SORX metrics",
            "metrics.list",
        ));
        tools.tools.push(metric_tool(
            "sorx.metrics.get",
            "Get one SORX metric definition",
            "metrics.get",
        ));
        for metric in &metrics.metrics {
            tools.tools.push(metric_query_tool(metric));
        }
    }
    tools
}

fn metric_tool(
    name: impl Into<String>,
    description: impl Into<String>,
    operation_id: impl Into<String>,
) -> McpToolDefinition {
    let operation_id = operation_id.into();
    McpToolDefinition {
        name: name.into(),
        description: Some(description.into()),
        endpoint_id: operation_id.clone(),
        operation_id,
        risk: RiskLevel::Low,
        input_schema: Some(json!({ "type": "object" })),
    }
}

fn metric_query_tool(metric: &RuntimeMetric) -> McpToolDefinition {
    McpToolDefinition {
        name: format!("sorx.metrics.query.{}", metric.name),
        description: Some(format!("Query metric `{}`", metric.name)),
        endpoint_id: format!("metrics.query.{}", metric.name),
        operation_id: format!("metrics.query.{}", metric.name),
        risk: RiskLevel::Low,
        input_schema: Some(metric_query_tool_schema(metric)),
    }
}

fn metric_query_tool_schema(metric: &RuntimeMetric) -> Value {
    let dimension_names = metric
        .dimensions
        .iter()
        .map(|dimension| Value::String(dimension.name.clone()))
        .collect::<Vec<_>>();
    json!({
        "type": "object",
        "properties": {
            "dimensions": {
                "type": "array",
                "items": {
                    "type": "string",
                    "enum": dimension_names
                }
            },
            "filters": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["field", "operator", "value"],
                    "properties": {
                        "field": { "type": "string" },
                        "operator": {
                            "type": "string",
                            "enum": ["eq", "ne", "gt", "gte", "lt", "lte", "in"]
                        },
                        "value": {}
                    }
                }
            },
            "from": { "type": "string" },
            "to": { "type": "string" },
            "grain": { "type": "string" }
        }
    })
}

fn metric_summary_json(metric: &RuntimeMetric) -> Value {
    json!({
        "name": metric.name,
        "label": metric.label,
        "kind": match &metric.kind {
            RuntimeMetricKind::Aggregate { .. } => "aggregate",
            RuntimeMetricKind::Formula { .. } => "formula",
        },
        "dimensions": metric.dimensions,
        "cache": metric.cache
    })
}

fn metric_query_from_json(value: Value, namespace: ProviderNamespace) -> MetricQuery {
    let filters = value
        .get("filters")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|filter| {
                    Some(MetricQueryFilter {
                        field: filter.get("field")?.as_str()?.to_string(),
                        operator: filter
                            .get("operator")
                            .and_then(Value::as_str)
                            .map(normalize_metric_filter_operator)
                            .unwrap_or_else(|| "eq".to_string()),
                        value: filter.get("value").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    MetricQuery {
        namespace,
        from: value
            .get("from")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        to: value
            .get("to")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        grain: value
            .get("grain")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        dimensions: value
            .get("dimensions")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        filters,
    }
}

fn sensitive_requested_dimension(metric: &RuntimeMetric, query: &MetricQuery) -> Option<String> {
    metric
        .dimensions
        .iter()
        .find(|dimension| dimension.sensitive && query.dimensions.contains(&dimension.name))
        .map(|dimension| dimension.name.clone())
}

fn default_metric_dimensions(metric: &RuntimeMetric) -> Vec<String> {
    metric
        .dimensions
        .iter()
        .filter(|dimension| !dimension.sensitive)
        .map(|dimension| dimension.name.clone())
        .collect()
}

#[derive(Clone)]
struct StoreMetricProvider {
    providers: ProviderRegistry,
    entity_provider_ids: BTreeMap<String, String>,
    default_provider_id: String,
}

impl MetricRuntimeProvider for StoreMetricProvider {
    fn query_metric(
        &self,
        definition: &RuntimeMetric,
        query: &MetricQuery,
    ) -> SorxResult<MetricQueryResult> {
        let RuntimeMetricKind::Aggregate {
            source_entity,
            collection,
            aggregate,
            field,
        } = &definition.kind
        else {
            return Err(SorxError::new(
                "metric_kind_invalid",
                "store metric provider only supports aggregate metrics",
            ));
        };
        let provider_id = self
            .entity_provider_ids
            .get(source_entity)
            .unwrap_or(&self.default_provider_id);
        let provider = self.providers.store(provider_id)?;
        let filters = metric_query_filters(definition, query);
        let records = provider.query(QueryOp {
            namespace: query.namespace.clone(),
            entity: source_entity.clone(),
            collection: collection.clone(),
            filter: equality_filter(&filters),
            order_by: Vec::new(),
        })?;
        let requested_dimensions = query.dimensions.clone();
        let dimension_defs = requested_dimensions
            .iter()
            .map(|name| {
                definition
                    .dimensions
                    .iter()
                    .find(|dimension| dimension.name == *name)
                    .ok_or_else(|| {
                        SorxError::new(
                            "metric_dimension_unknown",
                            format!(
                                "metric `{}` does not define dimension `{name}`",
                                definition.name
                            ),
                        )
                    })
            })
            .collect::<SorxResult<Vec<_>>>()?;
        let mut groups: BTreeMap<String, (Vec<Value>, AggregateState)> = BTreeMap::new();
        for record in records
            .records
            .iter()
            .filter(|record| metric_filters_match(&record.data, &filters))
        {
            let key = dimension_defs
                .iter()
                .map(|dimension| {
                    field_value(
                        &record.data,
                        dimension.field.as_deref().unwrap_or(&dimension.name),
                    )
                    .cloned()
                    .unwrap_or(Value::Null)
                })
                .collect::<Vec<_>>();
            let group_key = serde_json::to_string(&key)
                .map_err(|err| SorxError::new("metric_group_key_failed", err.to_string()))?;
            groups
                .entry(group_key)
                .or_insert_with(|| (key, AggregateState::default()))
                .1
                .push(record_value(&record.data, field));
        }
        if groups.is_empty() && requested_dimensions.is_empty() {
            groups.insert("[]".to_string(), (Vec::new(), AggregateState::default()));
        }
        let rows = groups
            .into_iter()
            .map(|(_, (key, state))| {
                let dimensions = dimension_defs
                    .iter()
                    .zip(key)
                    .filter_map(|(dimension, value)| {
                        if value.is_null() {
                            None
                        } else {
                            Some((dimension.name.clone(), value))
                        }
                    })
                    .collect::<BTreeMap<_, _>>();
                Ok(MetricResultRow {
                    dimensions,
                    value: state.value(*aggregate),
                })
            })
            .collect::<SorxResult<Vec<_>>>()?;
        Ok(MetricQueryResult {
            metric: definition.name.clone(),
            rows,
        })
    }
}

fn metric_query_filters(definition: &RuntimeMetric, query: &MetricQuery) -> Vec<MetricQueryFilter> {
    definition
        .filters
        .iter()
        .cloned()
        .chain(query.filters.iter().cloned())
        .collect()
}

#[derive(Default)]
struct AggregateState {
    count: usize,
    sum: f64,
    numeric_count: usize,
    min: Option<f64>,
    max: Option<f64>,
    distinct: Vec<Value>,
}

impl AggregateState {
    fn push(&mut self, value: Option<&Value>) {
        self.count += 1;
        if let Some(value) = value {
            if !self.distinct.contains(value) {
                self.distinct.push(value.clone());
            }
            if let Some(number) = value.as_f64() {
                self.sum += number;
                self.numeric_count += 1;
                self.min = Some(
                    self.min
                        .map(|current| current.min(number))
                        .unwrap_or(number),
                );
                self.max = Some(
                    self.max
                        .map(|current| current.max(number))
                        .unwrap_or(number),
                );
            }
        }
    }

    fn value(&self, aggregate: MetricAggregate) -> f64 {
        match aggregate {
            MetricAggregate::Count => self.count as f64,
            MetricAggregate::Sum => self.sum,
            MetricAggregate::Avg => {
                if self.numeric_count == 0 {
                    0.0
                } else {
                    self.sum / self.numeric_count as f64
                }
            }
            MetricAggregate::Min => self.min.unwrap_or(0.0),
            MetricAggregate::Max => self.max.unwrap_or(0.0),
            MetricAggregate::DistinctCount => self.distinct.len() as f64,
        }
    }
}

fn equality_filter(filters: &[MetricQueryFilter]) -> Value {
    let mut filter = Map::new();
    for metric_filter in filters {
        if normalize_metric_filter_operator(&metric_filter.operator) == "eq"
            && !metric_filter.field.contains('.')
        {
            filter.insert(metric_filter.field.clone(), metric_filter.value.clone());
        }
    }
    Value::Object(filter)
}

fn metric_filters_match(data: &Value, filters: &[MetricQueryFilter]) -> bool {
    filters.iter().all(|filter| {
        let Some(actual) = field_value(data, &filter.field) else {
            return false;
        };
        match normalize_metric_filter_operator(&filter.operator).as_str() {
            "eq" => actual == &filter.value,
            "ne" => actual != &filter.value,
            "gt" => compare_numbers(actual, &filter.value, |left, right| left > right),
            "gte" => compare_numbers(actual, &filter.value, |left, right| left >= right),
            "lt" => compare_numbers(actual, &filter.value, |left, right| left < right),
            "lte" => compare_numbers(actual, &filter.value, |left, right| left <= right),
            "in" => filter
                .value
                .as_array()
                .is_some_and(|values| values.iter().any(|value| value == actual)),
            _ => false,
        }
    })
}

fn compare_numbers(
    actual: &Value,
    expected: &Value,
    compare: impl FnOnce(f64, f64) -> bool,
) -> bool {
    match (actual.as_f64(), expected.as_f64()) {
        (Some(actual), Some(expected)) => compare(actual, expected),
        _ => false,
    }
}

fn record_value<'a>(data: &'a Value, field: &Option<String>) -> Option<&'a Value> {
    field.as_deref().and_then(|field| field_value(data, field))
}

fn field_value<'a>(value: &'a Value, field: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in field.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

fn provider_registry(config: &SorxRuntimeConfig) -> SorxResult<ProviderRegistry> {
    let mut registry = ProviderRegistry::new();
    for (binding, provider) in &config.providers {
        match StoreProviderKind::parse(&provider.kind) {
            StoreProviderKind::Memory => {
                registry.register_canonical_store(binding, Arc::new(MemoryStoreProvider::new()))
            }
            StoreProviderKind::FoundationDb => {
                let adapter =
                    FoundationDbProviderAdapter::new(FoundationDbProviderConfig::from_parts(
                        provider.config_ref.clone(),
                        provider.config.clone(),
                    ))?;
                registry.register_canonical_store(binding, Arc::new(adapter));
            }
            StoreProviderKind::External(other) => {
                return Err(SorxError::new(
                    "provider_unsupported",
                    format!("provider `{binding}` has unsupported kind `{other}`"),
                ));
            }
        }
    }
    Ok(registry)
}

pub fn route_list(
    deployment_id: &str,
    exposure: &str,
    pack: &LoadedSorlaPack,
    router: &EndpointRouter,
    versions: &RouteVersionMetadata,
) -> RouteList {
    RouteList {
        schema: "greentic.sorx.routes.v1".to_string(),
        routes: router
            .endpoints
            .values()
            .map(|endpoint| RouteInfo {
                method: method_from_endpoint(endpoint.method).to_string(),
                path: endpoint.path.clone(),
                endpoint_id: endpoint.endpoint_id.clone(),
                operation_id: endpoint.operation_id.clone(),
                risk: format!("{:?}", endpoint.risk).to_ascii_lowercase(),
                deployment_id: deployment_id.to_string(),
                pack_name: pack.pack_name.clone(),
                pack_version: pack.pack_version.clone(),
                pack_digest: pack.pack_digest.clone(),
                api_version_label: versions.api_version_label.clone(),
                view_version: versions.view_version.clone(),
                canonical_version: versions.canonical_version.clone(),
                state_namespace: versions.state_namespace.clone(),
                exposure: exposure.to_string(),
            })
            .collect(),
    }
}

fn clean_route_segment(value: &str) -> String {
    let cleaned = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
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

fn business_action_json(action: &BusinessAction) -> Value {
    json!({
        "schema": "greentic.sorx.business-action.v1",
        "id": action.id,
        "version": action.version,
        "label": action.label,
        "description": action.description,
        "aliases": action.aliases,
        "contract_hash": contract_hash(action),
        "execution": action.execution,
        "input_schema": action.input_schema,
        "output_schema": action.output_schema,
        "risk": action.risk,
        "approval": action.approval,
        "idempotency": action.idempotency,
        "designer": action.designer
    })
}

fn action_ref_json(action: &BusinessAction, contract_hash: &str) -> Value {
    json!({
        "id": action.id,
        "version": action.version,
        "contract_hash": contract_hash
    })
}

fn capability_action_ref_json(
    action: &BusinessAction,
    locked_contract_hash: Option<String>,
) -> Value {
    json!({
        "id": action.id,
        "version": action.version,
        "contract_hash": locked_contract_hash.unwrap_or_else(|| contract_hash(action))
    })
}

fn execution_target_json(action: &BusinessAction, endpoint: &EndpointDefinition) -> Value {
    json!({
        "endpoint_id": endpoint.endpoint_id,
        "operation_id": endpoint.operation_id,
        "tool_name": action.execution.tool_name
    })
}

fn missing_action_code(assets: Option<&BusinessActionAssets>, id: &str) -> &'static str {
    if assets.is_some_and(|assets| assets.catalog.actions.iter().any(|action| action.id == id)) {
        "unknown_action_version"
    } else {
        "unknown_action"
    }
}

fn request_roles(headers: &BTreeMap<String, String>) -> Vec<String> {
    headers
        .get("x-greentic-caller-role")
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|role| !role.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|roles| !roles.is_empty())
        .unwrap_or_else(|| vec!["local".to_string()])
}

fn string_array(value: &Value) -> Option<Vec<String>> {
    Some(
        value
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect(),
    )
}

fn business_action_capability(pack_name: &str, action: &BusinessAction) -> String {
    format!(
        "cap://greentic/business-functions/{}/{}/v{}",
        clean_capability_segment(pack_name),
        clean_capability_segment(&action.id),
        clean_capability_segment(&action.version)
    )
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

fn collect_command_event_topics(
    steps: &[greentic_sorx_core::CommandStep],
    events: &mut Vec<String>,
) {
    for step in steps {
        match step {
            greentic_sorx_core::CommandStep::EmitEvent { event, .. } if !events.contains(event) => {
                events.push(event.clone());
            }
            greentic_sorx_core::CommandStep::Foreach { steps, .. } => {
                collect_command_event_topics(steps, events);
            }
            _ => {}
        }
    }
}

fn policy_decision_label(action: &PolicyAction) -> &'static str {
    match action {
        PolicyAction::Execute => "allow",
        PolicyAction::RequireApproval => "require_approval",
        PolicyAction::Deny => "deny",
    }
}

fn manager_submit_roles(body: &Map<String, Value>) -> Option<Vec<String>> {
    body.get("sorx_role")
        .or_else(|| body.get("role"))
        .and_then(Value::as_str)
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|role| !role.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|roles| !roles.is_empty())
}

fn validate_action_schema(schema: &Value, value: &Value) -> Result<(), String> {
    validate_action_schema_at(schema, value, "")
}

fn validate_action_schema_at(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
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
            return Err(format!("{} expected {expected}", display_json_path(path)));
        }
    }
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for key in required.iter().filter_map(Value::as_str) {
            if value.get(key).is_none() {
                return Err(format!("missing required input `{key}`"));
            }
        }
    }
    if schema.get("additionalProperties").and_then(Value::as_bool) == Some(false)
        && let Some(object) = value.as_object()
    {
        let known = schema
            .get("properties")
            .and_then(Value::as_object)
            .map(|properties| properties.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        for key in object.keys() {
            if !known.contains(key) {
                return Err(format!("unknown input field `{key}`"));
            }
        }
    }
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        for (key, child_schema) in properties {
            if let Some(child) = value.get(key) {
                validate_action_schema_at(child_schema, child, &join_json_path(path, key))?;
            }
        }
    }
    Ok(())
}

fn join_json_path(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_string()
    } else {
        format!("{parent}.{key}")
    }
}

fn display_json_path(path: &str) -> &str {
    if path.is_empty() { "$" } else { path }
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn valid_contract_hash_format(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn business_action_error(status: u16, code: &str, message: &str) -> HttpResponse {
    business_action_error_with_details(status, code, message, json!({}))
}

fn manager_model_submit_record_response(record: EntityRecord) -> HttpResponse {
    json_response(
        200,
        json!({
            "ok": true,
            "schema": "greentic.sorx.manager-submit-result.v1",
            "status": "completed",
            "result": {
                "id": record.id,
                "record": record.entity,
                "data": record.data
            },
            "events": []
        }),
    )
}

fn business_action_error_with_details(
    status: u16,
    code: &str,
    message: &str,
    details: Value,
) -> HttpResponse {
    json_response(
        status,
        json!({
            "ok": false,
            "error": {
                "code": code,
                "message": message,
                "details": details
            }
        }),
    )
}

fn method_from_endpoint(method: EndpointMethod) -> &'static str {
    match method {
        EndpointMethod::Get => "GET",
        EndpointMethod::Post => "POST",
        EndpointMethod::Put => "PUT",
        EndpointMethod::Patch => "PATCH",
        EndpointMethod::Delete => "DELETE",
    }
}

fn match_path(pattern: &str, path: &str) -> Option<BTreeMap<String, String>> {
    let pattern_parts = pattern.trim_matches('/').split('/').collect::<Vec<_>>();
    let path_parts = path.trim_matches('/').split('/').collect::<Vec<_>>();
    if pattern_parts.len() != path_parts.len() {
        return None;
    }
    let mut params = BTreeMap::new();
    for (pattern, actual) in pattern_parts.iter().zip(path_parts) {
        if pattern.starts_with('{') && pattern.ends_with('}') {
            let name = pattern.trim_start_matches('{').trim_end_matches('}');
            params.insert(name.to_string(), actual.to_string());
        } else if *pattern != actual {
            return None;
        }
    }
    Some(params)
}

fn header_or_local(
    headers: &BTreeMap<String, String>,
    name: &str,
    fallback: &str,
    environment: &str,
) -> SorxResult<String> {
    if let Some(value) = headers.get(name)
        && !value.is_empty()
    {
        return Ok(value.clone());
    }
    if environment == "local" {
        Ok(fallback.to_string())
    } else {
        Err(SorxError::new(
            "context_missing",
            format!("missing required header `{name}`"),
        ))
    }
}

/// Parses the self-asserted `x-greentic-caller-role` header into a role list,
/// falling back to `["local"]` when the header is absent or empty.
///
/// This is the legacy, caller-asserted role source. When the admin roles
/// overlay is configured these roles are deliberately ignored (see
/// [`compute_effective_roles`]).
fn header_caller_roles(headers: &BTreeMap<String, String>) -> Vec<String> {
    headers
        .get("x-greentic-caller-role")
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|role| !role.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|roles| !roles.is_empty())
        .unwrap_or_else(|| vec!["local".to_string()])
}

/// Decides the effective policy roles for a request.
///
/// - No overlay configured (the default): returns `header_roles` verbatim, so
///   behavior is byte-identical to the pre-overlay request path.
/// - Overlay configured: the admin system-of-record is authoritative and the
///   self-asserted `x-greentic-caller-role` header is IGNORED (this is the
///   whole point of the security gate — a caller must not be able to grant
///   itself roles). The overlay result maps to:
///   - `Some(non-empty)` => those admin-granted roles,
///   - `Some(empty)`     => the user is known but holds no roles => `["local"]`,
///   - `None`            => unresolved (no caller email, or admin unreachable
///     with no cached map) => `["local"]` (no asserted roles).
fn compute_effective_roles(
    overlay: Option<&AdminRolesOverlay>,
    tenant_slug: &str,
    caller_email: Option<&str>,
    header_roles: Vec<String>,
) -> Vec<String> {
    match overlay {
        None => header_roles,
        Some(overlay) => match overlay.roles_for(tenant_slug, caller_email) {
            Some(roles) if !roles.is_empty() => roles,
            Some(_) => vec!["local".to_string()],
            None => vec!["local".to_string()],
        },
    }
}

fn request_json(
    request: &HttpRequest,
    path_params: &BTreeMap<String, String>,
    endpoint: Option<&EndpointDefinition>,
) -> Result<Value, String> {
    let mut object = if request.body.trim().is_empty() {
        Map::new()
    } else {
        serde_json::from_str::<Value>(&request.body)
            .map_err(|err| err.to_string())?
            .as_object()
            .cloned()
            .ok_or_else(|| "request body must be a JSON object".to_string())?
    };
    for (key, value) in path_params {
        object.insert(key.clone(), coerce_path_param(endpoint, key, value));
    }
    if path_params.len() == 1
        && !object.contains_key("id")
        && let Some((_, value)) = path_params.iter().next()
    {
        object.insert("id".to_string(), coerce_path_param(endpoint, "id", value));
    }
    Ok(Value::Object(object))
}

fn coerce_path_param(endpoint: Option<&EndpointDefinition>, key: &str, value: &str) -> Value {
    let Some(schema) = endpoint.and_then(|endpoint| endpoint.input_schema.as_ref()) else {
        return Value::String(value.to_string());
    };
    let Some(type_name) = schema
        .get("properties")
        .and_then(|properties| properties.get(key))
        .and_then(|property| property.get("type"))
        .and_then(Value::as_str)
    else {
        return Value::String(value.to_string());
    };
    match type_name {
        "boolean" => match value {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => Value::String(value.to_string()),
        },
        "integer" => value
            .parse::<i64>()
            .ok()
            .map(|number| Value::Number(number.into()))
            .unwrap_or_else(|| Value::String(value.to_string())),
        "number" => value
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(value.to_string())),
        _ => Value::String(value.to_string()),
    }
}

#[derive(Debug, Clone)]
struct HttpRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: String,
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|err| err.to_string())?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| "missing HTTP method".to_string())?
        .to_string();
    let path = parts
        .next()
        .ok_or_else(|| "missing HTTP path".to_string())?
        .to_string();

    let mut headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).map_err(|err| err.to_string())?;
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(
                key.trim().to_ascii_lowercase(),
                value.trim().trim_end_matches('\r').to_string(),
            );
        }
    }
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0; content_length];
    reader
        .read_exact(&mut body)
        .map_err(|err| err.to_string())?;
    let body = String::from_utf8(body).map_err(|err| err.to_string())?;

    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

struct HttpResponse {
    status: u16,
    body: Value,
    headers: Vec<(String, String)>,
}

fn json_response(status: u16, body: Value) -> HttpResponse {
    HttpResponse { status, body, headers: Vec::new() }
}

fn error_response(status: u16, code: &str, message: &str) -> HttpResponse {
    json_response(
        status,
        json!({
            "ok": false,
            "error": {
                "code": code,
                "message": message,
                "details": {}
            }
        }),
    )
}

fn sorx_error_response(status: u16, err: SorxError) -> HttpResponse {
    json_response(
        status,
        json!({
            "ok": false,
            "error": {
                "code": err.code,
                "message": err.message,
                "details": {
                    "path": err.path
                }
            }
        }),
    )
}

impl HttpResponse {
    fn with_header(mut self, name: &str, value: &str) -> HttpResponse {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    fn as_bytes(&self) -> Vec<u8> {
        let body = serde_json::to_vec(&self.body).unwrap_or_else(|_| b"{}".to_vec());
        let reason = match self.status {
            200 => "OK",
            400 => "Bad Request",
            403 => "Forbidden",
            404 => "Not Found",
            _ => "Internal Server Error",
        };
        let mut head = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Accept, Accept-Language, X-Greentic-Tenant-Id, X-Greentic-Caller-Id, X-Greentic-Caller-Role, X-Greentic-Channel, X-Greentic-Locale, X-Greentic-Idempotency-Key\r\nContent-Length: {}\r\nConnection: close\r\n",
            self.status,
            reason,
            body.len()
        );
        for (name, value) in &self.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        let mut response = head.into_bytes();
        response.extend(body);
        response
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};

    use greentic_sorx_core::{
        AdminActionContext, AdminActionRequest, AdminActionResponse, AdminObserverEvent,
        ControlDecision, ControlHook, ManagerFieldView, ManagerNavItem, ManagerPolicyDecision,
        ManagerRecordView, ManagerViewModel, MemoryAuditSink, ObserverHook, default_start_schema,
        normalize_start_answers, runtime_config_from_answers,
    };
    use greentic_sorx_pack::{
        BusinessAction, BusinessActionAssets, BusinessActionCatalog, BusinessActionExecution,
        BusinessActionIdempotency, BusinessActionLock, BusinessActionLockEntry, BusinessActionRisk,
        LoadedSorlaPack, MetricAssets, MetricCatalog, OntologyAssets, OntologyConcept,
        OntologyGraph, OntologyRecordRef, OntologyRelationship, PackIdentity, PackManifest,
        SorlaAssets, SorxAssets, ValidationSuiteStatus, contract_hash,
    };
    use serde_json::{Value, json};

    use super::*;
    use crate::admin_roles::{AdminRolesOverlay, RolesResolver};
    use std::collections::HashMap as StdHashMap;
    use std::time::Duration;

    const GENERIC_RUNTIME_CONFIG: &str =
        include_str!("../tests/e2e/fixtures/generic_runtime_host/runtime-config.json");

    /// Minimal resolver returning a fixed map; used to exercise the seam.
    struct SeamResolver {
        map: StdHashMap<String, Vec<String>>,
    }

    impl RolesResolver for SeamResolver {
        fn tenant_user_roles(
            &self,
            _tenant_slug: &str,
        ) -> Result<StdHashMap<String, Vec<String>>, String> {
            Ok(self.map.clone())
        }
    }

    fn seam_overlay(pairs: &[(&str, &[&str])]) -> AdminRolesOverlay {
        let map = pairs
            .iter()
            .map(|(email, roles)| {
                (
                    email.to_ascii_lowercase(),
                    roles.iter().map(|r| r.to_string()).collect::<Vec<_>>(),
                )
            })
            .collect();
        AdminRolesOverlay::new(Box::new(SeamResolver { map }), Duration::from_secs(300))
    }

    #[test]
    fn manager_model_field_reads_presentation_hints() {
        use serde_json::json;
        let field = json!({
            "name": "email",
            "type": "string",
            "display_label": "Your Email",
            "hidden": true,
            "display_order": 3,
            "display_group": "Contact"
        });
        let result = manager_model_field("User", &field).unwrap();
        assert_eq!(result.label, "Your Email");
        assert!(result.hidden);
        assert_eq!(result.display_order, Some(3));
        assert_eq!(result.display_group.as_deref(), Some("Contact"));
    }

    #[test]
    fn manager_table_fields_excludes_hidden_field() {
        // Build a record with one hidden field and one visible field.  The table
        // path (manager_table_fields) must surface the visible field and silently
        // drop the hidden one.
        fn make_field(field_name: &str, is_hidden: bool) -> ManagerFieldView {
            ManagerFieldView {
                name: field_name.to_string(),
                label_key: format!("field.record.{field_name}.label"),
                label: field_name.to_string(),
                json_type: Some("string".to_string()),
                rules: None,
                generated: false,
                relationship: None,
                required: false,
                read_only: false,
                redacted: false,
                value: None,
                hidden: is_hidden,
                display_order: None,
                display_group: None,
                policy: ManagerPolicyDecision::allow(),
            }
        }

        let record = ManagerRecordView {
            record: "Item".to_string(),
            collection: "items".to_string(),
            label_key: "record.item.label".to_string(),
            label: "Item".to_string(),
            plural_label_key: "record.item.plural".to_string(),
            plural_label: "Items".to_string(),
            create_field_names: Vec::new(),
            fields: vec![make_field("secret_token", true), make_field("title", false)],
            endpoint_ids: Vec::new(),
            policy: ManagerPolicyDecision::allow(),
        };

        let table_fields = manager_table_fields(&record);
        let field_names: Vec<&str> = table_fields.iter().map(|f| f.name.as_str()).collect();

        assert!(
            !field_names.contains(&"secret_token"),
            "hidden field 'secret_token' must not appear in table fields; got: {field_names:?}"
        );
        assert!(
            field_names.contains(&"title"),
            "visible field 'title' must appear in table fields; got: {field_names:?}"
        );
    }

    #[test]
    fn header_caller_roles_parses_and_falls_back() {
        let mut headers = BTreeMap::new();
        headers.insert(
            "x-greentic-caller-role".to_string(),
            "admin, reviewer ,,".to_string(),
        );
        assert_eq!(
            header_caller_roles(&headers),
            vec!["admin".to_string(), "reviewer".to_string()]
        );
        // Absent header => ["local"].
        assert_eq!(
            header_caller_roles(&BTreeMap::new()),
            vec!["local".to_string()]
        );
    }

    #[test]
    fn compute_effective_roles_no_overlay_passthrough() {
        // No overlay => header roles are used verbatim (today's behavior).
        let header = vec!["admin".to_string()];
        assert_eq!(
            compute_effective_roles(None, "t", Some("alice@x"), header.clone()),
            header
        );
        // Including the ["local"] fallback when the header was absent.
        assert_eq!(
            compute_effective_roles(None, "t", None, vec!["local".to_string()]),
            vec!["local".to_string()]
        );
    }

    #[test]
    fn compute_effective_roles_overlay_ignores_header() {
        let overlay = seam_overlay(&[("alice@x", &["sorla_composer"])]);
        // Caller asserts "admin" via header, but the overlay is authoritative:
        // the header is ignored and the admin role is used.
        let roles = compute_effective_roles(
            Some(&overlay),
            "t",
            Some("alice@x"),
            vec!["admin".to_string()],
        );
        assert_eq!(roles, vec!["sorla_composer".to_string()]);
    }

    #[test]
    fn compute_effective_roles_overlay_unknown_user_local() {
        let overlay = seam_overlay(&[("alice@x", &["sorla_composer"])]);
        // Known tenant, unknown user (Some(empty)) => ["local"], header ignored.
        let roles = compute_effective_roles(
            Some(&overlay),
            "t",
            Some("nobody@x"),
            vec!["admin".to_string()],
        );
        assert_eq!(roles, vec!["local".to_string()]);
    }

    #[test]
    fn compute_effective_roles_overlay_no_email_local() {
        let overlay = seam_overlay(&[("alice@x", &["sorla_composer"])]);
        // No caller email (None) => ["local"], header still ignored.
        let roles = compute_effective_roles(Some(&overlay), "t", None, vec!["admin".to_string()]);
        assert_eq!(roles, vec!["local".to_string()]);
    }

    #[derive(Debug)]
    struct AdminDenyControl;

    impl ControlHook for AdminDenyControl {
        fn pre_admin(
            &self,
            _context: &AdminActionContext,
            _request: &AdminActionRequest,
        ) -> SorxResult<ControlDecision> {
            Ok(ControlDecision::deny("admin blocked"))
        }
    }

    #[derive(Debug)]
    struct AdminPostPatchControl;

    impl ControlHook for AdminPostPatchControl {
        fn post_admin(
            &self,
            _context: &AdminActionContext,
            _request: &AdminActionRequest,
            _response: &AdminActionResponse,
        ) -> SorxResult<ControlDecision> {
            Ok(ControlDecision::allow_with_patch(json!({
                "admin_pipeline": "patched"
            })))
        }
    }

    #[derive(Debug, Default)]
    struct AdminRecordingObserver {
        events: Mutex<Vec<String>>,
    }

    impl AdminRecordingObserver {
        fn events(&self) -> Vec<String> {
            self.events.lock().unwrap().clone()
        }
    }

    impl ObserverHook for AdminRecordingObserver {
        fn observe_admin(&self, event: &AdminObserverEvent) -> SorxResult<()> {
            self.events.lock().unwrap().push(event.event_type.clone());
            Ok(())
        }
    }

    fn gateway() -> Value {
        json!({
            "schema": "greentic.sorla.agent-gateway.v1",
            "endpoints": [
                {
                    "endpoint_id": "tenant.create",
                    "operation_id": "tenant.create",
                    "operation": "create",
                    "method": "POST",
                    "path": "/v1/agent/tenants/create",
                    "entity": "Tenant",
                    "collection": "tenants",
                    "provider_binding": "store",
                    "risk": "medium",
                    "input_schema": {
                        "type": "object",
                        "required": ["id", "name", "active"],
                        "properties": {
                            "id": { "type": "string" },
                            "name": { "type": "string" },
                            "active": { "type": "boolean" }
                        }
                    }
                },
                {
                    "endpoint_id": "tenant.get",
                    "operation_id": "tenant.get",
                    "operation": "get",
                    "method": "GET",
                    "path": "/v1/agent/tenants/{tenant_id}",
                    "entity": "Tenant",
                    "collection": "tenants",
                    "provider_binding": "store"
                },
                {
                    "endpoint_id": "tenant.update",
                    "operation_id": "tenant.update",
                    "operation": "update",
                    "method": "PATCH",
                    "path": "/v1/agent/tenants/{tenant_id}",
                    "entity": "Tenant",
                    "collection": "tenants",
                    "provider_binding": "store",
                    "risk": "low",
                    "input_schema": {
                        "type": "object",
                        "required": ["id", "patch"],
                        "properties": {
                            "id": { "type": "string" },
                            "patch": { "type": "object" }
                        }
                    }
                },
                {
                    "endpoint_id": "tenant.terminate",
                    "operation_id": "tenant.terminate",
                    "operation": "delete",
                    "method": "DELETE",
                    "path": "/v1/agent/tenants/{tenant_id}",
                    "entity": "Tenant",
                    "collection": "tenants",
                    "provider_binding": "store",
                    "risk": "high",
                    "approval": {
                        "required": true,
                        "roles": ["landlord_admin"],
                        "reason_required": true
                    }
                },
                {
                    "endpoint_id": "tenant.query",
                    "operation_id": "tenant.query",
                    "operation": "query",
                    "method": "POST",
                    "path": "/v1/agent/tenants/query",
                    "entity": "Tenant",
                    "collection": "tenants",
                    "provider_binding": "store"
                },
                {
                    "endpoint_id": "tenant.generate_code",
                    "operation_id": "tenant.generate_code",
                    "operation": "command",
                    "method": "POST",
                    "path": "/v1/agent/tenants/generate-code",
                    "entity": "Tenant",
                    "collection": "tenants",
                    "provider_binding": "store",
                    "risk": "low",
                    "input_schema": {
                        "type": "object",
                        "required": ["id"],
                        "properties": {
                            "id": { "type": "string" }
                        }
                    },
                    "command": {
                        "kind": "record_mutation",
                        "action": "generate_code",
                        "steps": [
                            {
                                "op": "find_one",
                                "as": "record",
                                "where": { "id": "$input.id" },
                                "required": true
                            },
                            {
                                "op": "update_where",
                                "as": "update",
                                "where": { "id": "$input.id" },
                                "set": {
                                    "code": {
                                        "coalesce": [
                                            "$steps.record.data.code",
                                            "$generated.short_code"
                                        ]
                                    }
                                }
                            },
                            {
                                "op": "emit_event",
                                "event": "tenant.code_generated",
                                "payload": {
                                    "tenant_id": "$input.id",
                                    "code": "$steps.update.records.0.data.code"
                                }
                            }
                        ],
                        "return": {
                            "id": "$input.id",
                            "code": "$steps.update.records.0.data.code"
                        }
                    }
                },
                {
                    "endpoint_id": "tenant.active_by_landlord",
                    "operation_id": "tenant.active_by_landlord",
                    "operation": "query",
                    "method": "GET",
                    "path": "/v1/agent/landlords/{landlord_id}/active-tenants/{active}",
                    "entity": "Tenant",
                    "collection": "tenants",
                    "provider_binding": "store",
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "landlord_id": { "type": "string" },
                            "active": { "type": "boolean" }
                        }
                    }
                }
            ]
        })
    }

    fn pack() -> LoadedSorlaPack {
        LoadedSorlaPack {
            pack_path: "landlord.gtpack".into(),
            pack_name: "landlord-tenant-sor".to_string(),
            pack_version: "0.1.0".to_string(),
            pack_digest: Some("sha256:test".to_string()),
            manifest: PackManifest {
                schema: "greentic.gtpack.manifest.sorla.v1".to_string(),
                pack: PackIdentity {
                    name: "landlord-tenant-sor".to_string(),
                    version: "0.1.0".to_string(),
                    kind: Some("application".to_string()),
                },
                extension: json!({ "extension": "greentic.sorx.runtime.v1" }),
                integrity: None,
                assets: Vec::new(),
            },
            lock: None,
            sorla_assets: SorlaAssets {
                model_cbor: Vec::new(),
                agent_gateway_json: gateway(),
                openapi_overlay_yaml: None,
                arazzo_yaml: None,
                mcp_tools_json: Some(json!({
                    "schema": "greentic.sorla.mcp-tools.v1",
                    "tools": [{"name": "tenant.create", "endpoint_id": "tenant.create"}]
                })),
                llms_txt_fragment: None,
                ontology: None,
                business_actions: Some(business_action_assets()),
                metrics: Some(metric_assets()),
                operational_indexes: None,
            },
            sorx_assets: SorxAssets {
                start_schema_json: default_start_schema(),
                start_questions_cbor: None,
                runtime_template_yaml: None,
                provider_bindings_template_yaml: None,
                validation_suite_cbor: None,
                validation_suite_json: None,
                validation_fixture_paths: Vec::new(),
                validation_fixtures_json: Default::default(),
                validation_openapi_expected_json: None,
            },
            validation_suite_status: ValidationSuiteStatus::Missing,
            entries: BTreeSet::new(),
            doctor_errors: Vec::new(),
            doctor_warnings: Vec::new(),
        }
    }

    fn business_action_assets() -> BusinessActionAssets {
        let action = BusinessAction {
            id: "record_rent_payment".to_string(),
            version: "0.1.0".to_string(),
            label: Some("Record rent payment".to_string()),
            description: None,
            aliases: vec!["rent paid".to_string()],
            execution: BusinessActionExecution {
                endpoint_id: Some("tenant.create".to_string()),
                operation_id: Some("tenant.create".to_string()),
                tool_name: None,
            },
            input_schema: Some(json!({
                "type": "object",
                "required": ["id", "name", "active"],
                "properties": {
                    "id": { "type": "string" },
                    "name": { "type": "string" },
                    "active": { "type": "boolean" }
                },
                "additionalProperties": false
            })),
            output_schema: Some(json!({ "type": "object" })),
            input_bindings: Vec::new(),
            risk: Some(BusinessActionRisk::Medium),
            approval: None,
            idempotency: Some(BusinessActionIdempotency { required: true }),
            designer: Some(json!({ "category": "payments" })),
            metadata: None,
        };
        let lock = BusinessActionLock {
            schema: "greentic.sorla.business-actions.lock.v1".to_string(),
            entries: vec![BusinessActionLockEntry {
                id: action.id.clone(),
                version: action.version.clone(),
                contract_hash: contract_hash(&action),
            }],
        };
        BusinessActionAssets {
            catalog: BusinessActionCatalog {
                schema: "greentic.sorla.business-actions.v1".to_string(),
                actions: vec![action],
            },
            lock: Some(lock),
            hashes_valid: true,
            execution_targets_valid: true,
        }
    }

    fn business_action_hash() -> String {
        business_action_assets()
            .lock
            .unwrap()
            .entries
            .first()
            .unwrap()
            .contract_hash
            .clone()
    }

    fn metric_assets() -> MetricAssets {
        let catalog_json = json!({
            "schema": "greentic.sorla.metrics.v1",
            "package": { "name": "landlord-tenant-sor", "version": "0.1.0" },
            "metrics": [
                {
                    "name": "daily_clicks",
                    "source": { "entity": "Click", "collection": "clicks" },
                    "measure": { "aggregate": "count" },
                    "dimensions": [{ "name": "user_email", "field": "user_email", "sensitive": true }],
                    "time": { "field": "clicked_at", "grains": ["day"] }
                },
                {
                    "name": "monthly_revenue",
                    "source": { "entity": "Payment", "collection": "payments" },
                    "measure": { "aggregate": "sum", "field": "amount" },
                    "time": { "field": "paid_at", "grains": ["month"] }
                },
                {
                    "name": "monthly_cost",
                    "source": { "entity": "Cost", "collection": "costs" },
                    "measure": { "aggregate": "sum", "field": "amount" },
                    "time": { "field": "incurred_at", "grains": ["month"] }
                },
                {
                    "name": "gross_margin",
                    "formula": {
                        "expression": "monthly_revenue - monthly_cost",
                        "dependencies": ["monthly_revenue", "monthly_cost"]
                    }
                },
                {
                    "name": "lab_click_rate",
                    "dimensions": [{ "name": "lab_id", "field": "lab_id" }],
                    "formula": {
                        "expression": "daily_clicks / number_in_waiting_list",
                        "dependencies": ["daily_clicks", "number_in_waiting_list"]
                    }
                },
                {
                    "name": "number_in_waiting_list",
                    "source": { "entity": "waiting_list_entry", "collection": "waiting_list_entries" },
                    "measure": { "aggregate": "count" },
                    "dimensions": [{ "name": "lab_id", "field": "lab_id" }]
                }
            ]
        });
        MetricAssets {
            catalog: serde_json::from_value::<MetricCatalog>(catalog_json.clone()).unwrap(),
            catalog_json,
        }
    }

    fn answers(environment: &str) -> Value {
        json!({
            "tenant": { "tenant_id": "tenant-a", "environment": environment },
            "server": {
                "bind": "127.0.0.1:0",
                "public_base_url": "http://127.0.0.1:0"
            },
            "providers": {
                "store": {
                    "kind": "memory",
                    "config_ref": "providers.memory.local"
                }
            },
            "policy": { "approvals": {} },
            "audit": {},
            "deployment": {
                "tenant_id": "tenant-a",
                "sor_name": "landlord",
                "environment": environment
            },
            "exposure": {},
            "ghcr": {}
        })
    }

    fn runtime(environment: &str) -> HttpRuntime {
        runtime_with_answers(answers(environment))
    }

    fn runtime_with_answers(answers: Value) -> HttpRuntime {
        let pack = pack();
        let normalized = normalize_start_answers(&default_start_schema(), &answers, true).unwrap();
        let config = runtime_config_from_answers(&pack.pack_name, &normalized.answers).unwrap();
        HttpRuntime::from_pack("local", &pack, config).unwrap()
    }

    fn runtime_with_gateway(gateway: Value) -> HttpRuntime {
        let mut pack = pack();
        pack.pack_name = "generic-manager-sor".to_string();
        pack.manifest.pack.name = "generic-manager-sor".to_string();
        pack.sorla_assets.agent_gateway_json = gateway;
        pack.sorla_assets.mcp_tools_json = None;
        pack.sorla_assets.business_actions = None;
        pack.sorla_assets.metrics = None;
        let normalized =
            normalize_start_answers(&default_start_schema(), &answers("local"), true).unwrap();
        let config = runtime_config_from_answers(&pack.pack_name, &normalized.answers).unwrap();
        HttpRuntime::from_pack("local", &pack, config).unwrap()
    }

    fn runtime_with_model(model: Value) -> HttpRuntime {
        let mut pack = pack();
        pack.sorla_assets.model_cbor = encode_cbor(&model);
        pack.sorla_assets.mcp_tools_json = None;
        pack.sorla_assets.business_actions = None;
        pack.sorla_assets.metrics = None;
        let normalized =
            normalize_start_answers(&default_start_schema(), &answers("local"), true).unwrap();
        let config = runtime_config_from_answers(&pack.pack_name, &normalized.answers).unwrap();
        HttpRuntime::from_pack("local", &pack, config).unwrap()
    }

    fn runtime_with_gateway_and_model(gateway: Value, model: Value) -> HttpRuntime {
        let mut pack = pack();
        pack.pack_name = "generic-manager-sor".to_string();
        pack.manifest.pack.name = "generic-manager-sor".to_string();
        pack.sorla_assets.agent_gateway_json = gateway;
        pack.sorla_assets.model_cbor = encode_cbor(&model);
        pack.sorla_assets.mcp_tools_json = None;
        pack.sorla_assets.business_actions = None;
        pack.sorla_assets.metrics = None;
        let normalized =
            normalize_start_answers(&default_start_schema(), &answers("local"), true).unwrap();
        let config = runtime_config_from_answers(&pack.pack_name, &normalized.answers).unwrap();
        HttpRuntime::from_pack("local", &pack, config).unwrap()
    }

    fn runtime_with_gateway_and_ontology(gateway: Value, ontology: OntologyAssets) -> HttpRuntime {
        let mut pack = pack();
        pack.pack_name = "generic-manager-sor".to_string();
        pack.manifest.pack.name = "generic-manager-sor".to_string();
        pack.sorla_assets.agent_gateway_json = gateway;
        pack.sorla_assets.ontology = Some(ontology);
        pack.sorla_assets.mcp_tools_json = None;
        pack.sorla_assets.business_actions = None;
        pack.sorla_assets.metrics = None;
        let normalized =
            normalize_start_answers(&default_start_schema(), &answers("local"), true).unwrap();
        let config = runtime_config_from_answers(&pack.pack_name, &normalized.answers).unwrap();
        HttpRuntime::from_pack("local", &pack, config).unwrap()
    }

    fn runtime_with_initial_runtime_config(runtime_config: RuntimeConfig) -> HttpRuntime {
        let pack = pack();
        let normalized =
            normalize_start_answers(&default_start_schema(), &answers("local"), true).unwrap();
        let config = runtime_config_from_answers(&pack.pack_name, &normalized.answers).unwrap();
        HttpRuntime::from_pack_with_runtime_config("local", &pack, config, Some(runtime_config))
            .unwrap()
    }

    fn with_admin_control(mut runtime: HttpRuntime, control: Arc<dyn ControlHook>) -> HttpRuntime {
        runtime.runtime = Arc::new((*runtime.runtime).clone().with_control_hook(control));
        runtime
    }

    fn with_admin_observer(
        mut runtime: HttpRuntime,
        observer: Arc<dyn ObserverHook>,
    ) -> HttpRuntime {
        runtime.runtime = Arc::new(
            (*runtime.runtime)
                .clone()
                .with_observer_hook(observer, true),
        );
        runtime
    }

    fn with_audit_sink(mut runtime: HttpRuntime, audit: Arc<MemoryAuditSink>) -> HttpRuntime {
        runtime.runtime = Arc::new((*runtime.runtime).clone().with_audit_sink(audit));
        runtime
    }

    fn response(
        runtime: &HttpRuntime,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> HttpResponse {
        runtime.handle_request(HttpRequest {
            method: method.to_string(),
            path: path.to_string(),
            headers: headers
                .iter()
                .map(|(key, value)| (key.to_ascii_lowercase(), (*value).to_string()))
                .collect(),
            body: body.to_string(),
        })
    }

    fn request(
        runtime: &HttpRuntime,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Value {
        response(runtime, method, path, headers, body).body
    }

    fn tenant_headers() -> [(&'static str, &'static str); 2] {
        [
            ("X-Greentic-Tenant-Id", "tenant-a"),
            ("X-Greentic-Caller-Id", "tester"),
        ]
    }

    fn card_contains_text(value: &Value, needle: &str) -> bool {
        match value {
            Value::String(value) => value.contains(needle),
            Value::Array(values) => values.iter().any(|value| card_contains_text(value, needle)),
            Value::Object(values) => values
                .values()
                .any(|value| card_contains_text(value, needle)),
            _ => false,
        }
    }

    fn count_card_text(value: &Value, needle: &str) -> usize {
        match value {
            Value::String(value) => usize::from(value.contains(needle)),
            Value::Array(values) => values
                .iter()
                .map(|value| count_card_text(value, needle))
                .sum(),
            Value::Object(values) => values
                .values()
                .map(|value| count_card_text(value, needle))
                .sum(),
            _ => 0,
        }
    }

    fn card_has_action_title(value: &Value, title: &str) -> bool {
        match value {
            Value::Object(values) => {
                values.get("type").and_then(Value::as_str) == Some("Action.Submit")
                    && values.get("title").and_then(Value::as_str) == Some(title)
                    || values
                        .values()
                        .any(|value| card_has_action_title(value, title))
            }
            Value::Array(values) => values
                .iter()
                .any(|value| card_has_action_title(value, title)),
            _ => false,
        }
    }

    fn encode_cbor(value: &Value) -> Vec<u8> {
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(value, &mut bytes).unwrap();
        bytes
    }

    #[test]
    fn route_list_includes_deployment_and_pack_identity() {
        let runtime = runtime("local");
        let route = runtime
            .route_list()
            .routes
            .iter()
            .find(|route| route.endpoint_id == "tenant.create")
            .unwrap();
        assert_eq!(route.method, "POST");
        assert_eq!(route.path, "/v1/agent/tenants/create");
        assert_eq!(route.deployment_id, "local");
        assert_eq!(route.pack_name, "landlord-tenant-sor");
        assert_eq!(route.pack_version, "0.1.0");
        assert_eq!(route.pack_digest.as_deref(), Some("sha256:test"));
        assert_eq!(route.api_version_label, "local");
        assert_eq!(route.view_version, "local");
        assert_eq!(route.canonical_version, "0.1.0");
        assert_eq!(route.state_namespace, "sorx/tenant-a/landlord");
    }

    #[test]
    fn health_ready_and_routes_are_served() {
        let runtime = runtime("local");
        assert_eq!(
            request(&runtime, "GET", "/healthz", &[], "")["ok"],
            Value::Bool(true)
        );
        assert_eq!(
            request(&runtime, "GET", "/readyz", &[], "")["ok"],
            Value::Bool(true)
        );
        let routes = request(&runtime, "GET", "/v1/sorx/routes", &[], "");
        assert_eq!(routes["schema"], "greentic.sorx.routes.v1");
        assert_eq!(routes["routes"][0]["deployment_id"], "local");
        assert_eq!(routes["routes"][0]["api_version_label"], "local");
        assert_eq!(routes["routes"][0]["canonical_version"], "0.1.0");
        let public_routes = request(&runtime, "GET", "/v1/sorx/public-routes", &[], "");
        assert_eq!(public_routes["schema"], "greentic.sorx.public-routes.v1");
        assert_eq!(public_routes["routes"].as_array().unwrap().len(), 0);
        let tools = request(&runtime, "GET", "/v1/sorx/tools", &[], "");
        assert_eq!(tools["schema"], "greentic.sorx.tools.v1");
        assert_eq!(tools["tools"][0]["name"], "tenant.create");
        let deployment_routes = request(
            &runtime,
            "GET",
            "/v1/sorx/deployments/local/routes",
            &[],
            "",
        );
        assert_eq!(
            deployment_routes["routes"][0]["pack_name"],
            "landlord-tenant-sor"
        );
        let promotion_status = request(
            &runtime,
            "GET",
            "/v1/sorx/deployments/local/promotion-status",
            &[],
            "",
        );
        assert_eq!(
            promotion_status["schema"],
            "greentic.sorx.promotion-status.v1"
        );
        assert_eq!(promotion_status["registry_backed"], false);
    }

    #[test]
    fn model_authorization_controls_http_endpoint_and_record_access() {
        let runtime = runtime_with_model(json!({
            "roles": [
                { "id": "leasing-agent", "label": "Leasing agent" }
            ],
            "agent_endpoints": [
                {
                    "endpoint_id": "tenant.create",
                    "authorization": {
                        "roles": { "any_of": ["leasing-agent"] },
                        "policies": ["tenant.create"],
                        "conditions": { "environment": "local" }
                    }
                }
            ],
            "records": [
                {
                    "name": "Tenant",
                    "access": {
                        "create": {
                            "roles": { "all_of": ["leasing-agent"] },
                            "policies": ["tenant.write"]
                        }
                    }
                }
            ]
        }));

        let denied = response(
            &runtime,
            "POST",
            "/v1/agent/tenants/create",
            &tenant_headers(),
            r#"{"id":"tenant-auth-1","name":"Acme","active":true}"#,
        );
        assert_eq!(denied.status, 403);
        assert_eq!(denied.body["error"]["details"]["authorization"], "denied");

        let mut headers = tenant_headers().to_vec();
        headers.push(("X-Greentic-Caller-Role", "leasing-agent"));
        let allowed = response(
            &runtime,
            "POST",
            "/v1/agent/tenants/create",
            &headers,
            r#"{"id":"tenant-auth-2","name":"Acme","active":true}"#,
        );
        assert_eq!(allowed.status, 200);
        assert_eq!(allowed.body["ok"], true);
        assert_eq!(allowed.body["result"]["id"], "tenant-auth-2");
    }

    #[test]
    fn manager_view_hides_records_without_authorized_actions_for_role() {
        let runtime = runtime_with_model(json!({
            "roles": [
                { "id": "leasing-agent", "label": "Leasing agent" }
            ],
            "records": [
                {
                    "name": "Tenant",
                    "access": {
                        "read": { "roles": { "any_of": ["leasing-agent"] } },
                        "create": { "roles": { "any_of": ["leasing-agent"] } },
                        "update": { "roles": { "any_of": ["leasing-agent"] } },
                        "delete": { "roles": { "any_of": ["leasing-agent"] } }
                    }
                }
            ]
        }));

        let hidden = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/view",
            &tenant_headers(),
            "",
        );
        assert!(
            hidden["records"]
                .as_array()
                .unwrap()
                .iter()
                .all(|record| record["record"] != "Tenant")
        );

        let mut headers = tenant_headers().to_vec();
        headers.push(("X-Greentic-Caller-Role", "leasing-agent"));
        let visible = request(&runtime, "GET", "/v1/sorx/manager/view", &headers, "");
        assert!(
            visible["records"]
                .as_array()
                .unwrap()
                .iter()
                .any(|record| record["record"] == "Tenant")
        );
    }

    #[test]
    fn manager_submit_uses_carried_role_from_webchat_action_data() {
        let runtime = runtime_with_model(json!({
            "roles": [
                { "id": "leasing-agent", "label": "Leasing agent" }
            ],
            "records": [
                {
                    "name": "Tenant",
                    "access": {
                        "create": { "roles": { "any_of": ["leasing-agent"] } }
                    }
                }
            ]
        }));

        let denied = response(
            &runtime,
            "POST",
            "/v1/sorx/manager/submit",
            &tenant_headers(),
            r#"{
                "endpoint_id": "tenant.create",
                "operation_id": "tenant.create",
                "input": {"id":"tenant-role-denied","name":"Acme","active":true}
            }"#,
        );
        assert_eq!(denied.status, 403);

        let allowed = response(
            &runtime,
            "POST",
            "/v1/sorx/manager/submit",
            &tenant_headers(),
            r#"{
                "endpoint_id": "tenant.create",
                "operation_id": "tenant.create",
                "sorx_role": "leasing-agent",
                "input": {"id":"tenant-role-allowed","name":"Acme","active":true}
            }"#,
        );
        assert_eq!(allowed.status, 200);
        assert_eq!(allowed.body["result"]["id"], "tenant-role-allowed");
    }

    #[test]
    fn manager_submit_create_rows_sort_before_older_rows_without_user_timestamps() {
        let runtime = runtime("local");
        let first = response(
            &runtime,
            "POST",
            "/v1/sorx/manager/submit",
            &tenant_headers(),
            r#"{
                "endpoint_id": "tenant.create",
                "operation_id": "tenant.create",
                "input": {"id":"tenant-z","name":"First Tenant","active":true}
            }"#,
        );
        assert_eq!(first.status, 200);

        std::thread::sleep(std::time::Duration::from_millis(1));

        let second = response(
            &runtime,
            "POST",
            "/v1/sorx/manager/submit",
            &tenant_headers(),
            r#"{
                "endpoint_id": "tenant.create",
                "operation_id": "tenant.create",
                "input": {"id":"tenant-a","name":"Second Tenant","active":true}
            }"#,
        );
        assert_eq!(second.status, 200);

        let card = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/Tenant",
            &tenant_headers(),
            "",
        );
        let encoded = serde_json::to_string(&card).unwrap();
        let second_position = encoded.find("Second Tenant").unwrap();
        let first_position = encoded.find("First Tenant").unwrap();
        assert!(second_position < first_position);
    }

    #[test]
    fn manager_submit_accepts_webchat_top_level_input_fields() {
        let runtime = runtime("local");
        let submitted = response(
            &runtime,
            "POST",
            "/v1/sorx/manager/submit",
            &tenant_headers(),
            r#"{
                "endpoint_id": "tenant.create",
                "operation_id": "tenant.create",
                "record": "Tenant",
                "id": "tenant-top-level",
                "name": "Top Level Tenant",
                "active": true
            }"#,
        );
        assert_eq!(submitted.status, 200);
        assert_eq!(submitted.body["result"]["data"]["name"], "Top Level Tenant");
        assert_eq!(submitted.body["result"]["data"]["active"], true);

        let card = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/Tenant",
            &tenant_headers(),
            "",
        );
        assert!(card_contains_text(&card, "Top Level Tenant"));
    }

    #[test]
    fn manager_submit_record_create_command_persists_webchat_fields() {
        let runtime = runtime_with_gateway(json!({
            "schema": "greentic.sorla.agent-gateway.v1",
            "endpoints": [
                {
                    "endpoint_id": "create_landlord",
                    "operation_id": "create_landlord",
                    "operation": "command",
                    "command": { "kind": "record-create", "record": "Landlord" },
                    "method": "POST",
                    "path": "/v1/agent/landlords/create",
                    "entity": "Landlord",
                    "collection": "landlords",
                    "provider_binding": "store",
                    "risk": "low",
                    "input_schema": {
                        "type": "object",
                        "required": ["email", "full_name"],
                        "properties": {
                            "email": { "type": "email" },
                            "full_name": { "type": "string" }
                        }
                    }
                }
            ]
        }));

        let submitted = response(
            &runtime,
            "POST",
            "/v1/sorx/manager/submit",
            &tenant_headers(),
            r#"{
                "endpoint_id": "create_landlord",
                "operation_id": "create_landlord",
                "record": "Landlord",
                "email": "webchat-landlord@example.com",
                "full_name": "WebChat Landlord"
            }"#,
        );
        assert_eq!(submitted.status, 200);
        assert_eq!(submitted.body["result"]["result"]["created_count"], 1);

        let card = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/Landlord",
            &tenant_headers(),
            "",
        );
        assert!(card_contains_text(&card, "webchat-landlord@example.com"));
        assert!(card_contains_text(&card, "WebChat Landlord"));
    }

    #[test]
    fn manager_routes_serve_view_cards_graph_and_pickers() {
        let runtime = runtime("local");
        let mut headers = tenant_headers().to_vec();
        headers.push(("X-Greentic-Locale", "es-ES"));

        let view = request(&runtime, "GET", "/v1/sorx/manager/view", &headers, "");
        assert_eq!(view["schema"], "greentic.sorx.manager-view.v1");
        assert_eq!(view["locale"], "es-ES");
        assert_eq!(view["records"][0]["record"], "Tenant");
        assert_eq!(view["records"][0]["label"], "Inquilino");

        let dashboard = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/dashboard",
            &headers,
            "",
        );
        assert_eq!(dashboard["type"], "AdaptiveCard");
        assert_eq!(dashboard["lang"], "es-ES");
        assert_eq!(dashboard["metadata"]["locale"], "es-ES");
        assert_eq!(dashboard["actions"][0]["title"], "Inquilinos");
        assert_eq!(
            dashboard["metadata"]["schema"],
            "greentic.sorx.manager-card.v1"
        );

        let create_card = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/Tenant/create",
            &headers,
            "",
        );
        assert_eq!(create_card["metadata"]["kind"], "manager.record.create");

        let graph = request(&runtime, "GET", "/v1/sorx/manager/graph.json", &headers, "");
        assert_eq!(graph["schema"], "greentic.sorx.manager-graph.v1");
        assert!(
            graph["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|node| node["id"] == "Tenant")
        );

        let picker = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/pickers/Tenant",
            &headers,
            "",
        );
        assert_eq!(picker["schema"], "greentic.sorx.manager-picker.v1");
        assert_eq!(picker["tenant_id"], "tenant-a");

        let alias = request(&runtime, "GET", "/manager", &headers, "");
        assert_eq!(alias["schema"], "greentic.sorx.manager-shell.v1");
        let alias_view = request(&runtime, "GET", "/manager/view", &headers, "");
        assert_eq!(alias_view["schema"], "greentic.sorx.manager-view.v1");
    }

    #[test]
    fn manager_uses_record_hierarchy_and_endpoint_scoped_create_schema() {
        let runtime = runtime_with_gateway(json!({
            "schema": "greentic.sorla.agent-gateway.v1",
            "record_hierarchy": [
                { "record": "Building", "main": true },
                { "record": "Unit", "main": false, "parent": "Building" }
            ],
            "endpoints": [
                {
                    "endpoint_id": "building.create",
                    "operation_id": "building.create",
                    "operation": "create",
                    "method": "POST",
                    "path": "/v1/agent/buildings/create",
                    "entity": "Building",
                    "collection": "buildings",
                    "provider_binding": "store",
                    "input_schema": {
                        "type": "object",
                        "required": ["address"],
                        "properties": {
                            "address": { "type": "string" }
                        }
                    }
                },
                {
                    "endpoint_id": "building.patch-record",
                    "operation_id": "building.patch-record",
                    "operation": "update",
                    "method": "PATCH",
                    "path": "/v1/agent/records/{record_id}",
                    "entity": "Building",
                    "collection": "buildings",
                    "provider_binding": "store",
                    "execution": { "record_selector": { "record": "Building" } },
                    "input_schema": {
                        "type": "object",
                        "required": ["record_id", "patch_json", "reason"],
                        "properties": {
                            "record_id": { "type": "string" },
                            "patch_json": { "type": "object" },
                            "reason": { "type": "string" }
                        }
                    }
                },
                {
                    "endpoint_id": "unit.create",
                    "operation_id": "unit.create",
                    "operation": "create",
                    "method": "POST",
                    "path": "/v1/agent/units/create",
                    "entity": "Unit",
                    "collection": "units",
                    "provider_binding": "store",
                    "input_schema": {
                        "type": "object",
                        "required": ["building_id", "address"],
                        "properties": {
                            "building_id": { "type": "string", "format": "uuid" },
                            "address": { "type": "string" }
                        }
                    }
                }
            ]
        }));
        let headers = tenant_headers();

        let dashboard = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/dashboard",
            &headers,
            "",
        );
        assert!(card_has_action_title(&dashboard, "Buildings"));
        assert!(!card_has_action_title(&dashboard, "Units"));

        let building = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/Building",
            &headers,
            "",
        );
        assert!(!card_has_action_title(&building, "Units"));

        let provider = runtime.runtime.providers.store("store").unwrap();
        provider
            .create(greentic_sorx_core::CreateOp {
                namespace: ProviderNamespace {
                    tenant_id: "tenant-a".to_string(),
                    sor_name: runtime.runtime.config.deployment.sor_name.clone(),
                },
                entity: "Building".to_string(),
                collection: "buildings".to_string(),
                input: json!({ "id": "building-1", "address": "1 Main Street" }),
                idempotency_key: None,
                unique_indexes: Vec::new(),
                unique_behavior: greentic_sorx_core::UniqueConflictBehavior::Reject,
            })
            .unwrap();
        provider
            .create(greentic_sorx_core::CreateOp {
                namespace: ProviderNamespace {
                    tenant_id: "tenant-a".to_string(),
                    sor_name: runtime.runtime.config.deployment.sor_name.clone(),
                },
                entity: "Unit".to_string(),
                collection: "units".to_string(),
                input: json!({
                    "id": "unit-1",
                    "building_id": "building-1",
                    "address": "Unit 1"
                }),
                idempotency_key: None,
                unique_indexes: Vec::new(),
                unique_behavior: greentic_sorx_core::UniqueConflictBehavior::Reject,
            })
            .unwrap();
        provider
            .create(greentic_sorx_core::CreateOp {
                namespace: ProviderNamespace {
                    tenant_id: "tenant-a".to_string(),
                    sor_name: runtime.runtime.config.deployment.sor_name.clone(),
                },
                entity: "Unit".to_string(),
                collection: "units".to_string(),
                input: json!({
                    "id": "unit-2",
                    "building_id": "building-2",
                    "address": "Unit 2"
                }),
                idempotency_key: None,
                unique_indexes: Vec::new(),
                unique_behavior: greentic_sorx_core::UniqueConflictBehavior::Reject,
            })
            .unwrap();
        let building_detail = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/Building/building-1",
            &headers,
            "",
        );
        assert!(card_contains_text(&building_detail, "Units"));
        assert!(card_contains_text(&building_detail, "Unit 1"));
        assert!(!card_contains_text(&building_detail, "Unit 2"));
        let child_units = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/Unit?parent_record=Building&parent_id=building-1",
            &headers,
            "",
        );
        assert_eq!(child_units["metadata"]["total"], 1);
        assert!(card_contains_text(&child_units, "Unit 1"));
        assert!(!card_contains_text(&child_units, "Unit 2"));
        let child_create = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/Unit/create?parent_record=Building&parent_id=building-1",
            &headers,
            "",
        );
        assert!(card_contains_text(&child_create, "Address"));
        assert!(!card_contains_text(&child_create, "Building Id"));
        assert!(!card_contains_text(&child_create, "Search Building"));
        assert!(
            serde_json::to_string(&child_create)
                .unwrap()
                .contains("\"building_id\":\"building-1\"")
        );

        let create = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/Building/create",
            &headers,
            "",
        );
        assert!(card_contains_text(&create, "Address"));
        assert!(!card_contains_text(&create, "Patch Json"));
        assert!(!card_contains_text(&create, "Record Id"));
    }

    #[test]
    fn manager_hierarchy_can_use_model_only_parent_records() {
        let runtime = runtime_with_gateway_and_model(
            json!({
                "schema": "greentic.sorla.agent-gateway.v1",
                "record_hierarchy": {
                    "Lab": { "main": true },
                    "waiting_list_entry": { "parent": "Lab", "field": "lab_id" }
                },
                "endpoints": [
                    {
                        "endpoint_id": "join_waiting_list",
                        "operation_id": "join_waiting_list",
                        "operation": "command",
                        "method": "POST",
                        "path": "/v1/agent/waiting_list_entries/join_waiting_list",
                        "entity": "waiting_list_entry",
                        "collection": "waiting_list_entries",
                        "provider_binding": "store",
                        "input_schema": {
                            "type": "object",
                            "required": ["lab_id", "email", "name"],
                            "properties": {
                                "lab_id": { "type": "uuid" },
                                "email": { "type": "email" },
                                "name": { "type": "string" },
                                "invited_by_code": { "type": "string" }
                            }
                        },
                        "command": {
                            "kind": "record_mutation",
                            "action": "join_waiting_list",
                            "steps": [
                                {
                                    "op": "create",
                                    "as": "entry",
                                    "entity": "waiting_list_entry",
                                    "collection": "waiting_list_entries",
                                    "input": {
                                        "lab_id": "$input.lab_id",
                                        "email": "$input.email",
                                        "name": "$input.name"
                                    }
                                }
                            ]
                        }
                    }
                ]
            }),
            json!({
                "records": [
                    {
                        "name": "Lab",
                        "fields": [
                            { "name": "lab_id", "type": "uuid", "sensitive": false },
                            { "name": "name", "type": "string", "sensitive": false }
                        ]
                    },
                    {
                        "name": "waiting_list_entry",
                        "fields": [
                            { "name": "entry_id", "type": "uuid", "sensitive": false },
                            {
                                "name": "lab_id",
                                "type": "uuid",
                                "sensitive": false,
                                "references": { "record": "Lab", "field": "lab_id" }
                            },
                            { "name": "email", "type": "email", "sensitive": false },
                            { "name": "name", "type": "string", "sensitive": false },
                            { "name": "invitation_code", "type": "string", "sensitive": false },
                            { "name": "referred_count", "type": "integer", "sensitive": false }
                        ]
                    }
                ]
            }),
        );
        let provider = runtime.runtime.providers.store("store").unwrap();
        let namespace = ProviderNamespace {
            tenant_id: "tenant-a".to_string(),
            sor_name: runtime.runtime.config.deployment.sor_name.clone(),
        };
        let lab_id = "11111111-1111-4111-8111-111111111111";
        provider
            .create(greentic_sorx_core::CreateOp {
                namespace: namespace.clone(),
                entity: "Lab".to_string(),
                collection: "labs".to_string(),
                input: json!({ "lab_id": lab_id, "name": "Lab One" }),
                idempotency_key: None,
                unique_indexes: Vec::new(),
                unique_behavior: greentic_sorx_core::UniqueConflictBehavior::Reject,
            })
            .unwrap();
        provider
            .create(greentic_sorx_core::CreateOp {
                namespace,
                entity: "waiting_list_entry".to_string(),
                collection: "waiting_list_entries".to_string(),
                input: json!({
                    "entry_id": "entry-1",
                    "lab_id": lab_id,
                    "email": "one@example.com",
                    "name": "Entry One",
                    "invitation_code": "ABC123",
                    "referred_count": 0
                }),
                idempotency_key: None,
                unique_indexes: Vec::new(),
                unique_behavior: greentic_sorx_core::UniqueConflictBehavior::Reject,
            })
            .unwrap();

        let dashboard = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/dashboard",
            &tenant_headers(),
            "",
        );
        assert!(card_has_action_title(&dashboard, "Labs"));
        assert!(!card_has_action_title(&dashboard, "Waiting List Entries"));

        let labs = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/Lab",
            &tenant_headers(),
            "",
        );
        assert_eq!(labs["metadata"]["total"], 1);
        assert!(card_contains_text(&labs, "Lab One"));
        assert!(card_has_action_title(&labs, "Add Lab"));
        assert!(card_has_action_title(&labs, "Edit"));
        assert!(card_has_action_title(&labs, "X"));

        let lab_create = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/Lab/create",
            &tenant_headers(),
            "",
        );
        assert!(card_contains_text(&lab_create, "Name"));
        assert!(!card_contains_text(&lab_create, "Lab Id"));
        assert!(card_has_action_title(&lab_create, "Submit"));

        let lab_detail = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/Lab/labs-1",
            &tenant_headers(),
            "",
        );
        assert!(card_contains_text(&lab_detail, "Waiting List Entries"));
        assert!(card_contains_text(&lab_detail, "one@example.com"));
        assert!(card_has_action_title(&lab_detail, "Save"));
        assert!(card_has_action_title(&lab_detail, "X"));

        let child_list = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/waiting_list_entry?parent_record=Lab&parent_id=labs-1",
            &tenant_headers(),
            "",
        );
        let child_list_json = serde_json::to_string(&child_list).unwrap();
        assert!(
            child_list_json
                .contains("records/waiting_list_entry?page=1&parent_record=Lab&parent_id=labs-1")
        );
        assert!(
            child_list_json
                .contains("records/waiting_list_entry/create?parent_record=Lab&parent_id=labs-1")
        );
        assert!(child_list_json.contains("records/Lab/labs-1"));

        let child_create = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/waiting_list_entry/create?parent_record=Lab&parent_id=labs-1",
            &tenant_headers(),
            "",
        );
        assert!(card_contains_text(&child_create, "Email"));
        assert!(!card_contains_text(&child_create, "Lab Id"));
        assert!(
            serde_json::to_string(&child_create)
                .unwrap()
                .contains(&format!("\"lab_id\":\"{lab_id}\""))
        );
    }

    #[test]
    fn manager_submit_trims_string_input_values() {
        let mut input = json!({
            "email": " user@example.com ",
            "nested": { "code": " ABC123 " },
            "tags": [" one ", 2, true]
        });

        trim_manager_submit_string_values(&mut input);

        assert_eq!(input["email"], "user@example.com");
        assert_eq!(input["nested"]["code"], "ABC123");
        assert_eq!(input["tags"][0], "one");
    }

    #[test]
    fn manager_parent_create_and_pickers_use_business_record_ids() {
        let runtime = runtime_with_gateway(json!({
            "schema": "greentic.sorla.agent-gateway.v1",
            "record_hierarchy": [
                { "record": "Tenant", "main": true },
                { "record": "Unit", "main": true },
                {
                    "record": "Tenancy",
                    "main": false,
                    "parents": [
                        { "record": "Tenant", "field": "tenant_id" },
                        { "record": "Unit", "field": "unit_id" }
                    ]
                }
            ],
            "endpoints": [
                {
                    "endpoint_id": "tenant.create",
                    "operation_id": "tenant.create",
                    "operation": "create",
                    "method": "POST",
                    "path": "/v1/agent/tenants/create",
                    "entity": "Tenant",
                    "collection": "tenants",
                    "provider_binding": "store",
                    "input_schema": {
                        "type": "object",
                        "required": ["full_name", "email"],
                        "properties": {
                            "full_name": { "type": "string" },
                            "email": { "type": "email" }
                        }
                    }
                },
                {
                    "endpoint_id": "unit.create",
                    "operation_id": "unit.create",
                    "operation": "create",
                    "method": "POST",
                    "path": "/v1/agent/units/create",
                    "entity": "Unit",
                    "collection": "units",
                    "provider_binding": "store",
                    "input_schema": {
                        "type": "object",
                        "required": ["unit_number"],
                        "properties": {
                            "unit_number": { "type": "string" }
                        }
                    }
                },
                {
                    "endpoint_id": "assign_tenant_to_unit",
                    "operation_id": "assign_tenant_to_unit",
                    "operation": "command",
                    "command": { "kind": "record-create", "record": "Tenancy" },
                    "method": "POST",
                    "path": "/v1/agent/tenancies/assign_tenant_to_unit",
                    "entity": "Tenancy",
                    "collection": "tenancies",
                    "provider_binding": "store",
                    "input_schema": {
                        "type": "object",
                        "required": ["tenant_id", "unit_id", "start_date"],
                        "properties": {
                            "tenant_id": { "type": "uuid" },
                            "unit_id": { "type": "uuid" },
                            "start_date": { "type": "date" }
                        }
                    }
                }
            ]
        }));
        let provider = runtime.runtime.providers.store("store").unwrap();
        let namespace = ProviderNamespace {
            tenant_id: "tenant-a".to_string(),
            sor_name: runtime.runtime.config.deployment.sor_name.clone(),
        };
        provider
            .create(greentic_sorx_core::CreateOp {
                namespace: namespace.clone(),
                entity: "Tenant".to_string(),
                collection: "tenants".to_string(),
                input: json!({
                    "tenant_id": "9fda33b2-2ed6-43f7-8ab7-8d53a272b042",
                    "email": "tenant@example.com",
                    "full_name": "Tenant Picker"
                }),
                idempotency_key: None,
                unique_indexes: Vec::new(),
                unique_behavior: greentic_sorx_core::UniqueConflictBehavior::Reject,
            })
            .unwrap();
        provider
            .create(greentic_sorx_core::CreateOp {
                namespace,
                entity: "Unit".to_string(),
                collection: "units".to_string(),
                input: json!({
                    "unit_id": "6c393aba-02a0-461f-a084-49f813155d58",
                    "unit_number": "A1"
                }),
                idempotency_key: None,
                unique_indexes: Vec::new(),
                unique_behavior: greentic_sorx_core::UniqueConflictBehavior::Reject,
            })
            .unwrap();

        let unit_picker = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/pickers/Unit",
            &tenant_headers(),
            "",
        );
        assert_eq!(
            unit_picker["choices"][0]["value"],
            "6c393aba-02a0-461f-a084-49f813155d58"
        );

        let tenant_create = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/Tenancy/create?parent_record=Tenant&parent_id=tenants-1",
            &tenant_headers(),
            "",
        );
        assert!(!card_contains_text(&tenant_create, "Tenant Id"));
        assert!(card_contains_text(&tenant_create, "Unit Id"));
        assert!(
            serde_json::to_string(&tenant_create)
                .unwrap()
                .contains("\"tenant_id\":\"9fda33b2-2ed6-43f7-8ab7-8d53a272b042\"")
        );

        let unit_create = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/Tenancy/create?parent_record=Unit&parent_id=units-1",
            &tenant_headers(),
            "",
        );
        assert!(!card_contains_text(&unit_create, "Unit Id"));
        assert!(card_contains_text(&unit_create, "Tenant Id"));
        assert!(
            serde_json::to_string(&unit_create)
                .unwrap()
                .contains("\"unit_id\":\"6c393aba-02a0-461f-a084-49f813155d58\"")
        );
    }

    #[test]
    fn manager_record_list_card_shows_recent_rows_search_and_row_edit() {
        let runtime = runtime("local");
        let provider = runtime.runtime.providers.store("store").unwrap();
        let namespace = ProviderNamespace {
            tenant_id: "tenant-a".to_string(),
            sor_name: runtime.runtime.config.deployment.sor_name.clone(),
        };
        for index in 1..=12 {
            provider
                .create(greentic_sorx_core::CreateOp {
                    namespace: namespace.clone(),
                    entity: "Tenant".to_string(),
                    collection: "tenants".to_string(),
                    input: json!({
                        "id": format!("tenant-{index:02}"),
                        "name": format!("Tenant {index:02}"),
                        "active": index % 2 == 0,
                        "created_at": format!("2026-05-{index:02}T10:00:00Z")
                    }),
                    idempotency_key: None,
                    unique_indexes: Vec::new(),
                    unique_behavior: greentic_sorx_core::UniqueConflictBehavior::Reject,
                })
                .unwrap();
        }

        let card = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/Tenant",
            &tenant_headers(),
            "",
        );
        assert_eq!(card["metadata"]["kind"], "manager.record.list");
        assert_eq!(card["metadata"]["total"], 12);
        assert!(card_contains_text(&card, "Tenant 12"));
        assert!(!card_contains_text(&card, "Tenant 01"));
        assert!(card_has_action_title(&card, "Add Tenant"));
        assert!(card_has_action_title(&card, "Edit"));
        assert!(card_has_action_title(&card, "X"));
        assert!(card_has_action_title(&card, "Next"));
        assert!(card_has_action_title(&card, "< Main Menu"));
        assert!(card_contains_text(&card, "Open >"));
        let card_json = serde_json::to_string(&card).unwrap();
        assert!(card_json.contains("\"manager_search_input\""));
        assert!(card_json.contains("\"associatedInputs\":\"auto\""));

        let detail = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/Tenant/tenant-12",
            &tenant_headers(),
            "",
        );
        assert!(card_contains_text(&detail, "Tenant 12"));

        let next = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/Tenant?page=2",
            &tenant_headers(),
            "",
        );
        assert_eq!(next["metadata"]["page"], 2);
        assert!(card_contains_text(&next, "Tenant 02"));
        assert!(card_has_action_title(&next, "Previous"));

        let filtered = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/Tenant?q=Tenant+03",
            &tenant_headers(),
            "",
        );
        assert_eq!(filtered["metadata"]["total"], 1);
        assert!(card_contains_text(&filtered, "Tenant 03"));
        assert!(!card_contains_text(&filtered, "Tenant 04"));

        let described_runtime = runtime_with_model(json!({
            "records": [
                {
                    "name": "Tenant",
                    "description": "People or organisations renting a unit."
                }
            ]
        }));
        let described = request(
            &described_runtime,
            "GET",
            "/v1/sorx/manager/cards/records/Tenant",
            &tenant_headers(),
            "",
        );
        assert!(card_contains_text(
            &described,
            "People or organisations renting a unit."
        ));
        assert_eq!(count_card_text(&described, "No records found."), 1);
    }

    #[test]
    fn manager_submit_uses_runtime_invoke_path() {
        let runtime = runtime("local");
        let response = request(
            &runtime,
            "POST",
            "/v1/sorx/manager/submit",
            &tenant_headers(),
            r#"{
                "endpoint_id": "tenant.create",
                "operation_id": "tenant.create",
                "input": {"id":"tenant-manager-1","name":"Acme","active":true},
                "idempotency_key": "manager-submit-1"
            }"#,
        );
        assert_eq!(response["ok"], true);
        assert_eq!(response["result"]["id"], "tenant-manager-1");
    }

    #[test]
    fn manager_create_card_opens_for_record_mutation_create_step() {
        let gateway = json!({
            "schema": "greentic.sorla.agent-gateway.v1",
            "endpoints": [
                {
                    "endpoint_id": "join_waiting_list",
                    "operation_id": "join_waiting_list",
                    "operation": "command",
                    "method": "POST",
                    "path": "/v1/agent/waiting_list_entries/join_waiting_list",
                    "entity": "waiting_list_entry",
                    "collection": "waiting_list_entries",
                    "provider_binding": "store",
                    "risk": "medium",
                    "input_schema": {
                        "type": "object",
                        "required": ["lab_id", "email", "name"],
                        "properties": {
                            "lab_id": { "type": "uuid" },
                            "email": { "type": "email" },
                            "name": { "type": "string" },
                            "invited_by_code": { "type": "string" }
                        }
                    },
                    "command": {
                        "kind": "record_mutation",
                        "action": "join_waiting_list",
                        "steps": [
                            {
                                "op": "create",
                                "as": "entry",
                                "entity": "waiting_list_entry",
                                "collection": "waiting_list_entries",
                                "input": {
                                    "lab_id": "$input.lab_id",
                                    "email": "$input.email",
                                    "name": "$input.name"
                                }
                            }
                        ]
                    }
                }
            ]
        });
        let model = json!({
            "records": [
                {
                    "name": "waiting_list_entry",
                    "fields": [
                        { "name": "entry_id", "type": "uuid", "sensitive": false },
                        { "name": "lab_id", "type": "uuid", "sensitive": false },
                        { "name": "email", "type": "email", "sensitive": false },
                        { "name": "name", "type": "string", "sensitive": false },
                        { "name": "invitation_code", "type": "string", "sensitive": false },
                        { "name": "invited_by_code", "type": "string", "sensitive": false },
                        { "name": "referred_count", "type": "integer", "sensitive": false }
                    ]
                }
            ]
        });
        let runtime = runtime_with_gateway_and_model(gateway, model);

        let list_card = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/waiting_list_entry",
            &tenant_headers(),
            "",
        );
        assert!(card_has_action_title(&list_card, "Add Waiting List Entry"));
        assert!(card_contains_text(&list_card, "Invitation Code"));
        assert!(!card_contains_text(&list_card, "Invited By Code"));
        assert!(card_contains_text(&list_card, "Referred Count"));

        let create_card = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/waiting_list_entry/create",
            &tenant_headers(),
            "",
        );
        assert_eq!(create_card["metadata"]["kind"], "manager.record.create");
        assert!(card_contains_text(&create_card, "Email"));
        assert!(card_contains_text(&create_card, "Invited By Code"));
        assert!(!card_contains_text(&create_card, "Invitation Code"));
        assert!(card_has_action_title(&create_card, "Submit"));
    }

    #[test]
    fn manager_submit_combines_datetime_parts() {
        let mut input = json!({
            "scheduled_at__sorx_date": "2026-05-28",
            "scheduled_at__sorx_time": "09:45",
            "title": "Inspection"
        });
        combine_manager_datetime_inputs(&mut input, &datetime_manager_view(), "RecordAlpha");

        assert_eq!(input["scheduled_at"], "2026-05-28T09:45:00Z");
        assert!(input.get("scheduled_at__sorx_date").is_none());
        assert!(input.get("scheduled_at__sorx_time").is_none());
        assert_eq!(input["title"], "Inspection");
    }

    #[test]
    fn manager_generic_fixture_suite_uses_domain_neutral_records() {
        let runtime = runtime_with_gateway(
            serde_json::from_str(include_str!(
                "../tests/e2e/fixtures/manager/basic-records/agent-gateway.json"
            ))
            .unwrap(),
        );
        let headers = tenant_headers();

        let view = request(&runtime, "GET", "/v1/sorx/manager/view", &headers, "");
        let records = view["records"].as_array().unwrap();
        assert!(
            records
                .iter()
                .any(|record| record["record"] == "RecordAlpha")
        );
        assert!(
            records
                .iter()
                .any(|record| record["record"] == "RecordBeta")
        );
        assert!(!view.to_string().contains("Tenant"));

        let create_card = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/RecordBeta/create",
            &headers,
            "",
        );
        assert!(
            create_card["body"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["id"] == "owner_ref")
        );

        let approval_action = view["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|action| action["endpoint_id"] == "record_beta.approve")
            .unwrap();
        assert_eq!(approval_action["approval_required"], true);
    }

    #[test]
    fn manager_uses_ontology_for_generated_ids_and_relationship_pickers() {
        let runtime =
            runtime_with_gateway_and_ontology(relationship_gateway(), relationship_ontology());
        let headers = tenant_headers();

        let view = request(&runtime, "GET", "/v1/sorx/manager/view", &headers, "");
        let request_record = view["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|record| record["record"] == "MaintenanceRequest")
            .unwrap();
        let id_field = request_record["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["name"] == "id")
            .unwrap();
        assert_eq!(id_field["generated"], true);
        let tenant_field = request_record["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["name"] == "tenant_id")
            .unwrap();
        assert_eq!(tenant_field["relationship"]["to_record"], "Tenant");

        let card = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/MaintenanceRequest/create",
            &headers,
            "",
        );
        let body = card["body"].as_array().unwrap();
        assert!(!body.iter().any(|item| item["id"] == "id"));
        assert!(
            body.iter()
                .any(|item| item["metadata"]["relationship"]["to_record"] == "Tenant")
        );

        let provider = runtime.runtime.providers.store("store").unwrap();
        provider
            .create(greentic_sorx_core::CreateOp {
                namespace: ProviderNamespace {
                    tenant_id: "tenant-a".to_string(),
                    sor_name: runtime.runtime.config.deployment.sor_name.clone(),
                },
                entity: "Tenant".to_string(),
                collection: "tenants".to_string(),
                input: json!({"id": "tenant-1", "name": "Acme"}),
                idempotency_key: None,
                unique_indexes: Vec::new(),
                unique_behavior: greentic_sorx_core::UniqueConflictBehavior::Reject,
            })
            .unwrap();
        let picker = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/pickers/Tenant",
            &headers,
            "",
        );
        assert_eq!(picker["choices"][0]["title"], "Acme");
        assert_eq!(picker["choices"][0]["value"], "tenant-1");

        let mut input = json!({"tenant_id": "tenant-1", "summary": "Leaking tap"});
        let manager_view = request(&runtime, "GET", "/v1/sorx/manager/view", &headers, "");
        let view: ManagerViewModel = serde_json::from_value(manager_view).unwrap();
        fill_generated_manager_fields(
            &mut input,
            &view,
            "MaintenanceRequest",
            "maintenance_request.create",
        );
        assert!(input["id"].as_str().is_some_and(|value| value.len() == 36));
    }

    #[test]
    fn manager_infers_relationship_pickers_from_uuid_field_names() {
        let runtime = runtime_with_gateway(relationship_gateway());
        let card = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/records/MaintenanceRequest/create",
            &tenant_headers(),
            "",
        );
        let body = card["body"].as_array().unwrap();
        let tenant = body
            .iter()
            .find(|item| item["metadata"]["relationship"]["to_record"] == "Tenant")
            .unwrap();
        assert_eq!(tenant["columns"][0]["items"][0]["id"], "tenant_id");
        assert_eq!(
            tenant["columns"][1]["items"][0]["actions"][0]["data"]["routeToCardId"],
            "pickers_Tenant"
        );
    }

    fn datetime_manager_view() -> ManagerViewModel {
        ManagerViewModel {
            schema: "greentic.sorx.manager-view.v1".to_string(),
            tenant_id: "tenant-a".to_string(),
            sor_id: "generic-sor".to_string(),
            title: "Generic Sor".to_string(),
            description: "Manage Record Alpha.".to_string(),
            locale: "en".to_string(),
            navigation: vec![ManagerNavItem {
                record: "RecordAlpha".to_string(),
                label_key: "record.record_alpha.plural".to_string(),
                label: "Record Alpha".to_string(),
                collection: "record_alpha".to_string(),
            }],
            records: vec![ManagerRecordView {
                record: "RecordAlpha".to_string(),
                collection: "record_alpha".to_string(),
                label_key: "record.record_alpha.label".to_string(),
                label: "Record Alpha".to_string(),
                plural_label_key: "record.record_alpha.plural".to_string(),
                plural_label: "Record Alpha".to_string(),
                create_field_names: Vec::new(),
                fields: vec![ManagerFieldView {
                    name: "scheduled_at".to_string(),
                    label_key: "field.record_alpha.scheduled_at.label".to_string(),
                    label: "Scheduled At".to_string(),
                    json_type: Some("datetime".to_string()),
                    rules: None,
                    generated: false,
                    relationship: None,
                    required: true,
                    read_only: false,
                    redacted: false,
                    value: None,
                    hidden: false,
                    display_order: None,
                    display_group: None,
                    policy: ManagerPolicyDecision::allow(),
                }],
                endpoint_ids: Vec::new(),
                policy: ManagerPolicyDecision::allow(),
            }],
            relationships: Vec::new(),
            actions: Vec::new(),
            policies: Vec::new(),
        }
    }

    fn relationship_gateway() -> Value {
        json!({
            "schema": "greentic.sorla.agent-gateway.v1",
            "endpoints": [
                {
                    "endpoint_id": "maintenance_request.create",
                    "operation_id": "maintenance_request.create",
                    "operation": "create",
                    "method": "POST",
                    "path": "/v1/agent/maintenance-requests/create",
                    "entity": "MaintenanceRequest",
                    "collection": "maintenance_requests",
                    "provider_binding": "store",
                    "risk": "medium",
                    "input_schema": {
                        "type": "object",
                        "required": ["id", "tenant_id", "summary"],
                        "properties": {
                            "id": { "type": "uuid" },
                            "tenant_id": { "type": "uuid" },
                            "summary": { "type": "string" }
                        }
                    }
                },
                {
                    "endpoint_id": "tenant.query",
                    "operation_id": "tenant.query",
                    "operation": "query",
                    "method": "POST",
                    "path": "/v1/agent/tenants/query",
                    "entity": "Tenant",
                    "collection": "tenants",
                    "provider_binding": "store"
                }
            ]
        })
    }

    fn relationship_ontology() -> OntologyAssets {
        let graph = OntologyGraph {
            schema: "greentic.sorla.ontology.graph.v1".to_string(),
            concepts: vec![
                OntologyConcept {
                    id: "Tenant".to_string(),
                    label: Some("Tenant".to_string()),
                    records: vec!["Tenant".to_string()],
                    extra: Default::default(),
                },
                OntologyConcept {
                    id: "MaintenanceRequest".to_string(),
                    label: Some("Maintenance Request".to_string()),
                    records: vec!["MaintenanceRequest".to_string()],
                    extra: Default::default(),
                },
            ],
            relationships: vec![OntologyRelationship {
                id: "tenant_has_maintenance_request".to_string(),
                from: Some("Tenant".to_string()),
                to: Some("MaintenanceRequest".to_string()),
                label: Some("Tenant".to_string()),
                extra: Default::default(),
            }],
            records: vec![
                OntologyRecordRef {
                    id: "Tenant".to_string(),
                    concept_id: "Tenant".to_string(),
                },
                OntologyRecordRef {
                    id: "MaintenanceRequest".to_string(),
                    concept_id: "MaintenanceRequest".to_string(),
                },
            ],
            ir_sha256: None,
            ontology_ir_sha256: None,
            ir_hash: None,
            extra: Default::default(),
        };
        OntologyAssets {
            graph_json: serde_json::to_value(&graph).unwrap(),
            graph,
            ir_cbor: None,
            retrieval_bindings_json: None,
            retrieval_bindings: None,
        }
    }

    #[test]
    fn metrics_are_listed_queried_and_exposed_as_mcp_metadata() {
        let runtime = runtime("local");
        let provider = runtime.runtime.providers.store("store").unwrap();
        let namespace = ProviderNamespace {
            tenant_id: "tenant-a".to_string(),
            sor_name: "landlord".to_string(),
        };
        provider
            .create(greentic_sorx_core::CreateOp {
                namespace: namespace.clone(),
                entity: "Payment".to_string(),
                collection: "payments".to_string(),
                input: json!({ "id": "payment-1", "amount": 1250.0 }),
                idempotency_key: None,
                unique_indexes: Vec::new(),
                unique_behavior: greentic_sorx_core::UniqueConflictBehavior::Reject,
            })
            .unwrap();
        provider
            .create(greentic_sorx_core::CreateOp {
                namespace: namespace.clone(),
                entity: "Cost".to_string(),
                collection: "costs".to_string(),
                input: json!({ "id": "cost-1", "amount": 350.0 }),
                idempotency_key: None,
                unique_indexes: Vec::new(),
                unique_behavior: greentic_sorx_core::UniqueConflictBehavior::Reject,
            })
            .unwrap();
        provider
            .create(greentic_sorx_core::CreateOp {
                namespace,
                entity: "waiting_list_entry".to_string(),
                collection: "waiting_list_entries".to_string(),
                input: json!({
                    "id": "waiting-list-entry-1",
                    "lab_id": "example",
                    "user_id": "example",
                    "referred_count": 0
                }),
                idempotency_key: None,
                unique_indexes: Vec::new(),
                unique_behavior: greentic_sorx_core::UniqueConflictBehavior::Reject,
            })
            .unwrap();
        let metrics = request(&runtime, "GET", "/v1/sorx/metrics", &[], "");
        assert_eq!(metrics["schema"], "greentic.sorx.metrics.v1");
        assert_eq!(metrics["metrics"][0]["name"], "daily_clicks");

        let definition = request(&runtime, "GET", "/v1/sorx/metrics/gross_margin", &[], "");
        assert_eq!(
            definition["metric"]["kind"]["dependencies"][0],
            "monthly_revenue"
        );

        let result = request(
            &runtime,
            "POST",
            "/v1/sorx/metrics/gross_margin/query",
            &tenant_headers(),
            r#"{"from":"2026-01-01T00:00:00Z","to":"2026-02-01T00:00:00Z","grain":"month"}"#,
        );
        assert_eq!(result["schema"], "greentic.sorx.metric-query-result.v1");
        assert_eq!(result["result"]["metric"], "gross_margin");
        assert_eq!(result["result"]["rows"][0]["value"], 900.0);

        let waiting_list = request(
            &runtime,
            "POST",
            "/v1/sorx/metrics/number_in_waiting_list/query",
            &tenant_headers(),
            r#"{"dimensions":["lab_id"]}"#,
        );
        assert_eq!(waiting_list["result"]["metric"], "number_in_waiting_list");
        assert_eq!(
            waiting_list["result"]["rows"][0]["dimensions"]["lab_id"],
            "example"
        );
        assert_eq!(waiting_list["result"]["rows"][0]["value"], 1.0);
        let empty_dimensioned_query = request(
            &runtime,
            "POST",
            "/v1/sorx/metrics/number_in_waiting_list/query",
            &tenant_headers(),
            r#"{"dimensions":["lab_id"],"filters":[{"field":"lab_id","operator":"equals","value":"missing"}]}"#,
        );
        assert_eq!(
            empty_dimensioned_query["result"]["rows"]
                .as_array()
                .unwrap()
                .len(),
            0
        );

        let manager_metrics = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/metrics",
            &tenant_headers(),
            "",
        );
        assert_eq!(manager_metrics["metadata"]["kind"], "manager.metrics");
        assert!(card_contains_text(&manager_metrics, "Metric"));
        assert!(card_contains_text(&manager_metrics, "Value"));
        assert!(card_contains_text(&manager_metrics, "monthly_revenue"));
        assert!(card_contains_text(&manager_metrics, "1250"));

        let manager_metric_detail = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/metrics/number_in_waiting_list",
            &tenant_headers(),
            "",
        );
        assert_eq!(manager_metric_detail["metadata"]["kind"], "manager.metric");
        assert!(card_contains_text(&manager_metric_detail, "lab_id"));
        assert!(card_contains_text(&manager_metric_detail, "example"));
        assert!(card_contains_text(&manager_metric_detail, "1"));

        let failed_manager_metric_detail = request(
            &runtime,
            "GET",
            "/v1/sorx/manager/cards/metrics/lab_click_rate",
            &tenant_headers(),
            "",
        );
        assert_eq!(
            failed_manager_metric_detail["metadata"]["kind"],
            "manager.metric"
        );
        assert!(card_contains_text(
            &failed_manager_metric_detail,
            "Metric query failed."
        ));
        assert!(card_contains_text(
            &failed_manager_metric_detail,
            "does not define dimension `lab_id`"
        ));
        assert!(!card_contains_text(
            &failed_manager_metric_detail,
            "No metric data found."
        ));

        let tools = request(&runtime, "GET", "/v1/sorx/tools", &[], "");
        assert!(
            tools["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "sorx.metrics.query.gross_margin")
        );
        let waiting_tool = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "sorx.metrics.query.number_in_waiting_list")
            .unwrap();
        assert_eq!(
            waiting_tool["input_schema"]["properties"]["dimensions"]["items"]["enum"][0],
            "lab_id"
        );
    }

    #[test]
    fn metric_queries_emit_audit_and_deny_sensitive_dimensions() {
        let audit = Arc::new(MemoryAuditSink::new());
        let runtime = with_audit_sink(runtime("local"), audit.clone());
        runtime
            .runtime
            .providers
            .store("store")
            .unwrap()
            .create(greentic_sorx_core::CreateOp {
                namespace: ProviderNamespace {
                    tenant_id: "tenant-a".to_string(),
                    sor_name: "landlord".to_string(),
                },
                entity: "Click".to_string(),
                collection: "clicks".to_string(),
                input: json!({ "id": "click-1", "user_email": "person@example.com" }),
                idempotency_key: None,
                unique_indexes: Vec::new(),
                unique_behavior: greentic_sorx_core::UniqueConflictBehavior::Reject,
            })
            .unwrap();
        let denied = request(
            &runtime,
            "POST",
            "/v1/sorx/metrics/daily_clicks/query",
            &tenant_headers(),
            r#"{"dimensions":["user_email"]}"#,
        );
        assert_eq!(denied["error"]["code"], "SORX_METRIC_POLICY_DENIED");

        let allowed = request(
            &runtime,
            "POST",
            "/v1/sorx/metrics/daily_clicks/query",
            &tenant_headers(),
            r#"{"dimensions":[]}"#,
        );
        assert_eq!(allowed["result"]["rows"][0]["value"], 1.0);

        let allowed_with_sorla_operator = request(
            &runtime,
            "POST",
            "/v1/sorx/metrics/daily_clicks/query",
            &tenant_headers(),
            r#"{"filters":[{"field":"id","operator":"equals","value":"click-1"}]}"#,
        );
        assert_eq!(
            allowed_with_sorla_operator["result"]["rows"][0]["value"],
            1.0
        );

        let events = audit.events().unwrap();
        assert!(
            events
                .iter()
                .any(|event| event.event == "sorx.metric.query.rejected"
                    && event.decision.as_deref() == Some("denied"))
        );
        assert!(
            events
                .iter()
                .any(|event| event.event == "sorx.metric.cache_miss"
                    && event.decision.as_deref() == Some("miss"))
        );
        assert!(
            events
                .iter()
                .any(|event| event.event == "sorx.metric.queried"
                    && event.decision.as_deref() == Some("allow"))
        );
    }

    #[test]
    fn shared_secret_auth_protects_http_surface() {
        // SAFETY: this test sets a unique process env var before constructing the runtime
        // and never mutates it again.
        unsafe {
            std::env::set_var("SORX_TEST_HTTP_INGEST_SECRET", "correct-secret");
        }
        let mut answers = answers("local");
        answers["server"]["auth"] = json!({
            "mode": "shared_secret",
            "shared_secret_ref": "env:SORX_TEST_HTTP_INGEST_SECRET"
        });
        let runtime = runtime_with_answers(answers);

        assert_eq!(
            response(&runtime, "GET", "/healthz", &[], "").status,
            200,
            "healthz remains unauthenticated for probes"
        );

        let unauthorized = response(&runtime, "GET", "/v1/sorx/routes", &[], "");
        assert_eq!(unauthorized.status, 401);
        assert_eq!(unauthorized.body["error"]["code"], "SORX_UNAUTHORIZED");

        let authorized = response(
            &runtime,
            "GET",
            "/v1/sorx/routes",
            &[("Authorization", "Bearer correct-secret")],
            "",
        );
        assert_eq!(authorized.status, 200);
        assert_eq!(authorized.body["schema"], "greentic.sorx.routes.v1");

        let authorized_with_ingest_header = response(
            &runtime,
            "POST",
            "/v1/agent/tenants/create",
            &[
                ("X-Greentic-Sorx-Secret", "correct-secret"),
                ("X-Greentic-Tenant-Id", "tenant-a"),
                ("X-Greentic-Caller-Id", "tester"),
            ],
            r#"{"id":"tenant-1","name":"Acme","active":true}"#,
        );
        assert_eq!(authorized_with_ingest_header.status, 200);
        assert_eq!(authorized_with_ingest_header.body["ok"], true);
    }

    #[test]
    fn business_action_list_get_dry_run_and_invoke_work() {
        let runtime = runtime("local");
        let actions = request(&runtime, "GET", "/v1/sorx/business-actions", &[], "");
        assert_eq!(actions["schema"], "greentic.sorx.business-actions.v1");
        assert_eq!(actions["actions"][0]["id"], "record_rent_payment");
        assert_eq!(actions["actions"][0]["versions"][0], "0.1.0");

        let action = request(
            &runtime,
            "GET",
            "/v1/sorx/business-actions/record_rent_payment/versions/0.1.0",
            &[],
            "",
        );
        assert_eq!(action["id"], "record_rent_payment");
        assert!(action["input_schema"].is_object());

        let body = json!({
            "action_ref": { "contract_hash": business_action_hash() },
            "values": { "id": "tenant-ba-1", "name": "Acme", "active": true },
            "options": { "idempotency_key": "business-action-1" }
        })
        .to_string();
        let dry_run = request(
            &runtime,
            "POST",
            "/v1/sorx/business-actions/record_rent_payment/versions/0.1.0/dry-run",
            &tenant_headers(),
            &body,
        );
        assert_eq!(dry_run["valid"], true);
        let missing_after_dry_run = request(
            &runtime,
            "GET",
            "/v1/agent/tenants/tenant-ba-1",
            &tenant_headers(),
            "",
        );
        assert!(missing_after_dry_run["result"].is_null());

        let invoked = request(
            &runtime,
            "POST",
            "/v1/sorx/business-actions/record_rent_payment/versions/0.1.0/invoke",
            &tenant_headers(),
            &body,
        );
        assert_eq!(invoked["ok"], true);
        assert_eq!(invoked["action_ref"]["id"], "record_rent_payment");
        assert_eq!(invoked["result"]["id"], "tenant-ba-1");
    }

    #[test]
    fn capability_invoke_uses_business_action_runtime_path() {
        let runtime = runtime("local");
        let capability =
            "cap://greentic/business-functions/landlord-tenant-sor/record_rent_payment/v0.1.0";
        let dry_run = request(
            &runtime,
            "POST",
            "/admin/v1/capabilities/invoke",
            &tenant_headers(),
            &json!({
                "capability": capability,
                "dry_run": true,
                "input": { "id": "tenant-cap-1", "name": "Capstone", "active": true },
                "idempotency_key": "capability-action-1",
                "context": {
                    "tenant_id": "tenant-a",
                    "caller_id": "capability-client",
                    "roles": ["local"]
                }
            })
            .to_string(),
        );
        assert_eq!(dry_run["valid"], true);
        assert_eq!(dry_run["execution_target"]["endpoint_id"], "tenant.create");
        let missing_after_dry_run = request(
            &runtime,
            "GET",
            "/v1/agent/tenants/tenant-cap-1",
            &tenant_headers(),
            "",
        );
        assert!(missing_after_dry_run["result"].is_null());

        let invoked = request(
            &runtime,
            "POST",
            "/admin/v1/capabilities/invoke",
            &tenant_headers(),
            &json!({
                "capability": capability,
                "input": { "id": "tenant-cap-1", "name": "Capstone", "active": true },
                "idempotency_key": "capability-action-1",
                "context": {
                    "tenant_id": "tenant-a",
                    "caller_id": "capability-client",
                    "roles": ["local"]
                }
            })
            .to_string(),
        );
        assert_eq!(invoked["ok"], true);
        assert_eq!(
            invoked["schema"],
            "greentic.sorx.capability-invoke-result.v1"
        );
        assert_eq!(invoked["capability"], capability);
        assert_eq!(invoked["action_ref"]["id"], "record_rent_payment");
        assert_eq!(invoked["result"]["id"], "tenant-cap-1");
    }

    #[test]
    fn business_action_rejects_bad_version_hash_payload_and_missing_idempotency() {
        let runtime = runtime("local");
        let unknown_version = request(
            &runtime,
            "GET",
            "/v1/sorx/business-actions/record_rent_payment/versions/9.9.9",
            &[],
            "",
        );
        assert_eq!(unknown_version["error"]["code"], "unknown_action_version");

        let malformed_hash = request(
            &runtime,
            "POST",
            "/v1/sorx/business-actions/record_rent_payment/versions/0.1.0/invoke",
            &tenant_headers(),
            r#"{"action_ref":{"contract_hash":"sha256:bad"},"values":{"id":"tenant-ba-2","name":"Acme","active":true},"options":{"idempotency_key":"business-action-2"}}"#,
        );
        assert_eq!(malformed_hash["error"]["code"], "invalid_contract_hash");

        let bad_hash = request(
            &runtime,
            "POST",
            "/v1/sorx/business-actions/record_rent_payment/versions/0.1.0/invoke",
            &tenant_headers(),
            r#"{"action_ref":{"contract_hash":"sha256:0000000000000000000000000000000000000000000000000000000000000000"},"values":{"id":"tenant-ba-2","name":"Acme","active":true},"options":{"idempotency_key":"business-action-2"}}"#,
        );
        assert_eq!(bad_hash["error"]["code"], "contract_hash_mismatch");

        let invalid_payload = request(
            &runtime,
            "POST",
            "/v1/sorx/business-actions/record_rent_payment/versions/0.1.0/invoke",
            &tenant_headers(),
            &json!({
                "action_ref": { "contract_hash": business_action_hash() },
                "values": { "id": "tenant-ba-2", "name": "Acme", "active": true, "extra": true },
                "options": { "idempotency_key": "business-action-2" }
            })
            .to_string(),
        );
        assert_eq!(invalid_payload["error"]["code"], "invalid_payload");

        let missing_idempotency = request(
            &runtime,
            "POST",
            "/v1/sorx/business-actions/record_rent_payment/versions/0.1.0/invoke",
            &tenant_headers(),
            &json!({
                "action_ref": { "contract_hash": business_action_hash() },
                "values": { "id": "tenant-ba-2", "name": "Acme", "active": true }
            })
            .to_string(),
        );
        assert_eq!(
            missing_idempotency["error"]["code"],
            "missing_idempotency_key"
        );

        let conflicting_ref = request(
            &runtime,
            "POST",
            "/v1/sorx/business-actions/record_rent_payment/versions/0.1.0/invoke",
            &tenant_headers(),
            &json!({
                "action_ref": {
                    "id": "record_rent_payment",
                    "version": "9.9.9",
                    "contract_hash": business_action_hash()
                },
                "values": { "id": "tenant-ba-2", "name": "Acme", "active": true },
                "options": { "idempotency_key": "business-action-2" }
            })
            .to_string(),
        );
        assert_eq!(conflicting_ref["error"]["code"], "version_mismatch");
    }

    #[test]
    fn create_get_update_and_query_tenant_via_http() {
        let runtime = runtime("local");
        let created = request(
            &runtime,
            "POST",
            "/v1/agent/tenants/create",
            &tenant_headers(),
            r#"{"id":"tenant-1","name":"Acme","active":true,"landlord_id":"landlord-1"}"#,
        );
        assert_eq!(created["ok"], true);
        assert_eq!(created["result"]["id"], "tenant-1");

        let fetched = request(
            &runtime,
            "GET",
            "/v1/agent/tenants/tenant-1",
            &tenant_headers(),
            "",
        );
        assert_eq!(fetched["result"]["data"]["name"], "Acme");

        let updated = request(
            &runtime,
            "PATCH",
            "/v1/agent/tenants/tenant-1",
            &tenant_headers(),
            r#"{"patch":{"active":false}}"#,
        );
        assert_eq!(updated["result"]["data"]["active"], false);

        let queried = request(
            &runtime,
            "POST",
            "/v1/agent/tenants/query",
            &tenant_headers(),
            r#"{"filter":{"active":false}}"#,
        );
        assert_eq!(queried["result"]["records"].as_array().unwrap().len(), 1);
        assert_eq!(queried["result"]["records"][0]["id"], "tenant-1");

        let first_code = request(
            &runtime,
            "POST",
            "/v1/agent/tenants/generate-code",
            &tenant_headers(),
            r#"{"id":"tenant-1"}"#,
        );
        assert_eq!(first_code["ok"], true);
        assert_eq!(first_code["endpoint_id"], "tenant.generate_code");
        let code = first_code["result"]["code"].as_str().unwrap().to_string();
        assert!(!code.is_empty());

        let second_code = request(
            &runtime,
            "POST",
            "/v1/agent/tenants/generate-code",
            &tenant_headers(),
            r#"{"id":"tenant-1"}"#,
        );
        assert_eq!(second_code["result"]["code"], code);

        let queried_with_code = request(
            &runtime,
            "POST",
            "/v1/agent/tenants/query",
            &tenant_headers(),
            r#"{"filter":{"id":"tenant-1"}}"#,
        );
        assert_eq!(
            queried_with_code["result"]["records"][0]["data"]["code"],
            code
        );

        let active_by_landlord = request(
            &runtime,
            "GET",
            "/v1/agent/landlords/landlord-1/active-tenants/false",
            &tenant_headers(),
            "",
        );
        assert_eq!(
            active_by_landlord["result"]["records"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(active_by_landlord["result"]["records"][0]["id"], "tenant-1");
    }

    #[test]
    fn invalid_json_and_schema_errors_are_structured() {
        let runtime = runtime("local");
        let bad_json = request(
            &runtime,
            "POST",
            "/v1/agent/tenants/create",
            &tenant_headers(),
            "{",
        );
        assert_eq!(bad_json["ok"], false);
        assert_eq!(bad_json["error"]["code"], "SORX_INVALID_JSON");

        let invalid_input = request(
            &runtime,
            "POST",
            "/v1/agent/tenants/create",
            &tenant_headers(),
            r#"{"id":"tenant-1","name":"Acme"}"#,
        );
        assert_eq!(invalid_input["ok"], false);
        assert_eq!(invalid_input["error"]["code"], "invalid_input");
        assert_eq!(invalid_input["error"]["details"]["path"], "active");
    }

    #[test]
    fn missing_tenant_header_fails_outside_local_mode() {
        let runtime = runtime("production");
        let response = request(
            &runtime,
            "POST",
            "/v1/agent/tenants/create",
            &[("X-Greentic-Caller-Id", "tester")],
            r#"{"id":"tenant-1","name":"Acme","active":true}"#,
        );
        assert_eq!(response["ok"], false);
        assert_eq!(response["error"]["code"], "context_missing");
    }

    #[test]
    fn missing_caller_header_fails_outside_local_mode() {
        let runtime = runtime("production");
        let response = request(
            &runtime,
            "POST",
            "/v1/agent/tenants/create",
            &[("X-Greentic-Tenant-Id", "tenant-a")],
            r#"{"id":"tenant-1","name":"Acme","active":true}"#,
        );
        assert_eq!(response["ok"], false);
        assert_eq!(response["error"]["code"], "context_missing");
    }

    #[test]
    fn high_risk_http_operation_returns_pending_approval() {
        let runtime = runtime("local");
        let response = request(
            &runtime,
            "DELETE",
            "/v1/agent/tenants/tenant-1",
            &tenant_headers(),
            "",
        );
        assert_eq!(response["ok"], false);
        assert_eq!(response["status"], "approval_required");
        assert_eq!(response["approval"]["risk"], "high");
    }

    #[test]
    fn admin_api_is_disabled_by_default() {
        let runtime = runtime("local");
        let response = request(&runtime, "GET", "/v1/sorx/deployments", &[], "");
        assert_eq!(response["ok"], false);
        assert_eq!(response["error"]["code"], "SORX_ADMIN_API_DISABLED");
    }

    #[test]
    fn generic_admin_runtime_contract_reports_metadata_capabilities_and_health() {
        let runtime = runtime("local");
        let info = request(&runtime, "GET", "/admin/v1/runtime", &[], "");
        assert_eq!(info["schema"], "greentic.runtime.info.v1");
        assert_eq!(info["runtime_kind"], "runtime-host");
        assert_eq!(info["implementation"], "sorx");
        assert!(
            info["contracts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|contract| contract == "greentic.runtime.deployments.v1")
        );

        let capabilities = request(&runtime, "GET", "/admin/v1/capabilities", &[], "");
        assert_eq!(capabilities["schema"], "greentic.capabilities.v1");
        assert_eq!(
            capabilities["offers"][0]["capability"],
            "greentic.cap.runtime.host.v1"
        );
        let offers = capabilities["offers"].as_array().unwrap();
        let business_offer = offers
            .iter()
            .find(|offer| offer["metadata"]["kind"] == "business_function")
            .unwrap();
        assert_eq!(
            business_offer["capability"],
            "cap://greentic/business-functions/landlord-tenant-sor/record_rent_payment/v0.1.0"
        );
        assert_eq!(
            business_offer["metadata"]["action"]["contract_hash"],
            business_action_hash()
        );
        assert!(offers.iter().any(|offer| {
            offer["capability"] == "cap://greentic/events/landlord-tenant-sor/tenant.code_generated"
                && offer["metadata"]["kind"] == "business_event_topic"
        }));

        let health = request(&runtime, "GET", "/admin/v1/health", &[], "");
        assert_eq!(health["schema"], "greentic.runtime.health.v1");
        assert_eq!(health["status"], "ok");
        assert_eq!(health["deployment_count"], 0);
    }

    #[test]
    fn generic_admin_stage_warm_activate_traffic_drain_and_deactivate() {
        let runtime = runtime("local");
        let staged = request(
            &runtime,
            "POST",
            "/admin/v1/deployments/stage",
            &[],
            r#"{"deployment_id":"dep-a","revision_id":"rev-a","bundle_id":"bundle-a","stack_id":"stack-a","artifact_uri":"file:bundle-a.gtbundle"}"#,
        );
        assert_eq!(staged["deployment_id"], "dep-a");
        assert_eq!(staged["revisions"][0]["lifecycle"], "staged");

        let warmed = request(
            &runtime,
            "POST",
            "/admin/v1/deployments/dep-a/warm",
            &[],
            "",
        );
        assert_eq!(warmed["revisions"][0]["lifecycle"], "ready");

        let staged_zero_weight = request(
            &runtime,
            "POST",
            "/admin/v1/deployments/stage",
            &[],
            r#"{"deployment_id":"dep-a","revision_id":"rev-b","bundle_id":"bundle-a"}"#,
        );
        assert_eq!(staged_zero_weight["revisions"].as_array().unwrap().len(), 2);

        let activated = request(
            &runtime,
            "POST",
            "/admin/v1/deployments/dep-a/activate",
            &[],
            "",
        );
        assert_eq!(activated["revisions"][1]["lifecycle"], "ready");

        let traffic = request(
            &runtime,
            "POST",
            "/admin/v1/deployments/dep-a/traffic",
            &[],
            r#"{"entries":[{"revision_id":"rev-a","weight_bps":10000},{"revision_id":"rev-b","weight_bps":0}]}"#,
        );
        assert_eq!(traffic["schema"], "greentic.runtime.traffic.v1");
        assert_eq!(traffic["entries"][0]["weight_bps"], 10000);

        let drained = request(
            &runtime,
            "POST",
            "/admin/v1/deployments/dep-a/revisions/rev-a/drain",
            &[],
            "",
        );
        assert_eq!(drained["revisions"][0]["lifecycle"], "draining");
        assert_eq!(drained["revisions"][0]["weight_bps"], 0);

        let deactivated = request(
            &runtime,
            "POST",
            "/admin/v1/deployments/dep-a/deactivate",
            &[],
            "",
        );
        assert!(
            deactivated["revisions"]
                .as_array()
                .unwrap()
                .iter()
                .all(|revision| revision["weight_bps"] == 0)
        );
    }

    #[test]
    fn generic_admin_rejects_invalid_traffic_sum() {
        let runtime = runtime("local");
        let _ = request(
            &runtime,
            "POST",
            "/admin/v1/deployments/stage",
            &[],
            r#"{"deployment_id":"dep-a","revision_id":"rev-a","bundle_id":"bundle-a"}"#,
        );
        let response = request(
            &runtime,
            "POST",
            "/admin/v1/deployments/dep-a/traffic",
            &[],
            r#"{"entries":[{"revision_id":"rev-a","weight_bps":9000}]}"#,
        );
        assert_eq!(response["ok"], false);
        assert_eq!(response["error"]["code"], "runtime_traffic_invalid_sum");
    }

    #[test]
    fn generic_admin_rejects_unknown_revision_in_traffic_split() {
        let runtime = runtime("local");
        let _ = request(
            &runtime,
            "POST",
            "/admin/v1/deployments/stage",
            &[],
            r#"{"deployment_id":"dep-a","revision_id":"rev-a","bundle_id":"bundle-a"}"#,
        );
        let response = request(
            &runtime,
            "POST",
            "/admin/v1/deployments/dep-a/traffic",
            &[],
            r#"{"entries":[{"revision_id":"rev-missing","weight_bps":10000}]}"#,
        );
        assert_eq!(response["ok"], false);
        assert_eq!(response["error"]["code"], "runtime_revision_missing");
    }

    #[test]
    fn generic_admin_applies_runtime_config_snapshot() {
        let runtime = runtime("local");
        let response = request(
            &runtime,
            "POST",
            "/admin/v1/runtime-config",
            &[],
            &json!({
                "schema": "greentic.runtime-config.v1",
                "env_id": "local",
                "revisions": [
                    {
                        "deployment_id": "dep-a",
                        "revision_id": "rev-a",
                        "bundle_id": "bundle-a",
                        "pack_list_refs": ["locks/rev-a.json"],
                        "pack_config_refs": ["configs/rev-a.json"],
                        "weight_bps": 9000
                    },
                    {
                        "deployment_id": "dep-a",
                        "revision_id": "rev-b",
                        "bundle_id": "bundle-a",
                        "weight_bps": 1000
                    }
                ]
            })
            .to_string(),
        );
        assert_eq!(response["schema"], "greentic.runtime.deployments.v1");
        assert_eq!(response["deployments"][0]["deployment_id"], "dep-a");
        assert_eq!(
            response["deployments"][0]["revisions"][0]["lifecycle"],
            "ready"
        );
        assert_eq!(
            response["deployments"][0]["revisions"][0]["weight_bps"],
            9000
        );
    }

    #[test]
    fn generic_admin_reports_initial_runtime_config_snapshot() {
        let runtime = runtime_with_initial_runtime_config(RuntimeConfig {
            schema: "greentic.runtime-config.v1".to_string(),
            env_id: "local".to_string(),
            revisions: vec![greentic_sorx_core::RevisionRuntimeBlock {
                deployment_id: "dep-a".to_string(),
                revision_id: "rev-a".to_string(),
                bundle_id: "bundle-a".to_string(),
                pack_list_refs: Vec::new(),
                pack_config_refs: Vec::new(),
                weight_bps: 10000,
            }],
            extensions: greentic_sorx_core::RuntimeExtensions::default(),
        });
        let response = request(&runtime, "GET", "/admin/v1/deployments", &[], "");
        assert_eq!(response["deployments"][0]["deployment_id"], "dep-a");
        assert_eq!(
            response["deployments"][0]["revisions"][0]["revision_id"],
            "rev-a"
        );
        assert_eq!(
            response["deployments"][0]["revisions"][0]["weight_bps"],
            10000
        );
    }

    #[test]
    fn generic_runtime_host_fixture_drives_initial_snapshot_and_admin_surface() {
        let config: RuntimeConfig =
            serde_json::from_str(GENERIC_RUNTIME_CONFIG).expect("fixture should be valid JSON");
        let runtime = runtime_with_initial_runtime_config(config);
        let deployments = request(&runtime, "GET", "/admin/v1/deployments", &[], "");
        assert_eq!(deployments["schema"], "greentic.runtime.deployments.v1");
        assert_eq!(
            deployments["deployments"][0]["deployment_id"],
            "generic-deployment"
        );
        assert_eq!(
            deployments["deployments"][0]["revisions"][0]["revision_id"],
            "rev-a"
        );
        assert_eq!(
            deployments["deployments"][0]["revisions"][0]["weight_bps"],
            7500
        );
        assert_eq!(
            deployments["deployments"][0]["revisions"][1]["weight_bps"],
            2500
        );

        let surfaces = request(&runtime, "GET", "/admin/v1/admin-surfaces", &[], "");
        assert_eq!(surfaces["surfaces"][0]["surface_id"], "stack-console");
        assert_eq!(surfaces["surfaces"][0]["path"], "/admin/stacks");

        let page = request(&runtime, "GET", "/admin/stacks", &[], "");
        assert_eq!(page["schema"], "greentic.runtime.admin-surface.v1");
        assert_eq!(
            page["surface"]["source_pack_ref"],
            "extensions/admin/stack-console.gtpack"
        );
    }

    #[test]
    fn generic_admin_registers_and_lists_admin_surfaces() {
        let runtime = runtime("local");
        let registered = request(
            &runtime,
            "POST",
            "/admin/v1/admin-surfaces",
            &[],
            r#"{"surface_id":"settings.page","kind":"page","path":"/admin/settings","required_permissions":["settings.read"]}"#,
        );
        assert_eq!(registered["surface_id"], "settings.page");

        let listed = request(&runtime, "GET", "/admin/v1/admin-surfaces", &[], "");
        assert_eq!(listed["schema"], "greentic.runtime.admin-surfaces.v1");
        assert_eq!(listed["surfaces"][0]["surface_id"], "settings.page");
        assert_eq!(listed["surfaces"][0]["kind"], "page");
    }

    #[test]
    fn generic_admin_surface_routes_run_through_admin_pipeline() {
        let observer = Arc::new(AdminRecordingObserver::default());
        let runtime = with_admin_observer(runtime("local"), observer.clone());
        let _ = request(
            &runtime,
            "POST",
            "/admin/v1/admin-surfaces",
            &[],
            r#"{"surface_id":"stack-console","kind":"page","path":"/admin/stacks","source_pack_ref":"extensions/admin/stack-console.gtpack"}"#,
        );

        let page = request(&runtime, "GET", "/admin/stacks", &[], "");
        assert_eq!(page["schema"], "greentic.runtime.admin-surface.v1");
        assert_eq!(page["surface"]["surface_id"], "stack-console");

        let unsupported = request(&runtime, "POST", "/admin/stacks/api/actions", &[], "{}");
        assert_eq!(
            unsupported["error"]["code"],
            "RUNTIME_ADMIN_SURFACE_UNSUPPORTED_HANDLER"
        );
        assert_eq!(
            observer.events(),
            vec![
                "admin.action.started",
                "admin.action.completed",
                "admin.action.started",
                "admin.action.completed",
                "admin.action.started",
                "admin.action.failed"
            ]
        );
    }

    #[test]
    fn generic_admin_runs_through_control_pre_admin() {
        let runtime = with_admin_control(runtime("local"), Arc::new(AdminDenyControl));
        let response = request(&runtime, "GET", "/admin/v1/health", &[], "");
        assert_eq!(response["ok"], false);
        assert_eq!(response["error"]["code"], "RUNTIME_ADMIN_CONTROL_DENIED");
        assert_eq!(response["error"]["message"], "admin blocked");
    }

    #[test]
    fn generic_admin_runs_through_control_post_admin_patch() {
        let runtime = with_admin_control(runtime("local"), Arc::new(AdminPostPatchControl));
        let response = request(&runtime, "GET", "/admin/v1/runtime", &[], "");
        assert_eq!(response["schema"], "greentic.runtime.info.v1");
        assert_eq!(response["admin_pipeline"], "patched");
    }

    #[test]
    fn generic_admin_emits_observer_events() {
        let observer = Arc::new(AdminRecordingObserver::default());
        let runtime = with_admin_observer(runtime("local"), observer.clone());
        let response = request(&runtime, "GET", "/admin/v1/health", &[], "");
        assert_eq!(response["status"], "ok");
        assert_eq!(
            observer.events(),
            vec!["admin.action.started", "admin.action.completed"]
        );
    }

    #[test]
    fn idempotency_key_prevents_duplicate_create_via_http() {
        let runtime = runtime("local");
        let mut headers = tenant_headers().to_vec();
        headers.push(("Idempotency-Key", "tenant-create-1"));
        let first = request(
            &runtime,
            "POST",
            "/v1/agent/tenants/create",
            &headers,
            r#"{"id":"tenant-1","name":"Acme","active":true}"#,
        );
        let second = request(
            &runtime,
            "POST",
            "/v1/agent/tenants/create",
            &headers,
            r#"{"id":"tenant-2","name":"Changed","active":true}"#,
        );
        assert_eq!(first["result"]["id"], "tenant-1");
        assert_eq!(second["result"]["id"], "tenant-1");
        assert_eq!(second["result"]["data"]["name"], "Acme");
    }

    mod registry_http {
        use greentic_sorx_core::{
            CreateDeploymentRequest, DeploymentRegistry, DeploymentStatus, DeploymentVisibility,
            LocalDeploymentRegistryStore, PackArtifact, StateMode,
        };

        use super::super::{HttpResponse, handle_registry_request};

        pub(super) fn create_request(
            version: &str,
            digest: &str,
            api_version_label: &str,
        ) -> CreateDeploymentRequest {
            CreateDeploymentRequest {
                artifact: PackArtifact {
                    source: format!("fixtures/landlord-{version}.gtpack"),
                    name: "landlord-tenant-sor".to_string(),
                    version: version.to_string(),
                    digest: digest.to_string(),
                    signature: None,
                    signature_ref: None,
                },
                tenant_id: "acme".to_string(),
                sor_name: "landlord".to_string(),
                environment: "production".to_string(),
                api_version_label: api_version_label.to_string(),
                base_path: format!("/sorx/acme/landlord/{api_version_label}"),
                visibility: DeploymentVisibility::Private,
                state_mode: StateMode::SharedCompatible,
                state_namespace: None,
                deployment_id: None,
                allow_api_version_conflict: false,
                allow_shared_state_conflict: false,
            }
        }

        fn passing_report(deployment_id: &str, pack_digest: &str) -> serde_json::Value {
            serde_json::json!({
                "schema": "greentic.sorx.validation-report.v1",
                "deployment_id": deployment_id,
                "pack_digest": pack_digest,
                "result": "pass",
                "public_exposure_allowed": true,
                "tests": []
            })
        }

        struct Seed {
            store: LocalDeploymentRegistryStore,
            v1: String,
            v2: String,
            _temp: tempfile::TempDir,
        }

        /// Seed a registry file with two deployments (v1 + v2, both validated
        /// hence routable) and a `stable` alias pointing at v1.
        fn seeded_store() -> Seed {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("registry.json");
            let store = LocalDeploymentRegistryStore::new(&path);
            let mut registry = DeploymentRegistry::default();
            let v1 = registry
                .create_deployment(create_request("1.0.0", "sha256:111", "v1"))
                .unwrap();
            let v2 = registry
                .create_deployment(create_request("2.0.0", "sha256:222", "v2"))
                .unwrap();
            registry.validate_deployment(&v1.deployment_id).unwrap();
            registry.validate_deployment(&v2.deployment_id).unwrap();
            // Validation reports so promote_alias can run in the rollback test.
            registry
                .record_validation_report(passing_report(&v1.deployment_id, "sha256:111"))
                .unwrap();
            registry
                .record_validation_report(passing_report(&v2.deployment_id, "sha256:222"))
                .unwrap();
            registry
                .set_alias("acme", "landlord", "stable", &v1.deployment_id)
                .unwrap();
            store.save(&registry).unwrap();
            Seed {
                store,
                v1: v1.deployment_id,
                v2: v2.deployment_id,
                _temp: temp,
            }
        }

        fn call(seed: &Seed, method: &str, path: &str, body: &str) -> HttpResponse {
            handle_registry_request(&seed.store, method, path, body)
        }

        #[test]
        fn registry_routing_table_resolves_alias() {
            let seed = seeded_store();
            let response = call(&seed, "GET", "/v1/sorx/routing-table", "");
            assert_eq!(response.status, 200);
            let routes = response.body["routes"].as_array().unwrap();
            assert_eq!(routes.len(), 1);
            assert_eq!(routes[0]["alias"], "stable");
            assert_eq!(routes[0]["deployment_id"], seed.v1);
            assert_eq!(routes[0]["routable"], true);
            assert_eq!(routes[0]["pack_version"], "1.0.0");
        }

        #[test]
        fn registry_set_alias_then_resolve() {
            let seed = seeded_store();
            let put = call(
                &seed,
                "PUT",
                "/v1/sorx/aliases/acme/landlord/stable",
                &format!(r#"{{"target_deployment_id":"{}"}}"#, seed.v2),
            );
            assert_eq!(put.status, 200);
            assert_eq!(put.body["target_deployment_id"], seed.v2);

            let table = call(&seed, "GET", "/v1/sorx/routing-table", "");
            let routes = table.body["routes"].as_array().unwrap();
            assert_eq!(routes.len(), 1);
            assert_eq!(routes[0]["deployment_id"], seed.v2);
            assert_eq!(routes[0]["pack_version"], "2.0.0");
        }

        #[test]
        fn registry_set_alias_rejects_non_routable() {
            // Seed a pending (non-routable) deployment alongside the routable ones.
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("registry.json");
            let store = LocalDeploymentRegistryStore::new(&path);
            let mut registry = DeploymentRegistry::default();
            let routable = registry
                .create_deployment(create_request("1.0.0", "sha256:111", "v1"))
                .unwrap();
            registry
                .validate_deployment(&routable.deployment_id)
                .unwrap();
            let pending = registry
                .create_deployment(create_request("2.0.0", "sha256:222", "v2"))
                .unwrap();
            assert_eq!(
                registry.deployment(&pending.deployment_id).unwrap().status,
                DeploymentStatus::Pending
            );
            registry
                .set_alias("acme", "landlord", "stable", &routable.deployment_id)
                .unwrap();
            store.save(&registry).unwrap();
            let seed = Seed {
                store,
                v1: routable.deployment_id.clone(),
                v2: pending.deployment_id.clone(),
                _temp: temp,
            };

            let response = call(
                &seed,
                "PUT",
                "/v1/sorx/aliases/acme/landlord/stable",
                &format!(r#"{{"target_deployment_id":"{}"}}"#, pending.deployment_id),
            );
            assert!(
                (400..500).contains(&response.status),
                "expected 4xx, got {}",
                response.status
            );

            // Alias still points at the routable deployment.
            let table = call(&seed, "GET", "/v1/sorx/routing-table", "");
            let routes = table.body["routes"].as_array().unwrap();
            assert_eq!(routes.len(), 1);
            assert_eq!(routes[0]["deployment_id"], routable.deployment_id);
        }

        #[test]
        fn registry_rollback_alias() {
            let seed = seeded_store();
            // Promote v2 to public on the `stable` alias.
            let promote = call(
                &seed,
                "POST",
                &format!("/v1/sorx/deployments/{}/promote", seed.v2),
                r#"{"alias":"stable","visibility":"public"}"#,
            );
            assert_eq!(promote.status, 200, "promote body: {:?}", promote.body);
            let table = call(&seed, "GET", "/v1/sorx/routing-table", "");
            assert_eq!(table.body["routes"][0]["deployment_id"], seed.v2);

            // Roll the `stable` alias back to v1.
            let rollback = call(
                &seed,
                "POST",
                &format!("/v1/sorx/deployments/{}/rollback", seed.v2),
                &format!(r#"{{"alias":"stable","to_deployment_id":"{}"}}"#, seed.v1),
            );
            assert_eq!(rollback.status, 200, "rollback body: {:?}", rollback.body);

            let table = call(&seed, "GET", "/v1/sorx/routing-table", "");
            assert_eq!(table.body["routes"][0]["deployment_id"], seed.v1);

            // v2 is now marked RolledBack.
            let deployment = call(
                &seed,
                "GET",
                &format!("/v1/sorx/deployments/{}", seed.v2),
                "",
            );
            assert_eq!(deployment.status, 200);
            assert_eq!(deployment.body["status"], "rolled_back");
        }

        #[test]
        fn registry_list_deployments_and_aliases() {
            let seed = seeded_store();
            let deployments = call(
                &seed,
                "GET",
                "/v1/sorx/deployments?tenant=acme&sor=landlord",
                "",
            );
            assert_eq!(deployments.status, 200);
            assert_eq!(deployments.body["deployments"].as_array().unwrap().len(), 2);

            let none = call(&seed, "GET", "/v1/sorx/deployments?tenant=other", "");
            assert_eq!(none.body["deployments"].as_array().unwrap().len(), 0);

            let aliases = call(
                &seed,
                "GET",
                "/v1/sorx/aliases?tenant=acme&sor=landlord",
                "",
            );
            assert_eq!(aliases.status, 200);
            let aliases = aliases.body["aliases"].as_array().unwrap();
            assert_eq!(aliases.len(), 1);
            assert_eq!(aliases[0]["alias"], "stable");
            assert_eq!(aliases[0]["target_deployment_id"], seed.v1);
        }
    }

    #[test]
    fn registry_501_without_registry_path() {
        // Admin API surface enabled but no registry path attached -> 501.
        let mut runtime = runtime("local");
        runtime.admin_api_enabled = true;
        let response = response(&runtime, "GET", "/v1/sorx/deployments", &[], "");
        assert_eq!(response.status, 501);
        assert_eq!(
            response.body["error"]["code"],
            "SORX_ADMIN_API_NOT_IMPLEMENTED"
        );
    }

    #[test]
    fn registry_routing_table_served_when_registry_path_attached() {
        // Thin wiring test: a registry path makes handle_request serve the
        // routing-table from the persisted store instead of 501.
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("registry.json");
        let store = greentic_sorx_core::LocalDeploymentRegistryStore::new(&path);
        let mut registry = greentic_sorx_core::DeploymentRegistry::default();
        let created = registry
            .create_deployment(registry_http::create_request("1.0.0", "sha256:111", "v1"))
            .unwrap();
        registry
            .validate_deployment(&created.deployment_id)
            .unwrap();
        registry
            .set_alias("acme", "landlord", "stable", &created.deployment_id)
            .unwrap();
        store.save(&registry).unwrap();

        let runtime = runtime("local").with_registry_path(Some(path));
        let response = response(&runtime, "GET", "/v1/sorx/routing-table", &[], "");
        assert_eq!(response.status, 200);
        assert_eq!(
            response.body["routes"][0]["deployment_id"],
            created.deployment_id
        );
    }

    // ── business event sink wiring ────────────────────────────────────────────

    /// Build an `HttpRuntime` from the standard fixture answers plus the given
    /// `events` object merged into the answers JSON.
    fn runtime_with_events_answers(events_obj: Value) -> SorxResult<HttpRuntime> {
        let mut answers_json = answers("local");
        answers_json
            .as_object_mut()
            .expect("answers must be an object")
            .insert("events".to_string(), events_obj);
        let pack = pack();
        let normalized =
            normalize_start_answers(&default_start_schema(), &answers_json, true).unwrap();
        let config = runtime_config_from_answers(&pack.pack_name, &normalized.answers).unwrap();
        HttpRuntime::from_pack("local", &pack, config)
    }

    #[test]
    fn events_stdout_sink_is_wired_from_config() {
        let runtime = runtime_with_events_answers(json!({ "sink": "stdout" }));
        assert!(runtime.is_ok());
    }

    #[test]
    fn events_disabled_sink_is_default_passthrough() {
        let runtime = runtime_with_events_answers(json!({}));
        assert!(runtime.is_ok());
    }

    #[test]
    fn events_nats_sink_without_url_is_an_error() {
        let result = runtime_with_events_answers(json!({ "sink": "nats" }));
        match result {
            Err(err) => assert_eq!(err.code, "events_nats_url_missing"),
            Ok(_) => panic!("nats sink without nats_url must be an error"),
        }
    }

    #[test]
    fn events_disabled_explicit_sink_is_ok() {
        let runtime = runtime_with_events_answers(json!({ "sink": "disabled" }));
        assert!(runtime.is_ok());
    }

    #[test]
    fn capability_offers_include_entity_lifecycle_topics() {
        let runtime = runtime("local");
        let pack_name = &runtime.runtime.pack.name;

        let offers = runtime.business_event_topic_offers();

        // Fixture has create/update/delete for entity "Tenant".
        // Each must be offered as a dedicated capability.
        for (operation_label, event_suffix) in [
            ("created", "Tenant.created"),
            ("updated", "Tenant.updated"),
            ("deleted", "Tenant.deleted"),
        ] {
            let expected_topic =
                greentic_sorx_core::entity_event_topic(pack_name, "Tenant", operation_label);
            let matching_offer = offers.iter().find(|offer| {
                offer.metadata.as_ref().is_some_and(|meta| {
                    meta["event_type"] == event_suffix
                        && meta["kind"] == "business_event_topic"
                        && meta["topic"] == expected_topic
                })
            });
            assert!(
                matching_offer.is_some(),
                "expected a lifecycle offer for event_type={event_suffix} topic={expected_topic}"
            );
            let offer = matching_offer.unwrap();
            assert_eq!(
                offer.capability,
                format!(
                    "cap://greentic/events/{}/{}",
                    clean_capability_segment(pack_name),
                    clean_capability_segment(event_suffix)
                ),
                "capability URI mismatch for {event_suffix}"
            );
            assert!(
                offer
                    .contracts
                    .contains(&"greentic.sorx.business-event-topic.v1".to_string()),
                "lifecycle offer for {event_suffix} must declare the business-event-topic contract"
            );
        }
    }

    #[test]
    fn capability_offers_command_topics_include_topic_field() {
        let runtime = runtime("local");
        let pack_name = &runtime.runtime.pack.name;

        let offers = runtime.business_event_topic_offers();

        // The fixture command emits "tenant.code_generated" — verify the offer
        // now carries a "topic" field equal to command_event_topic().
        let expected_topic =
            greentic_sorx_core::command_event_topic(pack_name, "tenant.code_generated");
        let command_offer = offers.iter().find(|offer| {
            offer
                .metadata
                .as_ref()
                .is_some_and(|meta| meta["event_type"] == "tenant.code_generated")
        });
        assert!(
            command_offer.is_some(),
            "expected a command-event offer for tenant.code_generated"
        );
        assert_eq!(
            command_offer.unwrap().metadata.as_ref().unwrap()["topic"],
            expected_topic,
            "command-event offer must carry the canonical topic string"
        );
    }

    #[test]
    fn response_carries_custom_headers() {
        let resp = json_response(401, serde_json::json!({"ok": false}))
            .with_header("WWW-Authenticate", "Bearer x");
        assert_eq!(resp.status, 401);
        assert!(resp
            .headers
            .iter()
            .any(|(k, v)| k == "WWW-Authenticate" && v == "Bearer x"));
    }
}
