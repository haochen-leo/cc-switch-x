import { useCallback, useEffect, useRef, useState } from "react";
import { stripClaudeOneMMarker } from "./useModelState";

/**
 * 模型图片输入支持的三态声明。
 *
 * - auto: 不写显式声明，由内置纯文本模型名单 + 上游报错兜底决定；
 * - supported: 声明 inputModalities 包含 image；
 * - unsupported: 声明 inputModalities 仅包含 text。
 *
 * 持久化位置：供应商 settingsConfig 的 modelCatalog.models[]，与后端
 * model_capabilities.rs 的读取路径一致（Claude / Codex 供应商通用）。
 */
export type ModelImageSupport = "auto" | "supported" | "unsupported";

export interface ModelImageSupportEntry {
  name: string;
  support: ModelImageSupport;
}

type SettingsObject = Record<string, unknown>;

function normalizeModelKey(model: string): string {
  return stripClaudeOneMMarker(model).trim().toLowerCase();
}

/// 与后端 model_ids_match 对齐：忽略大小写、[1M] 后缀，允许命名空间尾匹配。
function modelKeysMatch(candidate: string, model: string): boolean {
  const a = normalizeModelKey(candidate);
  const b = normalizeModelKey(model);
  if (!a || !b) return false;
  if (a === b) return true;
  const aTail = a.split("/").pop() ?? a;
  const bTail = b.split("/").pop() ?? b;
  return aTail === b || a === bTail || aTail === bTail;
}

export function getImageSupportFromModalities(
  inputModalities: string[] | undefined,
): ModelImageSupport {
  if (!inputModalities || inputModalities.length === 0) return "auto";
  return inputModalities.some((item) => item.trim().toLowerCase() === "image")
    ? "supported"
    : "unsupported";
}

/// 在保留其他模态声明的前提下重写 text/image 两项；auto 时不写声明。
export function withImageSupportInModalities(
  existing: string[] | undefined,
  support: ModelImageSupport,
): string[] | undefined {
  const others = (existing ?? []).filter((item) => {
    const normalized = item.trim().toLowerCase();
    return normalized !== "" && normalized !== "text" && normalized !== "image";
  });
  if (support === "auto") return others.length > 0 ? others : undefined;
  return [
    "text",
    ...(support === "supported" ? (["image"] as const) : []),
    ...others,
  ];
}

function asSettingsObject(value: unknown): SettingsObject {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as SettingsObject)
    : {};
}

function parseSettingsJson(settingsConfig: string): SettingsObject {
  try {
    return asSettingsObject(settingsConfig ? JSON.parse(settingsConfig) : {});
  } catch {
    return {};
  }
}

function getCatalogModelEntries(settings: SettingsObject): SettingsObject[] {
  const catalog = asSettingsObject(settings.modelCatalog);
  const models = catalog.models;
  if (!Array.isArray(models)) return [];
  return models.filter(
    (item): item is SettingsObject =>
      item !== null && typeof item === "object" && !Array.isArray(item),
  );
}

function entryModelName(entry: SettingsObject): string {
  for (const field of ["model", "id", "name"]) {
    const value = entry[field];
    if (typeof value === "string" && value.trim()) return value;
  }
  return "";
}

/// 读取单个模型条目的图片支持声明，与后端 explicit_image_support 同序：
/// 先看布尔声明，再看各种模态数组写法。
function entryImageSupport(entry: SettingsObject): ModelImageSupport {
  for (const key of ["supportsImage", "supports_image", "vision"]) {
    const value = entry[key];
    if (typeof value === "boolean") return value ? "supported" : "unsupported";
  }
  const modalityArrays: unknown[] = [
    entry.input,
    asSettingsObject(entry.modalities).input,
    entry.input_modalities,
    entry.inputModalities,
  ];
  for (const value of modalityArrays) {
    if (Array.isArray(value)) {
      return getImageSupportFromModalities(
        value.filter((item): item is string => typeof item === "string"),
      );
    }
  }
  return "auto";
}

/// 汇总 settings.modelCatalog.models 中所有带显式图片声明的条目。
export function readAllImageSupport(
  settings: SettingsObject,
): ModelImageSupportEntry[] {
  const entries: ModelImageSupportEntry[] = [];
  for (const entry of getCatalogModelEntries(settings)) {
    const support = entryImageSupport(entry);
    if (support === "auto") continue;
    const name = entryModelName(entry);
    if (!name) continue;
    entries.push({ name, support });
  }
  return entries;
}

