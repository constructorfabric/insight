import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/api/fetch-with-auth", () => ({ fetchWithAuth: vi.fn() }));

import { fetchWithAuth } from "@/api/fetch-with-auth";

import {
  CustomApiError,
  fetchDashboard,
  fetchDashboardNames,
  fetchWidget,
  runMetric,
} from "./custom-client";

const mockFetch = fetchWithAuth as unknown as ReturnType<typeof vi.fn>;

function response(
  body: unknown,
  init?: { ok?: boolean; status?: number },
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
      "/api/v3/v1/dashboards?q=git+ops&limit=50&offset=50",
    );
  });

  it("surfaces a failure as an API error", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ title: "forbidden" }, { ok: false, status: 403 }),
    );

    await expect(fetchDashboardNames()).rejects.toBeInstanceOf(CustomApiError);
  });
});

describe("fetchDashboard", () => {
  it("reads one dashboard by name", async () => {
    const dashboard = { title: "Engineering", widgets: ["commits_table"] };
    mockFetch.mockResolvedValueOnce(response(dashboard));

    await expect(fetchDashboard("engineering")).resolves.toEqual(dashboard);
    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/dashboards/engineering");
  });

  it("encodes the name into the path", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ title: "x", widgets: [] }),
    );

    await fetchDashboard("a/b?c");

    expect(mockFetch).toHaveBeenCalledWith("/api/v3/v1/dashboards/a%2Fb%3Fc");
  });

  it("surfaces a failure as an API error", async () => {
    mockFetch.mockResolvedValueOnce(response(null, { ok: false, status: 404 }));

    await expect(fetchDashboard("nope")).rejects.toBeInstanceOf(CustomApiError);
  });
});

describe("fetchWidget", () => {
  it("reads one widget by name", async () => {
    const widget = { type: "table", metric: "commits_per_day", columns: ["day", "lines"] };
    mockFetch.mockResolvedValueOnce(response(widget));

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
      expect.objectContaining({ method: "POST" }),
    );
  });

  it("surfaces a failure as an API error rather than an empty result", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ detail: "unknown table `evnts`" }, { ok: false, status: 400 }),
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
