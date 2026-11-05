import { useEffect, useId, useRef, useState } from "react";

import { AnalyticsApiError } from "@/api/analytics-client";
import type { ScheduleRead } from "@/api/connector-health-client";
import { CronRows } from "./crons";
import { rowIndex, violations, type Violation } from "./refusals";

import { ConnectorScheduleHeader } from "@/components/portal/connector-schedule-header";
import { isScheduleOverride } from "@/lib/portal/connector-schedule";
import { Alert } from "@/components/ui/alert";
import {
  Button,
  Input,
  NativeSelect,
  NativeSelectOption,
} from "@gears-frontx/ui-kit";
import {
  HOUR_CHOICES,
  choiceCrons,
  choiceSummary,
  detectChoice,
  localTime,
  scheduleWords,
  type ScheduleChoice,
} from "@/lib/portal/connector-schedule-editor";

interface Props {
  read: ScheduleRead;
  save: (etag: string, crons: string[] | null) => Promise<ScheduleRead>;
  reload: () => Promise<ScheduleRead>;
  onClose: () => void;
}

function storedCrons(read: ScheduleRead): string[] {
  const { requested, applied } = read.schedule;
  return isScheduleOverride(requested)
    ? requested.crons
    : (applied?.crons ?? []);
}

export function ConnectorScheduleEditor({
  read,
  save,
  reload,
  onClose,
}: Props) {
  const id = useId();
  const [current, setCurrent] = useState(read);
  const [opened, setOpened] = useState(() => storedCrons(read));
  const [choice, setChoice] = useState<ScheduleChoice>(() =>
    detectChoice(storedCrons(read))
  );
  const [error, setError] = useState("");
  const [fieldErrors, setFieldErrors] = useState<Violation[]>([]);
  const [busy, setBusy] = useState(false);
  const form = useRef<HTMLFormElement>(null);
  const focusRefusal = useRef(false);
  useEffect(() => {
    if (busy || !focusRefusal.current) return;
    focusRefusal.current = false;
    const target = form.current?.querySelector<HTMLElement>(
      'input[aria-invalid="true"], [data-refusal]'
    );
    target?.focus();
  }, [busy, error, fieldErrors]);
  const crons = choiceCrons(choice);
  const changed = JSON.stringify(crons) !== JSON.stringify(opened);
  const retry = current.schedule.status === "error";
  const alerts = fieldErrors.filter((violation) => {
    const index = rowIndex(violation);
    return choice.kind !== "custom" || index === null || index >= crons.length;
  });
  const summary = choiceSummary(choice);
  const value = choice.kind === "hours" ? `hours-${choice.hours}` : choice.kind;

  function change(next: ScheduleChoice) {
    setChoice(next);
    setFieldErrors([]);
  }

  function select(value: string) {
    if (value === "custom") {
      change({ kind: "custom", crons: crons.length ? crons : [""] });
    } else if (value === "daily" || value === "weekdays") {
      change({
        kind: value,
        time: choice.kind === "custom" ? "09:00" : choice.time,
      });
    } else {
      change({
        kind: "hours",
        hours: Number(value.slice(6)),
        time: choice.kind === "custom" ? "00:00" : choice.time,
      });
    }
  }

  function prefill(next: ScheduleRead) {
    const crons = storedCrons(next);
    setCurrent(next);
    setOpened(crons);
    setChoice(detectChoice(crons));
  }

  async function submit(reset: boolean) {
    if (busy || (!reset && !changed && !retry)) return;
    setBusy(true);
    setError("");
    setFieldErrors([]);
    try {
      const clearing =
        reset ||
        (retry && !changed && !isScheduleOverride(current.schedule.requested));
      await save(current.etag, clearing ? null : crons);
      onClose();
    } catch (failure) {
      focusRefusal.current = true;
      if (failure instanceof AnalyticsApiError && failure.status === 412) {
        try {
          const next = await reload();
          prefill(next);
          setError(
            `Another admin changed this schedule at ${localTime(next.schedule.requested?.updated_at)}. It is reloaded below; check it and save again.`
          );
        } catch {
          setError(
            "Could not reload the changed schedule. Your input is kept; try again."
          );
        }
      } else if (
        failure instanceof AnalyticsApiError &&
        failure.status === 400
      ) {
        const descriptions = violations(failure);
        setFieldErrors(
          descriptions.length
            ? descriptions
            : [
                {
                  description:
                    "The schedule could not be saved. Check the expressions and try again.",
                },
              ]
        );
      } else {
        setError("Could not save the schedule. Your input is kept; try again.");
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <form
      ref={form}
      className="min-w-0 space-y-3 [overflow-wrap:anywhere] whitespace-normal"
      onSubmit={(event) => {
        event.preventDefault();
        void submit(false);
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape" && !busy) {
          event.preventDefault();
          onClose();
        }
      }}
    >
      <ConnectorScheduleHeader schedule={current.schedule} />
      {error && (
        <Alert role="alert" data-refusal tabIndex={-1} variant="destructive">
          {error}
        </Alert>
      )}
      <fieldset disabled={busy} className="min-w-0 space-y-3">
        <legend className="sr-only">Schedule</legend>
        <div className="min-w-0 space-y-1.5">
          <label
            htmlFor={`${id}-runs`}
            className="block text-xs font-medium text-muted-foreground"
          >
            Runs
          </label>
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <div className="w-[220px] max-w-full min-w-0">
              <NativeSelect
                id={`${id}-runs`}
                autoFocus
                size="sm"
                value={value}
                className="w-full min-w-0"
                onChange={(event) => select(event.target.value)}
              >
                {HOUR_CHOICES.map((hours) => (
                  <NativeSelectOption key={hours} value={`hours-${hours}`}>
                    {hours === 1 ? "Every hour" : `Every ${hours} hours`}
                  </NativeSelectOption>
                ))}
                <NativeSelectOption value="daily">Every day</NativeSelectOption>
                <NativeSelectOption value="weekdays">
                  Every weekday (Mon–Fri)
                </NativeSelectOption>
                <NativeSelectOption value="custom">
                  Custom cron (UTC)
                </NativeSelectOption>
              </NativeSelect>
            </div>
            {choice.kind !== "custom" && (
              <div className="contents text-sm text-muted-foreground">
                <div className="flex max-w-full min-w-0 items-center gap-2">
                  <span>
                    {choice.kind === "hours"
                      ? choice.hours === 1
                        ? "at minute"
                        : "starting"
                      : "at"}
                  </span>
                  {choice.kind === "hours" && choice.hours === 1 ? (
                    <Input
                      aria-label="Minute"
                      type="number"
                      min={0}
                      max={59}
                      className="h-8 w-[64px] max-w-full min-w-0 font-mono"
                      value={choice.time.slice(3)}
                      onChange={(event) =>
                        change({
                          ...choice,
                          time: `00:${event.target.value.padStart(2, "0")}`,
                        })
                      }
                    />
                  ) : (
                    <Input
                      aria-label={
                        choice.kind === "hours" ? "Start time" : "Time"
                      }
                      type="time"
                      className="flex h-8 w-[88px] max-w-full min-w-0 items-center px-1 font-mono [&::-webkit-date-and-time-value]:m-0 [&::-webkit-datetime-edit]:p-0"
                      value={choice.time}
                      onChange={(event) =>
                        change({ ...choice, time: event.target.value })
                      }
                    />
                  )}
                </div>
                <span>UTC</span>
              </div>
            )}
          </div>
          {opened.length > 1 && (
            <p className="text-xs text-warning">
              This schedule has {opened.length} expressions. Choosing a preset
              replaces all {opened.length}.
            </p>
          )}
        </div>
        {choice.kind === "custom" && (
          <CronRows
            id={id}
            crons={choice.crons}
            errors={fieldErrors}
            onChange={(crons) => change({ kind: "custom", crons })}
          />
        )}
      </fieldset>
      <div className="min-w-0 space-y-1 rounded-lg bg-muted p-3 text-sm">
        {summary && <p className="font-medium">{summary}</p>}
        {choice.kind !== "custom" && (
          <p className="text-xs text-muted-foreground">
            Stored as <code>{crons.join("; ") || "—"}</code> UTC.
          </p>
        )}
      </div>
      {alerts.length > 0 && (
        <Alert role="alert" data-refusal tabIndex={-1} variant="destructive">
          {alerts.map((violation, index) => (
            <p key={index}>{violation.description}</p>
          ))}
        </Alert>
      )}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          {isScheduleOverride(current.schedule.requested) && (
            <Button
              type="button"
              variant="link"
              size="sm"
              className="h-auto max-w-full px-0 whitespace-normal"
              disabled={busy}
              onClick={() => void submit(true)}
            >
              {current.schedule.inherited
                ? `Reset to the deployment schedule (${scheduleWords(current.schedule.inherited.crons)})`
                : "Reset to the default schedule"}
            </Button>
          )}
        </div>
        <div className="ml-auto flex gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={onClose}
          >
            Cancel
          </Button>
          <Button
            type="submit"
            size="sm"
            loading={busy}
            disabled={!changed && !retry}
          >
            Save
          </Button>
        </div>
      </div>
    </form>
  );
}
