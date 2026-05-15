use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::process::Command;

use serde_json::{Value, json};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn greentic_sorx_command() -> Command {
    // Test-only helper invokes Cargo's compiled test binary path.
    // foxguard: ignore[rs/no-command-injection]
    Command::new(env!("CARGO_BIN_EXE_greentic-sorx"))
}

fn full_answers() -> Value {
    json!({
        "tenant": {
            "tenant_id": "tenant-a"
        },
        "server": {
            "public_base_url": "http://127.0.0.1:8787"
        },
        "providers": {
            "store": {
                "kind": "memory",
                "config_ref": "providers.memory.local"
            }
        },
        "policy": {
            "approvals": {}
        },
        "audit": {},
        "deployment": {
            "tenant_id": "tenant-a",
            "sor_name": "landlord",
            "environment": "local"
        },
        "exposure": {},
        "ghcr": {}
    })
}

fn startup_schema() -> Value {
    greentic_sorx_core::default_start_schema()
}

fn valid_entries() -> BTreeMap<String, Vec<u8>> {
    let extension = json!({
        "extension": "greentic.sorx.runtime.v1",
        "sorla": {
            "model": "assets/sorla/model.cbor",
            "agent_gateway": "assets/sorla/agent-gateway.json"
        },
        "sorx": {
            "start_schema": "assets/sorx/start.schema.json"
        }
    });
    let manifest = json!({
        "schema": "greentic.gtpack.manifest.sorla.v1",
        "pack": {
            "name": "landlord-tenant-sor",
            "version": "0.1.0",
            "kind": "application"
        },
        "extension": extension,
        "integrity": null,
        "assets": [
            "assets/sorla/model.cbor",
            "assets/sorla/agent-gateway.json",
            "assets/sorx/start.schema.json"
        ]
    });

    let mut entries = BTreeMap::new();
    entries.insert("pack.cbor".to_string(), encode_cbor(&manifest));
    entries.insert(
        "assets/sorla/model.cbor".to_string(),
        vec![0xa1, 0x61, 0x78, 0x01],
    );
    entries.insert(
        "assets/sorla/agent-gateway.json".to_string(),
        br#"{"schema":"greentic.sorla.agent-gateway.v1","endpoints":[]}"#.to_vec(),
    );
    entries.insert(
        "assets/sorx/start.schema.json".to_string(),
        serde_json::to_vec_pretty(&startup_schema()).unwrap(),
    );
    entries
}

fn encode_cbor(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::ser::into_writer(value, &mut out).unwrap();
    out
}

fn write_pack() -> (TempDir, std::path::PathBuf) {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("landlord.gtpack");
    let file = File::create(&path).unwrap();
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .unix_permissions(0o644);
    for (name, bytes) in valid_entries() {
        writer.start_file(name, options).unwrap();
        writer.write_all(&bytes).unwrap();
    }
    writer.finish().unwrap();
    (temp, path)
}

fn write_json(temp: &TempDir, name: &str, value: &Value) -> std::path::PathBuf {
    let path = temp.path().join(name);
    std::fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    path
}

#[test]
fn start_schema_emits_embedded_pack_schema() {
    let (_temp, pack) = write_pack();
    let output = greentic_sorx_command()
        .args(["start", pack.to_str().unwrap(), "--schema"])
        .output()
        .expect("greentic-sorx binary should run");

    assert!(output.status.success(), "{output:?}");
    let stdout: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(stdout["schema"], "greentic.sorx.start.schema.v1");
    assert_eq!(
        stdout["properties"]["server"]["properties"]["bind"]["default"],
        "127.0.0.1:8787"
    );
}

#[test]
fn start_emit_answers_normalizes_defaults_stably() {
    let (temp, pack) = write_pack();
    let answers = write_json(&temp, "answers.json", &full_answers());
    let output = greentic_sorx_command()
        .args([
            "start",
            pack.to_str().unwrap(),
            "--answers",
            answers.to_str().unwrap(),
            "--emit-answers",
        ])
        .output()
        .expect("greentic-sorx binary should run");

    assert!(output.status.success(), "{output:?}");
    let stdout: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        stdout["schema"],
        "greentic.sorx.start.answers.normalized.v1"
    );
    assert_eq!(stdout["answers"]["tenant"]["environment"], "local");
    assert_eq!(stdout["answers"]["policy"]["approvals"]["critical"], "deny");
}

