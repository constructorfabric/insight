/**
 * The name a recorded path goes by in the product.
 *
 * Labels are read from the nav model the sidebar renders from, so a screen
 * renamed there is renamed in every usage report, including for rows already
 * recorded. A path this cannot name is returned as it stands: an honest path
 * beats an invented title.
 */

import { GROUPS } from "@/lib/insight/groups";
import { MANAGE_ITEMS, PEOPLE_ITEMS, zoneById, zoneItems } from "@/lib/portal/nav-model";

const SEPARATOR = " › ";

/** The two person views are different screens, and read as different rows. */
const PERSON_VIEWS: Record<string, string> = {
  personal: "Personal",
  team: "Team",
};

function itemLabel(zone: string, item: string): string {
  const items =
    zone === "manage" ? MANAGE_ITEMS : zone === "people" ? PEOPLE_ITEMS : zoneItems(zone);
  return items.find((candidate) => candidate.id === item)?.label ?? item;
}

function sectionLabel(section: string): string {
  return GROUPS.find((group) => group.id === section)?.title ?? section;
}

export function screenLabel(path: string): string {
  const [, first, second, third, fourth] = path.split("/");

  if (!first) return "Home";

  if (first === "portal") {
    if (!second) return "Portal";
    const zone = zoneById(second);
    if (!zone) return path;
    return third ? `${zone.label}${SEPARATOR}${itemLabel(second, third)}` : zone.label;
  }

  // `/ic/<person>/personal|team[/<section>]` — the person is already reduced to
  // `:id` before it is recorded.
  if (first === "ic" && second) {
    const view = PERSON_VIEWS[third ?? ""];
    if (!view) return path;
    const person = `Person${SEPARATOR}${view}`;
    return fourth ? `${person}${SEPARATOR}${sectionLabel(fourth)}` : person;
  }

  return path;
}
