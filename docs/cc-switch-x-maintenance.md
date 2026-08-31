# CC Switch X Fork and Maintenance Policy

## Positioning

CC Switch X is an unofficial independent fork of CC Switch. Its goals are to:

- keep the upstream database structure and migration chain mergeable;
- store X-specific persistent features in separate extension tables without
  taking over upstream's `PRAGMA user_version`;
- use a distinct application identity, data directory, proxy port, deep-link
  scheme, and release channel;
- allow a read-only first-run import from official CC Switch without sharing
  the official runtime database.

The current upstream baseline is commit `d8065cc6` from CC Switch 3.20.1.
`legacy/haochen-dev` contains the complete historical CC Switch X product work.
It is retained as a reference branch rather than treated as disposable code.
The release line preserves those local capabilities while continuing to merge
upstream fixes and features.

## Isolation Boundaries

| Item | Official CC Switch | CC Switch X |
| --- | --- | --- |
| Application name | CC Switch | CC Switch X |
| Bundle ID | `com.ccswitch.desktop` | `io.github.haochen-leo.ccswitchx` |
| Data directory | `~/.cc-switch` | `~/.cc-switch-x` |
| Default proxy port | `15721` | `15722` |
| Deep link | `ccswitch://` | `ccswitchx://` |
| Cloud-sync root | `cc-switch-sync` | `cc-switch-x-sync` |
| Managed Codex catalog | `cc-switch-model-catalog.json` | `cc-switch-x-model-catalog.json` |
| Official Codex proxy route ID | `cc-switch-official` | `cc-switch-x-official` |
| Claude Desktop managed profile | Official profile ID | X-specific profile ID |
| In-app updates | Official release channel | Disabled until X has its own signed channel |

Separate data directories do not isolate every external state. Both
applications may still modify live client configuration under `~/.claude`,
`~/.codex`, `~/.gemini`, and similar directories when the user switches a
provider or enables proxy takeover. Do not let both applications concurrently
control the same client; the last writer determines its active configuration.

## Database Rules

1. Upstream tables, migrations, and `SCHEMA_VERSION` keep their official
   semantics.
2. X extensions use `x_`-prefixed tables and a separate `x_schema_meta`
   version.
3. X-specific columns are not added to official tables. Historical local retry
   settings are imported into `x_proxy_retry_config`.
4. The X proxy port is written as migrated data instead of changing the
   official `15721` table default.
5. Cloud sync includes upstream-compatible tables only until a compatibility
   contract exists for X extension tables.

## First-Run Import

When `~/.cc-switch-x/cc-switch.db` does not exist and an official database is
detected, CC Switch X offers an import:

- the source SQLite database is opened read-only;
- only columns shared by both schemas are copied;
- providers, endpoints, MCP servers, prompts, skill metadata and repositories,
  profiles, and selected settings are imported;
- symbolic links are not followed while copying skill files;
- auto-launch, proxy takeover, cloud sync, device migration state, and official
  updater settings are not imported;
- `~/.cc-switch` is never modified, moved, or deleted.

If the source database is newer than the official `SCHEMA_VERSION` supported by
the current X build, import is skipped while X continues to start with its own
database.

## Future Upstream Schema Versions

The importer references the official `SCHEMA_VERSION` directly. For a future
upstream schema update:

1. merge the upstream migrations and tests;
2. preserve upstream `user_version` rules;
3. verify `x_schema_meta` and all X extension tables;
4. update shared-column import tests;
5. accept the newer official database only after those checks pass.

An older X build will skip an unsupported newer official database. It will not
downgrade or modify that database and will still start with its own data.

## Git and Release Workflow

The public repository uses:

- repository: `haochen-leo/cc-switch-x`;
- default branch: `main`;
- `origin`: the CC Switch X repository;
- `upstream`: `farion1231/cc-switch`.

Recommended upstream synchronization:

1. fetch `upstream/main`;
2. merge or rebase on a dedicated synchronization branch;
3. resolve official schema and configuration changes first;
4. reapply X identity and extension-table boundaries;
5. run Rust tests, frontend tests, formatting, type checks, and a production
   build;
6. merge through a reviewed pull request into `main`.

## Local Feature Preservation

The historical Codex proxy, Responses transformations, tool bridges, aggregated
routing, rate-limit handling, request logging, usage accounting, and related UI
are CC Switch X product capabilities. Upstream synchronization must preserve
their user-visible behavior.

Conflict priority:

1. preserve local functionality and user-visible behavior;
2. merge upstream security, compatibility, and feature improvements;
3. keep X-specific identity, directory, port, deep-link, managed-file, and
   release-channel boundaries;
4. retain official semantics for upstream tables and `SCHEMA_VERSION`;
5. move X persistence into `x_` tables instead of deleting functionality to
   simplify schema alignment.

After synchronization, verify that the local capabilities still work rather
than checking compilation alone.
