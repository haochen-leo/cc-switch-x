# Codex 第三方供应商转换：问题归类与链路收口评估

- 日期：2026-08-28（2026-08-29 更新：固定三方模板收口、`comp_hash` 硬钉、Mac mini 新版部署覆盖实测归档）
- 基线：`haochen-dev` @ `71a23800` + 工作区未提交改动（模板统一、L3 实测配套修复）
- 范围：2026-08-15 ~ 2026-08-28 期间 Codex 应用经 CC Switch 代理接入第三方供应商的相关提交、诊断会话与实测记录
- 验证：`cargo test --lib transform_codex` → 236 passed / 0 failed；`cargo test --lib codex_config` → 93 passed / 0 failed；provider 定向测试 → 48 passed / 0 failed；全库 `cargo test --lib` → 2787 passed / 1 failed / 5 ignored，唯一失败为 Claude Desktop 临时目录原子替换竞态，单独重跑通过

## 1. 三条链路的定义

| 链路 | 含义 | 核心代码 |
| --- | --- | --- |
| L1 原生 Responses 透传 | Codex 的 `/responses` 请求经 CC Switch 代理转发到第三方 native Responses 网关（DashScope/百炼、xAI 等），只做适配不改协议 | `transform_codex_responses_namespace.rs`、`transform_codex_responses_toolsearch.rs`、`transform_codex_responses_xai_sanitize.rs`、`transform_codex_apply_patch.rs` |
| L2 Responses 转 Chat 桥 | 供应商只支持 Chat Completions 时做双向协议转换 | `transform_codex_chat.rs`（6242 行：生产约 2559 + 测试约 3683） |
| L3 Responses 转 Anthropic 桥 | 供应商走 Anthropic Messages 协议时做双向转换 | `transform_codex_anthropic.rs`（3447 行：生产约 1706 + 测试约 1741） |

分流入口在 `forwarder.rs:1303-1306`：按 provider 的 base_url / wire_api 决定走 L2 或 L3，都不命中则走 L1。

**总大纲**：L2/L3 是「翻译」——两条桥要处理的问题集完全相同（Codex 私有协议面到目标协议的完整双向映射），区别只在目标方言（Chat 的 messages/tools vs Anthropic 的 thinking 签名/cache_control/max_tokens 等）；两桥共享 `CodexToolContext` 即是这一点的代码证据。L1 是「补缺」——协议本体不动，只解决第三方对 Responses 协议的实现度问题：严格网关 422（xAI 私有字段）、宽松网关静默丢（namespace）、私有契约未实现（tool_search）、ID 校验（replay 归一）、辅助端点缺失（relay）；手段全是请求预处理 + 响应后处理（flatten/sanitize/bridge/restore）。

**两层分工**：cc-switch 对私有面的处理统一遵循「目录层保留提示词/工具 harness，代理层转换上游协议；generic 第三方不从官方动态模板 clone」——三条链路的 generic 第三方条目都使用 cc-switch 内置 `codex_third_party_template.json`：5.6 prompt/tool harness + freeform `apply_patch`，并在模板里参照 DeepSeek 声明中间客户端兼容字段（`comp_hash: "3000"`、`tool_mode: null`、`multi_agent_version: "v2"`、`minimal_client_version: "0.144.0"`、`prefer_websockets: false`）；其中 `comp_hash` 生成时再次硬钉为 `"3000"`，避免第三方模型互切触发 hash-change compaction。其余仍不继承 OpenAI lite、计划/服务档、自动审查、Node REPL 等官方产品字段。Chat、Anthropic、Native 分别在各自转换层把 custom 工具桥成上游可接受的标准工具，再把响应恢复给 Codex。只有 DeepSeek 这种明确发布官方 Codex models.json 的 native host 走厂商官方目录分支；普通三方即使 model id 撞到官方 slug，也走三方模板。namespace 仍由代理层 flatten/restore；web_search 则由已知拒绝网关的顶层配置开关控制。

