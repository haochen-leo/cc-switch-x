# Codex Responses 特殊语义与三链路兼容矩阵

> 日期：2026-08-29  
> 范围：Codex / Grok Build 请求经 CC Switch 接入第三方供应商时，Native Responses、Responses → Chat Completions、Responses → Anthropic Messages 三条链路的语义兼容。  
> 目的：区分“接口能接收基础 Responses JSON”和“能够完整承载 Codex 会话”的差别，避免把单轮文本成功误判为完整兼容。

## 1. 先说结论

很多供应商所说的“支持 Responses API”，实际只覆盖基础语义：

- 接收 `model`、`instructions`、`input`
- 返回文本 `output`
- 支持普通 `function` 工具
- 支持基础流式文本
- 返回基础 token usage

Codex 实际使用的协议面更大。除了 OpenAI Responses 公共结构，还依赖客户端目录字段、ChatGPT 后端私有工具协议、历史回放约束、特殊 SSE 事件和辅助端点。

因此：

- **Native Responses** 是“同协议补缺”：保留 Responses 主体，只修第三方不认识的 Codex 私有语义。代码较少，但更依赖供应商真实实现。
- **Responses → Chat** 是“完整翻译”：重建 `messages`、工具调用、reasoning、usage 和 Responses SSE。
- **Responses → Anthropic** 也是“完整翻译”：重建 `system/messages/tool_use/tool_result/thinking/cache_control`，再恢复为 Responses。

三条链路的 generic 第三方模型目录共用 `codex_third_party_template.json`。提示词和客户端工具声明保持一致，协议差异在代理转换层处理。

## 2. 兼容等级

判断一个供应商的 Responses 兼容程度时，建议使用以下等级，不直接采用“已支持 Responses”的宣传口径。

| 等级 | 能力 | 典型验证 |
| --- | --- | --- |
| R0 JSON 兼容 | 路由存在，能够解析请求并返回 JSON | 单轮非流式文本 |
| R1 基础 Responses | 文本、普通 function、基础 SSE、基础 usage | 单轮工具调用和流式文本 |
| R2 可回放 Responses | function call/output、稳定 ID、历史续聊、状态和 usage 能回放 | 两轮工具调用闭环 |
| R3 Codex 工具兼容 | `namespace`、`tool_search`、`custom_tool_call`、`apply_patch` 可执行 | MCP/插件工具和真实文件修改 |
| R4 Codex 会话兼容 | reasoning、compaction、缓存、媒体、错误和辅助端点均可持续工作 | 长会话、压缩、切换模型、流式工具闭环 |

供应商只完成 R0/R1，不能视为 Codex 完整兼容。

## 3. 状态说明

| 标记 | 含义 |
| --- | --- |
| 完整转换 | CC Switch 明确重建该语义，不依赖上游理解 Codex 原始形态 |
| 补缺适配 | 保留 Responses 主体，仅转换已知不兼容结构 |
| 原生透传 | CC Switch 不重建语义，要求上游正确实现 |
| 条件支持 | 受供应商、模型、配置或路由条件限制 |
| 禁用/不转换 | 当前链路无法无损表达，主动关闭或不做映射 |

## 4. 特殊语义兼容矩阵

### 4.1 客户端目录和请求形态

