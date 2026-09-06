import { renderHook } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { useCodexConfigState } from "@/components/providers/forms/hooks/useCodexConfigState";

// 回归：编辑已存在的供应商时，读回 modelCatalog 必须保留隐藏能力字段，
// 否则保存会把并行工具、图像模态声明剥掉。
//
// 注意：initialData 必须是稳定引用（hook 的 init effect 依赖 [initialData]）。
// 写成内联字面量会每次 re-render 产生新引用 → effect 反复 setState → 死循环 OOM。
describe("useCodexConfigState catalog load", () => {
  it("preserves native-profile hidden fields (camelCase, DB SSOT)", () => {
    const initialData = {
      settingsConfig: {
        auth: { OPENAI_API_KEY: "sk-x" },
        config: "",
        modelCatalog: {
          models: [
            {
              model: "MiniMax-M3",
              displayName: "MiniMax-M3",
              contextWindow: 1000000,
              supportsParallelToolCalls: true,
              inputModalities: ["text", "image"],
              // Legacy persisted prompt override is intentionally discarded.
              baseInstructions: "legacy provider-specific prompt",
            },
          ],
        },
      },
    };

    const { result } = renderHook(() => useCodexConfigState({ initialData }));

    expect(result.current.codexCatalogModels).toEqual([
      {
        model: "MiniMax-M3",
        displayName: "MiniMax-M3",
        contextWindow: 1000000,
        supportsParallelToolCalls: true,
        inputModalities: ["text", "image"],
      },
    ]);
  });

  it("maps snake_case hidden fields (live reverse-parse fallback) to camelCase", () => {
    const initialData = {
      settingsConfig: {
        auth: {},
        config: "",
        modelCatalog: {
          models: [
            {
              model: "mimo-v2.5-pro",
              display_name: "MiMo V2.5 Pro",
              context_window: 262144,
              supports_parallel_tool_calls: false,
              input_modalities: ["text"],
              base_instructions: "legacy provider-specific prompt",
            },
          ],
        },
      },
    };

    const { result } = renderHook(() => useCodexConfigState({ initialData }));

    expect(result.current.codexCatalogModels).toEqual([
      {
        model: "mimo-v2.5-pro",
        displayName: "MiMo V2.5 Pro",
        contextWindow: 262144,
        supportsParallelToolCalls: false,
        inputModalities: ["text"],
      },
    ]);
  });
});
