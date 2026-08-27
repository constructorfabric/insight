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

// The people body virtualizes like the record table does; jsdom measures no
// viewport, so without this every row would be scrolled out of existence.
vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({
        index,
        start: index * 44,
      })),
    getTotalSize: () => count * 44,
    measureElement: vi.fn(),
  }),
}));

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

vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

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

type RecordsState = Extract<EvidenceDialogState, { kind: "records" }>;

const state: EvidenceDialogState = {
  kind: "records",
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

  it("does not add presentation dimensions for link resolution", async () => {
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
    expect(requested.display_dimensions).toEqual([]);

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
    ).toEqual([]);
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
          kind: "records",
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
    expect(requested.display_dimensions).toEqual(["type"]);

    await user.click(screen.getByRole("button", { name: /CSV/ }));
    await waitFor(() => expect(mocks.downloadMetricDrilldown).toHaveBeenCalled());
    const [exported] = mocks.downloadMetricDrilldown.mock.calls[0]!;
    const dimensions = (exported as { display_dimensions: string[] })
      .display_dimensions;
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
            kind: "records",
            targets: [...twoTargets] as RecordsState["targets"],
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

    it("keeps the caller's name with a single target — it says which subset", () => {
      render(
        <MetricEvidenceDialog
          state={{
            kind: "records",
            targets: [twoTargets[0]] as unknown as RecordsState["targets"],
            activeMetricKey: "git.commits",
            title: "Commits · 100–150 commits per person",
          }}
          onMetricChange={vi.fn()}
          onClose={vi.fn()}
        />
      );
      expect(
        screen.getByRole("heading", {
          name: "Commits · 100–150 commits per person",
        })
      ).toBeInTheDocument();
    });

    it("otherwise names the metric on screen, never a placeholder", () => {
      render(
        <MetricEvidenceDialog
          state={{
            kind: "records",
            targets: [...twoTargets] as RecordsState["targets"],
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


  describe("the people behind a figure", () => {
    const peopleView = {
      title: "Commits · 100–150 commits per person",
      metricKey: "git.commits",
      valueLabel: "Commits",
      rows: [
        {
          entityId: "e1",
          personId: "p1",
          name: "Ada Lovelace",
          value: 142,
          valueText: "142",
          target: {
            selection: {
              ...selection,
              entity: { type: "person" as const, id: "e1" },
            },
            label: "Commits · Ada Lovelace",
          },
        },
        {
          entityId: "e2",
          personId: null,
          name: "Grace Hopper",
          value: 131,
          valueText: "131",
          target: null,
        },
      ],
      allRecords: {
        selection: {
          ...selection,
          entity: { type: "persons" as const, ids: ["e1", "e2"] },
        },
        label: "Commits · 100–150 commits per person",
      },
    };
    const peopleState: EvidenceDialogState = {
      kind: "people",
      view: peopleView,
    };

    it("names who is behind the figure and what each of them did", () => {
      render(
        <MetricEvidenceDialog
          state={peopleState}
          onMetricChange={vi.fn()}
          onClose={vi.fn()}
        />
      );

      expect(
        screen.getByRole("heading", {
          name: "Commits · 100–150 commits per person",
        })
      ).toBeInTheDocument();
      expect(screen.getByText("2 people")).toBeInTheDocument();
      expect(screen.getByText("Ada Lovelace")).toBeInTheDocument();
      expect(screen.getByText("142")).toBeInTheDocument();
      // Same table furniture as the records it drills into: a head over the
      // values, so the number is named rather than floating.
      expect(
        screen.getByRole("columnheader", { name: "Commits" })
      ).toBeInTheDocument();
      // A list computed from values this session already had is not a server
      // read, so there is nothing for the export endpoint to produce.
      expect(
        screen.queryByRole("button", { name: /export/i })
      ).not.toBeInTheDocument();
    });

    it("asks for nothing until a row is opened", () => {
      render(
        <MetricEvidenceDialog
          state={peopleState}
          onMetricChange={vi.fn()}
          onClose={vi.fn()}
        />
      );

      expect(mocks.queryOptions?.enabled).toBe(false);
    });

    it("narrows the list by name", async () => {
      const user = userEvent.setup();
      render(
        <MetricEvidenceDialog
          state={peopleState}
          onMetricChange={vi.fn()}
          onClose={vi.fn()}
        />
      );

      await user.type(
        screen.getByRole("searchbox", { name: "Search people" }),
        "grace"
      );

      expect(screen.getByText("1 of 2 people")).toBeInTheDocument();
      expect(screen.queryByText("Ada Lovelace")).not.toBeInTheDocument();
    });

    it("takes a row into that person's records, and comes back to the list", async () => {
      const user = userEvent.setup();
      render(
        <MetricEvidenceDialog
          state={peopleState}
          onMetricChange={vi.fn()}
          onClose={vi.fn()}
        />
      );

      await user.click(screen.getByRole("button", { name: /Ada Lovelace/ }));

      expect(
        screen.getByRole("heading", { name: "Commits · Ada Lovelace" })
      ).toBeInTheDocument();
      const key = mocks.queryOptions?.queryKey as unknown[] | undefined;
      expect((key?.[2] as { entity?: unknown } | undefined)?.entity).toEqual({
        type: "person",
        id: "e1",
      });
      // The way out of a person's records, for a reader who wants the rest of
      // what is known about them.
      expect(
        screen.getByRole("link", { name: /Person page/ })
      ).toBeInTheDocument();

      await user.click(
        screen.getByRole("button", {
          name: /Commits · 100–150 commits per person/,
        })
      );

      expect(screen.getByText("Ada Lovelace")).toBeInTheDocument();
      expect(screen.getByText("2 people")).toBeInTheDocument();
    });

    it("keeps a row without evidence unopenable rather than opening nothing", () => {
      render(
        <MetricEvidenceDialog
          state={peopleState}
          onMetricChange={vi.fn()}
          onClose={vi.fn()}
        />
      );

      expect(screen.getByText("Grace Hopper")).toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: /Grace Hopper/ })
      ).not.toBeInTheDocument();
    });

    it("keeps the way back named, and takes focus with the view", async () => {
      const user = userEvent.setup();
      render(
        <MetricEvidenceDialog
          state={peopleState}
          onMetricChange={vi.fn()}
          onClose={vi.fn()}
        />
      );

      await user.click(
        screen.getByRole("button", { name: "Open records for Ada Lovelace" })
      );

      // The control that was clicked is gone: focus has to land somewhere that
      // says where the reader now is.
      expect(document.activeElement?.textContent).toBe(
        "Commits · Ada Lovelace"
      );
      expect(
        screen.getByRole("button", {
          name: "Back to Commits · 100–150 commits per person",
        })
      ).toBeInTheDocument();
    });

    it("leaves an export failure behind with the table it belongs to", async () => {
      const user = userEvent.setup();
      mocks.downloadMetricDrilldown.mockRejectedValue(
        new AnalyticsApiError(500, { detail: "Export failed" })
      );
      render(
        <MetricEvidenceDialog
          state={peopleState}
          onMetricChange={vi.fn()}
          onClose={vi.fn()}
        />
      );

      await user.click(
        screen.getByRole("button", { name: "Open records for Ada Lovelace" })
      );
      await user.click(screen.getByRole("button", { name: /CSV/ }));
      await waitFor(() =>
        expect(screen.getByRole("alert")).toBeInTheDocument()
      );

      await user.click(
        screen.getByRole("button", { name: /^Back to/ })
      );

      // A failure reported under a body that has no export control at all, and
      // then over the NEXT person's table, is a lie about both.
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });

    it("offers no person page for a table that is not one person's", async () => {
      const user = userEvent.setup();
      render(
        <MetricEvidenceDialog
          state={peopleState}
          onMetricChange={vi.fn()}
          onClose={vi.fn()}
        />
      );

      await user.click(screen.getByRole("button", { name: "All records" }));

      expect(
        screen.queryByRole("link", { name: /Person page/ })
      ).not.toBeInTheDocument();
    });

    it("closes on the way to the person's own page", async () => {
      const user = userEvent.setup();
      const onClose = vi.fn();
      render(
        <MetricEvidenceDialog
          state={peopleState}
          onMetricChange={vi.fn()}
          onClose={onClose}
        />
      );

      await user.click(
        screen.getByRole("button", { name: "Open records for Ada Lovelace" })
      );
      await user.click(screen.getByRole("link", { name: /Person page/ }));

      expect(onClose).toHaveBeenCalled();
    });

    it("says so when a search matches nobody", async () => {
      const user = userEvent.setup();
      render(
        <MetricEvidenceDialog
          state={peopleState}
          onMetricChange={vi.fn()}
          onClose={vi.fn()}
        />
      );

      await user.type(
        screen.getByRole("searchbox", { name: "Search people" }),
        "nobody"
      );
      expect(
        screen.getByText("No people match this search")
      ).toBeInTheDocument();

      await user.click(screen.getByRole("button", { name: "Clear search" }));
      expect(screen.getByText("2 people")).toBeInTheDocument();
    });

    it("carries no drill or search into the next figure", async () => {
      const user = userEvent.setup();
      const view = render(
        <MetricEvidenceDialog
          state={peopleState}
          onMetricChange={vi.fn()}
          onClose={vi.fn()}
        />
      );

      await user.type(
        screen.getByRole("searchbox", { name: "Search people" }),
        "ada"
      );
      await user.click(
        screen.getByRole("button", { name: "Open records for Ada Lovelace" })
      );

      view.rerender(
        <MetricEvidenceDialog
          state={{
            kind: "people",
            view: { ...peopleView, title: "Commits · busiest 2 of 9" },
          }}
          onMetricChange={vi.fn()}
          onClose={vi.fn()}
        />
      );

      expect(
        screen.getByRole("heading", { name: "Commits · busiest 2 of 9" })
      ).toBeInTheDocument();
      expect(screen.getByText("2 people")).toBeInTheDocument();
    });

    it("opens every row's records at once when asked for all of them", async () => {
      const user = userEvent.setup();
      render(
        <MetricEvidenceDialog
          state={peopleState}
          onMetricChange={vi.fn()}
          onClose={vi.fn()}
        />
      );

      await user.click(screen.getByRole("button", { name: "All records" }));

      const key = mocks.queryOptions?.queryKey as unknown[] | undefined;
      expect((key?.[2] as { entity?: unknown } | undefined)?.entity).toEqual({
        type: "persons",
        ids: ["e1", "e2"],
      });
    });
  });

  it("switches targets and reports export failures", async () => {
    const user = userEvent.setup();
    const onMetricChange = vi.fn();
    const multiState: EvidenceDialogState = {
      kind: "records",
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
        kind: "records",
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
