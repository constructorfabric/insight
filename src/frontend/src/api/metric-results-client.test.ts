import { beforeEach, describe, expect, it, vi } from "vitest";

import { AnalyticsApiError } from "@/api/analytics-client";
import {
  queryMetricResults,
  type MetricResultsRequest,
} from "@/api/metric-results-client";
import { fetchWithAuth } from "@/api/fetch-with-auth";
import { METRIC_RESULTS_RESPONSE_FIXTURE } from "@/mocks/metric-results-fixtures";

vi.mock("@/api/fetch-with-auth", () => ({ fetchWithAuth: vi.fn() }));

const mockFetch = vi.mocked(fetchWithAuth);

const REQUEST: MetricResultsRequest = {
  entity: { type: "person", ids: ["alice@example.com"] },
  period: { from: "2026-06-01", to: "2026-06-30" },
  metrics: [{ metric_key: "ai.accepted_lines", views: [{ view: "period" }] }],
};

function response(init: {
  ok: boolean;
  status?: number;
  json: () => Promise<unknown>;
}): Response {
  return {
    ok: init.ok,
    status: init.status ?? (init.ok ? 200 : 500),
    json: init.json,
  } as Response;
}

describe("queryMetricResults", () => {
  beforeEach(() => mockFetch.mockReset());

  it("returns the parsed response on success", async () => {
    mockFetch.mockResolvedValue(
      response({ ok: true, json: async () => METRIC_RESULTS_RESPONSE_FIXTURE }),
    );
    await expect(queryMetricResults(REQUEST)).resolves.toEqual(
      METRIC_RESULTS_RESPONSE_FIXTURE,
    );
  });

  it("throws AnalyticsApiError with status + body on a non-ok response", async () => {
    mockFetch.mockResolvedValue(
      response({ ok: false, status: 400, json: async () => ({ error: "bad" }) }),
    );
    await expect(queryMetricResults(REQUEST)).rejects.toMatchObject({
      status: 400,
      body: { error: "bad" },
    });
    await expect(queryMetricResults(REQUEST)).rejects.toBeInstanceOf(
      AnalyticsApiError,
    );
  });

  it("throws AnalyticsApiError('invalid_json') when an ok body fails to parse", async () => {
    mockFetch.mockResolvedValue(
      response({
        ok: true,
        json: async () => {
          throw new Error("not json");
        },
      }),
    );
    await expect(queryMetricResults(REQUEST)).rejects.toMatchObject({
      body: { error: "invalid_json" },
    });
  });
});
