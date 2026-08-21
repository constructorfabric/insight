import { afterEach, describe, expect, it, vi } from "vitest";

import { publishBuildInfo } from "./build-info";

const RELEASE = "1970.01.01.00.00-abc1234";

function answered(body: unknown): Response {
  return { ok: true, json: () => Promise.resolve(body) } as Response;
}

type Answer = (init?: RequestInit) => Promise<Response>;

function stubFetch(analytics: Answer): void {
  vi.stubGlobal(
    "fetch",
    vi.fn((url: string, init?: RequestInit) =>
      url.includes("analytics")
        ? analytics(init)
        : Promise.resolve(answered({ service: "identity", version: "id-2" })),
    ),
  );
}

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
  vi.useRealTimers();
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
    stubFetch(() =>
      Promise.resolve(answered({ service: "analytics", version: "an-1" })),
    );

    publishBuildInfo();

    await expect(window.__INSIGHT_BUILD__?.backend()).resolves.toEqual({
      analytics: "an-1",
      identity: "id-2",
    });
  });

  it.each([
    ["the network is down", () => Promise.reject(new Error("network down"))],
    [
      "the caller is refused",
      () => Promise.resolve({ ok: false, status: 401 } as Response),
    ],
    [
      "the answer carries no version",
      () => Promise.resolve(answered({ service: "analytics" })),
    ],
  ])(
    "keeps the services that answered when %s",
    async (_case, analytics: () => Promise<Response>) => {
      stubFetch(analytics);

      publishBuildInfo();

      await expect(window.__INSIGHT_BUILD__?.backend()).resolves.toEqual({
        analytics: "unreachable",
        identity: "id-2",
      });
    },
  );

  it("gives up on a service that never answers", async () => {
    vi.useFakeTimers();
    stubFetch(
      (init) =>
        new Promise<Response>((_resolve, reject) => {
          init?.signal?.addEventListener("abort", () =>
            reject(new Error("aborted")),
          );
        }),
    );

    publishBuildInfo();
    const pending = window.__INSIGHT_BUILD__?.backend();
    await vi.advanceTimersByTimeAsync(5000);

    await expect(pending).resolves.toEqual({
      analytics: "unreachable",
      identity: "id-2",
    });
  });
});
