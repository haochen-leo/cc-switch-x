//! OpenAI Web Search Sidecar
//!
//! When a Codex request carries `{type: "web_search"}` and the target upstream
//! is a third-party provider (Chat/Anthropic), this module provides the ability
//! to execute web searches via OpenAI's Responses API and feed results back to
//! the upstream model as tool results.
//!
//! Architecture:
//! - The hosted `{type: "web_search"}` tool is replaced with a synthetic function tool
//! - When the upstream model calls it, the proxy intercepts and executes the search
//!   via OpenAI's Responses API (the "sidecar")
//! - Results are fed back as tool results and the upstream is re-requested

use crate::proxy::error::ProxyError;
use crate::proxy::http_client;
use crate::proxy::providers::CHATGPT_CODEX_BASE_URL;
use serde_json::{json, Value};
use std::time::Duration;
use tauri::Manager;

/// The synthetic function tool name exposed to upstream models.
pub const WEB_SEARCH_TOOL_NAME: &str = "web_search";

/// Maximum number of search loop iterations per request.
pub const MAX_SEARCH_LOOPS: usize = 3;

/// Sidecar model for executing searches (lightweight to control cost).
const SIDECAR_MODEL: &str = "gpt-4o-mini";

/// Timeout for each sidecar search call.
const SIDECAR_TIMEOUT_SECS: u64 = 60;

/// OpenAI Responses API base URL for API key auth.
const OPENAI_API_BASE_URL: &str = "https://api.openai.com";

/// OpenAI OAuth token endpoint for refreshing access tokens.
const OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

/// Codex CLI client ID for OAuth.
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// Detected web search tool call from an upstream response.
#[derive(Debug, Clone)]
pub struct WebSearchCall {
    pub call_id: String,
    pub query: String,
}

/// Sidecar credentials resolved from available providers.
#[derive(Debug, Clone)]
pub struct SidecarCredentials {
    pub base_url: String,
    pub api_key: String,
    /// Whether this is a CodexOAuth token (needs different headers).
    pub is_oauth: bool,
}

/// Extract the hosted `{type: "web_search"}` tool config from a Responses request body.
/// Returns the full tool object (including any config like search_context_size) if present.
pub fn extract_hosted_web_search(body: &Value) -> Option<Value> {
    let tools = body.get("tools")?.as_array()?;
    for tool in tools {
        if tool.get("type").and_then(|v| v.as_str()) == Some("web_search") {
            return Some(tool.clone());
        }
    }
    None
}

/// Build the synthetic function tool that replaces the hosted web_search tool.
/// This is what the upstream model sees and can call.
pub fn synthetic_web_search_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": WEB_SEARCH_TOOL_NAME,
            "description": "Search the web for current, real-world, or post-training-cutoff information. Returns a concise answer synthesized from live results, with sources. Use it whenever the user asks about recent events, versions, prices, docs, or anything you are unsure is current.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "A single search query — a focused natural-language question or keywords."
                    }
                },
                "required": ["query"]
            }
        }
    })
}

/// Build the Anthropic-format synthetic tool (for Anthropic upstream path).
pub fn synthetic_web_search_tool_anthropic() -> Value {
    json!({
        "name": WEB_SEARCH_TOOL_NAME,
        "description": "Search the web for current, real-world, or post-training-cutoff information. Returns a concise answer synthesized from live results, with sources. Use it whenever the user asks about recent events, versions, prices, docs, or anything you are unsure is current.",
        "input_schema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "A single search query — a focused natural-language question or keywords."
                }
            },
            "required": ["query"]
        }
    })
}