**历史渊源与模板统一（2026-08-28 已落地，2026-08-29 收口为固定三方模板）**：native 模板的保守源自其出生提交 `15e712e7`（2026-06-27）的设计前提——当时 native 模式是「直连无代理」，目录必须先天干净（commit message 点名 MiMo 拒 custom 类型），reasoning 也只放验证过安全的 none/high。软接管后第三方默认全走代理，该前提已失效，独立模板反而造成 xhigh 钳位和提示词退化。最终收口为：删除 `codex_native_responses_template.json`；Native/Anthropic/Chat 的 generic 第三方条目共用 `codex_third_party_template.json`，不再继承 `models_cache` / 本地 Codex CLI 的官方动态模板；DeepSeek 官方 native host 单独镜像厂商 models.json。三条链路都保留 `base_instructions`、`model_messages`、`apply_patch_tool_type` 与三方模板声明的 reasoning 档位。Native 对严格网关新增 apply_patch custom↔function 双向桥，OpenAI 与 DeepSeek 官方原生 custom 路径不改写。2026-08-29 进一步移除测试专用的 `gpt5_6_template.json` 静态兜底——固定三方模板本身即在 5.6 基线上调整而来，该快照冗余；`gpt5_5_template.json` 仅保留为 parser-required 字段回填源。

## 2. Codex 协议私有面完整清单

以下是转换层需要处理的 Codex 私有协议点全集（对照：lite = #1，tool search = #2，function/namespace = #3/#4，msg 归一 = #5，其余为补充）。

归属列说明：「上游」指 cc-switch 官方 main（Jason/Yeeyzy/oasis/Tsukumi 等社区维护者，以 9a596158 合并基线为准逐一验证）；「昊尘」指本地 `haochen-dev` 提交（`9a596158..HEAD` 共 47 个本地提交全部为其所提交）；「混合」指上游打地基、本地扩展或反向。

