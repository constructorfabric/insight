import { Sparkles, TrendingDownIcon, TrendingUpIcon } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardAction,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { useSettings } from "@/hooks/use-settings";
import type { KpiTileData } from "@/lib/insight/kpi-row";
import type { GroupId } from "@/lib/insight/groups";
import { STATUS_TEXT_CLASS } from "@/lib/status";
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
    <Card
      className={cn(
        CARD_SURFACE,
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
      <CardHeader>
        <CardDescription className="min-w-0 truncate">
          {tile.label}
        </CardDescription>
        {tile.delta ? (
          <CardAction className="row-span-1">
            <Badge
              variant="outline"
              className={STATUS_TEXT_CLASS[tile.delta.status]}
            >
              {tile.delta.down ? <TrendingDownIcon /> : <TrendingUpIcon />}
              {tile.delta.text}
            </Badge>
          </CardAction>
        ) : null}
        {showExplanations ? (
          <CardDescription className="col-span-full line-clamp-2 min-h-[2lh] min-w-0 font-normal text-muted-foreground/70">
            {tile.context}
          </CardDescription>
        ) : null}
        <CardTitle
          className={cn(
            "text-2xl font-semibold tabular-nums @[250px]/card:text-3xl",
            tile.valueStatus !== "neutral" &&
              STATUS_TEXT_CLASS[tile.valueStatus]
          )}
        >
          {tile.value}
        </CardTitle>
      </CardHeader>
      <CardFooter className="text-sm text-muted-foreground">
        {tile.gapText && tile.medianLabel ? (
          <span>
            <span
              className={cn("font-medium", STATUS_TEXT_CLASS[tile.gapStatus])}
            >
              {tile.gapText}
            </span>{" "}
            vs {tile.medianLabel}
          </span>
        ) : (
          (tile.medianLabel ?? "No peer data")
        )}
      </CardFooter>
    </Card>
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
