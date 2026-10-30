import { Link, useNavigate } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";

import type { Alert, Condition } from "@/api/alerts-client";
import { AlertFields } from "@/components/alerts/alert-fields";
import { savingRefusal } from "@/components/alerts/field-refusals";
import { useAlertMetric } from "@/components/alerts/use-alert-metric";
import { useAlertPreview } from "@/components/alerts/use-alert-preview";
import { TextView } from "@/components/custom/editor/text-view";
import { refusal } from "@/components/custom/refusal";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { isRevisionConflict } from "@/lib/alerts/conflict";
import { fitCondition } from "@/lib/alerts/condition";
import { windowFor } from "@/lib/alerts/window";
import {
  blankForm,
  checkForm,
  contextColumnsOf,
  formOf,
  retypeDraft,
  textOf,
  type AlertDocument,
  type FieldErrors,
} from "@/lib/alerts/draft";
import {
  change,
  hold,
  sendable,
  type Held,
} from "@/lib/custom/editor/document";
import { TEXT_BODY, TEXT_LABEL, TEXT_TITLE } from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import {
  alertDestinationsQuery,
  alertQuery,
  useCreateAlert,
  useReadAlert,
  useReplaceAlert,
} from "@/queries/alerts";

/** The page a new alert is written on. */
export function NewAlertPage() {
  const destinations = useQuery(alertDestinationsQuery());

  if (destinations.isPending)
    return (
      <CenteredSpinner
        data-testid="page-custom-alert-new"
        className="min-h-40"
      />
    );
  if (destinations.isError) {
    return (
      <Shell data-testid="page-custom-alert-new" title="New alert">
        <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
          {refusal(destinations.error, "Couldn't load destinations.")}
        </p>
      </Shell>
    );
  }

  const only = destinations.data.length === 1 ? destinations.data[0].name : "";

  return (
    <Shell data-testid="page-custom-alert-new" title="New alert">
      <AlertEditor initial={blankForm(only)} />
    </Shell>
  );
}

/** The page an existing alert is changed on. */
export function EditAlertPage({ id }: { id: string }) {
  const alert = useQuery({
    ...alertQuery(id),
    // Seeded once; a refresh underneath the form would be neither shown nor wanted.
    refetchInterval: false,
    refetchOnWindowFocus: false,
  });

  if (alert.isError) {
    return (
      <Shell data-testid="page-custom-alert-edit" title="Edit alert">
        <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
          {refusal(alert.error, "Alert not found.")}
        </p>
      </Shell>
    );
  }
  // INVARIANT: the form seeds once, from what this page read now, never from
  // the copy the alert page cached, or a save would overwrite what changed since.
  if (!alert.isFetchedAfterMount || !alert.isSuccess) {
    return (
      <CenteredSpinner
        data-testid="page-custom-alert-edit"
        className="min-h-40"
      />
    );
  }

  return (
    <Shell
      data-testid="page-custom-alert-edit"
      title={`Edit ${alert.data.name}`}
      backTo={id}
    >
      <AlertEditor key={id} existing={alert.data} />
    </Shell>
  );
}

const CANCEL = cn(
  TEXT_BODY,
  "self-start underline decoration-dotted underline-offset-4"
);

/** Leaves the form for the list, or for the alert it was editing. */
function Cancel({ id }: { id?: string }) {
  return id ? (
    <Link to="/portal/custom/alerts/$id" params={{ id }} className={CANCEL}>
      Cancel
    </Link>
  ) : (
    <Link to="/portal/custom/alerts" className={CANCEL}>
      Cancel
    </Link>
  );
}

function Shell({
  "data-testid": testId,
  title,
  backTo,
  children,
}: {
  "data-testid": string;
  title: string;
  /** The alert the form edits, which Cancel returns to. */
  backTo?: string;
  children: ReactNode;
}) {
  return (
    <div
      data-testid={testId}
      className="flex max-w-3xl flex-col gap-4 p-4 md:p-6"
    >
      <header className="flex flex-col gap-1">
        <h1 className={cn(TEXT_TITLE, "break-words")}>{title}</h1>
        <Cancel id={backTo} />
      </header>
      {children}
    </div>
  );
}

