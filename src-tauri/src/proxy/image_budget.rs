use crate::proxy::error::ProxyError;
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD},
    Engine as _,
};
use image::{codecs::jpeg::JpegEncoder, imageops::FilterType, GenericImageView};
use serde_json::{json, Map, Value};

pub(crate) const CODEX_THIRD_PARTY_IMAGE_BUDGET_BYTES: usize = 5_900_000;
pub(crate) const MAX_DOWNSAMPLED_IMAGE_EDGE: u32 = 1_600;

const JPEG_QUALITY: u8 = 80;
const REMOVED_IMAGE_MARKER: &str = "[Image removed: request body byte budget exceeded]";

pub(crate) struct ImageBudgetResult {
    pub(crate) body_bytes: Vec<u8>,
    pub(crate) original_bytes: usize,
    pub(crate) downsampled_images: usize,
    pub(crate) removed_images: usize,
    pub(crate) budget_bytes: usize,
}

impl ImageBudgetResult {
    pub(crate) fn final_bytes(&self) -> usize {
        self.body_bytes.len()
    }

    pub(crate) fn changed(&self) -> bool {
        self.downsampled_images > 0 || self.removed_images > 0
    }
}

pub(crate) fn serialize_with_image_budget(
    body: &mut Value,
    budget_bytes: usize,
) -> Result<ImageBudgetResult, ProxyError> {
    let original_body_bytes = serialize_body(body)?;
    let original_bytes = original_body_bytes.len();

    if original_bytes <= budget_bytes {
        return Ok(ImageBudgetResult {
            body_bytes: original_body_bytes,
            original_bytes,
            downsampled_images: 0,
            removed_images: 0,
            budget_bytes,
        });
    }

    let downsampled_images = downsample_inline_images(body);
    let mut body_bytes = if downsampled_images > 0 {
        serialize_body(body)?
    } else {
        original_body_bytes
    };

    let mut removed_images = 0usize;
    while body_bytes.len() > budget_bytes && replace_oldest_inline_image_block(body) {
        removed_images += 1;
        body_bytes = serialize_body(body)?;
    }

    Ok(ImageBudgetResult {
        body_bytes,
        original_bytes,
        downsampled_images,
        removed_images,
        budget_bytes,
    })
}

fn serialize_body(body: &Value) -> Result<Vec<u8>, ProxyError> {
    serde_json::to_vec(body)
        .map_err(|e| ProxyError::Internal(format!("Failed to serialize request body: {e}")))
}

fn downsample_inline_images(value: &mut Value) -> usize {
    match value {
        Value::Array(items) => items.iter_mut().map(downsample_inline_images).sum(),
        Value::Object(object) => {
            let mut changed =
                downsample_image_url_field(object) + downsample_mime_data_fields(object);
            for child in object.values_mut() {
                changed += downsample_inline_images(child);
            }
            changed
        }
        _ => 0,
    }
}

fn downsample_image_url_field(object: &mut Map<String, Value>) -> usize {
    let Some(image_url) = object.get_mut("image_url") else {
        return 0;
    };

    if let Some(url) = image_url.as_str() {
        if let Some(rewritten) = downsample_data_url(url) {
            *image_url = Value::String(rewritten);
            return 1;
        }
        return 0;
    }

    let Some(image_url_object) = image_url.as_object_mut() else {
        return 0;
    };
    let Some(url) = image_url_object.get("url").and_then(Value::as_str) else {
        return 0;
    };
    let Some(rewritten) = downsample_data_url(url) else {
        return 0;
    };

    image_url_object.insert("url".to_string(), Value::String(rewritten));
    1
}

fn downsample_mime_data_fields(object: &mut Map<String, Value>) -> usize {
    let Some((mime_key, media_type)) = object_image_media_type(object) else {
        return 0;
    };
    let Some(data) = object.get("data").and_then(Value::as_str) else {
        return 0;
    };
    let Some(rewritten_data) = downsample_base64_image(media_type, data) else {
        return 0;
    };

    object.insert(
        mime_key.to_string(),
        Value::String("image/jpeg".to_string()),
    );
    object.insert("data".to_string(), Value::String(rewritten_data));
    1
}

fn object_image_media_type(object: &Map<String, Value>) -> Option<(&'static str, &str)> {
    for key in ["media_type", "mimeType", "mime_type"] {
        let Some(value) = object.get(key).and_then(Value::as_str) else {
            continue;
        };
        if media_type_is_image(value) {
            return Some((key, value));
        }
    }

    None
}

fn downsample_data_url(url: &str) -> Option<String> {
    let data_url = parse_image_data_url(url)?;
    let rewritten_data = downsample_base64_image(data_url.media_type, data_url.payload)?;
    Some(format!("data:image/jpeg;base64,{rewritten_data}"))
}

