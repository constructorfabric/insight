import type { IdentityPerson } from "@/types/insight";

/**
 * Slices — grouping a roster and defining peer cohorts by a person attribute.
 * The machinery is **attribute-agnostic**: it works over a generic
 * `{ key → {label, value} }` map, so new identity attributes become sliceable
 * with no change here or downstream. The ONE place attribute names live is
 * `ATTR_FIELDS` below — the interim adapter from the fixed `IdentityPerson`
 * shape. Once identity exposes attributes generically (see constructorfabric/
 * insight#1881), `personAttributes` reads that list and `ATTR_FIELDS` disappears.
 *
 * There is no slice catalog in the analytics API yet, so `availableSlices()`
 * discovers dimensions from the data itself and gates them by presence and
 * cardinality — junk attributes (unique-per-person like email/employee_id, or
 * single-valued) never show; good ones (division, seniority, location, …) show
 * automatically. `PLANNED_SLICES` are declared-but-unfed dims (functional team).
 */

export interface SliceAttr {
  key: string;
  label: string;
  value: string;
}

/** A sliceable dimension the roster actually supports (see `availableSlices`). */
export interface SliceDim {
  key: string;
  label: string;
  /** No data path yet — render an honest ComingSoon. */
  planned?: boolean;
}

/** A slice is usable only if it splits people without being near-unique. */
const MIN_DISTINCT = 2;
const MAX_DISTINCT = 60;
/** Above this distinct/people ratio the attribute is effectively an id, not a slice. */
const MAX_UNIQUE_RATIO = 0.9;

/**
 * Interim identity→attributes adapter — the only code that names attributes.
 * Replace its body with the profile's generic `attributes[]` when #1881 lands.
 */
const ATTR_FIELDS: readonly {
  key: string;
  label: string;
  get: (p: IdentityPerson) => string | null | undefined;
}[] = [
  { key: "division", label: "Division", get: (p) => p.division },
  { key: "department", label: "Department", get: (p) => p.department },
  { key: "title", label: "Title", get: (p) => p.job_title },
  // Keyed by EMAIL, not display name: two managers sharing a name must not
  // merge into one cohort. The email doubles as the visible unit label — less
  // pretty, but unambiguous; a display-label lookup is a follow-up.
  { key: "manager", label: "Manager", get: (p) => p.supervisor_email },
];

/** A person's sliceable attributes as a generic key→attr map (empty values dropped). */
export function personAttributes(p: IdentityPerson): Record<string, SliceAttr> {
  const out: Record<string, SliceAttr> = {};
  for (const f of ATTR_FIELDS) {
    const value = (f.get(p) ?? "").trim();
    if (value) out[f.key] = { key: f.key, label: f.label, value };
  }
  return out;
}

/**
 * Dimensions declared in the product but not backed by ingested data yet.
 * Empty today — "Functional team" was removed as speculative (no source is
 * even planned near-term); add entries here only when a real dimension has a
 * committed data path, and the by-unit section will render an honest
 * ComingSoon note for it until the data lands.
 */
export const PLANNED_SLICES: readonly SliceDim[] = [];

/**
 * Discover usable slice dimensions from a roster of attribute maps: present,
 * multi-valued, not near-unique, not absurdly high-cardinality. Order follows
 * first appearance so the adapter's order is preserved.
 */
export function availableSlices(
  rosters: Iterable<Record<string, SliceAttr>>,
): SliceDim[] {
  const label = new Map<string, string>();
  const order: string[] = [];
  const distinct = new Map<string, Set<string>>();
  const count = new Map<string, number>();

  for (const attrs of rosters) {
    for (const key of Object.keys(attrs)) {
      const a = attrs[key]!;
      if (!label.has(key)) {
        label.set(key, a.label);
        order.push(key);
        distinct.set(key, new Set());
      }
      distinct.get(key)!.add(a.value);
      count.set(key, (count.get(key) ?? 0) + 1);
    }
  }

  return order
    .filter((key) => {
      const d = distinct.get(key)!.size;
      const n = count.get(key) ?? 0;
      return d >= MIN_DISTINCT && d <= MAX_DISTINCT && d / n < MAX_UNIQUE_RATIO;
    })
    .map((key) => ({ key, label: label.get(key)! }));
}

/** Walk an org tree into `entityId → attribute map` (entity id via `keyOf`). */
export function collectRosterAttrs(
  root: IdentityPerson | null,
  keyOf: (personId: string) => string,
): Map<string, Record<string, SliceAttr>> {
  const m = new Map<string, Record<string, SliceAttr>>();
  const walk = (n: IdentityPerson) => {
    // Keyed by person id (the metric entity id), not email: a person with no
    // email still belongs to a cohort.
    if (n.person_id) m.set(keyOf(n.person_id), personAttributes(n));
    n.subordinates.forEach(walk);
  };
  if (root) walk(root);
  return m;
}

/**
 * Cohort key for a member under the active slice: the attribute's value, or
 * "all" when no slice is active (whole roster = one cohort), or null when the
 * member has no value for that attribute (excluded from cohort stats).
 */
export function cohortKey(
  attrs: Record<string, SliceAttr> | undefined,
  slice: string,
): string | null {
  if (!slice) return "all";
  return attrs?.[slice]?.value ?? null;
}
