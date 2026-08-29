//! Native Responses compatibility for Codex freeform `apply_patch` calls.
//!
//! Codex exposes `apply_patch` as a Responses custom/freeform tool. Many native
//! Responses-compatible gateways support standard function tools but reject the
//! `custom` tool type. The request bridge below converts only `apply_patch`
//! custom declarations/history into a single-string function contract; the
//! response bridge restores function calls to Codex custom-tool items.
//!
//! Some gateways additionally wrap the patch string in JSON. The final
//! sanitizer unwraps those shapes only when it can recover a valid patch
//! envelope.

use std::collections::{HashMap, HashSet};

use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use serde_json::{json, Value};

use crate::proxy::sse::{append_utf8_safe, strip_sse_field, take_sse_block};

const PATCH_BEGIN: &str = "*** Begin Patch";
const APPLY_PATCH_TOOL_NAME: &str = "apply_patch";
const APPLY_PATCH_FUNCTION_INPUT_FIELD: &str = "input";
const APPLY_PATCH_FUNCTION_INPUT_DESCRIPTION: &str =
    "Raw apply_patch patch text. Preserve the patch envelope and formatting exactly.";
const PATCH_FIELD_PRIORITY: &[&str] = &[
    "input",
    "patch",
    "data",
    "content",
    "cmd",
    "command",
    "parameters",
];
const MAX_UNWRAP_DEPTH: usize = 8;

pub(crate) fn bridge_request_apply_patch_custom_to_function(value: &mut Value) -> bool {
    let mut apply_patch_call_ids = HashSet::new();
    collect_apply_patch_custom_call_ids(value, &mut apply_patch_call_ids);
    bridge_request_value(value, &apply_patch_call_ids)
}

fn collect_apply_patch_custom_call_ids(value: &Value, call_ids: &mut HashSet<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_apply_patch_custom_call_ids(item, call_ids);
            }
        }
        Value::Object(obj) => {
            if obj.get("type").and_then(Value::as_str) == Some("custom_tool_call")
                && obj.get("name").and_then(Value::as_str) == Some(APPLY_PATCH_TOOL_NAME)
            {
                if let Some(call_id) = obj.get("call_id").and_then(Value::as_str) {
                    call_ids.insert(call_id.to_string());
                }
            }
            for child in obj.values() {
                collect_apply_patch_custom_call_ids(child, call_ids);
            }
        }
        _ => {}
    }
}

fn bridge_request_value(value: &mut Value, apply_patch_call_ids: &HashSet<String>) -> bool {
    let mut changed = false;
    match value {
        Value::Array(items) => {
            for item in items {
                changed |= bridge_request_value(item, apply_patch_call_ids);
            }
        }
        Value::Object(obj) => {
            let item_type = obj
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            match item_type.as_str() {
                "custom_tool_call"
                    if obj.get("name").and_then(Value::as_str) == Some(APPLY_PATCH_TOOL_NAME) =>
                {
                    let input = obj
                        .remove("input")
                        .map(custom_input_value_to_string)
                        .unwrap_or_default();
                    obj.insert("type".to_string(), json!("function_call"));
                    obj.insert(
                        "arguments".to_string(),
                        json!(function_arguments_from_custom_input(&input)),
                    );
                    changed = true;
                }
                "custom_tool_call_output"
                    if obj
                        .get("call_id")
                        .and_then(Value::as_str)
                        .is_some_and(|call_id| apply_patch_call_ids.contains(call_id)) =>
                {
                    obj.insert("type".to_string(), json!("function_call_output"));
                    changed = true;
                }
                _ => {}
            }

            if let Some(tool_choice) = obj.get_mut("tool_choice").and_then(Value::as_object_mut) {
                if tool_choice.get("type").and_then(Value::as_str) == Some("custom")
                    && tool_choice.get("name").and_then(Value::as_str)
                        == Some(APPLY_PATCH_TOOL_NAME)
                {
                    tool_choice.insert("type".to_string(), json!("function"));
                    changed = true;
                }
            }

            for key in ["tools", "additional_tools"] {
                if let Some(tools) = obj.get_mut(key).and_then(Value::as_array_mut) {
                    for tool in tools {
                        changed |= bridge_apply_patch_tool_definition(tool);
                    }
                }
            }

            for child in obj.values_mut() {
                changed |= bridge_request_value(child, apply_patch_call_ids);
            }
        }
        _ => {}
    }
    changed
}

