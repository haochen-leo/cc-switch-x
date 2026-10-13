use crate::{
    database::Database,
    provider::Provider,
    proxy::{handler_context::RequestContext, server::ProxyState},
};
use bytes::{Bytes, BytesMut};
use futures::{Stream, StreamExt};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{
    borrow::Cow,
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, LazyLock, Mutex},
};

const PAYLOAD_LOG_FILE_NAME: &str = "cc-switch-payload.log";
const REPEATED_TEXT_MIN_BYTES: usize = 1024;
const MB: u64 = 1024 * 1024;

static PAYLOAD_LOG_STATE: LazyLock<Mutex<PayloadLogState>> =
    LazyLock::new(|| Mutex::new(PayloadLogState::default()));

type TextHash = [u8; 32];

#[derive(Clone)]
struct TextReference {
    record_offset: u64,
    path: String,
}

#[derive(Default)]
struct PayloadLogState {
    path: Option<PathBuf>,
    text_references: HashMap<TextHash, TextReference>,
}

#[derive(Clone)]
pub(crate) struct PayloadCaptureContext {
    db: Arc<Database>,
    request_id: String,
    tag: &'static str,
    app_type: String,
    endpoint: String,
    provider_id: String,
    provider_name: String,
    session_id: String,
}

impl PayloadCaptureContext {
    pub(crate) fn from_request(state: &ProxyState, ctx: &RequestContext) -> Self {
        Self::from_parts(
            state.db.clone(),
            ctx.request_id.clone(),
            ctx.tag,
            ctx.app_type_str,
            &ctx.endpoint,
            &ctx.session_id,
            &ctx.provider,
        )
    }

    pub(crate) fn from_parts(
        db: Arc<Database>,
        request_id: String,
        tag: &'static str,
        app_type: &str,
        endpoint: &str,
        session_id: &str,
        provider: &Provider,
    ) -> Self {
        Self {
            db,
            request_id,
            tag,
            app_type: app_type.to_string(),
            endpoint: endpoint.to_string(),
            provider_id: provider.id.clone(),
            provider_name: provider.name.clone(),
            session_id: session_id.to_string(),
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.db
            .get_log_config()
            .map(|config| config.enabled && config.capture_payloads)
            .unwrap_or(false)
    }

    pub(crate) fn record_request(&self, body: &[u8]) {
        self.write_record("client_request", None, None, None, body);
    }

    /// 记录发往上游的请求正文；upstream_url 为脱敏后的上游目标 URL
    pub(crate) fn record_upstream_request(&self, upstream_url: &str, body: &[u8]) {
        self.write_record(
            "upstream_request",
            None,
            Some("application/json"),
            Some(upstream_url),
            body,
        );
    }

    pub(crate) fn record_bytes(
        &self,
        phase: &str,
        status: Option<u16>,
        content_type: Option<&str>,
        body: &[u8],
    ) {
        self.write_record(phase, status, content_type, None, body);
    }

    fn write_record(
        &self,
        phase: &str,
        status: Option<u16>,
        content_type: Option<&str>,
        upstream_url: Option<&str>,
        body: &[u8],
    ) {
        // 一次读取配置：开关判断与轮转参数共用，避免多次读库
        let config = match self.db.get_log_config() {
            Ok(config) if config.enabled && config.capture_payloads => config,
            _ => return,
        };

        let byte_len = body.len();
        let payload = summarize_body(body, content_type);
        let timestamp = chrono::Utc::now().to_rfc3339();
        let status_text = status
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string());
        let content_type_text = content_type.unwrap_or("-");
        let upstream_url = upstream_url.unwrap_or("-");

        let max_size = config.capture_max_size_mb.max(1) * MB;
        let build_record = |payload: &str| {
            format!(
                "[PAYLOAD] timestamp={timestamp} request_id={} phase={phase} tag={} app={} endpoint={} upstream_url={upstream_url} provider_id={} provider_name={} session_id={} status={status_text} content_type={content_type_text} bytes={}\n----- BEGIN PAYLOAD -----\n{}\n----- END PAYLOAD -----\n",
                self.request_id,
                self.tag,
                self.app_type,
                self.endpoint,
                self.provider_id,
                self.provider_name,
                self.session_id,
                byte_len,
                payload
            )
        };
        if let Err(error) = append_payload_record(
            &payload,
            content_type,
            max_size,
            config.capture_archives as usize,
            build_record,
        ) {
            log::warn!("[Payload] 写入正文日志失败: {error}");
        }
    }
}

