import { describe, expect, it, vi } from "vitest";

vi.mock("@/api/custom-client");

import * as customClient from "@/api/custom-client";

import {
  dashboardNamesQuery,
  dashboardQuery,
  definitionPagesQuery,
  metricResultQuery,
  widgetQuery,
} from "./custom";

describe("dashboardNamesQuery", () => {
  it("passes a needle through, and keys the cache by it", async () => {
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({
      names: ["engineering"],
      total: 1,
    });

    const searched = dashboardNamesQuery("git");

    await searched.queryFn?.(undefined as never);
    expect(customClient.fetchDashboardNames).toHaveBeenCalledWith({
      search: "git",
      limit: 200,
    });
    expect(searched.queryKey).not.toEqual(dashboardNamesQuery().queryKey);
  });

  it("asks fetchDashboardNames for its data", async () => {
    const page = { names: ["engineering"], total: 1 };
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue(page);

    const options = dashboardNamesQuery();

    await expect(options.queryFn?.(undefined as never)).resolves.toEqual(page);
    expect(customClient.fetchDashboardNames).toHaveBeenCalledWith({
      search: "",
      limit: 200,
    });
  });
});

describe("definitionPagesQuery", () => {
  it("asks for one page at a time, and stops once it has them all", async () => {
    vi.mocked(customClient.fetchMetricNames).mockResolvedValue({
      names: ["a", "b"],
      total: 3,
    });

    const options = definitionPagesQuery("metrics", "git");

    await options.queryFn?.({ pageParam: 0 } as never);
    expect(customClient.fetchMetricNames).toHaveBeenCalledWith({
      search: "git",
      limit: 50,
      offset: 0,
    });

    // Two of three read, so the next page starts at the third.
    expect(
      options.getNextPageParam({ names: ["a", "b"], total: 3 }, [
        { names: ["a", "b"], total: 3 },
      ], 0, [0]),
    ).toBe(2);
    expect(
      options.getNextPageParam({ names: ["c"], total: 3 }, [
        { names: ["a", "b"], total: 3 },
        { names: ["c"], total: 3 },
      ], 2, [0, 2]),
    ).toBeUndefined();
  });

  it("keys the cache by kind and needle", () => {
    expect(definitionPagesQuery("metrics", "git").queryKey).not.toEqual(
      definitionPagesQuery("widgets", "git").queryKey,
    );
    expect(definitionPagesQuery("metrics", "git").queryKey).not.toEqual(
      definitionPagesQuery("metrics", "").queryKey,
    );
  });
});

describe("dashboardQuery", () => {
  it("keys on the dashboard name and asks fetchDashboard", async () => {
    const dashboard = { title: "Engineering", widgets: ["commits_table"] };
    vi.mocked(customClient.fetchDashboard).mockResolvedValue(dashboard);

    const options = dashboardQuery("engineering");

    await expect(options.queryFn?.(undefined as never)).resolves.toEqual(
      dashboard,
    );
    expect(customClient.fetchDashboard).toHaveBeenCalledWith("engineering");
    expect(dashboardQuery("engineering").queryKey).not.toEqual(
      dashboardQuery("delivery").queryKey,
    );
  });
});

describe("widgetQuery", () => {
  it("keys on the widget name and asks fetchWidget", async () => {
    const widget = { type: "table" as const, metric: "m", columns: ["day"] };
    vi.mocked(customClient.fetchWidget).mockResolvedValue(widget);

    const options = widgetQuery("commits_table");

    await expect(options.queryFn?.(undefined as never)).resolves.toEqual(
      widget,
    );
    expect(customClient.fetchWidget).toHaveBeenCalledWith("commits_table");
  });
});

describe("metricResultQuery", () => {
  it("keys on the metric name and asks runMetric", async () => {
    const result = { columns: ["day"], rows: [["2026-09-01"]] };
    vi.mocked(customClient.runMetric).mockResolvedValue(result);

    const options = metricResultQuery("commits_per_day");

    await expect(options.queryFn?.(undefined as never)).resolves.toEqual(
      result,
    );
    expect(customClient.runMetric).toHaveBeenCalledWith(
      "commits_per_day",
      undefined,
    );
  });
});

describe("metricResultQuery cache identity", () => {
  it("gives every window and bucket mode an entry of its own", () => {
    const keys = [
      metricResultQuery("commits"),
      metricResultQuery("commits", { range: "P30D" }),
      metricResultQuery("commits", { range: "P1Y" }),
      metricResultQuery("commits", { range: "P30D", bucket: false }),
    ].map((options) => JSON.stringify(options.queryKey));

    expect(new Set(keys).size).toBe(keys.length);
  });

  it("asks the transport for exactly what the key says", async () => {
    const result = { columns: ["total"], rows: [[2]] };
    vi.mocked(customClient.runMetric).mockResolvedValue(result);

    const options = metricResultQuery("commits", {
      range: "P30D",
      bucket: false,
    });
    await options.queryFn?.(undefined as never);

    expect(customClient.runMetric).toHaveBeenCalledWith("commits", {
      range: "P30D",
      bucket: false,
    });
  });
});
