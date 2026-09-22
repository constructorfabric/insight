import { createFileRoute, Link, useRouterState } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";

import type { DeclaredField } from "@/api/custom-client";
import { Held } from "@/components/custom/held-by";
import { Button } from "@/components/ui/button";
import {
  ARRIVED,
  RecordTable,
  type Ordering,
} from "@/components/custom/record-table";
import { refusal } from "@/components/custom/refusal";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { useLocalStorageState } from "@/hooks/use-local-storage-state";
import {
  datasetDependentsQuery,
  datasetQuery,
  datasetRecordsQuery,
  PREVIEW_ROWS,
} from "@/queries/custom";
import { TEXT_BODY, TEXT_HEADING, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

export const Route = createFileRoute("/portal/custom/datasets/$name")({
  component: DatasetPage,
});

/** The dataset a path names, read from the path the rail navigated to. */
function datasetNameFromPath(pathname: string): string {
  const match = pathname.match(/^\/portal\/custom\/datasets\/([^/]+)/);

  return match ? decodeURIComponent(match[1]) : "";
}

function DatasetPage() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const name = datasetNameFromPath(pathname);
  const dataset = useQuery(datasetQuery(name));

  if (dataset.isPending) return <CenteredSpinner className="min-h-40" />;

  // A dataset mid-create or mid-removal is not found, so the page says which
  // one rather than asking for its records or what reads it.
  if (dataset.isError) {
    return (
      <div className="flex flex-col gap-2 p-4 md:p-6">
        <h1 className={cn(TEXT_HEADING, "font-mono")}>{name}</h1>
        <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
          {refusal(dataset.error, "That dataset is not there.")}
        </p>
        <Link
          to="/portal/custom/datasets"
          className={cn(
            TEXT_BODY,
            "underline decoration-dotted underline-offset-4"
          )}
        >
          Back to the catalogue
        </Link>
      </div>
    );
  }

  const { declaration } = dataset.data;

  return (
    <div className="flex flex-col gap-4 p-4 md:p-6">
      <header className="flex flex-wrap items-baseline gap-3">
        <div className="flex min-w-0 flex-col gap-1">
          <h1 className={cn(TEXT_HEADING, "font-mono")}>{name}</h1>
          <Link
            to="/portal/custom/datasets"
            className={cn(
              TEXT_BODY,
              "self-start underline decoration-dotted underline-offset-4"
            )}
          >
            Back to the catalogue
          </Link>
        </div>
        <span className={cn(TEXT_BODY, "text-muted-foreground")}>
          {declaration.title}
        </span>
      </header>
      {declaration.description ? (
        <p className={TEXT_BODY}>{declaration.description}</p>
      ) : null}

      <Declaration
        fields={declaration.fields}
        identity={declaration.row_identity}
      />
      <Records
        name={name}
        fields={declaration.fields}
        sent={declaration.source?.kind !== "relation"}
      />
      <Dependents name={name} />
    </div>
  );
}

/** What the records hold, field by field, as the declaration says it. */
function Declaration({
  fields,
  identity,
}: {
  fields: DeclaredField[];
  identity?: string[];
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className={TEXT_HEADING}>Declaration</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <ul className="flex flex-col gap-2">
          {fields.map((field) => (
            <li
              key={field.name}
              className="flex flex-wrap items-baseline gap-2"
            >
              <code className={cn(TEXT_BODY, "font-mono")}>{field.name}</code>
              <Badge variant="secondary">{field.type}</Badge>
              {field.role ? (
                <Badge variant="secondary">{field.role}</Badge>
              ) : null}
              {field.default_clock ? (
                <Badge variant="info">main date</Badge>
              ) : null}
              {field.person ? <Badge variant="info">person</Badge> : null}
              {field.description ? (
                <span className={cn(TEXT_BODY, "text-muted-foreground")}>
                  {field.description}
                </span>
              ) : null}
            </li>
          ))}
        </ul>
        {identity?.length ? (
          <p className={cn(TEXT_LABEL, "text-muted-foreground")}>
            One record per {identity.join(", ")}.
          </p>
        ) : (
          <p className={cn(TEXT_LABEL, "text-muted-foreground")}>
            Every record stands on its own.
          </p>
        )}
      </CardContent>
    </Card>
  );
}

/** How many declared fields a table shows before the reader picks. */
const COLUMNS_AT_FIRST = 10;

