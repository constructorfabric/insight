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

import { DashboardCard } from "./dashboard-card";

function card() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <DashboardCard name="delivery" />
    </QueryClientProvider>
  );
}

function reading() {
  vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
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
});
