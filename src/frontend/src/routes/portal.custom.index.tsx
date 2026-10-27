import { createFileRoute } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import type { ReactNode } from "react";

import type { DashboardListed, OwnedFilter } from "@/api/custom-client";
import {
  DefinitionCount,
  MoreDefinitions,
  type Paging,
} from "@/components/custom/definition-paging";
import { DashboardCard } from "@/components/custom/dashboard-card";
import { DefinitionSearch } from "@/components/custom/definition-search";
import { NewLink } from "@/components/custom/editor/edit-link";
import { OrphanedDashboards } from "@/components/custom/orphaned";
import { NoTaggedDashboards, TagFilter } from "@/components/custom/tag-filter";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { useDefinitionCatalogue } from "@/hooks/use-definition-catalogue";
import { useTagPicks } from "@/hooks/use-tag-picks";
import { usePortalSearch } from "@/lib/portal/portal-search";
import { foldersQuery } from "@/queries/custom";
import { useIsAdmin } from "@/queries/identity-me";
import { TEXT_BODY, TEXT_TITLE } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

export const Route = createFileRoute("/portal/custom/")({
  component: CustomDashboardIndex,
});

const UNFILED = "unfiled";

const OWNED_HEADING: Record<OwnedFilter, string> = {
  me: "Owned by me",
  shared: "Shared with me",
};

function emptyLabel(
  folder: string | undefined,
  owned: OwnedFilter | undefined,
  isAdmin: boolean
): string {
  if (owned === "shared") return "Nothing is shared with you yet.";
  if (owned === "me") return "You own no dashboards yet.";
  if (folder === UNFILED) return "Every dashboard is in a folder.";
  if (folder) return "No dashboards in this folder yet.";
  if (!isAdmin) return "Nothing is shared with you yet.";
  return "No dashboards yet. Describe one to the assistant and it will build it.";
}

interface Shown {
  folder?: string;
  heading: string;
  pending?: boolean;
  lost?: boolean;
}

function useShownFolder(): Shown {
  const { folder: asked, owned } = usePortalSearch();
  const folders = useQuery({
    ...foldersQuery(),
    enabled: asked != null && asked !== UNFILED,
  });

  if (!asked) return { heading: owned ? OWNED_HEADING[owned] : "Custom" };
  if (asked === UNFILED) return { folder: UNFILED, heading: "Unfiled" };
  if (folders.isError) return { folder: asked, heading: "Custom" };
  if (!folders.data) return { heading: "Custom", pending: true };

  const found = folders.data.folders.find((folder) => folder.id === asked);
  return found
    ? { folder: found.id, heading: found.name }
    : { heading: "Custom", lost: true };
}

function CustomDashboardIndex() {
  const { owned } = usePortalSearch();
  const { isAdmin } = useIsAdmin();
  const shown = useShownFolder();
  const tags = useTagPicks();
  const catalogue = useDefinitionCatalogue("dashboards", {
    folder: shown.folder,
    tags: tags.picked,
    owned,
    enabled: !shown.pending && !tags.pending,
  });
  const unnarrowed =
    !shown.folder &&
    !shown.pending &&
    !owned &&
    tags.picked.length === 0 &&
    !catalogue.paging.searching;

  return (
    <>
      <header className="mb-3 flex flex-wrap items-start gap-3">
        <div className="min-w-0 grow">
          <h1 className={TEXT_TITLE}>{shown.heading}</h1>
          {isAdmin ? (
            <p className={cn(TEXT_BODY, "text-muted-foreground")}>
              Dashboards built from your own data. Ask the assistant for a new
              one, or write one by hand.
            </p>
          ) : null}
        </div>
        {isAdmin ? <NewLink kind="dashboards" noun="dashboard" /> : null}
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
        <TagFilter tags={tags.tags} picked={tags.picked} onPick={tags.pick} />
        <DefinitionCount
          total={catalogue.paging.total}
          noun="dashboards"
          searching={catalogue.paging.searching}
        />
      </div>
      <CustomDashboardList
        dashboards={catalogue.items}
        isLoading={catalogue.isLoading}
        isError={catalogue.isError}
        onRetry={catalogue.refetch}
        paging={catalogue.paging}
        empty={
          tags.picked.length > 0 ? (
            <NoTaggedDashboards onClear={() => tags.pick([])} />
          ) : (
            <ComingSoon
              variant="card"
              state="empty"
              label={emptyLabel(shown.folder, owned, isAdmin)}
            />
          )
        }
      />
      {isAdmin && unnarrowed ? <OrphanedDashboards /> : null}
    </>
  );
}

function CustomDashboardList({
  dashboards,
  isLoading,
  isError,
  onRetry,
  paging,
  empty,
}: {
  dashboards: DashboardListed[] | undefined;
  isLoading: boolean;
  isError: boolean;
  onRetry: () => void;
  paging: Paging;
  empty: ReactNode;
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

  if (!dashboards) return null;

  if (dashboards.length === 0) return empty;

  return (
    <>
      <ul className="grid gap-3 @xl:grid-cols-2 @5xl:grid-cols-3">
        {dashboards.map((dashboard) => (
          <li key={dashboard.id}>
            <DashboardCard dashboard={dashboard} />
          </li>
        ))}
      </ul>
      <MoreDefinitions {...paging} />
    </>
  );
}
