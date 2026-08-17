import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ createTelemetry: vi.fn() }));

vi.mock("@gears-frontx/telemetry", () => {
  const service = {
    identify: () => service,
    start: () => service,
    logEvent: () => {},
    destroy: () => {},
  };
  mocks.createTelemetry.mockImplementation(() => service);
  return { createTelemetry: mocks.createTelemetry };
});

vi.mock("@/api/usage-client", () => ({
  getUsageConfig: () => Promise.resolve({ enabled: true }),
}));

const SESSION = { personId: "p1", impersonatorEmail: null };

beforeEach(() => {
  vi.resetModules();
  mocks.createTelemetry.mockClear();
});

afterEach(() => {
  vi.unstubAllEnvs();
});

describe("the release the collector reports", () => {
  it("is the one the image was built with", async () => {
    vi.stubEnv("VITE_APP_RELEASE", "2026.08.17.06.05-2d6d2b2");

    const { startUsageTelemetry } = await import("./telemetry");
    await startUsageTelemetry(SESSION as never);

    expect(mocks.createTelemetry).toHaveBeenCalledWith(
      expect.objectContaining({ appVersion: "2026.08.17.06.05-2d6d2b2" }),
    );
  });

  it("falls back to a placeholder only when nothing stamped one", async () => {
    vi.stubEnv("VITE_APP_RELEASE", "");

    const { startUsageTelemetry } = await import("./telemetry");
    await startUsageTelemetry(SESSION as never);

    expect(mocks.createTelemetry).toHaveBeenCalledWith(
      expect.objectContaining({ appVersion: "0.0.0" }),
    );
  });
});
