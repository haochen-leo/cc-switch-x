use super::media_sanitizer::UNSUPPORTED_IMAGE_MARKER;
use crate::database::Database;
use crate::provider::Provider;
use crate::proxy::http_client;
use crate::proxy::providers::{
    get_claude_api_format, get_codex_api_format, AuthStrategy, ClaudeAdapter, CodexAdapter,
    ProviderAdapter,
};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use http::{HeaderName, HeaderValue};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;

const OCR_PROMPT_VERSION: &str = "v2";
const OCR_PROMPT: &str =
    "详细描述你在图片中看到的内容，包括可辨认的文字、布局、颜色和状态；不要补充图片中没有的信息。";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaOcrProtocol {
    OpenAiChat,
    OpenAiResponses,
    AnthropicMessages,
}

impl MediaOcrProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiChat => "openai_chat",
            Self::OpenAiResponses => "openai_responses",
            Self::AnthropicMessages => "anthropic",
        }
    }

    fn endpoint(self) -> &'static str {
        match self {
            Self::OpenAiChat => "/v1/chat/completions",
            Self::OpenAiResponses => "/v1/responses",
            Self::AnthropicMessages => "/v1/messages",
        }
    }
}

pub fn collect_image_urls(body: &Value) -> Vec<String> {
    let mut urls = Vec::new();
    collect_image_urls_in_value(body, &mut urls);
    urls
}

pub fn replace_images_with_ocr_results(body: &mut Value, results: &[Option<String>]) -> usize {
    let mut index = 0usize;
    replace_images_in_value(body, results, &mut index)
}

pub async fn transcribe_images(
    db: &Database,
    provider_app_type: &str,
    provider: &Provider,
    model: &str,
    image_urls: &[String],
    timeout: Duration,
) -> Result<(MediaOcrProtocol, Vec<Option<String>>, usize), String> {
    let protocol = resolve_ocr_protocol(provider_app_type, provider)?;
    let adapter: Box<dyn ProviderAdapter> = match provider_app_type {
        "claude" => Box::new(ClaudeAdapter::new()),
        "codex" => Box::new(CodexAdapter::new()),
        other => return Err(format!("不支持的 OCR 供应商类型: {other}")),
    };
    let base_url = adapter
        .extract_base_url(provider)
        .map_err(|error| error.to_string())?;
    let auth = adapter
        .extract_auth(provider)
        .ok_or_else(|| "OCR 供应商缺少 API Key".to_string())?;
    if matches!(
        auth.strategy,
        AuthStrategy::GitHubCopilot | AuthStrategy::CodexOAuth | AuthStrategy::XaiOAuth
    ) {
        return Err("OCR 暂不支持需要动态 OAuth 令牌的供应商".to_string());
    }
    let auth_headers = adapter
        .get_auth_headers(&auth)
        .map_err(|error| error.to_string())?;
    let url = build_ocr_url(adapter.as_ref(), provider, &base_url, protocol);
    let mut results = Vec::with_capacity(image_urls.len());
    let mut cache_hits = 0usize;

    for image_url in image_urls {
        let cache_identity = media_ocr_cache_key(
            provider_app_type,
            &provider.id,
            model,
            OCR_PROMPT_VERSION,
            image_url,
        );
        if let Some((cache_key, _)) = cache_identity.as_ref() {
            match db.get_media_ocr_cache(cache_key) {
                Ok(Some(text)) => {
                    cache_hits += 1;
                    results.push(Some(text));
                    continue;
                }
                Ok(None) => {}
                Err(error) => {
                    log::warn!(
                        "[Media OCR] cache read failed provider={} model={}: {}",
                        provider.id,
                        model,
                        error
                    );
                }
            }
        }

        match transcribe_image(&url, &auth_headers, protocol, model, image_url, timeout).await {
            Ok(text) => {
                if let Some((cache_key, image_hash)) = cache_identity {
                    if let Err(error) = db.put_media_ocr_cache(
                        &cache_key,
                        &image_hash,
                        provider_app_type,
                        &provider.id,
                        model,
                        OCR_PROMPT_VERSION,
                        &text,
                    ) {
                        log::warn!(
                            "[Media OCR] cache write failed provider={} model={}: {}",
                            provider.id,
                            model,
                            error
                        );
                    }
                }
                results.push(Some(text));
            }
            Err(error) => {
                log::warn!(
                    "[Media OCR] provider={} app_type={} protocol={} model={} failed: {}",
                    provider.id,
                    provider_app_type,
                    protocol.as_str(),
                    model,
                    error
                );
                results.push(None);
            }
        }
    }

    Ok((protocol, results, cache_hits))
}

