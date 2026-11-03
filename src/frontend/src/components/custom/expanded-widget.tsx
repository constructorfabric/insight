import { useQuery } from "@tanstack/react-query";

import type { DashboardWidget, RunOptions } from "@/api/custom-client";
import { CustomWidget } from "@/components/custom/custom-widget";
import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { dashboardWidgetResultQuery } from "@/queries/custom";

// INVARIANT: runs with the card's options, so it reads the card's cached
// result instead of running the metric again.
export function ExpandedWidget({
  dashboard,
  widget,
  heading,
  options,
  allTime,
  onClose,
}: {
  dashboard: string;
  widget: DashboardWidget;
  heading: string;
  options: RunOptions | undefined;
  /** Whether the board picks a window this widget has no date to honour. */
  allTime: boolean;
  onClose: () => void;
}) {
  const result = useQuery(
    dashboardWidgetResultQuery(dashboard, widget.id, options)
  );

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="flex h-[96dvh] w-[98vw] max-w-[98vw] flex-col">
        <DialogHeader className="flex-row items-center gap-2 pr-10">
          <DialogTitle className="min-w-0 truncate">{heading}</DialogTitle>
          {allTime ? (
            <Badge variant="secondary" className="shrink-0">
              All time
            </Badge>
          ) : null}
        </DialogHeader>
        <div className="min-h-0 flex-1 overflow-auto [&_[data-slot=table-header]]:bg-popover">
          <CustomWidget
            widget={widget.body}
            result={result.data}
            error={result.error}
            windowed={Boolean(options)}
            pending={result.isPending || result.isFetching}
            onRetry={() => void result.refetch()}
            fillHeight
          />
        </div>
      </DialogContent>
    </Dialog>
  );
}
