import { Link } from "@tanstack/react-router";
import { useInfiniteQuery } from "@tanstack/react-query";

import { DashboardCard } from "@/components/custom/dashboard-card";
import { NewLink } from "@/components/custom/editor/edit-link";
import { Button } from "@/components/ui/button";
import { useZoneNav } from "@/lib/portal/use-zone-nav";
import { TEXT_EYEBROW } from "@/lib/type-scale";
import { definitionPagesQuery } from "@/queries/custom";

export function HomeDashboards() {
  const { zones } = useZoneNav();
  if (!zones.some((zone) => zone.id === "custom")) return null;

  return <DashboardSection />;
}

function DashboardSection() {
  const dashboards = useInfiniteQuery(definitionPagesQuery("dashboards"));
  const shown =
    dashboards.data?.pages.flatMap((page) => page.items).slice(0, 3) ?? [];

  return (
    <section aria-labelledby="home-dashboards" className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 id="home-dashboards" className={TEXT_EYEBROW}>
          Dashboards
        </h2>
        <div className="flex items-center gap-2">
          <Button
            variant="ghost"
            size="sm"
            nativeButton={false}
            render={<Link to="/portal/custom" />}
          >
            All dashboards
          </Button>
          <NewLink kind="dashboards" noun="dashboard" />
        </div>
      </div>
      {dashboards.isPending ? (
        <p role="status" className="text-sm text-muted-foreground">
          Loading dashboards…
        </p>
      ) : dashboards.isError ? (
        <div
          role="alert"
          className="flex flex-wrap items-center gap-3 rounded-xl border bg-card p-4"
        >
          <p className="text-sm text-muted-foreground">
            Couldn't load dashboards.
          </p>
          <Button
            variant="outline"
            size="sm"
            onClick={() => void dashboards.refetch()}
          >
            Try again
          </Button>
        </div>
      ) : shown.length ? (
        <ul className="grid gap-4 @2xl:grid-cols-2">
          {shown.map((dashboard) => (
            <li
              key={dashboard.id}
              className="[&_[data-slot=card-content]]:p-0 [&_[data-slot=card]]:p-4"
            >
              <DashboardCard dashboard={dashboard} />
            </li>
          ))}
        </ul>
      ) : (
        <p className="rounded-xl border bg-card p-4 text-sm text-muted-foreground">
          No dashboards yet.
        </p>
      )}
    </section>
  );
}
