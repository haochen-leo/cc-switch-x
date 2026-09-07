//! Codex native Responses protocol adaptation.
//!
//! OpenAI's Codex backend accepts a private superset of the public Responses
//! protocol. Third-party native Responses endpoints generally implement only
//! a subset, so request lowering and response restoration must be treated as
//! one attempt-scoped transform instead of being recomputed later by handlers.

use std::{collections::HashMap, pin::Pin};

use bytes::Bytes;
use futures::Stream;
use serde_json::{json, Value};

use super::{
    codex_native_responses_uses_openai_private_contract,
    provider_needs_responses_apply_patch_bridge, provider_needs_responses_namespace_flatten,
    provider_needs_responses_tool_search_bridge, provider_needs_xai_responses_sanitize,
    transform_codex_anthropic, transform_codex_apply_patch, transform_codex_chat,
    transform_codex_responses_namespace::{self, NamespacedName},
    transform_codex_responses_toolsearch, transform_codex_responses_xai_sanitize,
};
use crate::{provider::Provider, proxy::error::ProxyError};

pub(crate) type ResponseByteStream =
    Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResponsesDialect {
    OpenAiPrivateContract,
    ThirdPartyStandard,
    XaiStrict,
}

#[derive(Debug, Clone)]
struct TransformPolicy {
    namespace_flatten: bool,
    apply_patch_bridge: bool,
    tool_search_bridge: bool,
    xai_sanitize: bool,
    normalize_response_output_item_ids: bool,
    adapt_automation_update_schema: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct TransformContext {
    pub(crate) dialect: ResponsesDialect,
    pub(crate) provider_id: String,
    pub(crate) upstream_model: Option<String>,
    restore_map: HashMap<String, NamespacedName>,
    policy: TransformPolicy,
}

#[derive(Debug)]
pub(crate) struct PreparedRequest {
    pub(crate) body: Value,
    pub(crate) context: TransformContext,
}

#[derive(Debug)]
pub(crate) struct FinalizedRequest {
    pub(crate) body: Value,
    pub(crate) context: TransformContext,
}

pub(crate) fn prepare_request(
    mut body: Value,
    provider: &Provider,
    upstream_model: Option<&str>,
) -> Result<PreparedRequest, ProxyError> {
    let private_contract =
        codex_native_responses_uses_openai_private_contract(provider, upstream_model);
    let xai_sanitize = provider_needs_xai_responses_sanitize(provider);
    let dialect = if private_contract {
        ResponsesDialect::OpenAiPrivateContract
    } else if xai_sanitize {
        ResponsesDialect::XaiStrict
    } else {
        ResponsesDialect::ThirdPartyStandard
    };
    let policy = TransformPolicy {
        namespace_flatten: provider_needs_responses_namespace_flatten(provider, upstream_model),
        apply_patch_bridge: provider_needs_responses_apply_patch_bridge(provider, upstream_model),
        tool_search_bridge: provider_needs_responses_tool_search_bridge(provider, upstream_model),
        xai_sanitize,
        normalize_response_output_item_ids: !private_contract,
        adapt_automation_update_schema: !private_contract,
    };
    let mut context = TransformContext {
        dialect,
        provider_id: provider.id.clone(),
        upstream_model: upstream_model.map(str::to_string),
        restore_map: HashMap::new(),
        policy,
    };

    apply_request_transforms(&mut body, &mut context)?;
    Ok(PreparedRequest { body, context })
}

/// Re-apply idempotent provider-bound invariants after body overrides.
///
/// Overrides are intentionally authoritative, but they may replace `tools` or
/// `input`. Re-running the native Responses lowering here ensures the final
/// outbound body and the response restore context still describe the same
/// successful provider attempt.
pub(crate) fn finalize_request(
    mut context: TransformContext,
    mut final_body: Value,
) -> Result<FinalizedRequest, ProxyError> {
    apply_request_transforms(&mut final_body, &mut context)?;
    if context.policy.adapt_automation_update_schema {
        let changed = adapt_automation_update_function_schema(&mut final_body);
        if changed > 0 {
            log::debug!(
                "[Codex] Adapted {changed} automation_update tool schema(s) for native Responses upstream (provider={})",
                context.provider_id
            );
        }
    }
    context.upstream_model = final_body
        .get("model")
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty())
        .map(str::to_string)
        .or(context.upstream_model);
    Ok(FinalizedRequest {
        body: final_body,
        context,
    })
}

