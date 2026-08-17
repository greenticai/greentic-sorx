//! Construction of the wizard, setup and Sorx runtime answer documents.

use std::path::Path;

use serde_json::{Value, json};

use super::ids::default_collection_name;

pub(super) fn create_answers_value(
    locale: &str,
    bundle_id: &str,
    bundle_dir: &Path,
    webchat_ref: &str,
) -> Value {
    json!({
        "wizard_id": "greentic-bundle.wizard.run",
        "schema_id": "greentic-bundle.wizard.answers",
        "schema_version": "1.0.0",
        "locale": locale,
        "answers": {
            "access_rules": [],
            "advanced_setup": false,
            "app_pack_entries": [],
            "app_packs": [],
            "bundle_id": bundle_id,
            "bundle_name": bundle_id,
            "export_intent": false,
            "extension_provider_entries": [{
                "detected_kind": "oci",
                "display_name": "Greentic Messaging WebChat GUI (stable)",
                "provider_id": "greentic.messaging.webchat-gui.stable",
                "reference": webchat_ref,
                "version": "stable"
            }],
            "extension_providers": [webchat_ref],
            "mode": "create",
            "output_dir": bundle_dir,
            "remote_catalogs": [],
            "setup_answers": {},
            "setup_execution_intent": false,
            "setup_specs": {}
        }
    })
}

/// Derive one store binding per entity declared by the pack's agent gateway.
pub(super) fn build_entity_bindings(gateway: &Value) -> serde_json::Map<String, Value> {
    let mut entities = serde_json::Map::new();
    for endpoint in gateway
        .get("endpoints")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(entity) = endpoint
            .get("entity")
            .or_else(|| endpoint.get("record"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let collection = endpoint
            .get("collection")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| default_collection_name(entity));
        entities
            .entry(entity.to_string())
            .or_insert_with(|| json!({"provider": "store", "collection": collection}));
    }
    entities
}

pub(super) fn sorx_answers_value(
    bind: &str,
    base: &str,
    pack_name: &str,
    entities: serde_json::Map<String, Value>,
) -> Value {
    json!({
        "tenant": {"tenant_id": "demo", "environment": "local"},
        "server": {
            "bind": bind,
            "public_base_url": base,
            "auth": {"mode": "none"}
        },
        "mcp": {"enabled": false, "bind": "127.0.0.1:8790"},
        "providers": {"store": {"kind": "memory", "config_ref": "providers.memory.local"}},
        "bindings": {"entities": entities},
        "policy": {"approvals": {"low": "auto", "medium": "auto", "high": "require_approval", "critical": "deny"}},
        "audit": {"sink": "stdout"},
        "deployment": {
            "tenant_id": "demo",
            "sor_name": pack_name,
            "environment": "local",
            "deployment_mode": "local_single",
            "api_version_label": "local",
            "base_path": "/"
        },
        "exposure": {},
        "ghcr": {}
    })
}

/// Point an existing answers document at the runtime's bind address, whether it
/// is wrapped in an `answers` envelope or is a bare answers object.
pub(super) fn apply_server_overrides(
    value: &mut Value,
    bind: &str,
    base: &str,
) -> Result<(), String> {
    let root = if value.get("answers").is_some_and(Value::is_object) {
        value
            .get_mut("answers")
            .and_then(Value::as_object_mut)
            .expect("answers object checked above")
    } else {
        value
            .as_object_mut()
            .ok_or_else(|| "SORX startup answers must be a JSON object".to_string())?
    };
    let server = root
        .entry("server".to_string())
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| "SORX startup answers `server` must be an object".to_string())?;
    server.insert("bind".to_string(), json!(bind));
    server.insert("public_base_url".to_string(), json!(base));
    Ok(())
}

