import { beforeEach, describe, expect, it, vi } from "vitest";

import { AnalyticsApiError } from "@/api/analytics-client";
import {
  downloadMetricDrilldown,
  evidenceSelection,
  queryMetricDrilldown,
  type MetricDrilldownRequest,
} from "@/api/metric-drilldown-client";

const mocks = vi.hoisted(() => ({
  fetchWithAuth: vi.fn(),
  downloadBlob: vi.fn(),
}));

vi.mock("@/api/fetch-with-auth", () => ({
  fetchWithAuth: mocks.fetchWithAuth,
}));

vi.mock("@/lib/download", () => ({
  downloadBlob: mocks.downloadBlob,
}));

const selection = {
  metric_key: "git.commits",
  entity: { type: "person" as const, id: "person@example.com" },
  period: { from: "2026-07-01", to: "2026-07-31" },
  filters: [],
  display_dimensions: [],
};

function response({
  ok = true,
  status = 200,
  body,
  disposition,
}: {
  ok?: boolean;
  status?: number;
  body?: unknown;
  disposition?: string | null;
}) {
  return {
    ok,
    status,
    json:
      body instanceof Error
        ? vi.fn().mockRejectedValue(body)
        : vi.fn().mockResolvedValue(body),
    blob: vi.fn().mockResolvedValue(new Blob(["export"])),
    headers: new Headers(
      disposition == null ? undefined : { "content-disposition": disposition }
    ),
  } as unknown as Response;
}

describe("metric drilldown client", () => {
  beforeEach(() => {
    mocks.fetchWithAuth.mockReset();
    mocks.downloadBlob.mockReset();
  });

  it("queries evidence and forwards cancellation", async () => {
    const payload = { ...selection, columns: [], rows: [], next_cursor: null };
    mocks.fetchWithAuth.mockResolvedValue(response({ body: payload }));
    const controller = new AbortController();
    const request: MetricDrilldownRequest = { ...selection, limit: 100 };

    await expect(
      queryMetricDrilldown(request, controller.signal)
    ).resolves.toEqual(payload);
    expect(mocks.fetchWithAuth).toHaveBeenCalledWith(
      expect.stringContaining("/metric-drilldown"),
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify(request),
        signal: controller.signal,
      })
    );
  });

  it("classifies malformed success and error responses", async () => {
    mocks.fetchWithAuth.mockResolvedValueOnce(
      response({ body: new SyntaxError("invalid") })
    );
    await expect(
      queryMetricDrilldown({ ...selection, limit: 100 })
    ).rejects.toMatchObject({
      status: 200,
      body: { error: "invalid_json" },
    });

    mocks.fetchWithAuth.mockResolvedValueOnce(
      response({ ok: false, status: 400, body: { detail: "bad request" } })
    );
    await expect(
      queryMetricDrilldown({ ...selection, limit: 100 })
    ).rejects.toMatchObject({
      status: 400,
      body: { detail: "bad request" },
    });

    mocks.fetchWithAuth.mockResolvedValueOnce(
      response({ ok: false, status: 502, body: new SyntaxError("invalid") })
    );
    const error = await queryMetricDrilldown({
      ...selection,
      limit: 100,
    }).catch((failure: unknown) => failure);
    expect(error).toBeInstanceOf(AnalyticsApiError);
    expect(error).toMatchObject({ status: 502, body: null });
  });

  it("downloads exports using server and fallback filenames", async () => {
    mocks.fetchWithAuth.mockResolvedValueOnce(
      response({
        disposition: "attachment; filename*=UTF-8''commits%20July.csv",
      })
    );
    await downloadMetricDrilldown(selection, "csv");
    expect(mocks.downloadBlob).toHaveBeenLastCalledWith(
      expect.any(Blob),
      "commits July.csv"
    );

    mocks.fetchWithAuth.mockResolvedValueOnce(
      response({ disposition: "attachment; filename*=UTF-8''%ZZ" })
    );
    await downloadMetricDrilldown(selection, "xlsx");
    expect(mocks.downloadBlob).toHaveBeenLastCalledWith(
      expect.any(Blob),
      "git.commits.xlsx"
    );

    mocks.fetchWithAuth.mockResolvedValueOnce(
      response({ ok: false, status: 429, body: { detail: "busy" } })
    );
    await expect(
      downloadMetricDrilldown(selection, "csv")
    ).rejects.toMatchObject({
      status: 429,
    });
  });

  it("builds normalized selections from canonical results", () => {
    expect(evidenceSelection(undefined, "person")).toBeNull();
    expect(
      evidenceSelection(
        {
          metric_key: "git.commits",
          entity: { type: "person", ids: ["person"] },
          period: selection.period,
          filters: [{ dimension: "repository", values: ["org/repo"] }],
        },
        "person",
        undefined,
        undefined,
        ["category", "repository", "category"]
      )
    ).toEqual({
      ...selection,
      entity: { type: "person", id: "person" },
      filters: [{ dimension: "repository", values: ["org/repo"] }],
      display_dimensions: ["category", "repository"],
    });
  });
});
