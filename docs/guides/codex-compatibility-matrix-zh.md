# Codex 兼容性矩阵

> 日期：2026-08-20  
> 范围：CC Switch 接管 Codex / Grok Build 请求后，到第三方上游之间的协议适配层。  
> 目的：把已经落地的适配项按链路归档，后续排查 trace、补单测、发布验证时按表定位。

## 总览

CC Switch 在 Codex 链路上已经不只是转发代理，而是 Codex compatibility layer。核心职责是把 Codex 客户端发出的 OpenAI Responses 形态，转换成第三方上游能接受、同时 Codex 客户端又能继续续聊的结构。

当前适配集中在 8 类：

- 路由与模型目录
- Responses native 兼容
- Responses -> Chat Completions bridge
- Responses -> Anthropic Messages bridge
- tool call / `apply_patch`
- reasoning / thinking
- compaction / synthetic context
- trace / payload capture

## 兼容性矩阵

| 类别                      | 链路                                         | 触发条件                                                                         | 输入结构                                                        | 输出结构                                                       | 代码入口                                                                                                                                              | 开关                                        | 验证命令                                                                                                             |
| ------------------------- | -------------------------------------------- | -------------------------------------------------------------------------------- | --------------------------------------------------------------- | -------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| 路由与模型目录            | Codex provider -> route target               | Codex provider 使用聚合模型 slug                                                 | `model/provider` 语义需要稳定拆分                               | 路由按 model/provider 选择目标                                 | `src-tauri/src/proxy/provider_router.rs`、`src-tauri/src/services/codex_aggregation.rs`                                                               | 无                                          | `cargo test --manifest-path src-tauri/Cargo.toml codex_aggregation -- --nocapture`                                   |
| reasoning 默认值          | Codex Chat provider form                     | 添加 / 编辑 Codex Chat provider                                                  | provider 表单缺少默认 reasoning 选项                            | 写入默认 reasoning 配置                                        | `src/components/providers/forms/ProviderForm.tsx`                                                                                                     | 表单配置                                    | `pnpm test tests/components/ProviderForm.codexCatalog.test.ts`                                                       |
| Responses native ID       | Codex Responses -> native Responses upstream | 第三方 native Responses 返回非 Codex 可重放 ID                                   | output item id / replay item id 不符合 Codex 历史预期           | 归一化 replay/output item ID                                   | `src-tauri/src/proxy/forwarder.rs`、`src-tauri/src/proxy/providers/codex.rs`、`src-tauri/src/proxy/providers/transform_codex_chat.rs`                 | 自动                                        | `cargo test --manifest-path src-tauri/Cargo.toml normalize_response_output_item_ids -- --nocapture`                  |
| Chat bridge ID            | Codex Responses -> Chat Completions          | Chat bridge 入口缺少 canonical message id                                        | Responses input / output 没有稳定 `msg_`                        | 生成 canonical `msg_` ID                                       | `src-tauri/src/proxy/providers/transform_codex_chat.rs`、`src-tauri/src/proxy/providers/streaming_codex_chat.rs`                                      | 自动                                        | `cargo test --manifest-path src-tauri/Cargo.toml canonical -- --nocapture`                                           |
| Chat bridge 主转换        | Codex Responses -> Chat Completions          | provider upstream format 为 Chat                                                 | `input`、`instructions`、Responses tool / function call         | Chat `messages`、`tools`、stream delta                         | `src-tauri/src/proxy/providers/transform_codex_chat.rs`、`src-tauri/src/proxy/handlers.rs`                                                            | provider upstream format                    | `cargo test --manifest-path src-tauri/Cargo.toml transform_codex_chat -- --nocapture`                                |
| Anthropic bridge 主转换   | Codex Responses -> Anthropic Messages        | provider upstream format 为 Anthropic                                            | Responses `input`、`tools`、reasoning item、image / pdf part    | Anthropic `messages`、`system`、`tools`、`thinking`            | `src-tauri/src/proxy/providers/transform_codex_anthropic.rs`、`src-tauri/src/proxy/providers/streaming_codex_anthropic.rs`                            | provider upstream format                    | `cargo test --manifest-path src-tauri/Cargo.toml transform_codex_anthropic -- --nocapture`                           |
| `apply_patch` 参数        | upstream Responses output -> Codex tool call | 第三方模型把 freeform patch 包进 JSON 字段                                       | `custom_tool_call.input` 为 `{"patch":"*** Begin Patch..."}` 等 | 解包成 raw freeform patch；`{}` 保持不可恢复                   | `src-tauri/src/proxy/providers/transform_codex_apply_patch.rs`、`src-tauri/src/proxy/handlers.rs`                                                     | 自动                                        | `cargo test --manifest-path src-tauri/Cargo.toml transform_codex_apply_patch -- --nocapture`                         |
| reasoning / thinking 回填 | Chat / Anthropic upstream -> Codex Responses | 上游用 `reasoning_content`、`reasoning`、`reasoning_details`、Anthropic thinking | 非标准 reasoning 字段或 Anthropic thinking block                | 转回 Codex 可消费的 reasoning / output 结构                    | `src-tauri/src/proxy/providers/codex_chat_common.rs`、`src-tauri/src/proxy/providers/transform_codex_anthropic.rs`、`src-tauri/src/proxy/handlers.rs` | provider 能力配置                           | `cargo test --manifest-path src-tauri/Cargo.toml reasoning -- --nocapture`                                           |
| compaction handoff        | Codex local compaction -> 任意第三方上游     | 文本以 Codex handoff prefix 开头                                                 | `role=user` + `input_text`，内容是历史 summary                  | `role=assistant` + `output_text` + `<conversation-checkpoint>` | `src-tauri/src/proxy/providers/transform_codex_compaction.rs`                                                                                         | `codexUserRoleContextNormalization`，默认开 | `cargo test --manifest-path src-tauri/Cargo.toml proxy::providers::transform_codex_compaction::tests -- --nocapture` |
| synthetic runtime context | Codex / Grok Build -> 任意第三方上游         | `<subagent_notification>` 或 `<turn_aborted>` 完整标签                           | `role=user` + `input_text`，内容是运行时通知                    | `role=assistant` + `output_text`，保留原标签                   | `src-tauri/src/proxy/providers/transform_codex_compaction.rs`、`src-tauri/src/proxy/forwarder.rs`                                                     | `codexUserRoleContextNormalization`，默认开 | `cargo test --manifest-path src-tauri/Cargo.toml codex_user_role_context_normalization -- --nocapture`               |
| payload capture           | Codex proxy request / response               | 设置中开启 payload capture                                                       | 原始 request / response / SSE                                   | trace archive，便于核对真实上游结构                            | `src-tauri/src/proxy/payload_capture.rs`、`src-tauri/src/proxy/handlers.rs`、`src-tauri/src/proxy/response_processor.rs`                              | log config                                  | `cargo test --manifest-path src-tauri/Cargo.toml payload_capture -- --nocapture`                                     |

