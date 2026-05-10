use std::process::Command;

use greentic_sorx_core::default_start_schema;
use greentic_sorx_pack::{PackIdentity, PackManifest};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

#[test]
fn binary_help_contains_command_surface() {
    let output = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .arg("--help")
        .output()
        .expect("greentic-sorx binary should run");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help output should be UTF-8");
    assert!(stdout.contains("doctor"));
    assert!(stdout.contains("inspect"));
    assert!(stdout.contains("routes"));
    assert!(stdout.contains("start"));
}

#[test]
fn binary_placeholder_command_fails_clearly() {
    let output = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args(["routes", "--deployment", "missing"])
        .output()
        .expect("greentic-sorx binary should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("error output should be UTF-8");
    assert!(stderr.contains("deployment `missing` does not exist"));
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn binary_start_schema_emits_pack_schema() {
    let fixture = PackFixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args(["start", fixture.pack.to_str().unwrap(), "--schema"])
        .output()
        .expect("greentic-sorx binary should run");

    assert!(output.status.success(), "{output:?}");
    let stdout: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("schema output should be JSON");
    assert_eq!(stdout["schema"], "greentic.sorx.start.schema.v1");
}

#[test]
fn binary_start_dry_run_emits_plan() {
    let fixture = PackFixture::new();
    let answers = fixture.write_answers(
        r#"{
  "tenant": { "tenant_id": "tenant-a" },
  "server": { "public_base_url": "http://127.0.0.1:8787" },
  "providers": { "store": { "kind": "memory", "config_ref": "providers.memory.local" } },
  "policy": { "approvals": {} },
  "audit": {},
  "deployment": { "tenant_id": "tenant-a", "sor_name": "landlord", "environment": "local" },
  "exposure": {},
  "ghcr": {}
}"#,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args([
            "start",
            fixture.pack.to_str().unwrap(),
            "--answers",
            answers.to_str().unwrap(),
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("greentic-sorx binary should run");

    assert!(output.status.success(), "{output:?}");
    let stdout: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("plan output should be JSON");
    assert_eq!(stdout["schema"], "greentic.sorx.start.plan.v1");
    assert_eq!(stdout["providers"][0]["id"], "store");
    assert_eq!(stdout["policy"]["high"], "require_approval");
}

#[test]
fn binary_routes_json_is_stable() {
    let fixture = PackFixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args(["routes", fixture.pack.to_str().unwrap(), "--json"])
        .output()
        .expect("greentic-sorx binary should run");

    assert!(output.status.success(), "{output:?}");
    let stdout: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("routes output should be JSON");
    assert_eq!(stdout["schema"], "greentic.sorx.routes.v1");
    assert_eq!(stdout["routes"][0]["endpoint_id"], "tenant.create");
}

#[test]
fn binary_start_emit_answers_applies_defaults() {
    let fixture = PackFixture::new();
    let answers = fixture.write_answers(
        r#"{
  "tenant": { "tenant_id": "tenant-a" },
  "server": { "public_base_url": "http://127.0.0.1:8787" },
  "providers": { "store": { "kind": "memory", "config_ref": "providers.memory.local" } },
  "policy": { "approvals": {} },
  "audit": {},
  "deployment": { "tenant_id": "tenant-a", "sor_name": "landlord", "environment": "local" },
  "exposure": {},
  "ghcr": {}
}"#,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args([
            "start",
            fixture.pack.to_str().unwrap(),
            "--answers",
            answers.to_str().unwrap(),
            "--emit-answers",
        ])
        .output()
        .expect("greentic-sorx binary should run");

    assert!(output.status.success(), "{output:?}");
    let stdout: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("answers output should be JSON");
    assert_eq!(
        stdout["schema"],
        "greentic.sorx.start.answers.normalized.v1"
    );
    assert_eq!(stdout["answers"]["server"]["bind"], "127.0.0.1:8787");
}

#[test]
fn binary_start_non_interactive_missing_answers_fails_clearly() {
    let fixture = PackFixture::new();
    let answers = fixture.write_answers(r#"{"tenant":{},"server":{},"providers":{},"policy":{},"audit":{},"deployment":{},"exposure":{},"ghcr":{}}"#);
    let output = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args([
            "--non-interactive",
            "start",
            fixture.pack.to_str().unwrap(),
            "--answers",
            answers.to_str().unwrap(),
        ])
        .output()
        .expect("greentic-sorx binary should run");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(4));
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("missing required startup answers"));
    assert!(stderr.contains("tenant.tenant_id"));
}

