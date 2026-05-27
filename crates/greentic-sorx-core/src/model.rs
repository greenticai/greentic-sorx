use std::collections::BTreeMap;

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Create,
    Get,
    Update,
    Query,
    Delete,
    Command(Box<CommandSpec>),
}

impl OperationKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "create" => Some(Self::Create),
            "get" | "read" => Some(Self::Get),
            "update" | "patch" => Some(Self::Update),
            "query" | "list" | "search" => Some(Self::Query),
            "delete" | "remove" => Some(Self::Delete),
            "command" => Some(Self::Command(Box::default())),
            _ => None,
        }
    }

    pub fn is_mutating(&self) -> bool {
        matches!(
            self,
            Self::Create | Self::Update | Self::Delete | Self::Command(_)
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CommandSpec {
    pub kind: Option<String>,
    pub action: Option<String>,
    pub target: Option<String>,
    pub idempotency: Option<String>,
    #[serde(default)]
    pub constraints: CommandConstraints,
    pub steps: Vec<CommandStep>,
    #[serde(default, rename = "return")]
    pub return_value: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandConstraints {
    #[serde(default)]
    pub idempotency: Option<CommandIdempotencyConstraint>,
    #[serde(default)]
    pub unique: Vec<CommandUniqueConstraint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandIdempotencyConstraint {
    #[serde(default)]
    pub index: Option<String>,
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandUniqueConstraint {
    #[serde(default)]
    pub index: Option<String>,
    #[serde(default)]
    pub record: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum CommandStep {
    Create {
        #[serde(default, rename = "as")]
        as_name: Option<String>,
        #[serde(default)]
        when: Option<Value>,
        entity: Option<String>,
        collection: Option<String>,
        input: Option<Value>,
    },
    DeleteWhere {
        #[serde(default, rename = "as")]
        as_name: Option<String>,
        #[serde(default)]
        when: Option<Value>,
        entity: Option<String>,
        collection: Option<String>,
        #[serde(default)]
        r#where: Value,
    },
    UpdateWhere {
        #[serde(default, rename = "as")]
        as_name: Option<String>,
        #[serde(default)]
        when: Option<Value>,
        entity: Option<String>,
        collection: Option<String>,
        #[serde(default)]
        r#where: Value,
        #[serde(default)]
        set: Value,
    },
    IncrementWhere {
        #[serde(default, rename = "as")]
        as_name: Option<String>,
        #[serde(default)]
        when: Option<Value>,
        entity: Option<String>,
        collection: Option<String>,
        #[serde(default)]
        r#where: Value,
        #[serde(default)]
        increments: Value,
    },
    Query {
        #[serde(default, rename = "as")]
        as_name: Option<String>,
        #[serde(default)]
        when: Option<Value>,
        entity: Option<String>,
        collection: Option<String>,
        #[serde(default)]
        r#where: Value,
        #[serde(default)]
        order_by: Vec<CommandOrderBy>,
    },
    FindOne {
        #[serde(default, rename = "as")]
        as_name: Option<String>,
        #[serde(default)]
        when: Option<Value>,
        entity: Option<String>,
        collection: Option<String>,
        #[serde(default)]
        r#where: Value,
        #[serde(default)]
        required: bool,
    },
    EmitEvent {
        #[serde(default, rename = "as")]
        as_name: Option<String>,
        #[serde(default)]
        when: Option<Value>,
        event: String,
        #[serde(default)]
        payload: Value,
        stream: Option<String>,
    },
    Foreach {
        #[serde(default, rename = "as")]
        as_name: Option<String>,
        #[serde(default)]
        when: Option<Value>,
        items: Value,
        #[serde(default, rename = "do")]
        steps: Vec<CommandStep>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOrderBy {
    pub field: String,
    #[serde(default)]
    pub direction: CommandOrderDirection,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandOrderDirection {
    #[default]
    Asc,
    Desc,
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
    pub view: ViewTransform,
    pub query_plan: QueryPlan,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewTransform {
    pub read_only: bool,
    pub input_field_map: BTreeMap<String, String>,
    pub output_field_map: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryPlan {
    pub index: Option<IndexRequirement>,
    pub traversal: Option<TraversalRequirement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexRequirement {
    pub name: String,
    pub capability: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraversalRequirement {
    pub name: String,
    pub capability: String,
    pub max_depth: u8,
    pub relationships: Vec<String>,
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
    #[serde(default)]
    pub operational_indexes: Vec<RuntimeOperationalIndex>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeOperationalIndex {
    pub id: String,
    pub record: String,
    pub collection: Option<String>,
    pub kind: String,
    pub fields: Vec<String>,
    pub unique: bool,
}
