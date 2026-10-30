import { Link, useNavigate } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";

import type { EditableKind } from "@/api/custom-client";
import { DefinitionEditor } from "@/components/custom/editor/definition-editor";
import { refusal } from "@/components/custom/refusal";
import { RemoveDataset } from "@/components/custom/remove-dataset";
import { RemoveDefinition } from "@/components/custom/remove-definition";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { dashboardAllows } from "@/lib/custom/dashboard-access";
import { NOUNS } from "@/lib/custom/editor/kinds";
import { definitionBodyQuery } from "@/queries/custom";
import { TEXT_BODY, TEXT_HEADING, TEXT_TITLE } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

const CATALOGUE: Record<EditableKind, string> = {
  datasets: "/portal/custom/datasets",
  metrics: "/portal/custom/metrics",
  widgets: "/portal/custom/widgets",
  dashboards: "/portal/custom",
};

const PAGE: Record<EditableKind, string> = {
  datasets: "/portal/custom/datasets/$name",
  metrics: "/portal/custom/metrics/$name",
  widgets: "/portal/custom/widgets/$name",
  dashboards: "/portal/custom/$name",
};

export function EditorPage({
  kind,
  id,
}: {
  kind: EditableKind;
  /** Absent for a definition that is being created. */
  id?: string;
}) {
  const navigate = useNavigate();
  const noun = NOUNS[kind];
  const stored = useQuery({
    ...definitionBodyQuery(kind, id ?? ""),
    enabled: id !== undefined,
  });
  // INVARIANT: the editor seeds its state once, from the document it is
  // handed. It is handed what this page fetched now, never what the cache
  // held from the last time this definition was opened, or a save would
  // overwrite whatever changed it since.
  const fresh =
    stored.isFetchedAfterMount && stored.isSuccess ? stored.data : undefined;

  const done = () => void navigate({ to: CATALOGUE[kind] });
  const created = (made: string) =>
    void navigate({ to: PAGE[kind], params: { name: made } });

  return (
    <div
      data-testid={`page-custom-${id === undefined ? "new" : "edit"}-${noun}`}
      className="flex flex-col gap-4 p-4 md:p-6"
    >
      <div className="flex flex-col gap-1">
        <h1 className={TEXT_TITLE}>
          {id === undefined ? `New ${noun}` : (stored.data?.name ?? id)}
        </h1>
        <Link
          to={CATALOGUE[kind]}
          className={cn(
            TEXT_BODY,
            "self-start underline decoration-dotted underline-offset-4"
          )}
        >
          Back to the catalogue
        </Link>
      </div>

      {id === undefined ? (
        <DefinitionEditor kind={kind} onStored={created} />
      ) : stored.isError ? (
        <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
          {refusal(stored.error, `That ${noun} is not there.`)}
        </p>
      ) : fresh?.access && !dashboardAllows(fresh.access).edit ? (
        <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
          Only the owner or an editor can change this dashboard.
        </p>
      ) : fresh ? (
        <>
          <DefinitionEditor
            kind={kind}
            id={id}
            name={fresh.name}
            document={fresh.document}
            onStored={done}
          />
          {!fresh.access || dashboardAllows(fresh.access).remove ? (
            <Removal kind={kind} id={id} name={fresh.name} onRemoved={done} />
          ) : null}
        </>
      ) : (
        <CenteredSpinner className="min-h-40" />
      )}
    </div>
  );
}

function Removal({
  kind,
  id,
  name,
  onRemoved,
}: {
  kind: EditableKind;
  id: string;
  name: string;
  onRemoved: () => void;
}) {
  return (
    <section className="mt-6 flex flex-col gap-2 border-t border-border pt-4">
      <h2 className={TEXT_HEADING}>Remove</h2>
      <p className={cn(TEXT_BODY, "text-muted-foreground")}>
        {kind === "datasets"
          ? "Takes the dataset away with every record it holds. Refused while a metric reads it."
          : "Refused while something else still draws it."}
      </p>
      <div className="self-start">
        {kind === "datasets" ? (
          <RemoveDataset name={id} onRemoved={onRemoved} />
        ) : (
          <RemoveDefinition
            kind={kind}
            id={id}
            name={name}
            onRemoved={onRemoved}
          />
        )}
      </div>
    </section>
  );
}
