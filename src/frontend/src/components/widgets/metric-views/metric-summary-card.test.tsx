import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { MetricSummaryCard } from "@/components/widgets/metric-views/metric-summary-card";
import {
  normalizeMetricResults,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import type { MetricResult } from "@/api/metric-results-client";

vi.mock("@/hooks/use-settings", () => ({
  useSettings: () => ({ focusMode: "all", showExplanations: true }),
}));

function summaryMetric(
  opts: {
    value: number | null;
    median?: number;
    direction?: MetricResult["direction"];
    breakdown?: Array<{ tool: string; value: number | null }>;
  } = { value: 20 },
): NormalizedMetricResult {
  const median = opts.median ?? 10;
  const views: MetricResult["views"] = [
    { view: "period", values: [{ entity_id: "me@x.com", value: opts.value }] },
    {
      view: "peer",
      values: [
        {
          entity_id: "me@x.com",
          target_value: opts.value,
          p25: median * 0.5,
          median,
          p75: median * 1.5,
          min: 0,
          max: median * 3,
          n: 12,
        },
      ],
    },
  ];
  if (opts.breakdown) {
    views.push({
      view: "breakdown",
      dimensions: ["tool"],
      values: opts.breakdown.map((row) => ({
        entity_id: "me@x.com",
        dimensions: [{ key: "tool", value: row.tool, label: row.tool }],
        value: row.value,
      })),
    });
  }
  const result: MetricResult = {
    metric_key: "collab.meeting_hours",
    label: "Meeting hours",
    description: "Time in meetings",
    unit: "h",
    format: "integer",
    direction: opts.direction ?? "higher_is_better",
    computation: "sum",
    views,
  };
  return normalizeMetricResults([result]).get("collab.meeting_hours")!;
}

describe("MetricSummaryCard", () => {
  it("renders the headline value, unit, and collapses the breakdown by default", () => {
    render(
      <MetricSummaryCard
        metric={summaryMetric({
          value: 20,
          breakdown: [
            { tool: "slack", value: 12 },
            { tool: "teams", value: 8 },
          ],
        })}
        entityId="me@x.com"
      />,
    );
    expect(screen.getByText("Meeting hours")).toBeInTheDocument();
    expect(screen.getByText("20")).toBeInTheDocument();
    expect(screen.getByText("h")).toBeInTheDocument();
    const toggle = screen.getByRole("button", { name: /By tool/ });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("slack")).not.toBeInTheDocument();
  });

  it("reveals the ribbon and legend when expanded", async () => {
    const user = userEvent.setup();
    render(
      <MetricSummaryCard
        metric={summaryMetric({
          value: 20,
          breakdown: [
            { tool: "slack", value: 12 },
            { tool: "teams", value: 8 },
          ],
        })}
        entityId="me@x.com"
      />,
    );
    await user.click(screen.getByRole("button", { name: /By tool/ }));
    expect(screen.getByText("slack")).toBeInTheDocument();
    expect(screen.getByText("teams")).toBeInTheDocument();
  });

  it("hides the breakdown control when a single group has data", () => {
    render(
      <MetricSummaryCard
        metric={summaryMetric({
          value: 20,
          breakdown: [
            { tool: "slack", value: 20 },
            { tool: "teams", value: 0 },
          ],
        })}
        entityId="me@x.com"
      />,
    );
    expect(
      screen.queryByRole("button", { name: /By tool/ }),
    ).not.toBeInTheDocument();
  });

  it("renders an em dash when the entity has no value", () => {
    render(
      <MetricSummaryCard
        metric={summaryMetric({ value: null })}
        entityId="me@x.com"
      />,
    );
    expect(screen.getByText("—")).toBeInTheDocument();
  });
});
