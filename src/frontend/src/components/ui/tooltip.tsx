import {
  TooltipContent as KitTooltipContent,
  TooltipProvider as KitTooltipProvider,
  TooltipTrigger as KitTooltipTrigger,
  type TooltipContentProps,
  type TooltipProviderProps,
  type TooltipTriggerProps,
} from "@gears-frontx/ui-kit";

// The kit re-exports Base UI's provider untouched; its default delay is 600ms.
function TooltipProvider({ delay = 0, ...props }: TooltipProviderProps) {
  return <KitTooltipProvider delay={delay} {...props} />;
}

// The stand's UI journeys locate tooltips by these attributes; the kit drops them.
function TooltipTrigger(props: TooltipTriggerProps) {
  return <KitTooltipTrigger data-slot="tooltip-trigger" {...props} />;
}

function TooltipContent(props: TooltipContentProps) {
  return <KitTooltipContent data-slot="tooltip-content" {...props} />;
}

export { Tooltip } from "@gears-frontx/ui-kit";
export { TooltipProvider, TooltipTrigger, TooltipContent };
