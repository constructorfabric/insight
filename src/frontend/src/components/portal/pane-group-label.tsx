import type { ComponentProps } from "react";

import { SidebarGroupLabel } from "@/components/ui/sidebar";
import { cn } from "@/lib/utils";

/** A pane group's heading, set as the design's uppercase eyebrow. */
export function PaneGroupLabel({
  className,
  ...props
}: ComponentProps<typeof SidebarGroupLabel>) {
  return (
    <SidebarGroupLabel
      className={cn(
        "items-end pb-1 font-semibold tracking-[0.05em] text-muted-foreground uppercase",
        className
      )}
      {...props}
    />
  );
}
