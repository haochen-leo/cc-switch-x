import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { parse as parseToml } from "smol-toml";
import { configApi } from "@/lib/api";
import { normalizeTomlText } from "@/utils/textNormalization";

const LEGACY_STORAGE_KEY = "cc-switch:codex-common-config-snippet";

interface UseCodexCommonConfigProps {
  codexConfig: string;
}

/**
 * 管理 codex-official 中唯一的 Codex 通用配置。
 *
 * 供应商编辑器只编辑认证和路由；这里不再把通用配置临时合入供应商文本，
 * 避免界面形成一份看似可独立维护、保存时又会被后端剥离的副本。
 */
export function useCodexCommonConfig({
  codexConfig,
}: UseCodexCommonConfigProps) {
  const { t } = useTranslation();
  const [commonConfigSnippet, setCommonConfigSnippet] = useState("");
  const [commonConfigError, setCommonConfigError] = useState("");
  const [isExtracting, setIsExtracting] = useState(false);

  useEffect(() => {
    let mounted = true;

    const loadSnippet = async () => {
      try {
        let snippet = await configApi.getCommonConfigSnippet("codex");

        if (!snippet?.trim() && typeof window !== "undefined") {
          const legacySnippet = window.localStorage.getItem(LEGACY_STORAGE_KEY);
          if (legacySnippet?.trim()) {
            await configApi.setCommonConfigSnippet("codex", legacySnippet);
            window.localStorage.removeItem(LEGACY_STORAGE_KEY);
            snippet = legacySnippet;
          }
        }

        if (mounted) {
          setCommonConfigSnippet(snippet ?? "");
        }
      } catch (error) {
        if (mounted) {
          setCommonConfigError(String(error));
        }
      }
    };

    void loadSnippet();
    return () => {
      mounted = false;
    };
  }, []);

  const handleCommonConfigSnippetChange = useCallback(
    async (value: string): Promise<boolean> => {
      if (value.trim()) {
        try {
          parseToml(normalizeTomlText(value));
        } catch (error) {
          setCommonConfigError(
            error instanceof Error ? error.message : String(error),
          );
          return false;
        }
      }

      try {
        await configApi.setCommonConfigSnippet("codex", value);
        setCommonConfigSnippet(value);
        setCommonConfigError("");
        return true;
      } catch (error) {
        setCommonConfigError(
          t("codexConfig.saveFailed", { error: String(error) }),
        );
        return false;
      }
    },
    [t],
  );

  const handleExtract = useCallback(async () => {
    setIsExtracting(true);
    setCommonConfigError("");

    try {
      const extracted = await configApi.extractCommonConfigSnippet("codex", {
        settingsConfig: JSON.stringify({ config: codexConfig ?? "" }),
      });

      if (!extracted?.trim()) {
        setCommonConfigError(t("codexConfig.extractNoCommonConfig"));
        return;
      }

      await configApi.setCommonConfigSnippet("codex", extracted);
      setCommonConfigSnippet(extracted);
    } catch (error) {
      setCommonConfigError(
        t("codexConfig.extractFailed", { error: String(error) }),
      );
    } finally {
      setIsExtracting(false);
    }
  }, [codexConfig, t]);

  const clearCommonConfigError = useCallback(() => {
    setCommonConfigError("");
  }, []);

  return {
    commonConfigSnippet,
    commonConfigError,
    isExtracting,
    handleCommonConfigSnippetChange,
    handleExtract,
    clearCommonConfigError,
  };
}
