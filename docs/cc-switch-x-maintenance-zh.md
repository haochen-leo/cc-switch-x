# CC Switch X 分叉与维护策略

## 定位

CC Switch X 是 CC Switch 的非官方独立分叉。目标是：

- 保持上游核心数据库结构和迁移链可持续合并；
- 为 X 自有能力使用独立扩展表，不占用上游 `PRAGMA user_version`；
- 与官方应用使用不同的应用标识、数据目录、代理端口、深链接和发布通道；
- 允许用户在首次启动时只读导入官方数据，但不直接共用官方数据库。

当前 X 吸收的上游基线为提交 `43e1d99084ed9b2f5dc252fd35c5adaf29d6876e`
（CC Switch 3.20.4）。现有本地提交逐条保留，X 产品对齐作为新增提交，不再重放或压缩历史。
`main` 是 X 的开发主线，`release` 是发布分支；两者都属于 X，不是官方主线。
完整保留 `haochen-dev` 的本地功能，再持续吸收明确选定的上游 3.x 修复。

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

当前共享 schema 为 v20，同时保留官方 MCode 字段和已发布的 X OCR 缓存；
独立 X schema 为 v2。已有本地安装迁移时，先停应用、备份整个数据目录，
再将完整数据库和设置复制到 `.cc-switch-x`。下述首次导入器仅复制部分配置，
不能替代已有安装的完整数据迁移。
已有安装可以保留明确在用的代理端口；`15722` 是全新安装的默认值，
不要求已有客户端在迁移数据目录时同时更换端口。

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

## 后续上游 schema 版本

导入器支持版本直接引用 `SCHEMA_VERSION`。后续升级时：

1. 先合并对应上游迁移和测试；
2. 不改写上游 `user_version` 规则；
3. 确认 `x_schema_meta` 和 X 扩展表仍可正常应用；
4. 更新公共列导入测试；
5. 通过后，导入器随 `SCHEMA_VERSION` 接受受支持版本的官方数据库。

旧版 X 遇到不支持的新版数据库只会跳过导入，不会阻止自身启动，也不会降级或修改
官方数据库。

## Git 与发布建议

公开仓库为 `haochen-leo/cc-switch-x`，默认分支为 `main`。
当前本地 remote 为：

- `myfork`：`haochen-leo/cc-switch-x`
- `origin`：`farion1231/cc-switch`（上游，只读跟踪）

X 提交只推送到 X 仓库，不推送到官方 `origin`，经用户指定的授权推送机器完成。
远端常驻分支仅保留 `main` 和 `release`；发布使用独立的 `v0.1.0-beta.N` 标签，
不沿用上游版本号作为 X 的发布号。

日常在 `main` 开发；发布时将已验证的 `main` 快进同步到 `release`，在该提交上
打不可变的 annotated tag，再发布对应 Release。已整理历史不压缩、不重放，
不变基或强推这两条主线。本次旧远端历史完整归档后的一次性主线切换除外，
切换完成后立即恢复 `main` 的禁止删除、禁止非快进保护。

打包与发布全部由 GitHub Actions 完成，本机仅开发和测试。授权推送机器负责
同步提交及推送标签。设置 `X_RELEASE_PIPELINE_ENABLED=true` 后，`v*` 标签
自动触发各系统构建、SHA256 校验和生成及 Beta Release 发布。
Beta 默认使用 macOS ad-hoc 签名，不要求签名、公证凭据；
`X_MACOS_SIGNING_ENABLED` 和 `X_UPDATER_ENABLED` 保持关闭，直到配置 X 自有凭据。

推荐同步流程：

1. 拉取并明确选择上游 3.x 标签或提交，不把上游 4.x 的 `main` 合入这条 3.x 线；
2. 在本地临时同步分支合并所选上游提交，不变基 X 主线；
3. 先解决官方 schema 和配置格式变化；
4. 再处理 X 品牌常量和扩展表；
5. 本机运行 Rust、前端、格式和类型检查，GitHub Actions 运行构建检查；
6. 验证并审查后合并到 X 的 `main`；如使用远端临时 PR 分支，合并后删除；
7. 发版时快进更新 `release` 并打新标签，不改写已有发布标签。

## 本地功能保留原则

历史 `haochen-dev` 中的 Codex 代理、Responses 转换、工具桥接、聚合路由、429
重试、日志正文采集、用量统计及配套 UI 都属于 CC Switch X 的发布功能，合并上游时
必须完整保留。发生冲突时按以下优先级处理：

1. 保留本地功能和用户可见行为；
2. 合并上游安全修复、兼容性修复和新增能力；
3. 品牌、Bundle ID、目录、端口、Deep Link、托管文件名和发布通道使用 X 身份；
4. 上游数据库表与 `SCHEMA_VERSION` 保持官方语义；
5. X 专用持久化能力迁入 `x_` 表，不以删除功能换取 schema 对齐。

同步完成后应通过功能测试确认“本地功能仍在”，而不只是确认代码能够编译。
