vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});
vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return { ...actual, fetchDashboardRead: vi.fn() };
});
const OWNER = vi.hoisted(() => "01900000-0000-7000-8000-00000000000a");
vi.mock("@/queries/person-name", () => ({
  usePersonName: (personId: string | null) => ({
    isPending: false,
    data: personId === OWNER ? "Jane Doe" : null,
  }),
}));

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import type { DashboardListed } from "@/api/custom-client";

import { DashboardCard } from "./dashboard-card";

const LISTED: DashboardListed = {
  id: "delivery",
  name: "delivery",
  owner_person_id: null,
  access: "ownerless",
};

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
    access: "ownerless",
  });
}

beforeEach(() => {
  vi.resetAllMocks();
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

  it("names the owner, and no one for an ownerless dashboard", async () => {
    reading();

    const { unmount } = card({
      ...LISTED,
      owner_person_id: OWNER,
      access: "owner",
    });
    expect(await screen.findByText("Jane Doe")).toBeInTheDocument();
    unmount();

    card();
    await screen.findByText("Delivery");
    expect(screen.queryByText("Jane Doe")).toBeNull();
  });
});
