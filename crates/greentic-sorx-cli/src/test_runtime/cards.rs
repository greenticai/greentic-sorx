//! AdaptiveCard construction, WebChat normalisation and i18n extraction.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{Value, json};

use super::ids::{humanize, locale_codes, role_card_id};
use super::packing::{sorted_files, write_json};

pub(super) fn welcome_card(locale: &str, pack_id: &str, roles: &[String]) -> Value {
    let actions = roles
        .iter()
        .map(|role| {
            let card_id = role_card_id(role, "dashboard");
            json!({
                "type": "Action.Submit",
                "title": format!("Open as {}", humanize(role)),
                "data": {
                    "routeToCardId": card_id,
                    "cardId": card_id,
                    "action": card_id,
                    "sorx_role": role
                }
            })
        })
        .collect::<Vec<_>>();
    json!({
        "type": "AdaptiveCard",
        "version": "1.5",
        "lang": locale,
        "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
        "metadata": {"locale": locale},
        "body": [
            {"type": "TextBlock", "text": humanize(pack_id), "size": "Large", "weight": "Bolder", "wrap": true},
            {"type": "TextBlock", "text": "Continue to the manager dashboard card to inspect records and card navigation.", "wrap": true}
        ],
        "actions": actions
    })
}

pub(super) fn placeholder_dashboard_card(locale: &str) -> Value {
    json!({
        "type": "AdaptiveCard",
        "version": "1.5",
        "lang": locale,
        "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
        "metadata": {"locale": locale},
        "body": [
            {"type": "TextBlock", "text": "Sorx dashboard is starting", "size": "Large", "weight": "Bolder", "wrap": true},
            {"type": "TextBlock", "text": "The live dashboard card will be injected here after the Sorx runtime is ready.", "wrap": true}
        ],
        "actions": []
    })
}

pub(super) fn normalize_card_for_webchat(card: &mut Value, sorx_base: &str, role: &str) {
    normalize_actions(card, sorx_base, role);
    normalize_card_items(card);
}

fn normalize_actions(value: &mut Value, sorx_base: &str, role: &str) {
    match value {
        Value::Object(map) => {
            let is_submit = map
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|value| value == "Action.Submit");
            if is_submit && let Some(data) = map.get_mut("data").and_then(Value::as_object_mut) {
                if data.get("action").and_then(Value::as_str) == Some("manager_submit") {
                    data.entry("manager_submit_url".to_string())
                        .or_insert_with(|| json!(format!("{sorx_base}/v1/sorx/manager/submit")));
                    if let Some(record) = data
                        .get("record")
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned)
                    {
                        let target = format!("records/{record}");
                        let card_id = role_card_id(role, &target);
                        data.entry("manager_target".to_string())
                            .or_insert_with(|| json!(target));
                        data.entry("manager_cards_base_url".to_string())
                            .or_insert_with(|| json!(format!("{sorx_base}/v1/sorx/manager/cards")));
                        data.entry("routeToCardId".to_string())
                            .or_insert_with(|| json!(card_id));
                        data.entry("cardId".to_string())
                            .or_insert_with(|| json!(role_card_id(role, &target)));
                        data.entry("step".to_string())
                            .or_insert_with(|| json!("submit"));
                        data.entry("sorx_role".to_string())
                            .or_insert_with(|| json!(role));
                    }
                }
                if let Some(target) = data
                    .get("manager_target")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                {
                    let card_id = role_card_id(role, &target);
                    data.entry("manager_cards_base_url".to_string())
                        .or_insert_with(|| json!(format!("{sorx_base}/v1/sorx/manager/cards")));
                    data.insert("routeToCardId".to_string(), json!(card_id));
                    data.entry("cardId".to_string())
                        .or_insert_with(|| json!(role_card_id(role, &target)));
                    data.entry("step".to_string())
                        .or_insert_with(|| json!("open"));
                    data.entry("action".to_string())
                        .or_insert_with(|| json!(role_card_id(role, &target)));
                    data.entry("sorx_role".to_string())
                        .or_insert_with(|| json!(role));
                }
            }
            for child in map.values_mut() {
                normalize_actions(child, sorx_base, role);
            }
        }
        Value::Array(items) => {
            for child in items {
                normalize_actions(child, sorx_base, role);
            }
        }
        _ => {}
    }
}

