vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { MetricSummary } from "./definition-summary";

describe("<MetricSummary>", () => {
  it("names the dataset a metric reads, and its fields by their declared names", () => {
    render(
      <MetricSummary
        definition={{
          dataset: "commits",
          fields: [
            { field: "actor", type: "string", as_name: "actor" },
            { field: "lines", type: "int", agg: "sum", as_name: "total" },
          ],
          filters: [{ field: "actor", type: "string", op: "eq", value: "ann" }],
        }}
      />
    );

    expect(screen.getByText("Dataset")).toBeInTheDocument();
    expect(screen.getByText("commits")).toBeInTheDocument();
    expect(
      screen.getByText("actor as actor, sum(lines) as total")
    ).toBeInTheDocument();
    expect(screen.getByText("actor eq ann")).toBeInTheDocument();
  });

  it("names the warehouse table a metric reads, with its columns and JSON keys", () => {
    render(
      <MetricSummary
        definition={{
          database: "silver",
          table: "class_usage",
          fields: [
            { column: "tool", type: "string", as_name: "tool" },
            {
              column: "metrics_json",
              json: "sessions",
              type: "int",
              agg: "sum",
              as_name: "sessions",
            },
            { json: "actor", type: "string", as_name: "actor" },
          ],
        }}
      />
    );

    expect(screen.getByText("Table")).toBeInTheDocument();
    expect(screen.getByText("silver.class_usage")).toBeInTheDocument();
    // A json key with no column is read inside the row's own payload, which
    // is a different value from a column of that name.
    expect(
      screen.getByText(
        "tool as tool, sum(metrics_json.sessions) as sessions, raw_data.actor as actor"
      )
    ).toBeInTheDocument();
  });

  // The service reads a `database` of its own in preference to the one a
  // qualified name carries, so a summary that split the name would show a
  // table the metric does not read.
  it("names the table the service will read when both are written", () => {
    render(
      <MetricSummary
        definition={{
          database: "insight",
          table: "silver.class_usage",
          fields: [{ type: "int", agg: "count", as_name: "n" }],
        }}
      />
    );

    expect(screen.getByText("insight.silver.class_usage")).toBeInTheDocument();
  });

  it("shows a table written as database.table once", () => {
    render(
      <MetricSummary
        definition={{
          table: "silver.class_usage",
          fields: [{ type: "int", agg: "count", as_name: "n" }],
        }}
      />
    );

    expect(screen.getByText("silver.class_usage")).toBeInTheDocument();
  });
});
