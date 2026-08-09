import { Layers3, Loader2 } from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

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
  const mutation = useMutation({
    mutationFn: proxyApi.setCodexAggregation,
    onSuccess: async (nextStatus) => {
      queryClient.setQueryData(["codexAggregationStatus"], nextStatus);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["providers", "codex"] }),
        queryClient.invalidateQueries({ queryKey: ["proxyStatus"] }),
        queryClient.invalidateQueries({ queryKey: ["proxyTakeoverStatus"] }),
      ]);

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

  const enabled = status?.enabled ?? false;
  const tooltip = enabled
    ? `Codex Multi Provider 已启用：${status?.sourceProviderCount ?? 0} 个供应商，${status?.modelCount ?? 0} 个模型`
    : "开启后把 OpenAI Official 与第三方 Codex 模型汇集到同一个下拉列表";

  return (
    <div
      className={cn(
        "flex h-8 items-center gap-1.5 rounded-lg bg-muted/50 px-2 transition-all",
        className,
      )}
      title={tooltip}
    >
      {mutation.isPending ? (
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
        disabled={mutation.isPending}
        aria-label="Codex 多模型聚合"
      />
    </div>
  );
}
