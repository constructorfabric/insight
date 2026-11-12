import { useEffect, useId, useRef, useState } from "react";

import { ConnectorsApiError } from "@/api/connectors-client";
import type { ScheduleRead } from "@/api/connector-health-client";
import { CronRows } from "./crons";
import { MinuteField } from "./minute";
import { TimeSelect } from "./time";
import { ConnectorScheduleRuns } from "../connector-schedule-runs";
import { rowIndex, violations, type Violation } from "./refusals";
import { saveError } from "./save-error";

import { ConnectorScheduleHeader } from "@/components/portal/connector-schedule-header";
import { ConnectorScheduleTooltip } from "../connector-schedule-tooltip";
import { deploymentScheduleLines } from "@/lib/portal/connector-schedule-display";
import { scheduleClock } from "@/lib/portal/connector-schedule-time";
import { validateScheduleCrons } from "@/lib/portal/connector-schedule-validation";
import { isScheduleOverride } from "@/lib/portal/connector-schedule";
import { useConnectorScheduleAuthor } from "@/queries/connector-schedule-author";
import {
  Alert,
  Button,
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@gears-frontx/ui-kit";
import {
  HOUR_CHOICES,
  choiceCrons,
  detectChoice,
  type ScheduleChoice,
} from "@/lib/portal/connector-schedule-editor";

interface Props {
  read: ScheduleRead;
  save: (etag: string, crons: string[] | null) => Promise<ScheduleRead>;
  reload: () => Promise<ScheduleRead>;
  onClose: () => void;
}

const PRESET_OPTIONS = [
  ...HOUR_CHOICES.map((hours) => ({
    value: `hours-${hours}`,
    label: hours === 1 ? "Every hour" : `Every ${hours} hours`,
  })),
  { value: "daily", label: "Every day" },
  { value: "weekdays", label: "Every weekday (Mon–Fri)" },
  { value: "custom", label: "Custom cron (UTC)" },
];

function storedCrons(read: ScheduleRead): string[] {
  const { requested, live } = read.schedule;
  return isScheduleOverride(requested) ? requested.crons : (live?.crons ?? []);
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
  const [error, setError] = useState<string | { kind: "conflict" }>("");
  const [fieldErrors, setFieldErrors] = useState<Violation[]>([]);
  const [errorDetails, setErrorDetails] = useState("");
  const [busy, setBusy] = useState(false);
  const [validationAttempt, setValidationAttempt] = useState(0);
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
  const clientErrors =
    choice.kind === "custom" ? validateScheduleCrons(crons) : [];
  const canBuild = crons.length > 0 && clientErrors.length === 0;
  const errors = [...clientErrors, ...fieldErrors];
  const changed = JSON.stringify(crons) !== JSON.stringify(opened);
  const alerts = errors.filter((violation) => {
    const index = rowIndex(violation);
    return choice.kind !== "custom" || index === null || index >= crons.length;
  });
  const value = choice.kind === "hours" ? `hours-${choice.hours}` : choice.kind;
  const preset = PRESET_OPTIONS.find((option) => option.value === value);
  const person = useConnectorScheduleAuthor(
    current.schedule.requested?.updated_by
  );
  const author =
    person === "you"
      ? "You"
      : person === "another admin"
        ? "Another admin"
        : person;
  const message =
    typeof error === "string"
      ? error
      : `${author} changed this schedule${person === "you" ? " in another tab" : ""} at ${scheduleClock(current.schedule.requested?.updated_at)} UTC. It is reloaded below; check it and save again.`;

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
    if (!reset) setValidationAttempt((attempt) => attempt + 1);
    if (busy || (!reset && (!canBuild || !changed))) return;
    setBusy(true);
    setError("");
    setErrorDetails("");
    setFieldErrors([]);
    try {
      await save(current.etag, reset ? null : crons);
      onClose();
    } catch (failure) {
      focusRefusal.current = true;
      if (failure instanceof ConnectorsApiError && failure.status === 412) {
        try {
          const next = await reload();
          prefill(next);
          setError({ kind: "conflict" });
        } catch {
          setError(
            "Could not reload the changed schedule. Your input is kept; try again."
          );
        }
      } else if (
        failure instanceof ConnectorsApiError &&
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
      } else if (failure instanceof ConnectorsApiError) {
        const body = failure.body as { detail?: unknown } | null;
        const detail = typeof body?.detail === "string" ? body.detail : "";
        const failureMessage = saveError(detail);
        setError(failureMessage.summary);
        setErrorDetails(failureMessage.details);
        try {
          const next = await reload();
          const running = next.schedule.live?.crons;
          prefill(next);
          if (running) {
            setOpened(running);
            setChoice(detectChoice(running));
          }
        } catch {
          const running = read.schedule.live?.crons ?? opened;
          setCurrent(read);
          setOpened(running);
          setChoice(detectChoice(running));
        }
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
      className="grid min-w-0 grid-cols-1 gap-3 [overflow-wrap:anywhere] whitespace-normal @4xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]"
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
      <div className="@4xl:col-span-2">
        <ConnectorScheduleHeader schedule={current.schedule} />
      </div>
      {message && (
        <Alert
          role="alert"
          data-refusal
          tabIndex={-1}
          variant="destructive"
          className="@4xl:col-span-2"
        >
          {message}
          {errorDetails && (
            <details className="mt-2 text-xs">
              <summary className="cursor-pointer">Details</summary>
              <p className="mt-1">{errorDetails}</p>
            </details>
          )}
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
              <Select
                value={value}
                disabled={busy}
                onValueChange={(next) => {
                  if (typeof next === "string") select(next);
                }}
              >
                <SelectTrigger
                  id={`${id}-runs`}
                  autoFocus
                  size="sm"
                  className="h-8 w-full min-w-0"
                >
                  <SelectValue>{preset?.label}</SelectValue>
                </SelectTrigger>
                <SelectContent align="start">
                  {PRESET_OPTIONS.map((option) => (
                    <SelectItem key={option.value} value={option.value}>
                      {option.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
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
                    <MinuteField
                      key={current.etag}
                      value={choice.time.slice(3)}
                      onChange={(minute) =>
                        change({
                          ...choice,
                          time: minute ? `00:${minute}` : "",
                        })
                      }
                    />
                  ) : (
                    <TimeSelect
                      label={choice.kind === "hours" ? "Start time" : "Time"}
                      disabled={busy}
                      value={choice.time}
                      onChange={(time) => change({ ...choice, time })}
                    />
                  )}
                </div>
                <span>UTC</span>
              </div>
            )}
          </div>
          {opened.length > 1 && (
            <p className="text-xs text-warning">
              Replaces all {opened.length} expressions
            </p>
          )}
        </div>
        {choice.kind === "custom" && (
          <CronRows
            key={current.etag}
            id={id}
            crons={choice.crons}
            clientErrors={clientErrors}
            errors={fieldErrors}
            validationAttempt={validationAttempt}
            onChange={(crons) => change({ kind: "custom", crons })}
          />
        )}
      </fieldset>
      <div className="min-w-0 space-y-1 rounded-lg bg-muted p-3 text-sm">
        {choice.kind !== "custom" && canBuild && (
          <p className="flex min-w-0 flex-wrap items-baseline gap-x-1 text-xs text-muted-foreground">
            <code className="min-w-0">{crons.join("; ")}</code>{" "}
            <span className="shrink-0">UTC</span>
          </p>
        )}
        {!canBuild && choice.kind !== "custom" && (
          <p className="text-destructive">
            Enter a valid schedule before saving.
          </p>
        )}
        <ConnectorScheduleRuns crons={canBuild ? crons : []} />
      </div>
      {alerts.length > 0 && (
        <Alert
          role="alert"
          data-refusal
          tabIndex={-1}
          variant="destructive"
          className="@4xl:col-span-2"
        >
          {alerts.map((violation, index) => (
            <p key={index}>{violation.description}</p>
          ))}
        </Alert>
      )}
      <div className="flex flex-wrap items-center justify-between gap-3 @4xl:col-span-2">
        <div>
          {isScheduleOverride(current.schedule.requested) && (
            <ConnectorScheduleTooltip
              lines={deploymentScheduleLines(current.schedule.inherited)}
            >
              <Button
                type="button"
                variant="link"
                size="sm"
                className="h-auto max-w-full px-0 whitespace-normal"
                disabled={busy}
                onClick={() => void submit(true)}
              >
                Reset to default
              </Button>
            </ConnectorScheduleTooltip>
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
            disabled={!canBuild || !changed}
          >
            Save
          </Button>
        </div>
      </div>
    </form>
  );
}