| 特殊语义 | 风险 | Native Responses | Responses → Chat | Responses → Anthropic |
| --- | --- | --- | --- | --- |
| `use_responses_lite` | Codex 发送内部 header，并把工具/指令放入 `additional_tools` carrier；第三方通常不认识 | 目录统一强制 `false`；xAI 仍有 carrier 提升兜底 | 目录关闭；转换器会收集 `additional_tools` | 目录关闭；共享工具上下文会收集 `additional_tools` |
| `comp_hash` | 切换模型时 hash 变化会触发额外 compaction | 三方目录统一固定 `"3000"` | 同左 | 同左 |
| `apply_patch_tool_type=freeform` | Codex 发 `type=custom`；很多第三方只接受 function | 第三方 Native 做 custom ↔ function 双向桥；OpenAI、DeepSeek 官方原生路径不改写 | custom 转 Chat function，响应恢复 `custom_tool_call` | custom 转 Anthropic `tool_use`，响应恢复 `custom_tool_call` |
| `supports_search_tool` / `tool_search` | 依赖 ChatGPT 后端完成延迟工具发现，普通 Responses 网关不会 materialize | 普通第三方桥成 function 并恢复；xAI OAuth 会清除 carrier，当前不可用 | 通过共享 `CodexToolContext` 转 function 并恢复 | 转 Anthropic `tool_use` 并恢复 |
| `namespace` 工具 | Codex 0.142+ 对自定义 provider 默认发送；严格网关 422，宽松网关静默丢工具 | 所有非官方 Native flatten，响应恢复 `{namespace,name}` | 转换时展开为扁平 function，响应按上下文恢复 | 共享工具上下文展开为 Anthropic tool，响应恢复 |
| `tool_choice` | 自定义类型、namespace、无工具时的 choice 会使严格网关 400 | apply_patch choice 会转换；namespace choice 被中和；xAI 清理悬空 choice；其他形态原生透传 | 映射为 Chat choice；无有效工具时删除 `tool_choice` 和 `parallel_tool_calls` | 映射为 Anthropic choice；处理 thinking 与强制工具冲突 |
| `parallel_tool_calls` | 各协议表达不同，部分网关不支持 | 原生透传，依赖上游 | 转 Chat 字段；无工具时删除 | `false` 映射为 `disable_parallel_tool_use` |

### 4.2 工具、历史和上下文

| 特殊语义 | 风险 | Native Responses | Responses → Chat | Responses → Anthropic |
| --- | --- | --- | --- | --- |
| `custom_tool_call` / output | 第三方只认识 function call，历史无法回放 | apply_patch 做双向转换；其他 custom 类型仍依赖上游 | custom call/output 与 Chat tool call/result 双向转换 | custom call/output 与 `tool_use/tool_result` 双向转换 |
| `tool_search_call/output` | 第三方不认识 carrier，发现后的 MCP 工具下一轮仍不可见 | 提升已发现工具、改写历史 carrier、恢复调用类型和 namespace；xAI 除外 | 转为普通 tool call/result，并保留工具上下文 | 转为 `tool_use/tool_result`，并保留工具上下文 |
| replay item ID | 严格 Responses 网关校验 `msg_`、`rs_`、`fc_` 等前缀 | 请求和响应 item ID 规范化 | 转换入口生成稳定 ID，响应重建 Responses ID | 目标协议不使用 Responses item ID；返回时由转换器生成 Responses 结构并保留 tool call ID |
| `environment_context` 等 user-role 上下文 | 第三方把运行时上下文当成真实用户指令 | 三链分流前统一规范化，官方路径豁免 | 同左 | 同左 |
| compaction handoff | 压缩摘要若仍是 user 消息，会改变对话语义 | 三链分流前恢复 assistant summary；Native compact 请求仍要求上游具备对应 Responses 能力 | compact 请求走 Chat 转换，不要求 Chat 上游理解 Responses compact 结构 | compact 请求走 Anthropic 转换，不要求 Anthropic 上游理解 Responses compact 结构 |
| synthetic runtime context | `<subagent_notification>`、`<turn_aborted>` 等可能污染用户回合 | 分流前恢复为 assistant runtime context | 同左 | 同左 |
| 连续/不完整工具历史 | 严格网关拒绝缺失 tool result、空工具名或不完整 arguments | 上游 Responses 语义为主；已处理已知 ID 和 apply_patch 问题 | 合并连续 assistant，补齐工具上下文；对截断、空名、无 finish reason 做保护 | 删除不完整 tool turn，保证 Anthropic 历史从 user 开始且角色交替合法 |

### 4.3 Reasoning、缓存和计费

