vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});
vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return { ...actual, fetchDashboardRead: vi.fn() };
});
const OWNER = "01900000-0000-7000-8000-00000000000a";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import type { DashboardListed } from "@/api/custom-client";

import { DashboardCard } from "./dashboard-card";

const LISTED: DashboardListed = {
  id: "delivery",
  name: "delivery",
  owner_person_id: null,
  access: "ownerless",
  updated_at: "2026-09-10T09:00:00.000Z",
  visibility: "private",
};

const NOW = new Date("2026-09-10T12:00:00.000Z");
const HOUR_MS = 60 * 60 * 1000;

function changedBefore(ageMs: number): string {
  return new Date(NOW.getTime() - ageMs).toISOString();
}

function card(dashboard: DashboardListed = LISTED) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <DashboardCard dashboard={dashboard} />
    </QueryClientProvider>
  );
}

function reading() {
  vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
    name: "delivery",
    body: { title: "Delivery", widgets: [] },
    tags: [],
    ownerPersonId: null,
    ownerName: null,
    access: "ownerless",
    folders: [],
    carries: true,
  });
}

beforeEach(() => {
  vi.resetAllMocks();
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(NOW);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("<DashboardCard>", () => {
  it("opens the dashboard from its open link", async () => {
    reading();

    card();

    expect(
      await screen.findByRole("link", { name: "Open delivery" })
    ).toHaveAttribute("href", "/portal/custom/delivery");
  });

  it("offers no Edit or View beside the menu", async () => {
    reading();

    card();

    await screen.findByText("Delivery");
    expect(screen.queryByLabelText("Edit delivery")).toBeNull();
    expect(screen.queryByLabelText("View delivery")).toBeNull();
    expect(
      screen.getByRole("button", { name: "More for delivery" })
    ).toBeInTheDocument();
  });

  it("links by the dashboard's id, not its name", async () => {
    reading();

    card({ ...LISTED, id: "0190aaaa-0000-7000-8000-000000000001" });

    expect(
      await screen.findByRole("link", { name: "Open delivery" })
    ).toHaveAttribute(
      "href",
      "/portal/custom/0190aaaa-0000-7000-8000-000000000001"
    );
  });

  it("names the owner the listing names, and no one for an ownerless dashboard", async () => {
    reading();

    const { unmount } = card({
      ...LISTED,
      owner_person_id: OWNER,
      owner_name: "Jane Doe",
      access: "owner",
    });
    expect(await screen.findByText("Jane Doe")).toBeInTheDocument();
    unmount();

    card();
    await screen.findByText("Delivery");
    expect(screen.queryByText("Jane Doe")).toBeNull();
  });

  it.each([
    [2 * HOUR_MS, "2 h ago"],
    [30 * HOUR_MS, "yesterday"],
    [3 * 24 * HOUR_MS, "3 d ago"],
  ])("states a change %d ms back as %s", async (ageMs, shown) => {
    reading();

    card({ ...LISTED, updated_at: changedBefore(ageMs) });

    expect(await screen.findByText(shown)).toBeInTheDocument();
  });

  it("states no age for a stamp that is not a date", async () => {
    reading();

    card({ ...LISTED, updated_at: "not a date" });

    await screen.findByText("Delivery");
    expect(screen.queryByText(/ago$|^yesterday$/)).toBeNull();
  });

  it.each([
    ["private", "Private"],
    ["people", "Shared with people"],
    ["everyone", "Shared with org"],
  ] as const)("labels a %s dashboard %s for its owner", async (visibility, label) => {
    reading();

    card({ ...LISTED, access: "owner", visibility });

    expect(await screen.findByText(label)).toBeInTheDocument();
  });

  it("shows a viewer no sharing badge, since they cannot see its grants", async () => {
    reading();

    card({ ...LISTED, access: "viewer", visibility: "everyone" });

    await screen.findByText("Delivery");
    expect(screen.queryByText(/^Private$|^Shared with/)).toBeNull();
  });

  it("names the dashboard alone in its link, with age and badge beside it", async () => {
    reading();

    card({ ...LISTED, access: "owner" });

    await screen.findByText("Private");
    const [title] = screen
      .getAllByRole("link")
      .filter((link) => !link.getAttribute("aria-label"));
    expect(title?.textContent).toBeTruthy();
    expect(title?.textContent).not.toMatch(/ago|Private/);
  });

  it("draws title and tags straight from the listing, reading nothing per card", () => {
    card({ ...LISTED, title: "Delivery flow", tags: ["Ops", "Quality"] });

    expect(screen.getByText("Delivery flow")).toBeInTheDocument();
    expect(screen.getByText("Ops")).toBeInTheDocument();
    expect(screen.getByText("Quality")).toBeInTheDocument();
    expect(customClient.fetchDashboardRead).not.toHaveBeenCalled();
  });

  it("shows the name, not a stale key, when the listing has no title", () => {
    card({ ...LISTED, name: "Release notes", title: null, tags: [] });

    expect(screen.getByText("Release notes")).toBeInTheDocument();
    expect(customClient.fetchDashboardRead).not.toHaveBeenCalled();
  });
});
