import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Spinner } from "@/components/ui/spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { useSettings } from "@/hooks/use-settings";
import type { MetricGroup } from "@/lib/insight/groups";
import {
  teamMetricStandings,
  type TeamMetricStanding,
} from "@/lib/insight/team-metrics";
import {
  gradeSectionStanding,
  rankCounts,
  sectionStandingPhrase,
} from "@/lib/scoring";
import { applyFocus, PEER_TEXT } from "@/lib/peers";
import {
  STATUS_BG_CLASS,
  STATUS_STRIPE_LEFT,
  applyFocusStatus,
} from "@/lib/status";
import type { MetricCollectionResult } from "@/queries/metric-results";
import { cn } from "@/lib/utils";

export interface TeamMetricGroupCardProps {
  def: MetricGroup;
  data: MetricCollectionResult;
  memberIds: string[];
  onOpen: () => void;
  subtitle?: string;
}

/**
 * Card for a metrics-backed group over a roster of people. No pooled team
 * value — the roster is a set of individuals, not a unit — so each preview
 * row states how the members stand against their OWN cohorts in the shared
 * behind/ahead/on-par vocabulary; the drilldown names who.
 */
export function TeamMetricGroupCard({
  def,
  data,
  memberIds,
  onOpen,
  subtitle,
}: TeamMetricGroupCardProps) {
  const { focusMode } = useSettings();

  if (data.isPending) {
    // Keep the card's identity while it loads: the name in the header, a
    // spinner in the body. Not interactive — nothing to open yet.
    return (
      <Card>
        <CardHeader>
          <CardTitle className="text-base font-semibold">{def.title}</CardTitle>
          {subtitle ? (
            <CardDescription className="text-xs text-muted-foreground">
              {subtitle}
            </CardDescription>
          ) : null}
        </CardHeader>
        <CardContent className="flex items-center justify-center py-6">
          <Spinner
            className="size-5 text-muted-foreground"
            aria-label={`Loading ${def.title}`}
          />
        </CardContent>
      </Card>
    );
  }
  if (data.isError) {
    return (
      <ComingSoon
        variant="card"
        state="error"
        label={def.title}
        onRetry={data.refetch}
      />
    );
  }

  const standings = teamMetricStandings(def, data.byKey, memberIds);
  const scored = standings.filter((s) => s.scored > 0);
  const counts = rankCounts(
    standings.map((standing) => ({ row: standing, rank: standing.verdict })),
  );
  const status = applyFocusStatus(gradeSectionStanding(counts), focusMode);
  const badgeText = sectionStandingPhrase(counts);

  // Preview rows keep their slot even with nobody scorable — a silently
  // shrinking card reads as broken; the row states "no peer data" instead.
  const preview: TeamMetricStanding[] = def.card.preview
    .map((key) => standings.find((s) => s.metric.metric_key === key))
    .filter((s): s is TeamMetricStanding => s != null);
  const stripeClass = STATUS_STRIPE_LEFT[status];

  return (
    <Card
      render={
        <button
          type="button"
          onClick={onOpen}
          aria-label={`Open ${def.title} details`}
        />
      }
      className={cn(
        "text-left transition-colors hover:bg-accent/50",
        stripeClass,
      )}
    >
      <CardHeader>
        <CardTitle className="text-base font-semibold">{def.title}</CardTitle>
        <CardDescription className="flex flex-col gap-1 text-xs">
          {subtitle ? (
            <span className="text-muted-foreground">{subtitle}</span>
          ) : null}
          <span className="flex items-center gap-1.5">
            <span
              className={cn(
                "size-1.5 shrink-0 rounded-full",
                STATUS_BG_CLASS[status],
              )}
              aria-hidden
            />
            <span className="tabular-nums">{badgeText}</span>
          </span>
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3 pt-0">
        {scored.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No metrics with peer data for this period.
          </p>
        ) : (
          <ul className="flex flex-col gap-2 text-sm">
            {(preview.length > 0 ? preview : scored.slice(0, 3)).map(
              (standing) => (
                <li
                  key={standing.metric.metric_key}
                  className="flex items-baseline justify-between gap-2"
                >
                  <span className="min-w-0 truncate text-muted-foreground">
                    {standing.metric.label}
                  </span>
                  <RowStanding standing={standing} focusMode={focusMode} />
                </li>
              ),
            )}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}

/**
 * The row's verdict in the shared chip vocabulary — behind wins over ahead,
 * on par only when nothing sticks out, "no peer data" when nobody on the
 * roster is rankable. Counts only; the drilldown names who.
 */
function RowStanding({
  standing,
  focusMode,
}: {
  standing: TeamMetricStanding;
  focusMode: ReturnType<typeof useSettings>["focusMode"];
}) {
  const { top, bottom, scored } = standing;
  if (scored === 0) {
    return (
      <span className="shrink-0 text-xs text-muted-foreground">
        no peer data
      </span>
    );
  }
  if (bottom > 0) {
    return (
      <span
        className={cn(
          "shrink-0 text-xs tabular-nums",
          PEER_TEXT[applyFocus("bottom", focusMode)],
        )}
      >
        {bottom} behind
      </span>
    );
  }
  if (top > 0) {
    return (
      <span
        className={cn(
          "shrink-0 text-xs tabular-nums",
          PEER_TEXT[applyFocus("top", focusMode)],
        )}
      >
        {top} ahead
      </span>
    );
  }
  return <span className="shrink-0 text-xs text-muted-foreground">on par</span>;
}
