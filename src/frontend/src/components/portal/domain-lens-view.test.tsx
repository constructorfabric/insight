// @vitest-environment jsdom
/**
 * Behavioral tests for DomainLensView — the single renderer behind every
 * Directions lens and Overview item. Data enters through the query hooks
 * (viewer tree, roster, member grid, timeseries), which are stubbed at the
 * module boundary with REALISTIC payloads; every assertion is about what a
 * manager actually reads on screen: per-capita numbers, deltas, honest
 * not-ingested/suppression states, framing copy and roll-up math.
 */
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { portalRouter } from "@/test/portal-router";

import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { GROUPS } from "@/lib/insight/groups";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import { identityPerson, pid } from "@/test/identity";
import type { IdentityPerson } from "@/types/insight";

/* ── module mocks ────────────────────────────────────────────────────── */

const mocks = vi.hoisted(() => ({
  personId: null as string | null,
  tree: undefined as IdentityPerson | undefined,
  grid: {
    byKey: new Map<string, NormalizedMetricResult>(),
    previousByKey: new Map<string, NormalizedMetricResult>(),
    isPending: false,
    isFetching: false,
    isError: false,
    refetch: vi.fn(),
  },
  call: 0,
  /** Every collection the view asked for, in call order. */
  requested: [] as Array<{
    metrics: Array<{
      key: string;
      filters?: Array<{ dimension: string; values: string[] }>;
      views: unknown[];
    }>;
  }>,
  collectionSet: new Map<string, unknown>(),
  definitions: [] as unknown[],
  collections: [] as Array<{
    byKey: Map<string, NormalizedMetricResult>;
    isPending: boolean;
    isError: boolean;
    refetch: () => void;
  }>,
}));

vi.mock("@/auth", () => ({
  useViewer: () => ({ email: "boss@x", personId: mocks.personId }),
}));
vi.mock("@/queries/ic-dashboard", () => ({
  useIcPerson: () => ({
    data: mocks.tree,
    isPending: false,
    isLoading: false,
    isError: false,
    refetch: vi.fn(),
  }),
}));
// `useOrgScope` reads the deployment's visibility policy, and its flat branch
// reads the roster. This suite is about the view, so both answer statically.
vi.mock("@/queries/identity-me", () => ({
  useVisibilityPolicy: () => ({
    policy: "org_chart",
    isFlat: false,
    isPending: false,
  }),
}));
vi.mock("@/queries/visible-roster", () => ({
  useVisibleRoster: () => ({
    roster: [],
    truncated: false,
    isPending: false,
    isError: false,
    retry: () => {},
  }),
}));
vi.mock("@/queries/member-grid", () => ({
  useMemberGridData: () => mocks.grid,
}));
// DomainLensView calls useMetricCollection five times (trend, composition,
// event-histogram, dimension-table rollup, hour-block heatmap) in that order on
// every render. The counter is reset per test (see beforeEach): left running,
// each caller's slot would depend on how many renders every earlier test
// happened to do.
const COLLECTION_CALLS = 5;
vi.mock("@/queries/metric-results", () => ({
  useMetricCollection: (collection: unknown) => {
    mocks.requested.push(
      collection as (typeof mocks.requested)[number],
    );
    const r = mocks.collections[mocks.call % COLLECTION_CALLS] ?? emptyCollection();
    mocks.call += 1;
    return r;
  },
  // Only the coverage section reads the set form; it fetches its own
  // period-only collection rather than riding the zone grid.
  useMetricCollectionSet: () => mocks.collectionSet,
}));
vi.mock("@/queries/metric-definitions", async (orig) => ({
  ...(await orig<Record<string, unknown>>()),
  useMetricDefinitionsResponse: () => ({
    data: { metrics: mocks.definitions },
    isPending: false,
  }),
}));
vi.mock("@/hooks/use-portal-period", () => ({
  usePortalPeriod: () => ({
    period: "week",
    dateRange: { from: "2026-07-20", to: "2026-07-26" },
  }),
}));
// Charts are exercised by the browser/storybook project; here they'd render
// into a 0×0 jsdom box. Stub them with introspectable placeholders.
vi.mock("@/components/portal/section-trend", () => ({
  SectionTrend: ({ series }: { series: unknown[] }) => (
    <div data-testid="section-trend" data-series={JSON.stringify(series ?? []).length} />
  ),
}));
// The sparkle reads the AI config through react-query; this suite renders the
// view without a client, and what it explains is covered by its own tests.
vi.mock("@/components/widgets/dashboard/explain-with-ai", () => ({
  ExplainWithAi: () => null,
}));
// The drilldown fetches per member; this suite is about which drilldown the
// view asks for, so the dialog reports the state it was handed instead.
vi.mock("@/components/portal/trend-drilldown-dialog", () => ({
  TrendDrilldownDialog: ({ state }: { state: unknown }) =>
    state ? (
      <div data-testid="drilldown" data-state={JSON.stringify(state)} />
    ) : null,
}));


import { EvidenceDialogContext } from "@/components/metric-evidence-context";
import type { LensConfig } from "@/lib/portal/lens-configs";
import { DomainLensView } from "./domain-lens-view";

/* ── fixtures ────────────────────────────────────────────────────────── */

function emptyCollection() {
  return {
    byKey: new Map<string, NormalizedMetricResult>(),
    previousByKey: null,
    isPending: false,
    isFetching: false,
    isError: false,
    refetch: vi.fn(),
  };
}

function metric(
  key: string,
  period: Array<[string, number | null]>,
  over: Partial<NormalizedMetricResult> = {},
): NormalizedMetricResult {
  return {
    metric_key: key,
    label: over.label ?? key,
    unit: null,
    computation: "sum",
    format: "integer",
    direction: "higher_is_better",
    period: {
      view: "period",
      values: period.map(([entity_id, value]) => ({ entity_id, value })),
    },
    peer: {
      view: "peer",
      values: period.map(([entity_id, value]) => ({ entity_id, target_value: value })),
    },
    ...over,
  } as unknown as NormalizedMetricResult;
}

const person = (
  label: string,
  over: Partial<IdentityPerson> = {},
  subordinates: IdentityPerson[] = [],
): IdentityPerson => identityPerson(label, over, subordinates);


// Roster entity ids are person UUIDs (identity cutover); labels stay legible.
const LABELS = ["a", "b", "c", "d"];
const IDS = LABELS.map(pid);

