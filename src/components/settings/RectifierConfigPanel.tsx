import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Switch } from "@/components/ui/switch";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ModelDropdown } from "@/components/providers/forms/shared";
import {
  settingsApi,
  type RectifierConfig,
  type OptimizerConfig,
} from "@/lib/api/settings";
import { useProvidersQuery } from "@/lib/query";
import type { Provider } from "@/types";
import type { FetchedModel } from "@/lib/api/model-fetch";

const OCR_PROVIDER_NONE = "__none__";
type OcrProviderAppType = "claude" | "codex";
type OcrProtocol =
  | "anthropic"
  | "openai_chat"
  | "openai_responses"
  | "gemini_native";

function normalizeOcrProtocol(value?: string | null): OcrProtocol | null {
  switch (value?.trim().toLowerCase()) {
    case "anthropic":
    case "anthropic_messages":
    case "anthropic-messages":
    case "claude":
    case "messages":
      return "anthropic";
    case "chat":
    case "chat_completions":
    case "chat-completions":
    case "openai_chat":
    case "openai-chat":
    case "openai_chat_completions":
      return "openai_chat";
    case "responses":
    case "openai_responses":
    case "openai-responses":
      return "openai_responses";
    case "gemini_native":
      return "gemini_native";
    default:
      return null;
  }
}

function configuredOcrProtocol(
  appType: OcrProviderAppType,
  provider: Provider,
): OcrProtocol | null {
  if (
    appType === "claude" &&
    ["codex_oauth", "xai_oauth"].includes(provider.meta?.providerType ?? "")
  ) {
    return "openai_responses";
  }

  const explicit = normalizeOcrProtocol(
    provider.meta?.apiFormat ??
      (provider.settingsConfig.api_format as string | undefined) ??
      (provider.settingsConfig.apiFormat as string | undefined),
  );
  if (explicit) return explicit;
  if (appType === "claude") return "anthropic";

  if (
    provider.id === "codex-official" ||
    provider.meta?.providerType === "xai_oauth"
  ) {
    return "openai_responses";
  }
  if (provider.meta?.providerType === "codex_aggregate") return null;

  const config = provider.settingsConfig.config;
  if (typeof config === "string") {
    const wireApi = config.match(
      /(?:^|\n)\s*wire_api\s*=\s*["']([^"']+)["']/,
    )?.[1];
    const protocol = normalizeOcrProtocol(wireApi);
    if (protocol) return protocol;

    const baseUrl = config.match(
      /(?:^|\n)\s*base_url\s*=\s*["']([^"']+)["']/,
    )?.[1];
    if (baseUrl?.replace(/\/+$/, "").endsWith("/chat/completions")) {
      return "openai_chat";
    }
  }
  return null;
}

function providerModelSuggestions(provider?: Provider): FetchedModel[] {
  if (!provider) return [];
  const settings = provider.settingsConfig;
  const values = new Set<string>();
  const catalog = settings.modelCatalog as
    | { models?: Array<Record<string, unknown>> }
    | Array<Record<string, unknown>>
    | undefined;
  const entries = Array.isArray(catalog) ? catalog : catalog?.models;
  for (const entry of entries ?? []) {
    const model = [entry.model, entry.id, entry.name].find(
      (value): value is string =>
        typeof value === "string" && value.trim() !== "",
    );
    if (model) values.add(model.trim());
  }

  const env = settings.env as Record<string, unknown> | undefined;
  for (const key of [
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
  ]) {
    const model = env?.[key];
    if (typeof model === "string" && model.trim()) values.add(model.trim());
  }
  if (typeof settings.model === "string" && settings.model.trim()) {
    values.add(settings.model.trim());
  }

  return [...values].map((id) => ({ id, ownedBy: provider.name }));
}

