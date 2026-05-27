use std::sync::Arc;
use std::time::Instant;

use greentic_sorx_core::{
    CallerContext, EndpointDefinition, EndpointInvocation, EndpointMethod, EndpointRouter,
    EndpointStatus, InvocationSource, MemoryAuditSink, MemoryStoreProvider, ProviderRegistry,
    RuntimePack, SorxRuntime, SorxRuntimeConfig,
};
use greentic_sorx_pack::{LoadedSorlaPack, ValidationSuiteStatus, doctor_sorla_pack};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderMode {
    InMemory,
    Configured,
    Mock,
}

impl ProviderMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "in-memory" => Some(Self::InMemory),
            "configured" => Some(Self::Configured),
            "mock" => Some(Self::Mock),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationOptions {
    pub deployment_id: String,
    pub provider_mode: ProviderMode,
    pub preserve_state_on_failure: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationSuite {
    pub schema: String,
    #[serde(default)]
    pub suite_id: String,
    #[serde(default)]
    pub pack_name: String,
    #[serde(default)]
    pub pack_version: String,
    #[serde(default)]
    pub requires: Value,
    #[serde(default)]
    pub gates: ValidationGates,
    #[serde(default)]
    pub tests: Vec<ValidationTest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationGates {
    #[serde(default)]
    pub required_for_private_activation: bool,
    #[serde(default)]
    pub required_for_public_exposure: bool,
    #[serde(default = "default_minimum_pass_level")]
    pub minimum_pass_level: ValidationLevel,
}

impl Default for ValidationGates {
    fn default() -> Self {
        Self {
            required_for_private_activation: false,
            required_for_public_exposure: false,
            minimum_pass_level: ValidationLevel::Required,
        }
    }
}

fn default_minimum_pass_level() -> ValidationLevel {
    ValidationLevel::Required
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationLevel {
    #[default]
    Required,
    Recommended,
    Informational,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationTest {
    pub id: String,
    pub kind: ValidationTestKind,
    #[serde(default)]
    pub level: ValidationLevel,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub input_fixture: Option<String>,
    #[serde(default)]
    pub expect: Option<ValidationExpect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationTestKind {
    Doctor,
    ArtifactExists,
    ArtifactSchema,
    RouteGeneration,
    ProviderContract,
    EndpointCall,
    NegativeEndpointCall,
    AuditEventEmitted,
    Idempotency,
    PolicyDenial,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationExpect {
    #[serde(default)]
    pub status: Option<u16>,
    #[serde(default)]
    pub json_path: Option<String>,
    #[serde(default)]
    pub equals: Option<Value>,
    #[serde(default)]
    pub event: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub schema: String,
    pub deployment_id: String,
    pub pack_name: String,
    pub pack_version: String,
    pub pack_digest: String,
    pub suite_id: String,
    pub started_at: String,
    pub finished_at: String,
    pub result: ValidationResult,
    pub public_exposure_allowed: bool,
    pub tests: Vec<ValidationTestReport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationResult {
    Pass,
    Fail,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationTestReport {
    pub id: String,
    pub result: ValidationResult,
    pub level: ValidationLevel,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub code: String,
    pub message: String,
}

impl ValidationError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ValidationError {}

pub fn execute_validation_suite(
    pack: &LoadedSorlaPack,
    config: &SorxRuntimeConfig,
    options: &ValidationOptions,
) -> Result<ValidationReport, ValidationError> {
    let suite = load_suite(pack)?;
    let runtime = validation_runtime(pack, config, &options.provider_mode)?;
    let audit = MemoryAuditSink::new();
    let runtime = runtime.with_audit_sink(Arc::new(audit.clone()));
    let started_at = "1970-01-01T00:00:00Z".to_string();
    let tests = suite
        .tests
        .iter()
        .map(|test| run_test(pack, config, &runtime, &audit, test))
        .collect::<Vec<_>>();
    let required_failed = tests.iter().any(|test| {
        test.level == ValidationLevel::Required && test.result != ValidationResult::Pass
    });
    let result = if required_failed {
        ValidationResult::Fail
    } else {
        ValidationResult::Pass
    };
    let public_exposure_allowed = result == ValidationResult::Pass
        && (!suite.gates.required_for_public_exposure || !required_failed);

    Ok(ValidationReport {
        schema: "greentic.sorx.validation-report.v1".to_string(),
        deployment_id: options.deployment_id.clone(),
        pack_name: pack.pack_name.clone(),
        pack_version: pack.pack_version.clone(),
        pack_digest: pack.pack_digest.clone().unwrap_or_default(),
        suite_id: suite.suite_id,
        started_at: started_at.clone(),
        finished_at: started_at,
        result,
        public_exposure_allowed,
        tests,
    })
}

pub fn missing_suite_report(
    pack: &LoadedSorlaPack,
    deployment_id: impl Into<String>,
    require_for_public: bool,
) -> ValidationReport {
    ValidationReport {
        schema: "greentic.sorx.validation-report.v1".to_string(),
        deployment_id: deployment_id.into(),
        pack_name: pack.pack_name.clone(),
        pack_version: pack.pack_version.clone(),
        pack_digest: pack.pack_digest.clone().unwrap_or_default(),
        suite_id: "missing".to_string(),
        started_at: "1970-01-01T00:00:00Z".to_string(),
        finished_at: "1970-01-01T00:00:00Z".to_string(),
        result: ValidationResult::Skip,
        public_exposure_allowed: !require_for_public,
        tests: Vec::new(),
    }
}

fn load_suite(pack: &LoadedSorlaPack) -> Result<ValidationSuite, ValidationError> {
    if pack.validation_suite_status == ValidationSuiteStatus::Missing {
        return Err(ValidationError::new(
            "validation_suite_missing",
            "pack does not contain a validation suite",
        ));
    }
    if let Some(bytes) = &pack.sorx_assets.validation_suite_cbor {
        return ciborium::de::from_reader(bytes.as_slice()).map_err(|err| {
            ValidationError::new(
                "validation_suite_invalid",
                format!("validation-suite.cbor is invalid: {err}"),
            )
        });
    }
    let Some(value) = &pack.sorx_assets.validation_suite_json else {
        return Err(ValidationError::new(
            "validation_suite_missing",
            "pack does not contain validation-suite.json",
        ));
    };
    serde_json::from_value(value.clone()).map_err(|err| {
        ValidationError::new(
            "validation_suite_invalid",
            format!("validation-suite.json is invalid: {err}"),
        )
    })
}

fn validation_runtime(
    pack: &LoadedSorlaPack,
    config: &SorxRuntimeConfig,
    provider_mode: &ProviderMode,
) -> Result<SorxRuntime, ValidationError> {
    if matches!(provider_mode, ProviderMode::Configured) {
        return Err(ValidationError::new(
            "provider_mode_unavailable",
            "configured provider validation is not wired in this local runner yet",
        ));
    }
    let router = EndpointRouter::from_agent_gateway(&pack.sorla_assets.agent_gateway_json)
        .map_err(|err| ValidationError::new(err.code, err.message))?;
    let mut providers = ProviderRegistry::new();
    for provider_id in config.providers.keys() {
        providers.register_canonical_store(provider_id, Arc::new(MemoryStoreProvider::new()));
    }
    Ok(SorxRuntime::new(
        RuntimePack {
            name: pack.pack_name.clone(),
            version: pack.pack_version.clone(),
            digest: pack.pack_digest.clone(),
            operational_indexes: pack
                .sorla_assets
                .operational_indexes
                .as_ref()
                .map(|assets| {
                    assets
                        .catalog
                        .indexes
                        .iter()
                        .filter(|index| index.unique)
                        .map(|index| greentic_sorx_core::RuntimeOperationalIndex {
                            id: index.id.clone(),
                            record: index.record.clone(),
                            collection: index.collection.clone(),
                            kind: index.kind.clone(),
                            fields: index.fields.clone(),
                            unique: index.unique,
                        })
                        .collect()
                })
                .unwrap_or_default(),
        },
        config.clone(),
        router,
        providers,
    ))
}

fn run_test(
    pack: &LoadedSorlaPack,
    config: &SorxRuntimeConfig,
    runtime: &SorxRuntime,
    audit: &MemoryAuditSink,
    test: &ValidationTest,
) -> ValidationTestReport {
    let started = Instant::now();
    let result = match test.kind {
        ValidationTestKind::Doctor => doctor(pack),
        ValidationTestKind::ArtifactExists => artifact_exists(pack, test),
        ValidationTestKind::ArtifactSchema => artifact_exists(pack, test),
        ValidationTestKind::RouteGeneration => route_generation(runtime),
        ValidationTestKind::ProviderContract => provider_contract(config),
        ValidationTestKind::EndpointCall => endpoint_call(pack, runtime, test, false),
        ValidationTestKind::NegativeEndpointCall => endpoint_call(pack, runtime, test, true),
        ValidationTestKind::AuditEventEmitted => audit_event_emitted(audit, test),
        ValidationTestKind::Idempotency => idempotency(pack, runtime, test),
        ValidationTestKind::PolicyDenial => policy_denial(pack, runtime, test),
    };
    let (result, message) = match result {
        Ok(()) => (ValidationResult::Pass, None),
        Err(message) => (ValidationResult::Fail, Some(message)),
    };
    ValidationTestReport {
        id: test.id.clone(),
        result,
        level: test.level,
        duration_ms: started.elapsed().as_millis() as u64,
        message,
    }
}

fn doctor(pack: &LoadedSorlaPack) -> Result<(), String> {
    let report = doctor_sorla_pack(&pack.pack_path);
    if report.ok {
        Ok(())
    } else {
        Err(report
            .errors
            .first()
            .map(|issue| issue.message.clone())
            .unwrap_or_else(|| "doctor failed".to_string()))
    }
}

fn artifact_exists(pack: &LoadedSorlaPack, test: &ValidationTest) -> Result<(), String> {
    let path = test
        .path
        .as_deref()
        .ok_or_else(|| "artifact test is missing path".to_string())?;
    if pack.entries.contains(path) {
        Ok(())
    } else {
        Err(format!("artifact `{path}` is missing"))
    }
}

fn route_generation(runtime: &SorxRuntime) -> Result<(), String> {
    if runtime.router.endpoints.is_empty() {
        Err("route table is empty".to_string())
    } else {
        Ok(())
    }
}

fn provider_contract(config: &SorxRuntimeConfig) -> Result<(), String> {
    if config.providers.is_empty() {
        Err("no providers configured".to_string())
    } else {
        Ok(())
    }
}

fn endpoint_call(
    pack: &LoadedSorlaPack,
    runtime: &SorxRuntime,
    test: &ValidationTest,
    negative: bool,
) -> Result<(), String> {
    let endpoint = endpoint_for(runtime, test)?;
    let input = fixture_input(pack, test)?;
    let response = invoke_endpoint(runtime, endpoint, input);
    let status = response["status"].as_u64().unwrap_or(500) as u16;
    let expected_status = test
        .expect
        .as_ref()
        .and_then(|expect| expect.status)
        .unwrap_or(if negative { 400 } else { 200 });
    if status != expected_status {
        return Err(format!("expected status {expected_status}, got {status}"));
    }
    assert_expectation(&response, test)
}

fn idempotency(
    pack: &LoadedSorlaPack,
    runtime: &SorxRuntime,
    test: &ValidationTest,
) -> Result<(), String> {
    let endpoint = endpoint_for(runtime, test)?;
    let input = fixture_input(pack, test)?;
    let first = invoke_with_key(
        runtime,
        endpoint,
        input.clone(),
        Some("validation-idempotency"),
    );
    let second = invoke_with_key(runtime, endpoint, input, Some("validation-idempotency"));
    if first["result"] == second["result"] {
        Ok(())
    } else {
        Err("idempotent invocation returned different results".to_string())
    }
}

fn policy_denial(
    pack: &LoadedSorlaPack,
    runtime: &SorxRuntime,
    test: &ValidationTest,
) -> Result<(), String> {
    let endpoint = endpoint_for(runtime, test)?;
    let input = fixture_input(pack, test)?;
    let response = invoke_endpoint(runtime, endpoint, input);
    let status = response["status"].as_u64().unwrap_or(500);
    if status == 403 || status == 202 {
        Ok(())
    } else {
        Err(format!(
            "expected policy denial or approval requirement, got {status}"
        ))
    }
}

fn audit_event_emitted(audit: &MemoryAuditSink, test: &ValidationTest) -> Result<(), String> {
    let expected = test
        .expect
        .as_ref()
        .and_then(|expect| expect.event.as_deref())
        .unwrap_or("sorx.endpoint.completed");
    let events = audit.events().map_err(|err| err.message)?;
    if events.iter().any(|event| event.event == expected) {
        Ok(())
    } else {
        Err(format!("audit event `{expected}` was not emitted"))
    }
}

fn endpoint_for<'a>(
    runtime: &'a SorxRuntime,
    test: &ValidationTest,
) -> Result<&'a EndpointDefinition, String> {
    let method = test.method.as_deref().unwrap_or("POST");
    let path = test
        .path
        .as_deref()
        .ok_or_else(|| "endpoint test is missing path".to_string())?;
    runtime
        .router
        .endpoints
        .values()
        .find(|endpoint| method_string(endpoint.method) == method && endpoint.path == path)
        .ok_or_else(|| format!("no endpoint route matches {method} {path}"))
}

fn fixture_input(pack: &LoadedSorlaPack, test: &ValidationTest) -> Result<Value, String> {
    let Some(path) = &test.input_fixture else {
        return Ok(json!({}));
    };
    pack.sorx_assets
        .validation_fixtures_json
        .get(path)
        .cloned()
        .ok_or_else(|| format!("fixture `{path}` is missing or invalid"))
}

fn invoke_endpoint(runtime: &SorxRuntime, endpoint: &EndpointDefinition, input: Value) -> Value {
    invoke_with_key(runtime, endpoint, input, None)
}

fn invoke_with_key(
    runtime: &SorxRuntime,
    endpoint: &EndpointDefinition,
    input: Value,
    idempotency_key: Option<&str>,
) -> Value {
    match runtime.invoke(EndpointInvocation {
        tenant_id: runtime.config.tenant_id.clone(),
        endpoint_id: endpoint.endpoint_id.clone(),
        operation_id: endpoint.operation_id.clone(),
        input,
        caller: CallerContext {
            subject: "validation-suite".to_string(),
            roles: vec!["validator".to_string()],
        },
        idempotency_key: idempotency_key.map(ToString::to_string),
        source: InvocationSource::Direct,
    }) {
        Ok(result) => json!({
            "status": status_code(&result.status),
            "ok": matches!(
                result.status,
                EndpointStatus::Created | EndpointStatus::Ok | EndpointStatus::Deleted
            ),
            "result": result.output,
            "events": result.events
        }),
        Err(err) => json!({
            "status": 400,
            "ok": false,
            "error": {
                "code": err.code,
                "message": err.message,
                "path": err.path
            }
        }),
    }
}

fn assert_expectation(response: &Value, test: &ValidationTest) -> Result<(), String> {
    let Some(expect) = &test.expect else {
        return Ok(());
    };
    let Some(path) = &expect.json_path else {
        return Ok(());
    };
    let actual =
        json_path(response, path).ok_or_else(|| format!("json path `{path}` not found"))?;
    if let Some(expected) = &expect.equals
        && actual != expected
    {
        return Err(format!(
            "json path `{path}` expected {expected}, got {actual}"
        ));
    }
    Ok(())
}

fn json_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in path.strip_prefix("$.")?.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

fn status_code(status: &EndpointStatus) -> u16 {
    match status {
        EndpointStatus::Created | EndpointStatus::Ok | EndpointStatus::Deleted => 200,
        EndpointStatus::NotFound => 404,
        EndpointStatus::ApprovalRequired => 202,
        EndpointStatus::Denied => 403,
    }
}

fn method_string(method: EndpointMethod) -> &'static str {
    match method {
        EndpointMethod::Get => "GET",
        EndpointMethod::Post => "POST",
        EndpointMethod::Put => "PUT",
        EndpointMethod::Patch => "PATCH",
        EndpointMethod::Delete => "DELETE",
    }
}