#[test]
fn binary_doctor_pack_failure_uses_stable_exit_code() {
    let temp = TempDir::new().unwrap();
    let missing = temp.path().join("missing.gtpack");
    let output = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args(["doctor", missing.to_str().unwrap(), "--json"])
        .output()
        .expect("greentic-sorx binary should run");

    assert_eq!(output.status.code(), Some(3));
    let stdout: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor output should be JSON");
    assert_eq!(stdout["ok"], false);
}

#[test]
fn binary_mcp_tools_lists_pack_tools() {
    let fixture = PackFixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args(["mcp-tools", fixture.pack.to_str().unwrap()])
        .output()
        .expect("greentic-sorx binary should run");

    assert!(output.status.success(), "{output:?}");
    let stdout: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("MCP tools output should be JSON");
    assert_eq!(stdout["schema"], "greentic.sorx.mcp-tools.v1");
    assert_eq!(stdout["tools"][0]["name"], "sorla_create_tenant");
    assert_eq!(stdout["tools"][0]["endpoint_id"], "tenant.create");
}

#[test]
fn binary_validate_runs_pack_embedded_suite() {
    let fixture = PackFixture::new();
    let answers = fixture.write_answers(
        r#"{
  "tenant": { "tenant_id": "tenant-a" },
  "server": { "public_base_url": "http://127.0.0.1:8787" },
  "providers": { "store": { "kind": "memory", "config_ref": "providers.memory.local" } },
  "policy": { "approvals": {} },
  "audit": {},
  "deployment": { "tenant_id": "tenant-a", "sor_name": "landlord", "environment": "local" },
  "exposure": {},
  "ghcr": {}
}"#,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args([
            "validate",
            fixture.pack.to_str().unwrap(),
            "--answers",
            answers.to_str().unwrap(),
            "--provider-mode",
            "in-memory",
            "--json",
        ])
        .output()
        .expect("greentic-sorx binary should run");

    assert!(output.status.success(), "{output:?}");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema"], "greentic.sorx.validation-report.v1");
    assert_eq!(report["result"], "pass", "{report:#}");
    assert_eq!(report["public_exposure_allowed"], true);
    assert_eq!(report["tests"][0]["id"], "doctor.pack.valid");
    assert_eq!(
        report["tests"][3]["id"],
        "tenant.create.rejects_missing_active"
    );
    assert_eq!(report["tests"][3]["result"], "pass");
    assert_eq!(report["tests"][6]["id"], "recommended.missing.artifact");
    assert_eq!(report["tests"][6]["result"], "fail");
    assert_eq!(report["tests"][6]["level"], "recommended");
}

