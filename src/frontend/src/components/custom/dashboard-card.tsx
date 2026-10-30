import { Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { ChevronRight, LayoutDashboard } from "lucide-react";

import type { DashboardListed } from "@/api/custom-client";
import { DashboardByline } from "@/components/custom/dashboard-byline";
import { DashboardMenu } from "@/components/custom/dashboard-menu";
import { DashboardTags } from "@/components/custom/dashboard-tags";
import { Card, CardContent } from "@/components/ui/card";
import { dashboardQuery, dashboardTagsQuery } from "@/queries/custom";
import { TEXT_NAME } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

export function DashboardCard({ dashboard }: { dashboard: DashboardListed }) {
  const { id, name, access } = dashboard;
  const listed = dashboard.tags !== undefined;
  const { data } = useQuery({ ...dashboardQuery(id), enabled: !listed });
  const read = useQuery({ ...dashboardTagsQuery(id), enabled: !listed });
  const storedTitle = listed ? dashboard.title : data?.title;
  const title = storedTitle || name;
  const tags = dashboard.tags ?? read.data ?? [];

  return (
    <Card size="sm" className="h-full">
      <CardContent className="flex flex-1 gap-3">
        <div className="flex min-w-0 flex-1 flex-col gap-2">
          <Link
            to="/portal/custom/$name"
            params={{ name: id }}
            className="flex min-w-0 items-start gap-3 transition-opacity hover:opacity-80"
          >
            <LayoutDashboard
              className="mt-0.5 size-4 shrink-0 text-muted-foreground"
              aria-hidden
            />
            <span className={cn(TEXT_NAME, "min-w-0 truncate")} title={title}>
              {title}
            </span>
          </Link>
          <div className="flex flex-col gap-2">
            <DashboardByline
              owner={dashboard.owner_name}
              updatedAt={dashboard.updated_at}
              visibility={dashboard.visibility}
              access={access}
              className="ps-7"
            />
            {tags.length > 0 ? (
              <div className="h-5 ps-7">
                <DashboardTags tags={tags} />
              </div>
            ) : null}
          </div>
        </div>
        <div className="-my-1 flex shrink-0 flex-col items-center justify-between gap-1">
          <DashboardMenu
            id={id}
            name={name}
            title={storedTitle ?? undefined}
            access={access}
          />
          <Link
            to="/portal/custom/$name"
            params={{ name: id }}
            aria-label={`Open ${name}`}
            className="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <ChevronRight className="size-4" aria-hidden />
          </Link>
        </div>
      </CardContent>
    </Card>
  );
}
