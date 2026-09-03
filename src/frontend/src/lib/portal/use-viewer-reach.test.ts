/**
 * Who the portal's org zones are for. The distinction this hook exists to draw:
 * a leaf IC on a hierarchical install has nobody to roll up, while a member of
 * an organisation with no reporting lines can see everyone — and both are
 * served the same empty `subordinates`.
 */
import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { PeopleListItem } from "@/api/identity-client";

const mocks = vi.hoisted(() => ({
  roster: [] as PeopleListItem[],
  isFlat: false,
  policyPending: false,
  rosterPending: false,
}));

vi.mock("@/auth", () => ({ useViewer: () => ({ personId: "me" }) }));
vi.mock("@/queries/visible-roster", () => ({
  useVisibleRoster: () => ({
    roster: mocks.roster,
    isPending: mocks.rosterPending,
    isError: false,
  }),
}));
vi.mock("@/queries/identity-me", () => ({
  useVisibilityPolicy: () => ({
    policy: mocks.isFlat ? "flat" : "org_chart",
    isFlat: mocks.isFlat,
    isPending: mocks.policyPending,
  }),
}));

import { useViewerReach } from "./use-viewer-reach";

function person(id: string, managerPersonId: string | null = null): PeopleListItem {
  return {
    person_id: id,
    email: `${id}@example.com`,
    display_name: id,
    first_name: null,
    last_name: null,
    username: null,
    attributes: {},
    manager_person_id: managerPersonId,
  };
}

describe("useViewerReach", () => {
  it("opens the org zones for a lead on a hierarchical install", () => {
    mocks.roster = [person("me"), person("report", "me")];
    mocks.isFlat = false;
    mocks.policyPending = false;
    mocks.rosterPending = false;

    const { result } = renderHook(() => useViewerReach());

    expect(result.current.canSeeOthers).toBe(true);
    expect(result.current.isManager).toBe(true);
  });

  it("keeps them shut for a leaf IC on a hierarchical install", () => {
    mocks.roster = [person("me")];
    mocks.isFlat = false;

    const { result } = renderHook(() => useViewerReach());

    expect(result.current.canSeeOthers).toBe(false);
    expect(result.current.isManager).toBe(false);
  });

  it("opens them under a flat policy even with nobody to manage", () => {
    // The whole point: no reports, and still the organisation to look at.
    mocks.roster = [person("me")];
    mocks.isFlat = true;

    const { result } = renderHook(() => useViewerReach());

    expect(result.current.canSeeOthers).toBe(true);
    expect(result.current.isManager).toBe(false);
  });

  it("waits rather than deciding while either answer is in flight", () => {
    mocks.roster = [];
    mocks.isFlat = false;
    mocks.rosterPending = true;
    mocks.policyPending = true;

    const { result } = renderHook(() => useViewerReach());

    expect(result.current.isPending).toBe(true);
  });
});