#[test]
fn start_dry_run_emits_startup_plan() {
    let (temp, pack) = write_pack();
    let answers = write_json(&temp, "answers.json", &full_answers());
    let output = greentic_sorx_command()
        .args([
            "start",
            pack.to_str().unwrap(),
            "--answers",
            answers.to_str().unwrap(),
            "--dry-run",
        ])
        .output()
        .expect("greentic-sorx binary should run");

    assert!(output.status.success(), "{output:?}");
    let stdout: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(stdout["schema"], "greentic.sorx.start.plan.v1");
    assert_eq!(stdout["pack"]["name"], "landlord-tenant-sor");
    assert_eq!(stdout["providers"][0]["kind"], "memory");
}

#[test]
fn missing_answers_fail_with_paths_in_non_interactive_mode() {
    let (temp, pack) = write_pack();
    let answers = write_json(
        &temp,
        "partial.json",
        &json!({
            "tenant": {},
            "server": {},
            "providers": {},
            "policy": {},
            "audit": {},
            "deployment": {},
            "exposure": {},
            "ghcr": {}
        }),
    );
    let output = greentic_sorx_command()
        .args([
            "--non-interactive",
            "start",
            pack.to_str().unwrap(),
            "--answers",
            answers.to_str().unwrap(),
        ])
        .output()
        .expect("greentic-sorx binary should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("missing required startup answers in non-interactive mode"));
    assert!(stderr.contains("tenant.tenant_id"));
    assert!(stderr.contains("providers.store"));
}

#[test]
fn qa_answer_set_envelope_is_accepted_by_cli() {
    let (temp, pack) = write_pack();
    let answers = write_json(
        &temp,
        "qa-answer-set.json",
        &json!({
            "form_id": "greentic.sorx.start",
            "spec_version": "0.1.0",
            "answers": full_answers()
        }),
    );
    let output = greentic_sorx_command()
        .args([
            "start",
            pack.to_str().unwrap(),
            "--answers",
            answers.to_str().unwrap(),
            "--dry-run",
        ])
        .output()
        .expect("greentic-sorx binary should run");

    assert!(output.status.success(), "{output:?}");
    let stdout: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(stdout["schema"], "greentic.sorx.start.plan.v1");
    assert_eq!(stdout["server"]["bind"], "127.0.0.1:8787");
}

#[test]
fn inline_secret_like_answers_are_rejected() {
    let err = greentic_sorx_core::normalize_start_answers(
        &startup_schema(),
        &json!({
            "tenant": { "tenant_id": "tenant-a" },
            "server": { "public_base_url": "http://127.0.0.1:8787" },
            "providers": {
                "store": {
                    "kind": "memory",
                    "config": { "api_key": "plain-secret-value" }
                }
            },
            "policy": { "approvals": {} },
            "audit": {},
            "deployment": { "tenant_id": "tenant-a", "sor_name": "landlord", "environment": "local" },
            "exposure": {},
            "ghcr": {}
        }),
        true,
    )
    .unwrap_err();

    assert_eq!(err.code, "invalid_answers");
    assert!(
        err.issues
            .iter()
            .any(|issue| issue.path == "providers.store.config.api_key")
    );
}

#[test]
fn direct_provider_config_is_rejected_outside_local_or_test() {
    let err = greentic_sorx_core::normalize_start_answers(
        &startup_schema(),
        &json!({
            "tenant": { "tenant_id": "tenant-a", "environment": "production" },
            "server": { "public_base_url": "https://sorx.example.test" },
            "providers": {
                "store": {
                    "kind": "memory",
                    "config": { "namespace": "tenant-a" }
                }
            },
            "policy": { "approvals": {} },
            "audit": {},
            "deployment": { "tenant_id": "tenant-a", "sor_name": "landlord", "environment": "production" },
            "exposure": {},
            "ghcr": {}
        }),
        true,
    )
    .unwrap_err();

    assert_eq!(err.code, "invalid_answers");
    assert!(
        err.issues
            .iter()
            .any(|issue| issue.path == "providers.store.config")
    );
}

#[test]
fn start_dry_run_sorts_provider_capabilities_stably() {
    let (temp, pack) = write_pack();
    let mut answers = full_answers();
    answers["providers"]["store"]["capabilities"] = json!([
        "ontology-scoped-evidence-query",
        "entity-link",
        "ontology-scoped-evidence-query"
    ]);
    let answers = write_json(&temp, "answers.json", &answers);
    let output = greentic_sorx_command()
        .args([
            "start",
            pack.to_str().unwrap(),
            "--answers",
            answers.to_str().unwrap(),
            "--dry-run",
        ])
        .output()
        .expect("greentic-sorx binary should run");

    assert!(output.status.success(), "{output:?}");
    let stdout: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        stdout["providers"][0]["capabilities"],
        json!(["entity-link", "ontology-scoped-evidence-query"])
    );
}