function seedHappyOrg() {
  mocks.personId = pid("boss");
  mocks.tree = person("boss", {}, LABELS.map((l) => person(l)));
  // 4 members, 10+20+30+40 = 100 commits; everyone active.
  mocks.grid.byKey = new Map([
    ["t.commits", metric("t.commits", [[pid("a"), 10], [pid("b"), 20], [pid("c"), 30], [pid("d"), 40]], { short_label: "Commits", unit: "commits" })],
  ]);
  mocks.grid.previousByKey = new Map([
    ["t.commits", metric("t.commits", [[pid("a"), 20], [pid("b"), 40], [pid("c"), 60], [pid("d"), 80]])],
  ]);
  mocks.collections = [emptyCollection(), emptyCollection(), emptyCollection()];
}

const HEADLINE_CONFIG: LensConfig = {
  title: "Dev · Test",
  tagline: "test lens",
  sections: [{ kind: "headline", metrics: ["t.commits"] }],
};

beforeEach(() => {
  mocks.call = 0;
  mocks.requested = [];
  seedHappyOrg();
  mocks.grid.isPending = false;
  mocks.grid.isError = false;
  act(() => {
    portalRouter.set({ slice: undefined, repo: undefined });
    portalRouter.set({ scope: undefined, direct: false });
  });
});

afterEach(() => vi.clearAllMocks());

/* ── tests ───────────────────────────────────────────────────────────── */

describe("headline (rules 1–2: per-capita + PoP delta)", () => {
  it("leads with the team total the dialog explains, and keeps the per-person read", () => {
    render(<DomainLensView config={HEADLINE_CONFIG} />);
    // The figure is the roster's, because that is the set the evidence dialog
    // lists; 100 commits over 4 active people = 25/person underneath.
    expect(screen.getByText("100 commits")).toBeInTheDocument();
    expect(screen.getByText(/25 commits per active person/)).toBeInTheDocument();
    expect(screen.getByText("-50%")).toBeInTheDocument();
    // header carries the scope size + tagline
    expect(screen.getByText(/4 people · test lens/)).toBeInTheDocument();
  });

  it("divides by ACTIVE people only — zeros don't dilute the denominator", () => {
    mocks.grid.byKey = new Map([
      ["t.commits", metric("t.commits", [[pid("a"), 0], [pid("b"), 0], [pid("c"), 30], [pid("d"), 70]], { short_label: "Commits", unit: "commits" })],
    ]);
    mocks.grid.previousByKey = new Map();
    render(<DomainLensView config={HEADLINE_CONFIG} />);
    // 100 total / 2 active = 50 per person, not 25
    expect(screen.getByText(/50 commits per active person/)).toBeInTheDocument();
  });
});

describe("headline cards open the records behind them", () => {
  const openEvidenceTargets = vi.fn();
  const openEvidence = vi.fn();
  const openEvidencePeople = vi.fn();

  function drawWithEvidence(config: LensConfig = HEADLINE_CONFIG) {
    return render(
      <EvidenceDialogContext.Provider
        value={{ openEvidence, openEvidenceTargets, openEvidencePeople }}
      >
        <DomainLensView config={config} />
      </EvidenceDialogContext.Provider>
    );
  }

  it("opens the roster's own records, not one person's", async () => {
    const user = userEvent.setup();
    mocks.grid.byKey = new Map([
      [
        "t.commits",
        metric(
          "t.commits",
          IDS.map((id) => [id, 10] as [string, number]),
          {
            short_label: "Commits",
            unit: "commits",
            drilldown: { granularity: ["event"] },
            selection: {
              metric_key: "t.commits",
              entity: { type: "person", ids: IDS },
              period: { from: "2026-07-20", to: "2026-07-26" },
              filters: [],
            },
          } as Partial<NormalizedMetricResult>
        ),
      ],
    ]);
    drawWithEvidence();

    await user.click(screen.getByRole("button", { name: /Commits|t.commits/ }));

    expect(openEvidenceTargets).toHaveBeenCalledTimes(1);
    const [targets, options] = openEvidenceTargets.mock.calls[0]!;
    expect(options).toEqual({ activeMetricKey: "t.commits" });
    // The card's figure is the roster's, so its records are the roster's too —
    // asked for as one selection over every member.
    expect(targets[0].selection.entity).toEqual({
      type: "persons",
      ids: [...IDS].sort(),
    });
  });

  it("stays a plain card for a metric whose evidence cannot be read", () => {
    drawWithEvidence();

    // The seeded metric declares no drilldown: an affordance that opens an
    // empty dialog is worse than none.
    expect(
      screen.queryByRole("button", { name: /Commits|t.commits/ })
    ).not.toBeInTheDocument();
  });
});

describe("rule 6: honest not-ingested gate", () => {
  it("renders the family not-ingested note when nothing was ever observed", () => {
    mocks.grid.byKey = new Map([
      ["t.commits", metric("t.commits", IDS.map((id) => [id, 0]), {
        peer: { view: "peer", values: [] },
      } as never)],
    ]);
    mocks.grid.previousByKey = new Map();
    render(
      <DomainLensView
        config={{ ...HEADLINE_CONFIG, notIngested: "Git source isn't wired for this org yet." }}
      />,
    );
    expect(screen.getByText("Git source isn't wired for this org yet.")).toBeInTheDocument();
    expect(screen.queryByText(/team total/)).not.toBeInTheDocument();
  });
});

describe("org-scope gates", () => {
  it("shows the empty-roster label instead of a fabricated dashboard", () => {
    mocks.tree = person("boss");
    render(<DomainLensView config={HEADLINE_CONFIG} />);
    expect(screen.getByText(/No people in the current scope/)).toBeInTheDocument();
  });

  it("surfaces a grid failure as retryable error", () => {
    mocks.grid.isError = true;
    render(<DomainLensView config={HEADLINE_CONFIG} />);
    expect(screen.getByRole("button", { name: /retry/i })).toBeInTheDocument();
  });
});

describe("stat-tiles (medians, never sums)", () => {
  it("renders the cohort median for ratio metrics under a section title", () => {
    mocks.grid.byKey.set(
      "t.cycle",
      metric("t.cycle", [[pid("a"), 10], [pid("b"), 20], [pid("c"), 30], [pid("d"), 40]], {
        computation: "avg",
        label: "PR cycle",
        format: "float",
      } as never),
    );
    render(
      <DomainLensView
        config={{
          title: "T",
          sections: [{ kind: "stat-tiles", title: "Flow health", metrics: ["t.cycle"] }],
        }}
      />,
    );
    expect(screen.getByText("Flow health")).toBeInTheDocument();
    // median of 10,20,30,40 = 25
    expect(screen.getByText("25")).toBeInTheDocument();
    expect(screen.getByText(/median \/ person/)).toBeInTheDocument();
  });
});