## 按链路看

### Native Responses 上游

目标是保留 Codex Responses 协议，只修正第三方 provider 与 Codex 客户端之间的兼容差异。

必须关注：

- replay item ID / output item ID
- `apply_patch` output input
- compaction / synthetic context 的 user-role 污染
- payload trace 取证

核心入口：

- `src-tauri/src/proxy/forwarder.rs`
- `src-tauri/src/proxy/providers/codex.rs`
- `src-tauri/src/proxy/providers/transform_codex_apply_patch.rs`
- `src-tauri/src/proxy/providers/transform_codex_compaction.rs`

### Chat Completions 上游

目标是让 Codex 仍发送 Responses 请求，CC Switch 转成 Chat Completions，上游返回后再还原成 Codex 能续聊的 Responses 形态。

必须关注：

- `msg_` ID
- Responses input -> Chat messages
- Chat streaming delta -> Responses SSE
- tool call 参数
- reasoning 字段别名
- `apply_patch`
- compaction / synthetic context

核心入口：

- `src-tauri/src/proxy/providers/transform_codex_chat.rs`
- `src-tauri/src/proxy/providers/streaming_codex_chat.rs`
- `src-tauri/src/proxy/handlers.rs`

### Anthropic Messages 上游

目标是把 Codex Responses 映射到 Anthropic Messages，再把 Anthropic JSON / SSE 映射回 Responses。

必须关注：