| 特殊语义 | 风险 | Native Responses | Responses → Chat | Responses → Anthropic |
| --- | --- | --- | --- | --- |
| `reasoning.effort` | 不同供应商字段名和合法档位不同 | 原生透传，依赖上游；xAI 仅做已知结构清理 | 按 provider 映射为 `reasoning_effort`、`reasoning.effort`、`thinking`、`enable_thinking` 或 `reasoning_split`，并钳制合法档位 | 映射为 Anthropic `thinking` / `output_config.effort`，处理 adaptive thinking |
| reasoning 输出方言 | Chat 网关可能返回 `reasoning_content`、`reasoning`、`reasoning_details` | 原生透传，CC Switch 不重建 reasoning 主体 | 多种字段归一为 Responses reasoning；属于有损语义，不能等同签名 thinking | Anthropic thinking/redacted thinking 转 Responses reasoning |
| `reasoning.encrypted_content` | 工具回放时真 Anthropic 会校验 thinking 签名 | 原生透传，依赖上游 | 不提供 Anthropic 签名语义的等价保证 | strict/lenient 两种策略；签名和无签名 thinking 通过带前缀 carrier 回放 |
| `prompt_cache_key` | 只有部分 Chat 网关支持；乱发会 400 | 原生透传；xAI 删除不支持的 `prompt_cache_retention` | 按 provider 能力和会话注入 `prompt_cache_key` | 不使用该字段，转换为 Anthropic `cache_control` |
| Anthropic `cache_control` | Codex 原始请求没有 Anthropic 缓存断点 | 不适用 | 不适用 | 默认按桥接配置注入 5 分钟 ephemeral cache breakpoint |
| usage / cache token | 各协议字段不同，流式末尾可能不返回 usage | 原生透传，依赖上游 Responses usage 正确性 | 注入 `stream_options.include_usage`；映射 cached/read/write/reasoning tokens | 映射 input/output/thinking/cache read/cache write，SSE 聚合最终 usage |

### 4.4 响应、流式和错误

| 特殊语义 | 风险 | Native Responses | Responses → Chat | Responses → Anthropic |
| --- | --- | --- | --- | --- |
| Responses SSE 事件族 | 只返回 `data:` 文本不等于完整 Responses SSE | 保留上游事件；在流中叠加 apply_patch、namespace、tool_search、ID 恢复 | 从 Chat delta 重建 `response.created`、文本、reasoning、工具参数、`response.completed/failed` | 从 Anthropic SSE 重建同类 Responses 事件 |
| 流式工具参数 | arguments 可能分片、缺 name/id、流提前断开 | apply_patch function arguments delta/done 恢复为 custom input | 按 tool index 累积 name/id/arguments，处理稀疏 index、迟到字段和截断 | 按 content block 累积 `tool_use.input_json_delta`，恢复 function/custom 工具 |
| 完成状态 | `finish_reason`、`stop_reason` 与 Responses status 不等价 | 原生透传，依赖上游 | 映射 `stop/length/tool_calls/content_filter`，构造 incomplete details | 映射 `max_tokens/refusal/context_window` 等 stop reason |
| 2xx 错误 envelope | 部分网关 HTTP 200，但 body 为 `failed` 或包含 `error` | 通用响应层识别语义失败 | 转换层和通用错误处理共同覆盖 | 同左 |
| 非流式 JSON | 工具结构正确但客户端无法派发 | apply_patch、namespace、tool_search、ID 在 JSON 响应中恢复 | Chat JSON 完整重建 Responses output | Anthropic JSON 完整重建 Responses output |

### 4.5 搜索、媒体和辅助端点

