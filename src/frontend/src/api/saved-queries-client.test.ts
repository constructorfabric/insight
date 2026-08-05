import { beforeEach, describe, expect, it, vi } from "vitest";

import { AnalyticsApiError } from "@/api/analytics-client";
import { fetchWithAuth } from "@/api/fetch-with-auth";

import {
  createSavedQuery,
  deleteSavedQuery,
  getSavedQuery,
  listSavedQueries,
  runSavedQuery,
  updateSavedQuery,
} from "./saved-queries-client";

vi.mock("@/api/fetch-with-auth", () => ({ fetchWithAuth: vi.fn() }));

const mockFetch = vi.mocked(fetchWithAuth);
const BASE = "/api/analytics/v1";

function ok(body: unknown, status = 200): Response {
  return {
    ok: true,
    status,
    json: async () => body,
  } as unknown as Response;
}

function fail(status: number, body: unknown): Response {
  return {
    ok: false,
    status,
    json: async () => body,
  } as unknown as Response;
}

const SAVED = {
  id: "q-1",
  insight_tenant_id: "t-1",
  name: "Q",
  description: null,
  sql: "SELECT 1",
  created_at: "2026-07-01T00:00:00Z",
  updated_at: "2026-07-01T00:00:00Z",
};

beforeEach(() => mockFetch.mockReset());

describe("saved-queries-client happy paths", () => {
  it("list issues a GET and unwraps items", async () => {
    mockFetch.mockResolvedValue(ok({ items: [{ id: "q-1", name: "Q" }] }));
    const res = await listSavedQueries();
    expect(res.items).toHaveLength(1);
    expect(mockFetch).toHaveBeenCalledWith(`${BASE}/queries`, {
      method: "GET",
    });
  });

  it("get issues a GET by id", async () => {
    mockFetch.mockResolvedValue(ok(SAVED));
    await getSavedQuery("q-1");
    expect(mockFetch).toHaveBeenCalledWith(`${BASE}/queries/q-1`, {
      method: "GET",
    });
  });

  it("create POSTs the JSON body", async () => {
    mockFetch.mockResolvedValue(ok(SAVED, 201));
    await createSavedQuery({ name: "Q", description: null, sql: "SELECT 1" });
    expect(mockFetch).toHaveBeenCalledWith(`${BASE}/queries`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: "Q", description: null, sql: "SELECT 1" }),
    });
  });

  it("update PUTs the JSON body by id", async () => {
    mockFetch.mockResolvedValue(ok(SAVED));
    await updateSavedQuery("q-1", { name: "R" });
    expect(mockFetch).toHaveBeenCalledWith(`${BASE}/queries/q-1`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: "R" }),
    });
  });

  it("delete issues a DELETE and resolves on 204", async () => {
    mockFetch.mockResolvedValue({ ok: true, status: 204 } as Response);
    await expect(deleteSavedQuery("q-1")).resolves.toBeUndefined();
    expect(mockFetch).toHaveBeenCalledWith(`${BASE}/queries/q-1`, {
      method: "DELETE",
    });
  });

  it("run defaults to an empty body and returns rows", async () => {
    mockFetch.mockResolvedValue(ok({ rows: [{ a: 1 }] }));
    const res = await runSavedQuery("q-1");
    expect(res.rows).toEqual([{ a: 1 }]);
    expect(mockFetch).toHaveBeenCalledWith(`${BASE}/queries/q-1/run`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({}),
    });
  });

  it("run passes a period when supplied", async () => {
    mockFetch.mockResolvedValue(ok({ rows: [] }));
    await runSavedQuery("q-1", { period: "2026-01" });
    expect(mockFetch).toHaveBeenCalledWith(`${BASE}/queries/q-1/run`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ period: "2026-01" }),
    });
  });
});

describe("saved-queries-client error paths", () => {
  it("throws AnalyticsApiError with status + body on a non-OK response", async () => {
    mockFetch.mockResolvedValue(fail(400, { error: "invalid_argument" }));
    await expect(listSavedQueries()).rejects.toMatchObject({
      name: "AnalyticsApiError",
      status: 400,
      body: { error: "invalid_argument" },
    });
  });

  it("delete throws AnalyticsApiError on a non-OK response", async () => {
    mockFetch.mockResolvedValue(fail(404, { error: "not_found" }));
    await expect(deleteSavedQuery("q-1")).rejects.toBeInstanceOf(
      AnalyticsApiError
    );
  });

  it("throws when the OK body is not valid JSON", async () => {
    mockFetch.mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => {
        throw new SyntaxError("bad json");
      },
    } as unknown as Response);
    await expect(getSavedQuery("q-1")).rejects.toMatchObject({
      status: 200,
      body: { error: "invalid_json" },
    });
  });
});
