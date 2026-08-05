import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { MetricGroupCard } from "@/components/widgets/metric-views/metric-group-card";
import type { MetricGroup } from "@/lib/insight/groups";
import { normalizeMetricResults } from "@/lib/metrics/collection";
import type { MetricCollectionResult } from "@/queries/metric-results";
import type { MetricResult } from "@/api/metric-results-client";

vi.mock("@/hooks/use-settings", () => ({
  useSettings: () => ({ focusMode: "all", showExplanations: true }),
}));

function aiMetric(
  key: string,
  value: number | null,
  opts: { targetNull?: boolean; median?: number } = {},
): MetricResult {
  const median = opts.median ?? 10;
  return {
    metric_key: key,
    label: key,
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
            target_value: opts.targetNull ? null : value,
            p25: median * 0.5,
            median,
            p75: median * 1.5,
            min: 0,
            max: median * 3,
            n: 10,
          },
        ],
      },
    ],
  };
}

const DEF: MetricGroup = {
  id: "ai_adoption",
  title: "AI adoption",
  collection: {
    metrics: [
      { key: "ai.active_days", views: [{ view: "period" }, { view: "peer" }] },
      { key: "ai.cost", views: [{ view: "period" }, { view: "peer" }] },
    ],
  },
  card: { preview: ["ai.active_days", "ai.cost"] },
  drilldown: [],
};

function result(
  metrics: MetricResult[],
  overrides: Partial<MetricCollectionResult> = {},
): MetricCollectionResult {
  return {
    byKey: normalizeMetricResults(metrics),
    previousByKey: null,
    isPending: false,
    isFetching: false,
    isError: false,
    refetch: vi.fn(),
    ...overrides,
  };
}

describe("MetricGroupCard", () => {
  it("renders preview rows with response labels and values", () => {
    render(
      <MetricGroupCard
        def={DEF}
        data={result([aiMetric("ai.active_days", 20), aiMetric("ai.cost", 3)])}
        entityId="me@x.com"
        onOpen={vi.fn()}
      />,
    );
    expect(screen.getByText("AI adoption")).toBeInTheDocument();
    expect(screen.getByText("20 days")).toBeInTheDocument();
    expect(screen.getByText("3 days")).toBeInTheDocument();
  });

  it("shows a single empty state without a standing badge when no metric has data", () => {
    render(
      <MetricGroupCard
        def={DEF}
        data={result([
          aiMetric("ai.active_days", null),
          aiMetric("ai.cost", null),
        ])}
        entityId="me@x.com"
        onOpen={vi.fn()}
      />,
    );
    expect(
      screen.getByText("No metrics with data for this period."),
    ).toBeInTheDocument();
    // The badge would only restate the absence — one message, not two.
    expect(screen.queryByText("no peer data")).not.toBeInTheDocument();
    // Nothing to drill into — the card is not an interactive button.
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("keeps a fixed preview key with no value, rendering an em dash", () => {
    render(
      <MetricGroupCard
        def={DEF}
        data={result([
          aiMetric("ai.active_days", 20),
          aiMetric("ai.cost", null),
        ])}
        entityId="me@x.com"
        onOpen={vi.fn()}
      />,
    );
    // Both preview rows stay on the card — the valueless one shows "—", not
    // dropped, so the card's identity is stable across periods.
    expect(screen.getByText("ai.active_days")).toBeInTheDocument();
    expect(screen.getByText("ai.cost")).toBeInTheDocument();
    expect(screen.getByText("—")).toBeInTheDocument();
  });

  it("keeps the card name and shows a spinner while loading", () => {
    render(
      <MetricGroupCard
        def={DEF}
        data={result([], { isPending: true })}
        entityId="me@x.com"
        onOpen={vi.fn()}
      />,
    );
    expect(screen.getByText("AI adoption")).toBeInTheDocument();
    expect(
      screen.getByRole("status", { name: "Loading AI adoption" }),
    ).toBeInTheDocument();
    // Not interactive while loading — nothing to open yet.
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("gives unmeasured people no standing (regression: the fifth scoring surface)", () => {
    // Zero-filled period value with a null peer target_value: every other
    // selector calls this person neutral — the card must not red-flag them.
    render(
      <MetricGroupCard
        def={DEF}
        data={result([
          aiMetric("ai.active_days", 0, { targetNull: true }),
          aiMetric("ai.cost", 0, { targetNull: true }),
        ])}
        entityId="me@x.com"
        onOpen={vi.fn()}
      />,
    );
    // No metric is scorable → no rankable peers.
    expect(screen.getByText("no peer data")).toBeInTheDocument();
  });

  it("scores measured people against quartiles", () => {
    render(
      <MetricGroupCard
        def={DEF}
        data={result([
          aiMetric("ai.active_days", 20),
          aiMetric("ai.cost", 2, { median: 10 }),
        ])}
        entityId="me@x.com"
        onOpen={vi.fn()}
      />,
    );
    // active_days 20 ≥ p75 15 → top; cost 2 ≤ p25 5 → bottom → 1 bottom wins
    // the phrase (behind beats ahead), single bottom below the red bar → amber.
    expect(screen.getByText("1 behind peers")).toBeInTheDocument();
  });

  it("owns its error state with retry", () => {
    const refetch = vi.fn();
    render(
      <MetricGroupCard
        def={DEF}
        data={result([], { isError: true, refetch })}
        entityId="me@x.com"
        onOpen={vi.fn()}
      />,
    );
    expect(
      screen.getByRole("status", { name: "AI adoption" }),
    ).toBeInTheDocument();
    screen.getByRole("button").click();
    expect(refetch).toHaveBeenCalled();
  });
});
