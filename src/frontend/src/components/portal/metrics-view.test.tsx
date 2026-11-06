// @vitest-environment jsdom
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { portalRouter } from "@/test/portal-router";

import { act, render, renderHook, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { MetricDefinition } from "@/api/metric-definitions-client";
import type { PeopleListItem } from "@/api/identity-client";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import {
  setMetricsViewPaneShown,
  useMetricsViewPaneShown,
} from "@/lib/portal/metrics-view-pane";
import { identityPerson, peopleFromIdentityTree, pid } from "@/test/identity";

const mocks = vi.hoisted(() => ({
  personId: null as string | null,
  roster: [] as PeopleListItem[],
  definitions: [] as MetricDefinition[],
  requestedKeys: [] as readonly string[],
  data: {
    byKey: new Map<string, NormalizedMetricResult>(),
    isPending: false,
    isError: false,
    refetch: vi.fn(),
  },
}));

vi.mock("@/auth", () => ({
  useViewer: () => ({ email: "boss@x", personId: mocks.personId }),
}));
vi.mock("@/queries/identity-me", () => ({
  useVisibilityPolicy: () => ({ policy: "org_chart", isFlat: false, isPending: false }),
}));
vi.mock("@/queries/visible-roster", () => ({
  useVisibleRoster: () => ({
    roster: mocks.roster,
    isPending: false,
    isError: false,
    retry: () => {},
  }),
}));
vi.mock("@/queries/metric-definitions", () => ({
  useMetricDefinitionsResponse: () => ({
    data: { metrics: mocks.definitions },
    isPending: false,
    isError: false,
  }),
}));
vi.mock("@/queries/metrics-view", () => ({
  useMetricsViewData: (keys: readonly string[]) => {
    mocks.requestedKeys = keys;
    return mocks.data;
  },
}));
vi.mock("@/hooks/use-portal-period", () => ({
  usePortalPeriod: () => ({
    period: "month",
    dateRange: { from: "2026-07-01", to: "2026-07-31" },
  }),
}));

import { MetricsView } from "./metrics-view";

function definition(
  key: string,
  label: string,
  over: Partial<MetricDefinition> = {},
): MetricDefinition {
  return {
    metric_key: key,
    entity_type: "person",
    label,
    short_label: null,
    description: null,
    explanation: null,
    unit: null,
    format: "integer",
    direction: "higher_is_better",
    dimensions: [],
    is_enabled: true,
    origin: "builtin",
    schema_status: "ok",
    schema_error_code: null,
    first_observed_date: "2026-01-01",
    last_observed_date: "2026-07-31",
    ...over,
  } as MetricDefinition;
}

function result(
  key: string,
  label: string,
  values: Array<[string, number | null]>,
): NormalizedMetricResult {
  return {
    metric_key: key,
    label,
    unit: null,
    computation: "sum",
    format: "integer",
    direction: "higher_is_better",
    period: {
      view: "period",
      values: values.map(([entity_id, value]) => ({ entity_id, value })),
    },
  } as NormalizedMetricResult;
}

const rowHeaders = () =>
  within(screen.getByRole("table"))
    .getAllByRole("rowheader")
    .map((header) => header.textContent);

beforeEach(() => {
  const tree = identityPerson("boss", {}, [
    identityPerson("a", {}, [identityPerson("c")]),
    identityPerson("b"),
  ]);
  mocks.personId = pid("boss");
  mocks.roster = peopleFromIdentityTree(tree);
  mocks.definitions = [
    definition("collab.messages", "Messages"),
    definition("git.commits", "Commits"),
    definition("tasks.closed", "Tasks closed"),
    definition("ci.runs", "CI runs", { entity_type: "tenant" }),
  ];
  mocks.data.isPending = false;
  mocks.data.isError = false;
  mocks.data.byKey = new Map([
    [
      "git.commits",
      result("git.commits", "Commits", [
        [pid("boss"), 50],
        [pid("a"), 10],
        [pid("b"), 20],
        [pid("c"), 30],
      ]),
    ],
    [
      "collab.messages",
      result("collab.messages", "Messages", [
        [pid("a"), 4],
        [pid("b"), 6],
      ]),
    ],
    [
      "tasks.closed",
      result("tasks.closed", "Tasks closed", [
        [pid("a"), null],
        [pid("b"), null],
      ]),
    ],
  ]);
  act(() => {
    portalRouter.set({ scope: undefined, direct: false });
    setMetricsViewPaneShown(false);
  });
});

describe("MetricsView", () => {
  it("lists everyone in scope against every metric that has data", () => {
    render(<MetricsView />);

    expect([...rowHeaders()].sort()).toEqual(["a", "b", "boss", "c"]);
    expect(screen.getByRole("columnheader", { name: /Commits/ })).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: /Messages/ })).toBeInTheDocument();
    expect(screen.queryByRole("columnheader", { name: /Tasks closed/ })).not.toBeInTheDocument();
  });

  it("asks for every person-level metric, in family order", () => {
    render(<MetricsView />);

    expect(mocks.requestedKeys).toEqual(["git.commits", "tasks.closed", "collab.messages"]);
  });

  it("drops a metric the reader unticks", async () => {
    const user = userEvent.setup();
    render(<MetricsView />);

    await user.click(screen.getByRole("button", { name: "Metrics: 3 of 3" }));
    await user.click(screen.getByRole("checkbox", { name: "Messages" }));

    expect(mocks.requestedKeys).toEqual(["git.commits", "tasks.closed"]);
    expect(screen.queryByRole("columnheader", { name: /Messages/ })).not.toBeInTheDocument();
  });

  it("narrows the table to the team the reader picks", async () => {
    render(<MetricsView />);

    await userEvent.click(screen.getByRole("combobox", { name: "Team" }));
    await userEvent.click(await screen.findByRole("option", { name: /^a\b/ }));

    expect(portalRouter.search).toMatchObject({ scope: pid("a") });
  });

  it("keeps Compare off until two people are picked", () => {
    render(<MetricsView />);

    expect(screen.getByRole("switch", { name: "Compare" })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
  });

  it("compares the picked people side by side, one row per metric", async () => {
    const user = userEvent.setup();
    render(<MetricsView />);

    await user.click(screen.getByRole("button", { name: "Filter people, all 4 shown" }));
    await user.click(screen.getByRole("button", { name: "None" }));
    await user.click(screen.getByRole("checkbox", { name: "a" }));
    await user.click(screen.getByRole("checkbox", { name: "b" }));
    await user.click(screen.getByRole("switch", { name: "Compare" }));

    const table = screen.getByRole("table");
    expect(rowHeaders()).toEqual(["Commits", "Messages"]);
    expect(
      within(table)
        .getAllByRole("columnheader")
        .map((header) => header.textContent),
    ).toEqual(["Metric", "a", "b"]);
    expect(
      within(within(table).getByRole("row", { name: /Commits/ }))
        .getAllByRole("cell")
        .map((cell) => cell.textContent),
    ).toEqual(["10", "20"]);
  });

  it("gives the menu back on request", async () => {
    const shown = renderHook(() => useMetricsViewPaneShown());
    render(<MetricsView />);

    await userEvent.click(screen.getByRole("button", { name: "Show menu" }));

    expect(shown.result.current).toBe(true);
    expect(screen.getByRole("button", { name: "Hide menu" })).toBeInTheDocument();
  });
});
