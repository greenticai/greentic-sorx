use serde::{Deserialize, Serialize};

pub const SORX_PRESENCE_SCHEMA: &str = "greentic.sorx.presence.v1";

/// Announces a running SoR instance's reachability and capability offer over
/// NATS push-discovery, so consumers can find it without a directory lookup.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SorxPresence {
    pub schema: String,
    pub instance_id: String,
    pub tenant: String,
    pub environment: String,
    pub sor: String,
    pub pack_version: String,
    pub base_url: String,
    pub reachable: bool,
    pub offers: crate::generic_runtime::RuntimeCapabilities,
    pub ts: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generic_runtime::RuntimeCapabilities;

    #[test]
    fn sorx_presence_json_round_trips() {
        let p = SorxPresence {
            schema: SORX_PRESENCE_SCHEMA.to_string(),
            instance_id: "sorx-abc-1".into(),
            tenant: "acme".into(),
            environment: "local".into(),
            sor: "landlord-tenant-sor".into(),
            pack_version: "0.1.0".into(),
            base_url: "https://sor.acme.example".into(),
            reachable: true,
            offers: RuntimeCapabilities::sorx_runtime_host(),
            ts: "2026-07-27T00:00:00Z".into(),
        };
        let bytes = serde_json::to_vec(&p).unwrap();
        let back: SorxPresence = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(p, back);
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["schema"], SORX_PRESENCE_SCHEMA);
        assert_eq!(v["reachable"], true);
    }
}