describe("aggregate sections open the people behind them", () => {
  const openEvidenceTargets = vi.fn();
  const openEvidence = vi.fn();
  const openEvidencePeople = vi.fn();

  /** A metric with a real spread: 1, 2, 3 and 9 commits over the four members. */
  function seedSpread({ drillable }: { drillable: boolean }) {
    const evidence = drillable
      ? {
          drilldown: { granularity: ["event"] },
          selection: {
            metric_key: "t.commits",
            entity: { type: "person", ids: IDS },
            period: { from: "2026-07-20", to: "2026-07-26" },
            filters: [],
          },
        }
      : {};
    mocks.grid.byKey = new Map([
      [
        "t.commits",
        metric(
          "t.commits",
          [
            [pid("a"), 1],
            [pid("b"), 2],
            [pid("c"), 3],
            [pid("d"), 9],
          ],
          {
            label: "Commits",
            short_label: "Commits",
            unit: "commits",
            ...evidence,
          } as Partial<NormalizedMetricResult>
        ),
      ],
    ]);
    mocks.grid.previousByKey = new Map();
  }

  function drawWithEvidence(config: LensConfig) {
    return render(
      <EvidenceDialogContext.Provider
        value={{ openEvidence, openEvidenceTargets, openEvidencePeople }}
      >
        <DomainLensView config={config} />
      </EvidenceDialogContext.Provider>
    );
  }

  const SPREAD_CONFIG: LensConfig = {
    title: "T",
    sections: [
      {
        kind: "distribution",
        metric: "t.commits",
        title: "Spread",
        caption: "spread",
        unitLabel: "commits per person",
      },
    ],
  };

  const CONCENTRATION_CONFIG: LensConfig = {
    title: "T",
    sections: [
      { kind: "concentration", metrics: ["t.commits"], framing: "bus-factor" },
    ],
  };

  it("opens a band's own people, with the values the bars were drawn from", async () => {
    const user = userEvent.setup();
    seedSpread({ drillable: true });
    drawWithEvidence(SPREAD_CONFIG);

    // Step 1 over a maximum of 9 → unit-wide bands, and the top value is
    // clamped into the last one, so the lone 9 is the whole "8–9" band.
    await user.click(screen.getByRole("button", { name: /^8–9 commits/ }));

    expect(openEvidencePeople).toHaveBeenCalledTimes(1);
    const [view] = openEvidencePeople.mock.calls[0]!;
    expect(view.title).toContain("8–9 commits per person");
    expect(view.rows.map((row: { entityId: string }) => row.entityId)).toEqual([
      pid("d"),
    ]);
    expect(view.rows[0].value).toBe(9);
    // The roster's name and its route id, each from its own lookup: this is
    // what makes the list about people rather than about ids.
    expect(view.rows[0].name).toBe("d");
    expect(view.rows[0].personId).toBe(pid("d"));
    // Every row's records at once stays available, scoped to the same band.
    expect(view.allRecords.selection.entity).toEqual({
      type: "persons",
      ids: [pid("d")],
    });
  });

  it("lists the people of a metric whose records cannot be read", async () => {
    const user = userEvent.setup();
    seedSpread({ drillable: false });
    drawWithEvidence(SPREAD_CONFIG);

    await user.click(screen.getByRole("button", { name: /^8–9 commits/ }));

    // The values are the surface's own, so who is in the band is answerable
    // even where no source backs a record table.
    const [view] = openEvidencePeople.mock.calls[0]!;
    expect(view.rows[0].target).toBeNull();
    expect(view.allRecords).toBeNull();
  });

  it("opens the busiest tenth from the concentration card, ranked", async () => {
    const user = userEvent.setup();
    seedSpread({ drillable: true });
    drawWithEvidence(CONCENTRATION_CONFIG);

    await user.click(screen.getByRole("button", { name: /carried by/ }));

    const [view] = openEvidencePeople.mock.calls[0]!;
    expect(view.title).toContain("busiest 1 of 4");
    // Busiest tenth of 4 contributors = 1 person, and it is the busiest one.
    expect(view.rows.map((row: { value: number }) => row.value)).toEqual([9]);
  });
});

describe("distribution (rule 4: integer 1/2/5 bins, self-suppressing)", () => {
  it("suppresses a degenerate single-bin distribution entirely", () => {
    mocks.grid.byKey = new Map([
      ["t.commits", metric("t.commits", [[pid("a"), 5], [pid("b"), 5], [pid("c"), 5], [pid("d"), 5]], { short_label: "Commits" })],
    ]);
    render(
      <DomainLensView
        config={{
          title: "T",
          sections: [
            { kind: "headline", metrics: ["t.commits"] },
            { kind: "distribution", metric: "t.commits", title: "Spread", caption: "spread", unitLabel: "commits per person" },
          ],
        }}
      />,
    );
    expect(screen.queryByText("Spread")).not.toBeInTheDocument();
  });
});

describe("concentration (rule 5: top-decile share with framing)", () => {
  it("frames git concentration as bus-factor risk with the share and count", () => {
    render(
      <DomainLensView
        config={{
          title: "T",
          sections: [{ kind: "concentration", metrics: ["t.commits"], framing: "bus-factor" }],
        }}
      />,
    );
    // top 10% of 4 people = busiest 1 of 4 → 40 of 100 = 40%
    expect(screen.getByText("40%")).toBeInTheDocument();
    expect(screen.getByText(/carried by the busiest 1 of 4/)).toBeInTheDocument();
    expect(screen.getByText(/continuity risk/)).toBeInTheDocument();
  });

  it("frames collaboration concentration as load balance, not risk", () => {
    render(
      <DomainLensView
        config={{
          title: "T",
          sections: [{ kind: "concentration", metrics: ["t.commits"], framing: "load-balance" }],
        }}
      />,
    );
    expect(screen.queryByText(/continuity risk/)).not.toBeInTheDocument();
    expect(screen.getByText(/load/i)).toBeInTheDocument();
  });
});

