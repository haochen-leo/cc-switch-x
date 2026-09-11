/**
 * 内置官方供应商种子 id，与 src-tauri/src/database/dao/providers_seed.rs
 * 的 OFFICIAL_SEEDS 保持一致。这些行被聚合目录按固定 id 查找、被接管投影
 * 和历史数据键控，删除会引发"缺少官方供应商"事故，因此不可删除。
 */
const OFFICIAL_SEED_IDS = new Set([
  "claude-official",
  "claude-desktop-official",
  "codex-official",
  "gemini-official",
  "grokbuild-official",
]);

export function isOfficialSeedProviderId(id: string | undefined): boolean {
  return !!id && OFFICIAL_SEED_IDS.has(id);
}