/// Resolve sidecar credentials from available sources.
/// Strategy: iterate through all configured Codex providers that use native
/// Responses protocol, try each one until a search succeeds. Also checks
/// CodexOAuth and OpenAI API keys as fallback.
pub async fn resolve_sidecar_credentials(
    state: &crate::proxy::server::ProxyState,
) -> Option<SidecarCredentials> {
    // 1. Try CodexOAuthManager first (managed accounts with auto-refresh)
    if let Some(app_handle) = &state.app_handle {
        use crate::commands::CodexOAuthState;
        let codex_state = app_handle.state::<CodexOAuthState>();
        let codex_auth = codex_state.0.as_ref();

        if codex_auth.is_authenticated().await {
            match codex_auth.get_valid_token().await {
                Ok(token) => {
                    log::debug!("[WebSearchSidecar] Using CodexOAuth token for sidecar");
                    return Some(SidecarCredentials {
                        base_url: CHATGPT_CODEX_BASE_URL.to_string(),
                        api_key: token,
                        is_oauth: true,
                    });
                }
                Err(e) => {
                    log::warn!("[WebSearchSidecar] CodexOAuth token unavailable: {e}");
                }
            }
        }
    }

    // 2. Look through all configured providers for a usable Responses endpoint
    if let Ok(providers) = state.db.get_all_providers("codex") {
        for (_id, provider) in providers.iter() {
            // Check for ChatGPT OAuth tokens in the provider config
            let oauth_token = provider
                .settings_config
                .get("auth")
                .and_then(|a| a.get("tokens"))
                .and_then(|t| t.get("access_token"))
                .and_then(|v| v.as_str())
                .filter(|t| !t.is_empty() && t.starts_with("eyJ"));

            if let Some(token) = oauth_token {
                // Try refreshing the token
                let refresh_token = provider
                    .settings_config
                    .get("auth")
                    .and_then(|a| a.get("tokens"))
                    .and_then(|t| t.get("refresh_token"))
                    .and_then(|v| v.as_str())
                    .filter(|t| !t.is_empty());

                if let Some(rt) = refresh_token {
                    match refresh_oauth_token(rt).await {
                        Ok(new_token) => {
                            log::debug!(
                                "[WebSearchSidecar] Refreshed OAuth token from provider: {}",
                                provider.name
                            );
                            return Some(SidecarCredentials {
                                base_url: CHATGPT_CODEX_BASE_URL.to_string(),
                                api_key: new_token,
                                is_oauth: true,
                            });
                        }
                        Err(e) => {
                            log::debug!(
                                "[WebSearchSidecar] Token refresh failed for {}: {e}",
                                provider.name
                            );
                        }
                    }
                }
                continue;
            }

            // Check for native Responses providers (not Chat/Anthropic conversion)
            // These can potentially execute web_search server-side
            let api_format = provider
                .meta
                .as_ref()
                .and_then(|m| m.api_format.as_deref())
                .or_else(|| {
                    provider
                        .settings_config
                        .get("api_format")
                        .or_else(|| provider.settings_config.get("apiFormat"))
                        .and_then(|v| v.as_str())
                });

            // Skip providers that use Chat/Anthropic conversion (they can't do hosted search)
            let is_chat_or_anthropic = api_format.is_some_and(|f| {
                matches!(
                    f.trim().to_ascii_lowercase().as_str(),
                    "chat" | "chat_completions" | "chat-completions" | "openai_chat"
                        | "openai-chat" | "openai_chat_completions" | "anthropic"
                )
            });
            if is_chat_or_anthropic {
                continue;
            }

            // Get the provider's base_url and API key
            let base_url = provider
                .settings_config
                .get("base_url")
                .or_else(|| provider.settings_config.get("baseURL"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .or_else(|| {
                    provider
                        .settings_config
                        .get("config")
                        .and_then(|v| v.as_str())
                        .and_then(crate::codex_config::extract_codex_base_url)
                });

            let api_key = provider
                .settings_config
                .get("auth")
                .and_then(|a| a.get("OPENAI_API_KEY"))
                .and_then(|v| v.as_str())
                .filter(|k| !k.is_empty())
                .or_else(|| {
                    provider
                        .settings_config
                        .get("apiKey")
                        .and_then(|v| v.as_str())
                        .filter(|k| !k.is_empty())
                });

            if let (Some(url), Some(key)) = (base_url, api_key) {
                // This is a native Responses provider with an API key — use it
                log::debug!(
                    "[WebSearchSidecar] Using native Responses provider: {} ({})",
                    provider.name,
                    url
                );
                return Some(SidecarCredentials {
                    base_url: url,
                    api_key: key.to_string(),
                    is_oauth: false,
                });
            }
        }
    }

    log::debug!("[WebSearchSidecar] No usable credentials available for sidecar");
    None
}

/// Execute a web search via OpenAI's Responses API.
/// Returns the search result text.
pub async fn execute_sidecar_search(
    creds: &SidecarCredentials,
    query: &str,
    hosted_config: &Value,
) -> Result<String, ProxyError> {
    let client = http_client::get();

    // Build the Responses API request
    let mut web_search_tool = json!({"type": "web_search"});
    // Carry over any config from the original hosted tool (e.g., search_context_size)
    if let Some(obj) = hosted_config.as_object() {
        for (key, value) in obj {
            if key != "type" {
                web_search_tool[key] = value.clone();
            }
        }
    }

    let request_body = json!({
        "model": SIDECAR_MODEL,
        "input": format!("Search the web for: {}. Return a concise, factual answer with key details and sources.", query),
        "tools": [web_search_tool],
        "stream": false
    });

    let url = if creds.is_oauth {
        format!("{}/v1/responses", creds.base_url)
    } else {
        format!("{}/v1/responses", creds.base_url)
    };

    let mut request = client
        .post(&url)
        .header("Content-Type", "application/json")
        .timeout(Duration::from_secs(SIDECAR_TIMEOUT_SECS));

    if creds.is_oauth {
        request = request.header("Authorization", format!("Bearer {}", creds.api_key));
    } else {
        request = request.header("Authorization", format!("Bearer {}", creds.api_key));
    }

    let response = request
        .json(&request_body)
        .send()
        .await
        .map_err(|e| ProxyError::Internal(format!("Sidecar request failed: {e}")))?;

    let status = response.status();
    let body: Value = response
        .json()
        .await
        .map_err(|e| ProxyError::Internal(format!("Sidecar response parse failed: {e}")))?;

    if !status.is_success() {
        let error_msg = body
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("Unknown sidecar error");
        log::warn!(
            "[WebSearchSidecar] Search failed ({}): {}",
            status,
            error_msg
        );
        return Err(ProxyError::Internal(format!(
            "Sidecar search failed: {}",
            error_msg
        )));
    }

    // Extract the text output from the Responses API response
    extract_responses_output_text(&body)
}

/// Extract text content from a Responses API response.
fn extract_responses_output_text(response: &Value) -> Result<String, ProxyError> {
    let output = response
        .get("output")
        .and_then(|o| o.as_array())
        .ok_or_else(|| ProxyError::Internal("Sidecar response missing output".to_string()))?;

    let mut texts = Vec::new();
    for item in output {
        if item.get("type").and_then(|t| t.as_str()) == Some("message") {
            if let Some(content) = item.get("content").and_then(|c| c.as_array()) {
                for part in content {
                    if part.get("type").and_then(|t| t.as_str()) == Some("output_text") {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            texts.push(text.to_string());
                        }
                    }
                }
            }
        }
    }

    if texts.is_empty() {
        return Err(ProxyError::Internal(
            "Sidecar returned no text output".to_string(),
        ));
    }

    Ok(texts.join("\n"))
}

/// Detect web_search tool calls in a Chat Completions response.
/// Returns a list of detected calls with their call_id and query.
pub fn detect_web_search_calls_chat(chat_response: &Value) -> Vec<WebSearchCall> {
    let mut calls = Vec::new();

    let choices = match chat_response.get("choices").and_then(|c| c.as_array()) {
        Some(c) => c,
        None => return calls,
    };

    for choice in choices {
        let message = match choice.get("message") {
            Some(m) => m,
            None => continue,
        };

        let tool_calls = match message.get("tool_calls").and_then(|tc| tc.as_array()) {
            Some(tc) => tc,
            None => continue,
        };

        for tool_call in tool_calls {
            let function = match tool_call.get("function") {
                Some(f) => f,
                None => continue,
            };

            let name = function.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if name != WEB_SEARCH_TOOL_NAME {
                continue;
            }

            let call_id = tool_call
                .get("id")
                .and_then(|id| id.as_str())
                .unwrap_or("")
                .to_string();

            let query = parse_search_query(function.get("arguments").and_then(|a| a.as_str()));

            calls.push(WebSearchCall { call_id, query });
        }
    }

    calls
}

/// Detect web_search tool calls in an Anthropic Messages response.
pub fn detect_web_search_calls_anthropic(response: &Value) -> Vec<WebSearchCall> {
    let mut calls = Vec::new();

    let content = match response.get("content").and_then(|c| c.as_array()) {
        Some(c) => c,
        None => return calls,
    };

    for block in content {
        if block.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
            continue;
        }

        let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if name != WEB_SEARCH_TOOL_NAME {
            continue;
        }

        let call_id = block
            .get("id")
            .and_then(|id| id.as_str())
            .unwrap_or("")
            .to_string();

        let query = block
            .get("input")
            .and_then(|i| i.get("query"))
            .and_then(|q| q.as_str())
            .unwrap_or("")
            .to_string();

        calls.push(WebSearchCall { call_id, query });
    }

    calls
}

