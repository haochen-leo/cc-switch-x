import { Layers3, Loader2, Settings2 } from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuSeparator,
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
          `已聚合 ${nextStatus.sourceProviderCount} 个供应商、${nextStatus.modelCount} 个模型；完全退出并重启 Codex 后可在下拉列表选择。`,
          { closeButton: true, duration: 7000 },
        );
        if (nextStatus.warnings.length > 0) {
          toast.warning(nextStatus.warnings.join("；"), {
            closeButton: true,
            duration: 7000,
          });
        }
      } else {
        toast.success("已关闭 Codex 多模型聚合，并恢复开启前的供应商。", {
          closeButton: true,
        });
      }
    },
    onError: (error: Error) => {
      toast.error(`Codex 多模型聚合失败：${extractErrorMessage(error)}`, {
        closeButton: true,
        duration: 7000,
      });
    },
  });
  const sourcesMutation = useMutation({
    mutationFn: proxyApi.setCodexAggregationSources,
    onSuccess: async (nextStatus) => {
      queryClient.setQueryData(["codexAggregationStatus"], nextStatus);
      await invalidateAggregationViews();
      toast.success(
        nextStatus.enabled
          ? `已更新多模型来源：${nextStatus.sourceProviderCount} 个供应商、${nextStatus.modelCount} 个模型。`
          : `已保存 ${nextStatus.selectedProviderIds.length} 个多模型来源供应商。`,
        { closeButton: true },
      );
      if (nextStatus.warnings.length > 0) {
        toast.warning(nextStatus.warnings.join("；"), {
          closeButton: true,
          duration: 7000,
        });
      }
    },
    onError: (error: Error) => {
      toast.error(`更新多模型来源失败：${extractErrorMessage(error)}`, {
        closeButton: true,
        duration: 7000,
      });
    },
  });

  const enabled = status?.enabled ?? false;
  const selectedProviderIds = status?.selectedProviderIds ?? [];
  const sourceProviders = status?.sourceProviders ?? [];
  const busy = mutation.isPending || sourcesMutation.isPending;
  const tooltip = enabled
    ? `Codex Multi Provider 已启用：${status?.sourceProviderCount ?? 0} 个供应商，${status?.modelCount ?? 0} 个模型`
    : "开启后把 OpenAI Official 与第三方 Codex 模型汇集到同一个下拉列表";
  const updateSource = (providerId: string, checked: boolean) => {
    const selected = new Set(selectedProviderIds);
    if (checked) {
      selected.add(providerId);
    } else {
      selected.delete(providerId);
    }
    if (selected.size === 0) {
      toast.error("Codex 多模型至少选择一个供应商");
      return;
    }
    sourcesMutation.mutate(
      sourceProviders
        .filter((source) => selected.has(source.providerId))
        .map((source) => source.providerId),
    );
  };

  return (
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
      <span className="text-xs font-medium">多模型</span>
      <Switch
        checked={enabled}
        onCheckedChange={(checked) => mutation.mutate(checked)}
        disabled={busy}
        aria-label="Codex 多模型聚合"
      />
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="h-6 w-6"
            disabled={busy || sourceProviders.length === 0}
            title="选择多模型来源供应商"
            aria-label="选择多模型来源供应商"
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
          className="z-[200] max-h-72 min-w-64 overflow-y-auto"
        >
          <DropdownMenuLabel>模型来源供应商</DropdownMenuLabel>
          <DropdownMenuSeparator />
          {sourceProviders.map((source) => (
            <DropdownMenuCheckboxItem
              key={source.providerId}
              checked={source.selected}
              disabled={
                busy || (source.selected && selectedProviderIds.length <= 1)
              }
              onCheckedChange={(checked) =>
                updateSource(source.providerId, checked === true)
              }
              onSelect={(event) => event.preventDefault()}
              className="pl-8"
            >
              <span className="min-w-0 flex-1 truncate">{source.name}</span>
              {source.official && (
                <span className="text-xs text-muted-foreground">官方</span>
              )}
            </DropdownMenuCheckboxItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}
