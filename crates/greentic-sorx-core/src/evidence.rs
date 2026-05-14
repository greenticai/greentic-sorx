use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{SorxResult, TypePath};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OntologyScope {
    pub root_entities: Vec<ScopedEntity>,
    pub concepts: Vec<String>,
    pub relationships: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopedEntity {
    pub entity_type: String,
    pub entity_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceQueryFilter {
    pub query: String,
    pub scope: OntologyScope,
    pub max_depth: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceItem {
    pub evidence_id: String,
    pub source_ref: String,
    pub snippet: String,
    pub score: f64,
    pub linked_entities: Vec<ScopedEntity>,
    pub provenance: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceQueryResult {
    pub schema: String,
    pub query: String,
    pub ontology_scope: OntologyScope,
    pub evidence: Vec<EvidenceItem>,
    pub explain: EvidenceExplain,
    pub audit_events: Vec<OntologyAuditEvent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceExplain {
    pub retrieval_binding: Option<String>,
    pub provider_id: String,
    pub graph_paths_considered: Vec<TypePath>,
    pub ontology_graph_hash: String,
    pub concepts_used: Vec<String>,
    pub relationships_used: Vec<String>,
    pub providers_used: Vec<String>,
    pub evidence_used: Vec<String>,
    pub policy_decisions: Vec<String>,
    pub redactions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OntologyAuditEvent {
    pub schema: String,
    pub event: String,
    pub subject: String,
    pub details: Value,
}

pub fn ontology_audit_event(
    event: impl Into<String>,
    subject: impl Into<String>,
    details: Value,
) -> OntologyAuditEvent {
    OntologyAuditEvent {
        schema: "greentic.sorx.ontology.audit.v1".to_string(),
        event: event.into(),
        subject: subject.into(),
        details: redact_audit_value(details),
    }
}

pub fn redact_audit_value(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(redact_audit_value).collect()),
        Value::Object(object) => Value::Object(redact_audit_object(object)),
        other => other,
    }
}

fn redact_audit_object(object: Map<String, Value>) -> Map<String, Value> {
    object
        .into_iter()
        .map(|(key, value)| {
            if is_secret_like_key(&key) {
                (key, Value::String("[REDACTED]".to_string()))
            } else {
                (key, redact_audit_value(value))
            }
        })
        .collect()
}

fn is_secret_like_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "secret",
        "token",
        "password",
        "credential",
        "api_key",
        "apikey",
        "private_key",
        "access_key",
        "config_ref",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

pub trait EvidenceProvider: Send + Sync {
    fn query(&self, filter: EvidenceQueryFilter) -> SorxResult<Vec<EvidenceItem>>;
}

#[derive(Debug, Clone)]
pub struct DeterministicEvidenceProvider {
    provider_id: String,
}

impl DeterministicEvidenceProvider {
    pub fn new(provider_id: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
        }
    }
}

impl EvidenceProvider for DeterministicEvidenceProvider {
    fn query(&self, filter: EvidenceQueryFilter) -> SorxResult<Vec<EvidenceItem>> {
        let root = filter.scope.root_entities.first().cloned();
        let linked_entities = root.clone().into_iter().collect::<Vec<_>>();
        let root_label = root
            .map(|entity| format!("{}:{}", entity.entity_type, entity.entity_id))
            .unwrap_or_else(|| "unscoped".to_string());
        Ok(vec![EvidenceItem {
            evidence_id: format!("evidence:{}:{root_label}", self.provider_id),
            source_ref: format!("provider://{}/deterministic", self.provider_id),
            snippet: format!("{} [{}]", filter.query, root_label),
            score: 1.0,
            linked_entities,
            provenance: "deterministic-memory-evidence-provider".to_string(),
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_provider_returns_deterministic_result_with_provenance() {
        let provider = DeterministicEvidenceProvider::new("rag");
        let result = provider
            .query(EvidenceQueryFilter {
                query: "lease status".to_string(),
                max_depth: 2,
                scope: OntologyScope {
                    root_entities: vec![ScopedEntity {
                        entity_type: "Tenant".to_string(),
                        entity_id: "tenant-1".to_string(),
                    }],
                    concepts: vec!["Tenant".to_string()],
                    relationships: Vec::new(),
                },
            })
            .unwrap();
        assert_eq!(result[0].evidence_id, "evidence:rag:Tenant:tenant-1");
        assert_eq!(
            result[0].provenance,
            "deterministic-memory-evidence-provider"
        );
    }

    #[test]
    fn ontology_audit_event_redacts_secret_like_details() {
        let event = ontology_audit_event(
            "evidence.query.planned",
            "local-cli",
            serde_json::json!({
                "provider": {
                    "id": "rag",
                    "api_token": "should-not-leak",
                    "nested": {
                        "password": "also-hidden"
                    }
                }
            }),
        );
        assert_eq!(event.schema, "greentic.sorx.ontology.audit.v1");
        assert_eq!(event.details["provider"]["api_token"], "[REDACTED]");
        assert_eq!(
            event.details["provider"]["nested"]["password"],
            "[REDACTED]"
        );
        assert!(
            !serde_json::to_string(&event)
                .unwrap()
                .contains("should-not-leak")
        );
    }
}