fn normalize_card_items(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("TextBlock") {
                for key in ["size", "weight"] {
                    if let Some(text) = map.get(key).and_then(Value::as_str) {
                        let mut chars = text.chars();
                        if let Some(first) = chars.next() {
                            *map.get_mut(key).unwrap() = Value::String(format!(
                                "{}{}",
                                first.to_uppercase(),
                                chars.as_str()
                            ));
                        }
                    }
                }
            }
            if map.get("type").and_then(Value::as_str) == Some("Input.Text") {
                let label = map
                    .get("label")
                    .or_else(|| map.get("placeholder"))
                    .or_else(|| map.get("id"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                if let Some(label) = label {
                    map.entry("label".to_string())
                        .or_insert_with(|| json!(label));
                    map.entry("placeholder".to_string())
                        .or_insert_with(|| json!(label));
                    if map.get("isRequired").and_then(Value::as_bool) == Some(true) {
                        map.entry("errorMessage".to_string())
                            .or_insert_with(|| json!(format!("{label} is required.")));
                    }
                }
            }
            for child in map.values_mut() {
                normalize_card_items(child);
            }
        }
        Value::Array(items) => {
            for child in items {
                normalize_card_items(child);
            }
        }
        _ => {}
    }
}

pub(super) fn collect_navigable_targets(card: &Value) -> Vec<String> {
    let mut targets = BTreeSet::new();
    collect_targets(card, &mut targets);
    targets
        .into_iter()
        .filter(|target| {
            target == "metrics"
                || target.starts_with("metrics/")
                || (target.starts_with("records/") && !target.ends_with("/create"))
        })
        .collect()
}

fn collect_targets(value: &Value, targets: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("Action.Submit")
                && let Some(target) = map
                    .get("data")
                    .and_then(Value::as_object)
                    .and_then(|data| data.get("manager_target"))
                    .and_then(Value::as_str)
            {
                targets.insert(target.to_string());
            }
            for child in map.values() {
                collect_targets(child, targets);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_targets(child, targets);
            }
        }
        _ => {}
    }
}

pub(super) fn write_card_i18n(cards_dir: &Path, locale: &str) -> Result<(), String> {
    let i18n_dir = cards_dir
        .parent()
        .ok_or_else(|| format!("cards directory has no parent: {}", cards_dir.display()))?
        .join("i18n");
    std::fs::create_dir_all(&i18n_dir)
        .map_err(|err| format!("failed to create {}: {err}", i18n_dir.display()))?;
    let mut en = serde_json::Map::new();
    for file in sorted_files(cards_dir)? {
        if file.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Some(card_name) = file.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        let Ok(value) = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .ok_or(())
        else {
            continue;
        };
        collect_i18n(card_name, &value, Vec::new(), &mut en);
    }
    write_json(
        &i18n_dir.join("_manifest.json"),
        &json!({"locales": locale_codes(locale)}),
    )?;
    let en_value = Value::Object(en);
    write_json(&i18n_dir.join("en.json"), &en_value)?;
    for code in locale_codes(locale) {
        if code != "en" {
            write_json(&i18n_dir.join(format!("{code}.json")), &en_value)?;
        }
    }
    Ok(())
}

