use crate::app_config::AppType;
use crate::database::{Database, CODEX_OFFICIAL_PROVIDER_ID};
use crate::provider::{Provider, ProviderMeta};
use futures::future::join_all;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::fs;

pub const CODEX_AGGREGATE_PROVIDER_ID: &str = "codex-multi-provider";
pub const CODEX_AGGREGATE_PREVIOUS_PROVIDER_SETTING: &str = "codex_aggregate_previous_provider";
pub const CODEX_AGGREGATE_PREVIOUS_TAKEOVER_SETTING: &str = "codex_aggregate_previous_takeover";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAggregationStatus {
    pub enabled: bool,
    pub provider_id: String,
    pub model_count: usize,
    pub source_provider_count: usize,
    pub warnings: Vec<String>,
}

impl CodexAggregationStatus {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            provider_id: CODEX_AGGREGATE_PROVIDER_ID.to_string(),
            model_count: 0,
            source_provider_count: 0,
            warnings: Vec::new(),
        }
    }
}

pub struct CodexAggregationBuild {
    pub provider: Provider,
    pub model_count: usize,
    pub source_provider_count: usize,
    pub warnings: Vec<String>,
}

/// 从真实 Codex Provider 与官方本地模型缓存生成一个统一 Provider。
///
/// 官方模型固定路由到内置 `codex-official`；第三方模型优先读取各 Provider 已保存的
/// `modelCatalog`，未配置目录时在线调用该 Provider 的 OpenAI-compatible `/models`。
pub async fn build_codex_aggregate_provider(
    db: &Database,
) -> Result<CodexAggregationBuild, String> {
    let providers = db
        .get_all_providers(AppType::Codex.as_str())
        .map_err(|error| format!("读取 Codex 供应商失败: {error}"))?;
    let official = providers
        .get(CODEX_OFFICIAL_PROVIDER_ID)
        .cloned()
        .ok_or_else(|| "缺少 OpenAI Official 供应商，无法创建 Codex 聚合入口".to_string())?;

    let official_models = read_official_catalog_models()?;
    if official_models.is_empty() {
        return Err(
            "Codex 官方模型缓存为空；请先用 OpenAI Official 打开一次 Codex 模型列表".to_string(),
        );
    }

    let third_party = providers
        .into_values()
        .filter(|provider| {
            provider.id != CODEX_OFFICIAL_PROVIDER_ID && !provider.is_codex_aggregate()
        })
        .collect::<Vec<_>>();
    let loaded = join_all(third_party.into_iter().map(load_provider_models)).await;

    let mut merged_models = Vec::new();
    let mut routes = Map::new();
    let mut seen_models = HashSet::new();
    let mut source_provider_ids = HashSet::new();
    let mut warnings = Vec::new();

    for entry in official_models {
        let Some((model, entry)) = normalize_catalog_entry(entry, None) else {
            continue;
        };
        if seen_models.insert(model.clone()) {
            routes.insert(model, json!(CODEX_OFFICIAL_PROVIDER_ID));
            merged_models.push(entry);
            source_provider_ids.insert(CODEX_OFFICIAL_PROVIDER_ID.to_string());
        }
    }

    let mut third_party_model_count = 0usize;
    for loaded_provider in loaded {
        if let Some(warning) = loaded_provider.warning {
            warnings.push(warning);
        }
        let mut accepted = 0usize;
        for entry in loaded_provider.models {
            let Some((model, entry)) =
                normalize_catalog_entry(entry, loaded_provider.default_context_window)
            else {
                continue;
            };
            if !seen_models.insert(model.clone()) {
                continue;
            }
            routes.insert(model, json!(loaded_provider.provider.id));
            merged_models.push(entry);
            accepted += 1;
        }
        if accepted > 0 {
            third_party_model_count += accepted;
            source_provider_ids.insert(loaded_provider.provider.id);
        }
    }

    if third_party_model_count == 0 {
        let detail = if warnings.is_empty() {
            "第三方供应商未配置 modelCatalog，且 /models 未返回模型".to_string()
        } else {
            warnings.join("；")
        };
        return Err(format!("没有可聚合的第三方 Codex 模型：{detail}"));
    }

    let mut settings_config = official.settings_config.clone();
    let root = settings_config
        .as_object_mut()
        .ok_or_else(|| "OpenAI Official 配置格式错误".to_string())?;
    root.insert(
        "modelCatalog".to_string(),
        json!({ "models": merged_models }),
    );
    root.insert("codexAggregateRoutes".to_string(), Value::Object(routes));

    let mut provider = Provider::with_id(
        CODEX_AGGREGATE_PROVIDER_ID.to_string(),
        "Codex Multi Provider".to_string(),
        settings_config,
        None,
    );
    provider.category = Some("router".to_string());
    provider.created_at = Some(chrono::Utc::now().timestamp_millis());
    provider.notes =
        Some("由 CC Switch 自动维护：Codex 下拉列表同时展示官方与第三方模型。".to_string());
    provider.icon = Some("openai".to_string());
    provider.icon_color = Some("#10A37F".to_string());
    provider.meta = Some(ProviderMeta {
        provider_type: Some("codex_aggregate".to_string()),
        ..Default::default()
    });

    Ok(CodexAggregationBuild {
        model_count: provider
            .settings_config
            .pointer("/modelCatalog/models")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0),
        source_provider_count: source_provider_ids.len(),
        provider,
        warnings,
    })
}

