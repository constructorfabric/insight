import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/api/fetch-with-auth", () => ({ fetchWithAuth: vi.fn() }));

import { fetchWithAuth } from "@/api/fetch-with-auth";

import {
  CustomApiError,
  EVERYONE,
  addToFolder,
  createDefinition,
  createFolder,
  deleteDataset,
  deleteDefinition,
  deleteFolder,
  deleteGrant,
  duplicateDashboard,
  fetchDashboardDrilldownPage,
  fetchDashboardRead,
  fetchDashboardWidgets,
  fetchDashboards,
  fetchDataset,
  fetchDatasetDependents,
  fetchDatasetRecords,
  fetchDefinition,
  fetchDependents,
  fetchFolders,
  fetchGrants,
  fetchMetric,
  fetchMetrics,
  fetchOrphanedDashboards,
  fetchOrphanedFolders,
  fetchPins,
  fetchTags,
  fetchWidget,
  fetchWidgets,
  pinDashboard,
  putDataset,
  putDefinition,
  putGrant,
  removeFromFolder,
  renameDashboard,
  renameFolder,
  runDashboardWidget,
  runMetric,
  setDashboardTags,
  transferOrphan,
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
  it("reads the page, each dashboard with its owner and the caller's access", async () => {
    const page = {
      items: [
        {
          id: "engineering",
          name: "Engineering",
          owner_person_id: "00000000-0000-4000-8000-000000000001",
          access: "owner",
        },
        {
          id: "delivery",
          name: "Delivery",
          owner_person_id: null,
          access: "ownerless",
        },
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

  it.each(["me", "shared"] as const)(
    "narrows to the dashboards owned=%s",
    async (owned) => {
      mockFetch.mockResolvedValueOnce(response({ items: [], total: 0 }));

      await fetchDashboards({ owned, limit: 50 });

      expect(mockFetch).toHaveBeenCalledWith(
        `/api/v3/v1/dashboards?limit=50&owned=${owned}`
      );
    }
  );

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
    ["fetchDashboardRead", () => fetchDashboardRead("")],
    ["setDashboardTags", () => setDashboardTags("", [])],
    ["addToFolder (folder)", () => addToFolder("", "d1")],
    ["addToFolder (dashboard)", () => addToFolder("f1", "")],
    ["removeFromFolder", () => removeFromFolder("f1", "")],
    ["renameFolder", () => renameFolder("", "Product")],
    ["deleteFolder", () => deleteFolder("")],
    ["fetchDatasetRecords", () => fetchDatasetRecords("", { limit: 20 })],
    ["putDataset", () => putDataset("", {})],
    ["deleteDataset", () => deleteDataset("")],
    ["deleteDefinition", () => deleteDefinition("metrics", "")],
    ["renameDashboard", () => renameDashboard("", "to")],
    ["runMetric", () => runMetric("")],
    ["putDefinition", () => putDefinition("metrics", "", "name", {})],
    ["fetchGrants", () => fetchGrants({ kind: "dashboards", id: "" })],
    [
      "putGrant (dashboard)",
      () => putGrant({ kind: "dashboards", id: "" }, "p1", "viewer"),
    ],
    [
      "putGrant (grantee)",
      () => putGrant({ kind: "folders", id: "f1" }, "", "viewer"),
    ],
    ["deleteGrant", () => deleteGrant({ kind: "folders", id: "f1" }, "")],
    ["transferOrphan", () => transferOrphan("dashboards", "", "p1")],
    ["fetchDashboardWidgets", () => fetchDashboardWidgets("")],
    ["runDashboardWidget", () => runDashboardWidget("d1", "")],
    [
      "fetchDashboardDrilldownPage",
      () => fetchDashboardDrilldownPage("", "w1", {}),
    ],
    ["duplicateDashboard", () => duplicateDashboard("", "copy")],
    ["fetchDependents", () => fetchDependents("metrics", "")],
    ["fetchDatasetDependents", () => fetchDatasetDependents("")],
  ])("is refused before %s asks for it", async (_name, call) => {
    await expect(call()).rejects.toBeInstanceOf(CustomApiError);
    expect(mockFetch).not.toHaveBeenCalled();
  });
});

describe("folders", () => {
  it("lists every folder with what it holds, and the unfiled count", async () => {
    const list = {
      folders: [
        {
          id: "f1",
          name: "Platform",
          owner_person_id: "00000000-0000-4000-8000-000000000001",
          dashboards: 2,
        },
        { id: "f2", name: "Shared", owner_person_id: null, dashboards: 1 },
      ],
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

  it("adds a dashboard to a folder and reads whether the entry carries", async () => {
    mockFetch.mockResolvedValueOnce(response({ carries: true }));

    await expect(addToFolder("f1", "delivery")).resolves.toEqual({
      carries: true,
    });
    expect(mockFetch).toHaveBeenCalledWith(
      "/api/v3/v1/folders/f1/dashboards/delivery",
      { method: "PUT" }
    );
  });

  it("takes a dashboard out of one folder", async () => {
    mockFetch.mockResolvedValueOnce(response(null, { status: 204 }));

    await removeFromFolder("f1", "delivery");

    expect(mockFetch).toHaveBeenCalledWith(
      "/api/v3/v1/folders/f1/dashboards/delivery",
      { method: "DELETE" }
    );
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
    const read = {
      id: "delivery",
      name: "Delivery",
      body,
      owner_person_id: null,
      access: "ownerless",
    };
    const folders = [{ id: "f1", name: "Platform" }];
    mockFetch
      .mockResolvedValueOnce(
        response({
          ...read,
          owner_name: "Jane Doe",
          tags: ["Ops"],
          folders,
          carries: true,
        })
      )
      .mockResolvedValueOnce(response(read));

    await expect(fetchDashboardRead("delivery")).resolves.toEqual({
      name: "Delivery",
      body,
      tags: ["Ops"],
      ownerPersonId: null,
      ownerName: "Jane Doe",
      visibility: null,
      access: "ownerless",
      folders,
      carries: true,
    });
    await expect(fetchDashboardRead("delivery")).resolves.toEqual({
      name: "Delivery",
      body,
      tags: [],
      ownerPersonId: null,
      ownerName: null,
      visibility: null,
      access: "ownerless",
      folders: [],
      carries: false,
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

  it("refuses a copy the caller may not make with what the service said", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ detail: "not the owner" }, { ok: false, status: 403 })
    );

    await expect(
      duplicateDashboard("delivery", "Hiring")
    ).rejects.toMatchObject({ status: 403, body: { detail: "not the owner" } });
  });
});

describe("a dashboard's age", () => {
  it("reads when the dashboard was last changed, when the service says", async () => {
    const body = { title: "Delivery", widgets: [] };
    const read = {
      id: "delivery",
      name: "Delivery",
      body,
      tags: [],
      owner_person_id: "00000000-0000-4000-8000-000000000001",
      access: "viewer",
    };
    mockFetch
      .mockResolvedValueOnce(
        response({ ...read, updated_at: "2026-09-25T10:00:00Z" })
      )
      .mockResolvedValueOnce(response(read));

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

  it("renames a dashboard by replacing its name and title together, keeping the rest of its body", async () => {
    const body = { title: "Delivery", items: [{ widget: "w1" }] };
    mockFetch
      .mockResolvedValueOnce(response({ id: "d1", name: "Delivery", body }))
      .mockResolvedValueOnce(response(null, { status: 204 }));

    await renameDashboard("d1", "Delivery 2026");

    expect(mockFetch).toHaveBeenNthCalledWith(1, "/api/v3/v1/dashboards/d1");
    expect(mockFetch).toHaveBeenNthCalledWith(2, "/api/v3/v1/dashboards/d1", {
      method: "PUT",
      headers: JSON_POST,
      body: JSON.stringify({
        name: "Delivery 2026",
        body: { ...body, title: "Delivery 2026" },
      }),
    });
  });

  it("does not replace a definition it could not read", async () => {
    mockFetch.mockResolvedValueOnce(response(null, { ok: false, status: 404 }));

    await expect(
      renameDashboard("gone", "New")
    ).rejects.toMatchObject({ status: 404 });
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });
});

describe("fetchDependents", () => {
  it("reads the visible holders by id and name, and how many are hidden", async () => {
    const dependents = {
      holders: [{ kind: "widgets", id: "w1", name: "Commits table" }],
      hidden: 2,
    };
    mockFetch.mockResolvedValueOnce(response(dependents));

    await expect(fetchDependents("metrics", "m1")).resolves.toEqual(dependents);
    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/metrics/m1/dependents");
  });
});

describe("grants", () => {
  const PERSON = "00000000-0000-4000-8000-000000000002";

  it.each([
    ["dashboards", "d1"],
    ["folders", "f1"],
  ] as const)("lists who one of the %s is shared with", async (kind, id) => {
    const items = [
      {
        grantee: EVERYONE,
        level: "viewer",
        granted_by: "00000000-0000-4000-8000-000000000001",
        created_at: "2026-09-25T10:00:00Z",
      },
    ];
    mockFetch.mockResolvedValueOnce(response({ items }));

    await expect(fetchGrants({ kind, id })).resolves.toEqual(items);
    expect(mockFetch).toHaveBeenCalledWith(`/api/v3/v1/${kind}/${id}/grants`);
  });

  it("grants a level and withdraws it, by grantee", async () => {
    mockFetch.mockResolvedValue(response(null, { status: 204 }));

    await putGrant({ kind: "dashboards", id: "d1" }, PERSON, "editor");
    await deleteGrant({ kind: "folders", id: "f1" }, EVERYONE);

    expect(mockFetch).toHaveBeenNthCalledWith(
      1,
      `/api/v3/v1/dashboards/d1/grants/${PERSON}`,
      {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ level: "editor" }),
      }
    );
    expect(mockFetch).toHaveBeenNthCalledWith(
      2,
      "/api/v3/v1/folders/f1/grants/everyone",
      { method: "DELETE" }
    );
  });

  it("refuses with what the service said", async () => {
    const refusal = { detail: "only the owner or an editor can share this" };
    mockFetch.mockResolvedValueOnce(
      response(refusal, { ok: false, status: 403 })
    );

    await expect(
      putGrant({ kind: "dashboards", id: "d1" }, PERSON, "viewer")
    ).rejects.toMatchObject({ status: 403, body: refusal });
  });
});

describe("a dashboard's widgets", () => {
  it("reads them through the dashboard", async () => {
    const items = [
      {
        id: "w1",
        name: "Commits",
        body: { type: "stat", metric: "m1", value: "n" },
        clock: { field: "day", from: "metric" },
      },
    ];
    mockFetch.mockResolvedValueOnce(response({ items }));

    await expect(fetchDashboardWidgets("d1")).resolves.toEqual(items);
    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/dashboards/d1/widgets");
  });

  it("runs one with the window, and without one", async () => {
    mockFetch.mockResolvedValue(response({ columns: [], rows: [] }));

    await runDashboardWidget("d1", "w1", { range: "P30D", bucket: false });
    await runDashboardWidget("d1", "w1");

    expect(mockFetch).toHaveBeenNthCalledWith(
      1,
      "/api/v3/v1/dashboards/d1/widgets/w1/run",
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ range: "P30D", bucket: false }),
      }
    );
    expect(mockFetch).toHaveBeenNthCalledWith(
      2,
      "/api/v3/v1/dashboards/d1/widgets/w1/run",
      { method: "POST" }
    );
  });

  it("reads one page of the rows behind one", async () => {
    const page = { selection: {}, columns: [], rows: [], next_cursor: null };
    mockFetch.mockResolvedValueOnce(response(page));

    await expect(
      fetchDashboardDrilldownPage("d1", "w1", { limit: 100 })
    ).resolves.toEqual(page);
    expect(mockFetch).toHaveBeenCalledWith(
      "/api/v3/v1/dashboards/d1/widgets/w1/drilldown",
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ limit: 100 }),
        signal: undefined,
      }
    );
  });
});