#[test]
fn binary_deployment_registry_create_validate_alias_and_routes() {
    let fixture = PackFixture::new();
    let registry = fixture._temp.path().join("registry.json");
    let create = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args([
            "--registry",
            registry.to_str().unwrap(),
            "deployments",
            "create",
            "--pack",
            fixture.pack.to_str().unwrap(),
            "--tenant",
            "acme",
            "--sor",
            "landlord",
            "--environment",
            "production",
            "--api-version",
            "v1",
            "--base-path",
            "/sorx/acme/landlord/v1",
            "--visibility",
            "private",
        ])
        .output()
        .expect("greentic-sorx binary should run");
    assert!(create.status.success(), "{create:?}");
    let created: serde_json::Value = serde_json::from_slice(&create.stdout).unwrap();
    let deployment_id = created["deployment_id"].as_str().unwrap();
    assert_eq!(created["status"], "pending");
    assert!(registry.exists());

    let validate = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args([
            "--registry",
            registry.to_str().unwrap(),
            "deployments",
            "validate",
            deployment_id,
        ])
        .output()
        .expect("greentic-sorx binary should run");
    assert!(validate.status.success(), "{validate:?}");
    let validated: serde_json::Value = serde_json::from_slice(&validate.stdout).unwrap();
    assert_eq!(validated["status"], "validated");

    let alias = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args([
            "--registry",
            registry.to_str().unwrap(),
            "aliases",
            "set",
            "--tenant",
            "acme",
            "--sor",
            "landlord",
            "--alias",
            "stable",
            "--target",
            deployment_id,
        ])
        .output()
        .expect("greentic-sorx binary should run");
    assert!(alias.status.success(), "{alias:?}");
    let alias: serde_json::Value = serde_json::from_slice(&alias.stdout).unwrap();
    assert_eq!(alias["target_deployment_id"], deployment_id);

    let routes = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args([
            "--registry",
            registry.to_str().unwrap(),
            "routes",
            "--deployment",
            deployment_id,
            "--json",
        ])
        .output()
        .expect("greentic-sorx binary should run");
    assert!(routes.status.success(), "{routes:?}");
    let routes: serde_json::Value = serde_json::from_slice(&routes.stdout).unwrap();
    assert_eq!(routes["routes"][0]["deployment_id"], deployment_id);
    assert_eq!(
        routes["routes"][0]["path"],
        "/sorx/acme/landlord/v1/v1/tenants"
    );

    let mut registry_json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&registry).unwrap()).unwrap();
    registry_json["validation_reports"] = serde_json::json!([{
        "schema": "greentic.sorx.validation-report.v1",
        "deployment_id": deployment_id,
        "pack_name": created["pack_name"],
        "pack_version": created["pack_version"],
        "pack_digest": created["pack_digest"],
        "suite_id": "landlord-basic",
        "started_at": "1970-01-01T00:00:00Z",
        "finished_at": "1970-01-01T00:00:00Z",
        "result": "pass",
        "public_exposure_allowed": true,
        "tests": []
    }]);
    std::fs::write(
        &registry,
        serde_json::to_vec_pretty(&registry_json).unwrap(),
    )
    .unwrap();

    let promote = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args([
            "--registry",
            registry.to_str().unwrap(),
            "deployments",
            "promote",
            deployment_id,
            "--alias",
            "latest",
            "--public",
        ])
        .output()
        .expect("greentic-sorx binary should run");
    assert!(promote.status.success(), "{promote:?}");
    let promoted_alias: serde_json::Value = serde_json::from_slice(&promote.stdout).unwrap();
    assert_eq!(promoted_alias["alias"], "latest");
    assert_eq!(promoted_alias["target_deployment_id"], deployment_id);

    let public_routes = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args([
            "--registry",
            registry.to_str().unwrap(),
            "deployments",
            "public-routes",
        ])
        .output()
        .expect("greentic-sorx binary should run");
    assert!(public_routes.status.success(), "{public_routes:?}");
    let public_routes: serde_json::Value = serde_json::from_slice(&public_routes.stdout).unwrap();
    assert_eq!(public_routes["schema"], "greentic.sorx.public-routes.v1");
    assert_eq!(
        public_routes["deployments"][0]["deployment_id"],
        deployment_id
    );
}

#[test]
fn binary_webhook_fixture_replay_creates_pending_deployment() {
    let temp = TempDir::new().unwrap();
    let registry = temp.path().join("registry.json");
    let fixture = temp.path().join("github-ghcr-published.json");
    let payload = serde_json::json!({
        "repository": "greenticai/greentic-sorla",
        "workflow": "publish-gtpack.yml",
        "conclusion": "success",
        "artifact_kind": "sorla-gtpack",
        "oci_ref": "oci://ghcr.io/greenticai/sorla/landlord-tenant-sor:1.1.0",
        "digest": "sha256:abc123",
        "pack_name": "landlord-tenant-sor",
        "pack_version": "1.1.0",
        "tenant_id": "acme",
        "sor_name": "landlord",
        "environment": "staging",
        "api_version_label": "v1.1",
        "promotion_policy": "validate_then_private"
    });
    let secret = "fixture-secret";
    std::fs::write(
        &fixture,
        serde_json::to_vec_pretty(&serde_json::json!({
            "event": "repository_dispatch",
            "delivery": "delivery-1",
            "secret": secret,
            "resolved_digest": "sha256:abc123",
            "payload": payload
        }))
        .unwrap(),
    )
    .unwrap();
    let body = serde_json::to_vec(&payload).unwrap();
    let signature = greentic_sorx_core::github_signature(secret.as_bytes(), &body);

    let replay = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args([
            "--registry",
            registry.to_str().unwrap(),
            "webhook",
            "replay",
            "--fixture",
            fixture.to_str().unwrap(),
            "--signature",
            &signature,
        ])
        .output()
        .expect("greentic-sorx binary should run");
    assert!(replay.status.success(), "{replay:?}");
    let outcome: serde_json::Value = serde_json::from_slice(&replay.stdout).unwrap();
    assert_eq!(outcome["status"], "pending");
    assert_eq!(outcome["validation_job_requested"], true);
    assert_eq!(outcome["public_exposure_started"], false);

    let registry_json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&registry).unwrap()).unwrap();
    assert_eq!(
        registry_json["deployments"][0]["artifact"]["source"],
        payload["oci_ref"]
    );
    assert_eq!(registry_json["webhook_deliveries"][0], "delivery-1");
}

struct PackFixture {
    _temp: TempDir,
    pack: std::path::PathBuf,
}

