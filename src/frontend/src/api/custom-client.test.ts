import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/api/fetch-with-auth", () => ({ fetchWithAuth: vi.fn() }));

import { fetchWithAuth } from "@/api/fetch-with-auth";

import {
  CustomApiError,
  createDefinition,
  createFolder,
  deleteDataset,
  deleteDefinition,
  deleteFolder,
  duplicateDashboard,
  fetchDashboardFolder,
  fetchDashboardRead,
  fetchDashboards,
  fetchDataset,
  fetchDatasetDependents,
  fetchDatasetRecords,
  fetchDefinition,
  fetchDependents,
  fetchDrilldownPage,
  fetchFolders,
  fetchMetric,
  fetchMetrics,
  fetchPins,
  fetchTable,
  fetchTags,
  fetchWidget,
  fetchWidgets,
  moveDashboard,
  pinDashboard,
  putDataset,
  putDefinition,
  renameDefinition,
  renameFolder,
  runMetric,
  setDashboardTags,
  unpinDashboard,
} from "./custom-client";

const mockFetch = fetchWithAuth as unknown as ReturnType<typeof vi.fn>;

function response(
  body: unknown,
  init?: { ok?: boolean; status?: number }
): Response {
  return {
    ok: init?.ok ?? true,
    status: init?.status ?? 200,
    json: async () => body,
  } as unknown as Response;
}

beforeEach(() => {
  mockFetch.mockReset();
});

describe("fetchDashboards", () => {
  it("reads the page of ids and names, and how many it is a page of", async () => {
    const page = {
      items: [
        { id: "engineering", name: "Engineering" },
        { id: "delivery", name: "Delivery" },
      ],
      total: 2,
    };
    mockFetch.mockResolvedValueOnce(response(page));

    await expect(fetchDashboards()).resolves.toEqual(page);
    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/dashboards");
  });

  it("puts the needle and the window in the query string", async () => {
    mockFetch.mockResolvedValueOnce(response({ items: [], total: 0 }));

    await fetchDashboards({ search: "git ops", limit: 50, offset: 50 });

    expect(mockFetch).toHaveBeenCalledWith(
      "/api/v3/v1/dashboards?q=git+ops&limit=50&offset=50"
    );
  });

  it("surfaces a failure as an API error", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ title: "forbidden" }, { ok: false, status: 403 })
    );

    await expect(fetchDashboards()).rejects.toBeInstanceOf(CustomApiError);
  });
});

describe("the metric and widget catalogues", () => {
  it("read a page of ids and names", async () => {
    const page = { items: [{ id: "m1", name: "Commits" }], total: 1 };
    mockFetch.mockResolvedValue(response(page));

    await expect(fetchMetrics({ search: "commit" })).resolves.toEqual(page);
    await expect(fetchWidgets()).resolves.toEqual(page);
    expect(mockFetch).toHaveBeenNthCalledWith(1, "/api/v3/v1/metrics?q=commit");
    expect(mockFetch).toHaveBeenNthCalledWith(2, "/api/v3/v1/widgets");
  });
});

describe("fetchDefinition", () => {
  it("reads one definition by id, with its name and body", async () => {
    const read = { id: "m1", name: "Commits", body: { dataset: "commits" } };
    mockFetch.mockResolvedValueOnce(response(read));

    await expect(fetchDefinition("metrics", "m1")).resolves.toEqual(read);
    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/metrics/m1");
  });

  it("encodes the id into the path", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ id: "a/b?c", name: "x", body: {} })
    );

    await fetchDefinition("dashboards", "a/b?c");

    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/dashboards/a%2Fb%3Fc");
  });

  it("surfaces a failure as an API error", async () => {
    mockFetch.mockResolvedValueOnce(response(null, { ok: false, status: 404 }));

    await expect(fetchDefinition("widgets", "nope")).rejects.toBeInstanceOf(
      CustomApiError
    );
  });
});

