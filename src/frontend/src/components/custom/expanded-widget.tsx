import { useQuery } from "@tanstack/react-query";
import { Table2 } from "lucide-react";
import { useState } from "react";

import {
  isBusy,
  type DashboardWidget,
  type RunOptions,
} from "@/api/custom-client";
import { CustomWidget } from "@/components/custom/custom-widget";
import { WidgetDrilldown } from "@/components/custom/widget-drilldown";
import { Badge } from "@gears-frontx/ui-kit";
import { Button } from "@gears-frontx/ui-kit";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@gears-frontx/ui-kit";
import { dashboardWidgetResultQuery } from "@/queries/custom";
import { useIsAdmin } from "@/queries/identity-me";

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
  const { isAdmin } = useIsAdmin();
  const [drilldown, setDrilldown] = useState(false);
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
        <DialogHeader className="flex-row items-center gap-2 pr-14">
          <DialogTitle className="min-w-0 truncate">{heading}</DialogTitle>
          {allTime ? (
            <Badge variant="secondary" className="shrink-0">
              All time
            </Badge>
          ) : null}
          {isAdmin ? (
            <Button
              variant="outline"
              size="sm"
              className="ml-auto shrink-0"
              icon={<Table2 />}
              disabled={isBusy(result.error)}
              onClick={() => setDrilldown(true)}
            >
              Show data
            </Button>
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
            onPointClick={isAdmin ? () => setDrilldown(true) : undefined}
          />
        </div>
        {/* INVARIANT: rendered inside this dialog so it nests over it; a sibling
            dialog would fight its focus trap. */}
        {isAdmin ? (
          <WidgetDrilldown
            dashboard={dashboard}
            widgetId={widget.id}
            widget={widget.body}
            name={widget.name}
            label={heading}
            open={drilldown}
            onOpenChange={setDrilldown}
            options={options}
          />
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
