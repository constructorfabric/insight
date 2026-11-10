// @vitest-environment jsdom
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { MetricDirection } from "@/api/metric-results-client";
import { MetricsCompareTable } from "@/components/portal/metrics-compare-table";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import { AppTooltips, openTooltip } from "@/test/tooltip";

const ADA = "ada";
const BO = "bo";
const CY = "cy";
const PEOPLE = [
  { entityId: ADA, displayName: "Ada" },
  { entityId: BO, displayName: "Bo" },
  { entityId: CY, displayName: "Cy" },
];

function result(
  key: string,
  label: string,
  direction: MetricDirection,
  values: Array<[string, number | null]>
): NormalizedMetricResult {
  return {
    metric_key: key,
    label,
    unit: null,
    computation: "sum",
    format: "integer",
    direction,
    period: {
      view: "period",
      values: values.map(([entity_id, value]) => ({ entity_id, value })),
    },
  } as NormalizedMetricResult;
}

function renderTable(metrics: NormalizedMetricResult[]) {
  return render(
    <MetricsCompareTable
      people={PEOPLE}
      keys={metrics.map((metric) => metric.metric_key)}
      byKey={new Map(metrics.map((metric) => [metric.metric_key, metric]))}
      helpByKey={new Map()}
      roles={new Map()}
    />,
    { wrapper: AppTooltips }
  );
}

const cells = (label: string) =>
  within(
    screen.getByRole("row", {
      name: (accessibleName) => accessibleName.includes(label),
    })
  ).getAllByRole("cell");

describe("MetricsCompareTable", () => {
  const metrics = [
    result("git.commits", "Commits", "higher_is_better", [
      [ADA, 10],
      [BO, 40],
      [CY, 10],
    ]),
    result("git.commit_size", "Commit size", "lower_is_better", [
      [ADA, 8],
      [BO, 2],
      [CY, 2],
    ]),
    result("collab.messages_sent", "Chat messages sent", "neutral", [
      [ADA, 1],
      [BO, 9],
      [CY, 4],
    ]),
    result("git.prs", "Pull requests", "higher_is_better", [
      [ADA, 5],
      [BO, null],
      [CY, 15],
    ]),
  ];

  it("names a direction and explains it, and stays quiet when the metric has none", async () => {
    const user = userEvent.setup();
    renderTable(metrics);

    expect(
      within(screen.getByRole("rowheader", { name: /Commits/ })).getByRole(
        "button",
        {
          name: "Higher is better",
        }
      )
    ).toBeInTheDocument();
    const lower = within(
      screen.getByRole("rowheader", { name: /Commit size/ })
    ).getByRole("button", { name: "Lower is better" });
    expect(
      within(
        screen.getByRole("rowheader", { name: /Chat messages sent/ })
      ).queryByRole("button")
    ).not.toBeInTheDocument();

    await user.hover(lower);

    expect(openTooltip()).toHaveTextContent("Lower is better");
  });

  it("paints every tied best value and leaves the others, including a neutral metric, plain", () => {
    renderTable(metrics);

    const commits = cells("Commits");
    expect(commits.map((cell) => cell.textContent)).toEqual([
      "10−75%",
      "40 (best)",
      "10−75%",
      "10",
      "20",
    ]);
    expect(commits[1]).toHaveClass("text-success");
    expect(commits[0]).not.toHaveClass("text-success");
    expect(commits[3]).not.toHaveClass("text-success");

    const size = cells("Commit size");
    expect(size[1]).toHaveClass("text-success");
    expect(size[2]).toHaveClass("text-success");
    expect(size[0]).not.toHaveClass("text-destructive");

    for (const cell of cells("Chat messages sent")) {
      expect(cell).not.toHaveClass("text-success");
    }
  });

  it("leaves a missing value out of the median, the average and the best mark", () => {
    renderTable(metrics);

    const row = cells("Pull requests");
    expect(row.map((cell) => cell.textContent)).toEqual([
      "5−67%",
      "—",
      "15 (best)",
      "10",
      "10",
    ]);
    expect(row[1]).not.toHaveClass("text-success");
    expect(row[2]).toHaveClass("text-success");
  });

  it("says the median and the average are aggregates of the compared people", async () => {
    const user = userEvent.setup();
    renderTable(metrics);

    const median = screen.getByRole("columnheader", { name: /Median/ });
    expect(median).toHaveTextContent(/aggregate/i);

    await user.hover(within(median).getByText("Median"));

    expect(openTooltip()).toHaveTextContent(
      "Median of the people in this comparison. People with no value are left out."
    );
  });
});
