import type { ReactElement } from "react";

import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@gears-frontx/ui-kit";
import { SWEEP_TOOLTIP_DELAY_MS } from "@/lib/tooltip-delay";
import { metricHelp, type MetricHelpText } from "@/lib/insight/metric-help";
import { cn } from "@/lib/utils";

export interface MetricHelpTooltipProps {
  /** The catalog's words for the metric; null when it has none. */
  help: MetricHelpText | null;
  /** The metric's full name, for a surface that shortens or may cut it. */
  name?: string;
  /** The surface itself becomes the trigger — see the note below. */
  children: ReactElement;
}

/**
 * What a metric means, on the surface that shows it.
 *
 * The TRIGGER is the whole tile or row rather than a help icon beside the
 * name. A row of five tiles would otherwise grow five more icons — and the
 * tile is already a button, so making it the trigger costs no new tab stop
 * and answers a keyboard reader on focus, which a hover-only badge never
 * does.
 */
export function MetricHelpTooltip({
  help,
  name,
  children,
}: MetricHelpTooltipProps) {
  // Nothing to say ⇒ the surface renders unchanged, with no dead affordance.
  if (!help && !name) return children;
  return (
    <TooltipProvider delay={SWEEP_TOOLTIP_DELAY_MS}>
      <Tooltip>
        <TooltipTrigger render={children} />
        <TooltipContent
          data-testid="metric-help"
          side="top"
          className="flex-col items-start gap-1.5 text-left text-sm leading-relaxed"
        >
          {name ? <p className="font-medium">{name}</p> : null}
          {help?.description ? <p>{help.description}</p> : null}
          {help?.explanation ? (
            <p className="text-background/70">{help.explanation}</p>
          ) : null}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

/** The two copy fields every metric result and definition carries. */
interface MetricCopy {
  label: string;
  description?: string | null;
  explanation?: string | null;
}

export interface MetricNameProps {
  metric: MetricCopy;
  /** Overrides the rendered text (e.g. a short label) — the help is the same. */
  text?: string;
  className?: string;
  /** Makes the name a tab stop. Only where no control encloses it. */
  focusable?: boolean;
  /** Keeps the name on one line, cut with an ellipsis; the tooltip then names it in full. */
  truncate?: boolean;
}

/**
 * A metric's name, wherever it is written, carrying its own explanation.
 *
 * Named separately from `MetricHelpTooltip` because most metric names are NOT
 * on a surface of their own: they are rows inside a card that is itself a
 * button, so the name cannot be a second focusable element without nesting one
 * interactive control in another. The trigger is therefore a plain span —
 * hover and pointer only, unless the name sits on a surface that is not a
 * control, where `focusable` lets a keyboard reader open it too. Where the
 * surface IS the control (a KPI tile, an attention row), use
 * `MetricHelpTooltip` on that control instead, which answers focus too.
 */
export function MetricName({
  metric,
  text,
  className,
  focusable = false,
  truncate = false,
}: MetricNameProps) {
  const label = text ?? metric.label;
  const shortenedFrom = label === metric.label ? undefined : metric.label;
  const tooltipName = truncate ? metric.label : shortenedFrom;
  const help = metricHelp(metric);
  const spoken = [shortenedFrom, help?.description, help?.explanation]
    .filter(Boolean)
    .join(". ");
  const nameClass = cn(truncate && "min-w-0 truncate", className);
  if (!help && !tooltipName) return <span className={nameClass}>{label}</span>;
  return (
    <>
      <MetricHelpTooltip help={help} name={tooltipName}>
        <span
          tabIndex={focusable ? 0 : undefined}
          className={cn("cursor-help", nameClass)}
        >
          {label}
        </span>
      </MetricHelpTooltip>
      {/* The tooltip answers a pointer and nothing else, because its trigger
          cannot be made focusable here: these names sit inside cards that are
          themselves buttons, and a control nested in a control is invalid
          markup with broken semantics. So the words are given to assistive
          technology directly, in the reading order they belong to. */}
      {spoken ? <span className="sr-only">{spoken}</span> : null}
    </>
  );
}
