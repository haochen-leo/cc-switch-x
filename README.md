<div align="center">

# CC Switch X

> Unofficial fork of [CC Switch](https://github.com/farion1231/cc-switch), based on upstream `d8065cc6` (CC Switch 3.20.1). CC Switch X uses an isolated `~/.cc-switch-x` database and does not share the official runtime database.

See the [CC Switch X fork and maintenance policy](docs/cc-switch-x-maintenance-zh.md)
for database compatibility, future v19 upgrades, and the Git workflow.

### Multi-provider routing and compatibility for Claude and Codex, built on CC Switch

[![Version](https://img.shields.io/github/v/release/haochen-leo/cc-switch-x?color=blue&label=version)](https://github.com/haochen-leo/cc-switch-x/releases)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](https://github.com/haochen-leo/cc-switch-x/releases)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)
[![Downloads](https://img.shields.io/github/downloads/haochen-leo/cc-switch-x/total)](https://github.com/haochen-leo/cc-switch-x/releases/latest)

### Upstream project: **[CC Switch](https://github.com/farion1231/cc-switch)**

English | [中文](README_ZH.md) | [日本語](README_JA.md) | [Deutsch](README_DE.md) | [Changelog](CHANGELOG.md)

</div>

## Why CC Switch X?

CC Switch X is a focused fork for users who need Claude and Codex to work reliably across multiple official and third-party model providers. It keeps the desktop management foundation of [CC Switch](https://github.com/farion1231/cc-switch), while concentrating X development on provider aggregation, protocol compatibility, routing, and resilience.

- **Claude multi-provider, per model role** — Map display names and upstream models for each Claude role (Sonnet / Opus / Fable / Haiku), and route different roles to different providers — for example Opus from one endpoint, background Haiku from a cheaper one.
- **Codex multi-provider aggregation** — Official and third-party Codex providers are aggregated into a single entry, so models from every source appear side by side in the same Codex model dropdown and switch seamlessly with the official ones.
- **Deep Responses API adaptation** — Codex speaks the Responses API, whose semantics (reasoning items, tool-call replay, session state, caching behavior) are under-documented and still shifting, while most endpoints that advertise "OpenAI-compatible" behavior silently diverge from it. Making arbitrary upstreams behave correctly under Codex is where most of CC Switch X's adaptation work lives.
- **Robustness under pressure** — Automatic retry with backoff on 429 / rate-limit responses, circuit breaker, and a priority failover queue keep long sessions alive when a provider degrades.

## Screenshots

|                  Claude per-role model mapping & routing                  |                 Codex model source aggregation                 |
| :-----------------------------------------------------------------------: | :------------------------------------------------------------: |
| ![Claude per-role model mapping](assets/screenshots/claude-models-en.png) | ![Codex model sources](assets/screenshots/codex-models-en.png) |

|                                     Codex model picker (desktop app)                                     |
| :------------------------------------------------------------------------------------------------------: |
| ![Codex model picker with official and third-party models](assets/screenshots/codex-model-picker-en.png) |

## Built on CC Switch

[Full Changelog](CHANGELOG.md) | [Release Notes](docs/release-notes/v0.1.0-beta.1-en.md)

CC Switch X retains the upstream project's broad desktop-management capabilities: provider and configuration management for eight AI tools, MCP / Prompts / Skills, tray switching, usage tracking, sessions, cloud sync, import/export, backups, and cross-platform support. See the [upstream project](https://github.com/farion1231/cc-switch) for the general feature set and the [user manual](docs/user-manual/en/README.md) for usage details.

## FAQ

<details>
<summary><strong>Which AI tools does CC Switch support?</strong></summary>

CC Switch supports eight tools: **Claude Code**, **Claude Desktop**, **Codex**, **Gemini CLI**, **Grok Build**, **OpenCode**, **OpenClaw**, and **Hermes**. Each tool has dedicated provider presets and configuration management.

</details>

<details>
<summary><strong>Do I need to restart the terminal after switching providers?</strong></summary>

For most tools, yes — restart your terminal or the CLI tool for changes to take effect. The exception is **Claude Code**, which currently supports hot-switching of provider data without a restart.

</details>

<details>
<summary><strong>My plugin configuration disappeared after switching providers — what happened?</strong></summary>

CC Switch provides a "Shared Config Snippet" feature to pass common data (beyond API keys and endpoints) between providers. Go to "Edit Provider" → "Shared Config Panel" → click "Extract from Current Provider" to save all common data. When creating a new provider, check "Write Shared Config" (enabled by default) to include plugin data in the new provider. All your configuration items are preserved in the default provider imported when you first launched the app.

</details>

<details>
<summary><strong>macOS installation</strong></summary>

CC Switch X beta builds are not signed or notarized with an X-specific Apple certificate yet. Download only from this repository's Releases page, verify the source, and expect a Gatekeeper warning on first launch. We recommend using the `.dmg` installer when it is available.

</details>

<details>
<summary><strong>Why can't I delete the currently active provider?</strong></summary>

CC Switch follows a "minimal intrusion" design principle — even if you uninstall the app, your CLI tools will continue to work normally. The system always keeps one active configuration, because deleting all configurations would make the corresponding CLI tool unusable. If you rarely use a specific CLI tool, you can hide it in Settings. To switch back to official login, see the next question.

</details>

<details>
<summary><strong>How do I switch back to official login?</strong></summary>

Add an official provider from the preset list. After switching to it, run the Log out / Log in flow, and then you can freely switch between the official provider and third-party providers. Codex supports switching between different official providers, making it easy to switch between multiple Plus or Team accounts.

</details>

<details>
<summary><strong>Where is my data stored?</strong></summary>

- **Database**: `~/.cc-switch-x/cc-switch.db` (SQLite — providers, MCP, prompts, skills)
- **Local settings**: `~/.cc-switch-x/settings.json` (device-level UI preferences)
- **Backups**: `~/.cc-switch-x/backups/` (auto-rotated, keeps 10 most recent)
- **Skills**: `~/.cc-switch-x/skills/` (symlinked to corresponding apps by default)
- **Skill Backups**: `~/.cc-switch-x/skill-backups/` (created automatically before uninstall, keeps 20 most recent)

</details>

<details>
<summary><strong>Linux (Wayland + NVIDIA): clicks don't register and the window black-screens on resize</strong></summary>

The AppImage forces `GDK_BACKEND=x11` (XWayland) to avoid a historical native-Wayland crash. On newer Wayland + NVIDIA setups this can leave the web content area unclickable (the title-bar buttons still work) and black-screen on resize. Launch with the opt-in escape hatch to switch back to native Wayland:

```bash
CC_SWITCH_GDK_BACKEND=wayland ./CC-Switch-*.AppImage
```

If you launch from a desktop icon, add it to the `.desktop` `Exec=` line (e.g. `env CC_SWITCH_GDK_BACKEND=wayland /path/to/AppImage`) or set it in your session environment. The variable is generic: on tiling Wayland compositors (sway/Hyprland) where clicks don't register, try `CC_SWITCH_GDK_BACKEND=x11` instead. Leaving it unset keeps the default behavior.

</details>

## Documentation

For detailed guides on every feature, check out the **[User Manual](docs/user-manual/en/README.md)** — covering provider management, MCP/Prompts/Skills, proxy & failover, and more.

## Quick Start

### Basic Usage

1. **Add Provider**: Click "Add Provider" → Choose a preset or create custom configuration
2. **Switch Provider**:
   - Main UI: Select provider → Click "Enable"
   - System Tray: Click provider name directly (instant effect)
3. **Takes Effect**: Restart your terminal or the corresponding CLI tool to apply changes (Claude Code does not require a restart)
4. **Back to Official**: Add an "Official Login" preset, restart the CLI tool, then follow its login/OAuth flow

### MCP, Prompts, Skills & Sessions

- **MCP**: Click the "MCP" button → Add servers via templates or custom config → Toggle per-app sync
- **Prompts**: Click "Prompts" → Create presets with Markdown editor → Activate to sync to live files
- **Skills**: Click "Skills" → Browse GitHub repos → One-click install to supported apps
- **Sessions**: Click "Sessions" → Browse, search, and restore conversation history across supported session sources

> **Note**: On first launch, you can manually import existing CLI tool configs as the default provider.

## Download & Installation

### System Requirements

- **Windows**: Windows 10 and above
- **macOS**: macOS 12 (Monterey) and above
- **Linux**: Ubuntu 22.04+ / Debian 11+ / Fedora 34+ and other mainstream distributions

### Windows Users

Download the latest `CC-Switch-X-v{version}-Windows.msi` installer or `CC-Switch-X-v{version}-Windows-Portable.zip` portable version from the [Releases](../../releases) page.

### macOS Users

**Method 1: Homebrew**

CC Switch X does not have a dedicated Homebrew Cask yet. Use the manual download below or build from source.

**Method 2: Manual Download**

Download `CC-Switch-X-v{version}-macOS.dmg` (recommended) or `.zip` from the [Releases](../../releases) page.

> **Note**: CC Switch X beta builds are not signed or notarized yet. Verify the download source and expect a Gatekeeper warning.

### Arch Linux Users

CC Switch X does not have a dedicated AUR package yet. The upstream `cc-switch-bin` package installs official CC Switch, not CC Switch X.

### Linux Users

Download the latest Linux build from the [Releases](../../releases) page:

- `CC-Switch-X-v{version}-Linux-x86_64.deb` (Debian/Ubuntu)
- `CC-Switch-X-v{version}-Linux-x86_64.rpm` (Fedora/RHEL/openSUSE)
- `CC-Switch-X-v{version}-Linux-x86_64.AppImage` (Universal)

> **Flatpak**: Not included in official releases. You can build it yourself from the `.deb` — see [`flatpak/README.md`](flatpak/README.md) for instructions.

<details>
<summary><strong>Architecture Overview</strong></summary>

### Design Principles

```
┌─────────────────────────────────────────────────────────────┐
│                    Frontend (React + TS)                    │
│  ┌─────────────┐  ┌──────────────┐  ┌──────────────────┐    │
│  │ Components  │  │    Hooks     │  │  TanStack Query  │    │
│  │   (UI)      │──│ (Bus. Logic) │──│   (Cache/Sync)   │    │
│  └─────────────┘  └──────────────┘  └──────────────────┘    │
└────────────────────────┬────────────────────────────────────┘
                         │ Tauri IPC
┌────────────────────────▼────────────────────────────────────┐
│                  Backend (Tauri + Rust)                     │
│  ┌─────────────┐  ┌──────────────┐  ┌──────────────────┐    │
│  │  Commands   │  │   Services   │  │  Models/Config   │    │
│  │ (API Layer) │──│ (Bus. Layer) │──│     (Data)       │    │
│  └─────────────┘  └──────────────┘  └──────────────────┘    │
└─────────────────────────────────────────────────────────────┘
```

**Core Design Patterns**

- **SSOT** (Single Source of Truth): All data stored in `~/.cc-switch-x/cc-switch.db` (SQLite)
- **Dual-layer Storage**: SQLite for syncable data, JSON for device-level settings
- **Dual-way Sync**: Write to live files on switch, backfill from live when editing active provider
- **Atomic Writes**: Temp file + rename pattern prevents config corruption
- **Concurrency Safe**: Mutex-protected database connection avoids race conditions
- **Layered Architecture**: Clear separation (Commands → Services → DAO → Database)

**Key Components**

- **ProviderService**: Provider CRUD, switching, backfill, sorting
- **McpService**: MCP server management, import/export, live file sync
- **ProxyService**: Local proxy mode with hot-switching and format conversion
- **SessionManager**: Conversation history browsing across supported session sources
- **ConfigService**: Config import/export, backup rotation
- **SpeedtestService**: API endpoint latency measurement

</details>

<details>
<summary><strong>Development Guide</strong></summary>

### Environment Requirements

- Node.js 18+
- pnpm 8+
- Rust 1.85+
- Tauri CLI 2.8+

### Development Commands

```bash
# Install dependencies
pnpm install

# Dev mode (hot reload)
pnpm dev

# Type check
pnpm typecheck

# Format code
pnpm format

# Check code format
pnpm format:check

# Run frontend unit tests
pnpm test:unit

# Run tests in watch mode (recommended for development)
pnpm test:unit:watch

# Build application
pnpm build

# Build debug version
pnpm tauri build --debug
```

### Rust Backend Development

```bash
cd src-tauri

# Format Rust code
cargo fmt

# Run clippy checks
cargo clippy

# Run backend tests
cargo test

# Run specific tests
cargo test test_name

# Run tests with test-hooks feature
cargo test --features test-hooks
```

### Testing Guide

**Frontend Testing**:

- Uses **vitest** as test framework
- Uses **MSW (Mock Service Worker)** to mock Tauri API calls
- Uses **@testing-library/react** for component testing

**Running Tests**:

```bash
# Run all tests
pnpm test:unit

# Watch mode (auto re-run)
pnpm test:unit:watch

# With coverage report
pnpm test:unit --coverage
```

### Tech Stack

**Frontend**: React 18 · TypeScript · Vite · TailwindCSS 3.4 · TanStack Query v5 · react-i18next · react-hook-form · zod · shadcn/ui · @dnd-kit

**Backend**: Tauri 2.8 · Rust · serde · tokio · thiserror · tauri-plugin-updater/process/dialog/store/log

**Testing**: vitest · MSW · @testing-library/react

</details>

<details>
<summary><strong>Project Structure</strong></summary>

```
├── src/                        # Frontend (React + TypeScript)
│   ├── components/
│   │   ├── providers/          # Provider management
│   │   ├── mcp/                # MCP panel
│   │   ├── prompts/            # Prompts management
│   │   ├── skills/             # Skills management
│   │   ├── sessions/           # Session Manager
│   │   ├── proxy/              # Proxy mode panel
│   │   ├── openclaw/           # OpenClaw config panels
│   │   ├── settings/           # Settings (Terminal/Backup/About)
│   │   ├── deeplink/           # Deep Link import
│   │   ├── env/                # Environment variable management
│   │   ├── universal/          # Cross-app configuration
│   │   ├── usage/              # Usage statistics
│   │   └── ui/                 # shadcn/ui component library
│   ├── hooks/                  # Custom hooks (business logic)
│   ├── lib/
│   │   ├── api/                # Tauri API wrapper (type-safe)
│   │   └── query/              # TanStack Query config
│   ├── locales/                # Translations (zh/zh-TW/en/ja)
│   ├── config/                 # Presets (providers/mcp)
│   └── types/                  # TypeScript definitions
├── src-tauri/                  # Backend (Rust)
│   └── src/
│       ├── commands/           # Tauri command layer (by domain)
│       ├── services/           # Business logic layer
│       ├── database/           # SQLite DAO layer
│       ├── proxy/              # Proxy module
│       ├── session_manager/    # Session management
│       ├── deeplink/           # Deep Link handling
│       └── mcp/                # MCP sync module
├── tests/                      # Frontend tests
└── assets/                     # Screenshots & partner resources
```

</details>

## Contributing

Issues and suggestions are welcome!

Before submitting PRs, please ensure:

- Pass type check: `pnpm typecheck`
- Pass format check: `pnpm format:check`
- Pass unit tests: `pnpm test:unit`

For new features, please open an issue for discussion before submitting a PR. PRs for features that are not a good fit for the project may be closed.

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=haochen-leo/cc-switch-x&type=Date)](https://www.star-history.com/#haochen-leo/cc-switch-x&Date)

## License

MIT © Jason Young