| 特殊语义 | 风险 | Native Responses | Responses → Chat | Responses → Anthropic |
| --- | --- | --- | --- | --- |
| `/alpha/search` 专用交互 | 新版 Codex 的独立搜索请求/响应端点，不能塞进 Chat/Anthropic messages | 独立语义透传，要求目标端点或专用 relay 真正实现 | 不转换为 Chat；仍走独立 passthrough | 不转换为 Anthropic；仍走独立 passthrough |
| Responses hosted `web_search` | 只有标准 Responses hosted-tool 实现才会返回可识别的 `web_search_call` 事件；后台搜完只返回文本不算交互适配 | 已知拒绝网关按 host/model 黑名单关闭；其他网关仅原生透传，CC Switch 不合成搜索过程 | 不翻译 hosted tool，也不从后台结果合成 `web_search_call` | 目录层始终关闭，转换器过滤该 hosted tool |
| 供应商后台搜索开关 | `enable_search`、`forced_search` 等由供应商服务端执行，通常只体现在最终文本/引用，客户端看不到发起、查询和结果回放 | 即使供应商内部执行，也只能算后台搜索，不等于 Codex 搜索交互 | 同左；此前 sidecar 式后台搜索方案已回退 | 同左 |
| function 型联网工具 | 能显示普通 function call/result，但没有搜索专用事件、查询状态或 hosted search 语义 | 按普通 function 工具处理，是否联网由工具实现决定 | 按普通 Chat tool call/result 转换 | 按普通 Anthropic `tool_use/tool_result` 转换 |
| 输入图片/文件/音频 | 内容块结构和可接受媒体类型不同 | 原生透传，依赖上游 | 转 Chat multimodal content；不同上游支持度仍需实测 | 图片转 Anthropic image，PDF/file 转 document；不支持类型会降级或忽略 |
| 工具结果中的媒体 | MCP/插件返回图片时，直接塞 tool result 可能被目标协议拒绝 | 原生透传，依赖上游 | 媒体从 tool result 抽出并放入兼容的后续用户媒体消息 | 转为 Anthropic tool_result 中的 image/document block |
| images/files/memories/realtime 等辅助端点 | `/responses` 成功不代表这些端点存在 | 不属于 Responses 主转换；按端点专用 relay 或明确不支持处理，不能从主接口能力推导 | 同左 | 同左 |

## 5. 三条链路的实际覆盖结论

### 5.1 Native Responses

已明确适配：

- namespace flatten/restore
- 普通第三方 `tool_search` bridge
- apply_patch custom ↔ function
- replay/output item ID
- xAI `additional_tools` 提升、私有字段清理和工具白名单
- 流式与非流式的 apply_patch、namespace、tool_search 恢复
- 三链共用的 user-role、compaction 和 synthetic context 整流

仍依赖上游：

- Responses reasoning 主体和 encrypted content
- hosted web search 及其标准 `web_search_call` 事件
- usage、finish status 和基础 SSE 是否符合规范
- 未知的新 Codex/OpenAI 私有字段
- 多模态和 compact 端点的真实支持程度

明确降级：

- xAI OAuth 当前不桥接 `tool_search`，carrier 会被严格清理
- 普通 Native 对未知私有字段采取 fail-open；新严格网关可能需要单独 sanitizer

### 5.2 Responses → Chat

已明确适配：

- `instructions/input` → `system/messages`
- function/custom/namespace/tool_search 双向转换
- tool call 历史、连续 assistant 合并和媒体工具结果
- 多种 reasoning 请求参数和响应字段方言
- prompt cache key、stream usage 和 cache token
- Chat delta → Responses SSE
- finish reason、incomplete 和异常流结束
- 稳定 Responses item ID

限制：

- Chat 协议无法原样承载 Anthropic 签名 thinking
- 不把 hosted `web_search` 或供应商后台搜索结果合成为 Codex 搜索交互
- provider-specific reasoning 方言仍需按平台/模型维护

### 5.3 Responses → Anthropic

已明确适配：

- instructions、system/developer、messages 角色和历史
- function/custom/namespace/tool_search → tool_use/tool_result
- strict/lenient thinking、redacted thinking 和 encrypted carrier
- reasoning effort、adaptive thinking、forced tool choice 冲突
- cache_control、max_tokens、usage/cache token
- 图片、PDF 和工具结果媒体
- Anthropic JSON/SSE → Responses JSON/SSE
- stop reason、incomplete details 和异常流结束

限制：

- Codex hosted `web_search` 当前目录层关闭
- `/alpha/search` 不转换成 Anthropic server tool
- strict thinking 的真实行为取决于上游是否真的执行 Anthropic 签名校验

## 6. 当前未完全闭环的边界

以下项目不能仅靠单测宣称供应商已经完整支持：

