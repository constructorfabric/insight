import { describe, expect, it } from "vitest";

import {
  catalogueWithheldAt,
  visibleCatalogues,
} from "@/lib/custom/catalogues";
import { EMPTY_NAV_POLICY, parseNavPolicy } from "@/lib/portal/nav-policy";

const ALERTS_PLANNED = parseNavPolicy({ planned: ["zone:custom/item:alerts"] });
const ALERTS_HIDDEN = parseNavPolicy({ hide: ["zone:custom/item:alerts"] });
const ALERTS_BOTH = parseNavPolicy({
  hide: ["zone:custom/item:alerts"],
  planned: ["zone:custom/item:alerts"],
});
const METRICS_PLANNED = parseNavPolicy({
  planned: ["zone:custom/item:metrics"],
});

function shown(showPlanned: boolean, policy = EMPTY_NAV_POLICY) {
  return visibleCatalogues(showPlanned, policy).map(({ id, planned }) => ({
    id,
    planned,
  }));
}

describe("visibleCatalogues", () => {
  it("lists every catalogue as live when the install marks none", () => {
    expect(shown(false)).toEqual([
      { id: "metrics", planned: false },
      { id: "widgets", planned: false },
      { id: "datasets", planned: false },
      { id: "alerts", planned: false },
    ]);
  });

  it("drops a planned catalogue while planned sections are off", () => {
    expect(shown(false, ALERTS_PLANNED).map(({ id }) => id)).toEqual([
      "metrics",
      "widgets",
      "datasets",
    ]);
  });

  it("keeps a planned catalogue, marked planned, while planned sections are on", () => {
    expect(shown(true, ALERTS_PLANNED)).toContainEqual({
      id: "alerts",
      planned: true,
    });
  });

  it.each([
    { name: "hidden", policy: ALERTS_HIDDEN },
    { name: "both hidden and planned", policy: ALERTS_BOTH },
  ])("drops a catalogue $name whatever the reader chose", ({ policy }) => {
    expect(shown(true, policy).map(({ id }) => id)).not.toContain("alerts");
  });
});

describe("catalogueWithheldAt", () => {
  it.each([
    {
      path: "/portal/custom/alerts",
      showPlanned: false,
      policy: ALERTS_PLANNED,
    },
    {
      path: "/portal/custom/alerts/",
      showPlanned: false,
      policy: ALERTS_PLANNED,
    },
    {
      path: "/portal/custom/Alerts",
      showPlanned: false,
      policy: ALERTS_PLANNED,
    },
    {
      path: "/portal/custom/alerts/new",
      showPlanned: false,
      policy: ALERTS_PLANNED,
    },
    {
      path: "/portal/custom/alerts/a1/edit",
      showPlanned: false,
      policy: ALERTS_PLANNED,
    },
    {
      path: "/portal/custom/alerts/a1",
      showPlanned: true,
      policy: ALERTS_HIDDEN,
    },
    { path: "/portal/custom/alerts", showPlanned: true, policy: ALERTS_BOTH },
    {
      path: "/portal/custom/new/metrics",
      showPlanned: false,
      policy: METRICS_PLANNED,
    },
    {
      path: "/portal/custom/edit/metrics/m1",
      showPlanned: false,
      policy: METRICS_PLANNED,
    },
  ])(
    "withholds $path with showPlanned=$showPlanned",
    ({ path, showPlanned, policy }) => {
      expect(
        catalogueWithheldAt(path, showPlanned, policy),
        `should withhold ${path} (showPlanned=${showPlanned})`
      ).toBe(true);
    }
  );

  it.each([
    {
      path: "/portal/custom/alerts",
      showPlanned: true,
      policy: ALERTS_PLANNED,
    },
    {
      path: "/portal/custom/alerts",
      showPlanned: false,
      policy: EMPTY_NAV_POLICY,
    },
    {
      path: "/portal/custom/metrics",
      showPlanned: false,
      policy: ALERTS_PLANNED,
    },
    {
      path: "/portal/custom/alertsboard",
      showPlanned: false,
      policy: ALERTS_PLANNED,
    },
    { path: "/portal/custom", showPlanned: false, policy: ALERTS_PLANNED },
    {
      path: "/portal/custom/new/dashboards",
      showPlanned: false,
      policy: METRICS_PLANNED,
    },
    {
      path: "/portal/custom/edit/widgets/w1",
      showPlanned: false,
      policy: METRICS_PLANNED,
    },
  ])(
    "lets $path through with showPlanned=$showPlanned",
    ({ path, showPlanned, policy }) => {
      expect(
        catalogueWithheldAt(path, showPlanned, policy),
        `should let through ${path} (showPlanned=${showPlanned})`
      ).toBe(false);
    }
  );
});