describe("fetchMetric", () => {
  it("reads a metric with its name and the clock a window over it would use", async () => {
    const definition = {
      dataset: "commits",
      fields: [{ agg: "count", type: "int", as_name: "total" }],
    };
    mockFetch.mockResolvedValueOnce(
      response({
        id: "commits_per_day",
        name: "Commits per day",
        body: definition,
        clock: { field: "occurred_at", from: "dataset" },
      })
    );

    await expect(fetchMetric("commits_per_day")).resolves.toEqual({
      name: "Commits per day",
      definition,
      clock: { field: "occurred_at", from: "dataset" },
    });
  });

  // A metric nothing dates is windowed by nothing, and the absent clock is
  // what says so - the body cannot, because the date may be the dataset's.
  it("reads a metric that nothing dates without one", async () => {
    const definition = {
      dataset: "commits",
      fields: [{ agg: "count", type: "int", as_name: "total" }],
    };
    mockFetch.mockResolvedValueOnce(
      response({ id: "all_commits", name: "All commits", body: definition })
    );

    await expect(fetchMetric("all_commits")).resolves.toEqual({
      name: "All commits",
      definition,
      clock: undefined,
    });
  });
});

describe("fetchWidget", () => {
  it("reads one widget by id, with its name, out of the envelope it arrives in", async () => {
    const widget = {
      type: "table",
      metric: "commits_per_day",
      columns: ["day", "lines"],
    };
    mockFetch.mockResolvedValueOnce(
      response({ id: "commits_table", name: "Commits table", body: widget })
    );

    await expect(fetchWidget("commits_table")).resolves.toEqual({
      name: "Commits table",
      widget,
    });
    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/widgets/commits_table");
  });

  it("surfaces a failure as an API error", async () => {
    mockFetch.mockResolvedValueOnce(response(null, { ok: false, status: 404 }));

    await expect(fetchWidget("nope")).rejects.toBeInstanceOf(CustomApiError);
  });
});

describe("runMetric", () => {
  it("posts to the run endpoint and reads the result", async () => {
    const result = { columns: ["day", "lines"], rows: [["2026-09-01", 59]] };
    mockFetch.mockResolvedValueOnce(response(result));

    await expect(runMetric("commits_per_day")).resolves.toEqual(result);
    expect(mockFetch).toHaveBeenCalledWith(
      "/api/v3/v1/metrics/commits_per_day/run",
      expect.objectContaining({ method: "POST" })
    );
  });

  it("surfaces a failure as an API error rather than an empty result", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ detail: "unknown table `evnts`" }, { ok: false, status: 400 })
    );

    await expect(runMetric("broken")).rejects.toBeInstanceOf(CustomApiError);
  });
});

describe("runMetric", () => {
  it("sends no body at all when nothing was picked", async () => {
    mockFetch.mockResolvedValueOnce(response({ columns: [], rows: [] }));

    await runMetric("commits");

    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/metrics/commits/run", {
      method: "POST",
    });
  });

  it("sends the range, the zone and the bucket the reader asked for", async () => {
    mockFetch.mockResolvedValueOnce(response({ columns: [], rows: [] }));

    await runMetric("commits", {
      range: "P30D",
      bucket: false,
    });

    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/metrics/commits/run", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        range: "P30D",
        bucket: false,
      }),
    });
  });
});

describe("fetchDatasetRecords", () => {
  // The service sizes the page: its cap is an installation's setting, and
  // asking for a size of our own is how a reader ends up stepping over what
  // a narrower page left behind.
  it("asks for no page size of its own", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ records: [], total: 0, limit: 20 })
    );

    await fetchDatasetRecords("commits", {});

    expect(mockFetch).toHaveBeenCalledWith(
      "/api/v3/v1/datasets/commits/records?"
    );
  });

  // With no field named the service orders by the instant a record arrived,
  // and honours the direction there too; omitting it asks for the default,
  // which is the opposite of ascending.
  it("sends the direction whether or not a field is named", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ records: [], total: 0, limit: 20 })
    );

    await fetchDatasetRecords("commits", { descending: false });

    expect(mockFetch).toHaveBeenCalledWith(
      "/api/v3/v1/datasets/commits/records?direction=asc"
    );
  });

  it("sends the field, the direction and the offset the reader is at", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ records: [], total: 0, limit: 20 })
    );

    await fetchDatasetRecords("commits", {
      offset: 40,
      orderBy: "day",
      descending: true,
    });

    expect(mockFetch).toHaveBeenCalledWith(
      "/api/v3/v1/datasets/commits/records?offset=40&order_by=day&direction=desc"
    );
  });
});

