import { describe, expect, it } from "vitest";

import { GROUPS, visibleGroups } from "@/lib/insight/groups";
import {
  overviewCardDirections,
  visibleDirections,
  visibleLenses,
  visibleSections,
} from "./lens-configs";
import { DEFAULT_OVERVIEW_ITEM, OVERVIEW_ITEMS } from "./overview-configs";
import { landingDecision } from "./landing-zone";
import { parseNavPolicy, visiblePersonSections } from "./nav-policy";
import {
  DIRECTIONS,
  lensSlug,
  manageItemsFor,
  partitionByReadiness,
  peopleItemsFor,
  resolveZoneItem,
  zoneItems,
  zoneSections,
} from "./nav-model";

function hide(paths: string[]) {
  return parseNavPolicy({ hide: paths });
}

function planned(paths: string[]) {
  return parseNavPolicy({ planned: paths });
}

const dev = DIRECTIONS.find((d) => d.id === "dev")!;
const wiki = DIRECTIONS.find((d) => d.id === "wiki")!;

describe("zoneSections under nav.hide", () => {
  it("drops a hidden item from its group", () => {
    const policy = hide(["zone:overview/item:trend"]);

    const items = zoneSections("overview", policy).flatMap((g) => g.items);

    expect(items.map((i) => i.id)).not.toContain("trend");
  });

  it("drops a group whose every item is hidden", () => {
    const cost = zoneSections("aicost").find((g) => g.label === "Cost")!;
    const policy = hide(cost.items.map((i) => `zone:aicost/item:${i.id}`));

    const labels = zoneSections("aicost", policy).map((g) => g.label);

    expect(labels).not.toContain("Cost");
  });

  it("keeps the untouched groups intact", () => {
    const policy = hide(["zone:aicost/item:adoption-funnel"]);

    expect(zoneSections("overview", policy)).toEqual(zoneSections("overview"));
  });
});

describe("zoneSections under nav.planned", () => {
  it("demotes an item marked by the install policy", () => {
    const policy = planned(["zone:overview/item:trend"]);
    const items = zoneSections("overview", policy).flatMap((g) => g.items);

    const split = partitionByReadiness(items, true);

    expect(split.planned.map((i) => i.id)).toContain("trend");
    expect(split.live.map((i) => i.id)).not.toContain("trend");
  });

  it("hides a marked item from a reader who opted out of planned sections", () => {
    const policy = planned(["zone:overview/item:trend"]);
    const items = zoneSections("overview", policy).flatMap((g) => g.items);

    const split = partitionByReadiness(items, false);

    expect([...split.live, ...split.planned].map((i) => i.id)).not.toContain(
      "trend"
    );
  });

  it("marks an item only through the install policy", () => {
    const policy = planned(["zone:aicost/item:per-tool"]);
    const items = zoneSections("aicost", policy).flatMap((g) => g.items);

    expect(items.find((i) => i.id === "per-tool")?.readiness).toBe("planned");
  });
});

describe("resolveZoneItem under the install policy", () => {
  it("sends a deep link into a hidden item to the first shown entry", () => {
    const policy = hide(["zone:overview/item:trend"]);

    expect(resolveZoneItem("overview", "trend", policy)).toBe("at-a-glance");
  });

  it("skips a hidden item when picking the zone default", () => {
    const policy = hide(["zone:overview/item:at-a-glance"]);

    expect(resolveZoneItem("overview", null, policy)).toBe("by-direction");
  });

  it("skips a planned item as default but keeps it when the URL names it", () => {
    const policy = planned(["zone:overview/item:at-a-glance"]);

    expect(resolveZoneItem("overview", null, policy)).toBe("by-direction");
    expect(resolveZoneItem("overview", "at-a-glance", policy)).toBe(
      "at-a-glance"
    );
  });

  it("returns null when the policy hides everything built", () => {
    const policy = hide(
      zoneItems("people").map((i) => `zone:people/item:${i.id}`)
    );

    expect(resolveZoneItem("people", null, policy)).toBeNull();
  });
});

describe("pane lists under the install policy", () => {
  it("hides a People item in both deployment shapes", () => {
    const policy = hide(["zone:people/item:employees"]);

    for (const isFlat of [true, false]) {
      const ids = peopleItemsFor(isFlat, policy).map((i) => i.id);
      expect(ids, `isFlat=${isFlat}`).not.toContain("employees");
    }
  });

  it("hides a Manage item even for an admin", () => {
    const policy = hide(["zone:manage/item:metric-catalog"]);

    expect(manageItemsFor(true, policy).map((i) => i.id)).not.toContain(
      "metric-catalog"
    );
  });

  it("demotes a planned People or Manage item instead of dropping it", () => {
    const policy = planned([
      "zone:people/item:employees",
      "zone:manage/item:metric-catalog",
    ]);

    const people = peopleItemsFor(false, policy).find(
      (i) => i.id === "employees"
    );
    const manage = manageItemsFor(true, policy).find(
      (i) => i.id === "metric-catalog"
    );

    expect(people?.readiness).toBe("planned");
    expect(manage?.readiness).toBe("planned");
  });
});

