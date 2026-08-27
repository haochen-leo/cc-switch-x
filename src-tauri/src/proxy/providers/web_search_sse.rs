//! Responses JSON → SSE stream conversion for the web search sidecar.
//!
//! When the sidecar loop forces non-streaming upstream requests, the final
//! response is a Responses-format JSON object. This module converts it into
//! a proper SSE event stream so the Codex client (which expects SSE) can
//! consume it transparently.

use super::codex_responses_sse::sse_event;
use bytes::Bytes;
use futures::stream::{self, Stream};
use serde_json::{json, Value};

/// Convert a Responses JSON response into an SSE byte stream.
///
/// Emits the full lifecycle: response.created → response.in_progress →
/// output items → response.completed.
pub fn responses_json_to_sse_stream(
    response: &Value,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> {
    let mut events: Vec<Bytes> = Vec::new();

    // Build the "in_progress" version of the response for lifecycle events
    let mut in_progress_response = response.clone();
    if let Some(obj) = in_progress_response.as_object_mut() {
        obj.insert("status".to_string(), json!("in_progress"));
    }

    // response.created
    events.push(sse_event(
        "response.created",
        json!({ "type": "response.created", "response": in_progress_response }),
    ));

    // response.in_progress
    events.push(sse_event(
        "response.in_progress",
        json!({ "type": "response.in_progress", "response": in_progress_response }),
    ));

    // Emit each output item
    let output_items = response
        .get("output")
        .and_then(|o| o.as_array())
        .cloned()
        .unwrap_or_default();

    for (index, item) in output_items.iter().enumerate() {
        let output_index = index as u32;
        let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match item_type {
            "message" => {
                // Emit message item with text content
                let item_id = item
                    .get("id")
                    .and_then(|i| i.as_str())
                    .unwrap_or("msg_sidecar");

                // output_item.added (in_progress)
                events.push(sse_event(
                    "response.output_item.added",
                    json!({
                        "type": "response.output_item.added",
                        "output_index": output_index,
                        "item": {
                            "id": item_id,
                            "type": "message",
                            "status": "in_progress",
                            "role": "assistant",
                            "content": []
                        }
                    }),
                ));

                // Extract text from content parts
                let text = extract_message_text(item);

                // content_part.added
                events.push(sse_event(
                    "response.content_part.added",
                    json!({
                        "type": "response.content_part.added",
                        "item_id": item_id,
                        "output_index": output_index,
                        "content_index": 0,
                        "part": { "type": "output_text", "text": "", "annotations": [] }
                    }),
                ));

                // output_text.delta (emit the full text as one delta)
                if !text.is_empty() {
                    events.push(sse_event(
                        "response.output_text.delta",
                        json!({
                            "type": "response.output_text.delta",
                            "item_id": item_id,
                            "output_index": output_index,
                            "content_index": 0,
                            "delta": text
                        }),
                    ));
                }

                // output_text.done
                events.push(sse_event(
                    "response.output_text.done",
                    json!({
                        "type": "response.output_text.done",
                        "item_id": item_id,
                        "output_index": output_index,
                        "content_index": 0,
                        "text": text
                    }),
                ));

                // content_part.done
                events.push(sse_event(
                    "response.content_part.done",
                    json!({
                        "type": "response.content_part.done",
                        "item_id": item_id,
                        "output_index": output_index,
                        "content_index": 0,
                        "part": { "type": "output_text", "text": text, "annotations": [] }
                    }),
                ));

                // output_item.done
                events.push(sse_event(
                    "response.output_item.done",
                    json!({
                        "type": "response.output_item.done",
                        "output_index": output_index,
                        "item": item
                    }),
                ));
            }
            "function_call" | "custom_tool_call" | "tool_search_call" | "web_search_call" => {
                // Emit tool call items directly
                events.push(sse_event(
                    "response.output_item.added",
                    json!({
                        "type": "response.output_item.added",
                        "output_index": output_index,
                        "item": item
                    }),
                ));
                events.push(sse_event(
                    "response.output_item.done",
                    json!({
                        "type": "response.output_item.done",
                        "output_index": output_index,
                        "item": item
                    }),
                ));
            }
            "reasoning" => {
                // Emit reasoning items directly
                events.push(sse_event(
                    "response.output_item.added",
                    json!({
                        "type": "response.output_item.added",
                        "output_index": output_index,
                        "item": item
                    }),
                ));
                events.push(sse_event(
                    "response.output_item.done",
                    json!({
                        "type": "response.output_item.done",
                        "output_index": output_index,
                        "item": item
                    }),
                ));
            }
            _ => {
                // Unknown item types: emit as-is
                events.push(sse_event(
                    "response.output_item.added",
                    json!({
                        "type": "response.output_item.added",
                        "output_index": output_index,
                        "item": item
                    }),
                ));
                events.push(sse_event(
                    "response.output_item.done",
                    json!({
                        "type": "response.output_item.done",
                        "output_index": output_index,
                        "item": item
                    }),
                ));
            }
        }
    }

    // response.completed (with the original response that has status: "completed")
    events.push(sse_event(
        "response.completed",
        json!({ "type": "response.completed", "response": response }),
    ));

    stream::iter(events.into_iter().map(Ok))
}

/// Extract text content from a message item's content parts.
fn extract_message_text(item: &Value) -> String {
    let mut texts = Vec::new();
    if let Some(content) = item.get("content").and_then(|c| c.as_array()) {
        for part in content {
            if part.get("type").and_then(|t| t.as_str()) == Some("output_text") {
                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                    texts.push(text.to_string());
                }
            }
        }
    }
    texts.join("")
}
