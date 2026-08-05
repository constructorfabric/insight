import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { consumeOverrideParam } from "./override";

describe("consumeOverrideParam", () => {
  let assign: ReturnType<typeof vi.fn>;

  function stubLocation(href: string) {
    const url = new URL(href);
    Object.defineProperty(window, "location", {
      configurable: true,
      value: { ...window.location, href: url.href, assign },
    });
  }

  beforeEach(() => {
    assign = vi.fn();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("bounces into /auth/login with the target and a cleaned return_to", () => {
    stubLocation(
      "https://insight.test/team/board?__override=ao%40constructor.tech&tab=stats"
    );

    expect(consumeOverrideParam()).toBe(true);
    expect(assign).toHaveBeenCalledWith(
      `/auth/login?__override=${encodeURIComponent("ao@constructor.tech")}&return_to=${encodeURIComponent("/team/board?tab=stats")}`
    );
  });

  it("is a no-op without the parameter", () => {
    stubLocation("https://insight.test/?tab=stats");

    expect(consumeOverrideParam()).toBe(false);
    expect(assign).not.toHaveBeenCalled();
  });
});
