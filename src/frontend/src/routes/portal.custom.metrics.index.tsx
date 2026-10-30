import { createFileRoute } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";

import type { Listed } from "@/api/custom-client";
import {
  DefinitionActions,
  DefinitionCard,
  DefinitionList,
} from "@/components/custom/definition-list";
import { MetricSummary } from "@/components/custom/definition-summary";
import { NewLink } from "@/components/custom/editor/edit-link";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { useDefinitionCatalogue } from "@/hooks/use-definition-catalogue";
import { useAssistantAvailable } from "@/lib/portal/use-assistant-available";
import { metricQuery } from "@/queries/custom";
import { TEXT_BODY } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

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
      pageTestId="page-custom-metrics"
      title="Metrics"
      blurb={
        assistant
          ? "Every stored query. A widget draws one of these; the assistant can build more."
          : "Every stored query. A widget draws one of these."
      }
      items={catalogue.items}
      isLoading={catalogue.isLoading}
      isError={catalogue.isError}
      onRetry={catalogue.refetch}
      search={{ label: "Search metrics", ...catalogue.search }}
      paging={catalogue.paging}
      create={<NewLink kind="metrics" noun="metric" />}
      emptyLabel={
        assistant ? "No metrics yet. Ask the assistant for one." : "No metrics yet."
      }
      renderRow={(item) => <MetricRow item={item} />}
    />
  );
}

function MetricRow({ item }: { item: Listed }) {
  const { data, isPending, isError, error } = useQuery(metricQuery(item.id));

  return (
    <DefinitionCard
      name={item.name}
      actions={
        <DefinitionActions kind="metrics" id={item.id} name={item.name} />
      }
    >
      {isPending ? (
        <CenteredSpinner className="min-h-24" />
      ) : isError ? (
        <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
          {(error as Error).message}
        </p>
      ) : (
        <MetricSummary definition={data.definition} />
      )}
    </DefinitionCard>
  );
}
