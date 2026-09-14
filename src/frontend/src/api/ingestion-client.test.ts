/**
 * The wire contract for the ingestion lens.
 *
 * The response bodies here are the shapes the endpoint really returns —
 * `series=total` bands under `all`, the 1s grain stringifies buckets WITH
 * milliseconds and the 15m grain without — so a change on either side of the
 * boundary shows up as a failure rather than an empty chart.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/api/fetch-with-auth", () => ({ fetchWithAuth: vi.fn() }));

import { fetchWithAuth } from "@/api/fetch-with-auth";

import { getIngestionIntensity, intensityParams } from "./ingestion-client";
import { AnalyticsApiError } from "./analytics-client";

const mockFetch = fetchWithAuth as unknown as ReturnType<typeof vi.fn>;

function response(body: unknown, init?: { ok?: boolean; status?: number }): Response {
  return {
    ok: init?.ok ?? true,
    status: init?.status ?? 200,
    json: async () => body,
  } as unknown as Response;
}

/** A `series=connector` body in the exact shape the endpoint returns. */
const ORG_WIDE = {
  grain: "15m",
  series: "connector",
  from: "2026-08-25T14:00:00.000Z",
  to: "2026-08-26T14:00:00.000Z",
  truncated: false,
  points: [
    { bucket: "2026-08-25 14:00:00", key: "demo_chat", rows: 49 },
    { bucket: "2026-08-25 14:00:00", key: "demo_tasks", rows: 240 },
  ],
};

beforeEach(() => {
  mockFetch.mockReset();
});

describe("intensityParams", () => {
  it("sends only the grain when nothing else is pinned", () => {
    // Omitting the window is deliberate: the server's per-grain default IS the
    // window the live charts want, and an absent bound keeps the query key
    // stable while they refetch.
    expect(intensityParams({ grain: "1s" }, 0).toString()).toBe("grain=1s");
  });

  it("resolves a lookback into a day-anchored instant", () => {
    const at = Date.parse("2026-08-26T09:41:17Z");
    const params = intensityParams({ grain: "15m", series: "total", lookbackDays: 30 }, at);
    expect(params.get("series")).toBe("total");
    expect(params.get("from")).toBe("2026-07-27T00:00:00.000Z");
    // No `to`: the server's "now" is the honest upper bound, not the browser's.
    expect(params.get("to")).toBeNull();
  });

  it("prefers an explicit from over a lookback", () => {
    const params = intensityParams(
      { grain: "15m", from: "2026-08-01T00:00:00.000Z", lookbackDays: 30 },
      Date.parse("2026-08-26T09:41:17Z"),
    );
    expect(params.get("from")).toBe("2026-08-01T00:00:00.000Z");
  });

  it("omits a null scope rather than sending an empty one", () => {
    // The overview passes scope: null; an empty `scope=` would be a malformed
    // value the endpoint refuses with a 400.
    expect(intensityParams({ grain: "15m", scope: null }, 0).has("scope")).toBe(false);
  });

  it("sends the bronze database as the scope", () => {
    expect(intensityParams({ grain: "15m", scope: "bronze_jira" }, 0).get("scope")).toBe(
      "bronze_jira",
    );
  });
});

describe("getIngestionIntensity", () => {
  it("addresses the analytics prefix the gateway strips to", async () => {
    mockFetch.mockResolvedValueOnce(response(ORG_WIDE));

    await expect(getIngestionIntensity({ grain: "15m" })).resolves.toEqual(ORG_WIDE);
    expect(mockFetch).toHaveBeenCalledWith(
      "/api/analytics/v1/ingestion/intensity?grain=15m",
    );
  });

  it("carries a scoped drill-down through to the query string", async () => {
    mockFetch.mockResolvedValueOnce(
      response({
        ...ORG_WIDE,
        series: "stream",
        scope: "bronze_demo_tasks",
        points: [{ bucket: "2026-08-25 14:00:00", key: "comments", rows: 64 }],
      }),
    );

    const body = await getIngestionIntensity({ grain: "15m", scope: "bronze_demo_tasks" });

    expect(mockFetch).toHaveBeenCalledWith(
      "/api/analytics/v1/ingestion/intensity?grain=15m&scope=bronze_demo_tasks",
    );
    expect(body.scope).toBe("bronze_demo_tasks");
    expect(body.series).toBe("stream");
  });

  it("keeps the millisecond bucket the 1s grain returns", async () => {
    mockFetch.mockResolvedValueOnce(
      response({
        ...ORG_WIDE,
        grain: "1s",
        points: [{ bucket: "2026-08-26 13:59:41.250", key: "demo_docs", rows: 2 }],
      }),
    );

    const body = await getIngestionIntensity({ grain: "1s" });
    expect(body.points[0].bucket).toBe("2026-08-26 13:59:41.250");
  });

  it("surfaces a clipped read rather than dropping the flag", async () => {
    mockFetch.mockResolvedValueOnce(response({ ...ORG_WIDE, truncated: true }));

    await expect(getIngestionIntensity({ grain: "15m" })).resolves.toMatchObject({
      truncated: true,
    });
  });

  it("raises the refusal so the surface can report it", async () => {
    // The admin gate answers 403 to anyone without the grant, and the lens has
    // to show that rather than an empty chart.
    mockFetch.mockResolvedValueOnce(
      response({ status: 403, detail: "admin role required" }, { ok: false, status: 403 }),
    );

    await expect(getIngestionIntensity({ grain: "15m" })).rejects.toBeInstanceOf(
      AnalyticsApiError,
    );
  });
});
