import {
  ChevronRight,
  Minus,
  Sparkles,
  TrendingDownIcon,
  TrendingUpIcon,
} from "lucide-react";

import { MetricHelpTooltip } from "@/components/widgets/metric-help-tooltip";
import { Sparkline } from "@/components/widgets/dashboard/sparkline";
import {
  Card,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { useSettings } from "@/hooks/use-settings";
import type { KpiTileData } from "@/lib/insight/kpi-row";
import type { GroupId } from "@/lib/insight/groups";
import { STATUS_TEXT_CLASS } from "@/lib/status";
import { TEXT_FIGURE, TEXT_LABEL, TEXT_NAME } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

export interface KpiTileProps {
  tile: KpiTileData;
  /** "month" / "week" — names what the change is measured against. */
  periodNoun: string;
  /**
   * Finished readings, oldest first; absent while they load or when there are
   * too few to draw. Never blocks the tile — the numbers are the page, and the
   * line is what the page grows once it can.
   */
  trend?: (number | null)[] | null;
  onOpenGroup?: (id: GroupId) => void;
}

const CARD_SURFACE = "@container/card";

/**
 * Presentational KPI tile: everything display-ready arrives on `tile`
 * (selectors in `lib/insight/kpi-row.ts` own formatting and scoring).
 */
export function KpiTile({
  tile,
  periodNoun,
  trend,
  onOpenGroup,
}: KpiTileProps) {
  const { showExplanations } = useSettings();
  const primaryGroup = onOpenGroup ? tile.groupId : null;
  const interactive = primaryGroup != null;

  return (
    <MetricHelpTooltip help={tile.help}>
      <Card
        className={cn(
          CARD_SURFACE,
          "relative",
          interactive && "text-left transition-colors hover:bg-accent/50"
        )}
        render={
          primaryGroup ? (
            <button
              type="button"
              onClick={() => onOpenGroup?.(primaryGroup)}
              aria-label={`Open ${tile.label} details`}
            />
          ) : undefined
        }
      >
        {/* A STANDING affordance, not a hover one: until you point at it, a
            hover-only tile is indistinguishable from static text, so the screen
            has to be swept with a mouse to learn what opens. In the corner
            rather than beside the label, which is the scarcest line on a 13rem
            tile — it already carries a delta badge. */}
        {interactive ? (
          <ChevronRight
            className="absolute right-3 bottom-3 size-3.5 text-muted-foreground"
            aria-hidden
          />
        ) : null}
        <CardHeader>
          {/* Wraps to two lines rather than truncating: a name cut to "Pull
              requests mer…" is not a shorter name, it is a missing one. The
              fixed two-line height keeps tiles in a row aligned whether their
              names take one line or two. */}
          <CardDescription
            className={cn(TEXT_NAME, "line-clamp-2 min-h-[2lh] min-w-0")}
          >
            {tile.label}
          </CardDescription>
          {showExplanations ? (
            <CardDescription className={cn(TEXT_LABEL, "col-span-full line-clamp-2 min-h-[2lh] min-w-0 font-normal")}>
              {tile.help?.description}
            </CardDescription>
          ) : null}
          {/* Plain ink, and the badge is gone: its content moved into the
              line below, where it can say what it is comparing against
              instead of leaving "-13%" to be read as either comparison. */}
          <CardTitle
            className={cn(
              "col-span-full flex w-full flex-wrap items-baseline justify-between gap-x-2 gap-y-1",
              TEXT_FIGURE
            )}
          >
            <span>{tile.value}</span>
            {/* Beside the number it describes, not in a block of its own: the
                question "and before that?" is asked here, and an answer one
                scroll away is an answer nobody reads.
                Pinned to the right edge rather than trailing the value — the
                values are of different widths, so a line that starts where
                each one ends leaves four lines at four x-positions. On the
                edge they form a column the eye can read down. Absent until
                the readings arrive: the page does not wait for it. */}
            {trend ? (
              <Sparkline points={trend} className="self-center" />
            ) : null}
          </CardTitle>
        </CardHeader>
        {/* Pushed to the bottom edge so a row of tiles lines its footers up
            even when one of them wraps its badge onto a second line; the
            two-line reserve keeps a wrapped comparison from changing the
            card's height. */}
        <CardFooter className={cn("mt-auto min-h-[2lh] flex-col items-start gap-0.5", TEXT_LABEL)}>
          {/* The person's own change first, and the only line carrying a
              verdict. This is their page: their own last period is the one
              comparison they can act on. */}
          {tile.delta ? (
            <span
              className={cn(
                "flex items-center gap-1",
                STATUS_TEXT_CLASS[tile.delta.status]
              )}
            >
              {tile.delta.status === "neutral" ? (
                <Minus className="size-3.5" />
              ) : tile.delta.down ? (
                <TrendingDownIcon className="size-3.5" />
              ) : (
                <TrendingUpIcon className="size-3.5" />
              )}
              {tile.delta.text} since last {periodNoun}
            </span>
          ) : (
            <span>no earlier {periodNoun} to compare</span>
          )}
          {/* The cohort, stated and not judged: the reader did not choose these
              people, cannot see who they are, and cannot decide that their
              median is the right target. */}
          <span>
            {tile.medianLabel
              ? `Team ${tile.medianLabel}${tile.gapText ? ` · ${tile.gapText}` : ""}`
              : "No peer data"}
          </span>
        </CardFooter>
      </Card>
    </MetricHelpTooltip>
  );
}

export function KpiTilePlaceholder({ label }: { label?: string }) {
  const { showExplanations } = useSettings();
  return (
    <Card className={CARD_SURFACE}>
      <CardHeader>
        <CardDescription className="min-w-0 truncate">
          {label ?? " "}
        </CardDescription>
        {showExplanations ? (
          <CardDescription className="col-span-full min-h-[2lh]" />
        ) : null}
        <CardTitle className={TEXT_FIGURE}>—</CardTitle>
      </CardHeader>
      <CardFooter className={cn("gap-1.5", TEXT_LABEL)}>
        <Sparkles className="size-3.5 shrink-0" aria-hidden />
        Coming soon
      </CardFooter>
    </Card>
  );
}