1. **MiMo、xAI 的真实 Codex apply_patch 闭环**  
   当前代码和协议单测已覆盖，但缺少与 Qwen 同等级的真实客户端文件修改证据。

2. **xAI tool_search**  
   当前设计是清除不被严格 parser 接受的 carrier，不提供延迟工具发现。

3. **Native 未知私有字段**  
   除 xAI 外不做统一白名单清洗。Codex 新版本增加字段后，严格网关可能重新出现 400/422。

4. **辅助端点**  
   `/responses` 单轮成功不能证明 `/responses/compact`、`/alpha/search`、images、files、memories 或 realtime 可用。当前 realtime 是明确不支持，不应归类为已 relay。

5. **搜索能力**  
   当前只有 `/alpha/search` 属于 Codex 独立搜索交互。标准 Responses hosted `web_search` 只有在 Native 上游真实返回 `web_search_call` 事件时才有可见过程；Chat/Anthropic 不合成该事件。供应商后台搜索开关最多证明服务端联网，普通 function 型联网工具最多证明通用工具调用，两者都不能宣称已适配 Codex 搜索交互。

## 7. 真实验收标准

验证第三方“Codex Responses 兼容”时，至少检查：

1. 单轮流式文本，事件最终到达 `response.completed`。
2. 普通 function 工具真实执行，并把 output 回放到第二轮。
3. namespace/MCP 工具由 Codex 客户端真实派发，不只检查上游出现了扁平工具名。
4. `tool_search` 发现工具后，下一轮工具仍可见并可执行。
5. `apply_patch` 最终修改真实文件，响应形态为 Codex custom tool，而不是只看到 function call。
6. reasoning 在工具回放后仍连续，不丢失、串轮或触发签名错误。
7. usage 包含 input/output，缓存供应商还要检查 cached/read/write token。
8. 长会话执行 compaction 后能够继续工具调用。
9. 切换聚合模型后历史可继续回放，且不因 item ID 或 `comp_hash` 产生异常压缩。
10. 如果供应商宣称支持搜索，必须看到 `/alpha/search` 请求/响应、标准 `web_search_call` 事件，或客户端真实执行的 function call/result；仅凭答案包含实时信息或引用，只能记为“后台搜索，交互不可见”。图片和文件能力另行验证对应端点与媒体回放。

只有完成上述最终客户端结果，才能从“基础 Responses 兼容”提升为“Codex 会话兼容”。

## 8. 代码索引

| 领域 | 代码 |
| --- | --- |
| 三链分流和请求预处理 | `src-tauri/src/proxy/forwarder.rs` |
| 三链响应分流 | `src-tauri/src/proxy/handlers.rs` |
| provider 能力谓词 | `src-tauri/src/proxy/providers/codex.rs` |
| Native namespace | `src-tauri/src/proxy/providers/transform_codex_responses_namespace.rs` |
| Native tool_search | `src-tauri/src/proxy/providers/transform_codex_responses_toolsearch.rs` |
| Native xAI 清理 | `src-tauri/src/proxy/providers/transform_codex_responses_xai_sanitize.rs` |
| apply_patch 双向桥 | `src-tauri/src/proxy/providers/transform_codex_apply_patch.rs` |
| Chat 主转换 | `src-tauri/src/proxy/providers/transform_codex_chat.rs` |
| Chat 流转换 | `src-tauri/src/proxy/providers/streaming_codex_chat.rs` |
| Anthropic 主转换 | `src-tauri/src/proxy/providers/transform_codex_anthropic.rs` |
| Anthropic 流转换 | `src-tauri/src/proxy/providers/streaming_codex_anthropic.rs` |
| user-role / compaction | `src-tauri/src/proxy/providers/transform_codex_compaction.rs` |
| 模型目录和 web_search 开关 | `src-tauri/src/codex_config.rs` |
| 第三方共用模板 | `src-tauri/src/resources/codex_third_party_template.json` |

## 9. 最小回归命令

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib transform_codex_responses -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib transform_codex_chat -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib transform_codex_anthropic -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib codex_config -- --nocapture
```
