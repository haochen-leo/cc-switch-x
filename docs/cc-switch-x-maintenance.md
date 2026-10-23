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

The current upstream baseline is commit `43e1d99084ed9b2f5dc252fd35c5adaf29d6876e`
from CC Switch 3.20.4. The existing local commits remain individually preserved;
X product alignment is added on top without replaying or squashing that history.
`main` is the X development line and `release` is the release branch; neither
is the official upstream line. Both preserve the local capabilities from
`haochen-dev` while taking explicitly selected upstream 3.x fixes.

## Isolation Boundaries

| Item                           | Official CC Switch             | CC Switch X                                 |
| ------------------------------ | ------------------------------ | ------------------------------------------- |
| Application name               | CC Switch                      | CC Switch X                                 |
| Bundle ID                      | `com.ccswitch.desktop`         | `io.github.haochen-leo.ccswitchx`           |
| Data directory                 | `~/.cc-switch`                 | `~/.cc-switch-x`                            |
| Default proxy port             | `15721`                        | `15722`                                     |
| Deep link                      | `ccswitch://`                  | `ccswitchx://`                              |
| Cloud-sync root                | `cc-switch-sync`               | `cc-switch-x-sync`                          |
| Managed Codex catalog          | `cc-switch-model-catalog.json` | `cc-switch-x-model-catalog.json`            |
| Official Codex proxy route ID  | `cc-switch-official`           | `cc-switch-x-official`                      |
| Claude Desktop managed profile | Official profile ID            | X-specific profile ID                       |
| In-app updates                 | Official release channel       | Disabled until X has its own signed channel |

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

The current shared schema is v20, preserving both upstream MCode fields and
the previously released X OCR cache. The independent X schema remains v2.
When migrating an existing local installation, stop the app, back up the whole
data directory, and copy the complete database and settings into `.cc-switch-x`.
The first-run importer below is selective and does not replace this full-data
migration for an existing installation.
An existing installation may retain its active proxy port. `15722` is the
default for fresh installations, not a requirement to change existing clients
while migrating their data directory.

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

The public repository is `haochen-leo/cc-switch-x`, with default branch `main`.
The current local remotes are:

- `myfork`: `haochen-leo/cc-switch-x`;
- `origin`: `farion1231/cc-switch` (read-only upstream tracking).

Push X commits only to the X repository through the user's authorized push
machine, never to official `origin`. Keep only `main` and `release` as long-lived
remote branches. Use separate `v0.1.0-beta.N` X release tags rather than upstream
version numbers.

Develop on `main`. To publish, fast-forward `release` to a verified `main` commit,
create an immutable annotated tag there, and publish its Release. Do not squash,
replay, rebase, or force-push either mainline. The one-time replacement of the
old remote history is an exception only after a complete archive; immediately
restore `main` deletion and non-fast-forward protection afterward.

All packaging and publishing runs on GitHub Actions, not on the developer's
machine. The authorized push machine synchronizes commits and pushes tags.
With `X_RELEASE_PIPELINE_ENABLED=true`, `v*` tags trigger platform builds,
SHA256 checksum generation, and Beta Release publication. Beta macOS packages
use ad-hoc signing without signing/notarization credentials. Keep
`X_MACOS_SIGNING_ENABLED` and `X_UPDATER_ENABLED` disabled until X credentials
are configured.

Keep the application version as `0.1.0-beta.N` and update
`bundle.windows.wix.version` to an MSI-compatible numeric version at the same
time (for example, `0.1.0.7` for beta.7). MSI cannot derive its version from
the Beta string.

Recommended upstream synchronization:

1. fetch and explicitly select an upstream 3.x tag or commit; do not merge the
   upstream 4.x `main` into this 3.x line;
2. merge the selected upstream commits on a local temporary synchronization
   branch; do not rebase the X mainline;
3. resolve official schema and configuration changes first;
4. reapply X identity and extension-table boundaries;
5. run Rust tests, frontend tests, formatting, and type checks locally; run
   production builds on GitHub Actions;
6. review and merge the verified result into `main`; delete any temporary
   remote pull-request branch after merging;
7. fast-forward `release` and create a new tag when publishing; never rewrite
   an existing release tag.

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
