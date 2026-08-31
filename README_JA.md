<div align="center">

# CC Switch X

> [CC Switch](https://github.com/farion1231/cc-switch) を基にした非公式フォークです。実行データは独立した `~/.cc-switch-x` に保存されます。

### CC Switch を基盤に、Claude と Codex のマルチプロバイダルーティングと互換性を強化

[![Version](https://img.shields.io/github/v/release/haochen-leo/cc-switch-x?color=blue&label=version)](https://github.com/haochen-leo/cc-switch-x/releases)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](https://github.com/haochen-leo/cc-switch-x/releases)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)
[![Downloads](https://img.shields.io/github/downloads/haochen-leo/cc-switch-x/total)](https://github.com/haochen-leo/cc-switch-x/releases/latest)

### 上流プロジェクト：**[CC Switch](https://github.com/farion1231/cc-switch)**

[English](README.md) | [中文](README_ZH.md) | 日本語 | [Deutsch](README_DE.md) | [Changelog](CHANGELOG.md)

</div>

## CC Switch X を選ぶ理由

CC Switch X は、複数の公式・サードパーティモデルプロバイダで Claude と Codex を安定して利用したいユーザー向けのフォークです。[CC Switch](https://github.com/farion1231/cc-switch) のデスクトップ管理基盤を維持しつつ、プロバイダ集約、プロトコル互換性、モデルルーティング、実行時の堅牢性に開発の重点を置いています。

- **Claude マルチプロバイダ（モデルロール単位）** — Sonnet / Opus / Fable / Haiku の各ロールごとに表示名と上流モデルをマッピングし、ロール別に異なるプロバイダへルーティングできます——例：Opus はあるエンドポイントから、バックグラウンドの Haiku は安価な別エンドポイントから。
- **Codex マルチプロバイダ集約** — 公式・サードパーティの Codex プロバイダを 1 つのエントリに集約し、各ソースのモデルが Codex の同じモデルドロップダウンに並んで表示され、公式モデルとシームレスに切り替えられます。
- **Responses API への深い適合** — Codex は Responses API で通信しますが、そのセマンティクス（推論アイテム、ツール呼び出しの再生、セッション状態、キャッシュ動作）は文書化が不十分で変化し続け、「OpenAI 互換」を謳うエンドポイントの多くはサイレントに乖離しています。任意の上流を Codex 下で正しく動作させることが、CC Switch X が最も注力した適合領域です。
- **堅牢性の強化** — 429 / レート制限時の自動リトライとバックオフ、サーキットブレーカー、優先度フェイルオーバーキューにより、プロバイダが不安定なときも長時間セッションを維持します。

## スクリーンショット

|               Claude ロール別モデルマッピング & ルーティング                |                    Codex モデルソース集約                     |
| :-------------------------------------------------------------------------: | :-----------------------------------------------------------: |
| ![Claude ロール別モデルマッピング](assets/screenshots/claude-models-en.png) | ![Codex モデルソース](assets/screenshots/codex-models-en.png) |

|                              Codex モデル選択（デスクトップアプリ実機）                              |
| :--------------------------------------------------------------------------------------------------: |
| ![公式とサードパーティのモデルが並ぶ Codex モデル選択](assets/screenshots/codex-model-picker-en.png) |

## CC Switch を基盤として

[完全な更新履歴](CHANGELOG.md) | [リリースノート](docs/release-notes/v0.1.0-beta.1-ja.md)

CC Switch X は、8 種類の AI ツール向けプロバイダ・設定管理、MCP / Prompts / Skills、トレイ切り替え、使用量追跡、セッション管理、クラウド同期、インポート・エクスポート、バックアップ、クロスプラットフォーム対応など、上流の幅広いデスクトップ管理機能を継承しています。一般機能は[上流プロジェクト](https://github.com/farion1231/cc-switch)、利用方法は[ユーザーマニュアル](docs/user-manual/en/README.md)を参照してください。

## よくある質問

<details>
<summary><strong>CC Switch はどの AI ツールに対応していますか？</strong></summary>

CC Switch は **Claude Code**、**Claude Desktop**、**Codex**、**Gemini CLI**、**Grok Build**、**OpenCode**、**OpenClaw**、**Hermes** の 8 つのツールに対応しています。各ツールに専用のプロバイダプリセットと設定管理が用意されています。

</details>

<details>
<summary><strong>プロバイダを切り替えた後、ターミナルの再起動は必要ですか？</strong></summary>

ほとんどのツールでは、はい。変更を反映するにはターミナルまたは CLI ツールを再起動してください。ただし **Claude Code** は例外で、現在プロバイダデータのホットスイッチに対応しており、再起動は不要です。

</details>

<details>
<summary><strong>プロバイダを切り替えた後、プラグイン設定が消えてしまいました。どうすればよいですか？</strong></summary>

CC Switch には「共有設定スニペット」機能があり、APIキーやエンドポイント以外の共通データをプロバイダ間で引き継ぐことができます。「プロバイダ編集」→「共有設定パネル」→「現在のプロバイダから抽出」をクリックして、すべての共通データを保存してください。新しいプロバイダを作成する際に「共有設定を適用」にチェック（デフォルトで有効）を入れれば、プラグインなどのデータが新しいプロバイダ設定に含まれます。すべての設定項目は、アプリ初回起動時にインポートされたデフォルトプロバイダに保存されており、失われることはありません。

</details>

<details>
<summary><strong>macOS のインストールについて</strong></summary>

CC Switch X Beta ビルドは、X 固有の Apple 証明書による署名・公証にまだ対応していません。このリポジトリの Releases からのみダウンロードし、配布元を確認してください。初回起動時に Gatekeeper 警告が表示される場合があります。利用可能な場合は `.dmg` インストーラを推奨します。

</details>

<details>
<summary><strong>現在アクティブなプロバイダを削除できないのはなぜですか？</strong></summary>

CC Switch は「最小限の介入」という設計原則に従っています。アプリをアンインストールしても、CLI ツールは正常に動作し続けます。すべての設定を削除すると対応する CLI ツールが使用できなくなるため、システムは常にアクティブな設定を 1 つ保持します。特定の CLI ツールをあまり使用しない場合は、設定で非表示にできます。公式ログインに戻す方法は、次の質問をご覧ください。

</details>

<details>
<summary><strong>公式ログインに戻すにはどうすればよいですか？</strong></summary>

プリセットリストから公式プロバイダを追加してください。切り替え後、ログアウト／ログインのフローを実行すれば、以降は公式プロバイダとサードパーティプロバイダを自由に切り替えられます。Codex では異なる公式プロバイダ間の切り替えに対応しており、複数の Plus アカウントや Team アカウントの切り替えに便利です。

</details>

<details>
<summary><strong>データはどこに保存されますか？</strong></summary>

- **データベース**: `~/.cc-switch-x/cc-switch.db`（SQLite -- プロバイダ、MCP、Prompts、Skills）
- **ローカル設定**: `~/.cc-switch-x/settings.json`（デバイスレベルの UI 設定）
- **バックアップ**: `~/.cc-switch-x/backups/`（自動ローテーション、最新 10 件を保持）
- **Skills**: `~/.cc-switch-x/skills/`（デフォルトでシンボリックリンクにより対応アプリに接続）
- **Skill バックアップ**: `~/.cc-switch-x/skill-backups/`（アンインストール前に自動作成、最新 20 件を保持）

</details>

<details>
<summary><strong>Linux（Wayland + NVIDIA）：Web コンテンツがクリックできない・リサイズで黒画面になる</strong></summary>

AppImage は過去のネイティブ Wayland クラッシュを避けるため `GDK_BACKEND=x11`（XWayland）を強制します。新しい Wayland + NVIDIA 環境ではこれが原因で Web コンテンツ領域がクリックできなくなり（タイトルバーのボタンは動作します）、リサイズ時に黒画面になることがあります。内蔵のエスケープハッチでネイティブ Wayland に戻せます：

```bash
CC_SWITCH_GDK_BACKEND=wayland ./CC-Switch-*.AppImage
```

デスクトップアイコンから起動する場合は、`.desktop` の `Exec=` 行に追記するか（例：`env CC_SWITCH_GDK_BACKEND=wayland /path/to/AppImage`）、セッション環境で設定してください。この変数は汎用です：タイル型 Wayland コンポジタ（sway/Hyprland）でクリックが効かない場合は、逆に `CC_SWITCH_GDK_BACKEND=x11` を試してください。未設定の場合は既定の動作のままです。

</details>

## ドキュメント

各機能の詳しい使い方については、**[ユーザーマニュアル](docs/user-manual/ja/README.md)** をご覧ください。プロバイダ管理、MCP/Prompts/Skills、プロキシとフェイルオーバーなど、すべての機能を網羅しています。

## クイックスタート

### 基本的な使い方

1. **プロバイダ追加**: 「Add Provider」をクリック → プリセットを選ぶかカスタム設定を作成
2. **プロバイダ切り替え**:
   - メイン UI: プロバイダを選択 → 「Enable」をクリック
   - システムトレイ: プロバイダ名をクリック（即時反映）
3. **反映**: ターミナルまたは対応する CLI ツールを再起動して適用（Claude Code は再起動不要）
4. **公式設定に戻す**: 「Official Login」プリセットを追加し、CLI ツールを再起動してログイン/OAuth フローを実行

### MCP、Prompts、Skills & Sessions

- **MCP**: 「MCP」ボタンをクリック → テンプレートまたはカスタム設定でサーバーを追加 → アプリごとの同期をトグルで切り替え
- **Prompts**: 「Prompts」をクリック → Markdown エディタでプリセットを作成 → 有効化してライブファイルに同期
- **Skills**: 「Skills」をクリック → GitHub リポジトリを閲覧 → 対応アプリへワンクリックでインストール
- **Sessions**: 「Sessions」をクリック → 対応するセッションソースの会話履歴を閲覧・検索・復元

> **補足**: 初回起動時に、既存の CLI ツール設定を手動でインポートしてデフォルトプロバイダとして使用できます。

## ダウンロード & インストール

### システム要件

- **Windows**: Windows 10 以上
- **macOS**: macOS 12 (Monterey) 以上
- **Linux**: Ubuntu 22.04+ / Debian 11+ / Fedora 34+ など主要ディストリビューション

### Windows ユーザー

[Releases](https://github.com/haochen-leo/cc-switch-x/releases) ページから最新版の `CC-Switch-X-v{version}-Windows.msi` インストーラー、またはポータブル版 `CC-Switch-X-v{version}-Windows-Portable.zip` をダウンロード。

### macOS ユーザー

**方法 1: Homebrew**

CC Switch X にはまだ専用の Homebrew Cask がありません。以下の手動ダウンロードを使用するか、ソースからビルドしてください。

**方法 2: 手動ダウンロード**

[Releases](https://github.com/haochen-leo/cc-switch-x/releases) から `CC-Switch-X-v{version}-macOS.dmg`（推奨）または `.zip` をダウンロード。

> **注意**: 開発者アカウント未登録のため、初回起動時に「開発元を確認できません」と表示される場合があります。一度閉じてから「システム設定」→「プライバシーとセキュリティ」→「このまま開く」をクリックしてください。以降は通常通り起動できます。

### Arch Linux ユーザー

CC Switch X にはまだ専用の AUR パッケージがありません。上流の `cc-switch-bin` は公式 CC Switch をインストールするもので、CC Switch X ではありません。

### Linux ユーザー

[Releases](https://github.com/haochen-leo/cc-switch-x/releases) から最新版の Linux ビルドをダウンロード：

- `CC-Switch-X-v{version}-Linux-x86_64.deb`（Debian/Ubuntu）
- `CC-Switch-X-v{version}-Linux-x86_64.rpm`（Fedora/RHEL/openSUSE）
- `CC-Switch-X-v{version}-Linux-x86_64.AppImage`（汎用）

> **Flatpak**：公式リリースには含まれていません。`.deb` から自分でビルドできます — 手順は [`flatpak/README.md`](flatpak/README.md) を参照してください。

<details>
<summary><strong>アーキテクチャ概要</strong></summary>

### 設計原則

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

**コア設計パターン**

- **SSOT** (Single Source of Truth): すべてのデータを `~/.cc-switch-x/cc-switch.db`（SQLite）に集約
- **二層ストレージ**: 同期データは SQLite、デバイスデータは JSON
- **双方向同期**: 切り替え時はライブファイルへ書き込み、編集時はアクティブプロバイダから逆同期
- **アトミック書き込み**: 一時ファイル + rename パターンで設定破損を防止
- **並行安全**: Mutex で保護された DB 接続でレースコンディションを防止
- **レイヤードアーキテクチャ**: Commands → Services → DAO → Database を明確に分離

**主要コンポーネント**

- **ProviderService**: プロバイダの CRUD、切り替え、バックフィル、ソート
- **McpService**: MCP サーバー管理、インポート/エクスポート、ライブファイル同期
- **ProxyService**: ローカル Proxy モードのホットスイッチとフォーマット変換
- **SessionManager**: 対応する全アプリの会話履歴閲覧
- **ConfigService**: 設定のインポート/エクスポート、バックアップローテーション
- **SpeedtestService**: API エンドポイントの遅延計測

</details>

<details>
<summary><strong>開発ガイド</strong></summary>

### 開発環境

- Node.js 18+
- pnpm 8+
- Rust 1.85+
- Tauri CLI 2.8+

### 開発コマンド

```bash
# 依存関係をインストール
pnpm install

# ホットリロード付き開発モード
pnpm dev

# 型チェック
pnpm typecheck

# コード整形
pnpm format

# フォーマット検証
pnpm format:check

# フロントエンド単体テスト
pnpm test:unit

# ウォッチモード（開発に推奨）
pnpm test:unit:watch

# アプリをビルド
pnpm build

# デバッグビルド
pnpm tauri build --debug
```

### Rust バックエンド開発

```bash
cd src-tauri

# Rust コード整形
cargo fmt

# clippy チェック
cargo clippy

# バックエンドテスト
cargo test

# 特定テストのみ実行
cargo test test_name

# test-hooks フィーチャー付きでテスト
cargo test --features test-hooks
```

### テストガイド

**フロントエンドテスト**:

- テストフレームワークに **vitest** を使用
- **MSW (Mock Service Worker)** で Tauri API 呼び出しをモック
- コンポーネントテストに **@testing-library/react** を採用

**テスト実行**:

```bash
# 全テストを実行
pnpm test:unit

# ウォッチモード（自動再実行）
pnpm test:unit:watch

# カバレッジレポート付き
pnpm test:unit --coverage
```

### 技術スタック

**フロントエンド**: React 18 · TypeScript · Vite · TailwindCSS 3.4 · TanStack Query v5 · react-i18next · react-hook-form · zod · shadcn/ui · @dnd-kit

**バックエンド**: Tauri 2.8 · Rust · serde · tokio · thiserror · tauri-plugin-updater/process/dialog/store/log

**テスト**: vitest · MSW · @testing-library/react

</details>

<details>
<summary><strong>プロジェクト構成</strong></summary>

```
├── src/                        # フロントエンド (React + TypeScript)
│   ├── components/
│   │   ├── providers/          # プロバイダ管理
│   │   ├── mcp/                # MCP パネル
│   │   ├── prompts/            # Prompts 管理
│   │   ├── skills/             # Skills 管理
│   │   ├── sessions/           # Session Manager
│   │   ├── proxy/              # Proxy モードパネル
│   │   ├── openclaw/           # OpenClaw 設定パネル
│   │   ├── settings/           # 設定 (Terminal/Backup/About)
│   │   ├── deeplink/           # Deep Link インポート
│   │   ├── env/                # 環境変数管理
│   │   ├── universal/          # クロスアプリ設定
│   │   ├── usage/              # 使用量統計
│   │   └── ui/                 # shadcn/ui コンポーネントライブラリ
│   ├── hooks/                  # カスタムフック（ビジネスロジック）
│   ├── lib/
│   │   ├── api/                # Tauri API ラッパー（型安全）
│   │   └── query/              # TanStack Query 設定
│   ├── locales/                # 翻訳 (zh/zh-TW/en/ja)
│   ├── config/                 # プリセット (providers/mcp)
│   └── types/                  # TypeScript 型定義
├── src-tauri/                  # バックエンド (Rust)
│   └── src/
│       ├── commands/           # Tauri コマンド層（ドメイン別）
│       ├── services/           # ビジネスロジック層
│       ├── database/           # SQLite DAO 層
│       ├── proxy/              # Proxy モジュール
│       ├── session_manager/    # セッション管理
│       ├── deeplink/           # Deep Link 処理
│       └── mcp/                # MCP 同期モジュール
├── tests/                      # フロントエンドテスト
└── assets/                     # スクリーンショット & パートナーリソース
```

</details>

## 貢献

Issue や提案を歓迎します！

PR を送る前に以下をご確認ください：

- 型チェック: `pnpm typecheck`
- フォーマットチェック: `pnpm format:check`
- 単体テスト: `pnpm test:unit`

新機能の場合は、PR を送る前に Issue でディスカッションしてください。プロジェクトに合わない機能の PR はクローズされる場合があります。

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=haochen-leo/cc-switch-x&type=Date)](https://www.star-history.com/#haochen-leo/cc-switch-x&Date)

## ライセンス

MIT © Jason Young