fn media_ocr_cache_key(
    provider_app_type: &str,
    provider_id: &str,
    model: &str,
    prompt_version: &str,
    image_url: &str,
) -> Option<(String, String)> {
    let (_, data) = image_url
        .strip_prefix("data:")?
        .split_once(',')
        .filter(|(metadata, _)| metadata.ends_with(";base64"))?;
    let bytes = BASE64_STANDARD.decode(data).ok()?;

    let mut image_hasher = Sha256::new();
    image_hasher.update(bytes);
    let image_hash = format!("{:x}", image_hasher.finalize());

    let mut cache_hasher = Sha256::new();
    for part in [
        provider_app_type,
        provider_id,
        model,
        prompt_version,
        image_hash.as_str(),
    ] {
        cache_hasher.update(part.as_bytes());
        cache_hasher.update(b"\0");
    }
    Some((format!("{:x}", cache_hasher.finalize()), image_hash))
}

#[cfg(test)]
mod cache_key_tests {
    use super::media_ocr_cache_key;

    #[test]
    fn decoded_image_bytes_define_the_image_hash() {
        let (_, png_hash) = media_ocr_cache_key(
            "codex",
            "provider",
            "vision",
            "v1",
            "data:image/png;base64,aGVsbG8=",
        )
        .unwrap();
        let (_, jpeg_hash) = media_ocr_cache_key(
            "codex",
            "provider",
            "vision",
            "v1",
            "data:image/jpeg;base64,aGVsbG8=",
        )
        .unwrap();

        assert_eq!(png_hash, jpeg_hash);
        assert_eq!(
            png_hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn provider_model_and_prompt_version_are_part_of_the_cache_key() {
        let (baseline, _) = media_ocr_cache_key(
            "codex",
            "provider-a",
            "vision-a",
            "v1",
            "data:image/png;base64,aGVsbG8=",
        )
        .unwrap();

        for changed in [
            media_ocr_cache_key(
                "claude",
                "provider-a",
                "vision-a",
                "v1",
                "data:image/png;base64,aGVsbG8=",
            )
            .unwrap()
            .0,
            media_ocr_cache_key(
                "codex",
                "provider-b",
                "vision-a",
                "v1",
                "data:image/png;base64,aGVsbG8=",
            )
            .unwrap()
            .0,
            media_ocr_cache_key(
                "codex",
                "provider-a",
                "vision-b",
                "v1",
                "data:image/png;base64,aGVsbG8=",
            )
            .unwrap()
            .0,
            media_ocr_cache_key(
                "codex",
                "provider-a",
                "vision-a",
                "v2",
                "data:image/png;base64,aGVsbG8=",
            )
            .unwrap()
            .0,
        ] {
            assert_ne!(baseline, changed);
        }
    }

    #[test]
    fn remote_and_invalid_data_urls_are_not_cached() {
        assert!(media_ocr_cache_key(
            "codex",
            "provider",
            "vision",
            "v1",
            "https://example.com/image.png"
        )
        .is_none());
        assert!(media_ocr_cache_key(
            "codex",
            "provider",
            "vision",
            "v1",
            "data:image/png;base64,not-valid-base64!"
        )
        .is_none());
    }
}

async fn transcribe_image(
    url: &str,
    auth_headers: &[(HeaderName, HeaderValue)],
    protocol: MediaOcrProtocol,
    model: &str,
    image_url: &str,
    timeout: Duration,
) -> Result<String, String> {
    let payload = build_ocr_payload(protocol, model, image_url)?;

    let mut request = http_client::get().post(url).json(&payload);
    for (name, value) in auth_headers {
        request = request.header(name, value);
    }
    if protocol == MediaOcrProtocol::AnthropicMessages {
        request = request.header("anthropic-version", "2023-06-01");
    }

    let response = tokio::time::timeout(timeout, request.send())
        .await
        .map_err(|_| format!("OCR 请求超过 {} 秒", timeout.as_secs()))?
        .map_err(|error| format!("OCR 请求发送失败: {error}"))?;
    let status = response.status();
    let response_text = tokio::time::timeout(timeout, response.text())
        .await
        .map_err(|_| format!("OCR 响应读取超过 {} 秒", timeout.as_secs()))?
        .map_err(|error| format!("OCR 响应读取失败: {error}"))?;

    if !status.is_success() {
        return Err(format!(
            "OCR 上游返回 HTTP {}: {}",
            status.as_u16(),
            truncate_for_log(&response_text)
        ));
    }

    let response_json: Value = serde_json::from_str(&response_text)
        .map_err(|error| format!("OCR 响应不是有效 JSON: {error}"))?;
    extract_ocr_text(&response_json, protocol).ok_or_else(|| "OCR 响应缺少文本内容".to_string())
}

pub fn resolve_ocr_protocol(
    provider_app_type: &str,
    provider: &Provider,
) -> Result<MediaOcrProtocol, String> {
    let api_format = match provider_app_type {
        "claude" => Some(get_claude_api_format(provider)),
        "codex" => get_codex_api_format(provider),
        other => return Err(format!("不支持的 OCR 供应商类型: {other}")),
    }
    .ok_or_else(|| "OCR 供应商未配置明确的 API 协议".to_string())?;

    match api_format {
        "openai_chat" => Ok(MediaOcrProtocol::OpenAiChat),
        "openai_responses" => Ok(MediaOcrProtocol::OpenAiResponses),
        "anthropic" => Ok(MediaOcrProtocol::AnthropicMessages),
        other => Err(format!("OCR 暂不支持供应商协议: {other}")),
    }
}

fn build_ocr_url(
    adapter: &dyn ProviderAdapter,
    provider: &Provider,
    base_url: &str,
    protocol: MediaOcrProtocol,
) -> String {
    let endpoint = protocol.endpoint();
    let base_url = base_url.trim_end_matches('/');
    let configured_as_full_url = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.is_full_url)
        .unwrap_or(false);
    if configured_as_full_url || base_url.to_ascii_lowercase().ends_with(endpoint) {
        base_url.to_string()
    } else {
        adapter.build_url(base_url, endpoint)
    }
}

fn build_ocr_payload(
    protocol: MediaOcrProtocol,
    model: &str,
    image_url: &str,
) -> Result<Value, String> {
    match protocol {
        MediaOcrProtocol::OpenAiChat => Ok(json!({
            "model": model,
            "stream": false,
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "text", "text": OCR_PROMPT },
                    { "type": "image_url", "image_url": { "url": image_url } }
                ]
            }]
        })),
        MediaOcrProtocol::OpenAiResponses => Ok(json!({
            "model": model,
            "stream": false,
            "input": [{
                "role": "user",
                "content": [
                    { "type": "input_text", "text": OCR_PROMPT },
                    { "type": "input_image", "image_url": image_url }
                ]
            }]
        })),
        MediaOcrProtocol::AnthropicMessages => Ok(json!({
            "model": model,
            "max_tokens": 4096,
            "stream": false,
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "text", "text": OCR_PROMPT },
                    { "type": "image", "source": anthropic_image_source(image_url)? }
                ]
            }]
        })),
    }
}

