vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return {
    ...actual,
    fetchMetric: vi.fn(),
    fetchWidget: vi.fn(),
    fetchDrilldownPage: vi.fn(),
  };
});
vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({
        index,
        start: index * 44,
      })),
    getTotalSize: () => count * 44,
    measureElement: vi.fn(),
  }),
}));

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import { CustomApiError, type DrilldownPage, type Widget } from "@/api/custom-client";

import { WidgetDrilldown } from "./widget-drilldown";

function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return createElement(QueryClientProvider, { client: queryClient }, children);
}

/** A metric a window can select by, and the date it would use. */
const CLOCKED = {
  definition: {
    dataset: "pull_requests",
    time: { field: "merged_at" },
    fields: [{ field: "id", type: "int", agg: "count", as_name: "merged" }],
  },
  clock: { field: "merged_at", from: "metric" as const },
};

/** The same metric over a dataset that marks no date: nothing windows it. */
const CLOCKLESS = {
  definition: {
    dataset: "pull_requests",
    fields: [{ field: "id", type: "int", agg: "count", as_name: "total" }],
  },
};

const COLUMNS: DrilldownPage["columns"] = [
  { key: "service", label: "service", type: "string", sortable: true, percent: false },
  { key: "commits", label: "commits", type: "number", sortable: true, percent: false },
];

function page(
  rows: Record<string, unknown>[],
  overrides: Partial<DrilldownPage> = {}
): DrilldownPage {
  return {
    selection: { metric: "m", sort: { key: "service", direction: "asc" } },
    columns: COLUMNS,
    rows: rows.map((values) => ({ values })),
    next_cursor: null,
    ...overrides,
  };
}

const A_WIDGET: Widget = { type: "table", metric: "m", columns: ["service"] };

function renderDrilldown(
  widget: Widget = A_WIDGET,
  options?: { range: string; bucket: boolean }
) {
  return render(
    <WidgetDrilldown
      widget={widget}
      name="services"
      label="Service health"
      open
      onOpenChange={vi.fn()}
      options={options}
    />,
    { wrapper }
  );
}

/** A page whose answer the test releases when it is ready. */
function deferred<T>() {
  let resolve: (value: T) => void = () => {};
  const promise = new Promise<T>((settle) => {
    resolve = settle;
  });
  return { promise, resolve };
}

beforeEach(() => {
  vi.resetAllMocks();
});

