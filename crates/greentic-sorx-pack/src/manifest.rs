use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const EXT_CAPABILITIES_V1: &str = "greentic.ext.capabilities.v1";
pub const SORX_DEPLOYER_DESCRIPTOR: &str = "greentic.deployer.sorx@0.1.0";
pub const SORX_RUNTIME_HOST_PACK_ID: &str = "greentic-sorx-runtime-host";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackManifest {
    pub schema: String,
    pub pack: PackIdentity,
    #[serde(default)]
    pub extension: serde_json::Value,
    #[serde(default)]
    pub integrity: Option<PackIntegrity>,
    #[serde(default)]
    pub assets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackIdentity {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackIntegrity {
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub signature: Option<String>,
    #[serde(default)]
    pub signature_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackLock {
    pub schema: String,
    pub entries: BTreeMap<String, PackLockEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackLockEntry {
    pub size: u64,
    pub sha256: String,
}

pub fn sorx_runtime_host_capabilities() -> Value {
    json!({
        "schema": "greentic.capabilities.v1",
        "offers": [
            {
                "capability": "greentic.cap.runtime.host.v1",
                "contracts": [
                    "greentic.runtime.admin.v1",
                    "greentic.runtime.health.v1",
                    "greentic.runtime.deployments.v1",
                    "greentic.runtime.traffic.v1",
                    "greentic.runtime.invoke.v1"
                ]
            }
        ],
        "requires": [
            { "capability": "greentic.cap.secrets.v1", "optional": true },
            { "capability": "greentic.cap.telemetry.v1", "optional": true },
            { "capability": "greentic.cap.extension.control.v1", "optional": true },
            { "capability": "greentic.cap.extension.observer.v1", "optional": true },
            { "capability": "greentic.cap.extension.admin.v1", "optional": true }
        ]
    })
}

pub fn sorx_runtime_host_extension() -> Value {
    json!({
        "extension": EXT_CAPABILITIES_V1,
        "descriptor": SORX_DEPLOYER_DESCRIPTOR,
        "pack_id": SORX_RUNTIME_HOST_PACK_ID,
        "capabilities": sorx_runtime_host_capabilities()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorx_runtime_host_extension_declares_generic_capabilities() {
        let extension = sorx_runtime_host_extension();
        assert_eq!(extension["extension"], EXT_CAPABILITIES_V1);
        assert_eq!(extension["descriptor"], SORX_DEPLOYER_DESCRIPTOR);
        assert_eq!(
            extension["capabilities"]["offers"][0]["capability"],
            "greentic.cap.runtime.host.v1"
        );
        assert!(
            extension["capabilities"]["offers"][0]["contracts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|contract| contract == "greentic.runtime.health.v1")
        );
    }
}
