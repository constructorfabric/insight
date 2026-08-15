import { useNavigate, useSearch } from "@tanstack/react-router";
import { useCallback } from "react";

import { validateDateRange } from "@/api/period-to-date-range";
import type { PeriodValue } from "@/types/insight";

/**
 * The portal's navigation state, carried in the URL.
 *
 * Everything that answers "what am I looking at" lives here rather than in a
 * store: a reload restores the same screen, a link reproduces it for someone
 * else, and Back means what it says. Only true preferences — is the portal on,
 * are planned sections shown — stay in localStorage, because they describe the
 * reader, not the view.
 *
 * All fields are optional and the defaults are computed, not stored: an absent
 * `zone` means "the landing zone for this viewer", an absent `scope` means "the
 * viewer's own subtree". That keeps a shared link honest — it pins what the
 * sender actually chose and lets everything else resolve for the recipient.
 */
export interface PortalSearch {
  /** Theme zone (overview / directions / aicost / manage). Person and People
   *  come from the route instead, so they are never in here. */
  zone?: string;
  /** Selected item within a zone (an Overview view, a Manage surface). */
  item?: string;
  /** Selected account inside the Identities surface (an opaque account key). */
  acct?: string;
  /** Free-text narrowing of the Identities queue. */
  filter?: string;
  /** Which Identities mode is open (the review queue, a person). */
  mode?: string;
  /** Person under inspection in the Identities people mode. */
  person?: string;
  /** Expanded direction + its active lens, within the Directions zone. */
  dir?: string;
  lens?: string;
  /** Org-scope root: a manager's person id. Absent = the viewer's own subtree. */
  scope?: string;
  /** Narrow the scope to direct reports only. */
  direct?: boolean;
  /** Person-attribute that groups rosters and defines peer cohorts. */
  slice?: string;
  /** Period preset. A custom range rides in `from`/`to` beside it. */
  period?: PeriodValue;
  from?: string;
  to?: string;
}

const PERIODS = new Set<string>(["week", "month", "quarter", "year"]);
const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;

function str(v: unknown): string | undefined {
  return typeof v === "string" && v !== "" ? v : undefined;
}

/**
 * Validate rather than trust: search params are user-editable text, and a
 * malformed one must not reach a metric request or a date range. Anything
 * unrecognised is dropped, which degrades to the computed default instead of
 * rendering an error for a link someone mistyped.
 */
export function validatePortalSearch(raw: Record<string, unknown>): PortalSearch {
  const period = str(raw.period);
  const from = str(raw.from);
  const to = str(raw.to);
  // Format alone is not enough: `?from=2026-07-30&to=2026-01-01` is
  // well-formed, inverted, and used to reach `assertDateRange`, which throws
  // into the error boundary — breaking this validator's own promise to degrade
  // rather than error. Run the real check (ordering AND maximum span) and drop
  // the pair when it fails, falling back to the period preset.
  const custom =
    from &&
    to &&
    ISO_DATE.test(from) &&
    ISO_DATE.test(to) &&
    validateDateRange({ from, to }).valid;
  return {
    zone: str(raw.zone),
    item: str(raw.item),
    acct: str(raw.acct),
    filter: str(raw.filter),
    mode: str(raw.mode),
    person: str(raw.person)?.toLowerCase(),
    dir: str(raw.dir),
    lens: str(raw.lens),
    // Lower-cased to match `normalizePersonId`: the same id reaches us from a
    // link, an identity record or a hand-edited URL, and the resolver compares
    // it as a string. An id outside the viewer's subtree (or a pre-cutover
    // email) resolves to nothing and the scope falls back to the viewer.
    scope: str(raw.scope)?.toLowerCase(),
    // Omit rather than serialise `false`: a default has no business in a URL
    // people read and share.
    ...(raw.direct === true || raw.direct === "true" || raw.direct === 1
      ? { direct: true as const }
      : {}),
    slice: str(raw.slice),
    period: period && PERIODS.has(period) ? (period as PeriodValue) : undefined,
    ...(custom ? { from, to } : {}),
  };
}

/** Keys the portal owns — used to carry state across a route change. */
export const PORTAL_SEARCH_KEYS = [
  "zone",
  "item",
  "acct",
  "filter",
  "mode",
  "person",
  "dir",
  "lens",
  "scope",
  "direct",
  "slice",
  "period",
  "from",
  "to",
] satisfies Array<keyof PortalSearch>;

/** The validated portal params for the current route. */
export function usePortalSearch(): PortalSearch {
  // `strict: false` so the same hook serves both portal route families
  // (/portal and /ic/$person/*) without a per-route generic.
  return useSearch({ strict: false }) as PortalSearch;
}

/**
 * Patch the portal's search params. `undefined` in the patch CLEARS a key —
 * an explicit erase, distinct from "leave it alone" (omit it), which matters
 * for zone changes that must drop a now-meaningless item.
 */
export type PortalSearchPatch =
  | Partial<PortalSearch>
  | ((prev: PortalSearch) => Partial<PortalSearch>);

/**
 * Merge a patch into the current search, clearing a key by setting it to
 * `undefined` rather than removing it.
 *
 * The distinction is load-bearing. `retainSearchParams` restores any listed key
 * that is ABSENT from the result — its test is `key in copy` — so a deleted key
 * comes straight back with its old value. Present-but-undefined passes through
 * untouched, and both the serialiser and `validatePortalSearch` omit it.
 */
export function applySearchPatch(
  prev: Record<string, unknown>,
  patch: Partial<PortalSearch>,
): Record<string, unknown> {
  const next = { ...prev };
  for (const [key, value] of Object.entries(patch)) {
    next[key] = value === "" || value === false ? undefined : value;
  }
  return next;
}

/**
 * `replace` exists because not every write is a navigation the reader made.
 * An effect that pins the landing zone or syncs the scope from the route is
 * CORRECTING the URL, not moving through the app — pushing those makes Back
 * step into a half-built address (bare `/portal`, or a team URL with no scope).
 */
export function useSetPortalSearch(): (
  patch: PortalSearchPatch,
  opts?: { replace?: boolean },
) => void {
  const navigate = useNavigate();
  // Stable across renders — `navigate` is, and the patch may be a function of
  // the previous search, so nothing else needs to be captured. Callers put this
  // in effect dependency lists, where a fresh identity per render would loop.
  return useCallback(
    (patch, opts) => {
      void navigate({
        to: ".",
        search: (prev: Record<string, unknown>) => {
          const resolved =
            typeof patch === "function" ? patch(prev as PortalSearch) : patch;
          return applySearchPatch(prev, resolved);
        },
        replace: opts?.replace ?? false,
      });
    },
    [navigate],
  );
}