describe("<WidgetDrilldown>", () => {
  it("reads the window the card read, so the two cannot disagree", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKED);
    vi.mocked(customClient.fetchDrilldownPage).mockResolvedValue(page([]));

    renderDrilldown(
      { type: "stat", metric: "merged", value: "merged", label: "Merged" },
      { range: "P30D", bucket: false }
    );

    await waitFor(() => {
      expect(customClient.fetchDrilldownPage).toHaveBeenCalledWith(
        "merged",
        { range: "P30D", bucket: false, limit: 100, cursor: undefined },
        expect.anything()
      );
    });
  });

  it("does not window a metric that carries no clock of its own", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKLESS);
    vi.mocked(customClient.fetchDrilldownPage).mockResolvedValue(page([]));

    renderDrilldown(
      { type: "stat", metric: "total", value: "total", label: "Total" },
      { range: "P30D", bucket: false }
    );

    await waitFor(() => {
      expect(customClient.fetchDrilldownPage).toHaveBeenCalledWith(
        "total",
        { limit: 100, cursor: undefined },
        expect.anything()
      );
    });
  });

  it("draws the page the service ordered, headers announcing that order", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKLESS);
    vi.mocked(customClient.fetchDrilldownPage).mockResolvedValue(
      page([
        { service: "example-api", commits: 96 },
        { service: "example-web", commits: 128 },
      ])
    );

    renderDrilldown();

    expect(await screen.findByText("example-api")).toBeInTheDocument();
    expect(screen.getByText("example-web")).toBeInTheDocument();
    expect(
      screen.getByRole("columnheader", { name: /service/ })
    ).toHaveAttribute("aria-sort", "ascending");
    expect(
      screen.getByRole("columnheader", { name: /commits/ })
    ).toHaveAttribute("aria-sort", "none");
  });

  // The card and the rows behind it are the same numbers, so they are
  // written the same way: grouped, and with the unit the metric declares.
  it("writes numbers the way the card writes them, percentages included", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKLESS);
    vi.mocked(customClient.fetchDrilldownPage).mockResolvedValue(
      page([{ service: "example-api", commits: 1234567, share: 83.91167 }], {
        columns: [
          ...COLUMNS,
          { key: "share", label: "share", type: "number", sortable: true, percent: true },
        ],
      })
    );

    renderDrilldown();

    expect(await screen.findByText("1,234,567")).toBeInTheDocument();
    expect(screen.getByText("83.9%")).toBeInTheDocument();
  });

  // A header click is a question to the service; the answer is the next
  // page it sends. The window travels with it, or the rows would be the
  // right order of the wrong period.
  it("asks the service for the order a header click chooses, over the same window", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKED);
    vi.mocked(customClient.fetchDrilldownPage).mockResolvedValue(
      page([{ service: "example-api", commits: 96 }])
    );

    renderDrilldown(
      { type: "table", metric: "merged", columns: ["service"] },
      { range: "P30D", bucket: false }
    );
    await screen.findByText("example-api");

    await user.click(screen.getByRole("button", { name: /commits/ }));

    await waitFor(() => {
      expect(customClient.fetchDrilldownPage).toHaveBeenLastCalledWith(
        "merged",
        {
          range: "P30D",
          bucket: false,
          sort: { key: "commits", direction: "asc" },
          limit: 100,
          cursor: undefined,
        },
        expect.anything()
      );
    });
  });

  // The service picked `service ↑` itself. The first click on that header
  // has to flip it, not ask for what is already on screen.
  it("flips the column the service chose on the first click", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKLESS);
    vi.mocked(customClient.fetchDrilldownPage).mockResolvedValue(
      page([{ service: "example-api", commits: 96 }])
    );

    renderDrilldown();
    await screen.findByText("example-api");

    const header = screen.getByRole("columnheader", { name: /service/ });
    await user.click(within(header).getByRole("button"));

    await waitFor(() => {
      expect(customClient.fetchDrilldownPage).toHaveBeenLastCalledWith(
        "m",
        { sort: { key: "service", direction: "desc" }, limit: 100, cursor: undefined },
        expect.anything()
      );
    });
  });

  it("keeps the rows on screen while a new order is on its way, then follows the answer", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKLESS);
    const reordered = deferred<DrilldownPage>();
    vi.mocked(customClient.fetchDrilldownPage)
      .mockResolvedValueOnce(page([{ service: "example-api", commits: 96 }]))
      .mockReturnValueOnce(reordered.promise);

    renderDrilldown();
    await screen.findByText("example-api");

    await user.click(screen.getByRole("button", { name: /commits/ }));

    expect(screen.getByText("example-api")).toBeInTheDocument();
    expect(
      screen.getByRole("columnheader", { name: /service/ })
    ).toHaveAttribute("aria-sort", "ascending");

    reordered.resolve(
      page([{ service: "example-cli", commits: 7 }], {
        selection: { metric: "m", sort: { key: "commits", direction: "asc" } },
      })
    );

    expect(await screen.findByText("example-cli")).toBeInTheDocument();
    expect(
      screen.getByRole("columnheader", { name: /commits/ })
    ).toHaveAttribute("aria-sort", "ascending");
  });

  it("asks for the next page with the cursor the last one handed back", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKLESS);
    vi.mocked(customClient.fetchDrilldownPage)
      .mockResolvedValueOnce(
        page([{ service: "example-api", commits: 96 }], { next_cursor: "c1" })
      )
      .mockResolvedValue(page([{ service: "example-cli", commits: 7 }]));

    renderDrilldown();

    expect(await screen.findByText("example-cli")).toBeInTheDocument();
    expect(customClient.fetchDrilldownPage).toHaveBeenLastCalledWith(
      "m",
      { limit: 100, cursor: "c1" },
      expect.anything()
    );
  });

  it("says what went wrong and offers to try again", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKLESS);
    vi.mocked(customClient.fetchDrilldownPage)
      .mockRejectedValueOnce(new CustomApiError(504, { detail: "metric query timed out" }))
      .mockResolvedValue(page([{ service: "example-api", commits: 96 }]));

    renderDrilldown();

    expect(await screen.findByRole("alert")).toHaveTextContent("metric query timed out");

    await user.click(screen.getByRole("button", { name: "Retry" }));

    expect(await screen.findByText("example-api")).toBeInTheDocument();
  });

  it("links a cell that is a URL, so a row can be followed", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKLESS);
    vi.mocked(customClient.fetchDrilldownPage).mockResolvedValue(
      page([{ service: "https://example.com/repo/app", commits: 1 }])
    );

    renderDrilldown();

    const link = await screen.findByRole("link", {
      name: "https://example.com/repo/app",
    });
    expect(link).toHaveAttribute("href", "https://example.com/repo/app");
  });

  it("says there is no data rather than drawing an empty table", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKLESS);
    vi.mocked(customClient.fetchDrilldownPage).mockResolvedValue(page([]));

    renderDrilldown();

    expect(await screen.findByText("No data.")).toBeInTheDocument();
  });

  // The dialog is sized for a result of a few columns. A wider one is read by
  // filling the window rather than by scrolling a small frame.
  it("fills the window when asked, and gives the room back", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKLESS);
    vi.mocked(customClient.fetchDrilldownPage).mockResolvedValue(
      page([{ service: "example-api", commits: 96 }])
    );

    renderDrilldown();

    const dialog = await screen.findByRole("dialog");
    const width = () => dialog.className;

    expect(width()).toContain("80rem");

    await user.click(screen.getByRole("button", { name: "Fill the window" }));

    expect(width()).toContain("98vw");
    expect(width()).not.toContain("80rem");

    await user.click(
      screen.getByRole("button", { name: "Shrink the dialog back" })
    );

    expect(width()).toContain("80rem");
  });
});
