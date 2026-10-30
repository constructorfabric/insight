import { createFileRoute } from "@tanstack/react-router";
import {
  ChartArea,
  ChartColumn,
  ChartLine,
  ChartPie,
  Hash,
  Table2,
} from "lucide-react";

import type { Widget, WidgetListed } from "@/api/custom-client";
import { CatalogueCard } from "@/components/custom/catalogue-card";
import { DefinitionList } from "@/components/custom/definition-list";
import { NewLink } from "@/components/custom/editor/edit-link";
import { useDefinitionCatalogue } from "@/hooks/use-definition-catalogue";
import { useAssistantAvailable } from "@/lib/portal/use-assistant-available";

export const Route = createFileRoute("/portal/custom/widgets/")({
  component: WidgetsCatalogue,
});

const KIND: Record<Widget["type"], { label: string; icon: typeof Hash }> = {
  table: { label: "Table", icon: Table2 },
  line: { label: "Line chart", icon: ChartLine },
  bar: { label: "Bar chart", icon: ChartColumn },
  area: { label: "Area chart", icon: ChartArea },
  stat: { label: "Figure", icon: Hash },
  pie: { label: "Pie chart", icon: ChartPie },
};

function WidgetsCatalogue() {
  const catalogue = useDefinitionCatalogue("widgets");
  const assistant = useAssistantAvailable();

  return (
    <DefinitionList
      title="Widgets"
      items={catalogue.items}
      isLoading={catalogue.isLoading}
      isError={catalogue.isError}
      onRetry={catalogue.refetch}
      search={{ label: "Search widgets", ...catalogue.search }}
      paging={catalogue.paging}
      create={<NewLink kind="widgets" noun="widget" />}
      emptyLabel={
        assistant
          ? "No widgets yet. Ask the assistant for one."
          : "No widgets yet."
      }
      renderRow={(item) => <WidgetCard item={item} />}
    />
  );
}

function WidgetCard({ item }: { item: WidgetListed }) {
  const kind = KIND[item.body.type];

  return (
    <CatalogueCard
      kind="widgets"
      id={item.id}
      name={item.name}
      title={item.body.title || item.name}
      icon={kind.icon}
      facts={[kind.label, item.metric_name ?? "Missing metric"]}
      squeezed={1}
      updatedAt={item.updated_at}
    />
  );
}
