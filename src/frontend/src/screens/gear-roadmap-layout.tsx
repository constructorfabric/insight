import { Link, Outlet, useRouterState } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { SidebarTrigger } from "@/components/ui/sidebar";
import { cn } from "@/lib/utils";
import { useGearRoadmap } from "@/queries/gear-roadmap";

const TABS = [
  { to: "/gears", key: "overview" },
  { to: "/gears/items", key: "items" },
  { to: "/gears/roadmap", key: "roadmap" },
  { to: "/gears/gantt", key: "gantt" },
] as const;

export function GearRoadmapLayout() {
  const { t } = useTranslation();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const { data } = useGearRoadmap();

  return (
    <>
      <header className="sticky top-0 z-20 flex flex-wrap items-center gap-3 border-b bg-background/95 px-4 py-3 backdrop-blur-sm">
        <SidebarTrigger />
        <h1 className="text-lg font-semibold tracking-tight">
          {t("gear_roadmap.title")}
        </h1>
        {data ? (
          <Badge variant="outline" className="ms-auto font-normal">
            {t("gear_roadmap.capacity_note", {
              capacity: data.capacity_man_days_per_person,
            })}
          </Badge>
        ) : null}
      </header>

      <nav className="flex gap-1 border-b px-4 py-2">
        {TABS.map((tab) => (
          <Link
            key={tab.to}
            to={tab.to}
            className={cn(
              "rounded-md px-3 py-1.5 text-sm",
              pathname === tab.to
                ? "bg-accent font-medium text-accent-foreground"
                : "text-muted-foreground hover:bg-accent/50",
            )}
          >
            {t(`gear_roadmap.tabs.${tab.key}`)}
          </Link>
        ))}
      </nav>

      <div className="px-4 py-4">
        <Outlet />
      </div>
    </>
  );
}
