/**
 * `/portal/custom` and `/portal/custom/$name` are file routes nested under
 * `/portal`, and `PortalLayout` renders them through its `<Outlet/>` — so the
 * rail, the topbar and the context pane are around them, and the pane lists
 * the dashboards.
 *
 * Every other test for these two routes renders the route's `component`
 * directly with `@tanstack/react-router` mocked out (see
 * `src/test/portal-router.tsx`), so none of them would catch a page that
 * escaped the shell: the route matches, the component renders, and the shell
 * is never in the picture. This one builds a router from the real generated
 * `routeTree` and drives it with a memory history, so actual route matching
 * and parenting decide what renders — the same tree `router.ts` hands to
 * `RouterProvider` in the app.
 */

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  RouterProvider,
  createMemoryHistory,
  createRouter,
} from "@tanstack/react-router";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import { authStore } from "@/auth/auth-store";
import { makeSession } from "@/test/session";
import { routeTree } from "@/routeTree.gen";
import * as customClient from "@/api/custom-client";
import * as identityClient from "@/api/identity-client";
import { ADMIN_ROLE_ID } from "@/queries/identity-me";

vi.mock("@/api/custom-client");
vi.mock("@/api/identity-client");

/** The caller `GET /v1/me` describes, which is what the zone gates on. */
function signedInAs({ admin }: { admin: boolean }) {
  vi.mocked(identityClient.getMe).mockResolvedValue({
    person_id: "00000000-0000-4000-8000-000000000001",
    insight_tenant_id: "00000000-0000-4000-8000-0000000000ff",
    roles: admin ? [{ role_id: ADMIN_ROLE_ID, name: "admin" }] : [],
    visibility_policy: "org_chart",
  });
}

function renderAt(path: string) {
  const router = createRouter({
    routeTree,
    history: createMemoryHistory({ initialEntries: [path] }),
  });
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  );
}

beforeEach(() => {
  vi.resetAllMocks();
  // The shell measures the viewport to decide whether the context pane is in
  // flow or off-canvas; jsdom implements no media queries.
  vi.stubGlobal(
    "matchMedia",
    (query: string) =>
      ({
        matches: false,
        media: query,
        onchange: null,
        addEventListener: () => {},
        removeEventListener: () => {},
        addListener: () => {},
        removeListener: () => {},
        dispatchEvent: () => false,
      }) as unknown as MediaQueryList
  );
  signedInAs({ admin: true });
  // Empty personId skips the root's viewer-identity prefetch.
  authStore.setAuthenticated(makeSession({ personId: "" }));
});

afterEach(() => {
  authStore.reset();
  vi.unstubAllGlobals();
});

describe("the /portal/custom routes, through the real router", () => {
  it("renders the dashboard list at /portal/custom, inside the portal shell", async () => {
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: [
      "engineering",
    ], total: 1 });

    renderAt("/portal/custom");

    // The card on the page and the row in the context pane: the pane is the
    // nav the reader picks dashboards from, so it has to be one of them. They
    // read the catalogue through different queries — a page of names, and all
    // of them — so they arrive one after the other.
    // Two independent queries — a page of names and all of them — so the
    // second link can land well after the first on a loaded runner.
    const links = await waitFor(
      () => {
        const found = screen.getAllByRole("link", { name: "engineering" });
        expect(found.length).toBeGreaterThan(1);
        return found;
      },
      { timeout: 10000 }
    );
    for (const link of links) {
      expect(link).toHaveAttribute("href", "/portal/custom/engineering");
    }
    expect(
      document.querySelector('[data-slot="sidebar-wrapper"]')
    ).toBeInTheDocument();
  });

  it("renders a dashboard at /portal/custom/$name, inside the portal shell", async () => {
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: [
      "engineering",
    ], total: 1 });
    vi.mocked(customClient.fetchDashboard).mockResolvedValue({
      title: "Engineering",
      widgets: [],
    });

    renderAt("/portal/custom/engineering");

    expect(
      await screen.findByRole("heading", { name: "Engineering" })
    ).toBeInTheDocument();
    expect(
      document.querySelector('[data-slot="sidebar-wrapper"]')
    ).toBeInTheDocument();
  });

  it("keeps the thread when creating a dashboard navigates to it", async () => {
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: [
      "engineering",
    ], total: 1 });
    vi.mocked(customClient.fetchDashboard).mockResolvedValue({
      title: "Delivery",
      widgets: [],
    });
    vi.mocked(customClient.sendChat).mockResolvedValue({
      reply: "Built it",
      created: { widgets: [], dashboard: "delivery" },
    });

    renderAt("/portal/custom");
    await screen.findByRole("heading", { name: "Custom" });

    await userEvent.type(
      screen.getByTestId("chat-input"),
      "a dashboard about delivery"
    );
    await userEvent.click(screen.getByTestId("chat-send"));

    // The new dashboard opens (its title is in the pane too, so the heading
    // is what says the page changed)...
    expect(
      await screen.findByRole("heading", { name: "Delivery" })
    ).toBeInTheDocument();
    // ...and the conversation is still there. The chat used to be mounted per
    // page, so this navigation unmounted it and the reader lost what they had
    // just asked.
    expect(
      screen.getByText("a dashboard about delivery")
    ).toBeInTheDocument();
    expect(screen.getByText("Built it")).toBeInTheDocument();
  });

  it("shows a widget the chat adds to the open dashboard, without a reload", async () => {
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: [
      "engineering",
    ], total: 1 });
    vi.mocked(customClient.fetchDashboard)
      .mockResolvedValueOnce({ title: "Engineering", widgets: [] })
      .mockResolvedValue({ title: "Engineering", widgets: ["revenue_table"] });
    vi.mocked(customClient.fetchWidget).mockResolvedValue({
      type: "table",
      metric: "revenue_per_day",
      columns: ["day", "revenue"],
    });
    vi.mocked(customClient.runMetric).mockResolvedValue({
      columns: ["day", "revenue"],
      rows: [["2026-09-01", 100]],
    });
    vi.mocked(customClient.sendChat).mockResolvedValue({
      reply: "Added a revenue widget",
      created: { widgets: ["revenue_table"] },
    });

    renderAt("/portal/custom/engineering");
    await screen.findByRole("heading", { name: "Engineering" });

    await userEvent.type(
      screen.getByTestId("chat-input"),
      "add revenue tracking"
    );
    await userEvent.click(screen.getByTestId("chat-send"));

    expect(
      await screen.findByRole("cell", { name: "100" })
    ).toBeInTheDocument();
  });

  it("refuses the zone to a caller without the admin role", async () => {
    signedInAs({ admin: false });
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({ names: [
      "engineering",
    ], total: 1 });

    renderAt("/portal/custom");

    expect(
      await screen.findByText(/administrators only/i)
    ).toBeInTheDocument();
    // Nothing of the zone renders, not even the list it would have shown.
    expect(screen.queryByTestId("chat-input")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: "Custom" })
    ).not.toBeInTheDocument();
  });
});
