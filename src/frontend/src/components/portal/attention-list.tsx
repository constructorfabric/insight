import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { AlertTriangle, ArrowDownRight, Sparkles, TrendingDown } from "lucide-react";

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

/**
 * Shared "needs attention" panel: a rule-based summary line (placeholder for a
 * future AI insight) plus the ranked flag rows, each linking into that person.
 * Used by the team-state roster and the org overview.
 */
export function AttentionList({
  flags,
  summary,
  peopleLabel,
  max = 12,
}: {
  flags: AttentionFlag[];
  summary: string;
  /** e.g. "3 of 16 people" — shown top-right; omit to hide. */
  peopleLabel?: string;
  max?: number;
}) {
  const { setZone } = usePortalNavActions();
  const [expanded, setExpanded] = useState(false);
  const shown = expanded ? flags : flags.slice(0, max);
  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
          Needs attention
        </p>
        {peopleLabel ? (
          <span className="text-xs text-muted-foreground">{peopleLabel}</span>
        ) : null}
      </div>

      <Card>
        <CardContent className="flex items-start gap-2 p-3 text-sm">
          <Sparkles className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
          <div>
            <span className="text-foreground">{summary}</span>
            <span className="ml-1 text-xs text-muted-foreground">· rule-based</span>
          </div>
        </CardContent>
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
              </Link>
            );
          })}
          {flags.length > max ? (
            <button
              type="button"
              onClick={() => setExpanded((v) => !v)}
              className="self-start px-3 pt-1 text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
            >
              {expanded ? "Show less" : `+${flags.length - max} more`}
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
