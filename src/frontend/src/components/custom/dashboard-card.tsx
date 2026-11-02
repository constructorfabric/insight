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
      <CardContent className="flex flex-1 items-center gap-3">
        <LayoutDashboard
          className="size-4 shrink-0 text-muted-foreground"
          aria-hidden
        />
        <div className="flex min-w-0 flex-1 flex-col gap-2">
          <Link
            to="/portal/custom/$name"
            params={{ name: id }}
            className={cn(
              TEXT_NAME,
              "min-w-0 truncate transition-opacity hover:opacity-80"
            )}
            title={title}
          >
            {title}
          </Link>
          <DashboardByline
            owner={dashboard.owner_name}
            updatedAt={dashboard.updated_at}
            visibility={dashboard.visibility}
            access={access}
          />
          {tags.length > 0 ? (
            <div className="h-5">
              <DashboardTags tags={tags} />
            </div>
          ) : null}
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <Link
            to="/portal/custom/$name"
            params={{ name: id }}
            aria-label={`Open ${name}`}
            className="inline-flex size-8 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <ChevronRight className="size-4" aria-hidden />
          </Link>
          <DashboardMenu
            id={id}
            name={name}
            title={storedTitle ?? undefined}
            access={access}
          />
        </div>
      </CardContent>
    </Card>
  );
}
