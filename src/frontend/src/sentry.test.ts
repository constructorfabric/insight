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
});
