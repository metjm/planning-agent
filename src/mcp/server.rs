use super::protocol::{
    error_codes, InitializeParams, InitializeResult, JsonRpcRequest, JsonRpcResponse,
    ServerCapabilities, ServerInfo, Tool, ToolCallParams, ToolCallResult, ToolsCapability,
    ToolsListResult,
};
use super::review_schema::{get_plan_schema, submit_review_schema, SubmittedReview};
use anyhow::Result;
use serde_json::Value;
use std::io::{BufRead, Write};
use tokio::sync::mpsc;

const SERVER_NAME: &str = "planning-agent-review";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const PROTOCOL_VERSION: &str = "2024-11-05";

/// MCP Review Server that handles review feedback collection
pub struct McpReviewServer {
    /// Channel to send collected reviews back to the review phase
    review_tx: mpsc::Sender<SubmittedReview>,
    /// Plan content to serve via get_plan tool
    plan_content: String,
    /// Review instructions/prompt
    review_prompt: String,
    /// Whether a review has been submitted (only allow one)
    review_submitted: bool,
}

impl McpReviewServer {
    pub fn new(
        review_tx: mpsc::Sender<SubmittedReview>,
        plan_content: String,
        review_prompt: String,
    ) -> Self {
        Self {
            review_tx,
            plan_content,
            review_prompt,
            review_submitted: false,
        }
    }

    /// Run the MCP server synchronously, reading from stdin and writing to stdout.
    /// This is designed to be run in a subprocess.
    pub fn run_sync(mut self) -> Result<()> {
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout();

        for line in stdin.lock().lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            let response = self.handle_message(&line);
            if let Some(resp) = response {
                let json = serde_json::to_string(&resp)?;
                writeln!(stdout, "{}", json)?;
                stdout.flush()?;
            }
        }