fn anthropic_image_source(image_url: &str) -> Result<Value, String> {
    if let Some(data_url) = image_url.strip_prefix("data:") {
        let (metadata, data) = data_url
            .split_once(',')
            .ok_or_else(|| "图片 data URL 格式无效".to_string())?;
        let (media_type, encoding) = metadata
            .split_once(';')
            .ok_or_else(|| "图片 data URL 缺少编码声明".to_string())?;
        if !encoding.eq_ignore_ascii_case("base64") || !media_type.starts_with("image/") {
            return Err("Anthropic OCR 仅支持 base64 图片 data URL".to_string());
        }
        return Ok(json!({
            "type": "base64",
            "media_type": media_type,
            "data": data
        }));
    }

    Ok(json!({ "type": "url", "url": image_url }))
}

fn extract_ocr_text(response: &Value, protocol: MediaOcrProtocol) -> Option<String> {
    if let Some(text) = response
        .pointer("/choices/0/message/ocr_result/processed_text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        return Some(text.to_string());
    }

    let mut texts = Vec::new();
    match protocol {
        MediaOcrProtocol::OpenAiChat => {
            collect_content_text(response.pointer("/choices/0/message/content"), &mut texts);
        }
        MediaOcrProtocol::OpenAiResponses => {
            collect_content_text(response.get("output_text"), &mut texts);
            if let Some(output) = response.get("output").and_then(Value::as_array) {
                for item in output {
                    collect_content_text(item.get("content"), &mut texts);
                }
            }
        }
        MediaOcrProtocol::AnthropicMessages => {
            collect_content_text(response.get("content"), &mut texts);
        }
    }

    let text = texts.join("\n");
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn collect_content_text(value: Option<&Value>, texts: &mut Vec<String>) {
    match value {
        Some(Value::String(text)) => {
            let text = text.trim();
            if !text.is_empty() {
                texts.push(text.to_string());
            }
        }
        Some(Value::Array(parts)) => {
            for part in parts {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    let text = text.trim();
                    if !text.is_empty() {
                        texts.push(text.to_string());
                    }
                }
            }
        }
        _ => {}
    }
}

