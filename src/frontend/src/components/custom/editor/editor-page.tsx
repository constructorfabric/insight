import { Link, useNavigate } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";

import type { EditableKind } from "@/api/custom-client";
import { DefinitionEditor } from "@/components/custom/editor/definition-editor";
import { refusal } from "@/components/custom/refusal";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { DESCRIPTIONS } from "@/lib/custom/editor/kinds";
import { definitionBodyQuery } from "@/queries/custom";
import { TEXT_BODY, TEXT_HEADING } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

/** Where a kind's catalogue lives, which is where a save returns to. */
const CATALOGUE: Record<EditableKind, string> = {
  datasets: "/portal/custom/datasets",
  metrics: "/portal/custom/metrics",
  widgets: "/portal/custom/widgets",
  dashboards: "/portal/custom",
};

/** A definition being written: an existing one, or one that does not exist. */
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

  const done = () => void navigate({ to: CATALOGUE[kind] });

  return (
    <div className="flex flex-col gap-4 p-4 md:p-6">
      <div className="flex flex-col gap-1">
        <h1 className={cn(TEXT_HEADING)}>
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

      {name !== undefined && stored.isPending ? (
        <CenteredSpinner className="min-h-40" />
      ) : name !== undefined && stored.isError ? (
        <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
          {refusal(stored.error, `That ${description.noun} is not there.`)}
        </p>
      ) : (
        <DefinitionEditor
          kind={kind}
          name={name}
          document={stored.data}
          onStored={done}
        />
      )}
    </div>
  );
}
