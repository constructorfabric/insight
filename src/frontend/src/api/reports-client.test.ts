import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AnalyticsApiError } from "./analytics-client";
import { downloadReport, previewReport } from "./reports-client";

const mocks = vi.hoisted(() => ({ download: vi.fn() }));

vi.mock("@/lib/download", () => ({ downloadBlob: mocks.download }));

const peopleRecipe = {
  subject: {
    type: "people" as const,
    ids: ["00000000-0000-0000-0000-000000000001"],
  },
  period: { from: "2026-01-01", to: "2026-01-31" },
  granularity: "month" as const,
  metric_keys: ["git.commits"],
};

function jsonResponse(body: unknown, init: ResponseInit = {}): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
    ...init,
  });
}

describe("reports client", () => {
  beforeEach(() => {
    vi.stubGlobal("fetch", vi.fn());
    mocks.download.mockReset();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("sends people recipe to preview endpoint", async () => {
    (globalThis.fetch as ReturnType<typeof vi.fn>).mockResolvedValue(
      jsonResponse({ columns: [], rows: [], total_rows: 0 })
    );

    await previewReport(peopleRecipe);

    const [url, init] = (globalThis.fetch as ReturnType<typeof vi.fn>).mock
      .calls[0] as [string, RequestInit];
    expect(url).toBe("/api/analytics/v1/reports/preview");
    expect(init.body).toBe(JSON.stringify(peopleRecipe));
  });

  it("sends tenant recipe and format to export endpoint", async () => {
    (globalThis.fetch as ReturnType<typeof vi.fn>).mockResolvedValue(
      new Response("contents", {
        status: 200,
        headers: { "content-disposition": "attachment; filename=tenant.csv" },
      })
    );
    const recipe = {
      ...peopleRecipe,
      subject: { type: "tenant" as const },
      metric_keys: ["ci.runs"],
    };

    await downloadReport(recipe, "csv");

    const [url, init] = (globalThis.fetch as ReturnType<typeof vi.fn>).mock
      .calls[0] as [string, RequestInit];
    expect(url).toBe("/api/analytics/v1/reports/export");
    expect(init.body).toBe(JSON.stringify({ ...recipe, format: "csv" }));
    expect(mocks.download).toHaveBeenCalledWith(expect.any(Blob), "tenant.csv");
  });

  it("rejects preview failures with analytics error", async () => {
    (globalThis.fetch as ReturnType<typeof vi.fn>).mockResolvedValue(
      jsonResponse({ error: "invalid_request" }, { status: 400 })
    );

    await expect(previewReport(peopleRecipe)).rejects.toBeInstanceOf(
      AnalyticsApiError
    );
  });
});
