import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/api/fetch-with-auth", () => ({ fetchWithAuth: vi.fn() }));

import { fetchWithAuth } from "@/api/fetch-with-auth";

import {
  createAlert,
  deleteAlert,
  fetchAlert,
  fetchAlertDestinations,
  fetchAlertNotifications,
  fetchAlerts,
  replaceAlert,
  setAlertEnabled,
} from "./alerts-client";
import { CustomApiError } from "./custom-client";

const mockFetch = fetchWithAuth as unknown as ReturnType<typeof vi.fn>;

function response(body: unknown, status = 200): Response {
  return {
    ok: status < 400,
    status,
    json: async () => body,
  } as unknown as Response;
}

const DRAFT = {
  name: "Too many open PRs",
  metric: "prs-open",
  column: "total",
  operator: ">" as const,
  threshold: 10,
  interval_secs: 300,
  destination: "ops",
  enabled: true,
};

beforeEach(() => {
  mockFetch.mockReset();
  mockFetch.mockResolvedValue(response({}));
});

function called(): [string, RequestInit | undefined] {
  return mockFetch.mock.calls[0] as [string, RequestInit | undefined];
}

describe("alerts client", () => {
  it.each([
    [{}, "/api/v3/v1/alerts"],
    [
      { search: "prs", limit: 50, offset: 50 },
      "/api/v3/v1/alerts?q=prs&limit=50&offset=50",
    ],
    [{ offset: 0 }, "/api/v3/v1/alerts"],
  ])("lists alerts for %o at %s", async (slice, url) => {
    await fetchAlerts(slice);

    expect(called()[0]).toBe(url);
  });

  it("reads, replaces and removes an alert by its id", async () => {
    await fetchAlert("a1");
    expect(called()[0]).toBe("/api/v3/v1/alerts/a1");

    mockFetch.mockClear();
    await replaceAlert("a1", { ...DRAFT, expected_revision: 3 });
    expect(called()[0]).toBe("/api/v3/v1/alerts/a1");
    expect(called()[1]?.method).toBe("PUT");
    expect(JSON.parse(called()[1]?.body as string)).toEqual({
      ...DRAFT,
      expected_revision: 3,
    });

    mockFetch.mockClear();
    mockFetch.mockResolvedValue(response(null, 204));
    await deleteAlert("a1");
    expect(called()).toEqual(["/api/v3/v1/alerts/a1", { method: "DELETE" }]);
  });

  it("creates an alert with the draft as its body", async () => {
    await createAlert(DRAFT);

    expect(called()[0]).toBe("/api/v3/v1/alerts");
    expect(called()[1]?.method).toBe("POST");
    expect(JSON.parse(called()[1]?.body as string)).toEqual(DRAFT);
  });

  it.each([
    [true, "/api/v3/v1/alerts/a1/enable"],
    [false, "/api/v3/v1/alerts/a1/disable"],
  ])("turns checks %s at the revision it holds", async (enabled, url) => {
    await setAlertEnabled("a1", enabled, 4);

    expect(called()[0]).toBe(url);
    expect(JSON.parse(called()[1]?.body as string)).toEqual({
      expected_revision: 4,
    });
  });

  it("pages notifications and reads the destinations list", async () => {
    await fetchAlertNotifications("a1", { limit: 20, offset: 20 });
    expect(called()[0]).toBe(
      "/api/v3/v1/alerts/a1/notifications?limit=20&offset=20"
    );

    mockFetch.mockClear();
    mockFetch.mockResolvedValue(
      response({ destinations: [{ name: "ops", provider: "zulip" }] })
    );
    await expect(fetchAlertDestinations()).resolves.toEqual([
      { name: "ops", provider: "zulip" },
    ]);
    expect(called()[0]).toBe("/api/v3/v1/alert-destinations");
  });

  it("refuses with what the service said", async () => {
    mockFetch.mockResolvedValue(response({ detail: "no" }, 409));

    await expect(fetchAlert("a1")).rejects.toMatchObject({
      status: 409,
      body: { detail: "no" },
    });
  });

  it("never sends a request for an empty id", async () => {
    await expect(fetchAlert("")).rejects.toBeInstanceOf(CustomApiError);
    expect(mockFetch).not.toHaveBeenCalled();
  });
});
