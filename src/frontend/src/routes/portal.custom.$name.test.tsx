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
    fetchDashboard: vi.fn(),
    fetchWidget: vi.fn(),
    fetchMetric: vi.fn(),
    runMetric: vi.fn(),
    sendChat: vi.fn(),
  };
});

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import { portalRouter } from "@/test/portal-router";

import { Route } from "./portal.custom.$name";

const Component = (Route as unknown as { component: () => React.ReactNode })
  .component;

function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return createElement(QueryClientProvider, { client: queryClient }, children);
}

beforeEach(() => {
  vi.resetAllMocks();
  portalRouter.reset();
});

describe("/portal/custom/$name", () => {
  it("renders a widget per name in the dashboard", async () => {
    vi.mocked(customClient.fetchDashboard).mockResolvedValue({
      title: "Engineering",
      widgets: ["commits_table"],
    });
    vi.mocked(customClient.fetchWidget).mockResolvedValue({
      type: "table",
      metric: "commits_per_day",
      columns: ["day", "lines"],
    });
    vi.mocked(customClient.runMetric).mockResolvedValue({
      columns: ["day", "lines"],
      rows: [["2026-09-01", 59]],
    });
    portalRouter.go("/portal/custom/engineering");

    render(<Component />, { wrapper });

    expect(await screen.findByText("Engineering")).toBeInTheDocument();
    expect(
      await screen.findByRole("cell", { name: "2026-09-01" })
    ).toBeInTheDocument();
    expect(await screen.findByRole("cell", { name: "59" })).toBeInTheDocument();
  });

  it("draws items in order, with headings and prose between the widgets", async () => {
    vi.mocked(customClient.fetchDashboard).mockResolvedValue({
      title: "Engineering",
      items: [
        { heading: "Per person" },
        { widget: "commits_table" },
        { text: "Merge commits excluded." },
      ],
    });
    vi.mocked(customClient.fetchWidget).mockResolvedValue({
      type: "table",
      metric: "commits_per_day",
      columns: ["day"],
    });
    vi.mocked(customClient.runMetric).mockResolvedValue({
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
    vi.mocked(customClient.fetchDashboard).mockReturnValue(
      new Promise(() => {})
    );
    portalRouter.go("/portal/custom/engineering");

    render(<Component />, { wrapper });

    expect(
      screen.getByRole("status", { name: /loading/i })
    ).toBeInTheDocument();
  });

  it("says an unknown dashboard name was not found, with no retry offered", async () => {
    vi.mocked(customClient.fetchDashboard).mockRejectedValue(
      new customClient.CustomApiError(404, { title: "not found" })
    );
    portalRouter.go("/portal/custom/does-not-exist");

    render(<Component />, { wrapper });

    expect(await screen.findByText(/no dashboard named/i)).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /retry/i })
    ).not.toBeInTheDocument();
  });

  it("shows a retryable error state when the dashboard fails to load for another reason", async () => {
    vi.mocked(customClient.fetchDashboard).mockRejectedValue(
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
  const CLOCKED = {
    table: "events",
    time: { column: "occurred_at" },
    fields: [{ agg: "count", type: "int", as_name: "total" }],
  };
  const CLOCKLESS = {
    table: "events",
    fields: [{ agg: "count", type: "int", as_name: "total" }],
  };

  function board(extra: Record<string, unknown>) {
    vi.mocked(customClient.fetchDashboard).mockResolvedValue({
      title: "Engineering",
      widgets: ["opened_line"],
      ...extra,
    });
    vi.mocked(customClient.fetchWidget).mockResolvedValue({
      type: "line",
      metric: "opened",
      x: "bucket",
      y: "total",
    });
    vi.mocked(customClient.runMetric).mockResolvedValue({
      columns: ["bucket", "total"],
      rows: [["2026-09-01", 2]],
    });
    portalRouter.go("/portal/custom/engineering");
  }

  it("shows no picker, and asks for no window, when the board offers none", async () => {
    board({});
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKED);

    render(<Component />, { wrapper });

    expect(await screen.findByText("Engineering")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Last 30 days" }),
    ).not.toBeInTheDocument();
    await vi.waitFor(() => {
      expect(customClient.runMetric).toHaveBeenCalledWith("opened", undefined);
    });
  });

  it("opens on the board's default and asks the server for it", async () => {
    board({ time_ranges: ["PDC", "P30D"], default_range: "P30D" });
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKED);

    render(<Component />, { wrapper });

    expect(
      await screen.findByRole("button", { name: "Last 30 days" }),
    ).toHaveAttribute("aria-pressed", "true");
    await vi.waitFor(() => {
      expect(customClient.runMetric).toHaveBeenCalledWith("opened", {
        range: "P30D",
        bucket: true,
      });
    });
  });

  it("puts the reader's choice in the URL", async () => {
    board({ time_ranges: ["PDC", "P30D"], default_range: "P30D" });
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKED);
    render(<Component />, { wrapper });

    await userEvent.click(
      await screen.findByRole("button", { name: "Yesterday" }),
    );

    expect(portalRouter.search.range).toBe("PDC");
  });

  it("opens on the window the URL already names", async () => {
    board({ time_ranges: ["PDC", "P30D"], default_range: "P30D" });
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKED);
    portalRouter.set({ range: "PDC" });

    render(<Component />, { wrapper });

    await vi.waitFor(() => {
      expect(customClient.runMetric).toHaveBeenCalledWith("opened", {
        range: "PDC",
        bucket: true,
      });
    });
  });

  it("runs a clockless widget over everything and says so on its face", async () => {
    board({ time_ranges: ["PDC", "P30D"], default_range: "P30D" });
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKLESS);

    render(<Component />, { wrapper });

    expect(await screen.findByText("All time")).toBeInTheDocument();
    await vi.waitFor(() => {
      expect(customClient.runMetric).toHaveBeenCalledWith("opened", undefined);
    });
  });

  it("does not run a clocked metric unwindowed when its definition cannot be read", async () => {
    board({ time_ranges: ["P30D"], default_range: "P30D" });
    vi.mocked(customClient.fetchMetric).mockRejectedValue(new Error("nope"));

    render(<Component />, { wrapper });

    expect(await screen.findByText("Engineering")).toBeInTheDocument();
    await vi.waitFor(() => {
      expect(customClient.fetchMetric).toHaveBeenCalled();
    });
    expect(customClient.runMetric).not.toHaveBeenCalled();
  });

  it("asks for a stat as one number over the whole window", async () => {
    vi.mocked(customClient.fetchDashboard).mockResolvedValue({
      title: "Engineering",
      widgets: ["merged_stat"],
      time_ranges: ["P30D"],
      default_range: "P30D",
    });
    vi.mocked(customClient.fetchWidget).mockResolvedValue({
      type: "stat",
      metric: "merged",
      value: "total",
      label: "Merged",
    });
    vi.mocked(customClient.fetchMetric).mockResolvedValue(CLOCKED);
    vi.mocked(customClient.runMetric).mockResolvedValue({
      columns: ["total"],
      rows: [[7]],
    });
    portalRouter.go("/portal/custom/engineering");

    render(<Component />, { wrapper });

    await vi.waitFor(() => {
      expect(customClient.runMetric).toHaveBeenCalledWith("merged", {
        range: "P30D",
        bucket: false,
      });
    });
  });
});

describe("/portal/custom/$name — a definition that cannot be read", () => {
  it("says the metric could not be read and offers a retry", async () => {
    vi.mocked(customClient.fetchDashboard).mockResolvedValue({
      title: "Engineering",
      widgets: ["opened_line"],
      time_ranges: ["P30D"],
      default_range: "P30D",
    });
    vi.mocked(customClient.fetchWidget).mockResolvedValue({
      type: "line",
      metric: "opened",
      x: "bucket",
      y: "opened",
    });
    vi.mocked(customClient.fetchMetric).mockRejectedValue(new Error("down"));
    portalRouter.go("/portal/custom/engineering");

    render(<Component />, { wrapper });

    expect(
      await screen.findByRole("button", { name: /retry/i }),
    ).toBeVisible();
    expect(customClient.runMetric).not.toHaveBeenCalled();
  });
});