describe("an id or name that reached the client empty", () => {
  it.each([
    ["fetchDataset", () => fetchDataset("")],
    ["fetchMetric", () => fetchMetric("")],
    ["fetchWidget", () => fetchWidget("")],
    ["fetchDefinition", () => fetchDefinition("dashboards", "")],
    ["fetchDashboardFolder", () => fetchDashboardFolder("")],
    ["fetchDashboardRead", () => fetchDashboardRead("")],
    ["setDashboardTags", () => setDashboardTags("", [])],
    ["moveDashboard", () => moveDashboard("", null)],
    ["renameFolder", () => renameFolder("", "Product")],
    ["deleteFolder", () => deleteFolder("")],
    ["fetchDatasetRecords", () => fetchDatasetRecords("", { limit: 20 })],
    ["putDataset", () => putDataset("", {})],
    ["deleteDataset", () => deleteDataset("")],
    ["deleteDefinition", () => deleteDefinition("metrics", "")],
    ["renameDefinition", () => renameDefinition("metrics", "", "to")],
    ["runMetric", () => runMetric("")],
    ["putDefinition", () => putDefinition("metrics", "", "name", {})],
    ["duplicateDashboard", () => duplicateDashboard("", "copy")],
    ["fetchDrilldownPage", () => fetchDrilldownPage("", {})],
    ["fetchDependents", () => fetchDependents("metrics", "")],
    ["fetchDatasetDependents", () => fetchDatasetDependents("")],
    ["fetchTable (database)", () => fetchTable("", "commits")],
    ["fetchTable (table)", () => fetchTable("insight", "")],
  ])("is refused before %s asks for it", async (_name, call) => {
    await expect(call()).rejects.toBeInstanceOf(CustomApiError);
    expect(mockFetch).not.toHaveBeenCalled();
  });
});

describe("folders", () => {
  it("lists every folder with what it holds, and the unfiled count", async () => {
    const list = {
      folders: [{ id: "f1", name: "Platform", dashboards: 2 }],
      unfiled: 3,
    };
    mockFetch.mockResolvedValueOnce(response(list));

    await expect(fetchFolders()).resolves.toEqual(list);
    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/folders");
  });

  it("makes a folder from a name", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ id: "f1", name: "Platform" }, { status: 201 })
    );

    await expect(createFolder("Platform")).resolves.toEqual({
      id: "f1",
      name: "Platform",
    });
    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/folders", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: "Platform" }),
    });
  });

  it("refuses a name another folder holds with what the service said", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ detail: "taken" }, { ok: false, status: 409 })
    );

    await expect(createFolder("platform")).rejects.toMatchObject({
      status: 409,
      body: { detail: "taken" },
    });
  });

  it("renames a folder by its id", async () => {
    mockFetch.mockResolvedValueOnce(response({ id: "f1", name: "Product" }));

    await expect(renameFolder("f1", "Product")).resolves.toEqual({
      id: "f1",
      name: "Product",
    });
    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/folders/f1", {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: "Product" }),
    });
  });

  it("removes a folder by its id", async () => {
    mockFetch.mockResolvedValueOnce(response(null, { status: 204 }));

    await expect(deleteFolder("f1")).resolves.toBeUndefined();
    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/folders/f1", {
      method: "DELETE",
    });
  });

  it("moves a dashboard into a folder, and out of every folder", async () => {
    mockFetch.mockResolvedValue(response(null, { status: 204 }));

    await moveDashboard("delivery", "f1");
    await moveDashboard("delivery", null);

    expect(mockFetch).toHaveBeenNthCalledWith(
      1,
      "/api/v3/v1/dashboards/delivery/folder",
      {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ folder: "f1" }),
      }
    );
    expect(mockFetch).toHaveBeenNthCalledWith(
      2,
      "/api/v3/v1/dashboards/delivery/folder",
      {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ folder: null }),
      }
    );
  });

  it("reads the folder a dashboard is filed in, or none", async () => {
    const body = { title: "Delivery", items: [] };
    mockFetch
      .mockResolvedValueOnce(
        response({
          id: "delivery",
          name: "Delivery",
          body,
          folder: { id: "f1", name: "Platform" },
        })
      )
      .mockResolvedValueOnce(
        response({ id: "delivery", name: "Delivery", body, folder: null })
      );

    await expect(fetchDashboardFolder("delivery")).resolves.toEqual({
      id: "f1",
      name: "Platform",
    });
    await expect(fetchDashboardFolder("delivery")).resolves.toBeNull();
    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/dashboards/delivery");
  });

  it("asks for one folder's dashboards, or the unfiled ones", async () => {
    mockFetch.mockResolvedValue(response({ items: [], total: 0 }));

    await fetchDashboards({ folder: "unfiled", limit: 50 });

    expect(mockFetch).toHaveBeenCalledWith(
      "/api/v3/v1/dashboards?limit=50&folder=unfiled"
    );
  });
});

