import * as Sentry from "@sentry/react";
import type { BrowserOptions } from "@sentry/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { initSentry } from "./sentry";

vi.mock("@sentry/react", () => ({
  init: vi.fn(),
  tanstackRouterBrowserTracingIntegration: vi.fn(),
}));

const DSN = "https://public@sentry.example.com/1";
const ROUTER = { id: "router" };

function initWith(): BrowserOptions {
  vi.stubEnv("VITE_SENTRY_DSN", DSN);
  initSentry(ROUTER);
  return vi.mocked(Sentry.init).mock.calls[0][0]!;
}

beforeEach(() => {
  vi.mocked(Sentry.init).mockClear();
});

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("initSentry", () => {
  it("does nothing without a DSN", () => {
    vi.stubEnv("VITE_SENTRY_DSN", "");

    initSentry(ROUTER);

    expect(Sentry.init).not.toHaveBeenCalled();
  });

  it("labels the environment with the hostname off localhost", () => {
    vi.stubGlobal("location", new URL("https://stand.example.com/ic"));

    expect(initWith().environment).toBe("stand.example.com");
  });

  it("labels localhost as local", () => {
    expect(initWith().environment).toBe("local");
  });

  it("prefers the runtime config over the build-time env", () => {
    vi.stubEnv("VITE_SENTRY_DSN", "https://public@sentry.example.com/2");
    vi.stubGlobal("__INSIGHT_CONFIG__", { sentryDsn: DSN });

    initSentry(ROUTER);

    expect(vi.mocked(Sentry.init).mock.calls[0][0]!.dsn).toBe(DSN);
  });

  it("survives an init that throws", () => {
    vi.mocked(Sentry.init).mockImplementationOnce(() => {
      throw new Error("bad dsn");
    });
    vi.spyOn(console, "error").mockImplementation(() => {});

    expect(() => initWith()).not.toThrow();
  });

  // What the chart renders when the operator leaves `sentry.dsn` unset. The
  // env var is set here so the case fails if the read ever falls through it.
  it("does nothing when the runtime config carries an empty DSN", () => {
    vi.stubEnv("VITE_SENTRY_DSN", DSN);
    vi.stubGlobal("__INSIGHT_CONFIG__", { sentryDsn: "" });

    initSentry(ROUTER);

    expect(Sentry.init).not.toHaveBeenCalled();
  });
});
