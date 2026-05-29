use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use greentic_sorx_core::default_start_schema;
use greentic_sorx_pack::{
    BusinessAction, BusinessActionCatalog, BusinessActionExecution, BusinessActionIdempotency,
    BusinessActionLock, BusinessActionLockEntry, BusinessActionRisk, PackIdentity, PackLock,
    PackLockEntry, PackManifest as SorxPackManifest, contract_hash,
};
use greentic_types::{
    PackId, PackKind as GpackKind, PackManifest as GpackManifest, PackSignatures,
    encode_pack_manifest,
};
use semver::Version;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const AGENT_GATEWAY: &str = include_str!("e2e/fixtures/landlord_tenant/agent-gateway.json");
const MCP_TOOLS: &str = include_str!("e2e/fixtures/landlord_tenant/mcp-tools.json");
const ANSWERS_MEMORY: &str = include_str!("e2e/fixtures/landlord_tenant/answers.memory.json");
const ANSWERS_FOUNDATIONDB: &str =
    include_str!("e2e/fixtures/landlord_tenant/answers.foundationdb.json");
const EXPECTED_ROUTES: &str = include_str!("e2e/fixtures/landlord_tenant/expected-routes.json");
const SEED_DATA: &str = include_str!("e2e/fixtures/landlord_tenant/seed-data.json");

fn greentic_sorx_command() -> Command {
    // Test-only helper invokes Cargo's compiled test binary path.
    // foxguard: ignore[rs/no-command-injection]
    Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
}

