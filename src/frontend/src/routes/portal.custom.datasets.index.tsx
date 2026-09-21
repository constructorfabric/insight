import { createFileRoute } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";

import {
  DefinitionCard,
  DefinitionList,
} from "@/components/custom/definition-list";
import { DatasetSummary } from "@/components/custom/definition-summary";
import {
  EditLink,
  NewLink,
  PreviewLink,
} from "@/components/custom/editor/edit-link";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { useDefinitionCatalogue } from "@/hooks/use-definition-catalogue";
import { datasetQuery } from "@/queries/custom";
import { TEXT_BODY } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

// An index route beside `$name`, not its parent: a parent would have to draw
// the page through an Outlet, and the catalogue is a page of its own.
export const Route = createFileRoute("/portal/custom/datasets/")({
  component: DatasetsCatalogue,
});

function DatasetsCatalogue() {
  const catalogue = useDefinitionCatalogue("datasets");

  return (
    <DefinitionList
      title="Datasets"
      blurb="Every declared dataset. A metric reads one of these, and records are sent into them."
      names={catalogue.names}
      isLoading={catalogue.isLoading}
      isError={catalogue.isError}
      onRetry={catalogue.refetch}
      search={{ label: "Search datasets", ...catalogue.search }}
      paging={catalogue.paging}
      create={<NewLink kind="datasets" noun="dataset" />}
      emptyLabel="No datasets yet. Declare one to send records into it."
      renderRow={(name) => <DatasetRow name={name} />}
    />
  );
}

function DatasetRow({ name }: { name: string }) {
  const { data, isPending, isError, error } = useQuery(datasetQuery(name));

  return (
    <DefinitionCard
      name={name}
      actions={
        <>
          <EditLink kind="datasets" name={name} />
          <PreviewLink kind="datasets" name={name} />
        </>
      }
    >
      {isPending ? (
        <CenteredSpinner className="min-h-24" />
      ) : isError ? (
        <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
          {(error as Error).message}
        </p>
      ) : (
        <DatasetSummary declaration={data.declaration} />
      )}
    </DefinitionCard>
  );
}
