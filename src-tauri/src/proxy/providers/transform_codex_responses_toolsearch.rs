//! Codex `tool_search` (deferred tool discovery) bridging for native Responses
//! upstreams.
//!
//! Codex 0.142+ loads plugin/MCP tools lazily: the request declares a private
//! `{"type":"tool_search","execution":"client"}` tool, the model emits a
//! `tool_search_call` item, the Codex client executes it locally and appends a
//! `tool_search_output` item carrying the discovered tool schemas (often as
//! `namespace` declarations). The discovered tools are materialized into the
//! model's callable set by the *OpenAI backend* on the next call — the client
//! never adds them to `tools` itself.
//!
//! Third-party Responses-compatible upstreams (e.g. DashScope) do not
//! implement that backend role: the model never sees a callable `tool_search`
//! and discovered tools stay invisible, so plugin tools like `node_repl` are
//! unreachable on the native passthrough. The Chat transform already bridges
//! this contract; this module provides the equivalent for the native path:
//!
//! - **Request**: rewrite the `tool_search` declaration into a plain
//!   `function` tool so the model can initiate discovery
//!   ([`materialize_tool_search_declaration`]), and lift tools discovered in
//!   replayed `tool_search_output` items into top-level `tools` (flattening
//!   `namespace` declarations to the flat `<namespace>__<child>` names the
//!   upstream can call), converting the carrier history items into
//!   standard `function_call` / `function_call_output` items
//!   ([`promote_tool_search_output_tools`]).
//! - **Response**: rewrite a `function_call` named `tool_search` back into a
//!   `tool_search_call` item (`execution: "client"`) so the Codex client
//!   executes it locally — mirroring the Chat path, whose streamed
//!   `tool_search_call` items the client already accepts
//!   ([`rewrite_tool_search_function_calls`] /
//!   [`create_tool_search_call_sse_stream`]).
//!
//! Promoted namespace children DO need name restore on the response path: the
//! Codex client registers discovered tools under their `{namespace, name}`
//! pair (`ToolRegistry::tool` looks up `ToolName { namespace, name }`), so a
//! bare flat-name call like `mcp__node_repl__js` matches nothing and the
//! client answers `unsupported call: …`. The handler builds the flat-name →
//! `{namespace, name}` map from the same replayed `tool_search_output` items
//! via [`tool_search_namespace_restore_map`] and reuses the namespace
//! module's response restore to translate the model's calls back.
use std::collections::{HashMap, HashSet};

use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use serde_json::{json, Value};

use super::transform_codex_chat::flatten_namespace_tool_name;
use super::transform_codex_responses_namespace::{
    collect_tool_search_output_namespace_restore, NamespacedName,
};
use crate::proxy::sse::{append_utf8_safe, strip_sse_field, take_sse_block};

const TOOL_SEARCH_NAME: &str = "tool_search";
const TOOL_SEARCH_FALLBACK_DESCRIPTION: &str =
    "Search and load Codex tools, plugins, connectors, and MCP namespaces for the current task.";

/// Build the flat-name → `{namespace, name}` restore map for tools the bridge
/// promoted out of replayed `tool_search_output` carriers. Derived from the
/// client request (the same place the carrier history lives), so no state has
/// to be threaded between forwarder and response handler.
pub(crate) fn tool_search_namespace_restore_map(
    request_body: &Value,
) -> HashMap<String, NamespacedName> {
    let mut map = HashMap::new();
    if let Some(input) = request_body.get("input") {
        collect_tool_search_output_namespace_restore(input, &mut map);
    }
    map
}

