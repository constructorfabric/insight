import { useId, type ReactElement } from "react";

import { Tooltip, TooltipContent, TooltipTrigger } from "@gears-frontx/ui-kit";

export function ConnectorScheduleTooltip({
  children,
  lines,
}: {
  children: ReactElement;
  lines: string[];
}) {
  const id = useId();
  return (
    <Tooltip>
      <TooltipTrigger
        aria-describedby={id}
        render={
          <span
            tabIndex={0}
            className="inline-flex max-w-full min-w-0 cursor-help"
            onClick={(event) => event.stopPropagation()}
          />
        }
      >
        {children}
      </TooltipTrigger>
      <TooltipContent id={id} role="tooltip">
        <div className="flex flex-col">
          {lines.map((line, index) => (
            <span key={index} className="block">
              {line}
            </span>
          ))}
        </div>
      </TooltipContent>
    </Tooltip>
  );
}
