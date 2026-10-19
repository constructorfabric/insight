vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

const access = vi.hoisted(() => ({ dashboards: true }));
vi.mock("@/lib/portal/use-zone-nav", () => ({
  useZoneNav: () => ({ zones: access.dashboards ? [{ id: "custom" }] : [] }),
}));

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import {
  afterAll,
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";

import "@/i18n";

import { HomeDashboards } from "./home-dashboards";

const server = setupServer();
const base = "*/api/v3/v1";

function home() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <HomeDashboards />
    </QueryClientProvider>
  );
}

beforeAll(() => server.listen({ onUnhandledRequest: "error" }));
afterEach(() => server.resetHandlers());
afterAll(() => server.close());
beforeEach(() => {
  access.dashboards = true;
});

describe("Home dashboards", () => {
  it("does not request dashboards when navigation excludes them", () => {
    access.dashboards = false;
    home();
    expect(screen.queryByRole("region", { name: "Dashboards" })).toBeNull();
  });

  it("shows at most three dashboards with existing open, menu and creation actions", async () => {
    server.use(
      http.get(`${base}/dashboards`, () =>
        HttpResponse.json({ names: ["one", "two", "three", "four"], total: 4 })
      ),
      http.get(`${base}/dashboards/:name`, ({ params }) =>
        HttpResponse.json({
          body: { title: `Board ${params.name}`, widgets: [] },
          tags: [],
        })
      )
    );
    home();
    const section = screen.getByRole("region", { name: "Dashboards" });
    await within(section).findByText("Board three");
    expect(within(section).getAllByRole("listitem")).toHaveLength(3);
    expect(within(section).queryByText("Board four")).toBeNull();
    expect(
      within(section).getByRole("link", { name: "Open one" })
    ).toHaveAttribute("href", "/portal/custom/one");
    expect(
      within(section).getByRole("button", { name: "More for one" })
    ).toBeInTheDocument();
    expect(
      within(section).getByRole("button", { name: /New dashboard/i })
    ).toHaveAttribute("href", "/portal/custom/new/dashboards");
    expect(
      within(section).getByRole("button", { name: "All dashboards" })
    ).toHaveAttribute("href", "/portal/custom");
    expect(
      screen.queryByText(/starter|private|shared with|recent/i)
    ).toBeNull();
  });

  it("keeps creation available when no dashboards exist", async () => {
    server.use(
      http.get(`${base}/dashboards`, () =>
        HttpResponse.json({ names: [], total: 0 })
      )
    );
    home();
    expect(await screen.findByText("No dashboards yet.")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /New dashboard/i })
    ).toBeInTheDocument();
  });

  it("retries a failed listing without blocking the creation link", async () => {
    server.use(
      http.get(
        `${base}/dashboards`,
        () => new HttpResponse(null, { status: 503 })
      )
    );
    home();
    expect(
      await screen.findByText("Couldn't load dashboards.")
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /New dashboard/i })
    ).toBeInTheDocument();
    server.use(
      http.get(`${base}/dashboards`, () =>
        HttpResponse.json({ names: [], total: 0 })
      )
    );
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("No dashboards yet.")).toBeInTheDocument();
  });

  it("keeps creation available while the listing loads", () => {
    server.use(http.get(`${base}/dashboards`, () => new Promise(() => {})));
    home();
    expect(screen.getByRole("status")).toHaveTextContent("Loading dashboards");
    expect(
      screen.getByRole("button", { name: /New dashboard/i })
    ).toBeInTheDocument();
  });
});