| # | 私有面 | 形态 | 第三方风险 | cc-switch 处理 | 链路 | 归属 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | Responses Lite | 模型目录条目带 `use_responses_lite: true` 时（官方 gpt-5.6 条目即如此），Codex 发送 `x-openai-internal-codex-responses-lite` 头，并把 tools/instructions 移入 `additional_tools` input item | OpenAI 对非白名单模型拒绝该头；第三方网关不认识 `additional_tools` item 类型 | catalog 生成条目时强制 `use_responses_lite: false`；转换层兜底 lift `additional_tools`（三链各修过一次：`d20d25dd`/`63fe4c52`/`60266834`）。generic 三方不从官方动态模板 clone，因此 `tool_mode`、`multi_agent_version` 等客户端字段继承三方模板声明；`comp_hash` 例外，生成时硬钉 `"3000"` 防互切 compaction | 全部 | **昊尘**（`395272b0` 强制关闭；三链 lift 兜底） |
| 2 | tool_search 延迟发现 | `tool_search` 声明 + `tool_search_call`/`tool_search_output` items，依赖官方后端 materialize 发现的工具；暴露条件 = 目录字段 `supports_search_tool`（8/28 模板统一后三链均继承官方 true）&& provider capability `namespace_tools`（不可控，恒 true）——即目录层不再压制，代理层桥接/restore 是实际机制 | 第三方不实现该契约，MCP/插件工具不可见 | L1 桥接 + 响应侧扁平名 restore（`54171371`）；L2/L3 经 `CodexToolContext` 映射 | 全部 | **混合**：Chat 桥侧插件恢复为上游 Jason（`59683363`）；L1 原生桥接 + 响应 restore 为昊尘（`54171371`） |
| 3 | namespace 工具声明 | `{"type":"namespace"}` 包裹 function 工具，0.142+ 默认对所有自定义 provider 开启；**客户端无开关**：`ProviderCapabilities.namespace_tools` 对自定义 provider 恒 true（`provider.rs:353-365` 只区分 remote_compaction），无 config.toml 键、不在模型目录 JSON，代理层 flatten/restore 是唯一解 | 严格网关 422 `unknown variant "namespace"`；宽松网关静默丢工具 | L1 flatten + restore（`6b8a31fe`）；L2/L3 转换时自行 unwrap | 全部 | **混合**：flatten 框架为上游 Jason（`dbb5bd15`，仅 xAI）；扩到全部第三方 + restore map 合并为昊尘（`6b8a31fe`） |
| 4 | function 工具语义 | `tool_choice`/`parallel_tool_calls`、`custom_tool_call`（apply_patch 包裹形态） | 无有效工具时严格网关 400；包裹输出不识别；部分 Responses 网关声明 custom 成功但实际生成空 input | 三链都保留 catalog freeform `apply_patch`，由代理把请求侧 custom 桥成目标协议的标准 function/tool，再把响应侧 function/tool_use 恢复为 Codex `custom_tool_call`；L2 无有效工具时剥离 tool_choice | 全部 | **混合**：tool_choice 剥离为上游 oasis（`ea95f39a` #3640）；apply_patch 解包为昊尘（`79ab814d`），Native 双向桥为 8/28 工作区改动 |
| 5 | replay item ID 归一 | `msg_`/`rs_`/`fc_` 等带类型前缀的 item ID | 严格 Responses 网关校验前缀 | L1 跨 provider 规范化（`37c304ed`）；L2 桥入口生成规范 `msg_` ID（`fadf21f2`） | L1/L2 | **混合**：Chat 桥 SSE ID 映射为上游 Jason（`74acf1e3`）；跨 provider 归一与桥入口规范 ID 为昊尘（`37c304ed`/`fadf21f2`/`8603f377`） |
| 6 | reasoning + encrypted_content | `include: ["reasoning.encrypted_content"]`，thinking 块随历史回放 | 真 Anthropic 系验签；第三方不签不验 | L3 strict/lenient 策略分流，lenient 下 carrier 封装回放（`f871cafa`） | 全部 | **混合**：Anthropic 转换基础为上游（`99e11e08` #5071）；strict/lenient 策略为昊尘（`f871cafa`/`792bb8a8`） |
| 7 | user-role 上下文消息 | `environment_context` 等以 user 角色注入的上下文块 | 部分上游不接受该形态 | 三链分叉前统一整流，官方豁免（`b40b1950`/`bbd6b1bc`） | 全部 | **昊尘**（`b40b1950`/`bbd6b1bc`/`d91769a5`） |
| 8 | compaction | `/responses/compact` 端点 + 压缩历史的回放形态 | 端点缺失 404；回放形态不识别 | 端点 relay + 恢复为 assistant summary（`7b150f53`） | 全部 | **混合**：`/responses/compact` 路由为上游 Tsukumi（`0135abde` #1194）；assistant summary 形态为昊尘（`7b150f53`，模块 `10551db8`） |
| 9 | 缓存私有字段 | `prompt_cache_key` / `prompt_cache_retention` | 严格网关 400 | L2 按 provider meta 开关注入；xAI sanitize 剥离 | L1(xAI)/L2 | **上游** Jason（`a078b4b2` session 路由 + meta 开关） |
| 10 | 其他私有字段 | `external_web_access`（递归嵌套）、`safety_identifier`、`service_tier`、`include` | 严格网关 400/422 | xAI sanitize 递归剥离；L3 转换移除 `include`/`service_tier` | L1(xAI)/L3 | **上游为主** Jason（`dbb5bd15`）；昊尘补 additional_tools promote（`60266834`） |
| 11 | 身份指纹头 | `originator`、`session_id`、`chatgpt-account-id`、`OpenAI-Beta` 等 | 身份泄漏给第三方 / 上游拒绝 | forwarder 统一剥离，Anthropic 链另注入 Claude Code 指纹 | 全部 | **上游**（`99e11e08` #5071） |
| 12 | 辅助端点族 | images、alpha/search、memories、files、realtime 等非 `/responses` 端点 | 路由缺失 404 | 专用 raw relay 直连官方（`e59029e4`），realtime 有意 501 | 旁路 | **混合**：alpha/search + Claude WebSearch 为上游（`bdeaac75` #5681）；images/memories/files/realtime relay 与重复路由修复为昊尘（`e59029e4`/`04885015`） |

归属小结：12 项中纯上游 2 项（#9、#11），纯昊尘 1 项（#7），#1 以昊尘为主，其余 8 项为混合。结构性规律：三个转换桥（Chat `1c82b8a3`、Anthropic `99e11e08`、xAI namespace/sanitize `dbb5bd15`）的地基都是上游社区1打的；本地工作集中在两个方向——把「仅官方/仅 xAI」的适配扩展到全部第三方供应商，以及补齐 Codex 新版本引入的私有面（Responses Lite、tool_search L1 桥、user-role 上下文、compaction 形态）。

