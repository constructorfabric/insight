import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AnalyticsApiError } from "./analytics-client";
import { getConnectorHealth, type ConnectorRow } from "./connector-health-client";

const ENDPOINT = "/api/analytics/v1/connector-health";

function jsonResponse(body: unknown, init: ResponseInit = {}): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
    ...init,
  });
}

const CONNECTOR: ConnectorRow = {
  connector: "example",
  namespace: "bronze_example",
  streams: 3,
  streams_with_data: 2,
  rows: 10,
  last_write: "2020-01-02T00:00:00Z",
};

describe("getConnectorHealth", () => {
  beforeEach(() => {
    vi.stubGlobal("fetch", vi.fn());
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("returns the connector rows the service reports", async () => {
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse({ as_of: "2020-01-03T00:00:00Z", connectors: [CONNECTOR] })
    );

    const response = await getConnectorHealth();

    expect(response.connectors).toEqual([CONNECTOR]);
    expect(vi.mocked(fetch).mock.calls[0]?.[0]).toBe(ENDPOINT);
  });

  it("raises the API error for a failing status", async () => {
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse({ error: "internal" }, { status: 500 })
    );

    await expect(getConnectorHealth()).rejects.toBeInstanceOf(AnalyticsApiError);
  });

  it("raises rather than returning a half-read body when the payload is not JSON", async () => {
    vi.mocked(fetch).mockResolvedValue(
      new Response("not json", {
        status: 200,
        headers: { "Content-Type": "application/json" },
      })
    );

    await expect(getConnectorHealth()).rejects.toBeInstanceOf(AnalyticsApiError);
  });
});
