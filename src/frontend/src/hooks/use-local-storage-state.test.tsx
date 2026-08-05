import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  parseLocalStorageBoolean,
  serializeLocalStorageBoolean,
  useLocalStorageState,
} from "@/hooks/use-local-storage-state";

describe("useLocalStorageState", () => {
  beforeEach(() => {
    localStorage.clear();
    vi.restoreAllMocks();
  });

  it("parses and serializes booleans", () => {
    expect(parseLocalStorageBoolean("true")).toBe(true);
    expect(parseLocalStorageBoolean("false")).toBe(false);
    expect(parseLocalStorageBoolean("invalid")).toBeUndefined();
    expect(serializeLocalStorageBoolean(true)).toBe("true");
  });

  it("reads, updates, and persists a value", () => {
    localStorage.setItem("view", "false");
    const { result } = renderHook(() =>
      useLocalStorageState({
        key: "view",
        defaultValue: true,
        parse: parseLocalStorageBoolean,
        serialize: serializeLocalStorageBoolean,
      })
    );
    expect(result.current[0]).toBe(false);
    act(() => result.current[1]((current) => !current));
    expect(result.current[0]).toBe(true);
    expect(localStorage.getItem("view")).toBe("true");
  });

  it("resets from storage when the key changes", () => {
    localStorage.setItem("second", "true");
    const { result, rerender } = renderHook(
      ({ storageKey }) =>
        useLocalStorageState({
          key: storageKey,
          defaultValue: false,
          parse: parseLocalStorageBoolean,
          serialize: serializeLocalStorageBoolean,
        }),
      { initialProps: { storageKey: "first" } }
    );
    expect(result.current[0]).toBe(false);
    rerender({ storageKey: "second" });
    expect(result.current[0]).toBe(true);
  });

  it("falls back when storage access or parsing fails", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("denied");
    });
    const { result } = renderHook(() =>
      useLocalStorageState({
        key: "view",
        defaultValue: true,
        parse: parseLocalStorageBoolean,
        serialize: serializeLocalStorageBoolean,
      })
    );
    expect(result.current[0]).toBe(true);
  });
});