#[test]
fn landlord_tenant_memory_e2e_runs_from_gtpack() {
    let fixture = LandlordTenantFixture::new();

    let doctor = greentic_sorx_command()
        .args(["doctor", fixture.pack.to_str().unwrap()])
        .output()
        .expect("doctor should run");
    assert!(doctor.status.success(), "{doctor:?}");

    let inspect = greentic_sorx_command()
        .args(["inspect", fixture.pack.to_str().unwrap(), "--json"])
        .output()
        .expect("inspect should run");
    assert!(inspect.status.success(), "{inspect:?}");
    let inspect_json: Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(inspect_json["business_actions"]["present"], true);
    assert_eq!(inspect_json["business_actions"]["count"], 1);
    assert_eq!(inspect_json["business_actions"]["lock_present"], true);

    let mcp_tools = greentic_sorx_command()
        .args(["mcp-tools", fixture.pack.to_str().unwrap()])
        .output()
        .expect("mcp-tools should run");
    assert!(mcp_tools.status.success(), "{mcp_tools:?}");
    let tools: Value = serde_json::from_slice(&mcp_tools.stdout).unwrap();
    assert_eq!(tools["tools"].as_array().unwrap().len(), 3);

    let port = free_port();
    let answers = fixture.write_memory_answers(port);
    let _server = ServerProcess::start(&fixture.pack, &answers, port);
    let client = HttpClient { port };

    assert_eq!(client.get("/healthz").status, 200);
    assert_eq!(client.get("/readyz").status, 200);

    let routes = client.get_json("/v1/sorx/routes");
    let route_paths = routes["routes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|route| route["path"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    let expected: Value = serde_json::from_str(EXPECTED_ROUTES).unwrap();
    for expected_path in expected["routes"].as_array().unwrap() {
        assert!(route_paths.contains(&expected_path.as_str().unwrap().to_string()));
    }

    let seed: Value = serde_json::from_str(SEED_DATA).unwrap();
    let landlord = client.post_json("/v1/agent/landlords/create", &seed["landlord"], None);
    assert_created_event(&landlord, "landlord.create", "entity.created");

    let property = client.post_json("/v1/agent/properties/create", &seed["property"], None);
    assert_created_event(&property, "property.create", "entity.created");

    let unit = client.post_json("/v1/agent/units/create", &seed["unit"], None);
    assert_created_event(&unit, "unit.create", "entity.created");

    let tenant = client.post_json(
        "/v1/agent/tenants/create",
        &seed["tenant"],
        Some("tenant-create-once"),
    );
    assert_eq!(tenant["result"]["id"], "tenant-1");
    let repeated_tenant = client.post_json(
        "/v1/agent/tenants/create",
        &json!({
            "id": "tenant-changed",
            "landlord_id": "landlord-1",
            "name": "Changed",
            "active": "true"
        }),
        Some("tenant-create-once"),
    );
    assert_eq!(repeated_tenant["result"]["id"], "tenant-1");
    assert_eq!(repeated_tenant["result"]["data"]["name"], "Avery Stone");

    let tenancy = client.post_json("/v1/agent/tenancies/assign", &seed["tenancy"], None);
    assert_created_event(&tenancy, "tenancy.assign", "entity.created");

    let payment = client.post_json(
        "/v1/agent/payments/record",
        &seed["payment"],
        Some("payment-record-once"),
    );
    assert_eq!(payment["result"]["id"], "payment-1");
    let repeated_payment = client.post_json(
        "/v1/agent/payments/record",
        &json!({
            "id": "payment-changed",
            "tenancy_id": "tenancy-1",
            "amount_cents": 999,
            "status": "changed"
        }),
        Some("payment-record-once"),
    );
    assert_eq!(repeated_payment["result"]["id"], "payment-1");
    assert_eq!(repeated_payment["result"]["data"]["amount_cents"], 125000);

    let maintenance = client.post_json("/v1/agent/maintenance/open", &seed["maintenance"], None);
    assert_created_event(&maintenance, "maintenance.open", "entity.created");

    let active = client.get_json("/v1/agent/landlords/landlord-1/active-tenants/true");
    assert_eq!(active["result"]["records"].as_array().unwrap().len(), 1);
    assert_eq!(active["result"]["records"][0]["id"], "tenant-1");

    let updated = client.patch_json(
        "/v1/agent/tenants/tenant-1",
        &json!({ "patch": { "preferred_contact": "sms" } }),
    );
    assert_eq!(updated["result"]["data"]["preferred_contact"], "sms");

    let reread_tenant = client.get_json("/v1/agent/tenants/tenant-1");
    assert_eq!(reread_tenant["result"]["data"]["preferred_contact"], "sms");
    let reread_tenancy = client.get_json("/v1/agent/tenancies/tenancy-1");
    assert_eq!(reread_tenancy["result"]["data"]["tenant_id"], "tenant-1");
    let reread_landlord = client.get_json("/v1/agent/landlords/landlord-1");
    assert_eq!(
        reread_landlord["result"]["data"]["name"],
        "Green Acres Holdings"
    );
    let reread_payment = client.get_json("/v1/agent/payments/payment-1");
    assert_eq!(reread_payment["result"]["data"]["status"], "recorded");
    let reread_maintenance = client.get_json("/v1/agent/maintenance/maintenance-1");
    assert_eq!(reread_maintenance["result"]["data"]["status"], "open");

    let refund = client.post_json_raw("/v1/agent/payments/payment-1/refund", &json!({}), None);
    assert_eq!(refund.status, 202);
    assert_eq!(refund.body["status"], "approval_required");

    let business_actions = client.get_json("/v1/sorx/business-actions");
    assert_eq!(business_actions["actions"][0]["id"], "record_rent_payment");

    let action = client.get_json("/v1/sorx/business-actions/record_rent_payment/versions/0.1.0");
    let contract_hash = action["contract_hash"]
        .as_str()
        .or_else(|| business_actions["actions"][0]["contract_hash"].as_str())
        .unwrap_or(BUSINESS_ACTION_HASH_SENTINEL);
    assert_ne!(contract_hash, BUSINESS_ACTION_HASH_SENTINEL);

    let business_payment = json!({
        "action_ref": { "contract_hash": contract_hash },
        "values": {
            "id": "payment-business-1",
            "tenancy_id": "tenancy-1",
            "amount_cents": 125000,
            "status": "recorded"
        },
        "options": { "idempotency_key": "business-payment-once" }
    });
    let dry_run = client.post_json_raw(
        "/v1/sorx/business-actions/record_rent_payment/versions/0.1.0/dry-run",
        &business_payment,
        None,
    );
    assert_eq!(dry_run.status, 200);
    assert_eq!(dry_run.body["valid"], true);
    let missing_after_dry_run = client.get_json("/v1/agent/payments/payment-business-1");
    assert!(missing_after_dry_run["result"].is_null());

    let invoked = client.post_json(
        "/v1/sorx/business-actions/record_rent_payment/versions/0.1.0/invoke",
        &business_payment,
        None,
    );
    assert_eq!(invoked["action_ref"]["id"], "record_rent_payment");
    assert_eq!(invoked["result"]["id"], "payment-business-1");
    assert_eq!(invoked["audit"]["validation_result"], "passed");

    let wrong_hash = client.post_json_raw(
        "/v1/sorx/business-actions/record_rent_payment/versions/0.1.0/invoke",
        &json!({
            "action_ref": { "contract_hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000" },
            "values": {
                "id": "payment-business-2",
                "tenancy_id": "tenancy-1",
                "amount_cents": 125000,
                "status": "recorded"
            },
            "options": { "idempotency_key": "business-payment-wrong-hash" }
        }),
        None,
    );
    assert_eq!(wrong_hash.status, 409);
    assert_eq!(wrong_hash.body["error"]["code"], "contract_hash_mismatch");
}

const BUSINESS_ACTION_HASH_SENTINEL: &str = "missing";

struct LandlordTenantFixture {
    temp: TempDir,
    pack: std::path::PathBuf,
}

impl LandlordTenantFixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let pack = temp.path().join("landlord-tenant.gtpack");
        let mut entries = pack_entries()
            .into_iter()
            .collect::<std::collections::BTreeMap<_, _>>();
        if let Some(manifest) = entries.get("pack.cbor").cloned() {
            entries.insert("manifest.cbor".to_string(), gpack_manifest_bytes());
            entries.insert(
                "manifest.json".to_string(),
                serde_json::to_vec_pretty(
                    &ciborium::de::from_reader::<SorxPackManifest, _>(manifest.as_slice()).unwrap(),
                )
                .unwrap(),
            );
        }
        entries.insert(
            "pack.lock.cbor".to_string(),
            encode_cbor(&lock_for_entries(&entries)),
        );
        let file = std::fs::File::create(&pack).unwrap();
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .unix_permissions(0o644);
        for (name, bytes) in entries {
            writer.start_file(name, options).unwrap();
            writer.write_all(&bytes).unwrap();
        }
        writer.finish().unwrap();
        Self { temp, pack }
    }

    fn write_memory_answers(&self, port: u16) -> std::path::PathBuf {
        let mut answers: Value = serde_json::from_str(ANSWERS_MEMORY).unwrap();
        answers["server"]["bind"] = json!(format!("127.0.0.1:{port}"));
        answers["server"]["public_base_url"] = json!(format!("http://127.0.0.1:{port}"));
        let path = self.temp.path().join("landlord-tenant.answers.memory.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&answers).unwrap()).unwrap();
        path
    }
}