function Records({
  name,
  fields,
  sent,
}: {
  name: string;
  fields: readonly DeclaredField[];
  /** Whether records are sent into this dataset, rather than read from a
   * relation the warehouse builds. */
  sent: boolean;
}) {
  const [page, setPage] = useState(0);
  // A relation has no arrival column, so naming one would be naming a field
  // the dataset does not declare. Nothing is named, and the service orders by
  // the dataset's main date.
  const [ordering, setOrdering] = useState<Ordering>({
    by: sent ? ARRIVED : "",
    descending: true,
  });
  const [shown, setShown] = useLocalStorageState<string[]>({
    key: `insight.custom.dataset.${name}.columns`,
    defaultValue: fields.slice(0, COLUMNS_AT_FIRST).map((field) => field.name),
    parse: storedColumns,
    serialize: (value) => JSON.stringify(value),
  });

  // How wide a page is belongs to the service: its cap is an installation's
  // setting. Stepping by a number of our own would walk past whatever a
  // narrower page left behind. Only the first read, before any page has come
  // back, goes by the assumption.
  const [stride, setStride] = useState(PREVIEW_ROWS);

  const records = useQuery(
    datasetRecordsQuery(name, {
      offset: page * stride,
      orderBy: ordering.by || undefined,
      descending: ordering.descending,
    })
  );

  const waiting = records.isPlaceholderData;
  const held = records.data?.limit;
  if (held !== undefined && held !== stride) setStride(held);

  const order = (next: Ordering) => {
    setOrdering(next);
    setPage(0);
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle className={TEXT_HEADING}>Records</CardTitle>
      </CardHeader>
      <CardContent>
        {records.isPending ? (
          <CenteredSpinner className="min-h-24" />
        ) : records.isError ? (
          <div className="flex flex-col items-start gap-3">
            <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
              {refusal(records.error, "Couldn't read the records.")}
            </p>
            {page > 0 ? (
              <Button variant="ghost" size="sm" onClick={() => setPage(0)}>
                Back to the first page
              </Button>
            ) : null}
          </div>
        ) : records.data.total === 0 ? (
          <p className={cn(TEXT_BODY, "text-muted-foreground")}>
            {sent ? "Nothing has arrived yet." : "The relation holds no rows."}
          </p>
        ) : (
          <div
            className={cn("flex flex-col gap-3", waiting && "opacity-60")}
            aria-busy={waiting || undefined}
          >
            <RecordTable
              fields={fields}
              records={records.data.records}
              shown={shown}
              ordering={ordering}
              onShow={setShown}
              onOrder={order}
            />
            <Paging
              page={page}
              sent={sent}
              stride={stride}
              held={records.data.records.length}
              total={records.data.total}
              waiting={waiting}
              onPage={setPage}
            />
          </div>
        )}
      </CardContent>
    </Card>
  );
}

/** Which records of the whole this page is, and the way to the others. */
function Paging({
  page,
  sent,
  stride,
  held,
  total,
  waiting,
  onPage,
}: {
  page: number;
  /** Whether these rows were sent in, or read from a relation. */
  sent: boolean;
  /** The page size the service applied, which it reports with the page. */
  stride: number;
  held: number;
  total: number;
  /** The rows on screen are the page before this one, still.  */
  waiting: boolean;
  onPage: (page: number) => void;
}) {
  const first = page * stride + 1;
  const last = page * stride + held;

  return (
    <div className="flex flex-wrap items-center gap-2">
      <span
        aria-live="polite"
        className={cn(TEXT_LABEL, "text-muted-foreground")}
      >
        {waiting
          ? "Reading…"
          : held === 0
            ? `Nothing left past record ${first - 1} of ${total}`
            : `${first}–${last} of ${total} ${sent ? "records received" : "rows"}`}
      </span>
      <span className="ms-auto flex items-center gap-1">
        <Button
          variant="ghost"
          size="sm"
          disabled={waiting || page === 0}
          onClick={() => onPage(page - 1)}
        >
          Previous
        </Button>
        <Button
          variant="ghost"
          size="sm"
          disabled={waiting || held === 0 || last >= total}
          onClick={() => onPage(page + 1)}
        >
          Next
        </Button>
      </span>
    </div>
  );
}

/**
 * The columns a reader last picked, as they were stored.
 *
 * SAFETY: a stored value is whatever is in the browser, and asserting a shape
 * it never checked took the page down with a TypeError nothing in the UI
 * could clear. Anything unrecognised falls back to the default.
 */
function storedColumns(raw: string): string[] | undefined {
  const held: unknown = JSON.parse(raw);
  if (!Array.isArray(held)) return undefined;
  if (!held.every((name) => typeof name === "string")) return undefined;

  return held;
}

/** Every metric that reads this dataset, which a removal would break. */
function Dependents({ name }: { name: string }) {
  const dependents = useQuery(datasetDependentsQuery(name));

  return (
    <Held
      title="Metrics reading it"
      empty="Nothing reads it, so it can be taken away."
      pending={dependents.isPending}
      error={dependents.error}
      holders={dependents.data?.map((metric) => ({
        kind: "metrics" as const,
        name: metric,
      }))}
    />
  );
}
