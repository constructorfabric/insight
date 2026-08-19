import { useCallback, useMemo } from "react";
import { recordUsageEvent } from "@/telemetry";

import {
  resolveDateRange,
  validateDateRange,
  type DateRange,
} from "@/api/period-to-date-range";
import { usePortalSearch, useSetPortalSearch } from "@/lib/portal/portal-search";
import { usePeriodPreference, writePeriodPreference } from "@/hooks/use-period";
import type { CustomRange, PeriodValue } from "@/types/insight";

/**
 * The period, read from the URL.
 *
 * A link has to reproduce the whole picture, and the period is half of any
 * number on screen — "63 commits" means nothing without the window it covers.
 * So `?period=` (plus `?from=`/`?to=` for a custom range) is the truth, and
 * localStorage keeps only the DEFAULT for a URL that names none: a reader's
 * habitual window survives between sessions without leaking into a link they
 * send someone else.
 */
export function usePortalPeriod(): {
  period: PeriodValue;
  customRange: CustomRange | null;
  dateRange: DateRange;
  setPeriod: (period: PeriodValue) => void;
  setCustomRange: (range: CustomRange | null) => void;
} {
  const search = usePortalSearch();
  const setSearch = useSetPortalSearch();
  const preference = usePeriodPreference();

  const period = search.period ?? preference;
  // Memoised: these land in the dependency arrays of every metric query, so a
  // fresh object per render re-keys the queries on an unrelated re-render.
  const customRange = useMemo(
    () => (search.from && search.to ? { from: search.from, to: search.to } : null),
    [search.from, search.to],
  );
  const dateRange = useMemo(
    () => resolveDateRange(period, customRange),
    [period, customRange],
  );

  const setPeriod = useCallback(
    (next: PeriodValue) => {
      recordUsageEvent("period", next);
      // Remember the choice as the default for a link that names no period,
      // then put it in the URL where it belongs.
      writePeriodPreference(next);
      setSearch({ period: next, from: undefined, to: undefined });
    },
    [setSearch],
  );
  const setCustomRange = useCallback(
    (range: CustomRange | null) => {
      // Drop an invalid range instead of throwing: this runs in an event
      // handler, where no error boundary is watching, and the same policy the
      // URL validator follows (degrade to the preset) has to hold here too.
      // The picker does its own validation and keeps its own message.
      if (range && !validateDateRange(range).valid) return;
      recordUsageEvent("period", range ? "custom" : "preset");
      setSearch({ from: range?.from, to: range?.to });
    },
    [setSearch],
  );

  return useMemo(
    () => ({ period, customRange, dateRange, setPeriod, setCustomRange }),
    [period, customRange, dateRange, setPeriod, setCustomRange],
  );
}
