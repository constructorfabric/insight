// @vitest-environment jsdom
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { portalRouter } from "@/test/portal-router";

import { act, render, renderHook, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { MetricDefinition } from "@/api/metric-definitions-client";
import type { PeopleListItem } from "@/api/identity-client";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import {
  setMetricsViewPaneShown,
  useMetricsViewPaneShown,
} from "@/lib/portal/metrics-view-pane";
import { parseNavPolicy, type InstanceNavPolicy } from "@/lib/portal/nav-policy";
import { EvidenceDialogContext } from "@/components/metric-evidence-context";
import { identityPerson, peopleFromIdentityTree, pid } from "@/test/identity";
import { AppTooltips, openTooltip } from "@/test/tooltip";

const mocks = vi.hoisted(() => ({
  personId: null as string | null,
  layout: "wide" as "phone" | "narrow" | "wide",
  roster: [] as PeopleListItem[],
  definitions: [] as MetricDefinition[],
  requestedKeys: [] as readonly string[],
  requestedIds: [] as readonly string[],
  definitionsError: false,
  refetchDefinitions: vi.fn(),
  policy: null as InstanceNavPolicy | null,
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
vi.mock("@/lib/portal/nav-policy", async () => {
  const actual = await vi.importActual<typeof import("@/lib/portal/nav-policy")>(
    "@/lib/portal/nav-policy",
  );
  return { ...actual, navPolicy: () => mocks.policy ?? actual.EMPTY_NAV_POLICY };
});
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
    isError: mocks.definitionsError,
    refetch: mocks.refetchDefinitions,
  }),
}));
vi.mock("@/queries/metrics-view", () => ({
  useMetricsViewData: (keys: readonly string[], ids: readonly string[]) => {
    mocks.requestedKeys = keys;
    mocks.requestedIds = ids;
    return mocks.data;
  },
}));
vi.mock("@/lib/portal/use-shell-layout", () => ({
  useShellLayout: () => mocks.layout,
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
  over: Partial<NormalizedMetricResult> = {},
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
    ...over,
  } as NormalizedMetricResult;
}

async function compare(user: ReturnType<typeof userEvent.setup>, names: readonly string[]) {
  await user.click(screen.getByRole("button", { name: "Compare" }));
  for (const name of names) await user.click(screen.getByRole("checkbox", { name }));
  await user.click(screen.getByRole("button", { name: `Compare (${names.length})` }));
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
  mocks.layout = "wide";
  mocks.definitionsError = false;
  mocks.refetchDefinitions.mockReset();
  mocks.policy = null;
  mocks.roster = peopleFromIdentityTree(tree);
  mocks.definitions = [
    definition("collab.messages_sent", "Messages"),
    definition("git.default_branch_commits", "Default branch commits", {
      description: "Commits that landed on the default branch.",
      explanation: "Counted per author, merges included.",
    }),
    definition("tasks.closed", "Tasks closed"),
    definition("ci.runs", "CI runs", { entity_type: "tenant" }),
  ];
  mocks.data.isPending = false;
  mocks.data.isError = false;
  mocks.data.byKey = new Map([
    [
      "git.default_branch_commits",
      result(
        "git.default_branch_commits",
        "Default branch commits",
        [
          [pid("boss"), 50],
          [pid("a"), 10],
          [pid("b"), 20],
          [pid("c"), 30],
        ],
        {
          short_label: "Commits",
          drilldown: { granularity: ["event"] },
          selection: {
            metric_key: "git.default_branch_commits",
            entity: { type: "person", ids: [pid("boss")] },
            period: { from: "2026-07-01", to: "2026-07-31" },
            filters: [],
          },
        },
      ),
    ],
    [
      "collab.messages_sent",
      result("collab.messages_sent", "Messages", [
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
    portalRouter.set({ scope: undefined, direct: false, metrics: undefined, compare: undefined });
    setMetricsViewPaneShown(false);
  });
});

describe("MetricsView", () => {
  it("opens on the scope's manager with their direct reports under them", () => {
    render(<MetricsView />);

    expect(rowHeaders()).toEqual(["boss", "a", "b"]);
  });

  it("shows a manager's reports once expanded", async () => {
    render(<MetricsView />);

    await userEvent.click(screen.getByRole("button", { name: "Expand a" }));

    expect(rowHeaders()).toEqual(["boss", "a", "c", "b"]);
  });

  it("opens every manager at once", async () => {
    render(<MetricsView />);

    await userEvent.click(screen.getByRole("button", { name: "Expand all" }));

    expect(rowHeaders()).toEqual(["boss", "a", "c", "b"]);
  });

  it("names every metric column in full, under its family", () => {
    render(<MetricsView />);

    expect(screen.getByRole("columnheader", { name: "Default branch commits" })).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: "Development · Git" })).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: "Messages" })).toBeInTheDocument();
    expect(screen.queryByRole("columnheader", { name: /Tasks closed/ })).not.toBeInTheDocument();
  });

  it("shows each person's value as plain text in its cell", () => {
    render(<MetricsView />);

    const rowOfB = screen
      .getAllByRole("rowheader")
      .find((header) => header.textContent === "b")!
      .closest("tr")!;

    expect(
      within(rowOfB)
        .getAllByRole("cell")
        .map((cell) => cell.textContent),
    ).toEqual(["20", "6"]);
  });

  it("writes a count unit once, in its header, and only the number in each cell", () => {
    mocks.definitions.push(definition("git.pr_size", "PR size"));
    mocks.data.byKey.set(
      "git.pr_size",
      result("git.pr_size", "PR size", [[pid("b"), 237]], { unit: "lines" }),
    );
    render(<MetricsView />);

    const header = screen.getByRole("columnheader", { name: /PR size/ });
    const rowOfB = screen
      .getAllByRole("rowheader")
      .find((cell) => cell.textContent === "b")!
      .closest("tr")!;

    expect(header).toHaveTextContent("PR sizelines");
    expect(within(rowOfB).getAllByRole("cell").map((cell) => cell.textContent)).toContain("237");
  });

  it("keeps a duration's units in each cell", () => {
    mocks.definitions.push(definition("git.pr_cycle_time_h", "Cycle time"));
    mocks.data.byKey.set(
      "git.pr_cycle_time_h",
      result("git.pr_cycle_time_h", "Cycle time", [[pid("b"), 2.25]], { unit: "h", format: "decimal" }),
    );
    render(<MetricsView />);

    const rowOfB = screen
      .getAllByRole("rowheader")
      .find((cell) => cell.textContent === "b")!
      .closest("tr")!;

    expect(within(rowOfB).getAllByRole("cell").map((cell) => cell.textContent)).toContain(
      "2 h 15 min",
    );
  });

  it("shows each person's role beside their name, in the table and in a comparison", async () => {
    mocks.roster = peopleFromIdentityTree(
      identityPerson("boss", {}, [
        identityPerson("a", {}, [identityPerson("c")]),
        identityPerson("b", { job_title: "Software Engineer" }),
      ]),
    );
    const user = userEvent.setup();
    render(<MetricsView />);

    expect(within(screen.getByText("Software Engineer").closest("tr")!).getByRole("button", { name: "b" })).toBeInTheDocument();

    await compare(user, ["a", "b"]);

    expect(screen.getByRole("columnheader", { name: /^b/ })).toHaveTextContent("Software Engineer");
  });

  it("asks for the metric catalog again when Retry is pressed after it failed", async () => {
    mocks.definitionsError = true;
    render(<MetricsView />);

    await userEvent.click(screen.getByRole("button", { name: "Retry" }));

    expect(mocks.refetchDefinitions).toHaveBeenCalled();
  });

  it("asks for every person-level metric, in family order", () => {
    render(<MetricsView />);

    expect(mocks.requestedKeys).toEqual([
      "git.default_branch_commits",
      "tasks.closed",
      "collab.messages_sent",
    ]);
  });

  it("leaves out a metric no person section shows", () => {
    mocks.definitions.push(definition("git.prs_created", "PRs opened, any branch"));

    render(<MetricsView />);

    expect(mocks.requestedKeys).not.toContain("git.prs_created");
    expect(
      screen.queryByRole("columnheader", { name: /PRs opened, any branch/ }),
    ).not.toBeInTheDocument();
  });

  it("keeps a custom metric no person section shows", () => {
    mocks.definitions.push(
      definition("custom.velocity", "Velocity", { origin: "custom", last_observed_date: null }),
    );

    render(<MetricsView />);

    expect(mocks.requestedKeys).toContain("custom.velocity");
  });

  it("leaves out the metrics of a person section this install plans", () => {
    mocks.policy = parseNavPolicy({ planned: ["zone:person/section:collaboration"] });

    render(<MetricsView />);

    expect(mocks.requestedKeys).toEqual(["git.default_branch_commits", "tasks.closed"]);
  });

  it("drops a metric the reader unticks", async () => {
    const user = userEvent.setup();
    render(<MetricsView />);

    await user.click(screen.getByRole("button", { name: "Metrics: 3 of 3" }));
    await user.click(screen.getByRole("checkbox", { name: "Messages" }));

    expect(mocks.requestedKeys).toEqual(["git.default_branch_commits", "tasks.closed"]);
    expect(screen.queryByRole("columnheader", { name: /Messages/ })).not.toBeInTheDocument();
  });

  it("clears every metric at once, and brings them all back", async () => {
    const user = userEvent.setup();
    render(<MetricsView />);

    await user.click(screen.getByRole("button", { name: "Metrics: 3 of 3" }));
    await user.click(screen.getByRole("button", { name: "Clear all metrics" }));

    expect(mocks.requestedKeys).toEqual([]);
    expect(screen.getByText("No metrics picked.")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Select all metrics" }));

    expect(mocks.requestedKeys).toEqual(["git.default_branch_commits", "tasks.closed", "collab.messages_sent"]);
  });

  it("scopes the table to the person whose name is clicked", async () => {
    render(<MetricsView />);

    await userEvent.click(screen.getByRole("button", { name: "a" }));

    expect(portalRouter.search).toMatchObject({ scope: pid("a") });
  });

  it("names the scoped person and clears the scope from that chip", async () => {
    act(() => portalRouter.set({ scope: pid("a") }));
    render(<MetricsView />);

    expect(rowHeaders()).toEqual(["a", "c"]);
    await userEvent.click(screen.getByRole("button", { name: "Clear scope a" }));

    expect(portalRouter.search.scope).toBeFalsy();
  });

  it("ranks everyone in scope by a metric when its header is clicked", async () => {
    render(<MetricsView />);

    await userEvent.click(screen.getByRole("button", { name: "Sort by Default branch commits" }));

    expect(rowHeaders()).toEqual(["boss", "c", "b", "a"]);
  });

  it("opens the records behind a value when it is clicked", async () => {
    const openEvidenceTargets = vi.fn();
    render(
      <EvidenceDialogContext.Provider
        value={{ openEvidence: vi.fn(), openEvidenceTargets, openEvidencePeople: vi.fn() }}
      >
        <MetricsView />
      </EvidenceDialogContext.Provider>,
    );

    await userEvent.click(screen.getByRole("button", { name: "b — Default branch commits: 20" }));

    expect(openEvidenceTargets).toHaveBeenCalledWith(
      [
        expect.objectContaining({
          label: "Default branch commits",
          selection: expect.objectContaining({
            metric_key: "git.default_branch_commits",
            entity: { type: "person", id: pid("b") },
          }),
        }),
      ],
      {
        activeMetricKey: "git.default_branch_commits",
        title: expect.stringMatching(/^Default branch commits · b · 20 · 1 Jul( 2026)? – 31 Jul/),
      },
    );
  });

  it("explains a metric when its title is hovered", async () => {
    const user = userEvent.setup();
    render(<MetricsView />, { wrapper: AppTooltips });

    await user.hover(screen.getByRole("button", { name: "Sort by Default branch commits" }));

    await waitFor(() =>
      expect(openTooltip()).toHaveTextContent(
        "Commits that landed on the default branch.Counted per author, merges included.",
      ),
    );
  });

  it("keeps the comparison shut until two people are picked", async () => {
    const user = userEvent.setup();
    render(<MetricsView />);

    await user.click(screen.getByRole("button", { name: "Compare" }));
    await user.click(screen.getByRole("checkbox", { name: "a" }));

    expect(screen.getByRole("button", { name: "Compare (1)" })).toBeDisabled();
  });

  it("compares the people picked in the Compare picker, one row per metric", async () => {
    const user = userEvent.setup();
    render(<MetricsView />);

    await compare(user, ["a", "b"]);

    const table = screen.getByRole("table");
    expect(rowHeaders()).toEqual(["Default branch commits", "Messages"]);
    expect(
      within(table)
        .getAllByRole("columnheader")
        .map((header) => header.textContent),
    ).toEqual(["Metric", "a", "b"]);
    expect(
      within(within(table).getByRole("row", { name: /Default branch commits/ }))
        .getAllByRole("cell")
        .map((cell) => cell.textContent),
    ).toEqual(["10−50%", "20 (best)"]);
  });

  it("marks the smallest value best when lower is better", async () => {
    mocks.definitions.push(definition("git.pr_cycle_time_h", "PR cycle time"));
    mocks.data.byKey.set(
      "git.pr_cycle_time_h",
      result("git.pr_cycle_time_h", "PR cycle time", [[pid("a"), 10], [pid("b"), 40]], {
        direction: "lower_is_better",
      }),
    );
    const user = userEvent.setup();
    render(<MetricsView />);

    await compare(user, ["a", "b"]);

    expect(
      within(within(screen.getByRole("table")).getByRole("row", { name: /PR cycle time/ }))
        .getAllByRole("cell")
        .map((cell) => cell.textContent),
    ).toEqual(["10 (best)", "40+300%"]);
  });

  it("adds a whole team to the comparison in one pick", async () => {
    const user = userEvent.setup();
    render(<MetricsView />);

    await user.click(screen.getByRole("button", { name: "Compare" }));
    await user.click(screen.getByRole("checkbox", { name: "a's team" }));
    await user.click(screen.getByRole("button", { name: "Compare (2)" }));

    expect(
      within(screen.getByRole("table"))
        .getAllByRole("columnheader")
        .map((header) => header.textContent),
    ).toEqual(["Metric", "a", "c"]);
  });

  it("opens the records behind a compared value when it is clicked", async () => {
    const user = userEvent.setup();
    const openEvidenceTargets = vi.fn();
    render(
      <EvidenceDialogContext.Provider
        value={{ openEvidence: vi.fn(), openEvidenceTargets, openEvidencePeople: vi.fn() }}
      >
        <MetricsView />
      </EvidenceDialogContext.Provider>,
    );

    await compare(user, ["a", "b"]);
    await user.click(screen.getByRole("button", { name: "a — Default branch commits: 10" }));

    expect(openEvidenceTargets).toHaveBeenCalledWith(
      [
        expect.objectContaining({
          selection: expect.objectContaining({ entity: { type: "person", id: pid("a") } }),
        }),
      ],
      expect.objectContaining({ title: expect.stringMatching(/^Default branch commits · a · 10 · /) }),
    );
  });

  it("drops a whole team when it is picked again", async () => {
    const user = userEvent.setup();
    render(<MetricsView />);

    await user.click(screen.getByRole("button", { name: "Compare" }));
    await user.click(screen.getByRole("checkbox", { name: "a's team" }));
    expect(screen.getByRole("checkbox", { name: "a's team" })).toBeChecked();

    await user.click(screen.getByRole("checkbox", { name: "a's team" }));

    expect(screen.getByRole("checkbox", { name: "a's team" })).not.toBeChecked();
    expect(screen.getByRole("button", { name: "Compare (0)" })).toBeDisabled();
  });

  it("keeps the comparison while new numbers load", async () => {
    const user = userEvent.setup();
    const view = render(<MetricsView />);
    await compare(user, ["a", "b"]);

    mocks.data.isPending = true;
    view.rerender(<MetricsView />);
    expect(screen.getByRole("button", { name: "Exit compare" })).toBeInTheDocument();

    mocks.data.isPending = false;
    view.rerender(<MetricsView />);
    expect(rowHeaders()).toEqual(["Default branch commits", "Messages"]);
  });

  it("asks for everyone under the scope even with direct reports only on", () => {
    act(() => portalRouter.set({ direct: true }));
    render(<MetricsView />);

    expect([...mocks.requestedIds].sort()).toEqual(
      [pid("boss"), pid("a"), pid("b"), pid("c")].sort(),
    );
  });

  it("puts the picked metrics in the link", async () => {
    const user = userEvent.setup();
    render(<MetricsView />);

    await user.click(screen.getByRole("button", { name: "Metrics: 3 of 3" }));
    await user.click(screen.getByRole("checkbox", { name: "Messages" }));

    expect(portalRouter.search).toMatchObject({ metrics: "git.default_branch_commits,tasks.closed" });
  });

  it("opens on the metrics a link names", () => {
    act(() => portalRouter.set({ metrics: "git.default_branch_commits" }));
    render(<MetricsView />);

    expect(mocks.requestedKeys).toEqual(["git.default_branch_commits"]);
  });

  it("puts a comparison in the link and opens on one a link names", async () => {
    const user = userEvent.setup();
    const first = render(<MetricsView />);
    await compare(user, ["a", "b"]);

    expect(portalRouter.search).toMatchObject({ compare: `${pid("a")},${pid("b")}` });

    first.unmount();
    render(<MetricsView />);
    expect(screen.getByRole("button", { name: "Exit compare" })).toBeInTheDocument();
  });

  it("goes back to the table when the comparison is closed", async () => {
    const user = userEvent.setup();
    render(<MetricsView />);

    await compare(user, ["a", "b"]);
    await user.click(screen.getByRole("button", { name: "Exit compare" }));

    expect(rowHeaders()).toEqual(["boss", "a", "b"]);
  });

  it.each(["narrow", "phone"] as const)(
    "leaves the menu to the shell's own drawer on a %s screen",
    (layout) => {
      mocks.layout = layout;
      render(<MetricsView />);

      expect(screen.queryByRole("button", { name: "Show menu" })).not.toBeInTheDocument();
    },
  );

  it("gives the menu back on request", async () => {
    const shown = renderHook(() => useMetricsViewPaneShown());
    render(<MetricsView />);

    await userEvent.click(screen.getByRole("button", { name: "Show menu" }));

    expect(shown.result.current).toBe(true);
    expect(screen.getByRole("button", { name: "Hide menu" })).toBeInTheDocument();
  });
});
