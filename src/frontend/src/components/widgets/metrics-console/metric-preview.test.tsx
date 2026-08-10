import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import "@/i18n";

import type { MetricResultsResponse } from "@/api/metric-results-client";

import { MetricPreview } from "./metric-preview";

function periodResponse(
  values: Array<{ entity_id: string; value: number | null }>
): MetricResultsResponse {
  return {
    metrics: [
      {
        metric_key: "example.lines",
        label: "Lines",
        unit: "lines",
        format: "integer",
        direction: "higher_is_better",
        computation: "sum",
        views: [{ view: "period", values }],
      },
    ],
  };
}

describe("MetricPreview", () => {
  it("renders a row per period value, with a dash for a null value", () => {
    render(
      <MetricPreview
        result={periodResponse([
          { entity_id: "person-a", value: 42 },
          { entity_id: "person-b", value: null },
        ])}
      />
    );
    expect(screen.getByText("person-a")).toBeInTheDocument();
    expect(screen.getByText("42")).toBeInTheDocument();
    expect(screen.getByText("person-b")).toBeInTheDocument();
    expect(screen.getByText("—")).toBeInTheDocument();
  });

  it("shows an empty state when there are no period values", () => {
    render(<MetricPreview result={periodResponse([])} />);
    expect(
      screen.getByText("The metric returned no values.")
    ).toBeInTheDocument();
  });

  it("shows an empty state when the response carries no period view", () => {
    render(<MetricPreview result={{ metrics: [] }} />);
    expect(
      screen.getByText("The metric returned no values.")
    ).toBeInTheDocument();
  });
});
