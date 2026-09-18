import { Link, useNavigate } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";

import type { EditableKind } from "@/api/custom-client";
import { DefinitionEditor } from "@/components/custom/editor/definition-editor";
import { refusal } from "@/components/custom/refusal";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { DESCRIPTIONS } from "@/lib/custom/editor/kinds";
import { definitionBodyQuery } from "@/queries/custom";
import { TEXT_BODY, TEXT_TITLE } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

const CATALOGUE: Record<EditableKind, string> = {
  datasets: "/portal/custom/datasets",
  metrics: "/portal/custom/metrics",
  widgets: "/portal/custom/widgets",
  dashboards: "/portal/custom",
};

export function EditorPage({
  kind,
  name,
}: {
  kind: EditableKind;
  /** Absent for a definition that is being created. */
  name?: string;
}) {
  const navigate = useNavigate();
  const description = DESCRIPTIONS[kind];
  const stored = useQuery({
    ...definitionBodyQuery(kind, name ?? ""),
    enabled: name !== undefined,
  });
  // INVARIANT: the editor seeds its state once, from the document it is
  // handed. It is handed what this page fetched now, never what the cache
  // held from the last time this definition was opened, or a save would
  // overwrite whatever changed it since.
  const fresh = stored.isFetchedAfterMount && stored.isSuccess;

  const done = () => void navigate({ to: CATALOGUE[kind] });

  return (
    <div className="flex flex-col gap-4 p-4 md:p-6">
      <div className="flex flex-col gap-1">
        <h1 className={TEXT_TITLE}>
          {name === undefined ? `New ${description.noun}` : name}
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

      {name === undefined ? (
        <DefinitionEditor kind={kind} onStored={done} />
      ) : stored.isError ? (
        <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
          {refusal(stored.error, `That ${description.noun} is not there.`)}
        </p>
      ) : fresh ? (
        <DefinitionEditor
          kind={kind}
          name={name}
          document={stored.data}
          onStored={done}
        />
      ) : (
        <CenteredSpinner className="min-h-40" />
      )}
    </div>
  );
}
