import { describe, expect, it } from "vitest";

import {
  memberMetricEntries,
  memberMetricStanding,
  metricBelowCounts,
  teamMetricStandings,
} from "@/lib/insight/team-metrics";
import {
  GROUPS,
  type MetricGroup,
} from "@/lib/insight/groups";
import {
  buildMetricCollectionRequest,
  entityChunkSize,
  normalizeMetricResults,
  projectViews,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import type { MetricResult } from "@/api/metric-results-client";

function metricWithPeers(
  values: Array<{
    id: string;
    value: number | null;
    suppressed?: boolean;
    unmeasured?: boolean;
  }>,
): MetricResult {
  return {
    metric_key: "ai.active_days",
    label: "Active AI days",
    unit: "days",
    format: "integer",
    direction: "higher_is_better",
    computation: "sum",
    views: [
      {
        view: "period",
        values: values.map((v) => ({ entity_id: v.id, value: v.value })),
      },
      {
        view: "peer",
        // Thin-cohort suppression happens server-side: the backend returns
        // null percentiles (with the true `n`) below its disclosure floor.
        values: values.map((v) => ({
          entity_id: v.id,
          target_value: v.unmeasured ? null : v.value,
          p25: v.suppressed ? null : 5,
          median: v.suppressed ? null : 10,
          p75: v.suppressed ? null : 15,
          min: v.suppressed ? null : 0,
          max: v.suppressed ? null : 30,
          n: v.suppressed ? 3 : 12,
        })),
      },
    ],
  };
}

function defWith(metric: NormalizedMetricResult): MetricGroup {
  return {
    id: "ai_adoption",
    title: "AI adoption",
    collection: {
      metrics: [{ key: metric.metric_key, views: [{ view: "period" }, { view: "peer" }] }],
    },
    card: { preview: [metric.metric_key] },
    drilldown: [],
  };
}

describe("memberMetricStanding", () => {
  const byKey = normalizeMetricResults([
    metricWithPeers([
      { id: "top@x.com", value: 20 },
      { id: "mid@x.com", value: 10 },
      { id: "low@x.com", value: 2 },
      { id: "thin@x.com", value: 2, suppressed: true },
      { id: "unmeasured@x.com", value: 0, unmeasured: true },
      { id: "empty@x.com", value: null },
    ]),
  ]);
  const metric = byKey.get("ai.active_days")!;

  it("scores members against their own cohort quartiles", () => {
    expect(memberMetricStanding(metric, "top@x.com")).toBe("top");
    expect(memberMetricStanding(metric, "mid@x.com")).toBe("in_pack");
    expect(memberMetricStanding(metric, "low@x.com")).toBe("bottom");
  });

  it("collapses server-suppressed percentiles to unscored", () => {
    // The backend nulls percentiles below its disclosure floor; the selector
    // must treat those rows as unscorable, exactly like the legacy cells did
    // with their client-side n gate.
    expect(memberMetricStanding(metric, "thin@x.com")).toBeNull();
  });

  it("skips members without a value", () => {
    expect(memberMetricStanding(metric, "empty@x.com")).toBeNull();
  });

  it("skips unmeasured members (zero-filled value, null target_value)", () => {
    expect(memberMetricStanding(metric, "unmeasured@x.com")).toBeNull();
  });
});

describe("teamMetricStandings / metricBelowCounts", () => {
  const byKey = normalizeMetricResults([
    metricWithPeers([
      { id: "a@x.com", value: 2 },
      { id: "b@x.com", value: 3 },
      { id: "c@x.com", value: 20 },
    ]),
  ]);
  const def = defWith(byKey.get("ai.active_days")!);
  const members = ["a@x.com", "b@x.com", "c@x.com"];

  it("rolls up by plurality: more bottom than top → bottom", () => {
    const standings = teamMetricStandings(def, byKey, members);
    expect(standings).toHaveLength(1);
    expect(standings[0]?.verdict).toBe("bottom");
    expect(standings[0]?.bottom).toBe(2);
    expect(standings[0]?.top).toBe(1);
  });

  it("counts bottom standings per member", () => {
    const counts = metricBelowCounts(def, byKey, members);
    expect(counts.get("a@x.com")).toBe(1);
    expect(counts.get("c@x.com")).toBeUndefined();
  });
});

describe("memberMetricEntries", () => {
  const byKey = normalizeMetricResults([
    metricWithPeers([
      { id: "a@x.com", value: 2 },
      { id: "b@x.com", value: 20 },
      { id: "empty@x.com", value: null },
    ]),
  ]);
  const def = defWith(byKey.get("ai.active_days")!);

  it("builds per-person entries keyed by member id, own-cohort status included", () => {
    const out = memberMetricEntries(
      [def],
      (id) => (id === def.id ? byKey : undefined),
      ["a@x.com", "b@x.com", "empty@x.com"],
    );
    expect(out.get("a@x.com")?.map((e) => [e.key, e.status])).toEqual([
      ["ai.active_days", "bottom"],
    ]);
    expect(out.get("b@x.com")?.map((e) => [e.key, e.status])).toEqual([
      ["ai.active_days", "top"],
    ]);
    // No period value for this person → no entry, no map key.
    expect(out.has("empty@x.com")).toBe(false);
  });

  it("contributes nothing for groups whose data has not resolved", () => {
    const out = memberMetricEntries([def], () => undefined, ["a@x.com"]);
    expect(out.size).toBe(0);
  });
});

describe("team request row-limit projection", () => {
  it("chunks each metrics group so no request exceeds the backend row limit", () => {
    // Backend caps projected rows at 5000 per request and rejects the WHOLE
    // request beyond it. Projection: period/peer → one row per entity per
    // metric-view. The team surface requests period+peer only and
    // `useMetricCollectionSet` chunks the roster by `entityChunkSize`, so a
    // group larger than the unchunked budget (e.g. collaboration's 19 metrics)
    // is split across requests rather than rejected.
    const ROW_LIMIT = 5000;
    const roster = 200;
    for (const def of GROUPS) {
      const projected = projectViews(def.collection, ["period", "peer"]);
      // Roster surfaces carry no per-bucket / per-dimension views.
      expect(
        projected.metrics.flatMap((m) => m.views).some(
          (v) =>
            v.view === "timeseries" ||
            v.view === "breakdown" ||
            v.view === "histogram",
        ),
      ).toBe(false);
      const chunkSize = entityChunkSize(projected) ?? roster;
      const chunkIds = Array.from(
        { length: Math.min(chunkSize, roster) },
        (_, i) => `person${i}@x.com`,
      );
      const request = buildMetricCollectionRequest(
        projected,
        { type: "person", ids: chunkIds },
        { from: "2026-06-01", to: "2026-06-30" },
      );
      const projectedRows = request.metrics.reduce(
        (sum, metric) => sum + metric.views.length * chunkIds.length,
        0,
      );
      expect(projectedRows).toBeLessThanOrEqual(ROW_LIMIT);
    }
  });
});
