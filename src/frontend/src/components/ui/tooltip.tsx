import {
  TooltipProvider as KitTooltipProvider,
  type TooltipProviderProps,
} from "@gears-frontx/ui-kit";

// The kit re-exports Base UI's provider untouched; its default delay is 600ms.
function TooltipProvider({ delay = 0, ...props }: TooltipProviderProps) {
  return <KitTooltipProvider delay={delay} {...props} />;
}

export { Tooltip, TooltipTrigger, TooltipContent } from "@gears-frontx/ui-kit";
export { TooltipProvider };
