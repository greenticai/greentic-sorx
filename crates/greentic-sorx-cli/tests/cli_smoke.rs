use std::process::Command;

use greentic_sorx_core::default_start_schema;
use greentic_sorx_pack::{PackIdentity, PackManifest};
use sha2::{Digest, Sha256};
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
    assert_eq!(stdout["provider_compatibility"]["status"], "passed");
}

#[test]
fn binary_start_dry_run_reports_ontology_provider_compatibility() {
    let fixture = PackFixture::new_with_ontology();
    let answers = fixture.write_answers(ontology_answers());
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
    assert_eq!(stdout["provider_compatibility"]["status"], "passed");
    assert_eq!(
        stdout["provider_compatibility"]["bindings"][0]["requirement"],
        "entity.link"
    );
    assert_eq!(
        stdout["provider_compatibility"]["issues"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
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
fn binary_graph_paths_json_is_stable() {
    let fixture = PackFixture::new_with_ontology();
    let output = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args([
            "graph",
            "paths",
            fixture.pack.to_str().unwrap(),
            "--from",
            "Tenant",
            "--to",
            "Payment",
            "--json",
        ])
        .output()
        .expect("greentic-sorx binary should run");

    assert!(output.status.success(), "{output:?}");
    let stdout: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("graph output should be JSON");
    assert_eq!(stdout["schema"], "greentic.sorx.graph.paths.v1");
    assert_eq!(
        stdout["paths"][0]["relationships"][0],
        "tenant_makes_payment"
    );
}

#[test]
fn binary_graph_unknown_concept_fails_clearly() {
    let fixture = PackFixture::new_with_ontology();
    let output = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args([
            "graph",
            "paths",
            fixture.pack.to_str().unwrap(),
            "--from",
            "Tenant",
            "--to",
            "Missing",
            "--json",
        ])
        .output()
        .expect("greentic-sorx binary should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("ontology concept `Missing` does not exist"));
}

#[test]
fn binary_graph_relationship_policy_denies_traversal() {
    let fixture = PackFixture::new_with_denied_relationship();
    let output = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args([
            "graph",
            "paths",
            fixture.pack.to_str().unwrap(),
            "--from",
            "Tenant",
            "--to",
            "Payment",
            "--json",
        ])
        .output()
        .expect("greentic-sorx binary should run");

    assert_eq!(output.status.code(), Some(7));
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("relationship_traversal_denied"));
}

#[test]
fn binary_evidence_query_json_is_stable() {
    let fixture = PackFixture::new_with_ontology();
    let answers = fixture.write_answers(ontology_answers());
    let output = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args([
            "evidence",
            "query",
            fixture.pack.to_str().unwrap(),
            "--answers",
            answers.to_str().unwrap(),
            "--query",
            "lease status",
            "--entity-type",
            "Tenant",
            "--entity-id",
            "tenant-1",
            "--max-depth",
            "2",
            "--json",
        ])
        .output()
        .expect("greentic-sorx binary should run");

    assert!(output.status.success(), "{output:?}");
    let stdout: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("evidence output should be JSON");
    assert_eq!(stdout["schema"], "greentic.sorx.evidence-query-result.v1");
    assert_eq!(stdout["explain"]["provider_id"], "rag");
    assert_eq!(
        stdout["evidence"][0]["provenance"],
        "deterministic-memory-evidence-provider"
    );
    assert_eq!(
        stdout["ontology_scope"]["relationships"][0],
        "tenant_makes_payment"
    );
    assert_eq!(
        stdout["audit_events"][0]["schema"],
        "greentic.sorx.ontology.audit.v1"
    );
    assert_eq!(
        stdout["audit_events"][0]["event"],
        "provider.compatibility.checked"
    );
    assert_eq!(stdout["audit_events"][2]["event"], "evidence.query.planned");
    assert!(
        stdout["explain"]["ontology_graph_hash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert_eq!(stdout["explain"]["providers_used"][0], "rag");
}

#[test]
fn binary_deterministic_ontology_business_scenario_is_stable() {
    let fixture = PackFixture::new_with_business_ontology();
    let answers = fixture.write_answers(ontology_answers());

    let doctor = run_json(["doctor", fixture.pack.to_str().unwrap(), "--json"]);
    assert_eq!(doctor["ok"], true);

    let start = run_json([
        "start",
        fixture.pack.to_str().unwrap(),
        "--answers",
        answers.to_str().unwrap(),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(start["schema"], "greentic.sorx.start.plan.v1");
    assert_eq!(start["provider_compatibility"]["status"], "passed");

    let graph = run_json([
        "graph",
        "paths",
        fixture.pack.to_str().unwrap(),
        "--from",
        "Customer",
        "--to",
        "EvidenceDocument",
        "--json",
    ]);
    assert_eq!(graph["schema"], "greentic.sorx.graph.paths.v1");
    assert_eq!(graph["paths"][0]["concepts"][0], "Customer");
    assert_eq!(graph["paths"][0]["concepts"][2], "EvidenceDocument");
    assert_eq!(
        graph["paths"][0]["relationships"][0],
        "customer_has_contract"
    );
    assert_eq!(
        graph["paths"][0]["relationships"][1],
        "contract_has_evidence"
    );

    let evidence_args = [
        "evidence",
        "query",
        fixture.pack.to_str().unwrap(),
        "--answers",
        answers.to_str().unwrap(),
        "--query",
        "risk evidence",
        "--entity-type",
        "Customer",
        "--entity-id",
        "customer-001",
        "--max-depth",
        "3",
        "--json",
    ];
    let evidence = run_json(evidence_args);
    let evidence_repeat = run_json(evidence_args);
    assert_eq!(evidence, evidence_repeat);
    assert_eq!(evidence["schema"], "greentic.sorx.evidence-query-result.v1");
    assert_eq!(
        evidence["ontology_scope"]["root_entities"][0]["entity_type"],
        "Customer"
    );
    assert_eq!(
        evidence["evidence"][0]["evidence_id"],
        "evidence:rag:Customer:customer-001"
    );
    assert_eq!(
        evidence["explain"]["retrieval_binding"],
        "business-risk-evidence"
    );
    assert_eq!(evidence["explain"]["providers_used"][0], "rag");
    assert!(
        evidence["explain"]["concepts_used"]
            .as_array()
            .unwrap()
            .iter()
            .any(|concept| concept == "EvidenceDocument")
    );
    assert!(
        evidence["audit_events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["event"] == "policy.ontology.decision")
    );
}

#[test]
fn binary_artifact_validate_file_emits_combined_report() {
    let fixture = PackFixture::new_with_ontology();
    let report = run_json([
        "artifact",
        "validate",
        "--file",
        fixture.pack.to_str().unwrap(),
        "--json",
    ]);

    assert_eq!(
        report["schema"],
        "greentic.sorx.artifact.validation-report.v1"
    );
    assert_eq!(report["valid"], true);
    assert_eq!(report["doctor"]["ok"], true);
    assert_eq!(report["inspect"]["pack"]["name"], "landlord");
    assert_eq!(
        report["startup_schema"]["schema"],
        "greentic.sorx.start.schema.v1"
    );
    assert_eq!(report["provider_compatibility"], serde_json::Value::Null);
}

#[test]
fn binary_artifact_validate_json_with_answers_reports_provider_compatibility() {
    let fixture = PackFixture::new_with_ontology();
    let artifact = fixture.write_artifact_json("artifact.json", None, None, None, None);
    let answers = fixture.write_answers(ontology_answers());
    let report = run_json([
        "artifact",
        "validate",
        "--artifact-json",
        artifact.to_str().unwrap(),
        "--answers",
        answers.to_str().unwrap(),
        "--json",
    ]);

    assert_eq!(report["valid"], true);
    assert_eq!(
        report["artifact"]["sha256"],
        format!("sha256:{}", fixture.pack_sha256())
    );
    assert_eq!(report["provider_compatibility"]["status"], "passed");
    assert_eq!(
        report["provider_compatibility"]["bindings"][0]["requirement"],
        "entity.link"
    );
}

#[test]
fn binary_artifact_inspect_and_startup_schema_accept_artifact_json() {
    let fixture = PackFixture::new();
    let artifact = fixture.write_artifact_json("artifact.json", None, None, None, None);
    let inspect = run_json([
        "artifact",
        "inspect",
        "--artifact-json",
        artifact.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(inspect["schema"], "greentic.sorx.inspect.v1");
    assert_eq!(inspect["pack"]["name"], "landlord");

    let schema = run_json([
        "artifact",
        "startup-schema",
        "--artifact-json",
        artifact.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(schema["schema"], "greentic.sorx.start.schema.v1");
}

#[test]
fn binary_artifact_json_rejects_hash_mismatch() {
    let fixture = PackFixture::new();
    let artifact = fixture.write_artifact_json(
        "bad-hash.json",
        Some("0000000000000000000000000000000000000000000000000000000000000000"),
        None,
        None,
        None,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args([
            "artifact",
            "validate",
            "--artifact-json",
            artifact.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("greentic-sorx binary should run");

    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("artifact sha256 mismatch"));
}

#[test]
fn binary_artifact_json_rejects_wrong_media_type_and_kind() {
    let fixture = PackFixture::new();
    let bad_media = fixture.write_artifact_json(
        "bad-media.json",
        None,
        Some("application/octet-stream"),
        None,
        None,
    );
    let media_output = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args([
            "artifact",
            "validate",
            "--artifact-json",
            bad_media.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("greentic-sorx binary should run");
    assert_eq!(media_output.status.code(), Some(3));
    let media_stderr = String::from_utf8(media_output.stderr).expect("stderr should be UTF-8");
    assert!(media_stderr.contains("artifact media_type must be"));

    let bad_kind = fixture.write_artifact_json("bad-kind.json", None, None, Some("bundle"), None);
    let kind_output = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args([
            "artifact",
            "validate",
            "--artifact-json",
            bad_kind.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("greentic-sorx binary should run");
    assert_eq!(kind_output.status.code(), Some(3));
    let kind_stderr = String::from_utf8(kind_output.stderr).expect("stderr should be UTF-8");
    assert!(kind_stderr.contains("artifact kind must be"));
}

#[test]
fn binary_artifact_json_rejects_malformed_base64() {
    let fixture = PackFixture::new();
    let artifact = fixture.write_artifact_json("bad-base64.json", None, None, None, Some("@@"));
    let output = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args([
            "artifact",
            "validate",
            "--artifact-json",
            artifact.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("greentic-sorx binary should run");

    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("invalid base64"));
}

#[test]
fn binary_evidence_query_missing_provider_fails() {
    let fixture = PackFixture::new_with_ontology();
    let answers = fixture.write_answers(
        r#"{
  "tenant": { "tenant_id": "tenant-a" },
  "server": { "public_base_url": "http://127.0.0.1:8787" },
  "providers": { "store": { "kind": "memory" } },
  "policy": { "approvals": {} },
  "audit": {},
  "deployment": { "tenant_id": "tenant-a", "sor_name": "landlord", "environment": "local" },
  "exposure": {},
  "ghcr": {}
}"#,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args([
            "evidence",
            "query",
            fixture.pack.to_str().unwrap(),
            "--answers",
            answers.to_str().unwrap(),
            "--query",
            "lease status",
            "--entity-type",
            "Tenant",
            "--entity-id",
            "tenant-1",
        ])
        .output()
        .expect("greentic-sorx binary should run");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(5));
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("provider compatibility failed"));
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
        Self::from_entries(pack_entries())
    }

    fn new_with_ontology() -> Self {
        Self::from_entries(pack_entries_with_ontology(None))
    }

    fn new_with_denied_relationship() -> Self {
        Self::from_entries(pack_entries_with_ontology(Some(
            r#","policy":{"deny_relationships":["tenant_makes_payment"]}"#,
        )))
    }

    fn new_with_business_ontology() -> Self {
        Self::from_entries(pack_entries_with_business_ontology())
    }

    fn from_entries(entries: Vec<(String, Vec<u8>)>) -> Self {
        let temp = TempDir::new().unwrap();
        let pack = temp.path().join("landlord.gtpack");
        let file = std::fs::File::create(&pack).unwrap();
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .unix_permissions(0o644);
        for (name, bytes) in entries {
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

    fn pack_sha256(&self) -> String {
        let bytes = std::fs::read(&self.pack).unwrap();
        hex::encode(Sha256::digest(bytes))
    }

    fn write_artifact_json(
        &self,
        name: &str,
        sha256: Option<&str>,
        media_type: Option<&str>,
        kind: Option<&str>,
        bytes_base64: Option<&str>,
    ) -> std::path::PathBuf {
        let bytes = std::fs::read(&self.pack).unwrap();
        let path = self._temp.path().join(name);
        let sha256 = sha256
            .map(ToString::to_string)
            .unwrap_or_else(|| self.pack_sha256());
        let artifact = serde_json::json!({
            "kind": kind.unwrap_or("gtpack"),
            "filename": "landlord.gtpack",
            "media_type": media_type.unwrap_or("application/vnd.greentic.gtpack"),
            "sha256": sha256,
            "bytes_base64": bytes_base64
                .map(ToString::to_string)
                .unwrap_or_else(|| encode_base64(&bytes)),
            "metadata_json": {}
        });
        std::fs::write(&path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();
        path
    }
}

fn run_json<const N: usize>(args: [&str; N]) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
        .args(args)
        .output()
        .expect("greentic-sorx binary should run");
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).expect("command output should be JSON")
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn pack_entries_with_ontology(policy_fragment: Option<&str>) -> Vec<(String, Vec<u8>)> {
    let mut entries = pack_entries();
    let policy_fragment = policy_fragment.unwrap_or("");
    entries.push((
        "assets/sorla/ontology.graph.json".to_string(),
        format!(
            r#"{{"schema":"greentic.sorla.ontology.graph.v1","requires_entity_link":true,"concepts":[{{"id":"Tenant"}},{{"id":"Payment"}}],"relationships":[{{"id":"tenant_makes_payment","from":"Tenant","to":"Payment"}}]{policy_fragment}}}"#
        )
        .into_bytes(),
    ));
    entries.push((
        "assets/sorla/retrieval-bindings.json".to_string(),
        br#"{"schema":"greentic.sorla.retrieval-bindings.v1","bindings":[{"id":"tenant-evidence","concept_id":"Tenant"}]}"#.to_vec(),
    ));
    entries
}

fn pack_entries_with_business_ontology() -> Vec<(String, Vec<u8>)> {
    let mut entries = pack_entries();
    entries.push((
        "assets/sorla/ontology.graph.json".to_string(),
        br#"{
  "schema": "greentic.sorla.ontology.graph.v1",
  "requires_entity_link": true,
  "concepts": [
    { "id": "Asset" },
    { "id": "Contract" },
    { "id": "Customer" },
    { "id": "EvidenceDocument" },
    { "id": "Obligation" },
    { "id": "Party" },
    { "id": "Supplier" }
  ],
  "relationships": [
    { "id": "contract_governs_asset", "from": "Contract", "to": "Asset" },
    { "id": "contract_has_evidence", "from": "Contract", "to": "EvidenceDocument" },
    { "id": "customer_has_contract", "from": "Customer", "to": "Contract" },
    { "id": "supplier_fulfils_obligation", "from": "Supplier", "to": "Obligation" }
  ],
  "policy": {
    "sensitive_concepts": ["EvidenceDocument"]
  }
}"#
        .to_vec(),
    ));
    entries.push((
        "assets/sorla/retrieval-bindings.json".to_string(),
        br#"{
  "schema": "greentic.sorla.retrieval-bindings.v1",
  "bindings": [
    {
      "id": "business-risk-evidence",
      "concept_id": "Customer",
      "scope": {
        "concepts": ["Customer", "Contract", "EvidenceDocument"],
        "relationships": ["customer_has_contract", "contract_has_evidence"]
      }
    }
  ]
}"#
        .to_vec(),
    ));
    entries
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

fn ontology_answers() -> &'static str {
    r#"{
  "tenant": { "tenant_id": "tenant-a" },
  "server": { "public_base_url": "http://127.0.0.1:8787" },
  "providers": {
    "store": { "kind": "memory", "config_ref": "providers.memory.local" },
    "rag": {
      "kind": "memory",
      "capabilities": ["ontology-scoped-evidence-query", "entity-link"],
      "contract_version": "greentic.sorx.provider.v1"
    }
  },
  "policy": { "approvals": {} },
  "audit": {},
  "deployment": { "tenant_id": "tenant-a", "sor_name": "landlord", "environment": "local" },
  "exposure": {},
  "ghcr": {}
}"#
}