export function RectifierConfigPanel() {
  const { t } = useTranslation();
  const { data: codexProvidersData } = useProvidersQuery("codex");
  const { data: claudeProvidersData } = useProvidersQuery("claude");
  const [config, setConfig] = useState<RectifierConfig>({
    enabled: true,
    requestThinkingSignature: true,
    requestThinkingBudget: true,
    requestToolUseId: true,
    requestMediaFallback: true,
    requestMediaHeuristic: true,
    requestMediaOcrFallback: false,
    requestMediaOcrProviderAppType: "codex",
    requestMediaOcrProviderId: "",
    requestMediaOcrModel: "qwen3.8-flash",
  });
  const [optimizerConfig, setOptimizerConfig] = useState<OptimizerConfig>({
    enabled: false,
    thinkingOptimizer: true,
    cacheInjection: true,
    codexUserRoleContextNormalization: true,
  });
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    settingsApi
      .getRectifierConfig()
      .then(setConfig)
      .catch((e) => console.error("Failed to load rectifier config:", e))
      .finally(() => setIsLoading(false));
    settingsApi
      .getOptimizerConfig()
      .then(setOptimizerConfig)
      .catch((e) => console.error("Failed to load optimizer config:", e));
  }, []);

  const handleChange = async (updates: Partial<RectifierConfig>) => {
    const newConfig = { ...config, ...updates };
    setConfig(newConfig);
    try {
      await settingsApi.setRectifierConfig(newConfig);
    } catch (e) {
      console.error("Failed to save rectifier config:", e);
      toast.error(String(e));
      setConfig(config);
    }
  };

  const handleOptimizerChange = async (updates: Partial<OptimizerConfig>) => {
    const newConfig = { ...optimizerConfig, ...updates };
    setOptimizerConfig(newConfig);
    try {
      await settingsApi.setOptimizerConfig(newConfig);
    } catch (e) {
      console.error("Failed to save optimizer config:", e);
      toast.error(String(e));
      setOptimizerConfig(optimizerConfig);
    }
  };

  const ocrProviderGroups = useMemo(
    () => ({
      claude: Object.values(claudeProvidersData?.providers ?? {}),
      codex: Object.values(codexProvidersData?.providers ?? {}),
    }),
    [claudeProvidersData?.providers, codexProvidersData?.providers],
  );
  const selectedOcrProvider = ocrProviderGroups[
    config.requestMediaOcrProviderAppType
  ].find((provider) => provider.id === config.requestMediaOcrProviderId);
  const selectedOcrProtocol = selectedOcrProvider
    ? configuredOcrProtocol(
        config.requestMediaOcrProviderAppType,
        selectedOcrProvider,
      )
    : null;
  const ocrModelSuggestions = useMemo(
    () => providerModelSuggestions(selectedOcrProvider),
    [selectedOcrProvider],
  );

  if (isLoading) return null;

  return (
    <div className="space-y-6">
      <div className="border-t pt-6 mt-6 space-y-4">
        <h4 className="text-sm font-medium text-muted-foreground">
          {t("settings.advanced.rectifier.codexGroup")}
        </h4>
        <div className="flex items-center justify-between pl-4">
          <div className="space-y-0.5">
            <Label>
              {t(
                "settings.advanced.rectifier.codexUserRoleContextNormalization",
              )}
            </Label>
            <p className="text-xs text-muted-foreground">
              {t(
                "settings.advanced.rectifier.codexUserRoleContextNormalizationDescription",
              )}
            </p>
          </div>
          <Switch
            checked={optimizerConfig.codexUserRoleContextNormalization}
            onCheckedChange={(checked) =>
              handleOptimizerChange({
                codexUserRoleContextNormalization: checked,
              })
            }
          />
        </div>
      </div>

      <div className="border-t pt-6 mt-6 space-y-4">
        <div className="space-y-1">
          <h4 className="text-sm font-medium text-muted-foreground">
            {t("settings.advanced.rectifier.anthropicGroup")}
          </h4>
          <p className="text-xs text-muted-foreground">
            {t("settings.advanced.rectifier.anthropicGroupDescription")}
          </p>
        </div>

        <div className="flex items-center justify-between pl-4">
          <div className="space-y-0.5">
            <Label>{t("settings.advanced.rectifier.enabled")}</Label>
            <p className="text-xs text-muted-foreground">
              {t("settings.advanced.rectifier.enabledDescription")}
            </p>
          </div>
          <Switch
            checked={config.enabled}
            onCheckedChange={(checked) => handleChange({ enabled: checked })}
          />
        </div>

        <div className="flex items-center justify-between pl-4">
          <div className="space-y-0.5">
            <Label>{t("settings.advanced.rectifier.thinkingSignature")}</Label>
            <p className="text-xs text-muted-foreground">
              {t("settings.advanced.rectifier.thinkingSignatureDescription")}
            </p>
          </div>
          <Switch
            checked={config.requestThinkingSignature}
            disabled={!config.enabled}
            onCheckedChange={(checked) =>
              handleChange({ requestThinkingSignature: checked })
            }
          />
        </div>
        <div className="flex items-center justify-between pl-4">
          <div className="space-y-0.5">
            <Label>{t("settings.advanced.rectifier.thinkingBudget")}</Label>
            <p className="text-xs text-muted-foreground">
              {t("settings.advanced.rectifier.thinkingBudgetDescription")}
            </p>
          </div>
          <Switch
            checked={config.requestThinkingBudget}
            disabled={!config.enabled}
            onCheckedChange={(checked) =>
              handleChange({ requestThinkingBudget: checked })
            }
          />
        </div>
        <div className="flex items-center justify-between pl-4">
          <div className="space-y-0.5">
            <Label>{t("settings.advanced.rectifier.toolUseId")}</Label>
            <p className="text-xs text-muted-foreground">
              {t("settings.advanced.rectifier.toolUseIdDescription")}
            </p>
          </div>
          <Switch
            checked={config.requestToolUseId}
            disabled={!config.enabled}
            onCheckedChange={(checked) =>
              handleChange({ requestToolUseId: checked })
            }
          />
        </div>
      </div>

      <div className="border-t pt-6 mt-6 space-y-4">
        <div className="space-y-1">
          <h4 className="text-sm font-medium text-muted-foreground">
            {t("settings.advanced.rectifier.mediaGroup")}
          </h4>
          <p className="text-xs text-muted-foreground">
            {t("settings.advanced.rectifier.mediaGroupDescription")}
          </p>
        </div>

        <div className="flex items-center justify-between pl-4">
          <div className="space-y-0.5">
            <Label>{t("settings.advanced.rectifier.mediaFallback")}</Label>
            <p className="text-xs text-muted-foreground">
              {t("settings.advanced.rectifier.mediaFallbackDescription")}
            </p>
          </div>
          <Switch
            checked={config.requestMediaFallback}
            disabled={!config.enabled}
            onCheckedChange={(checked) =>
              handleChange({ requestMediaFallback: checked })
            }
          />
        </div>
        <div className="flex items-center justify-between pl-8">
          <div className="space-y-0.5">
            <Label>{t("settings.advanced.rectifier.mediaHeuristic")}</Label>
            <p className="text-xs text-muted-foreground">
              {t("settings.advanced.rectifier.mediaHeuristicDescription")}
            </p>
            <p className="text-xs text-muted-foreground break-words">
              {t("settings.advanced.rectifier.mediaHeuristicList")}
            </p>
          </div>
          <Switch
            checked={config.requestMediaHeuristic}
            disabled={!config.enabled || !config.requestMediaFallback}
            onCheckedChange={(checked) =>
              handleChange({ requestMediaHeuristic: checked })
            }
          />
        </div>
        <div className="flex items-center justify-between pl-8">
          <div className="space-y-0.5">
            <Label>{t("settings.advanced.rectifier.mediaOcrFallback")}</Label>
            <p className="text-xs text-muted-foreground">
              {t("settings.advanced.rectifier.mediaOcrFallbackDescription")}
            </p>
          </div>
          <Switch
            checked={config.requestMediaOcrFallback}
            disabled={!config.enabled || !config.requestMediaFallback}
            onCheckedChange={(checked) =>
              handleChange({ requestMediaOcrFallback: checked })
            }
          />
        </div>
        <div className="grid grid-cols-1 gap-3 pl-8 md:grid-cols-2">
          <div className="space-y-1.5">
            <Label>{t("settings.advanced.rectifier.mediaOcrProvider")}</Label>
            <Select
              value={
                config.requestMediaOcrProviderId
                  ? `${config.requestMediaOcrProviderAppType}:${config.requestMediaOcrProviderId}`
                  : OCR_PROVIDER_NONE
              }
              disabled={
                !config.enabled ||
                !config.requestMediaFallback ||
                !config.requestMediaOcrFallback
              }
              onValueChange={(value) => {
                if (value === OCR_PROVIDER_NONE) {
                  void handleChange({ requestMediaOcrProviderId: "" });
                  return;
                }
                const separator = value.indexOf(":");
                const appType = value.slice(0, separator) as OcrProviderAppType;
                const providerId = value.slice(separator + 1);
                void handleChange({
                  requestMediaOcrProviderAppType: appType,
                  requestMediaOcrProviderId: providerId,
                });
              }}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={OCR_PROVIDER_NONE}>
                  {t("settings.advanced.rectifier.mediaOcrProviderNone")}
                </SelectItem>
                {(["claude", "codex"] as const).map((appType) => (
                  <SelectGroup key={appType}>
                    <SelectLabel>
                      {t(
                        `settings.advanced.rectifier.mediaOcrProviderGroup.${appType}`,
                      )}
                    </SelectLabel>
                    {ocrProviderGroups[appType].map((provider) => {
                      const protocol = configuredOcrProtocol(appType, provider);
                      const supported =
                        protocol !== null && protocol !== "gemini_native";
                      return (
                        <SelectItem
                          key={`${appType}:${provider.id}`}
                          value={`${appType}:${provider.id}`}
                          disabled={!supported}
                        >
                          {provider.name}
                          {" · "}
                          {protocol
                            ? t(
                                `settings.advanced.rectifier.mediaOcrProtocol.${protocol}`,
                              )
                            : t(
                                "settings.advanced.rectifier.mediaOcrProtocol.unknown",
                              )}
                        </SelectItem>
                      );
                    })}
                  </SelectGroup>
                ))}
              </SelectContent>
            </Select>
            {selectedOcrProvider && (
              <p className="text-xs text-muted-foreground">
                {t("settings.advanced.rectifier.mediaOcrProtocolLabel")}
                {": "}
                {selectedOcrProtocol
                  ? t(
                      `settings.advanced.rectifier.mediaOcrProtocol.${selectedOcrProtocol}`,
                    )
                  : t("settings.advanced.rectifier.mediaOcrProtocol.unknown")}
              </p>
            )}
          </div>
          <div className="space-y-1.5">
            <Label>{t("settings.advanced.rectifier.mediaOcrModel")}</Label>
            <div className="flex gap-1">
              <Input
                key={config.requestMediaOcrModel}
                defaultValue={config.requestMediaOcrModel}
                className="flex-1"
                disabled={
                  !config.enabled ||
                  !config.requestMediaFallback ||
                  !config.requestMediaOcrFallback
                }
                onBlur={(event) => {
                  const value = event.currentTarget.value.trim();
                  if (value && value !== config.requestMediaOcrModel) {
                    void handleChange({ requestMediaOcrModel: value });
                  }
                }}
              />
              {ocrModelSuggestions.length > 0 && (
                <ModelDropdown
                  models={ocrModelSuggestions}
                  onSelect={(model) =>
                    void handleChange({ requestMediaOcrModel: model })
                  }
                />
              )}
            </div>
            <p className="text-xs text-muted-foreground">
              {t("settings.advanced.rectifier.mediaOcrModelDescription")}
            </p>
          </div>
        </div>
      </div>

      <div className="border-t pt-6 mt-6">
        <div className="space-y-1 mb-4">
          <h3 className="text-sm font-medium">
            {t("settings.advanced.optimizer.title")}
          </h3>
          <p className="text-xs text-muted-foreground">
            {t("settings.advanced.optimizer.description")}
          </p>
        </div>

        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <div className="space-y-0.5">
              <Label>{t("settings.advanced.optimizer.enabled")}</Label>
            </div>
            <Switch
              checked={optimizerConfig.enabled}
              onCheckedChange={(checked) =>
                handleOptimizerChange({ enabled: checked })
              }
            />
          </div>

          <div className="space-y-4 pl-4">
            <div className="flex items-center justify-between">
              <div className="space-y-0.5">
                <Label>
                  {t("settings.advanced.optimizer.thinkingOptimizer")}
                </Label>
                <p className="text-xs text-muted-foreground">
                  {t(
                    "settings.advanced.optimizer.thinkingOptimizerDescription",
                  )}
                </p>
              </div>
              <Switch
                checked={optimizerConfig.thinkingOptimizer}
                disabled={!optimizerConfig.enabled}
                onCheckedChange={(checked) =>
                  handleOptimizerChange({ thinkingOptimizer: checked })
                }
              />
            </div>

            <div className="flex items-center justify-between">
              <div className="space-y-0.5">
                <Label>{t("settings.advanced.optimizer.cacheInjection")}</Label>
                <p className="text-xs text-muted-foreground">
                  {t("settings.advanced.optimizer.cacheInjectionDescription")}
                </p>
              </div>
              <Switch
                checked={optimizerConfig.cacheInjection}
                disabled={!optimizerConfig.enabled}
                onCheckedChange={(checked) =>
                  handleOptimizerChange({ cacheInjection: checked })
                }
              />
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
