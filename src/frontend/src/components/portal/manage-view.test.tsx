// @vitest-environment jsdom
/**
 * Manage-zone surfaces read the UNIFIED registry, not the legacy catalog:
 * the table lists `metric_key`s `/v1/metric-results` actually serves and
 * spells out an unobserved definition as "no data yet" rather than hiding it.
 *
 * Connector health has its own test file; here only its place in the zone and
 * its gate are under test.
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

vi.mock("@/components/portal/connector-health", () => ({
  ConnectorHealthPane: () => <div data-testid="connector-health-pane" />,
}));

import { MANAGE_ITEMS } from "@/lib/portal/nav-model";

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
  // The gate is hoisted module state, so a test that flips it leaves it flipped
  // for every test below — order-dependent today, wrong tomorrow.
  adminGate.value = {
    isAdmin: false,
    isPending: false,
    isError: false,
    retry: () => undefined,
  };
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

describe("Manage · What's new", () => {
  it("renders the release notes without the legacy screen's own header", () => {
    render(<ManageView item="whats-new" />);

    expect(screen.getByText("Insight · What's new")).toBeInTheDocument();
    expect(
      screen.getByText("We've moved to the new interface for good"),
    ).toBeInTheDocument();
    expect(screen.queryByRole("banner")).not.toBeInTheDocument();
  });
});

describe("Manage · Connector health", () => {
  it("is instance-wide, so a non-admin is refused rather than shown an empty page", () => {
    adminGate.value = { ...adminGate.value, isAdmin: false };
    render(<ManageView item="connector-health" />);

    expect(
      screen.queryByTestId("connector-health-pane"),
    ).not.toBeInTheDocument();
    // A gate rendering nothing at all would satisfy the line above, and would
    // leave a non-admin on a blank screen with nothing to act on.
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });

  it("opens for an operator", () => {
    adminGate.value = { ...adminGate.value, isAdmin: true };
    render(<ManageView item="connector-health" />);

    expect(screen.getByTestId("connector-health-pane")).toBeInTheDocument();
  });

  it("no longer offers the pane it replaced", () => {
    // Asserting what `ManageView` does with `item="data-health"` would be
    // unfalsifiable: `item` is always the output of `resolveZoneItem`, and
    // `data-health` is no longer a manage item, so the app can never pass it.
    // What is reachable — and what a stale bookmark meets — is the nav model.
    expect(MANAGE_ITEMS.map((entry) => entry.id)).toContain("connector-health");
    expect(MANAGE_ITEMS.map((entry) => entry.id)).not.toContain("data-health");
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
