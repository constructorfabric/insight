import type { ReactNode } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
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
  declaredDimensions: new Map<string, ReadonlySet<string>>(),
}));

vi.mock("@tanstack/react-query", () => ({
  useInfiniteQuery: (options: Record<string, unknown>) => {
    mocks.queryOptions = options;
    return mocks.query;
  },
}));

// Which dimensions a metric will accept. Most of these tests declare none, so
// the selection they assert on is the caller's own; the export test below
// declares `source` because that is when the two diverge.
vi.mock("@/queries/metric-definitions", () => ({
  useDeclaredMetricDimensions: () => ({
    byMetricKey: mocks.declaredDimensions,
    isPending: false,
  }),
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
    mocks.declaredDimensions = new Map();
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

  // The dimension that makes a row linkable is asked for, hidden from the
  // table, and must not reach the file: an export that carries a column the
  // screen does not show is not an export OF that screen.
  it("asks for the source dimension but keeps it out of the export", async () => {
    const user = userEvent.setup();
    mocks.declaredDimensions = new Map([
      ["git.commits", new Set(["repository", "source"])],
    ]);

    render(
      <MetricEvidenceDialog
        state={state}
        onMetricChange={vi.fn()}
        onClose={vi.fn()}
      />
    );

    const requested = (mocks.queryOptions?.queryKey as unknown[])[2] as {
      display_dimensions: string[];
    };
    expect(requested.display_dimensions).toContain("source");

    await user.click(screen.getByRole("button", { name: /CSV/ }));
    await waitFor(() =>
      expect(mocks.downloadMetricDrilldown).toHaveBeenCalledWith(
        selection,
        "csv",
        expect.any(AbortSignal)
      )
    );
    const [exported] = mocks.downloadMetricDrilldown.mock.calls[0]!;
    expect(
      (exported as { display_dimensions: string[] }).display_dimensions
    ).not.toContain("source");
  });

  it("asks for the issue type and exports it, since the reader can see it", async () => {
    const user = userEvent.setup();
    const taskSelection = { ...selection, metric_key: "tasks.closed" };
    mocks.declaredDimensions = new Map([
      ["tasks.closed", new Set(["source", "type"])],
    ]);

    render(
      <MetricEvidenceDialog
        state={{
          targets: [{ selection: taskSelection, label: "Issues closed" }],
          activeMetricKey: "tasks.closed",
        }}
        onMetricChange={vi.fn()}
        onClose={vi.fn()}
      />
    );

    const requested = (mocks.queryOptions?.queryKey as unknown[])[2] as {
      display_dimensions: string[];
    };
    expect(requested.display_dimensions).toEqual(["source", "type"]);

    await user.click(screen.getByRole("button", { name: /CSV/ }));
    await waitFor(() => expect(mocks.downloadMetricDrilldown).toHaveBeenCalled());
    const [exported] = mocks.downloadMetricDrilldown.mock.calls[0]!;
    const dimensions = (exported as { display_dimensions: string[] })
      .display_dimensions;
    // The type is a column on screen; the source only backs a link.
    expect(dimensions).toEqual(["type"]);
  });

  describe("what the dialog is named", () => {
    const twoTargets = [
      { selection, label: "Commits" },
      {
        selection: { ...selection, metric_key: "wiki.pages" },
        label: "Wiki pages",
      },
    ] as const;

    it("takes the caller's name for the whole set when it gives one", () => {
      render(
        <MetricEvidenceDialog
          state={{
            targets: [...twoTargets] as EvidenceDialogState["targets"],
            activeMetricKey: "git.commits",
            title: "Commits & Wiki pages",
          }}
          onMetricChange={vi.fn()}
          onClose={vi.fn()}
        />
      );
      expect(
        screen.getByRole("heading", { name: "Commits & Wiki pages" })
      ).toBeInTheDocument();
    });

    it("otherwise names the metric on screen, never a placeholder", () => {
      render(
        <MetricEvidenceDialog
          state={{
            targets: [...twoTargets] as EvidenceDialogState["targets"],
            activeMetricKey: "wiki.pages",
          }}
          onMetricChange={vi.fn()}
          onClose={vi.fn()}
        />
      );
      expect(
        screen.getByRole("heading", { name: "Wiki pages" })
      ).toBeInTheDocument();
      expect(
        screen.queryByRole("heading", { name: "Metric evidence" })
      ).not.toBeInTheDocument();
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

  describe("searching and sorting", () => {
    const threeRows = readyQuery({
      data: {
        pages: [
          {
            columns: [
              { key: "ref", label: "Ref", type: "string" },
              { key: "value", label: "Value", type: "number" },
            ],
            rows: [
              { values: { ref: "add-parser", value: 12 } },
              { values: { ref: "fix-logging", value: 3 } },
              { values: { ref: "add-cache", value: 40 } },
            ],
            next_cursor: null,
          },
        ],
      },
    });

    function renderDialog(overrides: Record<string, unknown> = {}) {
      mocks.query = { ...threeRows, ...overrides };
      return render(
        <MetricEvidenceDialog
          state={state}
          onMetricChange={vi.fn()}
          onClose={vi.fn()}
        />
      );
    }

    function tableRefs(): unknown[] {
      const rows = mocks.tableProps?.rows as Array<{
        values: Record<string, unknown>;
      }>;
      return rows.map((row) => row.values.ref);
    }

    it("counts the records it is showing", () => {
      renderDialog();
      expect(screen.getByText("3 records")).toBeInTheDocument();
    });

    it("narrows the rows to the search and says how many of how many", async () => {
      const user = userEvent.setup();
      renderDialog();

      await user.type(
        screen.getByRole("searchbox", { name: "Search records" }),
        "add"
      );
      expect(tableRefs()).toEqual(["add-parser", "add-cache"]);
      expect(screen.getByText("2 of 3 records")).toBeInTheDocument();
    });

    it("offers a way back when the search matches nothing", async () => {
      const user = userEvent.setup();
      renderDialog();
      const box = screen.getByRole("searchbox", { name: "Search records" });

      await user.type(box, "nothing here");
      expect(screen.queryByText("evidence table")).not.toBeInTheDocument();

      await user.click(screen.getByRole("button", { name: "Clear search" }));
      expect(screen.getByText("evidence table")).toBeInTheDocument();
      expect(box).toHaveValue("");
    });

    function sortBy(key: string): void {
      const onSortChange = mocks.tableProps?.onSortChange as (
        column: string
      ) => void;
      act(() => onSortChange(key));
    }

    it("does not call a search empty while pages are still coming", async () => {
      const user = userEvent.setup();
      renderDialog({ fetchNextPage: vi.fn(), hasNextPage: true });

      await user.type(
        screen.getByRole("searchbox", { name: "Search records" }),
        "nothing"
      );
      expect(
        screen.getByText("Nothing matched yet — still loading the rest")
      ).toBeInTheDocument();
      expect(screen.getByText("0 of 3 records so far")).toBeInTheDocument();
      expect(
        screen.queryByText("No records match this search")
      ).not.toBeInTheDocument();
    });

    it("says the rest could not be loaded rather than claiming no match", async () => {
      const user = userEvent.setup();
      const fetchNextPage = vi.fn();
      renderDialog({
        fetchNextPage,
        hasNextPage: true,
        isFetchNextPageError: true,
      });

      await user.type(
        screen.getByRole("searchbox", { name: "Search records" }),
        "nothing"
      );
      expect(screen.getByRole("alert")).toHaveTextContent(
        "the rest could not be loaded"
      );
      expect(screen.getByText("0 of 3 records so far")).toBeInTheDocument();

      fetchNextPage.mockClear();
      await user.click(screen.getByRole("button", { name: "Retry" }));
      expect(fetchNextPage).toHaveBeenCalled();
    });

    it("calls a search empty once every page is in", async () => {
      const user = userEvent.setup();
      renderDialog();

      await user.type(
        screen.getByRole("searchbox", { name: "Search records" }),
        "nothing"
      );
      expect(
        screen.getByText("No records match this search")
      ).toBeInTheDocument();
      expect(screen.getByText("0 of 3 records")).toBeInTheDocument();
    });

    it("cycles a column through ascending, descending and back", () => {
      renderDialog();

      sortBy("value");
      expect(mocks.tableProps?.sort).toEqual({
        key: "value",
        direction: "asc",
      });
      expect(tableRefs()).toEqual(["fix-logging", "add-parser", "add-cache"]);

      sortBy("value");
      expect(mocks.tableProps?.sort).toEqual({
        key: "value",
        direction: "desc",
      });
      expect(tableRefs()).toEqual(["add-cache", "add-parser", "fix-logging"]);

      sortBy("value");
      expect(mocks.tableProps?.sort).toBeNull();
    });

    it("pulls in the remaining pages once a search is on, so it answers for all of them", async () => {
      const user = userEvent.setup();
      const fetchNextPage = vi.fn();
      renderDialog({ fetchNextPage, hasNextPage: true });
      fetchNextPage.mockClear();

      await user.type(
        screen.getByRole("searchbox", { name: "Search records" }),
        "add"
      );
      await waitFor(() => expect(fetchNextPage).toHaveBeenCalled());
    });

    it("stops pulling pages after one fails rather than retrying forever", async () => {
      const user = userEvent.setup();
      const fetchNextPage = vi.fn();
      renderDialog({
        fetchNextPage,
        hasNextPage: true,
        isFetchNextPageError: true,
      });
      fetchNextPage.mockClear();

      await user.type(
        screen.getByRole("searchbox", { name: "Search records" }),
        "add"
      );
      expect(fetchNextPage).not.toHaveBeenCalled();
    });

    it("drops the search and sort when the dialog moves to another metric", async () => {
      const user = userEvent.setup();
      const multiState: EvidenceDialogState = {
        targets: [
          { selection, label: "Commits" },
          {
            selection: { ...selection, metric_key: "wiki.pages" },
            label: "Wiki pages",
          },
        ],
        activeMetricKey: "git.commits",
      };
      mocks.query = threeRows;
      const view = render(
        <MetricEvidenceDialog
          state={multiState}
          onMetricChange={vi.fn()}
          onClose={vi.fn()}
        />
      );

      await user.type(
        screen.getByRole("searchbox", { name: "Search records" }),
        "add"
      );
      sortBy("value");
      expect(mocks.tableProps?.sort).not.toBeNull();

      view.rerender(
        <MetricEvidenceDialog
          state={{ ...multiState, activeMetricKey: "wiki.pages" }}
          onMetricChange={vi.fn()}
          onClose={vi.fn()}
        />
      );

      expect(mocks.tableProps?.sort).toBeNull();
      expect(
        screen.getByRole("searchbox", { name: "Search records" })
      ).toHaveValue("");
      expect(tableRefs()).toHaveLength(3);
    });
  });
});
