import { useQuery } from "@tanstack/react-query";

import { NewDashboardButton } from "@/components/custom/create-dashboard-dialog";
import { DashboardCard } from "@/components/custom/dashboard-card";
import { Button } from "@/components/ui/button";
import { useZoneNav } from "@/lib/portal/use-zone-nav";
import { TEXT_EYEBROW } from "@/lib/type-scale";
import { recentDashboardsQuery } from "@/queries/custom";

const RECENT_COUNT = 3;

export function HomeDashboards() {
  const { zones } = useZoneNav();
  if (!zones.some((zone) => zone.id === "custom")) return null;

  return <DashboardSection />;
}

function DashboardSection() {
  const dashboards = useQuery(recentDashboardsQuery(RECENT_COUNT));
  const shown = dashboards.data?.items ?? [];

  // A viewer with no dashboard to open has no section; while that is unknown
  // nothing is drawn, so the section never appears only to vanish.
  if (dashboards.isPending) return null;
  if (!dashboards.isError && shown.length === 0) return null;

  return (
    <section aria-labelledby="home-dashboards" className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 id="home-dashboards" className={TEXT_EYEBROW}>
          Recent dashboards
        </h2>
        <NewDashboardButton variant="outline" />
      </div>
      {dashboards.isError ? (
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
      ) : (
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
      )}
    </section>
  );
}
