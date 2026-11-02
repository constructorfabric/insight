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
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import { authStore } from "@/auth/auth-store";
import { makeSession } from "@/test/session";
import { routeTree } from "@/routeTree.gen";
import * as customClient from "@/api/custom-client";
import * as identityClient from "@/api/identity-client";
import { ADMIN_ROLE_ID } from "@/queries/identity-me";
import { setPortalShowPlanned } from "@/lib/portal/portal-store";

vi.mock("@/api/custom-client");
vi.mock("@/api/identity-client");

const LISTING = {
  items: [
    {
      id: "engineering",
      name: "engineering",
      owner_person_id: null,
      updated_at: "2026-09-01T00:00:00Z",
      visibility: "private" as const,
      access: "ownerless" as const,
    },
  ],
  total: 1,
};

const READ = {
  name: "engineering",
  ownerPersonId: null,
  ownerName: null,
  visibility: null,
  access: "ownerless" as const,
  folders: [],
  carries: true,
};

/** The caller `GET /v1/me` describes, which is what the zone gates on. */
function signedInAs({ admin }: { admin: boolean }) {
  vi.mocked(identityClient.getMe).mockResolvedValue({
    person_id: "00000000-0000-4000-8000-000000000001",
    insight_tenant_id: "00000000-0000-4000-8000-0000000000ff",
    roles: admin ? [{ role_id: ADMIN_ROLE_ID, name: "admin" }] : [],
    visibility_policy: "org_chart",
  });
}

/**
 * The assistant starts collapsed to a strip, and its composer is inside the
 * hidden panel until the reader opens it.
 */
async function openAssistant() {
  await userEvent.click(screen.getByRole("button", { name: "Open assistant" }));
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
  vi.mocked(customClient.fetchOrphanedDashboards).mockResolvedValue({
    items: [],
    total: 0,
  });
  vi.mocked(customClient.fetchOrphanedFolders).mockResolvedValue({
    items: [],
    total: 0,
  });
  // Opening the assistant is remembered per browser, and jsdom's storage
  // outlives a test.
  window.localStorage.clear();
  setPortalShowPlanned(true);
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
  setPortalShowPlanned(false);
  vi.unstubAllGlobals();
});

