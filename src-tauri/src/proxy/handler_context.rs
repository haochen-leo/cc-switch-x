//! 请求上下文模块
//!
//! 提供请求生命周期的上下文管理，封装通用初始化逻辑

use crate::app_config::AppType;
use crate::provider::Provider;
use crate::proxy::{
    extract_session_id,
    forwarder::RequestForwarder,
    server::ProxyState,
    types::{AppProxyConfig, CopilotOptimizerConfig, OptimizerConfig, RectifierConfig},
    ProxyError,
};
use axum::http::HeaderMap;
use std::time::Instant;

/// 流式超时配置
#[derive(Debug, Clone, Copy)]
pub struct StreamingTimeoutConfig {
    /// 首字节超时（秒），0 表示禁用
    pub first_byte_timeout: u64,
    /// 静默期超时（秒），0 表示禁用
    pub idle_timeout: u64,
}

/// 请求上下文
///
/// 贯穿整个请求生命周期，包含：
/// - 计时信息
/// - 应用级代理配置（per-app）
/// - 选中的 Provider 列表（用于故障转移）
/// - 请求模型名称
/// - 日志标签
/// - Session ID（用于日志关联）
pub struct RequestContext {
    /// 请求开始时间
    pub start_time: Instant,
    /// 请求关联 ID
    pub request_id: String,
    /// 客户端请求端点
    pub endpoint: String,
    /// 应用级代理配置（per-app，包含重试次数和超时配置）
    pub app_config: AppProxyConfig,
    /// 选中的 Provider（故障转移链的第一个）
    pub provider: Provider,
    /// 完整的 Provider 列表（用于故障转移）
    providers: Vec<Provider>,
    /// 请求开始时的"当前供应商"（用于判断是否需要同步 UI/托盘）
    ///
    /// 这里使用本地 settings 的设备级 current provider。
    /// 代理模式下如果实际使用的 provider 与此不一致，会触发切换以确保 UI 始终准确。
    pub current_provider_id: String,
    /// 请求中的模型名称
    pub request_model: String,
    /// 实际发往上游的模型名（路由接管/模型映射后的真值，forward 成功后回填）。
    ///
    /// usage 归因的兜底顺序：上游响应回显 → outbound_model → request_model。
    /// 不能直接用 request_model 兜底：接管场景下它是映射前的客户端别名。
    pub outbound_model: Option<String>,
    /// 日志标签（如 "Claude"、"Codex"、"Gemini"）
    pub tag: &'static str,
    /// 应用类型字符串（如 "claude"、"codex"、"gemini"）
    pub app_type_str: &'static str,
    /// 应用类型（预留，目前通过 app_type_str 使用）
    #[allow(dead_code)]
    pub app_type: AppType,
    /// Session ID（从客户端请求提取或新生成）
    pub session_id: String,
    /// Session ID 是否由客户端提供。生成的 UUID 不能作为上游缓存 key，否则每个请求都会换 key。
    pub session_client_provided: bool,
    /// 整流器配置
    pub rectifier_config: RectifierConfig,
    /// 优化器配置
    pub optimizer_config: OptimizerConfig,
    /// Copilot 优化器配置
    pub copilot_optimizer_config: CopilotOptimizerConfig,
    /// 是否命中 Claude 模型路由（用于抑制 current provider 回写）
    model_route_applied: bool,
}

impl RequestContext {
    /// 创建请求上下文
    ///
    /// # Arguments
    /// * `state` - 代理服务器状态
    /// * `body` - 请求体 JSON
    /// * `headers` - 请求头（用于提取 Session ID）
    /// * `app_type` - 应用类型
    /// * `tag` - 日志标签
    /// * `app_type_str` - 应用类型字符串
    ///
    /// # Errors
    /// 返回 `ProxyError` 如果 Provider 选择失败
    pub async fn new(
        state: &ProxyState,
        body: &serde_json::Value,
        headers: &HeaderMap,
        app_type: AppType,
        tag: &'static str,
        app_type_str: &'static str,
    ) -> Result<Self, ProxyError> {
        Self::new_inner(state, body, headers, app_type, tag, app_type_str, false).await
    }

