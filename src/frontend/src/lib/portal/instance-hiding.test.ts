/**
 * The install's `nav.hide` policy applied across the navigation model: every
 * nesting level hides, hidden entries vanish rather than demote to "Planned",
 * and the full catalog stays available where labels and telemetry read it.
 */
import { describe, expect, it } from "vitest";

import { visibleDirections, visibleLenses } from "./lens-configs";
import { landingDecision } from "./landing-zone";
import { parseNavHide } from "./nav-hide";
import {
  DIRECTIONS,
  lensSlug,
  manageItemsFor,
  peopleItemsFor,
  resolveZoneItem,
  zoneItems,
  zoneSections,
} from "./nav-model";

const dev = DIRECTIONS.find((d) => d.id === "dev")!;
const wiki = DIRECTIONS.find((d) => d.id === "wiki")!;

describe("zoneSections under nav.hide", () => {
  it("drops a hidden item from its group", () => {
    const policy = parseNavHide(["zone:overview/item:trend"]);

    const items = zoneSections("overview", policy).flatMap((g) => g.items);

    expect(items.map((i) => i.id)).not.toContain("trend");
  });

  it("drops a group whose every item is hidden", () => {
    const cost = zoneSections("aicost").find((g) => g.label === "Cost")!;
    const policy = parseNavHide(cost.items.map((i) => `zone:aicost/item:${i.id}`));

    const labels = zoneSections("aicost", policy).map((g) => g.label);

    expect(labels).not.toContain("Cost");
  });

  it("keeps the untouched groups intact", () => {
    const policy = parseNavHide(["zone:aicost/item:adoption-funnel"]);

    expect(zoneSections("overview", policy)).toEqual(zoneSections("overview"));
  });
});

describe("resolveZoneItem under nav.hide", () => {
  it("sends a deep link into a hidden item to the first shown entry", () => {
    const policy = parseNavHide(["zone:overview/item:trend"]);

    expect(resolveZoneItem("overview", "trend", policy)).toBe("at-a-glance");
  });

  it("skips a hidden item when picking the zone default", () => {
    const policy = parseNavHide(["zone:overview/item:at-a-glance"]);

    expect(resolveZoneItem("overview", null, policy)).toBe("by-direction");
  });

  it("returns null when the policy hides everything built", () => {
    const policy = parseNavHide(
      zoneItems("people").map((i) => `zone:people/item:${i.id}`),
    );

    expect(resolveZoneItem("people", null, policy)).toBeNull();
  });
});

describe("pane lists under nav.hide", () => {
  it("hides a People item in both deployment shapes", () => {
    const policy = parseNavHide(["zone:people/item:employees"]);

    for (const isFlat of [true, false]) {
      const ids = peopleItemsFor(isFlat, policy).map((i) => i.id);
      expect(ids, `isFlat=${isFlat}`).not.toContain("employees");
    }
  });

  it("hides a Manage item even for an admin", () => {
    const policy = parseNavHide(["zone:manage/item:metric-catalog"]);

    expect(manageItemsFor(true, policy).map((i) => i.id)).not.toContain(
      "metric-catalog",
    );
  });
});

describe("directions under nav.hide", () => {
  it("drops a hidden direction", () => {
    const policy = parseNavHide(["zone:directions/dir:sales"]);

    expect(visibleDirections(true, policy).map((d) => d.id)).not.toContain("sales");
  });

  it("drops a hidden lens by its slug", () => {
    const policy = parseNavHide(["zone:directions/dir:dev/lens:git-output"]);

    expect(visibleLenses(dev, true, policy)).not.toContain("Git output");
  });

  it("drops a direction whose every lens is hidden", () => {
    const policy = parseNavHide(
      wiki.lenses.map((l) => `zone:directions/dir:wiki/lens:${lensSlug(l)}`),
    );

    expect(visibleDirections(true, policy).map((d) => d.id)).not.toContain("wiki");
  });
});

describe("what nav.hide must NOT touch", () => {
  it("leaves the full item catalog for labels and telemetry", () => {
    // zoneItems is deliberately unfiltered — screen labels and the usage
    // catalog enumerate everything the model knows, hidden or not.
    expect(zoneItems("overview").map((i) => i.id)).toContain("trend");
  });
});

describe("landing under nav.hide", () => {
  const resolved = { mgrPending: false, adminPending: false, isAdmin: false };

  it("keeps the route-driven view instead of pinning a hidden Overview", () => {
    const decision = landingDecision({
      zone: null,
      ...resolved,
      canSeeOthers: true,
      overviewHidden: true,
    });

    expect(decision.kind).toBe("keep");
  });

  it("still pins Overview when the install shows it", () => {
    const decision = landingDecision({
      zone: null,
      ...resolved,
      canSeeOthers: true,
      overviewHidden: false,
    });

    expect(decision.kind).toBe("pin-overview");
  });
});