describe("Person content under the install policy", () => {
  const sections = [{ id: "git_output" }, { id: "collaboration" }];

  it("removes hidden sections from the content catalog", () => {
    const policy = hide(["zone:person/section:git_output"]);

    expect(visiblePersonSections(sections, true, policy)).toEqual([
      { id: "collaboration" },
    ]);
  });

  it("toggles planned sections in the content catalog", () => {
    const policy = planned(["zone:person/section:git_output"]);

    expect(visiblePersonSections(sections, false, policy)).toEqual([
      { id: "collaboration" },
    ]);
    expect(visiblePersonSections(sections, true, policy)).toEqual(sections);
  });
});

describe("directions under the install policy", () => {
  it("drops a hidden direction", () => {
    const policy = hide(["zone:directions/dir:sales"]);

    expect(visibleDirections(true, policy).map((d) => d.id)).not.toContain(
      "sales"
    );
  });

  it("drops a hidden lens by its slug", () => {
    const policy = hide(["zone:directions/dir:dev/lens:git-output"]);

    expect(visibleLenses(dev, true, policy)).not.toContain("Git output");
  });

  it("drops a direction whose every lens is hidden", () => {
    const policy = hide(
      wiki.lenses.map((l) => `zone:directions/dir:wiki/lens:${lensSlug(l)}`)
    );

    expect(visibleDirections(true, policy).map((d) => d.id)).not.toContain(
      "wiki"
    );
  });

  it("toggles a planned lens with the show-planned choice", () => {
    const policy = planned(["zone:directions/dir:dev/lens:git-output"]);

    expect(visibleLenses(dev, true, policy)).toContain("Git output");
    expect(visibleLenses(dev, false, policy)).not.toContain("Git output");
  });

  it("toggles a whole planned direction with the show-planned choice", () => {
    const policy = planned(["zone:directions/dir:dev"]);

    expect(visibleDirections(true, policy).map((d) => d.id)).toContain("dev");
    expect(visibleDirections(false, policy).map((d) => d.id)).not.toContain(
      "dev"
    );
  });

  it("offers no Overview card for a direction whose Overview lens is hidden", () => {
    const policy = hide(["zone:directions/dir:dev/lens:overview"]);

    expect(visibleDirections(true, policy).map((d) => d.id)).toContain("dev");
    expect(overviewCardDirections(true, policy).map((d) => d.id)).not.toContain(
      "dev"
    );
  });

  it("toggles the Overview card of a planned Overview lens with the reader's choice", () => {
    const policy = planned(["zone:directions/dir:dev/lens:overview"]);

    expect(overviewCardDirections(true, policy).map((d) => d.id)).toContain(
      "dev"
    );
    expect(
      overviewCardDirections(false, policy).map((d) => d.id)
    ).not.toContain("dev");
  });
});

describe("what the install policy must NOT touch", () => {
  it("leaves the full item catalog for labels and telemetry", () => {
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
      overviewVisible: false,
    });

    expect(decision.kind).toBe("keep");
  });

  it("still pins Overview when the install shows it", () => {
    const decision = landingDecision({
      zone: null,
      ...resolved,
      canSeeOthers: true,
      overviewVisible: true,
    });

    expect(decision.kind).toBe("pin-overview");
  });
});

describe("metric gating across the surfaces a metric reaches", () => {
  // The shape an install takes when a metric family is declared but not yet
  // trusted: the family and three individual keys under the reader's toggle.
  const policy = planned([
    "metric:ai.*",
    "metric:tasks.resolution_time",
    "metric:tasks.pickup_time",
    "metric:tasks.on_time_delivery",
  ]);

  it("takes the participation card off the Overview with its own section", () => {
    const overview = OVERVIEW_ITEMS[DEFAULT_OVERVIEW_ITEM]!;

    const kinds = visibleSections(overview, false, policy).sections.map(
      (s) => s.kind
    );

    expect(kinds).not.toContain("participation");
    expect(kinds).toContain("headline");
  });

  it("leaves the headline with only the metrics the install still shows", () => {
    const overview = OVERVIEW_ITEMS[DEFAULT_OVERVIEW_ITEM]!;
    const headline = visibleSections(overview, false, policy).sections.find(
      (s) => s.kind === "headline"
    );

    expect(headline).toBeDefined();
    expect(headline && "metrics" in headline ? headline.metrics : []).not.toContain(
      "ai.cost"
    );
  });

  it("stops the attention scan from ranking a gated metric", () => {
    const overview = OVERVIEW_ITEMS["attention"]!;
    const attention = visibleSections(overview, false, policy).sections.find(
      (s) => s.kind === "attention"
    );
    const keys = attention && "metrics" in attention ? attention.metrics : [];

    expect(keys).not.toContain("tasks.resolution_time");
    expect(keys.length).toBeGreaterThan(0);
  });

  it("drops the section a coverage count would otherwise call a gap", () => {
    expect(visibleGroups(false, policy).map((g) => g.id)).not.toContain(
      "ai_adoption"
    );
  });

  it("gives the reader everything back with planned sections on", () => {
    const overview = OVERVIEW_ITEMS[DEFAULT_OVERVIEW_ITEM]!;

    expect(visibleSections(overview, true, policy)).toEqual(overview);
    expect(visibleGroups(true, policy)).toEqual(GROUPS);
  });
});
