vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return {
    ...portalRouterMock(),
    createFileRoute: () => (options: Record<string, unknown>) => options,
  };
});
vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return {
    ...actual,
    fetchDashboardRead: vi.fn(),
    fetchDashboardWidgets: vi.fn(),
    runDashboardWidget: vi.fn(),
    fetchDashboardDrilldownPage: vi.fn(),
    fetchMetric: vi.fn(),
    runMetric: vi.fn(),
    sendChat: vi.fn(),
    fetchPins: vi.fn(),
    fetchFolders: vi.fn(),
    removeFromFolder: vi.fn(),
  };
});
const role = vi.hoisted(() => ({ isAdmin: true }));
vi.mock("@/queries/identity-me", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/queries/identity-me")>()),
  useIsAdmin: () => ({
    isAdmin: role.isAdmin,
    isPending: false,
    isError: false,
    retry: () => {},
  }),
}));
vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({
        index,
        start: index * 44,
      })),
    getTotalSize: () => count * 44,
    measureElement: () => {},
  }),
}));

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import type { DashboardWidget, Widget } from "@/api/custom-client";
import { portalRouter } from "@/test/portal-router";

import { Route } from "./portal.custom.$name";

/** What every read here carries besides its body. */
const READ = {
  name: "engineering",
  tags: [],
  ownerPersonId: null,
  ownerName: null,
  visibility: null,
  access: "ownerless" as const,
  folders: [],
  carries: true,
};

const CLOCK = { field: "occurred_at", from: "dataset" as const };

const Component = (Route as unknown as { component: () => React.ReactNode })
  .component;

function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return createElement(QueryClientProvider, { client: queryClient }, children);
}

function drawn(
  id: string,
  body: Widget,
  clock: DashboardWidget["clock"] = null
): DashboardWidget {
  return { id, name: "Widget", body, clock };
}

const COMMITS_TABLE = drawn("commits_table", {
  type: "table",
  metric: "commits_per_day",
  columns: ["day", "lines"],
});

function boardOfCommits(extra: Partial<typeof READ> | object = {}) {
  vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
    ...READ,
    body: { title: "Engineering", widgets: ["commits_table"] },
    ...extra,
  });
  vi.mocked(customClient.fetchDashboardWidgets).mockResolvedValue([
    COMMITS_TABLE,
  ]);
  vi.mocked(customClient.runDashboardWidget).mockResolvedValue({
    columns: ["day", "lines"],
    rows: [["2026-09-01", 59]],
  });
  portalRouter.go("/portal/custom/engineering");
}

beforeEach(() => {
  vi.resetAllMocks();
  role.isAdmin = true;
  portalRouter.reset();
  vi.mocked(customClient.fetchDashboardWidgets).mockResolvedValue([]);
  vi.mocked(customClient.fetchFolders).mockResolvedValue({
    folders: [],
    unfiled: 0,
  });
});