fn pack_entries() -> Vec<(String, Vec<u8>)> {
    let business_action = business_action();
    let business_action_lock = BusinessActionLock {
        schema: "greentic.sorla.business-actions.lock.v1".to_string(),
        entries: vec![BusinessActionLockEntry {
            id: business_action.id.clone(),
            version: business_action.version.clone(),
            contract_hash: contract_hash(&business_action),
        }],
    };
    let manifest = SorxPackManifest {
        schema: "greentic.gtpack.manifest.sorla.v1".to_string(),
        pack: PackIdentity {
            name: "landlord-tenant-sor".to_string(),
            version: "0.1.0".to_string(),
            kind: Some("application".to_string()),
        },
        extension: json!({
            "extension": "greentic.sorx.runtime.v1",
            "sorla": {
                "model": "assets/sorla/model.cbor",
                "agent_gateway": "assets/sorla/agent-gateway.json",
                "mcp_tools": "assets/sorla/mcp-tools.json",
                "business_actions": "assets/sorla/business-actions.json",
                "business_actions_lock": "assets/sorla/business-actions.lock.json"
            },
            "sorx": {
                "start_schema": "assets/sorx/start.schema.json"
            }
        }),
        integrity: None,
        assets: vec![
            "assets/sorla/model.cbor".to_string(),
            "assets/sorla/agent-gateway.json".to_string(),
            "assets/sorla/mcp-tools.json".to_string(),
            "assets/sorla/business-actions.json".to_string(),
            "assets/sorla/business-actions.lock.json".to_string(),
            "assets/sorx/start.schema.json".to_string(),
        ],
    };
    let mut manifest_bytes = Vec::new();
    ciborium::ser::into_writer(&manifest, &mut manifest_bytes).unwrap();
    vec![
        ("pack.cbor".to_string(), manifest_bytes),
        (
            "assets/sorla/model.cbor".to_string(),
            vec![0xa1, 0x61, 0x78, 0x01],
        ),
        (
            "assets/sorla/agent-gateway.json".to_string(),
            AGENT_GATEWAY.as_bytes().to_vec(),
        ),
        (
            "assets/sorla/mcp-tools.json".to_string(),
            MCP_TOOLS.as_bytes().to_vec(),
        ),
        (
            "assets/sorla/business-actions.json".to_string(),
            serde_json::to_vec_pretty(&BusinessActionCatalog {
                schema: "greentic.sorla.business-actions.v1".to_string(),
                actions: vec![business_action],
            })
            .unwrap(),
        ),
        (
            "assets/sorla/business-actions.lock.json".to_string(),
            serde_json::to_vec_pretty(&business_action_lock).unwrap(),
        ),
        (
            "assets/sorx/start.schema.json".to_string(),
            serde_json::to_vec_pretty(&default_start_schema()).unwrap(),
        ),
    ]
}

