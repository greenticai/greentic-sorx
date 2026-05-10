use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    CallerContext, EndpointInvocation, EndpointResult, EndpointRouter, InvocationSource, RiskLevel,
    SorxError, SorxResult, SorxRuntime,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpToolList {
    pub schema: String,
    pub tools: Vec<McpToolDefinition>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpToolDefinition {
    pub name: String,
    pub description: Option<String>,
    pub endpoint_id: String,
    pub operation_id: String,
    pub risk: RiskLevel,
    pub input_schema: Option<Value>,
}

#[derive(Clone)]
pub struct McpRuntime {
    runtime: SorxRuntime,
    tools: McpToolList,
}

impl McpRuntime {
    pub fn new(runtime: SorxRuntime, tools: McpToolList) -> Self {
        Self { runtime, tools }
    }

    pub fn tools(&self) -> &McpToolList {
        &self.tools
    }

    pub fn call_tool(
        &self,
        tool_name: &str,
        tenant_id: impl Into<String>,
        caller: CallerContext,
        input: Value,
        idempotency_key: Option<String>,
    ) -> SorxResult<EndpointResult> {
        let tool = self
            .tools
            .tools
            .iter()
            .find(|tool| tool.name == tool_name)
            .ok_or_else(|| {
                SorxError::new(
                    "unknown_mcp_tool",
                    format!("unknown MCP tool `{tool_name}`"),
                )
            })?;
        self.runtime.invoke(EndpointInvocation {
            tenant_id: tenant_id.into(),
            endpoint_id: tool.endpoint_id.clone(),
            operation_id: tool.operation_id.clone(),
            input,
            caller,
            idempotency_key,
            source: InvocationSource::Mcp,
        })
    }
}

pub fn mcp_tools_from_metadata(
    metadata: Option<&Value>,
    router: &EndpointRouter,
) -> SorxResult<McpToolList> {
    let Some(metadata) = metadata else {
        return Ok(McpToolList {
            schema: "greentic.sorx.mcp-tools.v1".to_string(),
            tools: Vec::new(),
        });
    };
    let tools = metadata
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SorxError::new(
                "invalid_mcp_tools",
                "mcp-tools.json must contain a tools array",
            )
        })?;

    let mut out = Vec::new();
    let mut names = std::collections::BTreeSet::new();
    for (index, value) in tools.iter().enumerate() {
        let path = format!("tools[{index}]");
        let name = value
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| SorxError::at_path("invalid_mcp_tools", "tool is missing name", &path))?
            .to_string();
        if !names.insert(name.clone()) {
            return Err(SorxError::at_path(
                "duplicate_mcp_tool",
                format!("duplicate MCP tool `{name}`"),
                &path,
            ));
        }
        let endpoint_id = value
            .get("endpoint_id")
            .and_then(Value::as_str)
            .or_else(|| value.get("endpointId").and_then(Value::as_str))
            .ok_or_else(|| {
                SorxError::at_path("invalid_mcp_tools", "tool is missing endpoint_id", &path)
            })?;
        let endpoint = router.endpoint(endpoint_id).map_err(|err| {
            SorxError::at_path(
                err.code,
                format!("MCP tool `{name}` references unknown endpoint `{endpoint_id}`"),
                &path,
            )
        })?;
        let operation_id = value
            .get("operation_id")
            .and_then(Value::as_str)
            .or_else(|| value.get("operationId").and_then(Value::as_str))
            .unwrap_or(&endpoint.operation_id);
        if operation_id != endpoint.operation_id {
            return Err(SorxError::at_path(
                "invalid_mcp_tools",
                format!(
                    "MCP tool `{name}` maps endpoint `{endpoint_id}` to wrong operation `{operation_id}`"
                ),
                &path,
            ));
        }
        out.push(McpToolDefinition {
            name,
            description: value
                .get("description")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            endpoint_id: endpoint.endpoint_id.clone(),
            operation_id: endpoint.operation_id.clone(),
            risk: endpoint.risk,
            input_schema: value
                .get("input_schema")
                .cloned()
                .or_else(|| endpoint.input_schema.clone()),
        });
    }

    Ok(McpToolList {
        schema: "greentic.sorx.mcp-tools.v1".to_string(),
        tools: out,
    })
}
