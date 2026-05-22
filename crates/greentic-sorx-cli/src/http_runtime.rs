use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

use greentic_sorx_core::{
    CallerContext, EndpointDefinition, EndpointInvocation, EndpointMethod, EndpointRouter,
    EndpointStatus, FoundationDbProviderAdapter, FoundationDbProviderConfig, InvocationSource,
    McpToolList, MemoryStoreProvider, PolicyAction, ProviderRegistry, RuntimePack, SorxDeployment,
    SorxError, SorxResult, SorxRuntime, SorxRuntimeConfig, StdoutAuditSink, StoreProviderKind,
};
use greentic_sorx_pack::{BusinessAction, BusinessActionAssets, LoadedSorlaPack, contract_hash};
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
    runtime: Arc<SorxRuntime>,
    routes: Arc<RouteList>,
    tools: Arc<McpToolList>,
    business_actions: Arc<Option<BusinessActionAssets>>,
}

impl HttpRuntime {
    pub fn from_pack(
        deployment_id: impl Into<String>,
        pack: &LoadedSorlaPack,
        config: SorxRuntimeConfig,
    ) -> SorxResult<Self> {
        let router = EndpointRouter::from_agent_gateway(&pack.sorla_assets.agent_gateway_json)?;
        let providers = provider_registry(&config)?;
        let runtime = configure_runtime_audit(
            SorxRuntime::new(
                RuntimePack {
                    name: pack.pack_name.clone(),
                    version: pack.pack_version.clone(),
                    digest: pack.pack_digest.clone(),
                },
                config.clone(),
                router,
                providers,
            ),
            &config,
        );
        let deployment_id = deployment_id.into();
        let exposure = config.exposure.default_visibility.clone();
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
        Ok(Self {
            deployment_id,
            admin_api_enabled: false,
            runtime: Arc::new(runtime),
            routes: Arc::new(routes),
            tools: Arc::new(tools),
            business_actions: Arc::new(pack.sorla_assets.business_actions.clone()),
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

#[derive(Debug)]
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

    use greentic_sorx_core::{
        default_start_schema, normalize_start_answers, runtime_config_from_answers,
    };
    use greentic_sorx_pack::{
        BusinessAction, BusinessActionAssets, BusinessActionCatalog, BusinessActionExecution,
        BusinessActionIdempotency, BusinessActionLock, BusinessActionLockEntry, BusinessActionRisk,
        LoadedSorlaPack, PackIdentity, PackManifest, SorlaAssets, SorxAssets,
        ValidationSuiteStatus, contract_hash,
    };
    use serde_json::{Value, json};

    use super::*;

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
        let pack = pack();
        let normalized =
            normalize_start_answers(&default_start_schema(), &answers(environment), true).unwrap();
        let config = runtime_config_from_answers(&pack.pack_name, &normalized.answers).unwrap();
        HttpRuntime::from_pack("local", &pack, config).unwrap()
    }

    fn request(
        runtime: &HttpRuntime,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Value {
        runtime
            .handle_request(HttpRequest {
                method: method.to_string(),
                path: path.to_string(),
                headers: headers
                    .iter()
                    .map(|(key, value)| (key.to_ascii_lowercase(), (*value).to_string()))
                    .collect(),
                body: body.to_string(),
            })
            .body
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
