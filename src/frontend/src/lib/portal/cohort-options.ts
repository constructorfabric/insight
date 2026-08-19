import type { MetricDefinitionListResponse } from "@/api/metric-definitions-client";
import { availableSlices, type SliceDim } from "@/lib/insight/slices";
import type { SliceAttr } from "@/lib/insight/slices";

/**
 * Where the cohort choices come from — one place, two sources, in priority
 * order.
 *
 * The portal used to derive them by walking the viewer's roster and keeping
 * attributes that took more than one value. That can only ever work for a
 * viewer who can see other people: identity serves a viewer their own subtree,
 * so someone with no reports has a roster of one person, no attribute has a
 * second value, and no comparison can be offered at all — which is most
 * readers.
 *
 * The replacement is a catalog: the server says which attributes a comparison
 * may be built on, having decided it from governed policy rather than from
 * whoever happens to be visible (epic constructorfabric/insight#2028, design
 * `docs/domain/person-attributes/specs/DESIGN.md` §3.3). It rides on
 * `/v1/metric-definitions`, a response the client already reads.
 *
 * Until that lands the roster walk stays as the fallback, so nothing regresses
 * for a manager today; when the catalog appears the client follows it without
 * a second code path to keep in step.
 */
export type CohortSource = "catalog" | "roster" | "none";

export interface CohortOptions {
  dims: SliceDim[];
  /** Which of the two produced them — "none" when neither could. */
  source: CohortSource;
}

/**
 * The catalog entry this client understands.
 *
 * Deliberately narrow: an id to send back and a label to show. Anything else
 * the server publishes about an attribute — sensitivity, how often it is
 * filled in, which source it came from — is not this control's business.
 */
interface CatalogAttribute {
  id: string;
  label: string;
}

/**
 * Read the catalog out of a metric-definitions response, if it carries one.
 *
 * The field is optional on purpose: this client is expected to run against
 * installations on either side of the change, so its absence is a normal
 * state, not an error. Entries missing an id or a label are dropped rather
 * than rendered as blanks — a nameless option cannot be chosen meaningfully.
 *
 * NOTE: the exact field name is not pinned by the design yet. It is read in
 * this one function so that agreeing on it later is a one-line change here,
 * not a search through the portal.
 */
export function catalogAttributes(
  response: MetricDefinitionListResponse | undefined
): CatalogAttribute[] {
  const raw = (response as { comparison_attributes?: unknown } | undefined)
    ?.comparison_attributes;
  if (!Array.isArray(raw)) return [];
  return raw.flatMap((entry) => {
    if (typeof entry !== "object" || entry === null) return [];
    const { id, label } = entry as { id?: unknown; label?: unknown };
    if (typeof id !== "string" || !id) return [];
    if (typeof label !== "string" || !label) return [];
    return [{ id, label }];
  });
}

/**
 * The cohort dimensions to offer, and where they came from.
 *
 * `roster` is only consulted when the catalog is absent — not merged with it.
 * Two sources of the same list would let a locally-derived attribute appear
 * beside a governed one, and the reader has no way to tell which of the two
 * they picked.
 */
export function cohortOptions(
  catalog: readonly CatalogAttribute[],
  roster: Iterable<Record<string, SliceAttr>>
): CohortOptions {
  if (catalog.length > 0) {
    return {
      dims: catalog.map((a) => ({ key: a.id, label: a.label })),
      source: "catalog",
    };
  }
  const dims = availableSlices(roster);
  return { dims, source: dims.length > 0 ? "roster" : "none" };
}

/**
 * Why the control has nothing to offer — shown to the reader, so it has to say
 * what is true rather than name an internal state.
 */
export const NO_COHORT_REASON =
  "Comparisons are made within each person's organization unit. Other cohorts are not available here yet.";