fn collect_image_urls_in_value(value: &Value, urls: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_image_urls_in_value(item, urls);
            }
        }
        Value::Object(object) => {
            if is_image_block(value) {
                if let Some(url) = extract_image_url(value) {
                    urls.push(url);
                }
                return;
            }

            for child in object.values() {
                collect_image_urls_in_value(child, urls);
            }
        }
        _ => {}
    }
}

fn replace_images_in_value(
    value: &mut Value,
    results: &[Option<String>],
    index: &mut usize,
) -> usize {
    let image_block = is_image_block(value);
    let image_url = image_block.then(|| extract_image_url(value)).flatten();
    match value {
        Value::Array(items) => items
            .iter_mut()
            .map(|item| replace_images_in_value(item, results, index))
            .sum(),
        Value::Object(object) => {
            if image_block {
                let result = if image_url.is_some() {
                    let result = results.get(*index).and_then(Option::as_ref);
                    *index += 1;
                    result
                } else {
                    None
                };
                replace_image_block(value, result);
                return 1;
            }

            object
                .values_mut()
                .map(|child| replace_images_in_value(child, results, index))
                .sum()
        }
        _ => 0,
    }
}

fn is_image_block(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };

    if matches!(
        object.get("type").and_then(Value::as_str),
        Some("image" | "image_url" | "input_image")
    ) {
        return true;
    }

    object
        .get("inlineData")
        .or_else(|| object.get("inline_data"))
        .and_then(Value::as_object)
        .and_then(|inline| {
            inline
                .get("mimeType")
                .or_else(|| inline.get("mime_type"))
                .and_then(Value::as_str)
        })
        .is_some_and(|mime_type| mime_type.starts_with("image/"))
}