describe("tags", () => {
  it("lists every tag with how many dashboards carry it", async () => {
    const list = { tags: [{ name: "Ops", dashboards: 2 }] };
    mockFetch.mockResolvedValueOnce(response(list));

    await expect(fetchTags()).resolves.toEqual(list);
    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/tags");
  });

  it("sets a dashboard's whole tag set, and clears it", async () => {
    mockFetch.mockResolvedValue(response(null, { status: 204 }));

    await setDashboardTags("delivery", ["Ops", "Platform"]);
    await setDashboardTags("delivery", []);

    expect(mockFetch).toHaveBeenNthCalledWith(
      1,
      "/api/v3/v1/dashboards/delivery/tags",
      {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ tags: ["Ops", "Platform"] }),
      }
    );
    expect(mockFetch).toHaveBeenNthCalledWith(
      2,
      "/api/v3/v1/dashboards/delivery/tags",
      {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ tags: [] }),
      }
    );
  });

  it("refuses a set past the bounds with what the service said", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ detail: "too many" }, { ok: false, status: 409 })
    );

    await expect(setDashboardTags("delivery", ["Ops"])).rejects.toMatchObject({
      status: 409,
      body: { detail: "too many" },
    });
  });

  it("reads a dashboard with the tags it carries, or none", async () => {
    const body = { title: "Delivery", widgets: [] };
    const read = { id: "delivery", name: "Delivery", body, folder: null };
    mockFetch
      .mockResolvedValueOnce(response({ ...read, tags: ["Ops"] }))
      .mockResolvedValueOnce(response(read));

    await expect(fetchDashboardRead("delivery")).resolves.toEqual({
      name: "Delivery",
      body,
      tags: ["Ops"],
    });
    await expect(fetchDashboardRead("delivery")).resolves.toEqual({
      name: "Delivery",
      body,
      tags: [],
    });
    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/dashboards/delivery");
  });

  it("asks for the dashboards carrying any of the tags, inside a folder", async () => {
    mockFetch.mockResolvedValue(response({ items: [], total: 0 }));

    await fetchDashboards({
      search: "cycle",
      folder: "f1",
      tags: ["Ops", "R&D"],
      limit: 50,
    });

    expect(mockFetch).toHaveBeenCalledWith(
      "/api/v3/v1/dashboards?q=cycle&limit=50&folder=f1&tag=Ops&tag=R%26D"
    );
  });
});

describe("pins", () => {
  it("lists the caller's pinned dashboards by id and name, oldest pin first", async () => {
    const pins = [
      { id: "delivery", name: "Delivery" },
      { id: "0192f0aa-0000-7000-8000-000000000001", name: "Hiring" },
    ];
    mockFetch.mockResolvedValueOnce(response({ pins }));

    await expect(fetchPins()).resolves.toEqual(pins);
    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/pins");
  });

  it("pins and unpins a dashboard by its id", async () => {
    mockFetch.mockResolvedValue(response(null, { status: 204 }));

    await pinDashboard("R&D board");
    await unpinDashboard("R&D board");

    expect(mockFetch).toHaveBeenNthCalledWith(
      1,
      "/api/v3/v1/pins/R%26D%20board",
      { method: "PUT" }
    );
    expect(mockFetch).toHaveBeenNthCalledWith(
      2,
      "/api/v3/v1/pins/R%26D%20board",
      { method: "DELETE" }
    );
  });

  it("refuses a pin past the limit with what the service said", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ detail: "at most 20 pins" }, { ok: false, status: 409 })
    );

    await expect(pinDashboard("delivery")).rejects.toMatchObject({
      status: 409,
      body: { detail: "at most 20 pins" },
    });
  });

  it("refuses to unpin a dashboard that is not there", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ detail: "no such dashboard" }, { ok: false, status: 404 })
    );

    await expect(unpinDashboard("gone")).rejects.toMatchObject({
      status: 404,
    });
  });
});

