import type { ReactNode } from "react";
import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { keepPreviousData } from "@tanstack/react-query";

import { AnalyticsApiError } from "@/api/analytics-client";
import {
  TrendDrilldownDialog,
  type TrendDrilldownState,
} from "@/components/portal/trend-drilldown-dialog";

const mocks = vi.hoisted(() => ({
  query: {} as Record<string, unknown>,
  queryOptions: null as Record<string, unknown> | null,
  queryMetricDrilldown: vi.fn(),
  tableProps: null as Record<string, unknown> | null,
}));

vi.mock("@tanstack/react-query", async (importOriginal) => {
  const original =
    await importOriginal<typeof import("@tanstack/react-query")>();
  return {
    ...original,
    useInfiniteQuery: (options: Record<string, unknown>) => {
      mocks.queryOptions = options;
      return mocks.query;
    },
  };
});

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
  return { ...original, queryMetricDrilldown: mocks.queryMetricDrilldown };
});

vi.mock("@/components/metric-evidence-table", () => ({
  MetricEvidenceTable: (props: Record<string, unknown>) => {
    mocks.tableProps = props;
    return <div>evidence table</div>;
  },
}));

vi.mock("@/components/ui/dialog", () => ({
  Dialog: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  DialogContent: ({ children }: { children: ReactNode }) => (
    <div>{children}</div>
  ),
  DialogHeader: ({ children }: { children: ReactNode }) => (
    <div>{children}</div>
  ),
  DialogTitle: ({ children }: { children: ReactNode }) => <h2>{children}</h2>,
}));

vi.mock("@/components/ui/tabs", () => ({
  Tabs: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  TabsList: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  TabsTrigger: ({ children }: { children: ReactNode }) => (
    <button type="button">{children}</button>
  ),
  TabsContent: ({ children }: { children: ReactNode }) => (
    <div>{children}</div>
  ),
}));

const MEMBERS = [
  { person_id: "person-a", name: "Ada" },
  { person_id: "person-b", name: "Grace" },
];

const state: TrendDrilldownState = {
  metricKey: "git.prs_merged",
  label: "Pull requests merged",
  bucketLabel: "week",
  period: { from: "2026-07-01", to: "2026-07-31" },
  members: MEMBERS,
  breakdown: [{ date: "2026-07-06", total: 4, contributors: ["Ada"] }],
};

function readyQuery(overrides: Record<string, unknown> = {}) {
  return {
    data: {
      pages: [
        {
          selection: { sort: { key: "date", direction: "desc" } },
          columns: [
            { key: "person", label: "Who", type: "string", sortable: false },
            { key: "ref", label: "Ref", type: "string", sortable: true },
          ],
          rows: [{ values: { person: "Ada", ref: "12" } }],
          next_cursor: null,
        },
      ],
    },
    isPending: false,
    isError: false,
    fetchNextPage: vi.fn(),
    hasNextPage: false,
    isFetchingNextPage: false,
    isFetchNextPageError: false,
    ...overrides,
  };
}

function renderDialog(next: TrendDrilldownState = state) {
  return render(<TrendDrilldownDialog state={next} onClose={vi.fn()} />);
}

