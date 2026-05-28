use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{EndpointDefinition, RiskLevel};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyConfig {
    pub approvals: BTreeMap<RiskLevel, PolicyMode>,
}

impl PolicyConfig {
    pub fn from_modes(modes: &BTreeMap<String, String>) -> Self {
        let mut approvals = Self::default().approvals;
        for (risk, mode) in modes {
            if let (Some(risk), Some(mode)) = (RiskLevel::parse(risk), PolicyMode::parse(mode)) {
                approvals.insert(risk, mode);
            }
        }
        Self { approvals }
    }

    pub fn mode_for(&self, risk: RiskLevel) -> PolicyMode {
        self.approvals.get(&risk).copied().unwrap_or_else(|| {
            let defaults = Self::default();
            defaults.approvals[&risk]
        })
    }
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            approvals: BTreeMap::from([
                (RiskLevel::Low, PolicyMode::Auto),
                (RiskLevel::Medium, PolicyMode::Auto),
                (RiskLevel::High, PolicyMode::RequireApproval),
                (RiskLevel::Critical, PolicyMode::Deny),
            ]),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyMode {
    Auto,
    RequireApproval,
    Deny,
}

impl PolicyMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "require_approval" => Some(Self::RequireApproval),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub action: PolicyAction,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyAction {
    Execute,
    RequireApproval,
    Deny,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PolicyEngine {
    pub config: PolicyConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationPolicyInput {
    pub principal_subject: String,
    pub principal_roles: Vec<String>,
    pub resource: AuthorizationPolicyResource,
    pub operation: String,
    pub policies: Vec<String>,
    pub conditions: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizationPolicyResource {
    Endpoint { endpoint_id: String },
    Record { record: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OntologyPolicySubject {
    pub subject: String,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OntologyPolicyResource {
    Concept {
        concept: String,
    },
    Entity {
        entity_type: String,
        entity_id: String,
    },
    Field {
        entity_type: String,
        field: String,
    },
    Relationship {
        relationship: String,
    },
    Evidence {
        entity_type: String,
        entity_id: String,
    },
    Action {
        endpoint_id: String,
    },
    ExternalReference {
        reference: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OntologyPolicyAction {
    Read,
    Traverse,
    RetrieveEvidence,
    Invoke,
    Execute,
    Resolve,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OntologyPolicyDecision {
    pub decision: OntologyPolicyDecisionKind,
    pub reasons: Vec<OntologyPolicyReason>,
    pub redactions: Vec<OntologyPolicyRedaction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OntologyPolicyDecisionKind {
    Allow,
    Deny,
    RequiresApproval,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OntologyPolicyReason {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OntologyPolicyRedaction {
    pub entity_type: String,
    pub field: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SensitivityContext {
    pub sensitive_fields: BTreeMap<String, BTreeSet<String>>,
    pub denied_relationships: BTreeSet<String>,
    pub evidence_requires_approval: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipTraversalPermission {
    Allowed,
    Denied,
    RequiresApproval,
}

impl PolicyEngine {
    pub fn new(config: PolicyConfig) -> Self {
        Self { config }
    }

    pub fn decide(&self, endpoint: &EndpointDefinition) -> PolicyDecision {
        match self.config.mode_for(endpoint.risk) {
            PolicyMode::Auto => {
                if endpoint
                    .approval
                    .as_ref()
                    .is_some_and(|approval| approval.required)
                {
                    PolicyDecision {
                        action: PolicyAction::RequireApproval,
                        reason: "Endpoint metadata requires approval".to_string(),
                    }
                } else {
                    PolicyDecision {
                        action: PolicyAction::Execute,
                        reason: "Policy allows automatic execution".to_string(),
                    }
                }
            }
            PolicyMode::RequireApproval => PolicyDecision {
                action: PolicyAction::RequireApproval,
                reason: "Risk policy requires approval".to_string(),
            },
            PolicyMode::Deny => PolicyDecision {
                action: PolicyAction::Deny,
                reason: "Risk policy denies execution".to_string(),
            },
        }
    }

    pub fn decide_authorization(&self, input: &AuthorizationPolicyInput) -> PolicyDecision {
        if input.policies.is_empty() && input.conditions.is_none() {
            return PolicyDecision {
                action: PolicyAction::Execute,
                reason: "No authorization policies were declared".to_string(),
            };
        }
        PolicyDecision {
            action: PolicyAction::Execute,
            reason: "Authorization policy inputs accepted".to_string(),
        }
    }

    pub fn decide_ontology(
        &self,
        subject: &OntologyPolicySubject,
        resource: &OntologyPolicyResource,
        action: OntologyPolicyAction,
        sensitivity: &SensitivityContext,
    ) -> OntologyPolicyDecision {
        match (resource, action) {
            (OntologyPolicyResource::Field { entity_type, field }, OntologyPolicyAction::Read)
                if sensitivity
                    .sensitive_fields
                    .get(entity_type)
                    .is_some_and(|fields| fields.contains(field))
                    && !subject.roles.iter().any(|role| role == "pii_reader") =>
            {
                OntologyPolicyDecision {
                    decision: OntologyPolicyDecisionKind::Deny,
                    reasons: vec![OntologyPolicyReason {
                        code: "pii_requires_policy".to_string(),
                        message: format!("{entity_type}.{field} is marked as sensitive"),
                    }],
                    redactions: vec![OntologyPolicyRedaction {
                        entity_type: entity_type.clone(),
                        field: field.clone(),
                    }],
                }
            }
            (
                OntologyPolicyResource::Relationship { relationship },
                OntologyPolicyAction::Traverse,
            ) if sensitivity.denied_relationships.contains(relationship) => {
                OntologyPolicyDecision {
                    decision: OntologyPolicyDecisionKind::Deny,
                    reasons: vec![OntologyPolicyReason {
                        code: "relationship_traversal_denied".to_string(),
                        message: format!("relationship `{relationship}` traversal is denied"),
                    }],
                    redactions: Vec::new(),
                }
            }
            (OntologyPolicyResource::Evidence { .. }, OntologyPolicyAction::RetrieveEvidence)
                if sensitivity.evidence_requires_approval =>
            {
                OntologyPolicyDecision {
                    decision: OntologyPolicyDecisionKind::RequiresApproval,
                    reasons: vec![OntologyPolicyReason {
                        code: "evidence_requires_approval".to_string(),
                        message: "evidence retrieval requires approval".to_string(),
                    }],
                    redactions: Vec::new(),
                }
            }
            (OntologyPolicyResource::Action { .. }, OntologyPolicyAction::Execute)
                if !subject.roles.iter().any(|role| role == "action_executor") =>
            {
                OntologyPolicyDecision {
                    decision: OntologyPolicyDecisionKind::RequiresApproval,
                    reasons: vec![OntologyPolicyReason {
                        code: "action_requires_approval".to_string(),
                        message: "side-effectful action requires approval".to_string(),
                    }],
                    redactions: Vec::new(),
                }
            }
            _ => OntologyPolicyDecision {
                decision: OntologyPolicyDecisionKind::Allow,
                reasons: Vec::new(),
                redactions: Vec::new(),
            },
        }
    }
}

#[cfg(test)]
mod ontology_policy_tests {
    use super::*;

    fn subject() -> OntologyPolicySubject {
        OntologyPolicySubject {
            subject: "tester".to_string(),
            roles: Vec::new(),
        }
    }

    #[test]
    fn concept_read_allowed() {
        let decision = PolicyEngine::default().decide_ontology(
            &subject(),
            &OntologyPolicyResource::Concept {
                concept: "Tenant".to_string(),
            },
            OntologyPolicyAction::Read,
            &SensitivityContext::default(),
        );
        assert_eq!(decision.decision, OntologyPolicyDecisionKind::Allow);
    }

    #[test]
    fn field_read_denied_because_sensitivity() {
        let mut sensitivity = SensitivityContext::default();
        sensitivity
            .sensitive_fields
            .entry("Tenant".to_string())
            .or_default()
            .insert("email".to_string());
        let decision = PolicyEngine::default().decide_ontology(
            &subject(),
            &OntologyPolicyResource::Field {
                entity_type: "Tenant".to_string(),
                field: "email".to_string(),
            },
            OntologyPolicyAction::Read,
            &sensitivity,
        );
        assert_eq!(decision.decision, OntologyPolicyDecisionKind::Deny);
        assert_eq!(decision.redactions[0].field, "email");
    }

    #[test]
    fn relationship_traversal_denied() {
        let mut sensitivity = SensitivityContext::default();
        sensitivity
            .denied_relationships
            .insert("tenant_makes_payment".to_string());
        let decision = PolicyEngine::default().decide_ontology(
            &subject(),
            &OntologyPolicyResource::Relationship {
                relationship: "tenant_makes_payment".to_string(),
            },
            OntologyPolicyAction::Traverse,
            &sensitivity,
        );
        assert_eq!(decision.decision, OntologyPolicyDecisionKind::Deny);
    }

    #[test]
    fn evidence_query_requires_approval() {
        let decision = PolicyEngine::default().decide_ontology(
            &subject(),
            &OntologyPolicyResource::Evidence {
                entity_type: "Tenant".to_string(),
                entity_id: "tenant-1".to_string(),
            },
            OntologyPolicyAction::RetrieveEvidence,
            &SensitivityContext {
                evidence_requires_approval: true,
                ..SensitivityContext::default()
            },
        );
        assert_eq!(
            decision.decision,
            OntologyPolicyDecisionKind::RequiresApproval
        );
    }

    #[test]
    fn side_effectful_action_requires_approval() {
        let decision = PolicyEngine::default().decide_ontology(
            &subject(),
            &OntologyPolicyResource::Action {
                endpoint_id: "tenant.create".to_string(),
            },
            OntologyPolicyAction::Execute,
            &SensitivityContext::default(),
        );
        assert_eq!(
            decision.decision,
            OntologyPolicyDecisionKind::RequiresApproval
        );
    }
}
