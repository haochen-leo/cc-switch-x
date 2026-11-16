//! Repair leading inline <think> blocks without rebuilding native Responses.
//!
//! Only the first text part of an assistant message is inspected. A complete
//! block becomes a separate reasoning item; ordinary text and incomplete blocks
//! retain their original representation. The Chat bridge owns the shared parser.

use std::collections::{BTreeSet, HashMap, HashSet};

use bytes::Bytes;
use futures::{Stream, StreamExt};
use serde_json::Value;

use super::{codex_chat_common::split_leading_think_block, codex_responses_sse as sse};
use crate::proxy::{
    json_canonical::short_sha256_hex,
    sse::{append_utf8_safe, strip_sse_field, take_sse_block},
};

fn message_text(item: &Value) -> Option<&str> {
    if item.get("type")?.as_str()? != "message"
        || item.get("role")?.as_str()? != "assistant"
        || item.pointer("/content/0/type")?.as_str()? != "output_text"
    {
        return None;
    }
    item.pointer("/content/0/text")?.as_str()
}

fn reasoning_item(message: &Value, text: &str) -> Value {
    let seed = message.get("id").and_then(Value::as_str).unwrap_or(text);
    sse::reasoning_item(
        &format!("rs_ccswitch_think_{}", short_sha256_hex(seed.as_bytes())),
        text,
    )
}

pub(crate) fn normalize_response(body: &mut Value) {
    if let Some(response) = body.get_mut("response") {
        normalize_response(response);
    }
    let Some(output) = body.get_mut("output").and_then(Value::as_array_mut) else {
        return;
    };
    let mut normalized = Vec::with_capacity(output.len());
    for mut item in std::mem::take(output) {
        if let Some((thinking, answer)) = message_text(&item).and_then(split_leading_think_block) {
            if !thinking.is_empty() {
                normalized.push(reasoning_item(&item, &thinking));
            }
            item["content"][0]["text"] = Value::String(answer);
        }
        normalized.push(item);
    }
    *output = normalized;
}

struct Block {
    raw: String,
    event: Option<Value>,
}

impl Block {
    fn parse(raw: String) -> Self {
        let data = raw
            .lines()
            .filter_map(|line| strip_sse_field(line, "data"))
            .collect::<Vec<_>>()
            .join("\n");
        let event = serde_json::from_str(&data).ok();
        Self { raw, event }
    }
}

struct PendingMessage {
    id: String,
    index: u64,
    item: Value,
    text: String,
    blocks: Vec<Block>,
}

impl PendingMessage {
    fn observe(&mut self, event: &Value) {
        if event.get("item_id").and_then(Value::as_str) == Some(&self.id)
            && event.get("content_index").and_then(Value::as_u64) == Some(0)
        {
            match event.get("type").and_then(Value::as_str) {
                Some("response.output_text.delta") => {
                    if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                        self.text.push_str(delta);
                    }
                }
                Some("response.output_text.done") => {
                    if let Some(text) = event.get("text").and_then(Value::as_str) {
                        self.text = text.to_string();
                    }
                }
                Some("response.content_part.done") => {
                    if let Some(text) = event.pointer("/part/text").and_then(Value::as_str) {
                        self.text = text.to_string();
                    }
                }
                _ => {}
            }
        }
        if event.pointer("/item/id").and_then(Value::as_str) == Some(&self.id) {
            if let Some(text) = event.get("item").and_then(message_text) {
                if !text.is_empty() {
                    self.text = text.to_string();
                }
            }
        }
        if let Some(items) = event.pointer("/response/output").and_then(Value::as_array) {
            if let Some(text) = items
                .iter()
                .find(|item| item.get("id").and_then(Value::as_str) == Some(&self.id))
                .and_then(message_text)
            {
                self.text = text.to_string();
            }
        }
    }

    fn at_boundary(&self, event: &Value) -> bool {
        matches!(
            event.get("type").and_then(Value::as_str),
            Some("response.completed" | "response.failed" | "response.incomplete")
        ) || (event.get("type").and_then(Value::as_str) == Some("response.output_item.done")
            && event.pointer("/item/id").and_then(Value::as_str) == Some(&self.id))
    }
}