/// Rewrite `{"type":"tool_search",…}` declarations in `tools` into plain
/// `function` tools, preserving the client-supplied description and parameter
/// schema (which enumerate the deferred sources). Returns whether anything
/// changed.
pub(crate) fn materialize_tool_search_declaration(body: &mut Value) -> bool {
    let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) else {
        return false;
    };
    let mut changed = false;
    for tool in tools.iter_mut() {
        if tool.get("type").and_then(Value::as_str) != Some("tool_search") {
            continue;
        }
        let description = tool
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or(TOOL_SEARCH_FALLBACK_DESCRIPTION)
            .to_string();
        let parameters = tool.get("parameters").cloned().unwrap_or_else(|| {
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query for tools or connectors to load."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of tool groups to return."
                    }
                },
                "required": ["query"]
            })
        });
        *tool = json!({
            "type": "function",
            "name": TOOL_SEARCH_NAME,
            "description": description,
            "parameters": parameters,
            "strict": false
        });
        changed = true;
    }
    changed
}

/// Lift tools discovered via replayed `tool_search_output` items into
/// top-level `tools`, then convert the `tool_search_call` /
/// `tool_search_output` carrier items in `input` into standard
/// `function_call` / `function_call_output` items so strict upstreams never
/// see the private item types. Returns whether anything changed.
pub(crate) fn promote_tool_search_output_tools(body: &mut Value) -> bool {
    let has_carrier = body
        .get("input")
        .and_then(Value::as_array)
        .map(|items| items.iter().any(is_tool_search_carrier))
        .unwrap_or(false);
    if !has_carrier {
        return false;
    }

    // Existing top-level tool names seed the dedup set.
    let mut seen: HashSet<String> = HashSet::new();
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        for tool in tools {
            if let Some(name) = tool.get("name").and_then(Value::as_str) {
                let name = name.trim();
                if !name.is_empty() {
                    seen.insert(name.to_string());
                }
            }
        }
    }

    // First pass (immutable): collect the discovered tools.
    let mut promoted: Vec<Value> = Vec::new();
    if let Some(input) = body.get("input").and_then(Value::as_array) {
        for item in input {
            if item.get("type").and_then(Value::as_str) != Some("tool_search_output") {
                continue;
            }
            if let Some(tools) = item.get("tools").and_then(Value::as_array) {
                for tool in tools {
                    lift_discovered_tool(tool, &mut seen, &mut promoted);
                }
            }
        }
    }

    // Second pass (mutable): rewrite the carrier history items.
    let mut changed = false;
    if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
        for item in input.iter_mut() {
            changed |= rewrite_tool_search_carrier(item);
        }
    }

    if !promoted.is_empty() {
        if let Some(obj) = body.as_object_mut() {
            let tools = obj.entry("tools").or_insert_with(|| json!([]));
            if let Some(arr) = tools.as_array_mut() {
                arr.extend(promoted);
                changed = true;
            }
        }
    }
    changed
}

fn is_tool_search_carrier(item: &Value) -> bool {
    matches!(
        item.get("type").and_then(Value::as_str),
        Some("tool_search_call") | Some("tool_search_output")
    )
}

