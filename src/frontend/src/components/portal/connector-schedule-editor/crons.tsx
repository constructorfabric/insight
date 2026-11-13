import { useEffect, useLayoutEffect, useRef, useState } from "react";

import { Button, Input } from "@gears-frontx/ui-kit";

import { rowIndex, type Violation } from "./refusals";

function CronRow({
  id,
  index,
  cron,
  clientErrors,
  errors,
  validationAttempt,
  inputRef,
  onChange,
  onRemove,
}: {
  id: string;
  index: number;
  cron: string;
  clientErrors: Violation[];
  errors: Violation[];
  validationAttempt: number;
  inputRef: (input: HTMLInputElement | null) => void;
  onChange: (cron: string) => void;
  onRemove?: () => void;
}) {
  const [addedAtAttempt] = useState(validationAttempt);
  const [edited, setEdited] = useState(false);
  const [revealed, setRevealed] = useState(false);
  const showClientErrors = revealed || validationAttempt > addedAtAttempt;
  const hasClientErrors = clientErrors.length > 0;

  useEffect(() => {
    if (!edited || showClientErrors || !cron.trim() || !hasClientErrors) return;
    const timer = setTimeout(() => setRevealed(true), 500);
    return () => clearTimeout(timer);
  }, [cron, edited, showClientErrors, hasClientErrors]);

  const rowErrors = [...(showClientErrors ? clientErrors : []), ...errors];
  return (
    <div className="flex min-w-0 flex-wrap items-start gap-2">
      <Input
        ref={inputRef}
        aria-label={`Cron expression ${index + 1}`}
        aria-invalid={rowErrors.length > 0}
        aria-describedby={id}
        className="w-[24ch] max-w-full min-w-0 font-mono text-sm"
        value={cron}
        placeholder="0 */6 * * *"
        onChange={(event) => {
          setEdited(true);
          onChange(event.target.value);
        }}
        onBlur={() => {
          if (cron.trim()) setRevealed(true);
        }}
      />
      <p
        id={id}
        role={rowErrors.length ? "alert" : undefined}
        className={`min-w-0 flex-1 basis-32 pt-2 text-xs ${rowErrors.length ? "text-destructive" : "text-muted-foreground"}`}
      >
        {rowErrors.length
          ? rowErrors.map((error) => error.description).join(" ")
          : !cron.trim()
            ? "Enter an expression or remove the row"
            : null}
      </p>
      {onRemove && (
        <Button
          type="button"
          size="sm"
          variant="ghost"
          icon="×"
          aria-label={`Remove expression ${index + 1}`}
          onClick={onRemove}
        />
      )}
    </div>
  );
}

export function CronRows({
  id,
  crons,
  clientErrors,
  errors,
  validationAttempt,
  onChange,
}: {
  id: string;
  crons: string[];
  clientErrors: Violation[];
  errors: Violation[];
  validationAttempt: number;
  onChange: (crons: string[]) => void;
}) {
  const [keys, setKeys] = useState(() => crons.map((_, index) => index));
  const nextKey = useRef(crons.length);
  const inputs = useRef<(HTMLInputElement | null)[]>([]);
  const focusAfterChange = useRef<number | null>(null);
  useLayoutEffect(() => {
    if (focusAfterChange.current === null) return;
    inputs.current[focusAfterChange.current]?.focus();
    focusAfterChange.current = null;
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
      {crons.map((cron, index) => (
        <CronRow
          key={keys[index]}
          id={`${id}-cron-${index}`}
          index={index}
          cron={cron}
          clientErrors={clientErrors.filter(
            (violation) => rowIndex(violation) === index
          )}
          errors={errors.filter((violation) => rowIndex(violation) === index)}
          validationAttempt={validationAttempt}
          inputRef={(input) => {
            inputs.current[index] = input;
          }}
          onChange={(cron) =>
            onChange(crons.map((value, row) => (row === index ? cron : value)))
          }
          onRemove={
            crons.length > 1
              ? () => {
                  focusAfterChange.current = Math.max(0, index - 1);
                  setKeys(keys.filter((_, row) => row !== index));
                  onChange(crons.filter((_, row) => row !== index));
                }
              : undefined
          }
        />
      ))}
      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          size="sm"
          variant="link"
          className="h-auto px-0"
          disabled={crons.length >= 10}
          onClick={() => {
            focusAfterChange.current = crons.length;
            setKeys([...keys, nextKey.current++]);
            onChange([...crons, ""]);
          }}
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
