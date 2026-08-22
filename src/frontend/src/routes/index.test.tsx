// @vitest-environment jsdom
/**
 * "/" is a redirect, not a page. The portal is the interface now, so there is
 * no second thing for this route to weigh up and nothing for it to render.
 */
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return {
    ...portalRouterMock(),
    // The real one throws its own control-flow object; this stands in for it so
    // the test can read the destination back off what was thrown.
    redirect: vi.fn((options: unknown) => ({ redirect: options })),
    createFileRoute: () => (options: Record<string, unknown>) => options,
  };
});

import { describe, expect, it, vi } from "vitest";

import { Route } from "./index";

const route = Route as unknown as {
  beforeLoad: () => void;
  component?: unknown;
};

describe("/", () => {
  it("sends every reader into the portal", () => {
    let thrown: unknown;
    try {
      route.beforeLoad();
    } catch (error) {
      thrown = error;
    }

    expect(thrown).toEqual({ redirect: { to: "/portal" } });
  });

  it("has nothing to render — the redirect is the whole route", () => {
    expect(route.component).toBeUndefined();
  });
});
