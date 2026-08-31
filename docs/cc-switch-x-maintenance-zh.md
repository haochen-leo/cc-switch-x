# CC Switch X 分叉与维护策略

## 定位

CC Switch X 是 CC Switch 的非官方独立分叉。目标是：

- 保持上游核心数据库结构和迁移链可持续合并；
- 为 X 自有能力使用独立扩展表，不占用上游 `PRAGMA user_version`；
- 与官方应用使用不同的应用标识、数据目录、代理端口、深链接和发布通道；
- 允许用户在首次启动时只读导入官方数据，但不直接共用官方数据库。

当前 X 吸收的上游基线为提交 `d8065cc6`（CC Switch 3.20.1）。
`legacy/haochen-dev` 是 CC Switch X 当前完整产品实现，也是发布内容的主体，不是待筛选
或准备删除的历史代码。发布分支从该分支创建，完整保留全部本地功能，再持续合并上游
修复与新功能。

## 隔离边界

| 项目                        | 官方 CC Switch                 | CC Switch X                       |
| --------------------------- | ------------------------------ | --------------------------------- |
| 应用名                      | CC Switch                      | CC Switch X                       |
| Bundle ID                   | `com.ccswitch.desktop`         | `io.github.haochen-leo.ccswitchx` |
| 数据目录                    | `~/.cc-switch`                 | `~/.cc-switch-x`                  |
| 默认代理端口                | `15721`                        | `15722`                           |
| Deep Link                   | `ccswitch://`                  | `ccswitchx://`                    |
| 云同步默认根目录            | `cc-switch-sync`               | `cc-switch-x-sync`                |
| Codex 托管目录文件          | `cc-switch-model-catalog.json` | `cc-switch-x-model-catalog.json`  |
| Codex 官方代理路由 ID       | `cc-switch-official`           | `cc-switch-x-official`            |
| Claude Desktop 托管 Profile | 官方 Profile ID                | X 独立 Profile ID                 |
| 应用内更新                  | 官方发布通道                   | 默认关闭，建立 X 发布签名后再启用 |

数据目录隔离不代表所有外部状态都能并发写入。两个应用仍会按用户操作修改
`~/.claude`、`~/.codex`、`~/.gemini` 等客户端实时配置。因此不建议同时开启两个
应用的代理接管或同时切换同一个客户端供应商；最后一次写入者决定客户端当前配置。

## 数据库规则

1. 上游表结构、迁移文件和 `SCHEMA_VERSION` 保持官方语义。
2. X 扩展只写入 `x_` 前缀表，并由 `x_schema_meta` 独立记录版本。
3. 不给官方表增加 X 专用列；历史本地 429 重试列只在导入时转存到
   `x_proxy_retry_config`。
4. X 默认端口作为迁移后的数据写入，不修改官方建表 SQL 中的 `15721` 默认值。
5. 云同步协议只同步官方兼容表；X 扩展表需要单独设计兼容协议后才能加入。

## 首次导入

当 `~/.cc-switch-x/cc-switch.db` 不存在且检测到官方数据库时，X 提示用户选择导入：

- 来源数据库以 SQLite 只读模式打开；
- 仅复制双方都存在的公共列；
- 导入供应商、端点、MCP、Prompts、Skills 元数据、Skill 仓库、Profiles 和常用设置；
- Skills 文件复制时不跟随符号链接；
- 不导入自动启动、代理接管、云同步、本机迁移状态和官方更新设置；
- 不修改、移动或删除 `~/.cc-switch`。

如果来源数据库版本高于当前 X 支持的官方 `SCHEMA_VERSION`，导入被拒绝，但 X
仍使用新的独立数据库继续启动。

## 上游 v19 及后续版本

导入器支持版本直接引用官方 `SCHEMA_VERSION`。因此未来上游升级到 v19 时：

1. 先合并上游 v19 的官方迁移和测试；
2. 不改写上游 `user_version` 规则；
3. 确认 `x_schema_meta` 和 X 扩展表仍可正常应用；
4. 更新公共列导入测试；
5. 通过后，导入器会随 `SCHEMA_VERSION = 19` 自动接受官方 v19 数据库。

旧版 X 遇到官方 v19 数据库只会跳过导入，不会阻止自身启动，也不会降级或修改
官方数据库。

## Git 与发布建议

建议新仓库使用：

- 仓库名：`cc-switch-x`
- 默认分支：`main`
- `origin`：`haochen-leo/cc-switch-x`
- `upstream`：`farion1231/cc-switch`

当前工作分支 `codex/cc-switch-x-release` 在新仓库建立前不要推送到官方 `origin`。
建立新仓库后再调整 remote、提交并推送。

推荐同步流程：

1. 拉取 `upstream/main`；
2. 在独立同步分支合并或变基；
3. 先解决官方 schema 和配置格式变化；
4. 再处理 X 品牌常量和扩展表；
5. 运行 Rust、前端、格式、类型和生产构建检查；
6. 通过 PR 合并到 X 的 `main`。

## 本地功能保留原则

`legacy/haochen-dev` 中的 Codex 代理、Responses 转换、工具桥接、聚合路由、429
重试、日志正文采集、用量统计及配套 UI 都属于 CC Switch X 的发布功能，合并上游时
必须完整保留。发生冲突时按以下优先级处理：

1. 保留本地功能和用户可见行为；
2. 合并上游安全修复、兼容性修复和新增能力；
3. 品牌、Bundle ID、目录、端口、Deep Link、托管文件名和发布通道使用 X 身份；
4. 上游数据库表与 `SCHEMA_VERSION` 保持官方语义；
5. X 专用持久化能力迁入 `x_` 表，不以删除功能换取 schema 对齐。

同步完成后应通过功能测试确认“本地功能仍在”，而不只是确认代码能够编译。