describe("/portal/custom/$name", () => {
  it("draws each widget through the dashboard's own routes", async () => {
    boardOfCommits();

    render(<Component />, { wrapper });

    expect(
      await screen.findByRole("cell", { name: "2026-09-01" })
    ).toBeInTheDocument();
    expect(await screen.findByRole("cell", { name: "59" })).toBeInTheDocument();
    expect(customClient.fetchDashboardWidgets).toHaveBeenCalledWith(
      "engineering"
    );
    expect(customClient.runDashboardWidget).toHaveBeenCalledWith(
      "engineering",
      "commits_table",
      undefined
    );
    expect(customClient.runMetric).not.toHaveBeenCalled();
    expect(customClient.fetchMetric).not.toHaveBeenCalled();
  });

  it("opens the rows behind a widget for an administrator, through the dashboard", async () => {
    boardOfCommits();
    vi.mocked(customClient.fetchMetric).mockResolvedValue({
      name: "Commits per day",
      definition: { dataset: "commits", fields: [] },
    });
    vi.mocked(customClient.fetchDashboardDrilldownPage).mockResolvedValue({
      selection: {
        metric: "commits_per_day",
        sort: { key: "day", direction: "asc" },
      },
      columns: [],
      rows: [],
      next_cursor: null,
    });

    render(<Component />, { wrapper });

    const [more] = await screen.findAllByRole("button", {
      name: "More for Widget",
    });
    await userEvent.click(more as HTMLElement);
    await userEvent.click(
      await screen.findByRole("menuitem", { name: "Show data" })
    );

    await waitFor(() =>
      expect(customClient.fetchDashboardDrilldownPage).toHaveBeenCalledWith(
        "engineering",
        "commits_table",
        { limit: 100, cursor: undefined },
        expect.anything()
      )
    );
  });

  it("draws a member's dashboard with no way into the rows behind it", async () => {
    role.isAdmin = false;
    boardOfCommits({
      ownerPersonId: "01900000-0000-7000-8000-00000000000a",
      access: "viewer",
    });

    render(<Component />, { wrapper });

    expect(await screen.findByRole("cell", { name: "59" })).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "More for Widget" })
    ).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Edit engineering" })
    ).toBeNull();
    expect(customClient.fetchMetric).not.toHaveBeenCalled();
    expect(customClient.fetchDashboardDrilldownPage).not.toHaveBeenCalled();
  });

  it("says a widget the dashboard no longer draws cannot be read", async () => {
    vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
      ...READ,
      body: { title: "Engineering", widgets: ["gone"] },
    });
    portalRouter.go("/portal/custom/engineering");

    render(<Component />, { wrapper });

    expect(
      await screen.findByText("This widget is no longer on the dashboard.")
    ).toBeInTheDocument();
    expect(customClient.runDashboardWidget).not.toHaveBeenCalled();
  });

  it.each([
    ["one folder", [{ id: "f-mine", name: "Mine" }], "in Mine"],
    [
      "two folders",
      [
        { id: "f-mine", name: "Mine" },
        { id: "f-viewed", name: "Viewed" },
      ],
      "in Mine +1 and Viewed",
    ],
  ] as const)(
    "says in its byline which folder holds it, for %s",
    async (_case, folders, line) => {
      vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
        ...READ,
        body: { title: "Engineering", widgets: [] },
        folders: [...folders],
      });
      portalRouter.go("/portal/custom/engineering");

      render(<Component />, { wrapper });

      expect(
        await screen.findByRole("link", { name: "Mine" }),
        `case: ${_case}`
      ).toHaveAttribute("href", "/portal/custom?folder=f-mine");
      expect(screen.getByText(/^in/).textContent).toBe(line);
    }
  );

  it("names no folder for a dashboard filed nowhere", async () => {
    vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
      ...READ,
      body: { title: "Engineering", widgets: [] },
      folders: [],
    });
    portalRouter.go("/portal/custom/engineering");

    render(<Component />, { wrapper });

    await screen.findByRole("heading", { name: "Engineering" });
    expect(screen.queryByText(/^in /)).toBeNull();
  });

  it("offers Edit and the ··· menu in the header", async () => {
    vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
      ...READ,
      body: { title: "Engineering", widgets: [] },
    });
    portalRouter.go("/portal/custom/engineering");

    render(<Component />, { wrapper });

    const header = (
      await screen.findByRole("heading", { level: 1, name: "Engineering" })
    ).closest("header") as HTMLElement;
    expect(
      within(header).getByRole("button", { name: "Edit engineering" })
    ).toHaveAttribute("href", "/portal/custom/edit/dashboards/engineering");
    expect(
      within(header).getByRole("button", { name: "More for engineering" })
    ).toBeInTheDocument();
  });

  it("names the owner the read names, and offers a viewer neither Edit nor Share", async () => {
    vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
      ...READ,
      body: { title: "Engineering", widgets: [] },
      ownerPersonId: "01900000-0000-7000-8000-00000000000a",
      ownerName: "Jane Doe",
      visibility: null,
      access: "viewer",
    });
    portalRouter.go("/portal/custom/engineering");

    render(<Component />, { wrapper });

    const header = (
      await screen.findByRole("heading", { level: 1, name: "Engineering" })
    ).closest("header") as HTMLElement;
    expect(within(header).getByText("Jane Doe")).toBeInTheDocument();
    expect(within(header).queryByText(/Owner:/)).toBeNull();
    expect(
      within(header).queryByRole("button", { name: "Edit engineering" })
    ).toBeNull();
    expect(
      within(header).queryByRole("button", { name: "Share engineering" })
    ).toBeNull();
    expect(
      within(header).getByRole("button", { name: "More for engineering" })
    ).toBeInTheDocument();
  });

  it("reads the dashboard by the id in the path and heads it by name when it has no title", async () => {
    vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
      ...READ,
      name: "Platform board",
      body: { title: "", widgets: [] },
    });
    portalRouter.go("/portal/custom/0190aaaa-0000-7000-8000-000000000001");

    render(<Component />, { wrapper });

    expect(
      await screen.findByRole("heading", { level: 1, name: "Platform board" })
    ).toBeInTheDocument();
    expect(customClient.fetchDashboardRead).toHaveBeenCalledWith(
      "0190aaaa-0000-7000-8000-000000000001"
    );
  });

  it("draws items in order, with headings and prose between the widgets", async () => {
    vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
      ...READ,
      body: {
        title: "Engineering",
        items: [
          { heading: "Per person" },
          { widget: "commits_table" },
          { text: "Merge commits excluded." },
        ],
      },
    });
    vi.mocked(customClient.fetchDashboardWidgets).mockResolvedValue([
      drawn("commits_table", {
        type: "table",
        metric: "commits_per_day",
        columns: ["day"],
      }),
    ]);
    vi.mocked(customClient.runDashboardWidget).mockResolvedValue({
      columns: ["day"],
      rows: [["2026-09-01"]],
    });
    portalRouter.go("/portal/custom/engineering");

    render(<Component />, { wrapper });

    expect(
      await screen.findByRole("heading", { name: "Per person" })
    ).toBeInTheDocument();
    expect(
      await screen.findByText("Merge commits excluded.")
    ).toBeInTheDocument();
    expect(
      await screen.findByRole("cell", { name: "2026-09-01" })
    ).toBeInTheDocument();
  });

  it("shows a loading state before the dashboard resolves", () => {
    vi.mocked(customClient.fetchDashboardRead).mockReturnValue(
      new Promise(() => {})
    );
    portalRouter.go("/portal/custom/engineering");

    render(<Component />, { wrapper });

    expect(
      screen.getByRole("status", { name: /loading/i })
    ).toBeInTheDocument();
  });

  it("says a dashboard it cannot read is not there, with no retry offered", async () => {
    vi.mocked(customClient.fetchDashboardRead).mockRejectedValue(
      new customClient.CustomApiError(404, { title: "not found" })
    );
    portalRouter.go("/portal/custom/does-not-exist");

    render(<Component />, { wrapper });

    expect(
      await screen.findByText("That dashboard is not there.")
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /retry/i })
    ).not.toBeInTheDocument();
  });

  it("shows a retryable error state when the dashboard fails to load for another reason", async () => {
    vi.mocked(customClient.fetchDashboardRead).mockRejectedValue(
      new Error("network down")
    );
    portalRouter.go("/portal/custom/engineering");

    render(<Component />, { wrapper });

    expect(
      await screen.findByRole("button", { name: /retry/i })
    ).toBeInTheDocument();
  });
});

