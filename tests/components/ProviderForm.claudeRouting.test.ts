import { describe, expect, it } from "vitest";
import { normalizeClaudeModelRoutingForSave } from "@/components/providers/forms/ProviderForm";

describe("ProviderForm Claude routing helpers", () => {
  it("preserves and trims the default fallback provider during edit-save", () => {
    expect(
      normalizeClaudeModelRoutingForSave({
        defaultProviderId: " default-route ",
        haikuProviderId: " haiku-route ",
      }),
    ).toEqual({
      defaultProviderId: "default-route",
      haikuProviderId: "haiku-route",
      sonnetProviderId: undefined,
      opusProviderId: undefined,
      fableProviderId: undefined,
    });
  });

  it("keeps a routing object that only has the default fallback", () => {
    expect(
      normalizeClaudeModelRoutingForSave({
        defaultProviderId: "default-route",
      }),
    ).toMatchObject({
      defaultProviderId: "default-route",
    });
  });

  it("drops an entirely empty routing object", () => {
    expect(
      normalizeClaudeModelRoutingForSave({
        defaultProviderId: " ",
        sonnetProviderId: "",
      }),
    ).toBeUndefined();
  });
});
