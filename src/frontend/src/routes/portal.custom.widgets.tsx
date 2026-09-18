import { createFileRoute } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";

import {
  DefinitionCard,
  DefinitionList,
} from "@/components/custom/definition-list";
import { WidgetSummary } from "@/components/custom/definition-summary";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { useDefinitionCatalogue } from "@/hooks/use-definition-catalogue";
import { widgetQuery } from "@/queries/custom";
import { TEXT_BODY } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

/** Static, so it wins over `$name` — see the note on the metrics route. */
export const Route = createFileRoute("/portal/custom/widgets")({
  component: WidgetsCatalogue,
});

function WidgetsCatalogue() {
  const catalogue = useDefinitionCatalogue("widgets");

  return (
    <DefinitionList
      title="Widgets"
      blurb="Every stored visual. Each draws one metric; a dashboard holds them."
      names={catalogue.names}
      isLoading={catalogue.isLoading}
      isError={catalogue.isError}
      onRetry={catalogue.refetch}
      search={{ label: "Search widgets", ...catalogue.search }}
      paging={catalogue.paging}
      emptyLabel="No widgets yet. Ask the assistant for one."
      renderRow={(name) => <WidgetRow name={name} />}
    />
  );
}

function WidgetRow({ name }: { name: string }) {
  const { data, isPending, isError, error } = useQuery(widgetQuery(name));

  return (
    <DefinitionCard name={name} kind="widgets">
      {isPending ? (
        <CenteredSpinner className="min-h-24" />
      ) : isError ? (
        <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
          {(error as Error).message}
        </p>
      ) : (
        <WidgetSummary widget={data} />
      )}
    </DefinitionCard>
  );
}
