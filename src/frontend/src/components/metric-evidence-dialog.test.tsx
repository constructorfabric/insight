import type { ReactNode } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AnalyticsApiError } from "@/api/analytics-client";
import type { EvidenceDialogState } from "@/components/metric-evidence-context";
import { MetricEvidenceDialog } from "@/components/metric-evidence-dialog";

const mocks = vi.hoisted(() => ({
  query: {} as Record<string, unknown>,
  queryOptions: null as Record<string, unknown> | null,
  queryMetricDrilldown: vi.fn(),
  downloadMetricDrilldown: vi.fn(),
  tableProps: null as Record<string, unknown> | null,
}));

vi.mock("@tanstack/react-query", () => ({
  useInfiniteQuery: (options: Record<string, unknown>) => {
    mocks.queryOptions = options;
    return mocks.query;
  },
}));

vi.mock("@/auth/use-auth", () => ({
  useAuth: () => ({
    session: {
      tenantId: "tenant",
      personId: "person",
      impersonatorEmail: null,
      roles: ["viewer"],
    },
  }),
}));

vi.mock("@/api/metric-drilldown-client", async (importOriginal) => {
  const original =
    await importOriginal<typeof import("@/api/metric-drilldown-client")>();
  return {
    ...original,
    queryMetricDrilldown: mocks.queryMetricDrilldown,
    downloadMetricDrilldown: mocks.downloadMetricDrilldown,
  };
});

vi.mock("@/components/metric-evidence-table", () => ({
  MetricEvidenceTable: (props: Record<string, unknown>) => {
    mocks.tableProps = props;
    return <div>evidence table</div>;
  },
}));

vi.mock("@/components/ui/dialog", () => ({
  Dialog: ({
    children,
    onOpenChange,
  }: {
    children: ReactNode;
    onOpenChange: (open: boolean) => void;
  }) => (
    <div>
      {children}
      <button type="button" onClick={() => onOpenChange(false)}>
        dismiss
      </button>
    </div>
  ),
  DialogContent: ({ children }: { children: ReactNode }) => (
    <div>{children}</div>
  ),
  DialogHeader: ({ children }: { children: ReactNode }) => (
    <div>{children}</div>
  ),
  DialogTitle: ({ children }: { children: ReactNode }) => <h2>{children}</h2>,
}));

vi.mock("@/components/ui/dropdown-menu", () => ({
  DropdownMenu: ({ children }: { children: ReactNode }) => (
    <div>{children}</div>
  ),
  DropdownMenuContent: ({ children }: { children: ReactNode }) => (
    <div>{children}</div>
  ),
  DropdownMenuTrigger: ({ render }: { render: ReactNode }) => render,
  DropdownMenuItem: ({
    children,
    onClick,
  }: {
    children: ReactNode;
    onClick: () => void;
  }) => (
    <button type="button" onClick={onClick}>
      {children}
    </button>
  ),
}));

vi.mock("@/components/ui/select", () => ({
  Select: ({
    children,
    onValueChange,
  }: {
    children: ReactNode;
    onValueChange: (value: string) => void;
  }) => (
    <div>
      {children}
      <button type="button" onClick={() => onValueChange("wiki.pages")}>
        choose wiki
      </button>
      <button type="button" onClick={() => onValueChange("")}>
        choose empty
      </button>
    </div>
  ),
  SelectContent: ({ children }: { children: ReactNode }) => (
    <div>{children}</div>
  ),
  SelectItem: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  SelectTrigger: ({ children }: { children: ReactNode }) => (
    <div>{children}</div>
  ),
  SelectValue: ({ children }: { children: ReactNode }) => (
    <span>{children}</span>
  ),
}));

const selection = {
  metric_key: "git.commits",
  entity: { type: "person" as const, id: "person" },
  period: { from: "2026-07-01", to: "2026-07-31" },
  filters: [],
  display_dimensions: [],
};

const state: EvidenceDialogState = {
  targets: [{ selection, label: "Commits" }],
  activeMetricKey: "git.commits",
};

function readyQuery(overrides: Record<string, unknown> = {}) {
  return {
    data: {
      pages: [
        {
          columns: [
            { key: "value", label: "Value", type: "number" },
            { key: "ref", label: "Ref", type: "string" },
          ],
          rows: [{ values: { ref: "abc", value: 1 } }],
          next_cursor: null,
        },
      ],
    },
    isPending: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
    fetchNextPage: vi.fn(),
    hasNextPage: false,
    isFetchingNextPage: false,
    isFetchNextPageError: false,
    ...overrides,
  };
}