const PLACED: ReadonlySet<string> = new Set([
  "name",
  "metric",
  "context_columns",
  "range",
  "interval_secs",
  "destination",
]);

function placedInEditor(field: string): boolean {
  return (
    PLACED.has(field) || field === "condition" || field.startsWith("condition.")
  );
}

function aboutCondition(field: string): boolean {
  return field === "condition" || field.startsWith("condition.");
}

function withoutCondition(errors: FieldErrors): FieldErrors {
  return Object.fromEntries(
    Object.entries(errors).filter(([field]) => !aboutCondition(field))
  );
}

// INVARIANT: the draft is one document shown two ways, and whichever view was
// edited last is believed.
function AlertEditor({
  initial,
  existing,
}: {
  initial?: AlertDocument;
  existing?: Alert;
}) {
  const [held, setHeld] = useState<Held>(() =>
    hold(existing ? formOf(existing) : (initial ?? blankForm()))
  );
  const [view, setView] = useState<"fields" | "text">("fields");
  // INVARIANT: a save expects the revision the form was filled from; keying
  // the form on the cache's revision remounts it mid-save and the navigation
  // is lost.
  const [base, setBase] = useState<Alert | undefined>(existing);
  const [errors, setErrors] = useState<FieldErrors>({});
  const [focusAsked, setFocusAsked] = useState(0);
  const formRef = useRef<HTMLFormElement>(null);
  const destinations = useQuery(alertDestinationsQuery());
  const metricName = textOf(held.document, "metric");
  const metric = useAlertMetric(metricName);
  const [refitFor, setRefitFor] = useState<string | undefined>();
  const conditionText = JSON.stringify(held.document.condition) ?? "";
  const [seenCondition, setSeenCondition] = useState(conditionText);
  const create = useCreateAlert();
  const replace = useReplaceAlert();
  const reload = useReadAlert();
  const navigate = useNavigate();

  const saving = create.isPending || replace.isPending;
  const failure = base ? replace.error : create.error;
  const conflict = isRevisionConflict(failure);
  const general =
    failure && !conflict ? savingRefusal(failure).general : undefined;
  const checked = checkForm(held.document, metric.kinds);
  const ready =
    metricName !== "" &&
    metric.typed &&
    (checked.ok || !Object.keys(checked.errors).some(aboutCondition));
  const range = textOf(held.document, "range");
  const preview = useAlertPreview(
    {
      metric: metricName,
      range: range === "" ? null : range,
      condition: held.document.condition as Condition,
      context_columns: contextColumnsOf(held.document) ?? [],
    },
    ready
  );
  const previewRefusal = preview.query.isError
    ? savingRefusal(preview.query.error).fields
    : {};
  const shown: FieldErrors = { ...previewRefusal, ...errors };
  const loose = Object.entries(shown).filter(
    ([field]) => !placedInEditor(field)
  );

  if (refitFor !== undefined && refitFor === metricName && metric.typed) {
    setRefitFor(undefined);
    setHeld((was) => {
      const window = windowFor(metric.stored, textOf(was.document, "range"));
      const fitted = change(
        was,
        ["condition"],
        fitCondition(was.document.condition, metric.kinds)
      );
      const cleared = change(fitted, ["context_columns"], []);
      return change(cleared, ["range"], window === "" ? null : window);
    });
  }
  if (seenCondition !== conditionText) {
    setSeenCondition(conditionText);
    setErrors(withoutCondition(errors));
  }

  useEffect(() => {
    if (focusAsked === 0) return;
    formRef.current
      ?.querySelector<HTMLElement>('[aria-invalid="true"]')
      ?.focus();
  }, [focusAsked]);

  const edit = useCallback((patch: AlertDocument) => {
    if (typeof patch.metric === "string" && patch.metric !== "") {
      setRefitFor(patch.metric);
    }
    setHeld((was) =>
      Object.entries(patch).reduce(
        (held, [key, value]) => change(held, [key], value),
        was
      )
    );
  }, []);

  function refuse(found: FieldErrors) {
    setErrors(found);
    setFocusAsked((count) => count + 1);
  }

  function save() {
    if (!sendable(held)) return;
    if (!checked.ok) {
      refuse(checked.errors);
      return;
    }
    setErrors({});
    reload.reset();

    const saved = (alert: Alert) =>
      void navigate({
        to: "/portal/custom/alerts/$id",
        params: { id: alert.id },
      });
    const refused = (error: unknown) => refuse(savingRefusal(error).fields);

    if (base) {
      replace.mutate(
        {
          id: base.id,
          draft: { ...checked.draft, expected_revision: base.revision },
        },
        { onSuccess: saved, onError: refused }
      );
      return;
    }
    create.mutate(checked.draft, { onSuccess: saved, onError: refused });
  }

  return (
    <form
      ref={formRef}
      noValidate
      className="flex flex-col gap-6"
      onSubmit={(event) => {
        event.preventDefault();
        save();
      }}
    >
      {destinations.data?.length === 0 ? (
        <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
          No destinations are configured.
        </p>
      ) : null}

      <div className="flex items-center gap-2">
        {(["fields", "text"] as const).map((each) => (
          <Button
            key={each}
            type="button"
            variant={view === each ? "secondary" : "ghost"}
            size="sm"
            aria-pressed={view === each}
            onClick={() => setView(each)}
          >
            {each === "fields" ? "Editor" : "JSON"}
          </Button>
        ))}
      </div>

      {view === "fields" ? (
        <>
          {loose.map(([field, said]) => (
            <p
              key={field}
              role="alert"
              className={cn(TEXT_BODY, "text-destructive")}
            >
              <span className="font-mono">{field}</span>: {said}
            </p>
          ))}
          <AlertFields
            document={held.document}
            errors={shown}
            destinations={destinations.data ?? []}
            metric={metric}
            preview={preview}
            onChange={edit}
          />
        </>
      ) : (
        <div className="flex flex-col gap-1">
          <TextView
            id="alert-text"
            label="Alert as JSON"
            text={held.text}
            unparsed={held.unparsed}
            onChange={(text) => setHeld((was) => retypeDraft(was, text))}
          />
          {Object.keys(shown).length === 0 ? null : (
            <ul
              className={cn(
                TEXT_LABEL,
                "flex flex-col gap-0.5 text-destructive"
              )}
            >
              {Object.entries(shown).map(([field, said]) => (
                <li key={field} role="alert">
                  <span className="font-mono">{field}</span>: {said}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      {conflict && base ? (
        <div role="alert" className="flex flex-wrap items-center gap-3">
          <p className={cn(TEXT_BODY, "text-destructive")}>
            This alert was changed by someone else.
          </p>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={reload.isPending}
            onClick={() =>
              reload.mutate(base.id, {
                onSuccess: (data) => {
                  setBase(data);
                  setHeld(hold(formOf(data)));
                  setErrors({});
                  replace.reset();
                },
              })
            }
          >
            Discard my edits and load the latest version
          </Button>
          {reload.isError ? (
            <p className={cn(TEXT_BODY, "text-destructive")}>
              {refusal(reload.error, "Couldn't load the latest version.")}
            </p>
          ) : null}
        </div>
      ) : general ? (
        <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
          {general}
        </p>
      ) : null}

      <div className="flex items-center gap-3">
        <Button type="submit" size="sm" disabled={saving || !sendable(held)}>
          {saving ? <Spinner className="size-3" /> : null}
          {base ? "Save changes" : "Create alert"}
        </Button>
        {held.unparsed ? (
          <span className={cn(TEXT_LABEL, "text-destructive")}>
            Fix the JSON to save.
          </span>
        ) : Object.keys(errors).length > 0 ? (
          <span className={cn(TEXT_LABEL, "text-destructive")}>
            Fix the highlighted fields.
          </span>
        ) : null}
      </div>
    </form>
  );
}
