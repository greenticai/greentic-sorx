use serde_json::{Value, json};

use super::{ManagerActionView, ManagerFieldView, ManagerRecordView, ManagerViewModel};

pub fn render_dashboard_card(view: &ManagerViewModel) -> Value {
    adaptive_card(
        view,
        "manager.dashboard",
        vec![
            text_block(&view.title, "large", true),
            text_block(&view.description, "default", false),
        ],
        view.navigation
            .iter()
            .map(|item| open_action(&item.label, &format!("records/{}", item.record)))
            .collect(),
    )
}

pub fn render_record_list_card(view: &ManagerViewModel, record_name: &str) -> Option<Value> {
    let record = view
        .records
        .iter()
        .find(|record| record.record == record_name)?;
    Some(adaptive_card(
        view,
        "manager.record.list",
        vec![
            text_block(&record.plural_label, "large", true),
            text_block(&record.collection, "default", false),
        ],
        vec![
            open_action(
                localized_static(&view.locale, "Create"),
                &format!("records/{}/create", record.record),
            ),
            open_action(localized_static(&view.locale, "Dashboard"), "dashboard"),
        ],
    ))
}

pub fn render_record_create_card(view: &ManagerViewModel, record_name: &str) -> Option<Value> {
    let record = view
        .records
        .iter()
        .find(|record| record.record == record_name)?;
    let mut body = form_body(&record.label, record, FormScope::Create);
    body.push(form_action_set(
        &view.locale,
        &record.record,
        None,
        create_action(view, &record.record),
    ));
    Some(adaptive_card(
        view,
        "manager.record.create",
        body,
        Vec::new(),
    ))
}

pub fn render_record_detail_card(
    view: &ManagerViewModel,
    record_name: &str,
    id: &str,
) -> Option<Value> {
    let record = view
        .records
        .iter()
        .find(|record| record.record == record_name)?;
    let can_update = update_action(view, &record.record).is_some();
    let mut body = form_body(
        &record.label,
        record,
        if can_update {
            FormScope::RecordEditable
        } else {
            FormScope::RecordReadOnly
        },
    );
    if can_update {
        body.push(form_action_set(
            &view.locale,
            &record.record,
            Some(id),
            update_action(view, &record.record),
        ));
    }
    Some(adaptive_card(
        view,
        "manager.record.detail",
        body,
        Vec::new(),
    ))
}

pub fn render_record_picker_card(view: &ManagerViewModel, record_name: &str) -> Option<Value> {
    let record = view
        .records
        .iter()
        .find(|record| record.record == record_name)?;
    Some(adaptive_card(
        view,
        "manager.record.picker",
        vec![
            text_block(
                &format!(
                    "{} {}",
                    localized_static(&view.locale, "Select"),
                    record.label
                ),
                "Large",
                true,
            ),
            text_block(
                localized_static(
                    &view.locale,
                    "Search and dropdown choices will appear here when records are available.",
                ),
                "Default",
                false,
            ),
            json!({
                "type": "Input.Text",
                "id": "query",
                "label": localized_static(&view.locale, "Search"),
                "placeholder": format!("{} {}", localized_static(&view.locale, "Search"), record.plural_label)
            }),
        ],
        vec![open_action(
            localized_static(&view.locale, "Dashboard"),
            "dashboard",
        )],
    ))
}

pub fn render_relationship_summary_card(view: &ManagerViewModel) -> Value {
    let mut body = vec![text_block(
        localized_static(&view.locale, "Relationships"),
        "large",
        true,
    )];
    if view.relationships.is_empty() {
        body.push(text_block(
            localized_static(&view.locale, "No relationship metadata is declared."),
            "default",
            false,
        ));
    } else {
        for relationship in &view.relationships {
            body.push(text_block(
                &format!(
                    "{}: {} -> {}",
                    relationship.label, relationship.from_record, relationship.to_record
                ),
                "default",
                false,
            ));
        }
    }
    adaptive_card(
        view,
        "manager.relationships",
        body,
        vec![open_action(
            localized_static(&view.locale, "Graph"),
            "graph.json",
        )],
    )
}

