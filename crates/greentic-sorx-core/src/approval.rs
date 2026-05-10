use serde::{Deserialize, Serialize};

use crate::{CallerContext, RiskLevel, SorxResult};

pub trait ApprovalBroker: Send + Sync {
    fn decide(&self, request: ApprovalRequest) -> SorxResult<ApprovalDecision>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub request_id: String,
    pub tenant_id: String,
    pub endpoint_id: String,
    pub operation_id: String,
    pub risk: RiskLevel,
    pub reason: String,
    pub caller: CallerContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalDecision {
    pub status: ApprovalStatus,
    pub request_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Approved,
    Denied,
    Pending,
}

#[derive(Debug, Default)]
pub struct LocalAutoApproveBroker;

impl ApprovalBroker for LocalAutoApproveBroker {
    fn decide(&self, request: ApprovalRequest) -> SorxResult<ApprovalDecision> {
        Ok(ApprovalDecision {
            status: ApprovalStatus::Approved,
            request_id: request.request_id,
            reason: "Local auto-approval broker approved request".to_string(),
        })
    }
}

#[derive(Debug, Default)]
pub struct LocalDenyBroker;

impl ApprovalBroker for LocalDenyBroker {
    fn decide(&self, request: ApprovalRequest) -> SorxResult<ApprovalDecision> {
        Ok(ApprovalDecision {
            status: ApprovalStatus::Denied,
            request_id: request.request_id,
            reason: "Local deny broker denied request".to_string(),
        })
    }
}

#[derive(Debug, Default)]
pub struct LocalPendingBroker;

impl ApprovalBroker for LocalPendingBroker {
    fn decide(&self, request: ApprovalRequest) -> SorxResult<ApprovalDecision> {
        Ok(ApprovalDecision {
            status: ApprovalStatus::Pending,
            request_id: request.request_id,
            reason: request.reason,
        })
    }
}