fn bridge_apply_patch_tool_definition(tool: &mut Value) -> bool {
    let is_apply_patch = match tool {
        Value::String(name) => name == APPLY_PATCH_TOOL_NAME,
        Value::Object(obj) => {
            obj.get("type").and_then(Value::as_str) == Some("custom")
                && obj.get("name").and_then(Value::as_str) == Some(APPLY_PATCH_TOOL_NAME)
        }
        _ => false,
    };
    if !is_apply_patch {
        return false;
    }

    let original_description = tool
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .unwrap_or("Apply a patch to files in the workspace.");
    *tool = json!({
        "type": "function",
        "name": APPLY_PATCH_TOOL_NAME,
        "description": original_description,
        "parameters": {
            "type": "object",
            "properties": {
                APPLY_PATCH_FUNCTION_INPUT_FIELD: {
                    "type": "string",
                    "description": APPLY_PATCH_FUNCTION_INPUT_DESCRIPTION
                }
            },
            "required": [APPLY_PATCH_FUNCTION_INPUT_FIELD]
        }
    });
    true
}

fn custom_input_value_to_string(value: Value) -> String {
    match value {
        Value::String(text) => text,
        other => serde_json::to_string(&other).unwrap_or_default(),
    }
}

fn function_arguments_from_custom_input(input: &str) -> String {
    serde_json::to_string(&json!({ APPLY_PATCH_FUNCTION_INPUT_FIELD: input })).unwrap_or_default()
}

pub(crate) fn restore_response_apply_patch_function_calls(value: &mut Value) -> bool {
    let mut apply_patch_call_ids = HashSet::new();
    collect_apply_patch_function_call_ids(value, &mut apply_patch_call_ids);
    restore_response_value(value, &apply_patch_call_ids)
}

fn collect_apply_patch_function_call_ids(value: &Value, call_ids: &mut HashSet<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_apply_patch_function_call_ids(item, call_ids);
            }
        }
        Value::Object(obj) => {
            if obj.get("type").and_then(Value::as_str) == Some("function_call")
                && obj.get("name").and_then(Value::as_str) == Some(APPLY_PATCH_TOOL_NAME)
            {
                if let Some(call_id) = obj.get("call_id").and_then(Value::as_str) {
                    call_ids.insert(call_id.to_string());
                }
            }
            for child in obj.values() {
                collect_apply_patch_function_call_ids(child, call_ids);
            }
        }
        _ => {}
    }
}

fn restore_response_value(value: &mut Value, apply_patch_call_ids: &HashSet<String>) -> bool {
    let mut changed = false;
    match value {
        Value::Array(items) => {
            for item in items {
                changed |= restore_response_value(item, apply_patch_call_ids);
            }
        }
        Value::Object(obj) => {
            let item_type = obj
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            match item_type.as_str() {
                "function_call"
                    if obj.get("name").and_then(Value::as_str) == Some(APPLY_PATCH_TOOL_NAME) =>
                {
                    let arguments = obj
                        .remove("arguments")
                        .map(custom_input_value_to_string)
                        .unwrap_or_default();
                    obj.insert("type".to_string(), json!("custom_tool_call"));
                    obj.insert(
                        "input".to_string(),
                        json!(custom_input_from_function_arguments(&arguments)),
                    );
                    changed = true;
                }
                "function_call_output"
                    if obj
                        .get("call_id")
                        .and_then(Value::as_str)
                        .is_some_and(|call_id| apply_patch_call_ids.contains(call_id)) =>
                {
                    obj.insert("type".to_string(), json!("custom_tool_call_output"));
                    changed = true;
                }
                _ => {}
            }

            for child in obj.values_mut() {
                changed |= restore_response_value(child, apply_patch_call_ids);
            }
        }
        _ => {}
    }
    changed
}

