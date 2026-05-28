use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagerPolicyEffect {
    Allow,
    ReadOnly,
    Redact,
    Hide,
    RequiresApproval,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagerPolicyDecision {
    pub effect: ManagerPolicyEffect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_hint: Option<String>,
}

impl ManagerPolicyDecision {
    pub fn allow() -> Self {
        Self {
            effect: ManagerPolicyEffect::Allow,
            reason_code: None,
            message_key: None,
            audit_hint: None,
        }
    }

    pub fn with_effect(effect: ManagerPolicyEffect) -> Self {
        Self {
            effect,
            reason_code: None,
            message_key: None,
            audit_hint: None,
        }
    }

    pub fn is_hidden(&self) -> bool {
        matches!(
            self.effect,
            ManagerPolicyEffect::Hide | ManagerPolicyEffect::Deny
        )
    }
}

impl Default for ManagerPolicyDecision {
    fn default() -> Self {
        Self::allow()
    }
}
