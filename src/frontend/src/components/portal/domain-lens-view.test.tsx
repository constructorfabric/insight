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
// DomainLensView calls useMetricCollection three times (trend, composition,
// event-histogram) in that order on every render. The counter is reset per test
// (see beforeEach): left running, each caller's slot would depend on how many
// renders every earlier test happened to do.
vi.mock("@/queries/metric-results", () => ({
  useMetricCollection: () => {
    const r = mocks.collections[mocks.call % 3] ?? emptyCollection();
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
  seedHappyOrg();
  mocks.grid.isPending = false;
  mocks.grid.isError = false;
  act(() => {
    portalRouter.set({ slice: undefined });
    portalRouter.set({ scope: undefined, direct: false });
  });
});

afterEach(() => vi.clearAllMocks());

/* ── tests ───────────────────────────────────────────────────────────── */

describe("headline (rules 1–2: per-capita + PoP delta)", () => {
  it("shows the per-active-person value, the team total and the delta", () => {
    render(<DomainLensView config={HEADLINE_CONFIG} />);
    // 100 commits over 4 active people = 25/person; halved from last period.
    expect(screen.getByText("25 commits")).toBeInTheDocument();
    expect(screen.getByText(/100 commits team total/)).toBeInTheDocument();
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
    // 100 total / 2 active = 50, not 25
    expect(screen.getByText("50 commits")).toBeInTheDocument();
  });
});

describe("headline cards open the records behind them", () => {
  const openEvidenceTargets = vi.fn();
  const openEvidence = vi.fn();

  function drawWithEvidence(config: LensConfig = HEADLINE_CONFIG) {
    return render(
      <EvidenceDialogContext.Provider
        value={{ openEvidence, openEvidenceTargets }}
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
  });
});
