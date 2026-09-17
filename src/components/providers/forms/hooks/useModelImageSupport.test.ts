import { describe, expect, it } from "vitest";
import {
  getImageSupportFromModalities,
  readAllImageSupport,
  withImageSupportInModalities,
  writeModelImageSupport,
} from "./useModelImageSupport";

describe("getImageSupportFromModalities", () => {
  it("returns auto when no modalities are declared", () => {
    expect(getImageSupportFromModalities(undefined)).toBe("auto");
    expect(getImageSupportFromModalities([])).toBe("auto");
  });

  it("detects image support case-insensitively", () => {
    expect(getImageSupportFromModalities(["text", "image"])).toBe("supported");
    expect(getImageSupportFromModalities(["text", "IMAGE"])).toBe("supported");
    expect(getImageSupportFromModalities(["text"])).toBe("unsupported");
  });
});

describe("withImageSupportInModalities", () => {
  it("auto clears the declaration and drops the key when nothing remains", () => {
    expect(withImageSupportInModalities(["text"], "auto")).toBeUndefined();
    expect(withImageSupportInModalities(undefined, "auto")).toBeUndefined();
  });

  it("writes text/image pair for supported and unsupported", () => {
    expect(withImageSupportInModalities(undefined, "supported")).toEqual([
      "text",
      "image",
    ]);
    expect(withImageSupportInModalities(undefined, "unsupported")).toEqual([
      "text",
    ]);
  });

  it("preserves other modalities while rewriting text/image", () => {
    expect(withImageSupportInModalities(["audio"], "supported")).toEqual([
      "text",
      "image",
      "audio",
    ]);
    expect(withImageSupportInModalities(["text", "audio"], "auto")).toEqual([
      "audio",
    ]);
  });
});

describe("readAllImageSupport", () => {
  it("collects only entries with explicit declarations", () => {
    const settings = {
      modelCatalog: {
        models: [
          { model: "glm-5.3", inputModalities: ["text"] },
          { model: "glm-5.3v" },
          { model: "qwen3-vl-plus", inputModalities: ["text", "image"] },
        ],
      },
    };
    expect(readAllImageSupport(settings)).toEqual([
      { name: "glm-5.3", support: "unsupported" },
      { name: "qwen3-vl-plus", support: "supported" },
    ]);
  });

  it("understands boolean and alternative modality shapes", () => {
    const settings = {
      modelCatalog: {
        models: [
          { model: "a", supportsImage: true },
          { model: "b", vision: false },
          { model: "c", modalities: { input: ["text", "image"] } },
          { model: "d", input_modalities: ["text"] },
        ],
      },
    };
    expect(readAllImageSupport(settings)).toEqual([
      { name: "a", support: "supported" },
      { name: "b", support: "unsupported" },
      { name: "c", support: "supported" },
      { name: "d", support: "unsupported" },
    ]);
  });

  it("returns empty for missing or malformed catalog", () => {
    expect(readAllImageSupport({})).toEqual([]);
    expect(readAllImageSupport({ modelCatalog: "nope" })).toEqual([]);
    expect(
      readAllImageSupport({ modelCatalog: { models: [null, "x", 1] } }),
    ).toEqual([]);
  });
});

describe("writeModelImageSupport", () => {
  it("creates modelCatalog structure on demand", () => {
    const settings = { env: { ANTHROPIC_AUTH_TOKEN: "k" } };
    const next = writeModelImageSupport(settings, "glm-5.3", "unsupported");
    expect(next).toEqual({
      env: { ANTHROPIC_AUTH_TOKEN: "k" },
      modelCatalog: {
        models: [{ model: "glm-5.3", inputModalities: ["text"] }],
      },
    });
  });

  it("strips the [1M] marker before persisting the model name", () => {
    const next = writeModelImageSupport({}, "GLM-5.3[1M]", "supported");
    expect(next).toEqual({
      modelCatalog: {
        models: [{ model: "GLM-5.3", inputModalities: ["text", "image"] }],
      },
    });
  });

  it("updates an existing entry in place and keeps other fields", () => {
    const settings = {
      modelCatalog: {
        models: [
          {
            model: "glm-5.3",
            displayName: "GLM",
            inputModalities: ["text"],
          },
        ],
      },
    };
    const next = writeModelImageSupport(settings, "glm-5.3", "supported");
    expect(next.modelCatalog).toEqual({
      models: [
        {
          model: "glm-5.3",
          displayName: "GLM",
          inputModalities: ["text", "image"],
        },
      ],
    });
  });

  it("matches entries case-insensitively and by namespace tail", () => {
    const settings = {
      modelCatalog: {
        models: [{ model: "GLM-5.3", inputModalities: ["text"] }],
      },
    };
    const next = writeModelImageSupport(settings, "zhipu/glm-5.3", "supported");
    // 命中既有条目而不是新增一条
    expect((next.modelCatalog as { models: unknown[] }).models).toHaveLength(1);
    expect(next.modelCatalog).toEqual({
      models: [{ model: "GLM-5.3", inputModalities: ["text", "image"] }],
    });
  });

  it("auto removes the declaration and prunes empty shells", () => {
    const settings = {
      env: {},
      modelCatalog: {
        models: [{ model: "glm-5.3", inputModalities: ["text"] }],
      },
    };
    const next = writeModelImageSupport(settings, "glm-5.3", "auto");
    expect(next).toEqual({ env: {} });
    expect("modelCatalog" in next).toBe(false);
  });

  it("auto keeps entries that carry other capability data", () => {
    const settings = {
      modelCatalog: {
        models: [
          {
            model: "glm-5.3",
            inputModalities: ["text"],
            contextWindow: 200000,
          },
        ],
      },
    };
    const next = writeModelImageSupport(settings, "glm-5.3", "auto");
    expect(next.modelCatalog).toEqual({
      models: [{ model: "glm-5.3", contextWindow: 200000 }],
    });
  });

  it("auto on an unknown model leaves settings untouched", () => {
    const settings = { env: { A: 1 } };
    const next = writeModelImageSupport(settings, "glm-5.3", "auto");
    expect(next).toEqual({ env: { A: 1 } });
    expect("modelCatalog" in next).toBe(false);
  });

  it("ignores blank model names", () => {
    const settings = { env: {} };
    expect(writeModelImageSupport(settings, "   ", "supported")).toEqual({
      env: {},
    });
  });
});
