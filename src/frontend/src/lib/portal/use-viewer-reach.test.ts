/**
 * Who the portal's org zones are for. The distinction this hook exists to draw:
 * a leaf IC on a hierarchical install has nobody to roll up, while a member of
 * an organisation with no reporting lines can see everyone — and both are
 * served the same empty `subordinates`.
 */
import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { IdentityPerson } from "@/types/insight";

const mocks = vi.hoisted(() => ({
  viewer: null as IdentityPerson | null,
  isFlat: false,
  policyPending: false,
  treePending: false,
}));

vi.mock("@/auth", () => ({ useViewer: () => ({ personId: "me" }) }));
vi.mock("@/queries/ic-dashboard", () => ({
  useIcPerson: () => ({ data: mocks.viewer, isPending: mocks.treePending }),
}));
vi.mock("@/queries/identity-me", () => ({
  useVisibilityPolicy: () => ({
    policy: mocks.isFlat ? "flat" : "org_chart",
    isFlat: mocks.isFlat,
    isPending: mocks.policyPending,
  }),
}));

import { useViewerReach } from "./use-viewer-reach";

function person(id: string, subordinates: IdentityPerson[] = []): IdentityPerson {
  return {
    person_id: id,
    email: `${id}@example.com`,
    display_name: id,
    subordinates,
  } as IdentityPerson;
}

describe("useViewerReach", () => {
  it("opens the org zones for a lead on a hierarchical install", () => {
    mocks.viewer = person("me", [person("report")]);
    mocks.isFlat = false;
    mocks.policyPending = false;
    mocks.treePending = false;

    const { result } = renderHook(() => useViewerReach());

    expect(result.current.canSeeOthers).toBe(true);
    expect(result.current.isManager).toBe(true);
  });

  it("keeps them shut for a leaf IC on a hierarchical install", () => {
    mocks.viewer = person("me");
    mocks.isFlat = false;

    const { result } = renderHook(() => useViewerReach());

    expect(result.current.canSeeOthers).toBe(false);
    expect(result.current.isManager).toBe(false);
  });

  it("opens them under a flat policy even with nobody to manage", () => {
    // The whole point: no reports, and still the organisation to look at.
    mocks.viewer = person("me");
    mocks.isFlat = true;

    const { result } = renderHook(() => useViewerReach());

    expect(result.current.canSeeOthers).toBe(true);
    expect(result.current.isManager).toBe(false);
  });

  it("waits rather than deciding while either answer is in flight", () => {
    mocks.viewer = null;
    mocks.isFlat = false;
    mocks.treePending = true;
    mocks.policyPending = true;

    const { result } = renderHook(() => useViewerReach());

    expect(result.current.isPending).toBe(true);
  });
});
