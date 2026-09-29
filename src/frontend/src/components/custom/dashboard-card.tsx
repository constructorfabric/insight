import { Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { ChevronRight, LayoutDashboard } from "lucide-react";

import { DashboardMenu } from "@/components/custom/dashboard-menu";
import { DashboardTags } from "@/components/custom/dashboard-tags";
import { Card, CardContent } from "@/components/ui/card";
import { dashboardQuery, dashboardTagsQuery } from "@/queries/custom";
import { TEXT_LABEL, TEXT_NAME } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

export function DashboardCard({ name }: { name: string }) {
  const { data } = useQuery(dashboardQuery(name));
  const tags = useQuery(dashboardTagsQuery(name));

  return (
    <Card size="sm">
      <CardContent className="flex flex-col gap-2">
        <div className="flex items-center gap-3">
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
            <Link
              to="/portal/custom/$name"
              params={{ name }}
              aria-label={`Open ${name}`}
              className="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
            >
              <ChevronRight className="size-4" aria-hidden />
            </Link>
            <DashboardMenu name={name} title={data?.title} />
          </span>
        </div>
        <DashboardTags tags={tags.data ?? []} className="ps-7" />
      </CardContent>
    </Card>
  );
}