describe("composition (rule 7: only real server dimensions)", () => {
  it("renders breakdown bars with shares from the composition query", () => {
    const comp = emptyCollection();
    comp.byKey.set("t.commits", {
      ...metric("t.commits", []),
      breakdown: {
        view: "breakdown",
        values: IDS.flatMap((id) => [
          { entity_id: id, dimensions: [{ key: "category", value: "docs" }], value: 30 },
          { entity_id: id, dimensions: [{ key: "category", value: "code" }], value: 10 },
        ]),
      },
    } as never);
    mocks.collections = [emptyCollection(), comp, emptyCollection()];
    render(
      <DomainLensView
        config={{
          title: "T",
          sections: [
            { kind: "headline", metrics: ["t.commits"] },
            { kind: "composition", metric: "t.commits", dimension: "category", title: "Lines by category" },
          ],
        }}
      />,
    );
    expect(screen.getByText("Lines by category")).toBeInTheDocument();
    expect(screen.getByText("docs")).toBeInTheDocument();
    // docs 120 of 160 total = 75%
    expect(screen.getByText(/75%/)).toBeInTheDocument();
  });

  it("explains a derived dimension under the bars, not above them", () => {
    // A category label says nothing about how the bucket was decided, and the
    // explanation belongs after the reading rather than in front of it.
    const comp = emptyCollection();
    comp.byKey.set("t.commits", {
      ...metric("t.commits", []),
      breakdown: {
        view: "breakdown",
        values: IDS.flatMap((id) => [
          { entity_id: id, dimensions: [{ key: "category", value: "docs" }], value: 30 },
          { entity_id: id, dimensions: [{ key: "category", value: "code" }], value: 10 },
        ]),
      },
    } as never);
    mocks.collections = [emptyCollection(), comp, emptyCollection()];
    render(
      <DomainLensView
        config={{
          title: "T",
          sections: [
            { kind: "headline", metrics: ["t.commits"] },
            {
              kind: "composition",
              metric: "t.commits",
              dimension: "category",
              title: "Lines by category",
              notes: ["First rule that matches wins.", "Code — everything else."],
            },
          ],
        }}
      />,
    );
    expect(
      screen.getByText("First rule that matches wins."),
    ).toBeInTheDocument();
    expect(screen.getByText("Code — everything else.")).toBeInTheDocument();
  });

  it("opens the records the clicked bar stands for, filtered to it", async () => {
    const user = userEvent.setup();
    const openEvidenceTargets = vi.fn();
    const comp = emptyCollection();
    comp.byKey.set("t.commits", {
      ...metric("t.commits", []),
      breakdown: {
        view: "breakdown",
        values: IDS.flatMap((id) => [
          {
            entity_id: id,
            dimensions: [
              { key: "repository", value: "src:acme/api", label: "acme/api" },
              {
                key: "branch_scope",
                value: "default",
                label: "Default branch",
              },
            ],
            value: 30,
          },
          {
            entity_id: id,
            dimensions: [
              { key: "repository", value: "src:acme/web", label: "acme/web" },
              {
                key: "branch_scope",
                value: "non_default",
                label: "Other branches",
              },
            ],
            value: 10,
          },
        ]),
      },
    } as never);
    mocks.collections = [emptyCollection(), comp, emptyCollection()];
    mocks.grid.byKey = new Map([
      [
        "t.commits",
        metric(
          "t.commits",
          IDS.map((id) => [id, 10] as [string, number]),
          {
            label: "Commits",
            drilldown: { granularity: ["event"] },
            selection: {
              metric_key: "t.commits",
              entity: { type: "person", ids: IDS },
              period: { from: "2026-07-20", to: "2026-07-26" },
              filters: [],
            },
          } as Partial<NormalizedMetricResult>
        ),
      ],
    ]);

    render(
      <EvidenceDialogContext.Provider
        value={{
          openEvidence: vi.fn(),
          openEvidenceTargets,
          openEvidencePeople: vi.fn(),
        }}
      >
        <DomainLensView
          config={{
            title: "T",
            sections: [
              {
                kind: "composition",
                metric: "t.commits",
                dimension: "repository",
                splitBy: "branch_scope",
                title: "Lines by repository",
              },
            ],
          }}
        />
      </EvidenceDialogContext.Provider>
    );

    await user.click(
      screen.getByRole("button", { name: /acme\/api · Default branch/ })
    );

    expect(openEvidenceTargets).toHaveBeenCalledTimes(1);
    const [targets] = openEvidenceTargets.mock.calls[0]!;
    // Narrowed to the repository AND the segment: a bar that opened the
    // unfiltered metric would answer a different question from the one asked.
    expect(targets[0].selection.filters).toEqual([
      { dimension: "repository", values: ["src:acme/api"] },
      { dimension: "branch_scope", values: ["default"] },
    ]);
    // And the row says which slice it belongs to.
    expect(targets[0].selection.display_dimensions).toEqual([
      "branch_scope",
      "repository",
    ]);
  });

  it("leaves a segment the response never named inert", async () => {
    const openEvidenceTargets = vi.fn();
    const comp = emptyCollection();
    comp.byKey.set("t.commits", {
      ...metric("t.commits", []),
      breakdown: {
        view: "breakdown",
        values: IDS.flatMap((id) => [
          {
            entity_id: id,
            dimensions: [
              { key: "repository", value: "src:acme/api", label: "acme/api" },
              { key: "branch_scope", value: "default", label: "Default branch" },
            ],
            value: 30,
          },
          // No `branch_scope` at all: the row lands in the synthetic segment.
          {
            entity_id: id,
            dimensions: [
              { key: "repository", value: "src:acme/web", label: "acme/web" },
            ],
            value: 10,
          },
        ]),
      },
    } as never);
    mocks.collections = [emptyCollection(), comp, emptyCollection()];
    mocks.grid.byKey = new Map([
      [
        "t.commits",
        metric("t.commits", IDS.map((id) => [id, 10] as [string, number]), {
          label: "Commits",
          drilldown: { granularity: ["event"] },
          selection: {
            metric_key: "t.commits",
            entity: { type: "person", ids: IDS },
            period: { from: "2026-07-20", to: "2026-07-26" },
            filters: [],
          },
        } as Partial<NormalizedMetricResult>),
      ],
    ]);

    render(
      <EvidenceDialogContext.Provider
        value={{
          openEvidence: vi.fn(),
          openEvidenceTargets,
          openEvidencePeople: vi.fn(),
        }}
      >
        <DomainLensView
          config={{
            title: "T",
            sections: [
              {
                kind: "composition",
                metric: "t.commits",
                dimension: "repository",
                splitBy: "branch_scope",
                title: "Lines by repository",
              },
            ],
          }}
        />
      </EvidenceDialogContext.Provider>,
    );

    // The named segment offers its records; the unnamed one has no value to
    // filter on, so it must not offer a dialog that would come back empty.
    expect(
      screen.getByRole("button", { name: /acme\/api · Default branch/ }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /acme\/web · unsplit/ }),
    ).not.toBeInTheDocument();
  });

  it("leaves the bars inert when the carrier's records cannot be read", () => {
    const comp = emptyCollection();
    comp.byKey.set("t.commits", {
      ...metric("t.commits", []),
      breakdown: {
        view: "breakdown",
        values: IDS.flatMap((id) => [
          {
            entity_id: id,
            dimensions: [{ key: "category", value: "docs" }],
            value: 30,
          },
          {
            entity_id: id,
            dimensions: [{ key: "category", value: "code" }],
            value: 10,
          },
        ]),
      },
    } as never);
    mocks.collections = [emptyCollection(), comp, emptyCollection()];
    render(
      <DomainLensView
        config={{
          title: "T",
          sections: [
            {
              kind: "composition",
              metric: "t.commits",
              dimension: "category",
              title: "By category",
            },
          ],
        }}
      />
    );

    // An affordance that opens an empty dialog is worse than none.
    expect(
      screen.queryByRole("button", { name: /Open the records behind/ })
    ).not.toBeInTheDocument();
  });

  it("shows a retryable error card when the breakdown request fails", () => {
    const comp = emptyCollection();
    comp.isError = true;
    mocks.collections = [emptyCollection(), comp, emptyCollection()];
    render(
      <DomainLensView
        config={{
          title: "T",
          sections: [
            { kind: "headline", metrics: ["t.commits"] },
            { kind: "composition", metric: "t.commits", dimension: "category", title: "Lines by category" },
          ],
        }}
      />,
    );
    expect(screen.getByText(/unable to load/i)).toBeInTheDocument();
  });
});