describe("orphans", () => {
  it("reads one page of them with their owner and last change", async () => {
    const page = {
      items: [
        {
          id: "d1",
          name: "Release notes",
          owner_person_id: "00000000-0000-4000-8000-000000000003",
          updated_at: "2026-09-25T10:00:00Z",
        },
      ],
      total: 1,
    };
    mockFetch.mockResolvedValueOnce(response(page));

    await expect(
      fetchOrphanedDashboards({ limit: 50, offset: 50 })
    ).resolves.toEqual(page);
    expect(mockFetch).toHaveBeenCalledWith(
      "/api/v3/v1/orphaned-dashboards?limit=50&offset=50"
    );
  });

  it("reads one page of the orphaned folders", async () => {
    const page = {
      items: [
        {
          id: "f1",
          name: "Handover",
          owner_person_id: "00000000-0000-4000-8000-000000000003",
          updated_at: "2026-09-25T10:00:00Z",
        },
      ],
      total: 1,
    };
    mockFetch.mockResolvedValueOnce(response(page));

    await expect(fetchOrphanedFolders({ limit: 50 })).resolves.toEqual(page);
    expect(mockFetch).toHaveBeenCalledWith(
      "/api/v3/v1/orphaned-folders?limit=50"
    );
  });

  it.each([
    ["dashboards", "d1"],
    ["folders", "f1"],
  ] as const)("transfers one of the %s to an administrator", async (kind, id) => {
    mockFetch.mockResolvedValueOnce(response(null, { status: 204 }));

    await transferOrphan(kind, id, "00000000-0000-4000-8000-000000000002");

    expect(mockFetch).toHaveBeenCalledWith(`/api/v3/v1/${kind}/${id}/transfer`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ to: "00000000-0000-4000-8000-000000000002" }),
    });
  });

  it("refuses a dashboard that is no longer orphaned with what the service said", async () => {
    const refused = {
      detail: "Failed precondition",
      context: {
        violations: [
          {
            type: "owner_is_administrator",
            subject: "owner",
            description: "the dashboard's owner is still an administrator",
          },
        ],
      },
    };
    mockFetch.mockResolvedValueOnce(
      response(refused, { ok: false, status: 409 })
    );

    await expect(transferOrphan("dashboards", "d1", "p1")).rejects.toMatchObject({
      status: 409,
      body: refused,
    });
  });
});
