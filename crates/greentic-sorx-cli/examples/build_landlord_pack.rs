//! Build the landlord/tenant `.gtpack` to a given output path.
//!
//! This reuses the exact pack-building recipe from the
//! `landlord_tenant_e2e` integration test so a REAL sorx runtime can be
//! started from the produced pack for live (NATS) end-to-end exercises.
//!
//! Usage:
//! ```text
//! cargo run -p greentic-sorx --example build_landlord_pack -- /tmp/landlord.gtpack
//! ```
//! The output path argument is optional; it defaults to `/tmp/landlord.gtpack`.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

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
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const AGENT_GATEWAY: &str =
    include_str!("../tests/e2e/fixtures/landlord_tenant/agent-gateway.json");
const MCP_TOOLS: &str = include_str!("../tests/e2e/fixtures/landlord_tenant/mcp-tools.json");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/landlord.gtpack"));

    build_landlord_pack(&output_path)?;

    println!(
        "Wrote landlord/tenant .gtpack to: {}",
        output_path.display()
    );
    println!("Endpoints contained in the pack:");
    for endpoint in agent_gateway_endpoints()? {
        println!(
            "  {} ({}) -> {} {}",
            endpoint.endpoint_id, endpoint.operation_id, endpoint.method, endpoint.path
        );
    }

    Ok(())
}

/// Human-readable summary of a single agent-gateway endpoint.
struct EndpointSummary {
    endpoint_id: String,
    operation_id: String,
    method: String,
    path: String,
}

/// Builds the landlord/tenant `.gtpack` archive at `output_path`.
///
/// Mirrors `LandlordTenantFixture::new()` from the e2e test: it assembles the
/// sorla manifest, business-action catalog + lock, the start schema, and the
/// top-level gpack manifest/lock, then writes everything into a Stored ZIP.
fn build_landlord_pack(output_path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut entries = pack_entries().into_iter().collect::<BTreeMap<_, _>>();
    if let Some(manifest) = entries.get("pack.cbor").cloned() {
        entries.insert("manifest.cbor".to_string(), gpack_manifest_bytes());
        entries.insert(
            "manifest.json".to_string(),
            serde_json::to_vec_pretty(&ciborium::de::from_reader::<SorxPackManifest, _>(
                manifest.as_slice(),
            )?)?,
        );
    }
    entries.insert(
        "pack.lock.cbor".to_string(),
        encode_cbor(&lock_for_entries(&entries)),
    );

    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }

    let file = std::fs::File::create(output_path)?;
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .unix_permissions(0o644);
    for (name, bytes) in entries {
        writer.start_file(name, options)?;
        writer.write_all(&bytes)?;
    }
    writer.finish()?;
    Ok(())
}

/// Extracts endpoint summaries from the bundled agent-gateway fixture for
/// human-readable reporting.
fn agent_gateway_endpoints() -> Result<Vec<EndpointSummary>, Box<dyn std::error::Error>> {
    let gateway: Value = serde_json::from_str(AGENT_GATEWAY)?;
    let endpoints = gateway["endpoints"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|endpoint| {
            let read = |key: &str| {
                endpoint
                    .get(key)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            EndpointSummary {
                endpoint_id: read("endpoint_id"),
                operation_id: read("operation_id"),
                method: read("method"),
                path: read("path"),
            }
        })
        .collect();
    Ok(endpoints)
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

fn lock_for_entries(entries: &BTreeMap<String, Vec<u8>>) -> PackLock {
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
