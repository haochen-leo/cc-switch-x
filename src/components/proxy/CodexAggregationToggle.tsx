import { useRef } from "react";
import type { SyntheticEvent } from "react";
import { ChevronRight, Layers3, Loader2, Settings2 } from "lucide-react";
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
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Switch } from "@/components/ui/switch";
import { proxyApi } from "@/lib/api/proxy";
import { cn } from "@/lib/utils";
import { extractErrorMessage } from "@/utils/errorUtils";
import type {
  CodexAggregationSourceModels,
  CodexAggregationSourceProvider,
} from "@/types/proxy";

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
              <SourceProviderRow
                key={source.providerId}
                source={source}
                excludedCount={
                  status?.modelExcludes?.[source.providerId]?.length ?? 0
                }
                toggleDisabled={
                  busy || (source.selected && selectedProviderIds.length <= 1)
                }
                onToggle={() =>
                  updateSource(source.providerId, !source.selected)
                }
                onAggregationChanged={invalidateAggregationViews}
              />
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </>
  );
}

/**
 * 来源供应商行：外观与普通勾选行一致（左侧 ✓ 指示符，无蓝框勾选框）。
 * 整行悬浮预览模型子菜单；点击左侧 = 选择/取消来源，点击右侧箭头 = 展开。
 */
