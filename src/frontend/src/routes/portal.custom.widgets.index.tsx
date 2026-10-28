import { createFileRoute } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";

import type { Listed } from "@/api/custom-client";
import {
  DefinitionActions,
  DefinitionCard,
  DefinitionList,
} from "@/components/custom/definition-list";
import { WidgetSummary } from "@/components/custom/definition-summary";
import { NewLink } from "@/components/custom/editor/edit-link";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { useDefinitionCatalogue } from "@/hooks/use-definition-catalogue";
import { useAssistantAvailable } from "@/lib/portal/use-assistant-available";
import { widgetQuery } from "@/queries/custom";
import { TEXT_BODY } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

/** Static, so it wins over `$name` — see the note on the metrics route. */
export const Route = createFileRoute("/portal/custom/widgets/")({
  component: WidgetsCatalogue,
});

function WidgetsCatalogue() {
  const catalogue = useDefinitionCatalogue("widgets");
  const assistant = useAssistantAvailable();

  return (
    <DefinitionList
      title="Widgets"
      blurb="Every stored visual. Each draws one metric; a dashboard holds them."
      items={catalogue.items}
      isLoading={catalogue.isLoading}
      isError={catalogue.isError}
      onRetry={catalogue.refetch}
      search={{ label: "Search widgets", ...catalogue.search }}
      paging={catalogue.paging}
      create={<NewLink kind="widgets" noun="widget" />}
      emptyLabel={
        assistant ? "No widgets yet. Ask the assistant for one." : "No widgets yet."
      }
      renderRow={(item) => <WidgetRow item={item} />}
    />
  );
}

function WidgetRow({ item }: { item: Listed }) {
  const { data, isPending, isError, error } = useQuery(widgetQuery(item.id));

  return (
    <DefinitionCard
      name={item.name}
      actions={
        <DefinitionActions kind="widgets" id={item.id} name={item.name} />
      }
    >
      {isPending ? (
        <CenteredSpinner className="min-h-24" />
      ) : isError ? (
        <p role="alert" className={cn(TEXT_BODY, "text-destructive")}>
          {(error as Error).message}
        </p>
      ) : (
        <WidgetSummary widget={data.widget} />
      )}
    </DefinitionCard>
  );
}
