//! Minimal JSON-RPC 2.0 framing for the MCP Streamable-HTTP transport.
//!
//! Pure: it maps MCP methods onto the existing `McpToolList` + an injected
//! [`Invoker`] (the SoRX runtime). No HTTP, no auth — those live in
//! `http_runtime.rs` and `mcp_auth.rs`.

use greentic_sorx_core::McpToolList;
use greentic_sorx_core::{
    CallerContext, EndpointInvocation, EndpointResult, EndpointStatus, InvocationSource, SorxResult,
};
use serde::Deserialize;
use serde_json::{Value, json};

/// MCP protocol revision this server speaks.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    /// Retained for protocol conformance validation but not inspected at runtime.
    #[allow(dead_code)]
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// Identity already resolved from the bearer token (see `mcp_auth`).
pub struct McpCaller {
    pub tenant_id: String,
    pub subject: String,
    pub roles: Vec<String>,
}

/// The runtime invoke seam. `http_runtime` adapts `SorxRuntime` to this.
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
pub fn dispatch(
    req: &JsonRpcRequest,
    tools: &McpToolList,
    caller: &McpCaller,
    invoker: &dyn Invoker,
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
        "tools/call" => {
            let name = match req.params.get("name").and_then(Value::as_str) {
                Some(n) => n,
                None => return err(&req.id, -32602, "tools/call requires a string `name`"),
            };
            let Some(tool) = tools.tools.iter().find(|t| t.name == name) else {
                return err(&req.id, -32602, "unknown MCP tool");
            };
            let arguments = req
                .params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let idempotency_key = req
                .params
                .get("_meta")
                .and_then(|m| m.get("idempotencyKey"))
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let invocation = EndpointInvocation {
                tenant_id: caller.tenant_id.clone(),
                endpoint_id: tool.endpoint_id.clone(),
                operation_id: tool.operation_id.clone(),
                input: arguments,
                caller: CallerContext {
                    subject: caller.subject.clone(),
                    roles: caller.roles.clone(),
                },
                idempotency_key,
                source: InvocationSource::Mcp,
            };
            match invoker.invoke(invocation) {
                Ok(result) => {
                    let is_error = !matches!(
                        result.status,
                        EndpointStatus::Ok | EndpointStatus::Created | EndpointStatus::Deleted
                    );
                    let text = serde_json::to_string(&json!({
                        "status": result.status,
                        "output": result.output,
                    }))
                    .unwrap_or_else(|_| "{}".to_string());
                    ok(
                        &req.id,
                        json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }),
                    )
                }
                Err(e) => ok(
                    &req.id,
                    json!({
                        "content": [{ "type": "text", "text": format!("{}: {}", e.code, e.message) }],
                        "isError": true
                    }),
                ),
            }
        }
        _ => err(&req.id, -32601, "method not found"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use greentic_sorx_core::{McpToolDefinition, McpToolList, RiskLevel};

    fn empty_tools() -> McpToolList {
        McpToolList {
            schema: "greentic.sorx.mcp-tools.v1".into(),
            tools: vec![],
        }
    }
    struct NoInvoke;
    impl Invoker for NoInvoke {
        fn invoke(
            &self,
            _: greentic_sorx_core::EndpointInvocation,
        ) -> greentic_sorx_core::SorxResult<greentic_sorx_core::EndpointResult> {
            panic!("not called")
        }
    }
    fn caller() -> McpCaller {
        McpCaller {
            tenant_id: "acme".into(),
            subject: "u1".into(),
            roles: vec![],
        }
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
            jsonrpc: "2.0".into(),
            id: serde_json::json!(2),
            method: "tools/list".into(),
            params: Value::Null,
        };
        let out = dispatch(&req, &one_tool(), &caller(), &NoInvoke);
        let tools = out["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "payment.record");
        assert_eq!(tools[0]["description"], "Record a payment");
        assert_eq!(tools[0]["inputSchema"]["type"], "object");
    }

    #[test]
    fn tools_list_defaults_missing_input_schema_to_object() {
        let tools = McpToolList {
            schema: "greentic.sorx.mcp-tools.v1".into(),
            tools: vec![McpToolDefinition {
                name: "noop".into(),
                description: None,
                endpoint_id: "noop".into(),
                operation_id: "noop".into(),
                risk: RiskLevel::Low,
                input_schema: None,
            }],
        };
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(3),
            method: "tools/list".into(),
            params: Value::Null,
        };
        let out = dispatch(&req, &tools, &caller(), &NoInvoke);
        let listed = out["result"]["tools"].as_array().unwrap();
        assert_eq!(
            listed[0]["inputSchema"],
            serde_json::json!({ "type": "object" })
        );
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
            jsonrpc: "2.0".into(),
            id: serde_json::json!("x"),
            method: "nope".into(),
            params: serde_json::Value::Null,
        };
        let out = dispatch(&req, &empty_tools(), &caller(), &NoInvoke);
        assert_eq!(out["error"]["code"], -32601);
    }

    struct OkInvoke {
        last: std::cell::RefCell<Option<EndpointInvocation>>,
    }
    impl Invoker for OkInvoke {
        fn invoke(
            &self,
            inv: EndpointInvocation,
        ) -> greentic_sorx_core::SorxResult<EndpointResult> {
            *self.last.borrow_mut() = Some(inv);
            Ok(EndpointResult {
                status: EndpointStatus::Ok,
                output: serde_json::json!({ "id": "pay_1" }),
                events: vec![],
            })
        }
    }

    #[test]
    fn tools_call_invokes_with_mcp_source_and_wraps_output() {
        let inv = OkInvoke {
            last: std::cell::RefCell::new(None),
        };
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(3),
            method: "tools/call".into(),
            params: serde_json::json!({
                "name": "payment.record",
                "arguments": { "amount": 10 }
            }),
        };
        let out = dispatch(&req, &one_tool(), &caller(), &inv);
        // routed to the runtime with the right identity + source
        let seen = inv.last.borrow();
        let seen = seen.as_ref().unwrap();
        assert_eq!(seen.endpoint_id, "payment.record");
        assert_eq!(seen.tenant_id, "acme");
        assert_eq!(seen.caller.subject, "u1");
        assert_eq!(seen.source, InvocationSource::Mcp);
        // wrapped result
        assert_eq!(out["result"]["isError"], false);
        let text = out["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("pay_1"));
    }

    #[test]
    fn tools_call_unknown_tool_is_invalid_params() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(4),
            method: "tools/call".into(),
            params: serde_json::json!({ "name": "nope", "arguments": {} }),
        };
        let out = dispatch(&req, &one_tool(), &caller(), &NoInvoke);
        assert_eq!(out["error"]["code"], -32602);
    }
}
