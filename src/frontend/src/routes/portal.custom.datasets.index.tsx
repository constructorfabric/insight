import { createFileRoute } from "@tanstack/react-router";
import { Database } from "lucide-react";

import type { DatasetListed } from "@/api/custom-client";
import { CatalogueCard } from "@/components/custom/catalogue-card";
import { DefinitionList } from "@/components/custom/definition-list";
import { NewLink } from "@/components/custom/editor/edit-link";
import { useDefinitionCatalogue } from "@/hooks/use-definition-catalogue";

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
      items={catalogue.items}
      isLoading={catalogue.isLoading}
      isError={catalogue.isError}
      onRetry={catalogue.refetch}
      search={{ label: "Search datasets", ...catalogue.search }}
      paging={catalogue.paging}
      create={<NewLink kind="datasets" noun="dataset" />}
      emptyLabel="No datasets yet. Declare one to send records into it."
      renderRow={(item) => <DatasetCard item={item} />}
    />
  );
}

function DatasetCard({ item }: { item: DatasetListed }) {
  const titled = item.title && item.title !== item.name;
  const fields = `${item.fields} ${item.fields === 1 ? "field" : "fields"}`;

  return (
    <CatalogueCard
      kind="datasets"
      id={item.name}
      name={item.name}
      title={item.title || item.name}
      icon={Database}
      facts={titled ? [item.name, fields] : [fields]}
      updatedAt={item.updated_at}
    />
  );
}