## 3. 主要问题归类

### A. Codex 私有协议面对第三方不可见（工具系统，最核心的一类）

典型场景：Codex 0.142+ 默认对所有自定义 provider 发送 `{"type":"namespace"}` 工具声明和 `tool_search` 延迟发现契约，这些是 ChatGPT 后端私有能力，第三方网关不实现。

- **namespace 工具声明**：严格网关直接 `422 unknown variant "namespace"`，宽松网关静默丢弃工具（用户看到“工具消失了”）。修复：请求侧 flatten + 响应侧 restore，覆盖所有非官方 provider（`6b8a31fe`）。
- **tool_search 延迟发现**：第三方不做后端 materialization，通过 tool_search 发现的 MCP/插件工具（node_repl、browser 等）不可见。修复：桥接契约（`54171371`）。
- **响应侧 restore 遗漏（半修复事故）**：第一版只做了请求侧，模型返回扁平名 `mcp__node_repl__js` 后客户端报 `unsupported call` 无法派发。响应侧必须把扁平名还原成 namespace/name 结构。这是“验证只到中间产物、没跟到工具真实执行结果”导致的返工。
- **additional_tools carrier**：三条链各自踩过一次，分别修（Chat `d20d25dd`、Anthropic `63fe4c52`、xAI `60266834`）。
- **apply_patch 包裹输入**：第三方输出包裹形态需解包（`79ab814d`）。

### B. 协议转换桥的上游方言兼容矩阵（L2/L3）

- **Chat 桥**：replay 消息需生成规范 `msg_` ID（`fadf21f2`）、合并连续 assistant 消息（`4948fd08`）、无有效工具时剥离 `tool_choice`/`parallel_tool_calls` 防严格网关 400、reasoning 方言按厂商适配（Kimi/Qianfan/BytePlus/StepFun 等 preset 提交）、`prompt_cache_key` 按 provider 开关。
- **Anthropic 桥**：thinking 签名策略分 strict（真 Anthropic 系，验签）与 lenient（第三方兼容端点，不验签）（`f871cafa`、`792bb8a8`）、`max_tokens` 兜底 8192、prompt cache 注入、`[1m]` 上下文标记、Claude Code UA 伪装与指纹头清理。

### C. 路由与接管

- **第三方直连绕过代理**：切换第三方 provider 后若不走代理，所有适配逻辑失效。修复：`ensure_codex_third_party_takeover` 默认软强制接管，官方豁免，用户直连只告警不阻断（`71a23800`）。实现教训：在同步代码里起服务触发 Tokio reactor panic，服务启动必须放在 async 命令边界。
- **辅助端点 404**：imagegen、alpha/search、memories、files、realtime 等 Codex 端点未注册导致 404（imagegen 实测三连 404）。修复：专用 raw relay 直连官方端点，明确拒绝 `/v1/*` 泛匹配（会冲垮 Responses/Chat 主路由）（`e59029e4`）；重复路由注册导致 router build panic（`04885015`）。realtime WebSocket 有意返回 501。

### D. 上下文与历史语义

- **user-role 上下文规范化**：Codex 发往官方的 user 角色上下文消息形态第三方不接受，统一在三链分叉前整流，官方豁免（`b40b1950`，开关 `bbd6b1bc`）。
- **compaction**：压缩历史恢复为 assistant summary 形态（`7b150f53`）。
- **replay item ID**：跨 provider 规范化（`37c304ed`；此前 `8603f377` 曾收缩为 GPT-only，后放宽）。

### E. 稳定性与运维

- 429 原地重试、可配退避（`45be317c`）。
- Kimi 首字节超时：已完成只读诊断，修复方案未验证（遗留）。
- web search sidecar：实验性加入后整体 revert（`05f147e3` 到 `21ef9aec`），属主动收口掉一条实验链。

### F. 验证方法论问题（过程教训）

