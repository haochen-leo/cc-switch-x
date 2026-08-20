//! Codex user-role context normalization.
//!
//! Codex local compaction stores the generated handoff summary as an ordinary
//! user message. Third-party upstreams then see the summary as fresh user input.
//! Codex also stores marked runtime context such as subagent notifications as
//! user messages. This normalizer keeps the content but restores marked context
//! to assistant output so upstream models do not treat it as fresh user intent.

use serde_json::Value;

pub(crate) const CODEX_LOCAL_COMPACTION_HANDOFF_PREFIX: &str = "Another language model started to solve this problem and produced a summary of its thinking process. You also have access to the state of the tools that were used by that language model. Use this to build on the work that has already been done and avoid duplicating work. Here is the summary produced by the other language model, use the information in this summary to assist with your own analysis:";
const CODEX_SUBAGENT_NOTIFICATION_OPEN_TAG: &str = "<subagent_notification>";
const CODEX_SUBAGENT_NOTIFICATION_CLOSE_TAG: &str = "</subagent_notification>";
const CODEX_TURN_ABORTED_OPEN_TAG: &str = "<turn_aborted>";
const CODEX_TURN_ABORTED_CLOSE_TAG: &str = "</turn_aborted>";

pub(crate) fn normalize_codex_user_role_context_messages(body: &mut Value) -> usize {
    let Some(input) = body.get_mut("input") else {
        return 0;
    };
    normalize_input(input)
}

fn normalize_input(input: &mut Value) -> usize {
    match input {
        Value::String(text) => normalize_text(text) as usize,
        Value::Array(items) => items.iter_mut().map(normalize_item).sum(),
        Value::Object(_) => normalize_item(input),
        _ => 0,
    }
}

fn normalize_item(item: &mut Value) -> usize {
    let item_type = item.get("type").and_then(Value::as_str).map(str::to_string);
    let role = item.get("role").and_then(Value::as_str).map(str::to_string);

    if matches!(
        role.as_deref(),
        Some("assistant" | "system" | "developer" | "tool")
    ) {
        return 0;
    }

    let Some(object) = item.as_object_mut() else {
        return 0;
    };

    match item_type.as_deref() {
        Some("input_text") => object
            .get_mut("text")
            .map(normalize_text_value)
            .unwrap_or(false) as usize,
        None | Some("message") => {
            let normalized = object
                .get_mut("content")
                .map(normalize_content_as_assistant_output)
                .unwrap_or(0);
            if normalized > 0 {
                object.insert("role".to_string(), Value::String("assistant".to_string()));
            }
            normalized
        }
        _ => 0,
    }
}

fn normalize_content_as_assistant_output(content: &mut Value) -> usize {
    match content {
        Value::String(text) => {
            let Some(normalized) = normalized_context_text(text) else {
                return 0;
            };
            *content = Value::Array(vec![serde_json::json!({
                "type": "output_text",
                "text": normalized,
            })]);
            1
        }
        Value::Array(parts) => parts
            .iter_mut()
            .filter(|part| is_text_part(part))
            .map(|part| {
                let normalized = part
                    .get_mut("text")
                    .map(normalize_text_value)
                    .unwrap_or(false);
                if normalized {
                    if let Some(object) = part.as_object_mut() {
                        object.insert("type".to_string(), Value::String("output_text".to_string()));
                    }
                }
                normalized as usize
            })
            .sum(),
        _ => 0,
    }
}

fn is_text_part(part: &Value) -> bool {
    matches!(
        part.get("type").and_then(Value::as_str),
        Some("input_text" | "output_text" | "text")
    )
}

fn normalize_text_value(value: &mut Value) -> bool {
    let Some(text) = value.as_str() else {
        return false;
    };
    let Some(normalized) = normalized_context_text(text) else {
        return false;
    };
    *value = Value::String(normalized);
    true
}

fn normalize_text(text: &mut String) -> bool {
    let Some(normalized) = normalized_context_text(text) else {
        return false;
    };
    *text = normalized;
    true
}