pub(super) fn setup_answers_value(webchat_url: &str, sorx_base: &str) -> Value {
    json!({
        "bundle_source": ".",
        "env": "dev",
        "greentic_setup_version": "1.0.0",
        "platform_setup": {
            "deployment_targets": [],
            "static_routes": {
                "default_route_prefix_policy": "pack_declared",
                "public_base_url": webchat_url,
                "public_surface_policy": "enabled",
                "public_web_enabled": true,
                "tenant_path_policy": "pack_declared"
            },
            "tunnel": {"mode": "off"}
        },
        "setup_answers": {
            "messaging-webchat-gui": {
                "base_url": webchat_url,
                "jwt_signing_key": "sorx-manager-local-signing-key-0123456789abcdef",
                "mode": "local_queue",
                "nav_links": [
                    {
                        "id": "sorx-manager",
                        "label": "Sorx Manager",
                        "url": format!("{sorx_base}/v1/sorx/manager")
                    },
                    {
                        "id": "sorx-dashboard-card",
                        "label": "Dashboard Card",
                        "url": format!("{sorx_base}/v1/sorx/manager/cards/dashboard")
                    }
                ],
                "presentation_mode": "standalone",
                "public_base_url": webchat_url,
                "route": "webchat",
                "skin": "default",
                "tenant_channel_id": "demo:webchat",
                "text_input_enabled": false
            }
        },
        "team": "default",
        "tenant": "demo"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "http://127.0.0.1:8788";

    #[test]
    fn create_answers_value_pins_the_bundle_id_output_dir_and_webchat_provider() {
        let value = create_answers_value(
            "en",
            "sorx-manager-demo",
            Path::new("/tmp/sorx-manager-demo-bundle"),
            "oci://example/webchat:stable",
        );
        assert_eq!(value["locale"], "en");
        assert_eq!(value["wizard_id"], "greentic-bundle.wizard.run");
        let answers = &value["answers"];
        assert_eq!(answers["bundle_id"], "sorx-manager-demo");
        assert_eq!(answers["bundle_name"], "sorx-manager-demo");
        assert_eq!(answers["mode"], "create");
        assert_eq!(answers["output_dir"], "/tmp/sorx-manager-demo-bundle");
        assert_eq!(
            answers["extension_providers"],
            json!(["oci://example/webchat:stable"])
        );
        assert_eq!(
            answers["extension_provider_entries"][0]["reference"],
            "oci://example/webchat:stable"
        );
        assert_eq!(
            answers["extension_provider_entries"][0]["detected_kind"],
            "oci"
        );
        assert_eq!(answers["advanced_setup"], false);
        assert_eq!(answers["export_intent"], false);
    }

    #[test]
    fn build_entity_bindings_derives_a_default_collection_from_the_entity() {
        let gateway = json!({"endpoints": [{"entity": "Tenant"}]});
        let entities = build_entity_bindings(&gateway);
        assert_eq!(
            entities["Tenant"],
            json!({"provider": "store", "collection": "tenants"})
        );
    }

    #[test]
    fn build_entity_bindings_prefers_an_explicit_collection() {
        let gateway = json!({"endpoints": [{"entity": "Tenant", "collection": "people"}]});
        assert_eq!(
            build_entity_bindings(&gateway)["Tenant"]["collection"],
            "people"
        );
    }

    #[test]
    fn build_entity_bindings_falls_back_to_the_record_key() {
        let gateway = json!({"endpoints": [{"record": "Property"}]});
        assert_eq!(
            build_entity_bindings(&gateway)["Property"]["collection"],
            "propertys"
        );
    }

    #[test]
    fn build_entity_bindings_prefers_entity_over_record() {
        let gateway = json!({"endpoints": [{"entity": "A", "record": "B"}]});
        let entities = build_entity_bindings(&gateway);
        assert!(entities.contains_key("A"));
        assert!(!entities.contains_key("B"));
    }

    #[test]
    fn build_entity_bindings_keeps_the_first_binding_for_a_repeated_entity() {
        let gateway = json!({"endpoints": [
            {"entity": "Tenant", "collection": "first"},
            {"entity": "Tenant", "collection": "second"}
        ]});
        assert_eq!(
            build_entity_bindings(&gateway)["Tenant"]["collection"],
            "first"
        );
    }

    #[test]
    fn build_entity_bindings_skips_endpoints_with_no_usable_entity() {
        let gateway = json!({"endpoints": [
            {},
            {"entity": ""},
            {"entity": 42},
            {"collection": "orphans"}
        ]});
        assert!(build_entity_bindings(&gateway).is_empty());
    }

    #[test]
    fn build_entity_bindings_ignores_an_empty_collection_and_a_missing_endpoints_array() {
        let gateway = json!({"endpoints": [{"entity": "Tenant", "collection": ""}]});
        assert_eq!(
            build_entity_bindings(&gateway)["Tenant"]["collection"],
            "tenants"
        );

        assert!(build_entity_bindings(&json!({})).is_empty());
        assert!(build_entity_bindings(&json!({"endpoints": "nope"})).is_empty());
    }

    #[test]
    fn sorx_answers_value_wires_the_bind_address_pack_name_and_entities() {
        let entities = build_entity_bindings(&json!({"endpoints": [{"entity": "Tenant"}]}));
        let value = sorx_answers_value("127.0.0.1:8788", BASE, "landlord-tenant", entities);
        assert_eq!(value["server"]["bind"], "127.0.0.1:8788");
        assert_eq!(value["server"]["public_base_url"], BASE);
        assert_eq!(value["server"]["auth"]["mode"], "none");
        assert_eq!(value["deployment"]["sor_name"], "landlord-tenant");
        assert_eq!(value["deployment"]["deployment_mode"], "local_single");
        assert_eq!(value["providers"]["store"]["kind"], "memory");
        assert_eq!(
            value["bindings"]["entities"]["Tenant"]["collection"],
            "tenants"
        );
        assert_eq!(value["policy"]["approvals"]["critical"], "deny");
        assert_eq!(value["mcp"]["enabled"], false);
    }

    #[test]
    fn apply_server_overrides_rewrites_a_bare_answers_object() {
        let mut value = json!({"tenant": {"tenant_id": "demo"}});
        apply_server_overrides(&mut value, "127.0.0.1:9000", BASE).expect("applies");
        assert_eq!(value["server"]["bind"], "127.0.0.1:9000");
        assert_eq!(value["server"]["public_base_url"], BASE);
        assert_eq!(value["tenant"]["tenant_id"], "demo");
    }

    #[test]
    fn apply_server_overrides_targets_the_inner_object_of_an_answers_envelope() {
        let mut value = json!({"schema_id": "x", "answers": {"server": {"bind": "stale", "auth": {"mode": "none"}}}});
        apply_server_overrides(&mut value, "127.0.0.1:9000", BASE).expect("applies");
        assert_eq!(value["answers"]["server"]["bind"], "127.0.0.1:9000");
        assert_eq!(value["answers"]["server"]["public_base_url"], BASE);
        // Untouched sibling keys survive.
        assert_eq!(value["answers"]["server"]["auth"]["mode"], "none");
        assert_eq!(value["schema_id"], "x");
        assert!(value.get("server").is_none());
    }

    #[test]
    fn apply_server_overrides_treats_a_non_object_answers_key_as_a_bare_document() {
        let mut value = json!({"answers": "not-an-object"});
        apply_server_overrides(&mut value, "bind", BASE).expect("applies");
        assert_eq!(value["server"]["bind"], "bind");
        assert_eq!(value["answers"], "not-an-object");
    }

    #[test]
    fn apply_server_overrides_rejects_a_non_object_document() {
        let mut value = json!([1, 2, 3]);
        assert_eq!(
            apply_server_overrides(&mut value, "bind", BASE).unwrap_err(),
            "SORX startup answers must be a JSON object"
        );
    }

    #[test]
    fn apply_server_overrides_rejects_a_non_object_server_key() {
        let mut value = json!({"server": "nope"});
        assert_eq!(
            apply_server_overrides(&mut value, "bind", BASE).unwrap_err(),
            "SORX startup answers `server` must be an object"
        );
    }

    #[test]
    fn setup_answers_value_points_webchat_at_the_sorx_manager_routes() {
        let value = setup_answers_value("http://127.0.0.1:3000", BASE);
        let webchat = &value["setup_answers"]["messaging-webchat-gui"];
        assert_eq!(webchat["base_url"], "http://127.0.0.1:3000");
        assert_eq!(webchat["public_base_url"], "http://127.0.0.1:3000");
        assert_eq!(webchat["route"], "webchat");
        assert_eq!(webchat["text_input_enabled"], false);
        assert_eq!(
            webchat["nav_links"][0]["url"],
            format!("{BASE}/v1/sorx/manager")
        );
        assert_eq!(
            webchat["nav_links"][1]["url"],
            format!("{BASE}/v1/sorx/manager/cards/dashboard")
        );
        assert_eq!(
            value["platform_setup"]["static_routes"]["public_base_url"],
            "http://127.0.0.1:3000"
        );
        assert_eq!(value["platform_setup"]["tunnel"]["mode"], "off");
        assert_eq!(value["tenant"], "demo");
        assert_eq!(value["team"], "default");
    }
}