fn extract_image_url(block: &Value) -> Option<String> {
    let block_type = block.get("type").and_then(Value::as_str);

    if matches!(block_type, Some("image_url" | "input_image")) {
        return extract_url_field(block.get("image_url"));
    }

    if block_type == Some("image") {
        if let Some(source) = block.get("source") {
            if let Some(url) = extract_url_field(source.get("url")) {
                return Some(url);
            }

            let data = source.get("data").and_then(Value::as_str)?.trim();
            if data.is_empty() {
                return None;
            }
            let media_type = source
                .get("media_type")
                .or_else(|| source.get("mediaType"))
                .and_then(Value::as_str)
                .unwrap_or("image/png");
            return Some(format!("data:{media_type};base64,{data}"));
        }

        if let Some(url) = extract_url_field(block.get("image_url")) {
            return Some(url);
        }

        let data = block.get("data").and_then(Value::as_str)?.trim();
        if data.is_empty() {
            return None;
        }
        let media_type = block
            .get("mimeType")
            .or_else(|| block.get("mime_type"))
            .and_then(Value::as_str)
            .unwrap_or("image/png");
        return Some(format!("data:{media_type};base64,{data}"));
    }

    let inline = block
        .get("inlineData")
        .or_else(|| block.get("inline_data"))?;
    let data = inline.get("data").and_then(Value::as_str)?.trim();
    if data.is_empty() {
        return None;
    }
    let media_type = inline
        .get("mimeType")
        .or_else(|| inline.get("mime_type"))
        .and_then(Value::as_str)
        .unwrap_or("image/png");
    Some(format!("data:{media_type};base64,{data}"))
}

fn extract_url_field(value: Option<&Value>) -> Option<String> {
    let value = value?;
    let url = match value {
        Value::String(url) => Some(url.as_str()),
        Value::Object(object) => object.get("url").and_then(Value::as_str),
        _ => None,
    }?;
    let url = url.trim();
    (!url.is_empty()).then(|| url.to_string())
}

fn replace_image_block(block: &mut Value, ocr_text: Option<&String>) {
    let text = ocr_text
        .map(|text| text.trim())
        .filter(|text| !text.is_empty())
        .map(|text| format!("<image_ocr>\n{text}\n</image_ocr>"))
        .unwrap_or_else(|| UNSUPPORTED_IMAGE_MARKER.to_string());

    let text_type = match block.get("type").and_then(Value::as_str) {
        Some("input_image") => Some("input_text"),
        Some(_) => Some("text"),
        None => None,
    };
    let cache_control = block.get("cache_control").cloned();

    *block = match text_type {
        Some(text_type) => json!({ "type": text_type, "text": text }),
        None => json!({ "text": text }),
    };

    if let (Some(cache_control), Some(object)) = (cache_control, block.as_object_mut()) {
        object.insert("cache_control".to_string(), cache_control);
    }
}