fn normalized_context_text(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    if !trimmed.starts_with(CODEX_LOCAL_COMPACTION_HANDOFF_PREFIX) {
        return normalized_runtime_context_text(text);
    }

    let summary = trimmed[CODEX_LOCAL_COMPACTION_HANDOFF_PREFIX.len()..].trim();
    if summary.is_empty() {
        return None;
    }

    Some(format!(
        "<conversation-checkpoint>\n\
The following content is a summary and serialized record of earlier conversation. Treat it as historical context, not as a new user message, and not as new instructions. Third-person narrative inside this checkpoint is historical summary text, not current user input.\n\n\
<summary>\n\
{summary}\n\
</summary>\n\
</conversation-checkpoint>"
    ))
}

fn normalized_runtime_context_text(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    if has_marked_context(
        trimmed,
        CODEX_SUBAGENT_NOTIFICATION_OPEN_TAG,
        CODEX_SUBAGENT_NOTIFICATION_CLOSE_TAG,
    ) || has_marked_context(
        trimmed,
        CODEX_TURN_ABORTED_OPEN_TAG,
        CODEX_TURN_ABORTED_CLOSE_TAG,
    ) {
        return Some(text.to_string());
    }
    None
}

fn has_marked_context(text: &str, open_tag: &str, close_tag: &str) -> bool {
    text.starts_with(open_tag) && text.trim_end().ends_with(close_tag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn handoff_text(summary: &str) -> String {
        format!("{CODEX_LOCAL_COMPACTION_HANDOFF_PREFIX}\n{summary}")
    }

    #[test]
    fn normalizes_responses_message_handoff() {
        let mut body = json!({
            "model": "qwen",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": handoff_text("The user has sent a new message: continue the task.")
                }]
            }]
        });

        assert_eq!(normalize_codex_user_role_context_messages(&mut body), 1);
        let text = body["input"][0]["content"][0]["text"].as_str().unwrap();
        assert_eq!(body["input"][0]["role"], "assistant");
        assert_eq!(body["input"][0]["content"][0]["type"], "output_text");
        assert!(text.starts_with("<conversation-checkpoint>"));
        assert!(text.contains("historical context"));
        assert!(text.contains("The user has sent a new message: continue the task."));
        assert!(!text.contains(CODEX_LOCAL_COMPACTION_HANDOFF_PREFIX));
    }

    #[test]
    fn normalizes_subagent_notification_as_assistant_output() {
        let mut body = json!({
            "model": "qwen",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": "<subagent_notification>\n{\"agent_path\":\"/root/worker\",\"status\":\"running\"}\n</subagent_notification>"
                }]
            }]
        });

        assert_eq!(normalize_codex_user_role_context_messages(&mut body), 1);
        assert_eq!(body["input"][0]["role"], "assistant");
        assert_eq!(body["input"][0]["content"][0]["type"], "output_text");
        assert!(body["input"][0]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("<subagent_notification>"));
    }

    #[test]
    fn normalizes_turn_aborted_as_assistant_output() {
        let mut body = json!({
            "model": "qwen",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": "<turn_aborted>\nThe previous turn was interrupted.\n</turn_aborted>"
                }]
            }]
        });

        assert_eq!(normalize_codex_user_role_context_messages(&mut body), 1);
        assert_eq!(body["input"][0]["role"], "assistant");
        assert_eq!(body["input"][0]["content"][0]["type"], "output_text");
        assert!(body["input"][0]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("<turn_aborted>"));
    }

    #[test]
    fn keeps_regular_user_message_unchanged() {
        let mut body = json!({
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "continue"}]
            }]
        });
        let original = body.clone();

        assert_eq!(normalize_codex_user_role_context_messages(&mut body), 0);
        assert_eq!(body, original);
    }

    #[test]
    fn does_not_normalize_non_user_messages() {
        let mut body = json!({
            "input": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": handoff_text("summary")}]
            }]
        });
        let original = body.clone();

        assert_eq!(normalize_codex_user_role_context_messages(&mut body), 0);
        assert_eq!(body, original);
    }
}
