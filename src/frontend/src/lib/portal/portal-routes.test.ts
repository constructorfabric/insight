import { describe, expect, it } from "vitest";

import { isPortalShellPath } from "./portal-routes";

describe("isPortalShellPath", () => {
  it("claims the org zones and the person screens", () => {
    expect(isPortalShellPath("/portal")).toBe(true);
    expect(isPortalShellPath("/ic/019e27bc-dec0-7626-81a9-c5524662a6a9/personal")).toBe(true);
    expect(isPortalShellPath("/ic/019e27bc-dec0-7626-81a9-c5524662a6a9/team")).toBe(true);
    expect(isPortalShellPath("/ic/019e27bc-dec0-7626-81a9-c5524662a6a9/team/")).toBe(true);
  });

  it("leaves the rest of the app its own chrome", () => {
    // An earlier "the portal replaces the app" branch swallowed these.
    for (const path of ["/", "/metrics", "/queries", "/whats-new"]) {
      expect(isPortalShellPath(path)).toBe(false);
    }
  });

  it("does not claim a person route it has no screen for", () => {
    // A future /ic/<id>/something renders through the app chrome rather than
    // silently handing the screen to a shell that would show nothing.
    expect(isPortalShellPath("/ic/019e27bc-dec0-7626-81a9-c5524662a6a9")).toBe(false);
    expect(isPortalShellPath("/ic/019e27bc-dec0-7626-81a9-c5524662a6a9/reports")).toBe(false);
  });

  it("is not fooled by a prefix", () => {
    expect(isPortalShellPath("/portal-admin")).toBe(false);
    expect(isPortalShellPath("/not/portal")).toBe(false);
  });
});