describe("TrendDrilldownDialog", () => {
  beforeEach(() => {
    mocks.query = readyQuery();
    mocks.queryOptions = null;
    mocks.queryMetricDrilldown.mockReset().mockResolvedValue({ rows: [] });
    mocks.tableProps = null;
  });

  async function requested(): Promise<Record<string, unknown>> {
    const options = mocks.queryOptions as {
      queryFn: (context: {
        pageParam?: string;
        signal: AbortSignal;
      }) => Promise<unknown>;
    };
    await options.queryFn({ signal: new AbortController().signal });
    return mocks.queryMetricDrilldown.mock.calls.at(-1)?.[0] as Record<
      string,
      unknown
    >;
  }

  // INVARIANT: one read for the roster. A request per member could only order
  // and narrow what each answer already held, and a team is many answers.
  it("reads the whole roster in one request", async () => {
    renderDialog();

    expect(await requested()).toMatchObject({
      metric_key: "git.prs_merged",
      entity: { type: "persons", ids: ["person-a", "person-b"] },
      period: state.period,
    });
    expect(mocks.queryMetricDrilldown).toHaveBeenCalledTimes(1);
  });

  it("announces the order the server actually served", () => {
    renderDialog();
    expect(mocks.tableProps?.sort).toEqual({ key: "date", direction: "desc" });
  });

  it("asks the server for a new order when a header is clicked", async () => {
    renderDialog();
    const onSortChange = mocks.tableProps?.onSortChange as (
      key: string
    ) => void;

    act(() => onSortChange("ref"));

    // The header keeps announcing the order the rows are in; only the request
    // moves ahead.
    expect(mocks.tableProps?.sort).toEqual({ key: "date", direction: "desc" });
    expect(await requested()).toMatchObject({
      sort: { key: "ref", direction: "asc" },
    });
  });

  it("keeps the rows on screen while a new order is fetched", () => {
    renderDialog();
    expect(mocks.queryOptions?.placeholderData).toBe(keepPreviousData);
  });

  // A scope with nobody in it has no records; telling its reader to narrow it
  // is the opposite of the truth.
  it("says an empty scope is empty, not that it is too wide", () => {
    renderDialog({ ...state, members: [] });

    expect(screen.getByText("No records in this window.")).toBeInTheDocument();
    expect(
      screen.queryByText(/more than one table can stand behind/)
    ).not.toBeInTheDocument();
  });

  it("stops paging at the cap rather than growing without end", () => {
    const pages = Array.from({ length: 50 }, () => ({
      selection: { sort: { key: "date", direction: "desc" } },
      columns: [{ key: "ref", label: "Ref", type: "string", sortable: true }],
      rows: [{ values: { ref: "12" } }],
      next_cursor: "more",
    }));
    mocks.query = readyQuery({ data: { pages }, hasNextPage: true });
    renderDialog();

    expect(mocks.tableProps?.pageLimitReached).toBe(true);
    expect(mocks.tableProps?.hasNextPage).toBe(false);
  });

  it("retries a network failure but not a refusal", () => {
    renderDialog();
    const retry = mocks.queryOptions?.retry as (
      count: number,
      error: unknown
    ) => boolean;

    expect(retry(0, new Error("network"))).toBe(true);
    expect(retry(0, new AnalyticsApiError(429, {}))).toBe(false);
    expect(retry(1, new AnalyticsApiError(500, {}))).toBe(false);
  });

  // "No records" is a claim about the data, and a read that failed made none.
  it("says nothing could be read rather than that there was nothing", () => {
    mocks.query = readyQuery({ isError: true, data: undefined });
    renderDialog();

    expect(screen.getByRole("alert")).toHaveTextContent(
      "could not be read, so nothing is claimed here"
    );
  });

  it("says the window is empty when the read came back empty", () => {
    mocks.query = readyQuery({
      data: { pages: [{ selection: {}, columns: [], rows: [], next_cursor: null }] },
    });
    renderDialog();

    expect(screen.getByText("No records in this window.")).toBeInTheDocument();
  });

  it("waits rather than claiming anything while the read is out", () => {
    mocks.query = readyQuery({ isPending: true, data: undefined });
    renderDialog();

    expect(screen.queryByText("evidence table")).not.toBeInTheDocument();
  });

  // A table that stood for only part of a scope would be a different figure
  // from the one on the chart, silently.
  it("refuses a scope wider than one read can stand behind", () => {
    renderDialog({
      ...state,
      members: Array.from({ length: 1001 }, (_, index) => ({
        person_id: `person-${index}`,
        name: `Person ${index}`,
      })),
    });

    expect(screen.getByText(/more than one table can stand behind/)).toBeInTheDocument();
  });

  // A card derived from another metric's rows has no catalog metric to
  // evidence, so the periods it is built from are all it can show.
  it("shows only the period breakdown for a derived card", () => {
    renderDialog({ ...state, metricKey: null });

    expect(screen.queryByText("evidence table")).not.toBeInTheDocument();
    expect(screen.getByText("2026-07-06")).toBeInTheDocument();
  });

  it("says so when the chart has no readings to break down", () => {
    renderDialog({ ...state, metricKey: null, breakdown: [] });

    expect(screen.getByText("No readings in this window.")).toBeInTheDocument();
  });
});