fn custom_input_from_function_arguments(arguments: &str) -> String {
    if arguments.trim().is_empty() {
        return String::new();
    }
    if let Some(patch) = normalized_patch_text(arguments) {
        return patch;
    }
    match serde_json::from_str::<Value>(arguments) {
        Ok(Value::Object(obj)) => {
            for key in PATCH_FIELD_PRIORITY {
                if let Some(Value::String(text)) = obj.get(*key) {
                    return normalized_patch_text(text).unwrap_or_else(|| text.to_string());
                }
            }
            arguments.to_string()
        }
        _ => arguments.to_string(),
    }
}

#[derive(Debug, Default)]
struct ApplyPatchFunctionStreamState {
    item_ids: HashSet<String>,
    buffered_arguments: HashMap<String, String>,
}

pub(crate) fn create_apply_patch_function_restore_sse_stream<E>(
    stream: impl Stream<Item = Result<Bytes, E>> + Send + 'static,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send
where
    E: std::error::Error + Send + 'static,
{
    async_stream::stream! {
        let mut buffer = String::new();
        let mut utf8_remainder: Vec<u8> = Vec::new();
        let mut state = ApplyPatchFunctionStreamState::default();

        tokio::pin!(stream);

        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes);
                    while let Some(block) = take_sse_block(&mut buffer) {
                        if block.trim().is_empty() {
                            continue;
                        }
                        for bytes in restore_function_sse_block(&block, &mut state) {
                            yield Ok(bytes);
                        }
                    }
                }
                Err(e) => {
                    yield Err(std::io::Error::other(e.to_string()));
                    return;
                }
            }
        }

        if !utf8_remainder.is_empty() {
            buffer.push_str(&String::from_utf8_lossy(&utf8_remainder));
        }
        let tail = std::mem::take(&mut buffer);
        if !tail.trim().is_empty() {
            for bytes in restore_function_sse_block(&tail, &mut state) {
                yield Ok(bytes);
            }
        }
    }
}

fn restore_function_sse_block(
    block: &str,
    state: &mut ApplyPatchFunctionStreamState,
) -> Vec<Bytes> {
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
        return vec![Bytes::from(format!("{block}\n\n"))];
    }

    let data = data_parts.join("\n");
    if data.trim() == "[DONE]" {
        return vec![Bytes::from(format!("{block}\n\n"))];
    }

    let mut event: Value = match serde_json::from_str(&data) {
        Ok(value) => value,
        Err(_) => return vec![Bytes::from(format!("{block}\n\n"))],
    };
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    if matches!(
        event_type.as_str(),
        "response.output_item.added" | "response.output_item.done"
    ) {
        if let Some(item) = event.get("item").and_then(Value::as_object) {
            let is_apply_patch = item.get("type").and_then(Value::as_str) == Some("function_call")
                && item.get("name").and_then(Value::as_str) == Some(APPLY_PATCH_TOOL_NAME);
            if is_apply_patch {
                if let Some(item_id) = item.get("id").and_then(Value::as_str) {
                    state.item_ids.insert(item_id.to_string());
                }
            }
        }
    }

    let item_id = event
        .get("item_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    if event_type == "response.function_call_arguments.delta" {
        if let Some(item_id) = item_id.as_deref() {
            if state.item_ids.contains(item_id) {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    state
                        .buffered_arguments
                        .entry(item_id.to_string())
                        .or_default()
                        .push_str(delta);
                }
                return Vec::new();
            }
        }
    }

    if event_type == "response.function_call_arguments.done" {
        if let Some(item_id) = item_id.as_deref() {
            if state.item_ids.contains(item_id) {
                let buffered = state.buffered_arguments.remove(item_id);
                let had_buffer = buffered.is_some();
                let arguments = event
                    .get("arguments")
                    .and_then(Value::as_str)
                    .filter(|arguments| !arguments.is_empty())
                    .map(str::to_string)
                    .or(buffered)
                    .unwrap_or_default();
                let input = custom_input_from_function_arguments(&arguments);
                let output_index = event.get("output_index").cloned().unwrap_or(json!(0));
                event["type"] = json!("response.custom_tool_call_input.done");
                event
                    .as_object_mut()
                    .expect("Responses SSE event must be an object")
                    .remove("arguments");
                event["input"] = json!(input.clone());

                let done = sse_event_bytes("response.custom_tool_call_input.done", event);
                if had_buffer {
                    return vec![
                        sse_event_bytes(
                            "response.custom_tool_call_input.delta",
                            json!({
                                "type": "response.custom_tool_call_input.delta",
                                "item_id": item_id,
                                "output_index": output_index,
                                "delta": input,
                            }),
                        ),
                        done,
                    ];
                }
                return vec![done];
            }
        }
    }

    let changed = restore_response_apply_patch_function_calls(&mut event);
    if changed && event_type == "response.output_item.added" {
        if let Some(item) = event.get_mut("item").and_then(Value::as_object_mut) {
            if item.get("type").and_then(Value::as_str) == Some("custom_tool_call")
                && item.get("name").and_then(Value::as_str) == Some(APPLY_PATCH_TOOL_NAME)
                && item.get("input").and_then(Value::as_str) == Some("")
            {
                item.remove("input");
            }
        }
    }

    let restored = serde_json::to_string(&event).unwrap_or(data);
    vec![sse_event_text(event_name, &restored)]
}

