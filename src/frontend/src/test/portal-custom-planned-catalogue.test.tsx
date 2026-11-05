/**
 * An install that marks a catalogue planned, through the real router.
 *
 * Its own file because `navPolicy()` reads `/config.js` once and memoises the
 * answer, so the policy has to be in place before anything renders.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  RouterProvider,
  createMemoryHistory,
  createRouter,
} from "@tanstack/react-router";
import { act, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.hoisted(() => {
  window.__INSIGHT_CONFIG__ = { nav: { planned: ["zone:custom/item:alerts"] } };
});

import "@/i18n";
import * as alertsClient from "@/api/alerts-client";
import * as customClient from "@/api/custom-client";
import * as identityClient from "@/api/identity-client";
import { authStore } from "@/auth/auth-store";
import { setPortalShowPlanned } from "@/lib/portal/portal-store";
import { ADMIN_ROLE_ID } from "@/queries/identity-me";
import { routeTree } from "@/routeTree.gen";
import { makeSession } from "@/test/session";

vi.mock("@/api/alerts-client");
vi.mock("@/api/custom-client");
vi.mock("@/api/identity-client");

const WAIT = { timeout: 10000 };

function signedInAs({ admin }: { admin: boolean }) {
  vi.mocked(identityClient.getMe).mockResolvedValue({
    person_id: "00000000-0000-4000-8000-000000000001",
    insight_tenant_id: "00000000-0000-4000-8000-0000000000ff",
    roles: admin ? [{ role_id: ADMIN_ROLE_ID, name: "admin" }] : [],
    visibility_policy: "org_chart",
  });
}

function renderAt(path: string) {
  const history = createMemoryHistory({ initialEntries: [path] });
  const router = createRouter({ routeTree, history });
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  );
  return router;
}

async function expectAlertsPage() {
  expect(
    await screen.findByTestId("page-custom-alerts", {}, WAIT)
  ).toBeInTheDocument();
  await waitFor(() => expect(alertsClient.fetchAlerts).toHaveBeenCalled());
}

beforeEach(() => {
  vi.resetAllMocks();
  window.localStorage.clear();
  signedInAs({ admin: true });
  vi.mocked(customClient.fetchDashboards).mockResolvedValue({
    items: [],
    total: 0,
  });
  vi.mocked(customClient.fetchOrphanedDashboards).mockResolvedValue({
    items: [],
    total: 0,
  });
  vi.mocked(customClient.fetchOrphanedFolders).mockResolvedValue({
    items: [],
    total: 0,
  });
  vi.mocked(alertsClient.fetchAlerts).mockResolvedValue({
    alerts: [],
    total: 0,
    limit: 50,
    offset: 0,
  });
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
  authStore.setAuthenticated(makeSession({ personId: "" }));
});

afterEach(() => {
  authStore.reset();
  setPortalShowPlanned(false);
  vi.unstubAllGlobals();
});

describe("a catalogue the install marks planned", { timeout: 20_000 }, () => {
  it.each(["/portal/custom/alerts", "/portal/custom/alerts/new"])(
    "sends an administrator with planned sections off from %s to the dashboards",
    async (path) => {
      setPortalShowPlanned(false);

      const router = renderAt(path);

      await waitFor(
        () => expect(router.state.location.pathname).toBe("/portal/custom"),
        WAIT
      );
      expect(
        await screen.findByRole("link", { name: "Metrics" }, WAIT)
      ).toBeInTheDocument();
      expect(screen.queryByRole("link", { name: "Alerts" })).toBeNull();
      expect(alertsClient.fetchAlerts).not.toHaveBeenCalled();
    }
  );

  it("sends a member to the dashboards rather than naming the page", async () => {
    signedInAs({ admin: false });
    setPortalShowPlanned(false);

    const router = renderAt("/portal/custom/alerts");

    await waitFor(
      () => expect(router.state.location.pathname).toBe("/portal/custom"),
      WAIT
    );
    expect(
      screen.queryByText("This page is open to administrators only.")
    ).toBeNull();
  });

  it("opens, and lists it as planned, while planned sections are on", async () => {
    setPortalShowPlanned(true);

    const router = renderAt("/portal/custom/alerts");

    await expectAlertsPage();
    expect(screen.getByRole("link", { name: "Alerts" })).toHaveAttribute(
      "data-planned"
    );
    expect(screen.getByRole("link", { name: "Metrics" })).not.toHaveAttribute(
      "data-planned"
    );
    expect(router.state.location.pathname).toBe("/portal/custom/alerts");
  });

  it("leaves the page when the reader turns planned sections off", async () => {
    setPortalShowPlanned(true);
    const router = renderAt("/portal/custom/alerts");
    await expectAlertsPage();

    act(() => setPortalShowPlanned(false));

    await waitFor(
      () => expect(router.state.location.pathname).toBe("/portal/custom"),
      WAIT
    );
  });
});
