import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useCodexCommonConfig } from "@/components/providers/forms/hooks/useCodexCommonConfig";
import { useGeminiCommonConfig } from "@/components/providers/forms/hooks/useGeminiCommonConfig";

const getCommonConfigSnippetMock = vi.fn();
const setCommonConfigSnippetMock = vi.fn();
const extractCommonConfigSnippetMock = vi.fn();
const updateTomlCommonConfigSnippetMock = vi.fn();

vi.mock("@/lib/api", () => ({
  configApi: {
    getCommonConfigSnippet: (...args: unknown[]) =>
      getCommonConfigSnippetMock(...args),
    setCommonConfigSnippet: (...args: unknown[]) =>
      setCommonConfigSnippetMock(...args),
    extractCommonConfigSnippet: (...args: unknown[]) =>
      extractCommonConfigSnippetMock(...args),
    updateTomlCommonConfigSnippet: (...args: unknown[]) =>
      updateTomlCommonConfigSnippetMock(...args),
  },
}));

describe("common config snippet saving", () => {
  beforeEach(() => {
    getCommonConfigSnippetMock.mockResolvedValue("");
    setCommonConfigSnippetMock.mockResolvedValue(undefined);
    extractCommonConfigSnippetMock.mockResolvedValue("");
    updateTomlCommonConfigSnippetMock.mockImplementation(
      async (configToml: string) => configToml,
    );
  });

  it("does not persist an invalid Codex common config snippet", async () => {
    const { result } = renderHook(() =>
      useCodexCommonConfig({
        codexConfig: 'model = "gpt-5"',
      }),
    );

    await waitFor(() => expect(getCommonConfigSnippetMock).toHaveBeenCalled());

    let saved = true;
    await act(async () => {
      saved = await result.current.handleCommonConfigSnippetChange(
        "base_url = https://bad.example/v1",
      );
    });

    expect(saved).toBe(false);
    expect(setCommonConfigSnippetMock).not.toHaveBeenCalled();
    expect(result.current.commonConfigError).toContain("invalid value");
  });

  it("saves Codex common config directly without rewriting provider config", async () => {
    getCommonConfigSnippetMock.mockResolvedValue(
      "[tui]\nnotifications = true\n",
    );

    const { result } = renderHook(() =>
      useCodexCommonConfig({
        codexConfig: 'model = "gpt-5"',
      }),
    );

    await waitFor(() =>
      expect(result.current.commonConfigSnippet).toContain("[tui]"),
    );

    let saved = false;
    await act(async () => {
      saved = await result.current.handleCommonConfigSnippetChange(
        '[projects."/tmp/project"]\ntrust_level = "trusted"\n',
      );
    });

    expect(saved).toBe(true);
    expect(setCommonConfigSnippetMock).toHaveBeenCalledWith(
      "codex",
      '[projects."/tmp/project"]\ntrust_level = "trusted"\n',
    );
    expect(updateTomlCommonConfigSnippetMock).not.toHaveBeenCalled();
    expect(result.current.commonConfigSnippet).toContain("[projects.");
  });

  it("does not persist an invalid Gemini common config snippet", async () => {
    const onEnvChange = vi.fn();
    const { result } = renderHook(() =>
      useGeminiCommonConfig({
        envValue: "",
        onEnvChange,
        envStringToObj: () => ({}),
        envObjToString: () => "",
      }),
    );

    await waitFor(() => expect(result.current.isLoading).toBe(false));

    let saved = false;
    act(() => {
      saved = result.current.handleCommonConfigSnippetChange(
        JSON.stringify({ GEMINI_MODEL: 123 }),
      );
    });

    expect(saved).toBe(false);
    expect(setCommonConfigSnippetMock).not.toHaveBeenCalled();
    expect(onEnvChange).not.toHaveBeenCalled();
    expect(result.current.commonConfigError).toBe(
      "geminiConfig.commonConfigInvalidValues",
    );
  });
});
