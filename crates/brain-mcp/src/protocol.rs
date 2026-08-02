use serde_json::{Value, json};

use crate::tools::BrainTools;

const PROTOCOL_VERSION: &str = "2025-06-18";

pub struct McpServer {
    tools: BrainTools,
    negotiated: bool,
}

impl McpServer {
    pub const fn new(tools: BrainTools) -> Self {
        Self {
            tools,
            negotiated: false,
        }
    }

    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let message = match serde_json::from_str::<Value>(line) {
            Ok(Value::Object(message)) => message,
            Ok(_) => return Some(error_response(Value::Null, -32600, "Invalid Request")),
            Err(error) => {
                return Some(error_response(
                    Value::Null,
                    -32700,
                    &format!("Parse error: {error}"),
                ));
            }
        };
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str);
        if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") || method.is_none() {
            return id.map(|id| error_response(id, -32600, "Invalid Request"));
        }
        let method = method.expect("checked above");
        if id.is_none() {
            if method == "notifications/initialized" {
                self.negotiated = true;
            }
            return None;
        }
        let id = id.expect("checked above");
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        let result = match method {
            "initialize" => {
                self.negotiated = true;
                Ok(json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "agent-brain", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": "Every tool requires an explicit registered project ID or path alias. Canonical SQLite retrieval remains available when optional providers are offline."
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" if self.negotiated => Ok(json!({"tools": self.tools.definitions()})),
            "tools/call" if self.negotiated => self.call_tool(params),
            "tools/list" | "tools/call" => Err((-32002, "Server is not initialized".to_owned())),
            _ => Err((-32601, format!("Method not found: {method}"))),
        };
        Some(match result {
            Ok(result) => success_response(id, result),
            Err((code, message)) => error_response(id, code, &message),
        })
    }

    fn call_tool(&self, params: Value) -> Result<Value, (i64, String)> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| (-32602, "tools/call requires a tool name".to_owned()))?;
        if !self.tools.has_tool(name) {
            return Err((-32602, format!("Unknown tool: {name}")));
        }
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        match self.tools.call(name, arguments) {
            Ok(structured) => {
                let text = serde_json::to_string_pretty(&structured)
                    .map_err(|error| (-32603, error.to_string()))?;
                Ok(json!({
                    "content": [{"type": "text", "text": text}],
                    "structuredContent": structured,
                    "isError": false
                }))
            }
            Err(error) => Ok(json!({
                "content": [{"type": "text", "text": error.to_string()}],
                "isError": true
            })),
        }
    }
}

fn success_response(id: Value, result: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

fn error_response(id: Value, code: i64, message: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message}
    })
    .to_string()
}