struct LoadedProviderModels {
    provider: Provider,
    models: Vec<Value>,
    default_context_window: Option<u64>,
    warning: Option<String>,
}

async fn load_provider_models(provider: Provider) -> LoadedProviderModels {
    let default_context_window = provider_default_context_window(&provider);
    let stored_models = provider
        .settings_config
        .pointer("/modelCatalog/models")
        .and_then(Value::as_array)
        .filter(|models| !models.is_empty())
        .cloned();

    if let Some(models) = stored_models {
        return LoadedProviderModels {
            provider,
            models,
            default_context_window,
            warning: None,
        };
    }

    let (base_url, api_key) = provider.resolve_usage_credentials(&AppType::Codex);
    if base_url.trim().is_empty() || api_key.trim().is_empty() {
        return LoadedProviderModels {
            warning: Some(format!(
                "{} 缺少可读取 /models 的 Base URL 或 API Key",
                provider.name
            )),
            provider,
            models: Vec::new(),
            default_context_window,
        };
    }

    let is_full_url = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.is_full_url)
        .unwrap_or(false);
    let user_agent = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.custom_user_agent_header().ok().flatten());
    match crate::services::model_fetch::fetch_models(
        &base_url,
        &api_key,
        is_full_url,
        None,
        user_agent,
    )
    .await
    {
        Ok(models) => LoadedProviderModels {
            provider,
            models: models
                .into_iter()
                .map(|model| {
                    json!({
                        "model": model.id,
                        "displayName": model.id,
                    })
                })
                .collect(),
            default_context_window,
            warning: None,
        },
        Err(error) => LoadedProviderModels {
            warning: Some(format!("{} 获取 /models 失败: {error}", provider.name)),
            provider,
            models: Vec::new(),
            default_context_window,
        },
    }
}

fn provider_default_context_window(provider: &Provider) -> Option<u64> {
    provider
        .settings_config
        .get("config")
        .and_then(Value::as_str)
        .and_then(|config| config.parse::<toml::Value>().ok())
        .and_then(|config| {
            config
                .get("model_context_window")
                .and_then(toml::Value::as_integer)
        })
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| *value > 0)
}

fn read_official_catalog_models() -> Result<Vec<Value>, String> {
    let codex_dir = crate::codex_config::get_codex_config_dir();
    let paths = [
        codex_dir.join("models_cache.json"),
        codex_dir.join("models_cache.cc-switch-backup.json"),
    ];
    let mut models = Vec::new();
    let mut seen = HashSet::new();
    let mut read_errors = Vec::new();

    for path in paths {
        if !path.exists() {
            continue;
        }
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) => {
                read_errors.push(format!("读取 {} 失败: {error}", path.display()));
                continue;
            }
        };
        let value: Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(error) => {
                read_errors.push(format!("解析 {} 失败: {error}", path.display()));
                continue;
            }
        };
        let entries = value
            .get("models")
            .and_then(Value::as_array)
            .or_else(|| value.get("data").and_then(Value::as_array))
            .or_else(|| value.get("items").and_then(Value::as_array));
        let Some(entries) = entries else {
            continue;
        };

        for entry in entries {
            let Some(id) = catalog_model_id(entry) else {
                continue;
            };
            if !is_official_codex_model_id(&id)
                || model_entry_is_explicitly_unavailable(entry)
                || !seen.insert(id.clone())
            {
                continue;
            }
            let display_name = entry
                .get("display_name")
                .or_else(|| entry.get("displayName"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or(&id);
            let mut normalized = Map::new();
            normalized.insert("model".to_string(), json!(id));
            normalized.insert("displayName".to_string(), json!(display_name));
            copy_first_field(
                entry,
                &mut normalized,
                &["context_window", "max_context_window", "contextWindow"],
                "contextWindow",
            );
            copy_first_field(
                entry,
                &mut normalized,
                &["input_modalities", "inputModalities"],
                "inputModalities",
            );
            copy_first_field(
                entry,
                &mut normalized,
                &["supports_parallel_tool_calls", "supportsParallelToolCalls"],
                "supportsParallelToolCalls",
            );
            copy_first_field(
                entry,
                &mut normalized,
                &["base_instructions", "baseInstructions"],
                "baseInstructions",
            );
            models.push(Value::Object(normalized));
        }
    }

    if models.is_empty() && !read_errors.is_empty() {
        Err(read_errors.join("；"))
    } else {
        Ok(models)
    }
}