fn adaptive_card(
    view: &ManagerViewModel,
    kind: &str,
    body: Vec<Value>,
    actions: Vec<Value>,
) -> Value {
    json!({
        "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
        "type": "AdaptiveCard",
        "version": "1.5",
        "lang": view.locale,
        "metadata": {
            "schema": "greentic.sorx.manager-card.v1",
            "kind": kind,
            "locale": view.locale
        },
        "body": body,
        "actions": actions
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormScope {
    Create,
    RecordEditable,
    RecordReadOnly,
}

fn render_field_for_scope(
    field: &ManagerFieldView,
    record: &ManagerRecordView,
    scope: FormScope,
) -> Option<Value> {
    let uuid_identifier_hidden =
        matches!(scope, FormScope::RecordEditable | FormScope::RecordReadOnly)
            && is_uuid_detail_field(field)
            && !record
                .create_field_names
                .iter()
                .any(|candidate| candidate == &field.name);
    if scope == FormScope::Create
        && !record.create_field_names.is_empty()
        && !record
            .create_field_names
            .iter()
            .any(|candidate| candidate == &field.name)
    {
        return None;
    }
    if field.redacted {
        return Some(text_block(
            &format!("{}: redacted", field.label),
            "Default",
            false,
        ));
    }
    if field.generated && field.value.is_none() || uuid_identifier_hidden {
        return None;
    }
    if scope == FormScope::RecordReadOnly
        || field.read_only
        || (scope == FormScope::RecordEditable && is_uuid_detail_field(field))
    {
        Some(read_only_field_block(field))
    } else {
        Some(input_for_field(field))
    }
}

fn form_body(title: &str, record: &ManagerRecordView, scope: FormScope) -> Vec<Value> {
    let mut body = vec![text_block(title, "Large", true)];

    // Sort by display_order when at least one field carries it
    let mut ordered: Vec<&ManagerFieldView> = record.fields.iter().collect();
    if ordered.iter().any(|f| f.display_order.is_some()) {
        ordered.sort_by(|a, b| {
            a.display_order
                .unwrap_or(u32::MAX)
                .cmp(&b.display_order.unwrap_or(u32::MAX))
                .then_with(|| a.name.cmp(&b.name))
        });
    }

    // Check if any visible field has grouping
    let has_groups = ordered
        .iter()
        .any(|f| f.display_group.is_some() && !f.hidden);

    if has_groups {
        // Collect ungrouped items first, then groups in first-appearance order
        let mut ungrouped: Vec<Value> = Vec::new();
        let mut group_order: Vec<String> = Vec::new();
        let mut groups: std::collections::HashMap<String, Vec<Value>> =
            std::collections::HashMap::new();

        for field in ordered.iter().copied() {
            if field.hidden {
                continue;
            }
            let item = render_field_for_scope(field, record, scope);
            let Some(item) = item else { continue };
            if let Some(ref group_name) = field.display_group {
                if !group_order.contains(group_name) {
                    group_order.push(group_name.clone());
                }
                groups.entry(group_name.clone()).or_default().push(item);
            } else {
                ungrouped.push(item);
            }
        }

        body.extend(ungrouped);
        for group_name in group_order {
            let items = groups.remove(&group_name).unwrap_or_default();
            body.push(json!({
                "type": "Container",
                "items": std::iter::once(text_block(&group_name, "Default", true))
                    .chain(items)
                    .collect::<Vec<_>>()
            }));
        }
    } else {
        for field in ordered.iter().copied() {
            if field.hidden {
                continue;
            }
            if let Some(item) = render_field_for_scope(field, record, scope) {
                body.push(item);
            }
        }
    }

    body
}

fn read_only_field_block(field: &ManagerFieldView) -> Value {
    text_block(
        &format!(
            "{}: {}",
            field.label,
            field
                .value
                .as_ref()
                .map(input_value_string)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "-".to_string())
        ),
        "Default",
        false,
    )
}

fn is_uuid_detail_field(field: &ManagerFieldView) -> bool {
    let name = field.name.to_ascii_lowercase();
    name == "id"
        || name.ends_with("_id")
        || name.ends_with("_uuid")
        || canonical_scalar_type(field.json_type.as_deref()) == "uuid"
}

fn input_for_field(field: &ManagerFieldView) -> Value {
    if field.relationship.is_some() && canonical_scalar_type(field.json_type.as_deref()) == "uuid" {
        if let Some(choices) = choice_values(field) {
            return choice_input(field, choices);
        }
        return relationship_picker_input(field);
    }
    if let Some(choices) = choice_values(field) {
        return choice_input(field, choices);
    }
    let scalar_type = canonical_scalar_type(field.json_type.as_deref());
    let mut input = match scalar_type {
        "boolean" => json!({
            "type": "Input.Toggle",
            "id": field.name,
            "title": field.label,
            "label": field.label,
            "valueOn": "true",
            "valueOff": "false",
        }),
        "date" => json!({
            "type": "Input.Date",
            "id": field.name,
            "label": field.label,
        }),
        "time" => json!({
            "type": "Input.Time",
            "id": field.name,
            "label": field.label,
        }),
        "datetime" => datetime_input_container(field),
        "decimal" | "integer" => json!({
            "type": "Input.Number",
            "id": field.name,
            "label": field.label,
            "placeholder": field.label,
        }),
        _ => json!({
            "type": "Input.Text",
            "id": field.name,
            "label": field.label,
            "placeholder": field.label,
        }),
    };

    input["isRequired"] = Value::Bool(field.required);
    if field.required {
        input["errorMessage"] = Value::String(format!("{} is required.", field.label));
    }
    if scalar_type != "datetime" {
        apply_field_rules(&mut input, field);
        if let Some(value) = field.value.as_ref() {
            input["value"] = Value::String(input_value_string(value));
        }
    }
    input
}

fn choice_input(field: &ManagerFieldView, choices: Vec<Value>) -> Value {
    let mut input = json!({
        "type": "Input.ChoiceSet",
        "id": field.name,
        "label": field.label,
        "style": "compact",
        "isMultiSelect": false,
        "choices": choices,
        "isRequired": field.required,
        "metadata": {
            "schema": "greentic.sorx.manager-input.v1",
            "scalar_type": canonical_scalar_type(field.json_type.as_deref()),
            "rules": field.rules.clone()
        }
    });
    if field.required {
        input["errorMessage"] = Value::String(format!("{} is required.", field.label));
    }
    if let Some(value) = field.value.as_ref() {
        input["value"] = Value::String(input_value_string(value));
    }
    input
}

fn input_value_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null => String::new(),
        value => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn choice_values(field: &ManagerFieldView) -> Option<Vec<Value>> {
    let rules = field.rules.as_ref().and_then(Value::as_object)?;
    let values = rules
        .get("enum")
        .or_else(|| rules.get("enum_values"))
        .or_else(|| rules.get("choices"))
        .or_else(|| rules.get("allowed_values"))?
        .as_array()?;
    let choices = values
        .iter()
        .filter_map(|value| {
            if let Some(raw) = value.as_str() {
                return Some(json!({
                    "title": humanize_choice(raw),
                    "value": raw
                }));
            }
            let object = value.as_object()?;
            let raw = object
                .get("value")
                .or_else(|| object.get("id"))
                .and_then(Value::as_str)?;
            let title = object
                .get("title")
                .or_else(|| object.get("label"))
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .unwrap_or_else(|| humanize_choice(raw));
            Some(json!({
                "title": title,
                "value": raw
            }))
        })
        .collect::<Vec<_>>();
    (!choices.is_empty()).then_some(choices)
}

fn humanize_choice(value: &str) -> String {
    value
        .split(['_', '-', ' '])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn relationship_picker_input(field: &ManagerFieldView) -> Value {
    let relationship = field
        .relationship
        .as_ref()
        .expect("relationship is present");
    let picker_card = route_card_id(&format!("pickers/{}", relationship.to_record));
    let mut input = json!({
        "type": "Input.Text",
        "id": field.name,
        "label": field.label,
        "placeholder": format!("Select {}", relationship.label),
        "isRequired": field.required
    });
    if field.required {
        input["errorMessage"] = Value::String(format!("{} is required.", field.label));
    }
    json!({
        "type": "ColumnSet",
        "metadata": {
            "schema": "greentic.sorx.manager-input.v1",
            "scalar_type": "uuid",
            "relationship": relationship,
            "picker": {
                "record": relationship.to_record,
                "card_id": picker_card
            }
        },
        "columns": [
            {
                "type": "Column",
                "width": "stretch",
                "items": [input]
            },
            {
                "type": "Column",
                "width": "auto",
                "verticalContentAlignment": "bottom",
                "items": [{
                    "type": "ActionSet",
                    "actions": [{
                        "type": "Action.Submit",
                        "title": format!("⌕ Search {}", relationship.label),
                        "associatedInputs": "none",
                        "data": {
                            "manager_target": format!("pickers/{}", relationship.to_record),
                            "routeToCardId": picker_card,
                            "cardId": picker_card,
                            "step": "open",
                            "relationship_id": relationship.relationship_id,
                            "field": field.name,
                            "record": relationship.to_record
                        }
                    }]
                }]
            }
        ]
    })
}

fn datetime_input_container(field: &ManagerFieldView) -> Value {
    let date_id = datetime_part_id(&field.name, "date");
    let time_id = datetime_part_id(&field.name, "time");
    let mut date_input = json!({
        "type": "Input.Date",
        "id": date_id,
        "label": format!("{} date", field.label),
        "metadata": {
            "schema": "greentic.sorx.manager-input-part.v1",
            "target": field.name,
            "part": "date",
            "scalar_type": "datetime"
        }
    });
    let mut time_input = json!({
        "type": "Input.Time",
        "id": time_id,
        "label": format!("{} time", field.label),
        "metadata": {
            "schema": "greentic.sorx.manager-input-part.v1",
            "target": field.name,
            "part": "time",
            "scalar_type": "datetime"
        }
    });
    if field.required {
        date_input["isRequired"] = Value::Bool(true);
        date_input["errorMessage"] = Value::String(format!("{} date is required.", field.label));
        time_input["isRequired"] = Value::Bool(true);
        time_input["errorMessage"] = Value::String(format!("{} time is required.", field.label));
    }
    if let Some(rules) = field.rules.as_ref().and_then(Value::as_object) {
        copy_date_rule(&mut date_input, rules, "after", "min");
        copy_date_rule(&mut date_input, rules, "before", "max");
    }

    json!({
        "type": "Container",
        "metadata": {
            "schema": "greentic.sorx.manager-input.v1",
            "scalar_type": "datetime",
            "target": field.name,
            "combine": "date_time_iso8601_utc",
            "rules": field.rules.clone()
        },
        "items": [
            text_block(&field.label, "Default", false),
            date_input,
            time_input
        ]
    })
}

pub fn datetime_part_id(field_name: &str, part: &str) -> String {
    format!("{field_name}__sorx_{part}")
}

fn apply_field_rules(input: &mut Value, field: &ManagerFieldView) {
    let scalar_type = canonical_scalar_type(field.json_type.as_deref());
    let rules = field.rules.as_ref().and_then(Value::as_object);
    if let Some(rules) = rules {
        match scalar_type {
            "decimal" | "integer" => {
                copy_rule(input, rules, "min", "min");
                copy_rule(input, rules, "max", "max");
            }
            "date" | "time" => {
                copy_rule(input, rules, "after", "min");
                copy_rule(input, rules, "before", "max");
            }
            "datetime" => {
                input["placeholder"] = Value::String("YYYY-MM-DDTHH:MM:SSZ".to_string());
            }
            _ => {}
        }
    }

    if input.get("type").and_then(Value::as_str) == Some("Input.Text") {
        if scalar_type == "uuid" {
            input["placeholder"] =
                Value::String("xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx".to_string());
            input["regex"] = Value::String(
                "^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$"
                    .to_string(),
            );
            input["errorMessage"] = Value::String(format!("{} must be a UUID.", field.label));
        }
        if let Some(rules) = rules {
            copy_rule(input, rules, "max_length", "maxLength");
            copy_rule(input, rules, "pattern", "regex");
        }
    }

    input["metadata"] = json!({
        "schema": "greentic.sorx.manager-input.v1",
        "scalar_type": scalar_type,
        "rules": field.rules.clone()
    });
}

fn copy_rule(
    input: &mut Value,
    rules: &serde_json::Map<String, Value>,
    source: &str,
    target: &str,
) {
    if let Some(value) = rules.get(source) {
        input[target] = value.clone();
    }
}

fn copy_date_rule(
    input: &mut Value,
    rules: &serde_json::Map<String, Value>,
    source: &str,
    target: &str,
) {
    if let Some(Value::String(value)) = rules.get(source) {
        input[target] = Value::String(
            value
                .split_once('T')
                .map(|(date, _)| date)
                .unwrap_or(value)
                .to_string(),
        );
    } else {
        copy_rule(input, rules, source, target);
    }
}

fn canonical_scalar_type(value: Option<&str>) -> &'static str {
    match value.unwrap_or("string").to_ascii_lowercase().as_str() {
        "bool" | "boolean" => "boolean",
        "int" | "integer" | "u32" => "integer",
        "decimal" | "number" | "float" | "double" => "decimal",
        "uuid" => "uuid",
        "email" => "email",
        "url" => "url",
        "date" => "date",
        "time" => "time",
        "datetime" | "timestamp" => "datetime",
        _ => "string",
    }
}

fn submit_actions(
    record: &str,
    id: Option<&str>,
    action: Option<&ManagerActionView>,
    title: &str,
) -> Vec<Value> {
    let mut data = json!({
        "record": record
    });
    if let Some(id) = id {
        data["id"] = Value::String(id.to_string());
    }
    if let Some(action) = action {
        data["endpoint_id"] = Value::String(action.endpoint_id.clone());
        data["operation_id"] = Value::String(action.operation_id.clone());
        data["action"] = Value::String("manager_submit".to_string());
        data["sorx_action_style"] = Value::String("positive".to_string());
    }
    vec![json!({
        "type": "Action.Submit",
        "title": title,
        "style": "positive",
        "data": data
    })]
}

fn form_action_set(
    locale: &str,
    record: &str,
    id: Option<&str>,
    action: Option<&ManagerActionView>,
) -> Value {
    let title = if id.is_some() {
        localized_static(locale, "Save")
    } else {
        localized_static(locale, "Submit")
    };
    let mut actions = submit_actions(record, id, action, title);
    let target = format!("records/{record}");
    actions.push(json!({
        "type": "Action.Submit",
        "title": localized_static(locale, "Cancel"),
        "style": "default",
        "associatedInputs": "none",
        "data": {
            "manager_target": target,
            "routeToCardId": route_card_id(&target),
            "cardId": route_card_id(&target),
            "step": "open",
            "sorx_action_style": "secondary"
        }
    }));
    json!({
        "type": "ActionSet",
        "spacing": "medium",
        "actions": actions
    })
}

fn localized_static<'a>(locale: &str, text: &'a str) -> &'a str {
    let language = locale
        .split(['-', '_'])
        .next()
        .unwrap_or(locale)
        .to_ascii_lowercase();
    if language != "es" {
        return text;
    }
    match text {
        "Create" => "Crear",
        "Cancel" => "Cancelar",
        "Dashboard" => "Panel",
        "Submit" => "Enviar",
        "Save" => "Guardar",
        "Search" => "Buscar",
        "Select" => "Seleccionar",
        "Relationships" => "Relaciones",
        "Graph" => "Grafo",
        "No relationship metadata is declared." => "No se han declarado metadatos de relaciones.",
        "Search and dropdown choices will appear here when records are available." => {
            "La busqueda y las opciones desplegables apareceran aqui cuando haya registros disponibles."
        }
        _ => text,
    }
}

