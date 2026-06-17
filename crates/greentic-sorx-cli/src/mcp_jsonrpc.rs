//! Minimal JSON-RPC 2.0 framing for the MCP Streamable-HTTP transport.
//!
//! Pure: it maps MCP methods onto the existing `McpToolList` + an injected
//! [`Invoker`] (the SoRX runtime). No HTTP, no auth — those live in
//! `http_runtime.rs` and `mcp_auth.rs`.

use greentic_sorx_core::McpToolList;
use greentic_sorx_core::{
    EndpointInvocation, EndpointResult, SorxResult,
};
use serde::Deserialize;
use serde_json::{json, Value};

/// MCP protocol revision this server speaks.
#[cfg_attr(not(test), allow(dead_code))]
pub const PROTOCOL_VERSION: &str = "2025-06-18";

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// Identity already resolved from the bearer token (see `mcp_auth`).
#[allow(dead_code)]
pub struct McpCaller {
    pub tenant_id: String,
    pub subject: String,
    pub roles: Vec<String>,
}

/// The runtime invoke seam. `http_runtime` adapts `SorxRuntime` to this.
#[allow(dead_code)]
pub trait Invoker {
    fn invoke(&self, inv: EndpointInvocation) -> SorxResult<EndpointResult>;
}

fn ok(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn err(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Dispatch a single JSON-RPC request. Always returns a JSON-RPC envelope.
#[cfg_attr(not(test), allow(dead_code))]
pub fn dispatch(
    req: &JsonRpcRequest,
    tools: &McpToolList,
    _caller: &McpCaller,
    _invoker: &dyn Invoker,
) -> Value {
    match req.method.as_str() {
        "initialize" => ok(
            &req.id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "greentic-sorx", "version": env!("CARGO_PKG_VERSION") }
            }),
        ),
        "tools/list" => {
            let listed: Vec<Value> = tools
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": t.input_schema.clone()
                            .unwrap_or_else(|| json!({ "type": "object" })),
                    })
                })
                .collect();
            ok(&req.id, json!({ "tools": listed }))
        }
        _ => err(&req.id, -32601, "method not found"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use greentic_sorx_core::{McpToolDefinition, McpToolList, RiskLevel};

    fn empty_tools() -> McpToolList {
        McpToolList { schema: "greentic.sorx.mcp-tools.v1".into(), tools: vec![] }
    }
    struct NoInvoke;
    impl Invoker for NoInvoke {
        fn invoke(&self, _: greentic_sorx_core::EndpointInvocation)
            -> greentic_sorx_core::SorxResult<greentic_sorx_core::EndpointResult> {
            panic!("not called")
        }
    }
    fn caller() -> McpCaller {
        McpCaller { tenant_id: "acme".into(), subject: "u1".into(), roles: vec![] }
    }

    fn one_tool() -> McpToolList {
        McpToolList {
            schema: "greentic.sorx.mcp-tools.v1".into(),
            tools: vec![McpToolDefinition {
                name: "payment.record".into(),
                description: Some("Record a payment".into()),
                endpoint_id: "payment.record".into(),
                operation_id: "payment.record".into(),
                risk: RiskLevel::Medium,
                input_schema: Some(serde_json::json!({ "type": "object" })),
            }],
        }
    }

    #[test]
    fn tools_list_projects_name_description_input_schema() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(), id: serde_json::json!(2),
            method: "tools/list".into(), params: Value::Null,
        };
        let out = dispatch(&req, &one_tool(), &caller(), &NoInvoke);
        let tools = out["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "payment.record");
        assert_eq!(tools[0]["description"], "Record a payment");
        assert_eq!(tools[0]["inputSchema"]["type"], "object");
    }

    #[test]
    fn initialize_returns_server_info_and_tool_capability() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(1),
            method: "initialize".into(),
            params: serde_json::json!({ "protocolVersion": "2025-06-18" }),
        };
        let out = dispatch(&req, &empty_tools(), &caller(), &NoInvoke);
        assert_eq!(out["jsonrpc"], "2.0");
        assert_eq!(out["id"], 1);
        assert_eq!(out["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert!(out["result"]["capabilities"]["tools"].is_object());
        assert_eq!(out["result"]["serverInfo"]["name"], "greentic-sorx");
    }

    #[test]
    fn unknown_method_returns_jsonrpc_error_minus_32601() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(), id: serde_json::json!("x"),
            method: "nope".into(), params: serde_json::Value::Null,
        };
        let out = dispatch(&req, &empty_tools(), &caller(), &NoInvoke);
        assert_eq!(out["error"]["code"], -32601);
    }
}
