import { Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { ChevronRight, LayoutDashboard } from "lucide-react";

import type { DashboardListed, DashboardVisibility } from "@/api/custom-client";
import { DashboardMenu } from "@/components/custom/dashboard-menu";
import { DashboardTags } from "@/components/custom/dashboard-tags";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { dashboardQuery, dashboardTagsQuery } from "@/queries/custom";
import { describeAge } from "@/lib/portal/connector-health";
import { TEXT_LABEL, TEXT_NAME } from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import { dashboardAllows } from "@/lib/custom/dashboard-access";

const DAY_MS = 24 * 60 * 60 * 1000;

const VISIBILITY: Record<
  DashboardVisibility,
  { label: string; variant: "secondary" | "info" }
> = {
  private: { label: "Private", variant: "secondary" },
  people: { label: "Shared with people", variant: "info" },
  everyone: { label: "Shared with org", variant: "info" },
};

function changedAgo(updatedAt: string): string | null {
  const changed = Date.parse(updatedAt);
  if (Number.isNaN(changed)) return null;

  const age = Math.max(0, Date.now() - changed);
  if (age >= DAY_MS && age < 2 * DAY_MS) return "yesterday";

  return describeAge(age);
}

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
            <CardMeta dashboard={dashboard} />
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

function CardMeta({ dashboard }: { dashboard: DashboardListed }) {
  const { access, owner_name: owner, updated_at } = dashboard;
  const changed = updated_at ? changedAgo(updated_at) : null;
  const visibility = dashboard.visibility
    ? VISIBILITY[dashboard.visibility]
    : undefined;
  const shareable = dashboardAllows(access).share;

  return (
    <div
      className={cn(
        TEXT_LABEL,
        "flex min-h-5 min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1 ps-7 font-normal"
      )}
    >
      {owner ? <span className="max-w-full truncate">{owner}</span> : null}
      {changed ? (
        <time
          dateTime={updated_at}
          className="shrink-0 whitespace-nowrap not-first:before:me-1.5 not-first:before:content-['·']"
        >
          {changed}
        </time>
      ) : null}
      {shareable && visibility ? (
        <Badge variant={visibility.variant} className="shrink-0">
          {visibility.label}
        </Badge>
      ) : null}
    </div>
  );
}
