import { beforeEach, describe, expect, it, vi } from "vitest";

import { AnalyticsApiError } from "@/api/analytics-client";
import { fetchWithAuth } from "@/api/fetch-with-auth";

import {
  createCustomMetric,
  deleteCustomMetric,
  exportCustomMetrics,
  getCustomMetric,
  importCustomMetrics,
  listCustomMetrics,
  updateCustomMetric,
  type CustomMetric,
  type CustomMetricGraph,
} from "./metrics-client";

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

const GRAPH: CustomMetricGraph = {
  metric_key: "example.accepted_lines",
  label: "Accepted lines",
  short_label: null,
  description: null,
  explanation: null,
  entity_type: "person",
  unit: "lines",
  format: "integer",
  direction: "higher_is_better",
  computation: "sum",
  scale: null,
  peer_cohort_key: null,
  transform: null,
  source_key: "example_source",
  observation_sql: "SELECT 1",
  measures: ["accepted_lines"],
  dimensions: ["repo"],
  inputs: [{ role: "value", measure_key: "accepted_lines" }],
};

const METRIC: CustomMetric = { ...GRAPH, origin: "custom" };

beforeEach(() => mockFetch.mockReset());

describe("metrics-client happy paths", () => {
  it("list issues a GET and returns items", async () => {
    mockFetch.mockResolvedValue(
      ok({
        items: [
          {
            metric_key: "example.accepted_lines",
            label: "Accepted lines",
            computation: "sum",
            entity_type: "person",
          },
        ],
      })
    );
    const res = await listCustomMetrics();
    expect(res.items).toHaveLength(1);
    expect(mockFetch).toHaveBeenCalledWith(`${BASE}/metrics`, {
      method: "GET",
    });
  });

  it("get issues a GET by metric key", async () => {
    mockFetch.mockResolvedValue(ok(METRIC));
    await getCustomMetric("example.accepted_lines");
    expect(mockFetch).toHaveBeenCalledWith(
      `${BASE}/metrics/example.accepted_lines`,
      { method: "GET" }
    );
  });

  it("create POSTs the JSON graph and returns a custom metric", async () => {
    mockFetch.mockResolvedValue(ok(METRIC, 201));
    const res = await createCustomMetric(GRAPH);
    expect(res.origin).toBe("custom");
    expect(mockFetch).toHaveBeenCalledWith(`${BASE}/metrics`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(GRAPH),
    });
  });

  it("update PUTs the JSON graph by metric key", async () => {
    mockFetch.mockResolvedValue(ok(METRIC));
    await updateCustomMetric("example.accepted_lines", GRAPH);
    expect(mockFetch).toHaveBeenCalledWith(
      `${BASE}/metrics/example.accepted_lines`,
      {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(GRAPH),
      }
    );
  });

  it("delete issues a DELETE and resolves on 204", async () => {
    mockFetch.mockResolvedValue({ ok: true, status: 204 } as Response);
    await expect(
      deleteCustomMetric("example.accepted_lines")
    ).resolves.toBeUndefined();
    expect(mockFetch).toHaveBeenCalledWith(
      `${BASE}/metrics/example.accepted_lines`,
      { method: "DELETE" }
    );
  });

  it("export issues a GET and returns graphs", async () => {
    mockFetch.mockResolvedValue(ok({ metrics: [GRAPH] }));
    const res = await exportCustomMetrics();
    expect(res.metrics).toHaveLength(1);
    expect(mockFetch).toHaveBeenCalledWith(`${BASE}/metrics/export`, {
      method: "GET",
    });
  });

  it("import POSTs graphs and returns a report", async () => {
    mockFetch.mockResolvedValue(ok({ imported: 1, skipped: [] }));
    const res = await importCustomMetrics({ metrics: [GRAPH] });
    expect(res.imported).toBe(1);
    expect(mockFetch).toHaveBeenCalledWith(`${BASE}/metrics/import`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ metrics: [GRAPH] }),
    });
  });
});

describe("metrics-client error paths", () => {
  it("throws AnalyticsApiError with status + body on a non-OK response", async () => {
    mockFetch.mockResolvedValue(fail(400, { error: "invalid_argument" }));
    await expect(listCustomMetrics()).rejects.toMatchObject({
      name: "AnalyticsApiError",
      status: 400,
      body: { error: "invalid_argument" },
    });
  });

  it("delete throws AnalyticsApiError on a non-OK response", async () => {
    mockFetch.mockResolvedValue(fail(404, { error: "not_found" }));
    await expect(deleteCustomMetric("nope")).rejects.toBeInstanceOf(
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
    await expect(getCustomMetric("x")).rejects.toMatchObject({
      status: 200,
      body: { error: "invalid_json" },
    });
  });
});