describe("participation (rule 8 variant: N of M active)", () => {
  it("counts members with a non-zero value and shows the share", () => {
    mocks.grid.byKey.set(
      "t.active",
      metric("t.active", [[pid("a"), 3], [pid("b"), 0], [pid("c"), 2], [pid("d"), 0]], { label: "Active days" }),
    );
    render(
      <DomainLensView
        config={{
          title: "T",
          sections: [
            { kind: "participation", metrics: ["t.active"], title: "AI adoption", noun: "People using AI" },
          ],
        }}
      />,
    );
    expect(screen.getByText("People using AI")).toBeInTheDocument();
    expect(screen.getByText("2 of 4")).toBeInTheDocument();
    expect(screen.getByText(/50% of the team/)).toBeInTheDocument();
  });
});

describe("by-unit auto-section (rule 7: slice cohorts inside scope)", () => {
  const CONFIG: LensConfig = {
    title: "T",
    sections: [{ kind: "headline", metrics: ["t.commits"] }],
  };

  function seedSliced() {
    // Two divisions of 4 people each with very different output.
    const labels = ["a1", "a2", "a3", "a4", "b1", "b2", "b3", "b4"];
    mocks.tree = person("boss", {}, labels.map((l) =>
      person(l, { division: l.startsWith("a") ? "R&D" : "Sales" } as never),
    ));
    mocks.grid.byKey = new Map([
      ["t.commits", metric("t.commits", labels.map((l) => [pid(l), l.startsWith("a") ? 10 : 30]), { short_label: "Commits" })],
    ]);
    mocks.grid.previousByKey = new Map();
  }

  it("renders per-active-person unit bars when a slice is active", () => {
    seedSliced();
    act(() => portalRouter.set({ slice: "division" }));
    render(<DomainLensView config={CONFIG} />);
    expect(screen.getByText(/by Division/)).toBeInTheDocument();
    expect(screen.getByText(/R&D · 4/)).toBeInTheDocument();
    expect(screen.getByText(/Sales · 4/)).toBeInTheDocument();
  });

  it("stays silent without a slice", () => {
    seedSliced();
    render(<DomainLensView config={CONFIG} />);
    expect(screen.queryByText(/by Division/)).not.toBeInTheDocument();
  });

  it("explains itself when units are too small to compare (never silent)", () => {
    act(() => portalRouter.set({ slice: "division" }));
    // default org: 4 people all WITHOUT division values → no comparable units
    render(<DomainLensView config={CONFIG} />);
    expect(screen.getByText(/Nothing to compare at this grouping/)).toBeInTheDocument();
  });
});

describe("direction-cards / attention sections", () => {
  it("renders attention rows for cohort outliers, named and linked (O3)", () => {
    // 7 healthy + 1 collapsed member
    const labels = ["m1", "m2", "m3", "m4", "m5", "m6", "m7", "z"];
    mocks.tree = person("boss", {}, labels.map((l) => person(l)));
    mocks.grid.byKey = new Map([
      ["t.commits", metric("t.commits", labels.map((l) => [pid(l), l === "z" ? 0 : 10]), { label: "Commits" })],
    ]);
    mocks.grid.previousByKey = new Map();
    render(
      <DomainLensView
        config={{
          title: "T",
          sections: [{ kind: "attention", metrics: ["t.commits"], max: 8 }],
        }}
      />,
    );
    expect(screen.getByText(/1 of 8 people stands out this period/)).toBeInTheDocument();
    // The metric leads; the person is named once it is opened.
    fireEvent.click(screen.getByRole("button", { name: /Commits\s*1 person/ }));
    // Identity owns the display name now.
    expect(screen.getByText("z")).toBeInTheDocument();
    expect(screen.getByText(/no commits/)).toBeInTheDocument();
  });

});

describe("coverage section (#2408)", () => {
  /** One group reading for the named people, the rest silent. */
  function coverageWorld(seenByGitOutput: string[]) {
    const ids = seenByGitOutput.map((l) => pid(l));
    mocks.definitions = GROUPS.flatMap((g) =>
      g.collection.metrics.map((m) => ({
        metric_key: m.key,
        is_enabled: true,
        schema_status: "ok",
        schema_error_code: null,
        // Only git output has ever observed anything for this tenant, so every
        // other part must read as "no connector" rather than as idle people.
        last_observed_date: g.id === "git_output" ? "2026-07-26" : null,
      })),
    );
    const gitKey = GROUPS.find((g) => g.id === "git_output")!.collection
      .metrics[0]!.key;
    mocks.collectionSet = new Map(
      GROUPS.map((g) => [
        g.id,
        {
          byKey:
            g.id === "git_output"
              ? new Map([
                  [gitKey, metric(gitKey, ids.map((id) => [id, 3]))],
                ])
              : new Map(),
          isPending: false,
        },
      ]),
    );
  }

  it("opens a level into exactly the people at it, and why each is thin", async () => {
    mocks.tree = person("boss", {}, [person("a"), person("b"), person("c")]);
    coverageWorld(["a"]);
    render(
      <DomainLensView
        config={{ title: "T", sections: [{ kind: "coverage-levels" }] }}
      />,
    );

    // One person reads in one part; the other two read in none.
    await userEvent.click(screen.getByRole("button", { name: /1 of 5/ }));
    expect(screen.getByText("a")).toBeInTheDocument();
    expect(screen.queryByText("b")).not.toBeInTheDocument();

    // And the reason is the actionable half: nothing feeds those parts for the
    // tenant, which is a plumbing job — not people who did no work.
    expect(screen.getAllByText(/not measured for anyone:/).length).toBeGreaterThan(0);
  });

  it("does not open a level nobody is at", async () => {
    mocks.tree = person("boss", {}, [person("a")]);
    coverageWorld([]);
    render(
      <DomainLensView
        config={{ title: "T", sections: [{ kind: "coverage-levels" }] }}
      />,
    );
    expect(screen.getByRole("button", { name: /5 of 5/ })).toBeDisabled();
  });});

