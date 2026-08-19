import type { ReactElement } from "react";

import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { metricHelp, type MetricHelpText } from "@/lib/insight/metric-help";
import { cn } from "@/lib/utils";

export interface MetricHelpTooltipProps {
  /** Null ⇒ the surface renders unchanged, with no dead affordance. */
  help: MetricHelpText | null;
  /** The surface itself becomes the trigger — see the note below. */
  children: ReactElement;
}

/** Long enough that sweeping the mouse across a row of tiles stays quiet. */
const HOVER_DELAY_MS = 400;

/**
 * What a metric means, on the surface that shows it.
 *
 * The TRIGGER is the whole tile or row rather than a help icon beside the
 * name. A row of five tiles would otherwise grow five more icons — and the
 * tile is already a button, so making it the trigger costs no new tab stop
 * and answers a keyboard reader on focus, which a hover-only badge never
 * does.
 */
export function MetricHelpTooltip({ help, children }: MetricHelpTooltipProps) {
  if (!help) return children;
  return (
    <TooltipProvider delay={HOVER_DELAY_MS}>
      <Tooltip>
        <TooltipTrigger render={children} />
        <TooltipContent
          data-testid="metric-help"
          side="top"
          className="flex-col items-start gap-1.5 text-left text-sm leading-relaxed"
        >
          {help.description ? <p>{help.description}</p> : null}
          {help.explanation ? (
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
}

/**
 * A metric's name, wherever it is written, carrying its own explanation.
 *
 * Named separately from `MetricHelpTooltip` because most metric names are NOT
 * on a surface of their own: they are rows inside a card that is itself a
 * button, so the name cannot be a second focusable element without nesting one
 * interactive control in another. The trigger is therefore a plain span —
 * hover and pointer only. A keyboard reader opens the card, where the same
 * copy is written out. Where the surface IS the control (a KPI tile, an
 * attention row), use `MetricHelpTooltip` on that control instead, which
 * answers focus too.
 */
export function MetricName({ metric, text, className }: MetricNameProps) {
  const label = text ?? metric.label;
  const help = metricHelp(metric);
  if (!help) return <span className={className}>{label}</span>;
  return (
    <>
      <MetricHelpTooltip help={help}>
        <span className={cn("cursor-help", className)}>{label}</span>
      </MetricHelpTooltip>
      {/* The tooltip answers a pointer and nothing else, because its trigger
          cannot be made focusable here: these names sit inside cards that are
          themselves buttons, and a control nested in a control is invalid
          markup with broken semantics. So the words are given to assistive
          technology directly, in the reading order they belong to. */}
      <span className="sr-only">
        {[help.description, help.explanation].filter(Boolean).join(". ")}
      </span>
    </>
  );
}