- “verified live” 只验证到中间产物（工具出现在下一轮回包），没验证客户端真实派发结果，导致半修复。
- 源码已改但运行中 App 是旧二进制：改后必须对比二进制/源码时间戳或重建重启再宣称生效。
- 在 payload 日志里全文搜索被 base64 淹没：先用结构化 `cc-switch.log` 定位路由，再精确提取单次 payload。
- xAI sanitize 与通用 restore 分支合并时必须保留各自语义，不能简单删 xAI 门控。

## 4. 三条链路是否收口

### 已实现的统一收口点

1. **分流统一**：L1/L2/L3 由 `should_convert_codex_responses_to_chat` 与 `should_convert_codex_responses_to_anthropic` 两个谓词在 forwarder 单点决策。
2. **能力谓词集中**（`providers/codex.rs`）：namespace flatten 覆盖所有非官方；tool_search 桥覆盖非官方且非 xAI；xAI sanitize 仅 xAI OAuth；replay-ID 规范化为非 Chat 非 Anthropic。三链对 Codex 私有协议面（namespace、tool_search、additional_tools）的覆盖已补齐。
3. **响应侧统一 restore**：`handlers.rs` 合并 namespace 与 tool_search 两张 restore map，SSE 与非 SSE 走同一恢复逻辑。
4. **链前共用整流**：user-role 规范化、compaction 在三链分叉前统一生效（官方豁免）。
5. **接管收口**：第三方 Codex provider 默认全部路由进代理，不存在“绕过代理导致适配失效”的常态路径。
6. **产品层决策**：Responses-only 聚合阀门已删除（`bb220e56`）。理由是三链已全覆盖私有协议面，阀门不再买安全；产品从“限制协议”转为“全源聚合 + 需要路由徽章提示”。这是 8/27 的明确决策，即产品层主动放弃收口，不再回退。

### 各链状态

| 链路 | 私有协议面 | 单测 | 实测 | 结论 |
| --- | --- | --- | --- | --- |
| L1 原生 Responses | flatten/restore、tool_search 桥、xAI sanitize、apply_patch custom↔function、replay-ID | 236 通过（8/28 工作区） | Mac mini：node_repl `37*41=1517` 通过；本机隔离代理 + 真实 Qwen `qwen3.8-max` 已验证 Codex 0.145 实际执行 `file_change`；MiMo/xAI 缺真实凭据；8/29 mini 新版部署：`gpt-5.4/token-free` 真实 relay 通过 | 代码已收口，三供应商实测门槛未齐 |
| L2 转 Chat | 工具上下文、additional_tools、msg_ ID、历史合并 | 同上 | Mac mini 8/28：node_repl `101*99=9999` 通过；8/29 mini：`deepseek-v4-flash-0731/dashscope` 真实请求通过；apply_patch 桥非流式产出真实 `custom_tool_call`（patch 原文无包裹），流式 SSE 终止于 `response.completed` | 已收口（维护面大，见 5 补充提示） |
| L3 转 Anthropic | thinking 策略、cache、max_tokens、[1m]、UA 伪装 | 同上 | Mac mini 8/28 下午补测（dashscope-anthropic / kimi-k3，5 组场景全过，见 5.2）；8/29 mini：`glm-5.2/dashscope-anthropic` 真实请求通过 | 已收口 |

## 5. 尚未收口 / 建议

1. **分支层未收口（最主要的缺口）**：`haochen-dev` 相对官方 main 分叉大（8/26 时点：本地领先 43 提交、+11845 行；官方 main 已前进到 `3217f72596f2`）。8/26 既定方案"从官方最新 main 新建干净分支、选择性重放 Responses 核心与辅助 relay"未执行。分叉越大，后续同步官方修复的成本越高。
2. **L3 Anthropic 链实测（8/28 已补测，缺口关闭）**：在 Mac mini 经聚合路由 `kimi-k3/dashscope-anthropic`（`dashscope.aliyuncs.com/apps/anthropic`，lenient thinking 策略）实测 5 组场景，payload 日志逐跳核对，全部通过：
   - 基本转换：Responses → Anthropic `/v1/messages`，`max_tokens` 8192 兜底注入，响应转回 Responses 格式；
   - prompt cache：user 消息与工具定义均注入 `cache_control: ephemeral`；
   - 工具回放：turn1 产出 `function_call`（`toolu_*` ID），turn2 出站正确还原为 `tool_use`/`tool_result` 且 ID 匹配，模型基于工具结果闭环作答；
   - thinking lenient：未签名 thinking（`signature:""`）以 `encrypted_content` carrier 带回，回放时还原为 Anthropic thinking block，跨轮上下文连续（391 → 782 推导正确）；
   - 流式 SSE：Anthropic SSE 转 Responses 事件序列完整，终止于 `response.completed`；namespace 声明在 L3 链正确 unwrap 为扁平工具（`demo_ns__ping`）。
   - 残余边界：strict 策略（真 Anthropic 验签端点）无真实环境未测；glm-5.2 模型未测；未经真实 Codex 客户端跑完整会话。
