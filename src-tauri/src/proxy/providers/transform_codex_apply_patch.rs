//! Response-side compatibility for Codex freeform `apply_patch` calls.
//!
//! Some native Responses-compatible upstreams return the freeform input wrapped
//! in a JSON string such as `{"patch":"*** Begin Patch\n..."}`. Codex passes
//! freeform custom-tool input directly to the patch parser, so the leading `{`
//! makes the call fail before the patch body is examined. This module only
//! unwraps `apply_patch` custom-tool inputs when the extracted text is a valid
//! patch envelope.

use std::collections::{HashMap, HashSet};

use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use serde_json::{json, Value};

use crate::proxy::sse::{append_utf8_safe, strip_sse_field, take_sse_block};

const PATCH_BEGIN: &str = "*** Begin Patch";
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
    fn unwraps_observed_apply_patch_wrappers() {
        let patch = sample_patch();
        let cases = [
            json!({ "patch": patch }).to_string(),
            json!({ "data": patch }).to_string(),
            json!({ "cmd": patch }).to_string(),
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
