import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/api/fetch-with-auth", () => ({ fetchWithAuth: vi.fn() }));

import { fetchWithAuth } from "@/api/fetch-with-auth";

import { getFeedback, submitFeedback } from "./feedback-client";
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

const SUBMISSION = {
  message: "the chart is empty",
  path: "/portal/overview",
  app_name: "insight-frontend",
  app_version: "0.0.1",
} as const;

beforeEach(() => {
  mockFetch.mockReset();
});

describe("submitFeedback", () => {
  it("posts the submission as JSON", async () => {
    mockFetch.mockResolvedValueOnce(response(null, { status: 204 }));

    await submitFeedback({ ...SUBMISSION });

    expect(mockFetch).toHaveBeenCalledWith("/api/analytics/v1/feedback", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(SUBMISSION),
    });
  });

  it("raises a refusal rather than reporting a send that never happened", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ detail: "message must not be empty" }, { ok: false, status: 400 }),
    );

    await expect(submitFeedback({ ...SUBMISSION })).rejects.toBeInstanceOf(
      AnalyticsApiError,
    );
  });
});

describe("getFeedback", () => {
  it("puts the window in the query string", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ since: "2026-08-01", until: "2026-08-16", items: [] }),
    );

    await getFeedback({ since: "2026-08-01", until: "2026-08-16" });

    expect(mockFetch).toHaveBeenCalledWith(
      "/api/analytics/v1/feedback?since=2026-08-01&until=2026-08-16",
    );
  });

  it("raises the refusal rather than resolving to an empty list", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ detail: "admin role required" }, { ok: false, status: 403 }),
    );

    await expect(
      getFeedback({ since: "2026-08-01", until: "2026-08-16" }),
    ).rejects.toBeInstanceOf(AnalyticsApiError);
  });
});
