import { runtimeConfig } from "@/lib/runtime-config";

/**
 * Per-installation navigation hiding: `nav.hide` in `/config.js` is a flat
 * list of typed paths naming the menu entries an operator wants off THIS
 * install's navigation, at any nesting level:
 *
 * - `zone:<id>`                              — a rail zone
 * - `zone:<id>/item:<id>`                    — a pane item of a theme /
 *                                              People / Manage zone
 * - `zone:directions/dir:<id>`               — a whole direction
 * - `zone:directions/dir:<id>/lens:<slug>`   — one lens (its URL slug)
 * - `zone:person/section:<id>`               — a Person-zone section
 *                                              (metric-group id)
 *
 * A hidden entry disappears entirely — it does not join the demoted
 * "Planned" group and ignores the reader's show-planned choice: this is the
 * operator's cut, not roadmap communication. A path that matches nothing is
 * ignored (malformed ones warn), so a config outliving a rename degrades to
 * a no-op rather than a crash.
 *
 * Presentation only, not authorization: a hand-typed URL may still render
 * the surface behind a hidden entry, and the server refuses on its own
 * regardless of what the menu draws (same stance as `adminOnly`).
 */
export interface NavHidePolicy {
  zones: ReadonlySet<string>;
  /** `<zoneId>/<itemId>` — item ids are unique only within their zone. */
  items: ReadonlySet<string>;
  directions: ReadonlySet<string>;
  /** `<directionId>/<lensSlug>` — lenses are named by their URL slug. */
  lenses: ReadonlySet<string>;
  /** Person-zone section ids — metric-group ids such as `git_output`. */
  personSections: ReadonlySet<string>;
}

export const EMPTY_NAV_HIDE: NavHidePolicy = {
  zones: new Set(),
  items: new Set(),
  directions: new Set(),
  lenses: new Set(),
  personSections: new Set(),
};

type HidePath =
  | { kind: "zone"; zone: string }
  | { kind: "item"; zone: string; item: string }
  | { kind: "direction"; direction: string }
  | { kind: "lens"; direction: string; lens: string }
  | { kind: "section"; section: string };

const SEGMENT = /^(zone|item|dir|lens|section):([a-z0-9][a-z0-9_-]*)$/;

function parsePath(raw: string): HidePath | null {
  const parts: [kind: string, value: string][] = [];
  for (const segment of raw.split("/")) {
    const match = SEGMENT.exec(segment);
    if (!match) return null;
    parts.push([match[1]!, match[2]!]);
  }

  const [first, second, third] = parts;
  if (!first || first[0] !== "zone") return null;
  const zone = first[1];
  if (parts.length === 1) return { kind: "zone", zone };

  if (zone === "directions" && second![0] === "dir") {
    if (parts.length === 2) return { kind: "direction", direction: second![1] };
    if (parts.length === 3 && third![0] === "lens")
      return { kind: "lens", direction: second![1], lens: third![1] };
    return null;
  }
  if (zone === "person" && parts.length === 2 && second![0] === "section")
    return { kind: "section", section: second![1] };
  if (parts.length === 2 && second![0] === "item")
    return { kind: "item", zone, item: second![1] };
  return null;
}

/** Parse the raw `nav.hide` value; anything unreadable warns and is skipped. */
export function parseNavHide(raw: unknown): NavHidePolicy {
  if (raw == null) return EMPTY_NAV_HIDE;
  if (!Array.isArray(raw)) {
    console.warn("[nav.hide] ignored: expected a list of paths, got", raw);
    return EMPTY_NAV_HIDE;
  }

  const zones = new Set<string>();
  const items = new Set<string>();
  const directions = new Set<string>();
  const lenses = new Set<string>();
  const personSections = new Set<string>();
  for (const entry of raw) {
    const path = typeof entry === "string" ? parsePath(entry) : null;
    if (!path) {
      console.warn("[nav.hide] ignored invalid entry:", entry);
      continue;
    }
    switch (path.kind) {
      case "zone":
        zones.add(path.zone);
        break;
      case "item":
        items.add(`${path.zone}/${path.item}`);
        break;
      case "direction":
        directions.add(path.direction);
        break;
      case "lens":
        lenses.add(`${path.direction}/${path.lens}`);
        break;
      case "section":
        personSections.add(path.section);
        break;
    }
  }
  return { zones, items, directions, lenses, personSections };
}

let active: NavHidePolicy | undefined;

/** This install's policy — parsed from `/config.js` once per page load. */
export function navHidePolicy(): NavHidePolicy {
  active ??= parseNavHide(runtimeConfig().nav?.hide);
  return active;
}

export function zoneHidden(zoneId: string, policy = navHidePolicy()): boolean {
  return policy.zones.has(zoneId);
}

export function itemHidden(
  zoneId: string,
  itemId: string,
  policy = navHidePolicy(),
): boolean {
  return policy.items.has(`${zoneId}/${itemId}`);
}

export function directionHidden(
  directionId: string,
  policy = navHidePolicy(),
): boolean {
  return policy.directions.has(directionId);
}

export function lensHidden(
  directionId: string,
  lensSlug: string,
  policy = navHidePolicy(),
): boolean {
  return policy.lenses.has(`${directionId}/${lensSlug}`);
}

export function personSectionHidden(
  sectionId: string,
  policy = navHidePolicy(),
): boolean {
  return policy.personSections.has(sectionId);
}