fn encode_cbor<T: serde::Serialize>(value: &T) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::ser::into_writer(value, &mut out).unwrap();
    out
}

fn lock_for_entries(entries: &std::collections::BTreeMap<String, Vec<u8>>) -> PackLock {
    PackLock {
        schema: "greentic.gtpack.lock.sorla.v1".to_string(),
        entries: entries
            .iter()
            .filter(|(path, _)| path.as_str() != "pack.lock.cbor")
            .map(|(path, bytes)| {
                (
                    path.clone(),
                    PackLockEntry {
                        size: bytes.len() as u64,
                        sha256: hex::encode(Sha256::digest(bytes)),
                    },
                )
            })
            .collect(),
    }
}

fn gpack_manifest_bytes() -> Vec<u8> {
    encode_pack_manifest(&GpackManifest {
        schema_version: "pack-v1".to_string(),
        pack_id: "landlord-tenant-sor".parse::<PackId>().unwrap(),
        name: Some("landlord-tenant-sor".to_string()),
        version: Version::parse("0.1.0").unwrap(),
        kind: GpackKind::Application,
        publisher: "greentic-sorx-tests".to_string(),
        components: Vec::new(),
        flows: Vec::new(),
        dependencies: Vec::new(),
        capabilities: Vec::new(),
        secret_requirements: Vec::new(),
        signatures: PackSignatures::default(),
        bootstrap: None,
        extensions: None,
    })
    .unwrap()
}

