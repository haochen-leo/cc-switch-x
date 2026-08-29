# Codex 模型目录字段三方对比：DeepSeek 官方条目 vs GPT-5.5 vs GPT-5.6-Sol

> 记录日期：2026-08-28
>
> 数据来源：
> - DeepSeek 条目：本仓库内置快照 `src-tauri/src/resources/codex_deepseek_catalog_template.json`，已与官方一键脚本 `https://cdn.deepseek.com/api-docs/codex-deepseek-setup-en.sh` 内嵌的 models.json 逐字节比对（flash/pro 两个条目完全一致）。
> - GPT-5.5 / GPT-5.6-Sol：本机 `~/.codex/models_cache.json`（官方 Codex 连接后拉取的真实目录，2026-08-28 10:28 刷新）。
> - 字段语义：本机 Codex 源码 `codex-rs/protocol/src/openai_models.rs`、`codex-rs/core/src/tools/`。

## 一、字段对比表

| 字段 | DeepSeek-V4-Flash | GPT-5.5 | GPT-5.6-Sol |
|---|---|---|---|
| `default_reasoning_level` | **high** | medium | low |
| `supported_reasoning_levels` | low/high/max | low/medium/high/xhigh | low~xhigh + max/ultra |
| `context_window` / `max_context_window` | **1M / 1M** | 272k / 272k | 272k / 872k |
| `use_responses_lite` | false | false | **true** |
| `tool_mode` | null（显式） | 无字段 | **code_mode_only** |
| `multi_agent_version` | **v2** | 无字段 | v2 |
| `comp_hash` | **"3000"** | "2911" | "3000" |
| `include_skills_usage_instructions` | false | true | false |
| `supports_reasoning_summaries` | **true** + format=experimental | 无字段 | 无字段 |
| `minimal_client_version` | **"0.144.0"** | 无字段 | 无字段 |
| `input_modalities` | **只 text** | text+image | text+image |
| `web_search_tool_type` | **text** | text_and_image | text_and_image |
| `additional_speed_tiers` / `service_tiers` | **无** | fast / priority 档 | fast / priority 档 |
| `instructions_variables` | 三个 key 全空串（≈写死） | friendly/pragmatic 两档可切换 | None（写死） |
| `model_messages` 附加块 | 只 approvals（null） | approvals/auto_review/permissions 全 null | 同 5.5 |

三方共有：`supports_parallel_tool_calls: true`、`apply_patch_tool_type: "freeform"`、`shell_type: "shell_command"`、`truncation_policy: {mode: tokens, limit: 10000}`、`default_verbosity: "low"`、`default_reasoning_summary: "none"`、`effective_context_window_percent: 95`、`supported_in_api: true`。

## 二、DeepSeek 条目的三层解读

1. **跟随 5.6 代的部分**：`multi_agent_version: v2`、`comp_hash: "3000"`、skills 说明关闭、人格写死——目录骨架照 5.6 代官方条目抄，不是 5.5 代。
2. **刻意关掉的部分**：`use_responses_lite` 和 `code_mode_only` 没跟 5.6 开。这两个是 OpenAI 内部白名单协议，第三方网关实现不了也不该开；DeepSeek 选普通 Responses + 直接工具面，CC Switch 的 generic 第三方模板同样声明 `use_responses_lite: false` 且 `tool_mode: null`。其中 `use_responses_lite` 会在生成器里强制关闭，`comp_hash` 会硬钉 `"3000"` 避免第三方模型互切触发 hash-change compaction，其他客户端兼容字段以模板为准。
3. **自家重定义的部分**：窗口 1M 全程（官方 5.6 是 272k 起步、872k 封顶）；思考档换成 low/high/max 且默认 high——比 5.5（medium）和 5.6（low）的默认都更重。砍掉图像输入、图文混合 web_search、priority 加速档，属能力减配。
4. **DeepSeek 独有的写法差异**：`supports_reasoning_summaries` 显式写 true + `reasoning_summary_format: "experimental"`（官方 cache 省略该字段，但 serde 缺省即为 true，二者有效值相同——DeepSeek 是显式声明，官方是缺省省略）；`minimal_client_version: "0.144.0"`（freeform apply_patch 注册要求的最低客户端版本）。

## 三、关键字段语义

### `tool_mode`（Direct / CodeMode / CodeModeOnly）

控制模型可见的工具面形态（`codex-rs/protocol/src/openai_models.rs`）：

