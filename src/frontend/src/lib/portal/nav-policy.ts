import { runtimeConfig } from "@/lib/runtime-config";

/**
 * Per-installation navigation policy, read from `nav` in `/config.js`. Two
 * lists of typed paths name entries at any nesting level:
 *
 * - `nav.hide`    — gone entirely: not in any menu, not in the "Planned"
 *                   group, deaf to the reader's show-planned choice. The
 *                   operator's cut, not roadmap communication.
 * - `nav.planned` — this install marks the entry as still in development:
 *                   it behaves exactly like a `readiness`-marked entry in
 *                   code — demoted to the "Planned" group and toggled by the
 *                   reader's "Show planned sections" switch. An entry the
 *                   code already marks keeps its code tier.
 *
 * `hide` outranks `planned`: a path in both is simply gone. Path forms:
 *
 * - `zone:<id>`                              — a rail zone
 * - `zone:<id>/item:<id>`                    — a pane item of a theme /
 *                                              People / Manage zone
 * - `zone:directions/dir:<id>`               — a whole direction
 * - `zone:directions/dir:<id>/lens:<slug>`   — one lens (its URL slug)
 * - `zone:person/section:<id>`               — a Person-zone section
 *                                              (metric-group id)
 *
 * A path that matches nothing is ignored (malformed ones warn), so a config
 * outliving a rename degrades to a no-op rather than a crash.
 *
 * Presentation only, not authorization: a hand-typed URL may still render
 * the surface behind a hidden entry, and the server refuses on its own
 * regardless of what the menu draws (same stance as `adminOnly`).
 */
export interface NavPathSet {
  zones: ReadonlySet<string>;
  /** `<zoneId>/<itemId>` — item ids are unique only within their zone. */
  items: ReadonlySet<string>;
  directions: ReadonlySet<string>;
  /** `<directionId>/<lensSlug>` — lenses are named by their URL slug. */
  lenses: ReadonlySet<string>;
  /** Person-zone section ids — metric-group ids such as `git_output`. */
  personSections: ReadonlySet<string>;
}

export interface InstanceNavPolicy {
  hide: NavPathSet;
  planned: NavPathSet;
}

export const EMPTY_NAV_PATHS: NavPathSet = {
  zones: new Set(),
  items: new Set(),
  directions: new Set(),
  lenses: new Set(),
  personSections: new Set(),
};

export const EMPTY_NAV_POLICY: InstanceNavPolicy = {
  hide: EMPTY_NAV_PATHS,
  planned: EMPTY_NAV_PATHS,
};

type NavPath =
  | { kind: "zone"; zone: string }
  | { kind: "item"; zone: string; item: string }
  | { kind: "direction"; direction: string }
  | { kind: "lens"; direction: string; lens: string }
  | { kind: "section"; section: string };

const SEGMENT = /^(zone|item|dir|lens|section):([a-z0-9][a-z0-9_-]*)$/;

function parsePath(raw: string): NavPath | null {
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

/** Parse one path list; anything unreadable warns (naming the field) and is skipped. */
export function parseNavPaths(raw: unknown, field: string): NavPathSet {
  if (raw == null) return EMPTY_NAV_PATHS;
  if (!Array.isArray(raw)) {
    console.warn(`[nav.${field}] ignored: expected a list of paths, got`, raw);
    return EMPTY_NAV_PATHS;
  }

  const zones = new Set<string>();
  const items = new Set<string>();
  const directions = new Set<string>();
  const lenses = new Set<string>();
  const personSections = new Set<string>();
  for (const entry of raw) {
    const path = typeof entry === "string" ? parsePath(entry) : null;
    if (!path) {
      console.warn(`[nav.${field}] ignored invalid entry:`, entry);
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

/** Parse the raw `nav` config value into a full policy. */
export function parseNavPolicy(nav: unknown): InstanceNavPolicy {
  if (nav == null) return EMPTY_NAV_POLICY;
  if (typeof nav !== "object" || Array.isArray(nav)) {
    console.warn("[nav] ignored: expected an object, got", nav);
    return EMPTY_NAV_POLICY;
  }
  const { hide, planned } = nav as { hide?: unknown; planned?: unknown };
  return {
    hide: parseNavPaths(hide, "hide"),
    planned: parseNavPaths(planned, "planned"),
  };
}

let active: InstanceNavPolicy | undefined;

/** This install's policy — parsed from `/config.js` once per page load. */
export function navPolicy(): InstanceNavPolicy {
  active ??= parseNavPolicy(runtimeConfig().nav);
  return active;
}

export function zoneHidden(zoneId: string, policy = navPolicy()): boolean {
  return policy.hide.zones.has(zoneId);
}

export function zonePlanned(zoneId: string, policy = navPolicy()): boolean {
  return policy.planned.zones.has(zoneId);
}

export function itemHidden(
  zoneId: string,
  itemId: string,
  policy = navPolicy()
): boolean {
  return policy.hide.items.has(`${zoneId}/${itemId}`);
}

export function itemPlanned(
  zoneId: string,
  itemId: string,
  policy = navPolicy()
): boolean {
  return policy.planned.items.has(`${zoneId}/${itemId}`);
}

export function directionHidden(
  directionId: string,
  policy = navPolicy()
): boolean {
  return policy.hide.directions.has(directionId);
}

export function directionPlanned(
  directionId: string,
  policy = navPolicy()
): boolean {
  return policy.planned.directions.has(directionId);
}

export function lensHidden(
  directionId: string,
  lensSlug: string,
  policy = navPolicy()
): boolean {
  return policy.hide.lenses.has(`${directionId}/${lensSlug}`);
}

export function lensPlanned(
  directionId: string,
  lensSlug: string,
  policy = navPolicy()
): boolean {
  return policy.planned.lenses.has(`${directionId}/${lensSlug}`);
}

export function personSectionHidden(
  sectionId: string,
  policy = navPolicy()
): boolean {
  return policy.hide.personSections.has(sectionId);
}

export function personSectionPlanned(
  sectionId: string,
  policy = navPolicy()
): boolean {
  return policy.planned.personSections.has(sectionId);
}

/**
 * Whether the Person pane lists a section: hidden ones never, planned ones
 * only for a reader who opted into planned sections. The Person pane has no
 * demoted "Planned" group — a planned section renders in place, muted.
 */
export function personSectionVisible(
  sectionId: string,
  showPlanned: boolean,
  policy = navPolicy()
): boolean {
  if (personSectionHidden(sectionId, policy)) return false;
  return showPlanned || !personSectionPlanned(sectionId, policy);
}

export function visiblePersonSections<T extends { id: string }>(
  sections: readonly T[],
  showPlanned: boolean,
  policy = navPolicy()
): T[] {
  return sections.filter((section) =>
    personSectionVisible(section.id, showPlanned, policy)
  );
}