fn apply_request_transforms(
    body: &mut Value,
    context: &mut TransformContext,
) -> Result<(), ProxyError> {
    extend_restore_map(body, &mut context.restore_map);

    if context.policy.xai_sanitize {
        transform_codex_responses_xai_sanitize::promote_additional_tools(body);
        // Promotion can expose namespace declarations that were nested in the
        // per-turn carrier. Capture them before flattening.
        extend_restore_map(body, &mut context.restore_map);
    }

    if context.policy.namespace_flatten {
        transform_codex_responses_namespace::flatten_request_namespaces(body)?;
    }
    if context.policy.apply_patch_bridge {
        transform_codex_apply_patch::bridge_request_apply_patch_custom_to_function(body);
    }
    if context.policy.xai_sanitize {
        transform_codex_responses_xai_sanitize::sanitize_xai_responses_request(body);
    } else if context.policy.tool_search_bridge {
        transform_codex_responses_toolsearch::materialize_tool_search_declaration(body);
        transform_codex_responses_toolsearch::promote_tool_search_output_tools(body);
    }

    transform_codex_anthropic::sanitize_anthropic_reasoning_envelopes_for_native_responses(body);
    if context.dialect == ResponsesDialect::OpenAiPrivateContract {
        transform_codex_chat::normalize_official_replayed_item_ids_for_responses_upstream(body);
    } else {
        transform_codex_chat::normalize_replayed_item_ids_for_responses_upstream(body);
    }
    Ok(())
}

fn extend_restore_map(body: &Value, restore_map: &mut HashMap<String, NamespacedName>) {
    restore_map.extend(transform_codex_responses_namespace::namespace_restore_map(
        body,
    ));
    restore_map
        .extend(transform_codex_responses_toolsearch::tool_search_namespace_restore_map(body));
}

pub(crate) fn transform_response(body: &mut Value, context: &TransformContext) {
    if context.policy.apply_patch_bridge {
        transform_codex_apply_patch::restore_response_apply_patch_function_calls(body);
    }
    if context.policy.normalize_response_output_item_ids {
        transform_codex_chat::normalize_response_output_item_ids(body);
    }
    if context.policy.tool_search_bridge {
        transform_codex_responses_toolsearch::rewrite_tool_search_function_calls(body);
    }
    if !context.restore_map.is_empty() {
        transform_codex_responses_namespace::restore_response_namespaces(
            body,
            &context.restore_map,
        );
    }
    transform_codex_apply_patch::sanitize_response_apply_patch_inputs(body);
}

pub(crate) fn transform_response_stream(
    stream: ResponseByteStream,
    context: TransformContext,
) -> ResponseByteStream {
    let stream = if context.policy.apply_patch_bridge {
        Box::pin(
            transform_codex_apply_patch::create_apply_patch_function_restore_sse_stream(stream),
        ) as ResponseByteStream
    } else {
        stream
    };
    let stream = if context.policy.normalize_response_output_item_ids {
        Box::pin(transform_codex_chat::create_response_output_id_normalize_sse_stream(stream))
            as ResponseByteStream
    } else {
        stream
    };
    let stream = if context.policy.tool_search_bridge {
        Box::pin(transform_codex_responses_toolsearch::create_tool_search_call_sse_stream(stream))
            as ResponseByteStream
    } else {
        stream
    };
    let stream = if !context.restore_map.is_empty() {
        Box::pin(
            transform_codex_responses_namespace::create_namespace_restore_sse_stream(
                stream,
                context.restore_map,
            ),
        ) as ResponseByteStream
    } else {
        stream
    };
    Box::pin(transform_codex_apply_patch::create_apply_patch_input_sanitize_sse_stream(stream))
}

/// Codex Desktop declares `automation_update` with a recursive
/// `$defs`/`$ref`/`oneOf` schema generated from its Rust protocol types. The
/// official backend accepts that private shape, but third-party native
/// Responses models misread it (observed: the tool is called with empty or
/// wrong arguments). Replace only this tool's parameters with a hand
/// maintained flat schema covering every automation field; all other tool
/// schemas pass through untouched.
fn adapt_automation_update_function_schema(body: &mut Value) -> usize {
    let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) else {
        return 0;
    };
    let mut changed = 0;
    for tool in tools {
        if tool.get("type").and_then(Value::as_str) != Some("function") {
            continue;
        }
        if tool.get("name").and_then(Value::as_str) != Some("automation_update") {
            continue;
        }
        let adapted = automation_update_flat_schema();
        if tool.get("parameters") != Some(&adapted) {
            tool["parameters"] = adapted;
            changed += 1;
        }
    }
    changed
}

