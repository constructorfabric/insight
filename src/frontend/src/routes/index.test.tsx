// @vitest-environment jsdom
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
  component?: () => React.ReactNode;
};

function thrownByBeforeLoad(): unknown {
  try {
    route.beforeLoad();
  } catch (error) {
    return error;
  }
  return undefined;
}

describe("/", () => {
  it("sends every reader into the portal", () => {
    expect(thrownByBeforeLoad()).toEqual({ redirect: { to: "/portal" } });
  });

  it("carries no page of its own to land on", () => {
    expect(route.component).toBeUndefined();
  });
});
