import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { publishBuildInfo } from "./build-info";

const RELEASE = "2026.08.20.10.11-f8db7bc";

function answered(body: unknown): Response {
  return { ok: true, json: () => Promise.resolve(body) } as Response;
}

beforeEach(() => {
  delete window.__INSIGHT_BUILD__;
});

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});

describe("the frontend build the browser is running", () => {
  it("is the release the image was built with", () => {
    vi.stubEnv("VITE_APP_RELEASE", RELEASE);

    publishBuildInfo();

    expect(window.__INSIGHT_BUILD__?.frontend).toBe(RELEASE);
  });

  it("reads unknown when nothing stamped a release", () => {
    vi.stubEnv("VITE_APP_RELEASE", "");

    publishBuildInfo();

    expect(window.__INSIGHT_BUILD__?.frontend).toBe("unknown");
  });
});

describe("the backend builds behind the gateway", () => {
  it("reports the version each service answers with", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn((url: string) =>
        Promise.resolve(
          answered({
            service: "s",
            version: url.includes("analytics") ? "an-1" : "id-2",
          }),
        ),
      ),
    );

    publishBuildInfo();

    await expect(window.__INSIGHT_BUILD__?.backend()).resolves.toEqual({
      analytics: "an-1",
      identity: "id-2",
    });
  });

  it("keeps the services that answered when one is unreachable", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn((url: string) =>
        url.includes("analytics")
          ? Promise.reject(new Error("network down"))
          : Promise.resolve(answered({ service: "identity", version: "id-2" })),
      ),
    );

    publishBuildInfo();

    await expect(window.__INSIGHT_BUILD__?.backend()).resolves.toEqual({
      analytics: "unreachable",
      identity: "id-2",
    });
  });

  it("keeps the services that answered when one refuses the caller", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn((url: string) =>
        Promise.resolve(
          url.includes("analytics")
            ? ({ ok: false, status: 401 } as Response)
            : answered({ service: "identity", version: "id-2" }),
        ),
      ),
    );

    publishBuildInfo();

    await expect(window.__INSIGHT_BUILD__?.backend()).resolves.toEqual({
      analytics: "unreachable",
      identity: "id-2",
    });
  });
});
