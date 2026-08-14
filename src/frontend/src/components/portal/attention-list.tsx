import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { AlertTriangle, ArrowDownRight, ChevronRight, TrendingDown } from "lucide-react";

import { Card, CardContent } from "@/components/ui/card";
import type { AttentionFlag, FlagKind } from "@/lib/insight/attention-flags";
import {
  usePortalNavActions,
} from "@/lib/portal/portal-nav";
import { cn } from "@/lib/utils";

const FLAG_ICON: Record<FlagKind, typeof AlertTriangle> = {
  outlier: ArrowDownRight,
  decline: TrendingDown,
  collapse: AlertTriangle,
};

/** Rows one subject may occupy before the rest wait on their own page. */
const MAX_ROWS_PER_SUBJECT = 2;

/**
 * Keep every subject's strongest findings and no more.
 *
 * A row is one finding, so a subject who trips five metrics takes five rows —
 * and because the list is ranked by severity, the subject in the most trouble
 * takes the most of it. The visible slice then fills with one person while
 * others wait behind "+N more", which is precisely backwards: the list hides
 * people better the worse things are.
 *
 * Capping keeps the shape — one row, one readable claim — and bounds what any
 * one subject can crowd out. What is dropped is never lost: every row opens
 * that subject's own page, where all of their findings are.
 *
 * Input order is preserved, so the strongest findings survive the cap.
 */
function capPerSubject(flags: AttentionFlag[]): AttentionFlag[] {
  const taken = new Map<string, number>();
  return flags.filter((f) => {
    const n = taken.get(f.personId) ?? 0;
    if (n >= MAX_ROWS_PER_SUBJECT) return false;
    taken.set(f.personId, n + 1);
    return true;
  });
}

/**
 * Shared "needs attention" panel: a rule-based summary line (placeholder for a
 * future AI insight) plus the ranked flag rows, each linking into that person.
 * Used by the team-state roster and the org overview.
 */
export function AttentionList({
  flags,
  summary,
  max = 12,
}: {
  flags: AttentionFlag[];
  summary: string;
  max?: number;
}) {
  const { setZone } = usePortalNavActions();
  const [expanded, setExpanded] = useState(false);
  const ranked = capPerSubject(flags);
  const shown = expanded ? ranked : ranked.slice(0, max);
  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        Needs attention
      </p>

      <Card>
        <CardContent className="p-3 text-sm">{summary}</CardContent>
      </Card>

      {shown.length > 0 ? (
        <div className="flex flex-col gap-1.5">
          {shown.map((f) => {
            const Icon = FLAG_ICON[f.kind];
            return (
              <Link
                key={`${f.personId}-${f.metricKey}`}
                to="/ic/$person/personal"
                params={{ person: f.personId }}
                // A pinned theme zone (Overview, Manage) wins over the route in
                // `useActiveZone` — clear it so the navigation actually lands
                // on the Person zone (same pattern as the rail).
                onClick={() => setZone(null)}
                className="flex items-center gap-3 rounded-lg border bg-card px-3 py-2 text-sm transition-colors hover:bg-accent"
              >
                <Icon
                  className={cn(
                    "size-4 shrink-0",
                    f.kind === "collapse" ? "text-destructive" : "text-warning",
                  )}
                />
                <span className="w-40 shrink-0 truncate font-medium">{f.name}</span>
                <span className="w-32 shrink-0 truncate text-muted-foreground">
                  {f.metricLabel}
                </span>
                <span className="shrink-0 font-medium tabular-nums">{f.valueText}</span>
                <span className="truncate text-xs text-muted-foreground">{f.reason}</span>
                {/* Standing affordance — the row opens that person's page, and
                    a border alone does not say so. */}
                <ChevronRight
                  className="ml-auto size-3.5 shrink-0 text-muted-foreground"
                  aria-hidden
                />
              </Link>
            );
          })}
          {ranked.length > max ? (
            <button
              type="button"
              onClick={() => setExpanded((v) => !v)}
              className="self-start px-3 pt-1 text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
            >
              {expanded ? "Show less" : `+${ranked.length - max} more`}
            </button>
          ) : null}
        </div>
      ) : (
        <div className="rounded-lg border bg-card p-4 text-sm text-muted-foreground">
          No outliers, declines, or collapses in this period — steady.
        </div>
      )}
    </section>
  );
}