- **Direct**（默认/字段缺省）：`exec_command`、`apply_patch` 等工具逐个作为 function/custom 工具直接暴露。
- **CodeMode**：直接工具 + code mode 入口（`exec`/`wait`）并存。
- **CodeModeOnly**：可被嵌套调用的直接工具全部从模型可见列表移除（`core/src/tools/spec_plan.rs` 的 `is_hidden_by_code_mode_only`），模型只剩 `exec` 入口，生成 JS 代码在沙箱 worker（node_repl）里以编程方式批量调用被藏的工具。收益是省上下文（工具 schema 不进 prompt）和降往返（多步操作一段代码完成）；前提是模型经过该形态训练。

生效优先级（`core/src/tools/mod.rs` 的 `requested_tool_mode`）：**catalog 声明优先于 feature flag**。`features.code_mode` / `features.code_mode_only` 默认 false 且标 UnderDevelopment，只在 catalog 未声明时兜底。worker 不可用的回落（`effective_tool_mode`）只兜 `CodeMode`；`CodeModeOnly` 不兜底，worker 起不来就 fail closed。

### `use_responses_lite`

开启后 Codex 发 `x-openai-internal-codex-responses-lite` 头，并把工具/指令挪进 `additional_tools` 输入项。OpenAI 对非白名单模型拒绝该头，第三方 Responses 网关不认识这个 item 类型——CC Switch 克隆条目时强制置 false（`codex_config.rs`）。

### `comp_hash`

官方源码注释为「compaction-compatible model configurations」的 opaque id。它不是三方上游协议能力，也不是提示词/tool harness；hash-change compaction 只在上一条目和当前条目都存在 hash 且值不同时触发，`Some <-> None` 本身不会触发。官方/厂商 exact 条目保留；generic 第三方不再继承官方动态 cache，并在模板与生成器两层参照 DeepSeek 硬钉为 `"3000"`，避免第三方模型互切触发 hash-change compaction，也避免 OpenAI 继续下发新 hash 时连带漂移。DeepSeek 官方目录同样沿用与 5.6 相同的 `"3000"`。

### `multi_agent_version: v2`

固定子代理（multi-agent）协议版本，与用户 feature flag 无关。generic 第三方固定模板参照 DeepSeek 写 `"v2"`，等于默认启用 V2 协作工具面；这是客户端行为字段，不是上游 wire protocol 字段。

## 四、harness（系统提示词）与授权语义差异

目录字段之外，三方内嵌的 `base_instructions` 分两套（行级相似度仅 0.216）：

- **GPT-5.5（及 5.4）**：约 21k 字符，"a coding agent based on GPT-5"，含 Engineering judgment、Frontend/Design instructions、Special user requests 等块；授权语义为**推定授权（opt-out）**——"Unless the user explicitly asks for a plan... you assume they want you to make the change... do not stop at a proposal; implement the fix."
- **GPT-5.6-Sol 与 DeepSeek**：17,730 字符且**逐字节相同**，"an agent based on GPT-5"；授权语义为**按请求类型授权（opt-in）**——Answer/Diagnose 类请求明确"do not authorize external writes""Do not implement the fix unless the user asks"，仅两条弱默认授权（只读操作不问、已授权工作流内的正常实现步骤不问）。

即"默认授权改代码"是 5.4/5.5 代的语义，5.6 已收回；DeepSeek 镜像的是 5.6 这套保守授权 harness。harness 教模型用 apply_patch，与目录 `apply_patch_tool_type: "freeform"` 必须保持一致（剥工具留 harness 会自相矛盾）。

## 五、CC Switch 侧的处理规则

- **DeepSeek 官方目录镜像**触发条件（全部满足，`codex_config.rs` 的 `codex_official_vendor_catalog_models`）：
  1. profile 为 NativeResponses（原生 Responses 直连，非代理接管/非 Anthropic 桥接）；
  2. 供应商 `modelCatalog.models` 非空；
  3. 激活 provider 的 `base_url` 小写后 `contains("deepseek.com")`（按 host 不按模型名——能力授权是网关行为，聚合站托管同名模型不授予）。
- 命中后逐字镜像官方条目；用户目录里未匹配官方 slug 的模型克隆官方旗舰条目并改写 slug/显示名（priority 1000+，"保持能力画像但不冒充旗舰"）；用户显式填的 displayName/contextWindow/inputModalities/baseInstructions 覆盖官方值。
- **厂商官方目录条目**：目前只对 DeepSeek native host 生效，镜像厂商官方 models.json；这类是厂商自己声明的 Codex 兼容能力。
- **generic 第三方条目**：不再从官方 `models_cache.json` / 本地 Codex CLI / 静态官方 fallback clone，统一从 `src-tauri/src/resources/codex_third_party_template.json` clone。该模板固定为 5.6 prompt/tool harness + freeform `apply_patch`，reasoning 默认只声明到 `max`、不带绑定自动委派的 `ultra`；中间客户端兼容字段在模板里参照 DeepSeek 声明为 `comp_hash: "3000"`、`tool_mode: null`、`multi_agent_version: "v2"`、`minimal_client_version: "0.144.0"`、`prefer_websockets: false`，且 `comp_hash` 在生成器里再次硬钉为 `"3000"`；仍不带 `available_in_plans`、`default_service_tier`、自动审查/Node REPL 产品开关等字段；`use_responses_lite` 显式 false。
- 目录模板源优先级：DeepSeek native host 走厂商官方 models.json；其他第三方固定走 `codex_third_party_template.json`。官方 `models_cache.json` / 本地 Codex CLI 的动态模板只保留为测试回归材料，不参与 generic 第三方 catalog 生成。