/* ── trend ───────────────────────────────────────────────────────────── */

const TREND_CONFIG: LensConfig = {
  title: "Dev · Test",
  sections: [
    { kind: "trend", metrics: ["t.commits"], activeContributorsFor: "t.commits" },
  ],
};

function timeseries(
  key: string,
  byEntity: Record<string, Array<[string, number | null]>>,
): NormalizedMetricResult {
  return {
    metric_key: key,
    label: key,
    unit: null,
    computation: "sum",
    format: "integer",
    direction: "higher_is_better",
    timeseries: {
      view: "timeseries",
      bucket: "day",
      series: Object.entries(byEntity).map(([entity_id, points]) => ({
        entity_id,
        points: points.map(([bucket_start, value]) => ({ bucket_start, value })),
      })),
    },
  } as unknown as NormalizedMetricResult;
}

/** Two buckets: one person contributes in the first, two in the second. */
function trendWorld() {
  const trend = emptyCollection();
  trend.byKey = new Map([
    [
      "t.commits",
      timeseries("t.commits", {
        [pid("a")]: [
          ["2026-07-20", 3],
          ["2026-07-21", 4],
        ],
        [pid("b")]: [
          ["2026-07-20", 0],
          ["2026-07-21", 3],
        ],
      }),
    ],
  ]);
  mocks.collections = [trend, emptyCollection(), emptyCollection()];
}

function openedDrilldown(): {
  metricKey: string | null;
  members: Array<{ person_id: string; name: string }>;
  breakdown: Array<{ date: string; total: number; contributors: string[] }>;
} {
  const node = screen.getByTestId("drilldown");
  return JSON.parse(node.getAttribute("data-state") ?? "{}");
}

describe("trend section", () => {
  it("charts each trend measure on its own card, plus the derived contributors", () => {
    trendWorld();
    render(<DomainLensView config={TREND_CONFIG} />);

    expect(
      screen.getByRole("button", { name: "Open Commits details" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", {
        name: "Open Active contributors · Commits details",
      }),
    ).toBeInTheDocument();
  });

  it("drills a measure into its own catalog metric", async () => {
    trendWorld();
    render(<DomainLensView config={TREND_CONFIG} />);

    await userEvent.click(
      screen.getByRole("button", { name: "Open Commits details" }),
    );

    const state = openedDrilldown();
    expect(state.metricKey).toBe("t.commits");
    expect(state.members.map((m) => m.name).length).toBe(4);
    expect(state.breakdown.map((b) => b.total)).toEqual([3, 7]);
  });

  it("drills the derived card into periods only, having no records of its own", async () => {
    trendWorld();
    render(<DomainLensView config={TREND_CONFIG} />);

    await userEvent.click(
      screen.getByRole("button", {
        name: "Open Active contributors · Commits details",
      }),
    );

    const state = openedDrilldown();
    expect(state.metricKey).toBeNull();
    // Still counted from the metric it is derived from.
    expect(state.breakdown.map((b) => b.contributors)).toEqual([
      ["a"],
      ["a", "b"],
    ]);
  });
});

