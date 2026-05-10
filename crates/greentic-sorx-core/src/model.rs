use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EndpointMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl EndpointMethod {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_uppercase().as_str() {
            "GET" => Some(Self::Get),
            "POST" => Some(Self::Post),
            "PUT" => Some(Self::Put),
            "PATCH" => Some(Self::Patch),
            "DELETE" => Some(Self::Delete),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "critical" => Some(Self::Critical),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Create,
    Get,
    Update,
    Query,
    Delete,
}

impl OperationKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "create" => Some(Self::Create),
            "get" | "read" => Some(Self::Get),
            "update" | "patch" => Some(Self::Update),
            "query" | "list" | "search" => Some(Self::Query),
            "delete" | "remove" => Some(Self::Delete),
            _ => None,
        }
    }

    pub fn is_mutating(self) -> bool {
        matches!(self, Self::Create | Self::Update | Self::Delete)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EndpointDefinition {
    pub endpoint_id: String,
    pub operation_id: String,
    pub operation: OperationKind,
    pub method: EndpointMethod,
    pub path: String,
    pub entity: Option<String>,
    pub collection: String,
    pub provider_binding: String,
    pub risk: RiskLevel,
    pub approval: Option<ApprovalRequirement>,
    pub input_schema: Option<Value>,
    pub output_schema: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequirement {
    pub required: bool,
    pub roles: Vec<String>,
    pub reason_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallerContext {
    pub subject: String,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EndpointInvocation {
    pub tenant_id: String,
    pub endpoint_id: String,
    pub operation_id: String,
    pub input: Value,
    pub caller: CallerContext,
    pub idempotency_key: Option<String>,
    pub source: InvocationSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationSource {
    Direct,
    Http,
    Mcp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointStatus {
    Created,
    Ok,
    NotFound,
    Deleted,
    ApprovalRequired,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EndpointResult {
    pub status: EndpointStatus,
    pub output: Value,
    pub events: Vec<SorxEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SorxEvent {
    pub schema: String,
    pub event_type: String,
    pub tenant_id: String,
    pub endpoint_id: String,
    pub operation_id: String,
    pub provider_binding: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimePack {
    pub name: String,
    pub version: String,
    pub digest: Option<String>,
}