fn text_block(text: &str, size: &str, weight: bool) -> Value {
    json!({
        "type": "TextBlock",
        "text": text,
        "wrap": true,
        "size": size,
        "weight": if weight { "Bolder" } else { "Default" }
    })
}

fn open_action(title: &str, target: &str) -> Value {
    let route_to_card_id = route_card_id(target);
    json!({
        "type": "Action.Submit",
        "title": title,
        "associatedInputs": "none",
        "data": {
            "manager_target": target,
            "routeToCardId": route_to_card_id,
            "cardId": route_to_card_id,
            "step": "open"
        }
    })
}

fn create_action<'a>(view: &'a ManagerViewModel, record: &str) -> Option<&'a ManagerActionView> {
    view.actions.iter().find(|action| {
        action.record.as_deref() == Some(record) && action_matches_operation(action, "create")
    })
}

fn update_action<'a>(view: &'a ManagerViewModel, record: &str) -> Option<&'a ManagerActionView> {
    view.actions.iter().find(|action| {
        matches!(action.record.as_deref(), Some(action_record) if action_record == record || action_record == "Record")
            && action_matches_operation(action, "update")
    })
}

fn action_matches_operation(action: &ManagerActionView, operation: &str) -> bool {
    let marker = format!(".{operation}");
    let underscore_marker = format!("_{operation}");
    let dash_marker = format!("-{operation}");
    let slash_marker = format!("/{operation}");
    action.label_key.ends_with(&format!("{marker}.label"))
        || [&action.endpoint_id, &action.operation_id]
            .into_iter()
            .any(|value| {
                value.contains(&marker)
                    || value.contains(&underscore_marker)
                    || value.contains(&dash_marker)
                    || value.contains(&slash_marker)
                    || value.starts_with(&format!("{operation}_"))
                    || value.starts_with(&format!("{operation}-"))
                    || value.ends_with(operation)
            })
}