fn automation_update_flat_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": {
                "type": "string",
                "description": "Automation id. Required for view, update, suggested_update and delete."
            },
            "mode": {
                "type": "string",
                "enum": [
                    "view",
                    "create",
                    "suggested_create",
                    "update",
                    "suggested_update",
                    "delete"
                ]
            },
            "name": {"type": "string"},
            "prompt": {"type": "string"},
            "rrule": {
                "type": "string",
                "description": "iCalendar RRULE schedule string, e.g. FREQ=DAILY;BYHOUR=9;BYMINUTE=0."
            },
            "status": {"type": "string", "enum": ["ACTIVE", "PAUSED"]},
            "kind": {"type": "string", "enum": ["cron", "heartbeat"]},
            "projectId": {"type": ["string", "null"]},
            "targetThreadId": {"type": ["string", "null"]},
            "destination": {"type": "string", "enum": ["local", "thread"]},
            "executionEnvironment": {"type": "string", "enum": ["local", "worktree"]},
            "localEnvironmentConfigPath": {"type": ["string", "null"]},
            "model": {"type": ["string", "null"]},
            "reasoningEffort": {"type": ["string", "null"]},
            "notificationPolicy": {"type": ["string", "null"]}
        },
        "required": ["mode"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(id: &str, base_url: &str) -> Provider {
        Provider {
            id: id.to_string(),
            name: id.to_string(),
            settings_config: json!({"base_url": base_url}),
            website_url: None,
            category: None,
            created_at: None,
            sort_index: None,
            notes: None,
            meta: None,
            icon: None,
            icon_color: None,
            in_failover_queue: false,
        }
    }

    fn contains_key(value: &Value, target: &str) -> bool {
        match value {
            Value::Object(object) => {
                object.contains_key(target)
                    || object.values().any(|value| contains_key(value, target))
            }
            Value::Array(values) => values.iter().any(|value| contains_key(value, target)),
            _ => false,
        }
    }

    #[test]
    fn third_party_attempt_repairs_automation_output_and_adapts_schema() {
        let body = json!({
            "model": "qwen3.8-max",
            "input": [{
                "type": "function_call_output",
                "id": "fco_automation_update",
                "name": "automation_update",
                "namespace": "codex_app",
                "output": "{\"mode\":\"view\",\"id\":\"daily\"}"
            }],
            "tools": [{
                "type": "function",
                "name": "automation_update",
                "parameters": {
                    "oneOf": [{"$ref": "#/$defs/view"}],
                    "$defs": {
                        "view": {
                            "type": "object",
                            "properties": {"mode": {"const": "view"}},
                            "required": ["mode"]
                        }
                    }
                }
            }, {
                "type": "function",
                "name": "exec_command",
                "parameters": {
                    "type": "object",
                    "properties": {"cmd": {"type": "string"}},
                    "required": ["cmd"]
                }
            }]
        });
        let prepared = prepare_request(
            body,
            &provider(
                "dashscope-native",
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
            ),
            Some("qwen3.8-max"),
        )
        .unwrap();

        assert_eq!(
            prepared.context.dialect,
            ResponsesDialect::ThirdPartyStandard
        );
        assert_eq!(prepared.body["input"].as_array().unwrap().len(), 2);
        assert_eq!(prepared.body["input"][0]["type"], "function_call");
        assert_eq!(
            prepared.body["input"][0]["call_id"],
            prepared.body["input"][1]["call_id"]
        );

        let finalized = finalize_request(prepared.context, prepared.body).unwrap();
        let parameters = &finalized.body["tools"][0]["parameters"];
        assert_eq!(parameters["type"], "object");
        for keyword in ["$defs", "$ref", "oneOf", "const"] {
            assert!(!contains_key(parameters, keyword), "left keyword {keyword}");
        }
        for field in [
            "mode",
            "id",
            "name",
            "prompt",
            "rrule",
            "status",
            "kind",
            "projectId",
            "targetThreadId",
            "destination",
            "executionEnvironment",
            "localEnvironmentConfigPath",
            "model",
            "reasoningEffort",
            "notificationPolicy",
        ] {
            assert!(
                parameters["properties"].get(field).is_some(),
                "dropped automation field {field}"
            );
        }
        assert_eq!(
            parameters["properties"]["mode"]["enum"],
            json!([
                "view",
                "create",
                "suggested_create",
                "update",
                "suggested_update",
                "delete"
            ])
        );

        // Other function tools must pass through untouched.
        assert_eq!(
            finalized.body["tools"][1]["parameters"],
            json!({
                "type": "object",
                "properties": {"cmd": {"type": "string"}},
                "required": ["cmd"]
            })
        );
    }

    #[test]
    fn official_attempt_preserves_private_schema_and_standalone_output() {
        let body = json!({
            "model": "gpt-5.6-sol",
            "input": [{
                "type": "function_call_output",
                "id": "fco_automation_update",
                "name": "automation_update",
                "namespace": "codex_app",
                "output": "{}"
            }],
            "tools": [{
                "type": "function",
                "name": "automation_update",
                "parameters": {
                    "oneOf": [{"$ref": "#/$defs/view"}],
                    "$defs": {"view": {"type": "object"}}
                }
            }]
        });
        let prepared = prepare_request(
            body.clone(),
            &provider(
                crate::database::CODEX_OFFICIAL_PROVIDER_ID,
                "https://chatgpt.com/backend-api/codex",
            ),
            Some("gpt-5.6-sol"),
        )
        .unwrap();
        assert_eq!(
            prepared.context.dialect,
            ResponsesDialect::OpenAiPrivateContract
        );
        assert_eq!(prepared.body["input"].as_array().unwrap().len(), 1);
        assert!(prepared.body["input"][0].get("call_id").is_none());

        let finalized = finalize_request(prepared.context, prepared.body).unwrap();
        assert_eq!(
            finalized.body["tools"][0]["parameters"],
            body["tools"][0]["parameters"]
        );
    }
}