pub(crate) fn sanitize_response_apply_patch_inputs(value: &mut Value) -> bool {
    let mut apply_patch_item_ids = HashSet::new();
    sanitize_value(value, &mut apply_patch_item_ids)
}

fn sanitize_value(value: &mut Value, apply_patch_item_ids: &mut HashSet<String>) -> bool {
    let mut changed = false;
    match value {
        Value::Array(items) => {
            for item in items {
                changed |= sanitize_value(item, apply_patch_item_ids);
            }
        }
        Value::Object(obj) => {
            let is_apply_patch_call = obj.get("type").and_then(Value::as_str)
                == Some("custom_tool_call")
                && obj.get("name").and_then(Value::as_str) == Some("apply_patch");
            if is_apply_patch_call {
                if let Some(id) = obj.get("id").and_then(Value::as_str) {
                    apply_patch_item_ids.insert(id.to_string());
                }
                if let Some(input) = obj.get("input").and_then(Value::as_str) {
                    if let Some(unwrapped) = unwrap_apply_patch_input(input) {
                        obj.insert("input".to_string(), json!(unwrapped));
                        changed = true;
                    }
                }
            }

            let event_type = obj
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if matches!(
                event_type.as_str(),
                "response.custom_tool_call_input.delta" | "response.custom_tool_call_input.done"
            ) {
                let field = if event_type.ends_with(".done") {
                    "input"
                } else {
                    "delta"
                };
                let is_apply_patch_item = obj
                    .get("item_id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| apply_patch_item_ids.contains(id));
                if is_apply_patch_item {
                    if let Some(input) = obj.get(field).and_then(Value::as_str) {
                        if let Some(unwrapped) = unwrap_apply_patch_input(input) {
                            obj.insert(field.to_string(), json!(unwrapped));
                            changed = true;
                        }
                    }
                }
            }

            for child in obj.values_mut() {
                changed |= sanitize_value(child, apply_patch_item_ids);
            }
        }
        _ => {}
    }
    changed
}

pub(crate) fn unwrap_apply_patch_input(input: &str) -> Option<String> {
    let normalized = normalized_patch_text(input)?;
    (normalized != input).then_some(normalized)
}

fn normalized_patch_text(input: &str) -> Option<String> {
    let trimmed = input.trim_start();
    if trimmed.starts_with(PATCH_BEGIN) {
        return Some(trimmed.to_string());
    }
    let value = serde_json::from_str::<Value>(trimmed).ok()?;
    extract_patch_text(&value, 0)
}

fn extract_patch_text(value: &Value, nested_json_depth: usize) -> Option<String> {
    if nested_json_depth >= MAX_UNWRAP_DEPTH {
        return None;
    }
    match value {
        Value::String(text) => {
            let trimmed = text.trim_start();
            if trimmed.starts_with(PATCH_BEGIN) {
                return Some(trimmed.to_string());
            }
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                let nested = serde_json::from_str::<Value>(trimmed).ok()?;
                return extract_patch_text(&nested, nested_json_depth + 1);
            }
            None
        }
        Value::Object(map) => {
            for key in PATCH_FIELD_PRIORITY {
                if let Some(child) = map.get(*key) {
                    if let Some(patch) = extract_patch_text(child, nested_json_depth) {
                        return Some(patch);
                    }
                }
            }
            for child in map.values() {
                if let Some(patch) = extract_patch_text(child, nested_json_depth) {
                    return Some(patch);
                }
            }
            None
        }
        Value::Array(items) => {
            for item in items {
                if let Some(patch) = extract_patch_text(item, nested_json_depth) {
                    return Some(patch);
                }
            }
            None
        }
        _ => None,
    }
}

