import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { RequestLogTable } from "@/components/usage/RequestLogTable";
import type { UsageRangeSelection } from "@/types/usage";

const useRequestLogsMock = vi.hoisted(() => vi.fn());

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (
      key: string,
      options?: {
        defaultValue?: string;
      },
    ) => options?.defaultValue ?? key,
    i18n: {
      resolvedLanguage: "en",
      language: "en",
    },
  }),
}));

vi.mock("@/lib/query/usage", () => ({
  useRequestLogs: (args: unknown) => useRequestLogsMock(args),
}));

vi.mock("@/components/ui/button", () => ({
  Button: ({ children, ...props }: any) => (
    <button {...props}>{children}</button>
  ),
}));

vi.mock("@/components/ui/input", () => ({
  Input: (props: any) => <input {...props} />,
}));

vi.mock("@/components/ui/select", () => ({
  Select: ({ children }: any) => <div>{children}</div>,
  SelectTrigger: ({ children, ...props }: any) => (
    <button type="button" {...props}>
      {children}
    </button>
  ),
  SelectValue: ({ placeholder }: any) => <span>{placeholder ?? null}</span>,
  SelectContent: () => null,
  SelectItem: () => null,
}));

vi.mock("@/components/ui/table", () => ({
  Table: ({ children }: any) => <table>{children}</table>,
  TableBody: ({ children }: any) => <tbody>{children}</tbody>,
  TableCell: ({ children, ...props }: any) => <td {...props}>{children}</td>,
  TableHead: ({ children, ...props }: any) => <th {...props}>{children}</th>,
  TableHeader: ({ children }: any) => <thead>{children}</thead>,
  TableRow: ({ children }: any) => <tr>{children}</tr>,
}));

describe("RequestLogTable", () => {
  beforeEach(() => {
    useRequestLogsMock.mockReset();
    useRequestLogsMock.mockImplementation(
      ({ page = 0, pageSize = 20 }: { page?: number; pageSize?: number }) => ({
        data: {
          data: [],
          total: 120,
          page,
          pageSize,
        },
        isLoading: false,
      }),
    );
  });

  it("resets pagination when the dashboard range changes", async () => {
    const initialRange: UsageRangeSelection = { preset: "today" };
    const nextRange: UsageRangeSelection = {
      preset: "custom",
      customStartDate: 1_710_000_000,
      customEndDate: 1_710_086_400,
    };

    const { rerender } = render(
      <RequestLogTable
        range={initialRange}
        rangeLabel="Today"
        appType="all"
        refreshIntervalMs={0}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "2" }));

    await waitFor(() => {
      expect(useRequestLogsMock).toHaveBeenLastCalledWith(
        expect.objectContaining({
          page: 1,
          range: initialRange,
        }),
      );
    });

    rerender(
      <RequestLogTable
        range={nextRange}
        rangeLabel="Custom"
        appType="all"
        refreshIntervalMs={0}
      />,
    );

    await waitFor(() => {
      expect(useRequestLogsMock).toHaveBeenLastCalledWith(
        expect.objectContaining({
          page: 0,
          range: nextRange,
        }),
      );
    });
  });

  it("shows output token speed after TTFT", () => {
    useRequestLogsMock.mockImplementation(
      ({ page = 0, pageSize = 20 }: { page?: number; pageSize?: number }) => ({
        data: {
          data: [
            {
              requestId: "request-speed",
              providerId: "provider-1",
              providerName: "Provider 1",
              appType: "codex",
              model: "test-model",
              requestModel: "test-model",
              costMultiplier: "1",
              inputTokens: 1000,
              outputTokens: 900,
              cacheReadTokens: 0,
              cacheCreationTokens: 0,
              inputCostUsd: "0.003",
              outputCostUsd: "0.0135",
              cacheReadCostUsd: "0",
              cacheCreationCostUsd: "0",
              totalCostUsd: "0.0165",
              isStreaming: true,
              latencyMs: 4000,
              firstTokenMs: 1000,
              statusCode: 200,
              createdAt: 1_760_000_000,
              dataSource: "proxy",
            },
          ],
          total: 1,
          page,
          pageSize,
        },
        isLoading: false,
      }),
    );

    render(
      <RequestLogTable
        range={{ preset: "today" }}
        rangeLabel="Today"
        refreshIntervalMs={0}
      />,
    );

    expect(screen.getByText("4.0s")).toBeInTheDocument();
    expect(screen.getByText("/1.0s")).toBeInTheDocument();
    expect(screen.getByText(/300\.0 tok\/s/)).toBeInTheDocument();
  });

  it("hides output token speed when generation time is too short", () => {
    useRequestLogsMock.mockImplementation(
      ({ page = 0, pageSize = 20 }: { page?: number; pageSize?: number }) => ({
        data: {
          data: [
            {
              requestId: "request-unreliable-speed",
              providerId: "provider-1",
              providerName: "Provider 1",
              appType: "codex",
              model: "test-model",
              requestModel: "test-model",
              costMultiplier: "1",
              inputTokens: 1000,
              outputTokens: 900,
              cacheReadTokens: 0,
              cacheCreationTokens: 0,
              inputCostUsd: "0.003",
              outputCostUsd: "0.0135",
              cacheReadCostUsd: "0",
              cacheCreationCostUsd: "0",
              totalCostUsd: "0.0165",
              isStreaming: true,
              latencyMs: 4000,
              firstTokenMs: 3990,
              statusCode: 200,
              createdAt: 1_760_000_000,
              dataSource: "proxy",
            },
          ],
          total: 1,
          page,
          pageSize,
        },
        isLoading: false,
      }),
    );

    render(
      <RequestLogTable
        range={{ preset: "today" }}
        rangeLabel="Today"
        refreshIntervalMs={0}
      />,
    );

    expect(screen.getByText("4.0s")).toBeInTheDocument();
    expect(screen.getByText("/4.0s")).toBeInTheDocument();
    expect(screen.queryByText(/tok\/s/)).not.toBeInTheDocument();
  });

  it("resets pagination when the dashboard app filter changes", async () => {
    const range: UsageRangeSelection = { preset: "today" };
    const { rerender } = render(
      <RequestLogTable
        range={range}
        rangeLabel="Today"
        appType="all"
        refreshIntervalMs={0}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "2" }));

    await waitFor(() => {
      expect(useRequestLogsMock).toHaveBeenLastCalledWith(
        expect.objectContaining({
          page: 1,
          range,
        }),
      );
    });

    rerender(
      <RequestLogTable
        range={range}
        rangeLabel="Today"
        appType="claude"
        refreshIntervalMs={0}
      />,
    );

    await waitFor(() => {
      expect(useRequestLogsMock).toHaveBeenLastCalledWith(
        expect.objectContaining({
          page: 0,
          range,
        }),
      );
    });
  });
});