describe("/portal/custom/$name — the window it is read over", () => {
  const LINE: Widget = {
    type: "line",
    metric: "opened",
    x: "bucket",
    y: "total",
  };

  function board(extra: Record<string, unknown>, clocked = true) {
    vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
      ...READ,
      body: { title: "Engineering", widgets: ["opened_line"], ...extra },
    });
    vi.mocked(customClient.fetchDashboardWidgets).mockResolvedValue([
      drawn("opened_line", LINE, clocked ? CLOCK : null),
    ]);
    vi.mocked(customClient.runDashboardWidget).mockResolvedValue({
      columns: ["bucket", "total"],
      rows: [["2026-09-01", 2]],
    });
    portalRouter.go("/portal/custom/engineering");
  }

  const ran = (options: unknown) =>
    vi.waitFor(() => {
      expect(customClient.runDashboardWidget).toHaveBeenCalledWith(
        "engineering",
        "opened_line",
        options
      );
    });

  it("shows no picker, and asks for no window, when the board offers none", async () => {
    board({});

    render(<Component />, { wrapper });

    expect(
      await screen.findByRole("heading", { level: 1, name: "Engineering" })
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Last 30 days" })
    ).not.toBeInTheDocument();
    await ran(undefined);
  });

  it("opens on the board's default and asks the server for it", async () => {
    board({ time_ranges: ["PDC", "P30D"], default_range: "P30D" });

    render(<Component />, { wrapper });

    expect(
      await screen.findByRole("button", { name: "Last 30 days" })
    ).toHaveAttribute("aria-pressed", "true");
    await ran({ range: "P30D", bucket: true });
  });

  it("keeps the window picker pinned to the top while the board scrolls", async () => {
    board({ time_ranges: ["PDC", "P30D"], default_range: "P30D" });

    render(<Component />, { wrapper });

    const picker = await screen.findByRole("button", { name: "Last 30 days" });
    expect(picker.closest('[role="toolbar"]')).toHaveClass("sticky");
  });

  it("puts the reader's choice in the URL", async () => {
    board({ time_ranges: ["PDC", "P30D"], default_range: "P30D" });
    render(<Component />, { wrapper });

    await userEvent.click(
      await screen.findByRole("button", { name: "Yesterday" })
    );

    expect(portalRouter.search.range).toBe("PDC");
  });

  it("opens on the window the URL already names", async () => {
    board({ time_ranges: ["PDC", "P30D"], default_range: "P30D" });
    portalRouter.set({ range: "PDC" });

    render(<Component />, { wrapper });

    await ran({ range: "PDC", bucket: true });
  });

  it("runs a clockless widget over everything and says so on its face", async () => {
    board({ time_ranges: ["PDC", "P30D"], default_range: "P30D" }, false);

    render(<Component />, { wrapper });

    expect(await screen.findByText("All time")).toBeInTheDocument();
    await ran(undefined);
  });

  it("asks for a stat as one number over the whole window", async () => {
    vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
      ...READ,
      body: {
        title: "Engineering",
        widgets: ["merged_stat"],
        time_ranges: ["P30D"],
        default_range: "P30D",
      },
    });
    vi.mocked(customClient.fetchDashboardWidgets).mockResolvedValue([
      drawn(
        "merged_stat",
        { type: "stat", metric: "merged", value: "total", label: "Merged" },
        CLOCK
      ),
    ]);
    vi.mocked(customClient.runDashboardWidget).mockResolvedValue({
      columns: ["total"],
      rows: [[7]],
    });
    portalRouter.go("/portal/custom/engineering");

    render(<Component />, { wrapper });

    await vi.waitFor(() => {
      expect(customClient.runDashboardWidget).toHaveBeenCalledWith(
        "engineering",
        "merged_stat",
        { range: "P30D", bucket: false }
      );
    });
  });
});