fn payload_log_path() -> PathBuf {
    crate::panic_hook::get_log_dir().join(PAYLOAD_LOG_FILE_NAME)
}

fn summarize_body(body: &[u8], content_type: Option<&str>) -> String {
    if content_type.is_some_and(is_image_content_type) {
        return serde_json::to_string(&image_summary(body))
            .unwrap_or_else(|_| "<payload serialization failed>".to_string());
    }
    if content_type.is_some_and(is_event_stream_content_type) {
        return summarize_sse(body);
    }
    summarize_payload(body)
}

// 只重写明确的图片载荷；没有图片时保持原始正文，避免日志内容发生无关变化。
fn summarize_payload(body: &[u8]) -> String {
    let Ok(mut value) = serde_json::from_slice::<Value>(body) else {
        return String::from_utf8_lossy(body).into_owned();
    };
    if !summarize_images(&mut value) {
        return String::from_utf8_lossy(body).into_owned();
    }
    serde_json::to_string(&value).unwrap_or_else(|_| "<payload serialization failed>".to_string())
}

// SSE 只处理包含图片的 JSON data 行，其余行保持原样。
fn summarize_sse(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let mut out = String::with_capacity(text.len());
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&summarize_sse_data_line(line));
    }
    out
}

fn summarize_sse_data_line(line: &str) -> Cow<'_, str> {
    let without_cr = line.strip_suffix('\r').unwrap_or(line);
    let Some(after_data) = without_cr.strip_prefix("data:") else {
        return Cow::Borrowed(line);
    };
    let payload_text = after_data.strip_prefix(' ').unwrap_or(after_data);
    let Ok(mut parsed) = serde_json::from_str::<Value>(payload_text) else {
        return Cow::Borrowed(line);
    };
    if !summarize_images(&mut parsed) {
        return Cow::Borrowed(line);
    }
    match serde_json::to_string(&parsed) {
        Ok(serialized) => {
            let prefix_len = without_cr.len() - payload_text.len();
            let mut rebuilt = String::with_capacity(prefix_len + serialized.len() + 1);
            rebuilt.push_str(&without_cr[..prefix_len]);
            rebuilt.push_str(&serialized);
            if without_cr.len() != line.len() {
                rebuilt.push('\r');
            }
            Cow::Owned(rebuilt)
        }
        Err(_) => Cow::Borrowed(line),
    }
}

fn summarize_images(value: &mut Value) -> bool {
    let mut changed = summarize_image_api_response(value);
    changed |= summarize_nested_images(value);
    changed
}

fn summarize_image_api_response(value: &mut Value) -> bool {
    let Some(items) = value
        .as_object_mut()
        .and_then(|object| object.get_mut("data"))
        .and_then(Value::as_array_mut)
    else {
        return false;
    };

    let mut changed = false;
    for item in items {
        if let Some(object) = item.as_object_mut() {
            changed |= summarize_string_field(object, "b64_json");
        }
    }
    changed
}

fn summarize_nested_images(value: &mut Value) -> bool {
    match value {
        Value::String(text) if is_image_data_url(text) => {
            *value = image_summary(text.as_bytes());
            true
        }
        Value::Object(map) => {
            let mut changed = false;

            if object_image_media_type(map).is_some() {
                changed |= summarize_string_field(map, "data");
            }
            if map.get("type").and_then(Value::as_str) == Some("image") {
                if let Some(source) = map.get_mut("source").and_then(Value::as_object_mut) {
                    if source.get("type").and_then(Value::as_str) == Some("base64") {
                        changed |= summarize_string_field(source, "data");
                    }
                }
            }
            if map.get("type").and_then(Value::as_str) == Some("image_generation_call") {
                changed |= summarize_string_field(map, "result");
            }

            for child in map.values_mut() {
                changed |= summarize_nested_images(child);
            }
            changed
        }
        Value::Array(items) => items.iter_mut().fold(false, |changed, item| {
            summarize_nested_images(item) || changed
        }),
        _ => false,
    }
}

fn summarize_string_field(map: &mut Map<String, Value>, field: &str) -> bool {
    let Some(text) = map.get(field).and_then(Value::as_str) else {
        return false;
    };
    let summary = image_summary(text.as_bytes());
    map.insert(field.to_string(), summary);
    true
}

