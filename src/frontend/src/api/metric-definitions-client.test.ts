import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AnalyticsApiError } from "./analytics-client";
import {
  listMetricDefinitions,
  type MetricDefinition,
} from "./metric-definitions-client";

const ENDPOINT = "/api/analytics/v1/metric-definitions";

function jsonResponse(body: unknown, init: ResponseInit = {}): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
    ...init,
  });
}

const METRIC: MetricDefinition = {
  metric_key: "git.commits",
  label: "Commits",
  short_label: null,
  description: "Commits authored in the period.",
  explanation: null,
  unit: null,
  format: "integer",
  direction: "higher_is_better",
  dimensions: ["repo"],
  is_enabled: true,
  schema_status: "ok",
  schema_error_code: null,
  last_observed_date: "2026-07-20",
};

describe("listMetricDefinitions", () => {
  beforeEach(() => {
    vi.stubGlobal("fetch", vi.fn());
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("issues GET against /metric-definitions and returns the body", async () => {
    (globalThis.fetch as ReturnType<typeof vi.fn>).mockResolvedValue(
      jsonResponse({ metrics: [METRIC] })
    );
    const result = await listMetricDefinitions();
    expect(globalThis.fetch).toHaveBeenCalledTimes(1);
    const [url, init] = (globalThis.fetch as ReturnType<typeof vi.fn>).mock
      .calls[0] as [string, RequestInit];
    expect(url).toBe(ENDPOINT);
    expect(init.method).toBe("GET");
    expect(result.metrics).toEqual([METRIC]);
  });

  it("throws AnalyticsApiError with the error body on non-2xx", async () => {
    (globalThis.fetch as ReturnType<typeof vi.fn>).mockResolvedValue(
      jsonResponse({ error: "internal" }, { status: 500 })
    );
    await expect(listMetricDefinitions()).rejects.toThrowError(
      AnalyticsApiError
    );
  });

  it("throws AnalyticsApiError on invalid JSON", async () => {
    (globalThis.fetch as ReturnType<typeof vi.fn>).mockResolvedValue(
      new Response("not json", {
        status: 200,
        headers: { "Content-Type": "application/json" },
      })
    );
    await expect(listMetricDefinitions()).rejects.toThrowError(
      AnalyticsApiError
    );
  });
});