## 六、CC Switch 内置 gpt5_5_template.json vs 官方 cache gpt-5.5

内置快照（`src-tauri/src/resources/gpt5_5_template.json`，2026-05-31 引入）与官方现行 cache 条目（2026-08-28）逐字段对比，差异如下。**注意角色变化**：2026-08-29 起 generic 第三方已改走固定 `codex_third_party_template.json`，本 5.5 快照只保留为 parser-required 字段回填源和测试回归材料，不再向第三方条目提供 harness 或 reasoning 面。

| 字段 | 内置快照 | 官方现行 cache | 说明 |
|---|---|---|---|
| `supported_reasoning_levels` | low/medium/high/xhigh/**max** | low/medium/high/xhigh | **max 是 CC Switch 本地手动追加**（`d7f4991e`，2026-08-11），非历史官方遗留——5/31 首次快照、7 月的 0.144.5/0.144.6 历史官方快照、当前 cache 三处官方 5.5 均无 max |
| `base_instructions` | 21,459 字符（`# Personality` 段直接烘焙进 base） | 19,737 字符（无 Personality 段，运行时由 `{{ personality }}` 变量注入） | 相似度 0.958，授权语义相同（均为 opt-out 推定授权） |
| `availability_nux` | 无字段（2026-08-29 已从快照移除） | null | 官方已下线该宣传；内置快照同步移除 |
| `comp_hash` | 无字段 | "2911" | 内置快照早于 comp_hash 引入 |
| `include_skills_usage_instructions` | 无字段 | true | 同上 |
| `supports_reasoning_summaries` | true | 无字段 | **无字段即 true**——Codex 协议该字段 `serde(default = "default_true", skip_serializing_if = "is_true")`，官方 cache 省略不写恰恰是"支持"的序列化形态；内置写 true 与官方有效值一致。且 CC Switch 注释记录外部目录缺此字段曾被 Codex 启动拒载（parser-required），不可删 |
| `use_responses_lite` | false（2026-08-29 起显式声明） | false | 语义与缺省等价，显式写出与官方形态对齐 |
| `priority` | 0 | 7 | 仅影响官方列表排序 |
| `model_messages` 附加块 | 无 | approvals/auto_review/permissions 全 null | 官方新增的空占位块 |

两者**完全相同的部分**：`instructions_template`（19,754 字符，含 `{{ personality }}` 占位符）与 `instructions_variables`（friendly 1723 / pragmatic 1598）逐字节一致；`context_window` 272k、`apply_patch_tool_type: freeform`、`web_search_tool_type: text_and_image`、`input_modalities: text+image`、`service_tiers` priority 档等能力字段一致。

**来源**：内置文件是 2026-05-31 官方 gpt-5.5 条目的原样快照（`5ef72a20` 引入，初为编译期兜底模板），此后唯一改动是上述 max 档追加。即"快照过旧"与"本地特性修改"两类差异叠加；但这些差异已不再直接进入 generic 第三方目录。

**逐字段影响**（评估时需叠加 CC Switch 生成目录的清洗规则。当前 generic 第三方模板独立后，5.5 快照差异基本只剩 parser-required 回填意义；三方真实 reasoning 面以 `codex_third_party_template.json` 为准：low/medium/high/xhigh/max，不带百炼已确认会 400 的 ultra）：

1. **`supported_reasoning_levels` 多个 max —— 历史风险已隔离**。该列表即客户端思考档下拉的来源；目录多声明时，用户选中后 Codex 会直接向上游发该 effort，客户端不校验上游能力。现 generic 第三方不再从 5.5 快照继承 reasoning 面，而由固定三方模板声明到 max。
2. **`base_instructions` 烘焙 Personality 段 —— 历史风格差异已隔离**。官方现行 base 无人格段（`personality_default` 为空串，默认渲染等于无人格；选 friendly/pragmatic 才经 `{{ personality }}` 注入）；内置把默认人格烘进 base（21,459 = 19,737 + Personality 段 1,722 字符）。现 generic 第三方不再从 5.5 快照继承 base。
3. **`availability_nux` —— 零影响**。上线宣传弹窗文案；CC Switch 生成条目时无条件置 null。
4. **`comp_hash` —— 硬钉 DeepSeek/GPT-5.6 的 `"3000"`**。generic 第三方不再跟随官方动态 cache，而是在三方模板和生成器两层固定 `"3000"`；后续 OpenAI 下发新 hash 不会自动扩散到三方，也减少第三方模型互切触发 hash-change compaction。
5. **`include_skills_usage_instructions` —— 三方模板自行决定**。该字段控制是否向模型注入 skills 目录的使用说明（`ext/skills/extension.rs`）。当前固定三方模板跟随 5.6，显式 false。
6. **`supports_reasoning_summaries` —— 保留 true，与官方一致，无风险**。控制客户端是否发送 `reasoning.summary` 参数（发送还需用户配置非 none）。官方 cache 里字段缺失的真实含义是 true（serde 缺省 true + true 时省略序列化），内置显式 true 与之等价；曾建议"删除对齐官方"系误读，已撤销。百炼实测（2026-08-29，qwen3.8-max，compatible-mode Responses）：不带 reasoning 参数默认即返回 reasoning 摘要；`summary: "detailed"` 接受并回显；`summary: "none"` 接受但抑制无效（摘要仍返回）——百炼属宽容接受型，不会因该参数报错。
7. **`use_responses_lite` —— 零影响**。2026-08-29 起内置快照也显式声明 false（此前为缺省，语义等价），且 CC Switch 生成条目时无条件强制 false。
8. **`priority` —— 零影响**。官方列表排序权重；CC Switch 生成条目统一覆写为 1000+index。
9. **`model_messages` 少 approvals/auto_review/permissions —— 零影响（当前）**。三者是官方为审批提示/自动审查/权限询问文案预留的占位，值全 null，当前无消费逻辑。"缺 key"与"key 存在但为 null"在 serde 反序列化下完全等价（`Option<T>` 均落为 `None`），仅 JSON 文本层面有区别。另：DeepSeek 条目只带 `approvals: null`、缺后两者，说明其目录制作早于这两个占位进入官方目录。

分层结论：真实影响是 1（max 档）、4（固定 comp_hash）、5（skills 说明）、6（推理摘要，条件触发）；2 为风格差异；3/7/8/9 在 CC Switch 清洗规则下被抹平或无关痛痒。


## 七、已知漂移与注意事项

- **百炼（DashScope compatible-mode Responses）特殊字段实测矩阵**（2026-08-29，qwen3.8-max，真实请求）：仅两项在协议层被拒——`reasoning.effort: "ultra"`（报错列出支持值 none/minimal/low/medium/high/xhigh/max）与 `background: true`（"Currently not support background."）。协议层接受：effort minimal~max 全档、text.verbosity 三档、web_search hosted 工具、service_tier priority/flex、truncation auto/disabled、parallel_tool_calls=false、include reasoning.encrypted_content、prompt_cache_key。注意"接受"不等于"执行"，已确认的语义层残缺：① `summary: "none"` 抑制无效（摘要恒返回）；② **custom(freeform) 工具实际不可用**——声明 200、auto 模式可触发 custom_tool_call、回放 custom_tool_call/output 也 200，但调用载荷 `input` 为空壳 `"{}"`（同名 function 工具对照组能产出完整 patch 文本）；③ thinking mode 下 `tool_choice: "required"` 被拒（400），但对象形式指向 function 工具可用——报"不匹配"仅限指名 custom 工具的情形（custom 不在其工具匹配集内）；`none`/`auto` 正常。结论：百炼路径的 freeform apply_patch 必须继续走代理桥接转 function，不能因"声明被接受"放开直连（桥接转 function 后 tool_choice 对象指名反而可用）；桥接层向百炼思考模型转发时应将 required 降级为 auto；百炼系克隆条目思考档可到 max 但**不能带 ultra**。
- 官方 models_cache 与仓库内置 `gpt5_5_template.json` 的逐字段漂移见第六节；generic 第三方真实行为以固定 `codex_third_party_template.json` 为准，不再随最新 `models_cache.json` 漂移。
- DeepSeek 官方脚本已含第三条目 `deepseek-v4-flash-vision-exp`（input_modalities: text+image、supports_image_detail_original: true、priority 3），仓库内置快照（2026-08-26）尚未收录。
- 本机 cache 中 gpt-5.6-terra / gpt-5.6-luna 的 harness 与 5.6-sol 同为 17,730 字符保守授权版。