fn object_image_media_type(map: &Map<String, Value>) -> Option<&str> {
    for field in ["media_type", "mime_type", "mimeType"] {
        if let Some(value) = map.get(field).and_then(Value::as_str) {
            if is_image_content_type(value) {
                return Some(value);
            }
        }
    }
    None
}

fn is_image_content_type(value: &str) -> bool {
    value
        .split(';')
        .next()
        .is_some_and(|mime| mime.trim().to_ascii_lowercase().starts_with("image/"))
}

fn is_image_data_url(value: &str) -> bool {
    let Some((metadata, payload)) = value.split_once(',') else {
        return false;
    };
    let metadata = metadata.to_ascii_lowercase();
    metadata.starts_with("data:image/")
        && metadata.split(';').any(|part| part == "base64")
        && !payload.is_empty()
}

fn image_summary(bytes: &[u8]) -> Value {
    let mut summary = Map::new();
    summary.insert("__payload_summary__".into(), Value::String("image".into()));
    summary.insert("length".into(), Value::from(bytes.len()));
    summary.insert("sha256".into(), Value::String(sha256_hex(bytes)));
    Value::Object(summary)
}

fn sha256_hex(bytes: &[u8]) -> String {
    sha256_digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sha256_digest(bytes: &[u8]) -> TextHash {
    Sha256::digest(bytes).into()
}

// 只去重 JSON/SSE 中至少 1 KiB 的字符串。首次出现保留原文，后续引用同一文件内
// 首次记录的字节偏移和 JSON Pointer；普通文本正文不参与，避免改变未知协议。
fn deduplicate_repeated_text(
    payload: &str,
    content_type: Option<&str>,
    known: &HashMap<TextHash, TextReference>,
    record_offset: u64,
) -> (String, HashMap<TextHash, TextReference>) {
    if content_type.is_some_and(is_event_stream_content_type) {
        return deduplicate_sse_text(payload, known, record_offset);
    }

    let Ok(mut value) = serde_json::from_str::<Value>(payload) else {
        return (payload.to_string(), HashMap::new());
    };
    let mut new_references = HashMap::new();
    let changed = deduplicate_value(&mut value, "$", known, &mut new_references, record_offset);
    if !changed {
        return (payload.to_string(), new_references);
    }
    (
        serde_json::to_string(&value).unwrap_or_else(|_| payload.to_string()),
        new_references,
    )
}

fn deduplicate_sse_text(
    payload: &str,
    known: &HashMap<TextHash, TextReference>,
    record_offset: u64,
) -> (String, HashMap<TextHash, TextReference>) {
    let mut new_references = HashMap::new();
    let mut out = String::with_capacity(payload.len());
    for (index, line) in payload.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let path = format!("$sse/{}", index + 1);
        out.push_str(&deduplicate_sse_data_line(
            line,
            &path,
            known,
            &mut new_references,
            record_offset,
        ));
    }
    (out, new_references)
}

fn deduplicate_sse_data_line<'a>(
    line: &'a str,
    path: &str,
    known: &HashMap<TextHash, TextReference>,
    new_references: &mut HashMap<TextHash, TextReference>,
    record_offset: u64,
) -> Cow<'a, str> {
    let without_cr = line.strip_suffix('\r').unwrap_or(line);
    let Some(after_data) = without_cr.strip_prefix("data:") else {
        return Cow::Borrowed(line);
    };
    let payload_text = after_data.strip_prefix(' ').unwrap_or(after_data);
    let Ok(mut value) = serde_json::from_str::<Value>(payload_text) else {
        return Cow::Borrowed(line);
    };
    if !deduplicate_value(&mut value, path, known, new_references, record_offset) {
        return Cow::Borrowed(line);
    }
    let Ok(serialized) = serde_json::to_string(&value) else {
        return Cow::Borrowed(line);
    };
    let prefix_len = without_cr.len() - payload_text.len();
    let mut rebuilt = String::with_capacity(prefix_len + serialized.len() + 1);
    rebuilt.push_str(&without_cr[..prefix_len]);
    rebuilt.push_str(&serialized);
    if without_cr.len() != line.len() {
        rebuilt.push('\r');
    }
    Cow::Owned(rebuilt)
}

