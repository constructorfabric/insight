import { runtimeConfig } from "@/lib/runtime-config";

export interface NavPathSet {
  zones: ReadonlySet<string>;
  items: ReadonlySet<string>;
  directions: ReadonlySet<string>;
  lenses: ReadonlySet<string>;
  personSections: ReadonlySet<string>;
  /** Whole metric keys, e.g. `tasks.resolution_time`. */
  metrics: ReadonlySet<string>;
  /** Family prefixes from `metric:ai.*`, stored WITH the dot (`ai.`). */
  metricFamilies: ReadonlySet<string>;
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
  metrics: new Set(),
  metricFamilies: new Set(),
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
  | { kind: "section"; section: string }
  | { kind: "metric"; metric: string }
  | { kind: "metricFamily"; family: string };

const SEGMENT = /^(zone|item|dir|lens|section):([a-z0-9][a-z0-9_-]*)$/;

/**
 * A metric path names a key of the analytics catalog, not a menu node, so it
 * is a root form rather than a segment under `zone:` — and its value carries
 * the dotted family (`tasks.resolution_time`). The trailing `.*` form takes a
 * whole family, because "every AI metric" is a standing product statement and
 * a list of today's keys would silently stop covering tomorrow's.
 */
const METRIC = /^metric:([a-z0-9][a-z0-9_]*)\.([a-z0-9][a-z0-9_]*|\*)$/;

function parsePath(raw: string): NavPath | null {
  const metric = METRIC.exec(raw);
  if (metric) {
    const [, family, leaf] = metric;
    return leaf === "*"
      ? { kind: "metricFamily", family: `${family}.` }
      : { kind: "metric", metric: `${family}.${leaf}` };
  }

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

export function parseNavPaths(raw: unknown, field: string): NavPathSet {
  if (raw == null) return EMPTY_NAV_PATHS;
  if (!Array.isArray(raw)) {
    console.warn("[nav] ignored: expected a list of paths", { field, raw });
    return EMPTY_NAV_PATHS;
  }

  const zones = new Set<string>();
  const items = new Set<string>();
  const directions = new Set<string>();
  const lenses = new Set<string>();
  const personSections = new Set<string>();
  const metrics = new Set<string>();
  const metricFamilies = new Set<string>();
  for (const entry of raw) {
    const path = typeof entry === "string" ? parsePath(entry) : null;
    if (!path) {
      console.warn("[nav] ignored invalid entry", { field, entry });
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
      case "metric":
        metrics.add(path.metric);
        break;
      case "metricFamily":
        metricFamilies.add(path.family);
        break;
    }
  }
  return {
    zones,
    items,
    directions,
    lenses,
    personSections,
    metrics,
    metricFamilies,
  };
}

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

/* ── Metrics ─────────────────────────────────────────────────────────── */

function matches(set: NavPathSet, metricKey: string): boolean {
  if (set.metrics.has(metricKey)) return true;
  for (const family of set.metricFamilies) {
    if (metricKey.startsWith(family)) return true;
  }
  return false;
}

export function metricHidden(
  metricKey: string,
  policy = navPolicy()
): boolean {
  return matches(policy.hide, metricKey);
}

export function metricPlanned(
  metricKey: string,
  policy = navPolicy()
): boolean {
  return matches(policy.planned, metricKey);
}

/**
 * Whether a metric may appear on screen at all.
 *
 * Applied to the metric KEY rather than to each surface, so a metric this
 * install does not show cannot reach a tile, an attention row, a heatmap
 * column or a report column by a route nobody remembered to gate.
 */
export function metricVisible(
  metricKey: string,
  showPlanned: boolean,
  policy = navPolicy()
): boolean {
  if (metricHidden(metricKey, policy)) return false;
  return showPlanned || !metricPlanned(metricKey, policy);
}

export function visibleMetricKeys(
  keys: readonly string[],
  showPlanned: boolean,
  policy = navPolicy()
): string[] {
  return keys.filter((key) => metricVisible(key, showPlanned, policy));
}

/** Whether this install gates any metric at all — lets callers skip the walk. */
export function gatesAnyMetric(policy = navPolicy()): boolean {
  return (
    policy.hide.metrics.size > 0 ||
    policy.hide.metricFamilies.size > 0 ||
    policy.planned.metrics.size > 0 ||
    policy.planned.metricFamilies.size > 0
  );
}