- `instructions` / system / developer 合并
- Anthropic `messages`
- tool schema / tool use / tool result
- thinking / redacted thinking / encrypted reasoning
- image / pdf part
- cache control
- stop reason
- compaction / synthetic context

核心入口：

- `src-tauri/src/proxy/providers/transform_codex_anthropic.rs`
- `src-tauri/src/proxy/providers/streaming_codex_anthropic.rs`
- `src-tauri/src/proxy/providers/reasoning_bridge.rs`

## 开关清单

| 开关                                | 默认             | 作用                                                        | 位置                                     |
| ----------------------------------- | ---------------- | ----------------------------------------------------------- | ---------------------------------------- |
| provider upstream format            | 按 provider 配置 | 选择 native Responses / Chat / Anthropic 链路               | provider 配置                            |
| `codexUserRoleContextNormalization` | 开               | 修复 compaction / subagent / turn aborted 的 user-role 污染 | 设置 -> 代理 -> 整流器 -> Codex 请求整流 |
| payload capture                     | 关               | 记录真实 request / response / SSE payload                   | 设置 -> 日志                             |
| Codex Chat reasoning options        | 按 provider 配置 | 控制 Chat bridge reasoning 能力                             | provider 表单                            |

## 最小回归命令

只改 Codex user-role context：

```bash
cargo test --manifest-path src-tauri/Cargo.toml proxy::providers::transform_codex_compaction::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml codex_user_role_context_normalization -- --nocapture
```

只改 `apply_patch`：

```bash
cargo test --manifest-path src-tauri/Cargo.toml transform_codex_apply_patch -- --nocapture
```

只改 Chat bridge：

```bash
cargo test --manifest-path src-tauri/Cargo.toml transform_codex_chat -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml streaming_codex_chat -- --nocapture
```

只改 Anthropic bridge：

```bash
cargo test --manifest-path src-tauri/Cargo.toml transform_codex_anthropic -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml streaming_codex_anthropic -- --nocapture
```

改日志 / payload capture：

```bash
cargo test --manifest-path src-tauri/Cargo.toml payload_capture -- --nocapture
```

改前端设置：

```bash
pnpm typecheck
```

## 最近适配提交

| Commit     | 日期       | 分类                    | 内容                                                                       |
| ---------- | ---------- | ----------------------- | -------------------------------------------------------------------------- |
| `f99db787` | 2026-08-17 | 路由与模型目录          | `refactor(codex): order aggregate model slug as model/provider`            |
| `51ebc319` | 2026-08-17 | reasoning 默认值        | `feat: default codex chat reasoning options`                               |
| `37c304ed` | 2026-08-18 | Responses native ID     | `fix(codex): normalize replay item IDs across native Responses providers`  |
| `79ab814d` | 2026-08-18 | `apply_patch`           | `feat(codex): unwrap wrapped apply_patch inputs in Responses output`       |
| `fadf21f2` | 2026-08-18 | Chat bridge ID          | `fix(codex): generate canonical msg_ IDs at Chat Completions bridge entry` |
| `10551db8` | 2026-08-20 | trace / payload capture | `Trace Codex proxy payloads`                                               |
| `7b150f53` | 2026-08-20 | compaction              | `Restore Codex compaction as assistant summary`                            |
| `b40b1950` | 2026-08-20 | synthetic context       | `fix(proxy): normalize Codex user-role context`                            |
| `bbd6b1bc` | 2026-08-20 | 设置开关                | `feat(settings): expose Codex context normalization toggle`                |
| `58ce5140` | 2026-08-20 | 文档                    | `docs: record Codex user-role context issue`                               |

## 排查新问题时的定位顺序

1. 先看真实 payload，不按 Codex UI 或模型复述判断。
2. 判断链路：native Responses、Chat bridge、Anthropic bridge。
3. 对照矩阵找对应类别：ID、role、content part、tool call、reasoning、SSE、compaction。
4. 如果 payload 里出现 Codex synthetic context 且仍是 `role=user`，先查 `codexUserRoleContextNormalization`。
5. 如果 payload 已被归一化，再查上游 provider 对该结构的接受程度。
6. 补修复时，同步在本文件新增一行矩阵记录和最小回归命令。