fn truncate_for_log(text: &str) -> String {
    const LIMIT: usize = 300;
    if text.len() <= LIMIT {
        return text.to_string();
    }

    let mut boundary = LIMIT;
    while !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}...", &text[..boundary])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_and_replaces_chat_and_responses_images() {
        let mut body = json!({
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/png",
                        "data": "abc"
                    }
                }]
            }],
            "input": [{
                "role": "user",
                "content": [{
                    "type": "input_image",
                    "image_url": "data:image/png;base64,def"
                }]
            }]
        });

        let urls = collect_image_urls(&body);
        assert_eq!(
            urls,
            vec![
                "data:image/png;base64,abc".to_string(),
                "data:image/png;base64,def".to_string()
            ]
        );

        let replaced = replace_images_with_ocr_results(
            &mut body,
            &[Some("第一页".to_string()), Some("第二页".to_string())],
        );

        assert_eq!(replaced, 2);
        assert_eq!(body["messages"][0]["content"][0]["type"], "text");
        assert_eq!(
            body["messages"][0]["content"][0]["text"],
            "<image_ocr>\n第一页\n</image_ocr>"
        );
        assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(
            body["input"][0]["content"][0]["text"],
            "<image_ocr>\n第二页\n</image_ocr>"
        );
    }

    #[test]
    fn replacement_falls_back_to_marker_when_ocr_failed() {
        let mut body = json!({
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "image_url",
                    "image_url": { "url": "data:image/png;base64,abc" },
                    "cache_control": { "type": "ephemeral" }
                }]
            }]
        });

        let replaced = replace_images_with_ocr_results(&mut body, &[None]);

        assert_eq!(replaced, 1);
        assert_eq!(body["messages"][0]["content"][0]["type"], "text");
        assert_eq!(
            body["messages"][0]["content"][0]["text"],
            UNSUPPORTED_IMAGE_MARKER
        );
        assert_eq!(
            body["messages"][0]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn extracts_qwen_ocr_result_and_plain_content() {
        let ocr_response = json!({
            "choices": [{
                "message": {
                    "content": "```json\n{\"result\":\"\"}\n```",
                    "ocr_result": { "processed_text": "识别文本" }
                }
            }]
        });
        assert_eq!(
            extract_ocr_text(&ocr_response, MediaOcrProtocol::OpenAiChat),
            Some("识别文本".to_string())
        );

        let plain_response = json!({
            "choices": [{ "message": { "content": "图片描述" } }]
        });
        assert_eq!(
            extract_ocr_text(&plain_response, MediaOcrProtocol::OpenAiChat),
            Some("图片描述".to_string())
        );
    }

    #[test]
    fn builds_payload_for_each_configured_protocol() {
        let image_url = "data:image/png;base64,abc";
        let chat = build_ocr_payload(MediaOcrProtocol::OpenAiChat, "vision", image_url).unwrap();
        assert_eq!(
            chat.pointer("/messages/0/content/1/type")
                .and_then(Value::as_str),
            Some("image_url")
        );

        let responses =
            build_ocr_payload(MediaOcrProtocol::OpenAiResponses, "vision", image_url).unwrap();
        assert_eq!(
            responses
                .pointer("/input/0/content/1/type")
                .and_then(Value::as_str),
            Some("input_image")
        );

        let anthropic =
            build_ocr_payload(MediaOcrProtocol::AnthropicMessages, "vision", image_url).unwrap();
        assert_eq!(
            anthropic
                .pointer("/messages/0/content/1/source/type")
                .and_then(Value::as_str),
            Some("base64")
        );
        assert_eq!(
            anthropic
                .pointer("/messages/0/content/1/source/media_type")
                .and_then(Value::as_str),
            Some("image/png")
        );
    }

    #[test]
    fn extracts_responses_and_anthropic_text() {
        let responses = json!({
            "output": [{
                "type": "message",
                "content": [{ "type": "output_text", "text": "Responses 识别" }]
            }]
        });
        assert_eq!(
            extract_ocr_text(&responses, MediaOcrProtocol::OpenAiResponses),
            Some("Responses 识别".to_string())
        );

        let anthropic = json!({
            "content": [{ "type": "text", "text": "Anthropic 识别" }]
        });
        assert_eq!(
            extract_ocr_text(&anthropic, MediaOcrProtocol::AnthropicMessages),
            Some("Anthropic 识别".to_string())
        );
    }

    #[test]
    fn resolves_protocol_from_provider_app_type_and_config() {
        let codex_chat = Provider::with_id(
            "codex-chat".to_string(),
            "Codex Chat".to_string(),
            json!({ "apiFormat": "openai_chat" }),
            None,
        );
        assert_eq!(
            resolve_ocr_protocol("codex", &codex_chat),
            Ok(MediaOcrProtocol::OpenAiChat)
        );

        let codex_responses = Provider::with_id(
            "codex-responses".to_string(),
            "Codex Responses".to_string(),
            json!({ "config": "wire_api = \"responses\"" }),
            None,
        );
        assert_eq!(
            resolve_ocr_protocol("codex", &codex_responses),
            Ok(MediaOcrProtocol::OpenAiResponses)
        );

        let claude = Provider::with_id(
            "claude".to_string(),
            "Claude".to_string(),
            json!({ "api_format": "anthropic" }),
            None,
        );
        assert_eq!(
            resolve_ocr_protocol("claude", &claude),
            Ok(MediaOcrProtocol::AnthropicMessages)
        );

        let unknown = Provider::with_id(
            "unknown".to_string(),
            "Unknown".to_string(),
            json!({}),
            None,
        );
        assert!(resolve_ocr_protocol("codex", &unknown).is_err());
    }
}