describe("the /portal/custom routes, through the real router", { timeout: 20_000 }, () => {
  it("renders the dashboard list at /portal/custom, inside the portal shell", async () => {
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(LISTING);

    renderAt("/portal/custom");

    const card = await screen.findByRole(
      "link",
      { name: "engineering" },
      { timeout: 10000 },
    );
    expect(card).toHaveAttribute("href", "/portal/custom/engineering");
    expect(screen.getByRole("link", { name: /All dashboards/ })).toHaveAttribute(
      "href",
      "/portal/custom",
    );
    expect(
      document.querySelector('[data-slot="sidebar-wrapper"]')
    ).toBeInTheDocument();
  });

  it("renders a dashboard at /portal/custom/$name, inside the portal shell", async () => {
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(LISTING);
    vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
      ...READ,
      body: { title: "Engineering", widgets: [] },
      tags: [],
    });

    renderAt("/portal/custom/engineering");

    expect(
      await screen.findByRole("heading", { name: "Engineering" }, { timeout: 10000 })
    ).toBeInTheDocument();
    expect(
      document.querySelector('[data-slot="sidebar-wrapper"]')
    ).toBeInTheDocument();
  });

  it("keeps the thread when creating a dashboard navigates to it", async () => {
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(LISTING);
    vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
      ...READ,
      body: { title: "Delivery", widgets: [] },
      tags: [],
    });
    vi.mocked(customClient.sendChat).mockResolvedValue({
      reply: "Built it",
      created: {
        metric: null,
        widgets: [],
        dashboard: { id: "delivery", name: "Delivery" },
      },
    });

    renderAt("/portal/custom");
    await screen.findByRole("heading", { name: "Dashboards" }, { timeout: 10000 });
    await openAssistant();

    await userEvent.type(
      screen.getByTestId("chat-input"),
      "a dashboard about delivery"
    );
    await userEvent.click(screen.getByTestId("chat-send"));

    // The new dashboard opens (its title is in the pane too, so the heading
    // is what says the page changed)...
    expect(
      await screen.findByRole("heading", { name: "Delivery" }, { timeout: 10000 })
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
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(LISTING);
    vi.mocked(customClient.fetchDashboardRead)
      .mockResolvedValueOnce({
        ...READ,
        body: { title: "Engineering", widgets: [] },
        tags: [],
      })
      .mockResolvedValue({
        ...READ,
        body: { title: "Engineering", widgets: ["revenue_table"] },
        tags: [],
      });
    vi.mocked(customClient.fetchDashboardWidgets).mockResolvedValue([
        {
          id: "revenue_table",
          name: "Revenue",
          body: {
            type: "table",
            metric: "revenue_per_day",
            columns: ["day", "revenue"],
          },
        },
      ]);
    vi.mocked(customClient.runDashboardWidget).mockResolvedValue({
      columns: ["day", "revenue"],
      rows: [["2026-09-01", 100]],
    });
    vi.mocked(customClient.sendChat).mockResolvedValue({
      reply: "Added a revenue widget",
      created: {
        metric: null,
        widgets: [{ id: "revenue_table", name: "Revenue" }],
        dashboard: null,
      },
    });

    renderAt("/portal/custom/engineering");
    await screen.findByRole("heading", { name: "Engineering" }, { timeout: 10000 });
    await openAssistant();

    await userEvent.type(
      screen.getByTestId("chat-input"),
      "add revenue tracking"
    );
    await userEvent.click(screen.getByTestId("chat-send"));

    expect(
      await screen.findByRole("cell", { name: "100" }, { timeout: 10000 })
    ).toBeInTheDocument();
  });

  it("lists orphaned dashboards and folders where their live ones are, for an administrator", async () => {
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(LISTING);
    vi.mocked(customClient.fetchOrphanedDashboards).mockResolvedValue({
      items: [
        {
          id: "release",
          name: "Release notes",
          owner_person_id: "01900000-0000-7000-8000-0000000000e0",
          updated_at: "2026-09-01T00:00:00.000Z",
        },
      ],
      total: 1,
    });
    vi.mocked(customClient.fetchOrphanedFolders).mockResolvedValue({
      items: [
        {
          id: "handover",
          name: "Handover",
          owner_person_id: "01900000-0000-7000-8000-0000000000e0",
          updated_at: "2026-08-15T00:00:00.000Z",
        },
      ],
      total: 1,
    });

    renderAt("/portal/custom");

    expect(
      await screen.findByRole(
        "heading",
        { level: 2, name: "Orphaned" },
        { timeout: 10000 },
      )
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Transfer Release notes" })
    ).toBeInTheDocument();
    expect(
      await screen.findByRole("button", { name: /Handover/ })
    ).toHaveTextContent("Orphaned");
  });

  it("opens the dashboard list to a member, without the library or the assistant", async () => {
    signedInAs({ admin: false });
    vi.mocked(customClient.fetchDashboards).mockResolvedValue({
      items: [{ ...LISTING.items[0]!, access: "viewer" as const }],
      total: 1,
    });
    vi.mocked(customClient.fetchFolders).mockResolvedValue({
      folders: [],
      unfiled: 1,
    });
    vi.mocked(customClient.fetchPins).mockResolvedValue([]);
    vi.mocked(customClient.fetchTags).mockResolvedValue({ tags: [] });

    renderAt("/portal/custom");

    expect(
      await screen.findByRole("link", { name: "engineering" }, { timeout: 10000 })
    ).toHaveAttribute("href", "/portal/custom/engineering");
    expect(
      within(screen.getByRole("navigation", { name: "Browse dashboards" })).getByRole(
        "link",
        { name: /Shared with me/ }
      )
    ).toBeInTheDocument();
    for (const hidden of [
      /My dashboards/,
      /^Metrics$/,
      /^Widgets$/,
      /^Datasets$/,
      /^Alerts$/,
    ]) {
      expect(
        screen.queryByRole("link", { name: hidden }),
        `should not link: ${hidden}`
      ).toBeNull();
    }
    expect(screen.queryByRole("button", { name: "New dashboard" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Open assistant" })).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Collapse assistant" })
    ).toBeNull();
    expect(screen.queryByTestId("chat-input")).not.toBeInTheDocument();
  });

  it("opens a shared dashboard to a member", async () => {
    signedInAs({ admin: false });
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(LISTING);
    vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
      ...READ,
      access: "viewer",
      body: { title: "Engineering", widgets: [] },
      tags: [],
    });
    vi.mocked(customClient.fetchDashboardWidgets).mockResolvedValue([]);

    renderAt("/portal/custom/engineering");

    expect(
      await screen.findByRole("heading", { name: "Engineering" }, { timeout: 10000 })
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Edit engineering" })).toBeNull();
  });

  it.each([
    "/portal/custom/metrics",
    "/portal/custom/widgets",
    "/portal/custom/datasets",
    "/portal/custom/alerts",
    "/portal/custom/new/dashboards",
    "/portal/custom/edit/dashboards/engineering",
  ])("refuses %s to a member", async (path) => {
    signedInAs({ admin: false });
    vi.mocked(customClient.fetchDashboards).mockResolvedValue(LISTING);

    renderAt(path);

    expect(
      await screen.findByText(
        "This page is open to administrators only.",
        undefined,
        { timeout: 10000 }
      )
    ).toBeInTheDocument();
    expect(customClient.fetchMetrics).not.toHaveBeenCalled();
    expect(customClient.fetchOrphanedDashboards).not.toHaveBeenCalled();
    expect(customClient.fetchDefinition).not.toHaveBeenCalled();
  });
});