/// Add a discovered tool to `out` (de-duplicated by flat name). `namespace`
/// declarations are flattened child-by-child into top-level function tools
/// using the same naming as the Chat/flatten paths; the response handler maps
/// the flat names back to `{namespace, name}` so the client can dispatch the
/// calls (see [`tool_search_namespace_restore_map`]).
fn lift_discovered_tool(tool: &Value, seen: &mut HashSet<String>, out: &mut Vec<Value>) {
    match tool.get("type").and_then(Value::as_str) {
        Some("function") => {
            let Some(name) = tool.get("name").and_then(Value::as_str).map(str::trim) else {
                return;
            };
            if name.is_empty() || !seen.insert(name.to_string()) {
                return;
            }
            out.push(tool.clone());
        }
        Some("namespace") => {
            let Some(namespace) = tool.get("name").and_then(Value::as_str).map(str::trim) else {
                return;
            };
            if namespace.is_empty() {
                return;
            }
            let children = tool
                .get("tools")
                .or_else(|| tool.get("children"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for child in children {
                if child.get("type").and_then(Value::as_str) != Some("function") {
                    continue;
                }
                let Some(name) = child.get("name").and_then(Value::as_str).map(str::trim) else {
                    continue;
                };
                if name.is_empty() {
                    continue;
                }
                let flat = flatten_namespace_tool_name(namespace, name);
                if !seen.insert(flat.clone()) {
                    continue;
                }
                let mut lifted = child.clone();
                if let Some(obj) = lifted.as_object_mut() {
                    obj.insert("name".to_string(), json!(flat));
                }
                out.push(lifted);
            }
        }
        _ => {}
    }
}

/// Convert one carrier history item into its standard Responses equivalent.
fn rewrite_tool_search_carrier(item: &mut Value) -> bool {
    let Some(typ) = item.get("type").and_then(Value::as_str) else {
        return false;
    };
    let replacement = match typ {
        "tool_search_call" => {
            let call_id = item
                .get("call_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            // The carrier stores arguments as an object; a function_call wants
            // a serialized JSON string.
            let arguments = match item.get("arguments") {
                Some(Value::String(raw)) => raw.clone(),
                Some(other) => serde_json::to_string(other).unwrap_or_else(|_| "{}".to_string()),
                None => "{}".to_string(),
            };
            let status = item
                .get("status")
                .cloned()
                .unwrap_or_else(|| json!("completed"));
            json!({
                "type": "function_call",
                "name": TOOL_SEARCH_NAME,
                "call_id": call_id,
                "arguments": arguments,
                "status": status
            })
        }
        "tool_search_output" => {
            let call_id = item
                .get("call_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            // Keep the whole original item as the tool result payload: the
            // discovered schemas are already materialized into `tools`, this
            // record only needs to keep the history narrative intact.
            let output = serde_json::to_string(item).unwrap_or_else(|_| "{}".to_string());
            json!({
                "type": "function_call_output",
                "call_id": call_id,
                "output": output
            })
        }
        _ => return false,
    };
    *item = replacement;
    true
}

/// Rewrite any `function_call` item named `tool_search` into the
/// `tool_search_call` shape the Codex client executes locally. Works on a
/// full (non-streaming) response body or a single parsed SSE event; returns
/// whether anything changed. `id`/`call_id`/`status` are preserved so the
/// surrounding `function_call_arguments.*` events stay correlated.
pub(crate) fn rewrite_tool_search_function_calls(value: &mut Value) -> bool {
    let mut changed = false;
    match value {
        Value::Array(items) => {
            for item in items {
                changed |= rewrite_tool_search_function_calls(item);
            }
        }
        Value::Object(obj) => {
            let is_tool_search_call = obj.get("type").and_then(Value::as_str)
                == Some("function_call")
                && obj.get("name").and_then(Value::as_str) == Some(TOOL_SEARCH_NAME);
            if is_tool_search_call {
                let parsed_arguments = obj
                    .get("arguments")
                    .and_then(Value::as_str)
                    .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                    .filter(|parsed| parsed.is_object())
                    .unwrap_or_else(|| json!({}));
                obj.insert("type".to_string(), json!("tool_search_call"));
                obj.insert("execution".to_string(), json!("client"));
                obj.insert("arguments".to_string(), parsed_arguments);
                obj.remove("name");
                changed = true;
            }
            for child in obj.values_mut() {
                changed |= rewrite_tool_search_function_calls(child);
            }
        }
        _ => {}
    }
    changed
}

/// Wrap a native Responses SSE byte stream, rewriting `function_call` items
/// named `tool_search` into `tool_search_call` items inside each event.
/// Events without an affected call pass through verbatim (only the block
/// delimiter is normalized to `\n\n`).
pub(crate) fn create_tool_search_call_sse_stream<E>(
    stream: impl Stream<Item = Result<Bytes, E>> + Send + 'static,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send
where
    E: std::error::Error + Send + 'static,
{
    async_stream::stream! {
        let mut buffer = String::new();
        let mut utf8_remainder: Vec<u8> = Vec::new();

        tokio::pin!(stream);

        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes);
                    while let Some(block) = take_sse_block(&mut buffer) {
                        if block.trim().is_empty() {
                            continue;
                        }
                        yield Ok(rewrite_sse_block(&block));
                    }
                }
                Err(e) => {
                    yield Err(std::io::Error::other(e.to_string()));
                    return;
                }
            }
        }

        // Flush any trailing partial block (streams normally end on a
        // delimiter, but be defensive so no bytes are dropped).
        if !utf8_remainder.is_empty() {
            buffer.push_str(&String::from_utf8_lossy(&utf8_remainder));
        }
        let tail = std::mem::take(&mut buffer);
        if !tail.trim().is_empty() {
            yield Ok(rewrite_sse_block(&tail));
        }
    }
}

/// Rewrite one SSE block. When the block's `data:` JSON carries a tool_search
/// function call, re-serialize just that line; otherwise the original block
/// text is preserved and only the `\n\n` delimiter re-appended.
fn rewrite_sse_block(block: &str) -> Bytes {
    let mut event_name: Option<&str> = None;
    let mut data_parts: Vec<&str> = Vec::new();
    for line in block.lines() {
        if let Some(event) = strip_sse_field(line, "event") {
            event_name = Some(event.trim());
        }
        if let Some(data) = strip_sse_field(line, "data") {
            data_parts.push(data);
        }
    }

    if data_parts.is_empty() {
        return Bytes::from(format!("{block}\n\n"));
    }

    let data = data_parts.join("\n");
    if data.trim() == "[DONE]" {
        return Bytes::from(format!("{block}\n\n"));
    }

    let mut event: Value = match serde_json::from_str(&data) {
        Ok(value) => value,
        // Non-JSON data (shouldn't happen on the Responses wire): pass through.
        Err(_) => return Bytes::from(format!("{block}\n\n")),
    };

    if !rewrite_tool_search_function_calls(&mut event) {
        return Bytes::from(format!("{block}\n\n"));
    }

    let rewritten = serde_json::to_string(&event).unwrap_or(data);
    let mut out = String::new();
    if let Some(name) = event_name {
        out.push_str("event: ");
        out.push_str(name);
        out.push('\n');
    }
    out.push_str("data: ");
    out.push_str(&rewritten);
    out.push_str("\n\n");
    Bytes::from(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;

    #[test]
    fn materializes_tool_search_declaration_preserving_description_and_schema() {
        let mut body = json!({
            "tools": [
                {"type": "function", "name": "exec_command"},
                {
                    "type": "tool_search",
                    "description": "# Tool discovery\n…node_repl…",
                    "execution": "client",
                    "parameters": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}
                },
                {"type": "web_search"}
            ]
        });
        assert!(materialize_tool_search_declaration(&mut body));
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools[0]["name"], "exec_command");
        assert_eq!(tools[1]["type"], "function");
        assert_eq!(tools[1]["name"], "tool_search");
        assert_eq!(tools[1]["description"], "# Tool discovery\n…node_repl…");
        assert_eq!(tools[1]["parameters"]["required"][0], "query");
        assert!(tools[1].get("execution").is_none());
        assert_eq!(tools[2]["type"], "web_search");
        // Idempotent once no declaration remains.
        assert!(!materialize_tool_search_declaration(&mut body));
    }

    #[test]
    fn promotes_discovered_namespace_tools_and_rewrites_carriers() {
        let mut body = json!({
            "tools": [{"type": "function", "name": "exec_command"}],
            "input": [
                {"type": "message", "role": "user", "content": []},
                {
                    "type": "tool_search_call",
                    "id": "tsc_1",
                    "call_id": "call_1",
                    "status": "completed",
                    "execution": "client",
                    "arguments": {"query": "node_repl js"}
                },
                {
                    "type": "tool_search_output",
                    "id": "tso_1",
                    "call_id": "call_1",
                    "status": "completed",
                    "execution": "client",
                    "tools": [
                        {
                            "type": "namespace",
                            "name": "mcp__node_repl",
                            "description": "node repl",
                            "tools": [
                                {"type": "function", "name": "js_reset", "parameters": {}},
                                {"type": "function", "name": "js", "parameters": {"type": "object"}}
                            ]
                        },
                        {"type": "function", "name": "exec_command"}
                    ]
                }
            ]
        });
        assert!(promote_tool_search_output_tools(&mut body));

        let names: Vec<&str> = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect();
        assert_eq!(
            names,
            vec![
                "exec_command",
                "mcp__node_repl__js_reset",
                "mcp__node_repl__js"
            ],
            "namespace children are lifted flat; the pre-existing exec_command is deduped"
        );

        let input = body["input"].as_array().unwrap();
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["name"], "tool_search");
        assert_eq!(input[1]["call_id"], "call_1");
        let args: Value = serde_json::from_str(input[1]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["query"], "node_repl js");
        assert_eq!(input[2]["type"], "function_call_output");
        assert_eq!(input[2]["call_id"], "call_1");
        let output: Value = serde_json::from_str(input[2]["output"].as_str().unwrap()).unwrap();
        assert_eq!(output["type"], "tool_search_output");

        // Second call is a no-op (carriers already converted).
        assert!(!promote_tool_search_output_tools(&mut body));
    }

    #[test]
    fn rewrites_tool_search_function_call_items() {
        let mut value = json!({
            "type": "response.output_item.done",
            "item": {
                "type": "function_call",
                "id": "fc_1",
                "call_id": "call_9",
                "status": "completed",
                "name": "tool_search",
                "arguments": "{\"query\":\"node_repl js\"}"
            }
        });
        assert!(rewrite_tool_search_function_calls(&mut value));
        let item = &value["item"];
        assert_eq!(item["type"], "tool_search_call");
        assert_eq!(item["execution"], "client");
        assert_eq!(item["arguments"]["query"], "node_repl js");
        assert_eq!(item["id"], "fc_1");
        assert_eq!(item["call_id"], "call_9");
        assert!(item.get("name").is_none());

        // Unrelated function calls stay untouched.
        let mut other = json!({"type": "function_call", "name": "exec_command", "arguments": "{}"});
        assert!(!rewrite_tool_search_function_calls(&mut other));
        assert_eq!(other["type"], "function_call");
    }

    #[tokio::test]
    async fn rewrites_streamed_tool_search_call_items() {
        let input = concat!(
            "event: response.output_item.done\n",
            "data:{\"type\":\"response.output_item.done\",\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"tool_search\",\"arguments\":\"{\\\"query\\\":\\\"chrome\\\"}\",\"status\":\"completed\"}}\n\n",
            "event: response.output_item.done\n",
            "data:{\"type\":\"response.output_item.done\",\"item\":{\"id\":\"fc_2\",\"type\":\"function_call\",\"call_id\":\"call_2\",\"name\":\"exec_command\",\"arguments\":\"{}\"}}\n\n"
        );
        let stream = stream::iter(vec![Ok::<Bytes, std::io::Error>(Bytes::from(input))]);
        let mut output = String::new();
        let mut rewritten = Box::pin(create_tool_search_call_sse_stream(stream));
        while let Some(chunk) = rewritten.next().await {
            output.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
        }
        assert!(output.contains("\"type\":\"tool_search_call\""));
        assert!(output.contains("\"execution\":\"client\""));
        assert!(output.contains("\"query\":\"chrome\""));
        // The exec_command block passes through untouched.
        assert!(output.contains("\"name\":\"exec_command\""));
        assert!(!output.contains("\"name\":\"tool_search\""));
    }

    #[test]
    fn restore_map_reads_tool_search_output_carriers() {
        let body = json!({
            "tools": [{"type": "function", "name": "exec_command"}],
            "input": [
                {
                    "type": "tool_search_output",
                    "call_id": "call_1",
                    "tools": [
                        {
                            "type": "namespace",
                            "name": "mcp__node_repl",
                            "tools": [
                                {"type": "function", "name": "js"},
                                {"type": "function", "name": "js_reset"}
                            ]
                        },
                        {"type": "function", "name": "plain_discovered"}
                    ]
                }
            ]
        });
        let map = tool_search_namespace_restore_map(&body);
        // Namespace children are keyed by their flat name; plain discovered
        // functions keep their identity and need no restore.
        assert_eq!(map.len(), 2);
        let js = &map["mcp__node_repl__js"];
        assert_eq!(js.namespace, "mcp__node_repl");
        assert_eq!(js.name, "js");
        assert_eq!(map["mcp__node_repl__js_reset"].name, "js_reset");
        assert!(!map.contains_key("plain_discovered"));

        // No carriers → empty map (and the response path stays a no-op).
        let empty = tool_search_namespace_restore_map(&json!({"input": []}));
        assert!(empty.is_empty());
    }

    #[test]
    fn restored_flat_call_matches_client_registry_shape() {
        use super::super::transform_codex_responses_namespace::restore_response_namespaces;

        let body = json!({
            "input": [{
                "type": "tool_search_output",
                "tools": [{
                    "type": "namespace",
                    "name": "mcp__node_repl",
                    "tools": [{"type": "function", "name": "js"}]
                }]
            }]
        });
        let map = tool_search_namespace_restore_map(&body);

        let mut response = json!({
            "output": [{
                "type": "function_call",
                "id": "fc_1",
                "call_id": "call_1",
                "name": "mcp__node_repl__js",
                "arguments": "{\"code\":\"1\"}"
            }]
        });
        assert!(restore_response_namespaces(&mut response, &map));
        let item = &response["output"][0];
        // The Codex client registry keys tools by {namespace, name} — exactly
        // what the restore emits.
        assert_eq!(item["name"], "js");
        assert_eq!(item["namespace"], "mcp__node_repl");
        assert_eq!(item["call_id"], "call_1");

        // Unknown flat names stay untouched.
        let mut other = json!({"type": "function_call", "name": "exec_command", "arguments": "{}"});
        assert!(!restore_response_namespaces(&mut other, &map));
        assert_eq!(other["name"], "exec_command");
        assert!(other.get("namespace").is_none());
    }

    #[tokio::test]
    async fn streamed_restore_composes_with_tool_search_rewrite() {
        use super::super::transform_codex_responses_namespace::create_namespace_restore_sse_stream;

        let body = json!({
            "input": [{
                "type": "tool_search_output",
                "tools": [{
                    "type": "namespace",
                    "name": "mcp__node_repl",
                    "tools": [{"type": "function", "name": "js"}]
                }]
            }]
        });
        let map = tool_search_namespace_restore_map(&body);

        let input = concat!(
            "event: response.output_item.done\n",
            "data:{\"type\":\"response.output_item.done\",\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"tool_search\",\"arguments\":\"{\\\"query\\\":\\\"node_repl\\\"}\"}}\n\n",
            "event: response.output_item.done\n",
            "data:{\"type\":\"response.output_item.done\",\"item\":{\"id\":\"fc_2\",\"type\":\"function_call\",\"call_id\":\"call_2\",\"name\":\"mcp__node_repl__js\",\"arguments\":\"{\\\"code\\\":\\\"1\\\"}\"}}\n\n"
        );
        let stream = stream::iter(vec![Ok::<Bytes, std::io::Error>(Bytes::from(input))]);
        let stream = create_tool_search_call_sse_stream(stream);
        let mut restored = Box::pin(create_namespace_restore_sse_stream(stream, map));
        let mut output = String::new();
        while let Some(chunk) = restored.next().await {
            output.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
        }
        assert!(output.contains("\"type\":\"tool_search_call\""));
        assert!(output.contains("\"name\":\"js\""));
        assert!(output.contains("\"namespace\":\"mcp__node_repl\""));
        assert!(!output.contains("mcp__node_repl__js"));
    }
}
