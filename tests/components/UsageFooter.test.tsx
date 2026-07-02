import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi, beforeEach } from "vitest";
import type { Provider, UsageResult } from "@/types";
import UsageFooter from "@/components/UsageFooter";

const useUsageQueryMock = vi.fn();

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string; count?: number }) =>
      options?.defaultValue ?? key,
  }),
}));

vi.mock("@/lib/query/queries", () => ({
  useUsageQuery: (...args: unknown[]) => useUsageQueryMock(...args),
}));

vi.mock("@/components/SubscriptionQuotaFooter", () => ({
  TierBadge: () => <div data-testid="tier-badge" />,
}));

function createProvider(overrides: Partial<Provider> = {}): Provider {
  return {
    id: overrides.id ?? "provider-1",
    name: overrides.name ?? "Example Provider",
    category: overrides.category ?? "third_party",
    settingsConfig: overrides.settingsConfig ?? {},
    meta: overrides.meta ?? {
      usage_script: {
        enabled: true,
        language: "javascript",
        code: "",
      },
    },
    ...overrides,
  };
}

describe("UsageFooter", () => {
  beforeEach(() => {
    useUsageQueryMock.mockReset();
  });

  it("shows the money plan before the count plan in expanded usage details", () => {
    const usageResult: UsageResult = {
      success: true,
      data: [
        {
          planName: "MO计划·次数",
          total: 4000,
          used: 1155,
          remaining: 2845,
          unit: "calls",
        },
        {
          planName: "MO计划·金额",
          total: 1200,
          used: 845.24,
          remaining: 354.76,
          unit: "RMB",
        },
      ],
    };

    useUsageQueryMock.mockReturnValue({
      data: usageResult,
      isFetching: false,
      lastQueriedAt: Date.now(),
      refetch: vi.fn(),
    });

    render(
      <UsageFooter
        provider={createProvider()}
        providerId="provider-1"
        appId="claude"
        usageEnabled={true}
        isCurrent={true}
        inline={false}
      />,
    );

    const amountPlan = screen.getByText("💰 MO计划·金额");
    const countPlan = screen.getByText("💰 MO计划·次数");

    expect(
      amountPlan.compareDocumentPosition(countPlan) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });
});
