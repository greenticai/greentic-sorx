use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ManagerPolicyDecision, ManagerPolicyEffect, ManagerPolicyHint, ManagerViewModel};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagerPolicySet {
    #[serde(default)]
    pub records: BTreeMap<String, ManagerPolicyDecision>,
    #[serde(default)]
    pub fields: BTreeMap<String, BTreeMap<String, ManagerPolicyDecision>>,
    #[serde(default)]
    pub actions: BTreeMap<String, ManagerPolicyDecision>,
    #[serde(default)]
    pub relationships: BTreeMap<String, ManagerPolicyDecision>,
}

pub fn filter_manager_view(
    mut view: ManagerViewModel,
    policies: &ManagerPolicySet,
) -> ManagerViewModel {
    view.records.retain_mut(|record| {
        let decision = policies
            .records
            .get(&record.record)
            .cloned()
            .unwrap_or_default();
        record.policy = decision.clone();
        if decision.is_hidden() {
            view.policies.push(ManagerPolicyHint {
                scope: "record".to_string(),
                target: record.record.clone(),
                decision,
            });
            return false;
        }

        let field_policies = policies.fields.get(&record.record);
        record.fields.retain_mut(|field| {
            let decision = field_policies
                .and_then(|fields| fields.get(&field.name))
                .cloned()
                .unwrap_or_default();
            field.policy = decision.clone();
            match decision.effect {
                ManagerPolicyEffect::Hide | ManagerPolicyEffect::Deny => {
                    view.policies.push(ManagerPolicyHint {
                        scope: "field".to_string(),
                        target: format!("{}.{}", record.record, field.name),
                        decision,
                    });
                    false
                }
                ManagerPolicyEffect::Redact => {
                    field.redacted = true;
                    field.value = Some(Value::String("redacted".to_string()));
                    true
                }
                ManagerPolicyEffect::ReadOnly => {
                    field.read_only = true;
                    true
                }
                ManagerPolicyEffect::Allow | ManagerPolicyEffect::RequiresApproval => true,
            }
        });
        true
    });

    let visible_records = view
        .records
        .iter()
        .map(|record| record.record.clone())
        .collect::<std::collections::BTreeSet<_>>();
    view.navigation
        .retain(|item| visible_records.contains(&item.record));

    view.actions.retain_mut(|action| {
        if action
            .record
            .as_ref()
            .is_some_and(|record| record != "Record" && !visible_records.contains(record))
        {
            return false;
        }
        let decision = policies
            .actions
            .get(&action.action_id)
            .or_else(|| policies.actions.get(&action.endpoint_id))
            .cloned()
            .unwrap_or_else(|| action.policy.clone());
        action.policy = decision.clone();
        match decision.effect {
            ManagerPolicyEffect::Hide | ManagerPolicyEffect::Deny => {
                view.policies.push(ManagerPolicyHint {
                    scope: "action".to_string(),
                    target: action.action_id.clone(),
                    decision,
                });
                false
            }
            ManagerPolicyEffect::RequiresApproval => {
                action.approval_required = true;
                true
            }
            ManagerPolicyEffect::ReadOnly
            | ManagerPolicyEffect::Redact
            | ManagerPolicyEffect::Allow => true,
        }
    });

    view.relationships.retain_mut(|relationship| {
        if !visible_records.contains(&relationship.from_record)
            || !visible_records.contains(&relationship.to_record)
        {
            return false;
        }
        let decision = policies
            .relationships
            .get(&relationship.id)
            .cloned()
            .unwrap_or_default();
        relationship.policy = decision.clone();
        if decision.is_hidden() {
            view.policies.push(ManagerPolicyHint {
                scope: "relationship".to_string(),
                target: relationship.id.clone(),
                decision,
            });
            return false;
        }
        if decision.effect == ManagerPolicyEffect::ReadOnly {
            relationship.limited_context = true;
        }
        true
    });

    view
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::{
        ManagerActionView, ManagerFieldView, ManagerNavItem, ManagerRecordView,
        ManagerRelationshipView, view_model::manager_key,
    };

    fn view() -> ManagerViewModel {
        ManagerViewModel {
            schema: "greentic.sorx.manager-view.v1".to_string(),
            tenant_id: "tenant-a".to_string(),
            sor_id: "generic-sor".to_string(),
            title: "Generic Sor".to_string(),
            description: "Manage Record Alpha and Record Beta.".to_string(),
            locale: "en".to_string(),
            navigation: vec![
                ManagerNavItem {
                    record: "RecordAlpha".to_string(),
                    label_key: "record.record_alpha.plural".to_string(),
                    label: "Record Alpha".to_string(),
                    collection: "record_alpha".to_string(),
                },
                ManagerNavItem {
                    record: "RecordBeta".to_string(),
                    label_key: "record.record_beta.plural".to_string(),
                    label: "Record Beta".to_string(),
                    collection: "record_beta".to_string(),
                },
            ],
            records: vec![
                record("RecordAlpha", ["id", "name", "sensitive_note"]),
                record("RecordBeta", ["id", "name"]),
            ],
            relationships: vec![ManagerRelationshipView {
                id: "alpha_to_beta".to_string(),
                from_record: "RecordAlpha".to_string(),
                to_record: "RecordBeta".to_string(),
                label_key: "relationship.alpha_to_beta.label".to_string(),
                label: "Alpha To Beta".to_string(),
                limited_context: false,
                policy: ManagerPolicyDecision::allow(),
            }],
            actions: vec![ManagerActionView {
                action_id: "record_alpha.create".to_string(),
                endpoint_id: "record_alpha.create".to_string(),
                operation_id: "record_alpha.create".to_string(),
                record: Some("RecordAlpha".to_string()),
                label_key: "action.record_alpha.create.label".to_string(),
                label: "Create Record Alpha".to_string(),
                risk: "high".to_string(),
                approval_required: false,
                policy: ManagerPolicyDecision::allow(),
            }],
            policies: Vec::new(),
        }
    }

    fn record<const N: usize>(name: &str, fields: [&str; N]) -> ManagerRecordView {
        ManagerRecordView {
            record: name.to_string(),
            collection: manager_key(name),
            label_key: format!("record.{}.label", manager_key(name)),
            label: name.to_string(),
            plural_label_key: format!("record.{}.plural", manager_key(name)),
            plural_label: name.to_string(),
            create_field_names: Vec::new(),
            fields: fields
                .into_iter()
                .map(|field| ManagerFieldView {
                    name: field.to_string(),
                    label_key: format!("field.{}.{}.label", manager_key(name), manager_key(field)),
                    label: field.to_string(),
                    json_type: Some("string".to_string()),
                    rules: None,
                    generated: false,
                    relationship: None,
                    required: false,
                    read_only: false,
                    redacted: false,
                    value: Some(Value::String(format!("{field}-value"))),
                    hidden: false,
                    display_order: None,
                    display_group: None,
                    policy: ManagerPolicyDecision::allow(),
                })
                .collect(),
            endpoint_ids: Vec::new(),
            policy: ManagerPolicyDecision::allow(),
        }
    }

    #[test]
    fn filters_record_hidden_entirely() {
        let policies = ManagerPolicySet {
            records: BTreeMap::from([(
                "RecordBeta".to_string(),
                ManagerPolicyDecision::with_effect(ManagerPolicyEffect::Deny),
            )]),
            ..Default::default()
        };
        let filtered = filter_manager_view(view(), &policies);

        assert_eq!(filtered.records.len(), 1);
        assert_eq!(filtered.navigation.len(), 1);
        assert!(filtered.relationships.is_empty());
    }

    #[test]
    fn filters_field_states() {
        let policies = ManagerPolicySet {
            fields: BTreeMap::from([(
                "RecordAlpha".to_string(),
                BTreeMap::from([
                    (
                        "name".to_string(),
                        ManagerPolicyDecision::with_effect(ManagerPolicyEffect::ReadOnly),
                    ),
                    (
                        "sensitive_note".to_string(),
                        ManagerPolicyDecision::with_effect(ManagerPolicyEffect::Redact),
                    ),
                ]),
            )]),
            ..Default::default()
        };
        let filtered = filter_manager_view(view(), &policies);
        let alpha = &filtered.records[0];

        assert!(
            alpha
                .fields
                .iter()
                .any(|field| field.name == "name" && field.read_only)
        );
        assert!(
            alpha
                .fields
                .iter()
                .any(|field| field.name == "sensitive_note" && field.redacted)
        );
    }

    #[test]
    fn filters_action_as_approval_required() {
        let policies = ManagerPolicySet {
            actions: BTreeMap::from([(
                "record_alpha.create".to_string(),
                ManagerPolicyDecision::with_effect(ManagerPolicyEffect::RequiresApproval),
            )]),
            ..Default::default()
        };
        let filtered = filter_manager_view(view(), &policies);

        assert!(filtered.actions[0].approval_required);
    }

    #[test]
    fn hides_denied_action() {
        let policies = ManagerPolicySet {
            actions: BTreeMap::from([(
                "record_alpha.create".to_string(),
                ManagerPolicyDecision::with_effect(ManagerPolicyEffect::Hide),
            )]),
            ..Default::default()
        };
        let filtered = filter_manager_view(view(), &policies);

        assert!(filtered.actions.is_empty());
    }
}
