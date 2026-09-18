use serde_json::{json, Value};
use sqlx::SqlitePool;
use tracing::{debug, warn};

use super::adapter;
use super::prompts;
use super::protocol::{
    error_codes, CallToolParams, GetPromptParams, Implementation, InitializeResult, JsonRpcRequest,
    JsonRpcResponse, PromptsCapability, ReadResourceParams, ResourcesCapability,
    ServerCapabilities, ToolsCapability, MCP_PROTOCOL_VERSION, SERVER_NAME, SERVER_VERSION,
};
use super::resources;
use crate::events::EventBroadcaster;

/// Core service engine dispatching MCP JSON-RPC protocol requests.
pub struct McpService<'a> {
    pub pool: &'a SqlitePool,
    pub broadcaster: Option<&'a EventBroadcaster>,
    pub owner_id: &'a str,
}

impl<'a> McpService<'a> {
    pub fn new(
        pool: &'a SqlitePool,
        broadcaster: Option<&'a EventBroadcaster>,
        owner_id: &'a str,
    ) -> Self {
        Self {
            pool,
            broadcaster,
            owner_id,
        }
    }

    /// Handles a single incoming JSON-RPC request and produces an optional response.
    pub async fn handle_request(&self, req: JsonRpcRequest) -> Option<JsonRpcResponse> {
        let id = req.id.clone().unwrap_or(Value::Null);

        match req.method.as_str() {
            "initialize" => {
                let result = InitializeResult {
                    protocol_version: MCP_PROTOCOL_VERSION.to_string(),
                    capabilities: ServerCapabilities {
                        tools: Some(ToolsCapability {
                            list_changed: false,
                        }),
                        resources: Some(ResourcesCapability {
                            subscribe: false,
                            list_changed: false,
                        }),
                        prompts: Some(PromptsCapability {
                            list_changed: false,
                        }),
                    },
                    server_info: Implementation {
                        name: SERVER_NAME.to_string(),
                        version: SERVER_VERSION.to_string(),
                    },
                };
                Some(JsonRpcResponse::success(id, json!(result)))
            }

            "notifications/initialized" => {
                debug!("MCP client initialized notification received");
                None
            }

            "ping" => Some(JsonRpcResponse::success(id, json!({}))),

            "tools/list" => {
                let tools = adapter::get_mcp_tools();
                Some(JsonRpcResponse::success(id, json!({ "tools": tools })))
            }

            "tools/call" => {
                let params: CallToolParams = match req.params {
                    Some(p) => match serde_json::from_value(p) {
                        Ok(val) => val,
                        Err(e) => {
                            return Some(JsonRpcResponse::error(
                                id,
                                error_codes::INVALID_PARAMS,
                                format!("Invalid params for tools/call: {}", e),
                            ));
                        }
                    },
                    None => {
                        return Some(JsonRpcResponse::error(
                            id,
                            error_codes::INVALID_PARAMS,
                            "Missing params for tools/call",
                        ));
                    }
                };

                let res =
                    adapter::call_mcp_tool(self.pool, self.broadcaster, self.owner_id, params)
                        .await;
                Some(JsonRpcResponse::success(id, json!(res)))
            }

            "resources/list" => {
                let resources = resources::list_resources();
                Some(JsonRpcResponse::success(
                    id,
                    json!({ "resources": resources }),
                ))
            }

            "resources/read" => {
                let params: ReadResourceParams = match req.params {
                    Some(p) => match serde_json::from_value(p) {
                        Ok(val) => val,
                        Err(e) => {
                            return Some(JsonRpcResponse::error(
                                id,
                                error_codes::INVALID_PARAMS,
                                format!("Invalid params for resources/read: {}", e),
                            ));
                        }
                    },
                    None => {
                        return Some(JsonRpcResponse::error(
                            id,
                            error_codes::INVALID_PARAMS,
                            "Missing params for resources/read",
                        ));
                    }
                };

                match resources::read_resource(self.pool, self.owner_id, &params.uri).await {
                    Ok(contents) => Some(JsonRpcResponse::success(id, json!(contents))),
                    Err(e) => Some(JsonRpcResponse::error(
                        id,
                        error_codes::INTERNAL_ERROR,
                        format!("Failed to read resource: {}", e),
                    )),
                }
            }

            "prompts/list" => {
                let prompts = prompts::list_prompts();
                Some(JsonRpcResponse::success(id, json!({ "prompts": prompts })))
            }

            "prompts/get" => {
                let params: GetPromptParams = match req.params {
                    Some(p) => match serde_json::from_value(p) {
                        Ok(val) => val,
                        Err(e) => {
                            return Some(JsonRpcResponse::error(
                                id,
                                error_codes::INVALID_PARAMS,
                                format!("Invalid params for prompts/get: {}", e),
                            ));
                        }
                    },
                    None => {
                        return Some(JsonRpcResponse::error(
                            id,
                            error_codes::INVALID_PARAMS,
                            "Missing params for prompts/get",
                        ));
                    }
                };

                match prompts::get_prompt(&params.name, params.arguments) {
                    Ok(prompt) => Some(JsonRpcResponse::success(id, json!(prompt))),
                    Err(e) => Some(JsonRpcResponse::error(
                        id,
                        error_codes::INTERNAL_ERROR,
                        format!("Failed to get prompt: {}", e),
                    )),
                }
            }

            other => {
                warn!("MCP unknown method called: {}", other);
                Some(JsonRpcResponse::error(
                    id,
                    error_codes::METHOD_NOT_FOUND,
                    format!("Method not found: {}", other),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mcp_service_initialize() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let service = McpService::new(&pool, None, "test_owner");

        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "initialize".to_string(),
            params: None,
        };

        let resp = service.handle_request(req).await.unwrap();
        assert_eq!(resp.id, json!(1));
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], MCP_PROTOCOL_VERSION);
    }

    #[tokio::test]
    async fn test_mcp_service_tools_list() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let service = McpService::new(&pool, None, "test_owner");

        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(2)),
            method: "tools/list".to_string(),
            params: None,
        };

        let resp = service.handle_request(req).await.unwrap();
        assert_eq!(resp.id, json!(2));
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert!(!tools.is_empty());
    }

    #[tokio::test]
    async fn test_mcp_service_resources_and_prompts() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let service = McpService::new(&pool, None, "test_owner");

        let req_res = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(3)),
            method: "resources/list".to_string(),
            params: None,
        };
        let resp_res = service.handle_request(req_res).await.unwrap();
        let resources = resp_res.result.unwrap()["resources"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(resources.len(), 5);

        let req_prompt = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(4)),
            method: "prompts/list".to_string(),
            params: None,
        };
        let resp_prompt = service.handle_request(req_prompt).await.unwrap();
        let prompts = resp_prompt.result.unwrap()["prompts"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(prompts.len(), 3);
    }
}
