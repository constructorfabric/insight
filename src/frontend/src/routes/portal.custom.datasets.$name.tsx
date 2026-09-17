import {
  createFileRoute,
  Link,
  useNavigate,
  useRouterState,
} from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";

import type { DatasetRecord, DeclaredField } from "@/api/custom-client";
import { refusal } from "@/components/custom/refusal";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Spinner } from "@/components/ui/spinner";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import {
  datasetDependentsQuery,
  datasetQuery,
  datasetRecordsQuery,
  useRemoveDataset,
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
      <header className="flex flex-wrap items-center gap-3">
        <h1 className={cn(TEXT_HEADING, "font-mono")}>{name}</h1>
        <span className={cn(TEXT_BODY, "text-muted-foreground")}>
          {declaration.title}
        </span>
        <span className="ms-auto">
          <RemoveDataset name={name} />
        </span>
      </header>
      {declaration.description ? (
        <p className={TEXT_BODY}>{declaration.description}</p>
      ) : null}

      <Declaration
        fields={declaration.fields}
        identity={declaration.row_identity}
      />
      <Records name={name} />
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

/** The latest records, as they arrived. */
function Records({ name }: { name: string }) {
  const records = useQuery(datasetRecordsQuery(name));

  return (
    <Card>
      <CardHeader>
        <CardTitle className={TEXT_HEADING}>Latest records</CardTitle>
      </CardHeader>
      <CardContent>
        {records.isPending ? (
          <CenteredSpinner className="min-h-24" />
        ) : records.isError ? (
          <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
            {refusal(records.error, "Couldn't read the records.")}
          </p>
        ) : records.data.length === 0 ? (
          <p className={cn(TEXT_BODY, "text-muted-foreground")}>
            Nothing has arrived yet.
          </p>
        ) : (
          <ul className="flex flex-col gap-2">
            {records.data.map((record) => (
              <RecordRow key={record.id} record={record} />
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}

function RecordRow({ record }: { record: DatasetRecord }) {
  return (
    <li className="flex flex-col gap-1">
      <span className={cn(TEXT_LABEL, "text-muted-foreground")}>
        {record.received_at}
      </span>
      <pre className={cn(TEXT_BODY, "overflow-x-auto font-mono")}>
        {JSON.stringify(record.raw_data)}
      </pre>
    </li>
  );
}

/** Every metric that reads this dataset, which a removal would break. */
function Dependents({ name }: { name: string }) {
  const dependents = useQuery(datasetDependentsQuery(name));

  return (
    <Card>
      <CardHeader>
        <CardTitle className={TEXT_HEADING}>Metrics reading it</CardTitle>
      </CardHeader>
      <CardContent>
        {dependents.isPending ? (
          <CenteredSpinner className="min-h-24" />
        ) : dependents.isError ? (
          <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
            {refusal(dependents.error, "Couldn't read what reads it.")}
          </p>
        ) : dependents.data.length === 0 ? (
          <p className={cn(TEXT_BODY, "text-muted-foreground")}>
            Nothing reads it, so it can be taken away.
          </p>
        ) : (
          <ul className="flex flex-col gap-1">
            {dependents.data.map((metric) => (
              <li key={metric}>
                <code className={cn(TEXT_BODY, "font-mono")}>{metric}</code>
              </li>
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}

/**
 * Takes the dataset away, with the records it holds, in two clicks.
 *
 * The service refuses while a metric reads it and names every one, so the
 * refusal is shown as it came back — the list above says the same thing.
 */
function RemoveDataset({ name }: { name: string }) {
  const [asked, setAsked] = useState(false);
  const navigate = useNavigate();
  const remove = useRemoveDataset();

  if (remove.isError) {
    return (
      <span role="alert" className={cn(TEXT_LABEL, "text-destructive")}>
        {refusal(remove.error, "Couldn't remove it.")}
      </span>
    );
  }

  if (!asked) {
    return (
      <Button variant="ghost" size="sm" onClick={() => setAsked(true)}>
        Remove
      </Button>
    );
  }

  return (
    <span className="flex items-center gap-1">
      <Button
        variant="ghost"
        size="sm"
        className="text-destructive"
        disabled={remove.isPending}
        onClick={() =>
          remove.mutate(name, {
            onSuccess: () => void navigate({ to: "/portal/custom/datasets" }),
          })
        }
      >
        {remove.isPending ? <Spinner className="size-3" /> : null}
        Remove it, with its records
      </Button>
      <Button variant="ghost" size="sm" onClick={() => setAsked(false)}>
        Keep
      </Button>
    </span>
  );
}