describe("dimension-table (rollup: one row per dimension value)", () => {
  const TABLE_CONFIG: LensConfig = {
    title: "T",
    sections: [
      { kind: "headline", metrics: ["t.commits"] },
      {
        kind: "dimension-table",
        title: "Repositories ranked",
        dimension: "repository",
        noun: "repositories",
        limit: 2,
        metrics: ["t.commits", "t.cycle"],
      },
    ],
  };

  function rollup(
    key: string,
    values: Array<[string, string, number, number]>,
    over: Partial<NormalizedMetricResult> = {},
  ): NormalizedMetricResult {
    return {
      ...metric(key, []),
      ...over,
      rollup: {
        view: "rollup",
        dimensions: ["repository"],
        values: values.map(([value, label, v, persons]) => ({
          dimensions: [{ key: "repository", value, label }],
          value: v,
          contributing_entity_count: persons,
        })),
      },
    } as never;
  }

  it("ranks rows by the first metric, joins columns and folds the tail into a remainder", () => {
    const table = emptyCollection();
    table.byKey.set(
      "t.commits",
      rollup("t.commits", [
        ["r1", "org/one", 40, 3],
        ["r2", "org/two", 60, 2],
        ["r3", "org/three", 10, 1],
      ]),
    );
    table.byKey.set(
      "t.cycle",
      rollup(
        "t.cycle",
        [
          ["r1", "org/one", 12, 3],
          ["r2", "org/two", 24, 2],
          ["r3", "org/three", 44, 1],
        ],
        { computation: "median" },
      ),
    );
    mocks.collections = [emptyCollection(), emptyCollection(), emptyCollection(), table];
    render(<DomainLensView config={TABLE_CONFIG} />);

    expect(screen.getByText("Repositories ranked · 3 repositories")).toBeInTheDocument();
    const rows = screen.getAllByRole("row").slice(1); // skip the header
    // Ranked by t.commits: org/two (60) before org/one (40); r3 folds away.
    expect(rows[0]).toHaveTextContent("org/two");
    expect(rows[0]).toHaveTextContent("60");
    expect(rows[0]).toHaveTextContent("24");
    expect(rows[0]).toHaveTextContent("2");
    expect(rows[1]).toHaveTextContent("org/one");
    // The remainder sums the sum metric and stays honest about the median.
    expect(rows[2]).toHaveTextContent("Other (1)");
    expect(rows[2]).toHaveTextContent("10");
    expect(rows[2]).toHaveTextContent("—");
  });

  it("keeps two repositories that share a label as two rows", () => {
    // The dimension VALUE identifies a row; a label is what a reader sees.
    // Two sources whose repositories are both called `org/app` are two
    // repositories, and keying the rows by label would collapse them in
    // React's reconciliation.
    const table = emptyCollection();
    table.byKey.set(
      "t.commits",
      rollup("t.commits", [
        ["src-a:org/app", "org/app", 40, 2],
        ["src-b:org/app", "org/app", 10, 1],
      ]),
    );
    mocks.collections = [emptyCollection(), emptyCollection(), emptyCollection(), table];
    render(<DomainLensView config={TABLE_CONFIG} />);

    const rows = screen.getAllByRole("row").slice(1);
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent("40");
    expect(rows[1]).toHaveTextContent("10");
  });

  it("renders nothing for a single row (empty-shell rule)", () => {
    const table = emptyCollection();
    table.byKey.set("t.commits", rollup("t.commits", [["r1", "org/one", 40, 3]]));
    mocks.collections = [emptyCollection(), emptyCollection(), emptyCollection(), table];
    render(<DomainLensView config={TABLE_CONFIG} />);
    expect(screen.queryByText(/Repositories ranked/)).not.toBeInTheDocument();
  });

  it("opens the tail from the remainder row, and folds it back", async () => {
    const user = userEvent.setup();
    const table = emptyCollection();
    table.byKey.set(
      "t.commits",
      rollup("t.commits", [
        ["r1", "org/one", 40, 3],
        ["r2", "org/two", 60, 2],
        ["r3", "org/three", 10, 1],
      ])
    );
    mocks.collections = [
      emptyCollection(),
      emptyCollection(),
      emptyCollection(),
      table,
    ];
    render(<DomainLensView config={TABLE_CONFIG} />);

    // The limit is 2, so org/three is inside the remainder and nowhere else:
    // without this control the tail is unreachable.
    expect(screen.queryByText("org/three")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Other (1)" }));
    expect(screen.getByText("org/three")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Show top 2" }));
    expect(screen.queryByText("org/three")).not.toBeInTheDocument();
  });

  it("shows a retryable error card when the rollup request fails", () => {
    const table = emptyCollection();
    table.isError = true;
    mocks.collections = [emptyCollection(), emptyCollection(), emptyCollection(), table];
    render(<DomainLensView config={TABLE_CONFIG} />);
    expect(screen.getByText(/unable to load/i)).toBeInTheDocument();
  });
});

describe("ownership (concentration risk per dimension value)", () => {
  const OWNERSHIP_CONFIG: LensConfig = {
    title: "T",
    sections: [
      { kind: "headline", metrics: ["t.commits"] },
      {
        kind: "ownership",
        metric: "t.commits",
        dimension: "repository",
        title: "Ownership concentration",
      },
    ],
  };

  function breakdownRow(id: string, repo: string, value: number) {
    return {
      entity_id: id,
      dimensions: [{ key: "repository", value: repo, label: repo }],
      value,
    };
  }

  it("computes top-1/top-3 shares per value without naming anyone", () => {
    const comp = emptyCollection();
    comp.byKey.set("t.commits", {
      ...metric("t.commits", []),
      breakdown: {
        view: "breakdown",
        values: [
          // alpha: a=70, b=20, c=10 → top-1 70% (flagged), top-3 100%
          breakdownRow(pid("a"), "alpha", 70),
          breakdownRow(pid("b"), "alpha", 20),
          breakdownRow(pid("c"), "alpha", 10),
          // beta: a=30, b=30, c=40, d=20 → top-1 33%, top-3 83%
          breakdownRow(pid("a"), "beta", 30),
          breakdownRow(pid("b"), "beta", 30),
          breakdownRow(pid("c"), "beta", 40),
          breakdownRow(pid("d"), "beta", 20),
        ],
      },
    } as never);
    mocks.collections = [emptyCollection(), comp, emptyCollection(), emptyCollection()];
    render(<DomainLensView config={OWNERSHIP_CONFIG} />);

    expect(screen.getByText("Ownership concentration")).toBeInTheDocument();
    expect(
      screen.getByRole("img", { name: /alpha: top person 70%, top three 100%/ }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("img", { name: /beta: top person 33%, top three 83%/ }),
    ).toBeInTheDocument();
    // No member names anywhere on the section.
    expect(screen.queryByText("a")).not.toBeInTheDocument();
  });

  it("keeps the names out of the row and puts them on the segment", async () => {
    const user = userEvent.setup();
    const comp = emptyCollection();
    comp.byKey.set("t.commits", {
      ...metric("t.commits", []),
      breakdown: {
        view: "breakdown",
        values: [
          breakdownRow(pid("a"), "alpha", 70),
          breakdownRow(pid("b"), "alpha", 20),
          breakdownRow(pid("c"), "alpha", 10),
          breakdownRow(pid("a"), "beta", 30),
          breakdownRow(pid("b"), "beta", 40),
        ],
      },
    } as never);
    mocks.collections = [
      emptyCollection(),
      comp,
      emptyCollection(),
      emptyCollection(),
    ];
    render(<DomainLensView config={OWNERSHIP_CONFIG} />);

    // Nothing on the row names anyone, which is the section's standing rule.
    expect(screen.queryByText(/Top person ·/)).not.toBeInTheDocument();

    // Hovering is the reader asking who, and the answer names the leader of
    // that value — a section that withheld it could not inform the decision
    // it exists for.
    const bar = screen.getByRole("img", { name: /alpha: top person 70%/ });
    await user.hover(bar.firstElementChild as Element);
    expect(await screen.findByText(/Top person · 70% · a/)).toBeInTheDocument();
  });

  it("reaches the rows beyond the first few, and folds them back", async () => {
    const user = userEvent.setup();
    const comp = emptyCollection();
    // One value more than the section shows, so the control is the only way in.
    const repos = Array.from({ length: 13 }, (_, i) => `repo-${i}`);
    comp.byKey.set("t.commits", {
      ...metric("t.commits", []),
      breakdown: {
        view: "breakdown",
        values: repos.flatMap((repo, i) => [
          breakdownRow(pid("a"), repo, 100 - i),
          breakdownRow(pid("b"), repo, 10),
        ]),
      },
    } as never);
    mocks.collections = [
      emptyCollection(),
      comp,
      emptyCollection(),
      emptyCollection(),
    ];
    render(<DomainLensView config={OWNERSHIP_CONFIG} />);

    const hidden = repos[repos.length - 1]!;
    expect(screen.queryByText(hidden)).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^\+\d+ more$/ }));
    expect(screen.getByText(hidden)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^Show top \d+$/ }));
    expect(screen.queryByText(hidden)).not.toBeInTheDocument();
  });

  it("renders nothing with fewer than two values", () => {
    const comp = emptyCollection();
    comp.byKey.set("t.commits", {
      ...metric("t.commits", []),
      breakdown: {
        view: "breakdown",
        values: [breakdownRow(pid("a"), "alpha", 70)],
      },
    } as never);
    mocks.collections = [emptyCollection(), comp, emptyCollection(), emptyCollection()];
    render(<DomainLensView config={OWNERSHIP_CONFIG} />);
    expect(screen.queryByText("Ownership concentration")).not.toBeInTheDocument();
  });
});