fn business_action() -> BusinessAction {
    BusinessAction {
        id: "record_rent_payment".to_string(),
        version: "0.1.0".to_string(),
        label: Some("Record rent payment".to_string()),
        description: None,
        aliases: vec!["rent paid".to_string()],
        execution: BusinessActionExecution {
            endpoint_id: Some("payment.record".to_string()),
            operation_id: Some("payment.record".to_string()),
            tool_name: None,
        },
        input_schema: Some(json!({
            "type": "object",
            "required": ["id", "tenancy_id", "amount_cents", "status"],
            "properties": {
                "id": { "type": "string" },
                "tenancy_id": { "type": "string" },
                "amount_cents": { "type": "integer" },
                "status": { "type": "string" }
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
    }
}

struct ServerProcess {
    child: Child,
}

impl ServerProcess {
    fn start(pack: &std::path::Path, answers: &std::path::Path, port: u16) -> Self {
        let child = greentic_sorx_command()
            .args([
                "start",
                pack.to_str().unwrap(),
                "--answers",
                answers.to_str().unwrap(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("server should start");

        let client = HttpClient { port };
        let started = Instant::now();
        loop {
            if started.elapsed() > Duration::from_secs(5) {
                panic!("server did not become ready");
            }
            if let Ok(response) = client.try_get("/healthz")
                && response.status == 200
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }

        Self { child }
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct HttpClient {
    port: u16,
}

impl HttpClient {
    fn get(&self, path: &str) -> HttpResponse {
        self.try_request("GET", path, None, None).unwrap()
    }

    fn try_get(&self, path: &str) -> std::io::Result<HttpResponse> {
        self.try_request("GET", path, None, None)
    }

    fn get_json(&self, path: &str) -> Value {
        let response = self.get(path);
        assert_eq!(response.status, 200, "{:?}", response.body);
        response.body
    }

    fn post_json(&self, path: &str, body: &Value, idempotency_key: Option<&str>) -> Value {
        let response = self.post_json_raw(path, body, idempotency_key);
        assert_eq!(response.status, 200, "{:?}", response.body);
        response.body
    }

    fn post_json_raw(
        &self,
        path: &str,
        body: &Value,
        idempotency_key: Option<&str>,
    ) -> HttpResponse {
        self.try_request("POST", path, Some(body), idempotency_key)
            .unwrap()
    }

    fn patch_json(&self, path: &str, body: &Value) -> Value {
        let response = self.try_request("PATCH", path, Some(body), None).unwrap();
        assert_eq!(response.status, 200, "{:?}", response.body);
        response.body
    }

    fn try_request(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        idempotency_key: Option<&str>,
    ) -> std::io::Result<HttpResponse> {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port))?;
        let body = body
            .map(|value| serde_json::to_string(value).unwrap())
            .unwrap_or_default();
        let mut request = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        );
        if let Some(key) = idempotency_key {
            request.push_str(&format!("Idempotency-Key: {key}\r\n"));
        }
        request.push_str("\r\n");
        request.push_str(&body);
        stream.write_all(request.as_bytes())?;
        stream.shutdown(std::net::Shutdown::Write)?;

        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        Ok(parse_response(&response))
    }
}

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    body: Value,
}

fn parse_response(response: &str) -> HttpResponse {
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let status = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse::<u16>()
        .unwrap();
    HttpResponse {
        status,
        body: serde_json::from_str(body).unwrap(),
    }
}

fn assert_created_event(response: &Value, operation_id: &str, event_type: &str) {
    assert_eq!(response["operation_id"], operation_id);
    assert!(response["events"].as_array().unwrap().iter().any(|event| {
        event["operation_id"] == operation_id && event["event_type"] == event_type
    }));
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn foundationdb_answers_are_present_for_manual_e2e() {
    let answers: Value = serde_json::from_str(ANSWERS_FOUNDATIONDB).unwrap();
    assert_eq!(answers["providers"]["store"]["kind"], "foundationdb");
}
