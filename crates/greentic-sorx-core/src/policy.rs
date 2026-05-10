use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
}
