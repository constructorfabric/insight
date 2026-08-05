import { useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";

import {
  Card,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { PeerComparison } from "@/components/widgets/metric-views/peer-comparison";
import { MetricCardActions } from "@/components/widgets/metric-views/metric-card-actions";
import { useSettings } from "@/hooks/use-settings";
import {
  formatMetricNumber,
  formatMetricValue,
  metricDisplayUnit,
} from "@/lib/format";
import {
  partitionPeerStory,
  type PeerStoryEntry,
} from "@/lib/metrics/peer-story";
import { formatGapMagnitude } from "@/lib/metrics/gap";
import { PEER_FILL, PEER_TEXT, type PeerCohortLabel } from "@/lib/peers";
import { STATUS_STRIPE_LEFT, STATUS_STRIPE_TOP } from "@/lib/status";
import { cn } from "@/lib/utils";

interface PeerStoryProps {
  entries: PeerStoryEntry[];
  cohortLabel?: PeerCohortLabel;
  emptyLabel?: string;
  className?: string;
}

/** Null when the gap collapses at display precision — render "at the median". */
function formatGap(entry: PeerStoryEntry): string | null {
  return formatGapMagnitude({
    value: entry.value,
    median: entry.stats?.p50 ?? null,
    gapPct: entry.gapPct,
    gapDelta: entry.gapDelta,
    format: entry.format,
    unit: entry.unit,
  });
}

function outlierText(status: PeerStoryEntry["status"]): string {
  return status === "bottom" ? "Bottom 25%" : "Top 25%";
}

function HeroCard({
  entry,
  cohortLabel,
}: {
  entry: PeerStoryEntry;
  cohortLabel: PeerCohortLabel;
}) {
  const isBad = entry.status === "bottom";
  const color = isBad ? "bottom" : "top";
  const unit = metricDisplayUnit(entry.format, entry.unit);
  return (
    <Card
      className={cn(
        "relative flex h-full min-h-72 flex-col gap-0 p-0",
        STATUS_STRIPE_TOP[isBad ? "bad" : "good"]
      )}
    >
      <MetricCardActions evidence={entry.evidence} label={entry.label} />
      <div className="flex flex-1 flex-col gap-3 p-5 sm:p-6">
        <div className="flex items-center gap-1.5 pr-8">
          <span className={cn("size-1.5 rounded-full", PEER_FILL[color])} />
          <span
            className={cn(
              "text-[10px] font-semibold tracking-widest uppercase",
              PEER_TEXT[color]
            )}
          >
            {isBad ? "Top issue" : "Top win"}
          </span>
        </div>
        <div>
          <h3 className="text-xl font-semibold tracking-tight sm:text-2xl">
            {entry.label}
          </h3>
          {entry.sublabel ? (
            <p className="mt-1 text-sm text-muted-foreground">
              {entry.sublabel}
            </p>
          ) : null}
        </div>
        <div className="flex flex-wrap items-baseline gap-x-2.5 gap-y-1.5">
          <span className="flex items-baseline gap-1">
            <span
              className={cn(
                "text-4xl font-semibold tracking-tight tabular-nums sm:text-[2.75rem]",
                PEER_TEXT[color]
              )}
            >
              {entry.format === "percent"
                ? formatMetricValue(entry.value, entry.format, entry.unit)
                : formatMetricNumber(entry.value, entry.format)}
            </span>
            {unit ? (
              <span className="text-base text-muted-foreground">{unit}</span>
            ) : null}
          </span>
          {entry.stats ? (
            <>
              <span aria-hidden className="text-sm text-muted-foreground">
                ·
              </span>
              <span className="text-sm text-muted-foreground tabular-nums">
                {formatGap(entry) == null ? (
                  <>at the {cohortLabel} median </>
                ) : (
                  <>
                    gap{" "}
                    <span className={cn("font-normal", PEER_TEXT[color])}>
                      {formatGap(entry)}
                    </span>{" "}
                    from {cohortLabel} median{" "}
                  </>
                )}
                <span className="text-foreground">
                  {formatMetricValue(entry.stats.p50, entry.format, entry.unit)}
                </span>
              </span>
            </>
          ) : null}
        </div>
        {entry.stats && entry.stats.max > entry.stats.min ? (
          <PeerComparison
            value={entry.value}
            stats={entry.stats}
            status={entry.status}
            higherIsBetter={entry.higherIsBetter}
            format={entry.format}
            unit={entry.unit}
          />
        ) : null}
        <span className={cn("mt-auto text-xs font-medium", PEER_TEXT[color])}>
          {isBad ? `Bottom 25% in ${cohortLabel}` : `Top 25% in ${cohortLabel}`}
        </span>
      </div>
    </Card>
  );
}

function SideCards({
  entries,
  cohortLabel,
}: {
  entries: PeerStoryEntry[];
  cohortLabel: PeerCohortLabel;
}) {
  if (entries.length === 0) return null;
  const stretchCards = entries.length > 1;
  return (
    <div
      className={cn(
        "grid gap-3",
        stretchCards ? "h-full" : "content-start",
        entries.length === 2 && "grid-rows-2",
        entries.length === 3 && "grid-rows-3"
      )}
    >
      {entries.map((entry) => (
        <SideCard
          key={entry.key}
          entry={entry}
          stretch={stretchCards}
          cohortLabel={cohortLabel}
        />
      ))}
    </div>
  );
}

function SideCard({
  entry,
  stretch,
  cohortLabel,
}: {
  entry: PeerStoryEntry;
  stretch: boolean;
  cohortLabel: PeerCohortLabel;
}) {
  const unit = metricDisplayUnit(entry.format, entry.unit);
  return (
    <Card
      className={cn(
        "relative min-h-28 p-0",
        stretch && "h-full",
        "border-current/20",
        PEER_TEXT[entry.status],
        entry.status === "top" && "bg-success/5",
        entry.status === "bottom" && "bg-destructive/5",
        entry.status === "top" && STATUS_STRIPE_LEFT.good,
        entry.status === "bottom" && STATUS_STRIPE_LEFT.bad
      )}
    >
      <MetricCardActions evidence={entry.evidence} label={entry.label} />
      <div className="flex h-full">
        <div className="flex min-w-0 flex-1 flex-col justify-between gap-3 p-4">
          <div className="min-w-0 pr-8">
            <div className="truncate text-sm font-semibold text-muted-foreground">
              {entry.label}
            </div>
            {entry.sublabel ? (
              <div className="mt-0.5 truncate text-xs text-muted-foreground">
                {entry.sublabel}
              </div>
            ) : null}
          </div>
          <div>
            {/* Mini-hero: value with the gap phrase beside it, rank as the
                footer — the hero's facts at side-card size (minus the band,
                which outgrows these cards), so chips are the only
                tooltip-gated tier. */}
            <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
              <span className="text-2xl font-semibold tabular-nums">
                {entry.format === "percent"
                  ? formatMetricValue(entry.value, entry.format, entry.unit)
                  : formatMetricNumber(entry.value, entry.format)}
                {unit ? (
                  <span className="ml-1 text-xs font-normal text-muted-foreground">
                    {unit}
                  </span>
                ) : null}
              </span>
              {entry.stats ? (
                <>
                  <span aria-hidden className="text-xs text-muted-foreground">
                    ·
                  </span>
                  <span className="text-xs text-muted-foreground tabular-nums">
                    {formatGap(entry) == null ? (
                      <>at the {cohortLabel} median </>
                    ) : (
                      <>
                        gap{" "}
                        <span
                          className={cn("font-normal", PEER_TEXT[entry.status])}
                        >
                          {formatGap(entry)}
                        </span>{" "}
                        from {cohortLabel} median{" "}
                      </>
                    )}
                    <span className="text-foreground">
                      {formatMetricValue(
                        entry.stats.p50,
                        entry.format,
                        entry.unit
                      )}
                    </span>
                  </span>
                </>
              ) : null}
            </div>
            <div className="mt-1 truncate text-[11px] text-muted-foreground">
              {outlierText(entry.status)}
            </div>
          </div>
        </div>
      </div>
    </Card>
  );
}

function ChipTooltip({
  entry,
  cohortLabel,
}: {
  entry: PeerStoryEntry;
  cohortLabel: PeerCohortLabel;
}) {
  return (
    <span className="flex flex-col gap-1 leading-snug">
      <span className="font-medium">{entry.label}</span>
      {entry.sublabel ? (
        <span className="text-background/70">{entry.sublabel}</span>
      ) : null}
      <span>
        {formatMetricValue(entry.value, entry.format, entry.unit)} ·{" "}
        {outlierText(entry.status)}
      </span>
      {entry.stats ? (
        <span className="text-background/70">
          {formatGap(entry) == null
            ? `at the ${cohortLabel} median `
            : `gap ${formatGap(entry)} · ${cohortLabel} median `}
          {formatMetricValue(entry.stats.p50, entry.format, entry.unit)}
        </span>
      ) : null}
    </span>
  );
}

function OutlierChips({
  entries,
  cohortLabel,
}: {
  entries: PeerStoryEntry[];
  cohortLabel: PeerCohortLabel;
}) {
  if (entries.length === 0) return null;
  return (
    <div className="flex flex-wrap gap-1.5">
      {entries.map((entry) => (
        <Tooltip key={entry.key}>
          <TooltipTrigger
            render={
              <button
                type="button"
                className={cn(
                  "inline-flex cursor-help items-center gap-1 rounded-full border bg-transparent px-2.5 py-1 text-xs focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none",
                  PEER_TEXT[entry.status]
                )}
              >
                <span
                  className={cn(
                    "size-1.5 rounded-full",
                    PEER_FILL[entry.status]
                  )}
                />
                {entry.label}
                <span className="font-mono tabular-nums">
                  {formatMetricValue(entry.value, entry.format, entry.unit)}
                </span>
              </button>
            }
          />
          <TooltipContent side="top" className="max-w-64">
            <ChipTooltip entry={entry} cohortLabel={cohortLabel} />
          </TooltipContent>
        </Tooltip>
      ))}
    </div>
  );
}

function FlatGridCard({ entry }: { entry: PeerStoryEntry }) {
  const unit = metricDisplayUnit(entry.format, entry.unit);
  return (
    <Card className="relative p-4">
      <MetricCardActions evidence={entry.evidence} label={entry.label} />
      <div className="min-w-0 pr-8">
        <div className="truncate text-sm font-semibold text-muted-foreground">
          {entry.label}
        </div>
        {entry.sublabel ? (
          <div className="mt-0.5 truncate text-xs text-muted-foreground">
            {entry.sublabel}
          </div>
        ) : null}
      </div>
      <div className="mt-5 text-2xl font-semibold tabular-nums">
        {entry.format === "percent"
          ? formatMetricValue(entry.value, entry.format, entry.unit)
          : formatMetricNumber(entry.value, entry.format)}
        {unit ? (
          <span className="ml-1 text-xs font-normal text-muted-foreground">
            {unit}
          </span>
        ) : null}
      </div>
    </Card>
  );
}

function FlatGrid({
  entries,
  className,
}: {
  entries: PeerStoryEntry[];
  className?: string;
}) {
  if (entries.length === 0) return null;
  return (
    <div
      className={cn(
        "grid [grid-template-columns:repeat(auto-fit,minmax(260px,1fr))] gap-3",
        className
      )}
    >
      {entries.map((entry) => (
        <FlatGridCard key={entry.key} entry={entry} />
      ))}
    </div>
  );
}

function EmptyState({ label }: { label: string }) {
  return <Card className="p-6 text-sm text-muted-foreground">{label}</Card>;
}

function SupportingFold({
  entries,
  cohortLabel,
}: {
  entries: PeerStoryEntry[];
  cohortLabel: PeerCohortLabel;
}) {
  const [open, setOpen] = useState(false);
  if (entries.length === 0) return null;
  const neutralCount = entries.filter(
    (entry) => entry.status === "neutral"
  ).length;
  const trueOnParCount = entries.length - neutralCount;
  const summaryLabel =
    neutralCount === 0
      ? `${entries.length} on-par metric${entries.length === 1 ? "" : "s"}`
      : trueOnParCount === 0
        ? `${entries.length} supporting metric${entries.length === 1 ? "" : "s"}`
        : `${entries.length} supporting and on-par metric${
            entries.length === 1 ? "" : "s"
          }`;
  const summaryDescription =
    neutralCount === 0
      ? `Metrics within the ${cohortLabel}'s normal range - no peer outlier`
      : "Additional metrics without a visible peer outlier";
  return (
    <div className="rounded-md border">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        className="flex w-full items-start justify-between gap-3 px-3 py-2 text-left transition-colors hover:bg-accent"
        aria-expanded={open}
      >
        <div className="min-w-0 flex-1">
          <div className="text-sm font-medium">
            {open ? "Hide" : "Show"} {summaryLabel}
          </div>
          <div className="text-[11px] text-muted-foreground">
            {summaryDescription}
          </div>
        </div>
        {open ? (
          <ChevronDown className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
        )}
      </button>
      {open ? (
        <div className="border-t p-3">
          <div className="grid gap-x-8 gap-y-2 text-sm md:grid-cols-[minmax(180px,280px)_auto_1fr]">
            {entries.map((entry) => (
              <SupportingRow
                key={entry.key}
                entry={entry}
                cohortLabel={cohortLabel}
              />
            ))}
          </div>
        </div>
      ) : null}
    </div>
  );
}

function SupportingRow({
  entry,
  cohortLabel,
}: {
  entry: PeerStoryEntry;
  cohortLabel: PeerCohortLabel;
}) {
  const unit = metricDisplayUnit(entry.format, entry.unit);
  return (
    <div className="contents">
      <div className="font-medium">{entry.label}</div>
      <div className="font-mono font-semibold tabular-nums">
        {entry.format === "percent"
          ? formatMetricValue(entry.value, entry.format, entry.unit)
          : formatMetricNumber(entry.value, entry.format)}
        {unit ? (
          <span className="ml-1 font-sans text-xs font-normal text-muted-foreground">
            {unit}
          </span>
        ) : null}
      </div>
      <div className="text-muted-foreground">
        {!entry.stats ? (
          <span>no peer data</span>
        ) : !entry.observed ? (
          <span>
            no recorded activity · {cohortLabel} median:{" "}
            {formatMetricValue(entry.stats.p50, entry.format, entry.unit)}
          </span>
        ) : entry.status === "in_pack" ? (
          <span>
            on par · {cohortLabel} median:{" "}
            {formatMetricValue(entry.stats.p50, entry.format, entry.unit)}
          </span>
        ) : (
          <span>
            {cohortLabel} median:{" "}
            {formatMetricValue(entry.stats.p50, entry.format, entry.unit)}
          </span>
        )}
      </div>
    </div>
  );
}

export function PeerStory({
  entries,
  cohortLabel = "department",
  emptyLabel = "No counter data yet.",
  className,
}: PeerStoryProps) {
  const { focusMode } = useSettings();
  const { hero, sideCards, chips, folded } = partitionPeerStory(
    entries,
    focusMode
  );

  if (entries.length === 0) {
    return (
      <Card className={className}>
        <CardHeader>
          <CardTitle className="text-sm font-semibold">Counters</CardTitle>
          <CardDescription>{emptyLabel}</CardDescription>
        </CardHeader>
      </Card>
    );
  }

  if (focusMode === "neutral") {
    return <FlatGrid entries={entries} className={className} />;
  }

  if (focusMode === "all" && !hero) {
    return <FlatGrid entries={entries} className={className} />;
  }

  return (
    <div className={cn("flex flex-col gap-4", className)}>
      {hero ? (
        <div className="grid grid-cols-1 items-stretch gap-4 lg:grid-cols-2">
          <HeroCard entry={hero} cohortLabel={cohortLabel} />
          <SideCards entries={sideCards} cohortLabel={cohortLabel} />
        </div>
      ) : focusMode === "critical" ? (
        <EmptyState label="No critical issues this period" />
      ) : focusMode === "rewards" ? (
        <EmptyState label="No standout wins this period" />
      ) : null}
      <OutlierChips entries={chips} cohortLabel={cohortLabel} />
      <SupportingFold entries={folded} cohortLabel={cohortLabel} />
    </div>
  );
}
