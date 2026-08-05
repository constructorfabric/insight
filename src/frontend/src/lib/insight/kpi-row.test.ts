import { describe, expect, it } from "vitest";

import type { MetricResult } from "@/api/metric-results-client";
import { metricKpiTiles } from "@/lib/insight/kpi-row";
import { normalizeMetricResults } from "@/lib/metrics/collection";

function metricResult(
  key: string,
  value: number | null,
  overrides: Partial<MetricResult> = {}
): MetricResult {
  return {
    metric_key: key,
    label: key,
    unit: null,
    format: "integer",
    direction: "higher_is_better",
    computation: "sum",
    views: [
      { view: "period", values: [{ entity_id: "me@x.com", value }] },
      {
        view: "peer",
        values: [
          {
            entity_id: "me@x.com",
            target_value: value,
            p25: 5,
            median: 10,
            p75: 15,
            min: 0,
            max: 30,
            n: 8,
          },
        ],
      },
    ],
    ...overrides,
  } as MetricResult;
}

describe("metricKpiTiles", () => {
  it("builds display-ready tiles with rank status and delta", () => {
    const byKey = normalizeMetricResults([
      metricResult("ai.active_days", 14),
      metricResult("ai.accepted_lines", 900),
    ]);
    const previous = normalizeMetricResults([
      metricResult("ai.active_days", 12),
      metricResult("ai.accepted_lines", 1000),
    ]);
    const tiles = metricKpiTiles(byKey, previous, "me@x.com", "all");
    expect(tiles.map((t) => t.key)).toEqual([
      "ai.active_days",
      "ai.accepted_lines",
    ]);
    const active = tiles[0]!;
    expect(active.value).toBe("14");
    // 14 sits inside the IQR (5..15) — with the pack, so no color even
    // though it's above the median.
    expect(active.valueStatus).toBe("neutral");
    expect(active.delta?.text).toBe("+17%");
    expect(active.medianLabel).toBe("median 10");
    expect(active.groupId).toBe("ai_adoption");
    const lines = tiles[1]!;
    // 900 ≥ p75 15 — top quartile earns the color.
    expect(lines.valueStatus).toBe("good");
    expect(lines.delta).toEqual({ text: "-10%", status: "bad", down: true });
  });

  it("renders neutral when the backend suppressed the peer stats", () => {
    // Below the disclosure floor the server returns null percentiles with
    // the true n; the tile must not score or show a median off them.
    const result = metricResult("ai.active_days", 14);
    const peerView = result.views[1];
    if (peerView?.view === "peer" && peerView.values[0]) {
      Object.assign(peerView.values[0], {
        p25: null,
        median: null,
        p75: null,
        min: null,
        max: null,
        n: 3,
      });
    }
    const tiles = metricKpiTiles(
      normalizeMetricResults([result]),
      null,
      "me@x.com",
      "all"
    );
    expect(tiles[0]?.valueStatus).toBe("neutral");
    expect(tiles[0]?.medianLabel).toBeNull();
  });

  it("renders neutral for unmeasured people (null peer target_value)", () => {
    const result = metricResult("ai.active_days", 0);
    const peerView = result.views[1];
    if (peerView?.view === "peer" && peerView.values[0]) {
      peerView.values[0].target_value = null;
    }
    const tiles = metricKpiTiles(
      normalizeMetricResults([result]),
      null,
      "me@x.com",
      "all"
    );
    expect(tiles[0]?.valueStatus).toBe("neutral");
  });

  it("computes pp deltas for percent-ratio tiles (focus time)", () => {
    const current = metricResult("collab.focus_time_pct", 77, {
      metric_key: "collab.focus_time_pct",
      format: "percent",
      computation: "ratio",
      scale: 100,
    } as Partial<MetricResult>);
    const previous = metricResult("collab.focus_time_pct", 72, {
      format: "percent",
      computation: "ratio",
      scale: 100,
    } as Partial<MetricResult>);
    const tiles = metricKpiTiles(
      normalizeMetricResults([current]),
      normalizeMetricResults([previous]),
      "me@x.com",
      "all"
    );
    expect(tiles[0]?.delta?.text).toBe("+5 pp");
  });
});
