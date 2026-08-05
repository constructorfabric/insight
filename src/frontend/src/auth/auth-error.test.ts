import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  vi,
  type MockInstance,
} from "vitest";

import { clearAuthErrorAttempts, consumeAuthErrorParam } from "./auth-error";

describe("consumeAuthErrorParam", () => {
  let replaceState: MockInstance<History["replaceState"]>;

  function stubLocation(href: string) {
    const url = new URL(href);
    Object.defineProperty(window, "location", {
      configurable: true,
      value: { ...window.location, href: url.href },
    });
  }

  beforeEach(() => {
    replaceState = vi
      .spyOn(window.history, "replaceState")
      .mockImplementation(() => undefined);
    sessionStorage.clear();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("is a no-op without the parameter", () => {
    stubLocation("https://insight.test/?tab=stats");

    expect(consumeAuthErrorParam()).toBeNull();
    expect(replaceState).not.toHaveBeenCalled();
  });

  it("consumes the parameter, keeps the rest of the URL, and allows one auto-retry", () => {
    stubLocation("https://insight.test/?auth_error=state_expired&tab=stats");

    expect(consumeAuthErrorParam()).toEqual({
      code: "state_expired",
      autoRetry: true,
    });
    expect(replaceState).toHaveBeenCalledWith(null, "", "/?tab=stats");
  });

  it("halts after the auto-retry budget is spent", () => {
    stubLocation("https://insight.test/?auth_error=state_expired");
    expect(consumeAuthErrorParam()?.autoRetry).toBe(true);

    // The retried login failed again: same bounce, same tab.
    stubLocation("https://insight.test/?auth_error=state_expired");
    expect(consumeAuthErrorParam()?.autoRetry).toBe(false);
  });

  it("strips an empty auth_error without counting it", () => {
    stubLocation("https://insight.test/?auth_error=&tab=stats");

    expect(consumeAuthErrorParam()).toBeNull();
    expect(replaceState).toHaveBeenCalledWith(null, "", "/?tab=stats");
    // The budget is untouched: the next real failure still auto-retries.
    stubLocation("https://insight.test/?auth_error=state_expired");
    expect(consumeAuthErrorParam()?.autoRetry).toBe(true);
  });

  it("preserves the hash when stripping the parameter", () => {
    stubLocation("https://insight.test/board?auth_error=idp_error#section-2");

    expect(consumeAuthErrorParam()?.code).toBe("idp_error");
    expect(replaceState).toHaveBeenCalledWith(null, "", "/board#section-2");
  });

  it("never auto-retries access_denied", () => {
    stubLocation("https://insight.test/?auth_error=access_denied");

    expect(consumeAuthErrorParam()).toEqual({
      code: "access_denied",
      autoRetry: false,
    });
  });

  it("clearAuthErrorAttempts restores the auto-retry budget", () => {
    stubLocation("https://insight.test/?auth_error=state_expired");
    expect(consumeAuthErrorParam()?.autoRetry).toBe(true);
    stubLocation("https://insight.test/?auth_error=state_expired");
    expect(consumeAuthErrorParam()?.autoRetry).toBe(false);

    clearAuthErrorAttempts();

    stubLocation("https://insight.test/?auth_error=state_expired");
    expect(consumeAuthErrorParam()?.autoRetry).toBe(true);
  });

  it("does not auto-retry when the attempt cannot be persisted", () => {
    stubLocation("https://insight.test/?auth_error=state_expired");
    // Reads work, writes fail (e.g. quota): the next bounce would read zero
    // again, so an unpersisted attempt must not spend a retry.
    vi.stubGlobal("sessionStorage", {
      getItem: () => null,
      setItem: () => {
        throw new Error("quota exceeded");
      },
      removeItem: () => undefined,
    });

    expect(consumeAuthErrorParam()).toEqual({
      code: "state_expired",
      autoRetry: false,
    });
  });

  it("fails closed to the error screen when storage is unavailable", () => {
    stubLocation("https://insight.test/?auth_error=state_expired");
    // Replace the global outright — spying on methods of the environment's
    // Storage object does not reliably intercept the module's binding.
    const disabled = () => {
      throw new Error("storage disabled");
    };
    vi.stubGlobal("sessionStorage", {
      getItem: disabled,
      setItem: disabled,
      removeItem: disabled,
    });

    expect(consumeAuthErrorParam()?.autoRetry).toBe(false);
  });
});
