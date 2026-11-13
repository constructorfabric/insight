import { useNavigate, useSearch } from "@tanstack/react-router";
import { useCallback } from "react";

import type { OwnedFilter } from "@/api/custom-client";
import { validateDateRange } from "@/api/period-to-date-range";
import { isRangeToken } from "@/lib/custom/time-range";
import { formatSources, parseSources } from "@/lib/identities/sources";
import type { PeriodValue } from "@/types/insight";

/**
 * The portal's view state, carried in the query string.
 *
 * The path says where the reader is; the query says how that page is set up.
 * Each route declares the keys it reads, so one page's state never rides along
 * to another. The settings that frame every page — scope, cohort, period — are
 * declared once on the shell and carried across every navigation.
 *
 * All fields are optional and defaults are computed, not stored: an absent
 * `scope` means the viewer's own subtree. A shared link pins only what the
 * sender chose.
 */
export interface GlobalSearch {
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

export interface ExploreSearch {
  /**
   * One repository under inspection, by its dimension VALUE (`<source>:<owner>/<repo>`).
   * The value and not the label, because two repositories can share a display
   * name and a link has to reproduce the one that was opened.
   */
  repo?: string;
}

export interface IdentitiesSearch {
  /** Selected account (an opaque account key). */
  acct?: string;
  src?: string;
  /** Person under inspection in the person mode. */
  person?: string;
  /** What was typed into the person roster, kept so leaving and coming back finds the same list. */
  find?: string;
}

export interface MetricsViewSearch {
  metrics?: string;
  compare?: string;
}

export interface DashboardListSearch {
  folder?: string;
  tag?: string[];
  /** Narrows the list to dashboards the reader owns, or those shared with them. */
  owned?: OwnedFilter;
}

export interface DashboardSearch {
  /**
   * The window a dashboard is read over: a preset like `P30D` or an ISO
   * interval. Separate from `period`, which every other page shares.
   */
  range?: string;
  /** The widget drawn over the whole window, by its id. */
  widget?: string;
}

/** Everything any portal route may carry; a page reads only its own keys. */
export type PortalSearch = GlobalSearch &
  ExploreSearch &
  IdentitiesSearch &
  MetricsViewSearch &
  DashboardListSearch &
  DashboardSearch;

const PERIODS = new Set<string>(["week", "month", "quarter", "year"]);
const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;
const METRIC_KEY = /^(none|[a-z][a-z0-9_]*\.[a-z0-9_.]+)$/;
const PERSON_ID = /^[0-9a-z-]+$/;

/** INVARIANT: mirror the identity-resolution service's `source` limits. */
const MAX_SOURCES = 20;
const MAX_SOURCE_CHARS = 30;

function str(v: unknown): string | undefined {
  return typeof v === "string" && v !== "" ? v : undefined;
}

function scalar(v: unknown): string | undefined {
  return typeof v === "number" || typeof v === "boolean" ? String(v) : str(v);
}

function list(v: unknown, part: RegExp): string | undefined {
  const parts = (str(v) ?? "").split(",").filter((value) => part.test(value));
  return parts.length > 0 ? parts.join(",") : undefined;
}

function strs(v: unknown): string[] | undefined {
  const values = (Array.isArray(v) ? v : [v])
    .map(scalar)
    .filter((value) => value !== undefined);

  return values.length > 0 ? values : undefined;
}

/*
 * Validate rather than trust: search params are user-editable text, and a
 * malformed one must not reach a metric request or a date range. Anything
 * unrecognised is dropped, which degrades to the computed default instead of
 * rendering an error for a link someone mistyped.
 *
 * INVARIANT: every validator returns each of its keys, `undefined` when
 * dropped — the router spreads the result over the raw query, so an omitted
 * key leaks through unvalidated.
 */

export function validateGlobalSearch(raw: Record<string, unknown>): GlobalSearch {
  const period = str(raw.period);
  const from = str(raw.from);
  const to = str(raw.to);
  // Format alone is not enough: an inverted or over-long pair is well-formed
  // and used to throw into the error boundary. Drop it and fall back to the preset.
  const custom =
    from && to && ISO_DATE.test(from) && ISO_DATE.test(to) && validateDateRange({ from, to }).valid;
  return {
    // Lower-cased to match `normalizePersonId`: an id outside the viewer's
    // subtree resolves to nothing and the scope falls back to the viewer.
    scope: str(raw.scope)?.toLowerCase(),
    // `undefined` rather than `false`: a default has no business in a URL.
    direct: raw.direct === true || raw.direct === "true" || raw.direct === 1 ? true : undefined,
    slice: str(raw.slice),
    period: period && PERIODS.has(period) ? (period as PeriodValue) : undefined,
    from: custom ? from : undefined,
    to: custom ? to : undefined,
  };
}

/** The keys carried across every navigation. */
export const GLOBAL_SEARCH_KEYS = [
  "scope",
  "direct",
  "slice",
  "period",
  "from",
  "to",
] satisfies Array<keyof GlobalSearch>;

export function validateExploreSearch(raw: Record<string, unknown>): ExploreSearch {
  return { repo: str(raw.repo) };
}

export function validateIdentitiesSearch(raw: Record<string, unknown>): IdentitiesSearch {
  return {
    acct: str(raw.acct),
    src: formatSources(
      parseSources(str(raw.src))
        .filter((source) => source.length <= MAX_SOURCE_CHARS)
        .slice(0, MAX_SOURCES),
    ),
    person: str(raw.person)?.toLowerCase(),
    find: str(raw.find),
  };
}

export function validateMetricsViewSearch(raw: Record<string, unknown>): MetricsViewSearch {
  return {
    metrics: list(raw.metrics, METRIC_KEY),
    compare: list(str(raw.compare)?.toLowerCase(), PERSON_ID),
  };
}

export function validateDashboardListSearch(raw: Record<string, unknown>): DashboardListSearch {
  return {
    folder: str(raw.folder),
    tag: strs(raw.tag),
    owned: raw.owned === "me" || raw.owned === "shared" ? raw.owned : undefined,
  };
}

export function validateDashboardSearch(raw: Record<string, unknown>): DashboardSearch {
  return {
    // The same check the run endpoint makes, so a hand-edited token degrades
    // to the board's default.
    range: isRangeToken(str(raw.range) ?? "") ? str(raw.range) : undefined,
    widget: str(raw.widget),
  };
}

/** The validated params of the matched routes. */
export function usePortalSearch(): PortalSearch {
  // `strict: false` merges every matched route's validated search, so one hook
  // serves every page.
  return useSearch({ strict: false }) as PortalSearch;
}

/**
 * Patch the current page's search params. `undefined` in the patch CLEARS a
 * key — an explicit erase, distinct from "leave it alone" (omit it).
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
 * untouched, and both the serialiser and the validators omit it.
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
 * An effect that syncs the scope from the route is CORRECTING the URL, not
 * moving through the app — pushing it makes Back step into a half-built address.
 */
export function useSetPortalSearch(): (
  patch: PortalSearchPatch,
  opts?: { replace?: boolean },
) => void {
  const navigate = useNavigate();
  // Stable across renders — callers put this in effect dependency lists, where
  // a fresh identity per render would loop.
  return useCallback(
    (patch, opts) => {
      void navigate({
        to: ".",
        search: (prev: Record<string, unknown>) => {
          const resolved = typeof patch === "function" ? patch(prev as PortalSearch) : patch;
          return applySearchPatch(prev, resolved);
        },
        replace: opts?.replace ?? false,
        // The page stays; only its setup changes, so the reader keeps their place.
        resetScroll: false,
      });
    },
    [navigate],
  );
}
