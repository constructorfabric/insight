import { useState } from "react";
import { Link } from "@tanstack/react-router";
import {
  AlertTriangle,
  ArrowDownRight,
  ArrowUpRight,
  ChevronDown,
  ChevronRight,
  TrendingDown,
  TrendingUp,
} from "lucide-react";

import { Card, CardContent } from "@/components/ui/card";
import {
  groupFlagsByTheme,
  type AttentionFlag,
} from "@/lib/insight/attention-flags";
import { usePortalNavActions } from "@/lib/portal/portal-nav";
import { cn } from "@/lib/utils";

const FLAG_ICON = {
  collapse: AlertTriangle,
  "decline-down": TrendingDown,
  "decline-up": TrendingUp,
  "outlier-down": ArrowDownRight,
  "outlier-up": ArrowUpRight,
} as const;

function iconKey(flag: AttentionFlag): keyof typeof FLAG_ICON {
  return flag.kind === "collapse" ? "collapse" : `${flag.kind}-${flag.moved}`;
}

function people(n: number): string {
  return `${n} ${n === 1 ? "person" : "people"}`;
}

function PersonRow({ flag }: { flag: AttentionFlag }) {
  const { setZone } = usePortalNavActions();
  const Icon = FLAG_ICON[iconKey(flag)];
  return (
    <Link
      to="/ic/$person/personal"
      params={{ person: flag.personId }}
      // A pinned theme zone (Overview, Manage) wins over the route in
      // `useActiveZone` — clear it so the navigation lands on the Person zone.
      onClick={() => setZone(null)}
      className="flex items-center gap-3 rounded-lg px-3 py-1.5 text-sm transition-colors hover:bg-accent"
    >
      <Icon
        className={cn(
          "size-4 shrink-0",
          flag.kind === "collapse" ? "text-destructive" : "text-warning"
        )}
      />
      <span className="w-44 shrink-0 truncate font-medium">{flag.name}</span>
      <span className="shrink-0 font-medium tabular-nums">
        {flag.valueText}
      </span>
      <span className="truncate text-xs text-muted-foreground">
        {flag.reason}
      </span>
      <ChevronRight
        className="ml-auto size-3.5 shrink-0 text-muted-foreground"
        aria-hidden
      />
    </Link>
  );
}

/** People listed inside an opened metric before the rest wait behind "+N more". */
const PEOPLE_PER_THEME = 12;

function MoreButton({
  expanded,
  hidden,
  onClick,
}: {
  expanded: boolean;
  hidden: number;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="self-start px-3 py-1 text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
    >
      {expanded ? "Show fewer" : `+${hidden} more`}
    </button>
  );
}

function Theme({
  metricKey,
  metricLabel,
  flags,
}: {
  metricKey: string;
  metricLabel: string;
  flags: AttentionFlag[];
}) {
  const [open, setOpen] = useState(false);
  const [all, setAll] = useState(false);
  const shown = all ? flags : flags.slice(0, PEOPLE_PER_THEME);
  return (
    // Clipping keeps the header fill inside the border's inner radius, which is
    // the outer radius minus the border width.
    <div className="overflow-hidden rounded-lg border bg-card">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={`attention-${metricKey}`}
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-3 px-3 py-2 text-left text-sm transition-colors hover:bg-accent"
      >
        {open ? (
          <ChevronDown className="size-4 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="size-4 shrink-0 text-muted-foreground" />
        )}
        <span className="font-medium">{metricLabel}</span>
        <span className="ml-auto shrink-0 text-muted-foreground tabular-nums">
          {people(flags.length)}
        </span>
      </button>
      {open ? (
        <div id={`attention-${metricKey}`} className="flex flex-col gap-0.5 border-t p-1.5">
          {shown.map((flag) => (
            <PersonRow key={flag.personId} flag={flag} />
          ))}
          {flags.length > PEOPLE_PER_THEME ? (
            <MoreButton
              expanded={all}
              hidden={flags.length - PEOPLE_PER_THEME}
              onClick={() => setAll((v) => !v)}
            />
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

/**
 * What stands out this period, gathered by the metric it is about.
 *
 * Used by the team-state roster and the org overview.
 */
export function AttentionList({
  flags,
  summary,
  max = 12,
}: {
  flags: AttentionFlag[];
  summary: string;
  /** Metrics listed before the rest wait behind "+N more". */
  max?: number;
}) {
  const [allThemes, setAllThemes] = useState(false);
  const themes = groupFlagsByTheme(flags);
  const shownThemes = allThemes ? themes : themes.slice(0, max);
  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        Needs attention
      </p>

      <Card>
        <CardContent className="p-3 text-sm">{summary}</CardContent>
      </Card>

      {themes.length > 0 ? (
        <div className="flex flex-col gap-1.5">
          {shownThemes.map((theme) => (
            <Theme
              key={theme.metricKey}
              metricKey={theme.metricKey}
              metricLabel={theme.metricLabel}
              flags={theme.flags}
            />
          ))}
          {themes.length > max ? (
            <MoreButton
              expanded={allThemes}
              hidden={themes.length - max}
              onClick={() => setAllThemes((v) => !v)}
            />
          ) : null}
        </div>
      ) : (
        <div className="rounded-lg border bg-card p-4 text-sm text-muted-foreground">
          Nothing stands out this period.
        </div>
      )}
    </section>
  );
}
