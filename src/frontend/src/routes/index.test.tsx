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

import { afterEach, describe, expect, it, vi } from "vitest";

import { setPortalShowPlanned } from "@/lib/portal/portal-store";

import { Route } from "./index";

const viewer = vi.hoisted(() => ({ personId: "aaaaaaaa-1111-4111-8111-111111111111" }));
vi.mock("@/auth", () => ({ getViewerPersonId: () => viewer.personId }));
vi.mock("@/lib/portal/nav-policy", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/portal/nav-policy")>();
  return { ...actual, zoneHidden: (zone: string) => policy.hidden.has(zone) };
});
const policy = vi.hoisted(() => ({ hidden: new Set<string>() }));

afterEach(() => {
  policy.hidden.clear();
  setPortalShowPlanned(false);
});

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
  it("sends the reader Home", () => {
    expect(thrownByBeforeLoad()).toEqual({ redirect: { to: "/home", params: {} } });
  });

  it("sends the reader to their own page where the install leaves Home out", () => {
    policy.hidden.add("overview");

    expect(thrownByBeforeLoad()).toEqual({
      redirect: { to: "/people/$person", params: { person: viewer.personId } },
    });
  });

  it("carries no page of its own to land on", () => {
    expect(route.component).toBeUndefined();
  });
});
