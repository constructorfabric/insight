// @vitest-environment jsdom
/**
 * The standing each section carries in the nav. It is the only thing on the
 * person page that answers "which section is worth opening", so its three
 * outcomes have to be distinguishable: a section behind its cohort, a section
 * with nothing this period, and a section still loading — the last two look
 * identical if the hook reports them the same way.
 */
import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { MetricResult } from "@/api/metric-results-client";
import { GROUPS } from "@/lib/insight/groups";
import { normalizeMetricResults } from "@/lib/metrics/collection";

const mocks = vi.hoisted(() => ({
  definitions: [] as unknown[],
  definitionsPending: false,
  byKey: new Map<string, unknown>(),
  isPending: false,
  cohort: [] as string[],
}));

vi.mock("@/queries/metric-definitions", () => ({
  useMetricDefinitionsResponse: () => ({
    data: { metrics: mocks.definitions },
    isPending: mocks.definitionsPending,
  }),
}));
vi.mock("@/queries/metric-results", () => ({
  useMetricCollectionSet: () =>
    new Map(
      [
        "task_delivery",
        "git_output",
        "collaboration",
        "ai_adoption",
        "wiki",
      ].map((id) => [
        id,
        {
          byKey: mocks.byKey,
          previousByKey: null,
          isPending: mocks.isPending,
          isFetching: false,
          isError: false,
          refetch: vi.fn(),
        },
      ])
    ),
}));
vi.mock("@/lib/portal/use-person-cohort", () => ({
  usePersonCohort: () => mocks.cohort,
}));
vi.mock("@/hooks/use-portal-period", () => ({
  usePortalPeriod: () => ({
    period: "month",
    dateRange: { from: "2026-07-01", to: "2026-07-31" },
  }),
}));

import { usePersonSectionStandings } from "./use-person-sections";

const ME = "019e27bc-dec0-7626-81a9-c5524662a6a9";

/** One metric with a peer row, so it has a standing to aggregate. */
function metric(
  key: string,
  value: number | null,
  median: number,
  n = 9
): MetricResult {
  return {
    metric_key: key,
    label: key,
    unit: null,
    format: "integer",
    computation: "sum",
    direction: "higher_is_better",
    views: [
      { view: "period", values: [{ entity_id: ME, value }] },
      {
        view: "peer",
        values: [
          {
            entity_id: ME,
            target_value: value,
            p25: median - 2,
            median,
            p75: median + 2,
            min: 0,
            max: median + 5,
            n,
          },
        ],
      },
    ],
  } as MetricResult;
}

function standings() {
  return renderHook(() => usePersonSectionStandings(ME)).result.current;
}

beforeEach(() => {
  mocks.byKey = new Map();
  mocks.isPending = false;
  mocks.cohort = [];
});

describe("usePersonSectionStandings", () => {
  it("marks a section whose metric sits below its cohort", () => {
    mocks.byKey = normalizeMetricResults([metric("git.commits", 1, 20)]);
    const git = standings().find((s) => s.id === "git_output")!;
    expect(git.hasData).toBe(true);
    expect(git.status).toBe("warn");
    expect(git.phrase).toContain("behind peers");
  });

  it("counts a measured zero as data — it is a finding, not a gap", () => {
    mocks.byKey = normalizeMetricResults([metric("git.commits", 0, 20)]);
    expect(standings().find((s) => s.id === "git_output")!.hasData).toBe(true);
  });

  it("reports no data when nothing in the section has a value", () => {
    mocks.byKey = normalizeMetricResults([metric("git.commits", null, 20)]);
    const git = standings().find((s) => s.id === "git_output")!;
    expect(git.hasData).toBe(false);
    expect(git.phrase).toBe("no comparison");
  });

  it("separates a section this person is absent from one nobody is measured in", () => {
    // Both arrive as a null own value, and the page says different things
    // about them. Which one is true is read from the tenant's definition
    // listing, never from whether this viewer's comparison pool happens to
    // hold readings: a viewer who can see few people would otherwise report a
    // live connector as missing, and the smaller their reach the more often.
    mocks.byKey = normalizeMetricResults([metric("git.commits", null, 20)]);

    mocks.definitions = [
      {
        metric_key: "git.commits",
        is_enabled: true,
        schema_status: "ok",
        schema_error_code: null,
        last_observed_date: "2026-07-26",
      },
    ];
    expect(standings().find((s) => s.id === "git_output")!.peersHaveData).toBe(
      true
    );

    // Same person, same empty own value — only the listing changed.
    mocks.definitions = [
      {
        metric_key: "git.commits",
        is_enabled: true,
        schema_status: "ok",
        schema_error_code: null,
        last_observed_date: null,
      },
    ];
    expect(standings().find((s) => s.id === "git_output")!.peersHaveData).toBe(
      false
    );
  });

  it("does not call a section unmeasured because this viewer's pool is empty", () => {
    // The regression this replaced: an empty pool used to mean "no data
    // reaches us". It now means nothing at all — the listing decides.
    mocks.byKey = normalizeMetricResults([metric("git.commits", null, 20, 0)]);
    mocks.definitions = [
      {
        metric_key: "git.commits",
        is_enabled: true,
        schema_status: "ok",
        schema_error_code: null,
        last_observed_date: "2026-07-26",
      },
    ];
    expect(standings().find((s) => s.id === "git_output")!.peersHaveData).toBe(
      true
    );
  });

  it("stays pending while the queries are, so the nav shows no mark yet", () => {
    // A section still loading must not be drawn as one with nothing: the
    // reader would take the grey mark for an answer.
    mocks.isPending = true;
    expect(standings().every((s) => s.isPending)).toBe(true);
  });

  it("covers every section, including ones the response never mentions", () => {
    // The nav draws one mark per section, so a section missing from this list
    // would silently lose its mark — asserting the whole set, in order, is
    // what catches that.
    expect(standings().map((s) => s.id)).toEqual(GROUPS.map((g) => g.id));
  });
});

describe("while the definition listing is still loading", () => {
  it("claims nothing about any section", () => {
    // Without the listing the reachable set is empty, so every section would
    // read as one nothing reaches — the page would tell a reader we see
    // nothing about this person, then flip a moment later. Pending has to
    // cover the listing too, not just the metric collection.
    mocks.definitionsPending = true;
    mocks.byKey = normalizeMetricResults([metric("git.commits", null, 20)]);
    expect(standings().every((s) => s.isPending)).toBe(true);
    mocks.definitionsPending = false;
  });
});