describe("duplicateDashboard", () => {
  it("copies a dashboard under a new name, and reads the copy's id", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ id: "0192f0aa-0000-7000-8000-000000000002" }, { status: 201 })
    );

    await expect(duplicateDashboard("R&D", "Delivery copy")).resolves.toBe(
      "0192f0aa-0000-7000-8000-000000000002"
    );
    expect(mockFetch).toHaveBeenCalledWith(
      "/api/v3/v1/dashboards/R%26D/duplicate",
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: "Delivery copy" }),
      }
    );
  });

  it("refuses a copy of a dashboard that is not there with what the service said", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ detail: "not found" }, { ok: false, status: 404 })
    );

    await expect(
      duplicateDashboard("delivery", "Hiring")
    ).rejects.toMatchObject({ status: 404, body: { detail: "not found" } });
  });
});

describe("a dashboard's age", () => {
  it("reads when the dashboard was last changed, when the service says", async () => {
    const body = { title: "Delivery", widgets: [] };
    mockFetch
      .mockResolvedValueOnce(
        response({
          id: "delivery",
          name: "Delivery",
          body,
          tags: [],
          updated_at: "2026-09-25T10:00:00Z",
        })
      )
      .mockResolvedValueOnce(
        response({ id: "delivery", name: "Delivery", body, tags: [] })
      );

    await expect(fetchDashboardRead("delivery")).resolves.toMatchObject({
      updatedAt: "2026-09-25T10:00:00Z",
    });
    await expect(fetchDashboardRead("delivery")).resolves.not.toHaveProperty(
      "updatedAt"
    );
  });
});

describe("writing a definition", () => {
  const JSON_POST = { "Content-Type": "application/json" };

  it("creates one from a name and a body, and reads the id it was given", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ id: "0192f0aa-0000-7000-8000-000000000003" }, { status: 201 })
    );

    await expect(
      createDefinition("widgets", "Commits table", { type: "table" })
    ).resolves.toBe("0192f0aa-0000-7000-8000-000000000003");
    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/widgets", {
      method: "POST",
      headers: JSON_POST,
      body: JSON.stringify({ name: "Commits table", body: { type: "table" } }),
    });
  });

  it("replaces one by id with its name and its body", async () => {
    mockFetch.mockResolvedValueOnce(response(null, { status: 204 }));

    await putDefinition("metrics", "m1", "Commits", { dataset: "commits" });

    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/metrics/m1", {
      method: "PUT",
      headers: JSON_POST,
      body: JSON.stringify({ name: "Commits", body: { dataset: "commits" } }),
    });
  });

  it("renames one by replacing it under the new name with the body it holds", async () => {
    const body = { title: "Delivery", items: [] };
    mockFetch
      .mockResolvedValueOnce(response({ id: "d1", name: "Delivery", body }))
      .mockResolvedValueOnce(response(null, { status: 204 }));

    await renameDefinition("dashboards", "d1", "Delivery 2026");

    expect(mockFetch).toHaveBeenNthCalledWith(1, "/api/v3/v1/dashboards/d1");
    expect(mockFetch).toHaveBeenNthCalledWith(2, "/api/v3/v1/dashboards/d1", {
      method: "PUT",
      headers: JSON_POST,
      body: JSON.stringify({ name: "Delivery 2026", body }),
    });
  });

  it("does not replace a definition it could not read", async () => {
    mockFetch.mockResolvedValueOnce(response(null, { ok: false, status: 404 }));

    await expect(
      renameDefinition("dashboards", "gone", "New")
    ).rejects.toMatchObject({ status: 404 });
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it("removes one by id", async () => {
    mockFetch.mockResolvedValueOnce(response(null, { status: 204 }));

    await deleteDefinition("widgets", "w1");

    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/widgets/w1", {
      method: "DELETE",
    });
  });
});

describe("fetchDependents", () => {
  it("reads every holder by kind, id and name", async () => {
    const holders = [{ kind: "widgets", id: "w1", name: "Commits table" }];
    mockFetch.mockResolvedValueOnce(response({ holders }));

    await expect(fetchDependents("metrics", "m1")).resolves.toEqual(holders);
    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/metrics/m1/dependents");
  });
});

describe("fetchDrilldownPage", () => {
  it("reads one page of the rows behind a metric, by its id", async () => {
    const page = { selection: {}, columns: [], rows: [], next_cursor: null };
    mockFetch.mockResolvedValueOnce(response(page));

    await expect(fetchDrilldownPage("m1", { limit: 100 })).resolves.toEqual(
      page
    );
    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/metrics/m1/drilldown", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ limit: 100 }),
      signal: undefined,
    });
  });
});
