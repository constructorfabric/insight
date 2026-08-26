/**
 * An address the app has no route for is a bounce to the default page rather
 * than a surface of its own, so a mistyped or stale link leaves the reader a
 * way forward. "/" owns which page that is.
 */
vi.mock("@tanstack/react-router", () => ({
  // The real one throws its own control-flow object; this stands in for it so
  // the test can read the destination back off what was thrown.
  redirect: vi.fn((options: unknown) => ({ redirect: options })),
  createFileRoute: () => (options: Record<string, unknown>) => options,
}));

import { describe, expect, it, vi } from "vitest";

import { Route } from "./$";

const route = Route as unknown as { beforeLoad: () => void };

function thrownByBeforeLoad(): unknown {
  try {
    route.beforeLoad();
  } catch (error) {
    return error;
  }
  return undefined;
}

describe("/$", () => {
  it("sends an unmatched address to the default page", () => {
    expect(thrownByBeforeLoad()).toEqual({ redirect: { to: "/" } });
  });
});