struct SplitMessage {
    reasoning: Value,
    prefix_bytes: usize,
    delta_bytes: usize,
    trim_separator: bool,
}

#[derive(Default)]
struct StreamState {
    pending: Option<PendingMessage>,
    inspected: HashSet<String>,
    splits: HashMap<String, SplitMessage>,
    insertions: BTreeSet<u64>,
    changed: bool,
    next_sequence: u64,
}

impl StreamState {
    fn handle(&mut self, block: Block) -> Vec<Bytes> {
        if self.pending.is_none() {
            if let Some(event) = block.event.as_ref() {
                if event.get("type").and_then(Value::as_str) == Some("response.output_item.added") {
                    if let Some(item) = event.get("item").filter(|item| {
                        item.get("type").and_then(Value::as_str) == Some("message")
                            && item.get("role").and_then(Value::as_str) == Some("assistant")
                            && (item.pointer("/content/0").is_none()
                                || message_text(item).is_some())
                    }) {
                        if let (Some(id), Some(index)) = (
                            item.get("id").and_then(Value::as_str),
                            event.get("output_index").and_then(Value::as_u64),
                        ) {
                            if self.inspected.insert(id.to_string()) {
                                self.pending = Some(PendingMessage {
                                    id: id.to_string(),
                                    index,
                                    item: item.clone(),
                                    text: String::new(),
                                    blocks: Vec::new(),
                                });
                            }
                        }
                    }
                }
            }
        }

        if let Some(mut pending) = self.pending.take() {
            let boundary = block.event.as_ref().is_some_and(|event| {
                pending.observe(event);
                pending.at_boundary(event)
            });
            pending.blocks.push(block);
            let split = split_leading_think_block(&pending.text);
            let prefix = pending.text.trim_start();
            let needs_more =
                prefix.is_empty() || "<think>".starts_with(prefix) || prefix.starts_with("<think>");
            if split.is_none() && needs_more && !boundary {
                self.pending = Some(pending);
                return Vec::new();
            }

            let mut out = Vec::new();
            if let Some((thinking, answer)) = split {
                self.changed = true;
                let reasoning = reasoning_item(&pending.item, &thinking);
                if !thinking.is_empty() {
                    let index =
                        pending.index + self.insertions.range(..pending.index).count() as u64;
                    self.insertions.insert(pending.index);
                    let id = reasoning["id"].as_str().unwrap();
                    let mut events = vec![
                        sse::reasoning_item_added(index as u32, id),
                        sse::reasoning_summary_part_added(index as u32, id),
                        sse::reasoning_summary_text_delta(index as u32, id, &thinking),
                    ];
                    events.extend(sse::reasoning_close(index as u32, id, &thinking).0);
                    for bytes in events {
                        let block = Block::parse(String::from_utf8(bytes.to_vec()).unwrap());
                        out.push(self.serialize(block.event.unwrap()));
                    }
                }
                self.splits.insert(
                    pending.id,
                    SplitMessage {
                        reasoning,
                        prefix_bytes: pending.text.len() - answer.len(),
                        delta_bytes: 0,
                        trim_separator: true,
                    },
                );
            }
            // Reprocess buffered events so interleaved messages can be inspected too.
            for block in pending.blocks {
                out.extend(self.handle(block));
            }
            return out;
        }

        self.emit(block).into_iter().collect()
    }

    fn serialize(&mut self, mut event: Value) -> Bytes {
        if self.changed {
            event["sequence_number"] = self.next_sequence.into();
        }
        self.next_sequence = event
            .get("sequence_number")
            .and_then(Value::as_u64)
            .unwrap_or(self.next_sequence)
            + 1;
        let event_type = event["type"].as_str().unwrap_or("message").to_string();
        sse::sse_event(&event_type, event)
    }

