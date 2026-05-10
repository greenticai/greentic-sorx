use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

use greentic_sorx_core::{
    CallerContext, EndpointDefinition, EndpointInvocation, EndpointMethod, EndpointRouter,
    EndpointStatus, FoundationDbProviderAdapter, FoundationDbProviderConfig, InvocationSource,
    McpToolList, MemoryStoreProvider, ProviderRegistry, RuntimePack, SorxError, SorxResult,
    SorxRuntime, SorxRuntimeConfig, StdoutAuditSink, StoreProviderKind,
};
use greentic_sorx_pack::LoadedSorlaPack;
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
    pub exposure: String,
}

#[derive(Clone)]
pub struct HttpRuntime {
    deployment_id: String,
    admin_api_enabled: bool,
    runtime: Arc<SorxRuntime>,
    routes: Arc<RouteList>,
    tools: Arc<McpToolList>,
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
        let routes = route_list(&deployment_id, &exposure, pack, &runtime.router);
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

        let input = match request_json(&request, &path_params) {
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
                registry.register_store(binding, Arc::new(MemoryStoreProvider::new()))
            }
            StoreProviderKind::FoundationDb => {
                registry.register_store(
                    binding,
                    Arc::new(FoundationDbProviderAdapter::unavailable(
                        FoundationDbProviderConfig::from_parts(
                            provider.config_ref.clone(),
                            provider.config.clone(),
                        ),
                    )),
                );
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
                exposure: exposure.to_string(),
            })
            .collect(),
    }
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
        object.insert(key.clone(), Value::String(value.clone()));
    }
    if path_params.len() == 1
        && !object.contains_key("id")
        && let Some((_, value)) = path_params.iter().next()
    {
        object.insert("id".to_string(), Value::String(value.clone()));
    }
    Ok(Value::Object(object))
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
        LoadedSorlaPack, PackIdentity, PackManifest, SorlaAssets, SorxAssets, ValidationSuiteStatus,
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
    fn create_get_update_and_query_tenant_via_http() {
        let runtime = runtime("local");
        let created = request(
            &runtime,
            "POST",
            "/v1/agent/tenants/create",
            &tenant_headers(),
            r#"{"id":"tenant-1","name":"Acme","active":true}"#,
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
