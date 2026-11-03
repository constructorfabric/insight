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
  navigate: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mocks.navigate,
}));
vi.mock("@/auth", () => ({ useViewer: () => ({ personId: "me-1" }) }));
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
  it("offers a viewer with no cohort Home, their own page and the dashboards shared with them", () => {
    mocks.canSeeOthers = false;
    mocks.reachPending = false;
    mocks.isAdmin = false;

    expect(zoneIds()).toEqual(["overview", "person", "custom"]);
  });

  it("opens Custom to a member who manages a cohort", () => {
    mocks.canSeeOthers = true;
    mocks.reachPending = false;
    mocks.isAdmin = false;

    expect(zoneIds()).toContain("custom");
    expect(zoneIds()).toContain("overview");
  });

  it("opens Custom to an admin who manages nobody", () => {
    mocks.canSeeOthers = false;
    mocks.reachPending = false;
    mocks.isAdmin = true;

    expect(zoneIds()).toEqual(["overview", "person", "custom", "manage"]);
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

  it("offers Reports with planned sections off, now that it has a page", () => {
    mocks.canSeeOthers = true;
    mocks.reachPending = false;
    mocks.showPlanned = false;
    expect(zoneIds()).toContain("reports");
  });

  it("opens Person on the viewer's own page, not on the person on screen", () => {
    const { result } = renderHook(() => useZoneNav());
    const person = result.current.zones.find((z) => z.id === "person")!;

    result.current.selectZone(person);

    expect(mocks.navigate).toHaveBeenCalledWith(
      expect.objectContaining({
        to: "/ic/$person/personal",
        params: { person: "me-1" },
      })
    );
  });

  it("opens Manage on the admin role rather than on a cohort", () => {
    mocks.canSeeOthers = false;
    mocks.reachPending = false;
    mocks.isAdmin = true;

    const ids = zoneIds();

    expect(ids).toContain("manage");
    expect(ids).not.toContain("directions");
  });
});
