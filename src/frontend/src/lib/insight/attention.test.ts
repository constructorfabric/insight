import { describe, expect, it } from "vitest";

import type { MetricResult } from "@/api/metric-results-client";
import { metricAttentionItems } from "@/lib/insight/attention";
import type { MetricGroup } from "@/lib/insight/groups";
import { normalizeMetricResults } from "@/lib/metrics/collection";

function aiMetric(value: number | null): MetricResult {
  return {
    metric_key: "ai.active_days",
    label: "Active AI days",
    unit: "days",
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
            median: 11,
            p75: 15,
            min: 0,
            max: 30,
            n: 9,
          },
        ],
      },
    ],
  };
}

const AI_DEF: MetricGroup = {
  id: "ai_adoption",
  title: "AI adoption",
  collection: {
    metrics: [
      { key: "ai.active_days", views: [{ view: "period" }, { view: "peer" }] },
    ],
  },
  card: { preview: [] },
  drilldown: [],
};

describe("metricAttentionItems", () => {
  it("surfaces bottom-quartile metrics with the same item shape", () => {
    const byKey = normalizeMetricResults([aiMetric(2)]);
    const items = metricAttentionItems(AI_DEF, byKey, "me@x.com");
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({
      group: "ai_adoption",
      label: "Active AI days",
      valueText: "2 days",
      medianText: "11 days",
      gapText: "-82%",
    });
  });

  it("never flags unmeasured people (null peer target_value)", () => {
    const unmeasured = aiMetric(0);
    const peerView = unmeasured.views[1];
    if (peerView?.view === "peer" && peerView.values[0]) {
      peerView.values[0].target_value = null;
    }
    expect(
      metricAttentionItems(
        AI_DEF,
        normalizeMetricResults([unmeasured]),
        "me@x.com"
      )
    ).toHaveLength(0);
  });

  it("ignores in-pack values and missing data", () => {
    expect(
      metricAttentionItems(
        AI_DEF,
        normalizeMetricResults([aiMetric(10)]),
        "me@x.com"
      )
    ).toHaveLength(0);
    expect(
      metricAttentionItems(
        AI_DEF,
        normalizeMetricResults([aiMetric(null)]),
        "me@x.com"
      )
    ).toHaveLength(0);
  });
});
