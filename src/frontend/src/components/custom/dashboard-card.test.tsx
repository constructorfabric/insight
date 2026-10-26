vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});
vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return { ...actual, fetchDashboardRead: vi.fn() };
});

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import type { Listed } from "@/api/custom-client";

import { DashboardCard } from "./dashboard-card";

const LISTED: Listed = { id: "delivery", name: "delivery" };

function card(dashboard: Listed = LISTED) {
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

  it("shows the name under a title that differs from it, and once when they match", async () => {
    reading();

    const { unmount } = card({ id: "d1", name: "Delivery board" });
    expect(await screen.findByText("Delivery")).toBeInTheDocument();
    expect(screen.getByText("Delivery board")).toBeInTheDocument();
    unmount();

    card({ id: "d1", name: "Delivery" });
    expect(await screen.findAllByText("Delivery")).toHaveLength(1);
  });
});
