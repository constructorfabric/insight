import {
  TooltipProvider as KitTooltipProvider,
  type TooltipProviderProps,
} from "@gears-frontx/ui-kit";

// The kit re-exports Base UI's provider untouched, whose default delay is
// 600ms. This app has always opened tooltips immediately.
function TooltipProvider({ delay = 0, ...props }: TooltipProviderProps) {
  return <KitTooltipProvider delay={delay} {...props} />;
}

export { Tooltip, TooltipTrigger, TooltipContent } from "@gears-frontx/ui-kit";
export { TooltipProvider };
