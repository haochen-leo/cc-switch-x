import { Layers3, Loader2, Settings2 } from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { ProviderStatusBadge } from "@/components/providers/ProviderStatusBadge";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Switch } from "@/components/ui/switch";
import { proxyApi } from "@/lib/api/proxy";
import { cn } from "@/lib/utils";
import { extractErrorMessage } from "@/utils/errorUtils";

export const CODEX_AGGREGATE_PROVIDER_ID = "codex-multi-provider";

interface CodexAggregationToggleProps {
  className?: string;
}

export function CodexAggregationToggle({
  className,
}: CodexAggregationToggleProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const { data: status } = useQuery({
    queryKey: ["codexAggregationStatus"],
    queryFn: proxyApi.getCodexAggregationStatus,
  });
  const invalidateAggregationViews = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["providers", "codex"] }),
      queryClient.invalidateQueries({ queryKey: ["proxyStatus"] }),
      queryClient.invalidateQueries({ queryKey: ["proxyTakeoverStatus"] }),
    ]);
  };
  const mutation = useMutation({
    mutationFn: proxyApi.setCodexAggregation,
    onSuccess: async (nextStatus) => {
      queryClient.setQueryData(["codexAggregationStatus"], nextStatus);
      await invalidateAggregationViews();

      if (nextStatus.enabled) {
        toast.success(
          t("proxy.codexAggregation.enabledToast", {
            providerCount: nextStatus.sourceProviderCount,
            modelCount: nextStatus.modelCount,
            defaultValue:
              "已聚合 {{providerCount}} 个供应商、{{modelCount}} 个模型；完全退出并重启 Codex 后可在下拉列表选择。",
          }),
          { closeButton: true, duration: 7000 },
        );
        if (nextStatus.warnings.length > 0) {
          toast.warning(
            nextStatus.warnings.join(
              t("proxy.codexAggregation.warningSeparator", {
                defaultValue: "；",
              }),
            ),
            {
              closeButton: true,
              duration: 7000,
            },
          );
        }
      } else {
        toast.success(
          t("proxy.codexAggregation.disabledToast", {
            defaultValue: "已关闭 Codex 多模型聚合，并恢复开启前的供应商。",
          }),
          {
            closeButton: true,
          },
        );
      }
    },
    onError: (error: Error) => {
      toast.error(
        t("proxy.codexAggregation.toggleFailed", {
          error: extractErrorMessage(error),
          defaultValue: "Codex 多模型聚合失败：{{error}}",
        }),
        {
          closeButton: true,
          duration: 7000,
        },
      );
    },
  });
  const sourcesMutation = useMutation({
    mutationFn: proxyApi.setCodexAggregationSources,
    onSuccess: async (nextStatus) => {
      queryClient.setQueryData(["codexAggregationStatus"], nextStatus);
      await invalidateAggregationViews();
      toast.success(
        nextStatus.enabled
          ? t("proxy.codexAggregation.sourcesUpdatedToast", {
              providerCount: nextStatus.sourceProviderCount,
              modelCount: nextStatus.modelCount,
              defaultValue:
                "已更新多模型来源：{{providerCount}} 个供应商、{{modelCount}} 个模型。",
            })
          : t("proxy.codexAggregation.sourcesSavedToast", {
              providerCount: nextStatus.selectedProviderIds.length,
              defaultValue: "已保存 {{providerCount}} 个多模型来源供应商。",
            }),
        { closeButton: true },
      );
      if (nextStatus.warnings.length > 0) {
        toast.warning(
          nextStatus.warnings.join(
            t("proxy.codexAggregation.warningSeparator", {
              defaultValue: "；",
            }),
          ),
          {
            closeButton: true,
            duration: 7000,
          },
        );
      }
    },
    onError: (error: Error) => {
      toast.error(
        t("proxy.codexAggregation.sourcesUpdateFailed", {
          error: extractErrorMessage(error),
          defaultValue: "更新多模型来源失败：{{error}}",
        }),
        {
          closeButton: true,
          duration: 7000,
        },
      );
    },
  });
  const enabled = status?.enabled ?? false;
  const selectedProviderIds = status?.selectedProviderIds ?? [];
  const sourceProviders = status?.sourceProviders ?? [];
  const busy = mutation.isPending || sourcesMutation.isPending;
  const tooltip = enabled
    ? t("proxy.codexAggregation.tooltipEnabled", {
        providerCount: status?.sourceProviderCount ?? 0,
        modelCount: status?.modelCount ?? 0,
        defaultValue:
          "Codex Multi Provider 已启用：{{providerCount}} 个供应商，{{modelCount}} 个模型",
      })
    : t("proxy.codexAggregation.tooltipInactive", {
        defaultValue:
          "开启后把 OpenAI Official 与第三方 Codex 模型汇集到同一个下拉列表",
      });
  const updateSource = (providerId: string, checked: boolean) => {
    const selected = new Set(selectedProviderIds);
    if (checked) {
      selected.add(providerId);
    } else {
      selected.delete(providerId);
    }
    if (selected.size === 0) {
      toast.error(
        t("proxy.codexAggregation.atLeastOneSource", {
          defaultValue: "Codex 多模型至少选择一个供应商",
        }),
      );
      return;
    }
    sourcesMutation.mutate(
      sourceProviders
        .filter((source) => selected.has(source.providerId))
        .map((source) => source.providerId),
    );
  };
  return (
    <>
      <div
        className={cn(
          "flex h-8 items-center gap-1.5 rounded-lg bg-muted/50 px-2 transition-all",
          className,
        )}
        title={tooltip}
      >
        {busy ? (
          <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
        ) : (
          <Layers3
            className={cn(
              "h-4 w-4 transition-colors",
              enabled ? "text-emerald-500" : "text-muted-foreground",
            )}
          />
        )}
        <span className="text-xs font-medium">
          {t("proxy.codexAggregation.shortTitle", {
            defaultValue: "多模型",
          })}
        </span>
        <Switch
          checked={enabled}
          onCheckedChange={(checked) => mutation.mutate(checked)}
          disabled={busy}
          aria-label={t("proxy.codexAggregation.switchAria", {
            defaultValue: "Codex 多模型聚合",
          })}
        />
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-6 w-6"
              disabled={busy || sourceProviders.length === 0}
              title={t("proxy.codexAggregation.sourceSettingsTitle", {
                defaultValue: "选择多模型来源供应商",
              })}
              aria-label={t("proxy.codexAggregation.sourceSettingsTitle", {
                defaultValue: "选择多模型来源供应商",
              })}
            >
              {sourcesMutation.isPending ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Settings2 className="h-3.5 w-3.5" />
              )}
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="end"
            className="z-[200] max-h-72 min-w-72 overflow-y-auto"
          >
            <DropdownMenuLabel className="pl-8 text-xs font-medium">
              {t("proxy.codexAggregation.sourceProvidersLabel", {
                defaultValue: "模型来源供应商",
              })}
            </DropdownMenuLabel>
            {sourceProviders.map((source) => (
              <DropdownMenuCheckboxItem
                key={source.providerId}
                checked={source.selected}
                disabled={
                  busy ||
                  (source.selected && selectedProviderIds.length <= 1)
                }
                onCheckedChange={(checked) =>
                  updateSource(source.providerId, checked === true)
                }
                onSelect={(event) => event.preventDefault()}
                className="pl-8 pr-2"
              >
                <span className="min-w-0 flex-1 truncate">{source.name}</span>
                {source.conversionRequired && (
                  <ProviderStatusBadge
                    tone="info"
                    label={t("proxy.codexAggregation.conversionRequired", {
                      defaultValue: "需要路由",
                    })}
                  />
                )}
                {source.official && (
                  <ProviderStatusBadge
                    tone="muted"
                    label={t("proxy.codexAggregation.official", {
                      defaultValue: "官方",
                    })}
                  />
                )}
              </DropdownMenuCheckboxItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </>
  );
}