    fn emit(&mut self, block: Block) -> Option<Bytes> {
        let Some(mut event) = block.event else {
            return Some(Bytes::from(block.raw));
        };
        if !self.changed {
            self.next_sequence = event
                .get("sequence_number")
                .and_then(Value::as_u64)
                .unwrap_or(self.next_sequence)
                + 1;
            return Some(Bytes::from(block.raw));
        }

        if let Some(index) = event.get("output_index").and_then(Value::as_u64) {
            event["output_index"] = (index + self.insertions.range(..=index).count() as u64).into();
        }
        let id = event
            .get("item_id")
            .or_else(|| event.pointer("/item/id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if let Some(split) = self.splits.get_mut(&id) {
            match event.get("type").and_then(Value::as_str) {
                Some("response.output_text.delta")
                    if event.get("content_index").and_then(Value::as_u64) == Some(0) =>
                {
                    if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                        let skip = split
                            .prefix_bytes
                            .saturating_sub(split.delta_bytes)
                            .min(delta.len());
                        split.delta_bytes += delta.len();
                        let mut answer = &delta[skip..];
                        if split.trim_separator {
                            answer = answer.trim_start_matches(['\r', '\n', '\t', ' ']);
                            split.trim_separator = answer.is_empty();
                        }
                        if answer.is_empty() {
                            return None;
                        }
                        event["delta"] = answer.to_string().into();
                    }
                }
                Some("response.output_text.done")
                    if event.get("content_index").and_then(Value::as_u64) == Some(0) =>
                {
                    strip_text(&mut event["text"]);
                }
                Some("response.content_part.added")
                    if event.get("content_index").and_then(Value::as_u64) == Some(0) =>
                {
                    event["part"]["text"] = "".into();
                }
                Some("response.content_part.done")
                    if event.get("content_index").and_then(Value::as_u64) == Some(0) =>
                {
                    strip_text(&mut event["part"]["text"]);
                }
                Some("response.output_item.added" | "response.output_item.done") => {
                    if let Some(text) = event.pointer_mut("/item/content/0/text") {
                        strip_text(text);
                    }
                }
                _ => {}
            }
        }
        if let Some(output) = event
            .pointer_mut("/response/output")
            .and_then(Value::as_array_mut)
        {
            let mut normalized = Vec::with_capacity(output.len() + self.splits.len());
            for mut item in std::mem::take(output) {
                if let Some(split) = item
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(|id| self.splits.get(id))
                {
                    if split.reasoning["summary"][0]["text"].as_str() != Some("") {
                        normalized.push(split.reasoning.clone());
                    }
                    strip_text(&mut item["content"][0]["text"]);
                }
                normalized.push(item);
            }
            *output = normalized;
        }
        Some(self.serialize(event))
    }