describe("/portal/custom/$name — widgets that cannot be read", () => {
  it("says the widgets could not be read and offers a retry", async () => {
    vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
      ...READ,
      body: { title: "Engineering", widgets: ["opened_line"] },
    });
    vi.mocked(customClient.fetchDashboardWidgets).mockRejectedValue(
      new Error("down")
    );
    portalRouter.go("/portal/custom/engineering");

    render(<Component />, { wrapper });

    expect(await screen.findByRole("button", { name: /retry/i })).toBeVisible();
    expect(customClient.runDashboardWidget).not.toHaveBeenCalled();
  });
});

describe("/portal/custom/$name — a widget the service is too busy to run", () => {
  it("says it is busy and runs it again on retry", async () => {
    boardOfCommits();
    vi.mocked(customClient.runDashboardWidget).mockRejectedValueOnce(
      new customClient.CustomApiError(429, { detail: "busy" })
    );

    render(<Component />, { wrapper });

    const busy = await screen.findByRole("status", {
      name: "Busy, try again.",
    });
    await userEvent.click(within(busy).getByRole("button", { name: "Retry" }));

    expect(await screen.findByRole("cell", { name: "59" })).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(customClient.runDashboardWidget).toHaveBeenCalledTimes(2);
  });
});

describe("/portal/custom/$name — a widget expanded over the window", () => {
  function cachedWrapper({ children }: { children: ReactNode }) {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false, staleTime: Infinity } },
    });
    return createElement(
      QueryClientProvider,
      { client: queryClient },
      children
    );
  }

  it("expands a widget into a dialog named in the URL, on the rows and window the card already ran", async () => {
    vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
      ...READ,
      body: {
        title: "Engineering",
        widgets: ["commits_table"],
        time_ranges: ["P30D"],
        default_range: "P30D",
      },
    });
    vi.mocked(customClient.fetchDashboardWidgets).mockResolvedValue([
      { ...COMMITS_TABLE, clock: CLOCK },
    ]);
    vi.mocked(customClient.runDashboardWidget).mockResolvedValue({
      columns: ["day", "lines"],
      rows: [["2026-09-01", 59]],
    });
    portalRouter.go("/portal/custom/engineering");
    render(<Component />, { wrapper: cachedWrapper });

    expect(await screen.findByRole("cell", { name: "59" })).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Expand Widget" })
    );

    expect(portalRouter.search.widget).toBe("commits_table");
    const dialog = await screen.findByRole("dialog", { name: "Widget" });
    expect(
      within(dialog).getByRole("cell", { name: "59" })
    ).toBeInTheDocument();
    expect(customClient.runDashboardWidget).toHaveBeenCalledTimes(1);
    expect(customClient.runDashboardWidget).toHaveBeenCalledWith(
      "engineering",
      "commits_table",
      { range: "P30D", bucket: false }
    );
  });

  it("opens on the widget the URL already names", async () => {
    boardOfCommits();
    portalRouter.set({ widget: "commits_table" });
    render(<Component />, { wrapper });

    const dialog = await screen.findByRole("dialog");
    expect(
      await within(dialog).findByRole("cell", { name: "59" })
    ).toBeInTheDocument();
  });

  it("closes in place, so Back does not reopen it, and returns focus to Expand", async () => {
    boardOfCommits();
    render(<Component />, { wrapper });

    const expand = await screen.findByRole("button", { name: "Expand Widget" });
    await userEvent.click(expand);
    await screen.findByRole("dialog");
    await userEvent.keyboard("{Escape}");

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(portalRouter.search.widget).toBeUndefined();
    expect(portalRouter.navigations.at(-1)).toMatchObject({ replace: true });
    expect(expand).toHaveFocus();
  });

  it("opens nothing for a widget the board does not draw", async () => {
    boardOfCommits();
    portalRouter.set({ widget: "not_on_this_board" });
    render(<Component />, { wrapper });

    expect(await screen.findByRole("cell", { name: "59" })).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("offers a member the expanded view too", async () => {
    role.isAdmin = false;
    boardOfCommits({ access: "viewer" });
    render(<Component />, { wrapper });

    expect(
      await screen.findByRole("button", { name: "Expand Widget" })
    ).toBeInTheDocument();
  });

  it("never expands a stat, which is one number, even when the URL names it", async () => {
    vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
      ...READ,
      body: { title: "Engineering", widgets: ["merged_stat"] },
    });
    vi.mocked(customClient.fetchDashboardWidgets).mockResolvedValue([
      drawn("merged_stat", {
        type: "stat",
        metric: "merged",
        value: "total",
        label: "Merged",
      }),
    ]);
    vi.mocked(customClient.runDashboardWidget).mockResolvedValue({
      columns: ["total"],
      rows: [[7]],
    });
    portalRouter.go("/portal/custom/engineering");
    portalRouter.set({ widget: "merged_stat" });
    render(<Component />, { wrapper });

    expect(await screen.findByText("7")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Expand/ })).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