#[derive(Debug, Default)]
struct ApplyPatchStreamState {
    item_ids: HashSet<String>,
    buffered_inputs: HashMap<String, String>,
}

pub(crate) fn create_apply_patch_input_sanitize_sse_stream<E>(
    stream: impl Stream<Item = Result<Bytes, E>> + Send + 'static,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send
where
    E: std::error::Error + Send + 'static,
{
    async_stream::stream! {
        let mut buffer = String::new();
        let mut utf8_remainder: Vec<u8> = Vec::new();
        let mut state = ApplyPatchStreamState::default();

        tokio::pin!(stream);

        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes);
                    while let Some(block) = take_sse_block(&mut buffer) {
                        if block.trim().is_empty() {
                            continue;
                        }
                        for bytes in sanitize_sse_block(&block, &mut state) {
                            yield Ok(bytes);
                        }
                    }
                }
                Err(e) => {
                    yield Err(std::io::Error::other(e.to_string()));
                    return;
                }
            }
        }

        if !utf8_remainder.is_empty() {
            buffer.push_str(&String::from_utf8_lossy(&utf8_remainder));
        }
        let tail = std::mem::take(&mut buffer);
        if !tail.trim().is_empty() {
            for bytes in sanitize_sse_block(&tail, &mut state) {
                yield Ok(bytes);
            }
        }
    }
}

fn sanitize_sse_block(block: &str, state: &mut ApplyPatchStreamState) -> Vec<Bytes> {
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
        return vec![Bytes::from(format!("{block}\n\n"))];
    }

    let data = data_parts.join("\n");
    if data.trim() == "[DONE]" {
        return vec![Bytes::from(format!("{block}\n\n"))];
    }

    let mut event: Value = match serde_json::from_str(&data) {
        Ok(value) => value,
        Err(_) => return vec![Bytes::from(format!("{block}\n\n"))],
    };

    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let item_id = event
        .get("item_id")
        .and_then(Value::as_str)
        .map(str::to_string);

    sanitize_value(&mut event, &mut state.item_ids);

    if event_type == "response.custom_tool_call_input.delta" {
        if let Some(item_id) = item_id.as_deref() {
            if state.item_ids.contains(item_id) {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    state
                        .buffered_inputs
                        .entry(item_id.to_string())
                        .or_default()
                        .push_str(delta);
                }
                return Vec::new();
            }
        }
    }

    if event_type == "response.custom_tool_call_input.done" {
        if let Some(item_id) = item_id.as_deref() {
            if state.item_ids.contains(item_id) {
                let buffered = state.buffered_inputs.remove(item_id);
                let had_buffer = buffered.is_some();
                let input = event
                    .get("input")
                    .and_then(Value::as_str)
                    .filter(|input| !input.is_empty())
                    .map(str::to_string)
                    .or(buffered);
                if let Some(input) = input {
                    if let Some(patch) = normalized_patch_text(&input) {
                        event["input"] = json!(patch.clone());
                        if had_buffer {
                            let output_index =
                                event.get("output_index").cloned().unwrap_or(json!(0));
                            return vec![
                                sse_event_bytes(
                                    "response.custom_tool_call_input.delta",
                                    json!({
                                        "type": "response.custom_tool_call_input.delta",
                                        "item_id": item_id,
                                        "output_index": output_index,
                                        "delta": patch,
                                    }),
                                ),
                                sse_event_bytes(event_name.unwrap_or(&event_type), event),
                            ];
                        }
                        return vec![sse_event_bytes(event_name.unwrap_or(&event_type), event)];
                    }
                }
            }
        }
    }

    let sanitized = serde_json::to_string(&event).unwrap_or(data);
    vec![sse_event_text(event_name, &sanitized)]
}

