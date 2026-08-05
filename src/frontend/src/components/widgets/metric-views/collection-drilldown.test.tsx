import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { CollectionDrilldown } from "@/components/widgets/metric-views/collection-drilldown";
import type { MetricGroup } from "@/lib/insight/groups";
import { normalizeMetricResults } from "@/lib/metrics/collection";
import type { MetricCollectionResult } from "@/queries/metric-results";
import {
  MEDIAN_METRIC_FIXTURE,
  RATIO_METRIC_FIXTURE,
  SUM_METRIC_FIXTURE,
} from "@/mocks/metric-results-fixtures";

vi.mock("@/hooks/use-settings", () => ({
  useSettings: () => ({ focusMode: "all", showExplanations: true }),
}));

const DEF: MetricGroup = {
  id: "ai_adoption",
  title: "AI adoption",
  collection: {
    metrics: [
      {
        key: "ai.accepted_lines",
        views: [
          { view: "period" },
          { view: "peer" },
          { view: "timeseries", bucket: "auto", dimensions: ["tool"] },
          { view: "breakdown", dimensions: ["tool"] },
        ],
      },
      {
        key: "ai.tool_acceptance_rate",
        views: [{ view: "period" }, { view: "peer" }],
      },
    ],
  },
  card: { preview: ["ai.accepted_lines"] },
  drilldown: [
    { chart: "bars", view: "breakdown", metrics: ["ai.accepted_lines"] },
    {
      id: "accepted-lines-by-tool",
      view: "timeseries",
      metrics: ["ai.accepted_lines"],
      groupBy: { default: "tool" },
    },
  ],
};

function result(
  overrides: Partial<MetricCollectionResult> = {}
): MetricCollectionResult {
  return {
    byKey: normalizeMetricResults([SUM_METRIC_FIXTURE, RATIO_METRIC_FIXTURE]),
    previousByKey: null,
    isPending: false,
    isFetching: false,
    isError: false,
    refetch: vi.fn(),
    ...overrides,
  };
}

describe("CollectionDrilldown", () => {
  it("renders the def's blocks and the peer story from wire data", () => {
    render(
      <CollectionDrilldown
        def={DEF}
        data={result()}
        entityId="alice@example.com"
      />
    );
    // Breakdown block: composition by tool with response-provided labels.
    expect(screen.getByText("Period total by tool")).toBeInTheDocument();
    expect(screen.getAllByText("Claude Code").length).toBeGreaterThan(0);
    // Peer story: both fixture metrics are in-pack (no outlier hero), so the
    // story falls back to the flat grid with one card per metric.
    expect(screen.getByText("Tool acceptance rate")).toBeInTheDocument();
    expect(screen.getByText("77%")).toBeInTheDocument();
  });

  it("dispatches a histogram block to the histogram renderer", () => {
    const histogramDef: MetricGroup = {
      id: "git_output",
      title: "Git output",
      collection: {
        metrics: [
          {
            key: "git.pr_cycle_time_h",
            views: [
              { view: "period" },
              { view: "peer" },
              { view: "histogram" },
            ],
          },
        ],
      },
      card: { preview: ["git.pr_cycle_time_h"] },
      drilldown: [
        {
          chart: "histogram",
          view: "histogram",
          metrics: ["git.pr_cycle_time_h"],
        },
      ],
    };
    render(
      <CollectionDrilldown
        def={histogramDef}
        data={result({
          byKey: normalizeMetricResults([MEDIAN_METRIC_FIXTURE]),
        })}
        entityId="alice@example.com"
      />
    );
    // Each histogram is its own card in the grid (no wrapping "Distributions"
    // card), shaded vs the peer median (the subtitle proves the diverging
    // render, not a bare chart).
    expect(screen.getByText(/vs peer median/)).toBeInTheDocument();
  });

  it("renders a summary-card block as headline cards", () => {
    const summaryDef: MetricGroup = {
      id: "collaboration",
      title: "Collaboration",
      collection: {
        metrics: [
          {
            key: "ai.accepted_lines",
            views: [
              { view: "period" },
              { view: "peer" },
              { view: "breakdown", dimensions: ["tool"] },
            ],
          },
        ],
      },
      card: { preview: ["ai.accepted_lines"] },
      drilldown: [
        {
          chart: "summary-card",
          view: "breakdown",
          metrics: ["ai.accepted_lines"],
        },
      ],
    };
    render(
      <CollectionDrilldown
        def={summaryDef}
        data={result()}
        entityId="alice@example.com"
      />
    );
    expect(screen.getAllByText("Accepted lines").length).toBeGreaterThan(0);
    expect(screen.getByRole("button", { name: /By tool/ })).toBeInTheDocument();
  });

  it("shows the error state with retry when the collection failed", () => {
    const refetch = vi.fn();
    render(
      <CollectionDrilldown
        def={DEF}
        data={result({ isError: true, refetch })}
        entityId="alice@example.com"
      />
    );
    expect(screen.getByText("Unable to load metrics")).toBeInTheDocument();
  });

  it("shows a spinner while pending", () => {
    const { container } = render(
      <CollectionDrilldown
        def={DEF}
        data={result({ isPending: true })}
        entityId="alice@example.com"
      />
    );
    expect(container.querySelector("svg")).toBeInTheDocument();
  });
});
