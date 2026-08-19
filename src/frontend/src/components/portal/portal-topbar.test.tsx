// @vitest-environment jsdom
/**
 * The seam between the catalog and the control.
 *
 * `cohortOptions` is tested on its own and so is `SliceSelect`; what neither
 * covers is that the bar actually asks the catalog and hands the answer down.
 * This test therefore runs the real query hook against a real QueryClient and
 * stubs only the network — so a wiring mistake here fails rather than hiding
 * behind a stubbed hook.
 */
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { SidebarProvider } from "@/components/ui/sidebar";
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { MetricDefinitionListResponse } from "@/api/metric-definitions-client";
import { identityPerson, pid } from "@/test/identity";

const mocks = vi.hoisted(() => ({
  definitions: { metrics: [] } as MetricDefinitionListResponse,
}));

// Only the network is stubbed; the query hook itself is the real one.
vi.mock("@/api/metric-definitions-client", () => ({
  listMetricDefinitions: () => Promise.resolve(mocks.definitions),
}));
vi.mock("@/queries/ic-dashboard", () => ({
  // A viewer with two reports who differ by division: enough for the roster
  // walk to offer something, so a catalog winning is a real preference and
  // not the absence of an alternative.
  useIcPerson: () => ({
    data: identityPerson("boss", {
      person_id: pid("boss"),
      division: "Alpha",
      subordinates: [
        identityPerson("one", { person_id: pid("one"), division: "Alpha" }),
        identityPerson("two", { person_id: pid("two"), division: "Beta" }),
      ],
    }),
  }),
}));
vi.mock("@/auth", () => ({ useViewer: () => ({ personId: pid("boss") }) }));
vi.mock("@/hooks/use-portal-period", () => ({
  usePortalPeriod: () => ({
    period: "month",
    customRange: null,
    setPeriod: vi.fn(),
    setCustomRange: vi.fn(),
  }),
}));
vi.mock("@/components/portal/scope-select", () => ({
  ScopeSelect: () => <div />,
}));
vi.mock("@/components/widgets/period-selector-bar", () => ({
  PeriodSelectorBar: () => <div />,
}));
// Stands in for the control so the test can read what it was given.
vi.mock("@/components/portal/slice-select", () => ({
  SliceSelect: ({ dims }: { dims: { key: string; label: string }[] }) => (
    <div data-testid="dims">{dims.map((d) => d.key).join(",")}</div>
  ),
}));

import { portalRouter } from "@/test/portal-router";

import { PortalTopBar } from "./portal-topbar";

function bar() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <SidebarProvider>
        <PortalTopBar />
      </SidebarProvider>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  window.matchMedia ??= ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
  mocks.definitions = { metrics: [] };
  portalRouter.reset();
});

describe("PortalTopBar", () => {
  it("offers what the catalog allows once the response carries it", async () => {
    mocks.definitions = {
      metrics: [],
      comparison_attributes: [{ id: "job_title", label: "Title" }],
    } as MetricDefinitionListResponse;
    bar();
    await waitFor(() =>
      expect(screen.getByTestId("dims")).toHaveTextContent("job_title"),
    );
  });

  it("prefers the catalog over what the roster happens to support", async () => {
    // The roster here would offer `division`; a governed answer wins, and the
    // two are never shown side by side.
    mocks.definitions = {
      metrics: [],
      comparison_attributes: [{ id: "job_title", label: "Title" }],
    } as MetricDefinitionListResponse;
    bar();
    await waitFor(() =>
      expect(screen.getByTestId("dims")).not.toHaveTextContent("division"),
    );
  });

  it("falls back to the roster while no catalog exists", async () => {
    bar();
    await waitFor(() =>
      expect(screen.getByTestId("dims")).toHaveTextContent("division"),
    );
  });
});

const filters = () => screen.queryByRole("group", { name: "View filters" });

describe("PortalTopBar · zones that filter nothing", () => {
  it("drops the whole filter group on Manage", async () => {
    portalRouter.set({ zone: "manage" });
    bar();

    // Awaited: the catalog resolves after the first paint.
    await waitFor(() => expect(screen.queryByTestId("dims")).toBeNull());
    expect(filters()).toBeNull();
  });

  it("keeps it on a zone whose numbers the filters move", async () => {
    portalRouter.set({ zone: "overview" });
    bar();

    await waitFor(() => expect(filters()).toBeInTheDocument());
  });
});
