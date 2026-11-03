import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it } from "vitest";
import { CodexAggregationToggle } from "@/components/proxy/CodexAggregationToggle";
import { server } from "../msw/server";
import { getSettings, setSettings } from "../msw/state";

const CLASSIC = "settings.codexStackClassicSubagents";

function renderToggle() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <CodexAggregationToggle />
    </QueryClientProvider>,
  );
}

async function openSettings(user: ReturnType<typeof userEvent.setup>) {
  const trigger = screen.getByRole("button", { name: "选择多模型来源供应商" });
  await waitFor(() => expect(trigger).toBeEnabled());
  await user.click(trigger);
  const checkbox = await screen.findByRole("menuitemcheckbox", {
    name: CLASSIC,
  });
  await waitFor(() => expect(checkbox).not.toHaveAttribute("data-disabled"));
  return checkbox;
}

describe("Codex aggregation classic subagent tools", () => {
  beforeEach(() => {
    server.use(
      http.post("http://tauri.local/get_codex_aggregation_status", () =>
        HttpResponse.json({
          enabled: true,
          providerId: "codex-multi-provider",
          modelCount: 2,
          sourceProviderCount: 1,
          selectedProviderIds: ["third-party"],
          sourceProviders: [
            {
              providerId: "third-party",
              name: "Third party",
              official: false,
              selected: true,
              conversionRequired: true,
            },
          ],
          modelExcludes: {},
          warnings: [],
        }),
      ),
    );
  });

  it("defaults on, persists off across reopening, and can enable it again", async () => {
    const user = userEvent.setup();
    setSettings({
      codexStackClassicTools: undefined,
      unifyCodexSessionHistory: true,
    });
    const view = renderToggle();
    const checkbox = await openSettings(user);
    expect(checkbox).toBeChecked();
    await user.click(checkbox);
    await waitFor(() =>
      expect(getSettings().codexStackClassicTools).toBe(false),
    );
    expect(getSettings().unifyCodexSessionHistory).toBe(true);

    view.unmount();
    renderToggle();
    const reopened = await openSettings(user);
    await waitFor(() => expect(reopened).not.toBeChecked());
    await user.click(reopened);
    await waitFor(() =>
      expect(getSettings().codexStackClassicTools).toBe(true),
    );
  });

  it("keeps the saved value when saving fails", async () => {
    const user = userEvent.setup();
    setSettings({ codexStackClassicTools: true });
    let attempted = false;
    server.use(
      http.post("http://tauri.local/save_settings", () => {
        attempted = true;
        return HttpResponse.json(false);
      }),
    );
    renderToggle();
    const checkbox = await openSettings(user);
    await user.click(checkbox);
    await waitFor(() => expect(attempted).toBe(true));
    await waitFor(() => expect(checkbox).not.toHaveAttribute("data-disabled"));
    expect(checkbox).toBeChecked();
    expect(getSettings().codexStackClassicTools).toBe(true);
  });
});