/// Parse the search query from tool call arguments JSON string.
fn parse_search_query(args: Option<&str>) -> String {
    let args = match args {
        Some(a) => a,
        None => return String::new(),
    };

    match serde_json::from_str::<Value>(args) {
        Ok(parsed) => parsed
            .get("query")
            .and_then(|q| q.as_str())
            .unwrap_or("")
            .to_string(),
        Err(_) => String::new(),
    }
}

/// Build a Chat Completions follow-up request body with web search results appended.
/// Takes the original chat body and appends the assistant tool call + tool result messages.
pub fn build_chat_followup_with_search_results(
    original_body: &mut Value,
    calls: &[WebSearchCall],
    results: &[String],
) {
    let messages = match original_body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        Some(m) => m,
        None => return,
    };

    // Build assistant message with tool_calls
    let tool_calls: Vec<Value> = calls
        .iter()
        .map(|call| {
            json!({
                "id": call.call_id,
                "type": "function",
                "function": {
                    "name": WEB_SEARCH_TOOL_NAME,
                    "arguments": json!({"query": call.query}).to_string()
                }
            })
        })
        .collect();

    messages.push(json!({
        "role": "assistant",
        "content": null,
        "tool_calls": tool_calls
    }));

    // Add tool result messages
    for (call, result) in calls.iter().zip(results.iter()) {
        messages.push(json!({
            "role": "tool",
            "tool_call_id": call.call_id,
            "content": result
        }));
    }
}

