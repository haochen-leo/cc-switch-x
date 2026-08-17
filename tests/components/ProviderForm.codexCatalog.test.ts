import { describe, expect, it } from "vitest";
import {
  ensureCodexChatReasoningDefaults,
  normalizeCodexCatalogModelsForSave,
} from "@/components/providers/forms/ProviderForm";

describe("ProviderForm Codex catalog helpers", () => {
  it("normalizes catalog rows and removes empty or duplicate models", () => {
    expect(
      normalizeCodexCatalogModelsForSave([
        { model: " deepseek-v4-flash ", displayName: " DeepSeek " },
        { model: "deepseek-v4-flash", displayName: "Duplicate" },
        { model: "", displayName: "Empty" },
        { model: "kimi-k2", contextWindow: "128000 tokens" },
      ]),
    ).toEqual([
      { model: "deepseek-v4-flash", displayName: "DeepSeek" },
      { model: "kimi-k2", contextWindow: 128000 },
    ]);
  });

  it("preserves native-profile overrides (parallel tool calls + input modalities + base instructions)", () => {
    expect(
      normalizeCodexCatalogModelsForSave([
        {
          model: "MiniMax-M3",
          displayName: "MiniMax-M3",
          contextWindow: 1000000,
          supportsParallelToolCalls: true,
          inputModalities: ["text", "image"],
          baseInstructions:
            "  You are Codex, a coding agent based on MiniMax-M3.  ",
        },
        // false must be preserved (not dropped as falsy); empty modalities dropped;
        // empty/whitespace baseInstructions dropped
        {
          model: "mimo-v2.5-pro",
          supportsParallelToolCalls: false,
          inputModalities: [],
          baseInstructions: "   ",
        },
      ]),
    ).toEqual([
      {
        model: "MiniMax-M3",
        displayName: "MiniMax-M3",
        contextWindow: 1000000,
        supportsParallelToolCalls: true,
        inputModalities: ["text", "image"],
        baseInstructions: "You are Codex, a coding agent based on MiniMax-M3.",
      },
      { model: "mimo-v2.5-pro", supportsParallelToolCalls: false },
    ]);
  });

  it("defaults Codex Chat reasoning switches and preserves custom parameter names", () => {
    expect(ensureCodexChatReasoningDefaults()).toMatchObject({
      supportsThinking: true,
      supportsEffort: true,
      thinkingParam: "thinking",
      effortParam: "reasoning_effort",
      effortValueMode: "passthrough",
      outputFormat: "auto",
    });

    expect(
      ensureCodexChatReasoningDefaults({
        thinkingParam: "enable_thinking",
        effortParam: "reasoning.effort",
        effortValueMode: "openrouter",
        outputFormat: "reasoning_details",
      }),
    ).toMatchObject({
      supportsThinking: true,
      supportsEffort: true,
      thinkingParam: "enable_thinking",
      effortParam: "reasoning.effort",
      effortValueMode: "openrouter",
      outputFormat: "reasoning_details",
    });
  });
});
