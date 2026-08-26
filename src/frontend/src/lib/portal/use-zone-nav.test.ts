/**
 * Which zones the rail offers. The gate is not "does this viewer manage
 * anyone" but "do the org zones have a cohort" — a member of an organisation
 * with no reporting lines has one, and used to be shown their own page alone.
 */
import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  canSeeOthers: false,
  reachPending: false,
  isAdmin: false,
  showPlanned: false,
}));

vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));
vi.mock("@/lib/portal/use-active-zone", () => ({
  useActiveZone: () => ({ activeZone: "person", activePerson: "p-1" }),
}));
vi.mock("@/lib/portal/use-viewer-reach", () => ({
  useViewerReach: () => ({
    canSeeOthers: mocks.canSeeOthers,
    isManager: false,
    isPending: mocks.reachPending,
  }),
}));
vi.mock("@/lib/portal/portal-store", () => ({
  usePortalShowPlanned: () => mocks.showPlanned,
}));
vi.mock("@/queries/identity-me", () => ({
  useIsAdmin: () => ({ isAdmin: mocks.isAdmin, isPending: false }),
}));

import { useZoneNav } from "./use-zone-nav";

function zoneIds(): string[] {
  const { result } = renderHook(() => useZoneNav());
  return result.current.zones.map((z) => z.id);
}

describe("useZoneNav", () => {
  it("offers a viewer with no cohort their own page only", () => {
    mocks.canSeeOthers = false;
    mocks.reachPending = false;
    mocks.isAdmin = false;

    expect(zoneIds()).toEqual(["person"]);
  });

  it("offers the org zones once the viewer has a cohort", () => {
    // The flat-organisation case: no reports, and still an organisation.
    mocks.canSeeOthers = true;

    const ids = zoneIds();

    expect(ids).toContain("overview");
    expect(ids).toContain("people");
    expect(ids).toContain("person");
  });

  it("assumes a cohort while the answer is in flight, to avoid a flash", () => {
    mocks.canSeeOthers = false;
    mocks.reachPending = true;

    expect(zoneIds()).toContain("overview");
  });

  it("opens Manage on the admin role rather than on a cohort", () => {
    mocks.canSeeOthers = false;
    mocks.reachPending = false;
    mocks.isAdmin = true;

    const ids = zoneIds();

    expect(ids).toContain("manage");
    expect(ids).not.toContain("overview");
  });
});
