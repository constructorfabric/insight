import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { TeamMetricGroupCard } from "@/components/widgets/metric-views/team-metric-group-card";
import type { MetricGroup } from "@/lib/insight/groups";
import { normalizeMetricResults } from "@/lib/metrics/collection";
import type { MetricCollectionResult } from "@/queries/metric-results";
import type { MetricResult } from "@/api/metric-results-client";

vi.mock("@/hooks/use-settings", () => ({
  useSettings: () => ({ focusMode: "all" }),
}));

const DEF: MetricGroup = {
  id: "ai_adoption",
  title: "AI adoption",
  collection: {
    metrics: [
      { key: "ai.active_days", views: [{ view: "period" }, { view: "peer" }] },
    ],
  },
  card: { preview: ["ai.active_days"] },
  drilldown: [],
};

const MEMBERS = ["a@x.com", "b@x.com", "c@x.com"];

/** One metric with per-member peer bands (median 10, quartiles 5/15). */
function metric(
  perMember: Array<{ id: string; value: number | null; suppressed?: boolean }>,
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
        values: perMember.map((m) => ({ entity_id: m.id, value: m.value })),
      },
      {
        view: "peer",
        values: perMember.map((m) => ({
          entity_id: m.id,
          target_value: m.value,
          p25: m.suppressed ? null : 5,
          median: m.suppressed ? null : 10,
          p75: m.suppressed ? null : 15,
          min: m.suppressed ? null : 0,
          max: m.suppressed ? null : 30,
          n: m.suppressed ? 3 : 12,
        })),
      },
    ],
  };
}

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

describe("TeamMetricGroupCard", () => {
  it("keeps the card name and shows a spinner while loading", () => {
    render(
      <TeamMetricGroupCard
        def={DEF}
        data={result([], { isPending: true })}
        memberIds={MEMBERS}
        onOpen={vi.fn()}
      />,
    );
    expect(screen.getByText("AI adoption")).toBeInTheDocument();
    expect(
      screen.getByRole("status", { name: "Loading AI adoption" }),
    ).toBeInTheDocument();
  });

  it("shows an error with retry", () => {
    render(
      <TeamMetricGroupCard
        def={DEF}
        data={result([], { isError: true })}
        memberIds={MEMBERS}
        onOpen={vi.fn()}
      />,
    );
    expect(
      screen.getByRole("status", { name: "AI adoption" }),
    ).toBeInTheDocument();
  });

  it("rolls up member standings into an ahead tally and a preview row", () => {
    render(
      <TeamMetricGroupCard
        def={DEF}
        data={result([
          metric([
            { id: "a@x.com", value: 20 }, // top
            { id: "b@x.com", value: 18 }, // top
            { id: "c@x.com", value: 2 }, // bottom
          ]),
        ])}
        memberIds={MEMBERS}
        onOpen={vi.fn()}
      />,
    );
    // Metric scores good (plurality top) → 1 ahead of peers.
    expect(screen.getByText("1 ahead of peers")).toBeInTheDocument();
    // Preview row: behind wins over ahead on a mixed profile.
    const row = screen.getByText("Active AI days").closest("li")!;
    expect(row).toHaveTextContent("1 behind");
  });

  it("shows the ahead count when nobody trails", () => {
    render(
      <TeamMetricGroupCard
        def={DEF}
        data={result([
          metric([
            { id: "a@x.com", value: 20 }, // top
            { id: "b@x.com", value: 8 }, // in pack
            { id: "c@x.com", value: 12 }, // in pack
          ]),
        ])}
        memberIds={MEMBERS}
        onOpen={vi.fn()}
      />,
    );
    const row = screen.getByText("Active AI days").closest("li")!;
    expect(row).toHaveTextContent("1 ahead");
  });

  it("shows the no-peer-data fallback when nothing is scorable", () => {
    render(
      <TeamMetricGroupCard
        def={DEF}
        data={result([
          metric([
            { id: "a@x.com", value: 2, suppressed: true },
            { id: "b@x.com", value: 3, suppressed: true },
            { id: "c@x.com", value: 4, suppressed: true },
          ]),
        ])}
        memberIds={MEMBERS}
        onOpen={vi.fn()}
      />,
    );
    expect(screen.getByText("no peer data")).toBeInTheDocument();
    expect(
      screen.getByText(/No metrics with peer data/i),
    ).toBeInTheDocument();
  });
});
