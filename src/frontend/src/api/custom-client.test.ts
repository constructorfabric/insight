import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/api/fetch-with-auth", () => ({ fetchWithAuth: vi.fn() }));

import { fetchWithAuth } from "@/api/fetch-with-auth";

import {
  CustomApiError,
  deleteDataset,
  deleteDefinition,
  fetchDashboard,
  fetchDashboardNames,
  fetchDataset,
  fetchDatasetDependents,
  fetchDatasetRecords,
  fetchDependents,
  fetchMetric,
  fetchTable,
  fetchWidget,
  putDataset,
  putDefinition,
  renameDefinition,
  runMetric,
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

describe("fetchDashboardNames", () => {
  it("reads the page and how many names it is a page of", async () => {
    const page = { names: ["engineering", "delivery"], total: 2 };
    mockFetch.mockResolvedValueOnce(response(page));

    await expect(fetchDashboardNames()).resolves.toEqual(page);
    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/dashboards");
  });

  it("puts the needle and the window in the query string", async () => {
    mockFetch.mockResolvedValueOnce(response({ names: [], total: 0 }));

    await fetchDashboardNames({ search: "git ops", limit: 50, offset: 50 });

    expect(mockFetch).toHaveBeenCalledWith(
      "/api/v3/v1/dashboards?q=git+ops&limit=50&offset=50"
    );
  });

  it("surfaces a failure as an API error", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ title: "forbidden" }, { ok: false, status: 403 })
    );

    await expect(fetchDashboardNames()).rejects.toBeInstanceOf(CustomApiError);
  });
});

describe("fetchDashboard", () => {
  it("reads one dashboard by name, out of the envelope it arrives in", async () => {
    const dashboard = { title: "Engineering", widgets: ["commits_table"] };
    mockFetch.mockResolvedValueOnce(response({ body: dashboard }));

    await expect(fetchDashboard("engineering")).resolves.toEqual(dashboard);
    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/dashboards/engineering");
  });

  it("encodes the name into the path", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ body: { title: "x", widgets: [] } })
    );

    await fetchDashboard("a/b?c");

    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/dashboards/a%2Fb%3Fc");
  });

  it("reads a metric with the clock a window over it would use", async () => {
    const definition = {
      dataset: "commits",
      fields: [{ agg: "count", type: "int", as_name: "total" }],
    };
    mockFetch.mockResolvedValueOnce(
      response({
        body: definition,
        clock: { field: "occurred_at", from: "dataset" },
      })
    );

    await expect(fetchMetric("commits_per_day")).resolves.toEqual({
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
    mockFetch.mockResolvedValueOnce(response({ body: definition }));

    await expect(fetchMetric("all_commits")).resolves.toEqual({
      definition,
      clock: undefined,
    });
  });

  it("surfaces a failure as an API error", async () => {
    mockFetch.mockResolvedValueOnce(response(null, { ok: false, status: 404 }));

    await expect(fetchDashboard("nope")).rejects.toBeInstanceOf(CustomApiError);
  });
});

describe("fetchWidget", () => {
  it("reads one widget by name, out of the envelope it arrives in", async () => {
    const widget = {
      type: "table",
      metric: "commits_per_day",
      columns: ["day", "lines"],
    };
    mockFetch.mockResolvedValueOnce(response({ body: widget }));

    await expect(fetchWidget("commits_table")).resolves.toEqual(widget);
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

describe("a name that reached the client empty", () => {
  it.each([
    ["fetchDataset", () => fetchDataset("")],
    ["fetchMetric", () => fetchMetric("")],
    ["fetchWidget", () => fetchWidget("")],
    ["fetchDashboard", () => fetchDashboard("")],
    ["fetchDatasetRecords", () => fetchDatasetRecords("", { limit: 20 })],
    ["putDataset", () => putDataset("", {})],
    ["deleteDataset", () => deleteDataset("")],
    ["deleteDefinition", () => deleteDefinition("metrics", "")],
    ["renameDefinition", () => renameDefinition("metrics", "", "to")],
    ["runMetric", () => runMetric("")],
    ["putDefinition", () => putDefinition("metrics", "", {})],
    ["fetchDependents", () => fetchDependents("metrics", "")],
    ["fetchDatasetDependents", () => fetchDatasetDependents("")],
    ["fetchTable (database)", () => fetchTable("", "commits")],
    ["fetchTable (table)", () => fetchTable("insight", "")],
  ])("is refused before %s asks for it", async (_name, call) => {
    await expect(call()).rejects.toBeInstanceOf(CustomApiError);
    expect(mockFetch).not.toHaveBeenCalled();
  });
});
