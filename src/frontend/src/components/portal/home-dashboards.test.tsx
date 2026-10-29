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
import { render, screen, waitFor, within } from "@testing-library/react";
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

const board = (id: string, access = "viewer") => ({
  id,
  name: id,
  owner_person_id: "p1",
  access,
  updated_at: "2026-09-01T00:00:00Z",
  visibility: "everyone",
  title: `Board ${id}`,
  tags: [],
});

const listing = (items: ReturnType<typeof board>[]) =>
  HttpResponse.json({ items, total: items.length, limit: 3, offset: 0 });

describe("Home dashboards", () => {
  const region = () =>
    screen.getByRole("region", { name: "Recent dashboards" });

  it("does not request dashboards when navigation excludes them", () => {
    access.dashboards = false;
    home();
    expect(screen.queryByRole("region")).toBeNull();
  });

  it("asks the service for the three dashboards the viewer can see changed most recently, in its order", async () => {
    const asked: URLSearchParams[] = [];
    server.use(
      http.get(`${base}/dashboards`, ({ request }) => {
        asked.push(new URL(request.url).searchParams);
        return listing([board("two"), board("three"), board("four")]);
      })
    );
    home();
    await within(
      await screen.findByRole("region", { name: "Recent dashboards" })
    ).findByText("Board two");
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
    ).toEqual([[null, "recent", "3"]]);
  });

  it.each([
    ["an administrator", true, true],
    ["a member", false, false],
  ])(
    "offers creation to %s beside the dashboards shared with them",
    async (_case, isAdmin, offered) => {
      access.isAdmin = isAdmin;
      server.use(http.get(`${base}/dashboards`, () => listing([board("two")])));
      home();
      await within(
        await screen.findByRole("region", { name: "Recent dashboards" })
      ).findByText("Board two");
      expect(
        within(region()).queryByRole("button", { name: /New dashboard/i }) !==
          null,
        `case: ${_case}`
      ).toBe(offered);
    }
  );

  it("leaves the section out for a viewer with no dashboard to open", async () => {
    let answered = false;
    server.use(
      http.get(`${base}/dashboards`, () => {
        answered = true;
        return listing([]);
      })
    );
    home();
    await waitFor(() => expect(answered).toBe(true));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.queryByRole("region")).toBeNull();
  });

  it("draws nothing while the listing loads", () => {
    server.use(http.get(`${base}/dashboards`, () => new Promise(() => {})));
    home();
    expect(screen.queryByRole("region")).toBeNull();
  });

  it("retries a failed listing", async () => {
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
    server.use(http.get(`${base}/dashboards`, () => listing([board("two")])));
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("Board two")).toBeInTheDocument();
  });
});
