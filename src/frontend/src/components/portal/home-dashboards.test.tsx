vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

const access = vi.hoisted(() => ({ dashboards: true, isAdmin: true }));
vi.mock("@/lib/portal/use-zone-nav", () => ({
  useZoneNav: () => ({ zones: access.dashboards ? [{ id: "custom" }] : [] }),
}));
vi.mock("@/queries/identity-me", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/queries/identity-me")>()),
  useIsAdmin: () => ({
    isAdmin: access.isAdmin,
    isPending: false,
    isError: false,
    retry: () => {},
  }),
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
  access.isAdmin = true;
});

describe("Home dashboards", () => {
  const region = () =>
    screen.getByRole("region", { name: "My dashboards · recent" });

  it("does not request dashboards when navigation excludes them", () => {
    access.dashboards = false;
    home();
    expect(screen.queryByRole("region")).toBeNull();
  });

  it("shows a member nothing, since only administrators own dashboards", () => {
    access.isAdmin = false;
    home();
    expect(screen.queryByRole("region")).toBeNull();
  });

  it("asks the service for the three own dashboards changed most recently, in its order", async () => {
    const asked: URLSearchParams[] = [];
    server.use(
      http.get(`${base}/dashboards`, ({ request }) => {
        asked.push(new URL(request.url).searchParams);
        return HttpResponse.json({
          items: ["two", "three", "four"].map((id) => ({
            id,
            name: id,
            owner_person_id: "p1",
            access: "owner",
            updated_at: "2026-09-01T00:00:00Z",
            visibility: "private",
            title: `Board ${id}`,
            tags: [],
          })),
          total: 7,
          limit: 3,
          offset: 0,
        });
      })
    );
    home();
    await within(region()).findByText("Board two");
    const shown = within(region())
      .getAllByRole("listitem")
      .map((item) =>
        within(item)
          .getByRole("link", { name: /^Open / })
          .getAttribute("href")
      );
    expect(shown).toEqual([
      "/portal/custom/two",
      "/portal/custom/three",
      "/portal/custom/four",
    ]);
    expect(
      asked.map((params) => [
        params.get("owned"),
        params.get("sort"),
        params.get("limit"),
      ])
    ).toEqual([["me", "recent", "3"]]);
    expect(
      within(region()).getByRole("button", { name: /New dashboard/i })
    ).toBeInTheDocument();
  });

  it("keeps creation available when the viewer owns no dashboards", async () => {
    server.use(
      http.get(`${base}/dashboards`, () =>
        HttpResponse.json({ items: [], total: 0, limit: 50, offset: 0 })
      )
    );
    home();
    expect(
      await screen.findByText("You own no dashboards yet.")
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /New dashboard/i })
    ).toBeInTheDocument();
  });

  it("retries a failed listing without blocking creation", async () => {
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
        HttpResponse.json({ items: [], total: 0, limit: 50, offset: 0 })
      )
    );
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(
      await screen.findByText("You own no dashboards yet.")
    ).toBeInTheDocument();
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