        Ok(())
    }

    /// Handle a single JSON-RPC message
    fn handle_message(&mut self, message: &str) -> Option<JsonRpcResponse> {
        let request: JsonRpcRequest = match serde_json::from_str(message) {
            Ok(req) => req,
            Err(e) => {
                return Some(JsonRpcResponse::error(
                    None,
                    error_codes::PARSE_ERROR,
                    format!("Failed to parse request: {}", e),
                ));
            }
        };

        // Handle notifications (no id) - don't send response
        if request.id.is_none() {
            self.handle_notification(&request);
            return None;
        }

        let result = match request.method.as_str() {
            "initialize" => self.handle_initialize(request.params),
            "tools/list" => self.handle_tools_list(),
            "tools/call" => self.handle_tool_call(request.params),
            _ => Err((
                error_codes::METHOD_NOT_FOUND,
                format!("Method not found: {}", request.method),
            )),
        };

        Some(match result {
            Ok(value) => JsonRpcResponse::success(request.id, value),
            Err((code, message)) => JsonRpcResponse::error(request.id, code, message),
        })
    }

    /// Handle notifications (messages without an id)
    fn handle_notification(&mut self, request: &JsonRpcRequest) {
        match request.method.as_str() {
            "notifications/initialized" => {
                // Client is ready, nothing to do
            }
            _ => {
                // Unknown notification, ignore
            }
        }
    }

    /// Handle the initialize request
    fn handle_initialize(&self, params: Option<Value>) -> Result<Value, (i32, String)> {
        let _init_params: InitializeParams = params
            .map(|p| serde_json::from_value(p))
            .transpose()
            .map_err(|e| {
                (
                    error_codes::INVALID_PARAMS,
                    format!("Invalid initialize params: {}", e),
                )
            })?
            .unwrap_or(InitializeParams {
                protocol_version: PROTOCOL_VERSION.to_string(),
                capabilities: Default::default(),
                client_info: super::protocol::ClientInfo {
                    name: "unknown".to_string(),
                    version: "0.0.0".to_string(),
                },
            });

        let result = InitializeResult {
            protocol_version: PROTOCOL_VERSION.to_string(),
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability { list_changed: false }),
                resources: None,
            },
            server_info: ServerInfo {
                name: SERVER_NAME.to_string(),
                version: SERVER_VERSION.to_string(),
            },
        };

        serde_json::to_value(result)
            .map_err(|e| (error_codes::INTERNAL_ERROR, format!("Serialization error: {}", e)))
    }

    /// Handle tools/list request
    fn handle_tools_list(&self) -> Result<Value, (i32, String)> {
        let tools = vec![
            Tool {
                name: "get_plan".to_string(),
                description: "Get the implementation plan content to review.".to_string(),
                input_schema: get_plan_schema(),
            },
            Tool {
                name: "submit_review".to_string(),
                description: "Submit your review feedback for the implementation plan. You must call get_plan first to read the plan, then submit your review using this tool.".to_string(),
                input_schema: submit_review_schema(),
            },
        ];

        let result = ToolsListResult { tools };
        serde_json::to_value(result)
            .map_err(|e| (error_codes::INTERNAL_ERROR, format!("Serialization error: {}", e)))
    }

    /// Handle tools/call request
    fn handle_tool_call(&mut self, params: Option<Value>) -> Result<Value, (i32, String)> {
        let call_params: ToolCallParams = params
            .ok_or((error_codes::INVALID_PARAMS, "Missing params".to_string()))?
            .try_into()
            .map_err(|_| {
                (
                    error_codes::INVALID_PARAMS,
                    "Invalid tool call params".to_string(),
                )
            })?;

        let result = match call_params.name.as_str() {
            "get_plan" => self.handle_get_plan(),
            "submit_review" => self.handle_submit_review(call_params.arguments),
            _ => ToolCallResult::error(format!("Unknown tool: {}", call_params.name)),
        };

        serde_json::to_value(result)
            .map_err(|e| (error_codes::INTERNAL_ERROR, format!("Serialization error: {}", e)))
    }

    /// Handle get_plan tool call
    fn handle_get_plan(&self) -> ToolCallResult {
        let content = format!(
            "# Implementation Plan\n\n{}\n\n---\n\n# Review Instructions\n\n{}",
            self.plan_content, self.review_prompt
        );
        ToolCallResult::text(content)
    }

    /// Handle submit_review tool call
    fn handle_submit_review(&mut self, arguments: Value) -> ToolCallResult {
        if self.review_submitted {
            return ToolCallResult::error(
                "Review already submitted. Only one review per session is allowed.".to_string(),
            );
        }

        let review: SubmittedReview = match serde_json::from_value(arguments) {
            Ok(r) => r,
            Err(e) => {
                return ToolCallResult::error(format!(
                    "Invalid review format: {}. Required fields: verdict (APPROVED or NEEDS_REVISION), summary (string).",
                    e
                ));
            }
        };

        // Validate required fields
        if review.summary.trim().is_empty() {
            return ToolCallResult::error("Summary cannot be empty.".to_string());
        }

        // Send the review without blocking the runtime thread
        if let Err(err) = self.review_tx.try_send(review.clone()) {
            let message = match err {
                tokio::sync::mpsc::error::TrySendError::Full(_) => {
                    "Failed to submit review: channel full."
                }
                tokio::sync::mpsc::error::TrySendError::Closed(_) => {
                    "Failed to submit review: channel closed."
                }
            };
            return ToolCallResult::error(message.to_string());
        }

        self.review_submitted = true;

        let verdict_str = match review.verdict {
            super::review_schema::ReviewVerdict::Approved => "APPROVED",
            super::review_schema::ReviewVerdict::NeedsRevision => "NEEDS_REVISION",
        };

        ToolCallResult::text(format!(
            "Review submitted successfully.\nVerdict: {}\nSummary: {}",
            verdict_str, review.summary
        ))
    }
}