fn downsample_base64_image(media_type: &str, payload: &str) -> Option<String> {
    if !media_type_is_image(media_type) {
        return None;
    }

    let image_bytes = decode_base64_payload(payload.trim())?;
    let image = image::load_from_memory(&image_bytes).ok()?;
    let (width, height) = image.dimensions();
    if width <= MAX_DOWNSAMPLED_IMAGE_EDGE && height <= MAX_DOWNSAMPLED_IMAGE_EDGE {
        return None;
    }

    let resized = image.resize(
        MAX_DOWNSAMPLED_IMAGE_EDGE,
        MAX_DOWNSAMPLED_IMAGE_EDGE,
        FilterType::Lanczos3,
    );
    let mut encoded = Vec::new();
    JpegEncoder::new_with_quality(&mut encoded, JPEG_QUALITY)
        .encode_image(&resized)
        .ok()?;

    let rewritten = STANDARD.encode(encoded);
    if rewritten.len() < payload.trim().len() {
        Some(rewritten)
    } else {
        None
    }
}

fn decode_base64_payload(payload: &str) -> Option<Vec<u8>> {
    STANDARD
        .decode(payload)
        .or_else(|_| URL_SAFE.decode(payload))
        .or_else(|_| URL_SAFE_NO_PAD.decode(payload))
        .ok()
}

fn replace_oldest_inline_image_block(value: &mut Value) -> bool {
    if let Value::Object(object) = value {
        for key in ["messages", "input", "contents"] {
            if let Some(child) = object.get_mut(key) {
                if replace_oldest_inline_image_block_generic(child) {
                    return true;
                }
            }
        }
    }

    replace_oldest_inline_image_block_generic(value)
}

