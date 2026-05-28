use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{EndpointDefinition, EndpointRouter, OperationKind, PolicyAction, PolicyEngine};

use super::{ManagerPolicyDecision, ManagerPolicyEffect, SorxManagerContext};

pub const MANAGER_VIEW_SCHEMA: &str = "greentic.sorx.manager-view.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManagerViewModel {
    pub schema: String,
    pub tenant_id: String,
    pub sor_id: String,
    pub title: String,
    pub description: String,
    pub locale: String,
    pub navigation: Vec<ManagerNavItem>,
    pub records: Vec<ManagerRecordView>,
    pub relationships: Vec<ManagerRelationshipView>,
    pub actions: Vec<ManagerActionView>,
    pub policies: Vec<ManagerPolicyHint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagerNavItem {
    pub record: String,
    pub label_key: String,
    pub label: String,
    pub collection: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManagerRecordView {
    pub record: String,
    pub collection: String,
    pub label_key: String,
    pub label: String,
    pub plural_label_key: String,
    pub plural_label: String,
    pub fields: Vec<ManagerFieldView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub create_field_names: Vec<String>,
    pub endpoint_ids: Vec<String>,
    pub policy: ManagerPolicyDecision,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManagerFieldView {
    pub name: String,
    pub label_key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<Value>,
    #[serde(default)]
    pub generated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship: Option<ManagerFieldRelationshipView>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub redacted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    pub policy: ManagerPolicyDecision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagerFieldRelationshipView {
    pub relationship_id: String,
    pub to_record: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagerRelationshipView {
    pub id: String,
    pub from_record: String,
    pub to_record: String,
    pub label_key: String,
    pub label: String,
    #[serde(default)]
    pub limited_context: bool,
    pub policy: ManagerPolicyDecision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagerActionView {
    pub action_id: String,
    pub endpoint_id: String,
    pub operation_id: String,
    pub record: Option<String>,
    pub label_key: String,
    pub label: String,
    pub risk: String,
    #[serde(default)]
    pub approval_required: bool,
    pub policy: ManagerPolicyDecision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagerPolicyHint {
    pub scope: String,
    pub target: String,
    pub decision: ManagerPolicyDecision,
}

pub fn generate_manager_view(
    context: &SorxManagerContext,
    router: &EndpointRouter,
    policy: &PolicyEngine,
) -> ManagerViewModel {
    let mut records = BTreeMap::<String, RecordBuilder>::new();
    let mut actions = Vec::new();

    for endpoint in router.endpoints.values() {
        if endpoint.record_selector {
            continue;
        }
        let Some(record) = endpoint.entity.clone() else {
            continue;
        };
        let builder = records
            .entry(record.clone())
            .or_insert_with(|| RecordBuilder::new(&record, &endpoint.collection));
        builder.collection = endpoint.collection.clone();
        builder.endpoint_ids.insert(endpoint.endpoint_id.clone());
        if !endpoint.record_selector {
            builder.collect_schema_fields(
                endpoint.input_schema.as_ref(),
                matches!(endpoint.operation, OperationKind::Create),
            );
        }
        builder.collect_schema(endpoint.output_schema.as_ref());

        let decision = policy.decide(endpoint);
        actions.push(ManagerActionView {
            action_id: endpoint.endpoint_id.clone(),
            endpoint_id: endpoint.endpoint_id.clone(),
            operation_id: endpoint.operation_id.clone(),
            record: Some(record.clone()),
            label_key: action_label_key(endpoint),
            label: humanize_identifier(&endpoint.endpoint_id),
            risk: format!("{:?}", endpoint.risk).to_ascii_lowercase(),
            approval_required: matches!(decision.action, PolicyAction::RequireApproval)
                || endpoint
                    .approval
                    .as_ref()
                    .is_some_and(|approval| approval.required),
            policy: ManagerPolicyDecision {
                effect: match decision.action {
                    PolicyAction::Execute => ManagerPolicyEffect::Allow,
                    PolicyAction::RequireApproval => ManagerPolicyEffect::RequiresApproval,
                    PolicyAction::Deny => ManagerPolicyEffect::Deny,
                },
                reason_code: Some(decision.reason),
                message_key: None,
                audit_hint: None,
            },
        });
    }

    let records = records
        .into_values()
        .map(RecordBuilder::finish)
        .collect::<Vec<_>>();
    let navigation = records
        .iter()
        .map(|record| ManagerNavItem {
            record: record.record.clone(),
            label_key: record.plural_label_key.clone(),
            label: record.plural_label.clone(),
            collection: record.collection.clone(),
        })
        .collect();

    ManagerViewModel {
        schema: MANAGER_VIEW_SCHEMA.to_string(),
        tenant_id: context.tenant_id.clone(),
        sor_id: context.sor_id.clone(),
        title: humanize_identifier(&context.sor_id),
        description: manager_description(&records),
        locale: context.locale.clone(),
        navigation,
        records,
        relationships: Vec::new(),
        actions,
        policies: Vec::new(),
    }
}

#[derive(Debug, Clone)]
struct RecordBuilder {
    record: String,
    collection: String,
    fields: BTreeMap<String, ManagerFieldView>,
    create_field_names: BTreeSet<String>,
    endpoint_ids: BTreeSet<String>,
}

impl RecordBuilder {
    fn new(record: &str, collection: &str) -> Self {
        Self {
            record: record.to_string(),
            collection: collection.to_string(),
            fields: BTreeMap::new(),
            create_field_names: BTreeSet::new(),
            endpoint_ids: BTreeSet::new(),
        }
    }

    fn collect_schema(&mut self, schema: Option<&Value>) {
        self.collect_schema_fields(schema, false)
    }

    fn collect_schema_fields(&mut self, schema: Option<&Value>, create_input: bool) {
        let Some(schema) = schema.and_then(Value::as_object) else {
            return;
        };
        let required = schema
            .get("required")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
            return;
        };
        for (name, property) in properties {
            if create_input {
                self.create_field_names.insert(name.clone());
            }
            self.fields
                .entry(name.clone())
                .or_insert_with(|| ManagerFieldView {
                    name: name.clone(),
                    label_key: format!(
                        "field.{}.{}.label",
                        manager_key(&self.record),
                        manager_key(name)
                    ),
                    label: humanize_identifier(name),
                    json_type: field_type(property),
                    rules: collect_field_rules(property),
                    generated: generated_field(&self.record, name, property),
                    relationship: None,
                    required: required.contains(name.as_str()),
                    read_only: false,
                    redacted: false,
                    value: None,
                    policy: ManagerPolicyDecision::allow(),
                });
        }
    }

    fn finish(self) -> ManagerRecordView {
        ManagerRecordView {
            label_key: format!("record.{}.label", manager_key(&self.record)),
            label: humanize_identifier(&self.record),
            plural_label_key: format!("record.{}.plural", manager_key(&self.record)),
            plural_label: humanize_identifier(&self.collection),
            record: self.record,
            collection: self.collection,
            create_field_names: self.create_field_names.into_iter().collect(),
            fields: self.fields.into_values().collect(),
            endpoint_ids: self.endpoint_ids.into_iter().collect(),
            policy: ManagerPolicyDecision::allow(),
        }
    }
}

fn action_label_key(endpoint: &EndpointDefinition) -> String {
    match endpoint.operation {
        OperationKind::Create => format!(
            "action.{}.create.label",
            endpoint
                .entity
                .as_deref()
                .map(manager_key)
                .unwrap_or_else(|| manager_key(&endpoint.endpoint_id))
        ),
        OperationKind::Get => format!(
            "action.{}.view.label",
            endpoint
                .entity
                .as_deref()
                .map(manager_key)
                .unwrap_or_else(|| manager_key(&endpoint.endpoint_id))
        ),
        OperationKind::Update => format!(
            "action.{}.update.label",
            endpoint
                .entity
                .as_deref()
                .map(manager_key)
                .unwrap_or_else(|| manager_key(&endpoint.endpoint_id))
        ),
        OperationKind::Delete => format!(
            "action.{}.delete.label",
            endpoint
                .entity
                .as_deref()
                .map(manager_key)
                .unwrap_or_else(|| manager_key(&endpoint.endpoint_id))
        ),
        OperationKind::Query => format!(
            "action.{}.query.label",
            endpoint
                .entity
                .as_deref()
                .map(manager_key)
                .unwrap_or_else(|| manager_key(&endpoint.endpoint_id))
        ),
        OperationKind::Command(_) => {
            format!("action.{}.label", manager_key(&endpoint.endpoint_id))
        }
    }
}

pub(crate) fn manager_key(value: &str) -> String {
    let mut out = String::new();
    for (index, ch) in value.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

pub(crate) fn humanize_identifier(value: &str) -> String {
    super::locale::humanize_identifier(value)
}

fn manager_description(records: &[ManagerRecordView]) -> String {
    let labels = records
        .iter()
        .map(|record| record.plural_label.as_str())
        .collect::<Vec<_>>();
    match labels.as_slice() {
        [] => "Manage this system of record.".to_string(),
        [one] => format!("Manage {one}."),
        [first, second] => format!("Manage {first} and {second}."),
        _ => {
            let last = labels.last().copied().unwrap_or_default();
            let prefix = labels[..labels.len() - 1].join(", ");
            format!("Manage {prefix}, and {last}.")
        }
    }
}

fn generated_field(record: &str, name: &str, property: &Value) -> bool {
    if property
        .get("generated")
        .or_else(|| property.get("auto_generated"))
        .or_else(|| property.get("autoGenerate"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return true;
    }
    field_type(property).as_deref() == Some("uuid") && generated_uuid_field_name(record, name)
}

fn generated_uuid_field_name(record: &str, name: &str) -> bool {
    if name == "id" {
        return true;
    }
    let normalized_record = manager_key(record);
    ["_id", "_uuid", "_ref"]
        .iter()
        .any(|suffix| name == format!("{normalized_record}{suffix}"))
}

fn collect_field_rules(property: &Value) -> Option<Value> {
    let object = property.as_object()?;
    if let Some(rules) = object.get("rules").filter(|rules| rules.is_object()) {
        return Some(rules.clone());
    }

    let mut rules = serde_json::Map::new();
    for key in [
        "min",
        "max",
        "min_length",
        "max_length",
        "pattern",
        "precision",
        "scale",
        "before",
        "after",
        "unique",
        "enum",
        "enum_values",
        "choices",
        "allowed_values",
    ] {
        if let Some(value) = object.get(key) {
            rules.insert(key.to_string(), value.clone());
        }
    }
    if rules.is_empty() {
        None
    } else {
        Some(Value::Object(rules))
    }
}

fn field_type(property: &Value) -> Option<String> {
    let raw_type = property.get("type").and_then(Value::as_str)?;
    let format = property.get("format").and_then(Value::as_str);
    match (raw_type, format) {
        ("string", Some("date")) => Some("date".to_string()),
        ("string", Some("time")) => Some("time".to_string()),
        ("string", Some("date-time")) => Some("datetime".to_string()),
        ("string", Some("email")) => Some("email".to_string()),
        ("string", Some("uri" | "url")) => Some("url".to_string()),
        ("string", Some("uuid")) => Some("uuid".to_string()),
        _ => Some(raw_type.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{
        EndpointDefinition, EndpointMethod, OperationKind, PolicyConfig, QueryPlan, RiskLevel,
        ViewTransform,
    };

    use super::*;

    fn context() -> SorxManagerContext {
        SorxManagerContext {
            tenant_id: "tenant-a".to_string(),
            environment_id: Some("local".to_string()),
            sor_id: "generic-sor".to_string(),
            team_id: None,
            caller_id: "tester".to_string(),
            channel: Default::default(),
            locale: "en".to_string(),
            roles: vec!["local".to_string()],
            groups: Vec::new(),
            claims: Value::Object(Default::default()),
        }
    }

    #[test]
    fn manager_view_is_generated_from_endpoint_metadata() {
        let router = EndpointRouter::new([EndpointDefinition {
            endpoint_id: "record_alpha.create".to_string(),
            operation_id: "record_alpha.create".to_string(),
            operation: OperationKind::Create,
            method: EndpointMethod::Post,
            path: "/v1/agent/record-alpha/create".to_string(),
            entity: Some("RecordAlpha".to_string()),
            collection: "record_alpha".to_string(),
            provider_binding: "store".to_string(),
            risk: RiskLevel::Low,
            approval: None,
            authorization: None,
            input_schema: Some(json!({
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": {"type": "string"},
                    "name": {"type": "string"}
                }
            })),
            output_schema: None,
            view: ViewTransform::default(),
            query_plan: QueryPlan::default(),
            record_selector: false,
        }])
        .unwrap();

        let view = generate_manager_view(&context(), &router, &PolicyEngine::default());
        assert_eq!(view.schema, MANAGER_VIEW_SCHEMA);
        assert_eq!(view.title, "Generic Sor");
        assert_eq!(view.description, "Manage Record Alpha.");
        assert_eq!(view.navigation[0].record, "RecordAlpha");
        assert_eq!(view.records[0].fields[0].name, "id");
        assert_eq!(view.actions[0].endpoint_id, "record_alpha.create");
    }

    #[test]
    fn record_scoped_uuid_identifier_is_generated() {
        let router = EndpointRouter::new([EndpointDefinition {
            endpoint_id: "landlord.create".to_string(),
            operation_id: "landlord.create".to_string(),
            operation: OperationKind::Create,
            method: EndpointMethod::Post,
            path: "/v1/agent/landlords/create".to_string(),
            entity: Some("Landlord".to_string()),
            collection: "landlords".to_string(),
            provider_binding: "store".to_string(),
            risk: RiskLevel::Low,
            approval: None,
            authorization: None,
            input_schema: Some(json!({
                "type": "object",
                "required": ["landlord_id", "name"],
                "properties": {
                    "landlord_id": {"type": "string", "format": "uuid"},
                    "name": {"type": "string"}
                }
            })),
            output_schema: None,
            view: ViewTransform::default(),
            query_plan: QueryPlan::default(),
            record_selector: false,
        }])
        .unwrap();

        let view = generate_manager_view(&context(), &router, &PolicyEngine::default());
        let landlord_id = view.records[0]
            .fields
            .iter()
            .find(|field| field.name == "landlord_id")
            .unwrap();
        assert!(landlord_id.generated);
    }

    #[test]
    fn high_risk_action_uses_existing_policy_decision() {
        let router = EndpointRouter::new([EndpointDefinition {
            endpoint_id: "record_alpha.delete".to_string(),
            operation_id: "record_alpha.delete".to_string(),
            operation: OperationKind::Delete,
            method: EndpointMethod::Delete,
            path: "/v1/agent/record-alpha/{id}".to_string(),
            entity: Some("RecordAlpha".to_string()),
            collection: "record_alpha".to_string(),
            provider_binding: "store".to_string(),
            risk: RiskLevel::High,
            approval: None,
            authorization: None,
            input_schema: None,
            output_schema: None,
            view: ViewTransform::default(),
            query_plan: QueryPlan::default(),
            record_selector: false,
        }])
        .unwrap();
        let policy = PolicyEngine::new(PolicyConfig::default());

        let view = generate_manager_view(&context(), &router, &policy);
        assert!(view.actions[0].approval_required);
        assert_eq!(
            view.actions[0].policy.effect,
            ManagerPolicyEffect::RequiresApproval
        );
    }
}