fn collect_i18n(
    card_name: &str,
    value: &Value,
    path: Vec<String>,
    out: &mut serde_json::Map<String, Value>,
) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if ["text", "title", "label", "placeholder", "errorMessage"].contains(&key.as_str())
                {
                    if let Some(text) = child.as_str() {
                        out.insert(
                            format!("cards.{card_name}.{}.{}", path.join("."), key),
                            json!(text),
                        );
                    }
                } else {
                    let mut next = path.clone();
                    next.push(key.clone());
                    collect_i18n(card_name, child, next, out);
                }
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                let mut next = path.clone();
                next.push(format!("i{index}"));
                collect_i18n(card_name, child, next, out);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const BASE: &str = "http://127.0.0.1:8788";

    fn roles(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn welcome_card_renders_one_submit_action_per_role() {
        let card = welcome_card("en", "landlord-tenant-sor", &roles(&["landlord", "tenant"]));
        let actions = card["actions"].as_array().expect("actions");
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0]["title"], "Open as Landlord");
        assert_eq!(
            actions[0]["data"]["routeToCardId"],
            "roles_landlord_dashboard"
        );
        assert_eq!(actions[0]["data"]["cardId"], "roles_landlord_dashboard");
        assert_eq!(actions[0]["data"]["action"], "roles_landlord_dashboard");
        assert_eq!(actions[0]["data"]["sorx_role"], "landlord");
        assert_eq!(actions[1]["data"]["sorx_role"], "tenant");
    }

    #[test]
    fn welcome_card_humanizes_the_pack_id_as_its_heading() {
        let card = welcome_card("pt-BR", "landlord-tenant-sor", &roles(&["admin"]));
        assert_eq!(card["body"][0]["text"], "Landlord Tenant Sor");
        assert_eq!(card["lang"], "pt-BR");
        assert_eq!(card["metadata"]["locale"], "pt-BR");
    }

    #[test]
    fn welcome_card_with_no_roles_has_no_actions() {
        let card = welcome_card("en", "demo", &[]);
        assert_eq!(card["actions"].as_array().expect("actions").len(), 0);
    }

    #[test]
    fn placeholder_dashboard_card_carries_the_locale_and_no_actions() {
        let card = placeholder_dashboard_card("es");
        assert_eq!(card["lang"], "es");
        assert_eq!(card["metadata"]["locale"], "es");
        assert_eq!(card["type"], "AdaptiveCard");
        assert_eq!(card["actions"].as_array().expect("actions").len(), 0);
    }

    #[test]
    fn normalize_actions_expands_a_manager_submit_with_a_record() {
        let mut card = json!({
            "type": "Action.Submit",
            "data": {"action": "manager_submit", "record": "tenant"}
        });
        normalize_card_for_webchat(&mut card, BASE, "landlord");
        let data = &card["data"];
        assert_eq!(
            data["manager_submit_url"],
            format!("{BASE}/v1/sorx/manager/submit")
        );
        assert_eq!(data["manager_target"], "records/tenant");
        assert_eq!(
            data["manager_cards_base_url"],
            format!("{BASE}/v1/sorx/manager/cards")
        );
        assert_eq!(data["routeToCardId"], "roles_landlord_records_tenant");
        assert_eq!(data["cardId"], "roles_landlord_records_tenant");
        assert_eq!(data["step"], "submit");
        assert_eq!(data["sorx_role"], "landlord");
        // `action` is preserved: the submit branch never overwrites it.
        assert_eq!(data["action"], "manager_submit");
    }

    #[test]
    fn normalize_actions_adds_only_the_submit_url_when_the_record_is_missing_or_empty() {
        for data in [
            json!({"action": "manager_submit"}),
            json!({"action": "manager_submit", "record": ""}),
        ] {
            let mut card = json!({"type": "Action.Submit", "data": data});
            normalize_card_for_webchat(&mut card, BASE, "admin");
            assert_eq!(
                card["data"]["manager_submit_url"],
                format!("{BASE}/v1/sorx/manager/submit")
            );
            assert!(card["data"].get("manager_target").is_none());
            assert!(card["data"].get("routeToCardId").is_none());
        }
    }

    #[test]
    fn normalize_actions_expands_a_bare_manager_target_as_an_open_action() {
        let mut card = json!({
            "type": "Action.Submit",
            "data": {"manager_target": "records/tenant"}
        });
        normalize_card_for_webchat(&mut card, BASE, "admin");
        let data = &card["data"];
        assert_eq!(data["routeToCardId"], "roles_admin_records_tenant");
        assert_eq!(data["cardId"], "roles_admin_records_tenant");
        assert_eq!(data["step"], "open");
        assert_eq!(data["action"], "roles_admin_records_tenant");
        assert_eq!(data["sorx_role"], "admin");
        assert!(data.get("manager_submit_url").is_none());
    }

    #[test]
    fn normalize_actions_overwrites_route_to_card_id_but_preserves_other_existing_keys() {
        let mut card = json!({
            "type": "Action.Submit",
            "data": {
                "manager_target": "records/tenant",
                "routeToCardId": "stale",
                "cardId": "kept",
                "step": "kept-step",
                "sorx_role": "kept-role"
            }
        });
        normalize_card_for_webchat(&mut card, BASE, "admin");
        let data = &card["data"];
        assert_eq!(data["routeToCardId"], "roles_admin_records_tenant");
        assert_eq!(data["cardId"], "kept");
        assert_eq!(data["step"], "kept-step");
        assert_eq!(data["sorx_role"], "kept-role");
    }

    #[test]
    fn normalize_actions_recurses_into_nested_arrays_and_objects() {
        let mut card = json!({
            "type": "AdaptiveCard",
            "body": [{"type": "Container", "items": [
                {"type": "Action.Submit", "data": {"manager_target": "metrics"}}
            ]}]
        });
        normalize_card_for_webchat(&mut card, BASE, "admin");
        let data = &card["body"][0]["items"][0]["data"];
        assert_eq!(data["routeToCardId"], "roles_admin_metrics");
        assert_eq!(data["step"], "open");
    }

    #[test]
    fn normalize_actions_ignores_non_submit_nodes_and_non_object_data() {
        let mut card = json!({
            "type": "Action.OpenUrl",
            "data": {"manager_target": "records/tenant"}
        });
        normalize_card_for_webchat(&mut card, BASE, "admin");
        assert!(card["data"].get("routeToCardId").is_none());

        let mut scalar_data = json!({"type": "Action.Submit", "data": "not-an-object"});
        normalize_card_for_webchat(&mut scalar_data, BASE, "admin");
        assert_eq!(scalar_data["data"], "not-an-object");
    }

    #[test]
    fn normalize_card_items_capitalises_text_block_size_and_weight() {
        let mut card = json!({"type": "TextBlock", "size": "large", "weight": "bolder"});
        normalize_card_for_webchat(&mut card, BASE, "admin");
        assert_eq!(card["size"], "Large");
        assert_eq!(card["weight"], "Bolder");
    }

    #[test]
    fn normalize_card_items_leaves_already_capitalised_and_non_string_values_alone() {
        let mut card = json!({"type": "TextBlock", "size": "Large", "weight": 3});
        normalize_card_for_webchat(&mut card, BASE, "admin");
        assert_eq!(card["size"], "Large");
        assert_eq!(card["weight"], 3);

        let mut empty = json!({"type": "TextBlock", "size": ""});
        normalize_card_for_webchat(&mut empty, BASE, "admin");
        assert_eq!(empty["size"], "");
    }

    #[test]
    fn normalize_card_items_backfills_input_label_and_placeholder_from_the_id() {
        let mut card = json!({"type": "Input.Text", "id": "tenant_name"});
        normalize_card_for_webchat(&mut card, BASE, "admin");
        assert_eq!(card["label"], "tenant_name");
        assert_eq!(card["placeholder"], "tenant_name");
        assert!(card.get("errorMessage").is_none());
    }

    #[test]
    fn normalize_card_items_prefers_label_then_placeholder_then_id() {
        let mut labelled =
            json!({"type": "Input.Text", "label": "L", "placeholder": "P", "id": "I"});
        normalize_card_for_webchat(&mut labelled, BASE, "admin");
        assert_eq!(labelled["label"], "L");
        assert_eq!(labelled["placeholder"], "P");

        let mut placeheld = json!({"type": "Input.Text", "placeholder": "P", "id": "I"});
        normalize_card_for_webchat(&mut placeheld, BASE, "admin");
        assert_eq!(placeheld["label"], "P");
    }

    #[test]
    fn normalize_card_items_adds_an_error_message_only_for_required_inputs() {
        let mut required = json!({"type": "Input.Text", "id": "email", "isRequired": true});
        normalize_card_for_webchat(&mut required, BASE, "admin");
        assert_eq!(required["errorMessage"], "email is required.");

        let mut optional = json!({"type": "Input.Text", "id": "email", "isRequired": false});
        normalize_card_for_webchat(&mut optional, BASE, "admin");
        assert!(optional.get("errorMessage").is_none());
    }

    #[test]
    fn normalize_card_items_ignores_an_input_with_no_label_placeholder_or_id() {
        let mut card = json!({"type": "Input.Text", "isRequired": true});
        normalize_card_for_webchat(&mut card, BASE, "admin");
        assert!(card.get("label").is_none());
        assert!(card.get("errorMessage").is_none());
    }

    #[test]
    fn collect_navigable_targets_keeps_metrics_and_record_list_targets() {
        let card = json!([
            {"type": "Action.Submit", "data": {"manager_target": "metrics"}},
            {"type": "Action.Submit", "data": {"manager_target": "metrics/open"}},
            {"type": "Action.Submit", "data": {"manager_target": "records/tenant"}}
        ]);
        assert_eq!(
            collect_navigable_targets(&card),
            vec!["metrics", "metrics/open", "records/tenant"]
        );
    }

    #[test]
    fn collect_navigable_targets_drops_create_forms_and_unknown_prefixes() {
        let card = json!([
            {"type": "Action.Submit", "data": {"manager_target": "records/tenant/create"}},
            {"type": "Action.Submit", "data": {"manager_target": "dashboard"}},
            {"type": "Action.Submit", "data": {"manager_target": "metricsx"}}
        ]);
        assert!(collect_navigable_targets(&card).is_empty());
    }

    #[test]
    fn collect_navigable_targets_deduplicates_and_sorts() {
        let card = json!([
            {"type": "Action.Submit", "data": {"manager_target": "records/b"}},
            {"type": "Action.Submit", "data": {"manager_target": "records/a"}},
            {"type": "Action.Submit", "data": {"manager_target": "records/a"}}
        ]);
        assert_eq!(
            collect_navigable_targets(&card),
            vec!["records/a", "records/b"]
        );
    }

    #[test]
    fn collect_navigable_targets_ignores_non_submit_nodes() {
        let card = json!({"type": "Action.OpenUrl", "data": {"manager_target": "records/tenant"}});
        assert!(collect_navigable_targets(&card).is_empty());
    }

    #[test]
    fn write_card_i18n_extracts_translatable_strings_from_every_card() {
        let dir = TempDir::new().expect("tempdir");
        let cards = dir.path().join("assets/cards");
        std::fs::create_dir_all(&cards).expect("create cards dir");
        write_json(
            &cards.join("welcome.json"),
            &json!({"body": [{"text": "Hello", "size": "Large"}], "actions": [{"title": "Go"}]}),
        )
        .expect("write card");
        // Non-JSON files and unparseable JSON are skipped, not fatal.
        std::fs::write(cards.join("notes.txt"), "ignored").expect("write txt");
        std::fs::write(cards.join("broken.json"), "{ not json").expect("write broken");

        write_card_i18n(&cards, "pt-BR").expect("write i18n");

        let i18n = dir.path().join("assets/i18n");
        let en: Value =
            serde_json::from_str(&std::fs::read_to_string(i18n.join("en.json")).expect("read"))
                .expect("parse");
        assert_eq!(en["cards.welcome.body.i0.text"], "Hello");
        assert_eq!(en["cards.welcome.actions.i0.title"], "Go");
        assert!(en.get("cards.welcome.body.i0.size").is_none());
        assert!(en.get("cards.broken.text").is_none());

        let manifest: Value = serde_json::from_str(
            &std::fs::read_to_string(i18n.join("_manifest.json")).expect("read"),
        )
        .expect("parse");
        assert_eq!(manifest["locales"], json!(["en", "es", "pt-BR", "pt"]));

        // Every non-en locale is seeded with the English strings.
        for code in ["es", "pt-BR", "pt"] {
            let value: Value = serde_json::from_str(
                &std::fs::read_to_string(i18n.join(format!("{code}.json"))).expect("read"),
            )
            .expect("parse");
            assert_eq!(value, en);
        }
    }

    #[test]
    fn write_card_i18n_reports_a_cards_directory_with_no_parent() {
        let err = write_card_i18n(Path::new("/"), "en").unwrap_err();
        assert_eq!(err, "cards directory has no parent: /");
    }

    #[test]
    fn write_card_i18n_reports_a_missing_cards_directory() {
        let dir = TempDir::new().expect("tempdir");
        let err = write_card_i18n(&dir.path().join("assets/cards"), "en").unwrap_err();
        assert!(err.starts_with("failed to read"), "unexpected error: {err}");
    }

    #[test]
    fn collect_i18n_walks_nested_arrays_and_objects() {
        let mut out = serde_json::Map::new();
        collect_i18n(
            "card",
            &json!({"body": [{"items": [{"label": "L", "errorMessage": "E"}]}]}),
            Vec::new(),
            &mut out,
        );
        assert_eq!(out["cards.card.body.i0.items.i0.label"], "L");
        assert_eq!(out["cards.card.body.i0.items.i0.errorMessage"], "E");
    }

    #[test]
    fn collect_i18n_ignores_non_string_values_for_translatable_keys() {
        let mut out = serde_json::Map::new();
        collect_i18n(
            "card",
            &json!({"text": 42, "title": null}),
            Vec::new(),
            &mut out,
        );
        assert!(out.is_empty());
    }
}