function SourceProviderRow({
  source,
  excludedCount,
  toggleDisabled,
  onToggle,
  onAggregationChanged,
}: {
  source: CodexAggregationSourceProvider;
  excludedCount: number;
  toggleDisabled: boolean;
  onToggle: () => void;
  onAggregationChanged: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const arrowZoneRef = useRef<HTMLSpanElement>(null);
  const isOnArrowZone = (event: SyntheticEvent) =>
    event.target instanceof Node &&
    arrowZoneRef.current?.contains(event.target) === true;

  return (
    <DropdownMenuSub>
      <DropdownMenuSubTrigger
        className="relative pl-8 pr-2 focus-visible:outline-none"
        onClick={(event) => {
          if (isOnArrowZone(event)) return; // 箭头区点击：默认行为展开子菜单
          event.preventDefault();
          if (!toggleDisabled) onToggle();
        }}
        onKeyDown={(event) => {
          // 空格 = 选择供应商；Enter/→ 保持默认（展开子菜单）
          if (event.key !== " ") return;
          event.preventDefault();
          if (!toggleDisabled) onToggle();
        }}
      >
        {/* 与 DropdownMenuCheckboxItem 相同的 ✓ 指示符（选中时显示） */}
        <span
          className={cn(
            "absolute left-2 flex h-3.5 w-3.5 items-center justify-center",
            toggleDisabled && "opacity-40",
          )}
        >
          {source.selected && (
            <svg
              className="h-4 w-4"
              viewBox="0 0 20 20"
              fill="none"
              xmlns="http://www.w3.org/2000/svg"
            >
              <path
                d="M16.704 5.292a1 1 0 0 1 .083 1.32l-.083.094-8 8a1 1 0 0 1-1.32.083l-.094-.083-4-4a1 1 0 0 1 1.32-1.497l.094.083L8 12.585l7.293-7.292a1 1 0 0 1 1.32-1.497l.094.083Z"
                fill="currentColor"
              />
            </svg>
          )}
        </span>
        <span
          className={cn(
            "flex min-w-0 flex-1 items-center gap-2",
            toggleDisabled && "cursor-not-allowed opacity-50",
          )}
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
          {excludedCount > 0 && (
            <ProviderStatusBadge
              tone="warning"
              label={t("proxy.codexAggregation.modelFilterExcludedCount", {
                count: excludedCount,
                defaultValue: "已排除 {{count}}",
              })}
            />
          )}
        </span>
        <span
          ref={arrowZoneRef}
          className="flex h-5 w-5 shrink-0 items-center justify-center"
        >
          <ChevronRight className="h-3.5 w-3.5 text-muted-foreground" />
        </span>
      </DropdownMenuSubTrigger>
      <DropdownMenuSubContent className="z-[200] max-h-64 min-w-56 overflow-y-auto">
        <DropdownMenuLabel className="truncate text-xs font-medium">
          {t("proxy.codexAggregation.modelFilterTitle", {
            name: source.name,
            defaultValue: "{{name}} 的模型",
          })}
        </DropdownMenuLabel>
        <SourceModelFilterItems
          providerId={source.providerId}
          onAggregationChanged={onAggregationChanged}
        />
      </DropdownMenuSubContent>
    </DropdownMenuSub>
  );
}

/**
 * 单家来源供应商的模型过滤列表（排除语义：勾选 = 纳入聚合目录）。
 * 子菜单展开时才挂载并发请求，避免一次性探测全部供应商。
 */
function SourceModelFilterItems({
  providerId,
  onAggregationChanged,
}: {
  providerId: string;
  onAggregationChanged: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const { data, isLoading, error } = useQuery({
    queryKey: ["codexAggregationSourceModels", providerId],
    queryFn: () => proxyApi.getCodexAggregationSourceModels(providerId),
    staleTime: 30_000,
  });
  const mutation = useMutation({
    mutationFn: (excludedModels: string[]) =>
      proxyApi.setCodexAggregationModelExcludes(providerId, excludedModels),
    onSuccess: async (nextStatus, excludedModels) => {
      queryClient.setQueryData(["codexAggregationStatus"], nextStatus);
      queryClient.setQueryData<CodexAggregationSourceModels | undefined>(
        ["codexAggregationSourceModels", providerId],
        (prev) => (prev ? { ...prev, excludedModels } : prev),
      );
      await onAggregationChanged();
      if (nextStatus.warnings.length > 0) {
        toast.warning(
          nextStatus.warnings.join(
            t("proxy.codexAggregation.warningSeparator", {
              defaultValue: "；",
            }),
          ),
          { closeButton: true, duration: 7000 },
        );
      }
    },
    onError: (error: Error) => {
      toast.error(
        t("proxy.codexAggregation.modelExcludesUpdateFailed", {
          error: extractErrorMessage(error),
          defaultValue: "更新模型过滤失败：{{error}}",
        }),
        { closeButton: true, duration: 7000 },
      );
    },
  });

  const models = data?.models ?? [];
  const excluded = new Set(data?.excludedModels ?? []);

  if (isLoading) {
    return (
      <DropdownMenuLabel className="flex items-center gap-2 pl-8 text-xs font-normal text-muted-foreground">
        <Loader2 className="h-3.5 w-3.5 animate-spin" />
        {t("proxy.codexAggregation.modelFilterLoading", {
          defaultValue: "模型加载中…",
        })}
      </DropdownMenuLabel>
    );
  }
  if (error) {
    return (
      <DropdownMenuLabel className="pl-8 text-xs font-normal text-destructive">
        {t("proxy.codexAggregation.modelFilterFailed", {
          error: extractErrorMessage(error),
          defaultValue: "模型列表加载失败：{{error}}",
        })}
      </DropdownMenuLabel>
    );
  }

  return (
    <>
      {data?.warning && (
        <DropdownMenuLabel className="pl-8 text-xs font-normal text-muted-foreground">
          {data.warning}
        </DropdownMenuLabel>
      )}
      {models.length === 0 && !data?.warning && (
        <DropdownMenuLabel className="pl-8 text-xs font-normal text-muted-foreground">
          {t("proxy.codexAggregation.modelFilterEmpty", {
            defaultValue: "该供应商暂无可过滤的模型",
          })}
        </DropdownMenuLabel>
      )}
      {models.map((model) => (
        <DropdownMenuCheckboxItem
          key={model.model}
          checked={!excluded.has(model.model)}
          disabled={mutation.isPending}
          onCheckedChange={(checked) => {
            const next = new Set(excluded);
            if (checked === true) {
              next.delete(model.model);
            } else {
              next.add(model.model);
            }
            mutation.mutate([...next]);
          }}
          onSelect={(event) => event.preventDefault()}
          className="pl-8 pr-2 focus-visible:outline-none"
        >
          <span className="min-w-0 flex-1 truncate" title={model.model}>
            {model.displayName}
          </span>
        </DropdownMenuCheckboxItem>
      ))}
    </>
  );
}
