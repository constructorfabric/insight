import { createFileRoute } from "@tanstack/react-router";
import { Sigma } from "lucide-react";

import type { MetricListed } from "@/api/custom-client";
import { CatalogueCard } from "@/components/custom/catalogue-card";
import { DefinitionList } from "@/components/custom/definition-list";
import { NewLink } from "@/components/custom/editor/edit-link";
import { useDefinitionCatalogue } from "@/hooks/use-definition-catalogue";
import { metricSource } from "@/lib/custom/metric-source";
import { useAssistantAvailable } from "@/lib/portal/use-assistant-available";

/**
 * A static segment, so it wins over `$name` in the route tree — a dashboard
 * literally called "metrics" would be unreachable, which is the trade for a
 * readable URL.
 */
export const Route = createFileRoute("/portal/custom/metrics/")({
  component: MetricsCatalogue,
});

function MetricsCatalogue() {
  const catalogue = useDefinitionCatalogue("metrics");
  const assistant = useAssistantAvailable();

  return (
    <DefinitionList
      title="Metrics"
      items={catalogue.items}
      isLoading={catalogue.isLoading}
      isError={catalogue.isError}
      onRetry={catalogue.refetch}
      search={{ label: "Search metrics", ...catalogue.search }}
      paging={catalogue.paging}
      create={<NewLink kind="metrics" noun="metric" />}
      emptyLabel={
        assistant
          ? "No metrics yet. Ask the assistant for one."
          : "No metrics yet."
      }
      renderRow={(item) => <MetricCard item={item} />}
    />
  );
}

function MetricCard({ item }: { item: MetricListed }) {
  const source = metricSource(item.body);
  const fields = item.body.fields?.length ?? 0;

  return (
    <CatalogueCard
      kind="metrics"
      id={item.id}
      name={item.name}
      title={item.name}
      icon={Sigma}
      facts={[source.named, `${fields} ${fields === 1 ? "column" : "columns"}`]}
      updatedAt={item.updated_at}
    />
  );
}