describe("descending into one dimension value", () => {
  const SCOPED_CONFIG: LensConfig = {
    title: "Dev · Repositories",
    tagline: "activity, reach & risk",
    sections: [
      { kind: "headline", metrics: ["t.commits"] },
      {
        kind: "dimension-table",
        title: "Repositories ranked",
        dimension: "repository",
        noun: "repositories",
        limit: 2,
        metrics: ["t.commits"],
      },
    ],
    drilldown: {
      dimension: "repository",
      tagline: "one repository",
      sections: [
        { kind: "headline", metrics: ["t.commits"] },
        {
          kind: "contributors",
          metric: "t.commits",
          title: "Top contributors",
        },
        {
          kind: "heatmap-hours",
          metric: "t.commits",
          title: "When commits land",
          caption: "Weekday × two-hour block, in UTC.",
        },
      ],
    },
  };

  function rollupOf(values: Array<[string, string, number, number]>) {
    const table = emptyCollection();
    table.byKey.set("t.commits", {
      ...metric("t.commits", []),
      rollup: {
        view: "rollup",
        dimensions: ["repository"],
        values: values.map(([value, label, v, persons]) => ({
          dimensions: [{ key: "repository", value, label }],
          value: v,
          contributing_entity_count: persons,
        })),
      },
    } as never);
    return table;
  }

  it("opens the value from a table row and comes back through the breadcrumb", async () => {
    const user = userEvent.setup();
    mocks.collections = [
      emptyCollection(),
      emptyCollection(),
      emptyCollection(),
      rollupOf([
        ["src:acme/api", "acme/api", 60, 3],
        ["src:acme/web", "acme/web", 40, 2],
      ]),
      emptyCollection(),
    ];
    render(<DomainLensView config={SCOPED_CONFIG} />);

    await user.click(screen.getByRole("button", { name: "acme/api" }));
    expect(portalRouter.search.repo).toBe("src:acme/api");

    // The URL is what makes a shared link reproduce this screen, so the id and
    // not the label is what it carries — the heading shows the label.
    expect(
      screen.getByRole("heading", { level: 1, name: "acme/api" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("Repositories ranked · 2 repositories"),
    ).not.toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "Dev · Repositories" }),
    );
    expect(portalRouter.search.repo).toBeUndefined();
  });

  it("filters every request it makes to the value under inspection", () => {
    act(() => portalRouter.set({ repo: "src:acme/api" }));
    render(<DomainLensView config={SCOPED_CONFIG} />);

    const asked = mocks.requested.flatMap((c) => c.metrics);
    expect(asked.length).toBeGreaterThan(0);
    // Not one request may answer about the whole tenant: a section that
    // forgot the filter would read as this repository carrying everyone's work.
    for (const metric of asked) {
      expect(metric.filters, metric.key).toEqual([
        { dimension: "repository", values: ["src:acme/api"] },
      ]);
    }
  });

  it("opens an hour block from its column, and leaves the cells inert", async () => {
    const user = userEvent.setup();
    const openEvidenceTargets = vi.fn();
    const hours = emptyCollection();
    hours.byKey.set("t.commits", {
      ...metric("t.commits", []),
      timeseries: {
        view: "timeseries",
        bucket: "day",
        dimensions: ["hour_block"],
        series: IDS.map((id) => ({
          entity_id: id,
          dimensions: [{ key: "hour_block", value: "08", label: "08–10" }],
          points: [{ bucket_start: "2026-10-05", value: 3 }],
        })),
      },
    } as never);
    mocks.collections = [
      emptyCollection(),
      emptyCollection(),
      emptyCollection(),
      emptyCollection(),
      hours,
    ];
    mocks.grid.byKey = new Map([
      [
        "t.commits",
        metric("t.commits", IDS.map((id) => [id, 10] as [string, number]), {
          label: "Commits",
          drilldown: { granularity: ["event"] },
          selection: {
            metric_key: "t.commits",
            entity: { type: "person", ids: IDS },
            period: { from: "2026-07-20", to: "2026-07-26" },
            filters: [],
          },
        } as Partial<NormalizedMetricResult>),
      ],
    ]);
    act(() => portalRouter.set({ repo: "src:acme/api" }));

    render(
      <EvidenceDialogContext.Provider
        value={{
          openEvidence: vi.fn(),
          openEvidenceTargets,
          openEvidencePeople: vi.fn(),
        }}
      >
        <DomainLensView config={SCOPED_CONFIG} />
      </EvidenceDialogContext.Provider>,
    );

    // Every block is openable, not only the labelled ones: the density choice
    // must not take six of twelve drilldowns with it.
    expect(
      screen.getAllByRole("button", { name: /Open the records from the/ }),
    ).toHaveLength(12);

    await user.click(
      screen.getByRole("button", {
        name: "Open the records from the 08:00 block",
      }),
    );

    const [targets] = openEvidenceTargets.mock.calls[0]!;
    // The block across the period, and still inside the repository. A CELL is
    // that block on ONE weekday, which the request has no predicate for — so
    // the cells stay inert rather than answering a wider question.
    expect(targets[0].selection.filters).toEqual([
      { dimension: "repository", values: ["src:acme/api"] },
      { dimension: "hour_block", values: ["08"] },
    ]);
    expect(screen.getByTitle(/^Mon 08:00/).tagName).toBe("DIV");
  });

  it("ignores a value the lens has no screen for", () => {
    act(() => portalRouter.set({ repo: "src:acme/api" }));
    render(
      <DomainLensView
        config={{
          title: "Dev · Test",
          sections: [{ kind: "headline", metrics: ["t.commits"] }],
        }}
      />,
    );

    // A `repo` left over from another lens must not turn this one into a
    // screen about a value it does not group by.
    expect(
      screen.getByRole("heading", { level: 1, name: "Dev · Test" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("navigation")).not.toBeInTheDocument();
    for (const metric of mocks.requested.flatMap((c) => c.metrics)) {
      expect(metric.filters, metric.key).toBeUndefined();
    }
  });

  it("draws the heatmap from the hour blocks the metric reported", () => {
    const hours = emptyCollection();
    hours.byKey.set("t.commits", {
      ...metric("t.commits", []),
      timeseries: {
        view: "timeseries",
        bucket: "day",
        dimensions: ["hour_block"],
        series: IDS.map((id) => ({
          entity_id: id,
          dimensions: [{ key: "hour_block", value: "08", label: "08–10" }],
          // 2026-10-05 is a Monday.
          points: [{ bucket_start: "2026-10-05", value: 3 }],
        })),
      },
    } as never);
    mocks.collections = [
      emptyCollection(),
      emptyCollection(),
      emptyCollection(),
      emptyCollection(),
      hours,
    ];
    act(() => portalRouter.set({ repo: "src:acme/api" }));
    render(<DomainLensView config={SCOPED_CONFIG} />);

    // 4 members × 3 = 12, all in Monday's 08 block.
    expect(screen.getByText(/When commits land · 12/)).toBeInTheDocument();
    expect(screen.getByTitle(/^Mon 08:00 · 12/)).toBeInTheDocument();
    // A block nobody committed in stays empty rather than borrowing a number.
    expect(screen.getByTitle(/^Tue 08:00 · 0/)).toBeInTheDocument();
  });
});