const IDENTIFIER_KEYS = new Set(["model", "id", "name"]);

/// 把某个模型的图片支持声明写回 settings 对象（原地修改并返回）。
/// auto 表示移除该模型的显式声明；条目清空后顺带清理空壳对象。
export function writeModelImageSupport(
  settings: SettingsObject,
  model: string,
  support: ModelImageSupport,
): SettingsObject {
  const trimmed = stripClaudeOneMMarker(model).trim();
  if (!trimmed) return settings;

  const catalog = asSettingsObject(settings.modelCatalog);
  const models = getCatalogModelEntries(settings).map((entry) => ({
    ...entry,
  }));
  const index = models.findIndex((entry) =>
    modelKeysMatch(entryModelName(entry), trimmed),
  );

  if (support === "auto") {
    if (index >= 0) {
      const entry = models[index];
      delete entry.inputModalities;
      const hasOtherData = Object.keys(entry).some(
        (key) => !IDENTIFIER_KEYS.has(key),
      );
      if (!hasOtherData) {
        models.splice(index, 1);
      }
    }
  } else {
    const existing =
      index >= 0 && Array.isArray(models[index].inputModalities)
        ? (models[index].inputModalities as unknown[]).filter(
            (item): item is string => typeof item === "string",
          )
        : undefined;
    const modalities = withImageSupportInModalities(existing, support);
    if (index >= 0) {
      models[index].inputModalities = modalities;
    } else {
      models.push({ model: trimmed, inputModalities: modalities });
    }
  }

  if (models.length > 0) {
    catalog.models = models;
    settings.modelCatalog = catalog;
  } else {
    delete catalog.models;
    if (Object.keys(catalog).length > 0) {
      settings.modelCatalog = catalog;
    } else {
      delete settings.modelCatalog;
    }
  }
  return settings;
}

interface UseModelImageSupportProps {
  settingsConfig: string;
  onConfigChange: (config: string) => void;
}

/**
 * 管理 Claude 供应商按模型的图片输入声明，读写 settingsConfig 的
 * modelCatalog.models。状态同步策略与 useModelState 一致。
 */
export function useModelImageSupport({
  settingsConfig,
  onConfigChange,
}: UseModelImageSupportProps) {
  const [entries, setEntries] = useState<ModelImageSupportEntry[]>(() =>
    readAllImageSupport(parseSettingsJson(settingsConfig)),
  );

  const isUserEditingRef = useRef(false);
  const lastConfigRef = useRef(settingsConfig);
  const latestConfigRef = useRef(settingsConfig);

  latestConfigRef.current = settingsConfig;

  // 仅在 settingsConfig 外部变化时同步（表单加载 / 切换预设）；
  // 用户正在编辑时跳过一次以避免回填覆盖。
  useEffect(() => {
    if (lastConfigRef.current === settingsConfig) {
      return;
    }
    if (isUserEditingRef.current) {
      isUserEditingRef.current = false;
      lastConfigRef.current = settingsConfig;
      return;
    }
    lastConfigRef.current = settingsConfig;
    setEntries(readAllImageSupport(parseSettingsJson(settingsConfig)));
  }, [settingsConfig]);

  const getModelImageSupport = useCallback(
    (model: string): ModelImageSupport => {
      const hit = entries.find((entry) => modelKeysMatch(entry.name, model));
      return hit?.support ?? "auto";
    },
    [entries],
  );

  const setModelImageSupport = useCallback(
    (model: string, support: ModelImageSupport) => {
      const trimmed = stripClaudeOneMMarker(model).trim();
      if (!trimmed) return;

      isUserEditingRef.current = true;
      setEntries((current) => {
        const next = current.filter(
          (entry) => !modelKeysMatch(entry.name, trimmed),
        );
        return support === "auto"
          ? next
          : [...next, { name: trimmed, support }];
      });

      try {
        const current = parseSettingsJson(latestConfigRef.current);
        const updated = writeModelImageSupport(current, trimmed, support);
        const updatedConfig = JSON.stringify(updated, null, 2);
        latestConfigRef.current = updatedConfig;
        onConfigChange(updatedConfig);
      } catch (err) {
        console.error("Failed to update model image support:", err);
      }
    },
    [onConfigChange],
  );

  return { getModelImageSupport, setModelImageSupport };
}