describe("MetricEvidenceDialog", () => {
  beforeEach(() => {
    mocks.query = readyQuery();
    mocks.queryOptions = null;
    mocks.queryMetricDrilldown.mockReset();
    mocks.downloadMetricDrilldown.mockReset().mockResolvedValue(undefined);
    mocks.tableProps = null;
  });

  it("loads, orders, paginates, exports, and closes evidence", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(
      <MetricEvidenceDialog
        state={state}
        onMetricChange={vi.fn()}
        onClose={onClose}
      />
    );

    expect(screen.getByText("evidence table")).toBeInTheDocument();
    expect(
      (mocks.tableProps?.columns as Array<{ key: string }>).map(
        (column) => column.key
      )
    ).toEqual(["ref", "value"]);

    const options = mocks.queryOptions as {
      queryFn: (context: {
        pageParam?: string;
        signal: AbortSignal;
      }) => Promise<unknown>;
      getNextPageParam: (page: {
        next_cursor: string | null;
      }) => string | undefined;
      retry: (count: number, error: unknown) => boolean;
    };
    const controller = new AbortController();
    mocks.queryMetricDrilldown.mockResolvedValue({ rows: [] });
    await options.queryFn({ pageParam: "cursor", signal: controller.signal });
    expect(mocks.queryMetricDrilldown).toHaveBeenCalledWith(
      expect.objectContaining({ cursor: "cursor", limit: 100 }),
      controller.signal
    );
    expect(options.getNextPageParam({ next_cursor: "next" })).toBe("next");
    expect(options.getNextPageParam({ next_cursor: null })).toBeUndefined();
    expect(options.retry(0, new Error("network"))).toBe(true);
    expect(options.retry(0, new AnalyticsApiError(400, {}))).toBe(false);
    expect(options.retry(1, new AnalyticsApiError(500, {}))).toBe(false);

    await user.click(screen.getByRole("button", { name: /CSV/ }));
    await waitFor(() =>
      expect(mocks.downloadMetricDrilldown).toHaveBeenCalledWith(
        selection,
        "csv",
        expect.any(AbortSignal)
      )
    );
    await user.click(screen.getByRole("button", { name: "dismiss" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("renders pending, error, empty, and page-limit states", async () => {
    const onMetricChange = vi.fn();
    mocks.query = readyQuery({ data: undefined, isPending: true });
    const view = render(
      <MetricEvidenceDialog
        state={state}
        onMetricChange={onMetricChange}
        onClose={vi.fn()}
      />
    );
    expect(screen.getByRole("status", { name: "Loading" })).toBeInTheDocument();

    const refetch = vi.fn();
    mocks.query = readyQuery({
      data: undefined,
      isError: true,
      error: new AnalyticsApiError(500, {
        detail: "Warehouse unavailable",
        trace_id: "trace-1",
      }),
      refetch,
    });
    view.rerender(
      <MetricEvidenceDialog
        state={state}
        onMetricChange={onMetricChange}
        onClose={vi.fn()}
      />
    );
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Warehouse unavailable Trace: trace-1"
    );
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(refetch).toHaveBeenCalledTimes(1);

    mocks.query = readyQuery({
      data: { pages: [{ columns: [], rows: [], next_cursor: null }] },
    });
    view.rerender(
      <MetricEvidenceDialog
        state={state}
        onMetricChange={onMetricChange}
        onClose={vi.fn()}
      />
    );
    expect(
      screen.getByText("No supporting data for this selection")
    ).toBeInTheDocument();

    mocks.query = readyQuery({
      data: {
        pages: Array.from({ length: 50 }, () => ({
          columns: [],
          rows: [{ values: {} }],
          next_cursor: "next",
        })),
      },
      hasNextPage: true,
    });
    view.rerender(
      <MetricEvidenceDialog
        state={state}
        onMetricChange={onMetricChange}
        onClose={vi.fn()}
      />
    );
    expect(mocks.tableProps).toMatchObject({
      hasNextPage: false,
      pageLimitReached: true,
    });
  });

  it("switches targets and reports export failures", async () => {
    const user = userEvent.setup();
    const onMetricChange = vi.fn();
    const multiState: EvidenceDialogState = {
      targets: [
        { selection, label: "Commits" },
        {
          selection: { ...selection, metric_key: "wiki.pages" },
          label: "Wiki pages",
        },
      ],
      activeMetricKey: "git.commits",
      title: "Combined",
    };
    mocks.downloadMetricDrilldown.mockRejectedValue(
      new AnalyticsApiError(500, { detail: "Export failed" })
    );
    render(
      <MetricEvidenceDialog
        state={multiState}
        onMetricChange={onMetricChange}
        onClose={vi.fn()}
      />
    );

    await user.click(screen.getByRole("button", { name: "choose empty" }));
    expect(onMetricChange).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "choose wiki" }));
    expect(onMetricChange).toHaveBeenCalledWith("wiki.pages");
    await user.click(screen.getByRole("button", { name: /Excel/ }));
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent("Export failed")
    );
  });
});
