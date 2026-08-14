import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { EvidenceDialogContext } from "@/components/metric-evidence-context";
import { MetricHistogram } from "@/components/widgets/metric-views/metric-histogram";
import {
  normalizeMetricResults,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import type { MetricResult } from "@/api/metric-results-client";

function medianMetric(
  bins: Array<{ lo: number; hi: number; count: number }>,
  opts: { ownMedian?: number | null; peerMedian?: number | null } = {},
): NormalizedMetricResult {
  const { ownMedian = 20, peerMedian = 24 } = opts;
  const result: MetricResult = {
    metric_key: "git.pr_cycle_time_h",
    label: "PR cycle time",
    unit: "h",
    format: "decimal",
    direction: "lower_is_better",
    computation: "median",
    views: [
      { view: "period", values: [{ entity_id: "me@x.com", value: ownMedian }] },
      {
        view: "peer",
        values: [
          {
            entity_id: "me@x.com",
            target_value: ownMedian,
            p25: null,
            median: peerMedian,
            p75: null,
            min: null,
            max: null,
            n: 10,
          },
        ],
      },
      { view: "histogram", values: [{ entity_id: "me@x.com", bins }] },
    ],
  };
  return normalizeMetricResults([result]).get("git.pr_cycle_time_h")!;
}

const BINS = [
  { lo: 0, hi: 10, count: 3 },
  { lo: 10, hi: 20, count: 5 },
  { lo: 20, hi: 30, count: 2 },
  { lo: 30, hi: 40, count: 1 },
];

describe("MetricHistogram", () => {
  it("renders the label (no 'distribution' word) and direction", () => {
    render(<MetricHistogram metric={medianMetric(BINS)} entityId="me@x.com" />);
    expect(screen.getByText("PR cycle time")).toBeInTheDocument();
    expect(
      screen.queryByText("PR cycle time distribution"),
    ).not.toBeInTheDocument();
    expect(screen.getByText(/Lower is better/)).toBeInTheDocument();
  });

  it("names the peer median it colors against", () => {
    render(<MetricHistogram metric={medianMetric(BINS)} entityId="me@x.com" />);
    expect(screen.getByText(/vs peer median/)).toBeInTheDocument();
  });

  it("drops the peer comparison when there is no peer median", () => {
    render(
      <MetricHistogram
        metric={medianMetric(BINS, { peerMedian: null })}
        entityId="me@x.com"
      />,
    );
    expect(screen.queryByText("Better than peers")).not.toBeInTheDocument();
    expect(screen.queryByText(/vs peer median/)).not.toBeInTheDocument();
  });

  it("shows the empty state for an entity with no bins", () => {
    render(<MetricHistogram metric={medianMetric([])} entityId="me@x.com" />);
    expect(screen.getByText("No values in this period")).toBeInTheDocument();
  });

  it("shows the empty state for an entity absent from the view", () => {
    render(<MetricHistogram metric={medianMetric(BINS)} entityId="nobody@x.com" />);
    expect(screen.getByText("No values in this period")).toBeInTheDocument();
  });

  describe("supporting data", () => {
    function drillable(
      metric: NormalizedMetricResult
    ): NormalizedMetricResult {
      return {
        ...metric,
        drilldown: { granularity: ["event"] },
        selection: {
          metric_key: "git.pr_cycle_time_h",
          entity: { type: "person", ids: ["me@x.com"] },
          period: { from: "2026-07-01", to: "2026-07-31" },
          filters: [],
        },
      };
    }

    function renderWithEvidence(metric: NormalizedMetricResult) {
      const openEvidence = vi.fn();
      render(
        <EvidenceDialogContext.Provider
          value={{ openEvidence, openEvidenceTargets: vi.fn() }}
        >
          <MetricHistogram metric={metric} entityId="me@x.com" />
        </EvidenceDialogContext.Provider>
      );
      return openEvidence;
    }

    it("opens the records behind the distribution", async () => {
      const user = userEvent.setup();
      const openEvidence = renderWithEvidence(drillable(medianMetric(BINS)));

      await user.click(
        screen.getByRole("button", { name: "More actions for PR cycle time" })
      );
      await user.click(
        await screen.findByRole("menuitem", { name: "View supporting data" })
      );
      expect(openEvidence).toHaveBeenCalledWith(
        expect.objectContaining({ metric_key: "git.pr_cycle_time_h" }),
        "PR cycle time"
      );
    });

    it("offers them on an empty period too, since the metric is still drillable", () => {
      renderWithEvidence(drillable(medianMetric([])));
      expect(
        screen.getByRole("button", { name: "More actions for PR cycle time" })
      ).toBeInTheDocument();
    });

    it("offers nothing for a metric that declares no drilldown", () => {
      renderWithEvidence(medianMetric(BINS));
      expect(
        screen.queryByRole("button", { name: /More actions/ })
      ).not.toBeInTheDocument();
    });
  });
});
