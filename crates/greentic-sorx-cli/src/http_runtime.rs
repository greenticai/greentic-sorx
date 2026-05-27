use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, RwLock};

use greentic_sorx_core::{
    AdminActionRequest, AdminActionResponse, AdminObserverEvent, AdminSurface, CallerContext,
    ControlDecisionAction, EndpointDefinition, EndpointInvocation, EndpointMethod, EndpointRouter,
    EndpointStatus, FoundationDbProviderAdapter, FoundationDbProviderConfig, InvocationSource,
    McpToolDefinition, McpToolList, MemoryStoreProvider, MetricAggregate, MetricQuery,
    MetricQueryFilter, MetricQueryResult, MetricResultRow, MetricRuntime, MetricRuntimeProvider,
    OperationKind, PolicyAction, ProviderNamespace, ProviderRegistry, QueryOp, RiskLevel,
    RuntimeConfig, RuntimeInfo, RuntimeMetric, RuntimeMetricCache, RuntimeMetricCatalog,
    RuntimeMetricDimension, RuntimeMetricKind, RuntimeOperationalIndex, RuntimePack,
    RuntimeSnapshot, SorxDeployment, SorxError, SorxResult, SorxRuntime, SorxRuntimeConfig,
    StageDeploymentRequest, StdoutAuditSink, StoreProviderKind, TrafficUpdateRequest,
    apply_value_patch,
};
use greentic_sorx_pack::{
    BusinessAction, BusinessActionAssets, LoadedSorlaPack, MetricDefinition, contract_hash,
};
use serde::Serialize;
use serde_json::{Map, Value, json};

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
    metrics: Arc<Option<RuntimeMetricCatalog>>,
    metric_provider: Arc<StoreMetricProvider>,
    runtime_snapshot: Arc<RwLock<RuntimeSnapshot>>,
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
        let router = EndpointRouter::from_agent_gateway(&pack.sorla_assets.agent_gateway_json)?;
        let providers = provider_registry(&config)?;
        let runtime = configure_runtime_audit(
            SorxRuntime::new(
                RuntimePack {
                    name: pack.pack_name.clone(),
                    version: pack.pack_version.clone(),
                    digest: pack.pack_digest.clone(),
                    operational_indexes: runtime_operational_indexes(pack),
                },
                config.clone(),
                router,
                providers,
            ),
            &config,
        );
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
            metrics: Arc::new(metrics),
            metric_provider: Arc::new(metric_provider),
            runtime_snapshot: Arc::new(RwLock::new(runtime_snapshot)),
        })
    }

    #[cfg(test)]
    fn route_list(&self) -> &RouteList {
        &self.routes
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
            return error_response(
                501,
                "SORX_ADMIN_API_NOT_IMPLEMENTED",
                "admin API storage is provided by the CLI registry in this build",
            );
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
        let roles = request
            .headers
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
            .unwrap_or_else(|| vec!["local".to_string()]);

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
                    serde_json::to_value(
                        greentic_sorx_core::RuntimeCapabilities::sorx_runtime_host(),
                    )
                    .unwrap(),
                );
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

fn is_admin_api_path(path: &str) -> bool {
    path == "/v1/sorx/deployments"
        || path == "/v1/sorx/aliases"
        || path.starts_with("/v1/sorx/deployments/")
        || path.starts_with("/v1/sorx/aliases/")
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
        cache: metric.cache.as_ref().map(|cache| RuntimeMetricCache {
            ttl_seconds: cache.ttl_seconds,
            scope: cache.scope.clone(),
        }),
    })
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
                            .unwrap_or("eq")
                            .to_string(),
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
        let records = provider.query(QueryOp {
            namespace: query.namespace.clone(),
            entity: source_entity.clone(),
            collection: collection.clone(),
            filter: equality_filter(query),
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
            .filter(|record| metric_filters_match(&record.data, &query.filters))
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
        if groups.is_empty() {
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

fn equality_filter(query: &MetricQuery) -> Value {
    let mut filter = Map::new();
    for metric_filter in &query.filters {
        if metric_filter.operator == "eq" && !metric_filter.field.contains('.') {
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
        match filter.operator.as_str() {
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
}

fn json_response(status: u16, body: Value) -> HttpResponse {
    HttpResponse { status, body }
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
    fn as_bytes(&self) -> Vec<u8> {
        let body = serde_json::to_vec(&self.body).unwrap_or_else(|_| b"{}".to_vec());
        let reason = match self.status {
            200 => "OK",
            400 => "Bad Request",
            403 => "Forbidden",
            404 => "Not Found",
            _ => "Internal Server Error",
        };
        let mut response = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.status,
            reason,
            body.len()
        )
        .into_bytes();
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
        ControlDecision, ControlHook, MemoryAuditSink, ObserverHook, default_start_schema,
        normalize_start_answers, runtime_config_from_answers,
    };
    use greentic_sorx_pack::{
        BusinessAction, BusinessActionAssets, BusinessActionCatalog, BusinessActionExecution,
        BusinessActionIdempotency, BusinessActionLock, BusinessActionLockEntry, BusinessActionRisk,
        LoadedSorlaPack, MetricAssets, MetricCatalog, PackIdentity, PackManifest, SorlaAssets,
        SorxAssets, ValidationSuiteStatus, contract_hash,
    };
    use serde_json::{Value, json};

    use super::*;

    const GENERIC_RUNTIME_CONFIG: &str =
        include_str!("../tests/e2e/fixtures/generic_runtime_host/runtime-config.json");

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
}
