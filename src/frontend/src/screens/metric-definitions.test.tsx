import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

import type { MetricDefinition } from "@/api/metric-definitions-client";
import type { MetricDefinitionGroup } from "@/queries/metric-definitions";

let queryState: {
  data?: MetricDefinitionGroup[];
  isPending: boolean;
  isError: boolean;
};

vi.mock("@/queries/metric-definitions", () => ({
  useMetricDefinitions: () => queryState,
}));

vi.mock("@/components/ui/sidebar", () => ({
  SidebarTrigger: () => null,
}));

import { MetricDefinitionsScreen } from "./metric-definitions";

function metric(over: Partial<MetricDefinition> = {}): MetricDefinition {
  return {
    metric_key: "git.commits",
    label: "Commits",
    short_label: null,
    description: "Commits authored in the period.",
    explanation: null,
    unit: null,
    format: "integer",
    direction: "higher_is_better",
    dimensions: [],
    is_enabled: true,
    schema_status: "ok",
    schema_error_code: null,
    last_observed_date: "2026-07-20",
    ...over,
  };
}

function group(
  prefix: string,
  metrics: MetricDefinition[]
): MetricDefinitionGroup {
  return { prefix, metrics };
}

beforeEach(() => {
  queryState = { data: undefined, isPending: false, isError: false };
});

describe("MetricDefinitionsScreen", () => {
  it("shows a spinner while pending", () => {
    queryState = { data: undefined, isPending: true, isError: false };
    render(<MetricDefinitionsScreen />);
    expect(screen.getByRole("status")).toBeInTheDocument();
  });

  it("shows an error alert when the query fails", () => {
    queryState = { data: undefined, isPending: false, isError: true };
    render(<MetricDefinitionsScreen />);
    expect(
      screen.getByText("Failed to load the metric catalog")
    ).toBeInTheDocument();
  });

  it("renders a group with its count and each metric's fields", () => {
    queryState = {
      data: [
        group("git", [
          metric({
            metric_key: "git.commits",
            label: "Commits",
            dimensions: ["repo"],
          }),
          metric({
            metric_key: "git.pr_cycle_time_h",
            label: "PR cycle time",
            direction: "lower_is_better",
            last_observed_date: null,
          }),
        ]),
      ],
      isPending: false,
      isError: false,
    };
    render(<MetricDefinitionsScreen />);

    expect(screen.getByText("git")).toBeInTheDocument();
    expect(screen.getByText("2")).toBeInTheDocument(); // group count
    expect(screen.getByText("git.commits")).toBeInTheDocument();
    expect(screen.getByText("Commits")).toBeInTheDocument();
    expect(screen.getByText("repo")).toBeInTheDocument();
    expect(screen.getByText("Higher")).toBeInTheDocument();
    expect(screen.getByText("Lower")).toBeInTheDocument();
    // last_observed_date null renders the em-dash placeholder.
    expect(screen.getByText("—")).toBeInTheDocument();
    // formatted date for the non-null row.
    expect(screen.getByText("20 Jul 2026")).toBeInTheDocument();
  });

  it("shows the compact label in parentheses when it differs", () => {
    queryState = {
      data: [
        group("collab", [
          metric({
            metric_key: "collab.meetings_count",
            label: "Meetings attended",
            short_label: "Meetings",
          }),
        ]),
      ],
      isPending: false,
      isError: false,
    };
    render(<MetricDefinitionsScreen />);
    expect(screen.getByText("(Meetings)")).toBeInTheDocument();
  });

  it("renders schema status badges including the error cause", () => {
    queryState = {
      data: [
        group("git", [
          metric({ metric_key: "git.a", schema_status: "unchecked" }),
          metric({ metric_key: "git.b", is_enabled: false }),
          metric({
            metric_key: "git.c",
            schema_status: "error",
            schema_error_code: "table_not_found",
          }),
        ]),
      ],
      isPending: false,
      isError: false,
    };
    render(<MetricDefinitionsScreen />);
    expect(screen.getByText("Unchecked")).toBeInTheDocument();
    expect(screen.getByText("Disabled")).toBeInTheDocument();
    expect(screen.getByText("Schema error")).toBeInTheDocument();
  });

  it("filters by the search box and shows the empty state on no match", async () => {
    const user = userEvent.setup();
    queryState = {
      data: [
        group("git", [
          metric({ metric_key: "git.commits", label: "Commits" }),
          metric({ metric_key: "git.prs_merged", label: "PRs merged" }),
        ]),
      ],
      isPending: false,
      isError: false,
    };
    render(<MetricDefinitionsScreen />);

    const box = screen.getByRole("searchbox");
    await user.type(box, "cycle");
    expect(screen.getByText("No metrics found")).toBeInTheDocument();

    await user.clear(box);
    await user.type(box, "merged");
    expect(screen.getByText("PRs merged")).toBeInTheDocument();
    expect(screen.queryByText("Commits")).not.toBeInTheDocument();
  });
});
