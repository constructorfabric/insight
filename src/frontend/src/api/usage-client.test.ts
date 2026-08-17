import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/api/fetch-with-auth", () => ({ fetchWithAuth: vi.fn() }));

import { fetchWithAuth } from "@/api/fetch-with-auth";

import { getUsageConfig, getUsageSummary } from "./usage-client";
import { AnalyticsApiError } from "./analytics-client";

const mockFetch = fetchWithAuth as unknown as ReturnType<typeof vi.fn>;

function response(body: unknown, init?: { ok?: boolean; status?: number }): Response {
  return {
    ok: init?.ok ?? true,
    status: init?.status ?? 200,
    json: async () => body,
  } as unknown as Response;
}

beforeEach(() => {
  mockFetch.mockReset();
});

describe("getUsageConfig", () => {
  it("reports an instance that collects nothing", async () => {
    mockFetch.mockResolvedValueOnce(response({ enabled: false }));

    await expect(getUsageConfig()).resolves.toEqual({ enabled: false });
    expect(mockFetch).toHaveBeenCalledWith("/api/analytics/v1/usage/config");
  });
});

describe("getUsageSummary", () => {
  it("puts the window in the query string", async () => {
    mockFetch.mockResolvedValueOnce(response({ since: "2026-08-01", until: "2026-08-16" }));

    await getUsageSummary({ since: "2026-08-01", until: "2026-08-16" });

    expect(mockFetch).toHaveBeenCalledWith(
      "/api/analytics/v1/usage/summary?since=2026-08-01&until=2026-08-16",
    );
  });

  it("raises the refusal rather than resolving to an empty page", async () => {
    // The summary is admin-only; a 403 that resolved would render as "nobody
    // used the product" instead of "you may not see this".
    mockFetch.mockResolvedValueOnce(
      response({ detail: "admin role required" }, { ok: false, status: 403 }),
    );

    await expect(
      getUsageSummary({ since: "2026-08-01", until: "2026-08-16" }),
    ).rejects.toBeInstanceOf(AnalyticsApiError);
  });

  it("still raises when the refusal carries no body", async () => {
    mockFetch.mockResolvedValueOnce({
      ok: false,
      status: 500,
      json: async () => {
        throw new Error("not json");
      },
    } as unknown as Response);

    await expect(
      getUsageSummary({ since: "2026-08-01", until: "2026-08-16" }),
    ).rejects.toBeInstanceOf(AnalyticsApiError);
  });
});