fn copy_first_field(
    source: &Value,
    target: &mut Map<String, Value>,
    candidates: &[&str],
    target_key: &str,
) {
    if let Some(value) = candidates.iter().find_map(|key| source.get(*key)).cloned() {
        target.insert(target_key.to_string(), value);
    }
}

fn normalize_catalog_entry(
    entry: Value,
    default_context_window: Option<u64>,
) -> Option<(String, Value)> {
    let model = catalog_model_id(&entry)?;
    let mut object = entry.as_object().cloned().unwrap_or_default();
    object.insert("model".to_string(), json!(model));
    if object
        .get("displayName")
        .or_else(|| object.get("display_name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .is_none_or(str::is_empty)
    {
        object.insert("displayName".to_string(), json!(model));
    }
    if object
        .get("contextWindow")
        .or_else(|| object.get("context_window"))
        .is_none()
    {
        if let Some(context_window) = default_context_window {
            object.insert("contextWindow".to_string(), json!(context_window));
        }
    }
    Some((model, Value::Object(object)))
}

fn catalog_model_id(entry: &Value) -> Option<String> {
    if let Some(model) = entry
        .as_str()
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        return Some(model.to_string());
    }
    ["model", "slug", "id", "name"]
        .iter()
        .find_map(|key| entry.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string)
}

fn is_official_codex_model_id(model_id: &str) -> bool {
    let id = model_id.trim().to_ascii_lowercase();
    if id.starts_with("gpt-") || id.starts_with("codex-") || id.starts_with("chatgpt-") {
        return true;
    }
    ["o1", "o3", "o4", "o5"].iter().any(|prefix| {
        id == *prefix
            || id
                .strip_prefix(prefix)
                .is_some_and(|suffix| suffix.starts_with('-'))
    })
}

fn model_entry_is_explicitly_unavailable(entry: &Value) -> bool {
    [
        "supported_in_api",
        "supportedInApi",
        "available",
        "is_available",
        "isAvailable",
        "enabled",
    ]
    .iter()
    .any(|key| entry.get(*key).and_then(Value::as_bool) == Some(false))
        || entry.get("disabled").and_then(Value::as_bool) == Some(true)
        || entry
            .get("visibility")
            .or_else(|| entry.get("status"))
            .or_else(|| entry.get("availability"))
            .and_then(Value::as_str)
            .map(|value| value.to_ascii_lowercase())
            .is_some_and(|value| {
                matches!(
                    value.as_str(),
                    "hide" | "hidden" | "disabled" | "unavailable" | "unsupported" | "denied"
                )
            })
}

pub fn aggregate_provider_stats(provider: &Provider) -> (usize, usize) {
    let model_count = provider
        .settings_config
        .pointer("/modelCatalog/models")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let source_provider_count = provider
        .settings_config
        .get("codexAggregateRoutes")
        .and_then(Value::as_object)
        .map(|routes| {
            routes
                .values()
                .filter_map(Value::as_str)
                .collect::<HashSet<_>>()
                .len()
        })
        .unwrap_or(0);
    (model_count, source_provider_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_model_filter_rejects_third_party_catalog_entries() {
        assert!(is_official_codex_model_id("gpt-5.6-sol"));
        assert!(is_official_codex_model_id("o4-mini"));
        assert!(!is_official_codex_model_id("qwen3.8-max"));
        assert!(!is_official_codex_model_id("kimi-k2.7-code"));
    }

    #[test]
    fn catalog_normalization_keeps_model_and_adds_provider_context() {
        let (model, entry) = normalize_catalog_entry(
            json!({ "model": "qwen3.8-max", "displayName": "Qwen" }),
            Some(262_144),
        )
        .expect("catalog entry");
        assert_eq!(model, "qwen3.8-max");
        assert_eq!(entry["contextWindow"], json!(262_144));
    }
}