fn sse_event_bytes(event_name: &str, data: Value) -> Bytes {
    let data = serde_json::to_string(&data).unwrap_or_default();
    sse_event_text(Some(event_name), &data)
}

fn sse_event_text(event_name: Option<&str>, data: &str) -> Bytes {
    let mut out = String::new();
    if let Some(name) = event_name {
        out.push_str("event: ");
        out.push_str(name);
        out.push('\n');
    }
    out.push_str("data: ");
    out.push_str(data);
    out.push_str("\n\n");
    Bytes::from(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;

    fn sample_patch() -> &'static str {
        "*** Begin Patch\n*** Update File: /tmp/a.txt\n@@\n-old\n+new\n*** End Patch"
    }

    #[test]
    fn bridges_request_custom_apply_patch_to_standard_function() {
        let patch = sample_patch();
        let mut request = json!({
            "tools": [{
                "type": "custom",
                "name": "apply_patch",
                "description": "Apply a patch.",
                "format": { "type": "grammar", "syntax": "lark", "definition": "start: /.+/" }
            }],
            "additional_tools": [{
                "type": "custom",
                "name": "apply_patch",
                "description": "Apply a discovered patch."
            }],
            "tool_choice": { "type": "custom", "name": "apply_patch" },
            "input": [
                {
                    "type": "custom_tool_call",
                    "id": "ctc_1",
                    "call_id": "call_1",
                    "name": "apply_patch",
                    "input": patch
                },
                {
                    "type": "custom_tool_call_output",
                    "call_id": "call_1",
                    "output": "Done!"
                }
            ]
        });

        assert!(bridge_request_apply_patch_custom_to_function(&mut request));
        for key in ["tools", "additional_tools"] {
            let tool = &request[key][0];
            assert_eq!(tool["type"], "function");
            assert_eq!(tool["name"], APPLY_PATCH_TOOL_NAME);
            assert_eq!(
                tool["parameters"]["required"],
                json!([APPLY_PATCH_FUNCTION_INPUT_FIELD])
            );
            assert!(tool.get("format").is_none());
        }
        assert_eq!(request["tool_choice"]["type"], "function");
        assert_eq!(request["input"][0]["type"], "function_call");
        assert_eq!(
            serde_json::from_str::<Value>(
                request["input"][0]["arguments"]
                    .as_str()
                    .expect("string arguments")
            )
            .expect("valid arguments")[APPLY_PATCH_FUNCTION_INPUT_FIELD],
            patch
        );
        assert_eq!(request["input"][1]["type"], "function_call_output");
    }

    #[test]
    fn restores_non_streaming_function_apply_patch_to_custom_call() {
        let patch = sample_patch();
        let mut response = json!({
            "id": "resp_1",
            "output": [{
                "type": "function_call",
                "id": "fc_1",
                "call_id": "call_1",
                "name": "apply_patch",
                "arguments": json!({ "input": patch }).to_string()
            }]
        });

        assert!(restore_response_apply_patch_function_calls(&mut response));
        assert_eq!(response["output"][0]["type"], "custom_tool_call");
        assert_eq!(response["output"][0]["input"], patch);
        assert!(response["output"][0].get("arguments").is_none());
    }

    #[test]
    fn bridged_apply_patch_survives_xai_strict_sanitizer() {
        let mut request = json!({
            "model": "grok-4.5",
            "tools": [{
                "type": "custom",
                "name": "apply_patch",
                "description": "Apply a patch.",
                "format": { "type": "grammar", "syntax": "lark", "definition": "start: /.+/" }
            }],
            "tool_choice": { "type": "custom", "name": "apply_patch" }
        });

        assert!(bridge_request_apply_patch_custom_to_function(&mut request));
        crate::proxy::providers::transform_codex_responses_xai_sanitize::sanitize_xai_responses_request(
            &mut request,
        );

        assert_eq!(request["tools"][0]["type"], "function");
        assert_eq!(request["tools"][0]["name"], APPLY_PATCH_TOOL_NAME);
        assert_eq!(request["tool_choice"]["type"], "function");
        assert_eq!(request["tool_choice"]["name"], APPLY_PATCH_TOOL_NAME);
    }

    #[tokio::test]
    async fn restores_streamed_function_arguments_to_custom_input() {
        let patch = sample_patch();
        let arguments = json!({ "input": patch }).to_string();
        let split = arguments.len() / 2;
        let input = format!(
            "event: response.output_item.added\n\
             data: {}\n\n\
             event: response.function_call_arguments.delta\n\
             data: {}\n\n\
             event: response.function_call_arguments.delta\n\
             data: {}\n\n\
             event: response.function_call_arguments.done\n\
             data: {}\n\n\
             event: response.output_item.done\n\
             data: {}\n\n",
            json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {
                    "type": "function_call",
                    "id": "fc_1",
                    "call_id": "call_1",
                    "name": "apply_patch",
                    "arguments": ""
                }
            }),
            json!({
                "type": "response.function_call_arguments.delta",
                "item_id": "fc_1",
                "output_index": 0,
                "delta": &arguments[..split]
            }),
            json!({
                "type": "response.function_call_arguments.delta",
                "item_id": "fc_1",
                "output_index": 0,
                "delta": &arguments[split..]
            }),
            json!({
                "type": "response.function_call_arguments.done",
                "item_id": "fc_1",
                "output_index": 0,
                "arguments": arguments
            }),
            json!({
                "type": "response.output_item.done",
                "output_index": 0,
                "item": {
                    "type": "function_call",
                    "id": "fc_1",
                    "call_id": "call_1",
                    "name": "apply_patch",
                    "arguments": json!({ "input": patch }).to_string()
                }
            })
        );
        let upstream = stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from(input))]);
        let output = create_apply_patch_function_restore_sse_stream(upstream)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(Result::unwrap)
            .fold(Vec::new(), |mut acc, bytes| {
                acc.extend_from_slice(&bytes);
                acc
            });
        let output = String::from_utf8(output).unwrap();

        assert!(output.contains("\"type\":\"custom_tool_call\""));
        assert!(output.contains("event: response.custom_tool_call_input.delta"));
        assert!(output.contains("event: response.custom_tool_call_input.done"));
        assert!(output.contains("\"delta\":\"*** Begin Patch\\n"));
        assert!(output.contains("\"input\":\"*** Begin Patch\\n"));
        assert!(!output.contains("response.function_call_arguments"));
    }

    #[test]
    fn unwraps_observed_apply_patch_wrappers() {
        let patch = sample_patch();
        let cases = [
            json!({ "patch": patch }).to_string(),
            json!({ "data": patch }).to_string(),
            json!({ "cmd": patch }).to_string(),
            json!({ "input": json!({ "patch": patch }).to_string() }).to_string(),
            json!({ "data": json!({ "patch": patch }).to_string() }).to_string(),
            json!({ "command": ["apply_patch", patch] }).to_string(),
        ];

        for input in cases {
            assert_eq!(unwrap_apply_patch_input(&input).as_deref(), Some(patch));
        }
    }

    #[test]
    fn unwraps_nested_wrappers_up_to_depth_limit() {
        let patch = sample_patch();

        // An unwrapped patch is already valid and therefore intentionally returns None.
        let mut input = patch.to_string();
        assert_eq!(unwrap_apply_patch_input(&input), None);

        for wrapper_count in 1..=MAX_UNWRAP_DEPTH {
            input = json!({ "data": input }).to_string();

            assert_eq!(
                unwrap_apply_patch_input(&input).as_deref(),
                Some(patch),
                "wrapper_count={wrapper_count}"
            );
        }

        let too_deep = (0..=MAX_UNWRAP_DEPTH).fold(sample_patch().to_string(), |value, _| {
            json!({ "data": value }).to_string()
        });
        assert_eq!(unwrap_apply_patch_input(&too_deep.to_string()), None);
    }

    #[test]
    fn ignores_non_patch_json_inputs() {
        assert_eq!(unwrap_apply_patch_input("{}"), None);
        assert_eq!(
            unwrap_apply_patch_input(r#"{"cmd":"mkdir -p /tmp/x"}"#),
            None
        );
        assert_eq!(unwrap_apply_patch_input(r#"{"patch":"not a patch"}"#), None);
    }

    #[test]
    fn sanitizes_non_streaming_custom_tool_call() {
        let patch = sample_patch();
        let mut response = json!({
            "id": "resp_1",
            "output": [{
                "type": "custom_tool_call",
                "id": "msg_1",
                "call_id": "call_1",
                "name": "apply_patch",
                "input": json!({ "data": json!({ "patch": patch }).to_string() }).to_string()
            }]
        });

        assert!(sanitize_response_apply_patch_inputs(&mut response));
        assert_eq!(response["output"][0]["input"], patch);
    }

    #[tokio::test]
    async fn sanitizes_streamed_done_item() {
        let patch = sample_patch();
        let input = format!(
            "event: response.output_item.done\n\
             data: {}\n\n",
            json!({
                "type": "response.output_item.done",
                "output_index": 0,
                "item": {
                    "type": "custom_tool_call",
                    "id": "msg_1",
                    "call_id": "call_1",
                    "name": "apply_patch",
                    "input": json!({ "patch": patch }).to_string()
                }
            })
        );
        let upstream = stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from(input))]);
        let output = create_apply_patch_input_sanitize_sse_stream(upstream)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(Result::unwrap)
            .fold(Vec::new(), |mut acc, bytes| {
                acc.extend_from_slice(&bytes);
                acc
            });
        let output = String::from_utf8(output).unwrap();

        assert!(output.contains("\"input\":\"*** Begin Patch\\n"));
        assert!(!output.contains("{\\\"patch\\\""));
    }

    #[tokio::test]
    async fn buffers_streamed_json_delta_until_done() {
        let patch = sample_patch();
        let wrapped = json!({ "patch": patch }).to_string();
        let input = format!(
            "event: response.output_item.added\n\
             data: {}\n\n\
             event: response.custom_tool_call_input.delta\n\
             data: {}\n\n\
             event: response.custom_tool_call_input.done\n\
             data: {}\n\n",
            json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {
                    "type": "custom_tool_call",
                    "id": "msg_1",
                    "call_id": "call_1",
                    "name": "apply_patch"
                }
            }),
            json!({
                "type": "response.custom_tool_call_input.delta",
                "item_id": "msg_1",
                "output_index": 0,
                "delta": wrapped
            }),
            json!({
                "type": "response.custom_tool_call_input.done",
                "item_id": "msg_1",
                "output_index": 0,
                "input": wrapped
            })
        );
        let upstream = stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from(input))]);
        let output = create_apply_patch_input_sanitize_sse_stream(upstream)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(Result::unwrap)
            .fold(Vec::new(), |mut acc, bytes| {
                acc.extend_from_slice(&bytes);
                acc
            });
        let output = String::from_utf8(output).unwrap();

        assert_eq!(
            output
                .matches("event: response.custom_tool_call_input.delta")
                .count(),
            1
        );
        assert!(output.contains("\"delta\":\"*** Begin Patch\\n"));
        assert!(output.contains("\"input\":\"*** Begin Patch\\n"));
        assert!(!output.contains("{\\\"patch\\\""));
    }
}
