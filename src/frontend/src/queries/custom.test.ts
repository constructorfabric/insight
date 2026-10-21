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
  orphanedPagesQuery,
  pinsQuery,
  sharesQuery,
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
  useShareDashboard,
  useStoreDefinition,
  useTransferDashboard,
  useUnshareDashboard,
  widgetQuery,
} from "./custom";

describe("definitionPagesQuery", () => {
  it("asks for one page at a time, and stops once it has them all", async () => {
    const first = {
      items: [
        { id: "a", name: "A" },
        { id: "b", name: "B" },
      ],
      total: 3,
    };
    const second = { items: [{ id: "c", name: "C" }], total: 3 };
    vi.mocked(customClient.fetchMetrics).mockResolvedValue(first);

    const options = definitionPagesQuery("metrics", "git");

    await options.queryFn?.({ pageParam: 0 } as never);
    expect(customClient.fetchMetrics).toHaveBeenCalledWith({
      search: "git",
      limit: 50,
      offset: 0,
    });

    // Two of three read, so the next page starts at the third.
    expect(options.getNextPageParam(first, [first], 0, [0])).toBe(2);
    expect(
      options.getNextPageParam(second, [first, second], 2, [0, 2]),
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

describe("datasets in a catalogue", () => {
  it("are listed under their name, which is their id", async () => {
    vi.mocked(customClient.fetchDatasetNames).mockResolvedValue({
      names: ["commits"],
      total: 1,
    });

    await expect(
      definitionPagesQuery("datasets").queryFn?.({ pageParam: 0 } as never),
    ).resolves.toEqual({ items: [{ id: "commits", name: "commits" }], total: 1 });
  });
});

describe("catalogueQuery", () => {
  it("offers every definition of a kind by id and name", async () => {
    const page = { items: [{ id: "w1", name: "Commits table" }], total: 1 };
    vi.mocked(customClient.fetchWidgets).mockResolvedValue(page);
    const options = catalogueQuery("widgets");

    const read = await options.queryFn?.(undefined as never);

    expect(customClient.fetchWidgets).toHaveBeenCalledWith({ limit: 200 });
    expect(options.select?.(read as never)).toEqual(page.items);
  });
});

describe("definitionBodyQuery", () => {
  it("opens a definition with its name and the body a save would send", async () => {
    vi.mocked(customClient.fetchDefinition).mockResolvedValue({
      id: "w1",
      name: "Commits table",
      body: { type: "table" },
    });

    await expect(
      definitionBodyQuery("widgets", "w1").queryFn?.(undefined as never),
    ).resolves.toEqual({ name: "Commits table", document: { type: "table" } });
    expect(customClient.fetchDefinition).toHaveBeenCalledWith("widgets", "w1");
  });

  it("opens a dataset with its declaration under its name", async () => {
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
      ownerPersonId: null,
      access: "ownerless",
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
    const stored = {
      name: "Commits table",
      widget: { type: "table" as const, metric: "m", columns: ["day"] },
    };
    vi.mocked(customClient.fetchWidget).mockResolvedValue(stored);

    const options = widgetQuery("commits_table");

    await expect(options.queryFn?.(undefined as never)).resolves.toEqual(
      stored,
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

    const options = definitionPagesQuery("dashboards", "", { folder: "f1" });
    await options.queryFn?.({ pageParam: 0 } as never);

    expect(customClient.fetchDashboards).toHaveBeenCalledWith({
      search: "",
      limit: 50,
      offset: 0,
      folder: "f1",
    });
    const keys = [
      definitionPagesQuery("dashboards", ""),
      definitionPagesQuery("dashboards", "", { folder: "f1" }),
      definitionPagesQuery("dashboards", "", { folder: "unfiled" }),
    ].map((query) => JSON.stringify(query.queryKey));
    expect(new Set(keys).size).toBe(keys.length);
  });
});

describe("definitionPagesQuery by ownership", () => {
  it("asks for the owned or shared dashboards and keeps them apart in the cache", async () => {
    vi.mocked(customClient.fetchDashboards).mockResolvedValue({
      items: [],
      total: 0,
    });

    await definitionPagesQuery("dashboards", "", { owned: "shared" }).queryFn?.(
      { pageParam: 0 } as never,
    );

    expect(customClient.fetchDashboards).toHaveBeenCalledWith({
      search: "",
      limit: 50,
      offset: 0,
      owned: "shared",
    });
    const keys = [
      definitionPagesQuery("dashboards", ""),
      definitionPagesQuery("dashboards", "", { owned: "me" }),
      definitionPagesQuery("dashboards", "", { owned: "shared" }),
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

    const options = definitionPagesQuery("dashboards", "", {
      folder: "f1",
      tags: ["Ops"],
    });
    await options.queryFn?.({ pageParam: 0 } as never);

    expect(customClient.fetchDashboards).toHaveBeenCalledWith({
      search: "",
      limit: 50,
      offset: 0,
      folder: "f1",
      tags: ["Ops"],
    });
    const keys = [
      definitionPagesQuery("dashboards", "", { folder: "f1" }),
      definitionPagesQuery("dashboards", "", { folder: "f1", tags: ["Ops"] }),
      definitionPagesQuery("dashboards", "", {
        folder: "f1",
        tags: ["Ops", "Platform"],
      }),
      definitionPagesQuery("dashboards", "", { tags: ["Ops"] }),
    ].map((query) => JSON.stringify(query.queryKey));
    expect(new Set(keys).size).toBe(keys.length);
  });

  it("asks without a tag filter when no tag is picked", async () => {
    vi.mocked(customClient.fetchDashboards).mockResolvedValue({
      items: [],
      total: 0,
    });

    await definitionPagesQuery("dashboards", "", { tags: [] }).queryFn?.({
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

  it("copies under the new name, answers the copy's id, then refreshes the lists and the counts", async () => {
    vi.mocked(customClient.duplicateDashboard).mockResolvedValueOnce("d2");
    const { result, invalidated } = rendered(() => useDuplicateDashboard());

    await expect(
      act(() => result.current.mutateAsync({ id: "delivery", to: "Delivery copy" })),
    ).resolves.toBe("d2");

    expect(customClient.duplicateDashboard).toHaveBeenCalledWith(
      "delivery",
      "Delivery copy",
    );
    expectListsAndCountsRefreshed(invalidated);
  });

  it("refreshes the lists and the counts after a copy is refused", async () => {
    vi.mocked(customClient.duplicateDashboard).mockRejectedValueOnce(
      new Error("refused"),
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
  it("renames by id and refreshes every cached read, keeping the id's entries", async () => {
    vi.mocked(customClient.renameDefinition).mockResolvedValue(undefined);
    const queryClient = new QueryClient({
      defaultOptions: { mutations: { retry: false } },
    });
    queryClient.setQueryData(["custom", "dashboard", "d1"], {
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

describe("useStoreDefinition", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("creates a definition with no id yet and answers the id it was given", async () => {
    vi.mocked(customClient.createDefinition).mockResolvedValue("w-new");
    const { result } = rendered(() => useStoreDefinition());

    await expect(
      act(() =>
        result.current.mutateAsync({
          kind: "widgets",
          name: "Commits table",
          body: { type: "table" },
        }),
      ),
    ).resolves.toBe("w-new");
    expect(customClient.createDefinition).toHaveBeenCalledWith(
      "widgets",
      "Commits table",
      { type: "table" },
    );
    expect(customClient.putDefinition).not.toHaveBeenCalled();
  });

  it("replaces a definition by its id, with its name, and answers that id", async () => {
    vi.mocked(customClient.putDefinition).mockResolvedValue(undefined);
    const { result } = rendered(() => useStoreDefinition());

    await expect(
      act(() =>
        result.current.mutateAsync({
          kind: "metrics",
          id: "m1",
          name: "Commits",
          body: { dataset: "commits" },
        }),
      ),
    ).resolves.toBe("m1");
    expect(customClient.putDefinition).toHaveBeenCalledWith(
      "metrics",
      "m1",
      "Commits",
      { dataset: "commits" },
    );
    expect(customClient.createDefinition).not.toHaveBeenCalled();
  });

  it("declares a dataset under its name, which is its id", async () => {
    vi.mocked(customClient.putDataset).mockResolvedValue({
      name: "commits",
      declaration: { title: "Commits", fields: [] },
    });
    const { result } = rendered(() => useStoreDefinition());

    await expect(
      act(() =>
        result.current.mutateAsync({
          kind: "datasets",
          name: "commits",
          body: { title: "Commits", fields: [] },
        }),
      ),
    ).resolves.toBe("commits");
    expect(customClient.putDataset).toHaveBeenCalledWith("commits", {
      title: "Commits",
      fields: [],
    });
  });
});

describe("sharing a dashboard", () => {
  const PERSON = "00000000-0000-4000-8000-000000000002";

  it("reads the shares of one dashboard under a key of its own", async () => {
    vi.mocked(customClient.fetchShares).mockResolvedValue([]);

    await sharesQuery("d1").queryFn?.(undefined as never);

    expect(customClient.fetchShares).toHaveBeenCalledWith("d1");
    expect(sharesQuery("d1").queryKey).not.toEqual(sharesQuery("d2").queryKey);
  });

  it("shares, then refreshes that dashboard's shares", async () => {
    const { result, invalidated } = rendered(() => useShareDashboard("d1"));

    await act(() => result.current.mutateAsync(PERSON));

    expect(customClient.shareDashboard).toHaveBeenCalledWith("d1", PERSON);
    expect(invalidated).toHaveBeenCalledWith({
      queryKey: sharesQuery("d1").queryKey,
    });
  });

  it("withdraws a share, then refreshes that dashboard's shares", async () => {
    const { result, invalidated } = rendered(() => useUnshareDashboard("d1"));

    await act(() => result.current.mutateAsync(PERSON));

    expect(customClient.unshareDashboard).toHaveBeenCalledWith("d1", PERSON);
    expect(invalidated).toHaveBeenCalledWith({
      queryKey: sharesQuery("d1").queryKey,
    });
  });
});

describe("orphaned dashboards", () => {
  it("are read a page at a time until all are in", async () => {
    const first = {
      items: [
        {
          id: "d1",
          name: "Release notes",
          owner_person_id: "p3",
          updated_at: "2026-09-25T10:00:00Z",
        },
      ],
      total: 2,
    };
    vi.mocked(customClient.fetchOrphanedDashboards).mockResolvedValue(first);
    const options = orphanedPagesQuery();

    await options.queryFn?.({ pageParam: 0 } as never);

    expect(customClient.fetchOrphanedDashboards).toHaveBeenCalledWith({
      limit: 50,
      offset: 0,
    });
    expect(options.getNextPageParam(first, [first], 0, [0])).toBe(1);
  });

  it("refreshes the orphaned list and the dashboard lists after a transfer", async () => {
    const { result, invalidated } = rendered(() => useTransferDashboard());

    await act(() => result.current.mutateAsync({ id: "d1", to: "p2" }));

    expect(customClient.transferDashboard).toHaveBeenCalledWith("d1", "p2");
    expect(invalidated).toHaveBeenCalledWith({
      queryKey: orphanedPagesQuery().queryKey,
    });
    expect(invalidated).toHaveBeenCalledWith({ queryKey: ["custom", "names"] });
  });
});