impl PackFixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let pack = temp.path().join("landlord.gtpack");
        let file = std::fs::File::create(&pack).unwrap();
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .unix_permissions(0o644);
        for (name, bytes) in pack_entries() {
            writer.start_file(name, options).unwrap();
            std::io::Write::write_all(&mut writer, &bytes).unwrap();
        }
        writer.finish().unwrap();
        Self { _temp: temp, pack }
    }

    fn write_answers(&self, text: &str) -> std::path::PathBuf {
        let path = self._temp.path().join("answers.json");
        std::fs::write(&path, text).unwrap();
        path
    }
}

fn pack_entries() -> Vec<(String, Vec<u8>)> {
    let manifest = PackManifest {
        schema: "greentic.gtpack.manifest.sorla.v1".to_string(),
        pack: PackIdentity {
            name: "landlord".to_string(),
            version: "0.1.0".to_string(),
            kind: Some("application".to_string()),
        },
        extension: serde_json::json!({
            "extension": "greentic.sorx.runtime.v1",
            "sorla": {
                "model": "assets/sorla/model.cbor",
                "agent_gateway": "assets/sorla/agent-gateway.json",
                "mcp_tools": "assets/sorla/mcp-tools.json"
            },
            "sorx": {
                "start_schema": "assets/sorx/start.schema.json",
                "validation_suite": "assets/sorx/validation-suite.json"
            }
        }),
        integrity: None,
        assets: vec![
            "assets/sorla/model.cbor".to_string(),
            "assets/sorla/agent-gateway.json".to_string(),
            "assets/sorla/mcp-tools.json".to_string(),
            "assets/sorx/start.schema.json".to_string(),
            "assets/sorx/validation-suite.json".to_string(),
            "assets/sorx/validation-fixtures/tenant-create.json".to_string(),
            "assets/sorx/validation-fixtures/tenant-create-invalid.json".to_string(),
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
            br#"{"schema":"greentic.sorla.agent-gateway.v1","endpoints":[{"endpoint_id":"tenant.create","operation_id":"tenant.create","operation":"create","method":"POST","path":"/v1/tenants","entity":"Tenant","collection":"tenants","provider_binding":"store","risk":"low","input_schema":{"type":"object","required":["id","name","active"],"properties":{"id":{"type":"string"},"name":{"type":"string"},"active":{"type":"boolean"}}}}]}"#.to_vec(),
        ),
        (
            "assets/sorla/mcp-tools.json".to_string(),
            br#"{"schema":"greentic.sorla.mcp-tools.v1","tools":[{"name":"sorla_create_tenant","endpoint_id":"tenant.create"}]}"#.to_vec(),
        ),
        (
            "assets/sorx/start.schema.json".to_string(),
            serde_json::to_vec_pretty(&default_start_schema()).unwrap(),
        ),
        (
            "assets/sorx/validation-suite.json".to_string(),
            br#"{"schema":"greentic.sorx.validation-suite.v1","suite_id":"landlord-basic","pack_name":"landlord","pack_version":"0.1.0","gates":{"required_for_public_exposure":true,"minimum_pass_level":"required"},"tests":[{"id":"doctor.pack.valid","kind":"doctor","level":"required"},{"id":"routes.generated","kind":"route_generation","level":"required"},{"id":"tenant.create.happy_path","kind":"endpoint_call","level":"required","method":"POST","path":"/v1/tenants","input_fixture":"assets/sorx/validation-fixtures/tenant-create.json","expect":{"status":200,"json_path":"$.ok","equals":true}},{"id":"tenant.create.rejects_missing_active","kind":"negative_endpoint_call","level":"required","method":"POST","path":"/v1/tenants","input_fixture":"assets/sorx/validation-fixtures/tenant-create-invalid.json","expect":{"status":400,"json_path":"$.ok","equals":false}},{"id":"tenant.create.idempotency","kind":"idempotency","level":"required","method":"POST","path":"/v1/tenants","input_fixture":"assets/sorx/validation-fixtures/tenant-create.json"},{"id":"audit.completed","kind":"audit_event_emitted","level":"recommended","expect":{"event":"sorx.endpoint.completed"}},{"id":"recommended.missing.artifact","kind":"artifact_exists","level":"recommended","path":"assets/sorx/validation-expected/not-present.json"}]}"#.to_vec(),
        ),
        (
            "assets/sorx/validation-fixtures/tenant-create.json".to_string(),
            br#"{"id":"tenant-1","name":"Acme","active":true}"#.to_vec(),
        ),
        (
            "assets/sorx/validation-fixtures/tenant-create-invalid.json".to_string(),
            br#"{"id":"tenant-2","name":"Acme"}"#.to_vec(),
        ),
    ]
}
