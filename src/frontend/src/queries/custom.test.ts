import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";

vi.mock("@/api/custom-client");

import * as customClient from "@/api/custom-client";

import {
  dashboardFolderQuery,
  dashboardQuery,
  definitionPagesQuery,
  foldersQuery,
  metricResultQuery,
  useCreateFolder,
  useDeleteFolder,
  useMoveDashboard,
  useRemoveDefinition,
  useRenameFolder,
  widgetQuery,
} from "./custom";

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

describe("definitionPagesQuery in a folder", () => {
  it("asks for that folder's dashboards and keeps them apart in the cache", async () => {
    vi.mocked(customClient.fetchDashboardNames).mockResolvedValue({
      names: [],
      total: 0,
    });

    const options = definitionPagesQuery("dashboards", "", "f1");
    await options.queryFn?.({ pageParam: 0 } as never);

    expect(customClient.fetchDashboardNames).toHaveBeenCalledWith({
      search: "",
      limit: 50,
      offset: 0,
      folder: "f1",
    });
    const keys = [
      definitionPagesQuery("dashboards", ""),
      definitionPagesQuery("dashboards", "", "f1"),
      definitionPagesQuery("dashboards", "", "unfiled"),
    ].map((query) => JSON.stringify(query.queryKey));
    expect(new Set(keys).size).toBe(keys.length);
  });
});

describe("foldersQuery", () => {
  it("asks fetchFolders", async () => {
    const list = { folders: [], unfiled: 0 };
    vi.mocked(customClient.fetchFolders).mockResolvedValue(list);

    await expect(foldersQuery().queryFn?.(undefined as never)).resolves.toEqual(
      list,
    );
  });
});

describe("dashboardFolderQuery", () => {
  it("asks for one dashboard's folder and is refreshed with the folders", async () => {
    vi.mocked(customClient.fetchDashboardFolder).mockResolvedValue(null);

    const options = dashboardFolderQuery("delivery");
    await options.queryFn?.(undefined as never);

    expect(customClient.fetchDashboardFolder).toHaveBeenCalledWith("delivery");
    expect(options.queryKey.slice(0, foldersQuery().queryKey.length)).toEqual(
      foldersQuery().queryKey,
    );
  });
});

describe("the folder mutations", () => {
  function rendered<T>(hook: () => T) {
    const queryClient = new QueryClient({
      defaultOptions: { mutations: { retry: false } },
    });
    const invalidated = vi.spyOn(queryClient, "invalidateQueries");
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(QueryClientProvider, { client: queryClient }, children);

    return { result: renderHook(hook, { wrapper }).result, invalidated };
  }

  function expectCountsAndListsRefreshed(
    invalidated: ReturnType<typeof vi.spyOn>,
  ) {
    expect(invalidated).toHaveBeenCalledWith({
      queryKey: foldersQuery().queryKey,
    });
    expect(invalidated).toHaveBeenCalledWith({
      queryKey: ["custom", "names"],
    });
  }

  it("makes a folder, then refreshes the counts and the lists", async () => {
    const { result, invalidated } = rendered(() => useCreateFolder());

    await act(() => result.current.mutateAsync("Platform"));

    expect(customClient.createFolder).toHaveBeenCalledWith("Platform");
    expectCountsAndListsRefreshed(invalidated);
  });

  it("renames a folder, then refreshes the counts and the lists", async () => {
    const { result, invalidated } = rendered(() => useRenameFolder());

    await act(() => result.current.mutateAsync({ id: "f1", name: "Product" }));

    expect(customClient.renameFolder).toHaveBeenCalledWith("f1", "Product");
    expectCountsAndListsRefreshed(invalidated);
  });

  it("removes a folder, then refreshes the counts and the lists", async () => {
    const { result, invalidated } = rendered(() => useDeleteFolder());

    await act(() => result.current.mutateAsync("f1"));

    expect(customClient.deleteFolder).toHaveBeenCalledWith("f1");
    expectCountsAndListsRefreshed(invalidated);
  });

  it("moves a dashboard, then refreshes the counts and the lists", async () => {
    const { result, invalidated } = rendered(() => useMoveDashboard());

    await act(() =>
      result.current.mutateAsync({ name: "delivery", folder: null }),
    );

    expect(customClient.moveDashboard).toHaveBeenCalledWith("delivery", null);
    expectCountsAndListsRefreshed(invalidated);
  });

  it.each([
    ["a folder another admin took the name of", () => useCreateFolder(), "Platform", customClient.createFolder],
    ["a move into a folder that went away", () => useMoveDashboard(), { name: "delivery", folder: "gone" }, customClient.moveDashboard],
  ] as const)("refreshes the counts and the lists after %s is refused", async (_case, hook, variables, call) => {
    vi.mocked(call).mockRejectedValueOnce(new Error("refused"));
    const { result, invalidated } = rendered(hook);

    await act(() =>
      (result.current.mutateAsync as (v: unknown) => Promise<unknown>)(variables).catch(() => undefined),
    );

    expectCountsAndListsRefreshed(invalidated);
  });

  it("refreshes the counts when a dashboard is removed", async () => {
    const { result, invalidated } = rendered(() => useRemoveDefinition());

    await act(() =>
      result.current.mutateAsync({ kind: "dashboards", name: "delivery" }),
    );

    expectCountsAndListsRefreshed(invalidated);
  });
});