3. **apply_patch 与模板统一（2026-08-28 代码已修，2026-08-29 mini 部署三链与桥接实测闭环，MiMo/xAI 除外）**：`apply_patch_tool_type` 是 Codex 模型目录字段（非提示词），决定客户端是否暴露 freeform custom 工具。历史独立模板造成两档 reasoning、无 apply_patch、系统提示词退化。现在三条链路的第三方 alias 共用固定三方模板：保留 5.6 prompt/tool harness 和 freeform `apply_patch`，中间客户端兼容字段参照 DeepSeek，但不继承官方动态 default 的产品字段。Anthropic 沿用既有 custom 工具桥；Native 新增 custom declaration、tool_choice、历史 `custom_tool_call/output` 到标准 function 的请求转换，并在非流式与 SSE 响应中把 `function_call`、arguments delta/done 恢复为 Codex custom call。Qwen 真实闭环已验证到 Codex 客户端执行 apply_patch。2026-08-29 Mac mini 真实部署覆盖实测（新构建二进制，SHA256 与本地构建一致）：重启后 catalog 投影重新生成，15 个聚合模型 `comp_hash` 全部 `"3000"`（含此前 `"2911"` 的 gpt-5.5 条目）、`tool_mode: null`、`use_responses_lite: false`、freeform `apply_patch`，无 OpenAI 产品字段残留；三链真实请求全通（L2 `deepseek-v4-flash-0731/dashscope`、L3 `glm-5.2/dashscope-anthropic`、L1 `gpt-5.4/token-free`）；apply_patch 桥流式/非流式均产出完整 `custom_tool_call`；期间 proxy_request_logs 全部 200。MiMo/xAI 仍需真实卡片与凭据，不能用协议单测代替。
4. **imagegen 第三方链路待复测**：工具调用已打通，但执行被 OpenAI 网络授权阻塞，需在授权网络环境复测。
5. **Kimi 首字节超时修复未验证**：诊断已完成，修复方案待实施验证。

### 补充提示（不展开，需授权再动）

- L2 Chat 桥维护面：6242 行中生产约 2559 行，per-provider 方言谓词在持续堆叠；新增供应商时回归成本高。后续可考虑把厂商方言收敛为 preset 元数据而非代码分支，但属于结构性重构，需单独评估。
- 官方 main 每次大版本可能改变 Codex 私有协议面（namespace/tool_search 就是 0.142 引入的），建议把“官方端点清单 + 私有协议字段”的 diff 检查纳入升级例行动作。

## 附：本报告涉及的提交（8/15-8/28，按链路与类别）

- L1：`37c304ed` replay-ID、`79ab814d` apply_patch 解包、`54171371` tool_search 桥、`6b8a31fe` namespace flatten、`60266834` xAI additional_tools
- L2：`fadf21f2` msg_ ID、`d20d25dd` additional_tools、`4948fd08` 历史合并
- L3：`63fe4c52` additional_tools、`792bb8a8` budget_tokens、`f871cafa` thinking lenient
- 路由/接管：`e59029e4` 辅助端点 relay、`04885015` 重复路由、`71a23800` 软接管、`2413de05` adapter 加固
- 上下文：`b40b1950`/`bbd6b1bc`/`d91769a5` user-role、`7b150f53` compaction、`58ce5140` 问题记录文档
- 稳定性：`45be317c` 429 重试
- 产品收口：`bb220e56` 删 Responses-only 阀门、`21ef9aec` revert web search sidecar
