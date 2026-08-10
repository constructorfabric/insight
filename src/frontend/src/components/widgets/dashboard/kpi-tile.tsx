import {
  ChevronRight,
  Minus,
  Sparkles,
  TrendingDownIcon,
  TrendingUpIcon,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { MetricHelpTooltip } from "@/components/widgets/metric-help-tooltip";
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
import { STATUS_BG_CLASS, STATUS_TEXT_CLASS } from "@/lib/status";
import { cn } from "@/lib/utils";

export interface KpiTileProps {
  tile: KpiTileData;
  onOpenGroup?: (id: GroupId) => void;
}

const CARD_SURFACE = "@container/card";

/**
 * Presentational KPI tile: everything display-ready arrives on `tile`
 * (selectors in `lib/insight/kpi-row.ts` own formatting and scoring).
 */
export function KpiTile({ tile, onOpenGroup }: KpiTileProps) {
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
            className="absolute right-3 bottom-3 size-3.5 text-muted-foreground/50"
            aria-hidden
          />
        ) : null}
        <CardHeader>
          {/* Wraps to two lines rather than truncating: a name cut to "Pull
              requests mer…" is not a shorter name, it is a missing one. The
              fixed two-line height keeps tiles in a row aligned whether their
              names take one line or two. */}
          <CardDescription className="line-clamp-2 min-h-[2lh] min-w-0">
            {tile.label}
          </CardDescription>
          {showExplanations ? (
            <CardDescription className="col-span-full line-clamp-2 min-h-[2lh] min-w-0 font-normal text-muted-foreground/70">
              {tile.help?.description}
            </CardDescription>
          ) : null}
          {/* The delta rides WITH the value, not up beside the label: it is a
              statement about this number ("−1 pp since last period"), and read
              from the label row it looked like a second, unrelated figure.
              Pinned to the right edge so the badges line up down a row of tiles
              — a column the eye can scan instead of chasing each value's end.
              Plain ink, always: the number states a quantity, and the verdict
              on it belongs to the line that explains the comparison. */}
          <CardTitle
            className={cn(
              "col-span-full flex w-full flex-wrap items-baseline justify-between gap-x-2 gap-y-1",
              "text-2xl font-semibold tabular-nums @[250px]/card:text-3xl"
            )}
          >
            <span>{tile.value}</span>
            {tile.delta ? (
              <Badge
                variant="outline"
                className={cn(
                  "self-center",
                  STATUS_TEXT_CLASS[tile.delta.status]
                )}
              >
                {tile.delta.status === "neutral" ? (
                  <Minus />
                ) : tile.delta.down ? (
                  <TrendingDownIcon />
                ) : (
                  <TrendingUpIcon />
                )}
                {tile.delta.text}
              </Badge>
            ) : null}
          </CardTitle>
        </CardHeader>
        {/* Pushed to the bottom edge so a row of tiles lines its footers up
            even when one of them wraps its badge onto a second line; the
            two-line reserve keeps a wrapped comparison from changing the
            card's height. */}
        <CardFooter className="mt-auto min-h-[2lh] items-start text-sm text-muted-foreground">
          {tile.medianLabel ? (
            <span>
              {/* The standing, on the sentence that states it — and marked by
                  a dot as well as a colour, so it survives being read by
                  someone who cannot tell the two colours apart. */}
              {tile.gapStatus !== "neutral" ? (
                <span
                  className={cn(
                    "mr-1.5 inline-block size-1.5 rounded-full align-middle",
                    STATUS_BG_CLASS[tile.gapStatus]
                  )}
                  aria-hidden
                />
              ) : null}
              <span className={STATUS_TEXT_CLASS[tile.gapStatus]}>
                {tile.gapText ? `${tile.gapText} vs ` : "at "}
                {tile.medianLabel}
              </span>
            </span>
          ) : (
            "No peer data"
          )}
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
        <CardTitle className="text-2xl font-semibold tabular-nums">—</CardTitle>
      </CardHeader>
      <CardFooter className="gap-1.5 text-sm text-muted-foreground">
        <Sparkles className="size-3.5 shrink-0" aria-hidden />
        Coming soon
      </CardFooter>
    </Card>
  );
}