fn deduplicate_value(
    value: &mut Value,
    path: &str,
    known: &HashMap<TextHash, TextReference>,
    new_references: &mut HashMap<TextHash, TextReference>,
    record_offset: u64,
) -> bool {
    match value {
        Value::String(text) if text.len() >= REPEATED_TEXT_MIN_BYTES => {
            let length = text.len();
            let hash = sha256_digest(text.as_bytes());
            let reference = new_references
                .get(&hash)
                .or_else(|| known.get(&hash))
                .cloned();
            if let Some(reference) = reference {
                *value = repeated_text_summary(length, &hash, &reference);
                true
            } else {
                new_references.insert(
                    hash,
                    TextReference {
                        record_offset,
                        path: path.to_string(),
                    },
                );
                false
            }
        }
        Value::Object(map) => {
            let mut changed = false;
            for (key, child) in map {
                let child_path = format!("{path}/{}", json_pointer_token(key));
                changed |=
                    deduplicate_value(child, &child_path, known, new_references, record_offset);
            }
            changed
        }
        Value::Array(items) => {
            let mut changed = false;
            for (index, child) in items.iter_mut().enumerate() {
                let child_path = format!("{path}/{index}");
                changed |=
                    deduplicate_value(child, &child_path, known, new_references, record_offset);
            }
            changed
        }
        _ => false,
    }
}