    fn finish(&mut self) -> Vec<Bytes> {
        self.pending
            .take()
            .map(|pending| {
                pending
                    .blocks
                    .into_iter()
                    .filter_map(|block| self.emit(block))
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn strip_text(text: &mut Value) {
    if let Some((_, answer)) = text.as_str().and_then(split_leading_think_block) {
        *text = answer.into();
    }
}

pub(crate) fn normalize_stream(
    stream: super::transform_codex_responses::ResponseByteStream,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        let mut stream = stream;
        let mut buffer = String::new();
        let mut utf8_remainder = Vec::new();
        let mut state = StreamState::default();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes);
                    while let Some(block) = take_sse_block(&mut buffer) {
                        for bytes in state.handle(Block::parse(format!("{block}\n\n"))) {
                            yield Ok(bytes);
                        }
                    }
                }
                Err(error) => {
                    for bytes in state.finish() {
                        yield Ok(bytes);
                    }
                    yield Err(error);
                    return;
                }
            }
        }
        if !utf8_remainder.is_empty() {
            buffer.push_str(&String::from_utf8_lossy(&utf8_remainder));
        }
        if !buffer.is_empty() {
            for bytes in state.handle(Block::parse(buffer)) {
                yield Ok(bytes);
            }
        }
        for bytes in state.finish() {
            yield Ok(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::TryStreamExt;
    use serde_json::json;

    fn message(text: &str) -> Value {
        json!({"type":"message","id":"msg_test","role":"assistant","status":"completed",
            "phase":"final_answer","content":[{"type":"output_text","text":text,"annotations":[]}]})
    }

    fn events(source: &str) -> Vec<Value> {
        source
            .lines()
            .filter_map(|line| strip_sse_field(line, "data"))
            .filter_map(|data| serde_json::from_str(data).ok())
            .collect()
    }

    async fn convert(source: &str, chunk_size: usize) -> String {
        let chunks = source
            .as_bytes()
            .chunks(chunk_size)
            .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
            .collect::<Vec<_>>();
        let bytes = normalize_stream(Box::pin(futures::stream::iter(chunks)))
            .try_fold(Vec::new(), |mut out, chunk| async move {
                out.extend_from_slice(&chunk);
                Ok(out)
            })
            .await
            .unwrap();
        String::from_utf8(bytes).unwrap()
    }

    fn stream(text_chunks: &[&str]) -> String {
        let text = text_chunks.concat();
        let mut list = vec![
            json!({"type":"response.created","response":{"id":"resp_test","output":[]}}),
            json!({"type":"response.output_item.added","output_index":0,
                "item":{"type":"message","id":"msg_test","role":"assistant","phase":"final_answer","content":[]}}),
            json!({"type":"response.content_part.added","output_index":0,"item_id":"msg_test","content_index":0,
                "part":{"type":"output_text","text":"","annotations":[]}}),
        ];
        for chunk in text_chunks {
            list.push(json!({"type":"response.output_text.delta","output_index":0,"item_id":"msg_test","content_index":0,"delta":chunk}));
        }
        list.extend([
            json!({"type":"response.output_text.done","output_index":0,"item_id":"msg_test","content_index":0,"text":text}),
            json!({"type":"response.content_part.done","output_index":0,"item_id":"msg_test","content_index":0,"part":message(&text)["content"][0]}),
            json!({"type":"response.output_item.done","output_index":0,"item":message(&text)}),
            json!({"type":"response.output_item.added","output_index":1,"item":{"id":"fc_test","type":"function_call","name":"exec","call_id":"call_test","arguments":"{}"}}),
            json!({"type":"response.function_call_arguments.done","output_index":1,"item_id":"fc_test","arguments":"{}"}),
            json!({"type":"response.completed","response":{"id":"resp_test","usage":{"input_tokens":10,"output_tokens":20,"output_tokens_details":{"reasoning_tokens":0}},
                "output":[message(&text),{"id":"fc_test","type":"function_call","name":"exec","call_id":"call_test","arguments":"{}"}]}}),
        ]);
        list.into_iter()
            .enumerate()
            .map(|(index, mut event)| {
                event["sequence_number"] = index.into();
                let name = event["type"].as_str().unwrap().to_string();
                String::from_utf8(sse::sse_event(&name, event).to_vec()).unwrap()
            })
            .collect()
    }

    fn assert_consistent(source: &str, transformed: &str) {
        let before = events(source);
        let after = events(transformed);
        let response = after.last().unwrap()["response"].clone();
        let output = response["output"].as_array().unwrap();
        let done_items: Vec<_> = after
            .iter()
            .filter(|event| event["type"] == "response.output_item.done")
            .map(|event| event["item"].clone())
            .collect();
        for item in done_items {
            assert!(
                output.contains(&item),
                "done item differs from final output"
            );
        }
        let text: String = after
            .iter()
            .filter(|event| event["type"] == "response.output_text.delta")
            .filter_map(|event| event["delta"].as_str())
            .collect();
        assert_eq!(text, message_text(&output[1]).unwrap());
        for (sequence, event) in after.iter().enumerate() {
            assert_eq!(event["sequence_number"], sequence);
            if let (Some(index), Some(id)) = (
                event["output_index"].as_u64(),
                event
                    .get("item_id")
                    .or_else(|| event.pointer("/item/id"))
                    .and_then(Value::as_str),
            ) {
                assert_eq!(output[index as usize]["id"], id);
            }
        }
        assert_eq!(
            response["usage"],
            before.last().unwrap()["response"]["usage"]
        );
        assert_eq!(
            output[1]["phase"],
            before.last().unwrap()["response"]["output"][0]["phase"]
        );
    }

    #[test]
    fn json_splits_only_complete_leading_blocks_and_preserves_native_fields() {
        let native =
            json!({"id":"rs_native","type":"reasoning","encrypted_content":"opaque","summary":[]});
        let mut body = json!({"id":"resp_test","usage":{"output_tokens":12},"output":[native.clone(),message("<think>推理</think>\n答案")]});
        normalize_response(&mut body);
        assert_eq!(body["output"][0], native);
        assert_eq!(body["output"][1]["summary"][0]["text"], "推理");
        assert!(body["output"][1].get("encrypted_content").is_none());
        assert_eq!(body["output"][2]["content"][0]["text"], "答案");
        assert_eq!(body["output"][2]["phase"], "final_answer");
        assert_eq!(body["usage"], json!({"output_tokens":12}));
        let once = body.clone();
        normalize_response(&mut body);
        assert_eq!(body, once);
        for text in ["正文 <think>示例</think>", "<think>没有结束", "普通回复"] {
            let mut body = json!({"output":[message(text)]});
            let original = body.clone();
            normalize_response(&mut body);
            assert_eq!(body, original);
        }
    }

    #[tokio::test]
    async fn stream_splits_fragmented_tags_and_preserves_tool_indexes_and_snapshots() {
        let source = stream(&[" \n<", "think>推", "理</thi", "nk>", "\n", " 答", "案"]);
        let transformed = convert(&source, 7).await;
        assert_consistent(&source, &transformed);
        let list = events(&transformed);
        assert_eq!(
            list.last().unwrap()["response"]["output"][0]["summary"][0]["text"],
            "推理"
        );
        assert_eq!(
            list.last().unwrap()["response"]["output"][2]["id"],
            "fc_test"
        );
    }

    #[tokio::test]
    async fn stream_passes_normal_reasoning_and_incomplete_blocks_unchanged() {
        for chunks in [
            vec!["普通", "回复"],
            vec!["<thi", "nk>未闭合"],
            vec!["文字 <think>只是示例</think>"],
        ] {
            let source = stream(&chunks);
            assert_eq!(convert(&source, 5).await, source);
        }
        let source = "event: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"id\":\"rs_native\",\"type\":\"reasoning\",\"encrypted_content\":\"opaque\",\"summary\":[]}}\n\n";
        assert_eq!(convert(source, 3).await, source);
    }

    #[tokio::test]
    async fn native_pipeline_normalizes_json_and_stream_without_configuration() {
        let provider = crate::provider::Provider::with_id(
            "native".into(),
            "native".into(),
            json!({"base_url":"https://example.test/v1"}),
            None,
        );
        let context = super::super::transform_codex_responses::prepare_request(
            json!({"input":[]}),
            &provider,
            Some("test-model"),
        )
        .unwrap()
        .context;
        let mut body = json!({"output":[message("<think>推理</think>答案")]});
        super::super::transform_codex_responses::transform_response(&mut body, &context);
        assert_eq!(body["output"][0]["summary"][0]["text"], "推理");
        assert_eq!(message_text(&body["output"][1]), Some("答案"));
        let source = stream(&["<think>推理</think>答案"]);
        let stream = super::super::transform_codex_responses::transform_response_stream(
            Box::pin(futures::stream::iter(vec![Ok(Bytes::from(source.clone()))])),
            context,
        );
        let chunks: Vec<_> = stream.try_collect().await.unwrap();
        let transformed = String::from_utf8(chunks.concat()).unwrap();
        assert_consistent(&source, &transformed);
        assert_eq!(
            events(&transformed).last().unwrap()["response"]["output"][0]["summary"][0]["text"],
            "推理"
        );
    }

    #[tokio::test]
    #[ignore = "requires a captured upstream SSE file; contains no committed conversation data"]
    async fn replay_captured_native_response() {
        let path = std::env::var("CC_SWITCH_NATIVE_THINK_REPLAY").unwrap();
        let source = std::fs::read_to_string(path).unwrap();
        let transformed = convert(&source, 113).await;
        assert_consistent(&source, &transformed);
        let original_events = events(&source);
        let mut expected = original_events.last().unwrap()["response"].clone();
        normalize_response(&mut expected);
        let after = events(&transformed);
        assert_eq!(after.last().unwrap()["response"], expected);
        let result = &expected["output"];
        eprintln!("real replay: {} source events, {} normalized events, {} reasoning chars, {} answer chars; usage unchanged",
            original_events.len(), after.len(), result[0]["summary"][0]["text"].as_str().unwrap().chars().count(),
            result[1]["content"][0]["text"].as_str().unwrap().chars().count());
    }
}
