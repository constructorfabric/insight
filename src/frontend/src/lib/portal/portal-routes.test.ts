import { describe, expect, it } from "vitest";

import { GROUPS } from "@/lib/insight/groups";
import { DIRECTIONS, MANAGE_ITEMS, PEOPLE_ITEMS, ZONE_SECTIONS, lensSlug } from "@/lib/portal/nav-model";

import { locationOf, pathOf, type NavTarget } from "./portal-routes";

/** Where a link lands, read back the way the matched route reads it. */
function roundTrip(target: NavTarget) {
  const { to, params } = pathOf(target);
  const zone = target.zone;
  const fixed = to.startsWith("/manage/identities")
    ? "identities"
    : to.startsWith("/manage/ingestion")
      ? "ingestion"
      : undefined;
  return locationOf(zone, fixed, params);
}

const PERSON = "aaaaaaaa-1111-4111-8111-111111111111";

describe("every pane item has a path that reads back as that item", () => {
  it.each(ZONE_SECTIONS.overview!.flatMap((group) => group.items.map((item) => item.id)))(
    "Home › %s",
    (item) => {
      expect(roundTrip({ zone: "overview", item }).item).toBe(item);
    },
  );

  it.each(ZONE_SECTIONS.aicost!.flatMap((group) => group.items.map((item) => item.id)))(
    "AI & Cost › %s",
    (item) => {
      expect(roundTrip({ zone: "aicost", item }).item).toBe(item);
    },
  );

  it.each(ZONE_SECTIONS.reports!.flatMap((group) => group.items.map((item) => item.id)))(
    "Reports › %s",
    (item) => {
      expect(roundTrip({ zone: "reports", item }).item).toBe(item);
    },
  );

  it.each(MANAGE_ITEMS.map((item) => item.id))("Manage › %s", (item) => {
    expect(roundTrip({ zone: "manage", item }).item).toBe(item);
  });

  it.each(PEOPLE_ITEMS.map((item) => item.id))("a team › %s", (item) => {
    const location = roundTrip({ zone: "people", person: PERSON, item });
    expect(location).toMatchObject({ item, person: PERSON });
  });

  it.each(GROUPS.map((group) => group.id))("a person › %s", (section) => {
    const location = roundTrip({ zone: "person", person: PERSON, section });
    expect(location).toMatchObject({ item: section, person: PERSON });
  });

  it.each(DIRECTIONS.flatMap((direction) => direction.lenses.map((lens) => [direction.id, lens])))(
    "Explore › %s › %s",
    (dir, lens) => {
      expect(roundTrip({ zone: "directions", dir, lens })).toMatchObject({ dir, lens });
    },
  );
});

describe("paths use short names", () => {
  it.each([
    [{ zone: "overview" }, "/home"],
    [{ zone: "overview", item: "at-a-glance" }, "/home"],
    [{ zone: "overview", item: "by-direction" }, "/home/$view"],
    [{ zone: "aicost", item: "overview" }, "/explore/ai"],
    [{ zone: "manage", item: "identities" }, "/manage/identities"],
    [{ zone: "manage", item: "ingestion" }, "/manage/ingestion"],
    [{ zone: "people", person: PERSON, item: "roster" }, "/people/$person/team"],
    [{ zone: "custom" }, "/dashboards"],
  ] as const)("%j lives at %s", (target, to) => {
    expect(pathOf(target as NavTarget).to).toBe(to);
  });

  it("spells a lens by its slug and a section with dashes", () => {
    expect(pathOf({ zone: "directions", dir: "dev", lens: "Git output" }).params).toEqual({
      dir: "dev",
      lens: lensSlug("Git output"),
    });
    expect(pathOf({ zone: "person", person: PERSON, section: "git_output" }).params).toEqual({
      person: PERSON,
      section: "git-output",
    });
  });

  it("falls back to the zone's own page for an item it does not have", () => {
    expect(pathOf({ zone: "overview", item: "nonsense" })).toEqual({ to: "/home", params: {} });
    expect(pathOf({ zone: "manage", item: "nonsense" })).toEqual({ to: "/manage", params: {} });
  });
});

describe("reading a path back", () => {
  it("names no item for a slug the zone does not have", () => {
    expect(locationOf("overview", undefined, { view: "nonsense" }).item).toBeNull();
  });

  it("names no lens a direction does not have", () => {
    expect(locationOf("directions", undefined, { dir: "dev", lens: "nonsense" })).toMatchObject({
      dir: "dev",
      lens: "",
    });
  });
});
