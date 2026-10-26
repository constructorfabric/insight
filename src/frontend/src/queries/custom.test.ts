import {
  QueryClient,
  QueryClientProvider,
  useQuery,
} from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/api/custom-client");

import * as customClient from "@/api/custom-client";

import {
  catalogueQuery,
  dashboardFolderQuery,
  dashboardQuery,
  dashboardTagsQuery,
  definitionBodyQuery,
  definitionPagesQuery,
  foldersQuery,
  metricResultQuery,
  pinsQuery,
  tagsQuery,
  useCreateFolder,
  useDeleteFolder,
  useDuplicateDashboard,
  useMoveDashboard,
  useRemoveDefinition,
  useRenameDefinition,
  useRenameFolder,
  useSetDashboardTags,
  useSetPinned,
  useStoreDefinition,
  widgetQuery,
} from "./custom";

describe("definitionPagesQuery", () => {
  it("asks for one page at a time, and stops once it has them all", async () => {
    const first = [
      { id: "a", name: "A" },
      { id: "b", name: "B" },
    ];
    const last = [{ id: "c", name: "C" }];
    vi.mocked(customClient.fetchMetrics).mockResolvedValue({
      items: first,
      total: 3,
    });

    const options = definitionPagesQuery("metrics", "git");

    await options.queryFn?.({ pageParam: 0 } as never);
    expect(customClient.fetchMetrics).toHaveBeenCalledWith({
      search: "git",
      limit: 50,
      offset: 0,
    });

    // Two of three read, so the next page starts at the third.
    expect(
      options.getNextPageParam({ items: first, total: 3 }, [
        { items: first, total: 3 },
      ], 0, [0]),
    ).toBe(2);
    expect(
      options.getNextPageParam({ items: last, total: 3 }, [
        { items: first, total: 3 },
        { items: last, total: 3 },
      ], 2, [0, 2]),
    ).toBeUndefined();
  });

  it("lists a dataset under its name, which is its id", async () => {
    vi.mocked(customClient.fetchDatasetNames).mockResolvedValue({
      names: ["commits"],
      total: 1,
    });

    await expect(
      definitionPagesQuery("datasets").queryFn?.({ pageParam: 0 } as never),
    ).resolves.toEqual({
      items: [{ id: "commits", name: "commits" }],
      total: 1,
    });
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

describe("catalogueQuery", () => {
  it("asks for every definition of a kind in one page, and keeps the ids and names", async () => {
    const items = [{ id: "w1", name: "Commits table" }];
    vi.mocked(customClient.fetchWidgets).mockResolvedValue({
      items,
      total: 1,
    });

    const options = catalogueQuery("widgets");
    const page = await options.queryFn?.(undefined as never);

    expect(customClient.fetchWidgets).toHaveBeenCalledWith({ limit: 200 });
    expect(page && options.select?.(page)).toEqual(items);
  });
});

describe("definitionBodyQuery", () => {
  it("opens a definition by id with its name and body", async () => {
    vi.mocked(customClient.fetchDefinition).mockResolvedValue({
      id: "m1",
      name: "Commits",
      body: { dataset: "commits" },
    });

    await expect(
      definitionBodyQuery("metrics", "m1").queryFn?.(undefined as never),
    ).resolves.toEqual({ name: "Commits", document: { dataset: "commits" } });
    expect(customClient.fetchDefinition).toHaveBeenCalledWith("metrics", "m1");
  });

  it("opens a dataset by its name with its declaration", async () => {
    vi.mocked(customClient.fetchDataset).mockResolvedValue({
      name: "commits",
      declaration: { title: "Commits", fields: [] },
    });

    await expect(
      definitionBodyQuery("datasets", "commits").queryFn?.(undefined as never),
    ).resolves.toEqual({
      name: "commits",
      document: { title: "Commits", fields: [] },
    });
  });
});

describe("useStoreDefinition", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("creates a definition with no id yet and answers with the id it was given", async () => {
    vi.mocked(customClient.createDefinition).mockResolvedValue("m-new");
    const { result } = rendered(() => useStoreDefinition());

    await expect(
      act(() =>
        result.current.mutateAsync({
          kind: "metrics",
          name: "Commits",
          body: { dataset: "commits" },
        }),
      ),
    ).resolves.toBe("m-new");
    expect(customClient.createDefinition).toHaveBeenCalledWith(
      "metrics",
      "Commits",
      { dataset: "commits" },
    );
    expect(customClient.putDefinition).not.toHaveBeenCalled();
  });

  it("replaces a definition by its id under the name it is given", async () => {
    const { result } = rendered(() => useStoreDefinition());

    await expect(
      act(() =>
        result.current.mutateAsync({
          kind: "widgets",
          id: "w1",
          name: "Renamed",
          body: { type: "stat" },
        }),
      ),
    ).resolves.toBe("w1");
    expect(customClient.putDefinition).toHaveBeenCalledWith(
      "widgets",
      "w1",
      "Renamed",
      { type: "stat" },
    );
  });

  it("stores a dataset under its name", async () => {
    const { result } = rendered(() => useStoreDefinition());

    await expect(
      act(() =>
        result.current.mutateAsync({
          kind: "datasets",
          name: "commits",
          body: { title: "Commits" },
        }),
      ),
    ).resolves.toBe("commits");
    expect(customClient.putDataset).toHaveBeenCalledWith("commits", {
      title: "Commits",
    });
  });
});

describe("dashboardQuery", () => {
  it("keys on the dashboard id", () => {
    expect(dashboardQuery("engineering").queryKey).not.toEqual(
      dashboardQuery("delivery").queryKey,
    );
  });

  it("reads the body and the tags out of one request", async () => {
    const dashboard = { title: "Engineering", widgets: ["commits_table"] };
    vi.mocked(customClient.fetchDashboardRead).mockResolvedValue({
      name: "Engineering",
      body: dashboard,
      tags: ["Ops"],
    });
    const queryClient = new QueryClient();
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(QueryClientProvider, { client: queryClient }, children);

    const { result } = renderHook(
      () => ({
        body: useQuery(dashboardQuery("engineering")).data,
        tags: useQuery(dashboardTagsQuery("engineering")).data,
      }),
      { wrapper },
    );

    await waitFor(() =>
      expect(result.current).toEqual({
        body: dashboard,
        tags: ["Ops"],
      }),
    );
    expect(customClient.fetchDashboardRead).toHaveBeenCalledTimes(1);
    expect(customClient.fetchDashboardRead).toHaveBeenCalledWith("engineering");
  });
});

describe("widgetQuery", () => {
  it("keys on the widget id and asks fetchWidget", async () => {
    const widget = {
      name: "Commits table",
      widget: { type: "table" as const, metric: "m", columns: ["day"] },
    };
    vi.mocked(customClient.fetchWidget).mockResolvedValue(widget);

    const options = widgetQuery("commits_table");

    await expect(options.queryFn?.(undefined as never)).resolves.toEqual(
      widget,
    );
    expect(customClient.fetchWidget).toHaveBeenCalledWith("commits_table");
  });
});

describe("metricResultQuery", () => {
  it("keys on the metric id and asks runMetric", async () => {
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
    vi.mocked(customClient.fetchDashboards).mockResolvedValue({
      items: [],
      total: 0,
    });

    const options = definitionPagesQuery("dashboards", "", "f1");
    await options.queryFn?.({ pageParam: 0 } as never);

    expect(customClient.fetchDashboards).toHaveBeenCalledWith({
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

function rendered<T>(hook: () => T) {
  const queryClient = new QueryClient({
    defaultOptions: { mutations: { retry: false } },
  });
  const invalidated = vi.spyOn(queryClient, "invalidateQueries");
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client: queryClient }, children);

  return { result: renderHook(hook, { wrapper }).result, invalidated };
}

describe("the folder mutations", () => {
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
      result.current.mutateAsync({ id: "delivery", folder: null }),
    );

    expect(customClient.moveDashboard).toHaveBeenCalledWith("delivery", null);
    expectCountsAndListsRefreshed(invalidated);
  });

  it("refreshes the counts and the lists after a taken name is refused", async () => {
    vi.mocked(customClient.createFolder).mockRejectedValueOnce(new Error("taken"));
    const { result, invalidated } = rendered(() => useCreateFolder());

    await act(() => result.current.mutateAsync("Platform").catch(() => undefined));

    expectCountsAndListsRefreshed(invalidated);
  });

  it("refreshes the counts and the lists after a move into a gone folder is refused", async () => {
    vi.mocked(customClient.moveDashboard).mockRejectedValueOnce(new Error("gone"));
    const { result, invalidated } = rendered(() => useMoveDashboard());

    await act(() =>
      result.current
        .mutateAsync({ id: "delivery", folder: "gone" })
        .catch(() => undefined),
    );

    expectCountsAndListsRefreshed(invalidated);
  });

  it("refreshes the counts when a dashboard is removed", async () => {
    const { result, invalidated } = rendered(() => useRemoveDefinition());

    await act(() =>
      result.current.mutateAsync({ kind: "dashboards", id: "delivery" }),
    );

    expectCountsAndListsRefreshed(invalidated);
  });
});

describe("definitionPagesQuery with tags", () => {
  it("asks for the dashboards carrying any of the tags and keeps them apart in the cache", async () => {
    vi.mocked(customClient.fetchDashboards).mockResolvedValue({
      items: [],
      total: 0,
    });

    const options = definitionPagesQuery("dashboards", "", "f1", ["Ops"]);
    await options.queryFn?.({ pageParam: 0 } as never);

    expect(customClient.fetchDashboards).toHaveBeenCalledWith({
      search: "",
      limit: 50,
      offset: 0,
      folder: "f1",
      tags: ["Ops"],
    });
    const keys = [
      definitionPagesQuery("dashboards", "", "f1"),
      definitionPagesQuery("dashboards", "", "f1", ["Ops"]),
      definitionPagesQuery("dashboards", "", "f1", ["Ops", "Platform"]),
      definitionPagesQuery("dashboards", "", undefined, ["Ops"]),
    ].map((query) => JSON.stringify(query.queryKey));
    expect(new Set(keys).size).toBe(keys.length);
  });

  it("asks without a tag filter when no tag is picked", async () => {
    vi.mocked(customClient.fetchDashboards).mockResolvedValue({
      items: [],
      total: 0,
    });

    await definitionPagesQuery("dashboards", "", undefined, []).queryFn?.({
      pageParam: 0,
    } as never);

    expect(customClient.fetchDashboards).toHaveBeenCalledWith({
      search: "",
      limit: 50,
      offset: 0,
    });
  });
});

describe("tagsQuery", () => {
  it("asks fetchTags", async () => {
    const list = { tags: [{ name: "Ops", dashboards: 1 }] };
    vi.mocked(customClient.fetchTags).mockResolvedValue(list);

    await expect(tagsQuery().queryFn?.(undefined as never)).resolves.toEqual(
      list,
    );
  });
});

describe("setting a dashboard's tags", () => {
  function expectTagsListsAndCardRefreshed(
    invalidated: ReturnType<typeof vi.spyOn>,
  ) {
    expect(invalidated).toHaveBeenCalledWith({
      queryKey: tagsQuery().queryKey,
    });
    expect(invalidated).toHaveBeenCalledWith({
      queryKey: ["custom", "names"],
    });
    expect(invalidated).toHaveBeenCalledWith({
      queryKey: dashboardQuery("delivery").queryKey,
    });
  }

  it("sends the whole set, then refreshes the tags, the lists and the dashboard", async () => {
    const { result, invalidated } = rendered(() => useSetDashboardTags());

    await act(() =>
      result.current.mutateAsync({ id: "delivery", tags: ["Ops"] }),
    );

    expect(customClient.setDashboardTags).toHaveBeenCalledWith("delivery", [
      "Ops",
    ]);
    expectTagsListsAndCardRefreshed(invalidated);
  });

  it("refreshes the tags, the lists and the dashboard after a refused set", async () => {
    vi.mocked(customClient.setDashboardTags).mockRejectedValueOnce(
      new Error("too many"),
    );
    const { result, invalidated } = rendered(() => useSetDashboardTags());

    await act(() =>
      result.current
        .mutateAsync({ id: "delivery", tags: ["Ops"] })
        .catch(() => undefined),
    );

    expectTagsListsAndCardRefreshed(invalidated);
  });

  it("refreshes the tags when a dashboard is removed", async () => {
    const { result, invalidated } = rendered(() => useRemoveDefinition());

    await act(() =>
      result.current.mutateAsync({ kind: "dashboards", id: "delivery" }),
    );

    expect(invalidated).toHaveBeenCalledWith({
      queryKey: tagsQuery().queryKey,
    });
  });
});

describe("pinsQuery", () => {
  it("asks fetchPins", async () => {
    const pins = [{ id: "delivery", name: "Delivery" }];
    vi.mocked(customClient.fetchPins).mockResolvedValue(pins);

    await expect(pinsQuery().queryFn?.(undefined as never)).resolves.toEqual(
      pins,
    );
  });
});

describe("pinning a dashboard", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("pins, then refreshes the pins", async () => {
    const { result, invalidated } = rendered(() => useSetPinned());

    await act(() =>
      result.current.mutateAsync({ id: "delivery", pinned: true }),
    );

    expect(customClient.pinDashboard).toHaveBeenCalledWith("delivery");
    expect(customClient.unpinDashboard).not.toHaveBeenCalled();
    expect(invalidated).toHaveBeenCalledWith({ queryKey: pinsQuery().queryKey });
  });

  it("unpins, then refreshes the pins", async () => {
    const { result, invalidated } = rendered(() => useSetPinned());

    await act(() =>
      result.current.mutateAsync({ id: "delivery", pinned: false }),
    );

    expect(customClient.unpinDashboard).toHaveBeenCalledWith("delivery");
    expect(customClient.pinDashboard).not.toHaveBeenCalled();
    expect(invalidated).toHaveBeenCalledWith({ queryKey: pinsQuery().queryKey });
  });

  it("refreshes the pins after a refused pin", async () => {
    vi.mocked(customClient.pinDashboard).mockRejectedValueOnce(
      new Error("too many"),
    );
    const { result, invalidated } = rendered(() => useSetPinned());

    await act(() =>
      result.current
        .mutateAsync({ id: "delivery", pinned: true })
        .catch(() => undefined),
    );

    expect(invalidated).toHaveBeenCalledWith({ queryKey: pinsQuery().queryKey });
  });

  it("refreshes the pins when a dashboard is removed", async () => {
    const { result, invalidated } = rendered(() => useRemoveDefinition());

    await act(() =>
      result.current.mutateAsync({ kind: "dashboards", id: "delivery" }),
    );

    expect(invalidated).toHaveBeenCalledWith({ queryKey: pinsQuery().queryKey });
  });
});

describe("duplicating a dashboard", () => {
  function expectListsAndCountsRefreshed(
    invalidated: ReturnType<typeof vi.spyOn>,
  ) {
    expect(invalidated).toHaveBeenCalledWith({ queryKey: ["custom", "names"] });
    expect(invalidated).toHaveBeenCalledWith({
      queryKey: foldersQuery().queryKey,
    });
    expect(invalidated).toHaveBeenCalledWith({ queryKey: tagsQuery().queryKey });
  }

  it("copies under the new name, then refreshes the lists and the counts", async () => {
    vi.mocked(customClient.duplicateDashboard).mockResolvedValueOnce("d-copy");
    const { result, invalidated } = rendered(() => useDuplicateDashboard());

    await expect(
      act(() =>
        result.current.mutateAsync({ id: "delivery", to: "Delivery copy" }),
      ),
    ).resolves.toBe("d-copy");

    expect(customClient.duplicateDashboard).toHaveBeenCalledWith(
      "delivery",
      "Delivery copy",
    );
    expectListsAndCountsRefreshed(invalidated);
  });

  it("refreshes the lists and the counts after a copy is refused", async () => {
    vi.mocked(customClient.duplicateDashboard).mockRejectedValueOnce(
      new Error("gone"),
    );
    const { result, invalidated } = rendered(() => useDuplicateDashboard());

    await act(() =>
      result.current
        .mutateAsync({ id: "delivery", to: "Hiring" })
        .catch(() => undefined),
    );

    expectListsAndCountsRefreshed(invalidated);
  });
});

describe("useRenameDefinition", () => {
  it("renames by id and refreshes what the custom zone has read", async () => {
    vi.mocked(customClient.renameDefinition).mockResolvedValue(undefined);
    const queryClient = new QueryClient({
      defaultOptions: { mutations: { retry: false } },
    });
    queryClient.setQueryData(["custom", "dashboard", "d1"], {
      name: "Engineering",
      body: {},
      tags: [],
    });
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(QueryClientProvider, { client: queryClient }, children);
    const { result } = renderHook(() => useRenameDefinition(), { wrapper });

    await act(() =>
      result.current.mutateAsync({
        kind: "dashboards",
        id: "d1",
        to: "Platform",
      }),
    );

    expect(customClient.renameDefinition).toHaveBeenCalledWith(
      "dashboards",
      "d1",
      "Platform",
    );
    expect(
      queryClient.getQueryState(["custom", "dashboard", "d1"])?.isInvalidated,
    ).toBe(true);
  });
});