    /// 创建 Codex compact 请求上下文。
    ///
    /// 与普通请求的区别仅在于：聚合历史模型路由已经失效时，允许从当前启用的
    /// 聚合模型中自动选择一个替代模型完成这次压缩。
    pub async fn new_for_compact(
        state: &ProxyState,
        body: &serde_json::Value,
        headers: &HeaderMap,
        app_type: AppType,
        tag: &'static str,
        app_type_str: &'static str,
    ) -> Result<Self, ProxyError> {
        Self::new_inner(state, body, headers, app_type, tag, app_type_str, true).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn new_inner(
        state: &ProxyState,
        body: &serde_json::Value,
        headers: &HeaderMap,
        app_type: AppType,
        tag: &'static str,
        app_type_str: &'static str,
        is_compact: bool,
    ) -> Result<Self, ProxyError> {
        let start_time = Instant::now();
        let request_id = uuid::Uuid::new_v4().to_string();

        // Codex 本地压缩（local compaction）以普通 /responses 请求发送，路径上
        // 与正式对话无法区分；按请求体中的压缩指令特征识别后同样允许聚合路由降级。
        let detected_local_compaction = !is_compact
            && app_type_str == AppType::Codex.as_str()
            && is_codex_local_compaction_request(body);
        let is_compact = is_compact || detected_local_compaction;

        // 从数据库读取应用级代理配置（per-app）
        let app_config = state
            .db
            .get_proxy_config_for_app(app_type_str)
            .await
            .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        // 从数据库读取整流器配置
        let rectifier_config = state.db.get_rectifier_config().unwrap_or_default();
        let optimizer_config = state.db.get_optimizer_config().unwrap_or_default();
        let copilot_optimizer_config = state.db.get_copilot_optimizer_config().unwrap_or_default();

        let current_provider_id =
            crate::settings::get_current_provider(&app_type).unwrap_or_default();

        // 从请求体提取模型名称
        let request_model = body
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown")
            .to_string();

        // 提取 Session ID
        let session_result = extract_session_id(headers, body, app_type_str);
        let session_id = session_result.session_id.clone();

        if detected_local_compaction {
            log::info!(
                "[{}] 识别到 Codex 本地压缩请求: model={}, session={}, 启用聚合路由自动降级",
                tag,
                request_model,
                session_id
            );
        }

        log::debug!(
            "[{}] Session ID: {} (from {:?}, client_provided: {})",
            tag,
            session_id,
            session_result.source,
            session_result.client_provided
        );

        // 使用共享的 ProviderRouter 选择 Provider（熔断器状态跨请求保持）
        // 注意：只在这里调用一次，结果传递给 forwarder，避免重复消耗 HalfOpen 名额
        let route_result = if is_compact {
            state
                .provider_router
                .select_providers_for_compact_request(app_type_str, body)
                .await
        } else {
            state
                .provider_router
                .select_providers_for_request(app_type_str, body)
                .await
        };
        let (providers, model_route_applied) = route_result.map_err(|e| {
            let message = e.to_string();
            log::error!(
                "[{}] Provider 路由失败: app={}, model={}, compact={}, session={}, error={}",
                tag,
                app_type_str,
                request_model,
                is_compact,
                session_id,
                message
            );
            match e {
                crate::error::AppError::AllProvidersCircuitOpen => {
                    ProxyError::AllProvidersCircuitOpen
                }
                crate::error::AppError::NoProvidersConfigured => ProxyError::NoProvidersConfigured,
                crate::error::AppError::Config(_)
                | crate::error::AppError::InvalidInput(_)
                | crate::error::AppError::Message(_) => ProxyError::InvalidRequest(message),
                _ => ProxyError::DatabaseError(message),
            }
        })?;

        let provider = providers
            .first()
            .cloned()
            .ok_or(ProxyError::NoAvailableProvider)?;

        log::debug!(
            "[{}] Provider: {}, model: {}, failover chain: {} providers, session: {}",
            tag,
            provider.name,
            request_model,
            providers.len(),
            session_id
        );

        Ok(Self {
            start_time,
            request_id,
            endpoint: String::new(),
            app_config,
            provider,
            providers,
            current_provider_id,
            request_model,
            outbound_model: None,
            tag,
            app_type_str,
            app_type,
            session_id,
            session_client_provided: session_result.client_provided,
            rectifier_config,
            optimizer_config,
            copilot_optimizer_config,
            model_route_applied,
        })
    }

    /// 从 URI 提取模型名称（Gemini 专用）
    ///
    /// Gemini API 的模型名称在 URI 中，格式如：
    /// `/v1beta/models/gemini-pro:generateContent`
    pub fn with_model_from_uri(mut self, uri: &axum::http::Uri) -> Self {
        // 用 path() 而不是 path_and_query()：模型名必须从路径段中解析，
        // 否则 GET /v1beta/models/<id>?key=... 会把 query 拼到 request_model 上。
        let endpoint = uri.path();

        self.request_model =
            extract_gemini_model_from_path(endpoint).unwrap_or_else(|| "unknown".to_string());

        self
    }

    /// 创建 RequestForwarder
    ///
    /// 使用共享的 ProviderRouter，确保熔断器状态跨请求保持
    ///
    /// 配置生效规则：
    /// - 故障转移开启：超时配置正常生效（0 表示禁用超时）
    /// - 故障转移关闭：超时配置不生效（全部传入 0）
    pub fn create_forwarder(&self, state: &ProxyState) -> RequestForwarder {
        let (non_streaming_timeout, first_byte_timeout, idle_timeout) =
            if self.app_config.auto_failover_enabled {
                // 故障转移开启：使用配置的值（0 = 禁用超时）
                (
                    self.app_config.non_streaming_timeout as u64,
                    self.app_config.streaming_first_byte_timeout as u64,
                    self.app_config.streaming_idle_timeout as u64,
                )
            } else {
                // 故障转移关闭：不启用超时配置
                log::debug!(
                    "[{}] Failover disabled, timeout configs are bypassed",
                    self.tag
                );
                (0, 0, 0)
            };

        // 故障转移关闭时强制 max_retries=0（仅尝试 1 个 provider），与「不超时 + 不切换」语义一致。
        let max_retries = if self.app_config.auto_failover_enabled {
            self.app_config.max_retries
        } else {
            0
        };

        RequestForwarder::new(
            state.db.clone(),
            self.request_id.clone(),
            self.tag,
            state.provider_router.clone(),
            non_streaming_timeout,
            state.status.clone(),
            state.current_providers.clone(),
            state.gemini_shadow.clone(),
            state.codex_chat_history.clone(),
            state.failover_manager.clone(),
            state.app_handle.clone(),
            self.current_provider_id.clone(),
            self.model_route_applied,
            self.session_id.clone(),
            self.session_client_provided,
            first_byte_timeout,
            idle_timeout,
            self.rectifier_config.clone(),
            self.optimizer_config.clone(),
            self.copilot_optimizer_config.clone(),
            max_retries,
            self.app_config.retry_429_enabled,
            self.app_config.retry_429_max_retries,
            self.app_config.retry_429_initial_delay_ms,
        )
    }

    /// 获取 Provider 列表（用于故障转移）
    ///
    /// 返回在创建上下文时已选择的 providers，避免重复调用 select_providers()
    pub fn get_providers(&self) -> Vec<Provider> {
        self.providers.clone()
    }

    /// 计算请求延迟（毫秒）
    #[inline]
    pub fn latency_ms(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }

    /// 获取流式超时配置
    ///
    /// 配置生效规则：
    /// - 故障转移开启：返回配置的值（0 表示禁用超时检查）
    /// - 故障转移关闭：返回 0（禁用超时检查）
    #[inline]
    pub fn streaming_timeout_config(&self) -> StreamingTimeoutConfig {
        if self.app_config.auto_failover_enabled {
            // 故障转移开启：使用配置的值（0 = 禁用超时）
            StreamingTimeoutConfig {
                first_byte_timeout: self.app_config.streaming_first_byte_timeout as u64,
                idle_timeout: self.app_config.streaming_idle_timeout as u64,
            }
        } else {
            // 故障转移关闭：禁用流式超时检查
            StreamingTimeoutConfig {
                first_byte_timeout: 0,
                idle_timeout: 0,
            }
        }
    }
}

/// Pull the Gemini model name out of an API path.
///
/// Accepts forms like `/v1beta/models/gemini-pro:generateContent`,
/// `/v1/models/gemini-1.5-flash`, `gemini/v1beta/models/<model>:streamGenerateContent`.
/// Returns `None` when no `models/<name>` segment is present.
pub(crate) fn extract_gemini_model_from_path(endpoint: &str) -> Option<String> {
    let segments: Vec<&str> = endpoint.split('/').collect();
    segments
        .iter()
        .position(|s| *s == "models")
        .and_then(|i| segments.get(i + 1).copied())
        // 防御性裁剪：即便调用方传入带 ? 或 :action 的字符串，也只保留 model id 本身
        .map(|s| s.split('?').next().unwrap_or(s))
        .map(|s| s.split(':').next().unwrap_or(s))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Codex 本地压缩（local compaction）提示词特征。
/// 来源：codex-rs `prompts/templates/compact/prompt.md`。app-server 在无法使用远端
/// 压缩（如 `disable_response_storage=true` 或 provider 不支持）时，把该指令作为
/// user message 追加到普通 `/responses` 请求中，正常对话请求不会携带。
/// 注意：用户通过 `compact_prompt` 配置自定义压缩提示词时，此特征不生效。
const CODEX_LOCAL_COMPACTION_PROMPT_MARKERS: &[&str] =
    &["You are performing a CONTEXT CHECKPOINT COMPACTION"];

/// 判断 Codex `/responses` 请求是否为本地压缩请求。
///
/// 本地压缩与正式对话共用 `/responses` 端点，只能通过请求体中注入的压缩指令识别。
fn is_codex_local_compaction_request(body: &serde_json::Value) -> bool {
    let Some(items) = body.get("input").and_then(|v| v.as_array()) else {
        return false;
    };
    items.iter().any(|item| {
        let is_user_message = item.get("type").and_then(|t| t.as_str()) == Some("message")
            && item.get("role").and_then(|r| r.as_str()) == Some("user");
        if !is_user_message {
            return false;
        }
        match item.get("content") {
            Some(serde_json::Value::String(text)) => contains_compaction_marker(text),
            Some(serde_json::Value::Array(parts)) => parts
                .iter()
                .filter_map(|part| part.get("text").and_then(|t| t.as_str()))
                .any(contains_compaction_marker),
            _ => false,
        }
    })
}

fn contains_compaction_marker(text: &str) -> bool {
    CODEX_LOCAL_COMPACTION_PROMPT_MARKERS
        .iter()
        .any(|marker| text.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::extract_gemini_model_from_path;
    use super::is_codex_local_compaction_request;

    #[test]
    fn detects_local_compaction_prompt_in_input() {
        let body = serde_json::json!({
            "model": "qwen3.8-max/dashscope",
            "input": [
                {"type": "message", "role": "user", "content": [
                    {"type": "input_text", "text": "生成最新一期的报表"}
                ]},
                {"type": "message", "role": "user", "content": [
                    {"type": "input_text", "text": "You are performing a CONTEXT CHECKPOINT COMPACTION. Create a handoff summary for another LLM that will resume the task.\n\nInclude:\n- Current progress"}
                ]}
            ]
        });
        assert!(is_codex_local_compaction_request(&body));
    }

    #[test]
    fn detects_local_compaction_prompt_with_string_content() {
        let body = serde_json::json!({
            "model": "kimi-k3/dashscope-chat",
            "input": [
                {"type": "message", "role": "user", "content": "You are performing a CONTEXT CHECKPOINT COMPACTION. Create a handoff summary for another LLM that will resume the task."}
            ]
        });
        assert!(is_codex_local_compaction_request(&body));
    }

    #[test]
    fn ignores_normal_turn_request() {
        let body = serde_json::json!({
            "model": "kimi-k3/dashscope-chat",
            "input": [
                {"type": "message", "role": "user", "content": [
                    {"type": "input_text", "text": "帮我看下 compaction 为什么报错"}
                ]}
            ]
        });
        assert!(!is_codex_local_compaction_request(&body));
    }

    #[test]
    fn ignores_marker_in_assistant_or_developer_messages() {
        let body = serde_json::json!({
            "input": [
                {"type": "message", "role": "assistant", "content": [
                    {"type": "output_text", "text": "You are performing a CONTEXT CHECKPOINT COMPACTION"}
                ]},
                {"type": "message", "role": "developer", "content": [
                    {"type": "input_text", "text": "You are performing a CONTEXT CHECKPOINT COMPACTION"}
                ]}
            ]
        });
        assert!(!is_codex_local_compaction_request(&body));
    }

    #[test]
    fn ignores_request_without_input_items() {
        assert!(!is_codex_local_compaction_request(
            &serde_json::json!({"model": "kimi-k3/dashscope-chat"})
        ));
    }

    #[test]
    fn extract_model_with_action() {
        assert_eq!(
            extract_gemini_model_from_path("/v1beta/models/gemini-pro:generateContent").as_deref(),
            Some("gemini-pro"),
        );
    }

    #[test]
    fn extract_model_with_dotted_version() {
        assert_eq!(
            extract_gemini_model_from_path("/v1beta/models/gemini-1.5-flash:streamGenerateContent")
                .as_deref(),
            Some("gemini-1.5-flash"),
        );
    }

    #[test]
    fn extract_model_without_action() {
        assert_eq!(
            extract_gemini_model_from_path("/v1/models/gemini-1.5-pro").as_deref(),
            Some("gemini-1.5-pro"),
        );
    }

    #[test]
    fn extract_model_with_proxy_prefix() {
        assert_eq!(
            extract_gemini_model_from_path("/gemini/v1beta/models/gemini-2.0-flash:countTokens")
                .as_deref(),
            Some("gemini-2.0-flash"),
        );
    }

    #[test]
    fn extract_model_with_query_string() {
        assert_eq!(
            extract_gemini_model_from_path("/v1beta/models/gemini-pro:generateContent?key=abc")
                .as_deref(),
            Some("gemini-pro"),
        );
    }

    #[test]
    fn extract_model_missing_segment() {
        assert_eq!(extract_gemini_model_from_path("/v1beta/operations"), None);
    }

    #[test]
    fn extract_model_trailing_models_segment() {
        // `/v1beta/models` (list endpoint) has no following segment → None.
        assert_eq!(extract_gemini_model_from_path("/v1beta/models"), None);
    }

    #[test]
    fn extract_model_get_with_query_only() {
        // GET /v1beta/models/<id>?key=... 无 action verb，仅靠 ':' 拆分会把 query 带进 model 名。
        // 修复后应该把 query 剥掉。
        assert_eq!(
            extract_gemini_model_from_path("/v1beta/models/gemini-pro?key=abc").as_deref(),
            Some("gemini-pro"),
        );
    }

    #[test]
    fn extract_model_get_with_proxy_prefix_and_query() {
        assert_eq!(
            extract_gemini_model_from_path("/gemini/v1beta/models/gemini-2.0-flash?key=abc")
                .as_deref(),
            Some("gemini-2.0-flash"),
        );
    }
}
