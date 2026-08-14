/**
 * The landing rules, as a table. The case that motivated the module: an
 * admin who manages nobody must keep `?zone=manage`, and must not be reset
 * while the role check is still in flight.
 */
import { describe, expect, it } from "vitest";

import { landingDecision, type LandingDecision } from "./landing-zone";

const resolved = { mgrPending: false, adminPending: false };

describe("landingDecision", () => {
  it.each<[string, Parameters<typeof landingDecision>[0], LandingDecision["kind"]]>([
    [
      "waits while manager status resolves",
      { zone: "manage", mgrPending: true, isManager: false, adminPending: false, isAdmin: false },
      "wait",
    ],
    [
      "manager on bare /portal lands on the org rollup",
      { zone: null, ...resolved, isManager: true, isAdmin: false },
      "pin-overview",
    ],
    [
      "manager keeps whatever zone they picked",
      { zone: "directions", ...resolved, isManager: true, isAdmin: false },
      "keep",
    ],
    [
      "an IC keeps the route-driven person view",
      { zone: null, ...resolved, isManager: false, isAdmin: false },
      "keep",
    ],
    [
      "an IC on an org zone is reset",
      { zone: "overview", ...resolved, isManager: false, isAdmin: false },
      "reset",
    ],
    [
      "a non-manager on manage WAITS for the admin answer instead of resetting",
      { zone: "manage", mgrPending: false, isManager: false, adminPending: true, isAdmin: false },
      "wait",
    ],
    [
      "an admin who manages nobody keeps manage",
      { zone: "manage", ...resolved, isManager: false, isAdmin: true },
      "keep",
    ],
    [
      "a non-admin non-manager on manage is reset once the answer is in",
      { zone: "manage", ...resolved, isManager: false, isAdmin: false },
      "reset",
    ],
  ])("%s", (_name, args, expected) => {
    expect(landingDecision(args).kind).toBe(expected);
  });
});