/// Build an Anthropic Messages follow-up request body with web search results appended.
pub fn build_anthropic_followup_with_search_results(
    original_body: &mut Value,
    calls: &[WebSearchCall],
    results: &[String],
) {
    let messages = match original_body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        Some(m) => m,
        None => return,
    };

    // Build assistant message with tool_use blocks
    let tool_use_blocks: Vec<Value> = calls
        .iter()
        .map(|call| {
            json!({
                "type": "tool_use",
                "id": call.call_id,
                "name": WEB_SEARCH_TOOL_NAME,
                "input": {"query": call.query}
            })
        })
        .collect();

    messages.push(json!({
        "role": "assistant",
        "content": tool_use_blocks
    }));

    // Add user message with tool_result blocks
    let tool_result_blocks: Vec<Value> = calls
        .iter()
        .zip(results.iter())
        .map(|(call, result)| {
            json!({
                "type": "tool_result",
                "tool_use_id": call.call_id,
                "content": result
            })
        })
        .collect();

    messages.push(json!({
        "role": "user",
        "content": tool_result_blocks
    }));
}

/// Refresh an OAuth access_token using a refresh_token.
/// Calls the OpenAI OAuth token endpoint.
async fn refresh_oauth_token(refresh_token: &str) -> Result<String, ProxyError> {
    let client = http_client::get();

    let response = client
        .post(OAUTH_TOKEN_URL)
        .timeout(Duration::from_secs(30))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CODEX_CLIENT_ID),
            ("scope", "openid profile email"),
        ])
        .send()
        .await
        .map_err(|e| ProxyError::Internal(format!("Token refresh request failed: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(ProxyError::Internal(format!(
            "Token refresh failed ({}): {}",
            status,
            text.chars().take(100).collect::<String>()
        )));
    }

    let body: Value = response
        .json()
        .await
        .map_err(|e| ProxyError::Internal(format!("Token refresh parse failed: {e}")))?;

    body.get("access_token")
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .ok_or_else(|| ProxyError::Internal("Token refresh response missing access_token".to_string()))
}
