// @vitest-environment jsdom
/**
 * Manage-zone surfaces read the UNIFIED registry, not the legacy catalog:
 * the table lists `metric_key`s `/v1/metric-results` actually serves, spells
 * out an unobserved definition as "no data yet" rather than hiding it, and
 * Data health separates "schema checks out" from "has ever produced a row".
 */
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import type { MetricDefinition } from "@/api/metric-definitions-client";
import type { MetricDefinitionGroup } from "@/queries/metric-definitions";

const mocks = vi.hoisted(() => ({
  q: {
    data: undefined as MetricDefinitionGroup[] | undefined,
    isLoading: false,
    isError: false,
    refetch: vi.fn(),
  },
}));

vi.mock("@/queries/metric-definitions", () => ({
  useMetricDefinitions: () => mocks.q,
}));

const adminGate = vi.hoisted(() => ({
  value: {
    isAdmin: false,
    isPending: false,
    isError: false,
    retry: () => undefined,
  },
}));
vi.mock("@/queries/identity-me", () => ({
  useIsAdmin: () => adminGate.value,
}));

// The console itself has its own test file; here only the gate is under test.
vi.mock("@/components/portal/identities-view", () => ({
  IdentitiesView: () => <div data-testid="identities-view" />,
}));

import { ManageView } from "./manage-view";

function def(over: Partial<MetricDefinition>): MetricDefinition {
  return {
    metric_key: "git.commits",
    label: "Commits",
    short_label: null,
    description: null,
    explanation: null,
    unit: "commits",
    format: "integer",
    direction: "higher_is_better",
    dimensions: [],
    is_enabled: true,
    schema_status: "ok",
    schema_error_code: null,
    last_observed_date: "2026-07-26",
    ...over,
  } as MetricDefinition;
}

beforeEach(() => {
  mocks.q.refetch.mockClear();
  mocks.q.isLoading = false;
  mocks.q.isError = false;
  mocks.q.data = [
    {
      prefix: "git",
      metrics: [
        def({
          metric_key: "git.prs_merged",
          label: "Pull requests merged",
          short_label: "PRs merged",
          dimensions: ["repository", "project"],
        }),
        def({ metric_key: "git.commits" }),
      ],
    },
    {
      prefix: "tasks",
      metrics: [
        def({
          metric_key: "tasks.closed",
          label: "Tasks closed",
          unit: null,
          direction: "higher_is_better",
          schema_status: "error",
          schema_error_code: "table_not_found",
          last_observed_date: null,
        }),
      ],
    },
  ];
});

describe("Manage · Metric catalog", () => {
  it("lists unified metric keys, sorted, with the endpoint it came from", () => {
    render(<ManageView item="metric-catalog" />);
    expect(screen.getByText("/v1/metric-definitions")).toBeInTheDocument();
    expect(screen.getByText("3 metrics", { exact: false })).toBeInTheDocument();
    const keys = screen
      .getAllByText(/^(git|tasks)\./)
      .map((el) => el.textContent);
    expect(keys).toEqual(["git.commits", "git.prs_merged", "tasks.closed"]);
  });

  it("prefers the short label and renders dimensions and direction", () => {
    render(<ManageView item="metric-catalog" />);
    expect(screen.getByText("PRs merged")).toBeInTheDocument();
    expect(screen.getByText("repository · project")).toBeInTheDocument();
    expect(screen.getAllByText("higher is better").length).toBe(3);
  });

  it("says 'no data yet' for a definition with no observation", () => {
    render(<ManageView item="metric-catalog" />);
    expect(screen.getByText("no data yet")).toBeInTheDocument();
    // the two observed definitions keep their date
    expect(screen.getAllByText("2026-07-26")).toHaveLength(2);
  });

  it("shows the schema error code next to a failing status", () => {
    render(<ManageView item="metric-catalog" />);
    expect(screen.getByText(/error · table_not_found/)).toBeInTheDocument();
  });

  it("offers retry when the registry request fails", async () => {
    mocks.q.isError = true;
    render(<ManageView item="metric-catalog" />);
    await userEvent.click(screen.getByRole("button", { name: /retry/i }));
    expect(mocks.q.refetch).toHaveBeenCalledOnce();
  });
});

describe("Manage · Data health", () => {
  it("counts schema statuses and, separately, definitions with no data", () => {
    render(<ManageView item="data-health" />);
    expect(screen.getByText(/across 3 metrics/)).toBeInTheDocument();
    // 2 ok · 1 error · 0 unchecked · 1 without any observation
    const tile = (label: string) =>
      screen.getByText(label).closest("div")?.parentElement?.textContent ?? "";
    expect(tile("ok")).toMatch(/^2/);
    expect(tile("error")).toMatch(/^1/);
    expect(tile("unchecked")).toMatch(/^0/);
    expect(tile("no data yet")).toMatch(/^1/);
  });
});

describe("Manage · unwired items", () => {
  it("renders an honest placeholder instead of a fake admin screen", () => {
    render(<ManageView item="taxonomy" />);
    expect(screen.getByText(/not built yet/i)).toBeInTheDocument();
  });
});

describe("identities gate", () => {
  const gate = (over: Partial<typeof adminGate.value>) => {
    adminGate.value = {
      isAdmin: false,
      isPending: false,
      isError: false,
      retry: () => undefined,
      ...over,
    };
  };

  it("refuses a non-admin explicitly — a pasted URL must not look broken", () => {
    gate({});
    render(<ManageView item="identities" />);

    expect(screen.getByRole("alert")).toHaveTextContent(/admin surface/i);
    expect(
      screen.queryByText(/under construction/i),
    ).not.toBeInTheDocument();
  });

  it("never flashes the console while the role check is in flight", () => {
    gate({ isPending: true });
    render(<ManageView item="identities" />);

    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(
      screen.queryByText(/under construction/i),
    ).not.toBeInTheDocument();
  });

  it("opens for an admin", () => {
    gate({ isAdmin: true });
    render(<ManageView item="identities" />);

    expect(screen.getByTestId("identities-view")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("says 'could not verify' with a retry when the check itself failed", async () => {
    const retry = vi.fn();
    gate({ isError: true, retry });
    render(<ManageView item="identities" />);

    // Still no console (fail closed) — but the copy must not send a real
    // admin to ask for a role they already hold.
    expect(screen.queryByTestId("identities-view")).not.toBeInTheDocument();
    expect(screen.queryByText(/admin surface/i)).not.toBeInTheDocument();
    expect(screen.getByText(/could not verify/i)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /retry/i }));
    expect(retry).toHaveBeenCalledOnce();
  });
});
