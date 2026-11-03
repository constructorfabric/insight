/**
 * The landing rules, as a table. The case that motivated the module: an
 * admin who manages nobody must keep `?zone=manage`, and must not be reset
 * while the role check is still in flight.
 */
import { describe, expect, it } from "vitest";

import { landingDecision, type LandingDecision } from "./landing-zone";

const resolved = { mgrPending: false, adminPending: false, homeShown: true };
const unresolved = { mgrPending: true, adminPending: true, homeShown: true };

describe("landingDecision", () => {
  it.each<
    [string, Parameters<typeof landingDecision>[0], LandingDecision["kind"]]
  >([
    [
      "waits while manager status resolves",
      {
        zone: "manage",
        mgrPending: true,
        canSeeOthers: false,
        adminPending: false,
        isAdmin: false,
        homeShown: true,
      },
      "wait",
    ],
    [
      "anyone on bare /portal lands on Home without waiting",
      { zone: null, ...unresolved, canSeeOthers: false, isAdmin: false },
      "keep",
    ],
    [
      "Home stands without waiting",
      { zone: "overview", ...unresolved, canSeeOthers: false, isAdmin: false },
      "keep",
    ],
    [
      "the viewer's own page stands without waiting",
      { zone: "person", ...unresolved, canSeeOthers: false, isAdmin: false },
      "keep",
    ],
    [
      "Dashboards stand without waiting",
      { zone: "custom", ...unresolved, canSeeOthers: false, isAdmin: false },
      "keep",
    ],
    [
      "Home is left when the install does not show it",
      {
        zone: "overview",
        ...resolved,
        homeShown: false,
        canSeeOthers: true,
        isAdmin: false,
      },
      "reset",
    ],
    [
      "an IC keeps Home",
      { zone: "overview", ...resolved, canSeeOthers: false, isAdmin: false },
      "keep",
    ],
    [
      "manager keeps whatever zone they picked",
      { zone: "directions", ...resolved, canSeeOthers: true, isAdmin: false },
      "keep",
    ],
    [
      "an IC on an org zone is reset",
      { zone: "directions", ...resolved, canSeeOthers: false, isAdmin: false },
      "reset",
    ],
    [
      "a non-manager on manage WAITS for the admin answer instead of resetting",
      {
        zone: "manage",
        mgrPending: false,
        canSeeOthers: false,
        adminPending: true,
        isAdmin: false,
        homeShown: true,
      },
      "wait",
    ],
    [
      "an admin who manages nobody keeps manage",
      { zone: "manage", ...resolved, canSeeOthers: false, isAdmin: true },
      "keep",
    ],
    [
      "a non-admin non-manager on manage is reset once the answer is in",
      { zone: "manage", ...resolved, canSeeOthers: false, isAdmin: false },
      "reset",
    ],
  ])("%s", (_name, args, expected) => {
    expect(landingDecision(args).kind).toBe(expected);
  });
});
