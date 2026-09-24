import { createFileRoute, Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { LayoutDashboard } from "lucide-react";

import {
  DefinitionCount,
  MoreDefinitions,
  type Paging,
} from "@/components/custom/definition-paging";
import { DefinitionSearch } from "@/components/custom/definition-search";
import { EditLink, NewLink } from "@/components/custom/editor/edit-link";
import { MoveToFolder } from "@/components/custom/move-to-folder";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { useDefinitionCatalogue } from "@/hooks/use-definition-catalogue";
import { usePortalSearch } from "@/lib/portal/portal-search";
import { dashboardQuery, foldersQuery } from "@/queries/custom";
import { TEXT_BODY, TEXT_LABEL, TEXT_NAME, TEXT_TITLE } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

export const Route = createFileRoute("/portal/custom/")({
  component: CustomDashboardIndex,
});

const UNFILED = "unfiled";

interface Shown {
  folder?: string;
  heading: string;
  pending?: boolean;
  lost?: boolean;
}

function useShownFolder(): Shown {
  const { folder: asked } = usePortalSearch();
  const folders = useQuery({
    ...foldersQuery(),
    enabled: asked != null && asked !== UNFILED,
  });

  if (!asked) return { heading: "Custom" };
  if (asked === UNFILED) return { folder: UNFILED, heading: "Unfiled" };
  if (folders.isError) return { folder: asked, heading: "Custom" };
  if (!folders.data) return { heading: "Custom", pending: true };

  const found = folders.data.folders.find((folder) => folder.id === asked);
  return found
    ? { folder: found.id, heading: found.name }
    : { heading: "Custom", lost: true };
}

function CustomDashboardIndex() {
  const shown = useShownFolder();
  const catalogue = useDefinitionCatalogue("dashboards", {
    folder: shown.folder,
    enabled: !shown.pending,
  });

  return (
    <>
      <header className="mb-3 flex flex-wrap items-start gap-3 pe-12">
        <div className="min-w-0 grow">
          <h1 className={TEXT_TITLE}>{shown.heading}</h1>
          <p className={cn(TEXT_BODY, "text-muted-foreground")}>
            Dashboards built from your own data. Ask the assistant for a new
            one, or write one by hand.
          </p>
        </div>
        <NewLink kind="dashboards" noun="dashboard" />
      </header>
      {shown.lost ? (
        <p
          role="status"
          className={cn(TEXT_BODY, "mb-3 text-muted-foreground")}
        >
          That folder no longer exists, so every dashboard is shown.
        </p>
      ) : null}
      <div className="mb-4 flex flex-wrap items-center gap-3">
        <DefinitionSearch
          label="Search dashboards"
          value={catalogue.search.value}
          onChange={catalogue.search.onChange}
        />
        <DefinitionCount
          total={catalogue.paging.total}
          noun="dashboards"
          searching={catalogue.paging.searching}
        />
      </div>
      <CustomDashboardList
        names={catalogue.names}
        isLoading={catalogue.isLoading}
        isError={catalogue.isError}
        onRetry={catalogue.refetch}
        paging={catalogue.paging}
      />
    </>
  );
}

/** Titled by the dashboard, with the identifier it is stored under beneath. */
function DashboardCard({ name }: { name: string }) {
  const { data } = useQuery(dashboardQuery(name));

  return (
    <Card size="sm">
      <CardContent className="flex items-center gap-3">
        <Link
          to="/portal/custom/$name"
          params={{ name }}
          className="flex min-w-0 flex-1 items-center gap-3 transition-opacity hover:opacity-80"
        >
          <LayoutDashboard
            className="size-4 shrink-0 text-muted-foreground"
            aria-hidden
          />
          <span className="flex min-w-0 flex-col">
            <span className={cn(TEXT_NAME, "truncate")}>
              {data?.title ?? name}
            </span>
            {data?.title ? (
              <span className={cn(TEXT_LABEL, "truncate font-mono")}>
                {name}
              </span>
            ) : null}
          </span>
        </Link>
        <span className="flex shrink-0 items-center gap-1">
          <MoveToFolder name={name} />
          <EditLink kind="dashboards" name={name} />
          <Button
            variant="ghost"
            size="sm"
            className="text-muted-foreground"
            aria-label={`View ${name}`}
            nativeButton={false}
            render={<Link to="/portal/custom/$name" params={{ name }} />}
          >
            View
          </Button>
        </span>
      </CardContent>
    </Card>
  );
}

function CustomDashboardList({
  names,
  isLoading,
  isError,
  onRetry,
  paging,
}: {
  names: string[] | undefined;
  isLoading: boolean;
  isError: boolean;
  onRetry: () => void;
  paging: Paging;
}) {
  if (isLoading) return <CenteredSpinner className="min-h-40" />;
  if (isError) {
    return (
      <ComingSoon
        variant="card"
        state="error"
        label="Couldn't load the dashboard list."
        onRetry={onRetry}
      />
    );
  }

  if (!names) return null;

  if (names.length === 0) {
    return (
      <ComingSoon
        variant="card"
        state="empty"
        label="No dashboards yet. Describe one to the assistant and it will build it."
      />
    );
  }

  return (
    <>
      <ul className="grid gap-3 @xl:grid-cols-2 @5xl:grid-cols-3">
        {names.map((name) => (
          <li key={name}>
            <DashboardCard name={name} />
          </li>
        ))}
      </ul>
      <MoreDefinitions {...paging} />
    </>
  );
}
