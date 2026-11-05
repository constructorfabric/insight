import { useLayoutEffect, useRef } from "react";

import { Button, Input } from "@gears-frontx/ui-kit";
import { cronTranslation } from "@/lib/portal/connector-schedule-editor";

import { rowIndex, type Violation } from "./refusals";

export function CronRows({
  id,
  crons,
  errors,
  onChange,
}: {
  id: string;
  crons: string[];
  errors: Violation[];
  onChange: (crons: string[]) => void;
}) {
  const inputs = useRef<(HTMLInputElement | null)[]>([]);
  const focusAfterRemoval = useRef<number | null>(null);
  useLayoutEffect(() => {
    if (focusAfterRemoval.current === null) return;
    inputs.current[focusAfterRemoval.current]?.focus();
    focusAfterRemoval.current = null;
  }, [crons]);

  return (
    <div className="min-w-0 space-y-2">
      <div className="flex flex-wrap justify-between gap-2 text-xs text-muted-foreground">
        <span>UTC · minute hour day month weekday</span>
        <a
          href="https://crontab.guru"
          target="_blank"
          rel="noopener noreferrer"
          className="text-primary hover:underline"
        >
          Syntax help ↗
        </a>
      </div>
      {crons.map((cron, index) => {
        const rowErrors = errors.filter(
          (violation) => rowIndex(violation) === index
        );
        return (
          <div key={index} className="flex min-w-0 flex-wrap items-start gap-2">
            <Input
              ref={(input) => {
                inputs.current[index] = input;
              }}
              aria-label={`Cron expression ${index + 1}`}
              aria-invalid={rowErrors.length > 0}
              aria-describedby={`${id}-cron-${index}`}
              className="w-[24ch] max-w-full min-w-0 font-mono text-sm"
              value={cron}
              onChange={(event) =>
                onChange(
                  crons.map((value, row) =>
                    row === index ? event.target.value : value
                  )
                )
              }
            />
            <p
              id={`${id}-cron-${index}`}
              role={rowErrors.length ? "alert" : undefined}
              className={`min-w-0 flex-1 basis-32 pt-2 text-xs ${rowErrors.length ? "text-destructive" : "text-muted-foreground"}`}
            >
              {rowErrors.length
                ? rowErrors.map((error) => error.description).join(" ")
                : cronTranslation(cron)}
            </p>
            {crons.length > 1 && (
              <Button
                type="button"
                size="sm"
                variant="ghost"
                icon="×"
                aria-label={`Remove expression ${index + 1}`}
                onClick={() => {
                  focusAfterRemoval.current = Math.max(0, index - 1);
                  onChange(crons.filter((_, row) => row !== index));
                }}
              />
            )}
          </div>
        );
      })}
      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          size="sm"
          variant="link"
          className="h-auto px-0"
          disabled={crons.length >= 10}
          onClick={() => onChange([...crons, ""])}
        >
          + Add expression
        </Button>
        {crons.length >= 10 && (
          <span className="text-xs text-muted-foreground">
            Up to 10 expressions
          </span>
        )}
      </div>
    </div>
  );
}