fn json_pointer_token(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn repeated_text_summary(length: usize, hash: &TextHash, reference: &TextReference) -> Value {
    let mut summary = Map::new();
    summary.insert(
        "__payload_summary__".into(),
        Value::String("repeated_text".into()),
    );
    summary.insert("length".into(), Value::from(length));
    summary.insert(
        "sha256".into(),
        Value::String(hash.iter().map(|byte| format!("{byte:02x}")).collect()),
    );
    let mut reference_value = Map::new();
    reference_value.insert("record_offset".into(), Value::from(reference.record_offset));
    reference_value.insert("path".into(), Value::String(reference.path.clone()));
    summary.insert("ref".into(), Value::Object(reference_value));
    Value::Object(summary)
}

fn is_event_stream_content_type(value: &str) -> bool {
    value.contains("text/event-stream")
}

fn append_payload_record<F>(
    payload: &str,
    content_type: Option<&str>,
    max_size: u64,
    archives_to_keep: usize,
    build_record: F,
) -> std::io::Result<()>
where
    F: Fn(&str) -> String,
{
    let mut state = PAYLOAD_LOG_STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = payload_log_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut current_size = fs::metadata(&path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if state.path.as_ref() != Some(&path) || current_size == 0 {
        state.path = Some(path.clone());
        state.text_references.clear();
    }

    let (mut deduplicated, mut new_references) =
        deduplicate_repeated_text(payload, content_type, &state.text_references, current_size);
    let mut record = build_record(&deduplicated);
    if current_size > 0 && current_size.saturating_add(record.len() as u64) > max_size {
        rotate_payload_log(&path, archives_to_keep)?;
        current_size = 0;
        state.text_references.clear();
        // 新文件不能引用刚轮转的归档，清空引用后重新生成本条记录。
        (deduplicated, new_references) =
            deduplicate_repeated_text(payload, content_type, &state.text_references, current_size);
        record = build_record(&deduplicated);
    }

    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    file.write_all(record.as_bytes())?;
    file.flush()?;
    state.text_references.extend(new_references);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_responses_anthropic_and_gemini_inline_images() {
        let input = serde_json::json!({
            "input": [
                {"type": "input_image", "image_url": "data:image/png;base64,RESPONSES_IMAGE"},
                {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "abc"}},
                {"inlineData": {"mimeType": "image/webp", "data": "GEMINI_IMAGE"}}
            ]
        });
        let serialized = summarize_payload(input.to_string().as_bytes());
        let parsed: Value = serde_json::from_str(&serialized).unwrap();

        assert_eq!(
            parsed["input"][1]["source"]["data"],
            serde_json::json!({
                "__payload_summary__": "image",
                "length": 3,
                "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            })
        );
        assert_eq!(
            parsed["input"][2]["inlineData"]["data"]["__payload_summary__"],
            "image"
        );
        assert!(!serialized.contains("RESPONSES_IMAGE"));
        assert!(!serialized.contains("GEMINI_IMAGE"));
    }

    #[test]
    fn summarizes_image_api_and_image_generation_results() {
        let input = serde_json::json!({
            "data": [{"b64_json": "IMAGE_API_RESULT", "revised_prompt": "kept"}],
            "output": [{"type": "image_generation_call", "result": "GENERATED_IMAGE"}]
        });
        let summarized = summarize_payload(input.to_string().as_bytes());
        let parsed: Value = serde_json::from_str(&summarized).unwrap();

        assert_eq!(
            parsed["data"][0]["b64_json"]["__payload_summary__"],
            "image"
        );
        assert_eq!(parsed["data"][0]["revised_prompt"], "kept");
        assert_eq!(
            parsed["output"][0]["result"]["__payload_summary__"],
            "image"
        );
    }

    #[test]
    fn keeps_non_image_payloads_byte_identical() {
        let body = r#"{
  "tools": [{"parameters": {"properties": {"source": {"type": "string"}, "image": {"type": "string"}, "file": {"type": "string"}}}}],
  "content": [{"type": "image", "name": "business metadata"}],
  "image_url": "https://example.com/image.png",
  "encrypted_content": "gAAAAABciphertext",
  "text": "ordinary large text"
}"#;

        assert_eq!(summarize_payload(body.as_bytes()), body);
    }

    #[test]
    fn keeps_encrypted_reasoning_when_an_image_is_summarized() {
        let input = serde_json::json!({
            "type": "reasoning",
            "encrypted_content": "gAAAAABciphertext",
            "content": [{"type": "input_image", "image_url": "data:image/jpeg;base64,IMAGE"}]
        });
        let summarized = summarize_payload(input.to_string().as_bytes());
        let parsed: Value = serde_json::from_str(&summarized).unwrap();

        assert_eq!(parsed["encrypted_content"], "gAAAAABciphertext");
        assert_eq!(
            parsed["content"][0]["image_url"]["__payload_summary__"],
            "image"
        );
    }

    #[test]
    fn summarizes_images_inside_sse_and_keeps_other_lines() {
        let sse = "event: response.output_item.added\r\ndata: {\"type\":\"input_image\",\"image_url\":\"data:image/png;base64,SSE_IMAGE\",\"encrypted_content\":\"ciphertext\"}\r\n\r\nevent: done\r\ndata: [DONE]\r\n\r\n";
        let out = summarize_sse(sse.as_bytes());

        assert!(out.contains("event: response.output_item.added"));
        assert!(out.contains("data: [DONE]"));
        assert!(out.contains("\"encrypted_content\":\"ciphertext\""));
        assert!(out.contains("\"__payload_summary__\":\"image\""));
        assert!(!out.contains("SSE_IMAGE"));
    }

    #[test]
    fn keeps_sse_lines_without_summarizable_content_byte_identical() {
        let sse = "event:ping\ndata:{\"type\":\"ping\"}\n\nevent: message\ndata: {\"type\":\"message_start\",\"usage\":{\"input_tokens\":10}}\n\ndata:[DONE]\n\n";
        assert_eq!(summarize_sse(sse.as_bytes()), sse);
    }

    #[test]
    fn summarizes_raw_image_response_from_content_type() {
        let summarized = summarize_body(b"abc", Some("image/png; charset=binary"));
        let parsed: Value = serde_json::from_str(&summarized).unwrap();

        assert_eq!(parsed["__payload_summary__"], "image");
        assert_eq!(parsed["length"], 3);
        assert_eq!(
            parsed["sha256"],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn keeps_first_long_text_and_references_later_records() {
        let long_text = "x".repeat(REPEATED_TEXT_MIN_BYTES);
        let payload = serde_json::json!({"text": long_text}).to_string();
        let known = HashMap::new();

        let (first, first_references) =
            deduplicate_repeated_text(&payload, Some("application/json"), &known, 0);
        assert_eq!(first, payload);
        assert_eq!(first_references.len(), 1);

        let (second, second_references) =
            deduplicate_repeated_text(&payload, Some("application/json"), &first_references, 4096);
        let parsed: Value = serde_json::from_str(&second).unwrap();
        assert!(second_references.is_empty());
        assert_eq!(parsed["text"]["__payload_summary__"], "repeated_text");
        assert_eq!(parsed["text"]["length"], REPEATED_TEXT_MIN_BYTES);
        assert_eq!(parsed["text"]["sha256"].as_str().unwrap().len(), 64);
        assert_eq!(parsed["text"]["ref"]["record_offset"], 0);
        assert_eq!(parsed["text"]["ref"]["path"], "$/text");
    }

    #[test]
    fn references_repeated_long_text_inside_the_same_record() {
        let long_text = "y".repeat(REPEATED_TEXT_MIN_BYTES);
        let payload = serde_json::json!({
            "first": long_text,
            "nested": [{"second": long_text}]
        })
        .to_string();

        let (deduplicated, new_references) =
            deduplicate_repeated_text(&payload, Some("application/json"), &HashMap::new(), 512);
        let parsed: Value = serde_json::from_str(&deduplicated).unwrap();

        assert_eq!(parsed["first"], long_text);
        assert_eq!(
            parsed["nested"][0]["second"]["__payload_summary__"],
            "repeated_text"
        );
        assert_eq!(parsed["nested"][0]["second"]["ref"]["record_offset"], 512);
        assert_eq!(parsed["nested"][0]["second"]["ref"]["path"], "$/first");
        assert_eq!(new_references.len(), 1);
    }

    #[test]
    fn deduplicates_long_text_inside_sse_data_lines() {
        let long_text = "z".repeat(REPEATED_TEXT_MIN_BYTES);
        let sse = format!(
            "data: {{\"text\":\"{long_text}\"}}\n\ndata: {{\"text\":\"{long_text}\"}}\n\ndata: [DONE]\n\n"
        );

        let (deduplicated, new_references) =
            deduplicate_repeated_text(&sse, Some("text/event-stream"), &HashMap::new(), 128);

        assert_eq!(deduplicated.matches(&long_text).count(), 1);
        assert!(deduplicated.contains("\"__payload_summary__\":\"repeated_text\""));
        assert!(deduplicated.contains("\"record_offset\":128"));
        assert!(deduplicated.contains("\"path\":\"$sse/1/text\""));
        assert!(deduplicated.contains("data: [DONE]"));
        assert_eq!(new_references.len(), 1);
    }

    #[test]
    fn keeps_short_and_non_json_payloads_unchanged() {
        let short = serde_json::json!({"text": "short"}).to_string();
        let (short_result, short_references) =
            deduplicate_repeated_text(&short, Some("application/json"), &HashMap::new(), 0);
        assert_eq!(short_result, short);
        assert!(short_references.is_empty());

        let plain = "x".repeat(REPEATED_TEXT_MIN_BYTES * 2);
        let (plain_result, plain_references) =
            deduplicate_repeated_text(&plain, Some("text/plain"), &HashMap::new(), 0);
        assert_eq!(plain_result, plain);
        assert!(plain_references.is_empty());
    }
}

fn rotate_payload_log(path: &Path, archives_to_keep: usize) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S_%3f");
    let mut archive = parent.join(format!("cc-switch-payload_{timestamp}.log"));
    let mut suffix = 1u32;
    while archive.exists() {
        archive = parent.join(format!("cc-switch-payload_{timestamp}_{suffix}.log"));
        suffix = suffix.saturating_add(1);
    }
    fs::rename(path, archive)?;

    let mut archives: Vec<PathBuf> = fs::read_dir(parent)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|candidate| {
            candidate
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("cc-switch-payload_") && name.ends_with(".log"))
                .unwrap_or(false)
        })
        .collect();
    archives.sort();
    let keep_from = archives.len().saturating_sub(archives_to_keep);
    for old_archive in archives.into_iter().take(keep_from) {
        // 清理失败不阻塞本次写入，只告警
        if let Err(error) = fs::remove_file(&old_archive) {
            log::warn!(
                "[Payload] 清理旧正文日志失败 {}: {error}",
                old_archive.display()
            );
        }
    }
    Ok(())
}

pub(crate) fn capture_stream(
    stream: impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
    context: PayloadCaptureContext,
    phase: &'static str,
    status: u16,
    content_type: Option<String>,
) -> Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> {
    if !context.enabled() {
        return Box::pin(stream);
    }

    Box::pin(async_stream::stream! {
        let mut body = BytesMut::new();
        let mut stream_error = None;
        tokio::pin!(stream);

        while let Some(item) = stream.next().await {
            match &item {
                Ok(bytes) => body.extend_from_slice(bytes),
                Err(error) => stream_error = Some(error.to_string()),
            }
            yield item;
        }

        context.record_bytes(
            phase,
            Some(status),
            content_type.as_deref(),
            &body,
        );
        if let Some(error) = stream_error {
            log::warn!(
                "[PAYLOAD] request_id={} phase={phase} stream_error={error}",
                context.request_id
            );
        }
    })
}
