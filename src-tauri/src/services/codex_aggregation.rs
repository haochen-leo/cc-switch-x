use crate::app_config::AppType;
use crate::database::{Database, CODEX_OFFICIAL_PROVIDER_ID};
use crate::provider::{Provider, ProviderMeta};
use futures::future::join_all;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;

pub const CODEX_AGGREGATE_PROVIDER_ID: &str = "codex-multi-provider";
pub const CODEX_AGGREGATE_PREVIOUS_PROVIDER_SETTING: &str = "codex_aggregate_previous_provider";
pub const CODEX_AGGREGATE_PREVIOUS_TAKEOVER_SETTING: &str = "codex_aggregate_previous_takeover";
pub const CODEX_AGGREGATE_SOURCE_PROVIDERS_SETTING: &str = "codex_aggregate_source_providers";
pub const CODEX_AGGREGATE_UPSTREAM_MODEL_KEY: &str = "codexAggregateUpstreamModel";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAggregationSourceProvider {
    pub provider_id: String,
    pub name: String,
    pub official: bool,
    pub selected: bool,
    pub conversion_required: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAggregationStatus {
    pub enabled: bool,
    pub provider_id: String,
    pub model_count: usize,
    pub source_provider_count: usize,
    pub selected_provider_ids: Vec<String>,
    pub source_providers: Vec<CodexAggregationSourceProvider>,
    pub warnings: Vec<String>,
}

impl CodexAggregationStatus {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            provider_id: CODEX_AGGREGATE_PROVIDER_ID.to_string(),
            model_count: 0,
            source_provider_count: 0,
            selected_provider_ids: Vec::new(),
            source_providers: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

pub struct CodexAggregationBuild {
    pub provider: Provider,
    pub model_count: usize,
    pub source_provider_count: usize,
    pub selected_provider_ids: Vec<String>,
    pub source_providers: Vec<CodexAggregationSourceProvider>,
    pub warnings: Vec<String>,
}

fn read_configured_source_provider_ids(db: &Database) -> Result<Option<HashSet<String>>, String> {
    let Some(raw) = db
        .get_setting(CODEX_AGGREGATE_SOURCE_PROVIDERS_SETTING)
        .map_err(|error| format!("读取 Codex 聚合来源设置失败: {error}"))?
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(None);
    };

    let ids = serde_json::from_str::<Vec<String>>(&raw)
        .map_err(|error| format!("Codex 聚合来源设置格式错误: {error}"))?
        .into_iter()
        .map(|provider_id| provider_id.trim().to_string())
        .filter(|provider_id| !provider_id.is_empty())
        .collect::<HashSet<_>>();

    Ok((!ids.is_empty()).then_some(ids))
}

fn real_codex_source_providers(providers: impl IntoIterator<Item = Provider>) -> Vec<Provider> {
    providers
        .into_iter()
        .filter(|provider| !provider.is_codex_aggregate())
        .collect()
}

fn resolve_selected_provider_ids(
    providers: &[Provider],
    configured: Option<&HashSet<String>>,
) -> HashSet<String> {
    let all_provider_ids = providers
        .iter()
        .map(|provider| provider.id.clone())
        .collect::<HashSet<_>>();
    let selected = configured
        .map(|ids| {
            ids.intersection(&all_provider_ids)
                .cloned()
                .collect::<HashSet<_>>()
        })
        .unwrap_or_else(|| all_provider_ids.clone());

    if selected.is_empty() {
        all_provider_ids
    } else {
        selected
    }
}

fn source_provider_statuses(
    providers: &[Provider],
    selected_provider_ids: &HashSet<String>,
) -> Vec<CodexAggregationSourceProvider> {
    providers
        .iter()
        .map(|provider| CodexAggregationSourceProvider {
            provider_id: provider.id.clone(),
            name: provider.name.clone(),
            official: provider.id == CODEX_OFFICIAL_PROVIDER_ID,
            selected: selected_provider_ids.contains(&provider.id),
            conversion_required: provider_requires_protocol_conversion(provider),
        })
        .collect()
}

fn provider_requires_protocol_conversion(provider: &Provider) -> bool {
    crate::proxy::providers::codex_provider_uses_chat_completions(provider)
        || crate::proxy::providers::codex_provider_uses_anthropic(provider)
}

pub fn codex_aggregation_source_providers(
    db: &Database,
) -> Result<Vec<CodexAggregationSourceProvider>, String> {
    let providers = real_codex_source_providers(
        db.get_all_providers(AppType::Codex.as_str())
            .map_err(|error| format!("读取 Codex 供应商失败: {error}"))?
            .into_values(),
    );
    let configured = read_configured_source_provider_ids(db)?;
    let selected = resolve_selected_provider_ids(&providers, configured.as_ref());
    Ok(source_provider_statuses(&providers, &selected))
}

pub fn normalize_codex_aggregation_source_ids(
    db: &Database,
    requested_provider_ids: &[String],
) -> Result<Vec<String>, String> {
    let sources = codex_aggregation_source_providers(db)?;
    let available_ids = sources
        .iter()
        .map(|source| source.provider_id.as_str())
        .collect::<HashSet<_>>();
    let requested = requested_provider_ids
        .iter()
        .map(|provider_id| provider_id.trim())
        .filter(|provider_id| !provider_id.is_empty())
        .collect::<HashSet<_>>();

    if requested.is_empty() {
        return Err("Codex 多模型至少选择一个供应商".to_string());
    }
    if let Some(provider_id) = requested
        .iter()
        .find(|provider_id| !available_ids.contains(**provider_id))
    {
        return Err(format!("Codex 聚合来源供应商不存在: {provider_id}"));
    }

    Ok(sources
        .into_iter()
        .filter(|source| requested.contains(source.provider_id.as_str()))
        .map(|source| source.provider_id)
        .collect())
}

pub fn serialize_codex_aggregation_source_ids(
    db: &Database,
    selected_provider_ids: &[String],
) -> Result<String, String> {
    let all_provider_ids = codex_aggregation_source_providers(db)?
        .into_iter()
        .map(|source| source.provider_id)
        .collect::<Vec<_>>();
    if all_provider_ids == selected_provider_ids {
        return Ok(String::new());
    }
    serde_json::to_string(selected_provider_ids)
        .map_err(|error| format!("保存 Codex 聚合来源设置失败: {error}"))
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
    let source_providers = real_codex_source_providers(providers.into_values());
    let official = source_providers
        .iter()
        .find(|provider| provider.id == CODEX_OFFICIAL_PROVIDER_ID)
        .cloned()
        .ok_or_else(|| "缺少 OpenAI Official 供应商，无法创建 Codex 聚合入口".to_string())?;
    let configured = read_configured_source_provider_ids(db)?;
    let selected_provider_ids = resolve_selected_provider_ids(&source_providers, configured.as_ref());
    let source_provider_statuses = source_provider_statuses(&source_providers, &selected_provider_ids);
    let selected_provider_ids_in_order = source_provider_statuses
        .iter()
        .filter(|source| source.selected)
        .map(|source| source.provider_id.clone())
        .collect::<Vec<_>>();

    let third_party = source_providers
        .iter()
        .filter(|provider| {
            provider.id != CODEX_OFFICIAL_PROVIDER_ID
                && selected_provider_ids.contains(&provider.id)
        })
        .cloned()
        .collect::<Vec<_>>();
    let loaded = join_all(third_party.into_iter().map(load_provider_models)).await;

    let mut merged_models = Vec::new();
    let mut routes = Map::new();
    let mut seen_catalog_models = HashSet::new();
    let mut source_provider_ids = HashSet::new();
    let mut warnings = Vec::new();

    if selected_provider_ids.contains(CODEX_OFFICIAL_PROVIDER_ID) {
        match read_official_catalog_models() {
            Ok(official_models) => {
                for entry in official_models {
                    let Some((model, entry)) = normalize_catalog_entry(entry, None) else {
                        continue;
                    };
                    let (catalog_model, entry) = namespace_catalog_entry(&official, &model, entry);
                    if seen_catalog_models.insert(catalog_model.clone()) {
                        routes.insert(catalog_model, aggregate_route(&official.id, model.as_str()));
                        merged_models.push(entry);
                        source_provider_ids.insert(CODEX_OFFICIAL_PROVIDER_ID.to_string());
                    }
                }
                if !source_provider_ids.contains(CODEX_OFFICIAL_PROVIDER_ID) {
                    warnings.push(
                        "OpenAI Official 模型缓存为空；请先用官方供应商打开一次 Codex 模型列表"
                            .to_string(),
                    );
                }
            }
            Err(error) => warnings.push(error),
        }
    }

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
            let (catalog_model, entry) =
                namespace_catalog_entry(&loaded_provider.provider, &model, entry);
            if !seen_catalog_models.insert(catalog_model.clone()) {
                continue;
            }
            routes.insert(
                catalog_model,
                aggregate_route(&loaded_provider.provider.id, model.as_str()),
            );
            merged_models.push(entry);
            accepted += 1;
        }
        if accepted > 0 {
            source_provider_ids.insert(loaded_provider.provider.id);
        }
    }

    if merged_models.is_empty() {
        let detail = if warnings.is_empty() {
            "所选供应商未配置 modelCatalog，且 /models 未返回模型".to_string()
        } else {
            warnings.join("；")
        };
        return Err(format!("所选供应商没有可聚合的 Codex 模型：{detail}"));
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
        selected_provider_ids: selected_provider_ids_in_order,
        source_providers: source_provider_statuses,
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
    let api_format = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.api_format.as_deref());
    let request_headers = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.local_proxy_request_overrides.as_ref())
        .map(|overrides| {
            overrides
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect::<BTreeMap<_, _>>()
        });
    match crate::services::model_fetch::fetch_models(
        &base_url,
        &api_key,
        is_full_url,
        None,
        user_agent,
        api_format,
        request_headers.as_ref(),
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

fn namespace_catalog_entry(
    provider: &Provider,
    upstream_model: &str,
    entry: Value,
) -> (String, Value) {
    let mut object = entry.as_object().cloned().unwrap_or_default();
    let model_name = object
        .get("displayName")
        .or_else(|| object.get("display_name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(upstream_model)
        .to_string();
    let provider_name = provider.name.trim();
    let provider_name = if provider_name.is_empty() {
        provider.id.as_str()
    } else {
        provider_name
    }
    .to_string();
    let catalog_model = if provider.id == CODEX_OFFICIAL_PROVIDER_ID {
        upstream_model.to_string()
    } else {
        format!("{upstream_model}/{provider_name}")
    };

    object.insert("model".to_string(), json!(catalog_model));
    let display_name = if provider.id == CODEX_OFFICIAL_PROVIDER_ID {
        model_name
    } else {
        format!("{model_name} / {provider_name}")
    };
    object.insert("displayName".to_string(), json!(display_name));
    (catalog_model, Value::Object(object))
}

fn aggregate_route(provider_id: &str, upstream_model: &str) -> Value {
    json!({
        "providerId": provider_id,
        "model": upstream_model,
    })
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
    // 带 `/` 的是聚合目录的命名空间键（如 `gpt-5.6-luna/token-free`），可能经由
    // models_cache.json 回流，不能被当成官方模型重新路由到 codex-official。
    if id.contains('/') {
        return false;
    }
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

/// 聚合路由表 "slug → 上游模型" 映射（键统一小写）。
///
/// 供会话日志解析回填模型使用：路由表键的顺序随版本变化
///（`provider/model` 与 `model/provider` 并存），调用方需双向查找。
pub fn aggregate_route_model_map(db: &Database) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(providers) = db.get_all_providers(AppType::Codex.as_str()) else {
        return map;
    };
    for provider in providers.into_values() {
        if !provider.is_codex_aggregate() {
            continue;
        }
        let Some(routes) = provider
            .settings_config
            .get("codexAggregateRoutes")
            .and_then(Value::as_object)
        else {
            continue;
        };
        for (slug, route) in routes {
            if let Some(model) = route
                .get("model")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|model| !model.is_empty())
            {
                map.insert(slug.to_ascii_lowercase(), model.to_string());
            }
        }
    }
    map
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
                .filter_map(|route| {
                    route.as_str().or_else(|| {
                        route
                            .get("providerId")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|provider_id| !provider_id.is_empty())
                    })
                })
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
    fn official_model_filter_rejects_namespaced_catalog_entries() {
        assert!(!is_official_codex_model_id("gpt-5.6-luna/token-free"));
        assert!(!is_official_codex_model_id("gpt-5.6-sol/token-free"));
        assert!(!is_official_codex_model_id("gpt-5.5/Some Provider"));
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

    #[test]
    fn default_source_selection_includes_all_real_providers() {
        let official = Provider::with_id(
            CODEX_OFFICIAL_PROVIDER_ID.to_string(),
            "OpenAI Official".to_string(),
            json!({}),
            None,
        );
        let third_party = Provider::with_id(
            "provider-a".to_string(),
            "Provider A".to_string(),
            json!({}),
            None,
        );
        let mut aggregate = Provider::with_id(
            CODEX_AGGREGATE_PROVIDER_ID.to_string(),
            "Codex Multi Provider".to_string(),
            json!({}),
            None,
        );
        aggregate.meta = Some(ProviderMeta {
            provider_type: Some("codex_aggregate".to_string()),
            ..Default::default()
        });

        let providers = real_codex_source_providers(vec![official, third_party, aggregate]);
        let selected = resolve_selected_provider_ids(&providers, None);

        assert_eq!(providers.len(), 2);
        assert!(selected.contains(CODEX_OFFICIAL_PROVIDER_ID));
        assert!(selected.contains("provider-a"));
    }

    #[test]
    fn conversion_source_providers_are_aggregatable_by_default() {
        let db = Database::memory().expect("in-memory database");
        let mut official = Provider::with_id(
            CODEX_OFFICIAL_PROVIDER_ID.to_string(),
            "OpenAI Official".to_string(),
            json!({}),
            None,
        );
        official.category = Some("official".to_string());
        let chat_provider = Provider::with_id(
            "provider-chat".to_string(),
            "Provider Chat".to_string(),
            json!({ "apiFormat": "openai_chat" }),
            None,
        );
        db.save_provider(AppType::Codex.as_str(), &official)
            .expect("save official");
        db.save_provider(AppType::Codex.as_str(), &chat_provider)
            .expect("save chat provider");

        let sources = codex_aggregation_source_providers(&db).expect("sources");
        let chat = sources
            .iter()
            .find(|source| source.provider_id == "provider-chat")
            .expect("chat source");
        assert!(chat.conversion_required);
        assert!(chat.selected);
        assert!(
            normalize_codex_aggregation_source_ids(&db, &[String::from("provider-chat")]).is_ok()
        );
    }

    #[tokio::test]
    async fn aggregate_build_uses_only_selected_source_providers() {
        let db = Database::memory().expect("in-memory database");
        let mut official = Provider::with_id(
            CODEX_OFFICIAL_PROVIDER_ID.to_string(),
            "OpenAI Official".to_string(),
            json!({ "auth": {}, "config": "" }),
            None,
        );
        official.category = Some("official".to_string());
        let provider_a = Provider::with_id(
            "provider-a".to_string(),
            "Provider A".to_string(),
            json!({
                "auth": { "OPENAI_API_KEY": "a" },
                "config": "",
                "modelCatalog": {
                    "models": [{ "model": "model-a", "displayName": "Model A" }]
                }
            }),
            None,
        );
        let provider_b = Provider::with_id(
            "provider-b".to_string(),
            "Provider B".to_string(),
            json!({
                "auth": { "OPENAI_API_KEY": "b" },
                "config": "",
                "modelCatalog": {
                    "models": [{ "model": "model-b", "displayName": "Model B" }]
                }
            }),
            None,
        );
        db.save_provider(AppType::Codex.as_str(), &official)
            .expect("save official");
        db.save_provider(AppType::Codex.as_str(), &provider_a)
            .expect("save provider a");
        db.save_provider(AppType::Codex.as_str(), &provider_b)
            .expect("save provider b");
        db.set_setting(
            CODEX_AGGREGATE_SOURCE_PROVIDERS_SETTING,
            r#"["provider-a"]"#,
        )
        .expect("save source selection");

        let build = build_codex_aggregate_provider(&db)
            .await
            .expect("build aggregate");
        let models = build
            .provider
            .settings_config
            .pointer("/modelCatalog/models")
            .and_then(Value::as_array)
            .expect("model catalog");

        assert_eq!(build.selected_provider_ids, vec!["provider-a"]);
        assert_eq!(build.source_provider_count, 1);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0]["model"], "model-a/Provider A");
        assert_eq!(
            build.provider.settings_config["codexAggregateRoutes"]["model-a/Provider A"]
                ["providerId"],
            "provider-a"
        );
        assert!(build.provider.settings_config["codexAggregateRoutes"]
            .get("model-b/Provider B")
            .is_none());
    }

    #[test]
    fn catalog_namespace_keeps_same_model_from_different_providers() {
        let provider_a = Provider::with_id(
            "provider-a".to_string(),
            "Provider A".to_string(),
            json!({}),
            None,
        );
        let provider_b = Provider::with_id(
            "provider-b".to_string(),
            "Provider B".to_string(),
            json!({}),
            None,
        );

        let (model_a, entry_a) = namespace_catalog_entry(
            &provider_a,
            "gpt-5.6-sol",
            json!({ "displayName": "GPT-5.6-Sol" }),
        );
        let (model_b, entry_b) = namespace_catalog_entry(
            &provider_b,
            "gpt-5.6-sol",
            json!({ "displayName": "GPT-5.6-Sol" }),
        );

        assert_eq!(model_a, "gpt-5.6-sol/Provider A");
        assert_eq!(model_b, "gpt-5.6-sol/Provider B");
        assert_ne!(model_a, model_b);
        assert_eq!(entry_a["displayName"], "GPT-5.6-Sol / Provider A");
        assert_eq!(entry_b["displayName"], "GPT-5.6-Sol / Provider B");
    }

    #[test]
    fn official_catalog_display_name_has_no_provider_suffix() {
        let official = Provider::with_id(
            CODEX_OFFICIAL_PROVIDER_ID.to_string(),
            "OpenAI Official".to_string(),
            json!({}),
            None,
        );

        let (model, entry) = namespace_catalog_entry(
            &official,
            "gpt-5.6-sol",
            json!({ "displayName": "GPT-5.6-Sol" }),
        );

        assert_eq!(model, "gpt-5.6-sol");
        assert_eq!(entry["displayName"], "GPT-5.6-Sol");
    }

    #[test]
    fn aggregate_stats_reads_structured_routes() {
        let provider = Provider::with_id(
            CODEX_AGGREGATE_PROVIDER_ID.to_string(),
            "Codex Multi Provider".to_string(),
            json!({
                "modelCatalog": {
                    "models": [
                        { "model": "gpt-5.6-sol/provider-a" },
                        { "model": "gpt-5.6-sol/provider-b" }
                    ]
                },
                "codexAggregateRoutes": {
                    "gpt-5.6-sol/provider-a": {
                        "providerId": "provider-a",
                        "model": "gpt-5.6-sol"
                    },
                    "gpt-5.6-sol/provider-b": {
                        "providerId": "provider-b",
                        "model": "gpt-5.6-sol"
                    }
                }
            }),
            None,
        );

        assert_eq!(aggregate_provider_stats(&provider), (2, 2));
    }
}