impl TryFrom<Value> for ToolCallParams {
    type Error = ();

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        serde_json::from_value(value).map_err(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_server() -> (McpReviewServer, mpsc::Receiver<SubmittedReview>) {
        let (tx, rx) = mpsc::channel(1);
        let server = McpReviewServer::new(
            tx,
            "# Test Plan\n\nSome content".to_string(),
            "Review this plan".to_string(),
        );
        (server, rx)
    }

    #[test]
    fn test_handle_initialize() {
        let (server, _rx) = create_test_server();
        let params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "test-client",
                "version": "1.0.0"
            }
        });

        let result = server.handle_initialize(Some(params));
        assert!(result.is_ok());

        let value = result.unwrap();
        assert_eq!(value["protocolVersion"], "2024-11-05");
        assert!(value["capabilities"]["tools"].is_object());
        assert!(value["capabilities"]["resources"].is_null());
    }

    #[tokio::test]
    async fn test_handle_submit_review_inside_runtime() {
        let (mut server, mut rx) = create_test_server();

        let args = serde_json::json!({
            "verdict": "APPROVED",
            "summary": "The plan looks good and is ready for implementation."
        });

        let result = server.handle_submit_review(args);
        assert!(!result.is_error);

        let review = rx.recv().await.unwrap();
        assert_eq!(review.verdict, super::super::review_schema::ReviewVerdict::Approved);
    }

    #[test]
    fn test_handle_tools_list() {
        let (server, _rx) = create_test_server();
        let result = server.handle_tools_list();
        assert!(result.is_ok());

        let value = result.unwrap();
        let tools = value["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);

        let tool_names: Vec<&str> = tools
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(tool_names.contains(&"get_plan"));
        assert!(tool_names.contains(&"submit_review"));
    }

    #[test]
    fn test_handle_get_plan() {
        let (server, _rx) = create_test_server();
        let result = server.handle_get_plan();
        assert!(!result.is_error);

        if let super::super::protocol::ToolContent::Text { text } = &result.content[0] {
            assert!(text.contains("# Test Plan"));
            assert!(text.contains("Review this plan"));
        } else {
            panic!("Expected text content");
        }
    }

    #[test]
    fn test_handle_submit_review_approved() {
        let (mut server, mut rx) = create_test_server();

        let args = serde_json::json!({
            "verdict": "APPROVED",
            "summary": "The plan looks good and is ready for implementation.",
            "critical_issues": [],
            "recommendations": ["Consider adding more tests"]
        });

        let result = server.handle_submit_review(args);
        assert!(!result.is_error);

        // Check that review was sent through channel
        let review = rx.blocking_recv().unwrap();
        assert_eq!(review.verdict, super::super::review_schema::ReviewVerdict::Approved);
        assert!(review.summary.contains("looks good"));
    }

    #[test]
    fn test_handle_submit_review_needs_revision() {
        let (mut server, mut rx) = create_test_server();

        let args = serde_json::json!({
            "verdict": "NEEDS_REVISION",
            "summary": "The plan has critical issues.",
            "critical_issues": ["Missing error handling", "No retry logic"],
            "recommendations": []
        });

        let result = server.handle_submit_review(args);
        assert!(!result.is_error);

        let review = rx.blocking_recv().unwrap();
        assert_eq!(
            review.verdict,
            super::super::review_schema::ReviewVerdict::NeedsRevision
        );
        assert_eq!(review.critical_issues.len(), 2);
    }

    #[test]
    fn test_handle_submit_review_duplicate() {
        let (mut server, _rx) = create_test_server();

        let args = serde_json::json!({
            "verdict": "APPROVED",
            "summary": "First review"
        });

        let result1 = server.handle_submit_review(args.clone());
        assert!(!result1.is_error);

        // Second submission should fail
        let result2 = server.handle_submit_review(args);
        assert!(result2.is_error);
    }

    #[test]
    fn test_handle_submit_review_empty_summary() {
        let (mut server, _rx) = create_test_server();

        let args = serde_json::json!({
            "verdict": "APPROVED",
            "summary": ""
        });

        let result = server.handle_submit_review(args);
        assert!(result.is_error);
    }

    #[test]
    fn test_handle_submit_review_invalid_verdict() {
        let (mut server, _rx) = create_test_server();

        let args = serde_json::json!({
            "verdict": "INVALID",
            "summary": "Some summary"
        });

        let result = server.handle_submit_review(args);
        assert!(result.is_error);
    }

    #[test]
    fn test_handle_message_parse_error() {
        let (mut server, _rx) = create_test_server();
        let response = server.handle_message("not valid json");
        assert!(response.is_some());
        let resp = response.unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_codes::PARSE_ERROR);
    }

    #[test]
    fn test_handle_message_method_not_found() {
        let (mut server, _rx) = create_test_server();
        let msg = r#"{"jsonrpc":"2.0","id":1,"method":"unknown/method"}"#;
        let response = server.handle_message(msg);
        assert!(response.is_some());
        let resp = response.unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_codes::METHOD_NOT_FOUND);
    }

    #[test]
    fn test_handle_notification_no_response() {
        let (mut server, _rx) = create_test_server();
        // Notifications have no id
        let msg = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let response = server.handle_message(msg);
        assert!(response.is_none());
    }
}