fn route_card_id(target: &str) -> String {
    if target == "dashboard" {
        return "sorx_dashboard".to_string();
    }
    target
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::manager::{
        ManagerFieldRelationshipView, ManagerFieldView, ManagerNavItem, ManagerPolicyDecision,
        ManagerRecordView, ManagerViewModel,
    };

    #[test]
    fn hidden_field_is_excluded_from_create_edit_and_detail_cards() {
        let mut v = view();
        v.records[0].fields = vec![
            ManagerFieldView {
                name: "id".to_string(),
                label_key: "field.record_alpha.id.label".to_string(),
                label: "Id".to_string(),
                json_type: Some("string".to_string()),
                rules: None,
                generated: false,
                relationship: None,
                required: true,
                read_only: false,
                redacted: false,
                value: None,
                hidden: false,
                display_order: None,
                display_group: None,
                policy: ManagerPolicyDecision::allow(),
            },
            ManagerFieldView {
                name: "secret".to_string(),
                label_key: "field.record_alpha.secret.label".to_string(),
                label: "Secret".to_string(),
                json_type: Some("string".to_string()),
                rules: None,
                generated: false,
                relationship: None,
                required: false,
                read_only: false,
                redacted: false,
                value: None,
                hidden: true,
                display_order: None,
                display_group: None,
                policy: ManagerPolicyDecision::allow(),
            },
        ];
        let create = render_record_create_card(&v, "RecordAlpha").unwrap();
        let body = create["body"].as_array().unwrap();
        assert!(
            body.iter().any(|item| item["id"] == "id"),
            "visible field should appear"
        );
        assert!(
            !body.iter().any(|item| item["id"] == "secret"),
            "hidden field must not appear in create"
        );

        let detail = render_record_detail_card(&v, "RecordAlpha", "x").unwrap();
        let body = detail["body"].as_array().unwrap();
        assert!(
            !body.iter().any(|item| item["id"] == "secret"),
            "hidden field must not appear in detail"
        );
        assert!(
            !body.iter().any(|item| item["text"]
                .as_str()
                .map(|t| t.contains("Secret"))
                .unwrap_or(false)),
            "hidden field label must not appear in detail"
        );
    }

    #[test]
    fn fields_render_in_display_order() {
        let mut v = view();
        v.records[0].fields = vec![
            ManagerFieldView {
                name: "zzz".to_string(),
                label_key: "field.record_alpha.zzz.label".to_string(),
                label: "Zzz".to_string(),
                json_type: Some("string".to_string()),
                rules: None,
                generated: false,
                relationship: None,
                required: false,
                read_only: false,
                redacted: false,
                value: None,
                hidden: false,
                display_order: Some(2),
                display_group: None,
                policy: ManagerPolicyDecision::allow(),
            },
            ManagerFieldView {
                name: "aaa".to_string(),
                label_key: "field.record_alpha.aaa.label".to_string(),
                label: "Aaa".to_string(),
                json_type: Some("string".to_string()),
                rules: None,
                generated: false,
                relationship: None,
                required: false,
                read_only: false,
                redacted: false,
                value: None,
                hidden: false,
                display_order: Some(1),
                display_group: None,
                policy: ManagerPolicyDecision::allow(),
            },
        ];
        let card = render_record_create_card(&v, "RecordAlpha").unwrap();
        let body = card["body"].as_array().unwrap();
        let inputs: Vec<_> = body
            .iter()
            .filter(|item| item["type"] == "Input.Text")
            .collect();
        assert_eq!(inputs.len(), 2);
        assert_eq!(
            inputs[0]["id"], "aaa",
            "aaa (order=1) should come before zzz (order=2)"
        );
        assert_eq!(inputs[1]["id"], "zzz");
    }

    #[test]
    fn display_label_overrides_label_in_card() {
        // display_label is fed via the label field in ManagerFieldView (simulating the decode path)
        let mut v = view();
        v.records[0].fields = vec![ManagerFieldView {
            name: "email".to_string(),
            label_key: "field.record_alpha.email.label".to_string(),
            label: "Your Email Address".to_string(), // simulates display_label winning
            json_type: Some("string".to_string()),
            rules: None,
            generated: false,
            relationship: None,
            required: false,
            read_only: false,
            redacted: false,
            value: None,
            hidden: false,
            display_order: None,
            display_group: None,
            policy: ManagerPolicyDecision::allow(),
        }];
        let card = render_record_create_card(&v, "RecordAlpha").unwrap();
        let body = card["body"].as_array().unwrap();
        let input = body.iter().find(|item| item["id"] == "email").unwrap();
        assert_eq!(input["label"], "Your Email Address");
    }

    #[test]
    fn display_group_wraps_fields_in_titled_containers() {
        let mut v = view();
        v.records[0].fields = vec![
            ManagerFieldView {
                name: "first_name".to_string(),
                label_key: "field.record_alpha.first_name.label".to_string(),
                label: "First Name".to_string(),
                json_type: Some("string".to_string()),
                rules: None,
                generated: false,
                relationship: None,
                required: false,
                read_only: false,
                redacted: false,
                value: None,
                hidden: false,
                display_order: Some(1),
                display_group: Some("Identity".to_string()),
                policy: ManagerPolicyDecision::allow(),
            },
            ManagerFieldView {
                name: "last_name".to_string(),
                label_key: "field.record_alpha.last_name.label".to_string(),
                label: "Last Name".to_string(),
                json_type: Some("string".to_string()),
                rules: None,
                generated: false,
                relationship: None,
                required: false,
                read_only: false,
                redacted: false,
                value: None,
                hidden: false,
                display_order: Some(2),
                display_group: Some("Identity".to_string()),
                policy: ManagerPolicyDecision::allow(),
            },
            ManagerFieldView {
                name: "notes".to_string(),
                label_key: "field.record_alpha.notes.label".to_string(),
                label: "Notes".to_string(),
                json_type: Some("string".to_string()),
                rules: None,
                generated: false,
                relationship: None,
                required: false,
                read_only: false,
                redacted: false,
                value: None,
                hidden: false,
                display_order: None,
                display_group: None,
                policy: ManagerPolicyDecision::allow(),
            },
        ];
        let card = render_record_create_card(&v, "RecordAlpha").unwrap();
        let body = card["body"].as_array().unwrap();
        // "notes" is ungrouped, should appear directly in body
        assert!(
            body.iter().any(|item| item["id"] == "notes"),
            "ungrouped field should appear in body directly"
        );
        // The "Identity" container should be present
        let container = body.iter().find(|item| {
            item["type"] == "Container"
                && item["items"].as_array().map_or(false, |items| {
                    items
                        .first()
                        .map_or(false, |first| first["text"] == "Identity")
                })
        });
        assert!(
            container.is_some(),
            "Identity group container should be present"
        );
        let container_items = container.unwrap()["items"].as_array().unwrap();
        // First item is the group title TextBlock
        assert_eq!(container_items[0]["text"], "Identity");
        assert_eq!(container_items[0]["weight"], "Bolder");
        // Then the two grouped fields
        assert!(
            container_items
                .iter()
                .any(|item| item["id"] == "first_name")
        );
        assert!(container_items.iter().any(|item| item["id"] == "last_name"));
    }

    #[test]
    fn no_hints_produces_unchanged_output() {
        // Fields with no hints (hidden=false, display_order=None, display_group=None)
        // should produce the same output as before.
        let v = view(); // view() returns a view with a single "id" field, no hints
        let card = render_record_create_card(&v, "RecordAlpha").unwrap();
        // The body should have: title TextBlock + Input.Text for "id" + ActionSet
        let body = card["body"].as_array().unwrap();
        assert_eq!(
            body[0]["type"], "TextBlock",
            "first body item should be title"
        );
        assert_eq!(
            body[1]["type"], "Input.Text",
            "second body item should be the id input"
        );
        assert_eq!(body[1]["id"], "id");
    }

    #[test]
    fn dashboard_card_is_adaptive_card_json() {
        let card = render_dashboard_card(&view());
        assert_eq!(card["type"], "AdaptiveCard");
        assert_eq!(card["lang"], "en");
        assert_eq!(card["metadata"]["schema"], "greentic.sorx.manager-card.v1");
        assert_eq!(card["metadata"]["locale"], "en");
        assert_eq!(card["body"][0]["text"], "Generic Sor");
        assert_eq!(card["body"][1]["text"], "Manage Record Alpha.");
        assert_eq!(
            card["actions"][0]["data"]["routeToCardId"],
            "records_RecordAlpha"
        );
    }

    #[test]
    fn create_card_contains_inputs_from_fields() {
        let card = render_record_create_card(&view(), "RecordAlpha").unwrap();
        assert_eq!(card["body"][1]["type"], "Input.Text");
        assert_eq!(card["body"][1]["id"], "id");
        assert_eq!(card["body"][1]["label"], "Id");
        assert_eq!(card["body"][1]["errorMessage"], "Id is required.");
        let actions = card["body"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "ActionSet")
            .unwrap();
        assert_eq!(
            actions["actions"][0]["data"]["endpoint_id"],
            "record_alpha.create"
        );
        assert_eq!(actions["actions"][0]["style"], "positive");
        assert_eq!(
            actions["actions"][0]["data"]["sorx_action_style"],
            "positive"
        );
        assert_eq!(actions["actions"][1]["title"], "Cancel");
    }

    #[test]
    fn create_card_uses_scalar_specific_inputs_and_rules() {
        let card = render_record_create_card(&typed_view(), "RecordAlpha").unwrap();
        assert_eq!(card["body"][1]["type"], "Input.Date");
        assert_eq!(card["body"][1]["min"], "2026-01-01");
        assert_eq!(card["body"][1]["max"], "2026-12-31");
        assert_eq!(card["body"][1]["metadata"]["scalar_type"], "date");

        assert_eq!(card["body"][2]["type"], "Input.Number");
        assert_eq!(card["body"][2]["min"], 0);
        assert_eq!(card["body"][2]["max"], 10000);

        assert_eq!(card["body"][3]["type"], "Input.Toggle");
        assert_eq!(card["body"][3]["valueOn"], "true");

        assert_eq!(card["body"][4]["type"], "Input.Text");
        assert_eq!(card["body"][4]["maxLength"], 120);
        assert_eq!(card["body"][4]["regex"], "^[A-Z].*");

        assert_eq!(card["body"][5]["type"], "Container");
        assert_eq!(card["body"][5]["metadata"]["scalar_type"], "datetime");
        assert_eq!(card["body"][5]["metadata"]["target"], "scheduled_at");
        assert_eq!(card["body"][5]["items"][1]["type"], "Input.Date");
        assert_eq!(card["body"][5]["items"][1]["id"], "scheduled_at__sorx_date");
        assert_eq!(card["body"][5]["items"][2]["type"], "Input.Time");
        assert_eq!(card["body"][5]["items"][2]["id"], "scheduled_at__sorx_time");

        assert_eq!(card["body"][6]["type"], "Input.ChoiceSet");
        assert_eq!(card["body"][6]["id"], "status");
        assert_eq!(card["body"][6]["choices"][0]["title"], "Pending");
        assert_eq!(card["body"][6]["choices"][0]["value"], "pending");
    }

    #[test]
    fn create_card_skips_generated_uuid_and_uses_picker_for_relationship_uuid() {
        let card = render_record_create_card(&relationship_view(), "RecordAlpha").unwrap();
        let body = card["body"].as_array().unwrap();
        assert!(!body.iter().any(|item| item["id"] == "id"));
        let tenant = body
            .iter()
            .find(|item| item["metadata"]["relationship"]["to_record"] == "Tenant")
            .unwrap();
        assert_eq!(tenant["columns"][0]["items"][0]["id"], "tenant_id");
        assert_eq!(
            tenant["columns"][1]["items"][0]["actions"][0]["data"]["routeToCardId"],
            "pickers_Tenant"
        );
    }

    #[test]
    fn create_card_skips_record_scoped_generated_uuid() {
        let mut view = view();
        view.records[0].record = "Landlord".to_string();
        view.records[0].label = "Landlord".to_string();
        view.records[0].fields = vec![
            ManagerFieldView {
                name: "landlord_id".to_string(),
                label_key: "field.landlord.landlord_id.label".to_string(),
                label: "Landlord Id".to_string(),
                json_type: Some("uuid".to_string()),
                rules: None,
                generated: true,
                relationship: None,
                required: true,
                read_only: false,
                redacted: false,
                value: None,
                hidden: false,
                display_order: None,
                display_group: None,
                policy: ManagerPolicyDecision::allow(),
            },
            ManagerFieldView {
                name: "name".to_string(),
                label_key: "field.landlord.name.label".to_string(),
                label: "Name".to_string(),
                json_type: Some("string".to_string()),
                rules: None,
                generated: false,
                relationship: None,
                required: true,
                read_only: false,
                redacted: false,
                value: None,
                hidden: false,
                display_order: None,
                display_group: None,
                policy: ManagerPolicyDecision::allow(),
            },
        ];

        let card = render_record_create_card(&view, "Landlord").unwrap();
        let body = card["body"].as_array().unwrap();
        assert!(!body.iter().any(|item| item["id"] == "landlord_id"));
        assert!(body.iter().any(|item| item["id"] == "name"));
    }

    #[test]
    fn uuid_create_fields_are_validated_and_visible_on_detail() {
        let lab_id = "11111111-1111-4111-8111-111111111111";
        let mut view = view();
        view.records[0].create_field_names = vec!["lab_id".to_string()];
        view.records[0].fields = vec![ManagerFieldView {
            name: "lab_id".to_string(),
            label_key: "field.record_alpha.lab_id.label".to_string(),
            label: "Lab Id".to_string(),
            json_type: Some("uuid".to_string()),
            rules: None,
            generated: false,
            relationship: None,
            required: true,
            read_only: false,
            redacted: false,
            value: Some(json!(lab_id)),
            hidden: false,
            display_order: None,
            display_group: None,
            policy: ManagerPolicyDecision::allow(),
        }];

        let create = render_record_create_card(&view, "RecordAlpha").unwrap();
        let input = create["body"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "lab_id")
            .unwrap();
        assert_eq!(input["type"], "Input.Text");
        assert_eq!(input["metadata"]["scalar_type"], "uuid");
        assert_eq!(
            input["regex"],
            "^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$"
        );
        assert_eq!(input["errorMessage"], "Lab Id must be a UUID.");

        let detail = render_record_detail_card(&view, "RecordAlpha", lab_id).unwrap();
        let body = detail["body"].as_array().unwrap();
        assert!(
            body.iter()
                .any(|item| item["text"] == format!("Lab Id: {lab_id}"))
        );
    }

    #[test]
    fn create_card_uses_create_endpoint_fields_not_record_wide_admin_fields() {
        let mut view = view();
        view.records[0].record = "Landlord".to_string();
        view.records[0].label = "Landlord".to_string();
        view.records[0].create_field_names = vec!["email".to_string(), "full_name".to_string()];
        view.records[0].fields = ["email", "full_name", "patch_json", "reason", "record_id"]
            .into_iter()
            .map(|name| ManagerFieldView {
                name: name.to_string(),
                label_key: format!("field.landlord.{name}.label"),
                label: name.to_string(),
                json_type: Some("string".to_string()),
                rules: None,
                generated: false,
                relationship: None,
                required: true,
                read_only: false,
                redacted: false,
                value: None,
                hidden: false,
                display_order: None,
                display_group: None,
                policy: ManagerPolicyDecision::allow(),
            })
            .collect();

        let card = render_record_create_card(&view, "Landlord").unwrap();
        let body = card["body"].as_array().unwrap();
        assert!(body.iter().any(|item| item["id"] == "email"));
        assert!(body.iter().any(|item| item["id"] == "full_name"));
        assert!(!body.iter().any(|item| item["id"] == "patch_json"));
        assert!(!body.iter().any(|item| item["id"] == "reason"));
        assert!(!body.iter().any(|item| item["id"] == "record_id"));
    }

    #[test]
    fn detail_card_is_read_only_without_update_action() {
        let mut view = view();
        view.records[0].fields[0].value = Some(json!("record-1"));

        let card = render_record_detail_card(&view, "RecordAlpha", "record-1").unwrap();
        let body = card["body"].as_array().unwrap();
        assert!(!body.iter().any(|item| item["text"] == "Id: record-1"));
        assert!(!body.iter().any(|item| item["type"] == "Input.Text"));
        assert!(card["actions"].as_array().unwrap().is_empty());
    }

    #[test]
    fn detail_card_allows_save_but_never_edits_uuid_fields() {
        let mut view = view();
        view.actions.push(crate::manager::ManagerActionView {
            action_id: "record_alpha.update".to_string(),
            endpoint_id: "record_alpha.update".to_string(),
            operation_id: "record_alpha.update".to_string(),
            record: Some("RecordAlpha".to_string()),
            label_key: "action.record_alpha.update.label".to_string(),
            label: "Update Record Alpha".to_string(),
            risk: "low".to_string(),
            approval_required: false,
            policy: ManagerPolicyDecision::allow(),
        });
        view.records[0].fields = vec![
            ManagerFieldView {
                name: "id".to_string(),
                label_key: "field.record_alpha.id.label".to_string(),
                label: "Id".to_string(),
                json_type: Some("uuid".to_string()),
                rules: None,
                generated: false,
                relationship: None,
                required: true,
                read_only: false,
                redacted: false,
                value: Some(json!("record-1")),
                hidden: false,
                display_order: None,
                display_group: None,
                policy: ManagerPolicyDecision::allow(),
            },
            ManagerFieldView {
                name: "name".to_string(),
                label_key: "field.record_alpha.name.label".to_string(),
                label: "Name".to_string(),
                json_type: Some("string".to_string()),
                rules: None,
                generated: false,
                relationship: None,
                required: false,
                read_only: false,
                redacted: false,
                value: Some(json!("Alpha")),
                hidden: false,
                display_order: None,
                display_group: None,
                policy: ManagerPolicyDecision::allow(),
            },
        ];

        let card = render_record_detail_card(&view, "RecordAlpha", "record-1").unwrap();
        let body = card["body"].as_array().unwrap();
        assert!(!body.iter().any(|item| item["text"] == "Id: record-1"));
        assert!(!body.iter().any(|item| item["id"] == "id"));
        assert!(body.iter().any(|item| item["id"] == "name"));
        assert!(body.iter().any(|item| {
            item["type"] == "ActionSet"
                && item["actions"]
                    .as_array()
                    .is_some_and(|actions| actions.iter().any(|action| action["title"] == "Save"))
        }));
        assert!(card["actions"].as_array().unwrap().is_empty());
    }

    fn view() -> ManagerViewModel {
        ManagerViewModel {
            schema: "greentic.sorx.manager-view.v1".to_string(),
            tenant_id: "tenant-a".to_string(),
            sor_id: "generic-sor".to_string(),
            title: "Generic Sor".to_string(),
            description: "Manage Record Alpha.".to_string(),
            locale: "en".to_string(),
            navigation: vec![ManagerNavItem {
                record: "RecordAlpha".to_string(),
                label_key: "record.record_alpha.plural".to_string(),
                label: "Record Alpha".to_string(),
                collection: "record_alpha".to_string(),
            }],
            records: vec![ManagerRecordView {
                record: "RecordAlpha".to_string(),
                collection: "record_alpha".to_string(),
                label_key: "record.record_alpha.label".to_string(),
                label: "Record Alpha".to_string(),
                plural_label_key: "record.record_alpha.plural".to_string(),
                plural_label: "Record Alpha".to_string(),
                create_field_names: Vec::new(),
                fields: vec![ManagerFieldView {
                    name: "id".to_string(),
                    label_key: "field.record_alpha.id.label".to_string(),
                    label: "Id".to_string(),
                    json_type: Some("string".to_string()),
                    rules: None,
                    generated: false,
                    relationship: None,
                    required: true,
                    read_only: false,
                    redacted: false,
                    value: None,
                    hidden: false,
                    display_order: None,
                    display_group: None,
                    policy: ManagerPolicyDecision::allow(),
                }],
                endpoint_ids: Vec::new(),
                policy: ManagerPolicyDecision::allow(),
            }],
            relationships: Vec::new(),
            actions: vec![crate::manager::ManagerActionView {
                action_id: "record_alpha.create".to_string(),
                endpoint_id: "record_alpha.create".to_string(),
                operation_id: "record_alpha.create".to_string(),
                record: Some("RecordAlpha".to_string()),
                label_key: "action.record_alpha.create.label".to_string(),
                label: "Create Record Alpha".to_string(),
                risk: "low".to_string(),
                approval_required: false,
                policy: ManagerPolicyDecision::allow(),
            }],
            policies: Vec::new(),
        }
    }

    fn typed_view() -> ManagerViewModel {
        let mut view = view();
        view.records[0].fields = vec![
            ManagerFieldView {
                name: "starts_on".to_string(),
                label_key: "field.record_alpha.starts_on.label".to_string(),
                label: "Starts On".to_string(),
                json_type: Some("date".to_string()),
                rules: Some(json!({"after": "2026-01-01", "before": "2026-12-31"})),
                generated: false,
                relationship: None,
                required: true,
                read_only: false,
                redacted: false,
                value: None,
                hidden: false,
                display_order: None,
                display_group: None,
                policy: ManagerPolicyDecision::allow(),
            },
            ManagerFieldView {
                name: "rent".to_string(),
                label_key: "field.record_alpha.rent.label".to_string(),
                label: "Rent".to_string(),
                json_type: Some("decimal".to_string()),
                rules: Some(json!({"min": 0, "max": 10000})),
                generated: false,
                relationship: None,
                required: false,
                read_only: false,
                redacted: false,
                value: None,
                hidden: false,
                display_order: None,
                display_group: None,
                policy: ManagerPolicyDecision::allow(),
            },
            ManagerFieldView {
                name: "active".to_string(),
                label_key: "field.record_alpha.active.label".to_string(),
                label: "Active".to_string(),
                json_type: Some("boolean".to_string()),
                rules: None,
                generated: false,
                relationship: None,
                required: false,
                read_only: false,
                redacted: false,
                value: None,
                hidden: false,
                display_order: None,
                display_group: None,
                policy: ManagerPolicyDecision::allow(),
            },
            ManagerFieldView {
                name: "summary".to_string(),
                label_key: "field.record_alpha.summary.label".to_string(),
                label: "Summary".to_string(),
                json_type: Some("string".to_string()),
                rules: Some(json!({"max_length": 120, "pattern": "^[A-Z].*"})),
                generated: false,
                relationship: None,
                required: false,
                read_only: false,
                redacted: false,
                value: None,
                hidden: false,
                display_order: None,
                display_group: None,
                policy: ManagerPolicyDecision::allow(),
            },
            ManagerFieldView {
                name: "scheduled_at".to_string(),
                label_key: "field.record_alpha.scheduled_at.label".to_string(),
                label: "Scheduled At".to_string(),
                json_type: Some("datetime".to_string()),
                rules: Some(json!({"after": "2026-01-01", "before": "2026-12-31"})),
                generated: false,
                relationship: None,
                required: true,
                read_only: false,
                redacted: false,
                value: None,
                hidden: false,
                display_order: None,
                display_group: None,
                policy: ManagerPolicyDecision::allow(),
            },
            ManagerFieldView {
                name: "status".to_string(),
                label_key: "field.record_alpha.status.label".to_string(),
                label: "Status".to_string(),
                json_type: Some("string".to_string()),
                rules: Some(json!({"enum": ["pending", "settled"]})),
                generated: false,
                relationship: None,
                required: true,
                read_only: false,
                redacted: false,
                value: None,
                hidden: false,
                display_order: None,
                display_group: None,
                policy: ManagerPolicyDecision::allow(),
            },
        ];
        view
    }

    fn relationship_view() -> ManagerViewModel {
        let mut view = view();
        view.records.push(ManagerRecordView {
            record: "Tenant".to_string(),
            collection: "tenants".to_string(),
            label_key: "record.tenant.label".to_string(),
            label: "Tenant".to_string(),
            plural_label_key: "record.tenant.plural".to_string(),
            plural_label: "Tenants".to_string(),
            create_field_names: Vec::new(),
            fields: Vec::new(),
            endpoint_ids: Vec::new(),
            policy: ManagerPolicyDecision::allow(),
        });
        view.records[0].fields = vec![
            ManagerFieldView {
                name: "id".to_string(),
                label_key: "field.record_alpha.id.label".to_string(),
                label: "Id".to_string(),
                json_type: Some("uuid".to_string()),
                rules: None,
                generated: true,
                relationship: None,
                required: true,
                read_only: false,
                redacted: false,
                value: None,
                hidden: false,
                display_order: None,
                display_group: None,
                policy: ManagerPolicyDecision::allow(),
            },
            ManagerFieldView {
                name: "tenant_id".to_string(),
                label_key: "field.record_alpha.tenant_id.label".to_string(),
                label: "Tenant".to_string(),
                json_type: Some("uuid".to_string()),
                rules: None,
                generated: false,
                relationship: Some(ManagerFieldRelationshipView {
                    relationship_id: "tenant_has_alpha".to_string(),
                    to_record: "Tenant".to_string(),
                    label: "Tenant".to_string(),
                }),
                required: true,
                read_only: false,
                redacted: false,
                value: None,
                hidden: false,
                display_order: None,
                display_group: None,
                policy: ManagerPolicyDecision::allow(),
            },
        ];
        view
    }
}
