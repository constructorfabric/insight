import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/api/fetch-with-auth", () => ({ fetchWithAuth: vi.fn() }));

import { fetchWithAuth } from "@/api/fetch-with-auth";

import { getConnectorHealth, getConnectorSyncs } from "./connector-health-client";
import { AnalyticsApiError } from "./analytics-client";

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

describe("getConnectorHealth", () => {
  it("reads the instance-wide summary", async () => {
    const summary = {
      as_of: "2026-01-15T09:12:00.000Z",
      checked_at: "2026-01-15T09:06:00.000Z",
      typical_read_interval_ms: 900_000,
      history_available: true,
      connectors: [],
    };
    mockFetch.mockResolvedValueOnce(response(summary));

    await expect(getConnectorHealth()).resolves.toEqual(summary);
    expect(mockFetch).toHaveBeenCalledWith("/api/analytics/v1/connector-health");
  });

  it("surfaces a refusal as an API error rather than an empty page", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ title: "forbidden" }, { ok: false, status: 403 }),
    );

    await expect(getConnectorHealth()).rejects.toBeInstanceOf(AnalyticsApiError);
  });
});

describe("getConnectorSyncs", () => {
  it("reads one connector's window", async () => {
    const history = { connector: "example-tracker", syncs: [], window: 50 };
    mockFetch.mockResolvedValueOnce(response(history));

    await expect(getConnectorSyncs({ connector: "example-tracker" })).resolves.toEqual(history);
    expect(mockFetch).toHaveBeenCalledWith(
      "/api/analytics/v1/connector-health/example-tracker/syncs",
    );
  });

  it("encodes the connector, so a name never becomes part of the path", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ connector: "x", syncs: [], window: 50 }),
    );

    await getConnectorSyncs({ connector: "a/b?c" });

    expect(mockFetch).toHaveBeenCalledWith(
      "/api/analytics/v1/connector-health/a%2Fb%3Fc/syncs",
    );
  });

  it("narrows the window to one installation when both halves are named", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ connector: "claude-team", syncs: [], window: 50 }),
    );

    await getConnectorSyncs({
      connector: "claude-team",
      tenant_id: "acme",
      source_id: "claude-team-second",
    });

    expect(mockFetch).toHaveBeenCalledWith(
      "/api/analytics/v1/connector-health/claude-team/syncs" +
        "?tenant_id=acme&source_id=claude-team-second",
    );
  });

  it("sends no scope at all when only one half is known", async () => {
    // The service refuses half an identity. Sending it would turn a question
    // about one installation into a 400 instead of the answer about the
    // connector the caller can actually be given.
    mockFetch.mockResolvedValueOnce(
      response({ connector: "claude-team", syncs: [], window: 50 }),
    );

    await getConnectorSyncs({ connector: "claude-team", source_id: "main" });

    expect(mockFetch).toHaveBeenCalledWith(
      "/api/analytics/v1/connector-health/claude-team/syncs",
    );
  });

  it("surfaces a failure rather than an empty window", async () => {
    mockFetch.mockResolvedValueOnce(
      response(null, { ok: false, status: 500 }),
    );

    await expect(getConnectorSyncs({ connector: "example-tracker" })).rejects.toBeInstanceOf(
      AnalyticsApiError,
    );
  });
});
