import { Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { ChevronRight, LayoutDashboard } from "lucide-react";

import type { DashboardListed } from "@/api/custom-client";
import { DashboardMenu } from "@/components/custom/dashboard-menu";
import { DashboardTags } from "@/components/custom/dashboard-tags";
import { PersonName } from "@/components/custom/person-name";
import { Card, CardContent } from "@/components/ui/card";
import { dashboardQuery, dashboardTagsQuery } from "@/queries/custom";
import { TEXT_LABEL, TEXT_NAME } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

export function DashboardCard({ dashboard }: { dashboard: DashboardListed }) {
  const { id, name, access, owner_person_id: owner } = dashboard;
  const { data } = useQuery(dashboardQuery(id));
  const tags = useQuery(dashboardTagsQuery(id));
  const title = data?.title || name;

  return (
    <Card size="sm">
      <CardContent className="flex flex-col gap-2">
        <div className="flex items-center gap-3">
          <Link
            to="/portal/custom/$name"
            params={{ name: id }}
            className="flex min-w-0 flex-1 items-center gap-3 transition-opacity hover:opacity-80"
          >
            <LayoutDashboard
              className="size-4 shrink-0 text-muted-foreground"
              aria-hidden
            />
            <span className="flex min-w-0 flex-col">
              <span className={cn(TEXT_NAME, "truncate")}>{title}</span>
              {title !== name ? (
                <span className={cn(TEXT_LABEL, "truncate")}>{name}</span>
              ) : null}
              {owner ? (
                <span className={cn(TEXT_LABEL, "truncate font-normal")}>
                  Owner: <PersonName personId={owner} />
                </span>
              ) : null}
            </span>
          </Link>
          <span className="flex shrink-0 items-center gap-1">
            <Link
              to="/portal/custom/$name"
              params={{ name: id }}
              aria-label={`Open ${name}`}
              className="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
            >
              <ChevronRight className="size-4" aria-hidden />
            </Link>
            <DashboardMenu
              id={id}
              name={name}
              title={data?.title}
              access={access}
            />
          </span>
        </div>
        <DashboardTags tags={tags.data ?? []} className="ps-7" />
      </CardContent>
    </Card>
  );
}