fn replace_oldest_inline_image_block_generic(value: &mut Value) -> bool {
    match value {
        Value::Array(items) => items
            .iter_mut()
            .any(replace_oldest_inline_image_block_generic),
        Value::Object(object) => {
            if let Some(replacement_kind) = inline_image_replacement_kind(object) {
                replace_image_block(value, replacement_kind);
                return true;
            }

            for child in object.values_mut() {
                if replace_oldest_inline_image_block_generic(child) {
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

#[derive(Clone, Copy)]
enum ReplacementKind {
    Text,
    InputText,
    GeminiText,
}

fn inline_image_replacement_kind(object: &Map<String, Value>) -> Option<ReplacementKind> {
    match object.get("type").and_then(Value::as_str) {
        Some("input_image") if image_url_field_has_inline_image(object) => {
            Some(ReplacementKind::InputText)
        }
        Some("image_url") if image_url_field_has_inline_image(object) => {
            Some(ReplacementKind::Text)
        }
        Some("image") if typed_image_block_has_inline_payload(object) => {
            Some(ReplacementKind::Text)
        }
        _ if gemini_part_has_inline_image(object) => Some(ReplacementKind::GeminiText),
        _ => None,
    }
}

fn typed_image_block_has_inline_payload(object: &Map<String, Value>) -> bool {
    mime_data_fields_are_inline_image(object)
        || object
            .get("source")
            .and_then(Value::as_object)
            .is_some_and(source_has_inline_image)
}

fn source_has_inline_image(source: &Map<String, Value>) -> bool {
    mime_data_fields_are_inline_image(source)
        || source
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(data_url_is_image)
        || (source.get("type").and_then(Value::as_str) == Some("base64")
            && source.get("data").and_then(Value::as_str).is_some())
}

fn image_url_field_has_inline_image(object: &Map<String, Value>) -> bool {
    let Some(image_url) = object.get("image_url") else {
        return false;
    };

    image_url.as_str().is_some_and(data_url_is_image)
        || image_url
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(data_url_is_image)
}

fn gemini_part_has_inline_image(object: &Map<String, Value>) -> bool {
    object
        .get("inlineData")
        .or_else(|| object.get("inline_data"))
        .and_then(Value::as_object)
        .is_some_and(mime_data_fields_are_inline_image)
}

fn mime_data_fields_are_inline_image(object: &Map<String, Value>) -> bool {
    object_image_media_type(object).is_some()
        && object.get("data").and_then(Value::as_str).is_some()
}

fn replace_image_block(value: &mut Value, replacement_kind: ReplacementKind) {
    let cache_control = value.get("cache_control").cloned();
    *value = match replacement_kind {
        ReplacementKind::Text => json!({
            "type": "text",
            "text": REMOVED_IMAGE_MARKER
        }),
        ReplacementKind::InputText => json!({
            "type": "input_text",
            "text": REMOVED_IMAGE_MARKER
        }),
        ReplacementKind::GeminiText => json!({
            "text": REMOVED_IMAGE_MARKER
        }),
    };

    if let (Some(cache_control), Some(object)) = (cache_control, value.as_object_mut()) {
        object.insert("cache_control".to_string(), cache_control);
    }
}

fn data_url_is_image(value: &str) -> bool {
    parse_image_data_url(value).is_some()
}

struct ImageDataUrl<'a> {
    media_type: &'a str,
    payload: &'a str,
}

fn parse_image_data_url(value: &str) -> Option<ImageDataUrl<'_>> {
    let (metadata, payload) = value.split_once(',')?;
    let media_type = metadata
        .strip_prefix("data:")?
        .split(';')
        .next()
        .filter(|media_type| media_type_is_image(media_type))?;
    if !metadata
        .split(';')
        .any(|part| part.eq_ignore_ascii_case("base64"))
    {
        return None;
    }
    if payload.is_empty() {
        return None;
    }

    Some(ImageDataUrl {
        media_type,
        payload,
    })
}

fn media_type_is_image(media_type: &str) -> bool {
    media_type
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("image/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{codecs::png::PngEncoder, ColorType, ImageBuffer, ImageEncoder, Rgb};
    use serde_json::json;

    fn noisy_png_base64(width: u32, height: u32) -> String {
        let image = ImageBuffer::from_fn(width, height, |x, y| {
            Rgb([
                ((x * 17 + y * 3) % 256) as u8,
                ((x * 5 + y * 29) % 256) as u8,
                ((x * 11 + y * 7) % 256) as u8,
            ])
        });
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(image.as_raw(), width, height, ColorType::Rgb8.into())
            .expect("encode png");
        STANDARD.encode(bytes)
    }

    #[test]
    fn downsamples_large_anthropic_image_before_dropping() {
        let png = noisy_png_base64(1_900, 1_200);
        let mut body = json!({
            "model": "qwen",
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "inspect"},
                    {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": png}}
                ]
            }]
        });
        let original = serde_json::to_vec(&body).expect("serialize").len();

        let result = serialize_with_image_budget(&mut body, original - 1).expect("budget");

        assert_eq!(result.downsampled_images, 1);
        assert_eq!(result.removed_images, 0);
        assert!(result.final_bytes() < original);
        assert_eq!(
            body["messages"][0]["content"][1]["source"]["media_type"],
            "image/jpeg"
        );
        let rewritten = body["messages"][0]["content"][1]["source"]["data"]
            .as_str()
            .expect("rewritten image data");
        let bytes = STANDARD.decode(rewritten).expect("decode jpeg");
        let image = image::load_from_memory(&bytes).expect("load jpeg");
        let (width, height) = image.dimensions();
        assert!(width <= MAX_DOWNSAMPLED_IMAGE_EDGE);
        assert!(height <= MAX_DOWNSAMPLED_IMAGE_EDGE);
    }

    #[test]
    fn removes_oldest_inline_image_when_downsample_cannot_fit_budget() {
        let first = format!("data:image/png;base64,{}", "A".repeat(10_000));
        let second = format!("data:image/png;base64,{}", "B".repeat(10_000));
        let mut body = json!({
            "model": "qwen",
            "input": [
                {"type": "message", "role": "user", "content": [
                    {"type": "input_text", "text": "old"},
                    {"type": "input_image", "image_url": first}
                ]},
                {"type": "message", "role": "user", "content": [
                    {"type": "input_text", "text": "new"},
                    {"type": "input_image", "image_url": second}
                ]}
            ]
        });
        let mut expected_after_one = body.clone();
        expected_after_one["input"][0]["content"][1] = json!({
            "type": "input_text",
            "text": REMOVED_IMAGE_MARKER
        });
        let budget = serde_json::to_vec(&expected_after_one)
            .expect("serialize expected")
            .len();

        let result = serialize_with_image_budget(&mut body, budget).expect("budget");

        assert_eq!(result.downsampled_images, 0);
        assert_eq!(result.removed_images, 1);
        assert_eq!(result.final_bytes(), budget);
        assert_eq!(body["input"][0]["content"][1]["type"], "input_text");
        assert_eq!(body["input"][1]["content"][1]["type"], "input_image");
    }

    #[test]
    fn removes_gemini_inline_image_part() {
        let mut body = json!({
            "model": "gemini",
            "contents": [{
                "role": "user",
                "parts": [
                    {"text": "old"},
                    {"inlineData": {"mimeType": "image/png", "data": "AAAA"}}
                ]
            }]
        });
        let budget = 100;

        let result = serialize_with_image_budget(&mut body, budget).expect("budget");

        assert_eq!(result.removed_images, 1);
        assert_eq!(
            body["contents"][0]["parts"][1]["text"],
            REMOVED_IMAGE_MARKER
        );
    }
}
