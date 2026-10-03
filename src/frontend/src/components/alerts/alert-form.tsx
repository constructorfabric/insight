import { Link, useNavigate } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useState, type ReactNode } from "react";

import type { Alert } from "@/api/alerts-client";
import { AlertFields } from "@/components/alerts/alert-fields";
import { savingRefusal } from "@/components/alerts/field-refusals";
import { refusal } from "@/components/custom/refusal";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { isRevisionConflict } from "@/lib/alerts/conflict";
import {
  blankForm,
  checkForm,
  formOf,
  type AlertField,
  type AlertForm as Form,
  type FieldErrors,
} from "@/lib/alerts/draft";
import { TEXT_BODY, TEXT_LABEL, TEXT_TITLE } from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import {
  alertDestinationsQuery,
  alertQuery,
  useCreateAlert,
  useReplaceAlert,
} from "@/queries/alerts";

/** The page a new alert is written on. */
export function NewAlertPage() {
  const destinations = useQuery(alertDestinationsQuery());

  if (destinations.isPending) return <CenteredSpinner className="min-h-40" />;
  if (destinations.isError) {
    return (
      <Shell title="New alert">
        <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
          {refusal(destinations.error, "Couldn't load destinations.")}
        </p>
      </Shell>
    );
  }

  const only = destinations.data.length === 1 ? destinations.data[0].name : "";

  return (
    <Shell title="New alert">
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

  if (alert.isPending) return <CenteredSpinner className="min-h-40" />;
  if (alert.isError) {
    return (
      <Shell title="Edit alert">
        <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
          {refusal(alert.error, "Alert not found.")}
        </p>
      </Shell>
    );
  }

  return (
    <Shell title={`Edit ${alert.data.name}`} backTo={id}>
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
  title,
  backTo,
  children,
}: {
  title: string;
  /** The alert the form edits, which Cancel returns to. */
  backTo?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex max-w-3xl flex-col gap-4 p-4 md:p-6">
      <header className="flex flex-col gap-1">
        <h1 className={cn(TEXT_TITLE, "break-words")}>{title}</h1>
        <Cancel id={backTo} />
      </header>
      {children}
    </div>
  );
}

/** The control each field's message is shown beside, in the order a reader meets them. */
const FIELD_CONTROLS: readonly [AlertField, string][] = [
  ["name", "alert-name"],
  ["metric", "alert-metric"],
  ["column", "alert-column"],
  ["range", "alert-range"],
  ["threshold", "alert-threshold"],
  ["interval_secs", "alert-interval"],
  ["destination", "alert-destination"],
];

/** Takes the reader to the first field that needs them. */
function focusFirst(errors: FieldErrors) {
  const first = FIELD_CONTROLS.find(([field]) => errors[field]);
  if (first) document.getElementById(first[1])?.focus();
}

/**
 * The form, and what saving it does.
 *
 * A new alert is created; an existing one is replaced at the revision the form
 * was opened at, so a change made elsewhere in the meantime is refused rather
 * than overwritten.
 */
function AlertEditor({
  initial,
  existing,
}: {
  initial?: Form;
  existing?: Alert;
}) {
  const [form, setForm] = useState<Form>(() =>
    existing ? formOf(existing) : (initial ?? blankForm())
  );
  // INVARIANT: a save expects the revision the form was filled from; keying the form on the cache's revision remounts it mid-save and the navigation is lost.
  const [base, setBase] = useState<Alert | undefined>(existing);
  const [errors, setErrors] = useState<FieldErrors>({});
  const destinations = useQuery(alertDestinationsQuery());
  const create = useCreateAlert();
  const replace = useReplaceAlert();
  const latest = useQuery({
    ...alertQuery(existing?.id ?? ""),
    enabled: false,
  });
  const navigate = useNavigate();

  const saving = create.isPending || replace.isPending;
  const failure = base ? replace.error : create.error;
  const conflict = isRevisionConflict(failure);
  const general =
    failure && !conflict ? savingRefusal(failure).general : undefined;

  function save() {
    const checked = checkForm(form);
    if (!checked.ok) {
      setErrors(checked.errors);
      focusFirst(checked.errors);
      return;
    }
    setErrors({});

    const saved = (alert: Alert) =>
      void navigate({
        to: "/portal/custom/alerts/$id",
        params: { id: alert.id },
      });
    const refused = (error: unknown) => {
      const { fields } = savingRefusal(error);
      setErrors(fields);
      focusFirst(fields);
    };

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

      <AlertFields
        form={form}
        errors={errors}
        destinations={destinations.data ?? []}
        onChange={setForm}
      />

      {conflict ? (
        <div role="alert" className="flex flex-wrap items-center gap-3">
          <p className={cn(TEXT_BODY, "text-destructive")}>
            This alert was changed by someone else.
          </p>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={latest.isFetching}
            onClick={() =>
              void latest.refetch().then(({ data }) => {
                if (!data) return;
                setBase(data);
                setForm(formOf(data));
                setErrors({});
                replace.reset();
              })
            }
          >
            Load latest version
          </Button>
        </div>
      ) : general ? (
        <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
          {general}
        </p>
      ) : null}

      <div className="flex items-center gap-3">
        <Button type="submit" size="sm" disabled={saving}>
          {saving ? <Spinner className="size-3" /> : null}
          {base ? "Save changes" : "Create alert"}
        </Button>
        {Object.keys(errors).length > 0 ? (
          <span className={cn(TEXT_LABEL, "text-destructive")}>
            Fix the highlighted fields.
          </span>
        ) : null}
      </div>
    </form>
  );
}
